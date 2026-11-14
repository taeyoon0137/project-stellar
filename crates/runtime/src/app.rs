//! 셸 경계용 앱 runtime: 프로젝트 열기·닫기, 문서 열기·닫기, 완료 event와 상태 snapshot.
//!
//! 네이티브 셸이 C ABI(`stellar-ffi`)를 거쳐 부르는 코어 쪽 진입점입니다. 모든 요청은 기다리지
//! 않고 [`RequestId`]를 돌려주며, 실제 작업은 document thread(유일한 작성자, XII.1.1)에서
//! [`CommandPriority::Command`](crate::command_queue::CommandPriority::Command) job으로 실행됩니다.
//! 결과는 두 경로로만 셸에 돌아갑니다.
//!
//! - **완료 event**: 요청마다 정확히 하나의 [`RuntimeEvent`]를 outbox에 쌓습니다. 셸은 pull
//!   방식으로 꺼냅니다([`AppRuntime::poll_event`], [`AppRuntime::pop_event`]). callback은 없습니다
//!   (II.4.5).
//! - **상태 snapshot**: 요청이 성공하면 document thread가 불변 [`RuntimeState`]를 새로 만들어
//!   게시합니다(실패한 요청은 상태를 바꾸지 않으므로 게시하지 않습니다). 셸은 [`AppRuntime::state`]로 최신 snapshot을 읽습니다. 완료 event보다 snapshot을
//!   먼저 게시하므로, 완료 event를 받은 뒤 읽은 snapshot에는 그 요청의 결과가 들어 있습니다.
//!
//! UI thread가 잡는 lock은 outbox·snapshot 교체의 O(1) 임계 구역뿐이며 job 실행·용량을 기다리지
//! 않습니다(XII.1.2 "blocking UI call 없음").
//!
//! # 프로젝트 열기
//!
//! [`AppRuntime::open_project`]는 경로 규칙(절대 경로, NUL 없음)만 바로 검사하고, document thread에서
//! 다음 순서로 엽니다.
//!
//! 1. 열기 분류(IV.2.4): `.git`이 없으면 [`RequestFailure::NotARepository`], `.git`은 있지만
//!    `stellar.yaml`이 없으면 [`RequestFailure::NotAStellarProject`]. 자동 변환은 하지 않습니다.
//! 2. 신뢰되지 않은 프로젝트용 안전 probe(IV.5.3): 매니페스트 메타데이터만 읽고 Git·필터를 실행하지
//!    않으며, symlink 매니페스트는 따르지 않습니다. 신뢰 저장소 연결 전이므로 모든 프로젝트를
//!    신뢰되지 않음으로 엽니다. `stellar.yaml`은 loader 상한(1 MiB)까지만 읽으므로 거대한
//!    매니페스트도 document thread를 오래 붙잡지 않고
//!    [`ManifestCorrupt`](crate::app::RequestFailure::ManifestCorrupt)(`input-too-large`)가
//!    됩니다.
//! 3. 매니페스트 `project.id`를 [`StableId`]로 바꾸고 [`ProjectIdentity`](IV.2.3)를 만듭니다.
//!
//! 각 실패는 고정 코드([`RequestFailure`])와 한국어 메시지로 [`RuntimeEvent::RequestFailed`]가
//! 됩니다. 문서 청크는 읽지 않습니다.
//!
//! # 문서 열기·닫기
//!
//! 문서는 열린 프로젝트 session 아래에서 [`StableId`]로 엽니다. 같은 session에서 다시 열면 기존
//! 문서 session을 돌려줍니다(`already_open`). 열린 문서 집합은 document thread에 하나뿐이므로,
//! 다른 프로젝트 session이 이미 연 문서는 [`RequestFailure::DocumentInOtherProject`]로 거부합니다
//! (창별 독립 memory model은 XII.2.4 소관). 프로젝트를 닫으면 그 아래 문서도 닫습니다. 저장된 문서
//! 내용 로드는 아직 하지 않으며, 새로 연 문서는 빈 문서(revision 0)입니다.
//!
//! # 창 session (XII.2.2)
//!
//! 창 하나는 [`ProjectWindowSession`](crate::window_session::ProjectWindowSession) 하나를 소유합니다.
//! [`AppRuntime::open_window_session`]은 열린 프로젝트 session 아래에서 디스크의 current generation과 작업
//! 문서를 로드해 창 session을 만들고 [`WindowSessionHandle`]을 돌려줍니다. 셸의 창 controller는 이 handle
//! 하나를 보관하고 창을 닫을 때 [`AppRuntime::close_window_session`]으로 한 번 닫습니다. 프로젝트 session을
//! 닫으면 그 아래 창 session도 닫습니다. 창 session은 서로 문서·history·선택을 공유하지 않습니다(XII.2.1).
//!
//! # outbox 상한
//!
//! 완료 event를 꺼내지 않은 요청 수(실행 중 포함)가 상한([`AppRuntime::DEFAULT_EVENT_CAPACITY`])에
//! 닿으면 새 요청을 [`RequestError::QueueFull`]로 거부합니다. 접수한 요청의 완료 event는 버리지
//! 않습니다.
//!
//! # 종료
//!
//! [`AppRuntime`]을 drop하면(셸 FFI destroy) 이미 접수한 요청을 실행한 뒤 document thread를
//! join하되 최대 [`AppRuntime::SHUTDOWN_TIMEOUT`]까지만 기다립니다. 실행 중인 요청이 응답 없는 파일
//! 시스템 호출에서 멈춰도 앱 종료가 멈추지 않도록, 시간을 넘기면 아직 시작하지 않은 요청을 버리고
//! thread를 분리합니다. 끝까지 기다리는 종료는 [`AppRuntime::shutdown`]입니다.
//!
//! # event payload (little-endian)
//!
//! 모든 payload는 `u32 schema_version`([`EVENT_SCHEMA_VERSION`])으로 시작합니다. 문자열은
//! `u32 길이 + UTF-8 바이트`이며 [`EVENT_TEXT_MAX`] 바이트에서 문자 경계로 자릅니다. StableId는
//! [`StableId::to_bytes`] 16바이트입니다.
//!
//! | kind | 이름 | 이어지는 필드 |
//! |---|---|---|
//! | `0x0101` | ProjectOpened | request u64, project u64, project_id \[16\], name str, root str |
//! | `0x0102` | ProjectClosed | request u64, project u64, closed_documents u32 |
//! | `0x0103` | DocumentOpened | request u64, project u64, document u64, document_id \[16\], already_open u8 |
//! | `0x0104` | DocumentClosed | request u64, document u64 |
//! | `0x0105` | RequestFailed | request u64, failure u32, message str |
//! | `0x0106` | WindowSessionOpened | request u64, project u64, session u64, window_id \[16\], document_id \[16\], generation u64 |
//! | `0x0107` | WindowSessionClosed | request u64, session u64 |
//!
//! `document_id`는 작업 문서가 없으면(저장한 적 없는 프로젝트) NIL(0 16바이트)입니다.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use stellar_core_types::ids::{StableId, WindowSessionId};
use stellar_core_types::revision::DocumentRevision;
use stellar_document::open_set::{OpenDocumentHandle, OpenOutcome};
use stellar_project::identity::{parse_project_id, ProjectIdentity};
use stellar_project::probe::{safe_probe, ProbeError, SafeProbeError};
use stellar_project::root::{classify_project_open, OpenError, RootError};
use stellar_project::trust::TrustStatus;

use crate::command_queue::QueueCapacity;
use crate::document_thread::{
    DocumentContext, DocumentLoopStats, DocumentSender, DocumentThread, DocumentThreadError,
    SubmitError,
};
use crate::window_session::ProjectWindowSession;

/// event payload 스키마 버전.
pub const EVENT_SCHEMA_VERSION: u32 = 1;

