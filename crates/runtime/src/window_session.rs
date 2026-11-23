//! 창 하나의 프로젝트 편집 session (XII.2.1).
//!
//! 같은 프로젝트를 여러 창에서 열 수 있고 각 창은 독립된 인메모리 문서 상태와 undo/redo stack을
//! 가집니다(기술 아키텍처 §22.2·§23.2). [`ProjectWindowSession`]은 창 하나가 소유하는 그 묶음입니다.
//!
//! | 필드 | 내용 |
//! |---|---|
//! | [`ProjectIdentity`] | 연 프로젝트(project StableId + root 경로, IV.2.3) |
//! | [`WindowSessionId`] | 창 편집 session id(V.9.1, VII.1.1) |
//! | 문서 state | 이 창만의 [`DocumentState`](모델 + revision). 다른 session과 공유하지 않습니다 |
//! | view state | 이 창의 [`WindowViewState`](선택 VII.3.1, 열린 탭·active view·탭별 viewport XII.2.5). history가 함께 소유합니다 |
//! | history | 이 창의 [`WindowHistory`](VII.5.1) |
//! | loaded generation | 창을 열 때 읽은 current generation([`LoadedGeneration`], 첫 저장 전이면 없음) |
//!
//! # 로드
//!
//! [`ProjectWindowSession::load`]는 디스크의 current generation을 검증해 읽고(V.4.4), 그 generation의 첫
//! root 문서를 이 창의 작업 문서로 로드합니다(VI.10.3). 다른 root 문서는 읽지 않으며 저장할 때 base
//! generation의 chunk를 그대로 이어 씁니다. 창 하나가 문서 하나를 편집하고, 문서 전환은 이 type의 범위
//! 밖입니다. current pointer가 없으면(저장한 적 없는 프로젝트) 작업 문서와 loaded generation이 모두
//! 없습니다.
//!
//! 로드할 때마다 디스크에서 새 모델을 만들므로, 같은 프로젝트를 연 두 session도 모델·revision·history·
//! 선택·탭·viewport를 공유하지 않습니다. session은 document thread(유일한 작성자, XII.1.1)의 session 표에서만
//! 살아 있고 `Send`가 아닌 context 밖으로 나가지 않습니다.
//!
//! # Undo·Redo·Save Now (XII.6.4)
//!
//! [`ProjectWindowSession::undo`]·[`ProjectWindowSession::redo`]는 이 창 history만 재생하고 이 창 문서·선택만
//! 바꿉니다(VII.5.2·VII.5.3). [`ProjectWindowSession::save_now`]는 작업 문서를 직렬화해(VI.10.2) loaded
//! generation 위의 SavePlan(V.5.1)을 만들고, 작업 문서가 차지하지 않은 base chunk는 그대로 이어 씁니다. 쓰기 전에
//! 앱·CLI 공용 저장 잠금을 잡고 current pointer가 loaded generation과 같은지 확인합니다(V.5.2). 다르면 [`WindowSessionSaveError::Stale`]로
//! 아무것도 쓰지 않습니다(다른 창·프로세스가 먼저 저장함, 선택지는 XII.7). 바뀐 chunk가 없으면 새 generation을
//! 만들지 않습니다(`written == false`). 성공하면 loaded generation을 새 generation으로 바꿉니다(V.5.4).
//!
//! 저장은 세 단계입니다: [`ProjectWindowSession::capture_save`](document thread에서 문서 레코드·기준 generation만 복사),
//! [`SaveCapture::write`](직렬화·잠금·base 검사·쓰기, 다른 thread에서도 됨), [`ProjectWindowSession::apply_save`](결과 반영).
//! Save Now는 셋을 차례로 실행하고, 자동 저장은 쓰기를 writer thread에서 합니다(XIII.1.4, ADR-0072).
//!
//! # scene camera (XII.4.3)
//!
//! [`ProjectWindowSession::camera`]는 active 탭 viewport와 캔버스 화면 크기로 scene camera 입력
//! ([`Viewport`](stellar_scene::visibility::Viewport): 화면 크기 + 문서 → 화면 변환)을 만듭니다. 가시 후보 계산
//! (XI.3.2)·hit test·pass 계획이 이 값을 씁니다. 이동·확대([`ProjectWindowSession::pan`]·[`ProjectWindowSession::zoom`])는
//! 창 view state의 viewport만 바꾸므로 작업 문서 revision·history·저장 bytes는 그대로입니다.
//!
//! # selection overlay (XII.4.4)
//!
//! [`ProjectWindowSession::selection_overlay`]는 이 창의 화면에 보일 선택(영역 선택 중이면 preview)과 camera로 문서 scene
//! 위에 그릴 transient scene layer([`SelectionOverlay`])를 만듭니다. overlay는 작업 문서·scene snapshot·저장 경로에
//! 들어가지 않습니다.
//!
//! # 텍스트 원본과 chunk cache (V.10.7, ADR-0088)
//!
//! 기본 원본은 텍스트 원본(`design/**/*.stellar.yaml`)이고 chunk generation은 로컬 cache입니다. 디스크 읽기는
//! [`source::read_disk_state`] 하나가 stamp 짝으로 계획([`OpenPlan`])을 정해 합니다. 원본과 cache가 짝이면 cache에서
//! 작업 문서만 읽고(빠른 열기·부분 로딩), 원본이 움직였으면 원본을 읽고 cache를 다시 만들며, cache만 움직였으면
//! (CLI·MCP의 generation 저장) cache를 읽고 원본을 맞춥니다. 원본이 없는 기존 chunk 프로젝트는 열 때 검증한 변환이
//! 있을 때만 원본으로 옮깁니다. 변환이 검증되지 않으면 그 session은 chunk 전용으로 남습니다.
//!
//! 저장([`SaveCapture::write`])은 같은 잠금 안에서 generation base 확인 → 원본(바뀐 파일만 저장 journal 한 거래, 기대
//! hash = 원본 base) → cache generation → stamp 순서입니다. 원본이 외부에서 바뀌었으면
//! [`WindowSessionSaveError::SourceStale`]이고 아무것도 쓰지 않습니다. cache 쓰기가 실패해도 원본 저장은 성공으로
//! 반영하고 [`SaveOutcome::cache_stale`]로 알립니다(다음 열기가 cache를 다시 만듭니다).
//!
//! 열 때 `stellar.yaml`의 활성 기본값 세트 이름을 모델 거울에 넣고(ADR-0082 결정 4), 저장할 때 거울이 열 때·마지막
//! 저장 때의 값과 다르면 같은 잠금 안에서 `stellar.yaml`에 되씁니다.
//!
//! # view state (XII.2.5)
//!
//! 작업 문서를 로드하면 문서의 첫 View(첫 페이지의 첫 View) 탭을 기본 viewport로 열어 active view로 둡니다. 탭
//! 열기·닫기·viewport 변경은 이 창의 [`WindowViewState`]만 바꾸고 문서 revision·history·저장 bytes에는 영향이
//! 없습니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_commands::change::ChangeSet;
use stellar_commands::command::CommandError;
use stellar_commands::id::CommandId;
use stellar_commands::selection::{Selection, SetSelection, WindowViewState};
use stellar_commands::snapshot_apply::ApplyExternalSnapshot;
use stellar_commands::state::DocumentState;
use stellar_commands::transaction::CommandTransaction;
use stellar_commands::view_tabs::{
    first_view, CanvasViewport, ViewTabs, ViewTabsError, ViewportError,
};
use stellar_core_types::ids::{StableId, WindowSessionId};
use stellar_core_types::revision::DocumentRevision;
use stellar_core_types::value::Size2;
use stellar_document::model::DocumentModel;
use stellar_document::records::DocumentRecords;
use stellar_history::entry::{HistoryError, HistorySequence};
use stellar_history::window::WindowHistory;
use stellar_ops::lock::{default_lock_dir, LockError, SaveLock};
use stellar_project::document_io::load::{load_document_from_generation, LoadDocumentError};
use stellar_project::document_io::serialize::{
    serialize_records, SerializeError, SerializedDocument,
};
use stellar_project::document_io::session::{
    commit_files, decode_source, plan_changes, read_source, read_stamp, write_stamp, OpenPlan,
    SourceBase, SourceCommitError, SourceLibrary, Stamp,
};
use stellar_project::external_change::source::SourceDiagnostic;
use stellar_project::identity::ProjectIdentity;
use stellar_project::manifest::CURRENT_SCHEMA_VERSION;
use stellar_project::migration::{decide, MigrationPolicy, SchemaDecision};
use stellar_project::window_session::{
    record_app_window_save, writer_provenance, StaleResolution, StaleSaveChoice, WriterProvenance,
};
mod source;
use source::{
    document_source_moved, library_moved, read_disk_state, DiskState, SourceMark, SourceMode,
};

use stellar_scene::overlay::{
    build_selection_overlay, OverlayError, OverlayStyle, SelectionOverlay,
};
use stellar_scene::snapshot::SceneSnapshot;
use stellar_scene::visibility::Viewport;
use stellar_storage::chunk::id::ChunkId;
use stellar_storage::chunk::planner::LogicalChunkState;
use stellar_storage::chunk::pointer::ChunkPointer;
use stellar_storage::generation::current::{read_current_pointer, CurrentPointerError};
use stellar_storage::generation::lazy::{
    ChunkReadRecorder, LazyGeneration, LazyLoadError, NoopRecorder,
};
use stellar_storage::generation::loader::{load_current_generation, LoadError, LoadedGeneration};
use stellar_storage::hash::ContentHash;
use stellar_storage::save::cancel::{run_save, CancelToken, SaveRunError};
use stellar_storage::save::commit::SavedGenerationState;
use stellar_storage::save::optimistic::{
    check_base_generation, check_base_manifest, BaseCheckError,
};
use stellar_storage::save::plan::{build_save_plan, SavePlanError};

/// 창 메뉴의 history 동작 이름표. 값은 셸의 영어 이름 표와 같습니다.
///
/// 이름표는 기록하는 함수([`ProjectWindowSession::apply`] 등)의 인자로 받아 기록과 같은 호출에서 붙입니다. 표에는
/// 지금 undo·redo 스택에 있는 sequence만 남깁니다.
pub(crate) mod history_label {
    /// 이름표 없음(셸은 `Undo`·`Redo`를 씁니다).
    pub const NONE: u8 = 0;
    pub const INSERT: u8 = 1;
    pub const PASTE: u8 = 2;
    pub const DELETE: u8 = 3;
    pub const CUT: u8 = 4;
    pub const DUPLICATE: u8 = 5;
    pub const MOVE: u8 = 6;
    pub const RESIZE: u8 = 7;
    pub const CHANGE_PROPERTY: u8 = 8;
    pub const RENAME: u8 = 9;
    pub const REORDER: u8 = 10;
    pub const TYPING: u8 = 13;
    pub const SHOW_HIDE: u8 = 14;
    pub const LOCK_UNLOCK: u8 = 15;
}

/// session 로드 실패.
#[derive(Debug)]
pub enum WindowSessionLoadError {
    /// current generation을 열거나 검증하지 못했습니다(누락 manifest·chunk, 다른 project id 등).
    Generation(LazyLoadError),
    /// 두 번 읽은 current generation이 서로 다릅니다(읽는 사이 다른 저장이 끝남). 다시 열면 됩니다.
    ConcurrentSave,
    /// 작업 문서를 chunk에서 로드하지 못했습니다.
    Document(LoadDocumentError),
    /// generation schema가 이 도구의 편집 schema가 아닙니다(IX.11.21). 높은 schema·경로 없는 schema는 거부이고,
    /// 낮은 schema의 메모리 migration 결과는 읽기 전용이라 편집 session으로 열지 않습니다.
    Schema(SchemaDecision),
    /// 텍스트 원본을 읽거나 검증하지 못했습니다(파일·줄 진단, ADR-0088). 원본이 Git 원본이므로 cache로 대신 열지
    /// 않습니다.
    Source(SourceDiagnostic),
}

impl fmt::Display for WindowSessionLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WindowSessionLoadError::Generation(error) => {
                write!(f, "저장된 generation을 열지 못했습니다: {error}")
            }
            WindowSessionLoadError::ConcurrentSave => {
                f.write_str("여는 동안 다른 저장이 끝나 generation이 바뀌었습니다")
            }
            WindowSessionLoadError::Document(error) => {
                write!(f, "문서를 로드하지 못했습니다: {error}")
            }
            WindowSessionLoadError::Schema(SchemaDecision::Reject(rejection)) => {
                write!(f, "{rejection}")
            }
            WindowSessionLoadError::Schema(decision) => {
                write!(f, "편집할 수 없는 generation schema입니다: {decision:?}")
            }
            WindowSessionLoadError::Source(diagnostic) => {
                write!(f, "텍스트 원본 오류: {diagnostic}")
            }
        }
    }
}

impl std::error::Error for WindowSessionLoadError {}

/// Save Now 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveOutcome {
    /// 저장 뒤 loaded generation 번호(저장한 적 없고 쓸 것도 없으면 0).
    pub generation: u64,
    /// 새 generation을 썼는지. 바뀐 chunk가 없거나 작업 문서가 없으면 `false`.
    pub written: bool,
    /// 텍스트 원본에서 쓰거나 지운 파일 수(ADR-0088).
    pub source_files: usize,
    /// 원본은 저장했지만 cache generation·stamp를 쓰지 못했는지. 다음 열기가 원본에서 cache를 다시 만듭니다.
    pub cache_stale: bool,
}

/// Save Now 실패. loaded generation과 디스크는 저장 전 그대로입니다(commit 전 실패).
#[derive(Debug)]
pub enum WindowSessionSaveError {
    /// 이미지 asset record 인코딩 실패.
    ImageMetadata,
    /// 다른 writer가 잠금을 사용 중이거나 OS 사용자 데이터의 잠금을 열 수 없습니다.
    Lock(LockError),
    /// current pointer가 loaded generation과 다릅니다(다른 저장이 먼저 끝남). 아무것도 쓰지 않았습니다.
    Stale(BaseCheckError),
    /// 작업 문서를 chunk로 직렬화하지 못했습니다.
    Serialize(SerializeError),
    /// SavePlan을 만들지 못했습니다.
    Plan(SavePlanError),
    /// 저장 실행(chunk·manifest 기록, current pointer 전환)에 실패했습니다.
    Run(SaveRunError),
    /// 저장은 끝났지만 새 current generation을 다시 읽지 못했습니다.
    Reload(LoadError),
    /// 자동 저장 쓰기가 끝나지 않았습니다(worker 중단, 또는 Save Now가 기다리는 제한 시간 초과). 편집은 메모리에
    /// 남습니다.
    Interrupted,
    /// UseCurrentWindow가 새 base로 삼을 최신 generation을 읽지 못했습니다(XII.7.5). 아무것도 쓰지 않았습니다.
    Rebase(Box<WindowSessionLoadError>),
    /// 텍스트 원본을 base로 읽은 뒤 외부가 바꾼 파일이 있습니다(`design/` 기준). 아무것도 쓰지 않았습니다.
    SourceStale {
        /// 바뀐 파일.
        paths: Vec<String>,
    },
    /// 텍스트 원본을 인코딩·읽기·쓰기하지 못했습니다. 아무것도 쓰지 않았습니다.
    Source(String),
    /// 활성 기본값 세트를 `stellar.yaml`에 되쓰지 못했습니다. 아무것도 쓰지 않았습니다.
    StyleDefaults(String),
}

impl fmt::Display for WindowSessionSaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WindowSessionSaveError::ImageMetadata => {
                f.write_str("이미지 에셋 메타데이터 인코딩 실패")
            }
            WindowSessionSaveError::Lock(error) => write!(f, "{error}"),
            WindowSessionSaveError::Stale(error) => {
                write!(f, "다른 저장이 먼저 끝나 저장하지 않았습니다: {error}")
            }
            WindowSessionSaveError::Serialize(error) => write!(f, "문서 직렬화 실패: {error}"),
            WindowSessionSaveError::Plan(error) => write!(f, "저장 계획 실패: {error}"),
            WindowSessionSaveError::Run(error) => write!(f, "저장 실패: {error}"),
            WindowSessionSaveError::Reload(error) => {
                write!(f, "저장 뒤 generation을 다시 읽지 못했습니다: {error}")
            }
            WindowSessionSaveError::Interrupted => {
                f.write_str("진행 중인 자동 저장이 끝나지 않았습니다")
            }
            WindowSessionSaveError::Rebase(error) => {
                write!(
                    f,
                    "최신 디스크 generation을 읽지 못해 저장하지 않았습니다: {error}"
                )
            }
            WindowSessionSaveError::SourceStale { paths } => {
                write!(
                    f,
                    "외부에서 바뀐 텍스트 원본이 있어 저장하지 않았습니다: {}",
                    paths.join(", ")
                )
            }
            WindowSessionSaveError::Source(error) => write!(f, "텍스트 원본 저장 실패: {error}"),
            WindowSessionSaveError::StyleDefaults(error) => {
                write!(f, "기본값 세트 선택을 저장하지 못했습니다: {error}")
            }
        }
    }
}

impl std::error::Error for WindowSessionSaveError {}

impl WindowSessionSaveError {
    /// 다른 writer가 먼저 저장해 쓰지 않은 실패인지(generation base 또는 텍스트 원본 base, XII.7).
    pub fn is_stale(&self) -> bool {
        matches!(
            self,
            WindowSessionSaveError::Stale(_) | WindowSessionSaveError::SourceStale { .. }
        )
    }
}

