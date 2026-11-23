//! 사용자 편집·외부 변경 관찰 test (XIX.6.7, XIX.7.2, ADR-0124 결정 4·5).

use stellar_agent_edit::agent_history::{EndReason, SessionOrigin};
use stellar_agent_edit::attached::edit::agent_transaction;
use stellar_agent_edit::lease::{LeaseState, LeaseTargets};
use stellar_agent_edit::permission::AgentSessionId;
use stellar_agent_edit::provenance::{DisplayState, UserUiAction};
use stellar_commands::agent_batch::AgentBatchId;
use stellar_commands::command::Command;
use stellar_commands::lock::lease::LeaseId;
use stellar_core_types::ids::StableId;
use stellar_ops::apply::{apply, ApplyMode, ApplyRequest, CommandInput};
use stellar_ops::lock::LockPolicy;
use stellar_ops::snapshot::open_snapshot;

use super::*;
use crate::window_session::test_fixtures::{identity, rename, saved_project, sid, window, TempDir};
use crate::window_session::{ExternalSync, ProjectWindowSession};

fn ui() -> UserUiAction {
    UserUiAction::from_user_interface()
}

fn session_id() -> AgentSessionId {
    AgentSessionId(sid(700))
}

fn lease() -> LeaseId {
    LeaseId(sid(900))
}

struct Fixture {
    project: TempDir,
    _agent_locks: TempDir,
    cli_locks: TempDir,
    session: ProjectWindowSession,
    agent: WindowAgentEdit,
}

fn fixture(tag: &str) -> Fixture {
    let project = saved_project(tag);
    let agent_locks = TempDir::new(&format!("{tag}-agent-locks"));
    let cli_locks = TempDir::new(&format!("{tag}-cli-locks"));
    let session = ProjectWindowSession::load(identity(project.path()), window(1)).unwrap();
    let agent = WindowAgentEdit::with_lock_dir(agent_locks.path().to_path_buf());
    Fixture {
        project,
        _agent_locks: agent_locks,
        cli_locks,
        session,
        agent,
    }
}

/// 세션 시작, node 11 lease, 이름 변경 batch 하나 적용.
fn agent_batch(window: &mut AgentWindow<'_>) -> AgentBatchId {
    window
        .start_session(session_id(), SessionOrigin::ExternalClient, "외부", 10)
        .unwrap();
    let targets = LeaseTargets {
        document: sid(1),
        targets: vec![sid(11)],
        externals: Vec::new(),
    };
    window
        .submit_lease(session_id(), lease(), &targets)
        .unwrap();
    assert_eq!(
        window.approve_lease(lease(), ui()).unwrap(),
        LeaseState::Granted
    );
    let state = window.session().document().unwrap();
    let tx = agent_transaction(
        state.revision(),
        lease(),
        stellar_commands::id::CommandId::from_stable_id(StableId::new_v7(1, 0x0d0d, 1)),
        vec![Command::rename_node(sid(11), "에이전트".to_string())],
    );
    let proposal = window.queue_proposal(session_id(), tx).unwrap();
    window.apply_proposal(session_id(), proposal, ui()).unwrap()
}

fn display(window: &AgentWindow<'_>, batch: AgentBatchId) -> DisplayState {
    window.agent().provenance().get(batch).unwrap().display()
}

fn user_rename(window: &mut AgentWindow<'_>, command: u64, node: u64, name: &str) {
    let tx = rename(window.session().document().unwrap(), command, node, name);
    window.session.apply(&tx, 0).unwrap();
}

/// `stellar apply`(headless)로 node 이름을 바꿉니다(외부 writer).
fn cli_rename(f: &Fixture, node: u64, name: &str) {
    let request = ApplyRequest {
        base_revision: open_snapshot(f.project.path()).unwrap().revision(),
        document: sid(1),
        commands: vec![CommandInput::Rename {
            node: sid(node),
            name: name.to_string(),
        }],
    };
    apply(
        f.project.path(),
        &request,
        ApplyMode::Write,
        &LockPolicy::Required {
            dir: f.cli_locks.path().to_path_buf(),
        },
    )
    .unwrap();
}

