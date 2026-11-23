//! 창 에이전트 편집 상태와 창 history 연결 test (XXIX.3.1 엔진 부분, ADR-0112).

use std::collections::BTreeSet;

use stellar_agent_edit::agent_history::{EndReason, SessionOrigin};
use stellar_agent_edit::attached::edit::agent_transaction;
use stellar_agent_edit::lease::{LeaseState, LeaseTargets};
use stellar_agent_edit::permission::{AgentSessionId, ConnectionApproval};
use stellar_agent_edit::provenance::{DisplayState, ProvenanceScope, UserUiAction};
use stellar_commands::agent_batch::AgentBatchId;
use stellar_commands::command::Command;
use stellar_commands::id::CommandId;
use stellar_commands::lock::lease::LeaseId;
use stellar_core_types::ids::StableId;

use super::*;
use crate::window_session::test_fixtures::{identity, rename, saved_project, sid, window, TempDir};
use crate::window_session::ProjectWindowSession;

const S: u64 = 700;
const L: u64 = 900;

fn ui() -> UserUiAction {
    UserUiAction::from_user_interface()
}

fn session_id() -> AgentSessionId {
    AgentSessionId(sid(S))
}

fn lease() -> LeaseId {
    LeaseId(sid(L))
}

fn id(n: u64) -> CommandId {
    CommandId::from_stable_id(StableId::new_v7(n, 0x0a0a, n))
}

fn no_definitions<'a>() -> Definitions<'a> {
    Definitions::Failed("정의 없음")
}

fn name(window: &AgentWindow<'_>, node: u64) -> String {
    window
        .session()
        .document()
        .unwrap()
        .model()
        .record(sid(node))
        .unwrap()
        .display_name
}

