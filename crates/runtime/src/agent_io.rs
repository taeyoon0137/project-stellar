//! 앱 Agent Host·IPC I/O 스레드의 코어 (XXIX.3.4, ADR-0112 결정 5).
//!
//! 에이전트(MCP 브리지 연결·내장 CLI 세션)가 보낸 요청은 이 module을 거쳐 document thread에 들어갑니다.
//!
//! - **문서를 쓰지 않음**: [`AgentIo`]와 [`AgentIoThread`]는 document thread job 접수기만 갖습니다. 문서·history·
//!   출처 색인은 document thread job 안에서만 바뀝니다.
//! - **검증은 이 스레드에서**: proposal은 접수 전에 닫힌 enum command로 바꿉니다
//!   (`stellar_agent_edit::attached::edit::decode_proposal`, `stellar apply`와 같은 변환). 바꿀 수 없는 요청은
//!   document thread에 넣지 않고 거부합니다.
//! - **priority**: 검증한 요청은 XII.1.2 queue의 [`CommandPriority::Command`](crate::command_queue::CommandPriority)
//!   lane에 넣습니다. 사용자 입력(`Input`)보다 늦게 처리됩니다. `Background` lane은 넘칠 때 오래된 항목을 밀어내므로
//!   에이전트 요청에 쓰지 않습니다.
//! - **busy**: 에이전트 요청은 처리 전 요청 수 상한([`DEFAULT_IN_FLIGHT_LIMIT`])을 따로 둡니다. 상한에 닿거나 queue
//!   lane이 차면 [`AgentIoError::Busy`]로 바로 거부합니다(기다리거나 이미 접수한 요청을 버리지 않음). 상한이 lane
//!   용량보다 작아서 에이전트 요청이 사용자 command 자리를 다 쓰지 못합니다.
//! - **기준 revision**: proposal의 `base_revision`은 document thread에서 창이 열어 둔 저장 generation
//!   ([`crate::agent_edit::project_revision`])과 대조하고, 다르면 대기로 두지 않고 거부합니다(ADR-0093 Consequences).
//! - **document thread 안 작업**: 제안은 승인 대기로 두기만 합니다(문서는 사용자가 승인한 뒤
//!   [`crate::agent_edit::AgentWindow::apply_proposal`]이 바꿈). lease 요청은 승인 대기로 두고 잠그지 않습니다.
//!
//! IPC 연결의 frame 읽기와 tool 인자 해석(`stellar-local-ipc`·MCP 브리지 쪽)은 [`AgentRequest`]를 만드는 앞단이고
//! 이 module 밖입니다.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::JoinHandle;

use stellar_agent_edit::agent_history::{
    CompletedEdit, EndReason, ProposalId, SessionEndReport, SessionError, SessionOrigin,
};
use stellar_agent_edit::attached::edit::{decode_proposal, AttachedEditError, DecodedProposal};
use stellar_agent_edit::lease::{LeaseState, LeaseTargets};
use stellar_agent_edit::permission::{AgentSessionId, ConnectionApproval};
use stellar_agent_host::session::SessionEvent;
use stellar_commands::id::CommandId;
use stellar_commands::lock::lease::LeaseId;
use stellar_ops::apply::ApplyRequest;

use crate::agent_edit::{
    monotonic_ms, project_revision, submit_window_job, AgentEditError, AgentJobError, AgentWindow,
};
use crate::app::{AppRuntime, JobPublisher, WindowSessionHandle};
use crate::command_queue::SubmitOptions;
use crate::document_thread::JobTicket;

/// 처리 전 에이전트 요청 수 기본 상한. XII.1.2 `Command` lane 기본 용량(1024)보다 작습니다.
pub const DEFAULT_IN_FLIGHT_LIMIT: usize = 64;

/// I/O 스레드 수신함 기본 용량.
pub const DEFAULT_INBOX_CAPACITY: usize = 256;