/// event 문자열 최대 바이트 수(넘으면 문자 경계에서 자름).
pub const EVENT_TEXT_MAX: usize = 4096;

/// event 종류 코드.
pub mod event_kind {
    /// 프로젝트를 열었습니다.
    pub const PROJECT_OPENED: u32 = 0x0101;
    /// 프로젝트를 닫았습니다.
    pub const PROJECT_CLOSED: u32 = 0x0102;
    /// 문서를 열었습니다.
    pub const DOCUMENT_OPENED: u32 = 0x0103;
    /// 문서를 닫았습니다.
    pub const DOCUMENT_CLOSED: u32 = 0x0104;
    /// 요청이 실패했습니다.
    pub const REQUEST_FAILED: u32 = 0x0105;
    /// 창 session을 열었습니다.
    pub const WINDOW_SESSION_OPENED: u32 = 0x0106;
    /// 창 session을 닫았습니다.
    pub const WINDOW_SESSION_CLOSED: u32 = 0x0107;
}

// ---------------------------------------------------------------------------
// 식별자
// ---------------------------------------------------------------------------

/// 요청 id. runtime마다 1부터 증가합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestId(
    /// id 값(0 = 없음).
    pub u64,
);

/// 열린 프로젝트 session id. runtime마다 1부터 증가하며 다시 쓰지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProjectSessionId(
    /// id 값(0 = 없음).
    pub u64,
);

/// 열린 문서 session id. runtime마다 1부터 증가하며 다시 쓰지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocumentSessionId(
    /// id 값(0 = 없음).
    pub u64,
);

/// 열린 창 session handle(XII.2.2). runtime마다 1부터 증가하며 다시 쓰지 않습니다. 창 편집 session의
/// 영속 식별자([`WindowSessionId`])와 별개인 runtime 정수 handle입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowSessionHandle(
    /// id 값(0 = 없음).
    pub u64,
);

// ---------------------------------------------------------------------------
// 실패 코드·event
// ---------------------------------------------------------------------------

/// 요청 실패 코드. 값은 C ABI에 노출되므로 바꾸지 않고 끝에 덧붙입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequestFailure {
    /// 열려 있지 않은 프로젝트 session.
    UnknownProject,
    /// 열려 있지 않은 문서 session.
    UnknownDocument,
    /// 다른 프로젝트 session이 이미 연 문서.
    DocumentInOtherProject,
    /// `.git`이 없어 Git 저장소가 아님.
    NotARepository,
    /// Git 저장소지만 `stellar.yaml`이 없어 Stellar 프로젝트가 아님(자동 변환 없음).
    NotAStellarProject,
    /// `stellar.yaml`을 읽지 못함.
    ManifestUnreadable,
    /// `stellar.yaml` YAML 문법 오류(손상).
    ManifestCorrupt,
    /// 매니페스트 스키마 오류.
    ManifestInvalid,
    /// 매니페스트 `project.id`가 UUID 형식이 아님.
    InvalidProjectId,
    /// 신뢰되지 않은 프로젝트의 symlink 매니페스트를 따르지 않음.
    UntrustedSymlink,
    /// 프로젝트 경로를 절대·실경로로 해석하지 못함.
    PathUnresolved,
    /// 열려 있지 않은 창 session.
    UnknownWindowSession,
    /// 창 session의 generation·작업 문서를 로드하지 못함.
    WindowSessionLoadFailed,
}

impl RequestFailure {
    /// 전체 코드(값 순서).
    pub const ALL: [RequestFailure; 13] = [
        RequestFailure::UnknownProject,
        RequestFailure::UnknownDocument,
        RequestFailure::DocumentInOtherProject,
        RequestFailure::NotARepository,
        RequestFailure::NotAStellarProject,
        RequestFailure::ManifestUnreadable,
        RequestFailure::ManifestCorrupt,
        RequestFailure::ManifestInvalid,
        RequestFailure::InvalidProjectId,
        RequestFailure::UntrustedSymlink,
        RequestFailure::PathUnresolved,
        RequestFailure::UnknownWindowSession,
        RequestFailure::WindowSessionLoadFailed,
    ];

    /// C ABI 코드 값.
    pub const fn code(self) -> u32 {
        match self {
            RequestFailure::UnknownProject => 1,
            RequestFailure::UnknownDocument => 2,
            RequestFailure::DocumentInOtherProject => 3,
            RequestFailure::NotARepository => 10,
            RequestFailure::NotAStellarProject => 11,
            RequestFailure::ManifestUnreadable => 12,
            RequestFailure::ManifestCorrupt => 13,
            RequestFailure::ManifestInvalid => 14,
            RequestFailure::InvalidProjectId => 15,
            RequestFailure::UntrustedSymlink => 16,
            RequestFailure::PathUnresolved => 17,
            RequestFailure::UnknownWindowSession => 4,
            RequestFailure::WindowSessionLoadFailed => 18,
        }
    }

    /// 코드 값에서 복원합니다.
    pub fn from_code(code: u32) -> Option<RequestFailure> {
        RequestFailure::ALL
            .into_iter()
            .find(|failure| failure.code() == code)
    }
}

/// 요청 완료 event. 요청마다 정확히 하나가 outbox에 쌓입니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeEvent {
    /// 프로젝트를 열었습니다.
    ProjectOpened {
        /// 요청 id.
        request: RequestId,
        /// 새 프로젝트 session.
        project: ProjectSessionId,
        /// 매니페스트 project id.
        project_id: StableId,
        /// 매니페스트 프로젝트 이름.
        name: String,
        /// 연 경로(절대·lexical, symlink 보존).
        root: String,
    },
    /// 프로젝트를 닫았습니다.
    ProjectClosed {
        /// 요청 id.
        request: RequestId,
        /// 닫은 프로젝트 session.
        project: ProjectSessionId,
        /// 함께 닫은 문서 수.
        closed_documents: u32,
    },
    /// 문서를 열었습니다.
    DocumentOpened {
        /// 요청 id.
        request: RequestId,
        /// 소속 프로젝트 session.
        project: ProjectSessionId,
        /// 문서 session.
        document: DocumentSessionId,
        /// 문서 StableId.
        document_id: StableId,
        /// 같은 프로젝트 session에서 이미 열려 있었는지.
        already_open: bool,
    },
    /// 문서를 닫았습니다.
    DocumentClosed {
        /// 요청 id.
        request: RequestId,
        /// 닫은 문서 session.
        document: DocumentSessionId,
    },
    /// 창 session을 열었습니다.
    WindowSessionOpened {
        /// 요청 id.
        request: RequestId,
        /// 소속 프로젝트 session.
        project: ProjectSessionId,
        /// 창 session handle.
        session: WindowSessionHandle,
        /// 창 편집 session id.
        window_id: WindowSessionId,
        /// 작업 문서 StableId(없으면 NIL).
        document_id: StableId,
        /// 로드한 generation 번호(저장한 적 없으면 0).
        generation: u64,
    },
    /// 창 session을 닫았습니다.
    WindowSessionClosed {
        /// 요청 id.
        request: RequestId,
        /// 닫은 창 session handle.
        session: WindowSessionHandle,
    },
    /// 요청이 실패했습니다. 상태는 바뀌지 않았습니다.
    RequestFailed {
        /// 요청 id.
        request: RequestId,
        /// 실패 코드.
        failure: RequestFailure,
        /// 사람이 읽는 메시지(한국어).
        message: String,
    },
}

/// event payload 해석 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventDecodeError {
    /// 알 수 없는 event 종류.
    UnknownKind(u32),
    /// 지원하지 않는 schema version.
    UnsupportedVersion(u32),
    /// payload가 짧거나 남는 바이트가 있습니다.
    Length,
    /// 문자열이 UTF-8이 아닙니다.
    Utf8,
    /// bool 바이트가 0/1이 아닙니다.
    Bool(u8),
    /// 알 수 없는 실패 코드.
    UnknownFailure(u32),
}