fn display(window: &AgentWindow<'_>, batch: AgentBatchId) -> DisplayState {
    window.agent().provenance().get(batch).unwrap().display()
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

/// 세션 시작, node 11 lease 승인, 이름 변경 제안 하나 적용까지.
fn applied_batch(window: &mut AgentWindow<'_>) -> AgentBatchId {
    assert_eq!(
        window
            .start_session(session_id(), SessionOrigin::ExternalClient, "외부", 10)
            .unwrap(),
        ConnectionApproval::Pending
    );
    let targets = LeaseTargets {
        document: sid(1),
        targets: vec![sid(11)],
        externals: Vec::new(),
    };
    assert_eq!(
        window
            .submit_lease(session_id(), lease(), &targets)
            .unwrap(),
        LeaseState::AwaitingApproval
    );
    assert!(
        window.agent().lease_table().unwrap().is_empty(),
        "승인 전에는 잠그지 않습니다"
    );
    assert_eq!(
        window.approve_lease(lease(), ui()).unwrap(),
        LeaseState::Granted
    );
    let state = window.session().document().unwrap();
    let transaction = agent_transaction(
        state.revision(),
        lease(),
        id(1),
        vec![Command::rename_node(sid(11), "에이전트".to_string())],
    );
    let proposal = window.queue_proposal(session_id(), transaction).unwrap();
    assert_eq!(
        name(window, 11),
        "n11",
        "승인 전 제안은 문서를 바꾸지 않습니다"
    );
    window.apply_proposal(session_id(), proposal, ui()).unwrap()
}

#[test]
fn completed_group_undo_redo_moves_provenance_and_names() {
    let mut f = fixture("agent-edit-undo");
    let mut window = AgentWindow::new(&mut f.session, &mut f.agent);
    let batch = applied_batch(&mut window);
    assert_eq!(name(&window, 11), "에이전트");
    let completed = window.complete(session_id(), id(2)).unwrap();
    assert_eq!(completed.batches, 1);
    assert!(
        window.agent().lease_table().unwrap().is_empty(),
        "완료 뒤 lease 해제"
    );
    assert_eq!(
        window.history_names(),
        HistoryNames {
            undo: Some("에이전트 편집"),
            redo: None
        }
    );
    // Undo: 그룹 항목 전체가 되돌아가고 표시는 실행 취소됨.
    assert!(window.replay(false).unwrap());
    assert_eq!(name(&window, 11), "n11");
    assert_eq!(display(&window, batch), DisplayState::Undone);
    assert_eq!(
        window.history_names(),
        HistoryNames {
            undo: None,
            redo: Some("에이전트 편집")
        }
    );
    // Redo: 이전 상태(검토 대기)로 돌아갑니다.
    assert!(window.replay(true).unwrap());
    assert_eq!(name(&window, 11), "에이전트");
    assert_eq!(display(&window, batch), DisplayState::PendingReview);
    assert!(!window.replay(true).unwrap(), "다시 적용할 항목이 없습니다");
}

#[test]
fn user_edit_undo_redo_is_observed_as_changed_later() {
    let mut f = fixture("agent-edit-observe");
    let mut window = AgentWindow::new(&mut f.session, &mut f.agent);
    let batch = applied_batch(&mut window);
    window.complete(session_id(), id(2)).unwrap();
    // 사용자가 같은 속성을 바꾸고 Undo·Redo합니다. 재생 change set이 observe에 들어갑니다.
    let edit = rename(window.session().document().unwrap(), 30, 11, "사용자");
    window.session.apply(&edit, 0).unwrap();
    assert!(window.replay(false).unwrap());
    assert_eq!(name(&window, 11), "에이전트");
    assert_eq!(display(&window, batch), DisplayState::PendingReview);
    assert!(window.replay(true).unwrap());
    assert_eq!(name(&window, 11), "사용자");
    assert_eq!(display(&window, batch), DisplayState::ChangedLater);
    assert!(window.replay(false).unwrap());
    assert_eq!(display(&window, batch), DisplayState::PendingReview);
}

#[test]
fn revert_is_one_history_entry_and_feeds_autosave() {
    let mut f = fixture("agent-edit-revert");
    let mut window = AgentWindow::new(&mut f.session, &mut f.agent);
    let batch = applied_batch(&mut window);
    window.complete(session_id(), id(2)).unwrap();
    let revision = window.session().revision();
    let preview = window
        .preview_revert(
            ProvenanceScope::Batch(batch),
            &BTreeSet::new(),
            no_definitions,
        )
        .unwrap();
    assert_eq!(
        window.session().revision(),
        revision,
        "미리보기는 문서를 바꾸지 않습니다"
    );
    let applied = window.apply_revert(&preview, id(3), ui()).unwrap();
    assert_eq!(name(&window, 11), "n11");
    assert_eq!(display(&window, batch), DisplayState::Reverted);
    assert_eq!(applied.changes.changed_nodes(), vec![sid(11)]);
    // 되돌리기는 일반 편집처럼 revision을 올려 저장 전 변경이 됩니다(자동 저장 예약 입력).
    assert!(window.session().revision() > revision);
    assert!(window.session().has_unsaved_changes());
    assert_eq!(window.history_names().undo, Some("에이전트 변경 되돌리기"));
    // 되돌리기 항목 Undo는 이전 표시 상태로 돌아갑니다.
    assert!(window.replay(false).unwrap());
    assert_eq!(name(&window, 11), "에이전트");
    assert_eq!(display(&window, batch), DisplayState::PendingReview);
    // 유지는 문서·history를 바꾸지 않고 표시만 지웁니다.
    let before = window.session().revision();
    assert_eq!(window.keep(ProvenanceScope::Batch(batch), ui()).unwrap(), 1);
    assert_eq!(window.session().revision(), before);
    assert_eq!(display(&window, batch), DisplayState::Kept);
}

#[test]
fn ending_and_closing_release_leases_and_permissions() {
    let mut f = fixture("agent-edit-close");
    let mut window = AgentWindow::new(&mut f.session, &mut f.agent);
    let batch = applied_batch(&mut window);
    assert_eq!(window.agent().lease_table().unwrap().len(), 1);
    // 연결 종료(외부 클라이언트): 종료 절차가 "중단됨" 항목 하나로 확정하고 lease·권한을 풉니다(rollback 없음).
    let report = window
        .on_bridge_disconnected(session_id(), id(4), 50)
        .unwrap()
        .unwrap();
    assert_eq!(report.reason, EndReason::McpDisconnected);
    assert_eq!(report.interrupted_batches, 1);
    assert_eq!(report.released_lease, Some(lease()));
    assert_eq!(report.diagnostic.duration_ms, 40);
    assert!(window.agent().lease_table().unwrap().is_empty());
    assert_eq!(name(&window, 11), "에이전트", "적용 변경은 남습니다");
    assert_eq!(display(&window, batch), DisplayState::PendingReview);
    assert_eq!(window.history_names().undo, Some("에이전트 편집(중단됨)"));
    let key = PermissionKey::new(
        window.session().identity().project_id(),
        window.session().identity().canonical_target().to_path_buf(),
        session_id(),
    );
    assert_eq!(window.agent().permissions().connection(&key), None);
    // 다른 세션이 lease를 쥔 채 창을 닫으면 그 세션도 끝나고 lease가 풀리며 출처 색인을 버립니다.
    let other = AgentSessionId(sid(701));
    window
        .start_session(other, SessionOrigin::AppStarted, "내장", 60)
        .unwrap();
    let targets = LeaseTargets {
        document: sid(1),
        targets: vec![sid(12)],
        externals: Vec::new(),
    };
    window
        .submit_lease(other, LeaseId(sid(901)), &targets)
        .unwrap();
    window.approve_lease(LeaseId(sid(901)), ui()).unwrap();
    assert_eq!(window.agent().lease_table().unwrap().len(), 1);
    let reports = window.close(70);
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].session, other);
    assert!(window.agent().lease_table().unwrap().is_empty());
    assert!(window.agent().provenance().is_empty());
    assert!(!window.agent().sessions().is_active(other));
}

