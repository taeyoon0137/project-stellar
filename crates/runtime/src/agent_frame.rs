//! bridge frame을 [`AgentRequest`]로 바꾸는 앞단 (XXIX.3.4, ADR-0124 결정 3).
//!
//! handshake를 마친 bridge 연결에서 읽은 frame(`stellar_local_ipc::frame`)을 I/O 스레드 요청으로 바꿉니다.
//!
//! - 형식 검사(StableId·tool 인자·대상 수·설명 길이)는 브리지가 이미 했습니다(`stellar_mcp::lease`, catalog 범위 검사).
//!   앞단은 값 해석만 기존 공개 함수로 합니다: base revision은 `ProjectRevision::from_str`, 전역 resource 종류는
//!   `ExternalKind::from_name`, command는 `CommandInput::Canonical`로 넣어 I/O 스레드의 `decode_proposal`(`stellar apply`와
//!   같은 `decode_commands`)이 닫힌 enum으로 바꿉니다. 같은 값을 따로 해석하는 규칙이 없습니다.
//! - lease id·transaction id·완료 항목 id는 앱이 만듭니다. 브리지가 보낸 id를 쓰지 않습니다. 연결마다 진행 중 lease를
//!   기억해 `Propose`에 씁니다(lease 요청이 접수된 뒤에만 기억, 완료하면 지움).
//! - 권한 상한은 host trait([`AttachedHost::permits`]·[`AttachedHost::callable_tool`])로 먼저 봅니다. `BeginEdit`은 Propose
//!   권한, `Propose`는 `document.propose_commands`입니다(attached에서 proposal은 사용자 승인 대기).
//! - 연결 종료는 `Disconnected` 요청입니다. [`AgentFrontEnd::serve`]가 연결 하나를 읽고 응답을 씁니다. 연결 하나는 요청을
//!   하나씩 처리합니다(ADR-0085 `MAX_IN_FLIGHT` 1).
//! - local-ipc 연결 감시와의 연결(ADR-0132 결정 2): [`FrameConnection`]이 `stellar_local_ipc::session::FrameService`를
//!   구현합니다. 감시 스레드가 handshake를 마친 연결을 넘기면 연결 하나에 앞단 하나를 두고 창·세션에 묶은 뒤
//!   [`AgentFrontEnd::serve`]로 끝까지 처리합니다. 연결 종료 절차는 `serve`가 넣는 `Disconnected` 하나입니다.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::str::FromStr;
use std::time::Duration;

use stellar_agent_edit::agent_history::SessionOrigin;
use stellar_agent_edit::lease::LeaseTargets;
use stellar_agent_edit::permission::AgentSessionId;
use stellar_agent_host::session::SessionMode;
use stellar_agent_protocol::catalog::tools::PROPOSE_COMMANDS;
use stellar_agent_protocol::host::{AttachedHost, HostRefusal};
use stellar_agent_protocol::limits::CALL_DEADLINE;
use stellar_agent_protocol::outcome::ResultKind;
use stellar_agent_protocol::permission::Permission;
use stellar_commands::id::CommandId;
use stellar_commands::lock::lease::{ExternalKind, ExternalResource, LeaseId};
use stellar_local_ipc::frame::{read_frame, write_frame, Frame, FrameError};
use stellar_local_ipc::session::{CloseCause, ConnectionId, FrameService};
use stellar_ops::apply::{ApplyRequest, CommandInput};
use stellar_ops::revision::ProjectRevision;

use crate::agent_attached::{refusal, AgentConnection, AppAgentHost, BoundConnection};
use crate::agent_io::{AgentIoError, AgentReply, AgentRequest, AgentTicket};
use crate::agent_session::{fresh_id, new_edit_session_id};
use crate::app::WindowSessionHandle;

const NO_EDIT: HostRefusal = HostRefusal::new(ResultKind::NotFound, "no-edit");

#[derive(Debug, Default)]
struct Open {
    lease: Option<LeaseId>,
}

/// frame 앞단(연결 표와 host).
#[derive(Debug)]
pub struct AgentFrontEnd {
    host: AppAgentHost,
    open: BTreeMap<AgentConnection, Open>,
}

/// 앞단이 만든 요청과, 접수되면 연결에 기억할 lease.
#[derive(Debug)]
pub struct Converted {
    /// I/O 스레드 요청.
    pub request: AgentRequest,
    /// `BeginEdit`이면 앱이 만든 lease id.
    pub lease: Option<LeaseId>,
}

fn invalid(code: &'static str) -> HostRefusal {
    HostRefusal::new(ResultKind::InvalidArguments, code)
}

