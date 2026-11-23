//! 검증한 해결 결과를 WorkingTree에 쓰고 `git add`로 resolved 처리 (XIV.12.4~XIV.12.6, ADR-0081 결정 6).
//!
//! 순서: (1) 세션의 모든 단위를 검증해 쓰기 계획을 만듭니다(XIV.12.3). (2) 세션을 연 뒤 unmerged 목록이나 대상 파일이
//! 바뀌었으면(stale) 아무것도 쓰지 않습니다. (3) 계획한 파일을 한 번에 씁니다(각 파일은 임시 파일 뒤 rename). (4) 관련
//! 경로를 `git add`합니다. (5) Git 상태(진행 중 작업, 남은 unmerged 경로)만 다시 읽어 돌려줍니다.
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
    /// 파일 쓰기 실패.
    Write {
        /// 실패한 경로.
        path: PathBuf,
        /// 원인.
        error: io::Error,
    },
    /// `git add` 실패.
    Add(GitPortError),
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApplyError::Plan(e) => write!(f, "{e}"),
            ApplyError::Stale(e) => write!(f, "세션을 연 뒤 상태가 바뀌었습니다: {e}"),
            ApplyError::Git(e) => write!(f, "{e}"),
            ApplyError::Write { path, error } => write!(f, "{} 쓰기 실패: {error}", path.display()),
            ApplyError::Add(e) => write!(f, "git add 실패: {e}"),
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
            })?
            .map(|bytes| ContentHash::compute(&bytes));
        if current != unit.observed() {
            return Err(ApplyError::Stale(unit.path().display().to_string()));
        }
    }
    Ok(())
}

/// 계획을 씁니다.
fn write_plan(
    files: &dyn TreeFiles,
    root: &Path,
    plan: &WritePlan,
) -> Result<Vec<PathBuf>, ApplyError> {
    let mut written = Vec::new();
    for write in &plan.writes {
        let path = root.join(&write.path);
        let outcome = match &write.bytes {
            Some(bytes) => files.write(&path, bytes),
            None => files.remove(&path),
        };
        outcome.map_err(|error| ApplyError::Write {
            path: write.path.clone(),
            error,
        })?;
        written.push(write.path.clone());
    }
    Ok(written)
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
    let written = write_plan(files, root, &plan)?;
    git.add(root, &plan.add).map_err(ApplyError::Add)?;
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
