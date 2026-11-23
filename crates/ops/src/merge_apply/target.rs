//! Merge Composer 결과 target 정의·검증과 새 branch+worktree 생성 (XXIII.3.1·XXIII.3.2, ADR-0110 결정 3·4).
//!
//! | target | 뜻 | 검증 |
//! |---|---|---|
//! | [`ResultTarget::CurrentWorktree`] | Merge Composer를 연 worktree의 프로젝트 | 프로젝트 열림·같은 project id, 진행 중 Git 작업·unmerged 없음 |
//! | [`ResultTarget::ExistingWorktree`] | `git worktree list`에 있는 다른 worktree의 같은 상대 경로 프로젝트 | 목록에 있음(bare·prunable 아님) + 위와 같음 |
//! | [`ResultTarget::NewBranchWorktree`] | 새 branch를 만들어 새 경로에 checkout한 worktree | branch 이름 형식·미존재, 경로 미존재·부모 있음·다른 worktree 밖, 시작 commit이 그 프로젝트를 담음 |
//!
//! 검증은 읽기만 합니다(`rev-parse`·`worktree list`·`ls-tree`·`ls-files -u`). 새 branch·worktree는 검증과 source 재검사를
//! 통과한 뒤 [`create_branch_worktree`]가 **시스템 Git `git worktree add -b <branch> -- <path> <start>` 하나로**
//! 만듭니다(XXIII.3.2). `.git` 아래 파일을 직접 만들거나 고치지 않고, 실패해도 만든 것을 지우는 파괴적 명령을 부르지
//! 않습니다(사용자가 결과를 보고 정리).
//!
//! 기존 target은 검증 시점의 프로젝트 revision을 기록합니다. 쓰기는 저장 잠금 안에서 그 revision을 다시 비교합니다
//! (XXIII.3.3).

use std::fmt;
use std::path::{Component, Path, PathBuf};

use stellar_git_porcelain::advanced::worktree::list_worktrees;
use stellar_git_porcelain::advanced::worktree_ops::{plan_add, AddTarget};
use stellar_git_porcelain::advanced::{execute_plan, PlanError};
use stellar_git_porcelain::composer::{resolve_source, SourceError, SourceRef};
use stellar_git_porcelain::location::worktree_root;
use stellar_git_porcelain::object::ObjectId;
use stellar_git_porcelain::refname::is_valid_ref_body;
use stellar_git_porcelain::snapshot::{read_tree_snapshot, SnapshotSource};
use stellar_git_porcelain::trust::GatedGitRunner;

use crate::candidate::git_port::{GatedMergeGit, GitOperation, MergeGit};
use crate::core_types::ids::StableId;
use crate::error::OpenError;
use crate::revision::ProjectRevision;
use crate::snapshot::open_snapshot;

/// 결과 target(XXIII.3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultTarget {
    /// Merge Composer를 연 worktree.
    CurrentWorktree,
    /// 같은 저장소의 다른 기존 worktree.
    ExistingWorktree {
        /// worktree root 경로.
        path: PathBuf,
    },
    /// 새 branch를 만들어 새 경로에 checkout합니다.
    NewBranchWorktree {
        /// 새 branch 이름(`refs/heads/` 뒤).
        branch: String,
        /// 새 worktree 경로(아직 없어야 함).
        path: PathBuf,
        /// 시작 commit(보통 Source A의 commit).
        start: ObjectId,
    },
}

impl ResultTarget {
    /// 안정 이름.
    pub fn name(&self) -> &'static str {
        match self {
            ResultTarget::CurrentWorktree => "current-worktree",
            ResultTarget::ExistingWorktree { .. } => "existing-worktree",
            ResultTarget::NewBranchWorktree { .. } => "new-branch-worktree",
        }
    }
}

/// 검증한 target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedTarget {
    target: ResultTarget,
    worktree: PathBuf,
    project_root: PathBuf,
    expected: Option<ProjectRevision>,
}

impl ValidatedTarget {
    /// 요청한 target.
    pub fn target(&self) -> &ResultTarget {
        &self.target
    }

    /// target worktree root(새 target이면 만들 경로).
    pub fn worktree(&self) -> &Path {
        &self.worktree
    }

