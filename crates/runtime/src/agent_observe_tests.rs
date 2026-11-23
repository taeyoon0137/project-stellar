//! 사용자 편집·외부 변경 관찰 test (XIX.6.7, XIX.7.2, ADR-0124 결정 4·5).

use stellar_agent_edit::agent_history::{EndReason, SessionOrigin};
use stellar_agent_edit::attached::edit::agent_transaction;
use stellar_agent_edit::lease::{LeaseState, LeaseTargets};
use stellar_agent_edit::permission::AgentSessionId;
use stellar_agent_edit::provenance::{DisplayState, UserUiAction};
use stellar_commands::agent_batch::AgentBatchId;
use stellar_commands::command::Command;
use stellar_commands::lock::lease::{ExternalKind, ExternalResource, LeaseId};
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

/// 세션 시작과 전역 resource를 함께 잠근 node 12 lease(승인됨).
fn external_lease(window: &mut AgentWindow<'_>, externals: Vec<ExternalResource>) {
    window
        .start_session(session_id(), SessionOrigin::ExternalClient, "외부", 10)
        .unwrap();
    let targets = LeaseTargets {
        document: sid(1),
        targets: vec![sid(12)],
        externals,
    };
    window
        .submit_lease(session_id(), lease(), &targets)
        .unwrap();
    assert_eq!(
        window.approve_lease(lease(), ui()).unwrap(),
        LeaseState::Granted
    );
}

const TOKEN_SET: u64 = 3_000;
const TOKEN: u64 = 3_001;

/// 원본에 token set 하나(값 `px`)를 더한 token 파일을 디스크에 씁니다(외부 편집기·CLI가 token 파일만 씀).
fn write_token_file(f: &Fixture, px: f32) {
    use stellar_core_types::typed_value::TypedValue;
    use stellar_core_types::value::Length;
    use stellar_project::document_io::session::read_source;
    use stellar_project::document_io::source_v4::project::{decode, encode};
    use stellar_project::document_io::source_v4::token::{TokenSetSource, TokenSource};
    let files = read_source(f.project.path()).unwrap().unwrap().files;
    let mut text = decode(&files).unwrap();
    text.token_sets.retain(|set| set.id != sid(TOKEN_SET));
    text.token_sets.push(TokenSetSource {
        computed: Vec::new(),
        id: sid(TOKEN_SET),
        name: "Brand Space".into(),
        tokens: vec![TokenSource {
            id: sid(TOKEN),
            name: "space/4".into(),
            expression: sid(3_002),
            value: TypedValue::Length(Length::new(px).unwrap()),
        }],
    });
    let (path, content) = encode(&text)
        .unwrap()
        .files
        .into_iter()
        .find(|(path, _)| path.starts_with("tokens/"))
        .unwrap();
    let target = f.project.path().join("design").join(path);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(target, content).unwrap();
}

fn token(id: u64) -> ExternalResource {
    ExternalResource {
        kind: ExternalKind::Token,
        id: sid(id),
    }
}

fn sync(f: &mut Fixture, n: u64) -> ExternalSync {
    f.session
        .sync_external(stellar_commands::id::CommandId::from_stable_id(
            StableId::new_v7(n, 0x0f0f, n),
        ))
}

#[test]
fn library_only_change_of_a_locked_token_ends_the_session() {
    // 검증 기준(XIX.7.2, ADR-0132 결정 4): 문서 밖 의미 파일만 바뀌는 외부 변경(adopt_library_change)도 lease 전역
    // resource와 겹치면 세션을 LeaseConflict로 끝냅니다(자동 rollback 없음, lease 0).
    let mut f = fixture("agent-observe-library-hit");
    {
        let mut window = AgentWindow::new(&mut f.session, &mut f.agent);
        external_lease(&mut window, vec![token(TOKEN)]);
        drain(&mut window);
        assert!(window.session.agent_watch.external);
    }
    write_token_file(&f, 16.0);
    let revision = f.session.revision();
    assert!(
        matches!(
            sync(&mut f, 80),
            ExternalSync::Applied { changes: None, .. }
        ),
        "문서 밖 의미만 받음"
    );
    let mut window = AgentWindow::new(&mut f.session, &mut f.agent);
    let ended = drain(&mut window);
    assert_eq!(ended.len(), 1, "{ended:?}");
    assert_eq!(ended[0].reason, EndReason::LeaseConflict);
    assert!(!window.agent().sessions().is_active(session_id()));
    assert!(window.agent().lease_table().unwrap().is_empty(), "lease 0");
    assert_eq!(window.session().revision(), revision, "문서 그대로");
}

#[test]
fn library_change_outside_the_locked_resources_keeps_the_session() {
    // 잠그지 않은 token의 변경과, 잠금 없이 node만 쥔 lease는 세션을 유지합니다(XIX.7.3, XIX.4.4 자동 연쇄 잠금 없음).
    let mut f = fixture("agent-observe-library-apart");
    {
        let mut window = AgentWindow::new(&mut f.session, &mut f.agent);
        external_lease(&mut window, vec![token(4_444)]);
        drain(&mut window);
    }
    write_token_file(&f, 16.0);
    assert!(matches!(sync(&mut f, 81), ExternalSync::Applied { .. }));
    let mut window = AgentWindow::new(&mut f.session, &mut f.agent);
    assert!(drain(&mut window).is_empty());
    assert!(
        window.agent().sessions().is_active(session_id()),
        "세션 유지"
    );
    assert_eq!(window.agent().lease_table().unwrap().len(), 1, "lease 유지");
}

