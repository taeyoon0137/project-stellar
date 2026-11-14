//! Git 소유 index lock 아래 기준을 확인한 뒤 한 번의 index-info로 후보를 적용합니다.
//! 직접 index/lock 쓰기·삭제·rename은 하지 않습니다. 미확정 결과를 자동 덮어쓰지 않습니다.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::blob_write::{WrittenCandidate, WrittenFileDelta};
use crate::cancel::GitCancelToken;
use crate::decode::GitPath;
use crate::object::FileMode;
use crate::ops::{require_success, GitOpError};
use crate::runner::{argv, GitRunError};
use crate::snapshot::index::read_index_snapshot_cancellable;
use crate::snapshot::materialize::{materialize_snapshot, MaterializedSnapshot};
use crate::snapshot::{IndexSnapshot, SnapshotEntry};
use crate::trust::{GatedGitRunner, GatedRunError, GitTrust};

mod ownership;

/// 성공 확인 뒤 발급하는 일회용 조건부 복원 증표입니다. 외부에서 만들거나 복제할 수 없습니다.
/// 이후 index bytes가 달라지면 복원하지 않습니다. drop은 아무 쓰기도 하지 않습니다.
#[derive(Debug)]
pub struct PublishedIndex {
    inverse: WrittenCandidate,
    bytes: Option<Vec<u8>>,
    root: PathBuf,
    isolated_worktree: Option<MaterializedSnapshot>,
}

impl PublishedIndex {
    /// 실제 Index provider로 확인한 적용 결과.
    pub fn snapshot(&self) -> &IndexSnapshot {
        self.inverse.base_index()
    }
}

struct AppliedIndex {
    snapshot: IndexSnapshot,
    bytes: Option<Vec<u8>>,
    root: PathBuf,
}

/// 실제 적용과 provider 확인이 성공한 경우에만 복원 증표를 발급합니다.
pub fn publish_with_receipt(
    git: &GatedGitRunner,
    worktree: &Path,
    candidate: &WrittenCandidate,
    cancel: &GitCancelToken,
) -> Result<PublishedIndex, PublishError> {
    publish_receipt(git, worktree, candidate, cancel, None)
}

/// 앱의 선택 Stage/Unstage. 격리된 빈 작업 트리를 사용해 Git의 index stat 갱신이 실제
/// 파일에 clean 필터를 실행하지 않게 합니다. 적용과 조건부 복원 모두 Git hook을 끕니다.
pub fn publish_isolated(
    git: &GatedGitRunner,
    worktree: &Path,
    parent: &Path,
    candidate: &WrittenCandidate,
    cancel: &GitCancelToken,
) -> Result<PublishedIndex, PublishError> {
    git.runner()?;
    let parent = parent
        .canonicalize()
        .map_err(|e| PublishError::Io(e.kind()))?;
    let root = worktree
        .canonicalize()
        .map_err(|e| PublishError::Io(e.kind()))?;
    if parent.starts_with(root) {
        return Err(PublishError::Unsupported("격리 폴더가 작업 트리 안에 있음"));
    }
    let empty = materialize_snapshot(git, worktree, &[], &parent)
        .map_err(|_| PublishError::Unsupported("빈 임시 작업 트리 생성 실패"))?;
    publish_receipt(git, worktree, candidate, cancel, Some(empty))
}

fn publish_receipt(
    git: &GatedGitRunner,
    worktree: &Path,
    candidate: &WrittenCandidate,
    cancel: &GitCancelToken,
    isolated_worktree: Option<MaterializedSnapshot>,
) -> Result<PublishedIndex, PublishError> {
    let applied = publish_inner(
        git,
        worktree,
        candidate,
        cancel,
        None,
        isolated_worktree.as_ref(),
    )?;
    Ok(PublishedIndex {
        inverse: candidate.inverse(applied.snapshot),
        bytes: applied.bytes,
        root: applied.root,
        isolated_worktree,
    })
}