impl AgentFrontEnd {
    /// host로 접수하는 앞단.
    pub fn new(host: AppAgentHost) -> AgentFrontEnd {
        AgentFrontEnd {
            host,
            open: BTreeMap::new(),
        }
    }

    /// host(읽기).
    pub fn host(&self) -> &AppAgentHost {
        &self.host
    }

    /// 연결의 진행 중 lease(접수된 `BeginEdit`, 완료 전).
    pub fn lease(&self, connection: AgentConnection) -> Option<LeaseId> {
        self.open.get(&connection).and_then(|o| o.lease)
    }

    /// host(모드 반영 등).
    pub fn host_mut(&mut self) -> &mut AppAgentHost {
        &mut self.host
    }

    /// 외부 클라이언트 bridge 연결을 창에 묶고 세션 등록 요청을 넣습니다. 세션 id는 앱이 만듭니다.
    pub fn connect_external(
        &mut self,
        connection: AgentConnection,
        window: WindowSessionHandle,
        name: &str,
    ) -> Result<(AgentSessionId, AgentTicket), HostRefusal> {
        let session = new_edit_session_id();
        self.host.bind(
            connection,
            BoundConnection {
                window,
                session,
                origin: SessionOrigin::ExternalClient,
                mode: None,
            },
        )?;
        self.open.insert(connection, Open::default());
        let ticket = self.host.submit(
            connection,
            AgentRequest::Start {
                session,
                origin: SessionOrigin::ExternalClient,
                name: name.to_string(),
            },
        );
        if ticket.is_err() {
            self.host.unbind(connection);
            self.open.remove(&connection);
        }
        ticket.map(|ticket| (session, ticket))
    }

    /// 앱이 시작한 세션(`AgentWindow::start_hosted`로 이미 등록)의 bridge 연결을 묶고, 결합을 AgentSession에 알리는
    /// 요청(`Bound`)을 넣어 결과를 기다립니다(최대 `deadline`). 재연결 유예 안의 재결합이면 세션의 진행 중 편집 lease를
    /// 이 연결에 다시 기억해 같은 편집을 이어 갑니다(ADR-0005 §4.2). 유예가 지나 세션이 끝났거나 접수·처리가 실패하면
    /// 결합을 풉니다.
    pub fn bind_hosted(
        &mut self,
        connection: AgentConnection,
        window: WindowSessionHandle,
        session: AgentSessionId,
        mode: SessionMode,
        deadline: Duration,
    ) -> Result<AgentReply, HostRefusal> {
        self.host.bind(
            connection,
            BoundConnection {
                window,
                session,
                origin: SessionOrigin::AppStarted,
                mode: Some(mode),
            },
        )?;
        self.open.insert(connection, Open::default());
        let outcome = self
            .host
            .submit(connection, AgentRequest::Bound { session })
            .and_then(|ticket| wait_reply(ticket, deadline));
        match &outcome {
            Ok(reply @ AgentReply::Bound { .. }) => self.settle(connection, None, reply),
            _ => {
                self.host.unbind(connection);
                self.open.remove(&connection);
            }
        }
        outcome
    }

