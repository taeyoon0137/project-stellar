//! 캔버스 도구 상호작용 상태 기계 테스트 (XII.3.6).

use std::f32::consts::{FRAC_PI_2, PI};

use super::*;
use crate::fixtures::{id, p};
use crate::transient::{ControlPointId, HandleMetrics, TransientLayer};

/// 비교 허용 오차(초월 함수 결과).
const EPS: f32 = 1e-4;

fn camera(scale: f32, tx: f32, ty: f32) -> Transform2D {
    Transform2D::new(scale, 0.0, 0.0, scale, tx, ty).unwrap()
}

fn quad(x0: f32, y0: f32, x1: f32, y1: f32) -> [Vec2; 4] {
    [p(x0, y0), p(x1, y0), p(x1, y1), p(x0, y1)]
}

/// world (100,100)~(300,200) 선택 상자. camera는 2배 확대 + (10, 20) 이동.
const SELECTION: [u64; 2] = [1, 2];

fn selection() -> Vec<StableId> {
    SELECTION.iter().map(|&n| id(n)).collect()
}

fn context(tool: InteractionTool, target: PressTarget, ids: &[StableId]) -> PressContext<'_> {
    PressContext {
        tool,
        camera: camera(2.0, 10.0, 20.0),
        target,
        selection: ids,
        selection_box: Some(quad(100.0, 100.0, 300.0, 200.0)),
    }
}

/// world 점의 view 좌표(camera 2배 + (10, 20)).
fn view(x: f32, y: f32) -> Vec2 {
    p(2.0 * x + 10.0, 2.0 * y + 20.0)
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= EPS * b.abs().max(1.0)
}

fn assert_point(actual: Vec2, expected: Vec2, label: &str) {
    assert!(
        close(actual.x(), expected.x()) && close(actual.y(), expected.y()),
        "{label}: {actual:?} != {expected:?}"
    );
}

fn assert_transform(actual: Transform2D, expected: Transform2D, label: &str) {
    let (a, e) = (actual.components(), expected.components());
    let pairs = [
        (a.0, e.0),
        (a.1, e.1),
        (a.2, e.2),
        (a.3, e.3),
        (a.4, e.4),
        (a.5, e.5),
    ];
    assert!(
        pairs.iter().all(|&(x, y)| close(x, y)),
        "{label}: {a:?} != {e:?}"
    );
}

/// gesture 하나의 fixture: 누름 context·점, 이동 점들, 뗌 점, 기대 결과.
struct Case {
    name: &'static str,
    tool: InteractionTool,
    target: PressTarget,
    down: Vec2,
    moves: Vec<Vec2>,
    up: Vec2,
    kind: InteractionKind,
    /// 마지막 이동 뒤 preview.
    preview: InteractionPreview,
    /// 뗌 결과.
    outcome: InteractionOutcome,
}

fn handle(kind: HandleKind) -> PressTarget {
    PressTarget::Handle(kind)
}