impl SaveOutcome {
    /// 디스크에 무엇이든 썼는지(새 generation 또는 원본 파일).
    pub fn wrote_anything(&self) -> bool {
        self.written || self.source_files > 0
    }
}

/// 같은 작업 문서 revision의 불변 memory snapshot(XIII.8.2). 자동 저장과 복구 체크포인트가 함께 씁니다.
///
/// document thread는 revision마다 레코드를 한 번만 복사하고([`ProjectWindowSession::shared_snapshot`]), chunk
/// 직렬화는 먼저 필요한 worker가 한 번만 합니다([`SharedSnapshot::serialized`]). 그래서 같은 revision을 자동 저장과
/// 복구가 모두 기록해도 문서 직렬화는 한 번입니다.
#[derive(Debug)]
pub(crate) struct SharedSnapshot {
    document_id: StableId,
    revision: DocumentRevision,
    records: DocumentRecords,
    serialized: std::sync::OnceLock<Result<SerializedDocument, SerializeError>>,
    serializations: std::sync::atomic::AtomicUsize,
}

impl SharedSnapshot {
    /// 캡처한 revision.
    #[cfg(test)]
    pub(crate) fn revision(&self) -> DocumentRevision {
        self.revision
    }

    /// 문서 레코드.
    pub(crate) fn records(&self) -> &DocumentRecords {
        &self.records
    }

    /// chunk 직렬화 결과. 처음 부른 thread가 한 번만 직렬화합니다.
    pub(crate) fn serialized(&self) -> Result<&SerializedDocument, SerializeError> {
        self.serialized
            .get_or_init(|| {
                self.serializations
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                serialize_records(&self.records)
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    /// 이 snapshot을 직렬화한 횟수(0 또는 1).
    #[cfg(test)]
    pub(crate) fn serialization_count(&self) -> usize {
        self.serializations
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// [`SaveCapture::write_timed`]가 잰 writer 쪽 시간(XIII.1.7).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SaveTimings {
    /// 문서 레코드 → chunk 직렬화.
    pub(crate) serialize: std::time::Duration,
    /// 계획·잠금·base 검사·chunk 쓰기·current pointer 교체.
    pub(crate) write: std::time::Duration,
}

/// 저장 snapshot의 텍스트 원본 부분(ADR-0088).
#[derive(Debug, Clone)]
pub(crate) struct TextCapture {
    /// 캡처할 때의 원본 base(journal 기대 hash).
    base: SourceBase,
    /// 문서 밖 의미. `None`이면 writer가 base와 같은 디스크 원본에서 읽습니다.
    library: Option<std::sync::Arc<SourceLibrary>>,
}

/// 저장 snapshot([`ProjectWindowSession::capture_save`]). document thread에서 문서 레코드와 기준 generation만
/// 복사해 두고, 직렬화·원본 인코딩·쓰기는 [`SaveCapture::write`]가 다른 thread에서 합니다(XIII.1.4).
#[derive(Debug)]
pub(crate) struct SaveCapture {
    identity: ProjectIdentity,
    base: Option<LoadedGeneration>,
    revision: DocumentRevision,
    snapshot: std::sync::Arc<SharedSnapshot>,
    owned_chunks: BTreeSet<ChunkId>,
    image_records:
        std::sync::Arc<std::collections::BTreeMap<StableId, stellar_assets::record::AssetRecord>>,
    /// 텍스트 원본 session이면 원본 부분(chunk 전용이면 `None`).
    text: Option<TextCapture>,
    /// (모델 거울의 활성 기본값 세트, 열 때·마지막 저장 때의 값). 다르면 `stellar.yaml`에 되씁니다.
    style_defaults: (String, String),
}

/// 쓴 원본 상태([`SaveWritten`] 안).
#[derive(Debug)]
pub(crate) struct TextWritten {
    /// 캡처할 때의 base(반영 때 session base와 같아야 함).
    before: SourceBase,
    /// 저장 뒤 base.
    after: SourceBase,
    library: std::sync::Arc<SourceLibrary>,
    /// 쓰거나 지운 파일 수.
    files: usize,
}

/// [`SaveCapture::write`] 결과. [`ProjectWindowSession::apply_save`]가 document thread에서 반영합니다.
#[derive(Debug)]
pub(crate) struct SaveWritten {
    /// 캡처한 기준 generation의 manifest hash.
    base: Option<ContentHash>,
    document_id: StableId,
    /// 디스크와 내용이 같아진 작업 문서 revision.
    revision: DocumentRevision,
    /// 새로 쓴 generation과 작업 문서가 차지한 chunk(쓸 것이 없었으면 `None`).
    saved: Option<(LoadedGeneration, BTreeSet<ChunkId>)>,
    /// 텍스트 원본 결과(chunk 전용이면 `None`).
    text: Option<TextWritten>,
    /// 저장한 활성 기본값 세트 이름.
    style_defaults: String,
    /// 원본은 저장했지만 cache generation·stamp를 쓰지 못함.
    cache_stale: bool,
}

impl SaveWritten {
    /// 새 generation이나 원본 파일을 썼는지.
    pub(crate) fn wrote(&self) -> bool {
        self.saved.is_some() || self.text.as_ref().is_some_and(|text| text.files > 0)
    }
}

/// writer가 인코딩한 원본.
struct EncodedSource {
    capture: TextCapture,
    library: std::sync::Arc<SourceLibrary>,
    files: BTreeMap<String, String>,
}

fn source_error(error: impl fmt::Display) -> WindowSessionSaveError {
    WindowSessionSaveError::Source(error.to_string())
}

impl SaveCapture {
    /// 캡처한 revision.
    pub(crate) fn revision(&self) -> DocumentRevision {
        self.revision
    }

    /// 캡처한 불변 memory snapshot(복구 체크포인트와 공유, XIII.8.2).
    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> &std::sync::Arc<SharedSnapshot> {
        &self.snapshot
    }

    /// 직렬화하고 loaded generation 위의 SavePlan을 씁니다. 공용 저장 잠금 안에서 current pointer가 캡처한 기준
    /// generation과 다르면 [`WindowSessionSaveError::Stale`]로 아무것도 쓰지 않습니다(current pointer를 바꾸지 않음).
    /// 텍스트 원본 session이면 원본을 먼저 씁니다(모듈 문서 "텍스트 원본과 chunk cache").
    pub(crate) fn write(
        self,
        lock_dir: impl FnOnce() -> Result<std::path::PathBuf, LockError>,
    ) -> Result<SaveWritten, WindowSessionSaveError> {
        self.write_timed(lock_dir).0
    }

    /// [`SaveCapture::write`]와 같고 직렬화·쓰기 시간을 함께 돌려줍니다(XIII.1.7).
    pub(crate) fn write_timed(
        self,
        lock_dir: impl FnOnce() -> Result<std::path::PathBuf, LockError>,
    ) -> (Result<SaveWritten, WindowSessionSaveError>, SaveTimings) {
        let mut timings = SaveTimings::default();
        let result = self.write_measured(lock_dir, &mut timings);
        (result, timings)
    }

    fn write_measured(
        self,
        lock_dir: impl FnOnce() -> Result<std::path::PathBuf, LockError>,
        timings: &mut SaveTimings,
    ) -> Result<SaveWritten, WindowSessionSaveError> {
        let root = self.identity.canonical_target();
        let base = self.base.as_ref();
        let started = std::time::Instant::now();
        let serialized = self.snapshot.serialized();
        let source = match self.text.clone() {
            Some(text) => match self.encode_source(root, text) {
                Ok(source) => Some(source),
                // 원본과 generation이 함께 움직였으면(다른 앱 창의 저장) generation stale로 알립니다. 두 선택지 판정이
                // generation writer 기록을 먼저 보기 때문입니다.
                Err(error) if error.is_stale() => {
                    check_base_manifest(root, base.map(|base| base.manifest_hash))
                        .map_err(WindowSessionSaveError::Stale)?;
                    return Err(error);
                }
                Err(error) => return Err(error),
            },
            None => None,
        };
        timings.serialize = started.elapsed();
        let started = std::time::Instant::now();
        let result = self.write_serialized(root, base, serialized, source, lock_dir);
        timings.write = started.elapsed();
        result
    }

    /// 문서 레코드와 문서 밖 의미로 원본 파일 집합을 만듭니다. 문서 밖 의미를 아직 모르면 디스크 원본을 읽고, 그
    /// 원본이 캡처한 base와 다르면 [`WindowSessionSaveError::SourceStale`]입니다.
    fn encode_source(
        &self,
        root: &std::path::Path,
        mut capture: TextCapture,
    ) -> Result<EncodedSource, WindowSessionSaveError> {
        let mut library = match &capture.library {
            Some(library) => std::sync::Arc::clone(library),
            None => {
                let disk = read_source(root).map_err(source_error)?;
                let now = disk
                    .as_ref()
                    .map(|disk| disk.base.clone())
                    .unwrap_or_default();
                // 문서 밖 의미 파일만 바뀌었으면 디스크 것을 씁니다(그 파일은 이 저장이 바꾸지 않음).
                if now != capture.base && !capture.base.differs_only_in_library(&now) {
                    return Err(WindowSessionSaveError::SourceStale {
                        paths: capture.base.changed_paths(&now).into_iter().collect(),
                    });
                }
                capture.base = now;
                let library = match disk {
                    Some(disk) => {
                        SourceLibrary::split(decode_source(&disk).map_err(source_error)?).1
                    }
                    None => SourceLibrary::default(),
                };
                std::sync::Arc::new(library)
            }
        };
        let records = self.snapshot.records();
        let referenced = referenced_images(records);
        let added: Vec<_> = self
            .image_records
            .iter()
            .filter(|(id, record)| {
                referenced.contains(*id) && library.images.get(*id) != Some(*record)
            })
            .collect();
        if !added.is_empty() {
            let mut next = (*library).clone();
            for (id, record) in added {
                next.images.insert(*id, record.clone());
            }
            library = std::sync::Arc::new(next);
        }
        let (model, _) = DocumentModel::from_records(records.clone()).map_err(source_error)?;
        let files = stellar_project::document_io::source_v4::project::encode(&library.join(model))
            .map_err(source_error)?
            .files;
        Ok(EncodedSource {
            capture,
            library,
            files,
        })
    }

    fn write_serialized(
        &self,
        root: &std::path::Path,
        base: Option<&LoadedGeneration>,
        serialized: Result<&SerializedDocument, SerializeError>,
        source: Option<EncodedSource>,
        lock_dir: impl FnOnce() -> Result<std::path::PathBuf, LockError>,
    ) -> Result<SaveWritten, WindowSessionSaveError> {
        let serialized = serialized.map_err(WindowSessionSaveError::Serialize)?;
        let document_id = serialized.document_id;
        let new_chunks: BTreeSet<ChunkId> = serialized
            .chunks()
            .into_iter()
            .map(|chunk| chunk.chunk_id)
            .collect();
        let mut states = serialized.to_dirty_states();
        // 이미지 asset metadata chunk: 새로 가져온 record와 원본 catalog의 record 가운데 문서가 참조하고 base에 없는
        // 것을 cache에 더합니다(원본에서 다시 만든 cache도 이미지를 그릴 수 있게).
        let referenced = referenced_images(self.snapshot.records());
        let mut images: BTreeMap<StableId, &stellar_assets::record::AssetRecord> = BTreeMap::new();
        if let Some(source) = &source {
            images.extend(
                source
                    .library
                    .images
                    .iter()
                    .map(|(id, record)| (*id, record)),
            );
        }
        images.extend(self.image_records.iter().map(|(id, record)| (*id, record)));
        for (id, record) in images.iter().filter(|(id, _)| referenced.contains(*id)) {
            let chunk_id = ChunkId::from_stable_id(*id);
            if base.is_some_and(|g| {
                g.manifest
                    .chunk_pointers
                    .iter()
                    .any(|p| p.chunk_id == chunk_id)
            }) {
                continue;
            }
            states.push(LogicalChunkState::Dirty {
                chunk_id,
                kind: stellar_storage::chunk::kinds::KIND_ASSET_METADATA,
                schema_version: 1,
                payload: record
                    .encode()
                    .map_err(|_| WindowSessionSaveError::ImageMetadata)?,
            });
        }
        let mut roots = base.map_or_else(Vec::new, |base| base.manifest.root_document_ids.clone());
        if let Some(base) = base {
            // 작업 문서가 차지하지 않은 base chunk(다른 문서·에셋 등)는 그대로 이어 씁니다. 작업 문서가 로드할 때
            // 차지했지만 이번에 만들지 않은 chunk(지운 subtree 등)는 새 generation에서 빠집니다.
            states.extend(
                base.manifest
                    .chunk_pointers
                    .iter()
                    .filter(|pointer| {
                        !self.owned_chunks.contains(&pointer.chunk_id)
                            && !new_chunks.contains(&pointer.chunk_id)
                    })
                    .map(|pointer| LogicalChunkState::Clean {
                        chunk_id: pointer.chunk_id,
                    }),
            );
        }
        if !roots.contains(&document_id) {
            roots.push(document_id);
        }
        let schema_version =
            base.map_or(CURRENT_SCHEMA_VERSION, |base| base.manifest.schema_version);
        let plan = build_save_plan(
            base,
            self.identity.project_id(),
            schema_version,
            &states,
            roots,
        )
        .map_err(WindowSessionSaveError::Plan)?;
        let generation_unchanged = base.is_some_and(|base| {
            plan.dirty_chunks.is_empty()
                && plan.manifest.chunk_pointers == base.manifest.chunk_pointers
                && plan.manifest.root_document_ids == base.manifest.root_document_ids
        });
        let source_unchanged = source
            .as_ref()
            .is_none_or(|source| plan_changes(&source.capture.base, &source.files).is_empty());
        let (style_now, style_base) = &self.style_defaults;
        let style_unchanged = style_now == style_base;
        let base_hash = base.map(|base| base.manifest_hash);
        let next_source = source.as_ref().map(|source| SourceBase::of(&source.files));
        let finish =
            |saved, source: Option<EncodedSource>, files: usize, cache_stale: bool| SaveWritten {
                base: base_hash,
                document_id,
                revision: self.revision,
                saved,
                text: source.map(|source| TextWritten {
                    after: SourceBase::of(&source.files),
                    before: source.capture.base,
                    library: source.library,
                    files,
                }),
                style_defaults: style_now.clone(),
                cache_stale,
            };
        if generation_unchanged && source_unchanged && style_unchanged {
            check_base_generation(root, &plan).map_err(WindowSessionSaveError::Stale)?;
            // 원본과 cache가 이미 짝인데 stamp가 다르면(옛 stamp) 맞춥니다. 실패해도 다음 열기가 다시 맞춥니다.
            let mut cache_stale = false;
            if let (Some(next), Some(manifest)) = (&next_source, base_hash) {
                let stamp = Stamp {
                    source: next.digest(),
                    manifest,
                };
                if read_stamp(root) != Some(stamp) {
                    cache_stale = write_stamp(root, &stamp).is_err();
                }
            }
            return Ok(finish(None, source, 0, cache_stale));
        }
        // 계획과 원본 인코딩은 메모리에서만 만듭니다. 실제 쓰기만 CLI와 직렬화하고 잠금 안에서 base를 확인합니다.
        let dir = lock_dir().map_err(WindowSessionSaveError::Lock)?;
        let _guard =
            SaveLock::for_identity(&dir, &self.identity).map_err(WindowSessionSaveError::Lock)?;
        check_base_generation(root, &plan).map_err(WindowSessionSaveError::Stale)?;
        if !style_unchanged {
            stellar_project::manifest_style_defaults::set_active(root, style_now)
                .map_err(|error| WindowSessionSaveError::StyleDefaults(error.to_string()))?;
        }
        let mut files = 0;
        if let (Some(source), Some(next)) = (&source, &next_source) {
            // 앱 창이 쓰는 원본임을 쓰기 **전에** 기록합니다(다른 창 감시가 이 저장을 외부 writer로 오판하지 않게).
            record_app_window_save(root, next.digest());
            let commit = match commit_files(root, &source.capture.base, &source.files) {
                Ok(commit) => commit,
                Err(SourceCommitError::Stale { paths }) => {
                    return Err(WindowSessionSaveError::SourceStale { paths })
                }
                Err(error) => return Err(source_error(error)),
            };
            files = commit.written + commit.deleted;
        }
        let saved = if generation_unchanged {
            Ok(None)
        } else {
            self.write_generation(root, base, &plan, new_chunks)
        };
        let saved = match saved {
            Ok(saved) => saved,
            // 원본은 이미 저장했습니다. cache 실패는 저장 실패가 아니라 cache가 뒤처진 상태입니다(stamp는 옛 짝이라 다음
            // 열기가 원본에서 다시 만듭니다).
            Err(error) if source.is_some() => {
                eprintln!("Stellar cache generation: {error}");
                return Ok(finish(None, source, files, true));
            }
            Err(error) => return Err(error),
        };
        let mut cache_stale = false;
        if let Some(next) = &next_source {
            let manifest = saved
                .as_ref()
                .map(|(loaded, _)| loaded.manifest_hash)
                .or(base_hash);
            if let Some(manifest) = manifest {
                cache_stale = write_stamp(
                    root,
                    &Stamp {
                        source: next.digest(),
                        manifest,
                    },
                )
                .is_err();
            }
        }
        Ok(finish(saved, source, files, cache_stale))
    }

    /// 잠금 안에서 generation을 씁니다(V.5). 새 generation과 작업 문서 chunk를 돌려줍니다.
    fn write_generation(
        &self,
        root: &std::path::Path,
        base: Option<&LoadedGeneration>,
        plan: &stellar_storage::save::plan::SavePlan,
        new_chunks: BTreeSet<ChunkId>,
    ) -> Result<Option<(LoadedGeneration, BTreeSet<ChunkId>)>, WindowSessionSaveError> {
        let mut memory = SavedGenerationState::new(
            base.map(|base| (base.manifest.generation, base.manifest_hash)),
        );
        // 앱 창이 쓰는 generation임을 current pointer를 바꾸기 **전에** 기록합니다. 다른 창의 감시가 pointer 변화를
        // 먼저 보고 이 저장을 외부 writer로 오판하지 않게 합니다(ADR-0083 결정 2). 쓰기가 실패해 current가 되지
        // 못한 hash가 남아도 판정에 쓰이지 않습니다.
        if let Ok((_, manifest_hash)) = plan.manifest.encode_with_hash() {
            record_app_window_save(root, manifest_hash);
        }
        run_save(root, plan, &mut memory, &CancelToken::new())
            .map_err(WindowSessionSaveError::Run)?;
        // commit(V.5.4)이 성공하면 메모리 상태에 새 (generation, manifest hash)가 들어 있습니다. 없으면 디스크에서
        // 다시 읽습니다.
        let loaded = match memory.saved() {
            Some((_, manifest_hash)) => LoadedGeneration {
                manifest_hash,
                manifest: plan.manifest.clone(),
            },
            None => load_current_generation(root, self.identity.project_id())
                .map_err(WindowSessionSaveError::Reload)?,
        };
        // 미리 기록한 hash와 같아야 합니다. manifest encode가 실패했던 경우를 위해 실제 hash도 기록합니다(중복은 무시).
        record_app_window_save(root, loaded.manifest_hash);
        Ok(Some((loaded, new_chunks)))
    }
}

/// 열기 전에 중단된 원본 저장 거래를 앞 세대 또는 새 세대로 맞춥니다(ADR-0078 복구, ADR-0088 결정 3). 원본을
/// 읽기 전에 해야 반쯤 쓴 파일 집합을 원본으로 읽지 않습니다. journal 폴더가 없으면(원본 저장 전) 아무것도 만들지
/// 않습니다. 다른 저장이 journal을 잡고 있거나(Busy) 복구가 실패해도 열기는 계속합니다. 남은 거래는 다음 저장
/// 거래가 시작할 때 다시 복구합니다.
fn recover_source_journal(identity: &ProjectIdentity) {
    let root = identity.canonical_target();
    if !root.join(".stellar").join("journal").is_dir() {
        return;
    }
    match stellar_storage::journal::recover(root) {
        Ok(_) | Err(stellar_storage::journal::JournalError::Busy) => {}
        Err(error) => eprintln!("Stellar text source journal recovery: {error}"),
    }
}

/// 문서가 참조하는 이미지 asset StableId.
fn referenced_images(records: &DocumentRecords) -> BTreeSet<StableId> {
    records
        .nodes
        .iter()
        .filter_map(|node| match &node.payload {
            stellar_document::records::NodePayload::Image(image) => Some(image.asset.id()),
            _ => None,
        })
        .collect()
}

/// stale 선택 실행 결과([`ProjectWindowSession::resolve_stale`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleResolved {
    /// UseCurrentWindow: 최신 generation 위에 이 창 문서를 썼습니다.
    Saved(SaveOutcome),
    /// UseLatestDisk: 최신 generation을 다시 읽고 로컬 편집·history를 버렸습니다.
    Reloaded {
        /// 새 loaded generation 번호.
        generation: u64,
        /// 버린 history 항목 수(undo + redo).
        discarded_history_entries: usize,
    },
}

/// stale 선택 실행 실패. 이 창 문서·history는 그대로입니다.
#[derive(Debug)]
pub enum StaleResolveError {
    /// 디스크가 이 창의 base와 같습니다(고를 것이 없음).
    NotStale,
    /// 외부 writer가 만든 generation이라 두 선택지 대신 reconciliation을 기다립니다(ADR-0083 결정 2).
    ReconciliationRequired,
    /// UseCurrentWindow 쓰기 실패(다시 stale 포함).
    Save(WindowSessionSaveError),
    /// UseLatestDisk 읽기 실패. 메모리 편집을 유지합니다.
    Load(WindowSessionLoadError),
}

impl fmt::Display for StaleResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StaleResolveError::NotStale => {
                f.write_str("디스크가 이 창의 기준 generation과 같습니다")
            }
            StaleResolveError::ReconciliationRequired => {
                f.write_str("외부 writer의 변경이라 두 선택지 대신 병합을 기다립니다")
            }
            StaleResolveError::Save(error) => write!(f, "{error}"),
            StaleResolveError::Load(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for StaleResolveError {}

/// 외부 변경을 반영하지 않은 이유([`ExternalSync::Rejected`]). 문서·history·base는 그대로입니다.
#[derive(Debug)]
pub enum ExternalRejection {
    /// current pointer를 읽지 못했습니다(형식 오류·I/O).
    Pointer(CurrentPointerError),
    /// current pointer가 사라졌습니다.
    PointerMissing,
    /// 새 generation 검증이나 이 창 문서 읽기에 실패했습니다(손상·누락·schema).
    Load(WindowSessionLoadError),
    /// 새 문서의 레코드를 만들지 못했습니다.
    Records(String),
    /// 레코드 변경 집합을 만들지 못했습니다(다른 문서 등).
    Snapshot(CommandError),
    /// 반영 command가 검증을 통과하지 못했습니다.
    Record(HistoryError),
}

impl fmt::Display for ExternalRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExternalRejection::Pointer(error) => {
                write!(f, "current pointer를 읽지 못했습니다: {error}")
            }
            ExternalRejection::PointerMissing => f.write_str("current pointer가 없습니다"),
            ExternalRejection::Load(error) => write!(f, "{error}"),
            ExternalRejection::Records(error) => {
                write!(f, "외부 문서 레코드를 만들지 못했습니다: {error}")
            }
            ExternalRejection::Snapshot(error) => {
                write!(f, "외부 변경 집합을 만들지 못했습니다: {error}")
            }
            ExternalRejection::Record(error) => {
                write!(f, "외부 변경을 적용하지 못했습니다: {error}")
            }
        }
    }
}

/// [`ProjectWindowSession::sync_external`] 결과(UX.4.1).
#[derive(Debug)]
pub enum ExternalSync {
    /// 디스크가 이 창의 base와 같습니다(자기 저장 event 포함).
    Unchanged,
    /// 제스처가 진행 중이라 미뤘습니다.
    Deferred,
    /// clean 창에 새 generation을 반영했습니다.
    Applied {
        /// 새 loaded generation 번호.
        generation: u64,
        /// [`ApplyExternalSnapshot`]를 기록했으면 그 변경(cache 무효화 입력). 문서 내용이 같아 base만 바꿨거나
        /// 문서 없던 창을 새로 열었으면 `None`입니다(history 항목 없음).
        changes: Option<ChangeSet>,
    },
    /// 미저장 편집이 있는 창의 base가 낡았습니다.
    Stale {
        /// 디스크 generation을 만든 writer.
        writer: WriterProvenance,
        /// 다음 할 일(두 선택지 또는 reconciliation 대기).
        resolution: StaleResolution,
    },
    /// 깨진 외부 변경이라 반영하지 않았습니다(UX.4.4).
    Rejected(ExternalRejection),
}

/// 다시 불러온 문서에 이전 탭 가운데 남은 View를 viewport와 함께 다시 엽니다. 하나도 없으면 첫 View를 엽니다.
fn reopen_tabs(history: &mut WindowHistory, model: &DocumentModel, old: &ViewTabs) {
    let tabs = history.view_mut().tabs_mut();
    for tab in old.tabs() {
        if tabs.open(model, tab.view) == Ok(true) {
            let _ = tabs.set_active_viewport(tab.viewport);
        }
    }
    if let Some(active) = old.active_view() {
        if tabs.tabs().iter().any(|tab| tab.view == active) {
            let _ = tabs.activate(active);
        }
    }
    if tabs.is_empty() {
        if let Some(view) = first_view(model) {
            let _ = tabs.open(model, view);
        }
    }
}

/// 작업 문서 하나: 이 창의 문서 state와 로드할 때 읽은 그 문서의 chunk.
#[derive(Debug)]
pub(crate) struct WindowDocument {
    pub(crate) state: DocumentState,
    /// 마지막으로 디스크의 loaded generation과 내용이 같음을 확인한 문서 revision.
    saved_revision: DocumentRevision,
    /// 로드할 때 읽은 이 문서의 chunk(DocumentManifest·ViewTree·subtree). 저장할 때 다른 문서의 chunk와
    /// 구분해, 이 집합 밖의 base chunk만 그대로 이어 씁니다.
    pub(crate) owned_chunks: BTreeSet<ChunkId>,
}

/// 창 하나의 프로젝트 편집 session(모듈 문서).
#[derive(Debug)]
pub struct ProjectWindowSession {
    identity: ProjectIdentity,
    window: WindowSessionId,
    pub(crate) document: Option<WindowDocument>,
    pub(crate) history: WindowHistory,
    history_labels: BTreeMap<HistorySequence, u8>,
    pub(crate) loaded: Option<LoadedGeneration>,
    pub(crate) asset_chunks: std::sync::Arc<std::collections::BTreeMap<StableId, ContentHash>>,
    pub(crate) image_records:
        std::sync::Arc<std::collections::BTreeMap<StableId, stellar_assets::record::AssetRecord>>,
    /// 마지막으로 만든 불변 memory snapshot(XIII.8.2). 문서·revision이 같으면 다시 씁니다.
    snapshot_cache: std::sync::Mutex<Option<std::sync::Arc<SharedSnapshot>>>,
    /// 원본 상태(텍스트 원본 또는 chunk 전용, ADR-0088).
    pub(crate) source: SourceMode,
    /// 마지막으로 디스크를 읽은 계획.
    open_plan: OpenPlan,
    /// 열 때·마지막 저장 때 `stellar.yaml`의 활성 기본값 세트 이름.
    style_defaults_base: String,
    /// 저장 잠금 디렉터리(열기 뒤 맞춤 쓰기·Save Now).
    lock_dir: LockDir,
    /// 외부 변경 반영 뒤 원본·cache 맞춤 쓰기가 남았는지. 다음 저장(자동 저장 writer)이 합니다.
    reconcile_pending: bool,
}

/// 저장 잠금 디렉터리 공급자. 기본은 OS 사용자 데이터의 공용 잠금 디렉터리입니다.
#[derive(Clone)]
pub(crate) struct LockDir(
    std::sync::Arc<dyn Fn() -> Result<std::path::PathBuf, LockError> + Send + Sync>,
);

impl LockDir {
    /// `f`가 잠금 디렉터리를 정합니다(테스트의 임시 디렉터리 등).
    pub(crate) fn new(
        f: impl Fn() -> Result<std::path::PathBuf, LockError> + Send + Sync + 'static,
    ) -> LockDir {
        LockDir(std::sync::Arc::new(f))
    }