    /// target 프로젝트 root(worktree + Merge Composer 프로젝트의 저장소 상대 경로).
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// 기존 target의 검증 시점 프로젝트 revision. 새 target은 만든 뒤에 정해지므로 `None`입니다.
    pub fn expected_revision(&self) -> Option<ProjectRevision> {
        self.expected
    }
}

/// target 검증·생성 실패. 새 target 생성 전 실패는 아무것도 만들지 않았습니다.
#[derive(Debug)]
pub enum TargetError {
    /// 경로가 Git worktree 안이 아니거나 worktree root를 구하지 못했습니다.
    Location(String),
    /// `git worktree list`에 없는 경로입니다.
    NotAWorktree(PathBuf),
    /// branch 이름 형식이 맞지 않습니다(Git 실행 전 거부).
    InvalidBranch(String),
    /// 같은 이름의 branch가 이미 있습니다.
    BranchExists(String),
    /// 새 worktree 경로가 이미 있습니다.
    PathExists(PathBuf),
    /// 새 worktree 경로가 쓸 수 없는 형태입니다(부모 없음·상대 구성 요소 등).
    InvalidPath(PathBuf),
    /// 새 worktree 경로가 기존 worktree 안입니다.
    InsideWorktree {
        /// 요청 경로.
        path: PathBuf,
        /// 겹치는 worktree.
        worktree: PathBuf,
    },
    /// 시작 commit이 없거나 commit이 아닙니다.
    StartNotFound(ObjectId),
    /// 시작 commit에 이 프로젝트(`stellar.yaml`·`design/current`)가 없습니다.
    StartWithoutProject(ObjectId),
    /// target 프로젝트를 열 수 없습니다.
    Project(OpenError),
    /// target 프로젝트의 project id가 다릅니다.
    ProjectMismatch {
        /// Merge Composer 프로젝트 id.
        expected: StableId,
        /// target의 id.
        found: StableId,
    },
    /// target worktree에 진행 중인 Git 작업이 있습니다(merge·rebase 등).
    OperationInProgress(GitOperation),
    /// target worktree index에 unmerged 경로가 있습니다.
    Unmerged(usize),
    /// Git 실행·해석 실패.
    Git(String),
}

impl fmt::Display for TargetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TargetError::Location(e) => write!(f, "worktree 위치를 확인할 수 없습니다: {e}"),
            TargetError::NotAWorktree(p) => {
                write!(f, "{}는 이 저장소의 worktree가 아닙니다", p.display())
            }
            TargetError::InvalidBranch(name) => write!(f, "branch 이름 형식이 아닙니다: {name}"),
            TargetError::BranchExists(name) => write!(f, "branch {name}가 이미 있습니다"),
            TargetError::PathExists(p) => write!(f, "{}가 이미 있습니다", p.display()),
            TargetError::InvalidPath(p) => {
                write!(f, "새 worktree 경로로 쓸 수 없습니다: {}", p.display())
            }
            TargetError::InsideWorktree { path, worktree } => write!(
                f,
                "새 worktree 경로 {}가 worktree {} 안입니다",
                path.display(),
                worktree.display()
            ),
            TargetError::StartNotFound(oid) => write!(f, "시작 commit {oid}가 없습니다"),
            TargetError::StartWithoutProject(oid) => {
                write!(
                    f,
                    "시작 commit {oid}에 이 프로젝트의 저장된 generation이 없습니다"
                )
            }
            TargetError::Project(e) => write!(f, "target 프로젝트: {e}"),
            TargetError::ProjectMismatch { expected, found } => write!(
                f,
                "target 프로젝트 id {found}가 Merge Composer 프로젝트 {expected}와 다릅니다"
            ),
            TargetError::OperationInProgress(op) => {
                write!(
                    f,
                    "target worktree에 진행 중인 git {}가 있습니다",
                    op.name()
                )
            }
            TargetError::Unmerged(n) => {
                write!(f, "target worktree에 unmerged 경로 {n}개가 있습니다")
            }
            TargetError::Git(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for TargetError {}

fn canonical(path: &Path) -> Result<PathBuf, TargetError> {
    std::fs::canonicalize(path).map_err(|e| TargetError::Location(format!("{e}")))
}

/// Merge Composer 프로젝트의 worktree root와 그 안의 프로젝트 상대 경로.
pub(crate) fn project_location(
    git: &GatedGitRunner,
    project_root: &Path,
) -> Result<(PathBuf, PathBuf), TargetError> {
    let project = canonical(project_root)?;
    let worktree =
        worktree_root(git, &project).map_err(|e| TargetError::Location(e.to_string()))?;
    let worktree = canonical(&worktree)?;
    let relative = project
        .strip_prefix(&worktree)
        .map_err(|_| TargetError::Location("프로젝트가 worktree 밖입니다".into()))?
        .to_path_buf();
    Ok((worktree, relative))
}

/// 프로젝트 상대 경로를 Git 경로 접두(`a/b/`, root면 빈 문자열)로 바꿉니다.
fn git_prefix(relative: &Path) -> Result<String, TargetError> {
    let mut prefix = String::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => {
                let part = part
                    .to_str()
                    .ok_or_else(|| TargetError::InvalidPath(relative.to_path_buf()))?;
                prefix.push_str(part);
                prefix.push('/');
            }
            _ => return Err(TargetError::InvalidPath(relative.to_path_buf())),
        }
    }
    Ok(prefix)
}

