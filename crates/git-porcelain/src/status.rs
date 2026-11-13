//! `git status --porcelain=v2 -z --branch` 해석 (XIV.2.1).
//!
//! machine parse는 porcelain v2 + `-z`(NUL 종료 레코드)만 사용합니다. 경로는 quote 없는 원시
//! 바이트([`GitPath`])로 보존합니다. 레코드 종류:
//!
//! | 첫 글자 | 종류 | 형식 |
//! |---|---|---|
//! | `#` | header | `# branch.oid`, `# branch.head`, `# branch.upstream`, `# branch.ab` |
//! | `1` | ordinary | `1 XY sub mH mI mW hH hI path` |
//! | `2` | rename/copy | `2 XY sub mH mI mW hH hI Xscore path` + 다음 레코드 `origPath` |
//! | `u` | unmerged | `u XY sub m1 m2 m3 mW h1 h2 h3 path` |
//! | `?` | untracked | `? path` |
//! | `!` | ignored | `! path` |
//!
//! Git 문서대로 모르는 header(`# stash` 등)는 무시하되 원시 바이트를 보존합니다. 모르는 레코드
//! 종류나 형식 오류는 추측하지 않고 [`ParseFailure`]로 실패합니다(부분 결과를 성공으로 반환하지
//! 않음). 실행은 trust gate를 거치고, 다른 Git 명령과 index lock이 경합하지 않도록
//! `--no-optional-locks`를 붙입니다.

use std::path::Path;

use crate::decode::{split_nul_terminated, GitPath};
use crate::object::{FileMode, ObjectId};
use crate::ops::{run_ok, GitOpError, ParseFailure};
use crate::runner::argv;
use crate::trust::GatedGitRunner;

const COMMAND: &str = "status --porcelain=v2";

/// index 또는 worktree 쪽 변경 코드(`XY`의 한 글자).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatusCode {
    /// `.` 변경 없음.
    Unmodified,
    /// `M` 수정.
    Modified,
    /// `T` 종류 변경(파일↔symlink 등).
    TypeChanged,
    /// `A` 추가.
    Added,
    /// `D` 삭제.
    Deleted,
    /// `R` 이름 변경.
    Renamed,
    /// `C` 복사.
    Copied,
    /// `U` 병합되지 않음(unmerged).
    Unmerged,
}

impl StatusCode {
    /// 상태 글자에서 만듭니다.
    pub fn from_byte(b: u8) -> Option<StatusCode> {
        Some(match b {
            b'.' => StatusCode::Unmodified,
            b'M' => StatusCode::Modified,
            b'T' => StatusCode::TypeChanged,
            b'A' => StatusCode::Added,
            b'D' => StatusCode::Deleted,
            b'R' => StatusCode::Renamed,
            b'C' => StatusCode::Copied,
            b'U' => StatusCode::Unmerged,
            _ => return None,
        })
    }

    /// Git 상태 글자.
    pub fn as_char(self) -> char {
        match self {
            StatusCode::Unmodified => '.',
            StatusCode::Modified => 'M',
            StatusCode::TypeChanged => 'T',
            StatusCode::Added => 'A',
            StatusCode::Deleted => 'D',
            StatusCode::Renamed => 'R',
            StatusCode::Copied => 'C',
            StatusCode::Unmerged => 'U',
        }
    }

    /// 변경이 있는지(`.`가 아님).
    pub fn is_changed(self) -> bool {
        self != StatusCode::Unmodified
    }
}

/// submodule 상태(`sub` 필드).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubmoduleState {
    /// submodule이 아님(`N...`).
    NotSubmodule,
    /// submodule(`S<c><m><u>`).
    Submodule {
        /// 기록된 commit과 다른 commit을 가리킴(`C`).
        commit_changed: bool,
        /// tracked 변경이 있음(`M`).
        has_tracked_changes: bool,
        /// untracked 파일이 있음(`U`).
        has_untracked_changes: bool,
    },
}

impl SubmoduleState {
    fn parse(raw: &[u8]) -> Option<SubmoduleState> {
        match raw {
            b"N..." => Some(SubmoduleState::NotSubmodule),
            [b'S', c, m, u] => {
                let flag = |b: u8, set: u8| match b {
                    b'.' => Some(false),
                    x if x == set => Some(true),
                    _ => None,
                };
                Some(SubmoduleState::Submodule {
                    commit_changed: flag(*c, b'C')?,
                    has_tracked_changes: flag(*m, b'M')?,
                    has_untracked_changes: flag(*u, b'U')?,
                })
            }
            _ => None,
        }
    }
}

/// ordinary 또는 rename/copy 항목의 공통 필드.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedEntry {
    /// index 쪽(HEAD→index) 변경.
    pub index: StatusCode,
    /// worktree 쪽(index→worktree) 변경.
    pub worktree: StatusCode,
    /// submodule 상태.
    pub submodule: SubmoduleState,
    /// HEAD의 mode.
    pub mode_head: FileMode,
    /// index의 mode.
    pub mode_index: FileMode,
    /// worktree의 mode.
    pub mode_worktree: FileMode,
    /// HEAD의 object ID.
    pub oid_head: ObjectId,
    /// index의 object ID.
    pub oid_index: ObjectId,
    /// 경로(원시 바이트).
    pub path: GitPath,
}

