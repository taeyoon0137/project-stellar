//! blob content reader — `git cat-file --batch` (XIV.2.6).
//!
//! 요청한 object ID를 한 줄에 하나씩 stdin으로 넘기고(`<oid>\n`), 출력
//! `<oid> SP <type> SP <size> LF <contents> LF`(없으면 `<oid> SP missing LF`)를 요청 순서대로
//! 해석합니다. content는 Git이 저장한 **원시 바이트**이며 filter·textconv를 적용하지 않습니다.
//!
//! 크기 상한(SEC-3):
//! - 내용을 읽기 전에 `git cat-file --batch-check`로 모든 blob의 종류·크기를 확인합니다. blob 하나가
//!   [`BlobLimits::max_blob_bytes`]를 넘으면 [`BlobError::BlobTooLarge`], 읽을 내용 전체가
//!   [`BlobLimits::max_total_bytes`]를 넘으면 [`BlobError::TotalTooLarge`]이며 내용은 읽지 않습니다.
//! - 한 번의 `cat-file --batch` 실행은 [`BATCH_CHUNK`]개와 [`BlobLimits::max_batch_bytes`] 중 먼저 닿는
//!   쪽까지만 읽습니다. batch 상한보다 큰 blob 하나는 단독으로 읽습니다. 그래서 한 실행의 출력은 batch
//!   상한과 blob 상한 중 큰 값(+ header)을 넘지 않습니다.
//! - materialize(XIV.2.6)와 stage candidate(XIV.5.5)는 요청 순서대로 읽는 내부 reader로 batch 하나만
//!   메모리에 두고 Git 출력 buffer에서 바로 씁니다(blob별 복사 없음). [`read_blobs`]는 결과 전체를
//!   메모리에 담으므로 전체 상한이 곧 메모리 상한입니다(blob이 하나인 batch는 출력 buffer를 그대로
//!   씁니다).
//! - 해석 오류에는 원시 출력의 앞 [`RAW_PREFIX_BYTES`]만 보존합니다.
//!
//! 같은 ID는 한 번만 요청합니다. 응답 ID가 요청과 다르거나, 크기가 batch-check와 다르거나, 크기만큼
//! 바이트가 없거나, 끝 LF가 없거나, 남는 바이트가 있으면 해석 오류입니다. 없는 object는
//! [`BlobError::Missing`], blob이 아닌 object는 [`BlobError::NotABlob`]입니다.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::Path;

use crate::object::{ObjectId, ObjectKind};
use crate::ops::{require_success, GitOpError, ParseFailure};
use crate::runner::argv;
use crate::trust::GatedGitRunner;

const COMMAND: &str = "cat-file --batch";
const CHECK_COMMAND: &str = "cat-file --batch-check";

/// 한 번의 `cat-file --batch` 실행에 넘기는 최대 ID 수.
pub const BATCH_CHUNK: usize = 256;

/// 한 번의 `cat-file --batch-check` 실행에 넘기는 최대 ID 수(출력은 ID당 한 줄).
const CHECK_CHUNK: usize = 4096;

/// 해석 오류([`ParseFailure::raw`])에 보존하는 원시 출력 앞부분의 최대 크기.
pub const RAW_PREFIX_BYTES: usize = 4096;

const MIB: u64 = 1024 * 1024;

/// blob 읽기 크기 상한(SEC-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlobLimits {
    /// blob 하나의 최대 크기(byte). blob 하나는 통째로 메모리에 올라가므로 최대 메모리를 정합니다.
    pub max_blob_bytes: u64,
    /// 한 번의 `cat-file --batch` 실행이 읽는 content 합의 상한(byte). 이보다 큰 blob 하나는 단독으로
    /// 읽습니다.
    pub max_batch_bytes: u64,
    /// 한 번의 호출이 읽는 content 전체 합의 상한(byte).
    pub max_total_bytes: u64,
}

impl BlobLimits {
    /// 임시 디렉터리에 쓰는 materialize 기본값: blob 256 MiB, batch 64 MiB, 전체 4 GiB(디스크 사용).
    pub const MATERIALIZE: BlobLimits = BlobLimits {
        max_blob_bytes: 256 * MIB,
        max_batch_bytes: 64 * MIB,
        max_total_bytes: 4096 * MIB,
    };