/// 여섯 상호작용 각각의 진행 → 뗌 fixture.
fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "pan: hand로 view (50,60)만큼 이동",
            tool: InteractionTool::Hand,
            target: PressTarget::Node(id(1)),
            down: p(400.0, 300.0),
            moves: vec![p(410.0, 310.0), p(450.0, 360.0)],
            up: p(450.0, 360.0),
            kind: InteractionKind::Pan,
            preview: InteractionPreview::Camera {
                kind: InteractionKind::Pan,
                camera: camera(2.0, 60.0, 80.0),
            },
            outcome: InteractionOutcome::Pan {
                camera: camera(2.0, 60.0, 80.0),
            },
        },
        Case {
            name: "zoom: 오른쪽 200px 끌기 = 누른 점 기준 2배",
            tool: InteractionTool::Zoom,
            target: PressTarget::Empty,
            down: p(100.0, 100.0),
            moves: vec![p(150.0, 100.0), p(300.0, 120.0)],
            up: p(300.0, 120.0),
            kind: InteractionKind::Zoom,
            // view x' = 2(2x + 10) − 100 = 4x − 80, y' = 2(2y + 20) − 100 = 4y − 60.
            preview: InteractionPreview::Camera {
                kind: InteractionKind::Zoom,
                camera: camera(4.0, -80.0, -60.0),
            },
            outcome: InteractionOutcome::Zoom {
                camera: camera(4.0, -80.0, -60.0),
            },
        },
        Case {
            name: "select: 빈 곳에서 영역 선택",
            tool: InteractionTool::Move,
            target: PressTarget::Empty,
            down: view(50.0, 40.0),
            moves: vec![view(60.0, 60.0), view(20.0, 90.0)],
            up: view(10.0, 80.0),
            kind: InteractionKind::Select,
            preview: InteractionPreview::Marquee {
                rect: Aabb::new(20.0, 40.0, 50.0, 90.0).unwrap(),
            },
            outcome: InteractionOutcome::Marquee {
                rect: Aabb::new(10.0, 40.0, 50.0, 80.0).unwrap(),
            },
        },
        Case {
            name: "drag: 선택된 node를 world (30,-15) 이동",
            tool: InteractionTool::Move,
            target: PressTarget::Node(id(2)),
            down: view(150.0, 150.0),
            moves: vec![view(160.0, 150.0), view(170.0, 140.0)],
            up: view(180.0, 135.0),
            kind: InteractionKind::Drag,
            preview: InteractionPreview::Transform {
                kind: InteractionKind::Drag,
                transform: camera(1.0, 20.0, -10.0),
            },
            outcome: InteractionOutcome::Drag {
                targets: selection(),
                delta: p(30.0, -15.0),
                transform: camera(1.0, 30.0, -15.0),
            },
        },
        Case {
            name: "resize: 오른쪽 아래 모서리를 (+100,+50) = 왼쪽 위 고정 1.5배",
            tool: InteractionTool::Move,
            target: handle(HandleKind::ResizeCorner(Corner::BottomRight)),
            down: view(300.0, 200.0),
            moves: vec![view(350.0, 220.0)],
            up: view(400.0, 250.0),
            kind: InteractionKind::Resize,
            // x' = 100 + 1.25(x − 100), y' = 100 + 1.2(y − 100).
            preview: InteractionPreview::Transform {
                kind: InteractionKind::Resize,
                transform: Transform2D::new(1.25, 0.0, 0.0, 1.2, -25.0, -20.0).unwrap(),
            },
            outcome: InteractionOutcome::Resize {
                targets: selection(),
                handle: ResizeHandle::Corner(Corner::BottomRight),
                transform: Transform2D::new(1.5, 0.0, 0.0, 1.5, -50.0, -50.0).unwrap(),
                quad: quad(100.0, 100.0, 400.0, 250.0),
            },
        },
        Case {
            name: "rotate: 상자 중심(200,150) 기준 오른쪽 위 → 오른쪽 아래 90°",
            tool: InteractionTool::Move,
            target: handle(HandleKind::Rotate(Corner::TopRight)),
            down: view(250.0, 150.0),
            moves: vec![view(240.0, 180.0)],
            up: view(200.0, 200.0),
            kind: InteractionKind::Rotate,
            // (240,180) − (200,150) = (40, 30)의 각도.
            preview: InteractionPreview::Transform {
                kind: InteractionKind::Rotate,
                transform: rotation(p(200.0, 150.0), 30f32.atan2(40.0)).unwrap(),
            },
            outcome: InteractionOutcome::Rotate {
                targets: selection(),
                center: p(200.0, 150.0),
                angle: FRAC_PI_2,
                transform: Transform2D::new(0.0, 1.0, -1.0, 0.0, 350.0, -50.0).unwrap(),
            },
        },
    ]
}

fn assert_preview(actual: InteractionPreview, expected: InteractionPreview, label: &str) {
    match (actual, expected) {
        (
            InteractionPreview::Camera { kind, camera },
            InteractionPreview::Camera {
                kind: expected_kind,
                camera: expected_camera,
            },
        ) => {
            assert_eq!(kind, expected_kind, "{label}");
            assert_transform(camera, expected_camera, label);
        }
        (
            InteractionPreview::Transform { kind, transform },
            InteractionPreview::Transform {
                kind: expected_kind,
                transform: expected_transform,
            },
        ) => {
            assert_eq!(kind, expected_kind, "{label}");
            assert_transform(transform, expected_transform, label);
        }
        (actual, expected) => assert_eq!(actual, expected, "{label}"),
    }
}

fn assert_outcome(actual: &InteractionOutcome, expected: &InteractionOutcome, label: &str) {
    match (actual, expected) {
        (InteractionOutcome::Pan { camera: a }, InteractionOutcome::Pan { camera: e })
        | (InteractionOutcome::Zoom { camera: a }, InteractionOutcome::Zoom { camera: e }) => {
            assert_transform(*a, *e, label)
        }
        (
            InteractionOutcome::Drag {
                targets,
                delta,
                transform,
            },
            InteractionOutcome::Drag {
                targets: et,
                delta: ed,
                transform: ex,
            },
        ) => {
            assert_eq!(targets, et, "{label}");
            assert_point(*delta, *ed, label);
            assert_transform(*transform, *ex, label);
        }
        (
            InteractionOutcome::Resize {
                targets,
                handle,
                transform,
                quad,
            },
            InteractionOutcome::Resize {
                targets: et,
                handle: eh,
                transform: ex,
                quad: eq,
            },
        ) => {
            assert_eq!((targets, handle), (et, eh), "{label}");
            assert_transform(*transform, *ex, label);
            for (a, e) in quad.iter().zip(eq) {
                assert_point(*a, *e, label);
            }
        }
        (
            InteractionOutcome::Rotate {
                targets,
                center,
                angle,
                transform,
            },
            InteractionOutcome::Rotate {
                targets: et,
                center: ec,
                angle: ea,
                transform: ex,
            },
        ) => {
            assert_eq!(targets, et, "{label}");
            assert_point(*center, *ec, label);
            assert!(close(*angle, *ea), "{label}: angle {angle} != {ea}");
            assert_transform(*transform, *ex, label);
        }
        (actual, expected) => assert_eq!(actual, expected, "{label}"),
    }
}

