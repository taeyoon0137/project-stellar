//! 생성 도구 fixture: 도구별 드래그·클릭·수식키, 흐름 삽입 index, 생성 1회 history 1건.

use super::*;
use crate::canvas::transform::fixtures::{
    add_shape, at, canvas_document, history_len, model, open, scene,
};
use crate::window_session::test_fixtures::sid;
use stellar_commands::style_defaults::SetLayoutProperty;
use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::value::FiniteF32;
use stellar_document::layout_record::{LayoutLonghand, LayoutValue};

fn paint() -> ShapePaint {
    let gray = Color::new(ColorSpace::Srgb, 0.85, 0.85, 0.85, 1.0).unwrap();
    let black = Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0).unwrap();
    ShapePaint {
        fill: Some(gray),
        stroke: Some((black, FiniteF32::new(1.0).unwrap())),
    }
}

const NONE: CreateModifiers = CreateModifiers {
    shift: false,
    option: false,
    space: false,
};

fn drag(tool: CreateTool, from: [f32; 2], to: [f32; 2], m: CreateModifiers) -> CreateShape {
    let mut g = CreateGesture::begin(tool, from, 1.0).unwrap();
    g.update(to, m)
}

#[test]
fn drag_click_and_modifiers_for_every_tool() {
    // 검증 기준(XII.10.26): 도구별 드래그·클릭·수식키 fixture.
    for tool in [
        CreateTool::Frame,
        CreateTool::Rectangle,
        CreateTool::Ellipse,
        CreateTool::Polygon,
    ] {
        // 드래그: 거꾸로 끌어도 양수 크기.
        assert_eq!(
            drag(tool, [50., 50.], [10., 20.], NONE),
            CreateShape::Box([10., 20., 40., 30.])
        );
        // Shift: 긴 변 기준 정비례.
        let shift = CreateModifiers {
            shift: true,
            ..NONE
        };
        assert_eq!(
            drag(tool, [0., 0.], [40., -10.], shift),
            CreateShape::Box([0., -40., 40., 40.])
        );
        // Option: 시작점 중심.
        let option = CreateModifiers {
            option: true,
            ..NONE
        };
        assert_eq!(
            drag(tool, [100., 100.], [130., 110.], option),
            CreateShape::Box([70., 90., 60., 20.])
        );
        // 클릭(3pt 미만 이동): 기본 100×100, Option이면 중심.
        assert_eq!(
            drag(tool, [5., 5.], [6., 6.], NONE),
            CreateShape::Box([5., 5., 100., 100.])
        );
        assert_eq!(
            drag(tool, [5., 5.], [5., 5.], option),
            CreateShape::Box([-45., -45., 100., 100.])
        );
    }
    // Line: 45도 단위, 중심 기준, 클릭 기본 길이.
    let shift = CreateModifiers {
        shift: true,
        ..NONE
    };
    let CreateShape::Line(a, flat) = drag(CreateTool::Line, [0., 0.], [30., 4.], shift) else {
        panic!()
    };
    assert_eq!((a, flat[1]), ([0., 0.], 0.0), "0도로 맞춤");
    assert!((flat[0] - 916f32.sqrt()).abs() < 1e-4, "길이 유지");
    let CreateShape::Line(_, end) = drag(CreateTool::Line, [0., 0.], [20., 22.], shift) else {
        panic!()
    };
    assert!((end[0] - end[1]).abs() < 1e-3, "45도: {end:?}");
    assert_eq!(
        drag(
            CreateTool::Line,
            [10., 10.],
            [20., 15.],
            CreateModifiers {
                option: true,
                ..NONE
            }
        ),
        CreateShape::Line([0., 5.], [20., 15.])
    );
    assert_eq!(
        drag(CreateTool::Line, [10., 10.], [10., 10.], NONE),
        CreateShape::Line([10., 10.], [110., 10.])
    );
    // 끌기 시작 거리는 화면 pt: 배율 4면 부모 1px 이동도 4pt라 드래그입니다.
    let mut zoomed = CreateGesture::begin(CreateTool::Rectangle, [0., 0.], 4.0).unwrap();
    zoomed.update([1., 1.], NONE);
    assert!(zoomed.dragging());
}

#[test]
fn space_moves_the_start_point_and_keeps_the_size() {
    let mut g = CreateGesture::begin(CreateTool::Rectangle, [0., 0.], 1.0).unwrap();
    assert_eq!(
        g.update([40., 30.], NONE),
        CreateShape::Box([0., 0., 40., 30.])
    );
    let space = CreateModifiers {
        space: true,
        ..NONE
    };
    g.update([40., 30.], space);
    assert_eq!(
        g.update([60., 50.], space),
        CreateShape::Box([20., 20., 40., 30.])
    );
    // Space를 떼면 옮긴 시작점에서 다시 크기를 바꿉니다.
    assert_eq!(
        g.update([80., 50.], NONE),
        CreateShape::Box([20., 20., 60., 30.])
    );
}