#[test]
fn second_lease_request_in_one_session_is_refused_without_locking() {
    let mut f = fixture("agent-edit-second-lease");
    let mut window = AgentWindow::new(&mut f.session, &mut f.agent);
    applied_batch(&mut window);
    let targets = LeaseTargets {
        document: sid(1),
        targets: vec![sid(12)],
        externals: Vec::new(),
    };
    assert!(matches!(
        window.submit_lease(session_id(), LeaseId(sid(902)), &targets),
        Err(AgentEditError::Session(SessionError::EditInProgress(_)))
    ));
    assert_eq!(window.agent().lease_table().unwrap().len(), 1);
    // 모르는 세션은 lease를 요청할 수 없습니다.
    assert!(matches!(
        window.submit_lease(AgentSessionId(sid(799)), LeaseId(sid(903)), &targets),
        Err(AgentEditError::Session(SessionError::UnknownSession(_)))
    ));
}

#[test]
fn another_session_batch_on_the_same_property_is_changed_later() {
    // 첫 세션이 끝난 뒤 다른 세션이 같은 속성을 바꾸면 첫 batch는 "이후 변경됨"입니다(ADR-0100 결정 7).
    let mut f = fixture("agent-edit-other-session");
    let mut window = AgentWindow::new(&mut f.session, &mut f.agent);
    let first = applied_batch(&mut window);
    window.complete(session_id(), id(2)).unwrap();
    let other = AgentSessionId(sid(702));
    let other_lease = LeaseId(sid(904));
    window
        .start_session(other, SessionOrigin::ExternalClient, "둘째", 20)
        .unwrap();
    let targets = LeaseTargets {
        document: sid(1),
        targets: vec![sid(11)],
        externals: Vec::new(),
    };
    window.submit_lease(other, other_lease, &targets).unwrap();
    window.approve_lease(other_lease, ui()).unwrap();
    let state = window.session().document().unwrap();
    let transaction = agent_transaction(
        state.revision(),
        other_lease,
        id(5),
        vec![Command::rename_node(sid(11), "둘째".to_string())],
    );
    let proposal = window.queue_proposal(other, transaction).unwrap();
    let second = window.apply_proposal(other, proposal, ui()).unwrap();
    assert_eq!(display(&window, first), DisplayState::ChangedLater);
    assert_eq!(display(&window, second), DisplayState::PendingReview);
}

/// 같은 디스크 상태에서 headless snapshot(CLI·MCP)이 보는 프로젝트 revision.
fn headless_revision(root: &std::path::Path) -> ProjectRevision {
    stellar_ops::snapshot::open_snapshot(root)
        .unwrap()
        .revision()
}