/// 에이전트가 보낸 요청(앞단이 tool 인자·IPC frame을 해석한 결과).
#[derive(Debug, Clone, PartialEq)]
pub enum AgentRequest {
    /// 세션 등록(bridge 연결·내장 세션 시작).
    Start {
        /// 세션.
        session: AgentSessionId,
        /// 세션 출처.
        origin: SessionOrigin,
        /// CLI가 보고한 세션 표시 이름.
        name: String,
    },
    /// `document.begin_agent_edit`: 대상 선언과 lease 요청(승인 대기).
    BeginEdit {
        /// 세션.
        session: AgentSessionId,
        /// lease(편집 세션) id.
        lease: LeaseId,
        /// 대상 선언.
        targets: LeaseTargets,
    },
    /// command proposal(승인 대기).
    Propose {
        /// 세션.
        session: AgentSessionId,
        /// 세션의 lease.
        lease: LeaseId,
        /// transaction id.
        id: CommandId,
        /// 해석한 proposal 요청.
        request: ApplyRequest,
    },
    /// 정상 완료(CompleteAgentEdit).
    Complete {
        /// 세션.
        session: AgentSessionId,
        /// 확정할 history 항목 id.
        entry_id: CommandId,
    },
    /// 결합된 bridge가 연결됨(앱 시작 세션이면 AgentSession `BridgeBound`, ADR-0124 결정 1).
    Bound {
        /// 세션.
        session: AgentSessionId,
    },
    /// bridge 연결 종료.
    Disconnected {
        /// 세션.
        session: AgentSessionId,
        /// "중단됨" 항목 id.
        entry_id: CommandId,
    },
    /// 종료 절차(agent process 종료, 재연결 유예 초과 등).
    End {
        /// 세션.
        session: AgentSessionId,
        /// 이유.
        reason: EndReason,
        /// "중단됨" 항목 id.
        entry_id: CommandId,
    },
}

/// document thread가 처리한 결과.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentReply {
    /// 세션 등록, 첫 연결 승인 상태.
    Started(ConnectionApproval),
    /// lease 요청 상태.
    LeaseRequested(LeaseState),
    /// 승인 대기 제안.
    Queued(ProposalId),
    /// 정상 완료.
    Completed(CompletedEdit),
    /// 종료 절차(연결 종료에서 앱 시작 세션을 유지하면 `None`).
    Ended(Option<Box<SessionEndReport>>),
    /// bridge 결합을 기록함. 재연결이면 세션의 진행 중 편집 lease를 함께 돌려줘 앞단이 이어서 쓰게 합니다
    /// (재연결 유예 안에서 같은 편집을 계속함, ADR-0005 §4.2).
    Bound {
        /// 세션의 진행 중 편집 lease(없으면 `None`).
        lease: Option<LeaseId>,
    },
}

/// 에이전트 요청 실패. 어느 경우든 문서는 바뀌지 않았습니다.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentIoError {
    /// 처리 전 요청 상한·queue 용량이 찼습니다. 나중에 다시 보냅니다.
    Busy,
    /// document thread 또는 I/O 스레드가 멈췄습니다.
    Stopped,
    /// 열려 있지 않은 창 session입니다.
    UnknownWindow(WindowSessionHandle),
    /// proposal을 닫힌 enum command로 바꿀 수 없거나, 이 창 문서의 것이 아니거나, `base_revision`이 창이 열어 둔
    /// 저장 generation과 다릅니다(`AttachedEditError::StaleBase`).
    Invalid(AttachedEditError),
    /// 세션·lease 절차가 거부했습니다.
    Edit(AgentEditError),
    /// 그 세션의 첫 연결 승인이 아직 없거나 거부됐습니다(XIX.3.3, ADR-0124 결정 3). lease 요청·제안을 받지 않습니다.
    ConnectionNotApproved(ConnectionApproval),
}