/// 성공 증표를 소비해 공식 Git 명령으로 원래 mode/OID/path를 복원합니다.
/// 잠금 안에서 증표의 전체 index bytes와 비교하며, 외부 변경이면 StaleBase입니다.
/// stat cache의 바이트 표현까지 복원하지는 않습니다. unrelated flags/REUC는 현재 Git이 유지합니다.
pub fn restore_published(
    git: &GatedGitRunner,
    receipt: PublishedIndex,
    cancel: &GitCancelToken,
) -> Result<IndexSnapshot, PublishError> {
    publish_inner(
        git,
        &receipt.root,
        &receipt.inverse,
        cancel,
        Some(&receipt.bytes),
        receipt.isolated_worktree.as_ref(),
    )
    .map(|r| r.snapshot)
}

/// 실제 index 적용 실패. `OutcomeUnknown`이면 재조회가 필요하며 임의 rollback하면 안 됩니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishError {
    /// Git 실행·출력 오류(승인 전).
    Git(GitOpError),
    /// 준비 기준 또는 승인 직전 index bytes가 바뀌었습니다.
    StaleBase,
    /// 자식의 쓰기 FD와 lock inode의 소유 관계를 증명하지 못했습니다.
    LockOwnershipUnconfirmed,
    /// 취소한 Git 뒤 lock이 남거나 상태를 읽을 수 없습니다. 직접 삭제하지 않습니다.
    LockCleanupUnconfirmed,
    /// 현재 지원하지 않는 index 상태입니다. 입력은 전달하지 않습니다.
    Unsupported(&'static str),
    /// 읽기 전용 파일 확인 실패.
    Io(std::io::ErrorKind),
    /// 입력 승인 이후 실패·취소·timeout·재조회 불일치. 타인의 index를 덮어쓰지 않습니다.
    OutcomeUnknown {
        /// 실패 명령. 명령 성공 뒤 provider 불일치이면 None.
        cause: Option<GitOpError>,
    },
}

impl std::fmt::Display for PublishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Git(e) => write!(f, "{e}"),
            Self::StaleBase => f.write_str("기준 index가 변경되어 적용하지 않았습니다"),
            Self::LockOwnershipUnconfirmed => {
                f.write_str("Git 자식의 index lock 소유를 확인하지 못했습니다")
            }
            Self::LockCleanupUnconfirmed => {
                f.write_str("Git 취소 뒤 lock 정리를 확인하지 못했습니다")
            }
            Self::Unsupported(reason) => write!(f, "지원하지 않는 index 상태: {reason}"),
            Self::Io(kind) => write!(f, "index 읽기 실패: {kind:?}"),
            Self::OutcomeUnknown { .. } => {
                f.write_str("index 적용 결과를 확정하지 못했습니다. 재조회가 필요합니다")
            }
        }
    }
}
impl std::error::Error for PublishError {}
impl From<GitOpError> for PublishError {
    fn from(e: GitOpError) -> Self {
        Self::Git(e)
    }
}
impl From<GatedRunError> for PublishError {
    fn from(e: GatedRunError) -> Self {
        Self::Git(e.into())
    }
}

fn index_bytes(path: &Path) -> Result<Option<Vec<u8>>, PublishError> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(PublishError::Io(e.kind())),
        Ok(m) if !m.is_file() => Err(PublishError::Unsupported("index가 일반 파일이 아님")),
        Ok(_) => std::fs::read(path)
            .map(Some)
            .map_err(|e| PublishError::Io(e.kind())),
    }
}

fn git_path(
    git: &GatedGitRunner,
    cwd: &Path,
    query: &[&str],
    cancel: &GitCancelToken,
) -> Result<PathBuf, PublishError> {
    let output = require_success(git.run_cancellable(&argv(query), cwd, cancel)?)?;
    let raw = output
        .stdout
        .strip_suffix(b"\n")
        .ok_or(PublishError::Unsupported("Git 경로 출력"))?;
    let path = GitPath::from_bytes(raw)
        .to_path_buf()
        .ok_or(PublishError::Unsupported("Git 경로 인코딩"))?;
    if !path.is_absolute() {
        return Err(PublishError::Unsupported("Git 절대 경로 필요"));
    }
    Ok(path)
}

