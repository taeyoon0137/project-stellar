//! 실제 index → semantic 선택 → 임시 저장 후보 → 기존 loader 재열기(XIV.5.5).
//! 동기식 API이며 호출자가 기존 worker 실행 문맥을 선택합니다. prepare는 읽기 전용이고,
//! apply는 gated Git으로 index만 적용·검증·조건부 복원합니다. 작업 트리·문서는 쓰지 않습니다.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use stellar_git_porcelain::candidate_tree::{
    write_candidate_tree, CandidateFile, CandidateTree, CandidateTreeError,
};
use stellar_git_porcelain::classify::{classify_project_path, ProjectFileClass};
use stellar_git_porcelain::decode::GitPath;
use stellar_git_porcelain::object::FileModeKind;
use stellar_git_porcelain::ops::GitOpError;
use stellar_git_porcelain::snapshot::working_tree::{
    read_working_tree_file, WorkingTreeFile, WorkingTreeFileKind,
};
use stellar_git_porcelain::snapshot::{
    materialize_snapshot, read_index_snapshot, read_tree_snapshot, resolve_tree, MaterializeError,
    ResolvedTree, SnapshotError, SnapshotSource,
};
use stellar_git_porcelain::trust::GatedGitRunner;
use stellar_ops::candidate::{
    build_stage_candidate, build_unstage_candidate, saved_semantic_snapshot, serialize_candidate,
    CandidateIndexSnapshot, CandidateStorageError, SelectionOperation, SemanticDiff, StageError,
    StageSelectionRequest,
};
use stellar_project::saved_project::{SavedProjectError, SavedProjectLoader};

/// 저장 파일 재열기까지 검증된 후보. drop하면 임시 tree가 삭제됩니다.
#[derive(Debug)]
pub struct PreparedStageCandidate {
    /// 선택 적용 결과.
    pub semantic: CandidateIndexSnapshot,
    /// 기존 index의 파일을 보존한 후보.
    pub tree: CandidateTree,
    // Unstage가 기준으로 삼은 HEAD. 기록 직전과 결과 재검증에서도 확인합니다.
    head: Option<ResolvedTree>,
}

/// 선택 staging 준비 실패. 실제 index에는 어떤 변경도 쓰지 않습니다.
#[derive(Debug)]
pub enum PrepareStageError {
    /// Git index 읽기 실패(신뢰 거부 포함).
    Git(GitOpError),
    /// Unstage의 기준 HEAD를 해석하지 못했습니다.
    Head(SnapshotError),
    /// 기준 HEAD가 준비 이후 바뀌었습니다.
    HeadChanged,
    /// Git 저장소 위치 판별 실패.
    Location(stellar_git_porcelain::location::LocationError),
    /// index에 conflict가 있습니다.
    Conflicted,
    /// 기준 index materialize 실패.
    Materialize(MaterializeError),
    /// 저장 프로젝트 열기 실패.
    Open(SavedProjectError),
    /// 선택 또는 필수 의존 승인 실패.
    Selection(StageError),
    /// 화면에 표시한 선택 변경과 현재 Stage/Unstage 원본 값이 다릅니다.
    StalePreview,
    /// 무손실 검사·저장 직렬화 실패.
    Storage(CandidateStorageError),
    /// 후보 writer 실패.
    Write(CandidateTreeError),
    /// 준비 중 index가 바뀌었습니다.
    IndexChanged,
    /// 재열기 결과가 후보와 다릅니다.
    ReopenMismatch,
    /// 임시 디렉터리 경로 확인 실패.
    TempParent(std::io::ErrorKind),
    /// 기존 immutable 파일을 비교하지 못했습니다.
    Preserve(std::io::ErrorKind),
}

