//! 창 session을 [`AppRuntime`] 경계로 여닫는 테스트(XII.2.2 이후 창 session 행 공용).

use std::path::Path;
use std::time::{Duration, Instant};

use stellar_core_types::ids::StableId;
use stellar_core_types::revision::DocumentRevision;

use crate::app::{
    AppRuntime, ProjectSessionId, RequestFailure, RequestId, RuntimeEvent, WindowSessionHandle,
};
use crate::window_session::test_fixtures::{saved_project, sid, unsaved_project};

/// 다음 완료 event를 최대 5초 기다립니다.
pub(crate) fn next_event(app: &AppRuntime) -> RuntimeEvent {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(event) = app.pop_event() {
            return event;
        }
        assert!(Instant::now() < deadline, "완료 event가 오지 않았습니다");
        std::thread::yield_now();
    }
}

pub(crate) fn open_project(app: &AppRuntime, root: &Path) -> ProjectSessionId {
    let request = app.open_project(root).unwrap();
    match next_event(app) {
        RuntimeEvent::ProjectOpened {
            request: got,
            project,
            ..
        } => {
            assert_eq!(got, request);
            project
        }
        other => panic!("프로젝트 열기 실패: {other:?}"),
    }
}

/// 창 session을 열고 (handle, 작업 문서 id, generation)을 돌려줍니다.
pub(crate) fn open_window(
    app: &AppRuntime,
    project: ProjectSessionId,
) -> (WindowSessionHandle, StableId, u64) {
    let request = app.open_window_session(project).unwrap();
    match next_event(app) {
        RuntimeEvent::WindowSessionOpened {
            request: got,
            project: owner,
            session,
            document_id,
            generation,
            ..
        } => {
            assert_eq!(got, request);
            assert_eq!(owner, project);
            (session, document_id, generation)
        }
        other => panic!("창 session 열기 실패: {other:?}"),
    }
}

pub(crate) fn close_window(app: &AppRuntime, session: WindowSessionHandle) {
    let request = app.close_window_session(session).unwrap();
    assert_eq!(
        next_event(app),
        RuntimeEvent::WindowSessionClosed { request, session }
    );
}

fn expect_failure(app: &AppRuntime, request: RequestId, expected: RequestFailure) {
    match next_event(app) {
        RuntimeEvent::RequestFailed {
            request: got,
            failure,
            message,
        } => {
            assert_eq!(got, request);
            assert_eq!(failure, expected, "{message}");
        }
        other => panic!("실패 event가 아님: {other:?}"),
    }
}

#[test]
fn two_windows_open_and_close_with_one_session_each() {
    // 검증 기준(XII.2.2, 코어 측): 창 2개 open/close lifecycle.
    let first = saved_project("app-window-a");
    let second = saved_project("app-window-b");
    let app = AppRuntime::start().unwrap();
    let project_a = open_project(&app, first.path());
    let project_b = open_project(&app, second.path());

    let (a, document_a, generation_a) = open_window(&app, project_a);
    let (b, document_b, generation_b) = open_window(&app, project_b);
    assert_ne!(a, b);
    assert_eq!(document_a, sid(1));
    assert_eq!(document_b, sid(1));
    assert_eq!((generation_a, generation_b), (1, 1));

    let state = app.state();
    assert_eq!(state.windows.len(), 2);
    let summary_a = state.window(a).unwrap();
    let summary_b = state.window(b).unwrap();
    assert_eq!(summary_a.project, project_a);
    assert_eq!(summary_b.project, project_b);
    assert_ne!(summary_a.window_id, summary_b.window_id, "창마다 다른 id");
    assert_eq!(summary_a.revision, DocumentRevision(0));
    assert_eq!(summary_a.undo_depth, 0);

    close_window(&app, a);
    let state = app.state();
    assert!(state.window(a).is_none());
    assert!(state.window(b).is_some(), "다른 창 session은 그대로");
    // 닫은 handle은 다시 쓸 수 없습니다.
    let request = app.close_window_session(a).unwrap();
    expect_failure(&app, request, RequestFailure::UnknownWindowSession);

    close_window(&app, b);
    assert!(app.state().windows.is_empty());

    // 같은 프로젝트에서 다시 열면 새 handle·새 창 id입니다(handle 재사용 없음).
    let (c, _, _) = open_window(&app, project_a);
    assert!(c.0 > b.0);
    assert_ne!(
        app.state().window(c).unwrap().window_id,
        summary_a.window_id
    );
    app.shutdown().unwrap();
}

#[test]
fn closing_a_project_closes_its_window_sessions() {
    let fixture = saved_project("app-window-project-close");
    let other = saved_project("app-window-other");
    let app = AppRuntime::start().unwrap();
    let project = open_project(&app, fixture.path());
    let keep = open_project(&app, other.path());
    let (first, _, _) = open_window(&app, project);
    let (second, _, _) = open_window(&app, project);
    let (kept, _, _) = open_window(&app, keep);
    let request = app.close_project(project).unwrap();
    assert!(matches!(
        next_event(&app),
        RuntimeEvent::ProjectClosed { request: got, .. } if got == request
    ));
    let state = app.state();
    assert!(state.window(first).is_none());
    assert!(state.window(second).is_none());
    assert!(state.window(kept).is_some());
    app.shutdown().unwrap();
}