impl std::fmt::Display for AgentIoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentIoError::Busy => f.write_str("앱이 바빠 에이전트 요청을 받지 못했습니다"),
            AgentIoError::Stopped => f.write_str("앱 document thread가 멈췄습니다"),
            AgentIoError::UnknownWindow(handle) => {
                write!(f, "열려 있지 않은 창 session입니다: {}", handle.0)
            }
            AgentIoError::Invalid(e) => write!(f, "{e}"),
            AgentIoError::Edit(e) => write!(f, "{e}"),
            AgentIoError::ConnectionNotApproved(state) => {
                write!(f, "에이전트 연결이 승인되지 않았습니다({state:?})")
            }
        }
    }
}

impl std::error::Error for AgentIoError {}

impl From<AgentJobError> for AgentIoError {
    fn from(error: AgentJobError) -> AgentIoError {
        match error {
            AgentJobError::UnknownWindow(handle) => AgentIoError::UnknownWindow(handle),
            AgentJobError::Busy => AgentIoError::Busy,
            AgentJobError::Stopped => AgentIoError::Stopped,
        }
    }
}

/// 처리 결과 ticket.
pub type AgentTicket = JobTicket<Result<Result<AgentReply, AgentIoError>, AgentJobError>>;

/// 검증을 마친 요청(document thread로 옮기는 값).
#[derive(Debug)]
enum Prepared {
    Start {
        session: AgentSessionId,
        origin: SessionOrigin,
        name: String,
    },
    BeginEdit {
        session: AgentSessionId,
        lease: LeaseId,
        targets: LeaseTargets,
    },
    Propose {
        session: AgentSessionId,
        lease: LeaseId,
        id: CommandId,
        proposal: DecodedProposal,
    },
    Complete {
        session: AgentSessionId,
        entry_id: CommandId,
    },
    Bound {
        session: AgentSessionId,
    },
    Disconnected {
        session: AgentSessionId,
        entry_id: CommandId,
    },
    End {
        session: AgentSessionId,
        reason: EndReason,
        entry_id: CommandId,
    },
}

/// 이 스레드에서 하는 검증: proposal을 닫힌 enum command로 바꿉니다. 문서에 닿지 않습니다.
fn prepare(request: AgentRequest) -> Result<Prepared, AgentIoError> {
    Ok(match request {
        AgentRequest::Start {
            session,
            origin,
            name,
        } => Prepared::Start {
            session,
            origin,
            name,
        },
        AgentRequest::BeginEdit {
            session,
            lease,
            targets,
        } => Prepared::BeginEdit {
            session,
            lease,
            targets,
        },
        AgentRequest::Propose {
            session,
            lease,
            id,
            request,
        } => Prepared::Propose {
            session,
            lease,
            id,
            proposal: decode_proposal(&request).map_err(AgentIoError::Invalid)?,
        },
        AgentRequest::Complete { session, entry_id } => Prepared::Complete { session, entry_id },
        AgentRequest::Bound { session } => Prepared::Bound { session },
        AgentRequest::Disconnected { session, entry_id } => {
            Prepared::Disconnected { session, entry_id }
        }
        AgentRequest::End {
            session,
            reason,
            entry_id,
        } => Prepared::End {
            session,
            reason,
            entry_id,
        },
    })
}

/// lease 요청·제안 전에 그 세션의 첫 연결 승인을 봅니다(XIX.3.3). 모르는 세션은 세션 절차의 오류로 넘깁니다.
fn require_connection(
    window: &AgentWindow<'_>,
    session: AgentSessionId,
) -> Result<(), AgentIoError> {
    match window
        .agent()
        .permissions()
        .connection(&window.permission_key(session))
    {
        None | Some(ConnectionApproval::Approved) => Ok(()),
        Some(state) => Err(AgentIoError::ConnectionNotApproved(state)),
    }
}