impl fmt::Display for PrepareStageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git(e) => write!(f, "{e}"),
            Self::Head(e) => write!(f, "{e}"),
            Self::HeadChanged => f.write_str("Unstage 기준 HEAD가 바뀌었습니다. 새로고침하세요"),
            Self::Location(e) => write!(f, "{e}"),
            Self::Materialize(e) => write!(f, "{e}"),
            Self::Open(e) => write!(f, "{e}"),
            Self::Selection(e) => write!(f, "{e}"),
            Self::StalePreview => {
                f.write_str("표시한 변경이 바뀌었습니다. 선택을 새로 확인해야 합니다")
            }
            Self::Storage(e) => write!(f, "{e}"),
            Self::Write(e) => write!(f, "{e}"),
            Self::Conflicted => f.write_str("index conflict가 있습니다"),
            Self::IndexChanged => f.write_str("준비 중 index가 바뀌었습니다"),
            Self::ReopenMismatch => f.write_str("후보 파일의 재열기 내용이 일치하지 않습니다"),
            Self::TempParent(kind) => write!(f, "임시 디렉터리 확인 실패: {kind:?}"),
            Self::Preserve(kind) => write!(f, "기존 파일 보존 검사 실패: {kind:?}"),
        }
    }
}

impl std::error::Error for PrepareStageError {}

fn checked_temp_parent(
    git: &GatedGitRunner,
    worktree: &Path,
    parent: &Path,
) -> Result<PathBuf, PrepareStageError> {
    let canonical_worktree = worktree
        .canonicalize()
        .map_err(|e| PrepareStageError::TempParent(e.kind()))?;
    let location = stellar_git_porcelain::location::locate_repository(git, worktree)
        .map_err(PrepareStageError::Location)?;
    if canonical_worktree != location.worktree_root {
        return Err(PrepareStageError::Storage(
            CandidateStorageError::Unsupported("worktree root 경로가 필요합니다"),
        ));
    }
    let canonical_parent = parent
        .canonicalize()
        .map_err(|e| PrepareStageError::TempParent(e.kind()))?;
    if canonical_parent.starts_with(&canonical_worktree) {
        return Err(PrepareStageError::Storage(
            CandidateStorageError::Unsupported("임시 디렉터리는 작업 트리 밖이어야 합니다"),
        ));
    }
    Ok(canonical_parent)
}

/// 현재 실제 index와 저장된 working generation에서 선택 후보를 준비합니다.
/// `parent`는 작업 트리 밖의 호출자 소유 임시 디렉터리여야 합니다.
pub fn prepare_stage_candidate(
    git: &GatedGitRunner,
    worktree: &Path,
    parent: &Path,
    request: &StageSelectionRequest,
) -> Result<PreparedStageCandidate, PrepareStageError> {
    prepare_stage_candidate_with_preview(git, worktree, parent, request, None)
}

/// 선택 화면에서 관찰한 변경을 현재 저장 상태와 대조한 뒤 후보를 준비합니다.
/// `preview`에는 사용자가 선택한 변경과 함께 승인한 필수 변경만 canonical diff로 담습니다.
/// ID가 같아도 변경 내용이 달라지면 `StalePreview`로 거부합니다. 다른 속성의 독립 변경은 허용합니다.
/// `None`은 기존 [`prepare_stage_candidate`]처럼 현재 상태에서 ID만 해석합니다.
pub fn prepare_stage_candidate_with_preview(
    git: &GatedGitRunner,
    worktree: &Path,
    parent: &Path,
    request: &StageSelectionRequest,
    preview: Option<&SemanticDiff>,
) -> Result<PreparedStageCandidate, PrepareStageError> {
    if request.operation != SelectionOperation::Stage {
        return Err(PrepareStageError::Selection(StageError::WrongOperation {
            expected: SelectionOperation::Stage,
            found: request.operation,
        }));
    }
    prepare_selection(git, worktree, parent, request, preview)
}

/// HEAD → Index에 표시한 변경을 현재 Index에서 역적용할 후보를 만듭니다.
/// 비교 원본은 HEAD와 Index입니다. 선택 화면의 diff가 있으면 값과 승인한 필수 변경까지 비교합니다.
pub fn prepare_unstage_candidate(
    git: &GatedGitRunner,
    worktree: &Path,
    parent: &Path,
    request: &StageSelectionRequest,
    preview: Option<&SemanticDiff>,
) -> Result<PreparedStageCandidate, PrepareStageError> {
    if request.operation != SelectionOperation::Unstage {
        return Err(PrepareStageError::Selection(StageError::WrongOperation {
            expected: SelectionOperation::Unstage,
            found: request.operation,
        }));
    }
    prepare_selection(git, worktree, parent, request, preview)
}