/// fixture를 누름 → 이동까지 진행하고 마지막 preview를 검사합니다.
fn run_until_last_move(case: &Case, ids: &[StableId]) -> InteractionController {
    let mut controller = InteractionController::default();
    let ctx = context(case.tool, case.target, ids);
    assert_eq!(
        controller.pointer_down(case.down, &ctx).unwrap(),
        InteractionStep::Started(case.kind),
        "{}: 시작",
        case.name
    );
    let mut last = None;
    for &point in &case.moves {
        match controller.pointer_move(point).unwrap() {
            InteractionStep::Updated(preview) => last = Some(preview),
            other => panic!("{}: 이동 결과 {other:?}", case.name),
        }
    }
    assert_eq!(
        controller.kind(),
        Some(case.kind),
        "{}: 진행 종류",
        case.name
    );
    assert_preview(last.expect("이동 있음"), case.preview, case.name);
    assert_eq!(controller.preview(), last, "{}: 상태 preview", case.name);
    controller
}

#[test]
fn complete_fixture_passes() {
    // 검증 기준(XII.3.6): 여섯 상호작용 각각 누름 → 이동 → 뗌이 정해진 결과 하나를 내고 Idle로 돌아갑니다.
    let ids = selection();
    let cases = cases();
    assert_eq!(cases.len(), 6);
    for case in &cases {
        let mut controller = run_until_last_move(case, &ids);
        let step = controller.pointer_up(case.up);
        let InteractionStep::Completed(outcome) = step else {
            panic!("{}: 뗌 결과 {step:?}", case.name);
        };
        assert_outcome(&outcome, &case.outcome, case.name);
        assert_eq!(controller.state(), &InteractionState::Idle, "{}", case.name);
        assert_eq!(controller.preview(), None);
        // 끝난 뒤의 이동·뗌·취소는 무시합니다.
        assert_eq!(
            controller.pointer_move(case.up),
            Ok(InteractionStep::Ignored)
        );
        assert_eq!(controller.pointer_up(case.up), InteractionStep::Ignored);
        assert_eq!(controller.cancel(), InteractionStep::Ignored);
    }
}

#[test]
fn cancel_fixture_passes() {
    // 검증 기준(XII.3.6): 같은 fixture를 이동 뒤 취소하면 결과 없이 Idle이고, pan·zoom은 시작 camera를
    // 되돌릴 값으로 줍니다. 취소 뒤 뗌은 결과를 만들지 않습니다.
    let ids = selection();
    for case in &cases() {
        let mut controller = run_until_last_move(case, &ids);
        let restore = matches!(case.kind, InteractionKind::Pan | InteractionKind::Zoom)
            .then(|| camera(2.0, 10.0, 20.0));
        assert_eq!(
            controller.cancel(),
            InteractionStep::Cancelled(Cancellation {
                kind: case.kind,
                restore_camera: restore,
                failure: None,
            }),
            "{}",
            case.name
        );
        assert_eq!(controller.state(), &InteractionState::Idle);
        assert_eq!(controller.targets(), &[] as &[StableId]);
        assert_eq!(
            controller.pointer_up(case.up),
            InteractionStep::Ignored,
            "{}: 취소 뒤 뗌",
            case.name
        );
        // 취소 뒤 새 gesture를 시작할 수 있습니다.
        let ctx = context(case.tool, case.target, &ids);
        assert!(matches!(
            controller.pointer_down(case.down, &ctx),
            Ok(InteractionStep::Started(_))
        ));
    }
}

#[test]
fn pressed_cancel_and_click_without_moves() {
    // 누른 직후 취소도 결과가 없고, 이동 없이 떼면 클릭 결과입니다.
    let ids = selection();
    let expected_clicks = [
        (
            InteractionTool::Hand,
            PressTarget::Empty,
            InteractionOutcome::Unchanged {
                kind: InteractionKind::Pan,
            },
        ),
        (
            InteractionTool::Move,
            PressTarget::Empty,
            InteractionOutcome::Click { target: None },
        ),
        (
            InteractionTool::Move,
            PressTarget::Node(id(7)),
            InteractionOutcome::Click {
                target: Some(id(7)),
            },
        ),
        (
            InteractionTool::Move,
            PressTarget::Node(id(2)),
            InteractionOutcome::Click {
                target: Some(id(2)),
            },
        ),
        (
            InteractionTool::Move,
            handle(HandleKind::ResizeEdge(Edge::Left)),
            InteractionOutcome::Unchanged {
                kind: InteractionKind::Resize,
            },
        ),
        (
            InteractionTool::Move,
            handle(HandleKind::Rotate(Corner::BottomLeft)),
            InteractionOutcome::Unchanged {
                kind: InteractionKind::Rotate,
            },
        ),
    ];
    for (tool, target, expected) in expected_clicks {
        let ctx = context(tool, target, &ids);
        let mut controller = InteractionController::default();
        controller.pointer_down(p(200.0, 200.0), &ctx).unwrap();
        assert!(matches!(controller.cancel(), InteractionStep::Cancelled(_)));
        controller.pointer_down(p(200.0, 200.0), &ctx).unwrap();
        // threshold(3px) 안의 이동은 진행하지 않습니다.
        assert_eq!(
            controller.pointer_move(p(202.0, 202.0)).unwrap(),
            InteractionStep::Updated(InteractionPreview::Pending {
                kind: controller.kind().unwrap()
            })
        );
        assert_eq!(
            controller.pointer_up(p(201.0, 199.0)),
            InteractionStep::Completed(expected.clone()),
            "{tool:?} {target:?}"
        );
    }
}

