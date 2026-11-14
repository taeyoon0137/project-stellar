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
use crate::snapshot::{IndexSnapshot, SnapshotEntry};
use crate::trust::{GatedGitRunner, GatedRunError, GitTrust};

mod ownership;

/// 실제 index 적용 실패. `OutcomeUnknown`이면 재조회가 필요하며 임의 rollback하면 안 됩니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishError {
    /// Git 실행·출력 오류(승인 전).
    Git(GitOpError),
    /// 준비 기준 또는 승인 직전 index bytes가 바뀌었습니다.
    StaleBase,
    /// 자식의 쓰기 FD와 lock inode의 소유 관계를 증명하지 못했습니다.
    LockOwnershipUnconfirmed,
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

fn input(candidate: &WrittenCandidate) -> Vec<u8> {
    let mut input = Vec::new();
    let oid_len = candidate
        .base_index()
        .entries
        .first()
        .map(|e| e.oid.as_str().len())
        .unwrap_or(40);
    let removal = format!("0 {}\t", "0".repeat(oid_len));
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
    Ok(())
}

/// 승인된 blob 후보를 실제 index에 적용하고 Index provider 결과를 대조합니다.
/// Working Tree나 인메모리 문서에 접근하지 않습니다. 아직 UI/FFI staging 진입점은 아닙니다.
/// macOS의 Apple/upstream Git 2.54.0에서 자식의 FD 소유를 확인할 수 있을 때만 승인합니다.
/// Linux 등 다른 OS·Git 버전은 실측 검증 전까지 진단합니다.
pub fn publish_candidate(
    git: &GatedGitRunner,
    worktree: &Path,
    candidate: &WrittenCandidate,
    cancel: &GitCancelToken,
) -> Result<IndexSnapshot, PublishError> {
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
    let version = require_success(git.run_cancellable(&argv(["--version"]), worktree, cancel)?)?;
    if crate::version::parse_git_version_output(&String::from_utf8_lossy(&version.stdout))
        != Some((2, 54, 0))
    {
        return Err(PublishError::Unsupported(
            "검증된 Git 2.54.0 (Apple/upstream) 필요",
        ));
    }
    let expected = expected_index(candidate)?;
    if candidate.changes().is_empty() {
        return if read_index_snapshot_cancellable(git, worktree, cancel)? == expected {
            Ok(expected)
        } else {
            Err(PublishError::StaleBase)
        };
    }
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
    let bytes = input(candidate);
    let mut failure = None;
    let mut approved = false;
    let mut check = |pid, deadline: Instant| {
        match ownership::owns_lock(pid, &lock) {
            Ok(false) => return Ok(false),
            Err(_) => {
                failure = Some(PublishError::LockOwnershipUnconfirmed);
                return Err(GitRunError::Cancelled);
            }
            Ok(true) => {}
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
    let result = git.run_with_checked_input(
        &argv([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.ignorestat=false",
            "update-index",
            "-z",
            "--index-info",
        ]),
        &root,
        &bytes,
        cancel,
        &mut check,
    );
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
    match read_index_snapshot_cancellable(git, &root, cancel) {
        Ok(actual) if actual == expected => Ok(actual),
        Ok(_) => Err(PublishError::OutcomeUnknown { cause: None }),
        Err(cause) => Err(PublishError::OutcomeUnknown { cause: Some(cause) }),
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests;