    fn get(&self) -> Result<std::path::PathBuf, LockError> {
        (self.0)()
    }
}

impl Default for LockDir {
    fn default() -> LockDir {
        LockDir::new(default_lock_dir)
    }
}

impl fmt::Debug for LockDir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LockDir")
    }
}

/// 디스크 원본·cache 관찰값(stale 판정, ADR-0088).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DiskMark {
    /// current manifest hash(없으면 `None`).
    manifest: Option<ContentHash>,
    /// base와 다른 원본 관찰값(같으면 `None`).
    source: Option<SourceMark>,
}

/// 읽은 chunk id를 모읍니다.
#[derive(Default)]
struct ChunkCollector(BTreeSet<ChunkId>);

impl ChunkReadRecorder for ChunkCollector {
    fn on_read(&mut self, pointer: &ChunkPointer) {
        self.0.insert(pointer.chunk_id);
    }
}

/// current generation에서 어떤 문서를 작업 문서로 읽을지.
#[derive(Debug, Clone, Copy)]
enum DocumentPick {
    /// 첫 root 문서(창 열기).
    FirstRoot,
    /// 이 창이 편집하던 문서(다시 불러오기·외부 변경 반영·새 base 위 저장).
    Document(StableId),
}

/// 디스크 current generation 하나와 그 위의 작업 문서(모두 검증하고 읽은 결과).
#[derive(Debug)]
struct DiskDocument {
    loaded: LoadedGeneration,
    /// 작업 문서 모델과 그 문서가 차지한 chunk. 고른 문서가 generation에 없으면(`FirstRoot`의 root 없음) `None`.
    document: Option<(DocumentModel, BTreeSet<ChunkId>)>,
}

/// 디스크의 current generation을 검증하고 `pick` 문서를 읽습니다. current pointer가 없으면 `Ok(None)`입니다.
///
/// generation schema는 제품 migration 정책으로 판정합니다(IX.11.21, ADR-0077 결정 4). 편집 session은 같은
/// schema만 엽니다. 읽기 사이 다른 저장이 끝나면 [`WindowSessionLoadError::ConcurrentSave`]입니다. 이 함수는
/// 아무것도 쓰지 않으며, 실패하면 호출자 상태를 바꾸지 않도록 결과만 돌려줍니다.
fn read_disk_document(
    identity: &ProjectIdentity,
    pick: DocumentPick,
) -> Result<Option<DiskDocument>, WindowSessionLoadError> {
    let root = identity.canonical_target();
    let lazy = match LazyGeneration::open(root, identity.project_id(), &mut NoopRecorder) {
        Ok(lazy) => lazy,
        Err(LazyLoadError::Load(LoadError::CurrentPointer(CurrentPointerError::Missing {
            ..
        }))) => return Ok(None),
        Err(error) => return Err(WindowSessionLoadError::Generation(error)),
    };
    let decision = decide(lazy.manifest().schema_version, &MigrationPolicy::PRODUCT);
    if !decision.is_editable() {
        return Err(WindowSessionLoadError::Schema(decision));
    }
    let document_id = match pick {
        DocumentPick::FirstRoot => lazy.manifest().root_document_ids.first().copied(),
        DocumentPick::Document(id) => Some(id),
    };
    let document = match document_id {
        None => None,
        Some(document_id) => {
            let mut owned = ChunkCollector::default();
            let document = load_document_from_generation(&lazy, document_id, &mut owned)
                .map_err(WindowSessionLoadError::Document)?;
            // DocumentManifest chunk는 열기 때 읽었으므로 그 문서의 것만 골라 더합니다.
            for (pointer, payload) in lazy.document_manifest_records() {
                let manifest =
                    stellar_project::document_io::manifest::DocumentManifestChunk::decode_for_schema(
                        pointer.schema_version, payload,
                    )
                    .map_err(|error| {
                        WindowSessionLoadError::Document(LoadDocumentError::Manifest(error))
                    })?;
                if manifest.definition.id == document_id {
                    owned.0.insert(pointer.chunk_id);
                }
            }
            Some((document.model, owned.0))
        }
    };
    let loaded = lazy.into_loaded();
    let current = read_current_pointer(root).map_err(|error| {
        WindowSessionLoadError::Generation(LazyLoadError::Load(LoadError::CurrentPointer(error)))
    })?;
    if loaded.manifest_hash != current {
        return Err(WindowSessionLoadError::ConcurrentSave);
    }
    Ok(Some(DiskDocument { loaded, document }))
}

/// generation의 이미지 asset metadata chunk(asset id → hash).
fn asset_chunks_of(loaded: &LoadedGeneration) -> BTreeMap<StableId, ContentHash> {
    loaded
        .manifest
        .chunk_pointers
        .iter()
        .filter(|p| {
            p.kind == stellar_storage::chunk::kinds::KIND_ASSET_METADATA && p.schema_version == 1
        })
        .map(|p| (p.chunk_id.stable_id(), p.content_hash))
        .collect()
}

impl ProjectWindowSession {
    /// 프로젝트 `identity`를 열어 창 `window`의 session을 만듭니다(모듈 문서 "텍스트 원본과 chunk cache").
    ///
    /// 원본과 cache가 짝이면 cache에서 첫 root 문서만 읽습니다. 계획이 맞춤 쓰기(원본 전환·cache 다시 만들기)를
    /// 요구하면 연 뒤 같은 저장 경로로 씁니다. 그 쓰기가 실패해도 session은 열리며 다음 저장이 다시 맞춥니다.
    /// 저장한 적 없는 프로젝트는 작업 문서·loaded generation 없이 엽니다.
    pub fn load(
        identity: ProjectIdentity,
        window: WindowSessionId,
    ) -> Result<ProjectWindowSession, WindowSessionLoadError> {
        ProjectWindowSession::load_with_lock_dir(identity, window, LockDir::default())
    }

    /// [`ProjectWindowSession::load`]와 같고 저장 잠금 디렉터리를 정합니다.
    pub(crate) fn load_with_lock_dir(
        identity: ProjectIdentity,
        window: WindowSessionId,
        lock_dir: LockDir,
    ) -> Result<ProjectWindowSession, WindowSessionLoadError> {
        recover_source_journal(&identity);
        let disk = read_disk_state(&identity, DocumentPick::FirstRoot, None)?;
        let reconcile = disk.needs_reconcile();
        let mut session = ProjectWindowSession::empty(identity, window);
        session.lock_dir = lock_dir;
        session.adopt(disk, DocumentRevision(0), None);
        if reconcile {
            if let Err(error) = session.reconcile() {
                eprintln!("Stellar text source reconcile: {error}");
                // 다음 저장(자동 저장 writer·창 닫기 flush)이 다시 맞추도록 남깁니다.
                session.reconcile_pending = true;
            }
        }
        Ok(session)
    }