    /// 결과 전체를 메모리에 담는 [`read_blobs`] 기본값: 전체 512 MiB(나머지는 materialize와 같음).
    pub const IN_MEMORY: BlobLimits = BlobLimits {
        max_total_bytes: 512 * MIB,
        ..BlobLimits::MATERIALIZE
    };
}

/// blob 읽기 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobError {
    /// Git 실행·해석 실패.
    Op(GitOpError),
    /// object가 저장소에 없음(LFS pointer만 있는 경우와 별개로, Git object 자체가 없음).
    Missing(ObjectId),
    /// blob이 아닌 object.
    NotABlob {
        /// 요청한 ID.
        oid: ObjectId,
        /// 실제 종류.
        kind: ObjectKind,
    },
    /// blob 하나가 [`BlobLimits::max_blob_bytes`]보다 큼(내용을 읽지 않음).
    BlobTooLarge {
        /// 요청한 ID.
        oid: ObjectId,
        /// `cat-file --batch-check`가 알려 준 크기(byte).
        size: u64,
        /// 상한(byte).
        max_bytes: u64,
    },
    /// 읽을 내용 전체가 [`BlobLimits::max_total_bytes`]보다 큼(내용을 읽지 않음).
    TotalTooLarge {
        /// 상한(byte).
        max_bytes: u64,
    },
}

impl std::fmt::Display for BlobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlobError::Op(e) => write!(f, "{e}"),
            BlobError::Missing(oid) => write!(f, "Git object 없음: {oid}"),
            BlobError::NotABlob { oid, kind } => {
                write!(f, "blob이 아닌 object: {oid} ({})", kind.as_str())
            }
            BlobError::BlobTooLarge {
                oid,
                size,
                max_bytes,
            } => write!(
                f,
                "blob이 크기 상한을 넘습니다: {oid} ({size} bytes, 상한 {max_bytes} bytes)"
            ),
            BlobError::TotalTooLarge { max_bytes } => {
                write!(f, "읽을 blob 전체 크기가 상한 {max_bytes} bytes를 넘습니다")
            }
        }
    }
}

impl std::error::Error for BlobError {}

impl From<GitOpError> for BlobError {
    fn from(e: GitOpError) -> BlobError {
        BlobError::Op(e)
    }
}

/// 해석 오류. 원시 출력은 앞 [`RAW_PREFIX_BYTES`]만 보존합니다(큰 blob 내용을 복사하지 않음).
fn parse_error(command: &'static str, detail: &str, raw: &[u8]) -> BlobError {
    let prefix = &raw[..raw.len().min(RAW_PREFIX_BYTES)];
    BlobError::Op(GitOpError::Parse(ParseFailure::new(
        command, detail, prefix,
    )))
}

/// 요청 ID를 한 줄에 하나씩 담은 stdin 입력.
fn oid_lines<'a>(oids: impl IntoIterator<Item = &'a ObjectId>) -> Vec<u8> {
    let mut input = Vec::new();
    for oid in oids {
        input.extend_from_slice(oid.as_str().as_bytes());
        input.push(b'\n');
    }
    input
}

/// `<oid> SP <type> SP <size>` header(LF 제외)를 해석합니다. blob이면 크기를 돌려줍니다.
fn parse_header(command: &'static str, header: &[u8], oid: &ObjectId) -> Result<u64, BlobError> {
    let mut fields = header.split(|b| *b == b' ');
    let got = fields.next().unwrap_or_default();
    if got != oid.as_str().as_bytes() {
        return Err(parse_error(command, "응답 ID가 요청과 다름", header));
    }
    let (kind, size) = match (fields.next(), fields.next(), fields.next()) {
        (Some(b"missing"), None, None) => return Err(BlobError::Missing(oid.clone())),
        (Some(kind), Some(size), None) => (kind, size),
        _ => return Err(parse_error(command, "잘못된 header", header)),
    };
    let kind =
        ObjectKind::from_name(kind).ok_or_else(|| parse_error(command, "잘못된 종류", header))?;
    let size: u64 = std::str::from_utf8(size)
        .ok()
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| parse_error(command, "잘못된 크기", header))?;
    if kind != ObjectKind::Blob {
        return Err(BlobError::NotABlob {
            oid: oid.clone(),
            kind,
        });
    }
    Ok(size)
}