#[test]
fn select_on_unselected_node_hands_off_to_drag_of_that_node() {
    let ids = selection();
    let ctx = context(InteractionTool::Move, PressTarget::Node(id(9)), &ids);
    let mut controller = InteractionController::default();
    assert_eq!(
        controller.pointer_down(view(0.0, 0.0), &ctx).unwrap(),
        InteractionStep::Started(InteractionKind::Select)
    );
    assert_eq!(controller.kind(), Some(InteractionKind::Select));
    // threshold를 넘으면 그 node 하나의 Drag로 바뀝니다(누른 점 기준 전체 이동량).
    let step = controller.pointer_move(view(5.0, 0.0)).unwrap();
    assert_eq!(controller.kind(), Some(InteractionKind::Drag));
    assert_eq!(controller.targets(), &[id(9)]);
    let InteractionStep::Updated(InteractionPreview::Transform { kind, transform }) = step else {
        panic!("drag preview가 아님: {step:?}");
    };
    assert_eq!(kind, InteractionKind::Drag);
    assert_transform(transform, camera(1.0, 5.0, 0.0), "handoff");
    let InteractionStep::Completed(outcome) = controller.pointer_up(view(5.0, 5.0)) else {
        panic!("완료 아님");
    };
    assert_outcome(
        &outcome,
        &InteractionOutcome::Drag {
            targets: vec![id(9)],
            delta: p(5.0, 5.0),
            transform: camera(1.0, 5.0, 5.0),
        },
        "handoff 완료",
    );
}

#[test]
fn up_beyond_threshold_without_moves_uses_the_up_point() {
    // 이동 이벤트가 병합돼 오지 않아도 뗀 위치로 마지막 이동을 반영합니다.
    let ids = selection();
    let ctx = context(InteractionTool::Move, PressTarget::Node(id(1)), &ids);
    let mut controller = InteractionController::default();
    controller.pointer_down(view(0.0, 0.0), &ctx).unwrap();
    let InteractionStep::Completed(InteractionOutcome::Drag { delta, .. }) =
        controller.pointer_up(view(-8.0, 4.0))
    else {
        panic!("drag 완료 아님");
    };
    assert_point(delta, p(-8.0, 4.0), "up delta");
}

#[test]
fn zoom_click_scrub_left_and_clamp() {
    let ids = selection();
    let ctx = context(InteractionTool::Zoom, PressTarget::Empty, &ids);
    // 클릭: 누른 점 기준 2배.
    let mut controller = InteractionController::default();
    controller.pointer_down(p(100.0, 100.0), &ctx).unwrap();
    let InteractionStep::Completed(InteractionOutcome::Zoom { camera: clicked }) =
        controller.pointer_up(p(100.0, 100.0))
    else {
        panic!("zoom 클릭 아님");
    };
    assert_transform(clicked, camera(4.0, -80.0, -60.0), "click");
    // 누른 view 점은 zoom 전후 같은 world 점을 가리킵니다.
    let before = camera(2.0, 10.0, 20.0)
        .inverse()
        .unwrap()
        .apply(p(100.0, 100.0));
    let after = clicked.inverse().unwrap().apply(p(100.0, 100.0));
    assert_point(after.unwrap(), before.unwrap(), "anchor");

    // 왼쪽 200px = 1/2배.
    controller.pointer_down(p(100.0, 100.0), &ctx).unwrap();
    controller.pointer_move(p(-100.0, 100.0)).unwrap();
    let InteractionStep::Completed(InteractionOutcome::Zoom { camera: out }) =
        controller.pointer_up(p(-100.0, 100.0))
    else {
        panic!("zoom 아님");
    };
    assert_transform(out, camera(1.0, 55.0, 60.0), "zoom out");

    // 아주 멀리 끌어도 배율은 [min, max]로 고정됩니다.
    for (x, expected) in [
        (1.0e6, InteractionConfig::DEFAULT_MAX_ZOOM),
        (-1.0e6, InteractionConfig::DEFAULT_MIN_ZOOM),
    ] {
        controller.pointer_down(p(100.0, 100.0), &ctx).unwrap();
        controller.pointer_move(p(x, 100.0)).unwrap();
        let Some(InteractionPreview::Camera { camera, .. }) = controller.preview() else {
            panic!("camera preview 아님");
        };
        assert!(close(camera.determinant().abs().sqrt(), expected), "{x}");
        controller.cancel();
    }
}