fn check_head(
    git: &GatedGitRunner,
    worktree: &Path,
    head: Option<&ResolvedTree>,
) -> Result<(), PrepareStageError> {
    if let Some(expected) = head {
        let actual =
            resolve_tree(git, worktree, &SnapshotSource::Head).map_err(PrepareStageError::Head)?;
        if &actual != expected {
            return Err(PrepareStageError::HeadChanged);
        }
    }
    Ok(())
}

fn prepare_selection(
    git: &GatedGitRunner,
    worktree: &Path,
    parent: &Path,
    request: &StageSelectionRequest,
    preview: Option<&SemanticDiff>,
) -> Result<PreparedStageCandidate, PrepareStageError> {
    let index = read_index_snapshot(git, worktree).map_err(PrepareStageError::Git)?;
    if index.has_conflicts() {
        return Err(PrepareStageError::Conflicted);
    }
    let canonical_parent = checked_temp_parent(git, worktree, parent)?;
    let base_tree = materialize_snapshot(git, worktree, &index.entries, &canonical_parent)
        .map_err(PrepareStageError::Materialize)?;
    let loader = SavedProjectLoader::for_materialized_snapshot();
    let base = loader
        .open(base_tree.root())
        .map_err(PrepareStageError::Open)?;
    let indexed = saved_semantic_snapshot(&base).map_err(PrepareStageError::Storage)?;
    let (semantic, head) = match request.operation {
        SelectionOperation::Stage => {
            let working = SavedProjectLoader::new()
                .open(worktree)
                .map_err(PrepareStageError::Open)?;
            if working.project_id != base.project_id {
                return Err(PrepareStageError::Storage(
                    CandidateStorageError::Unsupported("서로 다른 project id"),
                ));
            }
            let working = saved_semantic_snapshot(&working).map_err(PrepareStageError::Storage)?;
            let candidate = build_stage_candidate(&indexed, &working, request)
                .map_err(PrepareStageError::Selection)?;
            if preview.is_some_and(|shown| shown != candidate.applied()) {
                return Err(PrepareStageError::StalePreview);
            }
            (candidate, None)
        }
        SelectionOperation::Unstage => {
            let head = read_tree_snapshot(git, worktree, &SnapshotSource::Head)
                .map_err(PrepareStageError::Head)?;
            if head.resolved == ResolvedTree::Unborn {
                return Err(PrepareStageError::Storage(
                    CandidateStorageError::Unsupported(
                        "Unstage 기준으로 커밋된 프로젝트가 필요합니다",
                    ),
                ));
            }
            let tree = materialize_snapshot(git, worktree, &head.entries, &canonical_parent)
                .map_err(PrepareStageError::Materialize)?;
            let committed = loader.open(tree.root()).map_err(PrepareStageError::Open)?;
            if committed.project_id != base.project_id {
                return Err(PrepareStageError::Storage(
                    CandidateStorageError::Unsupported("서로 다른 project id"),
                ));
            }
            let committed =
                saved_semantic_snapshot(&committed).map_err(PrepareStageError::Storage)?;
            let candidate = build_unstage_candidate(&committed, &indexed, request)
                .map_err(PrepareStageError::Selection)?;
            if preview.is_some_and(|shown| shown.inverse() != *candidate.applied()) {
                return Err(PrepareStageError::StalePreview);
            }
            (candidate, Some(head.resolved))
        }
    };
    let bytes = serialize_candidate(&base, &semantic).map_err(PrepareStageError::Storage)?;
    let index_by_path: BTreeMap<_, _> = index
        .entries
        .iter()
        .map(|entry| (&entry.path, entry))
        .collect();
    let mut files: BTreeMap<GitPath, CandidateFile> = index
        .entries
        .iter()
        .filter(|e| classify_project_path(e.path.as_bytes()) != ProjectFileClass::NonStellar)
        .map(|e| {
            (
                e.path.clone(),
                CandidateFile::KeepIndex {
                    path: e.path.clone(),
                },
            )
        })
        .collect();
    for (path, bytes) in bytes {
        // serializer 경로는 storage가 만든 ASCII 상대 경로뿐입니다.
        let path = GitPath::from_bytes(path.to_string_lossy().replace('\\', "/").as_bytes());
        let existing = index_by_path.get(&path);
        if existing.is_some() && path.as_bytes() != b"design/current" {
            let kind = match existing.and_then(|entry| entry.kind()) {
                Some(FileModeKind::Regular) => WorkingTreeFileKind::Regular,
                Some(FileModeKind::Executable) => WorkingTreeFileKind::Executable,
                _ => {
                    return Err(PrepareStageError::Storage(
                        CandidateStorageError::Unsupported("immutable 경로의 파일 종류"),
                    ))
                }
            };
            let existing_bytes = read_working_tree_file(
                base_tree.root(),
                &WorkingTreeFile {
                    path: path.clone(),
                    kind,
                    len: bytes.len() as u64,
                },
            )
            .map_err(PrepareStageError::Preserve)?;
            if existing_bytes != bytes {
                return Err(PrepareStageError::Storage(
                    CandidateStorageError::Unsupported(
                        "기존 immutable object 또는 generation과 후보 bytes 충돌",
                    ),
                ));
            }
            continue;
        }
        let kind = existing
            .and_then(|entry| entry.kind())
            .unwrap_or(FileModeKind::Regular);
        files.insert(path.clone(), CandidateFile::Bytes { path, kind, bytes });
    }
    if read_index_snapshot(git, worktree).map_err(PrepareStageError::Git)? != index {
        return Err(PrepareStageError::IndexChanged);
    }
    let tree = write_candidate_tree(
        git,
        worktree,
        &canonical_parent,
        &files.into_values().collect::<Vec<_>>(),
    )
    .map_err(PrepareStageError::Write)?;
    let reopened = loader.open(tree.root()).map_err(PrepareStageError::Open)?;
    if saved_semantic_snapshot(&reopened).map_err(PrepareStageError::Storage)?
        != *semantic.snapshot()
    {
        return Err(PrepareStageError::ReopenMismatch);
    }
    if read_index_snapshot(git, worktree).map_err(PrepareStageError::Git)? != index {
        return Err(PrepareStageError::IndexChanged);
    }
    check_head(git, worktree, head.as_ref())?;
    Ok(PreparedStageCandidate {
        semantic,
        tree,
        head,
    })
}

