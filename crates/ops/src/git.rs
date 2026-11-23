//! 앱 없이 쓰는 Git 엔진 진입점(ADR-0080).
//!
//! 같은 구현을 `stellar git` CLI, 앱 runtime worker, 테스트가 함께 씁니다. 모든 Git 실행은
//! [`GatedGitRunner`]를 거치므로 신뢰하지 않은 프로젝트에서는 프로세스를 하나도 만들지 않습니다(XVI.7.3).
//! Stellar는 hook·filter 파일을 직접 실행하지 않습니다. hook은 시스템 Git이 `git commit` 안에서 실행합니다.
//!
//! | 기능 | 함수 | 결정 |
//! |---|---|---|
//! | 상태 조회 | [`GitSession::status`] | HEAD·upstream·remote·identity·원본 단위·semantic 변경 page. 계산 revision을 붙입니다 |
//! | 선택 stage·unstage | [`GitSession::select`] | 변경 ID와 원본 단위(View generation 묶음·파일). 필수 동반 변경은 자동 포함하지 않고 목록으로 돌려줍니다 |
//! | 재검증 | [`GitSession::validate_staged`] | staged snapshot을 전체 validator로 다시 검사합니다 |
//! | commit | [`GitSession::commit`] | trusted에서만, 재검증 통과 시에만 `git commit`을 실행하고 뒤에 HEAD·index·status를 다시 읽습니다. push하지 않습니다 |
//!
//! push·force push·remote 편집 API는 이 모듈에 없습니다(push는 시스템 Git에 맡깁니다).
//!
//! 선택 staging 파이프라인(실제 index → semantic 선택 → 임시 저장 후보 → 기존 loader 재열기 → index 기록 →
//! 재검증, XIV.5.5~XIV.7.3)도 이 모듈에 있습니다. 앱 runtime은 `stellar_runtime::staging`에서 같은 함수를
//! 재노출합니다. 작업 트리와 문서는 쓰지 않습니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use stellar_document::records::{DocumentRecords, NodePayload};
use stellar_git_porcelain::blob_write::write_candidate_blobs;
use stellar_git_porcelain::cancel::GitCancelToken;
pub use stellar_git_porcelain::cancel::GitCancelToken as CancelToken;
use stellar_git_porcelain::candidate_delta::{compute_candidate_delta, CandidateDeltaError};
use stellar_git_porcelain::candidate_tree::{
    write_candidate_tree, CandidateFile, CandidateTree, CandidateTreeError,
};
use stellar_git_porcelain::classify::{
    classify_project_path, partition_status, ProjectFileClass, RawFileStatus, StellarFileKind,
};
pub use stellar_git_porcelain::commit::CommitError as GitCommitError;
use stellar_git_porcelain::commit::{commit_with_message, CommitError};
use stellar_git_porcelain::decode::GitPath;
use stellar_git_porcelain::discovery::{discover_git, GitDiscoveryConfig};
use stellar_git_porcelain::file_status::{stellar_file_statuses, FileChange, StellarFileStatus};
pub use stellar_git_porcelain::identity::IdentityKey;
use stellar_git_porcelain::identity::{
    resolve_commit_identity, IdentityReadError, MissingIdentity,
};
use stellar_git_porcelain::index_publish::{publish_with_receipt, restore_published, PublishError};
pub use stellar_git_porcelain::location::WorktreeKind;
use stellar_git_porcelain::location::{locate_repository, LocationError, RepositoryLocation};
use stellar_git_porcelain::object::{FileModeKind, ObjectId};
use stellar_git_porcelain::ops::GitOpError;
use stellar_git_porcelain::remote::list_remotes;
use stellar_git_porcelain::repo_status::{
    read_in_progress_operations, read_repository_status, RepositoryStatus,
};
pub use stellar_git_porcelain::repo_status::{
    HeadState, RepositoryStatus as GitRepositoryStatus, UpstreamState,
};
use stellar_git_porcelain::runner::{GitRunError, GitRunner};
use stellar_git_porcelain::snapshot::index::read_index_snapshot_cancellable;
use stellar_git_porcelain::snapshot::working_tree::{
    list_working_tree_files, read_working_tree_file, WorkingTreeFile, WorkingTreeFileKind,
};
use stellar_git_porcelain::snapshot::{
    materialize_snapshot, read_index_snapshot, read_tree_snapshot, resolve_tree, IndexSnapshot,
    MaterializeError, ResolvedTree, SnapshotEntry, SnapshotError, SnapshotSource,
};
use stellar_git_porcelain::status::{read_status, StatusOptions};
use stellar_git_porcelain::trust::{GatedGitRunner, GatedRunError, GitTrust};
use stellar_project::document_io::component_source::read_project_library;
use stellar_project::path_resolver::ExternalAccess;
use stellar_project::private_temp::PrivateTempDir;
use stellar_project::saved_project::{SavedProject, SavedProjectError, SavedProjectLoader};
use stellar_semantic_diff::change::SemanticChange;
pub use stellar_semantic_diff::change::SemanticChangeKind;
use stellar_semantic_diff::codec::Reader;
use stellar_semantic_diff::status::SemanticStatus;
pub use stellar_semantic_diff::status::StatusArea;
use stellar_storage::hash::ContentHash;

use crate::candidate::{
    build_stage_candidate, build_unstage_candidate, saved_semantic_snapshot, serialize_candidate,
    CandidateIndexSnapshot, CandidateStorageError, ChangeId, SelectionOperation, SemanticDiff,
    SemanticSnapshot, StageError, StageSelectionRequest,
};
use crate::core_types::ids::StableId;
use crate::core_types::property::PropertyPath;

// ───────────────────────────── 실행 환경 ─────────────────────────────

/// 시스템 Git에 전달하는 환경 변수 allowlist. 앱 runtime의 프로젝트 Git 실행과 같은 목록입니다.
/// identity 환경 변수는 Git과 같은 identity 해석을 위해 전달합니다(XIV.8.3).
pub const SYSTEM_GIT_ENV: [&str; 11] = [
    "HOME",
    "XDG_CONFIG_HOME",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_SYSTEM",
    "GIT_CONFIG_NOSYSTEM",
    "PATH",
    "GIT_AUTHOR_NAME",
    "GIT_AUTHOR_EMAIL",
    "GIT_COMMITTER_NAME",
    "GIT_COMMITTER_EMAIL",
    "EMAIL",
];

/// 시스템 Git 명령 하나의 제한 시간.
pub const SYSTEM_GIT_TIMEOUT: Duration = Duration::from_secs(60);

/// 시스템 Git을 찾아 실행기를 만듭니다. 프로세스를 만들지 않습니다(파일 존재만 확인).
pub fn system_git_runner() -> Result<GitRunner, GitError> {
    let git = discover_git(&GitDiscoveryConfig::default()).ok_or(GitError::GitUnavailable)?;
    Ok(GitRunner::new(
        git,
        SYSTEM_GIT_ENV.iter().map(Into::into).collect(),
        SYSTEM_GIT_TIMEOUT,
    ))
}

/// OS 사용자 데이터의 신뢰 등록으로 `root`의 Git 신뢰 상태를 정합니다. 등록은 실제 경로 단위라 linked
/// worktree는 별도 project identity로 신뢰해야 합니다(ADR-0080 결정 6). 파일을 쓰지 않습니다.
pub fn os_git_trust(root: &Path) -> GitTrust {
    match crate::trusted::TrustedProject::select(root) {
        Ok(_) => GitTrust::Trusted,
        Err(_) => GitTrust::NotTrusted,
    }
}

// ───────────────────────────── 원본 단위·변경 ID ─────────────────────────────

/// Git index에 한 번에 포함하거나 빼는 원본 단위(UX.5.4).
///
/// View 원본이 아직 generation(`design/current`·`design/generations/`·`design/objects/`)인 동안 모든 View는
/// [`SourceUnit::Generation`] 묶음 하나입니다. 그 밖의 Stellar 파일(컴포넌트 YAML, `stellar.yaml`,
/// `stellar.lock.yaml`, asset)은 파일 하나가 단위입니다.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceUnit {
    /// `design/components/` 밖의 `design/` 전체(View generation 묶음).
    Generation,
    /// Stellar 파일 하나(저장소 root 기준 경로).
    File(GitPath),
}

/// 컴포넌트 텍스트 원본 폴더.
pub const COMPONENTS_DIR: &str = "design/components/";