/// 열린 target 프로젝트의 id·Git 상태를 확인하고 revision을 돌려줍니다.
pub(crate) fn check_project(
    git: &GatedGitRunner,
    worktree: &Path,
    project_root: &Path,
    project_id: StableId,
) -> Result<ProjectRevision, TargetError> {
    let snapshot = open_snapshot(project_root).map_err(TargetError::Project)?;
    if snapshot.project_id() != project_id {
        return Err(TargetError::ProjectMismatch {
            expected: project_id,
            found: snapshot.project_id(),
        });
    }
    let port = GatedMergeGit::new(git.clone());
    match port.operation(worktree) {
        Ok(GitOperation::None) => {}
        Ok(operation) => return Err(TargetError::OperationInProgress(operation)),
        Err(e) => return Err(TargetError::Git(e.to_string())),
    }
    let unmerged = port
        .unmerged(worktree)
        .map_err(|e| TargetError::Git(e.to_string()))?;
    if !unmerged.is_empty() {
        return Err(TargetError::Unmerged(unmerged.len()));
    }
    Ok(snapshot.revision())
}

/// target을 검증합니다(XXIII.3.1, 모듈 문서 표). `project_root`는 Merge Composer를 연 프로젝트, `project_id`는 그
/// 프로젝트 id입니다. 파일·ref를 만들거나 바꾸지 않습니다.
pub fn validate_target(
    git: &GatedGitRunner,
    project_root: &Path,
    project_id: StableId,
    target: &ResultTarget,
) -> Result<ValidatedTarget, TargetError> {
    let (current, relative) = project_location(git, project_root)?;
    match target {
        ResultTarget::CurrentWorktree => {
            let project = current.join(&relative);
            let expected = check_project(git, &current, &project, project_id)?;
            Ok(ValidatedTarget {
                target: target.clone(),
                worktree: current,
                project_root: project,
                expected: Some(expected),
            })
        }
        ResultTarget::ExistingWorktree { path } => {
            let wanted = canonical(path).map_err(|_| TargetError::NotAWorktree(path.clone()))?;
            let entries =
                list_worktrees(git, &current).map_err(|e| TargetError::Git(e.to_string()))?;
            let found = entries
                .iter()
                .filter(|e| !e.bare && !e.prunable.is_set())
                .any(|e| std::fs::canonicalize(&e.path).is_ok_and(|p| p == wanted));
            if !found {
                return Err(TargetError::NotAWorktree(path.clone()));
            }
            let project = wanted.join(&relative);
            let expected = check_project(git, &wanted, &project, project_id)?;
            Ok(ValidatedTarget {
                target: target.clone(),
                worktree: wanted,
                project_root: project,
                expected: Some(expected),
            })
        }
        ResultTarget::NewBranchWorktree {
            branch,
            path,
            start,
        } => {
            if !is_valid_ref_body(branch) {
                return Err(TargetError::InvalidBranch(branch.clone()));
            }
            match resolve_source(git, &current, &SourceRef::Branch(branch.clone())) {
                Err(SourceError::NotFound(_)) => {}
                Ok(_) | Err(SourceError::NotACommit(_)) => {
                    return Err(TargetError::BranchExists(branch.clone()))
                }
                Err(e) => return Err(TargetError::Git(e.to_string())),
            }
            match resolve_source(git, &current, &SourceRef::Commit(start.clone())) {
                Ok(_) => {}
                Err(SourceError::NotFound(_)) => {
                    return Err(TargetError::StartNotFound(start.clone()))
                }
                Err(e) => return Err(TargetError::Git(e.to_string())),
            }
            let worktree = new_worktree_path(git, &current, path)?;
            let prefix = git_prefix(&relative)?;
            let tree = read_tree_snapshot(git, &current, &SnapshotSource::Commit(start.clone()))
                .map_err(|e| TargetError::Git(e.to_string()))?;
            let has = |name: &str| {
                let wanted = format!("{prefix}{name}");
                tree.entries
                    .iter()
                    .any(|e| e.path.as_bytes() == wanted.as_bytes())
            };
            if !(has("stellar.yaml") && has("design/current")) {
                return Err(TargetError::StartWithoutProject(start.clone()));
            }
            Ok(ValidatedTarget {
                target: target.clone(),
                project_root: worktree.join(&relative),
                worktree,
                expected: None,
            })
        }
    }
}