impl fmt::Display for EventDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventDecodeError::UnknownKind(kind) => write!(f, "알 수 없는 event 종류: {kind:#x}"),
            EventDecodeError::UnsupportedVersion(version) => {
                write!(f, "지원하지 않는 event schema version: {version}")
            }
            EventDecodeError::Length => write!(f, "event payload 길이가 맞지 않습니다"),
            EventDecodeError::Utf8 => write!(f, "event 문자열이 UTF-8이 아닙니다"),
            EventDecodeError::Bool(value) => write!(f, "event bool 값이 0/1이 아닙니다: {value}"),
            EventDecodeError::UnknownFailure(code) => write!(f, "알 수 없는 실패 코드: {code}"),
        }
    }
}

impl std::error::Error for EventDecodeError {}

/// `EVENT_TEXT_MAX` 바이트 안에서 문자 경계로 자른 문자열.
fn clamp_text(text: &str) -> &str {
    if text.len() <= EVENT_TEXT_MAX {
        return text;
    }
    let mut end = EVENT_TEXT_MAX;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

struct Writer(Vec<u8>);

impl Writer {
    fn new() -> Writer {
        let mut writer = Writer(Vec::new());
        writer.u32(EVENT_SCHEMA_VERSION);
        writer
    }

    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }

    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn id(&mut self, id: StableId) {
        self.0.extend_from_slice(&id.to_bytes());
    }

    fn text(&mut self, text: &str) {
        let text = clamp_text(text);
        // EVENT_TEXT_MAX(4096) 이하이므로 u32로 표현됩니다.
        self.u32(text.len() as u32);
        self.0.extend_from_slice(text.as_bytes());
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], EventDecodeError> {
        if self.bytes.len() < len {
            return Err(EventDecodeError::Length);
        }
        let (head, rest) = self.bytes.split_at(len);
        self.bytes = rest;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], EventDecodeError> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, EventDecodeError> {
        Ok(self.array::<1>()?[0])
    }

    fn u32(&mut self) -> Result<u32, EventDecodeError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, EventDecodeError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn id(&mut self) -> Result<StableId, EventDecodeError> {
        Ok(StableId::from_bytes(self.array()?))
    }

    fn bool(&mut self) -> Result<bool, EventDecodeError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(EventDecodeError::Bool(other)),
        }
    }

    fn text(&mut self) -> Result<String, EventDecodeError> {
        let len = self.u32()? as usize;
        if len > EVENT_TEXT_MAX {
            return Err(EventDecodeError::Length);
        }
        let bytes = self.take(len)?;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| EventDecodeError::Utf8)
    }

    fn finish(self) -> Result<(), EventDecodeError> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err(EventDecodeError::Length)
        }
    }
}

impl RuntimeEvent {
    /// event 종류 코드([`event_kind`]).
    pub fn kind(&self) -> u32 {
        match self {
            RuntimeEvent::ProjectOpened { .. } => event_kind::PROJECT_OPENED,
            RuntimeEvent::ProjectClosed { .. } => event_kind::PROJECT_CLOSED,
            RuntimeEvent::DocumentOpened { .. } => event_kind::DOCUMENT_OPENED,
            RuntimeEvent::DocumentClosed { .. } => event_kind::DOCUMENT_CLOSED,
            RuntimeEvent::RequestFailed { .. } => event_kind::REQUEST_FAILED,
            RuntimeEvent::WindowSessionOpened { .. } => event_kind::WINDOW_SESSION_OPENED,
            RuntimeEvent::WindowSessionClosed { .. } => event_kind::WINDOW_SESSION_CLOSED,
        }
    }

    /// 이 event를 만든 요청 id.
    pub fn request(&self) -> RequestId {
        match self {
            RuntimeEvent::ProjectOpened { request, .. }
            | RuntimeEvent::ProjectClosed { request, .. }
            | RuntimeEvent::DocumentOpened { request, .. }
            | RuntimeEvent::DocumentClosed { request, .. }
            | RuntimeEvent::WindowSessionOpened { request, .. }
            | RuntimeEvent::WindowSessionClosed { request, .. }
            | RuntimeEvent::RequestFailed { request, .. } => *request,
        }
    }

    /// payload를 canonical little-endian 바이트로 씁니다(모듈 문서 표).
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            RuntimeEvent::ProjectOpened {
                request,
                project,
                project_id,
                name,
                root,
            } => {
                w.u64(request.0);
                w.u64(project.0);
                w.id(*project_id);
                w.text(name);
                w.text(root);
            }
            RuntimeEvent::ProjectClosed {
                request,
                project,
                closed_documents,
            } => {
                w.u64(request.0);
                w.u64(project.0);
                w.u32(*closed_documents);
            }
            RuntimeEvent::DocumentOpened {
                request,
                project,
                document,
                document_id,
                already_open,
            } => {
                w.u64(request.0);
                w.u64(project.0);
                w.u64(document.0);
                w.id(*document_id);
                w.u8(u8::from(*already_open));
            }
            RuntimeEvent::DocumentClosed { request, document } => {
                w.u64(request.0);
                w.u64(document.0);
            }
            RuntimeEvent::WindowSessionOpened {
                request,
                project,
                session,
                window_id,
                document_id,
                generation,
            } => {
                w.u64(request.0);
                w.u64(project.0);
                w.u64(session.0);
                w.id(window_id.stable_id());
                w.id(*document_id);
                w.u64(*generation);
            }
            RuntimeEvent::WindowSessionClosed { request, session } => {
                w.u64(request.0);
                w.u64(session.0);
            }
            RuntimeEvent::RequestFailed {
                request,
                failure,
                message,
            } => {
                w.u64(request.0);
                w.u32(failure.code());
                w.text(message);
            }
        }
        w.0
    }

    /// 종류 코드와 payload에서 event를 복원합니다. 길이·version·UTF-8·코드를 모두 검사합니다.
    pub fn decode(kind: u32, payload: &[u8]) -> Result<RuntimeEvent, EventDecodeError> {
        let mut r = Reader { bytes: payload };
        let version = r.u32()?;
        if version != EVENT_SCHEMA_VERSION {
            return Err(EventDecodeError::UnsupportedVersion(version));
        }
        let event = match kind {
            event_kind::PROJECT_OPENED => RuntimeEvent::ProjectOpened {
                request: RequestId(r.u64()?),
                project: ProjectSessionId(r.u64()?),
                project_id: r.id()?,
                name: r.text()?,
                root: r.text()?,
            },
            event_kind::PROJECT_CLOSED => RuntimeEvent::ProjectClosed {
                request: RequestId(r.u64()?),
                project: ProjectSessionId(r.u64()?),
                closed_documents: r.u32()?,
            },
            event_kind::DOCUMENT_OPENED => RuntimeEvent::DocumentOpened {
                request: RequestId(r.u64()?),
                project: ProjectSessionId(r.u64()?),
                document: DocumentSessionId(r.u64()?),
                document_id: r.id()?,
                already_open: r.bool()?,
            },
            event_kind::DOCUMENT_CLOSED => RuntimeEvent::DocumentClosed {
                request: RequestId(r.u64()?),
                document: DocumentSessionId(r.u64()?),
            },
            event_kind::WINDOW_SESSION_OPENED => RuntimeEvent::WindowSessionOpened {
                request: RequestId(r.u64()?),
                project: ProjectSessionId(r.u64()?),
                session: WindowSessionHandle(r.u64()?),
                window_id: WindowSessionId::from_stable_id(r.id()?),
                document_id: r.id()?,
                generation: r.u64()?,
            },
            event_kind::WINDOW_SESSION_CLOSED => RuntimeEvent::WindowSessionClosed {
                request: RequestId(r.u64()?),
                session: WindowSessionHandle(r.u64()?),
            },
            event_kind::REQUEST_FAILED => {
                let request = RequestId(r.u64()?);
                let code = r.u32()?;
                let failure = RequestFailure::from_code(code)
                    .ok_or(EventDecodeError::UnknownFailure(code))?;
                RuntimeEvent::RequestFailed {
                    request,
                    failure,
                    message: r.text()?,
                }
            }
            other => return Err(EventDecodeError::UnknownKind(other)),
        };
        r.finish()?;
        Ok(event)
    }
}

