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
//!
//! # chunk cache는 Git 밖(ADR-0122)
//!
//! chunk cache는 Git이 추적하지 않는 `.stellar/cache/chunks/`에 있고 Git은 텍스트 원본만 담습니다. 그래서
//! materialize한 Git tree(HEAD·index·후보)에는 cache가 없고, 저장소 loader로 열기 전에 그 snapshot 안에서 원본으로
//! cache를 만듭니다(`stellar_project::cache::prepare_snapshot`). 작업 트리 쪽은 작업 트리 cache가 원본과 짝인지와
//! 관계없이 원본만 비공개 사본으로 복사해 엽니다(`text_snapshot`). 후보 index에는 원본 파일만 넣고 cache 경로는
//! 넣지 않습니다. 옛 배치(`design/current` 등)를 추적하던 index는 stage·unstage·commit 전에 그 경로를 뺍니다
//! ([`GitSession::migrate_cache`]).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use stellar_document::records::DocumentRecords;
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

/// 필수 동반 원본 단위 계산(컴포넌트 정의·token 의존, ADR-0127 결정 4).
mod companion;
use companion::{companion_closure, component_files, component_refs, token_files, CompanionInputs};

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
/// 문서 전체는 [`SourceUnit::Generation`] 묶음 하나입니다. 옛 배치(ADR-0122 이전 Git이 추적하던
/// `design/current`·`design/generations/`·`design/objects/`)의 경로와 텍스트 원본·컴포넌트가 아닌 나머지 `design/` 경로가
/// 이 묶음이고, 텍스트 원본 프로젝트에서 이 묶음을 고르면 문서를 이루는 원본 파일(컴포넌트 제외)도 함께 덮습니다(ADR-0122
/// 결정 5). chunk cache(`.stellar/cache/chunks/`)는 Git 밖이라 어떤 단위에도 들지 않습니다. 텍스트 원본(ADR-0077 결정 1)은 의미 단위 파일 하나가 단위입니다:
/// `design/document.stellar.yaml`(문서·페이지 목록), View 파일 `design/pages/<page>/<view>.stellar.yaml`(그 View의
/// 분할 Frame 파일 `design/pages/<page>/<view>/**.stellar.yaml`을 함께 묶음), token set `design/tokens/<slug>.stellar.yaml`,
/// asset catalog `design/assets/catalog.stellar.yaml`, 컴포넌트 `design/components/<이름>.stellar.yaml`(ADR-0080 개정).
/// 그 밖의 Stellar 파일(`stellar.yaml`, `stellar.lock.yaml`, asset)도 파일 하나가 단위입니다.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceUnit {
    /// generation 저장과, 텍스트 원본·컴포넌트가 아닌 나머지 `design/` 경로(View generation 묶음).
    Generation,
    /// Stellar 파일 하나(저장소 root 기준 경로).
    File(GitPath),
}

/// 컴포넌트 텍스트 원본 폴더.
pub const COMPONENTS_DIR: &str = "design/components/";

/// 텍스트 원본 폴더(`design/`).
const DESIGN_DIR: &str = "design/";
/// 텍스트 원본 파일 확장자(ADR-0077 결정 1).
const SOURCE_EXT: &str = ".stellar.yaml";