#[test]
fn resize_edges_rotated_box_and_flip() {
    let ids = selection();
    let start = |target, selection_box| PressContext {
        tool: InteractionTool::Move,
        camera: Transform2D::identity(),
        target,
        selection: &ids,
        selection_box,
    };
    let finish = |controller: &mut InteractionController, to: Vec2| match controller.pointer_up(to)
    {
        InteractionStep::Completed(InteractionOutcome::Resize { quad, .. }) => quad,
        other => panic!("resize 완료 아님: {other:?}"),
    };
    let axis_box = Some(quad(100.0, 100.0, 300.0, 200.0));

    // 위 변을 30 올림: 아래 변 고정, 높이만 130.
    let mut controller = InteractionController::default();
    controller
        .pointer_down(
            p(200.0, 100.0),
            &start(handle(HandleKind::ResizeEdge(Edge::Top)), axis_box),
        )
        .unwrap();
    let out = finish(&mut controller, p(260.0, 70.0));
    for (a, e) in out.iter().zip(quad(100.0, 70.0, 300.0, 200.0).iter()) {
        assert_point(*a, *e, "top edge");
    }

    // 왼쪽 변을 오른쪽 변 너머(350)로 끌면 뒤집힙니다: 오른쪽 변 고정, x 배율 −0.25.
    controller
        .pointer_down(
            p(100.0, 150.0),
            &start(handle(HandleKind::ResizeEdge(Edge::Left)), axis_box),
        )
        .unwrap();
    let out = finish(&mut controller, p(350.0, 150.0));
    assert_point(out[0], p(350.0, 100.0), "flip top_left");
    assert_point(out[1], p(300.0, 100.0), "flip top_right");

    // 90° 회전한 상자(top_left가 오른쪽 위에 있는 상자)의 bottom_right 모서리: 상자 축으로 계산합니다.
    // 상자 축: u = top_left→top_right = (0, 200), v = top_left→bottom_left = (−100, 0).
    let rotated = Some([
        p(300.0, 100.0),
        p(300.0, 300.0),
        p(200.0, 300.0),
        p(200.0, 100.0),
    ]);
    controller
        .pointer_down(
            p(200.0, 300.0),
            &start(
                handle(HandleKind::ResizeCorner(Corner::BottomRight)),
                rotated,
            ),
        )
        .unwrap();
    // world (−50, +100) = 상자 축 (s +0.5, t +0.5).
    let out = finish(&mut controller, p(150.0, 400.0));
    for (a, e) in out.iter().zip(
        [
            p(300.0, 100.0),
            p(300.0, 400.0),
            p(150.0, 400.0),
            p(150.0, 100.0),
        ]
        .iter(),
    ) {
        assert_point(*a, *e, "rotated corner");
    }
}

#[test]
fn rotation_accumulates_past_half_turn() {
    let ids = selection();
    let ctx = PressContext {
        tool: InteractionTool::Move,
        camera: Transform2D::identity(),
        target: handle(HandleKind::Rotate(Corner::TopRight)),
        selection: &ids,
        selection_box: Some(quad(-10.0, -10.0, 10.0, 10.0)),
    };
    let mut controller = InteractionController::default();
    controller.pointer_down(p(100.0, 0.0), &ctx).unwrap();
    // 시계 방향으로 90°씩 세 번(아래 → 왼쪽 → 위) = 270°. 접으면 −90°가 되지만 이어 붙입니다.
    for point in [p(0.0, 100.0), p(-100.0, 0.0), p(0.0, -100.0)] {
        controller.pointer_move(point).unwrap();
    }
    let InteractionStep::Completed(InteractionOutcome::Rotate { angle, center, .. }) =
        controller.pointer_up(p(0.0, -100.0))
    else {
        panic!("rotate 완료 아님");
    };
    assert_point(center, p(0.0, 0.0), "center");
    assert!(close(angle, 1.5 * PI), "angle {angle}");
}