// ---------------------------------------------------------------------------
// 상태 snapshot
// ---------------------------------------------------------------------------

/// 열린 프로젝트 session 요약.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSummary {
    /// session id.
    pub project: ProjectSessionId,
    /// 매니페스트 project id.
    pub project_id: StableId,
    /// 매니페스트 프로젝트 이름.
    pub name: String,
    /// 연 경로(절대·lexical).
    pub root: PathBuf,
    /// 신뢰 상태(신뢰 저장소 연결 전에는 항상 `NotTrusted`).
    pub trust: TrustStatus,
}

/// 열린 문서 session 요약.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentSummary {
    /// session id.
    pub document: DocumentSessionId,
    /// 소속 프로젝트 session.
    pub project: ProjectSessionId,
    /// 문서 StableId.
    pub document_id: StableId,
    /// snapshot 게시 시점의 문서 revision.
    pub revision: DocumentRevision,
    /// snapshot 게시 시점의 node 수.
    pub node_count: u64,
}

/// 열린 창 session 요약.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowSessionSummary {
    /// 창 session handle.
    pub session: WindowSessionHandle,
    /// 소속 프로젝트 session.
    pub project: ProjectSessionId,
    /// 창 편집 session id.
    pub window_id: WindowSessionId,
    /// 작업 문서 StableId(없으면 `None`).
    pub document_id: Option<StableId>,
    /// snapshot 게시 시점의 작업 문서 revision(문서가 없으면 0).
    pub revision: DocumentRevision,
    /// loaded generation 번호(저장한 적 없으면 0).
    pub generation: u64,
    /// undo stack 깊이.
    pub undo_depth: u32,
    /// redo stack 깊이.
    pub redo_depth: u32,
    /// 선택된 node 수.
    pub selection_len: u32,
}

/// document thread가 게시한 불변 상태 snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeState {
    /// 게시 세대. 성공한 요청마다 1 증가합니다(0 = 아직 게시 없음).
    pub generation: u64,
    /// 열린 프로젝트(session id 순).
    pub projects: Vec<ProjectSummary>,
    /// 열린 문서(session id 순).
    pub documents: Vec<DocumentSummary>,
    /// 열린 창 session(handle 순).
    pub windows: Vec<WindowSessionSummary>,
}

impl RuntimeState {
    /// 문서 session 요약.
    pub fn document(&self, document: DocumentSessionId) -> Option<&DocumentSummary> {
        self.documents
            .binary_search_by_key(&document, |summary| summary.document)
            .ok()
            .map(|index| &self.documents[index])
    }

    /// 프로젝트 session 요약.
    pub fn project(&self, project: ProjectSessionId) -> Option<&ProjectSummary> {
        self.projects
            .binary_search_by_key(&project, |summary| summary.project)
            .ok()
            .map(|index| &self.projects[index])
    }

    /// 창 session 요약.
    pub fn window(&self, session: WindowSessionHandle) -> Option<&WindowSessionSummary> {
        self.windows
            .binary_search_by_key(&session, |summary| summary.session)
            .ok()
            .map(|index| &self.windows[index])
    }
}

// ---------------------------------------------------------------------------
// document thread 쪽 session 표
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct ProjectSession {
    identity: ProjectIdentity,
    name: String,
    documents: BTreeSet<DocumentSessionId>,
    windows: BTreeSet<WindowSessionHandle>,
}

/// 창 session 표 항목.
#[derive(Debug)]
pub(crate) struct WindowEntry {
    pub(crate) project: ProjectSessionId,
    pub(crate) session: ProjectWindowSession,
}

#[derive(Debug)]
struct DocumentSession {
    project: ProjectSessionId,
    handle: OpenDocumentHandle,
    document_id: StableId,
}

/// document thread만 소유하는 프로젝트·문서·창 session 표([`DocumentContext`] 안에 있음).
#[derive(Debug, Default)]
pub(crate) struct AppSessions {
    next_project: u64,
    next_document: u64,
    next_window: u64,
    generation: u64,
    projects: BTreeMap<ProjectSessionId, ProjectSession>,
    documents: BTreeMap<DocumentSessionId, DocumentSession>,
    by_document_id: BTreeMap<StableId, DocumentSessionId>,
    pub(crate) windows: BTreeMap<WindowSessionHandle, WindowEntry>,
    window_ids: WindowIdSource,
}

/// 창 편집 session id 공급기: UUIDv7(현재 Unix ms + 시각·프로세스·순번에서 섞은 난수 bit).
///
/// 영속 식별자라 프로세스를 다시 시작해도 겹치지 않아야 합니다(복구 snapshot 키, V.9.1). 시각의 nanosecond와
/// 프로세스 id, runtime 안 순번을 SplitMix64로 섞어 random bit를 채우고, rand_b 하위 32 bit에는 순번을 그대로
/// 넣어 같은 runtime 안에서는 항상 다르게 합니다.
#[derive(Debug, Default)]
struct WindowIdSource {
    issued: u64,
}

impl WindowIdSource {
    fn next(&mut self) -> WindowSessionId {
        self.issued = self.issued.wrapping_add(1);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let unix_ms = u64::try_from(now.as_millis()).unwrap_or(u64::MAX);
        let seed = u64::from(now.subsec_nanos())
            ^ (u64::from(std::process::id()) << 32)
            ^ self.issued.rotate_left(48);
        let a = splitmix64(seed);
        let b = splitmix64(a ^ self.issued);
        let rand_b = (b & !0xFFFF_FFFF) | (self.issued & 0xFFFF_FFFF);
        WindowSessionId::from_stable_id(StableId::new_v7(unix_ms, (a & 0x0FFF) as u16, rand_b))
    }
}