#[test]
fn later_library_changes_are_judged_against_the_last_seen_library() {
    // 기억한 의미는 비울 때마다 지금 값으로 바뀝니다. 잠근 token의 값만 바뀌어도(같은 파일, 같은 id) 겹침입니다.
    let mut f = fixture("agent-observe-library-later");
    write_token_file(&f, 16.0);
    assert!(matches!(sync(&mut f, 82), ExternalSync::Applied { .. }));
    {
        let mut window = AgentWindow::new(&mut f.session, &mut f.agent);
        external_lease(&mut window, vec![token(TOKEN)]);
        drain(&mut window);
        assert!(drain(&mut window).is_empty(), "바뀌지 않으면 그대로");
    }
    write_token_file(&f, 24.0);
    assert!(matches!(sync(&mut f, 83), ExternalSync::Applied { .. }));
    let mut window = AgentWindow::new(&mut f.session, &mut f.agent);
    let ended = drain(&mut window);
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].reason, EndReason::LeaseConflict);
}

#[test]
fn library_diff_counts_resources_by_stable_id() {
    use stellar_core_types::typed_value::TypedValue;
    use stellar_core_types::value::Length;
    use stellar_project::document_io::source_v4::token::{TokenSetSource, TokenSource};
    let token_source = |id: u64, px: f32| TokenSource {
        id: sid(id),
        name: format!("t{id}"),
        expression: sid(id + 100),
        value: TypedValue::Length(Length::new(px).unwrap()),
    };
    let library = |sets: Vec<TokenSetSource>| SourceLibrary {
        token_sets: sets,
        ..SourceLibrary::default()
    };
    let old = library(vec![TokenSetSource {
        computed: Vec::new(),
        id: sid(10),
        name: "a".into(),
        tokens: vec![token_source(11, 1.0), token_source(12, 2.0)],
    }]);
    // 12의 값 변경, 13 추가, 같은 set.
    let new = library(vec![TokenSetSource {
        computed: Vec::new(),
        id: sid(10),
        name: "a".into(),
        tokens: vec![
            token_source(11, 1.0),
            token_source(12, 3.0),
            token_source(13, 1.0),
        ],
    }]);
    assert_eq!(
        changed_resources(&old, &new),
        [token(10), token(12), token(13)].into_iter().collect(),
        "set 구성 변경은 set id, 바뀐 token은 그 id"
    );
    assert!(changed_resources(&old, &old.clone()).is_empty());
    let seen = |library: Option<SourceLibrary>| LibrarySeen {
        base: None,
        library: library.map(Arc::new),
    };
    let same = Arc::new(old.clone());
    assert_eq!(
        library_change(
            &LibrarySeen {
                base: None,
                library: Some(Arc::clone(&same))
            },
            &LibrarySeen {
                base: None,
                library: Some(same)
            }
        ),
        LibraryChange::Unchanged
    );
    assert!(matches!(
        library_change(&seen(Some(old)), &seen(Some(new))),
        LibraryChange::Resources(_)
    ));
    assert_eq!(
        library_change(&seen(None), &seen(None)),
        LibraryChange::Unchanged,
        "원본 hash도 없으면 판정할 변경이 없음"
    );
}

#[test]
fn unreadable_lease_table_ends_leased_sessions_fail_closed() {
    // 검증 기준(ADR-0132 결정 4): 외부 변경 겹침을 판정할 lease 표를 읽지 못하면 lease를 쥔 이 창 세션을 끝냅니다.
    let mut f = fixture("agent-observe-fail-closed");
    {
        let mut window = AgentWindow::new(&mut f.session, &mut f.agent);
        agent_batch(&mut window);
        drain(&mut window);
    }
    // 조정 디렉터리에 읽을 수 없는 lease 항목(디렉터리)을 두면 lease 표 읽기가 실패합니다.
    let coordinator = find_lease_dir(f._agent_locks.path()).expect("lease 조정 디렉터리");
    std::fs::create_dir_all(coordinator.join("lease-broken.txt")).unwrap();
    assert!(f.agent.lease_table().is_err(), "lease 표 읽기 실패");
    cli_rename(&f, 12, "외부");
    assert!(matches!(sync(&mut f, 84), ExternalSync::Applied { .. }));
    let mut window = AgentWindow::new(&mut f.session, &mut f.agent);
    let ended = drain(&mut window);
    assert_eq!(
        ended.len(),
        1,
        "겹치지 않는 변경이어도 판정할 수 없으면 끝냄"
    );
    assert_eq!(ended[0].reason, EndReason::LeaseConflict);
    assert!(!window.agent().sessions().is_active(session_id()));
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
        "자동 rollback 없음"
    );
}

/// `root` 아래에서 `lease-*.txt` 항목이 있는 디렉터리.
fn find_lease_dir(root: &std::path::Path) -> Option<std::path::PathBuf> {
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("lease-") && name.ends_with(".txt") {
            return Some(root.to_path_buf());
        }
        if path.is_dir() {
            if let Some(found) = find_lease_dir(&path) {
                return Some(found);
            }
        }
    }
    None
}