/// 텍스트 원본 경로(`design/` 아래)의 의미 단위 파일. 원본이 아니면 `None`입니다. View의 분할 Frame 파일은 그
/// View 파일로 묶습니다(`$ref`로 서로 기대므로 따로 stage하면 원본이 깨짐).
fn text_source_unit(path: &[u8]) -> Option<GitPath> {
    use stellar_project::document_io::source_v4::{catalog::CATALOG_PATH, project::DOCUMENT_PATH};
    let relative = std::str::from_utf8(path.strip_prefix(DESIGN_DIR.as_bytes())?).ok()?;
    if relative == DOCUMENT_PATH || relative == CATALOG_PATH {
        return Some(GitPath::from_bytes(path));
    }
    if !relative.ends_with(SOURCE_EXT) {
        return None;
    }
    let parts: Vec<&str> = relative.split('/').collect();
    match parts.as_slice() {
        ["tokens", name] if name.len() > SOURCE_EXT.len() => Some(GitPath::from_bytes(path)),
        ["pages", page, view] if !page.is_empty() && view.len() > SOURCE_EXT.len() => {
            Some(GitPath::from_bytes(path))
        }
        ["pages", page, view, rest @ ..]
            if !page.is_empty() && !view.is_empty() && !rest.is_empty() =>
        {
            Some(GitPath::from_bytes(
                format!("{DESIGN_DIR}pages/{page}/{view}{SOURCE_EXT}").as_bytes(),
            ))
        }
        _ => None,
    }
}