impl SourceUnit {
    /// 경로가 속한 원본 단위. non-Stellar 파일은 `None`입니다(raw Git status만).
    pub fn of_path(path: &[u8]) -> Option<SourceUnit> {
        match classify_project_path(path) {
            ProjectFileClass::NonStellar => None,
            ProjectFileClass::Stellar(StellarFileKind::Design)
                if !path.starts_with(COMPONENTS_DIR.as_bytes()) =>
            {
                Some(SourceUnit::Generation)
            }
            ProjectFileClass::Stellar(_) => Some(SourceUnit::File(GitPath::from_bytes(path))),
        }
    }

    /// 경로가 이 단위에 속하는지.
    pub fn contains(&self, path: &[u8]) -> bool {
        SourceUnit::of_path(path).as_ref() == Some(self)
    }

    /// 단위가 Stellar 원본을 가리키는지(파일 단위가 generation 경로나 non-Stellar 경로면 거짓).
    pub fn is_well_formed(&self) -> bool {
        match self {
            SourceUnit::Generation => true,
            SourceUnit::File(path) => SourceUnit::of_path(path.as_bytes()).as_ref() == Some(self),
        }
    }
}

impl fmt::Display for SourceUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceUnit::Generation => f.write_str("generation"),
            SourceUnit::File(path) => match std::str::from_utf8(path.as_bytes()) {
                Ok(text) => write!(f, "file:{text}"),
                Err(_) => write!(f, "file-hex:{}", hex(path.as_bytes())),
            },
        }
    }
}

/// stage·unstage 대상 하나: semantic 변경 또는 원본 단위.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GitItem {
    /// semantic status의 변경 하나(XIV.5.1 [`ChangeId`]).
    Change(ChangeId),
    /// 원본 단위.
    Unit(SourceUnit),
}

/// 대상 ID 해석 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemParseError(pub String);

impl fmt::Display for ItemParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "대상 ID 형식 오류: {}", self.0)
    }
}

impl std::error::Error for ItemParseError {}

/// 변경 ID의 텍스트 형식: `change:` + canonical bytes 16진수. 값은 담지 않습니다.
pub fn change_id_text(id: &ChangeId) -> String {
    let mut bytes = Vec::new();
    id.write_canonical(&mut bytes);
    format!("change:{}", hex(&bytes))
}

impl fmt::Display for GitItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GitItem::Change(id) => f.write_str(&change_id_text(id)),
            GitItem::Unit(unit) => write!(f, "{unit}"),
        }
    }
}

impl std::str::FromStr for GitItem {
    type Err = ItemParseError;

    /// `change:<hex>`, `generation`, `file:<경로>`, `file-hex:<hex>`.
    fn from_str(text: &str) -> Result<GitItem, ItemParseError> {
        let bad = || ItemParseError(text.to_string());
        if text == "generation" {
            return Ok(GitItem::Unit(SourceUnit::Generation));
        }
        let unit = |raw: Vec<u8>| {
            let unit = SourceUnit::File(GitPath::from_bytes(&raw));
            if unit.is_well_formed() {
                Ok(GitItem::Unit(unit))
            } else {
                Err(bad())
            }
        };
        if let Some(path) = text.strip_prefix("file:") {
            return unit(path.as_bytes().to_vec());
        }
        if let Some(raw) = text.strip_prefix("file-hex:") {
            return unit(unhex(raw).ok_or_else(bad)?);
        }
        let raw = text
            .strip_prefix("change:")
            .and_then(unhex)
            .ok_or_else(bad)?;
        let mut reader = Reader::new(&raw);
        let id = ChangeId::read_canonical(&mut reader).map_err(|_| bad())?;
        reader.finish().map_err(|_| bad())?;
        Ok(GitItem::Change(id))
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if text.is_empty()
        || !text.len().is_multiple_of(2)
        || !text.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

/// 원본 단위 선택: 고른 단위와 이전 결과의 필수 동반 단위 중 호출자가 승인한 단위.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnitSelection {
    /// 고른 단위.
    pub selected: BTreeSet<SourceUnit>,
    /// 승인한 필수 동반 단위.
    pub approved: BTreeSet<SourceUnit>,
}

/// 필수 동반 원본 단위가 필요한 이유 하나.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CompanionReason {
    /// 필요한 단위.
    pub unit: SourceUnit,
    /// 끊기는 컴포넌트 정의 참조.
    pub component: StableId,
}

// ───────────────────────────── 선택 staging 파이프라인 ─────────────────────────────

/// 저장 파일 재열기까지 검증된 후보. drop하면 임시 tree가 삭제됩니다.
#[derive(Debug)]
pub struct PreparedStageCandidate {
    /// 선택 적용 결과.
    pub semantic: CandidateIndexSnapshot,
    /// 기존 index의 파일을 보존한 후보.
    pub tree: CandidateTree,
    // Unstage가 기준으로 삼은 HEAD. 기록 직전과 결과 재검증에서도 확인합니다.
    head: Option<ResolvedTree>,
    // generation 단위를 포함하면 기대 semantic snapshot이 선택 후보 대신 원천(WT·HEAD) snapshot입니다.
    generation: Option<SemanticSnapshot>,
    // 실제로 포함한 원본 단위(선택 + 승인).
    units: BTreeSet<SourceUnit>,
}

impl PreparedStageCandidate {
    /// 적용 뒤 실제 index가 가져야 하는 semantic snapshot.
    pub fn expected_snapshot(&self) -> &SemanticSnapshot {
        self.generation
            .as_ref()
            .unwrap_or_else(|| self.semantic.snapshot())
    }

    /// 포함한 원본 단위(선택 + 승인).
    pub fn units(&self) -> &BTreeSet<SourceUnit> {
        &self.units
    }

    /// 테스트 fault injection: 검증 기준 semantic만 바꾼 후보.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn with_semantic_for_test(mut self, semantic: CandidateIndexSnapshot) -> Self {
        self.semantic = semantic;
        self.head = None;
        self.generation = None;
        self
    }
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
    Location(LocationError),
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
    /// 원본 단위 선택이 올바르지 않습니다(예: generation 단위와 변경 ID를 함께 고름).
    InvalidUnits(&'static str),
    /// 필수 동반 원본 단위를 승인하지 않았습니다. 자동 포함하지 않습니다(ADR-0080 결정 4).
    CompanionRequired {
        /// 승인이 필요한 전체 단위.
        units: BTreeSet<SourceUnit>,
        /// 단위별 이유.
        reasons: Vec<CompanionReason>,
    },
    /// 필수 동반 단위가 아닌 승인(오래된 승인).
    UnexpectedUnitApproval(SourceUnit),
    /// 어떤 원본 단위로도 해결할 수 없는 끊긴 컴포넌트 참조.
    DanglingComponent(StableId),
    /// 컴포넌트 텍스트 원본을 읽지 못했습니다.
    ComponentSource(String),
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
            Self::InvalidUnits(reason) => write!(f, "원본 단위 선택 오류: {reason}"),
            Self::CompanionRequired { units, .. } => {
                write!(f, "필수 동반 원본 {}개의 승인이 필요합니다", units.len())
            }
            Self::UnexpectedUnitApproval(unit) => {
                write!(f, "필수 동반 원본이 아닌 승인: {unit}")
            }
            Self::DanglingComponent(id) => {
                write!(f, "컴포넌트 정의 {id} 참조가 끊기며 포함할 원본이 없습니다")
            }
            Self::ComponentSource(e) => write!(f, "컴포넌트 원본 읽기 실패: {e}"),
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
    let location = locate_repository(git, worktree).map_err(PrepareStageError::Location)?;
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
    require_operation(request, SelectionOperation::Stage)?;
    prepare_selection(
        git,
        worktree,
        parent,
        request,
        preview,
        &UnitSelection::default(),
    )
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
    require_operation(request, SelectionOperation::Unstage)?;
    prepare_selection(
        git,
        worktree,
        parent,
        request,
        preview,
        &UnitSelection::default(),
    )
}

