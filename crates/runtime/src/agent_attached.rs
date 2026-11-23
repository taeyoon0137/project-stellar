//! 앱 runtime의 attached host 구현 (ADR-0005 §8, ADR-0124 결정 2).
//!
//! [`AppAgentHost`]는 `stellar-agent-protocol`의 [`AttachedHost`]를 구현합니다. bridge 연결 하나를 창 하나·에이전트 편집
//! 세션 하나에 묶고(XIX.9.4 결합 단위), attached `tools/list`·`tools/call` 판정과 요청 접수를 합니다.
//!
//! - 앱이 시작한 내장 세션: 목록·판정은 agent-host `builtin_tools`·`grant_route`(모드 상한, 터미널·파일 삭제 제외,
//!   ADR-0101)입니다.
//! - 외부 클라이언트 세션: attached catalog의 기본 namespace 목록입니다. 권한 상태는 document thread의 권한 저장소가
//!   판정합니다(XIX.3, 첫 연결 승인은 [`crate::agent_io`]가 봄).
//! - 접수는 [`AgentIo::submit`]입니다(검증·priority·busy, ADR-0112 결정 5). 이 host는 문서에 닿는 수단이 없습니다.

use std::collections::BTreeMap;

use stellar_agent_edit::agent_history::{SessionError, SessionOrigin};
use stellar_agent_edit::permission::AgentSessionId;
use stellar_agent_host::mode::{builtin_tools, grant_route, GrantRoute, ToolRefusal};
use stellar_agent_host::session::SessionMode;
use stellar_agent_protocol::catalog::resources::Backend;
use stellar_agent_protocol::catalog::tools::{listed_tools, EnabledNamespaces, ToolDef};
use stellar_agent_protocol::host::{AttachedHost, HostRefusal};
use stellar_agent_protocol::outcome::ResultKind;
use stellar_agent_protocol::permission::Permission;

use crate::agent_edit::AgentEditError;
use crate::agent_io::{AgentIo, AgentIoError, AgentRequest, AgentTicket};
use crate::app::WindowSessionHandle;

/// bridge 연결 handle(앱 프로세스 안에서 유일, IPC 연결 id 등).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AgentConnection(pub u64);

/// 연결이 묶인 창·세션.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundConnection {
    /// 결합 창.
    pub window: WindowSessionHandle,
    /// 에이전트 편집 세션.
    pub session: AgentSessionId,
    /// 세션 출처.
    pub origin: SessionOrigin,
    /// 앱 시작 세션의 모드(상한). 외부 클라이언트는 `None`.
    pub mode: Option<SessionMode>,
}

/// 앱 runtime의 attached host.
#[derive(Debug)]
pub struct AppAgentHost {
    io: AgentIo,
    connections: BTreeMap<AgentConnection, BoundConnection>,
}

const UNBOUND: HostRefusal = HostRefusal::new(ResultKind::NotTrusted, "unbound-connection");

impl AppAgentHost {
    /// `io`로 접수하는 host.
    pub fn new(io: AgentIo) -> AppAgentHost {
        AppAgentHost {
            io,
            connections: BTreeMap::new(),
        }
    }

    /// 연결을 창·세션에 묶습니다. 이미 묶인 연결이면 거부합니다(결합은 세션 시작 때 고정).
    pub fn bind(
        &mut self,
        connection: AgentConnection,
        bound: BoundConnection,
    ) -> Result<(), HostRefusal> {
        if self.connections.contains_key(&connection) {
            return Err(HostRefusal::new(
                ResultKind::InvalidArguments,
                "connection-already-bound",
            ));
        }
        self.connections.insert(connection, bound);
        Ok(())
    }

    /// 연결 결합을 풉니다(연결 종료).
    pub fn unbind(&mut self, connection: AgentConnection) -> Option<BoundConnection> {
        self.connections.remove(&connection)
    }

    /// 결합 정보.
    pub fn bound(&self, connection: AgentConnection) -> Option<&BoundConnection> {
        self.connections.get(&connection)
    }