    /// 디스크 상태를 이 session의 작업 문서·base로 삼습니다(history는 새로 시작). `revision`이 0이 아니면 그
    /// revision으로 문서를 만들고(다시 불러오기), `tabs`가 있으면 남은 View 탭을 다시 엽니다.
    fn adopt(&mut self, disk: DiskState, revision: DocumentRevision, tabs: Option<&ViewTabs>) {
        let mut history = WindowHistory::new(self.window);
        self.document = disk.model.map(|mut model| {
            model.set_style_defaults(disk.style_defaults.clone());
            let state = if revision.0 == 0 {
                DocumentState::new(model)
            } else {
                DocumentState::with_revision(model, revision)
            };
            match tabs {
                Some(old) => reopen_tabs(&mut history, state.model(), old),
                None => {
                    if let Some(view) = first_view(state.model()) {
                        // 같은 모델에서 고른 View이므로 열기는 실패하지 않습니다(UnknownView만 가능한 오류).
                        let opened = history.view_mut().tabs_mut().open(state.model(), view);
                        debug_assert_eq!(opened, Ok(true));
                    }
                }
            }
            WindowDocument {
                saved_revision: state.revision(),
                state,
                owned_chunks: disk.owned,
            }
        });
        self.replace_history(history);
        self.asset_chunks = std::sync::Arc::new(
            disk.loaded
                .as_ref()
                .map(asset_chunks_of)
                .unwrap_or_default(),
        );
        self.loaded = disk.loaded;
        self.source = disk.source;
        self.open_plan = disk.plan;
        self.style_defaults_base = disk.style_defaults;
        self.reconcile_pending = false;
    }

    /// 저장된 generation이 없는 프로젝트의 session.
    fn empty(identity: ProjectIdentity, window: WindowSessionId) -> ProjectWindowSession {
        ProjectWindowSession {
            identity,
            window,
            document: None,
            history: WindowHistory::new(window),
            history_labels: BTreeMap::new(),
            loaded: None,
            asset_chunks: Default::default(),
            image_records: Default::default(),
            snapshot_cache: Default::default(),
            source: SourceMode::Text {
                base: SourceBase::default(),
                library: Some(Default::default()),
            },
            open_plan: OpenPlan::Empty,
            style_defaults_base: stellar_document::css::style_defaults::DEFAULT_SET.to_owned(),
            lock_dir: LockDir::default(),
            reconcile_pending: false,
        }
    }

    /// 마지막으로 디스크를 읽은 계획(열기·다시 불러오기·외부 변경 반영).
    pub fn open_plan(&self) -> OpenPlan {
        self.open_plan
    }

    /// 텍스트 원본으로 옮기지 못해 chunk generation이 원본으로 남은 이유. 텍스트 원본 session이면 `None`.
    pub fn chunk_only_reason(&self) -> Option<&str> {
        match &self.source {
            SourceMode::ChunkOnly { reason } => Some(reason),
            SourceMode::Text { .. } => None,
        }
    }

    /// 텍스트 원본 base(chunk 전용이면 `None`).
    pub fn source_base(&self) -> Option<&SourceBase> {
        self.source.base()
    }

    /// 외부 변경 반영 뒤 원본·cache 맞춤 쓰기가 남았는지. 미저장 편집이 아니므로
    /// [`ProjectWindowSession::has_unsaved_changes`]에는 들지 않습니다.
    pub fn reconcile_pending(&self) -> bool {
        self.reconcile_pending
    }

    /// 저장 경로가 쓸 것이 있는지: 미저장 편집이나 남은 맞춤 쓰기. 자동 저장 일정이 이 값을 봅니다.
    pub fn needs_write(&self) -> bool {
        self.has_unsaved_changes() || self.reconcile_pending
    }

    /// 계획이 요구하는 맞춤 쓰기를 합니다: 작업 문서와 문서 밖 의미를 바뀐 것이 없어도 저장 경로로 씁니다. 원본과
    /// cache 가운데 디스크와 다른 쪽만 실제로 쓰고 stamp를 맞춥니다. chunk 전용이면 아무것도 하지 않습니다.
    pub(crate) fn reconcile(&mut self) -> Result<SaveOutcome, WindowSessionSaveError> {
        let idle = SaveOutcome {
            generation: self.generation_number(),
            written: false,
            source_files: 0,
            cache_stale: false,
        };
        if !matches!(self.source, SourceMode::Text { .. }) {
            return Ok(idle);
        }
        let Some(capture) = self.capture(true)? else {
            return Ok(idle);
        };
        let lock = self.lock_dir.clone();
        let written = capture.write(move || lock.get())?;
        Ok(self.apply_save(written))
    }

    /// 지금 작업 문서 revision의 불변 memory snapshot(XIII.8.2). 같은 문서·revision이면 앞서 만든 것을 돌려주므로
    /// 자동 저장 캡처와 복구 캡처가 레코드 복사와 직렬화를 나눠 씁니다. 작업 문서가 없으면 빈 문서 오류입니다.
    pub(crate) fn shared_snapshot(
        &self,
    ) -> Option<Result<std::sync::Arc<SharedSnapshot>, stellar_document::model::ModelError>> {
        let document = self.document.as_ref()?;
        Some(self.snapshot_of(document))
    }

    fn snapshot_of(
        &self,
        document: &WindowDocument,
    ) -> Result<std::sync::Arc<SharedSnapshot>, stellar_document::model::ModelError> {
        let document_id = document.state.model().document_id();
        let revision = document.state.revision();
        let mut cache = self
            .snapshot_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(snapshot) = cache
            .as_ref()
            .filter(|s| s.document_id == document_id && s.revision == revision)
        {
            return Ok(std::sync::Arc::clone(snapshot));
        }
        let snapshot = std::sync::Arc::new(SharedSnapshot {
            document_id,
            revision,
            records: document.state.model().to_records()?,
            serialized: std::sync::OnceLock::new(),
            serializations: std::sync::atomic::AtomicUsize::new(0),
        });
        *cache = Some(std::sync::Arc::clone(&snapshot));
        Ok(snapshot)
    }

    /// 연 프로젝트.
    pub fn identity(&self) -> &ProjectIdentity {
        &self.identity
    }

    /// 창 편집 session id.
    pub fn window(&self) -> WindowSessionId {
        self.window
    }

    /// 이 창의 작업 문서 state. 저장된 문서가 없으면 `None`.
    pub fn document(&self) -> Option<&DocumentState> {
        self.document.as_ref().map(|document| &document.state)
    }

    /// 작업 문서 StableId.
    pub fn document_id(&self) -> Option<StableId> {
        self.document().map(|state| state.model().document_id())
    }

    /// 작업 문서 revision(문서가 없으면 0).
    pub fn revision(&self) -> DocumentRevision {
        self.document()
            .map_or(DocumentRevision(0), DocumentState::revision)
    }

    /// 현재 history 위치. Undo/Redo는 해당 항목의 sequence로 복귀하고 새 분기는 새 sequence를 받습니다.
    pub(crate) fn history_sequence(&self) -> u64 {
        self.history
            .history()
            .entries()
            .last()
            .map_or(0, |entry| entry.sequence.get())
    }

    /// 기록 직전 redo 갈래의 sequence. 새 항목(문서·선택)이 확정되면 history가 이 갈래를 버립니다.
    fn redo_sequences(&self) -> Vec<HistorySequence> {
        self.history
            .history()
            .redo_entries()
            .iter()
            .map(|entry| entry.sequence)
            .collect()
    }

    /// 기록에 성공한 직후 같은 호출 안에서 부릅니다. 버려진 redo 갈래의 이름표를 지우고, `label`이 있으면 방금
    /// 기록한(또는 grouping으로 합친) 마지막 항목에 붙입니다. 기록 전에 있던 redo가 그대로면(기록하지 않음) 지우지
    /// 않습니다.
    fn labeled(&mut self, discarded: Vec<HistorySequence>, label: u8) {
        if self.history.history().redo_entries().is_empty() {
            for sequence in discarded {
                self.history_labels.remove(&sequence);
            }
        }
        if label == history_label::NONE {
            return;
        }
        if let Some(entry) = self.history.history().entries().last() {
            self.history_labels.insert(entry.sequence, label);
        }
    }

    /// 연속 조작(scrub) preview를 한 항목으로 확정하고 이름표를 붙입니다. preview 도중 버려진 redo 갈래의
    /// 시점을 알 수 없으므로, 표를 지금 undo·redo 스택의 sequence로 다시 맞춥니다(드문 경로).
    pub(crate) fn end_preview(
        &mut self,
        label: u8,
    ) -> Result<(), stellar_history::entry::HistoryError> {
        self.history.end_preview()?;
        let history = self.history.history();
        let live: std::collections::HashSet<HistorySequence> = history
            .entries()
            .iter()
            .chain(history.redo_entries())
            .map(|entry| entry.sequence)
            .collect();
        self.history_labels
            .retain(|sequence, _| live.contains(sequence));
        self.labeled(Vec::new(), label);
        Ok(())
    }

    /// history를 새로 시작할 때(복구 복원) 이전 sequence의 이름표를 버립니다.
    pub(crate) fn replace_history(&mut self, history: WindowHistory) {
        self.history = history;
        self.history_labels.clear();
    }

    #[cfg(test)]
    pub(crate) fn label_count(&self) -> usize {
        self.history_labels.len()
    }

    /// Undo 맨 위 항목의 이름표. 세션 밖에서 만든 항목은 0입니다.
    pub fn undo_label(&self) -> u8 {
        self.history.history().entries().last().map_or(0, |entry| {
            self.history_labels
                .get(&entry.sequence)
                .copied()
                .unwrap_or(0)
        })
    }

    /// Redo 맨 위 항목의 이름표.
    pub fn redo_label(&self) -> u8 {
        self.history
            .history()
            .redo_entries()
            .last()
            .map_or(0, |entry| {
                self.history_labels
                    .get(&entry.sequence)
                    .copied()
                    .unwrap_or(0)
            })
    }

    /// 마지막 저장 뒤 문서가 바뀌었는지(O(1)). 선택·viewport는 문서 revision을 바꾸지 않습니다.
    pub fn has_unsaved_changes(&self) -> bool {
        self.document
            .as_ref()
            .is_some_and(|document| document.state.revision() != document.saved_revision)
    }

    /// 작업 문서가 base generation에서 차지한 chunk(DocumentManifest·ViewTree·subtree). 문서가 없으면
    /// `None`. 저장은 이 집합 밖의 base chunk만 그대로 이어 씁니다.
    pub fn document_chunks(&self) -> Option<&BTreeSet<ChunkId>> {
        self.document
            .as_ref()
            .map(|document| &document.owned_chunks)
    }

    /// 이 창의 history.
    pub fn history(&self) -> &WindowHistory {
        &self.history
    }

    /// 이 창의 view state(선택, 탭, viewport).
    pub fn view(&self) -> &WindowViewState {
        self.history.view()
    }

    /// 열린 탭·active view·탭별 viewport.
    pub fn tabs(&self) -> &ViewTabs {
        self.history.view().tabs()
    }

    /// 작업 문서의 View 탭을 열고 active로 만듭니다(이미 열려 있으면 active만, `Ok(false)`).
    pub fn open_view(&mut self, view: StableId) -> Result<bool, WindowSessionEditError> {
        let document = self
            .document
            .as_ref()
            .ok_or(WindowSessionEditError::NoDocument)?;
        self.history
            .view_mut()
            .tabs_mut()
            .open(document.state.model(), view)
            .map_err(WindowSessionEditError::View)
    }

    /// 열린 View 탭을 닫습니다.
    pub fn close_view(&mut self, view: StableId) -> Result<(), WindowSessionEditError> {
        self.history
            .view_mut()
            .tabs_mut()
            .close(view)
            .map_err(WindowSessionEditError::View)
    }

    /// active 탭 viewport와 캔버스 화면 크기(point)로 scene camera 입력을 만듭니다(XII.4.3). active 탭이 없거나 크기·
    /// 변환이 유효하지 않으면(0 크기, f32 범위 초과) `None`입니다.
    pub fn camera(&self, view_size: Size2) -> Option<Viewport> {
        let viewport = self.tabs().active_viewport()?;
        if view_size.width() <= 0.0 || view_size.height() <= 0.0 {
            return None;
        }
        let view = viewport.world_to_view(view_size.width(), view_size.height())?;
        Viewport::new(view_size, view).ok()
    }

    /// active 탭 viewport를 화면에서 `(dx, dy)` point만큼 옮깁니다(XII.4.3). 문서는 바뀌지 않습니다.
    pub fn pan(&mut self, dx: f64, dy: f64) -> Result<CanvasViewport, WindowSessionEditError> {
        let current = self.active_viewport()?;
        let next = current
            .panned(dx, dy)
            .map_err(WindowSessionEditError::Viewport)?;
        self.set_viewport(next)?;
        Ok(next)
    }

    /// active 탭 배율을 `factor`배 합니다. 화면 중심에서 `(anchor_x, anchor_y)` point인 점을 고정합니다(XII.4.3).
    pub fn zoom(
        &mut self,
        factor: f64,
        anchor_x: f64,
        anchor_y: f64,
    ) -> Result<CanvasViewport, WindowSessionEditError> {
        let current = self.active_viewport()?;
        let next = current
            .zoomed(factor, anchor_x, anchor_y)
            .map_err(WindowSessionEditError::Viewport)?;
        self.set_viewport(next)?;
        Ok(next)
    }

    /// active 탭 배율을 100%로 되돌립니다(화면 중심 고정).
    pub fn zoom_to_actual_size(&mut self) -> Result<CanvasViewport, WindowSessionEditError> {
        let current = self.active_viewport()?;
        let next = current
            .with_zoom(1.0)
            .map_err(WindowSessionEditError::Viewport)?;
        self.set_viewport(next)?;
        Ok(next)
    }

    /// 문서 scene `scene`(이 창 active view에서 만든 snapshot) 위에 그릴 selection overlay를 만듭니다(XII.4.4). active 탭이
    /// 없거나 화면 크기가 유효하지 않으면 `None`입니다. 표시할 선택은 영역 선택 preview를 포함한 화면 선택입니다.
    pub fn selection_overlay(
        &self,
        scene: &SceneSnapshot,
        view_size: Size2,
        style: &OverlayStyle,
    ) -> Option<Result<SelectionOverlay, OverlayError>> {
        let camera = self.camera(view_size)?;
        let selection = self.view().displayed_selection();
        Some(build_selection_overlay(
            scene,
            selection.ids(),
            selection.primary(),
            &camera,
            style,
        ))
    }

    fn active_viewport(&self) -> Result<CanvasViewport, WindowSessionEditError> {
        self.tabs()
            .active_viewport()
            .ok_or(WindowSessionEditError::View(ViewTabsError::NoActiveTab))
    }

    /// active 탭의 viewport를 바꾸고 이전 값을 돌려줍니다. 문서·history는 바뀌지 않습니다.
    pub fn set_viewport(
        &mut self,
        viewport: CanvasViewport,
    ) -> Result<CanvasViewport, WindowSessionEditError> {
        self.history
            .view_mut()
            .tabs_mut()
            .set_active_viewport(viewport)
            .map_err(WindowSessionEditError::View)
    }

    /// 창을 열 때(또는 마지막 저장 뒤) 기준이 된 generation. 첫 저장 전이면 `None`.
    pub fn loaded_generation(&self) -> Option<&LoadedGeneration> {
        self.loaded.as_ref()
    }

    /// loaded generation 번호(없으면 0).
    pub fn generation_number(&self) -> u64 {
        self.loaded
            .as_ref()
            .map_or(0, |loaded| loaded.manifest.generation)
    }

    /// 작업 문서에 transaction을 실행하고 이 창 history에 기록합니다. 작업 문서가 없으면
    /// [`WindowSessionEditError::NoDocument`].
    ///
    /// `label`은 Undo·Redo 메뉴 이름표([`history_label`])입니다.
    pub fn apply(
        &mut self,
        transaction: &CommandTransaction,
        label: u8,
    ) -> Result<ChangeSet, WindowSessionEditError> {
        let discarded = self.redo_sequences();
        let document = self
            .document
            .as_mut()
            .ok_or(WindowSessionEditError::NoDocument)?;
        let changes = self
            .history
            .record_document(&mut document.state, transaction, None)
            .map_err(WindowSessionEditError::History)?;
        self.labeled(discarded, label);
        Ok(changes)
    }

    /// 연속 입력 grouping으로 transaction을 기록합니다(Text 입력, VII.4.3).
    pub(crate) fn apply_grouped<C: stellar_core_types::clock::MonotonicClock>(
        &mut self,
        transaction: &CommandTransaction,
        key: stellar_history::entry::GroupingKey,
        clock: &C,
        label: u8,
    ) -> Result<ChangeSet, WindowSessionEditError> {
        let discarded = self.redo_sequences();
        let document = self
            .document
            .as_mut()
            .ok_or(WindowSessionEditError::NoDocument)?;
        let changes = self
            .history
            .record_grouped(&mut document.state, transaction, key, clock)
            .map_err(WindowSessionEditError::History)?;
        self.labeled(discarded, label);
        Ok(changes)
    }