/// rename과 copy 구분.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameKind {
    /// `R`.
    Renamed,
    /// `C`.
    Copied,
}

/// rename/copy 항목.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamedEntry {
    /// 공통 필드(`path`는 새 경로).
    pub entry: ChangedEntry,
    /// rename 또는 copy.
    pub kind: RenameKind,
    /// 유사도 점수(0~100).
    pub score: u8,
    /// 원래 경로.
    pub original_path: GitPath,
}

/// unmerged 항목의 충돌 종류(`XY`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnmergedKind {
    /// `DD` 양쪽 삭제.
    BothDeleted,
    /// `AU` 우리 쪽 추가.
    AddedByUs,
    /// `UD` 상대 쪽 삭제.
    DeletedByThem,
    /// `UA` 상대 쪽 추가.
    AddedByThem,
    /// `DU` 우리 쪽 삭제.
    DeletedByUs,
    /// `AA` 양쪽 추가.
    BothAdded,
    /// `UU` 양쪽 수정.
    BothModified,
}

impl UnmergedKind {
    fn parse(xy: &[u8]) -> Option<UnmergedKind> {
        Some(match xy {
            b"DD" => UnmergedKind::BothDeleted,
            b"AU" => UnmergedKind::AddedByUs,
            b"UD" => UnmergedKind::DeletedByThem,
            b"UA" => UnmergedKind::AddedByThem,
            b"DU" => UnmergedKind::DeletedByUs,
            b"AA" => UnmergedKind::BothAdded,
            b"UU" => UnmergedKind::BothModified,
            _ => return None,
        })
    }

    /// Git `XY` 문자열.
    pub fn as_str(self) -> &'static str {
        match self {
            UnmergedKind::BothDeleted => "DD",
            UnmergedKind::AddedByUs => "AU",
            UnmergedKind::DeletedByThem => "UD",
            UnmergedKind::AddedByThem => "UA",
            UnmergedKind::DeletedByUs => "DU",
            UnmergedKind::BothAdded => "AA",
            UnmergedKind::BothModified => "UU",
        }
    }
}

/// unmerged 항목.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmergedEntry {
    /// 충돌 종류.
    pub kind: UnmergedKind,
    /// submodule 상태.
    pub submodule: SubmoduleState,
    /// stage 1(base)·2(ours)·3(theirs)의 mode.
    pub stage_modes: [FileMode; 3],
    /// worktree의 mode.
    pub mode_worktree: FileMode,
    /// stage 1·2·3의 object ID.
    pub stage_oids: [ObjectId; 3],
    /// 경로.
    pub path: GitPath,
}

/// status 항목.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusEntry {
    /// 일반 변경(`1`).
    Ordinary(ChangedEntry),
    /// rename/copy(`2`).
    Renamed(RenamedEntry),
    /// 병합되지 않은 항목(`u`).
    Unmerged(UnmergedEntry),
    /// untracked(`?`).
    Untracked(GitPath),
    /// ignored(`!`).
    Ignored(GitPath),
}

impl StatusEntry {
    /// 항목 경로(rename/copy는 새 경로).
    pub fn path(&self) -> &GitPath {
        match self {
            StatusEntry::Ordinary(e) => &e.path,
            StatusEntry::Renamed(e) => &e.entry.path,
            StatusEntry::Unmerged(e) => &e.path,
            StatusEntry::Untracked(p) | StatusEntry::Ignored(p) => p,
        }
    }
}

/// `# branch.oid`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchOid {
    /// 아직 commit이 없음(`(initial)`, unborn branch).
    Initial,
    /// 현재 commit.
    Commit(ObjectId),
}

/// `# branch.head`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchHead {
    /// detached HEAD(`(detached)`).
    Detached,
    /// branch 이름(원시 바이트).
    Branch(Vec<u8>),
}

/// `# branch.ab +<ahead> -<behind>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AheadBehind {
    /// upstream보다 앞선 commit 수.
    pub ahead: u64,
    /// upstream보다 뒤처진 commit 수.
    pub behind: u64,
}

/// `--branch` header 묶음.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchHeaders {
    /// 현재 commit.
    pub oid: BranchOid,
    /// 현재 branch.
    pub head: BranchHead,
    /// upstream 이름(설정된 경우).
    pub upstream: Option<Vec<u8>>,
    /// ahead/behind(upstream이 없거나 사라졌으면 Git이 생략).
    pub ahead_behind: Option<AheadBehind>,
}

/// 해석한 status 전체.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusReport {
    /// branch header(`--branch`로 실행했을 때).
    pub branch: Option<BranchHeaders>,
    /// 항목(Git 출력 순서 유지).
    pub entries: Vec<StatusEntry>,
    /// 해석하지 않은 header 원시 바이트(`# ` 이후).
    pub unknown_headers: Vec<Vec<u8>>,
}

/// `status` 실행 옵션(기본: ignored 제외).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatusOptions {
    /// ignored 항목 포함(`--ignored=matching`).
    pub include_ignored: bool,
}