#[test]
fn commit_inserts_selects_and_returns_to_move_in_one_entry() {
    // 검증 기준(XII.10.26): 생성 1회 history 1건(InsertNode + 새 선택), 생성 후 Move.
    let (_dir, mut w) = open("create-commit", &canvas_document());
    let s = scene(&w);
    for (n, tool) in [
        CreateTool::Frame,
        CreateTool::Rectangle,
        CreateTool::Ellipse,
        CreateTool::Line,
        CreateTool::Polygon,
    ]
    .into_iter()
    .enumerate()
    {
        let mut tools = CanvasToolState::new();
        tools.select(match tool {
            CreateTool::Frame => CanvasTool::Frame,
            CreateTool::Rectangle => CanvasTool::Rectangle,
            CreateTool::Ellipse => CanvasTool::Ellipse,
            CreateTool::Line => CanvasTool::Line,
            CreateTool::Polygon => CanvasTool::Polygon,
        });
        let before = history_len(&w);
        let parent = creation_parent(model(&w), &s, sid(10), [300., 250.]);
        assert_eq!(parent, sid(10), "빈 곳은 View root");
        let start = [250. + n as f32, 200.];
        let shape = drag(tool, start, [start[0] + 40., start[1] + 20.], NONE);
        let node = commit(
            &mut w,
            &s,
            &mut tools,
            tool,
            parent,
            shape,
            [270., 210.],
            paint(),
        )
        .unwrap();
        assert_eq!(history_len(&w), before + 1, "{tool:?}");
        assert_eq!(w.view().selection().ids(), &[node]);
        assert_eq!(tools.base(), CanvasTool::Move);
        assert_eq!(model(&w).children_in_order(sid(10)).last(), Some(&node));
        // 위치: Frame은 transform, 도형은 맨 앞 Transform modifier.
        let local = stellar_commands::arrange::place::local_transform(model(&w), node).unwrap();
        assert_eq!(local.components().4, start[0]);
        assert_eq!(local.components().5, start[1]);
    }
    // undo 한 번이 생성과 선택을 함께 되돌립니다.
    let last = w.view().selection().ids()[0];
    assert!(w.undo().unwrap());
    assert!(!model(&w).contains(last));
    assert!(!w.view().selection().contains(last));
}

#[test]
fn creation_over_frames_and_into_flow_order() {
    // 검증 기준(XII.10.26): 흐름 삽입 index fixture.
    let mut doc = canvas_document();
    for (id, x) in [(30u64, 0.0f32), (31, 30.0), (32, 60.0)] {
        add_shape(
            &mut doc,
            id,
            11,
            ShapeGeometry::Rectangle(RectangleParams {
                size: Size2::new(20., 20.).unwrap(),
                corner_radius: Default::default(),
            }),
            at(x, 0.0),
        );
    }
    let (_dir, mut w) = open("create-flow", &doc);
    // 자유 배치 Frame 11 위: 부모는 11, 맨 뒤 순서.
    let s = scene(&w);
    assert_eq!(creation_parent(model(&w), &s, sid(10), [55., 60.]), sid(11));
    let flex = vec![Command::SetLayoutProperty(SetLayoutProperty {
        node: sid(11),
        longhand: LayoutLonghand::Display,
        value: LayoutValue::Keyword("flex"),
    })];
    crate::canvas::transform::commit(&mut w, flex, 0).unwrap();
    let s = scene(&w);
    let mut tools = CanvasToolState::new();
    tools.select(CanvasTool::Rectangle);
    // world x 45: 30(중심 20) 뒤, 31(중심 50) 앞 → index 1.
    let node = commit(
        &mut w,
        &s,
        &mut tools,
        CreateTool::Rectangle,
        sid(11),
        CreateShape::Box([0., 0., 20., 20.]),
        [45., 30.],
        paint(),
    )
    .unwrap();
    assert_eq!(
        model(&w).children_in_order(sid(11)),
        vec![sid(30), node, sid(31), sid(32)]
    );
    // 흐름 자식은 위치 Transform modifier가 없습니다.
    let stack = model(&w).modifier_stack(node).unwrap();
    assert!(stack
        .entries()
        .iter()
        .all(|e| !matches!(e.payload, ModifierPayload::Transform(_))));
}

#[test]
fn rejects_unpainted_and_empty_shapes() {
    let (_dir, mut w) = open("create-reject", &canvas_document());
    let s = scene(&w);
    let mut tools = CanvasToolState::new();
    let unpainted = ShapePaint {
        fill: None,
        stroke: None,
    };
    let before = history_len(&w);
    for (tool, shape, paint) in [
        (
            CreateTool::Rectangle,
            CreateShape::Box([0., 0., 10., 10.]),
            unpainted,
        ),
        (
            CreateTool::Rectangle,
            CreateShape::Box([0., 0., 0., 10.]),
            paint(),
        ),
        (
            CreateTool::Line,
            CreateShape::Line([1., 1.], [1., 1.]),
            paint(),
        ),
        (
            CreateTool::Line,
            CreateShape::Box([0., 0., 10., 10.]),
            paint(),
        ),
    ] {
        assert!(commit(
            &mut w,
            &s,
            &mut tools,
            tool,
            sid(10),
            shape,
            [0., 0.],
            paint
        )
        .is_err());
    }
    assert_eq!(history_len(&w), before);
}