fn splitmix64(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl AppSessions {
    fn allocate_window(&mut self) -> WindowSessionHandle {
        self.next_window = self.next_window.saturating_add(1);
        WindowSessionHandle(self.next_window)
    }

    fn allocate_project(&mut self) -> ProjectSessionId {
        self.next_project = self.next_project.saturating_add(1);
        ProjectSessionId(self.next_project)
    }

    fn allocate_document(&mut self) -> DocumentSessionId {
        self.next_document = self.next_document.saturating_add(1);
        DocumentSessionId(self.next_document)
    }
}

/// 실패 event 재료.
struct Failure {
    failure: RequestFailure,
    message: String,
}

impl Failure {
    fn new(failure: RequestFailure, message: impl Into<String>) -> Failure {
        Failure {
            failure,
            message: message.into(),
        }
    }
}

fn root_failure(error: RootError) -> RequestFailure {
    match error {
        RootError::MissingManifest => RequestFailure::NotAStellarProject,
        RootError::MissingGit => RequestFailure::NotARepository,
    }
}

fn probe_failure(error: &ProbeError) -> RequestFailure {
    match error {
        ProbeError::Root(root) => root_failure(*root),
        ProbeError::ManifestRead { .. } => RequestFailure::ManifestUnreadable,
        ProbeError::ManifestSyntax(_) => RequestFailure::ManifestCorrupt,
        ProbeError::ManifestSchema(_) => RequestFailure::ManifestInvalid,
    }
}

/// document thread에서 프로젝트를 엽니다(모듈 문서 순서).
fn open_project_on_thread(
    ctx: &mut DocumentContext,
    request: RequestId,
    root: &Path,
) -> Result<RuntimeEvent, Failure> {
    classify_project_open(root).map_err(|error| {
        let failure = match error {
            OpenError::NotARepository => RequestFailure::NotARepository,
            OpenError::NotAStellarProject => RequestFailure::NotAStellarProject,
        };
        Failure::new(failure, error.to_string())
    })?;
    let trust = TrustStatus::NotTrusted;
    let probe = safe_probe(root, trust).map_err(|error| match &error {
        SafeProbeError::Probe(probe) => Failure::new(probe_failure(probe), error.to_string()),
        SafeProbeError::UntrustedSymlinkRefused { .. } => {
            Failure::new(RequestFailure::UntrustedSymlink, error.to_string())
        }
    })?;
    let project_id = parse_project_id(&probe.manifest.project.id).ok_or_else(|| {
        Failure::new(
            RequestFailure::InvalidProjectId,
            "매니페스트 project.id가 UUID 형식이 아닙니다",
        )
    })?;
    let identity = ProjectIdentity::new(project_id, root).map_err(|error| {
        Failure::new(
            RequestFailure::PathUnresolved,
            format!("프로젝트 경로를 해석하지 못했습니다: {:?}", error.kind()),
        )
    })?;
    let name = probe.manifest.project.name;
    let sessions = ctx.app_sessions_mut();
    let project = sessions.allocate_project();
    let root_text = identity.root_path().to_string_lossy().into_owned();
    sessions.projects.insert(
        project,
        ProjectSession {
            identity,
            name: name.clone(),
            documents: BTreeSet::new(),
            windows: BTreeSet::new(),
        },
    );
    Ok(RuntimeEvent::ProjectOpened {
        request,
        project,
        project_id,
        name,
        root: root_text,
    })
}

fn close_project_on_thread(
    ctx: &mut DocumentContext,
    request: RequestId,
    project: ProjectSessionId,
) -> Result<RuntimeEvent, Failure> {
    let session = ctx
        .app_sessions_mut()
        .projects
        .remove(&project)
        .ok_or_else(|| unknown_project(project))?;
    let mut closed_documents = 0u32;
    for document in session.documents {
        if let Some(handle) = forget_document(ctx, document) {
            ctx.close_document(handle);
            closed_documents = closed_documents.saturating_add(1);
        }
    }
    // 프로젝트를 닫으면 그 아래 창 session(문서 state·history·선택)도 폐기합니다.
    let sessions = ctx.app_sessions_mut();
    for window in session.windows {
        sessions.windows.remove(&window);
    }
    Ok(RuntimeEvent::ProjectClosed {
        request,
        project,
        closed_documents,
    })
}

fn unknown_project(project: ProjectSessionId) -> Failure {
    Failure::new(
        RequestFailure::UnknownProject,
        format!("열려 있지 않은 프로젝트 session입니다: {}", project.0),
    )
}

/// 문서 session을 표에서 지우고 열린 문서 handle을 돌려줍니다.
fn forget_document(
    ctx: &mut DocumentContext,
    document: DocumentSessionId,
) -> Option<OpenDocumentHandle> {
    let sessions = ctx.app_sessions_mut();
    let session = sessions.documents.remove(&document)?;
    sessions.by_document_id.remove(&session.document_id);
    if let Some(project) = sessions.projects.get_mut(&session.project) {
        project.documents.remove(&document);
    }
    Some(session.handle)
}

fn open_document_on_thread(
    ctx: &mut DocumentContext,
    request: RequestId,
    project: ProjectSessionId,
    document_id: StableId,
) -> Result<RuntimeEvent, Failure> {
    let sessions = ctx.app_sessions_mut();
    if !sessions.projects.contains_key(&project) {
        return Err(unknown_project(project));
    }
    if let Some(existing) = sessions.by_document_id.get(&document_id).copied() {
        let owner = sessions
            .documents
            .get(&existing)
            .map(|session| session.project);
        if owner == Some(project) {
            return Ok(RuntimeEvent::DocumentOpened {
                request,
                project,
                document: existing,
                document_id,
                already_open: true,
            });
        }
        return Err(Failure::new(
            RequestFailure::DocumentInOtherProject,
            "다른 프로젝트 session이 이미 연 문서입니다",
        ));
    }
    // 이 표 밖에서 열린 문서(Existing)도 이 session에 묶습니다. AppRuntime은 job 접수 handle을
    // 노출하지 않으므로 정상 경로에서는 항상 NewlyOpened입니다.
    let handle = match ctx.open_document(document_id) {
        OpenOutcome::NewlyOpened(handle) | OpenOutcome::Existing(handle) => handle,
    };
    let sessions = ctx.app_sessions_mut();
    let document = sessions.allocate_document();
    sessions.documents.insert(
        document,
        DocumentSession {
            project,
            handle,
            document_id,
        },
    );
    sessions.by_document_id.insert(document_id, document);
    if let Some(owner) = sessions.projects.get_mut(&project) {
        owner.documents.insert(document);
    }
    Ok(RuntimeEvent::DocumentOpened {
        request,
        project,
        document,
        document_id,
        already_open: false,
    })
}

fn close_document_on_thread(
    ctx: &mut DocumentContext,
    request: RequestId,
    document: DocumentSessionId,
) -> Result<RuntimeEvent, Failure> {
    let handle = forget_document(ctx, document).ok_or_else(|| {
        Failure::new(
            RequestFailure::UnknownDocument,
            format!("열려 있지 않은 문서 session입니다: {}", document.0),
        )
    })?;
    ctx.close_document(handle);
    Ok(RuntimeEvent::DocumentClosed { request, document })
}

fn unknown_window(session: WindowSessionHandle) -> Failure {
    Failure::new(
        RequestFailure::UnknownWindowSession,
        format!("열려 있지 않은 창 session입니다: {}", session.0),
    )
}

/// document thread에서 창 session을 엽니다: current generation과 작업 문서를 로드합니다(XII.2.2).
fn open_window_session_on_thread(
    ctx: &mut DocumentContext,
    request: RequestId,
    project: ProjectSessionId,
) -> Result<RuntimeEvent, Failure> {
    let sessions = ctx.app_sessions_mut();
    let identity = sessions
        .projects
        .get(&project)
        .map(|owner| owner.identity.clone())
        .ok_or_else(|| unknown_project(project))?;
    let window_id = sessions.window_ids.next();
    let session = ProjectWindowSession::load(identity, window_id).map_err(|error| {
        Failure::new(RequestFailure::WindowSessionLoadFailed, error.to_string())
    })?;
    let document_id = session.document_id().unwrap_or(StableId::NIL);
    let generation = session.generation_number();
    let handle = sessions.allocate_window();
    sessions
        .windows
        .insert(handle, WindowEntry { project, session });
    if let Some(owner) = sessions.projects.get_mut(&project) {
        owner.windows.insert(handle);
    }
    Ok(RuntimeEvent::WindowSessionOpened {
        request,
        project,
        session: handle,
        window_id,
        document_id,
        generation,
    })
}

fn close_window_session_on_thread(
    ctx: &mut DocumentContext,
    request: RequestId,
    session: WindowSessionHandle,
) -> Result<RuntimeEvent, Failure> {
    let sessions = ctx.app_sessions_mut();
    let entry = sessions
        .windows
        .remove(&session)
        .ok_or_else(|| unknown_window(session))?;
    if let Some(owner) = sessions.projects.get_mut(&entry.project) {
        owner.windows.remove(&session);
    }
    Ok(RuntimeEvent::WindowSessionClosed { request, session })
}

/// 창 session 요약(상태 snapshot용).
fn window_summary(handle: WindowSessionHandle, entry: &WindowEntry) -> WindowSessionSummary {
    let session = &entry.session;
    let clamp = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    WindowSessionSummary {
        session: handle,
        project: entry.project,
        window_id: session.window(),
        document_id: session.document_id(),
        revision: session.revision(),
        generation: session.generation_number(),
        undo_depth: clamp(session.history().history().len()),
        redo_depth: clamp(session.history().history().redo_entries().len()),
        selection_len: clamp(session.view().selection().len()),
    }
}

/// 현재 session 표로 불변 상태 snapshot을 만듭니다(세대 1 증가).
fn build_state(ctx: &mut DocumentContext) -> RuntimeState {
    let generation = {
        let sessions = ctx.app_sessions_mut();
        sessions.generation = sessions.generation.saturating_add(1);
        sessions.generation
    };
    let sessions = ctx.app_sessions();
    let projects = sessions
        .projects
        .iter()
        .map(|(project, session)| ProjectSummary {
            project: *project,
            project_id: session.identity.project_id(),
            name: session.name.clone(),
            root: session.identity.root_path().to_path_buf(),
            trust: TrustStatus::NotTrusted,
        })
        .collect();
    let documents = sessions
        .documents
        .iter()
        .map(|(document, session)| DocumentSummary {
            document: *document,
            project: session.project,
            document_id: session.document_id,
            revision: ctx.revision(session.handle).unwrap_or(DocumentRevision(0)),
            node_count: ctx
                .snapshot(session.handle)
                .map_or(0, |view| view.len() as u64),
        })
        .collect();
    let windows = sessions
        .windows
        .iter()
        .map(|(handle, entry)| window_summary(*handle, entry))
        .collect();
    RuntimeState {
        generation,
        projects,
        documents,
        windows,
    }
}

// ---------------------------------------------------------------------------
// AppRuntime
// ---------------------------------------------------------------------------

/// 요청 접수 거부.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestError {
    /// 완료 event를 꺼내지 않은 요청이 상한에 닿았거나 command lane이 찼습니다.
    QueueFull,
    /// runtime이 종료 중이거나 종료됐습니다.
    Stopped,
    /// 프로젝트 경로가 규칙(절대 경로, NUL 없음)에 맞지 않습니다.
    InvalidPath {
        /// 이유.
        reason: &'static str,
    },
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RequestError::QueueFull => write!(f, "처리 대기 요청이 가득 찼습니다"),
            RequestError::Stopped => write!(f, "runtime이 종료됐습니다"),
            RequestError::InvalidPath { reason } => write!(f, "프로젝트 경로 오류: {reason}"),
        }
    }
}