impl SourceUnit {
    /// 경로가 속한 원본 단위. non-Stellar 파일은 `None`입니다(raw Git status만).
    pub fn of_path(path: &[u8]) -> Option<SourceUnit> {
        match classify_project_path(path) {
            ProjectFileClass::NonStellar => None,
            ProjectFileClass::Stellar(StellarFileKind::Design)
                if !path.starts_with(COMPONENTS_DIR.as_bytes()) =>
            {
                Some(text_source_unit(path).map_or(SourceUnit::Generation, SourceUnit::File))
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
    /// 끊기는 컴포넌트 정의 참조. token 의존이면 그 token을 참조하는 컴포넌트 정의입니다.
    pub component: StableId,
    /// 끊기는 token 참조(컴포넌트 정의 `local-tokens` → token set, ADR-0127 결정 4). 정의 참조면 `None`입니다.
    pub token: Option<StableId>,
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
    /// token set 텍스트 원본(`design/tokens/`)을 읽지 못했습니다.
    TokenSource(String),
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
            Self::TokenSource(e) => write!(f, "token set 원본 읽기 실패: {e}"),
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

/// 옛 배치 generation 저장 경로(`design/current`, `design/generations/`, `design/objects/`, ADR-0122 이전 Git 추적).
fn is_generation_storage(path: &[u8]) -> bool {
    stellar_project::cache::is_legacy_cache_path(path)
}

/// materialize한 Git snapshot을 저장소 loader로 엽니다. 열기 전에 snapshot 안에서 옛 배치를 옮기고 원본으로 cache를
/// 만듭니다(모듈 문서 "chunk cache는 Git 밖").
fn open_snapshot_tree(
    loader: &SavedProjectLoader,
    root: &Path,
) -> Result<SavedProject, PrepareStageError> {
    stellar_project::cache::prepare_snapshot(root)
        .map_err(|e| PrepareStageError::Storage(CandidateStorageError::Invalid(e.to_string())))?;
    loader.open(root).map_err(PrepareStageError::Open)
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

/// 텍스트 원본 후보 파일 하나: 경로와 새 bytes(`None`이면 지움).
type TextCandidateFile = (GitPath, Option<Vec<u8>>);

/// index tree(`index_root`)에 텍스트 원본이 있으면 후보 문서를 원본으로 인코딩해 바뀐 파일(`Some`)과 지울 파일
/// (`None`)을 돌려줍니다(ADR-0113 결정 6). 문서 밖 의미(token·이미지 record·Context)는 index 원본 것을 쓰고, 컴포넌트
/// 원본 파일은 원본 단위가 다루므로 넣지 않습니다. 원본이 없거나 후보가 문서를 바꾸지 않으면 빈 목록입니다.
fn text_candidate_files(
    index_root: &Path,
    candidate: &CandidateIndexSnapshot,
) -> Result<Vec<TextCandidateFile>, PrepareStageError> {
    use stellar_project::document_io::session::{decode_source, read_source, SourceLibrary};
    let unsupported = |reason: &'static str| {
        PrepareStageError::Storage(CandidateStorageError::Unsupported(reason))
    };
    let Some(files) =
        read_source(index_root).map_err(|_| unsupported("index 텍스트 원본을 읽을 수 없습니다"))?
    else {
        return Ok(Vec::new());
    };
    let (model, library) = SourceLibrary::split(
        decode_source(&files).map_err(|_| unsupported("index 텍스트 원본을 읽을 수 없습니다"))?,
    );
    let document = model.document_id();
    let Some(projection) = candidate
        .documents()
        .iter()
        .find(|projection| projection.records.definition.id == document)
    else {
        return Ok(Vec::new());
    };
    let (mut next, _) =
        stellar_document::model::DocumentModel::from_records(projection.records.clone())
            .map_err(|_| unsupported("후보 문서를 모델로 만들 수 없습니다"))?;
    next.replace_context(model.context().live(&model));
    let encoded = stellar_project::document_io::source_v4::project::encode(&library.join(next))
        .map_err(|_| unsupported("후보 문서를 텍스트 원본으로 쓸 수 없습니다"))?;
    let path_of = |relative: &str| GitPath::from_bytes(format!("design/{relative}").as_bytes());
    let mut out = Vec::new();
    for (relative, text) in &encoded.files {
        if relative.starts_with("components/") {
            continue;
        }
        if files.files.get(relative) != Some(text) {
            out.push((path_of(relative), Some(text.clone().into_bytes())));
        }
    }
    for relative in files.files.keys() {
        if !relative.starts_with("components/") && !encoded.files.contains_key(relative) {
            out.push((path_of(relative), None));
        }
    }
    Ok(out)
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
    let base = open_snapshot_tree(&loader, base_tree.root())?;
    let indexed = saved_semantic_snapshot(&base).map_err(PrepareStageError::Storage)?;
    // 변경 ID 단위 선택은 index의 텍스트 원본을 후보 문서로 다시 씁니다. 원본이 없는 index(옛 배치만 추적하는 chunk
    // 프로젝트)는 cache를 Git에 넣지 않으므로 변경 단위로 표현할 수 없습니다(ADR-0122 결정 5).
    if has_changes
        && stellar_project::document_io::session::read_source(base_tree.root())
            .ok()
            .flatten()
            .is_none()
    {
        return Err(PrepareStageError::Storage(
            CandidateStorageError::Unsupported(
                "텍스트 원본이 없는 index는 변경 단위로 stage할 수 없습니다(stellar text-source adopt 뒤 다시)",
            ),
        ));
    }
    // 작업 트리 원본의 비공개 사본(stage 원천). 후보를 쓸 때까지 살아 있어야 합니다.
    let mut working_copy = None;
    // Unstage 원천(HEAD) tree는 후보를 쓸 때까지 살아 있어야 합니다.
    let mut head_tree = None;
    let mut head_entries: Vec<SnapshotEntry> = Vec::new();
    let (semantic, head, source_project, source_semantic) = match request.operation {
        SelectionOperation::Stage => {
            // 작업 트리는 원본(Git 원본)을 기준으로 엽니다. cache가 원본과 짝이 아니어도(Git pull·clone 직후) 같은
            // 의미입니다. 원본이 없으면 옛 chunk 작업 트리의 cache를 엽니다.
            let copy = stellar_project::cache::text_snapshot(worktree, &canonical_parent).map_err(
                |e| PrepareStageError::Storage(CandidateStorageError::Invalid(e.to_string())),
            )?;
            let working = match &copy {
                Some(copy) => loader.open(copy.path()).map_err(PrepareStageError::Open)?,
                None => SavedProjectLoader::new()
                    .open(worktree)
                    .map_err(PrepareStageError::Open)?,
            };
            working_copy = copy;
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
            let committed = open_snapshot_tree(&loader, tree.root())?;
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

    // 컴포넌트 정의·token 참조의 필수 동반 단위(UX.5.4, ADR-0127 결정 4). 자동으로 포함하지 않습니다.
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
    let index_tokens = token_files(base_tree.root())?;
    let source_tokens = token_files(source_root)?;
    let inputs = CompanionInputs {
        has_changes,
        base_refs: &base_refs,
        selection_refs: &selection_refs,
        source_refs: &source_refs,
        index_defs: &index_defs,
        source_defs: &source_defs,
        index_tokens: &index_tokens,
        source_tokens: &source_tokens,
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
    // 이때 바꾸는 경로는 generation 저장 경로뿐입니다. 고르지 않은 `design/` 아래 다른 파일은 포함하지 않습니다.
    let implicit_generation = has_changes && semantic.snapshot() == &source_semantic;
    if implicit_generation {
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
        // chunk cache는 Git 밖입니다(ADR-0122). 후보 index에는 원본만 넣습니다.
        if stellar_project::cache::is_cache_path(path.as_bytes()) {
            continue;
        }
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
    // index에 텍스트 원본이 있으면 원본(Git 원본)도 후보 문서로 다시 씁니다(ADR-0113 결정 6). cache generation만
    // stage하면 commit의 원본과 cache가 갈라집니다. 컴포넌트 원본은 원본 단위가 따로 다룹니다.
    if has_changes {
        for (path, text) in text_candidate_files(base_tree.root(), &semantic)? {
            match text {
                Some(bytes) => {
                    // index에 있던 원본 파일은 mode(실행 bit)를 그대로 둡니다.
                    let kind = index_by_path
                        .get(&path)
                        .and_then(|entry| entry.kind())
                        .filter(|kind| {
                            matches!(kind, FileModeKind::Regular | FileModeKind::Executable)
                        })
                        .unwrap_or(FileModeKind::Regular);
                    files.insert(path.clone(), CandidateFile::Bytes { path, kind, bytes });
                }
                None => {
                    files.remove(&path);
                }
            }
        }
    }

    // 원본 단위: 단위 경로를 원천(stage: 저장된 작업 트리, unstage: HEAD)과 같게 만듭니다.
    if !units.is_empty() {
        // generation 묶음(문서 전체)은 텍스트 원본 프로젝트에서 문서를 이루는 원본 파일(컴포넌트 제외)도 덮습니다.
        // 원본 파일 하나하나도 원본 단위이지만, 문서 전체를 고르는 선택은 옛 배치와 같은 뜻이어야 하기 때문입니다
        // (ADR-0122 결정 5). 암묵 generation(변경 ID 선택 결과가 원천과 같음)은 옛 배치 경로만 덮습니다.
        let covered = |path: &GitPath| {
            units.iter().any(|u| {
                let bundle =
                    *u == SourceUnit::Generation && text_source_unit(path.as_bytes()).is_some();
                (u.contains(path.as_bytes()) || bundle)
                    && (!implicit_generation
                        || *u != SourceUnit::Generation
                        || is_generation_storage(path.as_bytes()))
            })
        };
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
    // 옛 배치 index(원본 없음)의 generation 묶음 선택은 원천 의미 그대로가 기대값입니다. 원본 index에서 원본 단위를
    // 고르면(파일 단위 stage) 기대값은 후보 tree의 원본이 읽히는 의미입니다(아래, ADR-0122 결정 5).
    let text_index = stellar_project::document_io::session::read_source(base_tree.root())
        .ok()
        .flatten()
        .is_some();
    let text_units = text_index
        && units
            .iter()
            .any(|u| !(implicit_generation && *u == SourceUnit::Generation));
    let generation =
        (!text_index && units.contains(&SourceUnit::Generation)).then(|| source_semantic.clone());

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
    let reopened = open_snapshot_tree(&loader, tree.root())?;
    drop(working_copy);
    let reopened_semantic =
        saved_semantic_snapshot(&reopened).map_err(PrepareStageError::Storage)?;
    let generation = if text_units {
        Some(reopened_semantic.clone())
    } else {
        generation
    };
    let expected = generation.as_ref().unwrap_or_else(|| semantic.snapshot());
    if reopened_semantic != *expected {
        return Err(PrepareStageError::ReopenMismatch);
    }
    // 후보 tree의 텍스트 원본도 같은 의미여야 합니다(ADR-0113 결정 6).
    if has_changes {
        let text = crate::snapshot::source_semantic(tree.root()).map_err(|_| {
            PrepareStageError::Storage(CandidateStorageError::Unsupported(
                "후보 텍스트 원본을 읽을 수 없습니다",
            ))
        })?;
        if text.is_some_and(|text| text != *expected) {
            return Err(PrepareStageError::ReopenMismatch);
        }
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
        let loaded = open_snapshot_tree(
            &SavedProjectLoader::for_materialized_snapshot(),
            tree.root(),
        )?;
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
    /// page 크기가 1..=[`MAX_STATUS_LIMIT`] 밖입니다.
    InvalidLimit(usize),
    /// staged snapshot이 유효한 Stellar 프로젝트가 아니라 `git commit`을 실행하지 않았습니다(XIV.8.1).
    InvalidStagedSnapshot(Vec<StagedFinding>),
    /// 재검증 뒤 commit 전에 index가 바뀌어 `git commit`을 실행하지 않았습니다.
    IndexChangedBeforeCommit,
    /// commit 실패(identity 누락·hook 실패 등 Git 결과 그대로, XIV.8.5).
    Commit(CommitError),
    /// 옛 배치 chunk cache를 옮기거나 index에서 빼지 못했습니다(ADR-0122).
    Cache(String),
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
            Self::Cache(e) => write!(f, "chunk cache 정리 실패: {e}"),
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
            Self::InvalidLimit(limit) => {
                write!(f, "page 크기 {limit}는 1..={MAX_STATUS_LIMIT}이어야 합니다")
            }
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

    /// 옛 배치 chunk cache(ADR-0122 이전 `design/current`·`design/generations/`·`design/objects/`)를 정리합니다:
    /// 작업 트리의 옛 배치를 cache 위치로 옮기고(`stellar_project::cache::migrate_layout`), 텍스트 원본이 있으면 그
    /// 경로를 index에서 뺍니다(`untrack_legacy`, 작업 트리·HEAD는 그대로). 그래서 다음 commit이 옛 cache를 Git에서
    /// 지웁니다. stage·unstage·commit이 먼저 부릅니다. 돌려주는 값은 index에서 뺀 경로입니다.
    pub fn migrate_cache(&self) -> Result<Vec<String>, GitError> {
        self.git.runner()?;
        stellar_project::cache::migrate_layout(&self.root)
            .map_err(|e| GitError::Cache(e.to_string()))?;
        stellar_project::cache::untrack_legacy(&self.git, &self.root)
            .map_err(|e| GitError::Cache(e.to_string()))
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

/// materialize한 tree(또는 작업 트리)의 저장 semantic snapshot. 텍스트 원본이 있으면 원본(Git 원본)을 읽고
/// (ADR-0113 결정 6), 없으면 generation을 읽습니다(옛 commit의 옛 배치는 먼저 cache 위치로 옮김, ADR-0122). 둘 다
/// 없으면 빈 snapshot입니다.
fn semantic_of(root: &Path, loader: &SavedProjectLoader) -> Result<SemanticSnapshot, String> {
    if let Some(semantic) = crate::snapshot::source_semantic(root)? {
        return Ok(semantic);
    }
    stellar_project::cache::migrate_layout(root).map_err(|e| e.to_string())?;
    if !stellar_storage::generation::current::current_pointer_path(root).exists() {
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
    /// 내용 hash로 이름 붙은 immutable 경로(옛 배치 `design/objects/`, `design/generations/`)는 경로만 넣습니다.
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
            if is_generation_storage(bytes)
                && bytes != stellar_project::cache::LEGACY_PATHS[0].as_bytes()
            {
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
            return Err(GitError::InvalidLimit(request.limit));
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
        self.migrate_cache()?;
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

impl From<crate::validate::Finding> for StagedFinding {
    fn from(finding: crate::validate::Finding) -> StagedFinding {
        StagedFinding {
            error: finding.severity == crate::validate::FindingSeverity::Error,
            code: finding.code,
            document: Some(finding.document),
            message: finding.message,
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

impl GitSession {
    /// staged snapshot이 유효한 Stellar 프로젝트인지 전체 validator로 다시 검사합니다(XIV.8.1).
    ///
    /// 저장소 무결성(loader), 문서 판정, 저장 무손실 왕복, semantic 구조 무결성, 컴포넌트 원본을 봅니다.
    /// 문서 판정은 `stellar validate`와 같은 [`crate::validate::validate_loaded_document`]라 같은 tree의 문서 진단이
    /// 같습니다. 미해결 참조와 정의 없는 인스턴스는 경고이고 commit을 막지 않습니다. index·작업 트리를 쓰지 않습니다.
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
        // staged tree에는 cache가 없으므로 staged 원본으로 만듭니다(ADR-0122). 옛 배치만 있는 staged tree는 옮겨 엽니다.
        let opened = stellar_project::cache::prepare_snapshot(root)
            .map_err(|e| e.to_string())
            .and_then(|_| {
                SavedProjectLoader::for_materialized_snapshot()
                    .open(root)
                    .map_err(|e| e.to_string())
            });
        let project = match opened {
            Ok(project) => project,
            Err(e) => {
                findings.push(StagedFinding::error("storage-invalid", None, e));
                return Ok(StagedValidation { index, findings });
            }
        };
        if index
            .entries
            .iter()
            .any(|entry| is_generation_storage(entry.path.as_bytes()))
        {
            findings.push(StagedFinding {
                error: false,
                code: "legacy-cache-tracked",
                document: None,
                message: "옛 배치 chunk cache(design/current 등)가 index에 있습니다(stellar cache migrate로 뺌)"
                    .into(),
            });
        }
        // 컴포넌트 원본은 문서와 별개로 프로젝트 단위 검사이고, 읽은 정의는 문서 판정의 인스턴스 대조에 씁니다.
        let definitions: Result<BTreeSet<StableId>, String> =
            match read_project_library(root, ExternalAccess::MetadataOnly) {
                Ok((library, _)) => Ok(library.iter().map(|e| e.definition().id).collect()),
                Err(e) => {
                    findings.push(StagedFinding::error(
                        "component-library",
                        None,
                        e.to_string(),
                    ));
                    Err(e.to_string())
                }
            };
        match project.document_ids() {
            Ok(ids) => {
                for id in ids {
                    let loaded = project
                        .load_document(id)
                        .map_err(|e| e.to_string())
                        .and_then(|d| d.model.to_records().map_err(|e| e.to_string()))
                        .map(|mut r| {
                            r.sort_canonical();
                            r
                        });
                    // 문서 판정은 `stellar validate`와 같은 함수입니다(ops::validate, 이중 구현 없음). 두 경로는
                    // 문서를 어디서 읽는지만 다릅니다.
                    let judged = crate::validate::validate_loaded_document(
                        id,
                        loaded.as_ref().map_err(String::as_str),
                        || match &definitions {
                            Ok(ids) => crate::validate::Definitions::Known(ids),
                            Err(reason) => crate::validate::Definitions::Failed(reason),
                        },
                    );
                    findings.extend(judged.findings.into_iter().map(StagedFinding::from));
                }
            }
            Err(e) => findings.push(StagedFinding::error("document-load", None, e.to_string())),
        }
        // 텍스트 원본(Git 원본)이 있으면 staged 원본이 읽혀야 합니다(ADR-0113 결정 6). staged cache와 의미가 다르면
        // 경고입니다: 원본이 이기므로(stamp 없는 clone은 원본에서 읽음) commit은 막지 않습니다.
        let cache_semantic = saved_semantic_snapshot(&project);
        match crate::snapshot::source_semantic(root) {
            Ok(Some(text)) => {
                if cache_semantic.as_ref().is_ok_and(|cache| *cache != text) {
                    findings.push(StagedFinding {
                        error: false,
                        code: "text-source-cache-differs",
                        document: None,
                        message: "staged 텍스트 원본과 staged cache generation의 의미가 다릅니다(원본이 기준)"
                            .into(),
                    });
                }
            }
            Ok(None) => {}
            Err(e) => findings.push(StagedFinding::error("text-source-invalid", None, e)),
        }
        match cache_semantic {
            Ok(semantic) => {
                // 참조 대상이 있는지는 위의 문서 판정(`validate_records`)만 봅니다. 미해결 참조(III.2.4)와 정의 없는
                // 인스턴스는 `stellar validate`처럼 경고이고, 같은 사실을 semantic 무결성에서 다시 판정해 오류로
                // 만들지 않습니다. semantic 무결성은 구조(parent·순서·소속·interface)만 봅니다.
                for violation in stellar_semantic_diff::validate::validate_snapshot(&semantic)
                    .into_iter()
                    .filter(|v| {
                        !matches!(
                            v,
                            stellar_semantic_diff::validate::SnapshotViolation::DanglingReference { .. }
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
        self.migrate_cache()?;
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
        // 경로 map으로 대조합니다. generation 묶음의 object가 수천 개여도 항목마다 전체를 훑지 않습니다.
        let by_path = |entries: &[SnapshotEntry]| -> BTreeMap<GitPath, SnapshotEntry> {
            entries
                .iter()
                .map(|e| (e.path.clone(), e.clone()))
                .collect()
        };
        let tree = by_path(&committed.entries);
        let now = by_path(&index.entries);
        let staged_remaining = tree
            .keys()
            .chain(now.keys())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|p| tree.get(*p) != now.get(*p))
            .count();
        let tree_matches_validated = tree == by_path(&validation.index.entries);
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
    use stellar_document::records::NodePayload;

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
        // 텍스트 원본은 의미 단위 파일이고, View의 분할 Frame 파일은 그 View 파일로 묶습니다.
        let file = |p: &[u8]| Some(SourceUnit::File(GitPath::from_bytes(p)));
        for path in [
            &b"design/document.stellar.yaml"[..],
            b"design/assets/catalog.stellar.yaml",
            b"design/tokens/brand.stellar.yaml",
            b"design/pages/home/main.stellar.yaml",
        ] {
            assert_eq!(SourceUnit::of_path(path), file(path));
        }
        assert_eq!(
            SourceUnit::of_path(b"design/pages/home/main/hero.stellar.yaml"),
            file(b"design/pages/home/main.stellar.yaml")
        );
        assert_eq!(
            SourceUnit::of_path(b"design/pages/home/main/hero/deep.stellar.yaml"),
            file(b"design/pages/home/main.stellar.yaml")
        );
        // 분할 파일 경로는 단위 ID가 아닙니다(View 파일 단위가 대신함).
        assert!(!SourceUnit::File(GitPath::from_bytes(
            b"design/pages/home/main/hero.stellar.yaml"
        ))
        .is_well_formed());
        // 원본 배치가 아닌 `design/` 경로는 generation 묶음 그대로입니다.
        for path in [
            &b"design/pages/home.stellar.yaml"[..],
            b"design/tokens/a/b.stellar.yaml",
            b"design/pages/home/notes.txt",
            b"design/generations/1/manifest",
        ] {
            assert_eq!(
                SourceUnit::of_path(path),
                Some(SourceUnit::Generation),
                "{path:?}"
            );
        }
    }

    #[test]
    fn every_encoded_text_source_file_is_a_file_unit() {
        // codec이 실제로 쓰는 경로가 모두 generation 묶음이 아닌 파일 단위입니다(배치 규칙 이중 정의 확인).
        use stellar_project::document_io::source_v4::project::{encode, TextProject};
        let model = crate::test_support::large_document(3);
        let project = TextProject {
            document: model,
            components: Default::default(),
            token_sets: Vec::new(),
            images: Default::default(),
        };
        let encoded = encode(&project).unwrap();
        assert!(encoded.files.keys().any(|p| p.starts_with("pages/")));
        for relative in encoded.files.keys() {
            let path = format!("design/{relative}");
            let unit = SourceUnit::of_path(path.as_bytes());
            assert!(
                matches!(&unit, Some(SourceUnit::File(_))),
                "{path}: {unit:?}"
            );
        }
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

    /// staged 재검증과 `stellar validate`는 같은 tree의 문서를 같은 진단으로 판정합니다(ADR-0080 개정).
    /// 미해결 참조와 정의 없는 인스턴스는 둘 다 경고라 commit을 막지 않습니다. 이미지 node의 asset은 staged
    /// semantic snapshot(문서만 담음)에 없지만, 그 사실만으로 commit을 막지 않습니다.
    #[test]
    fn staged_judgment_matches_project_validation() {
        use crate::core_types::reference::{AssetRef, ComponentRef};
        use crate::core_types::value::Size2;
        use crate::test_support::{
            document_with_unresolved_clips, save_text_document, scaffold_project,
        };
        use stellar_document::component::instance::ComponentInstancePayload;
        use stellar_document::image::ImagePayload;
        use stellar_document::records::NodeRecord;
        use stellar_git_porcelain::fixture::GitFixture;

        let Some(fx) = GitFixture::with_system_git("ops-staged-judgment").unwrap() else {
            panic!("system Git required");
        };
        scaffold_project(fx.work());
        // 미해결 clip 참조와 정의 없는 인스턴스를 함께 가진 문서.
        let mut instance = document_with_unresolved_clips(2);
        let root = instance.definition().pages[0].views[0].root_frame;
        instance
            .insert_node(NodeRecord {
                id: StableId::new_v7(1_960_000_000_000, 0, 1),
                parent: Some(root),
                order_key: instance.next_child_order_key(root),
                visible: true,
                locked: false,
                display_name: "Placed".into(),
                payload: NodePayload::ComponentInstance(ComponentInstancePayload::new(
                    ComponentRef::new(StableId::new_v7(1_960_000_000_000, 0, 2)),
                )),
            })
            .unwrap();
        instance
            .insert_node(NodeRecord {
                id: StableId::new_v7(1_960_000_000_000, 0, 3),
                parent: Some(root),
                order_key: instance.next_child_order_key(root),
                visible: true,
                locked: false,
                display_name: "Photo".into(),
                payload: NodePayload::Image(ImagePayload::new(
                    AssetRef::new(StableId::new_v7(1_960_000_000_000, 0, 4)),
                    Size2::new(4.0, 4.0).unwrap(),
                )),
            })
            .unwrap();
        save_text_document(fx.work(), &instance);
        fx.git(["add", "-A"]).unwrap();

        let project = crate::validate::validate_project(fx.work()).unwrap();
        assert!(project.is_valid(), "{project:?}");
        let expected: Vec<StagedFinding> = project
            .documents
            .into_iter()
            .flat_map(|d| d.findings)
            .map(StagedFinding::from)
            .collect();
        assert!(expected.iter().any(|f| f.code == "unresolved-reference"));
        assert!(expected
            .iter()
            .any(|f| f.code == "missing-component-definition"));

        let session = GitSession::open(fx.gated(), fx.work()).unwrap();
        let staged = session.validate_staged().unwrap();
        let mut found = staged.findings.clone();
        let mut expected = expected;
        let key = |f: &StagedFinding| (f.document, f.code, f.message.clone());
        found.sort_by_key(key);
        expected.sort_by_key(key);
        assert_eq!(found, expected);
        assert!(staged.is_valid());
    }
}
