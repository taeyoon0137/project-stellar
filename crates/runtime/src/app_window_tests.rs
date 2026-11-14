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