impl std::error::Error for RequestError {}

/// 다음 event를 꺼내려 한 결과([`AppRuntime::poll_event`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventPoll {
    /// 꺼낼 event가 없습니다.
    Empty,
    /// 다음 event가 있지만 꺼내지 않았습니다(용량 미지정 또는 부족).
    Pending {
        /// event 종류.
        kind: u32,
        /// payload 바이트 수.
        len: usize,
    },
    /// event를 꺼냈습니다.
    Taken {
        /// event 종류.
        kind: u32,
        /// payload.
        payload: Vec<u8>,
    },
}

/// UI thread와 document thread가 공유하는 outbox·snapshot.
#[derive(Debug)]
struct AppShared {
    outbox: Mutex<VecDeque<RuntimeEvent>>,
    /// 완료 event를 아직 꺼내지 않은 요청 수(실행 중 포함).
    outstanding: AtomicUsize,
    state: Mutex<Arc<RuntimeState>>,
}

impl AppShared {
    fn outbox(&self) -> MutexGuard<'_, VecDeque<RuntimeEvent>> {
        // 임계 구역에서 panic하는 코드가 없으므로 poison 상태도 일관됩니다.
        self.outbox.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn publish_state(&self, state: RuntimeState) {
        let state = Arc::new(state);
        *self.state.lock().unwrap_or_else(PoisonError::into_inner) = state;
    }

    fn current_state(&self) -> Arc<RuntimeState> {
        Arc::clone(&self.state.lock().unwrap_or_else(PoisonError::into_inner))
    }

    fn release_one(&self) {
        // 꺼낸 event 수는 접수한 요청 수를 넘지 않습니다.
        let _ = self
            .outstanding
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1));
    }
}

/// 셸 경계용 앱 runtime. document thread를 소유합니다.
///
/// drop하면(셸 FFI destroy 포함) 접수를 닫고 이미 접수한 요청을 실행한 뒤 document thread를 join하되,
/// 최대 [`AppRuntime::SHUTDOWN_TIMEOUT`]까지만 기다립니다. 넘으면(예: 응답 없는 파일 시스템 위
/// 매니페스트 열기) 아직 시작하지 않은 요청을 버리고 thread를 분리합니다
/// ([`DocumentThread::shutdown_timeout`]). 완료 event는 어느 경우든 버립니다.
#[derive(Debug)]
pub struct AppRuntime {
    sender: DocumentSender,
    shared: Arc<AppShared>,
    next_request: AtomicU64,
    event_capacity: usize,
    /// drop이 document thread 종료를 기다리는 최대 시간.
    drop_timeout: Duration,
    document: DocumentThread,
}

impl Drop for AppRuntime {
    fn drop(&mut self) {
        // shutdown·shutdown_timeout으로 이미 종료했으면 아무것도 하지 않습니다.
        let _ = self.document.stop_within(Some(self.drop_timeout));
    }
}

impl AppRuntime {
    /// 완료 event를 꺼내지 않은 요청 수 기본 상한.
    pub const DEFAULT_EVENT_CAPACITY: usize = 1024;

    /// drop(셸 FFI destroy)이 document thread 종료를 기다리는 최대 시간.
    pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

    /// 기본 설정으로 시작합니다(document thread 생성).
    pub fn start() -> Result<AppRuntime, DocumentThreadError> {
        AppRuntime::start_with(QueueCapacity::default(), AppRuntime::DEFAULT_EVENT_CAPACITY)
    }

    /// lane 용량과 event 상한을 지정해 시작합니다. `event_capacity`가 0이면 1로 올립니다.
    pub fn start_with(
        capacity: QueueCapacity,
        event_capacity: usize,
    ) -> Result<AppRuntime, DocumentThreadError> {
        let document = DocumentThread::spawn_with(capacity)?;
        Ok(AppRuntime {
            sender: document.sender(),
            shared: Arc::new(AppShared {
                outbox: Mutex::new(VecDeque::new()),
                outstanding: AtomicUsize::new(0),
                state: Mutex::new(Arc::new(RuntimeState::default())),
            }),
            next_request: AtomicU64::new(1),
            event_capacity: event_capacity.max(1),
            drop_timeout: AppRuntime::SHUTDOWN_TIMEOUT,
            document,
        })
    }

