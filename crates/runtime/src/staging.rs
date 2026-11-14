//! 실제 index → semantic 선택 → 임시 저장 후보 → 기존 loader 재열기(XIV.5.5).
//! 동기식 준비 작업이며 호출자가 기존 worker 실행 문맥을 선택합니다. index·작업 트리는 읽기만 합니다.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

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
    materialize_snapshot, read_index_snapshot, MaterializeError,
};
use stellar_git_porcelain::trust::GatedGitRunner;
use stellar_ops::candidate::{
    build_stage_candidate, saved_semantic_snapshot, serialize_candidate, CandidateIndexSnapshot,
    CandidateStorageError, StageError, StageSelectionRequest,
};
use stellar_project::saved_project::{SavedProjectError, SavedProjectLoader};

/// 저장 파일 재열기까지 검증된 후보. drop하면 임시 tree가 삭제됩니다.
#[derive(Debug)]
pub struct PreparedStageCandidate {
    /// 선택 적용 결과.
    pub semantic: CandidateIndexSnapshot,
    /// 기존 index의 파일을 보존한 후보.
    pub tree: CandidateTree,
}

/// 선택 staging 준비 실패. 실제 index에는 어떤 변경도 쓰지 않습니다.
#[derive(Debug)]
pub enum PrepareStageError {
    /// Git index 읽기 실패(신뢰 거부 포함).
    Git(GitOpError),
    /// index에 conflict가 있습니다.
    Conflicted,
    /// 기준 index materialize 실패.
    Materialize(MaterializeError),
    /// 저장 프로젝트 열기 실패.
    Open(SavedProjectError),
    /// 선택 또는 필수 의존 승인 실패.
    Selection(StageError),
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
            Self::Materialize(e) => write!(f, "{e}"),
            Self::Open(e) => write!(f, "{e}"),
            Self::Selection(e) => write!(f, "{e}"),
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

/// 현재 실제 index와 저장된 working generation에서 선택 후보를 준비합니다.
/// `parent`는 작업 트리 밖의 호출자 소유 임시 디렉터리여야 합니다.
pub fn prepare_stage_candidate(
    git: &GatedGitRunner,
    worktree: &Path,
    parent: &Path,
    request: &StageSelectionRequest,
) -> Result<PreparedStageCandidate, PrepareStageError> {
    let index = read_index_snapshot(git, worktree).map_err(PrepareStageError::Git)?;
    if index.has_conflicts() {
        return Err(PrepareStageError::Conflicted);
    }
    let canonical_worktree = worktree
        .canonicalize()
        .map_err(|e| PrepareStageError::TempParent(e.kind()))?;
    let canonical_parent = parent
        .canonicalize()
        .map_err(|e| PrepareStageError::TempParent(e.kind()))?;
    if canonical_parent.starts_with(&canonical_worktree) {
        return Err(PrepareStageError::Storage(
            CandidateStorageError::Unsupported("임시 디렉터리는 작업 트리 밖이어야 합니다"),
        ));
    }
    let base_tree = materialize_snapshot(git, worktree, &index.entries, &canonical_parent)
        .map_err(PrepareStageError::Materialize)?;
    let loader = SavedProjectLoader::for_materialized_snapshot();
    let base = loader
        .open(base_tree.root())
        .map_err(PrepareStageError::Open)?;
    let working = SavedProjectLoader::new()
        .open(worktree)
        .map_err(PrepareStageError::Open)?;
    if base.project_id != working.project_id {
        return Err(PrepareStageError::Storage(
            CandidateStorageError::Unsupported("서로 다른 project id"),
        ));
    }
    let base_semantic = saved_semantic_snapshot(&base).map_err(PrepareStageError::Storage)?;
    let working_semantic = saved_semantic_snapshot(&working).map_err(PrepareStageError::Storage)?;
    let semantic = build_stage_candidate(&base_semantic, &working_semantic, request)
        .map_err(PrepareStageError::Selection)?;
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
    Ok(PreparedStageCandidate { semantic, tree })
}

#[cfg(all(test, unix))]
mod tests;