/// status를 실행하는 argv(옵션 반영). untracked는 디렉터리로 접지 않고 파일 단위로 봅니다.
pub fn status_args(options: StatusOptions) -> Vec<std::ffi::OsString> {
    let mut args = argv([
        "--no-optional-locks",
        "status",
        "--porcelain=v2",
        "-z",
        "--branch",
        "--untracked-files=all",
    ]);
    if options.include_ignored {
        args.extend(argv(["--ignored=matching"]));
    }
    args
}

/// worktree에서 status를 실행해 해석합니다(trust gate 적용).
pub fn read_status(
    git: &GatedGitRunner,
    worktree: &Path,
    options: StatusOptions,
) -> Result<StatusReport, GitOpError> {
    let output = run_ok(git, worktree, &status_args(options))?;
    Ok(parse_status_v2(&output.stdout)?)
}

/// 공백으로 나눈 앞쪽 필드와 나머지(경로)를 돌려줍니다.
fn split_fields(record: &[u8], fixed: usize) -> Option<(Vec<&[u8]>, &[u8])> {
    let mut fields = Vec::with_capacity(fixed);
    let mut rest = record;
    for _ in 0..fixed {
        let i = rest.iter().position(|b| *b == b' ')?;
        fields.push(&rest[..i]);
        rest = &rest[i + 1..];
    }
    Some((fields, rest))
}

fn mode(raw: &[u8], record: &[u8], name: &str) -> Result<FileMode, ParseFailure> {
    FileMode::from_octal_bytes(raw)
        .ok_or_else(|| ParseFailure::new(COMMAND, format!("잘못된 mode 필드 {name}"), record))
}

fn oid(raw: &[u8], record: &[u8], name: &str) -> Result<ObjectId, ParseFailure> {
    ObjectId::from_hex_bytes(raw)
        .ok_or_else(|| ParseFailure::new(COMMAND, format!("잘못된 object ID 필드 {name}"), record))
}

fn path_field(raw: &[u8], record: &[u8]) -> Result<GitPath, ParseFailure> {
    if raw.is_empty() {
        return Err(ParseFailure::new(COMMAND, "빈 경로", record));
    }
    Ok(GitPath::from_bytes(raw))
}

fn xy(raw: &[u8], record: &[u8]) -> Result<(StatusCode, StatusCode), ParseFailure> {
    match raw {
        [x, y] => match (StatusCode::from_byte(*x), StatusCode::from_byte(*y)) {
            (Some(x), Some(y)) => Ok((x, y)),
            _ => Err(ParseFailure::new(COMMAND, "잘못된 XY 필드", record)),
        },
        _ => Err(ParseFailure::new(COMMAND, "잘못된 XY 필드", record)),
    }
}

fn submodule(raw: &[u8], record: &[u8]) -> Result<SubmoduleState, ParseFailure> {
    SubmoduleState::parse(raw).ok_or_else(|| ParseFailure::new(COMMAND, "잘못된 sub 필드", record))
}

fn changed_entry(
    fields: &[&[u8]],
    path: &[u8],
    record: &[u8],
) -> Result<ChangedEntry, ParseFailure> {
    let (index, worktree) = xy(fields[1], record)?;
    Ok(ChangedEntry {
        index,
        worktree,
        submodule: submodule(fields[2], record)?,
        mode_head: mode(fields[3], record, "mH")?,
        mode_index: mode(fields[4], record, "mI")?,
        mode_worktree: mode(fields[5], record, "mW")?,
        oid_head: oid(fields[6], record, "hH")?,
        oid_index: oid(fields[7], record, "hI")?,
        path: path_field(path, record)?,
    })
}

fn parse_score(raw: &[u8], record: &[u8]) -> Result<(RenameKind, u8), ParseFailure> {
    let bad = || ParseFailure::new(COMMAND, "잘못된 rename score 필드", record);
    let (kind, digits) = match raw.split_first() {
        Some((b'R', rest)) => (RenameKind::Renamed, rest),
        Some((b'C', rest)) => (RenameKind::Copied, rest),
        _ => return Err(bad()),
    };
    if digits.is_empty() || digits.len() > 3 || !digits.iter().all(u8::is_ascii_digit) {
        return Err(bad());
    }
    let score = digits
        .iter()
        .fold(0u32, |acc, d| acc * 10 + u32::from(d - b'0'));
    u8::try_from(score)
        .ok()
        .filter(|s| *s <= 100)
        .map(|s| (kind, s))
        .ok_or_else(bad)
}