/// semantic 선택과 원본 단위 선택을 함께 준비합니다(UX.5.4).
///
/// 단위 포함은 stage면 그 단위의 index 경로를 저장된 작업 트리 파일과 같게, unstage면 HEAD와 같게 만듭니다.
/// 컴포넌트 정의 참조가 새로 끊기면 필요한 원본 단위를 [`PrepareStageError::CompanionRequired`]로 돌려주고
/// 자동으로 포함하지 않습니다. generation 단위는 변경 ID 선택과 함께 쓸 수 없습니다.
pub fn prepare_selection_with_units(
    git: &GatedGitRunner,
    worktree: &Path,
    parent: &Path,
    request: &StageSelectionRequest,
    preview: Option<&SemanticDiff>,
    units: &UnitSelection,
) -> Result<PreparedStageCandidate, PrepareStageError> {
    prepare_selection(git, worktree, parent, request, preview, units)
}

fn require_operation(
    request: &StageSelectionRequest,
    expected: SelectionOperation,
) -> Result<(), PrepareStageError> {
    if request.operation != expected {
        return Err(PrepareStageError::Selection(StageError::WrongOperation {
            expected,
            found: request.operation,
        }));
    }
    Ok(())
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

/// 프로젝트의 문서 레코드(canonical 정렬).
fn project_records(
    project: &SavedProject,
) -> Result<BTreeMap<StableId, DocumentRecords>, PrepareStageError> {
    let invalid = |e: &dyn fmt::Display| {
        PrepareStageError::Storage(CandidateStorageError::Invalid(e.to_string()))
    };
    let mut out = BTreeMap::new();
    for id in project.document_ids().map_err(|e| invalid(&e))? {
        let mut records = project
            .load_document(id)
            .map_err(|e| invalid(&e))?
            .model
            .to_records()
            .map_err(|e| invalid(&e))?;
        records.sort_canonical();
        out.insert(id, records);
    }
    Ok(out)
}

/// 문서들이 참조하는 컴포넌트 정의 ID.
fn component_refs<'a>(
    records: impl IntoIterator<Item = &'a DocumentRecords>,
) -> BTreeSet<StableId> {
    records
        .into_iter()
        .flat_map(|r| r.nodes.iter())
        .filter_map(|n| match &n.payload {
            NodePayload::ComponentInstance(i) => Some(i.component().id()),
            _ => None,
        })
        .collect()
}

/// 프로젝트 root의 컴포넌트 원본: 저장소 경로 → 정의 ID. 폴더가 없으면 빈 목록입니다.
fn component_files(root: &Path) -> Result<BTreeMap<GitPath, StableId>, PrepareStageError> {
    let (_, files) = read_project_library(root, ExternalAccess::MetadataOnly)
        .map_err(|e| PrepareStageError::ComponentSource(e.to_string()))?;
    Ok(files
        .into_iter()
        .filter_map(|(id, (path, _))| {
            let name = path.file_name()?.to_str()?;
            Some((
                GitPath::from_bytes(format!("{COMPONENTS_DIR}{name}").as_bytes()),
                id,
            ))
        })
        .collect())
}

/// 후보의 컴포넌트 정의 집합: 단위에 속한 경로는 원천(WT·HEAD), 나머지는 기준 index.
fn candidate_definitions(
    index: &BTreeMap<GitPath, StableId>,
    source: &BTreeMap<GitPath, StableId>,
    units: &BTreeSet<SourceUnit>,
) -> BTreeSet<StableId> {
    let covered = |path: &GitPath| units.iter().any(|u| u.contains(path.as_bytes()));
    index
        .iter()
        .filter(|(path, _)| !covered(path))
        .chain(source.iter().filter(|(path, _)| covered(path)))
        .map(|(_, id)| *id)
        .collect()
}

struct CompanionInputs<'a> {
    operation: SelectionOperation,
    has_changes: bool,
    base_refs: &'a BTreeSet<StableId>,
    selection_refs: &'a BTreeSet<StableId>,
    source_refs: &'a BTreeSet<StableId>,
    index_defs: &'a BTreeMap<GitPath, StableId>,
    source_defs: &'a BTreeMap<GitPath, StableId>,
}

/// 새로 끊기는 컴포넌트 참조가 없어질 때까지 필요한 단위를 더한 집합(고정점)과 이유.
fn companion_closure(
    inputs: &CompanionInputs<'_>,
    selected: &BTreeSet<SourceUnit>,
) -> Result<(BTreeSet<SourceUnit>, Vec<CompanionReason>), PrepareStageError> {
    let index_def_ids: BTreeSet<StableId> = inputs.index_defs.values().copied().collect();
    let base_dangling: BTreeSet<StableId> = inputs
        .base_refs
        .difference(&index_def_ids)
        .copied()
        .collect();
    let mut units = selected.clone();
    let mut reasons = Vec::new();
    loop {
        let refs = if units.contains(&SourceUnit::Generation) {
            inputs.source_refs
        } else {
            inputs.selection_refs
        };
        let defs = candidate_definitions(inputs.index_defs, inputs.source_defs, &units);
        let mut added = false;
        for id in refs.difference(&defs) {
            if base_dangling.contains(id) {
                continue;
            }
            let needed = match inputs.source_defs.iter().find(|(_, def)| *def == id) {
                Some((path, _)) => SourceUnit::File(path.clone()),
                None if inputs.operation == SelectionOperation::Unstage
                    && !inputs.has_changes
                    && !units.contains(&SourceUnit::Generation) =>
                {
                    SourceUnit::Generation
                }
                None => return Err(PrepareStageError::DanglingComponent(*id)),
            };
            if units.contains(&needed) {
                return Err(PrepareStageError::DanglingComponent(*id));
            }
            reasons.push(CompanionReason {
                unit: needed.clone(),
                component: *id,
            });
            units.insert(needed);
            added = true;
        }
        if !added {
            break;
        }
    }
    reasons.sort();
    Ok((units, reasons))
}

fn read_tree_file_bytes(root: &Path, path: &GitPath, kind: FileModeKind) -> Option<Vec<u8>> {
    let local = root.join(path.to_path_buf()?);
    match kind {
        FileModeKind::Symlink => {
            let target = std::fs::read_link(local).ok()?;
            Some(target.to_string_lossy().into_owned().into_bytes())
        }
        FileModeKind::Regular | FileModeKind::Executable => std::fs::read(local).ok(),
        _ => None,
    }
}