    /// 사용자가 앱 UI에서 바꾼 앱 시작 세션 모드를 이 host의 판정에 반영합니다. 권한 폐기는 document thread의
    /// `AgentWindow::change_hosted_mode`가 합니다.
    pub fn set_mode(&mut self, connection: AgentConnection, mode: SessionMode) {
        if let Some(bound) = self.connections.get_mut(&connection) {
            if bound.mode.is_some() {
                bound.mode = Some(mode);
            }
        }
    }
}

impl AttachedHost for AppAgentHost {
    type Connection = AgentConnection;
    type Request = AgentRequest;
    type Accepted = AgentTicket;

    fn listed_tools(&self, connection: AgentConnection) -> Vec<&'static ToolDef> {
        match self.connections.get(&connection) {
            None => Vec::new(),
            Some(BoundConnection {
                mode: Some(mode), ..
            }) => builtin_tools(*mode),
            Some(_) => listed_tools(Backend::Attached, EnabledNamespaces::default()).collect(),
        }
    }

    fn permits(
        &self,
        connection: AgentConnection,
        permission: Permission,
    ) -> Result<(), HostRefusal> {
        let bound = self.connections.get(&connection).ok_or(UNBOUND)?;
        let Some(mode) = bound.mode else {
            return Ok(());
        };
        match grant_route(mode, permission) {
            GrantRoute::Session | GrantRoute::PerExecution => Ok(()),
            GrantRoute::Excluded => Err(HostRefusal::new(
                ResultKind::UnknownTool,
                ToolRefusal::Excluded(permission).code(),
            )),
            GrantRoute::AboveMode => Err(HostRefusal::new(
                ResultKind::OutOfScope,
                ToolRefusal::AboveMode { mode, permission }.code(),
            )),
        }
    }

    fn submit(
        &self,
        connection: AgentConnection,
        request: AgentRequest,
    ) -> Result<AgentTicket, HostRefusal> {
        let bound = self.connections.get(&connection).ok_or(UNBOUND)?;
        self.io
            .submit(bound.window, request)
            .map_err(|error| refusal(&error))
    }
}

/// 에이전트 요청 실패의 결과 종류와 고정 코드(ADR-0085 결정 7).
pub fn refusal(error: &AgentIoError) -> HostRefusal {
    match error {
        AgentIoError::Busy => HostRefusal::new(ResultKind::Busy, "busy"),
        AgentIoError::Stopped => HostRefusal::new(ResultKind::Unavailable, "app-stopped"),
        AgentIoError::UnknownWindow(_) => HostRefusal::new(ResultKind::NotFound, "unknown-window"),
        AgentIoError::Invalid(e) => HostRefusal::new(e.kind(), "invalid-proposal"),
        AgentIoError::ConnectionNotApproved(_) => {
            HostRefusal::new(ResultKind::NotTrusted, "connection-not-approved")
        }
        AgentIoError::Edit(e) => edit_refusal(e),
    }
}

fn edit_refusal(error: &AgentEditError) -> HostRefusal {
    match error {
        AgentEditError::NoDocument => HostRefusal::new(ResultKind::NotFound, "no-document"),
        AgentEditError::Lease(e) => HostRefusal::new(e.kind(), "lease"),
        AgentEditError::Session(SessionError::Apply(e)) => HostRefusal::new(e.kind(), "apply"),
        AgentEditError::Session(SessionError::Lease(e)) => HostRefusal::new(e.kind(), "lease"),
        AgentEditError::Session(
            SessionError::UnknownSession(_)
            | SessionError::UnknownProposal(_)
            | SessionError::NoEdit,
        ) => HostRefusal::new(ResultKind::NotFound, "session"),
        AgentEditError::Session(_) | AgentEditError::Link(_) => {
            HostRefusal::new(ResultKind::Rejected, "session")
        }
        AgentEditError::Revert(_) | AgentEditError::Provenance(_) => {
            HostRefusal::new(ResultKind::Internal, "internal")
        }
    }
}

#[cfg(test)]
#[path = "agent_attached_tests.rs"]
mod tests;