fn parse_branch_header(
    body: &[u8],
    record: &[u8],
    oid_h: &mut Option<BranchOid>,
    head_h: &mut Option<BranchHead>,
    upstream: &mut Option<Vec<u8>>,
    ab: &mut Option<AheadBehind>,
) -> Result<bool, ParseFailure> {
    let dup = || ParseFailure::new(COMMAND, "중복된 branch header", record);
    if let Some(value) = body.strip_prefix(b"branch.oid ") {
        let parsed = if value == b"(initial)" {
            BranchOid::Initial
        } else {
            BranchOid::Commit(oid(value, record, "branch.oid")?)
        };
        if oid_h.replace(parsed).is_some() {
            return Err(dup());
        }
    } else if let Some(value) = body.strip_prefix(b"branch.head ") {
        if value.is_empty() {
            return Err(ParseFailure::new(COMMAND, "빈 branch.head", record));
        }
        let parsed = if value == b"(detached)" {
            BranchHead::Detached
        } else {
            BranchHead::Branch(value.to_vec())
        };
        if head_h.replace(parsed).is_some() {
            return Err(dup());
        }
    } else if let Some(value) = body.strip_prefix(b"branch.upstream ") {
        if value.is_empty() || upstream.replace(value.to_vec()).is_some() {
            return Err(dup());
        }
    } else if let Some(value) = body.strip_prefix(b"branch.ab ") {
        let bad = || ParseFailure::new(COMMAND, "잘못된 branch.ab", record);
        let text = std::str::from_utf8(value).map_err(|_| bad())?;
        let (a, b) = text.split_once(' ').ok_or_else(bad)?;
        let ahead = a
            .strip_prefix('+')
            .and_then(|n| n.parse().ok())
            .ok_or_else(bad)?;
        let behind = b
            .strip_prefix('-')
            .and_then(|n| n.parse().ok())
            .ok_or_else(bad)?;
        if ab.replace(AheadBehind { ahead, behind }).is_some() {
            return Err(dup());
        }
    } else {
        return Ok(false);
    }
    Ok(true)
}