fn prepare_selection(
    git: &GatedGitRunner,
    worktree: &Path,
    parent: &Path,
    request: &StageSelectionRequest,
    preview: Option<&SemanticDiff>,
    unit_selection: &UnitSelection,
) -> Result<PreparedStageCandidate, PrepareStageError> {
    for unit in unit_selection
        .selected
        .iter()
        .chain(&unit_selection.approved)
    {
        if !unit.is_well_formed() {
            return Err(PrepareStageError::InvalidUnits(
                "Stellar 원본이 아닌 경로이거나 generation 묶음 안 경로입니다",
            ));
        }
    }
    let has_changes = !request.selected.is_empty() || !request.approved.is_empty();
    if has_changes
        && unit_selection
            .selected
            .iter()
            .chain(&unit_selection.approved)
            .any(|u| *u == SourceUnit::Generation)
    {
        return Err(PrepareStageError::InvalidUnits(
            "generation 묶음은 변경 ID 선택과 함께 포함할 수 없습니다",
        ));
    }
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
    // Unstage 원천(HEAD) tree는 후보를 쓸 때까지 살아 있어야 합니다.
    let mut head_tree = None;
    let mut head_entries: Vec<SnapshotEntry> = Vec::new();
    let (semantic, head, source_project, source_semantic) = match request.operation {
        SelectionOperation::Stage => {
            let working = SavedProjectLoader::new()
                .open(worktree)
                .map_err(PrepareStageError::Open)?;
            if working.project_id != base.project_id {
                return Err(PrepareStageError::Storage(
                    CandidateStorageError::Unsupported("서로 다른 project id"),
                ));
            }
            let working_semantic =
                saved_semantic_snapshot(&working).map_err(PrepareStageError::Storage)?;
            let candidate = build_stage_candidate(&indexed, &working_semantic, request)
                .map_err(PrepareStageError::Selection)?;
            if preview.is_some_and(|shown| shown != candidate.applied()) {
                return Err(PrepareStageError::StalePreview);
            }
            (candidate, None, working, working_semantic)
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
            let committed_semantic =
                saved_semantic_snapshot(&committed).map_err(PrepareStageError::Storage)?;
            let candidate = build_unstage_candidate(&committed_semantic, &indexed, request)
                .map_err(PrepareStageError::Selection)?;
            if preview.is_some_and(|shown| shown.inverse() != *candidate.applied()) {
                return Err(PrepareStageError::StalePreview);
            }
            head_entries = head.entries;
            head_tree = Some(tree);
            (
                candidate,
                Some(head.resolved),
                committed,
                committed_semantic,
            )
        }
    };
    let source_root: &Path = match &head_tree {
        Some(tree) => tree.root(),
        None => worktree,
    };

    // 컴포넌트 정의 참조의 필수 동반 단위(UX.5.4). 자동으로 포함하지 않습니다.
    let base_records = project_records(&base)?;
    let mut selection_records = base_records.clone();
    for projection in semantic.documents() {
        selection_records.insert(projection.records.definition.id, projection.records.clone());
    }
    let source_records = project_records(&source_project)?;
    let base_refs = component_refs(base_records.values());
    let selection_refs = component_refs(selection_records.values());
    let source_refs = component_refs(source_records.values());
    let index_defs = component_files(base_tree.root())?;
    let source_defs = component_files(source_root)?;
    let inputs = CompanionInputs {
        operation: request.operation,
        has_changes,
        base_refs: &base_refs,
        selection_refs: &selection_refs,
        source_refs: &source_refs,
        index_defs: &index_defs,
        source_defs: &source_defs,
    };
    let (closure, reasons) = companion_closure(&inputs, &unit_selection.selected)?;
    let required: BTreeSet<SourceUnit> = closure
        .difference(&unit_selection.selected)
        .cloned()
        .collect();
    if let Some(unit) = unit_selection
        .approved
        .iter()
        .find(|u| !required.contains(*u))
    {
        return Err(PrepareStageError::UnexpectedUnitApproval(unit.clone()));
    }
    if !required.is_subset(&unit_selection.approved) {
        return Err(PrepareStageError::CompanionRequired {
            units: required,
            reasons,
        });
    }
    let mut units = closure;
    if has_changes && units.contains(&SourceUnit::Generation) {
        return Err(PrepareStageError::InvalidUnits(
            "generation 묶음은 변경 ID 선택과 함께 포함할 수 없습니다",
        ));
    }
    // 선택 결과가 원천(stage: 저장된 WT, unstage: HEAD) generation과 의미가 같으면 새 generation을 만들지 않고
    // 원천 bytes를 그대로 씁니다. 저장 왕복이 무손실로 증명된 snapshot끼리의 같음이라 같은 원본입니다.
    // 그래서 전부 unstage하면 `git diff --cached`가 비고, 전부 stage하면 generation 파일이 WT와 같아집니다.
    if has_changes && semantic.snapshot() == &source_semantic {
        units.insert(SourceUnit::Generation);
    }

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

    // 원본 단위: 단위 경로를 원천(stage: 저장된 작업 트리, unstage: HEAD)과 같게 만듭니다.
    if !units.is_empty() {
        let covered = |path: &GitPath| units.iter().any(|u| u.contains(path.as_bytes()));
        files.retain(|path, _| !covered(path));
        match request.operation {
            SelectionOperation::Stage => {
                let listing =
                    list_working_tree_files(git, worktree).map_err(PrepareStageError::Git)?;
                for file in listing.files.iter().filter(|f| covered(&f.path)) {
                    files.insert(
                        file.path.clone(),
                        CandidateFile::FromWorkingTree {
                            path: file.path.clone(),
                        },
                    );
                }
            }
            SelectionOperation::Unstage => {
                let root = head_tree.as_ref().map(|t| t.root().to_path_buf());
                for entry in head_entries.iter().filter(|e| covered(&e.path)) {
                    let kind = entry.kind().ok_or(PrepareStageError::Storage(
                        CandidateStorageError::Unsupported("HEAD 경로의 파일 종류"),
                    ))?;
                    let bytes = root
                        .as_deref()
                        .and_then(|root| read_tree_file_bytes(root, &entry.path, kind))
                        .ok_or(PrepareStageError::Storage(
                            CandidateStorageError::Unsupported("HEAD 경로 bytes"),
                        ))?;
                    files.insert(
                        entry.path.clone(),
                        CandidateFile::Bytes {
                            path: entry.path.clone(),
                            kind,
                            bytes,
                        },
                    );
                }
            }
        }
    }
    let generation = units
        .contains(&SourceUnit::Generation)
        .then(|| source_semantic.clone());

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
    let expected = generation.as_ref().unwrap_or_else(|| semantic.snapshot());
    if saved_semantic_snapshot(&reopened).map_err(PrepareStageError::Storage)? != *expected {
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
        generation,
        units,
    })
}