/// `cat-file --batch` 출력에서 요청 순서대로 각 blob 내용의 범위를 찾습니다(복사 없음).
fn parse_batch_ranges(
    stdout: &[u8],
    requested: &[ObjectId],
) -> Result<Vec<Range<usize>>, BlobError> {
    let mut offset = 0usize;
    let mut ranges = Vec::with_capacity(requested.len());
    for oid in requested {
        let rest = &stdout[offset..];
        let lf = rest
            .iter()
            .position(|b| *b == b'\n')
            .ok_or_else(|| parse_error(COMMAND, "header LF 없음", rest))?;
        let header = &rest[..lf];
        let size = parse_header(COMMAND, header, oid)?;
        let start = offset + lf + 1;
        // 크기는 신뢰할 수 없는 출력이므로 `start + size`를 overflow 없이 계산하고, 끝 LF까지 남은
        // 바이트가 없으면 해석 오류로 돌려줍니다(slice 범위 panic 방지).
        let end = usize::try_from(size)
            .ok()
            .and_then(|size| start.checked_add(size))
            .filter(|end| *end < stdout.len())
            .ok_or_else(|| parse_error(COMMAND, "content가 잘림", header))?;
        if stdout[end] != b'\n' {
            return Err(parse_error(COMMAND, "content 뒤 LF 없음", header));
        }
        ranges.push(start..end);
        offset = end + 1;
    }
    if offset != stdout.len() {
        return Err(parse_error(
            COMMAND,
            "요청보다 많은 출력",
            &stdout[offset..],
        ));
    }
    Ok(ranges)
}

/// `cat-file --batch` 출력을 요청 순서대로 해석합니다.
pub fn parse_cat_file_batch(
    stdout: &[u8],
    requested: &[ObjectId],
) -> Result<Vec<(ObjectId, Vec<u8>)>, BlobError> {
    let ranges = parse_batch_ranges(stdout, requested)?;
    Ok(requested
        .iter()
        .cloned()
        .zip(ranges.into_iter().map(|range| stdout[range].to_vec()))
        .collect())
}

/// `cat-file --batch-check` 출력(`<oid> SP <type> SP <size> LF`)을 요청 순서대로 해석해 크기를
/// 돌려줍니다.
fn parse_batch_check(stdout: &[u8], requested: &[ObjectId]) -> Result<Vec<u64>, BlobError> {
    let mut rest = stdout;
    let mut sizes = Vec::with_capacity(requested.len());
    for oid in requested {
        let lf = rest
            .iter()
            .position(|b| *b == b'\n')
            .ok_or_else(|| parse_error(CHECK_COMMAND, "LF 없음", rest))?;
        sizes.push(parse_header(CHECK_COMMAND, &rest[..lf], oid)?);
        rest = &rest[lf + 1..];
    }
    if !rest.is_empty() {
        return Err(parse_error(CHECK_COMMAND, "요청보다 많은 출력", rest));
    }
    Ok(sizes)
}

/// 정렬·중복 제거한 ID의 blob 크기를 `cat-file --batch-check`로 확인합니다. blob 하나가 상한을 넘으면
/// 오류입니다.
fn blob_sizes(
    git: &GatedGitRunner,
    worktree: &Path,
    unique: &[ObjectId],
    limits: &BlobLimits,
) -> Result<BTreeMap<ObjectId, u64>, BlobError> {
    let mut sizes = BTreeMap::new();
    for chunk in unique.chunks(CHECK_CHUNK) {
        let output = git
            .run_with_input(
                &argv(["cat-file", "--batch-check"]),
                worktree,
                &oid_lines(chunk),
            )
            .map_err(GitOpError::Gate)?;
        let output = require_success(output)?;
        for (oid, size) in chunk.iter().zip(parse_batch_check(&output.stdout, chunk)?) {
            if size > limits.max_blob_bytes {
                return Err(BlobError::BlobTooLarge {
                    oid: oid.clone(),
                    size,
                    max_bytes: limits.max_blob_bytes,
                });
            }
            sizes.insert(oid.clone(), size);
        }
    }
    Ok(sizes)
}

/// `order`에 담긴 크기의 합(같은 ID 반복 포함)이 전체 상한 안인지 확인합니다.
fn check_total<'a>(
    order: impl IntoIterator<Item = &'a ObjectId>,
    sizes: &BTreeMap<ObjectId, u64>,
    limits: &BlobLimits,
) -> Result<(), BlobError> {
    order
        .into_iter()
        .try_fold(0u64, |total, oid| {
            total.checked_add(sizes.get(oid).copied().unwrap_or(0))
        })
        .filter(|total| *total <= limits.max_total_bytes)
        .map(|_| ())
        .ok_or(BlobError::TotalTooLarge {
            max_bytes: limits.max_total_bytes,
        })
}