    /// 새 Text 삽입을 이어지는 입력과 같은 group의 첫 항목으로 기록하고 선택합니다.
    pub(crate) fn insert_grouped_and_select<C: stellar_core_types::clock::MonotonicClock>(
        &mut self,
        transaction: &CommandTransaction,
        key: stellar_history::entry::GroupingKey,
        inserted: StableId,
        clock: &C,
        label: u8,
    ) -> Result<ChangeSet, WindowSessionEditError> {
        let discarded = self.redo_sequences();
        let document = self
            .document
            .as_mut()
            .ok_or(WindowSessionEditError::NoDocument)?;
        let selection = SetSelection::new(Selection::new([inserted], Some(inserted)));
        let changes = self
            .history
            .record_grouped_and_select(&mut document.state, transaction, key, &selection, clock)
            .map_err(WindowSessionEditError::History)?;
        self.labeled(discarded, label);
        Ok(changes)
    }

    pub(crate) fn insert_and_select(
        &mut self,
        transaction: &CommandTransaction,
        inserted: StableId,
        label: u8,
    ) -> Result<ChangeSet, WindowSessionEditError> {
        let discarded = self.redo_sequences();
        let document = self
            .document
            .as_mut()
            .ok_or(WindowSessionEditError::NoDocument)?;
        let selection = SetSelection::new(Selection::new([inserted], Some(inserted)));
        let changes = self
            .history
            .record_document_and_select(&mut document.state, transaction, &selection)
            .map_err(WindowSessionEditError::History)?;
        self.labeled(discarded, label);
        Ok(changes)
    }

    /// transaction과 새 선택을 한 history 항목으로 기록합니다(복제 드래그처럼 여러 node를 만들고 고를 때).
    pub(crate) fn apply_and_select(
        &mut self,
        transaction: &CommandTransaction,
        selection: Selection,
        label: u8,
    ) -> Result<ChangeSet, WindowSessionEditError> {
        let discarded = self.redo_sequences();
        let document = self
            .document
            .as_mut()
            .ok_or(WindowSessionEditError::NoDocument)?;
        let changes = self
            .history
            .record_document_and_select(
                &mut document.state,
                transaction,
                &SetSelection::new(selection),
            )
            .map_err(WindowSessionEditError::History)?;
        self.labeled(discarded, label);
        Ok(changes)
    }

    /// 이 창 history의 마지막 항목을 되돌립니다. 되돌릴 항목이 없거나 작업 문서가 없으면 `Ok(false)`.
    pub fn undo(&mut self) -> Result<bool, WindowSessionEditError> {
        let Some(document) = self.document.as_mut() else {
            return Ok(false);
        };
        self.history
            .undo(&mut document.state)
            .map(|outcome| outcome.is_some())
            .map_err(WindowSessionEditError::History)
    }

    /// 이 창에서 마지막으로 되돌린 항목을 다시 적용합니다. 다시 적용할 항목이 없으면 `Ok(false)`.
    pub fn redo(&mut self) -> Result<bool, WindowSessionEditError> {
        let Some(document) = self.document.as_mut() else {
            return Ok(false);
        };
        self.history
            .redo(&mut document.state)
            .map(|outcome| outcome.is_some())
            .map_err(WindowSessionEditError::History)
    }

    /// 작업 문서를 지금 저장합니다(모듈 문서 "Undo·Redo·Save Now"). 캡처·쓰기·반영을 이 thread에서 차례로 합니다.
    pub fn save_now(&mut self) -> Result<SaveOutcome, WindowSessionSaveError> {
        let lock = self.lock_dir.clone();
        self.save_with_lock_dir(move || lock.get())
    }

    pub(crate) fn save_with_lock_dir(
        &mut self,
        lock_dir: impl FnOnce() -> Result<std::path::PathBuf, LockError>,
    ) -> Result<SaveOutcome, WindowSessionSaveError> {
        let Some(capture) = self.capture_save()? else {
            return Ok(SaveOutcome {
                generation: self.generation_number(),
                written: false,
                source_files: 0,
                cache_stale: false,
            });
        };
        let written = capture.write(lock_dir)?;
        Ok(self.apply_save(written))
    }

    /// 저장 snapshot을 캡처합니다(XIII.1.3·XIII.1.4). 문서 레코드·기준 generation·소유 chunk만 복사하고 직렬화와
    /// 쓰기는 [`SaveCapture::write`]가 합니다. 작업 문서가 없거나 저장 뒤 바뀐 것이 없으면 `Ok(None)`이고, 바뀐 것이
    /// 없을 때도 current pointer가 loaded generation과 같은지는 확인합니다(stale이면 오류).
    pub(crate) fn capture_save(&self) -> Result<Option<SaveCapture>, WindowSessionSaveError> {
        self.capture(false)
    }

    /// `forced`이면 저장 뒤 바뀐 것이 없어도 캡처합니다(맞춤 쓰기, [`ProjectWindowSession::reconcile`]).
    fn capture(&self, forced: bool) -> Result<Option<SaveCapture>, WindowSessionSaveError> {
        let Some(document) = self.document.as_ref() else {
            return Ok(None);
        };
        let base = self.loaded.as_ref();
        let revision = document.state.revision();
        if let Some(base) = base.filter(|_| !forced && !self.reconcile_pending) {
            if revision == document.saved_revision {
                check_base_manifest(self.identity.canonical_target(), Some(base.manifest_hash))
                    .map_err(WindowSessionSaveError::Stale)?;
                return Ok(None);
            }
        }
        let snapshot = self
            .snapshot_of(document)
            .map_err(|error| WindowSessionSaveError::Serialize(SerializeError::Model(error)))?;
        Ok(Some(SaveCapture {
            identity: self.identity.clone(),
            base: base.cloned(),
            revision,
            snapshot,
            owned_chunks: document.owned_chunks.clone(),
            image_records: std::sync::Arc::clone(&self.image_records),
            text: self.text_capture(),
            style_defaults: (
                document.state.model().style_defaults().to_owned(),
                self.style_defaults_base.clone(),
            ),
        }))
    }

    fn text_capture(&self) -> Option<TextCapture> {
        match &self.source {
            SourceMode::Text { base, library } => Some(TextCapture {
                base: base.clone(),
                library: library.clone(),
            }),
            SourceMode::ChunkOnly { .. } => None,
        }
    }

    /// [`SaveCapture::write`] 결과를 이 창에 반영합니다. 캡처 뒤 이 창의 기준 generation·원본 base나 작업 문서가
    /// 바뀌었으면(다른 저장·다시 불러오기·외부 변경 반영) 아무것도 바꾸지 않습니다. 캡처 뒤의 편집은 dirty로 남습니다.
    pub(crate) fn apply_save(&mut self, written: SaveWritten) -> SaveOutcome {
        let current = self.loaded.as_ref().map(|loaded| loaded.manifest_hash);
        let wrote = written.saved.is_some();
        let source_files = written.text.as_ref().map_or(0, |text| text.files);
        let outcome = |session: &ProjectWindowSession| SaveOutcome {
            generation: session.generation_number(),
            written: wrote,
            source_files,
            cache_stale: written.cache_stale,
        };
        let same_document = self
            .document
            .as_ref()
            .is_some_and(|document| document.state.model().document_id() == written.document_id);
        let same_source = match (&self.source, &written.text) {
            (SourceMode::Text { base, .. }, Some(text)) => *base == text.before,
            (SourceMode::ChunkOnly { .. }, None) => true,
            _ => false,
        };
        if !same_document || current != written.base || !same_source {
            return outcome(self);
        }
        let result = outcome(self);
        let document = self.document.as_mut().expect("같은 문서를 확인함");
        if let Some((loaded, chunks)) = written.saved {
            self.asset_chunks = std::sync::Arc::new(asset_chunks_of(&loaded));
            self.loaded = Some(loaded);
            document.owned_chunks = chunks;
        }
        document.saved_revision = written.revision;
        if let Some(text) = written.text {
            self.source = SourceMode::Text {
                base: text.after,
                library: Some(text.library),
            };
        }
        self.style_defaults_base = written.style_defaults;
        self.reconcile_pending = false;
        SaveOutcome {
            generation: self.generation_number(),
            ..result
        }
    }

    /// 디스크 current generation이나 텍스트 원본이 이 창의 base와 다르면 그 writer 출처와 다음 할 일을 돌려줍니다
    /// (XII.7.1, ADR-0083 결정 2, ADR-0088 결정 5). 같거나 저장한 적 없는 프로젝트면 `Ok(None)`입니다.
    pub fn stale_state(
        &self,
    ) -> Result<Option<(WriterProvenance, StaleResolution)>, CurrentPointerError> {
        Ok(self
            .stale_judgement()?
            .map(|(_, writer, resolution)| (writer, resolution)))
    }

    /// [`ProjectWindowSession::stale_state`]와 같고, 판정한 디스크 관찰값을 함께 돌려줍니다.
    ///
    /// 원본과 cache를 함께 쓰는 writer는 앱 창뿐이라(CLI·MCP는 아직 cache만, Git·편집기는 원본만 씀) 둘 가운데 하나라도
    /// 외부 writer가 만든 것이면 외부입니다. 원본은 파일 집합 digest로 앱 창 저장 기록을 찾습니다.
    fn stale_judgement(
        &self,
    ) -> Result<Option<(DiskMark, WriterProvenance, StaleResolution)>, CurrentPointerError> {
        let root = self.identity.canonical_target();
        let current = match read_current_pointer(root) {
            Ok(current) => Some(current),
            Err(CurrentPointerError::Missing { .. }) if self.loaded.is_none() => None,
            Err(error) => return Err(error),
        };
        let generation_moved = current != self.loaded.as_ref().map(|loaded| loaded.manifest_hash);
        let source = self
            .source
            .base()
            .and_then(|base| document_source_moved(&self.identity, base))
            .map(|(mark, _)| mark);
        if !generation_moved && source.is_none() {
            return Ok(None);
        }
        let generation_writer = current
            .filter(|_| generation_moved)
            .map(|current| writer_provenance(root, &current));
        let source_writer = source.map(|mark| match mark {
            SourceMark::Digest(digest) => writer_provenance(root, &digest),
            SourceMark::Unreadable => WriterProvenance::External,
        });
        let writer =
            if [generation_writer, source_writer].contains(&Some(WriterProvenance::External)) {
                WriterProvenance::External
            } else {
                WriterProvenance::AppWindow
            };
        Ok(Some((
            DiskMark {
                manifest: current,
                source,
            },
            writer,
            StaleResolution::for_writer(writer),
        )))
    }

    /// stale 저장의 사용자 선택을 실행합니다(XII.7.5·XII.7.6, ADR-0083 결정 1).
    ///
    /// 디스크가 이 창의 base와 같으면(stale이 아님) [`StaleResolveError::NotStale`], 디스크 generation·원본을 외부
    /// writer가 만들었으면 두 선택지 대신 reconciliation 대기이므로 [`StaleResolveError::ReconciliationRequired`]
    /// 입니다(결정 2). 어느 실패에서도 이 창 문서·history는 그대로입니다.
    pub fn resolve_stale(
        &mut self,
        choice: StaleSaveChoice,
    ) -> Result<StaleResolved, StaleResolveError> {
        let lock = self.lock_dir.clone();
        self.resolve_stale_with_lock_dir(choice, move || lock.get())
    }

    pub(crate) fn resolve_stale_with_lock_dir(
        &mut self,
        choice: StaleSaveChoice,
        lock_dir: impl FnOnce() -> Result<std::path::PathBuf, LockError>,
    ) -> Result<StaleResolved, StaleResolveError> {
        let state = self.stale_judgement().map_err(|error| {
            StaleResolveError::Load(WindowSessionLoadError::Generation(LazyLoadError::Load(
                LoadError::CurrentPointer(error),
            )))
        })?;
        match state {
            None => Err(StaleResolveError::NotStale),
            Some((_, _, StaleResolution::ReconciliationPending)) => {
                Err(StaleResolveError::ReconciliationRequired)
            }
            // UseCurrentWindow는 writer 출처를 판정한 그 generation·원본 위에서만 씁니다. 판정 뒤 다른 writer(외부
            // 포함)가 저장했으면 다시 stale입니다(ADR-0083 결정 1·2).
            Some((judged, _, StaleResolution::Choose)) => match choice {
                StaleSaveChoice::UseCurrentWindow => self
                    .save_over_judged(judged, lock_dir)
                    .map(StaleResolved::Saved)
                    .map_err(StaleResolveError::Save),
                StaleSaveChoice::UseLatestDisk => self
                    .reload_latest_disk()
                    .map(|discarded_history_entries| StaleResolved::Reloaded {
                        generation: self.generation_number(),
                        discarded_history_entries,
                    })
                    .map_err(StaleResolveError::Load),
            },
        }
    }

    /// [`ProjectWindowSession::save_over_judged`]를 generation 관찰값만으로 부릅니다. 원본은 지금 디스크 것을
    /// 판정값으로 씁니다(시험 경로).
    #[cfg(test)]
    pub(crate) fn save_over_latest_with_lock_dir(
        &mut self,
        judged: ContentHash,
        lock_dir: impl FnOnce() -> Result<std::path::PathBuf, LockError>,
    ) -> Result<SaveOutcome, WindowSessionSaveError> {
        let source = self
            .source
            .base()
            .and_then(|base| document_source_moved(&self.identity, base))
            .map(|(mark, _)| mark);
        self.save_over_judged(
            DiskMark {
                manifest: Some(judged),
                source,
            },
            lock_dir,
        )
    }

    /// UseCurrentWindow(XII.7.5): 디스크의 최신 generation·원본을 새 base로 삼아 이 창 문서 전체를 씁니다. 다른
    /// 문서·에셋 chunk는 최신 generation 것을 그대로 잇고, 이 창 문서가 최신 generation에서 차지한 chunk는 이 창
    /// 문서로 바뀝니다. 원본은 최신 파일 hash를 기대 hash로 씁니다. 쓰기는 일반 저장과 같은 잠금·base 검사를 거치므로,
    /// 최신을 읽은 뒤 commit 전에 다른 저장이 끝나면 다시 stale이고 아무것도 쓰지 않습니다. 성공하면 디스크의 이
    /// 문서는 창 문서와 같고, history는 그대로 남습니다.
    ///
    /// `judged`는 [`ProjectWindowSession::resolve_stale`]이 writer 출처를 판정한 디스크 관찰값입니다. 다시 읽은 최신이
    /// 그것과 다르면(판정 뒤 다른 저장이 끝남) 쓰지 않고 다시 stale입니다. 그래서 외부 writer의 저장을 두 선택지
    /// 경로로 덮어쓰지 않습니다(결정 2). 공개 진입점은 `resolve_stale` 하나입니다.
    fn save_over_judged(
        &mut self,
        judged: DiskMark,
        lock_dir: impl FnOnce() -> Result<std::path::PathBuf, LockError>,
    ) -> Result<SaveOutcome, WindowSessionSaveError> {
        let Some(document) = self.document.as_ref() else {
            return Ok(SaveOutcome {
                generation: self.generation_number(),
                written: false,
                source_files: 0,
                cache_stale: false,
            });
        };
        let document_id = document.state.model().document_id();
        let disk = read_disk_document(&self.identity, DocumentPick::Document(document_id));
        let disk = match disk {
            Ok(Some(disk)) => Some(disk),
            Ok(None) => None,
            // 최신 generation에서 이 문서가 지워졌으면 이 창 문서를 새 root로 더합니다.
            Err(WindowSessionLoadError::Document(LoadDocumentError::UnknownDocument {
                ..
            })) => {
                let root = self.identity.canonical_target();
                let loaded = load_current_generation(root, self.identity.project_id())
                    .map_err(WindowSessionSaveError::Reload)?;
                Some(DiskDocument {
                    loaded,
                    document: None,
                })
            }
            Err(error) => return Err(WindowSessionSaveError::Rebase(Box::new(error))),
        };
        let found = disk.as_ref().map(|disk| disk.loaded.manifest_hash);
        if found != judged.manifest {
            return Err(WindowSessionSaveError::Stale(BaseCheckError::StaleBase {
                expected: judged.manifest,
                found,
            }));
        }
        // 원본: 판정한 원본과 지금 원본이 같아야 하고, 그 파일 hash가 새 기대 hash입니다. 문서 밖 의미는 writer가 그
        // 원본에서 다시 읽습니다.
        let text = match self.source.base() {
            None => None,
            Some(base) => {
                let moved = document_source_moved(&self.identity, base);
                if moved.as_ref().map(|(mark, _)| *mark) != judged.source {
                    let paths = moved
                        .map(|(_, now)| base.changed_paths(&now).into_iter().collect())
                        .unwrap_or_default();
                    return Err(WindowSessionSaveError::SourceStale { paths });
                }
                if moved.as_ref().map(|(mark, _)| *mark) == Some(SourceMark::Unreadable) {
                    return Err(source_error("텍스트 원본을 읽지 못했습니다"));
                }
                Some(match moved {
                    Some((_, now)) => TextCapture {
                        base: now,
                        library: None,
                    },
                    None => TextCapture {
                        base: base.clone(),
                        library: self.text_capture().and_then(|text| text.library),
                    },
                })
            }
        };
        let (base, owned_chunks) = match disk {
            Some(disk) => (
                Some(disk.loaded),
                disk.document.map(|(_, chunks)| chunks).unwrap_or_default(),
            ),
            None => (None, BTreeSet::new()),
        };
        let revision = document.state.revision();
        let snapshot = self
            .snapshot_of(document)
            .map_err(|error| WindowSessionSaveError::Serialize(SerializeError::Model(error)))?;
        let capture = SaveCapture {
            identity: self.identity.clone(),
            base: base.clone(),
            revision,
            snapshot,
            owned_chunks: owned_chunks.clone(),
            image_records: std::sync::Arc::clone(&self.image_records),
            text,
            style_defaults: (
                document.state.model().style_defaults().to_owned(),
                self.style_defaults_base.clone(),
            ),
        };
        let written = capture.write(lock_dir)?;
        let wrote = written.saved.is_some();
        let source_files = written.text.as_ref().map_or(0, |text| text.files);
        let cache_stale = written.cache_stale;
        let (loaded, chunks) = match written.saved {
            Some((loaded, chunks)) => (Some(loaded), chunks),
            // 최신 generation의 이 문서가 이미 창 문서와 같음: 그 generation이 새 base입니다.
            None => (base, owned_chunks),
        };
        if let Some(loaded) = &loaded {
            self.asset_chunks = std::sync::Arc::new(asset_chunks_of(loaded));
        }
        self.loaded = loaded;
        if let Some(text) = written.text {
            self.source = SourceMode::Text {
                base: text.after,
                library: Some(text.library),
            };
        }
        self.style_defaults_base = written.style_defaults;
        if let Some(document) = self.document.as_mut() {
            document.owned_chunks = chunks;
            document.saved_revision = revision;
        }
        Ok(SaveOutcome {
            generation: self.generation_number(),
            written: wrote,
            source_files,
            cache_stale,
        })
    }