/// 실제 stage 준비·적용·검증 실패입니다.
#[derive(Debug)]
pub enum ApplyStageError {
    /// 임시 경로 확인 실패. index 적용 전입니다.
    Preparation(PrepareStageError),
    /// 기존 candidate file delta 계산 실패.
    Delta(stellar_git_porcelain::candidate_delta::CandidateDeltaError),
    /// blob 기록 실패. 미참조 object는 Git GC에 맡깁니다.
    Blob(GitOpError),
    /// 적용 실패 또는 결과 미확정. 복원 증표가 없어 역쓰기하지 않습니다.
    Publish(stellar_git_porcelain::index_publish::PublishError),
    /// 실제 Index provider의 semantic 재검증 실패와 조건부 복원 결과.
    Verification {
        /// 검증 실패 원인.
        cause: Box<PrepareStageError>,
        /// Ok는 원래 index tree 복원, Err는 경합·취소·결과 미확정 진단입니다.
        rollback: Result<(), stellar_git_porcelain::index_publish::PublishError>,
    },
}

impl fmt::Display for ApplyStageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Preparation(e) => write!(f, "{e}"),
            Self::Delta(e) => write!(f, "{e}"),
            Self::Blob(e) => write!(f, "{e}"),
            Self::Publish(e) => write!(f, "{e}"),
            Self::Verification { cause, rollback } => {
                write!(f, "실제 index 검증 실패: {cause}; ")?;
                match rollback {
                    Ok(()) => f.write_str("원래 index tree를 복원했습니다"),
                    Err(error) => write!(f, "복원 미확정: {error}"),
                }
            }
        }
    }
}
impl std::error::Error for ApplyStageError {}

