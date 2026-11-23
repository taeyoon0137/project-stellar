//! Semantic Merge 해결·등록이 쓰는 Git 접근 포트 (ADR-0081 결정 7).
//!
//! ops는 Git을 직접 실행하지 않습니다(허용 crate graph에 git-porcelain이 없음). 필요한 Git 동작만 이 trait로
//! 받습니다. 실행 구현은 호출자(지금은 `stellar` CLI의 시스템 Git adapter)가 주고, ops→git-porcelain 경로가 생기면
//! `GatedGitRunner` 기반 구현으로 바꿉니다.
//!
//! **없는 동작**: `merge --continue`·`rebase --continue`·`cherry-pick --continue`·`commit`·`push`와 전역·시스템
//! config 쓰기는 포트에 없습니다(XIV.12.5, XXIV.2.3). 해결 흐름은 구조적으로 그 명령을 부를 수 없습니다.

use std::fmt;
use std::path::{Path, PathBuf};

/// unmerged index의 stage blob 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageBlob {
    /// 파일 mode(예: `100644`).
    pub mode: String,
    /// blob object id.
    pub oid: String,
}

/// unmerged 경로 하나(`git ls-files -u` 묶음).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmergedEntry {
    /// 작업 트리 기준 경로(포트 호출 폴더 기준).
    pub path: PathBuf,
    /// stage 1(base)·2(ours)·3(theirs). 없는 stage는 그쪽에 파일이 없음입니다.
    pub stages: [Option<StageBlob>; 3],
}

/// 진행 중인 Git 작업.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitOperation {
    /// 진행 중 작업 없음.
    None,
    /// `git merge`.
    Merge,
    /// `git rebase`.
    Rebase,
    /// `git cherry-pick`.
    CherryPick,
    /// `git revert`.
    Revert,
}

impl GitOperation {
    /// 진단용 이름.
    pub fn name(self) -> &'static str {
        match self {
            GitOperation::None => "none",
            GitOperation::Merge => "merge",
            GitOperation::Rebase => "rebase",
            GitOperation::CherryPick => "cherry-pick",
            GitOperation::Revert => "revert",
        }
    }
}

/// Git 실행 실패(종료 코드·stderr 요약).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitPortError(pub String);

impl fmt::Display for GitPortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Git 실행 실패: {}", self.0)
    }
}

impl std::error::Error for GitPortError {}

/// Semantic Merge 해결·등록이 쓰는 Git 동작.
pub trait MergeGit {
    /// `git ls-files -u -z`: unmerged 경로와 stage blob(`worktree` 기준 경로).
    fn unmerged(&self, worktree: &Path) -> Result<Vec<UnmergedEntry>, GitPortError>;

    /// blob bytes. `limit` bytes를 넘으면 읽지 않고 실패합니다.
    fn blob(&self, worktree: &Path, oid: &str, limit: u64) -> Result<Vec<u8>, GitPortError>;

    /// `git add -- <paths>`(resolved 처리, XIV.12.4). 경로는 `worktree` 기준입니다.
    fn add(&self, worktree: &Path, paths: &[PathBuf]) -> Result<(), GitPortError>;

    /// 진행 중인 merge·rebase·cherry-pick·revert.
    fn operation(&self, worktree: &Path) -> Result<GitOperation, GitPortError>;

    /// 저장소 로컬 config 값(`git config --local --get`). 없으면 `None`.
    fn local_config(&self, worktree: &Path, key: &str) -> Result<Option<String>, GitPortError>;

    /// 저장소 로컬 config 값 쓰기(`git config --local`). 전역·시스템 config는 쓰지 않습니다.
    fn set_local_config(&self, worktree: &Path, key: &str, value: &str)
        -> Result<(), GitPortError>;
}