    /// chunk 전용 session이면 그 이유(디스크 다시 읽기가 원본을 보지 않게).
    fn chunk_only(&self) -> Option<String> {
        self.chunk_only_reason().map(str::to_owned)
    }

    /// UseLatestDisk(XII.7.6): 디스크의 최신 원본·generation을 검증하고 이 창 문서를 다시 읽은 **뒤에만** 로컬 미저장
    /// 편집과 이 창 history를 버리고 그것으로 바꿉니다. 열린 탭은 새 문서에 남은 View만 viewport와 함께 유지합니다
    /// (없으면 첫 View). 읽기에 실패하면 아무것도 바꾸지 않습니다. 버린 history 항목 수(undo + redo)를 돌려줍니다.
    pub fn reload_latest_disk(&mut self) -> Result<usize, WindowSessionLoadError> {
        let pick = self
            .document_id()
            .map_or(DocumentPick::FirstRoot, DocumentPick::Document);
        let chunk_only = self.chunk_only();
        let disk = read_disk_state(&self.identity, pick, chunk_only.as_deref())?;
        // 여기부터 실패 없음(맞춤 쓰기 실패는 다음 저장이 다시 맞춤).
        let discarded = self.history.history().len() + self.history.history().redo_len();
        let reconcile = disk.needs_reconcile();
        let revision = DocumentRevision(self.revision().0.saturating_add(1));
        let old_tabs = self.tabs().clone();
        self.adopt(disk, revision, Some(&old_tabs));
        // 맞춤 쓰기는 document thread가 아니라 다음 저장(자동 저장 writer)이 합니다(기술 아키텍처 §6.4).
        self.reconcile_pending = reconcile;
        Ok(discarded)
    }

    /// 디스크의 외부 변경을 이 창에 반영합니다(UX.4.1·UX.4.4, ADR-0083 결정 3, ADR-0088 결정 5).
    ///
    /// | 상태 | 결과 |
    /// |---|---|
    /// | current pointer·원본 = 이 창 base(자기 저장 event 포함) | [`ExternalSync::Unchanged`] |
    /// | 제스처(history preview) 진행 중 | [`ExternalSync::Deferred`]. 제스처가 끝난 뒤 다시 부릅니다 |
    /// | 미저장 편집 있음 | [`ExternalSync::Stale`](두 선택지 또는 reconciliation 대기). 문서는 그대로 |
    /// | clean, 새 디스크 상태가 정상 | [`ApplyExternalSnapshot`] 하나를 이 창 history에 기록하고 base를 바꿉니다. Undo 한 번이 외부 변경 전체를 되돌립니다. 선택은 남은 node만 유지합니다. 원본·cache 가운데 뒤처진 쪽을 맞춰 씁니다 |
    /// | 새 generation·원본이 깨짐 | [`ExternalSync::Rejected`]. 아무것도 반영하지 않습니다 |
    ///
    /// `command`는 반영 transaction의 id입니다(창 id와 generation hash에서 만든 값을 권장).
    pub fn sync_external(&mut self, command: CommandId) -> ExternalSync {
        // 문서 밖 의미 파일(컴포넌트·token)만 바뀌었으면 편집 여부와 관계없이 받습니다. 창 history 밖이고(ADR-0079
        // 결정 2) 이 창의 저장은 그 파일을 바꾸지 않습니다.
        if let Some(applied) = self.adopt_library_change() {
            return applied;
        }
        let current = match self.stale_state() {
            Ok(None) => return ExternalSync::Unchanged,
            Ok(Some(state)) => state,
            Err(error) => return ExternalSync::Rejected(ExternalRejection::Pointer(error)),
        };
        if self.history.history().preview_active() {
            return ExternalSync::Deferred;
        }
        if self.has_unsaved_changes() {
            let (writer, resolution) = current;
            return ExternalSync::Stale { writer, resolution };
        }
        let Some(document_id) = self.document_id() else {
            // 문서가 없던 창(저장 전 프로젝트)은 history 없이 새 디스크 상태를 엽니다.
            return match self.reload_latest_disk() {
                Ok(_) => ExternalSync::Applied {
                    generation: self.generation_number(),
                    changes: None,
                },
                Err(error) => ExternalSync::Rejected(ExternalRejection::Load(error)),
            };
        };
        let chunk_only = self.chunk_only();
        let disk = match read_disk_state(
            &self.identity,
            DocumentPick::Document(document_id),
            chunk_only.as_deref(),
        ) {
            Ok(disk) => disk,
            Err(error) => return ExternalSync::Rejected(ExternalRejection::Load(error)),
        };
        if disk.loaded.is_none() && disk.plan != OpenPlan::FromText {
            return ExternalSync::Rejected(ExternalRejection::PointerMissing);
        }
        let Some(model) = disk.model else {
            return ExternalSync::Rejected(ExternalRejection::Load(
                WindowSessionLoadError::Document(LoadDocumentError::UnknownDocument {
                    document: document_id,
                }),
            ));
        };
        let document = self
            .document
            .as_mut()
            .expect("document_id가 있으면 문서도 있음");
        let incoming = match model.to_records() {
            Ok(records) => records,
            Err(error) => {
                return ExternalSync::Rejected(ExternalRejection::Records(error.to_string()))
            }
        };
        let apply = match ApplyExternalSnapshot::between(document.state.model(), &incoming) {
            Ok(apply) => apply,
            Err(error) => return ExternalSync::Rejected(ExternalRejection::Snapshot(error)),
        };
        let mut changes = None;
        if !apply.is_empty() {
            let discarded = {
                let history = self.history.history();
                history
                    .redo_entries()
                    .iter()
                    .map(|e| e.sequence)
                    .collect::<Vec<_>>()
            };
            let transaction = apply.into_transaction(command, document.state.revision());
            changes = match self
                .history
                .record_document(&mut document.state, &transaction, None)
            {
                Ok(changes) => Some(changes),
                Err(error) => return ExternalSync::Rejected(ExternalRejection::Record(error)),
            };
            let _ = self
                .history
                .view_mut()
                .tabs_mut()
                .prune_missing(document.state.model());
            if self.history.view().tabs().is_empty() {
                if let Some(view) = first_view(document.state.model()) {
                    let _ = self
                        .history
                        .view_mut()
                        .tabs_mut()
                        .open(document.state.model(), view);
                }
            }
            document.saved_revision = document.state.revision();
            self.labeled(discarded, history_label::NONE);
        }
        // 활성 기본값 세트는 레코드 밖 거울이라 변경 집합에 들지 않습니다. 거울과 그 기준값을 그대로 두어 외부에서 바뀐
        // `stellar.yaml`을 되쓰지 않습니다(다시 열면 디스크 값을 읽음).
        if let Some(document) = self.document.as_mut() {
            document.owned_chunks = disk.owned;
        }
        let reconcile = matches!(disk.source, SourceMode::Text { .. })
            && matches!(
                disk.plan,
                OpenPlan::Forward | OpenPlan::FromText | OpenPlan::Migrate
            );
        self.asset_chunks = std::sync::Arc::new(
            disk.loaded
                .as_ref()
                .map(asset_chunks_of)
                .unwrap_or_default(),
        );
        self.loaded = disk.loaded;
        self.source = disk.source;
        self.open_plan = disk.plan;
        // 맞춤 쓰기는 document thread가 아니라 다음 저장(자동 저장 writer)이 합니다(기술 아키텍처 §6.4).
        self.reconcile_pending = reconcile;
        ExternalSync::Applied {
            generation: self.generation_number(),
            changes,
        }
    }

    /// 원본이 문서 밖 의미 파일에서만 바뀌었으면 그 의미와 base를 받습니다. 해당 없으면 `None`입니다.
    fn adopt_library_change(&mut self) -> Option<ExternalSync> {
        let base = self.source.base()?;
        let current = read_current_pointer(self.identity.canonical_target()).ok();
        if current != self.loaded.as_ref().map(|loaded| loaded.manifest_hash) {
            return None;
        }
        let files = library_moved(&self.identity, base)?;
        let project = match decode_source(&files) {
            Ok(project) => project,
            Err(error) => {
                let all: BTreeSet<String> = base.changed_paths(&files.base);
                return Some(ExternalSync::Rejected(ExternalRejection::Load(
                    WindowSessionLoadError::Source(
                        stellar_project::external_change::source::diagnose(
                            &files.files,
                            &all,
                            &error,
                        ),
                    ),
                )));
            }
        };
        let (_, library) = SourceLibrary::split(project);
        self.source = SourceMode::Text {
            base: files.base,
            library: Some(std::sync::Arc::new(library)),
        };
        // stamp 짝을 맞추는 일만 남습니다(cache에는 컴포넌트·token이 없음).
        self.reconcile_pending = true;
        Some(ExternalSync::Applied {
            generation: self.generation_number(),
            changes: None,
        })
    }

    /// 선택을 바꾸고 이 창 history에 기록합니다(같은 선택이면 기록하지 않음).
    pub fn select(
        &mut self,
        selection: Selection,
    ) -> Result<Option<HistorySequence>, WindowSessionEditError> {
        let discarded = self.redo_sequences();
        let document = self
            .document
            .as_ref()
            .ok_or(WindowSessionEditError::NoDocument)?;
        let recorded = self
            .history
            .record_selection(&document.state, &SetSelection::new(selection))
            .map_err(WindowSessionEditError::History)?;
        self.labeled(discarded, history_label::NONE);
        Ok(recorded)
    }
}

/// session 편집 실패. 문서·선택은 바뀌지 않았습니다.
#[derive(Debug, Clone, PartialEq)]
pub enum WindowSessionEditError {
    /// 작업 문서가 없습니다(저장된 문서가 없는 프로젝트).
    NoDocument,
    /// history 기록 또는 transaction 실행 실패.
    History(HistoryError),
    /// 탭·viewport 조작 실패.
    View(ViewTabsError),
    /// 이동·확대 결과 viewport가 유효하지 않음(유한 값 아님).
    Viewport(ViewportError),
}