/// document thread job 안에서 검증된 요청을 처리합니다.
fn run(window: &mut AgentWindow<'_>, prepared: Prepared) -> Result<AgentReply, AgentIoError> {
    let edit = AgentIoError::Edit;
    match &prepared {
        Prepared::BeginEdit { session, .. } | Prepared::Propose { session, .. } => {
            require_connection(window, *session)?;
        }
        _ => {}
    }
    match prepared {
        Prepared::Start {
            session,
            origin,
            name,
        } => window
            .start_session(session, origin, &name, monotonic_ms())
            .map(AgentReply::Started)
            .map_err(edit),
        Prepared::BeginEdit {
            session,
            lease,
            targets,
        } => window
            .submit_lease(session, lease, &targets)
            .map(AgentReply::LeaseRequested)
            .map_err(edit),
        Prepared::Propose {
            session,
            lease,
            id,
            proposal,
        } => {
            proposal
                .check_base(project_revision(window.session()))
                .map_err(AgentIoError::Invalid)?;
            let state = window
                .session()
                .document()
                .ok_or(AgentIoError::Edit(AgentEditError::NoDocument))?;
            let transaction = proposal
                .transaction(state, lease, id)
                .map_err(AgentIoError::Invalid)?;
            window
                .queue_proposal(session, transaction)
                .map(AgentReply::Queued)
                .map_err(edit)
        }
        Prepared::Complete { session, entry_id } => window
            .complete(session, entry_id)
            .map(AgentReply::Completed)
            .map_err(edit),
        Prepared::Bound { session } => {
            if let Some(host) = window.agent().hosted().host_of(session) {
                let report = window
                    .host_event(
                        host,
                        SessionEvent::BridgeBound,
                        CommandId::from_stable_id(crate::agent_session::fresh_id()),
                        monotonic_ms(),
                    )
                    .map_err(edit)?;
                // 유예가 지난 뒤의 재결합은 같은 세션으로 받지 않고 유예 초과로 끝냅니다(AgentSession 규칙).
                if let Some(ended) = report.ended {
                    return Ok(AgentReply::Ended(Some(ended)));
                }
            } else if !window.agent().sessions().is_active(session) {
                return Err(edit(AgentEditError::Session(SessionError::UnknownSession(
                    session,
                ))));
            }
            Ok(AgentReply::Bound {
                lease: window.agent().sessions().lease(session),
            })
        }
        Prepared::Disconnected { session, entry_id } => {
            let now = monotonic_ms();
            let report = window
                .on_bridge_disconnected(session, entry_id, now)
                .map_err(edit)?;
            // 앱 시작 세션은 유지하고 AgentSession의 재연결 유예를 시작합니다(ADR-0124 결정 1).
            if report.is_none() {
                if let Some(host) = window.agent().hosted().host_of(session) {
                    let _ =
                        window.host_event(host, SessionEvent::BridgeDisconnected, entry_id, now);
                }
            }
            Ok(AgentReply::Ended(report.map(Box::new)))
        }
        Prepared::End {
            session,
            reason,
            entry_id,
        } => {
            let now = monotonic_ms();
            let report = window.end(session, reason, entry_id, now).map_err(edit)?;
            // 앱 시작 세션이면 AgentSession 상태도 맞춥니다(ADR-0124 결정 1).
            window.notify_host_ended(&report, now);
            Ok(AgentReply::Ended(Some(Box::new(report))))
        }
    }
}

/// 처리 전 요청 자리 하나. job이 끝나거나 버려지면(drop) 자리를 돌려줍니다.
#[derive(Debug)]
struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// 에이전트 요청 접수기. 복제해 I/O 스레드로 옮깁니다. 문서에 직접 닿는 수단이 없습니다.
#[derive(Debug, Clone)]
pub struct AgentIo {
    publisher: JobPublisher,
    in_flight: Arc<AtomicUsize>,
    limit: usize,
}

impl AgentIo {
    /// 처리 전 요청 수.
    pub fn in_flight(&self) -> usize {
        self.in_flight.load(Ordering::Acquire)
    }

    /// 처리 전 요청 상한.
    pub fn limit(&self) -> usize {
        self.limit
    }

