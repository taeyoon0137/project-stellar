//! 진행 중 merge의 semantic conflict 읽기 진입점 (XIX.9.3, ADR-0085 결정 8, ADR-0081 결정 7).
//!
//! `stellar merge-driver resolve <프로젝트>`(`--apply` 없음)와 같은 [`ConflictSession`]을 앱 없이 엽니다. Git은
//! [`GitSession::open_for_os_user`]의 [`GatedGitRunner`](신뢰한 프로젝트만 실행)로만 실행하고, 이 모듈의
//! [`MergeGit`] 구현은 읽기 동작만 합니다. `add`·`set_local_config`는 실행하지 않고 실패합니다. 그래서 이 진입점은
//! 작업 트리·index·Git 설정을 바꾸지 않습니다.
//!
//! 이 모듈은 AI·모델·에이전트 코드가 아닙니다.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use stellar_git_porcelain::conflict::read_unmerged_paths;
use stellar_git_porcelain::object::ObjectId;
use stellar_git_porcelain::repo_status::{read_in_progress_operations, InProgressOperation};
use stellar_git_porcelain::snapshot::blob::{read_blobs_with_limits, BlobLimits};
use stellar_git_porcelain::trust::GatedGitRunner;

use crate::candidate::git_port::{GitOperation, GitPortError, MergeGit, StageBlob, UnmergedEntry};
use crate::candidate::session::{ConflictSession, SessionError};
use crate::git::{GitError, GitSession};

/// 읽기 전용 [`MergeGit`]: gate를 거친 실행기로 unmerged index·blob·진행 작업·로컬 config만 읽습니다.
pub struct ReadOnlyMergeGit<'a> {
    git: &'a GatedGitRunner,
}

impl<'a> ReadOnlyMergeGit<'a> {
    /// gate를 거친 실행기로 만듭니다.
    pub fn new(git: &'a GatedGitRunner) -> Self {
        ReadOnlyMergeGit { git }
    }
}

fn port(error: impl std::fmt::Display) -> GitPortError {
    GitPortError(error.to_string())
}

impl MergeGit for ReadOnlyMergeGit<'_> {
    fn unmerged(&self, worktree: &Path) -> Result<Vec<UnmergedEntry>, GitPortError> {
        let paths = read_unmerged_paths(self.git, worktree).map_err(port)?;
        paths
            .into_iter()
            .map(|path| {
                let stage = |blob: Option<&stellar_git_porcelain::conflict::StageBlob>| {
                    blob.map(|b| StageBlob {
                        mode: b.mode.to_octal_string(),
                        oid: b.oid.as_str().to_owned(),
                    })
                };
                Ok(UnmergedEntry {
                    path: path
                        .path
                        .to_path_buf()
                        .ok_or_else(|| GitPortError("UTF-8이 아닌 unmerged 경로".into()))?,
                    stages: [
                        stage(path.base.as_ref()),
                        stage(path.ours.as_ref()),
                        stage(path.theirs.as_ref()),
                    ],
                })
            })
            .collect()
    }

    fn blob(&self, worktree: &Path, oid: &str, limit: u64) -> Result<Vec<u8>, GitPortError> {
        let id = ObjectId::parse(oid).ok_or_else(|| GitPortError("blob ID 형식".into()))?;
        let limits = BlobLimits {
            max_blob_bytes: limit,
            max_batch_bytes: limit,
            max_total_bytes: limit,
        };
        read_blobs_with_limits(self.git, worktree, std::slice::from_ref(&id), &limits)
            .map_err(port)?
            .remove(&id)
            .ok_or_else(|| GitPortError("blob을 읽지 못했습니다".into()))
    }

    fn add(&self, _worktree: &Path, _paths: &[PathBuf]) -> Result<(), GitPortError> {
        Err(GitPortError(
            "읽기 전용 conflict 보기는 index를 바꾸지 않습니다".into(),
        ))
    }

    fn operation(&self, worktree: &Path) -> Result<GitOperation, GitPortError> {
        let operations = read_in_progress_operations(self.git, worktree).map_err(port)?;
        Ok(match operations.first() {
            None => GitOperation::None,
            Some(InProgressOperation::Merge) => GitOperation::Merge,
            Some(InProgressOperation::CherryPick) => GitOperation::CherryPick,
            Some(InProgressOperation::Revert) => GitOperation::Revert,
            Some(InProgressOperation::Rebase | InProgressOperation::ApplyMailbox) => {
                GitOperation::Rebase
            }
        })
    }

    fn local_config(&self, worktree: &Path, key: &str) -> Result<Option<String>, GitPortError> {
        let args: Vec<OsString> = ["config", "--local", "--get", key]
            .iter()
            .map(OsString::from)
            .collect();
        let output = self.git.run(&args, worktree).map_err(port)?;
        match output.exit_code {
            Some(0) => String::from_utf8(output.stdout)
                .map(|text| Some(text.trim_end_matches('\n').to_owned()))
                .map_err(|_| GitPortError("UTF-8이 아닌 Git 출력".into())),
            Some(1) => Ok(None),
            code => Err(GitPortError(format!("git config --get: 종료 {code:?}"))),
        }
    }

    fn set_local_config(
        &self,
        _worktree: &Path,
        _key: &str,
        _value: &str,
    ) -> Result<(), GitPortError> {
        Err(GitPortError(
            "읽기 전용 conflict 보기는 Git 설정을 바꾸지 않습니다".into(),
        ))
    }
}

/// conflict 세션 열기 실패.
#[derive(Debug)]
pub enum SemanticConflictsError {
    /// Git 세션을 열 수 없습니다(신뢰 없음·Git 없음·worktree root 아님).
    Git(GitError),
    /// unmerged index·blob을 읽을 수 없습니다.
    Session(SessionError),
}

impl std::fmt::Display for SemanticConflictsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SemanticConflictsError::Git(e) => write!(f, "{e}"),
            SemanticConflictsError::Session(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SemanticConflictsError {}

/// 신뢰한 프로젝트 루트 `root`의 진행 중 conflict 세션을 읽기 전용으로 엽니다. 해결은 적용하지 않습니다.
pub fn semantic_conflicts(root: &Path) -> Result<ConflictSession, SemanticConflictsError> {
    let session = GitSession::open_for_os_user(root).map_err(SemanticConflictsError::Git)?;
    ConflictSession::open(&ReadOnlyMergeGit::new(session.git()), session.root())
        .map_err(SemanticConflictsError::Session)
}