#[test]
fn window_session_failures_have_fixed_codes() {
    let app = AppRuntime::start().unwrap();
    let request = app.open_window_session(ProjectSessionId(42)).unwrap();
    expect_failure(&app, request, RequestFailure::UnknownProject);

    // 저장한 적 없는 프로젝트는 작업 문서 없이(NIL, generation 0) 열립니다.
    let empty = unsaved_project("app-window-unsaved");
    let project = open_project(&app, empty.path());
    let (session, document, generation) = open_window(&app, project);
    assert_eq!(document, StableId::NIL);
    assert_eq!(generation, 0);
    assert_eq!(app.state().window(session).unwrap().document_id, None);

    // generation이 손상된 프로젝트는 창 session을 만들지 않습니다.
    let broken = saved_project("app-window-broken");
    std::fs::remove_dir_all(broken.path().join("design").join("generations")).unwrap();
    let project = open_project(&app, broken.path());
    let request = app.open_window_session(project).unwrap();
    expect_failure(&app, request, RequestFailure::WindowSessionLoadFailed);
    assert_eq!(app.state().windows.len(), 1);
    app.shutdown().unwrap();
}

#[test]
fn view_state_changes_stay_in_their_window() {
    // 검증 기준(XII.2.5, runtime 경계): 창간 상태 격리 — 탭·active view·viewport.
    use stellar_commands::view_tabs::CanvasViewport;

    let first = saved_project("app-view-a");
    let second = saved_project("app-view-b");
    let app = AppRuntime::start().unwrap();
    let (a, _, _) = open_window(&app, open_project(&app, first.path()));
    let (b, _, _) = open_window(&app, open_project(&app, second.path()));
    let state = app.state();
    for session in [a, b] {
        let summary = state.window(session).unwrap();
        assert_eq!(summary.tab_count, 1);
        assert_eq!(summary.active_view, Some(sid(3)));
        assert_eq!(summary.viewport, Some(CanvasViewport::DEFAULT));
    }

    let request = app.open_view_tab(a, sid(4)).unwrap();
    assert_eq!(
        next_event(&app),
        RuntimeEvent::WindowViewChanged {
            request,
            session: a
        }
    );
    let zoomed = CanvasViewport::new(64.0, 32.0, 2.0).unwrap();
    let request = app.set_viewport(a, zoomed).unwrap();
    assert_eq!(
        next_event(&app),
        RuntimeEvent::WindowViewChanged {
            request,
            session: a
        }
    );
    let state = app.state();
    let summary_a = state.window(a).unwrap();
    let summary_b = state.window(b).unwrap();
    assert_eq!(
        (summary_a.tab_count, summary_a.active_view),
        (2, Some(sid(4)))
    );
    assert_eq!(summary_a.viewport, Some(zoomed));
    assert_eq!(
        (summary_b.tab_count, summary_b.active_view),
        (1, Some(sid(3)))
    );
    assert_eq!(summary_b.viewport, Some(CanvasViewport::DEFAULT));
    // view state 변경은 문서 revision·history를 바꾸지 않습니다.
    assert_eq!(summary_a.revision, DocumentRevision(0));
    assert_eq!(summary_a.undo_depth, 0);

    // 실패: 없는 View, 닫힌 탭, 없는 창.
    let request = app.open_view_tab(b, sid(99)).unwrap();
    expect_failure(&app, request, RequestFailure::UnknownView);
    let request = app.close_view_tab(b, sid(4)).unwrap();
    expect_failure(&app, request, RequestFailure::UnknownView);
    let request = app.set_viewport(WindowSessionHandle(999), zoomed).unwrap();
    expect_failure(&app, request, RequestFailure::UnknownWindowSession);
    // 마지막 탭을 닫으면 active 탭이 없어 viewport를 바꿀 수 없습니다.
    let request = app.close_view_tab(b, sid(3)).unwrap();
    assert!(matches!(
        next_event(&app),
        RuntimeEvent::WindowViewChanged { request: got, .. } if got == request
    ));
    let request = app.set_viewport(b, zoomed).unwrap();
    expect_failure(&app, request, RequestFailure::NoActiveView);
    assert_eq!(app.state().window(a).unwrap().viewport, Some(zoomed));
    app.shutdown().unwrap();
}