    /// frame 하나를 요청으로 바꿉니다(모듈 문서). 문서에 닿지 않고 연결 상태도 바꾸지 않습니다.
    pub fn convert(
        &self,
        connection: AgentConnection,
        frame: Frame,
    ) -> Result<Converted, HostRefusal> {
        let bound = self.host.bound(connection).ok_or(HostRefusal::new(
            ResultKind::NotTrusted,
            "unbound-connection",
        ))?;
        let session = bound.session;
        let current = self.open.get(&connection).and_then(|o| o.lease);
        match frame {
            Frame::BeginEdit {
                document,
                targets,
                externals,
                ..
            } => {
                self.host.permits(connection, Permission::Propose)?;
                let externals = externals
                    .into_iter()
                    .map(|(kind, id)| {
                        ExternalKind::from_name(&kind)
                            .map(|kind| ExternalResource { kind, id })
                            .ok_or(invalid("unknown-external-kind"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let lease = LeaseId(fresh_id());
                Ok(Converted {
                    request: AgentRequest::BeginEdit {
                        session,
                        lease,
                        targets: LeaseTargets {
                            document,
                            targets,
                            externals,
                        },
                    },
                    lease: Some(lease),
                })
            }
            Frame::Propose {
                base_revision,
                document,
                commands,
                ..
            } => {
                self.host.callable_tool(connection, PROPOSE_COMMANDS.name)?;
                let lease = current.ok_or(NO_EDIT)?;
                let base_revision = ProjectRevision::from_str(&base_revision)
                    .map_err(|_| invalid("invalid-base-revision"))?;
                Ok(Converted {
                    request: AgentRequest::Propose {
                        session,
                        lease,
                        id: CommandId::from_stable_id(fresh_id()),
                        request: ApplyRequest {
                            base_revision,
                            document,
                            commands: commands
                                .into_iter()
                                .map(|(kind, bytes)| CommandInput::Canonical { kind, bytes })
                                .collect(),
                        },
                    },
                    lease: None,
                })
            }
            Frame::Complete { .. } => Ok(Converted {
                request: AgentRequest::Complete {
                    session,
                    entry_id: CommandId::from_stable_id(fresh_id()),
                },
                lease: None,
            }),
            Frame::Reply { .. } => Err(invalid("reply-from-bridge")),
        }
    }

    /// 요청 처리 결과를 연결 상태에 반영합니다(접수된 lease 기억, 완료하면 지움).
    fn settle(
        &mut self,
        connection: AgentConnection,
        converted_lease: Option<LeaseId>,
        reply: &AgentReply,
    ) {
        let Some(open) = self.open.get_mut(&connection) else {
            return;
        };
        match reply {
            AgentReply::LeaseRequested(_) => open.lease = converted_lease,
            AgentReply::Bound { lease } => open.lease = *lease,
            AgentReply::Completed(_) | AgentReply::Ended(_) => open.lease = None,
            AgentReply::Started(_) | AgentReply::Queued(_) => {}
        }
    }

    /// 연결이 끝났습니다. 결합을 풀고 `Disconnected` 요청을 넣습니다(외부 클라이언트는 종료 절차, 앱 시작 세션은 유지).
    pub fn disconnected(
        &mut self,
        connection: AgentConnection,
    ) -> Result<AgentTicket, HostRefusal> {
        let ticket =
            self.host.submit(
                connection,
                AgentRequest::Disconnected {
                    session: self.host.bound(connection).map(|b| b.session).ok_or(
                        HostRefusal::new(ResultKind::NotTrusted, "unbound-connection"),
                    )?,
                    entry_id: CommandId::from_stable_id(fresh_id()),
                },
            );
        self.host.unbind(connection);
        self.open.remove(&connection);
        ticket
    }

    /// 연결 하나를 끝까지 처리합니다: frame을 읽어 요청으로 바꿔 접수하고, 처리 결과를 `Reply` frame으로 씁니다. 형식이
    /// 틀린 frame을 받거나 쓰기에 실패하면 멈춥니다(fail closed). 끝나면 `Disconnected`를 넣고 끝난 이유를 돌려줍니다.
    pub fn serve<S: Read + Write>(
        &mut self,
        connection: AgentConnection,
        stream: &mut S,
        deadline: Duration,
    ) -> FrameError {
        let end = loop {
            let frame = match read_frame(stream) {
                Ok(frame) => frame,
                Err(error) => break error,
            };
            let request = frame.request();
            let outcome = self.handle(connection, frame, deadline);
            if let Err(error) = write_frame(stream, &reply_frame(request, outcome)) {
                break error;
            }
        };
        let _ = self.disconnected(connection);
        end
    }

    /// frame 하나를 처리하고 결과를 기다립니다(최대 `deadline`, 기본은 ADR-0085 `CALL_DEADLINE`).
    pub fn handle(
        &mut self,
        connection: AgentConnection,
        frame: Frame,
        deadline: Duration,
    ) -> Result<AgentReply, HostRefusal> {
        let converted = self.convert(connection, frame)?;
        let ticket = self.host.submit(connection, converted.request)?;
        let outcome = wait_reply(ticket, deadline);
        if let Ok(reply) = &outcome {
            self.settle(connection, converted.lease, reply);
        }
        outcome
    }
}

/// frame 연결을 묶을 창·세션(연결 감시에 넘기기 전에 앱이 정함).
#[derive(Debug, Clone)]
pub enum ConnectionBinding {
    /// 외부 MCP client가 띄운 bridge. 연결마다 새 세션을 등록합니다([`AgentFrontEnd::connect_external`]).
    External {
        /// 결합 창(첫 연결 승인 화면에서 고른 창).
        window: WindowSessionHandle,
        /// 세션 표시 이름(데이터로만 다룸).
        name: String,
    },
    /// 앱이 시작한 세션의 bridge([`AgentFrontEnd::bind_hosted`], XIX.1.8 결합 token을 확인한 연결).
    Hosted {
        /// 시작한 창.
        window: WindowSessionHandle,
        /// 편집 세션.
        session: AgentSessionId,
        /// 세션 모드(상한).
        mode: SessionMode,
    },
}

/// local-ipc 연결 감시가 handshake 뒤 frame을 넘기는 연결 하나의 앞단(ADR-0132 결정 2).
///
/// 연결마다 앞단 하나(`AgentIo` 복제로 만든 host 하나)를 둡니다. 연결 상태(결합, 진행 중 lease)는 그 연결에만 있고
/// 세션·lease·권한 원본은 document thread에 있으므로 연결 사이에 공유할 상태가 없습니다.
#[derive(Debug)]
pub struct FrameConnection {
    front: AgentFrontEnd,
    binding: ConnectionBinding,
    deadline: Duration,
}

impl FrameConnection {
    /// `io`로 접수하는 연결 앞단. 응답 대기 상한은 ADR-0085 `CALL_DEADLINE`입니다.
    pub fn new(io: crate::agent_io::AgentIo, binding: ConnectionBinding) -> FrameConnection {
        FrameConnection {
            front: AgentFrontEnd::new(AppAgentHost::new(io)),
            binding,
            deadline: CALL_DEADLINE,
        }
    }

    /// 응답 대기 상한을 바꿉니다(테스트, 최대 `CALL_DEADLINE`).
    pub fn with_deadline(mut self, deadline: Duration) -> FrameConnection {
        self.deadline = deadline;
        self
    }

    /// 연결을 창·세션에 묶습니다. 거부되면(창 없음, 유예가 지난 세션, busy) 연결을 처리하지 않습니다.
    fn bind(&mut self, connection: AgentConnection) -> Result<(), HostRefusal> {
        match self.binding.clone() {
            ConnectionBinding::External { window, name } => {
                let (_, ticket) = self.front.connect_external(connection, window, &name)?;
                match wait_reply(ticket, self.deadline)? {
                    AgentReply::Started(_) => Ok(()),
                    _ => Err(HostRefusal::new(ResultKind::Internal, "unexpected-reply")),
                }
            }
            ConnectionBinding::Hosted {
                window,
                session,
                mode,
            } => match self
                .front
                .bind_hosted(connection, window, session, mode, self.deadline)?
            {
                AgentReply::Bound { .. } => Ok(()),
                _ => Err(HostRefusal::new(ResultKind::NotFound, "session-ended")),
            },
        }
    }
}

impl FrameService for FrameConnection {
    fn serve(
        &mut self,
        connection: ConnectionId,
        stream: &mut std::os::unix::net::UnixStream,
    ) -> CloseCause {
        let connection = AgentConnection(connection.get());
        if self.bind(connection).is_err() {
            // 결합하지 못한 연결은 frame을 읽지 않고 앱 쪽에서 닫습니다(fail closed). 외부 클라이언트 세션 등록이
            // 접수됐으면 같은 연결 종료 요청으로 정리합니다.
            if self.front.host().bound(connection).is_some() {
                let _ = self.front.disconnected(connection);
            }
            return CloseCause::AppShutdown;
        }
        let end = self.front.serve(connection, stream, self.deadline);
        CloseCause::of_frame_error(&end)
    }
}

/// 접수한 요청의 결과를 기다립니다(최대 `deadline`, ADR-0085 `CALL_DEADLINE`을 넘지 않음).
fn wait_reply(mut ticket: AgentTicket, deadline: Duration) -> Result<AgentReply, HostRefusal> {
    match ticket.wait_timeout(deadline.min(CALL_DEADLINE)) {
        Ok(Some(Ok(result))) => result.map_err(|error| refusal(&error)),
        Ok(Some(Err(job))) => Err(refusal(&AgentIoError::from(job))),
        Ok(None) => Err(HostRefusal::new(ResultKind::Timeout, "timeout")),
        Err(_) => Err(refusal(&AgentIoError::Stopped)),
    }
}

/// 처리 결과의 `Reply` frame. 결과 종류 이름과 문장만 싣습니다(문서 내용 없음).
pub fn reply_frame(request: u64, outcome: Result<AgentReply, HostRefusal>) -> Frame {
    let (result, message) = match outcome {
        Ok(reply) => (
            ResultKind::Ok.name(),
            match reply {
                AgentReply::Started(_) => "started",
                AgentReply::LeaseRequested(_) => "lease-requested",
                AgentReply::Queued(_) => "queued",
                AgentReply::Completed(_) => "completed",
                AgentReply::Ended(_) => "ended",
                AgentReply::Bound { .. } => "bound",
            },
        ),
        Err(refusal) => (refusal.kind.name(), refusal.code),
    };
    Frame::Reply {
        request,
        result: result.to_string(),
        message: message.to_string(),
    }
}

#[cfg(test)]
#[path = "agent_frame_tests.rs"]
mod tests;