fn change_path(change: &WrittenFileDelta) -> &GitPath {
    match change {
        WrittenFileDelta::Upsert { path, .. } | WrittenFileDelta::Remove { path } => path,
    }
}

fn expected_index(candidate: &WrittenCandidate) -> Result<IndexSnapshot, PublishError> {
    if candidate.base_index().has_conflicts() {
        return Err(PublishError::Unsupported("unmerged index"));
    }
    let mut entries: BTreeMap<_, _> = candidate
        .base_index()
        .entries
        .iter()
        .map(|e| (e.path.clone(), e.clone()))
        .collect();
    for change in candidate.changes() {
        let path = change_path(change);
        crate::snapshot::materialize::validate_snapshot_path(path.as_bytes())
            .map_err(|_| PublishError::Unsupported("후보 경로"))?;
        match change {
            WrittenFileDelta::Remove { path } => {
                entries.remove(path);
            }
            WrittenFileDelta::Upsert { path, mode, oid } => {
                if ![FileMode::REGULAR, FileMode::EXECUTABLE, FileMode::SYMLINK].contains(mode) {
                    return Err(PublishError::Unsupported("후보 mode"));
                }
                entries.insert(
                    path.clone(),
                    SnapshotEntry {
                        path: path.clone(),
                        mode: *mode,
                        oid: oid.clone(),
                    },
                );
            }
        }
    }
    for path in entries.keys() {
        for (i, byte) in path.as_bytes().iter().enumerate() {
            if *byte == b'/' && entries.contains_key(&GitPath::from_bytes(&path.as_bytes()[..i])) {
                return Err(PublishError::Unsupported("후보 D/F 충돌"));
            }
        }
    }
    let entries: Vec<_> = entries.into_values().collect();
    Ok(IndexSnapshot {
        entries,
        unmerged: Vec::new(),
    })
}

fn oid_len(candidate: &WrittenCandidate) -> usize {
    candidate
        .base_index()
        .entries
        .first()
        .map(|e| e.oid.as_str().len())
        .or_else(|| {
            candidate.changes().iter().find_map(|change| match change {
                WrittenFileDelta::Upsert { oid, .. } => Some(oid.as_str().len()),
                WrittenFileDelta::Remove { .. } => None,
            })
        })
        .unwrap_or(40)
}

fn check_input_budget(candidate: &WrittenCandidate) -> Result<(), PublishError> {
    // macOS sys/syslimits.h PIPE_BUF = 512. EOF가 commit인 protocol에서 부모 종료가
    // 유효한 일부 레코드만 전달하지 않도록 적용과 inverse 모두 한 atomic write에 맞춥니다.
    // 6-digit mode + space + OID + tab + path + NUL (remove는 더 짧습니다).
    let record_overhead = oid_len(candidate) + 9;
    let budget = candidate.changes().iter().fold(0usize, |total, change| {
        total.saturating_add(
            change_path(change)
                .as_bytes()
                .len()
                .saturating_add(record_overhead),
        )
    });
    if budget > 512 {
        return Err(PublishError::Unsupported(
            "index-info 적용/복원 입력의 512-byte 한도",
        ));
    }
    Ok(())
}

fn input(candidate: &WrittenCandidate) -> Vec<u8> {
    let mut input = Vec::new();
    let removal = format!("0 {}\t", "0".repeat(oid_len(candidate)));
    // 삭제를 먼저 모두 적용하므로 정상적인 D/F 전환도 하나의 transaction입니다.
    for change in candidate.changes() {
        if let WrittenFileDelta::Remove { path } = change {
            input.extend_from_slice(removal.as_bytes());
            input.extend_from_slice(path.as_bytes());
            input.push(0);
        }
    }
    for change in candidate.changes() {
        if let WrittenFileDelta::Upsert { path, mode, oid } = change {
            input.extend_from_slice(format!("{mode} {oid}\t").as_bytes());
            input.extend_from_slice(path.as_bytes());
            input.push(0);
        }
    }
    input
}

