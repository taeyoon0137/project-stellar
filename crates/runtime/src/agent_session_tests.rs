//! 앱 시작 세션(agent-host AgentSession)과 창 편집 세션 연결 test (XXIX.3.1, ADR-0124 결정 1).

use stellar_agent_edit::agent_protocol::permission::Permission;
use stellar_agent_edit::attached::edit::agent_transaction;
use stellar_agent_edit::lease::{LeaseState, LeaseTargets};
use stellar_agent_edit::permission::{Confirmation, ConnectionApproval};
use stellar_agent_edit::provenance::UserUiAction;
use stellar_agent_host::session::{UserStartRequest, DEFAULT_RECONNECT_GRACE_MS};
use stellar_agent_host::start::FirstConnectionApproval;
use stellar_commands::command::Command;
use stellar_commands::lock::lease::LeaseId;

use super::*;
use crate::agent_edit::WindowAgentEdit;
use crate::window_session::test_fixtures::{identity, saved_project, sid, window, TempDir};
use crate::window_session::ProjectWindowSession;

fn ui() -> UserUiAction {
    UserUiAction::from_user_interface()
}

fn id(n: u64) -> CommandId {
    CommandId::from_stable_id(StableId::new_v7(n, 0x0b0b, n))
}

struct Fixture {
    _project: TempDir,
    _locks: TempDir,
    session: ProjectWindowSession,
    agent: WindowAgentEdit,
}

fn fixture(tag: &str) -> Fixture {
    let project = saved_project(tag);
    let locks = TempDir::new(&format!("{tag}-locks"));
    let session = ProjectWindowSession::load(identity(project.path()), window(1)).unwrap();
    let agent = WindowAgentEdit::with_lock_dir(locks.path().to_path_buf());
    Fixture {
        _project: project,
        _locks: locks,
        session,
        agent,
    }
}

fn started(session: &ProjectWindowSession, host: u64, mode: SessionMode) -> StartedSession {
    StartedSession {
        session: AgentSession::start_by_user(UserStartRequest {
            session: HostSessionId(host),
            mode,
            now_ms: 10,
            reconnect_grace_ms: DEFAULT_RECONNECT_GRACE_MS,
        }),
        connection: FirstConnectionApproval {
            project: project_key(session.identity()),
            window: session.window(),
            session: HostSessionId(host),
            profile_id: "test-double".to_string(),
            mode,
            approved_at_ms: 10,
        },
    }
}

/// node 11 lease 승인과 이름 변경 하나 적용.
fn edit(window: &mut AgentWindow<'_>, edit: AgentSessionId, lease: LeaseId) {
    let targets = LeaseTargets {
        document: sid(1),
        targets: vec![sid(11)],
        externals: Vec::new(),
    };
    assert_eq!(
        window.submit_lease(edit, lease, &targets).unwrap(),
        LeaseState::AwaitingApproval
    );
    assert_eq!(
        window.approve_lease(lease, ui()).unwrap(),
        LeaseState::Granted
    );
    let state = window.session().document().unwrap();
    let tx = agent_transaction(
        state.revision(),
        lease,
        id(1),
        vec![Command::rename_node(sid(11), "앱 세션".to_string())],
    );
    let proposal = window.queue_proposal(edit, tx).unwrap();
    window.apply_proposal(edit, proposal, ui()).unwrap();
}

#[test]
fn start_records_first_connection_and_maps_both_ids() {
    let mut f = fixture("agent-session-start");
    let mut window = AgentWindow::new(&mut f.session, &mut f.agent);
    let start = started(window.session(), 7, SessionMode::ApplyInScope);
    let edit = window.start_hosted(start, "내장", ui(), 10).unwrap();
    let hosted = window.agent().hosted();
    assert_eq!(hosted.edit_of(HostSessionId(7)), Some(edit));
    assert_eq!(hosted.host_of(edit), Some(HostSessionId(7)));
    assert_eq!(hosted.mode_of(edit), Some(SessionMode::ApplyInScope));
    let key = window.permission_key(edit);
    assert_eq!(
        window.agent().permissions().connection(&key),
        Some(ConnectionApproval::Approved),
        "시작 확인이 첫 연결 승인"
    );
    assert!(window.agent().sessions().is_active(edit));
    // 같은 agent-host 세션을 두 번 연결하지 않습니다.
    let again = started(window.session(), 7, SessionMode::ApplyInScope);
    assert_eq!(
        window.start_hosted(again, "내장", ui(), 11),
        Err(AgentEditError::Link(LinkError::DuplicateHost(
            HostSessionId(7)
        )))
    );
    assert_ne!(new_edit_session_id(), new_edit_session_id());
}

#[test]
fn start_for_another_project_or_window_changes_nothing() {
    let mut f = fixture("agent-session-mismatch");
    let other = saved_project("agent-session-mismatch-other");
    let other_session = ProjectWindowSession::load(identity(other.path()), window(1)).unwrap();
    let mut window = AgentWindow::new(&mut f.session, &mut f.agent);
    let foreign = started(&other_session, 1, SessionMode::default());
    assert_eq!(
        window.start_hosted(foreign, "x", ui(), 10),
        Err(AgentEditError::Link(LinkError::ProjectMismatch))
    );
    let mut wrong_window = started(window.session(), 2, SessionMode::default());
    wrong_window.connection.window = crate::window_session::test_fixtures::window(2);
    assert_eq!(
        window.start_hosted(wrong_window, "x", ui(), 10),
        Err(AgentEditError::Link(LinkError::WindowMismatch))
    );
    assert_eq!(window.agent().sessions().active_len(), 0);
    assert!(window.agent().hosted().is_empty());
}