/// 준비된 Stage 또는 Unstage 후보를 적용하고 실제 Index provider로 다시 열어 검증합니다.
/// 실패한 검증은 성공 증표가 여전히 현재 index와 일치할 때만 복원합니다.
/// `parent`는 작업 트리 밖이어야 합니다. 문서 상태를 받거나 수정하지 않습니다.
/// 기존 delta/blob 준비는 각 Git 명령 timeout을 따르며 취소를 단계 사이에서 관찰합니다.
pub fn apply_stage_candidate(
    git: &GatedGitRunner,
    worktree: &Path,
    parent: &Path,
    prepared: &PreparedStageCandidate,
    cancel: &stellar_git_porcelain::cancel::GitCancelToken,
) -> Result<stellar_git_porcelain::snapshot::IndexSnapshot, ApplyStageError> {
    use stellar_git_porcelain::blob_write::write_candidate_blobs;
    use stellar_git_porcelain::candidate_delta::compute_candidate_delta;
    use stellar_git_porcelain::index_publish::{publish_isolated, restore_published, PublishError};
    use stellar_git_porcelain::snapshot::index::read_index_snapshot_cancellable;
    let check_cancel = || {
        if cancel.is_cancelled() {
            Err(ApplyStageError::Publish(PublishError::Git(
                GitOpError::Gate(stellar_git_porcelain::trust::GatedRunError::Run(
                    stellar_git_porcelain::runner::GitRunError::Cancelled,
                )),
            )))
        } else {
            Ok(())
        }
    };
    git.runner()
        .map_err(|e| ApplyStageError::Preparation(PrepareStageError::Git(e.into())))?;
    check_cancel()?;
    let parent =
        checked_temp_parent(git, worktree, parent).map_err(ApplyStageError::Preparation)?;
    check_head(git, worktree, prepared.head.as_ref()).map_err(ApplyStageError::Preparation)?;
    let delta =
        compute_candidate_delta(git, worktree, &prepared.tree).map_err(ApplyStageError::Delta)?;
    check_cancel()?;
    let written = write_candidate_blobs(git, worktree, &delta).map_err(ApplyStageError::Blob)?;
    check_cancel()?;
    check_head(git, worktree, prepared.head.as_ref()).map_err(ApplyStageError::Preparation)?;
    let receipt = publish_isolated(git, worktree, &parent, &written, cancel)
        .map_err(ApplyStageError::Publish)?;
    let verify = || -> Result<(), PrepareStageError> {
        check_head(git, worktree, prepared.head.as_ref())?;
        let actual = read_index_snapshot_cancellable(git, worktree, cancel)
            .map_err(PrepareStageError::Git)?;
        if &actual != receipt.snapshot() {
            return Err(PrepareStageError::IndexChanged);
        }
        let tree = materialize_snapshot(git, worktree, &actual.entries, &parent)
            .map_err(PrepareStageError::Materialize)?;
        let loaded = SavedProjectLoader::for_materialized_snapshot()
            .open(tree.root())
            .map_err(PrepareStageError::Open)?;
        if saved_semantic_snapshot(&loaded).map_err(PrepareStageError::Storage)?
            != *prepared.semantic.snapshot()
        {
            return Err(PrepareStageError::ReopenMismatch);
        }
        if read_index_snapshot_cancellable(git, worktree, cancel).map_err(PrepareStageError::Git)?
            != actual
        {
            return Err(PrepareStageError::IndexChanged);
        }
        check_head(git, worktree, prepared.head.as_ref())
    };
    if let Err(cause) = verify() {
        let rollback = restore_published(git, receipt, cancel).map(|_| ());
        return Err(ApplyStageError::Verification {
            cause: Box::new(cause),
            rollback,
        });
    }
    Ok(receipt.snapshot().clone())
}

#[cfg(all(test, unix))]
mod tests;