#[test]
fn degenerate_boxes_resize_only_the_axis_with_length() {
    // 수평·수직 선(Line 도구)의 선택 상자는 면적이 0입니다. 길이가 있는 축만 배율이 바뀌고, 길이 0인 축은
    // 배율 1로 고정됩니다.
    let ids = selection();
    let start = |kind, selection_box| PressContext {
        tool: InteractionTool::Move,
        camera: Transform2D::identity(),
        target: handle(kind),
        selection: &ids,
        selection_box: Some(selection_box),
    };
    let resize = |kind, selection_box: [Vec2; 4], down: Vec2, up: Vec2| {
        let mut controller = InteractionController::default();
        assert_eq!(
            controller.pointer_down(down, &start(kind, selection_box)),
            Ok(InteractionStep::Started(InteractionKind::Resize)),
            "{kind:?}"
        );
        match controller.pointer_up(up) {
            InteractionStep::Completed(InteractionOutcome::Resize {
                transform, quad, ..
            }) => (transform, quad),
            other => panic!("{kind:?}: resize 완료 아님 {other:?}"),
        }
    };
    let horizontal = quad(100.0, 150.0, 300.0, 150.0);

    // 수평 선의 오른쪽 변을 100 늘림: 왼쪽 끝 고정, x 배율 1.5, y 배율 1.
    let (transform, out) = resize(
        HandleKind::ResizeEdge(Edge::Right),
        horizontal,
        p(300.0, 150.0),
        p(400.0, 170.0),
    );
    assert_transform(
        transform,
        Transform2D::new(1.5, 0.0, 0.0, 1.0, -50.0, 0.0).unwrap(),
        "right edge",
    );
    for (a, e) in out.iter().zip(quad(100.0, 150.0, 400.0, 150.0).iter()) {
        assert_point(*a, *e, "right edge quad");
    }

    // 모서리는 두 축을 바꾸지만 길이 0인 세로 축은 고정됩니다(세로 이동 40은 무시).
    let (transform, out) = resize(
        HandleKind::ResizeCorner(Corner::BottomRight),
        horizontal,
        p(300.0, 150.0),
        p(350.0, 190.0),
    );
    assert!(close(transform.components().3, 1.0), "y 배율 고정");
    for (a, e) in out.iter().zip(quad(100.0, 150.0, 350.0, 150.0).iter()) {
        assert_point(*a, *e, "corner quad");
    }

    // 수직 선의 위 변을 50 올림: 아래 끝 고정, y 배율 1.5, x 배율 1.
    let vertical = quad(80.0, 100.0, 80.0, 200.0);
    let (transform, out) = resize(
        HandleKind::ResizeEdge(Edge::Top),
        vertical,
        p(80.0, 100.0),
        p(60.0, 50.0),
    );
    assert!(close(transform.components().0, 1.0), "x 배율 고정");
    for (a, e) in out.iter().zip(quad(80.0, 50.0, 80.0, 200.0).iter()) {
        assert_point(*a, *e, "top edge quad");
    }

    // 45° 기울어진 선(회전한 Line의 상자): 상자 축 방향으로만 늘어납니다.
    let diagonal = [p(0.0, 0.0), p(100.0, 100.0), p(100.0, 100.0), p(0.0, 0.0)];
    let (_, out) = resize(
        HandleKind::ResizeEdge(Edge::Right),
        diagonal,
        p(100.0, 100.0),
        p(150.0, 150.0),
    );
    assert_point(out[0], p(0.0, 0.0), "diagonal start");
    assert_point(out[1], p(150.0, 150.0), "diagonal end");
    assert_point(out[3], p(0.0, 0.0), "diagonal bottom_left");
}

#[test]
fn every_layer_resize_handle_can_start_a_resize() {
    // 선택 상자 handle layer(XI.3.5)가 만든 크기 조절 handle은 모두 Resize를 시작하고, 만들지 않은
    // handle은 길이 0인 축만 바꾸는 handle이라 거부됩니다. 둘의 판정 기준은 `ResizeAxes` 하나입니다.
    let ids = selection();
    let boxes = [
        ("rect", quad(100.0, 100.0, 300.0, 200.0)),
        ("horizontal", quad(100.0, 150.0, 300.0, 150.0)),
        ("vertical", quad(80.0, 100.0, 80.0, 200.0)),
        ("point", [p(5.0, 5.0); 4]),
        (
            "parallel axes",
            [p(0.0, 0.0), p(10.0, 10.0), p(20.0, 20.0), p(10.0, 10.0)],
        ),
    ];
    let all_resize: Vec<HandleKind> = Corner::ALL
        .into_iter()
        .map(HandleKind::ResizeCorner)
        .chain(Edge::ALL.into_iter().map(HandleKind::ResizeEdge))
        .collect();
    for (name, selection_box) in boxes {
        let mut layer = TransientLayer::new();
        layer.push_selection_box(
            id(1),
            selection_box,
            HandleMetrics::new(4.0, 3.0, 16.0).unwrap(),
        );
        let in_layer: Vec<HandleKind> = layer.handles().iter().map(|h| h.kind).collect();
        for kind in &all_resize {
            let ctx = PressContext {
                tool: InteractionTool::Move,
                camera: Transform2D::identity(),
                target: handle(*kind),
                selection: &ids,
                selection_box: Some(selection_box),
            };
            let mut controller = InteractionController::default();
            let step = controller.pointer_down(selection_box[0], &ctx);
            if in_layer.contains(kind) {
                assert_eq!(
                    step,
                    Ok(InteractionStep::Started(InteractionKind::Resize)),
                    "{name} {kind:?}"
                );
            } else {
                assert_eq!(
                    step,
                    Err(InteractionError::DegenerateSelectionBox),
                    "{name} {kind:?}"
                );
            }
        }
    }
}

