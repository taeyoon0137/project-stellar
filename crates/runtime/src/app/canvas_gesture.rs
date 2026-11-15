//! 캔버스 직접 조작(ADR-0071).
//!
//! 셸은 그린 행·화면 결과(포인터 아래 행, world delta, local 상자)만 보내고, 카메라·선택·편집의 의미와 command
//! 구성은 여기서 정합니다. 카메라 조작은 문서·history를 바꾸지 않습니다.

use super::*;

impl AppRuntime {
    /// 화면 중심에서 `anchor` point인 점을 고정하고 배율을 `factor`배 합니다(핀치, ⌘ 스크롤).
    ///
    /// 배율 상하한은 viewport가 정합니다. 문서·history·저장 상태는 바뀌지 않습니다.
    pub fn zoom_viewport_by(
        &self,
        session: WindowSessionHandle,
        anchor: [f64; 2],
        factor: f64,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            change_view_on_thread(ctx, request, session, |window| {
                window.zoom(factor, anchor[0], anchor[1]).map(|_| ())
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_window_tests::{next_event, open_project, open_window};
    use crate::window_session::test_fixtures::{sample_document, saved_project_with};

    #[test]
    fn continuous_zoom_keeps_anchor_and_leaves_document_and_history_alone() {
        let dir = saved_project_with("canvas-zoom-by", &[&sample_document()]);
        let app = AppRuntime::start().unwrap();
        let owner = open_project(&app, dir.path());
        let (session, _, _) = open_window(&app, owner);
        let before = app.state().window(session).unwrap().clone();
        let camera = before.viewport.unwrap();
        app.zoom_viewport_by(session, [40.0, -20.0], 1.25).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::WindowViewChanged { .. }
        ));
        let after = app.state().window(session).unwrap().clone();
        let zoomed = after.viewport.unwrap();
        assert_eq!(zoomed.zoom(), camera.zoom() * 1.25);
        // anchor 화면 점 아래의 문서 점이 그대로입니다.
        let world = |c: CanvasViewport, x: f64| c.center_x() + x / c.zoom();
        assert!((world(camera, 40.0) - world(zoomed, 40.0)).abs() < 1e-9);
        assert_eq!(
            (after.revision, after.undo_depth, after.has_unsaved_changes),
            (before.revision, before.undo_depth, before.has_unsaved_changes)
        );
        app.zoom_viewport_by(session, [0.0, 0.0], 1e9).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::WindowViewChanged { .. }
        ));
        assert_eq!(
            app.state().window(session).unwrap().viewport.unwrap().zoom(),
            stellar_commands::view_tabs::MAX_ZOOM
        );
        app.zoom_viewport_by(session, [0.0, 0.0], 0.0).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::RequestFailed { .. }
        ));
    }
}