/// `order[start..]`에서 다음 batch를 고릅니다. batch는 [`BATCH_CHUNK`]개와 batch 바이트 상한 중 먼저
/// 닿는 쪽까지이며(첫 blob은 크기와 무관하게 넣음), 이미 batch에 든 ID의 반복은 함께 덮습니다.
/// `(batch가 덮는 order 범위의 끝, batch에 넣을 고유 ID와 크기)`를 돌려줍니다.
fn plan_batch(
    order: &[ObjectId],
    start: usize,
    sizes: &BTreeMap<ObjectId, u64>,
    limits: &BlobLimits,
) -> (usize, Vec<(ObjectId, u64)>) {
    let mut items: Vec<(ObjectId, u64)> = Vec::new();
    let mut members: BTreeSet<&ObjectId> = BTreeSet::new();
    let mut bytes = 0u64;
    let mut end = start;
    while let Some(oid) = order.get(end) {
        if !members.contains(oid) {
            let size = sizes.get(oid).copied().unwrap_or(0);
            if !items.is_empty()
                && (items.len() >= BATCH_CHUNK
                    || bytes.saturating_add(size) > limits.max_batch_bytes)
            {
                break;
            }
            members.insert(oid);
            items.push((oid.clone(), size));
            bytes = bytes.saturating_add(size);
        }
        end += 1;
    }
    (end, items)
}

/// 한 번의 `cat-file --batch` 결과: 출력 buffer와 blob별 내용 범위.
struct BlobBatch {
    stdout: Vec<u8>,
    ranges: BTreeMap<ObjectId, Range<usize>>,
}

impl BlobBatch {
    /// `items`(고유 ID와 batch-check 크기)를 한 번의 `cat-file --batch`로 읽습니다.
    fn read(
        git: &GatedGitRunner,
        worktree: &Path,
        items: &[(ObjectId, u64)],
    ) -> Result<BlobBatch, BlobError> {
        let oids: Vec<ObjectId> = items.iter().map(|(oid, _)| oid.clone()).collect();
        let output = git
            .run_with_input(&argv(["cat-file", "--batch"]), worktree, &oid_lines(&oids))
            .map_err(GitOpError::Gate)?;
        let stdout = require_success(output)?.stdout;
        let found = parse_batch_ranges(&stdout, &oids)?;
        let mut ranges = BTreeMap::new();
        for ((oid, expected), range) in items.iter().zip(found) {
            if u64::try_from(range.len()).ok() != Some(*expected) {
                return Err(parse_error(
                    COMMAND,
                    "크기가 batch-check와 다름",
                    oid.as_str().as_bytes(),
                ));
            }
            ranges.insert(oid.clone(), range);
        }
        Ok(BlobBatch { stdout, ranges })
    }

    /// `oid`의 내용(batch 출력 buffer 안).
    fn get(&self, oid: &ObjectId) -> Option<&[u8]> {
        self.ranges
            .get(oid)
            .and_then(|range| self.stdout.get(range.clone()))
    }

    /// 내용을 소유 값으로 넘깁니다. blob이 하나면 출력 buffer를 복사 없이 그대로 씁니다.
    fn into_contents(self) -> Vec<(ObjectId, Vec<u8>)> {
        let BlobBatch { mut stdout, ranges } = self;
        if ranges.len() == 1 {
            return ranges
                .into_iter()
                .map(|(oid, range)| {
                    stdout.truncate(range.end);
                    stdout.drain(..range.start);
                    (oid, std::mem::take(&mut stdout))
                })
                .collect();
        }
        ranges
            .into_iter()
            .map(|(oid, range)| (oid, stdout[range].to_vec()))
            .collect()
    }
}

/// blob content를 읽습니다([`BlobLimits::IN_MEMORY`] 상한, 중복 ID는 한 번만 요청).
///
/// 결과 전체를 메모리에 담습니다. 크기를 먼저 확인하고 batch 단위로 읽습니다(모듈 문서).
pub fn read_blobs(
    git: &GatedGitRunner,
    worktree: &Path,
    oids: &[ObjectId],
) -> Result<BTreeMap<ObjectId, Vec<u8>>, BlobError> {
    read_blobs_with_limits(git, worktree, oids, &BlobLimits::IN_MEMORY)
}