impl fmt::Display for WindowSessionEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WindowSessionEditError::NoDocument => f.write_str("편집할 문서가 없습니다"),
            WindowSessionEditError::History(error) => write!(f, "{error}"),
            WindowSessionEditError::View(error) => write!(f, "{error}"),
            WindowSessionEditError::Viewport(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for WindowSessionEditError {}

#[cfg(test)]
pub(crate) mod test_fixtures {
    //! 저장된 문서가 있는 임시 프로젝트 fixture(테스트 전용).

    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use stellar_commands::command::Command;
    use stellar_commands::document::definitions::{
        DocumentDefinition, PageDefinition, ViewDefinition,
    };
    use stellar_commands::document::frame::FramePayload;
    use stellar_commands::document::model::DocumentModel;
    use stellar_commands::document::records::{NodePayload, NodeRecord};
    use stellar_commands::id::{CommandId, CommandOrigin};
    use stellar_commands::state::DocumentState;
    use stellar_commands::transaction::CommandTransaction;
    use stellar_core_types::ids::{StableId, WindowSessionId};
    use stellar_core_types::order_key::OrderKey;
    use stellar_project::document_io::serialize::serialize_document;
    use stellar_project::identity::{parse_project_id, ProjectIdentity};
    use stellar_project::scaffold::write_default_scaffold;
    use stellar_storage::save::cancel::{run_save, CancelToken};
    use stellar_storage::save::commit::SavedGenerationState;
    use stellar_storage::save::plan::build_save_plan;

    pub const PROJECT_ID: &str = "0195c530-3acd-7bc1-8414-c213f61cb965";

    /// drop 시 지우는 임시 디렉터리.
    pub struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        pub fn new(tag: &str) -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "stellar-window-session-{tag}-{}-{n}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }

        pub fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// 결정론적 id.
    pub fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    pub fn window(n: u64) -> WindowSessionId {
        WindowSessionId::from_stable_id(StableId::new_v7(9_000 + n, 0x0123, n))
    }

    fn frame(id: u64, parent: Option<u64>, key: u8) -> NodeRecord {
        NodeRecord {
            id: sid(id),
            parent: parent.map(sid),
            order_key: OrderKey::from_bytes(vec![key]).expect("key"),
            visible: true,
            locked: false,
            display_name: format!("n{id}"),
            payload: NodePayload::Frame(FramePayload::default()),
        }
    }

    /// 문서(1): 페이지(2) 아래 View 3(root 10)·View 4(root 20). 10 아래 11·12.
    pub fn sample_document() -> DocumentModel {
        let mut definition = DocumentDefinition::new(sid(1), "문서".to_string());
        let mut page = PageDefinition::new(sid(2), "페이지".to_string());
        page.add_view(ViewDefinition::new(sid(3), "첫 화면".to_string(), sid(10)))
            .unwrap();
        page.add_view(ViewDefinition::new(
            sid(4),
            "둘째 화면".to_string(),
            sid(20),
        ))
        .unwrap();
        definition.add_page(page).unwrap();
        let mut model = DocumentModel::new(definition);
        model.insert_node(frame(10, None, 0x10)).unwrap();
        model.insert_node(frame(11, Some(10), 0x10)).unwrap();
        model.insert_node(frame(12, Some(10), 0x20)).unwrap();
        model.insert_node(frame(20, None, 0x20)).unwrap();
        model
    }

    /// `stellar.yaml` + `.git` + [`sample_document`]를 저장한 generation 1을 가진 프로젝트.
    pub fn saved_project(tag: &str) -> TempDir {
        saved_project_with(tag, &[&sample_document()])
    }

    /// `documents`를 root 문서 순서로 저장한 generation 1을 가진 프로젝트.
    pub fn saved_project_with(tag: &str, documents: &[&DocumentModel]) -> TempDir {
        let dir = TempDir::new(tag);
        write_default_scaffold(dir.path(), "창 session", PROJECT_ID).unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        let serialized: Vec<_> = documents
            .iter()
            .map(|model| serialize_document(model).unwrap())
            .collect();
        let states: Vec<_> = serialized
            .iter()
            .flat_map(|document| document.to_dirty_states())
            .collect();
        let roots = serialized
            .iter()
            .map(|document| document.document_id)
            .collect();
        let plan = build_save_plan(None, project_id(), 1, &states, roots).unwrap();
        let mut memory = SavedGenerationState::new(None);
        run_save(dir.path(), &plan, &mut memory, &CancelToken::new()).unwrap();
        dir
    }

    /// 둘째 문서(500): View 501(root 510) 하나, root 아래 511.
    pub fn other_document() -> DocumentModel {
        let mut definition = DocumentDefinition::new(sid(500), "다른 문서".to_string());
        let mut page = PageDefinition::new(sid(502), String::new());
        page.add_view(ViewDefinition::new(sid(501), String::new(), sid(510)))
            .unwrap();
        definition.add_page(page).unwrap();
        let mut model = DocumentModel::new(definition);
        model.insert_node(frame(510, None, 0x10)).unwrap();
        model.insert_node(frame(511, Some(510), 0x10)).unwrap();
        model
    }

    /// 저장한 generation 없이 scaffold만 있는 프로젝트.
    pub fn unsaved_project(tag: &str) -> TempDir {
        let dir = TempDir::new(tag);
        write_default_scaffold(dir.path(), "빈 프로젝트", PROJECT_ID).unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        dir
    }

    pub fn project_id() -> StableId {
        parse_project_id(PROJECT_ID).unwrap()
    }

    pub fn identity(root: &Path) -> ProjectIdentity {
        ProjectIdentity::new(project_id(), root).unwrap()
    }

    /// 현재 revision 기준 이름 변경 transaction.
    pub fn rename(doc: &DocumentState, command: u64, node: u64, name: &str) -> CommandTransaction {
        CommandTransaction::new(
            CommandId::from_stable_id(StableId::new_v7(command, 0x0c0c, command)),
            CommandOrigin::User,
        )
        .with_command(
            doc.revision(),
            Command::rename_node(sid(node), name.to_string()),
        )
    }
}

#[cfg(test)]
mod stale_tests;

#[cfg(test)]
mod source_tests;

#[cfg(test)]
mod source_bench;

#[cfg(test)]
mod tests {
    use super::test_fixtures::*;
    use super::*;
    use stellar_commands::document::test_support::semantic_snapshot::model_snapshot_json;

    fn snapshot(session: &ProjectWindowSession) -> String {
        model_snapshot_json(session.document().unwrap().model()).unwrap()
    }

    /// 저장 결과의 (generation, 새 generation을 썼는지).
    fn generation_written(outcome: SaveOutcome) -> (u64, bool) {
        (outcome.generation, outcome.written)
    }

    #[test]
    fn loads_identity_window_document_history_and_generation() {
        let project = saved_project("load");
        let session = ProjectWindowSession::load(identity(project.path()), window(1)).unwrap();
        assert_eq!(session.identity().project_id(), project_id());
        assert_eq!(session.window(), window(1));
        assert_eq!(session.document_id(), Some(sid(1)));
        assert_eq!(session.revision(), DocumentRevision(0));
        assert_eq!(session.generation_number(), 1);
        assert!(session.loaded_generation().is_some());
        assert!(session.history().history().is_empty());
        assert_eq!(session.view().window(), window(1));
        assert!(session.view().selection().is_empty());
        assert_eq!(
            snapshot(&session),
            model_snapshot_json(&sample_document()).unwrap()
        );
        // 작업 문서 chunk: manifest + ViewTree 2 + subtree 2(root 10·20).
        let owned = session.document_chunks().unwrap();
        assert_eq!(owned.len(), 5);
        let pointers: BTreeSet<ChunkId> = session
            .loaded_generation()
            .unwrap()
            .manifest
            .chunk_pointers
            .iter()
            .map(|p| p.chunk_id)
            .collect();
        assert_eq!(
            owned, &pointers,
            "문서가 하나뿐이면 generation chunk 전부가 그 문서의 것"
        );
    }

    #[test]
    fn two_sessions_do_not_share_state() {
        // 검증 기준(XII.2.1): 두 session이 서로 state를 공유하지 않습니다.
        let project = saved_project("isolation");
        let mut a = ProjectWindowSession::load(identity(project.path()), window(1)).unwrap();
        let mut b = ProjectWindowSession::load(identity(project.path()), window(2)).unwrap();
        let original = snapshot(&a);
        assert_eq!(snapshot(&b), original);
        assert_ne!(a.window(), b.window());

        // A 편집: B의 문서·revision·history는 그대로.
        let tx = rename(a.document().unwrap(), 100, 11, "A가 바꾼 이름");
        a.apply(&tx, 0).unwrap();
        assert_eq!(a.revision(), DocumentRevision(1));
        assert_ne!(snapshot(&a), original);
        assert_eq!(snapshot(&b), original, "B 문서는 A 편집을 보지 않습니다");
        assert_eq!(b.revision(), DocumentRevision(0));
        assert_eq!(a.history().history().len(), 1);
        assert!(b.history().history().is_empty());

        // B 선택: A 선택·history는 그대로.
        b.select(Selection::new([sid(12)], None)).unwrap();
        assert_eq!(b.view().selection().ids(), &[sid(12)]);
        assert!(a.view().selection().is_empty());
        assert_eq!(a.history().history().len(), 1);
        assert_eq!(b.history().history().len(), 1);

        // B 편집은 A가 바꾼 node를 보지 않은 B 문서 위에서 실행됩니다.
        let tx = rename(b.document().unwrap(), 200, 11, "B가 바꾼 이름");
        b.apply(&tx, 0).unwrap();
        let name = |session: &ProjectWindowSession| {
            session
                .document()
                .unwrap()
                .model()
                .record(sid(11))
                .unwrap()
                .display_name
        };
        assert_eq!(name(&a), "A가 바꾼 이름");
        assert_eq!(name(&b), "B가 바꾼 이름");

        // loaded generation은 각 session이 따로 들고 있습니다(같은 디스크 세대).
        assert_eq!(a.generation_number(), b.generation_number());
        assert!(!std::ptr::eq(
            a.loaded_generation().unwrap(),
            b.loaded_generation().unwrap()
        ));
        // A를 버려도 B는 그대로 동작합니다.
        drop(a);
        assert_eq!(name(&b), "B가 바꾼 이름");
        assert_eq!(b.revision(), DocumentRevision(1));
    }

    #[test]
    fn view_state_opens_the_first_view_and_stays_per_window() {
        // 검증 기준(XII.2.5): 창간 상태 격리 — 탭·active view·viewport·선택.
        let project = saved_project("view-state");
        let mut a = ProjectWindowSession::load(identity(project.path()), window(11)).unwrap();
        let mut b = ProjectWindowSession::load(identity(project.path()), window(12)).unwrap();
        for session in [&a, &b] {
            assert_eq!(session.tabs().len(), 1);
            assert_eq!(session.tabs().active_view(), Some(sid(3)), "첫 View");
            assert_eq!(
                session.tabs().active_viewport(),
                Some(CanvasViewport::DEFAULT)
            );
        }
        let saved_records = a.document().unwrap().model().to_records().unwrap();

        // A: 둘째 View 탭을 열고 확대·이동, 선택.
        assert_eq!(a.open_view(sid(4)), Ok(true));
        let zoomed = CanvasViewport::new(320.0, -40.0, 3.0).unwrap();
        assert_eq!(a.set_viewport(zoomed), Ok(CanvasViewport::DEFAULT));
        a.select(Selection::new([sid(11)], None)).unwrap();
        // B: 첫 탭 viewport만 바꿉니다.
        let panned = CanvasViewport::new(-10.0, 5.0, 0.5).unwrap();
        b.set_viewport(panned).unwrap();

        assert_eq!(a.tabs().len(), 2);
        assert_eq!(a.tabs().active_view(), Some(sid(4)));
        assert_eq!(a.tabs().active_viewport(), Some(zoomed));
        assert_eq!(b.tabs().len(), 1, "A가 연 탭은 B에 없음");
        assert_eq!(b.tabs().active_view(), Some(sid(3)));
        assert_eq!(b.tabs().active_viewport(), Some(panned));
        assert_eq!(a.view().selection().ids(), &[sid(11)]);
        assert!(b.view().selection().is_empty());

        // 탭·viewport는 문서 revision·history·레코드를 바꾸지 않습니다(선택만 history에 1개).
        assert_eq!(a.revision(), DocumentRevision(0));
        assert_eq!(a.history().history().len(), 1);
        assert!(b.history().history().is_empty());
        assert_eq!(
            a.document().unwrap().model().to_records().unwrap(),
            saved_records
        );

        // A에서 첫 탭으로 돌아가면 그 탭 viewport는 기본값 그대로, 탭을 닫으면 B는 영향 없음.
        a.open_view(sid(3)).unwrap();
        assert_eq!(a.tabs().active_viewport(), Some(CanvasViewport::DEFAULT));
        a.close_view(sid(3)).unwrap();
        assert_eq!(a.tabs().active_view(), Some(sid(4)));
        assert_eq!(b.tabs().active_view(), Some(sid(3)));

        // 문서에 없는 View·열리지 않은 탭은 오류.
        assert_eq!(
            a.open_view(sid(99)),
            Err(WindowSessionEditError::View(ViewTabsError::UnknownView {
                view: sid(99)
            }))
        );
        assert_eq!(
            b.close_view(sid(4)),
            Err(WindowSessionEditError::View(ViewTabsError::NotOpen {
                view: sid(4)
            }))
        );
    }

    fn name_of(session: &ProjectWindowSession, node: u64) -> String {
        session
            .document()
            .unwrap()
            .model()
            .record(sid(node))
            .unwrap()
            .display_name
    }

    #[test]
    fn history_labels_follow_the_recorded_entry_and_drop_discarded_redo() {
        use super::history_label::{NONE, RENAME, REORDER};
        let project = saved_project("history-labels");
        let mut a = ProjectWindowSession::load(identity(project.path()), window(31)).unwrap();
        assert_eq!((a.undo_label(), a.redo_label()), (NONE, NONE));
        a.apply(&rename(a.document().unwrap(), 300, 11, "A1"), RENAME)
            .unwrap();
        a.apply(&rename(a.document().unwrap(), 301, 11, "A2"), REORDER)
            .unwrap();
        assert_eq!(a.undo_label(), REORDER);
        // 선택만 기록한 항목은 이름표가 없고, 앞 항목의 이름을 물려받지 않습니다.
        a.select(Selection::new([sid(12)], None)).unwrap();
        assert_eq!(a.undo_label(), NONE);
        assert_eq!(a.label_count(), 2);
        assert_eq!(a.undo(), Ok(true));
        assert_eq!(a.undo(), Ok(true));
        assert_eq!((a.undo_label(), a.redo_label()), (RENAME, REORDER));
        // 새 확정은 redo 갈래를 버리고 그 이름표도 표에서 지웁니다.
        a.apply(&rename(a.document().unwrap(), 302, 11, "A3"), RENAME)
            .unwrap();
        assert_eq!((a.undo_label(), a.redo_label()), (RENAME, NONE));
        assert_eq!(a.label_count(), 2);
        // 실패한 기록은 이름표를 바꾸지 않습니다.
        assert!(a
            .apply(&rename(a.document().unwrap(), 303, 99, "없음"), REORDER)
            .is_err());
        assert_eq!(a.undo_label(), RENAME);
        // 반복 편집을 한 표에서 1만 번 해도 표는 살아 있는 항목 수를 넘지 않습니다.
        for index in 0..10_000u64 {
            a.apply(
                &rename(a.document().unwrap(), 400 + index, 11, &format!("n{index}")),
                RENAME,
            )
            .unwrap();
            if index % 2 == 0 {
                assert_eq!(a.undo(), Ok(true));
            }
        }
        let live = a.history().history().len() + a.history().history().redo_len();
        assert!(a.label_count() <= live, "{} > {live}", a.label_count());
    }

    #[test]
    fn undo_and_redo_replay_only_their_own_window_history() {
        // 검증 기준(XII.6.4): 창별 history.
        let project = saved_project("undo-redo");
        let mut a = ProjectWindowSession::load(identity(project.path()), window(21)).unwrap();
        let mut b = ProjectWindowSession::load(identity(project.path()), window(22)).unwrap();
        a.apply(&rename(a.document().unwrap(), 300, 11, "A1"), 0)
            .unwrap();
        a.apply(&rename(a.document().unwrap(), 301, 11, "A2"), 0)
            .unwrap();
        b.apply(&rename(b.document().unwrap(), 400, 12, "B1"), 0)
            .unwrap();
        b.select(Selection::new([sid(12)], None)).unwrap();

        // A undo: A의 마지막 편집만 되돌리고 B는 그대로.
        assert_eq!(a.undo(), Ok(true));
        assert_eq!(name_of(&a, 11), "A1");
        assert_eq!(name_of(&b, 12), "B1");
        assert_eq!(b.view().selection().ids(), &[sid(12)]);
        // B undo: B의 마지막 항목(선택)을 되돌립니다. 문서 이름은 그대로.
        assert_eq!(b.undo(), Ok(true));
        assert!(b.view().selection().is_empty());
        assert_eq!(name_of(&b, 12), "B1");
        assert_eq!(b.undo(), Ok(true));
        assert_eq!(name_of(&b, 12), "n12");
        assert_eq!(b.undo(), Ok(false), "B에는 더 되돌릴 것이 없음");
        assert_eq!(name_of(&a, 11), "A1", "B undo가 A 문서를 바꾸지 않음");

        // redo도 창별입니다.
        assert_eq!(a.redo(), Ok(true));
        assert_eq!(name_of(&a, 11), "A2");
        assert_eq!(a.redo(), Ok(false));
        assert_eq!(b.redo(), Ok(true));
        assert_eq!(name_of(&b, 12), "B1");
        assert_eq!(a.history().history().len(), 2);
        assert_eq!(b.history().history().redo_entries().len(), 1);
    }

    #[test]
    fn save_now_writes_this_windows_document_as_a_new_generation() {
        // 검증 기준(XII.6.4): 창별 save 호출.
        let project = saved_project("save-now");
        let mut a = ProjectWindowSession::load(identity(project.path()), window(31)).unwrap();
        let mut b = ProjectWindowSession::load(identity(project.path()), window(32)).unwrap();

        // 바뀐 것이 없으면 새 generation을 쓰지 않습니다.
        let current = std::fs::read(project.path().join("design").join("current")).unwrap();
        assert_eq!(generation_written(a.save_now().unwrap()), (1, false));
        assert_eq!(
            std::fs::read(project.path().join("design").join("current")).unwrap(),
            current
        );

        a.apply(&rename(a.document().unwrap(), 500, 11, "저장된 이름"), 0)
            .unwrap();
        b.apply(&rename(b.document().unwrap(), 600, 12, "B 편집"), 0)
            .unwrap();
        assert_eq!(generation_written(a.save_now().unwrap()), (2, true));
        assert_eq!(a.generation_number(), 2);
        assert_eq!(b.generation_number(), 1, "B의 loaded generation은 그대로");

        // 디스크에서 새로 연 창은 A의 편집을 보고 B의 저장 안 한 편집은 보지 않습니다.
        let reopened = ProjectWindowSession::load(identity(project.path()), window(33)).unwrap();
        assert_eq!(reopened.generation_number(), 2);
        assert_eq!(name_of(&reopened, 11), "저장된 이름");
        assert_eq!(name_of(&reopened, 12), "n12");

        // B는 오래된 base(1) 위에서 저장하려 하므로 아무것도 쓰지 않고 Stale입니다.
        let current = std::fs::read(project.path().join("design").join("current")).unwrap();
        assert!(matches!(
            b.save_now(),
            Err(WindowSessionSaveError::Stale(_))
        ));
        assert_eq!(
            std::fs::read(project.path().join("design").join("current")).unwrap(),
            current,
            "Stale 저장은 current pointer를 바꾸지 않음"
        );
        assert_eq!(b.generation_number(), 1);

        // A: 다시 저장할 것이 없으면 그대로, undo 후 저장하면 generation 3.
        assert!(!a.save_now().unwrap().written);
        assert_eq!(a.undo(), Ok(true));
        assert_eq!(generation_written(a.save_now().unwrap()), (3, true));
        let reopened = ProjectWindowSession::load(identity(project.path()), window(34)).unwrap();
        assert_eq!(name_of(&reopened, 11), "n11");
        assert_eq!(
            a.history().history().redo_entries().len(),
            1,
            "저장은 history를 바꾸지 않음"
        );
    }

    #[test]
    fn unchanged_save_checks_the_disk_even_after_undo_restores_saved_content() {
        let project = saved_project("save-revision");
        let mut a = ProjectWindowSession::load(identity(project.path()), window(31)).unwrap();
        let mut b = ProjectWindowSession::load(identity(project.path()), window(32)).unwrap();
        a.apply(&rename(a.document().unwrap(), 500, 11, "임시 이름"), 0)
            .unwrap();
        assert!(a.undo().unwrap());
        assert_eq!(generation_written(a.save_now().unwrap()), (1, false));
        assert_eq!(a.document.as_ref().unwrap().saved_revision, a.revision());
        assert!(!a.save_now().unwrap().written);

        b.apply(&rename(b.document().unwrap(), 600, 12, "외부 창 편집"), 0)
            .unwrap();
        assert!(b.save_now().unwrap().written);
        assert!(matches!(
            a.save_now(),
            Err(WindowSessionSaveError::Stale(
                BaseCheckError::StaleBase { .. }
            ))
        ));
        assert_eq!(a.generation_number(), 1);
        assert_eq!(name_of(&a, 12), "n12");
    }

    /// 자식 프로세스만 임시 OS 사용자 데이터 환경에서 실제 Save Now를 실행합니다.
    #[test]
    fn cli_generation_lock_and_window_save_share_one_identity() {
        use std::process::Command;
        use stellar_ops::test_support::file_digest;

        const PROJECT: &str = "STELLAR_TEST_WINDOW_SAVE_PROJECT";
        if let Some(root) = std::env::var_os(PROJECT) {
            let mode = std::env::var("STELLAR_TEST_SAVE_MODE").unwrap();
            // 열기의 원본 전환(ADR-0088)은 저장 잠금이 필요합니다. 잠금을 얻을 수 있는 mode에서만 열기가 원본을 맞추므로,
            // 되돌린 편집이 아무것도 쓰지 않음은 그 mode에서 확인합니다.
            let mut session =
                ProjectWindowSession::load(identity(root.as_ref()), window(1)).unwrap();
            if mode == "write" {
                assert!(!session.save_now().unwrap().wrote_anything());
            }
            session
                .apply(&rename(session.document().unwrap(), 500, 11, "앱 변경"), 0)
                .unwrap();
            assert!(session.undo().unwrap());
            if mode == "write" {
                assert!(!session.save_now().unwrap().wrote_anything());
            }
            assert!(session.redo().unwrap());
            match mode.as_str() {
                "write" => {
                    assert!(session.save_now().unwrap().written);
                    assert_eq!(session.generation_number(), 2);
                }
                mode => {
                    let error = session.save_now().unwrap_err();
                    match mode {
                        "busy" => assert!(matches!(
                            error,
                            WindowSessionSaveError::Lock(LockError::Busy { .. })
                        )),
                        "missing-home" => assert!(matches!(
                            error,
                            WindowSessionSaveError::Lock(LockError::NoUserDataDir)
                        )),
                        _ => panic!("알 수 없는 fixture mode"),
                    }
                    assert!(session.has_unsaved_changes());
                    assert_eq!(session.generation_number(), 1);
                    assert_eq!(name_of(&session, 11), "앱 변경");
                }
            }
            return;
        }

        let project = saved_project("cli-lock");
        let home = TempDir::new("lock-user-data");
        let base = if cfg!(target_os = "macos") {
            home.path().join("Library/Application Support")
        } else if cfg!(windows) {
            home.path().join("AppData/Roaming")
        } else {
            home.path().join(".local/share")
        };
        let dir = base
            .join(stellar_ops::lock::APP_DATA_DIR)
            .join(stellar_ops::lock::LOCKS_DIR);
        #[cfg(unix)]
        let opened = home.path().join("opened-project");
        #[cfg(unix)]
        std::os::unix::fs::symlink(project.path(), &opened).unwrap();
        #[cfg(not(unix))]
        let opened = project.path();
        let run = |mode: &str| {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "window_session::tests::cli_generation_lock_and_window_save_share_one_identity",
                ])
                .env_remove("XDG_DATA_HOME")
                .env_remove("HOME")
                .env_remove("APPDATA")
                .env("STELLAR_TEST_SAVE_MODE", mode)
                .env(PROJECT, &opened);
            if mode != "missing-home" {
                command.env("HOME", home.path()).env("APPDATA", &base);
            }
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
        };
        let before = file_digest(project.path());
        run("missing-home");
        assert!(!dir.exists());
        let held = SaveLock::try_acquire(&dir, project_id(), project.path()).unwrap();
        run("busy");
        assert_eq!(file_digest(project.path()), before);
        drop(held);
        run("write");
        let reopened = ProjectWindowSession::load(identity(project.path()), window(2)).unwrap();
        assert_eq!(name_of(&reopened, 11), "앱 변경");
    }

    #[cfg(unix)]
    #[test]
    fn saving_after_a_symlink_is_repointed_updates_the_opened_project() {
        use std::os::unix::fs::symlink;
        let first = saved_project("alias-original");
        let second = saved_project("alias-new");
        let dir = TempDir::new("alias");
        let alias = dir.path().join("project");
        symlink(first.path(), &alias).unwrap();
        let mut session = ProjectWindowSession::load(identity(&alias), window(1)).unwrap();
        session
            .apply(
                &rename(session.document().unwrap(), 500, 11, "원래 프로젝트"),
                0,
            )
            .unwrap();
        std::fs::remove_file(&alias).unwrap();
        symlink(second.path(), &alias).unwrap();
        assert!(session.save_now().unwrap().written);
        let original = ProjectWindowSession::load(identity(first.path()), window(2)).unwrap();
        let other = ProjectWindowSession::load(identity(&alias), window(3)).unwrap();
        assert_eq!(name_of(&original, 11), "원래 프로젝트");
        assert_eq!(other.generation_number(), 1);
        assert_eq!(name_of(&other, 11), "n11");
    }

    #[test]
    fn save_now_keeps_other_documents_of_the_generation() {
        let project =
            saved_project_with("save-other-docs", &[&sample_document(), &other_document()]);
        let mut session = ProjectWindowSession::load(identity(project.path()), window(41)).unwrap();
        assert_eq!(session.document_id(), Some(sid(1)), "첫 root 문서");
        let base_pointers = session
            .loaded_generation()
            .unwrap()
            .manifest
            .chunk_pointers
            .clone();
        let other_chunks: Vec<_> = base_pointers
            .iter()
            .filter(|pointer| {
                !session
                    .document_chunks()
                    .unwrap()
                    .contains(&pointer.chunk_id)
            })
            .copied()
            .collect();
        assert_eq!(
            other_chunks.len(),
            3,
            "다른 문서 chunk(manifest·ViewTree·subtree)"
        );

        session
            .apply(
                &rename(session.document().unwrap(), 700, 12, "첫 문서 편집"),
                0,
            )
            .unwrap();
        assert!(session.save_now().unwrap().written);
        let saved = session.loaded_generation().unwrap();
        assert_eq!(saved.manifest.root_document_ids, vec![sid(1), sid(500)]);
        for pointer in &other_chunks {
            assert!(
                saved.manifest.chunk_pointers.contains(pointer),
                "다른 문서 chunk를 그대로 이어 씀"
            );
        }
        // 둘째 문서는 저장 뒤에도 디스크에서 그대로 로드됩니다.
        let project_id = project_id();
        let lazy = LazyGeneration::open(project.path(), project_id, &mut ChunkCollector::default())
            .unwrap();
        let other =
            load_document_from_generation(&lazy, sid(500), &mut ChunkCollector::default()).unwrap();
        assert_eq!(
            model_snapshot_json(&other.model).unwrap(),
            model_snapshot_json(&other_document()).unwrap()
        );
    }

    #[test]
    fn unsaved_project_opens_without_document_or_generation() {
        let project = unsaved_project("unsaved");
        let mut session = ProjectWindowSession::load(identity(project.path()), window(3)).unwrap();
        assert!(session.document().is_none());
        assert!(session.loaded_generation().is_none());
        assert_eq!(session.generation_number(), 0);
        assert_eq!(
            session.select(Selection::empty()),
            Err(WindowSessionEditError::NoDocument)
        );
        assert!(session.tabs().is_empty());
        assert_eq!(
            session.open_view(sid(3)),
            Err(WindowSessionEditError::NoDocument)
        );
        assert_eq!(
            session.set_viewport(CanvasViewport::DEFAULT),
            Err(WindowSessionEditError::View(ViewTabsError::NoActiveTab))
        );
        assert_eq!(session.undo(), Ok(false));
        assert_eq!(session.redo(), Ok(false));
        assert_eq!(generation_written(session.save_now().unwrap()), (0, false));
        assert!(!project.path().join("design").join("current").exists());
    }

    #[test]
    fn corrupt_generation_is_a_typed_error() {
        let project = saved_project("corrupt");
        // current pointer가 가리키는 manifest를 지우면 열기 검증이 실패합니다.
        let manifest_dir = project.path().join("design").join("generations");
        assert!(manifest_dir.is_dir(), "generation manifest 디렉터리");
        std::fs::remove_dir_all(&manifest_dir).unwrap();
        let error = ProjectWindowSession::load(identity(project.path()), window(4)).unwrap_err();
        assert!(
            matches!(error, WindowSessionLoadError::Generation(_)),
            "{error}"
        );
        assert!(error.to_string().contains("generation"));
    }

    /// `root` 아래 일반 파일(상대 경로 → bytes). 숨김 파일(`.git` 등)은 넣지 않습니다.
    fn tree(root: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        fn walk(
            base: &std::path::Path,
            dir: &std::path::Path,
            out: &mut std::collections::BTreeMap<String, Vec<u8>>,
        ) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                if path.is_dir() {
                    walk(base, &path, out);
                } else {
                    let relative = path.strip_prefix(base).unwrap().to_string_lossy();
                    out.insert(relative.replace('\\', "/"), std::fs::read(&path).unwrap());
                }
            }
        }
        let mut out = std::collections::BTreeMap::new();
        walk(root, root, &mut out);
        out
    }

    #[test]
    fn committed_fixture_project_matches_the_sample_project() {
        // 셸 검사(`--window-session-check`)와 FFI 테스트가 복사해 쓰는 저장 프로젝트 fixture입니다. 저장 형식이
        // 바뀌면 `STELLAR_UPDATE_FIXTURES=1 cargo test -p stellar-runtime committed_fixture`로 다시 만듭니다.
        let generated = saved_project("fixture");
        let expected = tree(generated.path());
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("window-session-project");
        if std::env::var("STELLAR_UPDATE_FIXTURES").as_deref() == Ok("1") {
            for (relative, bytes) in &expected {
                let path = fixture.join(relative);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, bytes).unwrap();
            }
        }
        assert_eq!(
            tree(&fixture),
            expected,
            "fixture가 sample 프로젝트 저장 결과와 다릅니다"
        );
        // fixture 그대로 열면(.git만 더함) 같은 작업 문서가 로드됩니다.
        let copy = TempDir::new("fixture-copy");
        for (relative, bytes) in &expected {
            let path = copy.path().join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        let session = ProjectWindowSession::load(identity(copy.path()), window(5)).unwrap();
        assert_eq!(
            snapshot(&session),
            model_snapshot_json(&sample_document()).unwrap()
        );
    }

    /// world (x, y)에 크기 w×h인 Frame node 하나씩을 둔 scene(테스트용, z = 1부터 순서대로).
    pub(crate) fn frame_scene(
        frames: &[(u64, f32, f32, f32, f32)],
    ) -> stellar_scene::snapshot::SceneSnapshot {
        use stellar_core_types::revision::SceneRevision;
        use stellar_core_types::value::Transform2D;
        use stellar_scene::builder::{FrameClipInput, FrameInput, NodeInput, SceneBuilder};
        use stellar_scene::node::CornerRadii;

        let mut builder = SceneBuilder::new();
        let mut root = builder.root_scope();
        for &(node, x, y, width, height) in frames {
            let input = FrameInput {
                node: NodeInput::new(
                    sid(node),
                    Transform2D::new(1.0, 0.0, 0.0, 1.0, x, y).unwrap(),
                ),
                size: Size2::new(width, height).unwrap(),
                corner_radii: CornerRadii::ZERO,
                background: None,
                border: None,
                clip: FrameClipInput::None,
            };
            let frame = builder.begin_frame(&root, &input).unwrap();
            builder.finish_frame(&mut root, frame).unwrap();
        }
        let roots = builder.finish(root);
        stellar_scene::snapshot::SceneSnapshot::new(DocumentRevision(0), SceneRevision(0), roots)
            .unwrap()
    }

    fn overlay_style() -> OverlayStyle {
        use stellar_core_types::color::{Color, ColorSpace};
        use stellar_scene::transient::HandleMetrics;
        let accent = Color::new(ColorSpace::Srgb, 0.0, 0.48, 1.0, 1.0).unwrap();
        OverlayStyle {
            outline: accent,
            outline_width: 1.0,
            handle_fill: Color::new(ColorSpace::Srgb, 1.0, 1.0, 1.0, 1.0).unwrap(),
            handle_stroke: accent,
            handle_stroke_width: 1.0,
            handle_half: 4.0,
            hit: HandleMetrics::new(5.0, 4.0, 12.0).unwrap(),
        }
    }

    #[test]
    fn selection_overlay_renders_as_a_transient_layer_outside_the_project_snapshot() {
        // 검증 기준(XII.4.4): project snapshot에 overlay node 없음.
        use stellar_project::document_io::serialize::serialize_document;
        use stellar_renderer_core::clip_plan::ClipCapabilities;
        use stellar_renderer_core::pass_compiler::{compile_frame, CompileInput, EffectBoundaries};
        use stellar_renderer_core::render_graph::{ClearColor, SampleCount, TextureFormat};

        let project = saved_project("overlay");
        let design = project.path().join("design");
        // 열기는 기존 chunk 프로젝트를 텍스트 원본으로 옮깁니다(ADR-0088). 그 뒤의 bytes가 기준입니다.
        let mut session = ProjectWindowSession::load(identity(project.path()), window(61)).unwrap();
        let disk_before = tree(&design);
        let records_before = session.document().unwrap().model().to_records().unwrap();
        let chunks_before = serialize_document(session.document().unwrap().model()).unwrap();
        // 문서 View 3(root 10)의 scene: root 10 아래 11·12 위치를 둔 Frame scene.
        let scene = frame_scene(&[(11, 0.0, 0.0, 120.0, 80.0), (12, 200.0, 0.0, 60.0, 60.0)]);
        let scene_hash = scene.semantic_hash();
        let size = Size2::new(800.0, 600.0).unwrap();

        // 선택이 없으면 빈 overlay.
        let empty = session
            .selection_overlay(&scene, size, &overlay_style())
            .unwrap()
            .unwrap();
        assert!(empty.is_empty());

        session.select(Selection::new([sid(11)], None)).unwrap();
        session.zoom(2.0, 0.0, 0.0).unwrap();
        let overlay = session
            .selection_overlay(&scene, size, &overlay_style())
            .unwrap()
            .unwrap();
        assert_eq!(overlay.targets().len(), 1);
        assert_eq!(overlay.targets()[0].node, sid(11));
        // camera: view = world × 2 + (400, 300).
        assert_eq!(
            overlay.targets()[0].view_quad[2],
            stellar_core_types::value::Vec2::new(640.0, 460.0).unwrap()
        );
        assert_eq!(overlay.handle_count(), 4);
        assert_eq!(overlay.layer().node_count(), 1 + 4);
        assert_eq!(overlay.hit_layer().len(), 12, "회전 4·변 4·모서리 4");

        // overlay layer는 공통 pass compiler로 그릴 수 있는 transient layer입니다(항등 view, 화면 좌표).
        let screen =
            Viewport::new(size, stellar_core_types::value::Transform2D::identity()).unwrap();
        let plan = compile_frame(&CompileInput {
            snapshot: overlay.layer(),
            viewport: &screen,
            visible: None,
            surface_format: TextureFormat::Bgra8Unorm,
            clear: ClearColor::TRANSPARENT,
            boundaries: EffectBoundaries::Minimal,
            clip_caps: ClipCapabilities {
                stencil_bits: 8,
                samples: SampleCount::X1,
            },
        })
        .unwrap();
        assert!(
            plan.draw_count() >= 5,
            "외곽선 1 + handle 4 draw: {}",
            plan.draw_count()
        );

        // project snapshot(문서 scene·문서 모델·저장 chunk·디스크)에는 overlay node가 없습니다.
        assert_eq!(scene.semantic_hash(), scene_hash);
        assert_eq!(scene.node_count(), 2);
        assert_eq!(
            session.document().unwrap().model().to_records().unwrap(),
            records_before
        );
        assert_eq!(
            serialize_document(session.document().unwrap().model()).unwrap(),
            chunks_before
        );
        assert_eq!(generation_written(session.save_now().unwrap()), (1, false));
        assert_eq!(tree(&design), disk_before);

        // active 탭이 없으면 overlay도 없습니다.
        session.close_view(sid(3)).unwrap();
        assert!(session
            .selection_overlay(&scene, size, &overlay_style())
            .is_none());
    }

    #[test]
    fn pan_and_zoom_move_the_scene_camera_without_changing_saved_bytes() {
        // 검증 기준(XII.4.3): pan/zoom 후 save bytes 동일, viewport matrix는 scene camera 입력.
        use stellar_commands::view_tabs::ZOOM_STEP;
        use stellar_project::document_io::serialize::serialize_document;
        use stellar_scene::spatial::SceneSpatialIndex;
        use stellar_scene::visibility::visible_candidates;

        let project = saved_project("camera");
        let design = project.path().join("design");
        // 열기는 기존 chunk 프로젝트를 텍스트 원본으로 옮깁니다(ADR-0088). 그 뒤의 bytes가 기준입니다.
        let mut session = ProjectWindowSession::load(identity(project.path()), window(51)).unwrap();
        let disk_before = tree(&design);
        let chunks_before = serialize_document(session.document().unwrap().model()).unwrap();
        let size = Size2::new(800.0, 600.0).unwrap();

        // 기본 viewport(원점 중심, 100%): 문서 원점이 화면 중심(400, 300).
        let camera = session.camera(size).unwrap();
        assert_eq!(
            camera.view().components(),
            (1.0, 0.0, 0.0, 1.0, 400.0, 300.0)
        );
        // 문서 (0,0) 근처 Frame 11과 멀리 있는 Frame 12(x = 2000).
        let scene = frame_scene(&[
            (11, -50.0, -50.0, 100.0, 100.0),
            (12, 2_000.0, 0.0, 100.0, 100.0),
        ]);
        let index = SceneSpatialIndex::build(&scene);
        let visible = |camera: &Viewport| {
            visible_candidates(&index, camera)
                .items
                .iter()
                .map(|item| item.node.source())
                .collect::<Vec<_>>()
        };
        assert_eq!(visible(&camera), vec![sid(11)]);

        // 확대 ×2 → 이동(내용을 왼쪽으로 4000pt) → 축소 ÷2 at anchor → 100%.
        session.zoom(ZOOM_STEP, 0.0, 0.0).unwrap();
        assert_eq!(
            session.camera(size).unwrap().view().components(),
            (2.0, 0.0, 0.0, 2.0, 400.0, 300.0)
        );
        session.pan(-4_100.0, 0.0).unwrap();
        let moved = session.camera(size).unwrap();
        assert_eq!(
            session.tabs().active_viewport().unwrap().center_x(),
            2_050.0
        );
        assert_eq!(
            visible(&moved),
            vec![sid(12)],
            "camera가 scene 가시 후보를 바꿈"
        );
        session.zoom(1.0 / ZOOM_STEP, 100.0, 0.0).unwrap();
        session.zoom_to_actual_size().unwrap();
        assert_eq!(session.tabs().active_viewport().unwrap().zoom(), 1.0);
        assert!(
            session.camera(Size2::new(0.0, 600.0).unwrap()).is_none(),
            "0 크기 화면"
        );

        // 문서 geometry·revision·history는 그대로이고, 저장해도 새 bytes가 없습니다.
        assert_eq!(session.revision(), DocumentRevision(0));
        assert!(session.history().history().is_empty());
        assert_eq!(
            serialize_document(session.document().unwrap().model()).unwrap(),
            chunks_before
        );
        assert_eq!(generation_written(session.save_now().unwrap()), (1, false));
        assert_eq!(tree(&design), disk_before, "pan/zoom 후 저장 bytes 동일");

        // 이동·확대 오류는 viewport를 바꾸지 않습니다.
        let before = session.tabs().active_viewport();
        assert!(matches!(
            session.pan(f64::NAN, 0.0),
            Err(WindowSessionEditError::Viewport(_))
        ));
        assert!(matches!(
            session.zoom(0.0, 0.0, 0.0),
            Err(WindowSessionEditError::Viewport(_))
        ));
        assert_eq!(session.tabs().active_viewport(), before);
    }
}