/// ls-files --debug는 안정된 protocol이 아니므로 구조가 달라지면 승인하지 않습니다.
fn check_flags(mut output: &[u8], candidate: &WrittenCandidate) -> Result<(), PublishError> {
    let mut base_entries = candidate.base_index().entries.iter();
    while !output.is_empty() {
        let nul = output
            .iter()
            .position(|b| *b == 0)
            .ok_or(PublishError::Unsupported("index flags 출력"))?;
        let path = &output[..nul];
        output = &output[nul + 1..];
        let mut last = &b""[..];
        for prefix in [
            b"  ctime: ".as_slice(),
            b"  mtime: ",
            b"  dev: ",
            b"  uid: ",
            b"  size: ",
        ] {
            let end = output
                .iter()
                .position(|b| *b == b'\n')
                .ok_or(PublishError::Unsupported("index flags 출력"))?;
            last = &output[..end];
            if !last.starts_with(prefix) {
                return Err(PublishError::Unsupported("index flags 출력"));
            }
            output = &output[end + 1..];
        }
        let flags = last
            .windows(7)
            .position(|b| b == b"flags: ")
            .and_then(|n| std::str::from_utf8(&last[n + 7..]).ok())
            .and_then(|s| u32::from_str_radix(s, 16).ok())
            .ok_or(PublishError::Unsupported("index flags 출력"))?;
        // CE_VALID, CE_EXTENDED, CE_INTENT_TO_ADD, CE_SKIP_WORKTREE만 보존을 검증했습니다.
        if flags & !0x6000_c000 != 0 {
            return Err(PublishError::Unsupported("검증되지 않은 index flags"));
        }
        if base_entries.next().map(|entry| entry.path.as_bytes()) != Some(path) {
            return Err(PublishError::Unsupported("index flags 경로 목록 불일치"));
        }
        if flags != 0
            && candidate
                .changes()
                .iter()
                .any(|c| change_path(c).as_bytes() == path)
        {
            return Err(PublishError::Unsupported(
                "변경 경로의 index flags (assume/skip/ITA 등)",
            ));
        }
    }
    if base_entries.next().is_some() {
        return Err(PublishError::Unsupported("index flags 경로 목록 불일치"));
    }
    Ok(())
}

fn protocol_runner(git: &GatedGitRunner) -> Result<GatedGitRunner, GatedRunError> {
    Ok(GatedGitRunner::new(
        git.runner()?.clone().with_added_env(
            [
                ("GIT_TRACE2_EVENT", "2"),
                ("GIT_TRACE2_EVENT_BRIEF", "0"),
                ("GIT_TRACE2_EVENT_NESTING", "10"),
                ("GIT_TRACE2_PARENT_SID", ""),
                ("GIT_TRACE2_CONFIG_PARAMS", ""),
                ("GIT_TRACE2_ENV_VARS", ""),
            ]
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect(),
        ),
        GitTrust::Trusted,
    ))
}

// v2.54.0 read_index_from emits this after lock creation/cleanup registration even for an absent index.
// This is readiness only: the separate kernel FD check still proves lock ownership.
fn index_read_ready(pid: u32, stderr: &[u8]) -> bool {
    let suffix = format!("-P{pid:08x}");
    stderr.split(|b| *b == b'\n').any(|line| {
        let Ok(line) = std::str::from_utf8(line) else {
            return false;
        };
        let Some(rest) = line.strip_prefix("{\"event\":\"region_leave\",\"sid\":\"") else {
            return false;
        };
        let Some((sid, fields)) = rest.split_once('"') else {
            return false;
        };
        sid.ends_with(&suffix)
            && line.ends_with('}')
            && fields.contains("\"category\":\"index\"")
            && fields.contains("\"label\":\"do_read_index\"")
    })
}