/// 새 worktree 경로 검사: 아직 없고, 부모가 있으며, 기존 worktree 안이 아닙니다. 정규화한 경로를 돌려줍니다.
fn new_worktree_path(
    git: &GatedGitRunner,
    current: &Path,
    path: &Path,
) -> Result<PathBuf, TargetError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(TargetError::InvalidPath(path.to_path_buf()));
    }
    if std::fs::symlink_metadata(path).is_ok() {
        return Err(TargetError::PathExists(path.to_path_buf()));
    }
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(TargetError::InvalidPath(path.to_path_buf()));
    };
    let parent =
        std::fs::canonicalize(parent).map_err(|_| TargetError::InvalidPath(path.to_path_buf()))?;
    let wanted = parent.join(name);
    let entries = list_worktrees(git, current).map_err(|e| TargetError::Git(e.to_string()))?;
    for entry in entries.iter().filter(|e| !e.bare) {
        if let Ok(root) = std::fs::canonicalize(&entry.path) {
            if wanted.starts_with(&root) {
                return Err(TargetError::InsideWorktree {
                    path: path.to_path_buf(),
                    worktree: root,
                });
            }
        }
    }
    Ok(wanted)
}

/// 검증한 새 branch+worktree target을 시스템 Git으로 만들고(XXIII.3.2), 만든 프로젝트를 열어 revision을 정합니다.
///
/// 명령은 `git worktree add -b <branch> -- <path> <start>` 하나입니다. 다른 target이면 아무것도 하지 않고 그대로
/// 돌려줍니다. 만든 뒤 프로젝트 확인이 실패해도 branch·worktree를 지우지 않습니다(오류가 경로를 알림).
pub fn create_branch_worktree(
    git: &GatedGitRunner,
    project_root: &Path,
    project_id: StableId,
    validated: ValidatedTarget,
) -> Result<ValidatedTarget, TargetError> {
    let ResultTarget::NewBranchWorktree { branch, start, .. } = &validated.target else {
        return Ok(validated);
    };
    let (current, _) = project_location(git, project_root)?;
    let plan = plan_add(
        &validated.worktree,
        &AddTarget::NewBranch {
            name: branch.clone(),
            start: Some(start.as_str().to_owned()),
        },
    )
    .map_err(plan_error)?;
    execute_plan(git, &current, &plan, None).map_err(plan_error)?;
    let expected = check_project(
        git,
        &validated.worktree,
        &validated.project_root,
        project_id,
    )?;
    Ok(ValidatedTarget {
        expected: Some(expected),
        ..validated
    })
}

fn plan_error(e: PlanError) -> TargetError {
    TargetError::Git(e.to_string())
}
