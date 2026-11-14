//! conflict 세 stage virtual project 열기 (XIV.11.2 연결).
//!
//! git-porcelain의 [`materialize_conflict_snapshots`]가 병합되지 않은 index에서 base(stage 1)·
//! ours(stage 2)·theirs(stage 3)를 각각 `.git` 없는 임시 프로젝트 디렉터리로 만들면(conflict 없는 경로는
//! index stage 0과 결합), 이 모듈은 세 디렉터리를 실제 프로젝트 열기 loader
//! ([`SavedProjectLoader`]: `stellar.yaml` → current pointer → manifest → chunk, V.4.4)로 엽니다.
//! side는 사용자가 checkout하지 않은 Git 객체에서 온 비신뢰 입력이라
//! [`SavedProjectLoader::for_materialized_snapshot`]으로 엽니다: `design/` 저장소 경로의 symlink를
//! 따르지 않고(snapshot 밖 파일을 읽지 않음), 정규 파일이 아니거나 크기 상한을 넘는 저장소 파일은 읽기
//! 전에 거부합니다.
//!
//! - side별 열기 결과는 따로 보존합니다. 한 side의 프로젝트가 그 side에서 삭제됐거나 pointer·chunk가
//!   서로 다른 stage에서 와서 검증에 실패해도 다른 side와 raw conflict 목록은 그대로 씁니다(semantic
//!   merge 단계가 판단).
//! - 임시 디렉터리는 소유자 전용 session root([`PrivateTempDir`], XVI.7.5) 아래에 만듭니다. 결과를
//!   drop하면 세 snapshot과 session root가 모두 지워집니다.
//! - 작업 트리와 index는 읽기만 합니다.

use std::fmt;
use std::path::Path;

use stellar_git_porcelain::conflict::UnmergedPath;
use stellar_git_porcelain::conflict_snapshot::{
    materialize_conflict_snapshots, ConflictSide, ConflictSnapshotError, ConflictSnapshots,
};
use stellar_git_porcelain::decode::GitPath;
use stellar_git_porcelain::trust::GatedGitRunner;

use crate::private_temp::{PrivateTempDir, PrivateTempError};
use crate::saved_project::{SavedProject, SavedProjectError, SavedProjectLoader};

/// session root 접두사.
pub const CONFLICT_SESSION_PREFIX: &str = "stellar-conflict";

/// 연 conflict 세 side 프로젝트.
///
/// 필드 순서상 snapshot 디렉터리가 session root보다 먼저 지워집니다.
#[derive(Debug)]
pub struct ConflictProjects {
    /// base(stage 1) 프로젝트 열기 결과.
    pub base: Result<SavedProject, SavedProjectError>,
    /// ours(stage 2) 프로젝트 열기 결과.
    pub ours: Result<SavedProject, SavedProjectError>,
    /// theirs(stage 3) 프로젝트 열기 결과.
    pub theirs: Result<SavedProject, SavedProjectError>,
    snapshots: ConflictSnapshots,
    session: PrivateTempDir,
}

impl ConflictProjects {
    /// side의 열기 결과.
    pub fn side(&self, side: ConflictSide) -> &Result<SavedProject, SavedProjectError> {
        match side {
            ConflictSide::Base => &self.base,
            ConflictSide::Ours => &self.ours,
            ConflictSide::Theirs => &self.theirs,
        }
    }

    /// side의 virtual project root(`.git` 없음).
    pub fn root(&self, side: ConflictSide) -> &Path {
        self.snapshots.side(side).root()
    }

    /// 세 snapshot을 담은 소유자 전용 session root.
    pub fn session_root(&self) -> &Path {
        self.session.path()
    }

    /// semantic 대상 Stellar conflict 경로(Git index 순서).
    pub fn stellar_conflicts(&self) -> &[GitPath] {
        &self.snapshots.stellar_conflicts
    }

    /// non-Stellar conflict(raw Git conflict로 표시).
    pub fn raw_conflicts(&self) -> &[UnmergedPath] {
        &self.snapshots.raw_conflicts
    }
}

/// conflict 프로젝트 열기 실패(side별 프로젝트 열기 실패는 결과에 보존).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictOpenError {
    /// 비공개 session root를 만들지 못함(Git 미실행).
    Session(PrivateTempError),
    /// index 읽기 또는 side materialize 실패(만든 디렉터리는 정리됨).
    Snapshot(ConflictSnapshotError),
}

impl fmt::Display for ConflictOpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConflictOpenError::Session(e) => write!(f, "conflict session 디렉터리 실패: {e}"),
            ConflictOpenError::Snapshot(e) => write!(f, "conflict snapshot 실패: {e}"),
        }
    }
}

impl std::error::Error for ConflictOpenError {}

/// 현재 conflict 상태의 base·ours·theirs를 `temp_parent` 아래 비공개 session에 materialize하고 실제
/// 프로젝트 loader로 엽니다.
pub fn open_conflict_projects(
    git: &GatedGitRunner,
    worktree: &Path,
    temp_parent: &Path,
) -> Result<ConflictProjects, ConflictOpenError> {
    let session = PrivateTempDir::create_in(temp_parent, CONFLICT_SESSION_PREFIX)
        .map_err(ConflictOpenError::Session)?;
    let snapshots = materialize_conflict_snapshots(git, worktree, session.path())
        .map_err(ConflictOpenError::Snapshot)?;
    // side는 checkout하지 않은 Git 객체에서 온 비신뢰 입력이라 저장소 경로의 symlink를 따르지 않습니다.
    let loader = SavedProjectLoader::for_materialized_snapshot();
    let open = |side: ConflictSide| loader.open(snapshots.side(side).root());
    Ok(ConflictProjects {
        base: open(ConflictSide::Base),
        ours: open(ConflictSide::Ours),
        theirs: open(ConflictSide::Theirs),
        snapshots,
        session,
    })
}