/// 승인된 blob 후보를 실제 index에 적용하고 Index provider 결과를 대조합니다.
/// Working Tree나 인메모리 문서에 접근하지 않습니다. 아직 UI/FFI staging 진입점은 아닙니다.
/// macOS에서 실제 update-index 자식의 쓰기 FD 소유와 선택한 Git의 index 조회 결과를 확인한 뒤 승인합니다.
/// 적용과 복원 입력 모두 atomic pipe 쓰기 한도(512 bytes) 안에 들어야 합니다.
pub fn publish_candidate(
    git: &GatedGitRunner,
    worktree: &Path,
    candidate: &WrittenCandidate,
    cancel: &GitCancelToken,
) -> Result<IndexSnapshot, PublishError> {
    publish_inner(git, worktree, candidate, cancel, None, None).map(|r| r.snapshot)
}

fn publish_inner(
    git: &GatedGitRunner,
    worktree: &Path,
    candidate: &WrittenCandidate,
    cancel: &GitCancelToken,
    required_bytes: Option<&Option<Vec<u8>>>,
    isolated_worktree: Option<&MaterializedSnapshot>,
) -> Result<AppliedIndex, PublishError> {
    let runner = git.runner()?;
    if !cfg!(target_os = "macos") {
        return Err(PublishError::Unsupported(
            "검증된 macOS lock 소유 확인 필요",
        ));
    }
    for name in [
        "GIT_INDEX_FILE",
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
    ] {
        if runner.child_env_var(name).is_some() {
            return Err(PublishError::Unsupported(
                "Git 저장소/index 환경 경로 재지정",
            ));
        }
    }
    if cancel.is_cancelled() {
        return Err(GatedRunError::Run(GitRunError::Cancelled).into());
    }
    let expected = expected_index(candidate)?;
    check_input_budget(candidate)?;
    let root = git_path(
        git,
        worktree,
        &["rev-parse", "--path-format=absolute", "--show-toplevel"],
        cancel,
    )?;
    let index = git_path(
        git,
        &root,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
        cancel,
    )?;
    let mut lock = index.as_os_str().to_os_string();
    lock.push(".lock");
    let lock = PathBuf::from(lock);
    let before = index_bytes(&index)?;
    if required_bytes.is_some_and(|required| required != &before) {
        return Err(PublishError::StaleBase);
    }
    if candidate.changes().is_empty() {
        return if read_index_snapshot_cancellable(git, &root, cancel)? == expected
            && index_bytes(&index)? == before
        {
            Ok(AppliedIndex {
                snapshot: expected,
                bytes: before,
                root,
            })
        } else {
            Err(PublishError::StaleBase)
        };
    }
    let bytes = input(candidate);
    let mut failure = None;
    let mut approved = false;
    let mut owned = false;
    let mut check = |pid, deadline: Instant, stderr: &[u8]| {
        if !index_read_ready(pid, stderr) {
            return Ok(false);
        }
        match ownership::owns_lock(pid, &lock) {
            Ok(false) => return Ok(false),
            Err(_) => {
                failure = Some(PublishError::LockOwnershipUnconfirmed);
                return Err(GitRunError::Cancelled);
            }
            Ok(true) => {
                owned = true;
            }
        }
        let validate = || -> Result<(), PublishError> {
            if index_bytes(&index)? != before {
                return Err(PublishError::StaleBase);
            }
            let limited = GatedGitRunner::new(
                git.runner()?
                    .clone()
                    .with_timeout(deadline.saturating_duration_since(Instant::now())),
                GitTrust::Trusted,
            );
            let current = read_index_snapshot_cancellable(&limited, &root, cancel)?;
            if &current != candidate.base_index() {
                return Err(PublishError::StaleBase);
            }
            // split/sparse index metadata 복원은 아직 보장하지 않습니다.
            let run = |args: &[&str]| -> Result<Vec<u8>, PublishError> {
                let limited = GatedGitRunner::new(
                    git.runner()?
                        .clone()
                        .with_timeout(deadline.saturating_duration_since(Instant::now())),
                    GitTrust::Trusted,
                );
                Ok(require_success(limited.run_cancellable(&argv(args), &root, cancel)?)?.stdout)
            };
            let shared = run(&["rev-parse", "--shared-index-path"])?;
            if shared != b"\n" && !shared.is_empty() {
                return Err(PublishError::Unsupported("split index"));
            }
            let sparse = run(&["ls-files", "--sparse", "--stage", "-z"])?;
            if sparse.split(|b| *b == 0).any(|r| r.starts_with(b"040000 ")) {
                return Err(PublishError::Unsupported("sparse index"));
            }
            check_flags(&run(&["ls-files", "--debug", "-z"])?, candidate)?;
            if index_bytes(&index)? != before {
                return Err(PublishError::StaleBase);
            }
            if !ownership::owns_lock(pid, &lock).unwrap_or(false) {
                return Err(PublishError::LockOwnershipUnconfirmed);
            }
            Ok(())
        };
        match validate() {
            Ok(()) => {
                approved = true;
                Ok(true)
            }
            Err(error) => {
                failure = Some(error);
                Err(GitRunError::Cancelled)
            }
        }
    };
    let mut protocol_git = protocol_runner(git)?;
    let mut args = Vec::new();
    let child_cwd = if let Some(empty) = isolated_worktree {
        // Git can refresh other index entries even with --index-info. Both cwd and
        // GIT_WORK_TREE must be empty, otherwise relative stats can run clean filters.
        let git_dir = git_path(git, &root, &["rev-parse", "--absolute-git-dir"], cancel)?;
        protocol_git = GatedGitRunner::new(
            protocol_git.runner()?.clone().with_added_env(vec![
                ("GIT_DIR".into(), git_dir.into_os_string()),
                ("GIT_WORK_TREE".into(), empty.root().as_os_str().to_owned()),
            ]),
            GitTrust::Trusted,
        );
        args.extend(argv(["-c", "core.hooksPath=/dev/null"]));
        empty.root()
    } else {
        &root
    };
    args.extend(argv([
        "-c",
        "core.fsmonitor=false",
        "-c",
        "core.ignorestat=false",
        "update-index",
        "-z",
        "--index-info",
    ]));
    let result = protocol_git.run_with_checked_input(&args, child_cwd, &bytes, cancel, &mut check);
    if !approved && lock.try_exists().unwrap_or(true) {
        return Err(if owned {
            PublishError::LockCleanupUnconfirmed
        } else {
            PublishError::LockOwnershipUnconfirmed
        });
    }
    if let Some(error) = failure {
        return Err(error);
    }
    if !approved {
        if let Err(GatedRunError::Run(GitRunError::Cancelled)) = result {
            return Err(GatedRunError::Run(GitRunError::Cancelled).into());
        }
        return Err(PublishError::LockOwnershipUnconfirmed);
    }
    let completion = result.map_err(GitOpError::from).and_then(require_success);
    if let Err(cause) = completion {
        return Err(PublishError::OutcomeUnknown { cause: Some(cause) });
    }
    // 하나의 안정된 실제 index 표현을 provider 결과에 묶습니다. 이후 복원은 이 bytes와 CAS합니다.
    let installed =
        index_bytes(&index).map_err(|_| PublishError::OutcomeUnknown { cause: None })?;
    match read_index_snapshot_cancellable(git, &root, cancel) {
        Ok(actual) if actual == expected => {
            if index_bytes(&index).ok().as_ref() != Some(&installed) {
                return Err(PublishError::OutcomeUnknown { cause: None });
            }
            Ok(AppliedIndex {
                snapshot: actual,
                bytes: installed,
                root,
            })
        }
        Ok(_) => Err(PublishError::OutcomeUnknown { cause: None }),
        Err(cause) => Err(PublishError::OutcomeUnknown { cause: Some(cause) }),
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests;