/// `limits` 상한으로 blob content를 읽습니다(중복 ID는 한 번만 요청).
///
/// 전체 상한은 고유 ID 크기의 합(결과 메모리)에 적용합니다. 상한을 넘으면 내용을 읽기 전에 오류입니다.
pub fn read_blobs_with_limits(
    git: &GatedGitRunner,
    worktree: &Path,
    oids: &[ObjectId],
    limits: &BlobLimits,
) -> Result<BTreeMap<ObjectId, Vec<u8>>, BlobError> {
    let mut unique: Vec<ObjectId> = oids.to_vec();
    unique.sort();
    unique.dedup();
    let sizes = blob_sizes(git, worktree, &unique, limits)?;
    check_total(&unique, &sizes, limits)?;
    let mut blobs = BTreeMap::new();
    let mut start = 0;
    while start < unique.len() {
        let (end, items) = plan_batch(&unique, start, &sizes, limits);
        blobs.extend(BlobBatch::read(git, worktree, &items)?.into_contents());
        start = end;
    }
    Ok(blobs)
}

/// 정해진 순서의 blob 요청을 batch 하나씩 읽는 reader(materialize·stage candidate용).
///
/// 생성할 때 모든 blob의 크기를 확인하고 상한을 검사하므로, 상한을 넘으면 내용을 읽거나 파일을 쓰기 전에
/// 실패합니다. 전체 상한은 요청 순서의 크기 합(같은 ID 반복 포함, 즉 쓸 바이트)에 적용합니다. 메모리에는
/// 현재 batch의 출력 buffer 하나만 둡니다.
pub(crate) struct OrderedBlobReader<'a> {
    git: &'a GatedGitRunner,
    worktree: &'a Path,
    limits: BlobLimits,
    order: Vec<ObjectId>,
    sizes: BTreeMap<ObjectId, u64>,
    cursor: usize,
    batch_end: usize,
    batch: Option<BlobBatch>,
}