#[test]
fn invalid_presses_leave_the_state_unchanged() {
    let ids = selection();
    let mut controller = InteractionController::default();
    // modifier control point는 이 기계 밖입니다.
    let control = context(
        InteractionTool::Move,
        handle(HandleKind::ModifierControl(ControlPointId(3))),
        &ids,
    );
    assert_eq!(
        controller.pointer_down(p(0.0, 0.0), &control),
        Ok(InteractionStep::Ignored)
    );
    assert!(!controller.is_active());
    // 되돌릴 수 없는 camera.
    let mut flat = context(InteractionTool::Hand, PressTarget::Empty, &ids);
    flat.camera = Transform2D::new(0.0, 0.0, 0.0, 0.0, 1.0, 1.0).unwrap();
    assert_eq!(
        controller.pointer_down(p(0.0, 0.0), &flat),
        Err(InteractionError::CameraNotInvertible)
    );
    // handle인데 선택 상자가 없음.
    let mut no_box = context(
        InteractionTool::Move,
        handle(HandleKind::Rotate(Corner::TopLeft)),
        &ids,
    );
    no_box.selection_box = None;
    assert_eq!(
        controller.pointer_down(p(0.0, 0.0), &no_box),
        Err(InteractionError::MissingSelectionBox)
    );
    // 길이 0인 축만 바꾸는 handle(수평 선의 위 변)과 점 상자의 모서리는 크기 조절할 수 없습니다.
    for (kind, selection_box) in [
        (
            HandleKind::ResizeEdge(Edge::Top),
            quad(0.0, 0.0, 100.0, 0.0),
        ),
        (HandleKind::ResizeCorner(Corner::TopLeft), [p(5.0, 5.0); 4]),
    ] {
        let mut degenerate = context(InteractionTool::Move, handle(kind), &ids);
        degenerate.selection_box = Some(selection_box);
        assert_eq!(
            controller.pointer_down(p(0.0, 0.0), &degenerate),
            Err(InteractionError::DegenerateSelectionBox),
            "{kind:?}"
        );
    }
    assert_eq!(controller.state(), &InteractionState::Idle);
    // 진행 중 다시 누르면 거부하고 진행 상태를 유지합니다.
    let pan = context(InteractionTool::Hand, PressTarget::Empty, &ids);
    controller.pointer_down(p(0.0, 0.0), &pan).unwrap();
    controller.pointer_move(p(10.0, 0.0)).unwrap();
    let before = controller.state().clone();
    assert_eq!(
        controller.pointer_down(p(5.0, 5.0), &pan),
        Err(InteractionError::AlreadyActive)
    );
    assert_eq!(controller.state(), &before);
}

#[test]
fn non_finite_moves_keep_the_previous_state() {
    // camera 배율이 아주 작으면 view 이동의 world 값이 f32 범위를 넘습니다.
    let ids = selection();
    let mut ctx = context(InteractionTool::Move, PressTarget::Node(id(1)), &ids);
    ctx.camera = Transform2D::new(1.0e-15, 0.0, 0.0, 1.0e-15, 0.0, 0.0).unwrap();
    let mut controller = InteractionController::default();
    controller.pointer_down(p(0.0, 0.0), &ctx).unwrap();
    assert_eq!(
        controller.pointer_move(p(1.0, 0.0)),
        Ok(InteractionStep::Updated(InteractionPreview::Pending {
            kind: InteractionKind::Drag
        }))
    );
    let before = controller.state().clone();
    assert_eq!(
        controller.pointer_move(p(3.0e38, 3.0e38)),
        Err(InteractionError::NonFinite)
    );
    assert_eq!(controller.state(), &before);
}

#[test]
fn failed_up_ends_the_gesture_as_a_cancellation() {
    // 뗌은 운영체제 pointer가 이미 올라간 뒤입니다. 뗀 위치를 계산할 수 없어도 gesture를 끝내야 다음
    // 누름이 AlreadyActive로 막히지 않습니다. 호출자가 cancel()을 따로 부르지 않습니다.
    let ids = selection();
    // Drag: camera 배율이 아주 작아 뗀 위치의 world 값이 f32 범위를 넘습니다.
    let tiny = Transform2D::new(1.0e-15, 0.0, 0.0, 1.0e-15, 0.0, 0.0).unwrap();
    // Pan: 누른 점과 뗀 점의 view 차이가 f32 범위를 넘습니다.
    let identity = Transform2D::identity();
    let cases = [
        (
            InteractionTool::Move,
            PressTarget::Node(id(1)),
            tiny,
            [p(0.0, 0.0), p(10.0, 0.0), p(3.0e38, 3.0e38)],
            InteractionKind::Drag,
            None,
        ),
        (
            InteractionTool::Hand,
            PressTarget::Empty,
            identity,
            [p(-3.0e38, 0.0), p(-2.9e38, 0.0), p(3.0e38, 0.0)],
            InteractionKind::Pan,
            Some(identity),
        ),
    ];
    for (tool, target, camera, [down, moved, up], kind, restore) in cases {
        let mut ctx = context(tool, target, &ids);
        ctx.camera = camera;
        let mut controller = InteractionController::default();
        controller.pointer_down(down, &ctx).unwrap();
        // 진행 단계로 들어간 뒤(threshold 밖) 뗀 위치가 비유한 결과를 냅니다.
        assert!(matches!(
            controller.pointer_move(moved),
            Ok(InteractionStep::Updated(
                InteractionPreview::Transform { .. } | InteractionPreview::Camera { .. }
            ))
        ));
        assert_eq!(
            controller.pointer_up(up),
            InteractionStep::Cancelled(Cancellation {
                kind,
                restore_camera: restore,
                failure: Some(InteractionError::NonFinite),
            }),
            "{kind:?}"
        );
        assert_eq!(controller.state(), &InteractionState::Idle, "{kind:?}");
        assert_eq!(controller.preview(), None);
        // 끝났으므로 뒤늦은 이동·뗌·취소는 무시하고, 새 gesture를 바로 시작할 수 있습니다.
        assert_eq!(
            controller.pointer_move(p(1.0, 1.0)),
            Ok(InteractionStep::Ignored)
        );
        assert_eq!(controller.pointer_up(p(1.0, 1.0)), InteractionStep::Ignored);
        assert_eq!(controller.cancel(), InteractionStep::Ignored);
        assert!(matches!(
            controller.pointer_down(down, &ctx),
            Ok(InteractionStep::Started(_))
        ));
    }
}