/// 사용자 세트 파일 하나를 씁니다.
fn write_set(root: &std::path::Path, name: &str, text: &str) {
    let dir = root.join(stellar_project::manifest_style_defaults::STYLE_DEFAULTS_DIR);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{name}.stellar.yaml")), text).unwrap();
}

#[test]
fn window_revision_matches_the_headless_revision_for_each_style_defaults_choice() {
    // ADR-0135 결정 3: 창 revision과 headless revision은 같은 디스크 상태에서 같은 표기입니다. 기본 세트, 내장
    // 세트, 사용자 세트(파일 편집 포함) 모두 봅니다.
    let f = fixture("agent-edit-revision-css");
    let root = f.session.identity().canonical_target().to_path_buf();
    let css = project_revision(&f.session);
    assert_eq!(css, headless_revision(&root));
    assert_eq!(css.generation(), f.session.generation_number());
    assert_eq!(
        css.hash(),
        f.session.loaded_generation().map(|g| g.manifest_hash),
        "기본 세트 cache 상태는 manifest hash 그대로"
    );
    drop(f);

    let project = saved_project("agent-edit-revision-rn");
    stellar_project::manifest_style_defaults::set_active(project.path(), "react-native").unwrap();
    let session = ProjectWindowSession::load(identity(project.path()), window(2)).unwrap();
    assert_eq!(session.saved_style_defaults(), "react-native");
    let rn = project_revision(&session);
    assert_eq!(rn, headless_revision(project.path()));
    assert_ne!(
        rn.hash(),
        session.loaded_generation().map(|g| g.manifest_hash)
    );
    drop(session);

    let project = saved_project("agent-edit-revision-user");
    write_set(
        project.path(),
        "brand",
        "extends: react-native\nrow-gap: 4px\n",
    );
    stellar_project::manifest_style_defaults::set_active(project.path(), "brand").unwrap();
    let session = ProjectWindowSession::load(identity(project.path()), window(3)).unwrap();
    let first = project_revision(&session);
    assert_eq!(first, headless_revision(project.path()));
    // 세트 파일 값을 바꾸면 두 경로 모두 바뀌고 여전히 같습니다(낡은 base revision은 맞지 않음).
    write_set(
        project.path(),
        "brand",
        "extends: react-native\nrow-gap: 8px\n",
    );
    let edited = project_revision(&session);
    assert_ne!(edited, first);
    assert_eq!(edited, headless_revision(project.path()));
}

#[test]
fn window_revision_matches_the_headless_revision_while_the_cache_lags_the_source() {
    // ADR-0135 결정 3: 창이 외부 원본 변경을 반영하고 cache 맞춤 쓰기를 남긴 동안 headless는 원본에서 읽습니다
    // (FromText). 두 revision은 같은 text revision이고, 맞춤 쓰기 뒤에는 둘 다 새 cache generation입니다.
    let mut f = fixture("agent-edit-revision-text");
    let root = f.session.identity().canonical_target().to_path_buf();
    let opened = project_revision(&f.session);
    assert_eq!(opened, headless_revision(&root));

    let mut text =
        stellar_project::external_change::source::TextSourceSession::open(&root, window(77))
            .unwrap();
    let tx = rename(text.document(), 911, 11, "외부 원본");
    text.apply(&tx).unwrap();
    text.save().unwrap();
    assert!(matches!(
        f.session.sync_external(id(70)),
        crate::window_session::ExternalSync::Applied { .. }
    ));
    assert!(f.session.reconcile_pending());
    let lagging = project_revision(&f.session);
    assert_ne!(lagging, opened);
    assert_eq!(lagging, headless_revision(&root));
    assert_eq!(lagging.generation(), f.session.generation_number() + 1);

    assert!(f.session.save_now().unwrap().written);
    assert!(!f.session.reconcile_pending());
    let reconciled = project_revision(&f.session);
    assert_eq!(reconciled, headless_revision(&root));
    assert_eq!(
        reconciled.hash(),
        f.session.loaded_generation().map(|g| g.manifest_hash)
    );
}