#[test]
fn user_edits_reach_the_provenance_index_at_state_publication() {
    let mut f = fixture("agent-observe-user");
    let mut window = AgentWindow::new(&mut f.session, &mut f.agent);
    // 표시된 batch가 없으면 쌓지 않습니다.
    drain(&mut window);
    user_rename(&mut window, 50, 12, "먼저");
    assert_eq!(window.session.agent_watch.len(), 0, "관찰 비용 0");
    let batch = agent_batch(&mut window);
    drain(&mut window);
    assert!(
        window.session.agent_watch.changes,
        "표시 batch가 있으면 켜짐"
    );
    // 사용자가 표시된 속성을 바꾸면 다음 게시 앞에서 "이후 변경됨"입니다.
    user_rename(&mut window, 51, 11, "사용자");
    assert_eq!(
        display(&window, batch),
        DisplayState::PendingReview,
        "게시 전"
    );
    drain(&mut window);
    assert_eq!(display(&window, batch), DisplayState::ChangedLater);
    // 적용 직후 값으로 되돌리면 다시 검토 대기입니다.
    user_rename(&mut window, 52, 11, "에이전트");
    drain(&mut window);
    assert_eq!(display(&window, batch), DisplayState::PendingReview);
    // 표시 batch와 무관한 node의 편집은 표시를 바꾸지 않습니다.
    user_rename(&mut window, 53, 12, "무관");
    drain(&mut window);
    assert_eq!(display(&window, batch), DisplayState::PendingReview);
}

#[test]
fn history_preview_is_observed_once_at_the_end() {
    let mut f = fixture("agent-observe-preview");
    let mut window = AgentWindow::new(&mut f.session, &mut f.agent);
    let batch = agent_batch(&mut window);
    drain(&mut window);
    let mut last = None;
    for (step, name) in ["a", "b", "c"].into_iter().enumerate() {
        let session = &mut *window.session;
        let document = session.document.as_mut().unwrap();
        if step > 0 {
            session.history.cancel_preview(&mut document.state).unwrap();
        }
        session.history.begin_scrub(7).unwrap();
        let tx = rename(&document.state, 60 + step as u64, 11, name);
        let changes = session
            .history
            .update_preview(&mut document.state, &tx)
            .unwrap();
        last = Some((changes, tx.origin));
        assert_eq!(session.agent_watch.len(), 0, "preview 중에는 쌓지 않음");
    }
    let (changes, origin) = last.unwrap();
    window
        .session
        .end_preview(0, Some((&changes, origin)))
        .unwrap();
    assert_eq!(window.session.agent_watch.len(), 1, "끝에서 한 번");
    drain(&mut window);
    assert_eq!(display(&window, batch), DisplayState::ChangedLater);
}

#[test]
fn overlapping_external_change_ends_the_session_and_keeps_agent_work() {
    let mut f = fixture("agent-observe-external-overlap");
    let batch = {
        let mut window = AgentWindow::new(&mut f.session, &mut f.agent);
        let batch = agent_batch(&mut window);
        drain(&mut window);
        assert!(
            window.session.agent_watch.external,
            "lease가 있으면 외부 관찰 켜짐"
        );
        batch
    };
    cli_rename(&f, 11, "외부");
    let revision = f.session.revision();
    let sync = f
        .session
        .sync_external(stellar_commands::id::CommandId::from_stable_id(
            StableId::new_v7(70, 0x0e0e, 70),
        ));
    // 창과 외부가 같은 node를 바꿔 reconciliation은 두 선택지로 멈춥니다(semantic conflict 경로).
    assert!(matches!(sync, ExternalSync::Stale { .. }), "{sync:?}");
    let mut window = AgentWindow::new(&mut f.session, &mut f.agent);
    let ended = drain(&mut window);
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].reason, EndReason::LeaseConflict);
    assert!(window.agent().lease_table().unwrap().is_empty(), "lease 0");
    assert!(!window.agent().sessions().is_active(session_id()));
    assert_eq!(window.session().revision(), revision, "자동 rollback 없음");
    assert_eq!(
        window
            .session()
            .document()
            .unwrap()
            .model()
            .record(sid(11))
            .unwrap()
            .display_name,
        "에이전트",
        "에이전트 변경 자동 삭제 0"
    );
    assert!(window
        .agent()
        .provenance()
        .get(batch)
        .unwrap()
        .display()
        .is_shown());
    assert!(!window.session.agent_watch.external, "lease가 없으면 꺼짐");
}

#[test]
fn non_overlapping_external_change_keeps_the_session() {
    let mut f = fixture("agent-observe-external-apart");
    let batch = {
        let mut window = AgentWindow::new(&mut f.session, &mut f.agent);
        let batch = agent_batch(&mut window);
        drain(&mut window);
        batch
    };
    cli_rename(&f, 12, "외부");
    let sync = f
        .session
        .sync_external(stellar_commands::id::CommandId::from_stable_id(
            StableId::new_v7(71, 0x0e0e, 71),
        ));
    assert!(matches!(sync, ExternalSync::Applied { .. }), "{sync:?}");
    let mut window = AgentWindow::new(&mut f.session, &mut f.agent);
    assert!(drain(&mut window).is_empty());
    assert!(
        window.agent().sessions().is_active(session_id()),
        "세션 유지"
    );
    assert_eq!(window.agent().lease_table().unwrap().len(), 1, "lease 유지");
    assert_eq!(display(&window, batch), DisplayState::PendingReview);
}
