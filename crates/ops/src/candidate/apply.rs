//! 검증한 해결 결과를 WorkingTree에 쓰고 `git add`로 resolved 처리 (XIV.12.4~XIV.12.6, ADR-0081 결정 6).
//!
//! 순서: (1) 세션의 모든 단위를 검증해 쓰기 계획을 만듭니다(XIV.12.3). (2) 세션을 연 뒤 unmerged 목록이나 대상 파일이
//! 바뀌었으면(stale) 아무것도 쓰지 않습니다. (3) 계획한 파일을 한 번에 씁니다(각 파일은 임시 파일 뒤 rename). (4) 관련
//! 경로를 `git add`합니다. (5) Git 상태(진행 중 작업, 남은 unmerged 경로)만 다시 읽어 돌려줍니다.
//!
//! **fault rollback(XIV.12.6)**: 쓰기 전에 대상 파일의 이전 bytes(없으면 없음)를 모두 읽어 둡니다. 쓰기 i번째가
//! 실패하거나 모든 쓰기 뒤 `git add`가 실패하면, 이미 쓴 파일을 역순으로 이전 bytes로 되돌리고(새로 만든 파일은 지움)
//! unresolved 상태와 복구 안내를 돌려줍니다. index는 `git add`가 한 번의 원자적 갱신이라 실패하면 바뀌지 않습니다.
//! 되돌리기마저 실패한 파일은 오류에 경로를 남겨 사용자가 확인하게 합니다. commit은 어떤 경우에도 하지 않습니다.
//! 다중 파일 journal(ADR-0078)이 들어오면 이 되돌리기를 journal 복구로 바꿉니다.
//!
//! **continue·commit 없음(XIV.12.5)**: merge·rebase·cherry-pick을 이어 가거나 commit하지 않습니다. 포트(`MergeGit`)에
//! 그런 동작이 없어 이 경로는 그 명령을 부를 수 없습니다. 사용자가 결과를 보고 `git merge --continue`·`git commit` 등을
//! 직접 실행합니다(기술 아키텍처 §27.2).

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use stellar_storage::hash::ContentHash;

use super::git_port::{GitOperation, GitPortError, MergeGit};
use super::session::{ConflictSession, PlanError, WritePlan};

/// 작업 트리 파일 접근(쓰기 실패 주입을 위한 경계).
pub trait TreeFiles {
    /// 파일 bytes(없으면 `None`).
    fn read(&self, path: &Path) -> io::Result<Option<Vec<u8>>>;
    /// 파일 쓰기(부모 폴더 생성, 임시 파일 뒤 rename).
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    /// 파일 삭제(없으면 성공).
    fn remove(&self, path: &Path) -> io::Result<()>;
}

/// 실제 파일 시스템.
#[derive(Debug, Clone, Copy, Default)]
pub struct DiskFiles;

impl TreeFiles for DiskFiles {
    fn read(&self, path: &Path) -> io::Result<Option<Vec<u8>>> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "부모 폴더가 없음"))?;
        std::fs::create_dir_all(parent)?;
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let temp = parent.join(format!(".{name}.stellar-merge-{}", std::process::id()));
        std::fs::write(&temp, bytes)?;
        std::fs::rename(&temp, path).inspect_err(|_| {
            let _ = std::fs::remove_file(&temp);
        })
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        match std::fs::remove_file(path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

/// 적용 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyReport {
    /// 쓰거나 지운 파일(프로젝트 기준).
    pub written: Vec<PathBuf>,
    /// resolved 처리한 경로.
    pub added: Vec<PathBuf>,
    /// add 뒤 진행 중인 Git 작업(읽지 못하면 `None`). 이어 가기는 사용자가 합니다.
    pub operation: Option<GitOperation>,
    /// add 뒤 남은 unmerged 경로(raw conflict 등, 읽지 못하면 `None`).
    pub remaining: Option<Vec<PathBuf>>,
}

/// 적용 실패.
#[derive(Debug)]
pub enum ApplyError {
    /// 쓰기 조건 미충족 또는 결과 직렬화 실패. 아무것도 쓰지 않았습니다.
    Plan(PlanError),
    /// 세션을 연 뒤 Git 상태·파일이 바뀜. 아무것도 쓰지 않았습니다.
    Stale(String),
    /// Git 읽기 실패. 아무것도 쓰지 않았습니다.
    Git(GitPortError),
    /// 파일 쓰기 실패. 이미 쓴 파일은 되돌렸습니다(`unrestored`에 남은 경로 제외).
    Write {
        /// 실패한 경로.
        path: PathBuf,
        /// 원인.
        error: io::Error,
        /// 되돌리지 못한 경로.
        unrestored: Vec<PathBuf>,
    },
    /// `git add` 실패. 쓴 파일은 되돌렸습니다(`unrestored`에 남은 경로 제외).
    Add {
        /// 원인.
        error: GitPortError,
        /// 되돌리지 못한 경로.
        unrestored: Vec<PathBuf>,
    },
}