    fn reserve(&self) -> Option<Slot> {
        self.in_flight
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < self.limit).then_some(n + 1)
            })
            .ok()
            .map(|_| Slot(Arc::clone(&self.in_flight)))
    }

    /// 요청을 검증해 document thread에 낮은 priority로 넣습니다. 검증 실패는 넣지 않고 거부하고, 상한·queue가
    /// 차면 [`AgentIoError::Busy`]입니다. 기다리지 않습니다.
    pub fn submit(
        &self,
        window: WindowSessionHandle,
        request: AgentRequest,
    ) -> Result<AgentTicket, AgentIoError> {
        let prepared = prepare(request)?;
        let slot = self.reserve().ok_or(AgentIoError::Busy)?;
        submit_window_job(
            &self.publisher,
            SubmitOptions::command(),
            window,
            move |agent: &mut AgentWindow<'_>| {
                let _slot = slot;
                run(agent, prepared)
            },
        )
        .map_err(AgentIoError::from)
    }
}

impl AppRuntime {
    /// 에이전트 요청 접수기(처리 전 요청 상한 [`DEFAULT_IN_FLIGHT_LIMIT`]).
    pub fn agent_io(&self) -> AgentIo {
        self.agent_io_with_limit(DEFAULT_IN_FLIGHT_LIMIT)
    }

    /// 처리 전 요청 상한을 정해 접수기를 만듭니다(1 이상).
    pub fn agent_io_with_limit(&self, limit: usize) -> AgentIo {
        AgentIo {
            publisher: self.job_publisher(),
            in_flight: Arc::new(AtomicUsize::new(0)),
            limit: limit.max(1),
        }
    }
}

/// I/O 스레드로 보내는 요청 하나와 접수 결과를 받을 곳.
type Envelope = (
    WindowSessionHandle,
    AgentRequest,
    mpsc::Sender<Result<AgentTicket, AgentIoError>>,
);

/// 앱 Agent Host·IPC I/O 스레드. 앞단이 해석한 요청을 받아 검증하고 document thread에 넣은 뒤 접수 결과(ticket 또는
/// 거부)를 돌려줍니다. document thread 처리를 기다리지 않아 다음 frame을 바로 읽습니다.
#[derive(Debug)]
pub struct AgentIoThread {
    inbox: Option<SyncSender<Envelope>>,
    handle: Option<JoinHandle<()>>,
}

impl AgentIoThread {
    /// `io`로 접수하는 스레드를 시작합니다. 수신함 용량은 `inbox_capacity`(1 이상)입니다.
    pub fn spawn(io: AgentIo, inbox_capacity: usize) -> std::io::Result<AgentIoThread> {
        let (inbox, receiver): (SyncSender<Envelope>, Receiver<Envelope>) =
            mpsc::sync_channel(inbox_capacity.max(1));
        let handle = std::thread::Builder::new()
            .name("stellar-agent-io".into())
            .spawn(move || {
                for (window, request, reply) in receiver {
                    let _ = reply.send(io.submit(window, request));
                }
            })?;
        Ok(AgentIoThread {
            inbox: Some(inbox),
            handle: Some(handle),
        })
    }

    /// 요청을 스레드에 넘깁니다. 수신함이 차면 [`AgentIoError::Busy`], 스레드가 끝났으면
    /// [`AgentIoError::Stopped`]입니다. 접수 결과는 돌려준 수신자로 옵니다.
    pub fn send(
        &self,
        window: WindowSessionHandle,
        request: AgentRequest,
    ) -> Result<mpsc::Receiver<Result<AgentTicket, AgentIoError>>, AgentIoError> {
        let inbox = self.inbox.as_ref().ok_or(AgentIoError::Stopped)?;
        let (reply, accepted) = mpsc::channel();
        inbox
            .try_send((window, request, reply))
            .map_err(|error| match error {
                TrySendError::Full(_) => AgentIoError::Busy,
                TrySendError::Disconnected(_) => AgentIoError::Stopped,
            })?;
        Ok(accepted)
    }
}

impl Drop for AgentIoThread {
    fn drop(&mut self) {
        self.inbox.take();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
#[path = "agent_io_tests.rs"]
mod tests;