#[test]
fn press_target_comes_from_canvas_hit() {
    // XI.3.5 canvas hit → 누름 대상: handle 종류, node 원본 id, 빈 곳.
    let mut layer = TransientLayer::new();
    layer.push_selection_box(
        id(1),
        quad(100.0, 100.0, 300.0, 200.0),
        HandleMetrics::new(4.0, 3.0, 16.0).unwrap(),
    );
    let viewport = crate::visibility::Viewport::new(
        stellar_core_types::value::Size2::new(1000.0, 1000.0).unwrap(),
        Transform2D::identity(),
    )
    .unwrap();
    let hit = layer.hit(&viewport, p(300.0, 200.0)).expect("모서리");
    assert_eq!(
        PressTarget::from_hit(&CanvasHit::Handle(hit)),
        handle(HandleKind::ResizeCorner(Corner::BottomRight))
    );
    assert_eq!(PressTarget::from_hit(&CanvasHit::Empty), PressTarget::Empty);
}

#[test]
fn config_rejects_invalid_values() {
    let ok = InteractionConfig::new(0.0, 1.0, 0.5, 1.0, 1.0).unwrap();
    assert_eq!(ok.drag_threshold(), 0.0);
    assert_eq!(ok.click_zoom_factor(), 0.5);
    assert!(InteractionConfig::new(-1.0, 1.0, 1.0, 1.0, 1.0).is_err());
    assert!(InteractionConfig::new(1.0, 0.0, 1.0, 1.0, 1.0).is_err());
    assert!(InteractionConfig::new(1.0, 1.0, 0.0, 1.0, 1.0).is_err());
    assert!(InteractionConfig::new(1.0, 1.0, 1.0, 2.0, 1.0).is_err());
    assert!(InteractionConfig::new(f32::NAN, 1.0, 1.0, 1.0, 1.0).is_err());
    let default = InteractionConfig::default();
    assert_eq!(
        (
            default.drag_threshold(),
            default.zoom_drag_per_doubling(),
            default.min_zoom(),
            default.max_zoom()
        ),
        (3.0, 200.0, 0.02, 256.0)
    );
}

#[test]
fn every_state_has_a_kind_and_preview() {
    // 닫힌 enum: 모든 진행 상태는 종류와 preview를 가집니다(새 변형을 넣으면 이 match가 컴파일 실패).
    fn label(state: &InteractionState) -> &'static str {
        match state {
            InteractionState::Idle => "idle",
            InteractionState::Pan(PanState::Pressed { .. }) => "pan-pressed",
            InteractionState::Pan(PanState::Panning { .. }) => "panning",
            InteractionState::Zoom(ZoomState::Pressed { .. }) => "zoom-pressed",
            InteractionState::Zoom(ZoomState::Scrubbing { .. }) => "scrubbing",
            InteractionState::Select(SelectState::Pressed { .. }) => "select-pressed",
            InteractionState::Select(SelectState::Marquee { .. }) => "marquee",
            InteractionState::Drag(DragState::Pressed { .. }) => "drag-pressed",
            InteractionState::Drag(DragState::Dragging { .. }) => "dragging",
            InteractionState::Resize(ResizeState::Pressed { .. }) => "resize-pressed",
            InteractionState::Resize(ResizeState::Resizing { .. }) => "resizing",
            InteractionState::Rotate(RotateState::Pressed { .. }) => "rotate-pressed",
            InteractionState::Rotate(RotateState::Rotating { .. }) => "rotating",
        }
    }
    let ids = selection();
    let mut seen = Vec::new();
    for case in &cases() {
        let mut controller = InteractionController::default();
        controller
            .pointer_down(case.down, &context(case.tool, case.target, &ids))
            .unwrap();
        seen.push(label(controller.state()));
        assert!(controller.preview().is_some());
        controller
            .pointer_move(*case.moves.last().unwrap())
            .unwrap();
        seen.push(label(controller.state()));
        assert_eq!(controller.state().kind(), Some(case.kind));
    }
    assert_eq!(
        seen,
        [
            "pan-pressed",
            "panning",
            "zoom-pressed",
            "scrubbing",
            "select-pressed",
            "marquee",
            "drag-pressed",
            "dragging",
            "resize-pressed",
            "resizing",
            "rotate-pressed",
            "rotating",
        ]
    );
    assert_eq!(label(&InteractionState::default()), "idle");
}