impl ApplyError {
    /// 사용자 복구 안내(XIV.12.6). conflict는 unresolved 그대로입니다.
    pub fn guidance(&self) -> String {
        match self {
            ApplyError::Write { unrestored, .. } | ApplyError::Add { unrestored, .. }
                if !unrestored.is_empty() =>
            {
                let paths: Vec<String> =
                    unrestored.iter().map(|p| p.display().to_string()).collect();
                format!(
                    "다음 파일을 이전 상태로 되돌리지 못했습니다: {}. 내용을 확인하고 `git checkout --merge -- <경로>`로 \
                     conflict 상태를 다시 만들 수 있습니다. conflict는 unresolved이며 commit하지 않았습니다",
                    paths.join(", ")
                )
            }
            ApplyError::Write { .. } | ApplyError::Add { .. } => {
                "작업 트리를 이전 상태로 되돌렸습니다. conflict는 unresolved이며 commit하지 않았습니다. 원인을 해결한 뒤 \
                 다시 실행하세요"
                    .into()
            }
            _ => "아무것도 쓰지 않았습니다. conflict는 unresolved입니다".into(),
        }
    }
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApplyError::Plan(e) => write!(f, "{e}"),
            ApplyError::Stale(e) => write!(f, "세션을 연 뒤 상태가 바뀌었습니다: {e}"),
            ApplyError::Git(e) => write!(f, "{e}"),
            ApplyError::Write { path, error, .. } => {
                write!(f, "{} 쓰기 실패: {error}", path.display())
            }
            ApplyError::Add { error, .. } => write!(f, "git add 실패: {error}"),
        }
    }
}

impl std::error::Error for ApplyError {}

fn check_fresh(
    git: &dyn MergeGit,
    files: &dyn TreeFiles,
    session: &ConflictSession,
) -> Result<(), ApplyError> {
    let root = session.root();
    if git.unmerged(root).map_err(ApplyError::Git)? != session.entries() {
        return Err(ApplyError::Stale("unmerged 목록".into()));
    }
    for unit in session.units() {
        let current = files
            .read(&root.join(unit.path()))
            .map_err(|error| ApplyError::Write {
                path: unit.path().to_path_buf(),
                error,
                unrestored: Vec::new(),
            })?
            .map(|bytes| ContentHash::compute(&bytes));
        if current != unit.observed() {
            return Err(ApplyError::Stale(unit.path().display().to_string()));
        }
    }
    Ok(())
}

/// 쓰기 전 이전 bytes(되돌리기용).
type Previous = Vec<(PathBuf, Option<Vec<u8>>)>;

/// 쓴 파일을 역순으로 이전 bytes로 되돌립니다. 되돌리지 못한 경로를 돌려줍니다.
fn restore(files: &dyn TreeFiles, root: &Path, previous: &Previous, count: usize) -> Vec<PathBuf> {
    let mut unrestored = Vec::new();
    for (path, bytes) in previous[..count].iter().rev() {
        let full = root.join(path);
        let outcome = match bytes {
            Some(bytes) => files.write(&full, bytes),
            None => files.remove(&full),
        };
        if outcome.is_err() {
            unrestored.push(path.clone());
        }
    }
    unrestored
}

/// 계획을 씁니다. 실패하면 이미 쓴 파일을 되돌립니다(XIV.12.6).
fn write_plan(
    files: &dyn TreeFiles,
    root: &Path,
    plan: &WritePlan,
) -> Result<(Vec<PathBuf>, Previous), ApplyError> {
    let mut previous = Vec::with_capacity(plan.writes.len());
    for write in &plan.writes {
        let bytes = files
            .read(&root.join(&write.path))
            .map_err(|error| ApplyError::Write {
                path: write.path.clone(),
                error,
                unrestored: Vec::new(),
            })?;
        previous.push((write.path.clone(), bytes));
    }
    let mut written = Vec::new();
    for (index, write) in plan.writes.iter().enumerate() {
        let path = root.join(&write.path);
        let outcome = match &write.bytes {
            Some(bytes) => files.write(&path, bytes),
            None => files.remove(&path),
        };
        if let Err(error) = outcome {
            // 실패한 파일도 임시 파일 뒤 rename이라 그대로일 수 있지만, 안전하게 함께 되돌립니다.
            let unrestored = restore(files, root, &previous, index + 1);
            return Err(ApplyError::Write {
                path: write.path.clone(),
                error,
                unrestored,
            });
        }
        written.push(write.path.clone());
    }
    Ok((written, previous))
}

/// 세션의 해결 결과를 작업 트리에 쓰고 resolved 처리합니다(XIV.12.4).
pub fn apply_session(
    git: &dyn MergeGit,
    files: &dyn TreeFiles,
    session: &ConflictSession,
) -> Result<ApplyReport, ApplyError> {
    let plan = session.plan().map_err(ApplyError::Plan)?;
    check_fresh(git, files, session)?;
    let root = session.root();
    let (written, previous) = write_plan(files, root, &plan)?;
    if let Err(error) = git.add(root, &plan.add) {
        let unrestored = restore(files, root, &previous, previous.len());
        return Err(ApplyError::Add { error, unrestored });
    }
    // add 뒤에는 상태만 읽습니다(XIV.12.5). 읽기 실패는 결과(쓰기·add 성공)를 바꾸지 않습니다.
    let operation = git.operation(root).ok();
    let remaining = git
        .unmerged(root)
        .ok()
        .map(|entries| entries.into_iter().map(|e| e.path).collect());
    Ok(ApplyReport {
        written,
        added: plan.add,
        operation,
        remaining,
    })
}

#[cfg(test)]
pub(crate) mod tests;