    /// 요청 하나를 접수합니다. job은 document thread에서 상태를 바꾸고, 바뀌었으면 snapshot을
    /// 게시한 뒤 완료 event를 outbox에 넣습니다.
    fn submit<F>(&self, run: F) -> Result<RequestId, RequestError>
    where
        F: FnOnce(&mut DocumentContext, RequestId) -> Result<RuntimeEvent, Failure>
            + Send
            + 'static,
    {
        // 상한 예약: 실패하면 접수하지 않습니다.
        let reserved =
            self.shared
                .outstanding
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                    (n < self.event_capacity).then_some(n + 1)
                });
        if reserved.is_err() {
            return Err(RequestError::QueueFull);
        }
        let request = RequestId(self.next_request.fetch_add(1, Ordering::Relaxed));
        let shared = Arc::clone(&self.shared);
        let submitted = self.sender.submit(move |ctx: &mut DocumentContext| {
            let event = match run(ctx, request) {
                Ok(event) => {
                    shared.publish_state(build_state(ctx));
                    event
                }
                Err(failure) => RuntimeEvent::RequestFailed {
                    request,
                    failure: failure.failure,
                    message: failure.message,
                },
            };
            shared.outbox().push_back(event);
        });
        match submitted {
            Ok(_ticket) => Ok(request),
            Err(error) => {
                self.shared.release_one();
                Err(match error {
                    SubmitError::QueueFull { .. } => RequestError::QueueFull,
                    SubmitError::Stopped => RequestError::Stopped,
                })
            }
        }
    }

    /// 프로젝트 폴더를 엽니다(모듈 문서). 경로는 절대 경로여야 하고 NUL을 포함할 수 없습니다.
    pub fn open_project(&self, root: &Path) -> Result<RequestId, RequestError> {
        if root.as_os_str().is_empty() {
            return Err(RequestError::InvalidPath {
                reason: "빈 경로입니다",
            });
        }
        if root.as_os_str().as_encoded_bytes().contains(&0) {
            return Err(RequestError::InvalidPath {
                reason: "NUL 바이트를 포함합니다",
            });
        }
        if !root.is_absolute() {
            return Err(RequestError::InvalidPath {
                reason: "절대 경로가 아닙니다",
            });
        }
        let root = root.to_path_buf();
        self.submit(move |ctx, request| open_project_on_thread(ctx, request, &root))
    }

    /// 프로젝트 session을 닫습니다. 그 아래 열린 문서도 닫습니다.
    pub fn close_project(&self, project: ProjectSessionId) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| close_project_on_thread(ctx, request, project))
    }

    /// 프로젝트 session 아래에서 문서를 엽니다.
    pub fn open_document(
        &self,
        project: ProjectSessionId,
        document_id: StableId,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| open_document_on_thread(ctx, request, project, document_id))
    }

    /// 문서 session을 닫습니다.
    pub fn close_document(&self, document: DocumentSessionId) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| close_document_on_thread(ctx, request, document))
    }

    /// 프로젝트 session 아래에서 창 session을 엽니다(XII.2.2). 디스크의 current generation과 작업 문서를
    /// document thread에서 로드하고, 성공하면 [`RuntimeEvent::WindowSessionOpened`]가 옵니다.
    pub fn open_window_session(
        &self,
        project: ProjectSessionId,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| open_window_session_on_thread(ctx, request, project))
    }

    /// 창 session을 닫습니다. 그 창의 문서 state·history·선택을 폐기합니다(저장하지 않음).
    pub fn close_window_session(
        &self,
        session: WindowSessionHandle,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| close_window_session_on_thread(ctx, request, session))
    }

    /// 최신 상태 snapshot. document thread를 기다리지 않습니다.
    pub fn state(&self) -> Arc<RuntimeState> {
        self.shared.current_state()
    }

    /// 다음 완료 event를 꺼냅니다(FIFO).
    pub fn pop_event(&self) -> Option<RuntimeEvent> {
        let event = self.shared.outbox().pop_front();
        if event.is_some() {
            self.shared.release_one();
        }
        event
    }

    /// 다음 완료 event를 payload 바이트로 꺼냅니다.
    ///
    /// `capacity`가 `None`이거나 payload보다 작으면 꺼내지 않고 [`EventPoll::Pending`]으로 종류와
    /// 길이만 알립니다. 판정과 꺼내기는 한 임계 구역에서 합니다.
    pub fn poll_event(&self, capacity: Option<usize>) -> EventPoll {
        let mut outbox = self.shared.outbox();
        let Some(front) = outbox.front() else {
            return EventPoll::Empty;
        };
        let kind = front.kind();
        let payload = front.encode();
        if capacity.is_none_or(|capacity| capacity < payload.len()) {
            return EventPoll::Pending {
                kind,
                len: payload.len(),
            };
        }
        outbox.pop_front();
        drop(outbox);
        self.shared.release_one();
        EventPoll::Taken { kind, payload }
    }

    /// 완료 event를 아직 꺼내지 않은 요청 수(실행 중 포함).
    pub fn outstanding(&self) -> usize {
        self.shared.outstanding.load(Ordering::Acquire)
    }

    /// document thread를 종료합니다. 이미 접수한 요청은 실행하고, 끝날 때까지 기다립니다.
    pub fn shutdown(mut self) -> Result<DocumentLoopStats, DocumentThreadError> {
        self.document.stop_within(None)
    }

    /// [`AppRuntime::shutdown`]과 같이 종료하되 최대 `timeout`까지만 기다립니다. 넘으면 아직 시작하지
    /// 않은 요청을 버리고 thread를 분리한 뒤 [`DocumentThreadError::ShutdownTimedOut`]입니다.
    pub fn shutdown_timeout(
        mut self,
        timeout: Duration,
    ) -> Result<DocumentLoopStats, DocumentThreadError> {
        self.document.stop_within(Some(timeout))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(seed: u64) -> StableId {
        StableId::new_v7(seed, 0x0abc, seed)
    }

    fn sample_events() -> Vec<RuntimeEvent> {
        vec![
            RuntimeEvent::ProjectOpened {
                request: RequestId(1),
                project: ProjectSessionId(2),
                project_id: id(3),
                name: "디자인 시스템".to_string(),
                root: "/tmp/프로젝트".to_string(),
            },
            RuntimeEvent::ProjectClosed {
                request: RequestId(4),
                project: ProjectSessionId(2),
                closed_documents: 3,
            },
            RuntimeEvent::DocumentOpened {
                request: RequestId(5),
                project: ProjectSessionId(2),
                document: DocumentSessionId(6),
                document_id: id(7),
                already_open: true,
            },
            RuntimeEvent::DocumentClosed {
                request: RequestId(8),
                document: DocumentSessionId(6),
            },
            RuntimeEvent::RequestFailed {
                request: RequestId(9),
                failure: RequestFailure::NotAStellarProject,
                message: "Stellar 프로젝트가 아닙니다".to_string(),
            },
            RuntimeEvent::WindowSessionOpened {
                request: RequestId(10),
                project: ProjectSessionId(2),
                session: WindowSessionHandle(11),
                window_id: WindowSessionId::from_stable_id(id(12)),
                document_id: id(13),
                generation: 14,
            },
            RuntimeEvent::WindowSessionClosed {
                request: RequestId(15),
                session: WindowSessionHandle(11),
            },
        ]
    }

    #[test]
    fn window_session_payload_layout_is_fixed() {
        let event = RuntimeEvent::WindowSessionOpened {
            request: RequestId(1),
            project: ProjectSessionId(2),
            session: WindowSessionHandle(3),
            window_id: WindowSessionId::from_stable_id(StableId::from_bytes([0xAA; 16])),
            document_id: StableId::NIL,
            generation: 7,
        };
        let mut expected = vec![1, 0, 0, 0];
        expected.extend_from_slice(&1u64.to_le_bytes());
        expected.extend_from_slice(&2u64.to_le_bytes());
        expected.extend_from_slice(&3u64.to_le_bytes());
        expected.extend_from_slice(&[0xAA; 16]);
        expected.extend_from_slice(&[0; 16]);
        expected.extend_from_slice(&7u64.to_le_bytes());
        assert_eq!(event.encode(), expected);
        assert_eq!(event.kind(), 0x0106);
    }

    #[test]
    fn window_ids_are_distinct_v7_ids() {
        let mut source = WindowIdSource::default();
        let ids: BTreeSet<WindowSessionId> = (0..1_000).map(|_| source.next()).collect();
        assert_eq!(ids.len(), 1_000);
        for id in &ids {
            assert_eq!(id.stable_id().to_bytes()[6] >> 4, 7, "UUIDv7");
        }
    }

    #[test]
    fn events_round_trip_through_canonical_bytes() {
        for event in sample_events() {
            let bytes = event.encode();
            assert_eq!(&bytes[..4], &EVENT_SCHEMA_VERSION.to_le_bytes());
            assert_eq!(
                RuntimeEvent::decode(event.kind(), &bytes),
                Ok(event.clone())
            );
            // 결정론: 같은 event는 같은 바이트입니다.
            assert_eq!(event.encode(), bytes);
        }
    }

    #[test]
    fn document_closed_payload_layout_is_fixed() {
        let event = RuntimeEvent::DocumentClosed {
            request: RequestId(0x0102_0304_0506_0708),
            document: DocumentSessionId(9),
        };
        assert_eq!(
            event.encode(),
            [
                1, 0, 0, 0, // schema version
                8, 7, 6, 5, 4, 3, 2, 1, // request
                9, 0, 0, 0, 0, 0, 0, 0, // document
            ]
        );
    }

    #[test]
    fn decode_rejects_malformed_payloads() {
        let opened = sample_events().remove(2);
        let bytes = opened.encode();
        let kind = opened.kind();
        assert_eq!(
            RuntimeEvent::decode(kind, &bytes[..bytes.len() - 1]),
            Err(EventDecodeError::Length)
        );
        let mut extra = bytes.clone();
        extra.push(0);
        assert_eq!(
            RuntimeEvent::decode(kind, &extra),
            Err(EventDecodeError::Length)
        );
        let mut bad_bool = bytes.clone();
        *bad_bool.last_mut().unwrap() = 2;
        assert_eq!(
            RuntimeEvent::decode(kind, &bad_bool),
            Err(EventDecodeError::Bool(2))
        );
        let mut bad_version = bytes.clone();
        bad_version[0] = 2;
        assert_eq!(
            RuntimeEvent::decode(kind, &bad_version),
            Err(EventDecodeError::UnsupportedVersion(2))
        );
        assert_eq!(
            RuntimeEvent::decode(0x0999, &bytes),
            Err(EventDecodeError::UnknownKind(0x0999))
        );
        let failed = sample_events().remove(4);
        let mut bad_code = failed.encode();
        bad_code[12..16].copy_from_slice(&999u32.to_le_bytes());
        assert_eq!(
            RuntimeEvent::decode(failed.kind(), &bad_code),
            Err(EventDecodeError::UnknownFailure(999))
        );
        let mut bad_utf8 = failed.encode();
        let last = bad_utf8.len() - 1;
        bad_utf8[last] = 0xFF;
        assert_eq!(
            RuntimeEvent::decode(failed.kind(), &bad_utf8),
            Err(EventDecodeError::Utf8)
        );
        // 길이 필드가 상한을 넘으면 읽기 전에 거부합니다.
        let mut huge = failed.encode();
        huge[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            RuntimeEvent::decode(failed.kind(), &huge),
            Err(EventDecodeError::Length)
        );
    }

    #[test]
    fn long_text_is_clamped_at_a_char_boundary() {
        let name = "가".repeat(EVENT_TEXT_MAX); // 3바이트 문자
        let event = RuntimeEvent::ProjectOpened {
            request: RequestId(1),
            project: ProjectSessionId(1),
            project_id: id(1),
            name,
            root: String::new(),
        };
        let RuntimeEvent::ProjectOpened { name, .. } =
            RuntimeEvent::decode(event.kind(), &event.encode()).unwrap()
        else {
            panic!("종류가 바뀜");
        };
        assert!(name.len() <= EVENT_TEXT_MAX);
        assert_eq!(name.len() % 3, 0);
        assert!(name.chars().all(|c| c == '가'));
    }

    #[test]
    fn failure_codes_are_stable_and_unique() {
        let codes: Vec<u32> = RequestFailure::ALL.iter().map(|f| f.code()).collect();
        assert_eq!(codes, [1, 2, 3, 10, 11, 12, 13, 14, 15, 16, 17, 4, 18]);
        let unique: BTreeSet<u32> = codes.iter().copied().collect();
        assert_eq!(unique.len(), codes.len());
        for failure in RequestFailure::ALL {
            assert_eq!(RequestFailure::from_code(failure.code()), Some(failure));
        }
        assert_eq!(RequestFailure::from_code(0), None);
    }

    #[test]
    fn errors_display_in_korean() {
        assert!(RequestError::QueueFull.to_string().contains("가득"));
        assert!(RequestError::Stopped.to_string().contains("종료"));
        assert!(EventDecodeError::Length.to_string().contains("길이"));
    }

    /// document thread를 막는 job(응답 없는 파일 시스템 호출 대역)을 넣고, 실행이 시작될 때까지
    /// 기다립니다. 돌려준 sender에 보내면(또는 버리면) job이 끝납니다.
    fn stall(app: &AppRuntime) -> std::sync::mpsc::Sender<()> {
        use std::sync::mpsc;
        let (release, wait) = mpsc::channel::<()>();
        let (entered, started) = mpsc::channel::<()>();
        app.sender
            .submit(move |_ctx| {
                let _ = entered.send(());
                let _ = wait.recv();
            })
            .unwrap();
        started
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("막는 job이 시작되지 않음");
        release
    }

    #[test]
    fn dropping_the_runtime_does_not_wait_forever_for_a_stalled_document_thread() {
        // 보안 리뷰 FR-1: document thread가 job 안에서 멈춰도 drop(FFI destroy)이 무기한 join하지
        // 않습니다. 멈춘 job 뒤에 접수된 요청은 실행하지 않고 버립니다.
        use std::sync::mpsc;
        use std::time::Instant;

        let mut app = AppRuntime::start().unwrap();
        assert_eq!(app.drop_timeout, AppRuntime::SHUTDOWN_TIMEOUT);
        // 테스트 시간을 줄이려고 drop 대기 상한만 줄입니다(동작은 같음).
        app.drop_timeout = Duration::from_millis(200);
        let release = stall(&app);
        let (ran, queued_ran) = mpsc::channel::<()>();
        app.sender
            .submit(move |_ctx| {
                let _ = ran.send(());
            })
            .unwrap();
        app.open_project(Path::new("/stellar-runtime-test-no-such-project"))
            .unwrap();

        let (dropped, drop_done) = mpsc::channel();
        let dropper = std::thread::spawn(move || {
            let started = Instant::now();
            drop(app);
            let _ = dropped.send(started.elapsed());
        });
        let waited = drop_done.recv_timeout(Duration::from_secs(10));
        // 멈춘 job을 풀어 document thread가 끝나게 합니다.
        drop(release);
        dropper.join().unwrap();
        let waited = waited.expect("drop이 멈춘 document thread를 무기한 기다림");
        assert!(
            waited >= Duration::from_millis(200) && waited < Duration::from_secs(5),
            "drop 대기 {waited:?}"
        );
        // 시간 초과 뒤 남은 job은 실행하지 않고 버립니다(closure가 버려져 channel이 끊김).
        assert_eq!(
            queued_ran.recv_timeout(Duration::from_secs(5)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        );
    }

    #[test]
    fn shutdown_timeout_reports_a_stalled_document_thread() {
        let app = AppRuntime::start().unwrap();
        let release = stall(&app);
        let request = app
            .open_project(Path::new("/stellar-runtime-test-no-such-project"))
            .unwrap();
        assert_eq!(request, RequestId(1));
        let timeout = Duration::from_millis(100);
        assert_eq!(
            app.shutdown_timeout(timeout),
            Err(DocumentThreadError::ShutdownTimedOut { timeout })
        );
        drop(release);
        // 시간 안에 끝나는 종료는 shutdown과 같습니다.
        let app = AppRuntime::start().unwrap();
        app.open_project(Path::new("/stellar-runtime-test-no-such-project"))
            .unwrap();
        let stats = app.shutdown_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(stats.jobs_run, 1);
    }
}