/// 테스트 전용: 창 session 작업 문서에서 node 이름을 바꾸고 그 창 history에 기록합니다(셸 편집 경로가 생기기 전).
fn rename_in(app: &AppRuntime, session: WindowSessionHandle, command: u64, node: u64, name: &str) {
    let name = name.to_string();
    let request = app
        .submit(move |ctx, request| {
            let window = crate::app::window_mut(ctx, session)?;
            let tx = crate::window_session::test_fixtures::rename(
                window.document().unwrap(),
                command,
                node,
                &name,
            );
            window.apply(&tx).unwrap();
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
        .unwrap();
    assert!(matches!(
        next_event(app),
        RuntimeEvent::WindowViewChanged { request: got, .. } if got == request
    ));
}

fn replay(app: &AppRuntime, session: WindowSessionHandle, redo: bool) -> (bool, DocumentRevision) {
    let request = if redo {
        app.redo(session).unwrap()
    } else {
        app.undo(session).unwrap()
    };
    match next_event(app) {
        RuntimeEvent::HistoryReplayed {
            request: got,
            session: owner,
            redo: was_redo,
            applied,
            revision,
        } => {
            assert_eq!((got, owner, was_redo), (request, session, redo));
            (applied, revision)
        }
        other => panic!("undo·redo 완료 event가 아님: {other:?}"),
    }
}

fn save(app: &AppRuntime, session: WindowSessionHandle) -> (u64, bool) {
    let request = app.save_now(session).unwrap();
    match next_event(app) {
        RuntimeEvent::WindowSessionSaved {
            request: got,
            session: owner,
            generation,
            written,
        } => {
            assert_eq!((got, owner), (request, session));
            (generation, written)
        }
        other => panic!("저장 완료 event가 아님: {other:?}"),
    }
}

#[test]
fn undo_redo_and_save_now_reach_only_the_target_window() {
    // 검증 기준(XII.6.4): 창별 history와 save 호출 fixture.
    let first = saved_project("app-commands-a");
    let second = saved_project("app-commands-b");
    let app = AppRuntime::start().unwrap();
    let (a, _, _) = open_window(&app, open_project(&app, first.path()));
    let (b, _, _) = open_window(&app, open_project(&app, second.path()));

    // A: 선택 1개, B: 이름 변경 1개.
    let request = app.set_selection(a, vec![sid(11)]).unwrap();
    assert!(matches!(
        next_event(&app),
        RuntimeEvent::WindowViewChanged { request: got, session } if got == request && session == a
    ));
    rename_in(&app, b, 800, 12, "B 이름");
    let state = app.state();
    assert_eq!(
        (
            state.window(a).unwrap().undo_depth,
            state.window(a).unwrap().selection_len
        ),
        (1, 1)
    );
    assert_eq!(state.window(b).unwrap().undo_depth, 1);
    assert_eq!(state.window(b).unwrap().revision, DocumentRevision(1));

    // A undo는 A의 선택만 되돌립니다.
    assert_eq!(replay(&app, a, false), (true, DocumentRevision(0)));
    let state = app.state();
    assert_eq!(state.window(a).unwrap().selection_len, 0);
    assert_eq!(state.window(a).unwrap().redo_depth, 1);
    assert_eq!(state.window(b).unwrap().undo_depth, 1, "B history 그대로");
    assert_eq!(state.window(b).unwrap().revision, DocumentRevision(1));
    assert_eq!(replay(&app, a, false), (false, DocumentRevision(0)));
    assert_eq!(replay(&app, a, true), (true, DocumentRevision(0)));
    assert_eq!(app.state().window(a).unwrap().selection_len, 1);

    // Save Now: A는 문서 변경이 없어 쓰지 않고, B는 generation 2를 씁니다. A의 프로젝트 디스크는 그대로.
    let a_current = std::fs::read(first.path().join("design").join("current")).unwrap();
    assert_eq!(save(&app, a), (1, false));
    assert_eq!(
        std::fs::read(first.path().join("design").join("current")).unwrap(),
        a_current
    );
    assert_eq!(save(&app, b), (2, true));
    assert_eq!(app.state().window(b).unwrap().generation, 2);
    assert_eq!(app.state().window(a).unwrap().generation, 1);
    // B undo 후 다시 저장하면 generation 3.
    assert_eq!(replay(&app, b, false), (true, DocumentRevision(2)));
    assert_eq!(save(&app, b), (3, true));

    // 같은 프로젝트의 둘째 창(B2)은 loaded generation 3이고, B가 다시 저장하면 B2 저장은 Stale입니다.
    let project_b = app.state().window(b).unwrap().project;
    let (b2, _, generation) = open_window(&app, project_b);
    assert_eq!(generation, 3);
    rename_in(&app, b, 801, 11, "B 다시");
    assert_eq!(save(&app, b), (4, true));
    rename_in(&app, b2, 900, 11, "B2 편집");
    let request = app.save_now(b2).unwrap();
    expect_failure(&app, request, RequestFailure::SaveStale);
    assert_eq!(app.state().window(b2).unwrap().generation, 3);

    // 없는 창.
    let request = app.undo(WindowSessionHandle(999)).unwrap();
    expect_failure(&app, request, RequestFailure::UnknownWindowSession);
    app.shutdown().unwrap();
}