#[test]
fn process_exit_runs_the_session_end_procedure_once() {
    let mut f = fixture("agent-session-exit");
    let mut window = AgentWindow::new(&mut f.session, &mut f.agent);
    let start = started(window.session(), 3, SessionMode::ApplyInScope);
    let edit_id = window.start_hosted(start, "내장", ui(), 10).unwrap();
    window
        .host_event(HostSessionId(3), SessionEvent::BridgeBound, id(9), 11)
        .unwrap();
    edit(&mut window, edit_id, LeaseId(sid(900)));
    let revision = window.session().revision();
    let report = window
        .host_event(HostSessionId(3), SessionEvent::ProcessExited, id(10), 20)
        .unwrap();
    let ended = report.ended.expect("종료 절차");
    assert_eq!(ended.reason, EndReason::ProcessExited);
    assert_eq!(ended.interrupted_batches, 1, "중단됨 항목 하나");
    assert_eq!(ended.released_lease, Some(LeaseId(sid(900))));
    assert!(window.agent().lease_table().unwrap().is_empty(), "lease 0");
    assert!(!window.agent().sessions().is_active(edit_id));
    assert_eq!(
        window
            .agent()
            .permissions()
            .connection(&window.permission_key(edit_id)),
        None,
        "권한 폐기"
    );
    assert_eq!(window.session().revision(), revision, "자동 rollback 없음");
    let forwarded: Vec<&str> = window
        .take_host_effects()
        .iter()
        .map(|(_, e)| e.kind())
        .collect();
    assert_eq!(
        forwarded,
        ["close-ui", "end-binding", "cleanup", "diagnostic"]
    );
    // 끝난 세션은 event를 받지 않고, 종료 절차를 다시 실행하지 않습니다.
    assert!(window
        .host_event(HostSessionId(3), SessionEvent::UserStop, id(11), 21)
        .is_err());
}

#[test]
fn edit_completed_and_mode_lowering_reach_the_session_manager() {
    let mut f = fixture("agent-session-complete");
    let mut window = AgentWindow::new(&mut f.session, &mut f.agent);
    let start = started(window.session(), 4, SessionMode::ApplyInScope);
    let edit_id = window.start_hosted(start, "내장", ui(), 10).unwrap();
    window
        .host_event(HostSessionId(4), SessionEvent::BridgeBound, id(9), 11)
        .unwrap();
    window
        .host_event(
            HostSessionId(4),
            SessionEvent::LeaseGranted(stellar_agent_host::session::LeaseId(1)),
            id(9),
            11,
        )
        .unwrap();
    edit(&mut window, edit_id, LeaseId(sid(901)));
    let report = window
        .host_event(HostSessionId(4), SessionEvent::EditCompleted, id(12), 30)
        .unwrap();
    assert_eq!(report.completed.map(|c| c.batches), Some(1));
    assert!(window.agent().lease_table().unwrap().is_empty());
    assert!(window.agent().sessions().is_active(edit_id), "세션은 유지");

    // 사용자가 모드를 낮추면 상한 밖이 된 Apply 확인만 버립니다.
    let key = window.permission_key(edit_id);
    let Confirmation::Pending(request) = window
        .agent
        .permissions
        .request(&key, Permission::Apply)
        .unwrap()
    else {
        panic!("대기 요청");
    };
    window
        .agent
        .permissions
        .respond(&key, request, true)
        .unwrap();
    window
        .host_event(
            HostSessionId(4),
            SessionEvent::PermissionGranted(Permission::Apply),
            id(9),
            31,
        )
        .unwrap();
    let lowered = window
        .change_hosted_mode(HostSessionId(4), SessionMode::ReviewThenApply, ui())
        .unwrap();
    assert_eq!(lowered.revoked, 1);
    assert_eq!(
        window
            .agent()
            .permissions()
            .confirmation(&key, Permission::Apply),
        Confirmation::NotRequested
    );
}

#[test]
fn closing_the_window_stops_hosted_sessions_with_process_termination() {
    let mut f = fixture("agent-session-close");
    let mut window = AgentWindow::new(&mut f.session, &mut f.agent);
    let start = started(window.session(), 5, SessionMode::default());
    let edit_id = window.start_hosted(start, "내장", ui(), 10).unwrap();
    let reports = window.close(40);
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].session, edit_id);
    assert_eq!(reports[0].reason, EndReason::UserStopped);
    let kinds: Vec<&str> = window
        .take_host_effects()
        .iter()
        .map(|(_, e)| e.kind())
        .collect();
    assert!(kinds.contains(&"terminate"), "{kinds:?}");
}

#[test]
fn reasons_map_with_the_same_diagnostic_codes() {
    for reason in [
        HostEndReason::UserStopped,
        HostEndReason::ProcessExited,
        HostEndReason::ReconnectGraceExpired,
        HostEndReason::TrustRevoked,
        HostEndReason::LeaseConflict,
        HostEndReason::UserReleasedLease,
        HostEndReason::UsageLimitReported,
        HostEndReason::AuthErrorReported,
    ] {
        assert_eq!(end_reason(reason).code(), reason.code());
    }
}
