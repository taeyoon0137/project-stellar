//! blob content reader — `git cat-file --batch` (XIV.2.6).
//!
//! 요청한 object ID를 한 줄에 하나씩 stdin으로 넘기고(`<oid>\n`), 출력
//! `<oid> SP <type> SP <size> LF <contents> LF`(없으면 `<oid> SP missing LF`)를 요청 순서대로
//! 해석합니다. content는 Git이 저장한 **원시 바이트**이며 filter·textconv를 적용하지 않습니다.
//!
//! - 같은 ID는 한 번만 요청하고, [`BATCH_CHUNK`]개씩 나눠 실행해 출력 크기를 제한합니다.
//! - 응답 ID가 요청과 다르거나, 크기만큼 바이트가 없거나, 끝 LF가 없거나, 남는 바이트가 있으면 해석
//!   오류입니다. 없는 object는 [`BlobError::Missing`], blob이 아닌 object는
//!   [`BlobError::NotABlob`]입니다.

use std::collections::BTreeMap;
use std::path::Path;

use crate::object::{ObjectId, ObjectKind};
use crate::ops::{require_success, GitOpError, ParseFailure};
use crate::runner::argv;
use crate::trust::GatedGitRunner;

const COMMAND: &str = "cat-file --batch";

/// 한 번의 `cat-file --batch` 실행에 넘기는 최대 ID 수.
pub const BATCH_CHUNK: usize = 256;

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
}

impl std::fmt::Display for BlobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlobError::Op(e) => write!(f, "{e}"),
            BlobError::Missing(oid) => write!(f, "Git object 없음: {oid}"),
            BlobError::NotABlob { oid, kind } => {
                write!(f, "blob이 아닌 object: {oid} ({})", kind.as_str())
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

fn parse_error(detail: &str, raw: &[u8]) -> BlobError {
    BlobError::Op(GitOpError::Parse(ParseFailure::new(COMMAND, detail, raw)))
}

/// `cat-file --batch` 출력을 요청 순서대로 해석합니다.
pub fn parse_cat_file_batch(
    stdout: &[u8],
    requested: &[ObjectId],
) -> Result<Vec<(ObjectId, Vec<u8>)>, BlobError> {
    let mut rest = stdout;
    let mut out = Vec::with_capacity(requested.len());
    for oid in requested {
        let lf = rest
            .iter()
            .position(|b| *b == b'\n')
            .ok_or_else(|| parse_error("header LF 없음", rest))?;
        let header = &rest[..lf];
        rest = &rest[lf + 1..];
        let mut fields = header.split(|b| *b == b' ');
        let got = fields.next().unwrap_or_default();
        if got != oid.as_str().as_bytes() {
            return Err(parse_error("응답 ID가 요청과 다름", header));
        }
        let (kind, size) = match (fields.next(), fields.next(), fields.next()) {
            (Some(b"missing"), None, None) => return Err(BlobError::Missing(oid.clone())),
            (Some(kind), Some(size), None) => (kind, size),
            _ => return Err(parse_error("잘못된 header", header)),
        };
        let kind = ObjectKind::from_name(kind).ok_or_else(|| parse_error("잘못된 종류", header))?;
        let size: usize = std::str::from_utf8(size)
            .ok()
            .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| parse_error("잘못된 크기", header))?;
        // 크기는 신뢰할 수 없는 출력이므로 `size + 1`을 overflow 없이 계산하고, 남은 바이트보다
        // 크면 해석 오류로 돌려줍니다(slice 범위 panic 방지).
        let end = match size.checked_add(1) {
            Some(end) if end <= rest.len() => end,
            _ => return Err(parse_error("content가 잘림", header)),
        };
        if rest[size] != b'\n' {
            return Err(parse_error("content 뒤 LF 없음", header));
        }
        let content = rest[..size].to_vec();
        rest = &rest[end..];
        if kind != ObjectKind::Blob {
            return Err(BlobError::NotABlob {
                oid: oid.clone(),
                kind,
            });
        }
        out.push((oid.clone(), content));
    }
    if !rest.is_empty() {
        return Err(parse_error("요청보다 많은 출력", rest));
    }
    Ok(out)
}

/// blob content를 읽습니다(중복 ID는 한 번만 요청, [`BATCH_CHUNK`]개씩 실행).
pub fn read_blobs(
    git: &GatedGitRunner,
    worktree: &Path,
    oids: &[ObjectId],
) -> Result<BTreeMap<ObjectId, Vec<u8>>, BlobError> {
    let mut unique: Vec<ObjectId> = oids.to_vec();
    unique.sort();
    unique.dedup();
    let mut blobs = BTreeMap::new();
    for chunk in unique.chunks(BATCH_CHUNK) {
        let mut input = Vec::with_capacity(chunk.len() * 65);
        for oid in chunk {
            input.extend_from_slice(oid.as_str().as_bytes());
            input.push(b'\n');
        }
        let output = git
            .run_with_input(&argv(["cat-file", "--batch"]), worktree, &input)
            .map_err(GitOpError::Gate)?;
        let output = require_success(output)?;
        for (oid, content) in parse_cat_file_batch(&output.stdout, chunk)? {
            blobs.insert(oid, content);
        }
    }
    Ok(blobs)
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
}