impl<'a> OrderedBlobReader<'a> {
    /// `order` 순서로 읽을 reader를 만듭니다(크기 확인·상한 검사).
    pub(crate) fn new(
        git: &'a GatedGitRunner,
        worktree: &'a Path,
        order: Vec<ObjectId>,
        limits: &BlobLimits,
    ) -> Result<OrderedBlobReader<'a>, BlobError> {
        let mut unique = order.clone();
        unique.sort();
        unique.dedup();
        let sizes = blob_sizes(git, worktree, &unique, limits)?;
        check_total(&order, &sizes, limits)?;
        Ok(OrderedBlobReader {
            git,
            worktree,
            limits: *limits,
            order,
            sizes,
            cursor: 0,
            batch_end: 0,
            batch: None,
        })
    }

    /// 다음 요청 `oid`의 내용. 생성할 때 준 순서대로 불러야 하며, 순서가 다르면
    /// [`BlobError::Missing`]입니다.
    pub(crate) fn next_content(&mut self, oid: &ObjectId) -> Result<&[u8], BlobError> {
        if self.order.get(self.cursor) != Some(oid) {
            debug_assert!(false, "OrderedBlobReader 요청 순서가 생성 순서와 다름");
            return Err(BlobError::Missing(oid.clone()));
        }
        if self.cursor >= self.batch_end {
            // 이전 batch를 먼저 버려 batch 두 개가 동시에 메모리에 있지 않게 합니다.
            self.batch = None;
            let (end, items) = plan_batch(&self.order, self.cursor, &self.sizes, &self.limits);
            self.batch = Some(BlobBatch::read(self.git, self.worktree, &items)?);
            self.batch_end = end;
        }
        self.cursor += 1;
        self.batch
            .as_ref()
            .and_then(|batch| batch.get(oid))
            .ok_or_else(|| BlobError::Missing(oid.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(c: char) -> ObjectId {
        ObjectId::parse(&c.to_string().repeat(40)).unwrap()
    }

    fn response(id: &ObjectId, kind: &str, content: &[u8]) -> Vec<u8> {
        let mut v = format!("{id} {kind} {}\n", content.len()).into_bytes();
        v.extend_from_slice(content);
        v.push(b'\n');
        v
    }

    #[test]
    fn parses_binary_content_with_newlines_and_nuls() {
        let (a, b) = (oid('a'), oid('b'));
        let binary: Vec<u8> = (0..=255u8).chain([b'\n', 0, b'\n']).collect();
        let mut out = response(&a, "blob", &binary);
        out.extend(response(&b, "blob", b""));
        let parsed = parse_cat_file_batch(&out, &[a.clone(), b.clone()]).unwrap();
        assert_eq!(parsed, vec![(a, binary), (b, Vec::new())]);
    }

    #[test]
    fn missing_and_non_blob_objects_are_errors() {
        let a = oid('a');
        let missing = format!("{a} missing\n");
        assert_eq!(
            parse_cat_file_batch(missing.as_bytes(), std::slice::from_ref(&a)),
            Err(BlobError::Missing(a.clone()))
        );
        assert_eq!(
            parse_cat_file_batch(&response(&a, "tree", b"x"), std::slice::from_ref(&a)),
            Err(BlobError::NotABlob {
                oid: a.clone(),
                kind: ObjectKind::Tree
            })
        );
    }

    #[test]
    fn malformed_batch_output_is_rejected() {
        let (a, b) = (oid('a'), oid('b'));
        let mut truncated = response(&a, "blob", b"hello");
        truncated.truncate(truncated.len() - 3);
        let mut no_lf = response(&a, "blob", b"hello");
        *no_lf.last_mut().unwrap() = b'X';
        let mut extra = response(&a, "blob", b"hi");
        extra.extend_from_slice(b"junk");
        for (bytes, requested) in [
            (truncated, vec![a.clone()]),
            (no_lf, vec![a.clone()]),
            (extra, vec![a.clone()]),
            (response(&b, "blob", b"x"), vec![a.clone()]),
            (format!("{a} blob x\nx\n").into_bytes(), vec![a.clone()]),
            (format!("{a} blob\n").into_bytes(), vec![a.clone()]),
            (format!("{a} blob 1").into_bytes(), vec![a.clone()]),
            (Vec::new(), vec![a.clone()]),
            // 크기 overflow·과대 크기는 panic 없이 해석 오류(리뷰 G1 회귀).
            (
                format!("{a} blob {}\nabc\n", usize::MAX).into_bytes(),
                vec![a.clone()],
            ),
            (
                format!("{a} blob {}\nabc\n", usize::MAX - 1).into_bytes(),
                vec![a.clone()],
            ),
            (
                format!("{a} blob 18446744073709551616\nabc\n").into_bytes(),
                vec![a.clone()],
            ),
            (format!("{a} blob 4\nabc\n").into_bytes(), vec![a.clone()]),
        ] {
            assert!(
                matches!(
                    parse_cat_file_batch(&bytes, &requested),
                    Err(BlobError::Op(GitOpError::Parse(_)))
                ),
                "{:?}",
                String::from_utf8_lossy(&bytes)
            );
        }
    }

    #[test]
    fn parse_failure_keeps_only_a_bounded_prefix_of_raw_output() {
        // SEC-3: 실패 경로가 남은 stdout 전체(큰 blob 내용)를 오류에 복사하지 않아야 합니다.
        let a = oid('a');
        let mut out = response(&a, "blob", b"ok");
        out.extend(std::iter::repeat_n(b'x', 1 << 20));
        for bytes in [out.clone(), vec![b'y'; 1 << 20]] {
            match parse_cat_file_batch(&bytes, std::slice::from_ref(&a)) {
                Err(BlobError::Op(GitOpError::Parse(failure))) => {
                    assert!(failure.raw.len() <= 4096, "{}", failure.raw.len());
                }
                other => panic!("{other:?}"),
            }
        }
    }

    fn limits(blob: u64, batch: u64, total: u64) -> BlobLimits {
        BlobLimits {
            max_blob_bytes: blob,
            max_batch_bytes: batch,
            max_total_bytes: total,
        }
    }

    #[test]
    fn batches_follow_count_and_byte_limits_and_share_repeated_ids() {
        let (a, b, c, d) = (oid('a'), oid('b'), oid('c'), oid('d'));
        let sizes: BTreeMap<ObjectId, u64> = [
            (a.clone(), 100),
            (b.clone(), 100),
            (c.clone(), 500),
            (d.clone(), 0),
        ]
        .into_iter()
        .collect();
        let lim = limits(1_000, 250, u64::MAX);
        // a, b(200) 다음 a 반복은 같은 batch에 공짜로, c(500)는 batch 상한을 넘으므로 다음 batch.
        let order = vec![
            a.clone(),
            b.clone(),
            a.clone(),
            c.clone(),
            d.clone(),
            a.clone(),
        ];
        assert_eq!(
            plan_batch(&order, 0, &sizes, &lim),
            (3, vec![(a.clone(), 100), (b.clone(), 100)])
        );
        // batch 상한보다 큰 blob은 단독 batch(첫 항목은 크기와 무관하게 넣음).
        assert_eq!(
            plan_batch(&order, 3, &sizes, &lim),
            (4, vec![(c.clone(), 500)])
        );
        assert_eq!(
            plan_batch(&order, 4, &sizes, &lim),
            (6, vec![(d.clone(), 0), (a.clone(), 100)])
        );
        assert_eq!(plan_batch(&order, 6, &sizes, &lim), (6, Vec::new()));
        // 개수 상한: 크기 0 blob이라도 BATCH_CHUNK개에서 끊습니다.
        let many: Vec<ObjectId> = (0..BATCH_CHUNK + 3)
            .map(|i| ObjectId::parse(&format!("{i:040x}")).unwrap())
            .collect();
        let zero: BTreeMap<ObjectId, u64> = many.iter().map(|o| (o.clone(), 0)).collect();
        let (end, items) = plan_batch(&many, 0, &zero, &lim);
        assert_eq!((end, items.len()), (BATCH_CHUNK, BATCH_CHUNK));
        assert_eq!(plan_batch(&many, end, &zero, &lim).0, BATCH_CHUNK + 3);
    }

    #[test]
    fn total_limit_counts_repeated_ids_in_order() {
        let (a, b) = (oid('a'), oid('b'));
        let sizes: BTreeMap<ObjectId, u64> =
            [(a.clone(), 60), (b.clone(), 30)].into_iter().collect();
        assert_eq!(check_total([&a, &b], &sizes, &limits(100, 100, 90)), Ok(()));
        assert_eq!(
            check_total([&a, &b, &a], &sizes, &limits(100, 100, 90)),
            Err(BlobError::TotalTooLarge { max_bytes: 90 })
        );
        // 합계 overflow도 상한 초과입니다.
        let huge: BTreeMap<ObjectId, u64> = [(a.clone(), u64::MAX)].into_iter().collect();
        assert_eq!(
            check_total([&a, &a], &huge, &limits(u64::MAX, u64::MAX, u64::MAX)),
            Err(BlobError::TotalTooLarge {
                max_bytes: u64::MAX
            })
        );
    }

    #[test]
    fn batch_check_output_is_parsed_strictly() {
        let (a, b) = (oid('a'), oid('b'));
        let ok = format!("{a} blob 12\n{b} blob 0\n");
        assert_eq!(
            parse_batch_check(ok.as_bytes(), &[a.clone(), b.clone()]),
            Ok(vec![12, 0])
        );
        assert_eq!(
            parse_batch_check(
                format!("{a} missing\n").as_bytes(),
                std::slice::from_ref(&a)
            ),
            Err(BlobError::Missing(a.clone()))
        );
        assert_eq!(
            parse_batch_check(format!("{a} tree 3\n").as_bytes(), std::slice::from_ref(&a)),
            Err(BlobError::NotABlob {
                oid: a.clone(),
                kind: ObjectKind::Tree
            })
        );
        for bad in [
            format!("{b} blob 1\n"),
            format!("{a} blob 1"),
            format!("{a} blob -1\n"),
            format!("{a} blob 1\nextra"),
            format!("{a} blob 18446744073709551616\n"),
            String::new(),
        ] {
            assert!(
                matches!(
                    parse_batch_check(bad.as_bytes(), std::slice::from_ref(&a)),
                    Err(BlobError::Op(GitOpError::Parse(_)))
                ),
                "{bad:?}"
            );
        }
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::runner::GitRunner;
        use crate::test_support::{hermetic_runner, init_repo_with_identity, system_git, TempDir};
        use crate::trust::GitTrust;

        const MIB: usize = 1024 * 1024;

        /// blob 내용(첫 바이트로 서로 다르게 만든 크기 `len` 바이트).
        fn content(tag: u8, len: usize) -> Vec<u8> {
            let mut bytes = vec![tag; len];
            bytes[0] = tag.wrapping_add(1);
            bytes
        }

        fn repo() -> Option<(TempDir, std::path::PathBuf, GitRunner)> {
            let git = system_git()?;
            let dir = TempDir::new("blob-limits");
            let repo = dir.path().join("repo");
            std::fs::create_dir_all(&repo).unwrap();
            let runner = hermetic_runner(git);
            init_repo_with_identity(&runner, &repo);
            Some((dir, repo, runner))
        }

        fn write(runner: &GitRunner, repo: &Path, bytes: &[u8]) -> ObjectId {
            ObjectId::parse(&crate::test_support::write_blob(runner, repo, bytes)).unwrap()
        }

        #[test]
        fn default_read_splits_cat_file_runs_by_content_bytes() {
            // SEC-3: 개수(256)만이 아니라 content 바이트로도 한 번의 `cat-file --batch`를 제한해야
            // 합니다. 24 MiB blob 3개(72 MiB)는 64 MiB batch 상한 때문에 두 번에 나눠 읽습니다.
            let Some((_dir, repo, setup)) = repo() else {
                return; // git 미설치 환경 건너뜀.
            };
            let blobs: Vec<(ObjectId, Vec<u8>)> = (0..3u8)
                .map(|tag| {
                    let bytes = content(tag, 24 * MIB);
                    (write(&setup, &repo, &bytes), bytes)
                })
                .collect();
            let runner = hermetic_runner(system_git().unwrap());
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            let oids: Vec<ObjectId> = blobs.iter().map(|(oid, _)| oid.clone()).collect();
            let read = read_blobs(&gate, &repo, &oids).unwrap();
            for (oid, bytes) in &blobs {
                assert!(read.get(oid) == Some(bytes), "{oid}");
            }
            // 크기 확인(batch-check) 1번 + content batch 2번.
            assert_eq!(runner.spawn_count(), 3);
        }

        #[test]
        fn oversized_blob_or_total_is_rejected_before_reading_content() {
            let Some((_dir, repo, setup)) = repo() else {
                return;
            };
            let small = write(&setup, &repo, &content(1, 10));
            let big = write(&setup, &repo, &content(2, 100));
            let runner = hermetic_runner(system_git().unwrap());
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            assert_eq!(
                read_blobs_with_limits(
                    &gate,
                    &repo,
                    &[small.clone(), big.clone()],
                    &limits(99, 1_000, 1_000)
                ),
                Err(BlobError::BlobTooLarge {
                    oid: big.clone(),
                    size: 100,
                    max_bytes: 99
                })
            );
            assert_eq!(
                read_blobs_with_limits(&gate, &repo, &[small, big], &limits(100, 1_000, 109)),
                Err(BlobError::TotalTooLarge { max_bytes: 109 })
            );
            // 두 번 모두 크기 확인만 실행하고 content는 읽지 않았습니다.
            assert_eq!(runner.spawn_count(), 2);
        }

        #[test]
        fn small_batch_limit_splits_runs_and_keeps_bytes() {
            // 작업량 계수: 100 bytes blob 10개를 batch 250 bytes로 읽으면 batch당 2개 → 5번 + 크기 확인 1번.
            let Some((_dir, repo, setup)) = repo() else {
                return;
            };
            let blobs: Vec<(ObjectId, Vec<u8>)> = (0..10u8)
                .map(|tag| {
                    let bytes = content(tag.wrapping_mul(7), 100);
                    (write(&setup, &repo, &bytes), bytes)
                })
                .collect();
            let runner = hermetic_runner(system_git().unwrap());
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            let mut oids: Vec<ObjectId> = blobs.iter().map(|(oid, _)| oid.clone()).collect();
            oids.push(oids[0].clone()); // 중복 ID는 한 번만 읽습니다.
            let read =
                read_blobs_with_limits(&gate, &repo, &oids, &limits(100, 250, 1_000)).unwrap();
            assert_eq!(read.len(), 10);
            for (oid, bytes) in &blobs {
                assert!(read.get(oid) == Some(bytes), "{oid}");
            }
            assert_eq!(runner.spawn_count(), 6);
            // 빈 요청은 Git을 실행하지 않습니다.
            assert!(read_blobs(&gate, &repo, &[]).unwrap().is_empty());
            assert_eq!(runner.spawn_count(), 6);
        }
    }
}
