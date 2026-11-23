//! attached host 구현 test (ADR-0124 결정 2).

use stellar_agent_edit::attached::edit::AttachedEditError;
use stellar_agent_protocol::catalog::tools::{PROPOSE_COMMANDS, VALIDATE_DOCUMENT};
use stellar_core_types::ids::StableId;

use super::*;
use crate::app::AppRuntime;

fn bound(mode: Option<SessionMode>) -> BoundConnection {
    BoundConnection {
        window: WindowSessionHandle(1),
        session: AgentSessionId(StableId::new_v7(1, 1, 1)),
        origin: if mode.is_some() {
            SessionOrigin::AppStarted
        } else {
            SessionOrigin::ExternalClient
        },
        mode,
    }
}

fn host() -> (AppRuntime, AppAgentHost) {
    let app = AppRuntime::start().unwrap();
    let host = AppAgentHost::new(app.agent_io());
    (app, host)
}

#[test]
fn builtin_sessions_follow_the_mode_cap() {
    let (_app, mut host) = host();
    let (read_only, review) = (AgentConnection(1), AgentConnection(2));
    host.bind(read_only, bound(Some(SessionMode::ReadOnly)))
        .unwrap();
    host.bind(review, bound(Some(SessionMode::ReviewThenApply)))
        .unwrap();
    assert_eq!(
        host.listed_tools(read_only),
        builtin_tools(SessionMode::ReadOnly),
        "내장 세션 목록은 agent-host builtin_tools"
    );
    assert!(!host
        .listed_tools(read_only)
        .iter()
        .any(|t| t.name == PROPOSE_COMMANDS.name));
    assert_eq!(
        host.callable_tool(read_only, PROPOSE_COMMANDS.name),
        Err(HostRefusal::UNKNOWN_TOOL)
    );
    assert_eq!(
        host.permits(read_only, Permission::Propose),
        Err(HostRefusal::new(ResultKind::OutOfScope, "above-mode"))
    );
    assert_eq!(
        host.permits(review, Permission::Terminal),
        Err(HostRefusal::new(
            ResultKind::UnknownTool,
            "excluded-from-builtin-session"
        ))
    );
    assert_eq!(
        host.callable_tool(review, PROPOSE_COMMANDS.name),
        Ok(&PROPOSE_COMMANDS)
    );
    // 사용자가 모드를 낮추면 이 host의 판정도 바뀝니다.
    host.set_mode(review, SessionMode::ReadOnly);
    assert!(host.permits(review, Permission::Propose).is_err());
}

#[test]
fn external_clients_use_the_attached_catalog_and_unbound_is_refused() {
    let (_app, mut host) = host();
    let external = AgentConnection(3);
    assert_eq!(
        host.permits(external, Permission::Read),
        Err(HostRefusal::new(
            ResultKind::NotTrusted,
            "unbound-connection"
        ))
    );
    assert!(host.listed_tools(external).is_empty());
    host.bind(external, bound(None)).unwrap();
    assert_eq!(
        host.listed_tools(external),
        listed_tools(Backend::Attached, EnabledNamespaces::default()).collect::<Vec<_>>()
    );
    assert_eq!(
        host.callable_tool(external, VALIDATE_DOCUMENT.name),
        Ok(&VALIDATE_DOCUMENT)
    );
    assert_eq!(
        host.callable_tool(external, "terminal.run"),
        Err(HostRefusal::UNKNOWN_TOOL)
    );
    assert!(host.bind(external, bound(None)).is_err(), "결합은 고정");
    assert!(host.unbind(external).is_some());
}

#[test]
fn refusals_use_the_result_kinds() {
    assert_eq!(refusal(&AgentIoError::Busy).kind, ResultKind::Busy);
    assert_eq!(
        refusal(&AgentIoError::Stopped).kind,
        ResultKind::Unavailable
    );
    assert_eq!(
        refusal(&AgentIoError::UnknownWindow(WindowSessionHandle(9))).kind,
        ResultKind::NotFound
    );
    assert_eq!(
        refusal(&AgentIoError::Invalid(AttachedEditError::StaleBase {
            expected: stellar_ops::revision::ProjectRevision::Unsaved,
            found: stellar_ops::revision::ProjectRevision::Unsaved,
        }))
        .kind,
        ResultKind::Stale
    );
    assert_eq!(
        refusal(&AgentIoError::ConnectionNotApproved(
            stellar_agent_edit::permission::ConnectionApproval::Pending
        ))
        .kind,
        ResultKind::NotTrusted
    );
    assert_eq!(
        refusal(&AgentIoError::Edit(AgentEditError::Session(
            SessionError::NoEdit
        )))
        .kind,
        ResultKind::NotFound
    );
}

#[test]
fn frame_limit_matches_the_mcp_message_limit() {
    assert_eq!(
        stellar_local_ipc::frame::MAX_FRAME_BYTES,
        stellar_agent_protocol::limits::MAX_MESSAGE_BYTES
    );
}