/// `git status --porcelain=v2 -z [--branch]` 출력 전체를 해석합니다.
pub fn parse_status_v2(stdout: &[u8]) -> Result<StatusReport, ParseFailure> {
    let records = split_nul_terminated(stdout)
        .map_err(|e| ParseFailure::new(COMMAND, e.to_string(), stdout))?;
    let mut entries = Vec::new();
    let mut unknown_headers = Vec::new();
    let (mut b_oid, mut b_head, mut b_up, mut b_ab) = (None, None, None, None);
    let mut seen_branch_header = false;

    let mut iter = records.into_iter();
    while let Some(record) = iter.next() {
        match record.first() {
            Some(b'#') => {
                let body = record
                    .strip_prefix(b"# ")
                    .ok_or_else(|| ParseFailure::new(COMMAND, "잘못된 header", record))?;
                if parse_branch_header(body, record, &mut b_oid, &mut b_head, &mut b_up, &mut b_ab)?
                {
                    seen_branch_header = true;
                } else {
                    unknown_headers.push(body.to_vec());
                }
            }
            Some(b'1') => {
                let (fields, path) = split_fields(record, 8)
                    .ok_or_else(|| ParseFailure::new(COMMAND, "ordinary 필드 부족", record))?;
                if fields[0] != b"1" {
                    return Err(ParseFailure::new(COMMAND, "알 수 없는 레코드", record));
                }
                entries.push(StatusEntry::Ordinary(changed_entry(&fields, path, record)?));
            }
            Some(b'2') => {
                let (fields, path) = split_fields(record, 9)
                    .ok_or_else(|| ParseFailure::new(COMMAND, "rename 필드 부족", record))?;
                if fields[0] != b"2" {
                    return Err(ParseFailure::new(COMMAND, "알 수 없는 레코드", record));
                }
                let (kind, score) = parse_score(fields[8], record)?;
                let original = iter.next().ok_or_else(|| {
                    ParseFailure::new(COMMAND, "rename 원래 경로 레코드 없음", record)
                })?;
                entries.push(StatusEntry::Renamed(RenamedEntry {
                    entry: changed_entry(&fields, path, record)?,
                    kind,
                    score,
                    original_path: path_field(original, record)?,
                }));
            }
            Some(b'u') => {
                let (fields, path) = split_fields(record, 10)
                    .ok_or_else(|| ParseFailure::new(COMMAND, "unmerged 필드 부족", record))?;
                if fields[0] != b"u" {
                    return Err(ParseFailure::new(COMMAND, "알 수 없는 레코드", record));
                }
                let kind = UnmergedKind::parse(fields[1])
                    .ok_or_else(|| ParseFailure::new(COMMAND, "잘못된 unmerged XY", record))?;
                entries.push(StatusEntry::Unmerged(UnmergedEntry {
                    kind,
                    submodule: submodule(fields[2], record)?,
                    stage_modes: [
                        mode(fields[3], record, "m1")?,
                        mode(fields[4], record, "m2")?,
                        mode(fields[5], record, "m3")?,
                    ],
                    mode_worktree: mode(fields[6], record, "mW")?,
                    stage_oids: [
                        oid(fields[7], record, "h1")?,
                        oid(fields[8], record, "h2")?,
                        oid(fields[9], record, "h3")?,
                    ],
                    path: path_field(path, record)?,
                }));
            }
            Some(b'?') | Some(b'!') => {
                let path = record
                    .get(2..)
                    .filter(|_| record.get(1) == Some(&b' '))
                    .ok_or_else(|| ParseFailure::new(COMMAND, "잘못된 경로 레코드", record))?;
                let path = path_field(path, record)?;
                entries.push(if record[0] == b'?' {
                    StatusEntry::Untracked(path)
                } else {
                    StatusEntry::Ignored(path)
                });
            }
            _ => return Err(ParseFailure::new(COMMAND, "알 수 없는 레코드 종류", record)),
        }
    }

    let branch = if seen_branch_header {
        match (b_oid, b_head) {
            (Some(oid), Some(head)) => Some(BranchHeaders {
                oid,
                head,
                upstream: b_up,
                ahead_behind: b_ab,
            }),
            _ => {
                return Err(ParseFailure::new(
                    COMMAND,
                    "branch.oid 또는 branch.head header 누락",
                    stdout,
                ))
            }
        }
    } else {
        None
    };
    Ok(StatusReport {
        branch,
        entries,
        unknown_headers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const Z40: &str = "0000000000000000000000000000000000000000";
    const E1: &str = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";
    const E2: &str = "8baef1b4abc478178b004d62031cf7fe6db6f903";

    fn oid(s: &str) -> ObjectId {
        ObjectId::parse(s).unwrap()
    }

    /// 합성 fixture corpus: (설명, 출력 바이트).
    fn corpus() -> Vec<(&'static str, Vec<u8>)> {
        let mut c: Vec<(&'static str, Vec<u8>)> = Vec::new();
        c.push(("empty", Vec::new()));
        c.push((
            "branch only",
            format!("# branch.oid {E1}\0# branch.head main\0").into_bytes(),
        ));
        c.push((
            "unborn",
            b"# branch.oid (initial)\0# branch.head main\0".to_vec(),
        ));
        c.push((
            "detached with stash header",
            format!("# branch.oid {E1}\0# branch.head (detached)\0# stash 3\0").into_bytes(),
        ));
        c.push((
            "upstream ab",
            format!(
                "# branch.oid {E1}\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -5\0"
            )
            .into_bytes(),
        ));
        c.push((
            "ordinary with spaces",
            format!("1 .M N... 100644 100644 100644 {E1} {E1} dir/a b.txt\0").into_bytes(),
        ));
        let mut rename =
            format!("2 R. N... 100644 100644 100644 {E1} {E1} R100 new name.txt\0").into_bytes();
        rename.extend_from_slice(b"old name.txt\0");
        c.push(("rename", rename));
        let mut copy =
            format!("2 C. N... 100755 100755 100755 {E2} {E2} C075 copy.sh\0").into_bytes();
        copy.extend_from_slice(b"orig.sh\0");
        c.push(("copy", copy));
        c.push((
            "unmerged",
            format!("u UU N... 100644 100644 100644 100644 {E1} {E2} {E1} conflict.txt\0")
                .into_bytes(),
        ));
        c.push((
            "submodule",
            format!("1 .M SCMU 160000 160000 160000 {E1} {E1} sub\0").into_bytes(),
        ));
        let mut nonutf8 = b"? bad\xff\xfe name\0! ignored.log\0".to_vec();
        nonutf8.extend_from_slice(b"? line\nbreak.txt\0");
        c.push(("untracked/ignored non-utf8", nonutf8));
        let sha256 = "b".repeat(64);
        c.push((
            "sha256 ids",
            format!(
                "1 A. N... 000000 100644 100644 {} {sha256} new.txt\0",
                "0".repeat(64)
            )
            .into_bytes(),
        ));
        c
    }

    #[test]
    fn whole_fixture_corpus_parses() {
        for (name, bytes) in corpus() {
            parse_status_v2(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    #[test]
    fn branch_headers_are_typed() {
        let r = parse_status_v2(b"# branch.oid (initial)\0# branch.head main\0").unwrap();
        assert_eq!(
            r.branch,
            Some(BranchHeaders {
                oid: BranchOid::Initial,
                head: BranchHead::Branch(b"main".to_vec()),
                upstream: None,
                ahead_behind: None,
            })
        );
        let bytes = format!(
            "# branch.oid {E1}\0# branch.head (detached)\0# branch.upstream origin/x\0# branch.ab +2 -5\0# stash 3\0"
        );
        let r = parse_status_v2(bytes.as_bytes()).unwrap();
        let b = r.branch.unwrap();
        assert_eq!(b.oid, BranchOid::Commit(oid(E1)));
        assert_eq!(b.head, BranchHead::Detached);
        assert_eq!(b.upstream, Some(b"origin/x".to_vec()));
        assert_eq!(
            b.ahead_behind,
            Some(AheadBehind {
                ahead: 2,
                behind: 5
            })
        );
        assert_eq!(r.unknown_headers, vec![b"stash 3".to_vec()]);
        // --branch 없이 실행한 출력은 branch None.
        assert_eq!(parse_status_v2(b"? a\0").unwrap().branch, None);
    }

    #[test]
    fn entry_kinds_are_distinguished() {
        let mut bytes = format!(
            "1 MM N... 100644 100644 100755 {E1} {E2} a b.txt\0\
             2 R. N... 100644 100644 100644 {E1} {E1} R087 new.txt\0old.txt\0\
             u AA N... 000000 100644 100644 100644 {Z40} {E1} {E2} both.txt\0\
             ? untracked.txt\0! ignored.log\0"
        )
        .into_bytes();
        bytes.extend_from_slice(b"? raw\xff.bin\0");
        let r = parse_status_v2(&bytes).unwrap();
        assert_eq!(r.entries.len(), 6);
        match &r.entries[0] {
            StatusEntry::Ordinary(e) => {
                assert_eq!(
                    (e.index, e.worktree),
                    (StatusCode::Modified, StatusCode::Modified)
                );
                assert_eq!(e.mode_worktree, FileMode::EXECUTABLE);
                assert_eq!(e.oid_index, oid(E2));
                assert_eq!(e.path.as_bytes(), b"a b.txt");
                assert_eq!(e.submodule, SubmoduleState::NotSubmodule);
            }
            other => panic!("{other:?}"),
        }
        match &r.entries[1] {
            StatusEntry::Renamed(e) => {
                assert_eq!(e.kind, RenameKind::Renamed);
                assert_eq!(e.score, 87);
                assert_eq!(e.entry.path.as_bytes(), b"new.txt");
                assert_eq!(e.original_path.as_bytes(), b"old.txt");
                assert_eq!(e.entry.index, StatusCode::Renamed);
            }
            other => panic!("{other:?}"),
        }
        match &r.entries[2] {
            StatusEntry::Unmerged(e) => {
                assert_eq!(e.kind, UnmergedKind::BothAdded);
                assert_eq!(e.stage_modes[0], FileMode::ABSENT);
                assert!(e.stage_oids[0].is_null());
                assert_eq!(e.stage_oids[2], oid(E2));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            r.entries[3],
            StatusEntry::Untracked(GitPath::from_bytes(b"untracked.txt"))
        );
        assert_eq!(
            r.entries[4],
            StatusEntry::Ignored(GitPath::from_bytes(b"ignored.log"))
        );
        assert_eq!(r.entries[5].path().as_bytes(), b"raw\xff.bin");
    }

    #[test]
    fn submodule_flags_parse() {
        let bytes = format!("1 .M S.M. 160000 160000 160000 {E1} {E1} sub\0");
        match &parse_status_v2(bytes.as_bytes()).unwrap().entries[0] {
            StatusEntry::Ordinary(e) => assert_eq!(
                e.submodule,
                SubmoduleState::Submodule {
                    commit_changed: false,
                    has_tracked_changes: true,
                    has_untracked_changes: false,
                }
            ),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn malformed_records_fail_without_partial_success() {
        let bad: Vec<Vec<u8>> = vec![
            b"? a".to_vec(),                                                      // NUL 종료 없음
            b"x weird\0".to_vec(),                                                // 모르는 종류
            b"?a\0".to_vec(),                                                     // 구분 공백 없음
            b"? \0".to_vec(),                                                     // 빈 경로
            format!("1 XM N... 100644 100644 100644 {E1} {E1} a\0").into_bytes(), // XY
            format!("1 .M N... 100645 100644 100644 {E1} {E1} a\0").into_bytes(), // mode
            format!("1 .M N... 100644 100644 100644 {E1} zz a\0").into_bytes(),   // oid
            format!("1 .M Nxx. 100644 100644 100644 {E1} {E1} a\0").into_bytes(), // sub
            format!("1 .M N... 100644 100644 100644 {E1} {E1}\0").into_bytes(),   // 필드 부족
            format!("2 R. N... 100644 100644 100644 {E1} {E1} R100 new\0").into_bytes(), // 원래 경로 없음
            format!("2 R. N... 100644 100644 100644 {E1} {E1} X100 new\0old\0").into_bytes(),
            format!("2 R. N... 100644 100644 100644 {E1} {E1} R101 new\0old\0").into_bytes(),
            format!("u XX N... 100644 100644 100644 100644 {E1} {E1} {E1} c\0").into_bytes(),
            b"# branch.head main\0".to_vec(), // oid 누락
            b"# branch.oid (initial)\0# branch.oid (initial)\0# branch.head m\0".to_vec(),
            format!("# branch.oid {E1}\0# branch.head m\0# branch.ab 2 5\0").into_bytes(),
            b"#branch.oid x\0".to_vec(),
        ];
        for bytes in bad {
            assert!(
                parse_status_v2(&bytes).is_err(),
                "{:?}",
                String::from_utf8_lossy(&bytes)
            );
        }
    }

    #[test]
    fn status_args_are_machine_readable() {
        let args: Vec<String> = status_args(StatusOptions::default())
            .into_iter()
            .map(|a| a.into_string().unwrap())
            .collect();
        assert_eq!(
            args,
            [
                "--no-optional-locks",
                "status",
                "--porcelain=v2",
                "-z",
                "--branch",
                "--untracked-files=all"
            ]
        );
        let args = status_args(StatusOptions {
            include_ignored: true,
        });
        assert_eq!(args.last().unwrap(), "--ignored=matching");
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::runner::GitRunner;
        use crate::test_support::{
            add_index_path, commit_file, git_ok, hermetic_runner, init_repo_with_identity,
            system_git, TempDir,
        };
        use crate::trust::{GatedGitRunner, GatedRunError, GitTrust};

        fn find<'a>(r: &'a StatusReport, path: &[u8]) -> &'a StatusEntry {
            r.entries
                .iter()
                .find(|e| e.path().as_bytes() == path)
                .unwrap_or_else(|| panic!("항목 없음: {:?}", String::from_utf8_lossy(path)))
        }

        fn ordinary(e: &StatusEntry) -> &ChangedEntry {
            match e {
                StatusEntry::Ordinary(c) => c,
                other => panic!("ordinary 아님: {other:?}"),
            }
        }

        /// 각종 상태가 섞인 저장소(ordinary·rename·copy·untracked·ignored·non-UTF8·type change).
        fn mixed_repo(runner: &GitRunner, repo: &Path) {
            init_repo_with_identity(runner, repo);
            for (name, body) in [
                ("a.txt", "a\n"),
                ("b.txt", "b\n"),
                ("c.txt", "c\n"),
                ("del.txt", "d\n"),
                ("exec.sh", "#!/bin/sh\n"),
                ("type.txt", "t\n"),
                ("공백 파일.txt", "s\n"),
                (
                    "rename-src.txt",
                    "rename body line 1\nline 2\nline 3\nline 4\n",
                ),
            ] {
                std::fs::write(repo.join(name), body).unwrap();
            }
            std::fs::write(repo.join(".gitignore"), "*.log\n").unwrap();
            git_ok(runner, repo, ["add", "-A"]);
            // APFS는 non-UTF8 파일 이름을 만들 수 없으므로 index에만 등록(작업 트리에는 없음 → .D).
            add_index_path(runner, repo, "100644", b"raw\xff.bin", b"\x00\x01");
            git_ok(runner, repo, ["commit", "-q", "-m", "base"]);

            std::fs::write(repo.join("a.txt"), "a2\n").unwrap(); // .M
            std::fs::write(repo.join("b.txt"), "b2\n").unwrap();
            git_ok(runner, repo, ["add", "b.txt"]); // M.
            std::fs::write(repo.join("c.txt"), "c2\n").unwrap();
            git_ok(runner, repo, ["add", "c.txt"]);
            std::fs::write(repo.join("c.txt"), "c3\n").unwrap(); // MM
            std::fs::remove_file(repo.join("del.txt")).unwrap(); // .D
            git_ok(runner, repo, ["mv", "rename-src.txt", "renamed.txt"]); // R.
            std::fs::write(repo.join("new.txt"), "n\n").unwrap();
            git_ok(runner, repo, ["add", "new.txt"]); // A.
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(repo.join("exec.sh"), std::fs::Permissions::from_mode(0o755))
                .unwrap(); // .M (mode)
            std::fs::remove_file(repo.join("type.txt")).unwrap();
            std::os::unix::fs::symlink("a.txt", repo.join("type.txt")).unwrap(); // .T
            std::fs::write(repo.join("공백 파일.txt"), "s2\n").unwrap();
            std::fs::write(repo.join("untracked.txt"), "u\n").unwrap();
            std::fs::write(repo.join("debug.log"), "l\n").unwrap();
        }

        #[test]
        fn real_mixed_repository_status_parses_every_kind() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("status-mixed");
            let runner = hermetic_runner(git);
            mixed_repo(&runner, dir.path());
            let gate = GatedGitRunner::new(runner, GitTrust::Trusted);
            let r = read_status(
                &gate,
                dir.path(),
                StatusOptions {
                    include_ignored: true,
                },
            )
            .expect("status");

            let b = r.branch.as_ref().unwrap();
            assert_eq!(b.head, BranchHead::Branch(b"main".to_vec()));
            assert!(matches!(b.oid, BranchOid::Commit(_)));
            assert_eq!(b.upstream, None);

            let codes = |p: &[u8]| {
                let e = ordinary(find(&r, p));
                (e.index.as_char(), e.worktree.as_char())
            };
            assert_eq!(codes(b"a.txt"), ('.', 'M'));
            assert_eq!(codes(b"b.txt"), ('M', '.'));
            assert_eq!(codes(b"c.txt"), ('M', 'M'));
            assert_eq!(codes(b"del.txt"), ('.', 'D'));
            assert_eq!(codes(b"new.txt"), ('A', '.'));
            assert_eq!(codes(b"type.txt"), ('.', 'T'));
            assert_eq!(codes("공백 파일.txt".as_bytes()), ('.', 'M'));
            let exec = ordinary(find(&r, b"exec.sh"));
            assert_eq!(exec.mode_index, FileMode::REGULAR);
            assert_eq!(exec.mode_worktree, FileMode::EXECUTABLE);
            match find(&r, b"renamed.txt") {
                StatusEntry::Renamed(e) => {
                    assert_eq!(e.kind, RenameKind::Renamed);
                    assert_eq!(e.original_path.as_bytes(), b"rename-src.txt");
                }
                other => panic!("{other:?}"),
            }
            assert_eq!(
                find(&r, b"untracked.txt"),
                &StatusEntry::Untracked(GitPath::from_bytes(b"untracked.txt"))
            );
            assert_eq!(
                find(&r, b"debug.log"),
                &StatusEntry::Ignored(GitPath::from_bytes(b"debug.log"))
            );
            // non-UTF8 tracked 경로(작업 트리에 없음)는 원시 바이트 그대로 .D.
            assert_eq!(codes(b"raw\xff.bin"), ('.', 'D'));
            // index에만 새로 등록한 non-UTF8 경로는 AD.
            add_index_path(
                gate.runner().unwrap(),
                dir.path(),
                "100755",
                b"new\xfe.bin",
                b"x",
            );
            let r = read_status(&gate, dir.path(), StatusOptions::default()).unwrap();
            let e = ordinary(find(&r, b"new\xfe.bin"));
            assert_eq!(
                (e.index, e.worktree),
                (StatusCode::Added, StatusCode::Deleted)
            );
            assert_eq!(e.mode_index, FileMode::EXECUTABLE);
            // ignored 미요청이면 ignored 항목 없음.
            assert!(!r
                .entries
                .iter()
                .any(|e| matches!(e, StatusEntry::Ignored(_))));
        }

        #[test]
        fn real_copy_detection_output_parses() {
            let Some(git) = system_git() else {
                return;
            };
            let dir = TempDir::new("status-copy");
            let runner = hermetic_runner(git);
            init_repo_with_identity(&runner, dir.path());
            let body = "copy body 1\ncopy body 2\ncopy body 3\ncopy body 4\n";
            commit_file(&runner, dir.path(), "orig.txt", body.as_bytes());
            std::fs::write(dir.path().join("copy.txt"), body).unwrap();
            // copy 원본 후보는 같은 변경에서 수정된 파일이므로 원본도 조금 바꿉니다.
            std::fs::write(dir.path().join("orig.txt"), format!("{body}tail\n")).unwrap();
            git_ok(&runner, dir.path(), ["add", "copy.txt", "orig.txt"]);
            // copy 감지는 사용자 config로 켤 수 있는 형식 → parser가 처리해야 함.
            let out = git_ok(
                &runner,
                dir.path(),
                [
                    "-c",
                    "status.renames=copies",
                    "status",
                    "--porcelain=v2",
                    "-z",
                    "--branch",
                ],
            );
            let r = parse_status_v2(&out.stdout).unwrap();
            match find(&r, b"copy.txt") {
                StatusEntry::Renamed(e) => {
                    assert_eq!(e.kind, RenameKind::Copied);
                    assert_eq!(e.original_path.as_bytes(), b"orig.txt");
                    assert_eq!(e.entry.index, StatusCode::Copied);
                }
                other => panic!("{other:?}"),
            }
        }

        #[test]
        fn real_merge_conflict_output_parses_unmerged() {
            let Some(git) = system_git() else {
                return;
            };
            let dir = TempDir::new("status-unmerged");
            let runner = hermetic_runner(git);
            let repo = dir.path();
            init_repo_with_identity(&runner, repo);
            commit_file(&runner, repo, "both.txt", b"base\n");
            commit_file(&runner, repo, "gone.txt", b"keep\n");
            git_ok(&runner, repo, ["switch", "-q", "-c", "other"]);
            commit_file(&runner, repo, "both.txt", b"theirs\n");
            commit_file(&runner, repo, "gone.txt", b"theirs edit\n");
            commit_file(&runner, repo, "added.txt", b"theirs add\n");
            git_ok(&runner, repo, ["switch", "-q", "main"]);
            commit_file(&runner, repo, "both.txt", b"ours\n");
            git_ok(&runner, repo, ["rm", "-q", "gone.txt"]);
            git_ok(&runner, repo, ["commit", "-q", "-m", "rm"]);
            commit_file(&runner, repo, "added.txt", b"ours add\n");
            let merge = runner
                .run(&crate::runner::argv(["merge", "-q", "other"]), repo)
                .unwrap();
            assert!(!merge.success(), "충돌이 생기지 않음");

            let gate = GatedGitRunner::new(runner, GitTrust::Trusted);
            let r = read_status(&gate, repo, StatusOptions::default()).unwrap();
            let kind = |p: &[u8]| match find(&r, p) {
                StatusEntry::Unmerged(u) => u.kind,
                other => panic!("{other:?}"),
            };
            assert_eq!(kind(b"both.txt"), UnmergedKind::BothModified);
            assert_eq!(kind(b"gone.txt"), UnmergedKind::DeletedByUs);
            assert_eq!(kind(b"added.txt"), UnmergedKind::BothAdded);
        }

        #[test]
        fn untrusted_status_spawns_nothing() {
            let runner = GitRunner::new(
                std::path::PathBuf::from("/nonexistent/git"),
                Vec::new(),
                std::time::Duration::from_secs(5),
            );
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::NotTrusted);
            assert_eq!(
                read_status(&gate, &std::env::temp_dir(), StatusOptions::default()),
                Err(GitOpError::Gate(GatedRunError::ProjectNotTrusted))
            );
            assert_eq!(runner.spawn_count(), 0);
        }
    }
}