/// 실제 stage 준비·적용·검증 실패입니다.
#[derive(Debug)]
pub enum ApplyStageError {
    /// 임시 경로 확인 실패. index 적용 전입니다.
    Preparation(PrepareStageError),
    /// 기존 candidate file delta 계산 실패.
    Delta(CandidateDeltaError),
    /// blob 기록 실패. 미참조 object는 Git GC에 맡깁니다.
    Blob(GitOpError),
    /// 적용 실패. 결과 미확정이면 게시 단계가 다시 읽은 index로 되돌렸는지(`RolledBack`) 또는 덮어쓰지
    /// 않았는지(`OutcomeUnknown`)를 알려 줍니다. 성공 증표가 없으므로 이 단계는 더 역쓰기하지 않습니다.
    Publish(PublishError),
    /// 실제 Index provider의 semantic 재검증 실패와 조건부 복원 결과.
    Verification {
        /// 검증 실패 원인.
        cause: Box<PrepareStageError>,
        /// Ok는 원래 index tree 복원, Err는 경합·취소·결과 미확정 진단입니다.
        rollback: Result<(), PublishError>,
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
    cancel: &GitCancelToken,
) -> Result<IndexSnapshot, ApplyStageError> {
    let check_cancel = || {
        if cancel.is_cancelled() {
            Err(ApplyStageError::Publish(PublishError::Git(
                GitOpError::Gate(GatedRunError::Run(GitRunError::Cancelled)),
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
    let receipt =
        publish_with_receipt(git, worktree, &written, cancel).map_err(ApplyStageError::Publish)?;
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
            != *prepared.expected_snapshot()
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

// ───────────────────────────── 세션·오류 ─────────────────────────────

/// Git 엔진 작업 실패.
#[derive(Debug)]
pub enum GitError {
    /// 시스템 Git을 찾지 못했습니다(프로세스 생성 없음).
    GitUnavailable,
    /// 신뢰하지 않은 프로젝트라 Git을 실행하지 않았습니다(프로세스 생성 0, XVI.7.3).
    NotTrusted,
    /// 경로를 확인하지 못했습니다.
    Path(std::io::ErrorKind),
    /// 경로가 Git worktree root가 아닙니다.
    NotWorktreeRoot,
    /// 저장소 위치 판별 실패.
    Location(LocationError),
    /// Git 실행·해석 실패.
    Op(GitOpError),
    /// HEAD·tree 읽기 실패.
    Snapshot(SnapshotError),
    /// 임시 materialize 실패.
    Materialize(MaterializeError),
    /// 비공개 임시 디렉터리를 만들지 못했습니다.
    Temp(String),
    /// 선택 후보 준비 실패.
    Prepare(PrepareStageError),
    /// 선택 후보 적용 실패.
    Apply(ApplyStageError),
    /// 필수 동반 변경을 승인하지 않았습니다. 자동 포함하지 않으며 목록을 돌려줍니다(ADR-0080 결정 4).
    CompanionRequired {
        /// 승인이 필요한 semantic 변경.
        changes: BTreeSet<ChangeId>,
        /// 승인이 필요한 원본 단위.
        units: BTreeSet<SourceUnit>,
        /// 원본 단위가 필요한 이유.
        reasons: Vec<CompanionReason>,
    },
    /// 필수 동반 변경이 아닌 승인(오래된 승인).
    UnexpectedApproval(GitItem),
    /// 요청한 변경 ID가 현재 status에 없습니다(오래된 요청).
    UnknownChange(ChangeId),
    /// 빈 선택.
    EmptySelection,
    /// 상태 page cursor가 현재 status 계산 revision과 맞지 않습니다.
    StaleCursor,
    /// cursor 형식 오류.
    InvalidCursor,
    /// staged snapshot이 유효한 Stellar 프로젝트가 아니라 `git commit`을 실행하지 않았습니다(XIV.8.1).
    InvalidStagedSnapshot(Vec<StagedFinding>),
    /// 재검증 뒤 commit 전에 index가 바뀌어 `git commit`을 실행하지 않았습니다.
    IndexChangedBeforeCommit,
    /// commit 실패(identity 누락·hook 실패 등 Git 결과 그대로, XIV.8.5).
    Commit(CommitError),
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GitUnavailable => f.write_str("시스템 Git을 찾지 못했습니다"),
            Self::NotTrusted => {
                f.write_str("신뢰하지 않은 프로젝트에서는 Git 명령을 실행하지 않습니다")
            }
            Self::Path(kind) => write!(f, "경로 확인 실패: {kind:?}"),
            Self::NotWorktreeRoot => f.write_str("Git worktree root 경로가 필요합니다"),
            Self::Location(e) => write!(f, "{e}"),
            Self::Op(e) => write!(f, "{e}"),
            Self::Snapshot(e) => write!(f, "{e}"),
            Self::Materialize(e) => write!(f, "{e}"),
            Self::Temp(e) => write!(f, "임시 디렉터리 생성 실패: {e}"),
            Self::Prepare(e) => write!(f, "{e}"),
            Self::Apply(e) => write!(f, "{e}"),
            Self::CompanionRequired { changes, units, .. } => write!(
                f,
                "필수 동반 변경 {}개와 원본 {}개의 승인이 필요합니다",
                changes.len(),
                units.len()
            ),
            Self::UnexpectedApproval(item) => write!(f, "필수 동반 변경이 아닌 승인: {item}"),
            Self::UnknownChange(id) => {
                write!(f, "현재 status에 없는 변경입니다: {}", change_id_text(id))
            }
            Self::EmptySelection => f.write_str("선택한 대상이 없습니다"),
            Self::StaleCursor => {
                f.write_str("status가 바뀌어 cursor를 쓸 수 없습니다. 처음부터 다시 조회하세요")
            }
            Self::InvalidCursor => f.write_str("cursor 형식 오류"),
            Self::InvalidStagedSnapshot(findings) => write!(
                f,
                "staged snapshot이 유효한 Stellar 프로젝트가 아닙니다(오류 {}건). commit하지 않았습니다",
                findings.iter().filter(|x| x.error).count()
            ),
            Self::IndexChangedBeforeCommit => {
                f.write_str("재검증 뒤 index가 바뀌어 commit하지 않았습니다")
            }
            Self::Commit(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for GitError {}

impl From<GatedRunError> for GitError {
    fn from(e: GatedRunError) -> Self {
        match e {
            GatedRunError::ProjectNotTrusted => GitError::NotTrusted,
            other => GitError::Op(GitOpError::Gate(other)),
        }
    }
}

impl From<GitOpError> for GitError {
    fn from(e: GitOpError) -> Self {
        match e {
            GitOpError::Gate(GatedRunError::ProjectNotTrusted) => GitError::NotTrusted,
            other => GitError::Op(other),
        }
    }
}

impl From<SnapshotError> for GitError {
    fn from(e: SnapshotError) -> Self {
        match e {
            SnapshotError::Op(op) => op.into(),
            other => GitError::Snapshot(other),
        }
    }
}

impl From<PrepareStageError> for GitError {
    fn from(e: PrepareStageError) -> Self {
        match e {
            PrepareStageError::Git(op) => op.into(),
            PrepareStageError::Selection(StageError::ApprovalRequired { required }) => {
                GitError::CompanionRequired {
                    changes: required,
                    units: BTreeSet::new(),
                    reasons: Vec::new(),
                }
            }
            PrepareStageError::Selection(StageError::UnexpectedApproval { id }) => {
                GitError::UnexpectedApproval(GitItem::Change(id))
            }
            PrepareStageError::Selection(StageError::UnknownChange(unknown)) => {
                GitError::UnknownChange(unknown.id)
            }
            PrepareStageError::CompanionRequired { units, reasons } => {
                GitError::CompanionRequired {
                    changes: BTreeSet::new(),
                    units,
                    reasons,
                }
            }
            PrepareStageError::UnexpectedUnitApproval(unit) => {
                GitError::UnexpectedApproval(GitItem::Unit(unit))
            }
            other => GitError::Prepare(other),
        }
    }
}

/// worktree root에 고정한 Git 엔진 세션. linked worktree는 자기 root·Git dir·index를 가진 별도 세션입니다.
#[derive(Debug, Clone)]
pub struct GitSession {
    git: GatedGitRunner,
    root: PathBuf,
    location: RepositoryLocation,
}

impl GitSession {
    /// `root`가 Git worktree root인지 확인하고 세션을 엽니다. 신뢰하지 않으면 프로세스를 만들지 않고 거부합니다.
    pub fn open(git: GatedGitRunner, root: &Path) -> Result<GitSession, GitError> {
        git.runner()?;
        let canonical = root.canonicalize().map_err(|e| GitError::Path(e.kind()))?;
        let location = locate_repository(&git, &canonical).map_err(|e| match e {
            LocationError::Gate(GatedRunError::ProjectNotTrusted) => GitError::NotTrusted,
            other => GitError::Location(other),
        })?;
        if location.worktree_root != canonical {
            return Err(GitError::NotWorktreeRoot);
        }
        Ok(GitSession {
            git,
            root: canonical,
            location,
        })
    }

    /// 시스템 Git과 OS 사용자 데이터의 신뢰 등록으로 엽니다(CLI·앱 공용).
    pub fn open_for_os_user(root: &Path) -> Result<GitSession, GitError> {
        let trust = os_git_trust(root);
        if trust == GitTrust::NotTrusted {
            return Err(GitError::NotTrusted);
        }
        GitSession::open(GatedGitRunner::new(system_git_runner()?, trust), root)
    }

    /// gate를 거친 실행기.
    pub fn git(&self) -> &GatedGitRunner {
        &self.git
    }

    /// canonical worktree root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// worktree·Git dir·common Git dir.
    pub fn location(&self) -> &RepositoryLocation {
        &self.location
    }

    fn temp(&self) -> Result<PrivateTempDir, GitError> {
        let parent = std::env::temp_dir();
        let dir = PrivateTempDir::create_in(&parent, "stellar-git")
            .map_err(|e| GitError::Temp(format!("{e:?}")))?;
        let canonical = dir
            .path()
            .canonicalize()
            .map_err(|e| GitError::Path(e.kind()))?;
        if canonical.starts_with(&self.root) {
            return Err(GitError::Temp(
                "임시 디렉터리가 작업 트리 안에 있습니다".into(),
            ));
        }
        Ok(dir)
    }

    fn head_commit(&self) -> Result<Option<ObjectId>, GitError> {
        Ok(
            match resolve_tree(&self.git, &self.root, &SnapshotSource::Head)? {
                ResolvedTree::Unborn => None,
                ResolvedTree::Tree { commit, .. } => commit,
            },
        )
    }
}

// ───────────────────────────── 상태 조회 ─────────────────────────────

/// semantic 변경 page 기본 크기.
pub const DEFAULT_STATUS_LIMIT: usize = 1000;
/// semantic 변경 page 최대 크기.
pub const MAX_STATUS_LIMIT: usize = 10_000;

/// 상태 조회 요청.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusRequest {
    /// 이전 page의 `next_cursor`.
    pub cursor: Option<String>,
    /// page 크기(1..=[`MAX_STATUS_LIMIT`]).
    pub limit: usize,
}

impl Default for StatusRequest {
    fn default() -> Self {
        StatusRequest {
            cursor: None,
            limit: DEFAULT_STATUS_LIMIT,
        }
    }
}

/// semantic 변경 한 줄.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeEntry {
    /// staged(HEAD → Index) 또는 unstaged(Index → WorkingTree).
    pub area: StatusArea,
    /// stage·unstage에 쓰는 ID.
    pub id: ChangeId,
    /// 변경 종류.
    pub kind: SemanticChangeKind,
    /// 대상 객체.
    pub object: StableId,
    /// 소속 문서.
    pub document: Option<StableId>,
    /// 속성 변경이면 경로.
    pub property: Option<PropertyPath>,
}

impl ChangeEntry {
    fn of(area: StatusArea, change: &SemanticChange) -> ChangeEntry {
        ChangeEntry {
            area,
            id: change.id(),
            kind: change.kind(),
            object: change.object_id(),
            document: change.document(),
            property: change.property_path().cloned(),
        }
    }
}

/// semantic 변경 page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticPage {
    /// staged 변경 수.
    pub staged_total: usize,
    /// unstaged 변경 수.
    pub unstaged_total: usize,
    /// 이 page의 변경(staged 먼저, 각 영역은 canonical 순서).
    pub entries: Vec<ChangeEntry>,
    /// 다음 page cursor.
    pub next_cursor: Option<String>,
}

/// 원본 단위 하나의 상태.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitStatus {
    /// 단위.
    pub unit: SourceUnit,
    /// HEAD와 index가 다른 경로가 있음(커밋에 포함됨).
    pub staged: bool,
    /// index와 저장된 작업 트리가 다른 경로가 있음.
    pub unstaged: bool,
    /// index에 없는 새 경로가 있음.
    pub untracked: bool,
    /// 병합되지 않은 경로가 있음.
    pub conflicted: bool,
    /// 바뀐 경로 수.
    pub paths: usize,
}

/// remote 요약. URL은 credential을 지운 값만 둡니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSummary {
    /// 이름.
    pub name: String,
    /// credential을 지운 fetch URL.
    pub fetch_url: Option<String>,
    /// push URL 수.
    pub push_urls: usize,
}

/// worker에서 계산한 Git 상태(UX.5.1). 모든 값은 디스크에 저장된 파일 기준입니다.
#[derive(Debug, Clone)]
pub struct GitStatusReport {
    /// 계산 revision: HEAD, index 항목, 저장된 Stellar 작업 파일, raw status의 hash.
    pub revision: ContentHash,
    /// worktree 위치.
    pub location: RepositoryLocation,
    /// HEAD·upstream·변경 수·진행 중 작업.
    pub repository: RepositoryStatus,
    /// commit identity 누락(없으면 `None`).
    pub missing_identity: Option<MissingIdentity>,
    /// remote 목록.
    pub remotes: Vec<RemoteSummary>,
    /// 원본 단위 상태(단위 순).
    pub units: Vec<UnitStatus>,
    /// non-Stellar 파일 raw status.
    pub raw: Vec<RawFileStatus>,
    /// semantic 변경 page. 프로젝트를 읽지 못하면 원인 문구입니다(raw 정보는 유효).
    pub semantic: Result<SemanticPage, String>,
}

fn unit_statuses(files: &[StellarFileStatus]) -> Vec<UnitStatus> {
    let mut units: BTreeMap<SourceUnit, UnitStatus> = BTreeMap::new();
    for file in files {
        let Some(unit) = SourceUnit::of_path(file.path.as_bytes()) else {
            continue;
        };
        let entry = units.entry(unit.clone()).or_insert(UnitStatus {
            unit,
            staged: false,
            unstaged: false,
            untracked: false,
            conflicted: false,
            paths: 0,
        });
        entry.paths += 1;
        entry.staged |= file.staged.is_some();
        entry.unstaged |= file
            .unstaged
            .as_ref()
            .is_some_and(|c| *c != FileChange::Untracked);
        entry.untracked |= file.is_untracked();
        entry.conflicted |= file.conflicted;
    }
    units.into_values().collect()
}

/// materialize한 tree(또는 작업 트리)의 저장 semantic snapshot. generation이 없으면 빈 snapshot입니다.
fn semantic_of(root: &Path, loader: &SavedProjectLoader) -> Result<SemanticSnapshot, String> {
    if !root.join("design/current").exists() {
        return Ok(SemanticSnapshot::new());
    }
    let project = loader.open(root).map_err(|e| e.to_string())?;
    saved_semantic_snapshot(&project).map_err(|e| e.to_string())
}

fn put(buf: &mut Vec<u8>, bytes: &[u8]) {
    buf.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    buf.extend_from_slice(bytes);
}

impl GitSession {
    /// 계산 revision. 같은 HEAD·index·저장된 Stellar 파일·raw status면 같은 값입니다.
    /// 내용 hash로 이름 붙은 immutable 경로(`design/objects/`, `design/generations/`)는 경로만 넣습니다.
    pub fn revision(&self) -> Result<ContentHash, GitError> {
        let report = read_status(&self.git, &self.root, StatusOptions::default())?;
        let partition = partition_status(&report);
        let index = read_index_snapshot(&self.git, &self.root)?;
        self.revision_of(self.head_commit()?, &index, &partition.raw)
    }

    fn revision_of(
        &self,
        head: Option<ObjectId>,
        index: &IndexSnapshot,
        raw: &[RawFileStatus],
    ) -> Result<ContentHash, GitError> {
        let mut buf = b"stellar-git-status-revision/1".to_vec();
        put(
            &mut buf,
            head.as_ref().map_or("", |h| h.as_str()).as_bytes(),
        );
        for entry in &index.entries {
            put(&mut buf, entry.path.as_bytes());
            put(&mut buf, entry.mode.to_string().as_bytes());
            put(&mut buf, entry.oid.as_str().as_bytes());
        }
        for entry in &index.unmerged {
            put(&mut buf, entry.path.as_bytes());
            put(&mut buf, entry.oid.as_str().as_bytes());
            buf.push(entry.stage);
        }
        let listing = list_working_tree_files(&self.git, &self.root)?;
        for file in &listing.files {
            if classify_project_path(file.path.as_bytes()) == ProjectFileClass::NonStellar {
                continue;
            }
            put(&mut buf, file.path.as_bytes());
            let bytes = file.path.as_bytes();
            if bytes.starts_with(b"design/objects/") || bytes.starts_with(b"design/generations/") {
                continue;
            }
            let content = read_working_tree_file(&self.root, file).map_err(GitError::Path)?;
            put(&mut buf, &content);
        }
        for path in &listing.missing_tracked {
            put(&mut buf, b"missing");
            put(&mut buf, path.as_bytes());
        }
        for entry in raw {
            put(&mut buf, entry.path.as_bytes());
            put(&mut buf, entry.xy.as_bytes());
        }
        Ok(ContentHash::compute(&buf))
    }

    /// 저장소 상태와 semantic 변경 page를 읽습니다(읽기 전용, UX.5.1). 앱은 worker에서 부릅니다.
    pub fn status(&self, request: &StatusRequest) -> Result<GitStatusReport, GitError> {
        if request.limit == 0 || request.limit > MAX_STATUS_LIMIT {
            return Err(GitError::InvalidCursor);
        }
        let report = read_status(&self.git, &self.root, StatusOptions::default())?;
        let operations = read_in_progress_operations(&self.git, &self.root)?;
        let repository = RepositoryStatus::from_report(&report, operations)
            .map_err(|e| GitError::Op(GitOpError::Parse(e)))?;
        let partition = partition_status(&report);
        let stellar_files = stellar_file_statuses(&partition.stellar);
        let index = read_index_snapshot(&self.git, &self.root)?;
        let head = self.head_commit()?;
        let revision = self.revision_of(head, &index, &partition.raw)?;
        let missing_identity = match resolve_commit_identity(&self.git, &self.root) {
            Ok(missing) => missing,
            Err(IdentityReadError::Gate(e)) => return Err(e.into()),
            Err(IdentityReadError::Git(failure)) => {
                return Err(GitError::Op(GitOpError::Git(failure)))
            }
        };
        let remotes = list_remotes(&self.git, &self.root)?
            .into_iter()
            .map(|r| RemoteSummary {
                name: r.name,
                fetch_url: r.fetch_url.map(|u| u.redacted()),
                push_urls: r.push_urls.len(),
            })
            .collect();
        let offset = match &request.cursor {
            None => 0,
            Some(cursor) => parse_cursor(cursor, &revision)?,
        };
        let semantic = self
            .semantic_status(&index)
            .map(|status| page(&status, offset, request.limit, &revision));
        Ok(GitStatusReport {
            revision,
            location: self.location.clone(),
            repository,
            missing_identity,
            remotes,
            units: unit_statuses(&stellar_files),
            raw: partition.raw,
            semantic,
        })
    }

    fn semantic_status(&self, index: &IndexSnapshot) -> Result<SemanticStatus, String> {
        if index.has_conflicts() {
            return Err("index conflict가 있어 semantic status를 계산하지 않습니다".into());
        }
        let temp = self.temp().map_err(|e| e.to_string())?;
        let loader = SavedProjectLoader::for_materialized_snapshot();
        let head = read_tree_snapshot(&self.git, &self.root, &SnapshotSource::Head)
            .map_err(|e| e.to_string())?;
        let head_semantic = match head.resolved {
            ResolvedTree::Unborn => SemanticSnapshot::new(),
            ResolvedTree::Tree { .. } => {
                let tree = materialize_snapshot(&self.git, &self.root, &head.entries, temp.path())
                    .map_err(|e| e.to_string())?;
                semantic_of(tree.root(), &loader).map_err(|e| format!("HEAD: {e}"))?
            }
        };
        let tree = materialize_snapshot(&self.git, &self.root, &index.entries, temp.path())
            .map_err(|e| e.to_string())?;
        let index_semantic =
            semantic_of(tree.root(), &loader).map_err(|e| format!("Index: {e}"))?;
        let working = semantic_of(&self.root, &SavedProjectLoader::new())
            .map_err(|e| format!("WorkingTree: {e}"))?;
        Ok(SemanticStatus::classify(
            &head_semantic,
            &index_semantic,
            &working,
        ))
    }
}

const CURSOR_REVISION_CHARS: usize = 16;

fn parse_cursor(cursor: &str, revision: &ContentHash) -> Result<usize, GitError> {
    let (rev, offset) = cursor.split_once('.').ok_or(GitError::InvalidCursor)?;
    let offset: usize = offset.parse().map_err(|_| GitError::InvalidCursor)?;
    if rev.len() != CURSOR_REVISION_CHARS {
        return Err(GitError::InvalidCursor);
    }
    if !revision.to_string().ends_with(rev) {
        return Err(GitError::StaleCursor);
    }
    Ok(offset)
}

fn page(
    status: &SemanticStatus,
    offset: usize,
    limit: usize,
    revision: &ContentHash,
) -> SemanticPage {
    let staged = status.staged().changes();
    let unstaged = status.unstaged().changes();
    let all = staged
        .iter()
        .map(|c| (StatusArea::Staged, c))
        .chain(unstaged.iter().map(|c| (StatusArea::Unstaged, c)));
    let total = staged.len() + unstaged.len();
    let entries: Vec<ChangeEntry> = all
        .skip(offset)
        .take(limit)
        .map(|(area, change)| ChangeEntry::of(area, change))
        .collect();
    let end = offset.saturating_add(entries.len());
    let rev = revision.to_string();
    let next_cursor = (end < total).then(|| {
        format!(
            "{}.{end}",
            &rev[rev.len().saturating_sub(CURSOR_REVISION_CHARS)..]
        )
    });
    SemanticPage {
        staged_total: staged.len(),
        unstaged_total: unstaged.len(),
        entries,
        next_cursor,
    }
}

// ───────────────────────────── stage·unstage ─────────────────────────────

/// stage·unstage 요청.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionInput {
    /// 작업.
    pub operation: SelectionOperation,
    /// 고른 변경·원본 단위.
    pub items: Vec<GitItem>,
    /// 이전 결과의 필수 동반 변경 중 승인한 항목.
    pub approved: Vec<GitItem>,
}

/// stage·unstage 결과.
#[derive(Debug, Clone)]
pub struct SelectionOutcome {
    /// 적용한 semantic 변경 수(선택 + 승인).
    pub applied_changes: usize,
    /// 포함한 원본 단위.
    pub units: BTreeSet<SourceUnit>,
    /// 적용 뒤 index 항목 수.
    pub index_entries: usize,
    /// 적용 뒤 계산 revision.
    pub revision: ContentHash,
}

fn split(items: &[GitItem]) -> (BTreeSet<ChangeId>, BTreeSet<SourceUnit>) {
    let mut changes = BTreeSet::new();
    let mut units = BTreeSet::new();
    for item in items {
        match item {
            GitItem::Change(id) => {
                changes.insert(id.clone());
            }
            GitItem::Unit(unit) => {
                units.insert(unit.clone());
            }
        }
    }
    (changes, units)
}

impl GitSession {
    /// 고른 변경·원본 단위를 index에 포함하거나(stage) 뺍니다(unstage). 작업 트리와 문서는 바꾸지 않습니다.
    /// 필수 동반 변경이 있으면 [`GitError::CompanionRequired`]로 목록만 돌려주고 index를 바꾸지 않습니다.
    pub fn select(
        &self,
        input: &SelectionInput,
        cancel: &GitCancelToken,
    ) -> Result<SelectionOutcome, GitError> {
        if input.items.is_empty() {
            return Err(GitError::EmptySelection);
        }
        let (selected, units) = split(&input.items);
        let (approved, approved_units) = split(&input.approved);
        let request = StageSelectionRequest {
            operation: input.operation,
            selected,
            approved,
        };
        let units = UnitSelection {
            selected: units,
            approved: approved_units,
        };
        let temp = self.temp()?;
        let prepared =
            prepare_selection(&self.git, &self.root, temp.path(), &request, None, &units)?;
        let index = apply_stage_candidate(&self.git, &self.root, temp.path(), &prepared, cancel)
            .map_err(|e| match e {
                ApplyStageError::Preparation(p) => GitError::from(p),
                other => GitError::Apply(other),
            })?;
        let applied_changes = prepared.semantic.applied().changes().len();
        let outcome_units = prepared.units.clone();
        drop(prepared);
        Ok(SelectionOutcome {
            applied_changes,
            units: outcome_units,
            index_entries: index.entries.len(),
            revision: self.revision()?,
        })
    }

    /// 편의 함수: stage.
    pub fn stage(
        &self,
        items: Vec<GitItem>,
        approved: Vec<GitItem>,
    ) -> Result<SelectionOutcome, GitError> {
        self.select(
            &SelectionInput {
                operation: SelectionOperation::Stage,
                items,
                approved,
            },
            &GitCancelToken::new(),
        )
    }

    /// 편의 함수: unstage.
    pub fn unstage(
        &self,
        items: Vec<GitItem>,
        approved: Vec<GitItem>,
    ) -> Result<SelectionOutcome, GitError> {
        self.select(
            &SelectionInput {
                operation: SelectionOperation::Unstage,
                items,
                approved,
            },
            &GitCancelToken::new(),
        )
    }
}

// ───────────────────────────── 재검증·commit ─────────────────────────────

/// staged snapshot 재검증 항목.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedFinding {
    /// 오류면 참(commit 차단), 경고면 거짓.
    pub error: bool,
    /// 고정 코드(kebab-case).
    pub code: &'static str,
    /// 관련 문서.
    pub document: Option<StableId>,
    /// 설명.
    pub message: String,
}

impl StagedFinding {
    fn error(code: &'static str, document: Option<StableId>, message: impl Into<String>) -> Self {
        StagedFinding {
            error: true,
            code,
            document,
            message: message.into(),
        }
    }
}

/// staged snapshot 재검증 결과.
#[derive(Debug, Clone)]
pub struct StagedValidation {
    /// 검증한 index.
    pub index: IndexSnapshot,
    /// 오류·경고.
    pub findings: Vec<StagedFinding>,
}

impl StagedValidation {
    /// 오류가 없는지.
    pub fn is_valid(&self) -> bool {
        !self.findings.iter().any(|f| f.error)
    }
}

/// commit 결과(XIV.8.4). push하지 않습니다.
#[derive(Debug, Clone)]
pub struct CommitOutcome {
    /// commit 전 HEAD commit.
    pub previous_head: Option<ObjectId>,
    /// 새 HEAD commit.
    pub head: ObjectId,
    /// commit 뒤 다시 읽은 저장소 상태.
    pub repository: RepositoryStatus,
    /// commit 뒤 HEAD tree와 index가 다른 경로 수(정상이면 0).
    pub staged_remaining: usize,
    /// 새 HEAD tree가 재검증한 index와 같은지. hook이 index를 바꾸면 거짓입니다(되돌리지 않음).
    pub tree_matches_validated: bool,
    /// 재검증 경고.
    pub warnings: Vec<StagedFinding>,
    /// commit 뒤 계산 revision.
    pub revision: ContentHash,
}

fn sorted(entries: &[SnapshotEntry]) -> Vec<SnapshotEntry> {
    let mut entries = entries.to_vec();
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    entries
}

impl GitSession {
    /// staged snapshot이 유효한 Stellar 프로젝트인지 전체 validator로 다시 검사합니다(XIV.8.1).
    ///
    /// 저장소 무결성(loader), 문서 validator, 저장 무손실 왕복, semantic 무결성, 컴포넌트 원본을 봅니다.
    /// 컴포넌트 정의가 없는 인스턴스는 `stellar validate`와 같이 경고입니다. index·작업 트리를 쓰지 않습니다.
    pub fn validate_staged(&self) -> Result<StagedValidation, GitError> {
        let index = read_index_snapshot(&self.git, &self.root)?;
        let mut findings = Vec::new();
        if index.has_conflicts() {
            findings.push(StagedFinding::error(
                "conflicted",
                None,
                "index에 병합되지 않은 경로가 있습니다",
            ));
            return Ok(StagedValidation { index, findings });
        }
        let temp = self.temp()?;
        let tree = materialize_snapshot(&self.git, &self.root, &index.entries, temp.path())
            .map_err(GitError::Materialize)?;
        let root = tree.root();
        if !root.join("stellar.yaml").is_file() {
            findings.push(StagedFinding::error(
                "not-a-project",
                None,
                "staged snapshot에 stellar.yaml이 없습니다",
            ));
            return Ok(StagedValidation { index, findings });
        }
        let project = match SavedProjectLoader::for_materialized_snapshot().open(root) {
            Ok(project) => project,
            Err(e) => {
                findings.push(StagedFinding::error("storage-invalid", None, e.to_string()));
                return Ok(StagedValidation { index, findings });
            }
        };
        let mut records = Vec::new();
        match project.document_ids() {
            Ok(ids) => {
                for id in ids {
                    match project.load_document(id).map(|d| d.model.to_records()) {
                        Ok(Ok(mut r)) => {
                            r.sort_canonical();
                            if let Err(error) = stellar_document::validate::validate_document(&r) {
                                for violation in &error.violations {
                                    findings.push(StagedFinding::error(
                                        "document-invalid",
                                        Some(id),
                                        violation.to_string(),
                                    ));
                                }
                            }
                            records.push(r);
                        }
                        Ok(Err(e)) => findings.push(StagedFinding::error(
                            "document-load",
                            Some(id),
                            e.to_string(),
                        )),
                        Err(e) => findings.push(StagedFinding::error(
                            "document-load",
                            Some(id),
                            e.to_string(),
                        )),
                    }
                }
            }
            Err(e) => findings.push(StagedFinding::error("document-load", None, e.to_string())),
        }
        match saved_semantic_snapshot(&project) {
            Ok(semantic) => {
                // 컴포넌트 정의는 문서 snapshot 밖 텍스트 원본이므로 인스턴스의 정의 참조는 아래에서 따로 봅니다.
                let instance_targets = component_refs(records.iter());
                for violation in stellar_semantic_diff::validate::validate_snapshot(&semantic)
                    .into_iter()
                    .filter(|v| {
                        !matches!(
                            v,
                            stellar_semantic_diff::validate::SnapshotViolation::DanglingReference {
                                target,
                                ..
                            } if instance_targets.contains(target)
                        )
                    })
                {
                    findings.push(StagedFinding::error(
                        "snapshot-invalid",
                        None,
                        format!("{violation:?}"),
                    ));
                }
            }
            Err(e) => findings.push(StagedFinding::error(
                "storage-roundtrip",
                None,
                e.to_string(),
            )),
        }
        match read_project_library(root, ExternalAccess::MetadataOnly) {
            Ok((library, _)) => {
                let defs: BTreeSet<StableId> = library.iter().map(|e| e.definition().id).collect();
                for id in component_refs(records.iter()).difference(&defs) {
                    findings.push(StagedFinding {
                        error: false,
                        code: "missing-component-definition",
                        document: None,
                        message: format!(
                            "staged 인스턴스가 참조하는 컴포넌트 정의 {id}가 없습니다"
                        ),
                    });
                }
            }
            Err(e) => findings.push(StagedFinding::error(
                "component-library",
                None,
                e.to_string(),
            )),
        }
        Ok(StagedValidation { index, findings })
    }

    /// staged 변경을 commit합니다(XIV.8.1·XIV.8.4·XIV.8.5).
    ///
    /// 1. 신뢰하지 않으면 프로세스를 만들지 않고 거부합니다.
    /// 2. staged snapshot을 전체 validator로 다시 검사하고 오류가 있으면 `git commit`을 실행하지 않습니다.
    /// 3. 검사 뒤 index가 바뀌었으면 실행하지 않습니다.
    /// 4. 시스템 Git `git commit`(message는 stdin)을 실행합니다. hook 실패는 우회하지 않고 Git 결과 그대로 돌려줍니다.
    /// 5. 성공하면 HEAD·index·status를 다시 읽습니다. push하지 않습니다.
    pub fn commit(&self, message: &str) -> Result<CommitOutcome, GitError> {
        self.git.runner()?;
        let validation = self.validate_staged()?;
        if !validation.is_valid() {
            return Err(GitError::InvalidStagedSnapshot(validation.findings));
        }
        if read_index_snapshot(&self.git, &self.root)? != validation.index {
            return Err(GitError::IndexChangedBeforeCommit);
        }
        let previous_head = self.head_commit()?;
        commit_with_message(&self.git, &self.root, message).map_err(|e| match e {
            CommitError::Gate(GatedRunError::ProjectNotTrusted) => GitError::NotTrusted,
            other => GitError::Commit(other),
        })?;
        let head = self.head_commit()?.ok_or(GitError::Op(GitOpError::Parse(
            stellar_git_porcelain::ops::ParseFailure::new(
                "rev-parse HEAD",
                "commit 뒤 HEAD가 없습니다",
                b"",
            ),
        )))?;
        let committed = read_tree_snapshot(&self.git, &self.root, &SnapshotSource::Head)?;
        let index = read_index_snapshot(&self.git, &self.root)?;
        let tree = sorted(&committed.entries);
        let now = sorted(&index.entries);
        let paths: BTreeSet<&GitPath> = tree.iter().chain(&now).map(|e| &e.path).collect();
        let staged_remaining = paths
            .into_iter()
            .filter(|p| tree.iter().find(|e| &e.path == *p) != now.iter().find(|e| &e.path == *p))
            .count();
        let tree_matches_validated = tree == sorted(&validation.index.entries);
        let repository = read_repository_status(&self.git, &self.root)?;
        Ok(CommitOutcome {
            previous_head,
            head,
            repository,
            staged_remaining,
            tree_matches_validated,
            warnings: validation.findings,
            revision: self.revision()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_types::property::PropertyKey;

    #[test]
    fn item_ids_round_trip_and_reject_bad_forms() {
        let object = StableId::new_v7(7, 0, 7);
        let ids = [
            ChangeId::add(object),
            ChangeId::remove(object),
            ChangeId::position(object),
            ChangeId::property(PropertyPath::with_keys(object, vec![PropertyKey(3)])),
        ];
        for id in ids {
            let item = GitItem::Change(id);
            assert_eq!(item.to_string().parse::<GitItem>().unwrap(), item);
        }
        for unit in [
            SourceUnit::Generation,
            SourceUnit::File(GitPath::from_bytes(b"design/components/a-1.stellar.yaml")),
            SourceUnit::File(GitPath::from_bytes(b"stellar.yaml")),
        ] {
            let item = GitItem::Unit(unit);
            assert_eq!(item.to_string().parse::<GitItem>().unwrap(), item);
        }
        for bad in [
            "",
            "change:",
            "change:zz",
            "change:0102",
            "file:README.md",
            "file:design/current",
            "file-hex:6",
            "unit",
        ] {
            assert!(bad.parse::<GitItem>().is_err(), "{bad}");
        }
    }

    #[test]
    fn source_units_split_generation_and_files() {
        assert_eq!(
            SourceUnit::of_path(b"design/current"),
            Some(SourceUnit::Generation)
        );
        assert_eq!(
            SourceUnit::of_path(b"design/objects/ab/cd"),
            Some(SourceUnit::Generation)
        );
        assert_eq!(
            SourceUnit::of_path(b"design/components/x.stellar.yaml"),
            Some(SourceUnit::File(GitPath::from_bytes(
                b"design/components/x.stellar.yaml"
            )))
        );
        assert_eq!(SourceUnit::of_path(b"README.md"), None);
    }

    #[test]
    fn untrusted_session_spawns_nothing() {
        let runner = GitRunner::new(
            PathBuf::from("/nonexistent/git"),
            Vec::new(),
            Duration::from_secs(5),
        );
        let gate = GatedGitRunner::new(runner.clone(), GitTrust::NotTrusted);
        assert!(matches!(
            GitSession::open(gate, &std::env::temp_dir()),
            Err(GitError::NotTrusted)
        ));
        assert_eq!(runner.spawn_count(), 0);
    }
}
