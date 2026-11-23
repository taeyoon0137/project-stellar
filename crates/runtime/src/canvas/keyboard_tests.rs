//! 키보드만으로 하는 캔버스 편집 경로 fixture (XII.10.41).

use super::*;
use crate::canvas::select_fixtures::{nested, view_point};
use crate::canvas::transform::fixtures::{
    canvas_document, close, history_len, matrix, model, open, scene, select,
};
use crate::window_session::test_fixtures::{sid, window};
use stellar_core_types::clock::ManualClock;
use stellar_document::css::registry::entries_named;
use stellar_document::layout_record::LayoutLonghand;
use stellar_document::records::NodePayload;
use stellar_document::shape::ShapeGeometry;
use stellar_scene::transient::{Corner, Edge};

const ARROWS: [Arrow; 4] = [Arrow::Left, Arrow::Right, Arrow::Up, Arrow::Down];

#[test]
fn key_table_follows_the_unit_adr() {
    for direction in ARROWS {
        let key = KeyboardKey::Arrow(direction);
        assert_eq!(
            keyboard_command(key, Modifiers::NONE),
            Some(KeyboardCommand::Move(direction, 1.0))
        );
        assert_eq!(
            keyboard_command(key, Modifiers::SHIFT),
            Some(KeyboardCommand::Move(direction, 10.0))
        );
        assert_eq!(
            keyboard_command(key, Modifiers::COMMAND),
            Some(KeyboardCommand::Resize(direction, 1.0))
        );
        assert_eq!(
            keyboard_command(key, Modifiers::SHIFT_COMMAND),
            Some(KeyboardCommand::Resize(direction, 10.0))
        );
        assert_eq!(keyboard_command(key, Modifiers::OPTION), None);
    }
    assert_eq!(
        keyboard_command(KeyboardKey::Tab, Modifiers::SHIFT),
        Some(KeyboardCommand::Select(CanvasKey::Tab, Modifiers::SHIFT))
    );
    assert_eq!(
        keyboard_command(KeyboardKey::Return, Modifiers::NONE),
        Some(KeyboardCommand::Select(CanvasKey::Return, Modifiers::NONE))
    );
    let control = Modifiers {
        control: true,
        ..Modifiers::NONE
    };
    for (key, modifiers) in [
        (KeyboardKey::F10, Modifiers::SHIFT),
        (KeyboardKey::ContextMenu, Modifiers::NONE),
        (KeyboardKey::Return, control),
    ] {
        assert_eq!(
            keyboard_command(key, modifiers),
            Some(KeyboardCommand::OpenContextMenu)
        );
    }
    assert_eq!(keyboard_command(KeyboardKey::F10, Modifiers::NONE), None);
    assert_eq!(
        keyboard_command(KeyboardKey::Character('a'), Modifiers::COMMAND),
        Some(KeyboardCommand::Select(
            CanvasKey::Character('a'),
            Modifiers::COMMAND
        ))
    );
    assert_eq!(
        keyboard_command(KeyboardKey::Character('a'), Modifiers::NONE),
        None
    );
}

#[test]
fn focus_ring_shows_only_for_keyboard_use() {
    let mut focus = CanvasFocus::default();
    assert!(!focus.ring_visible());
    focus.focus(true);
    assert!(focus.ring_visible());
    focus.pointer_input();
    assert!(focus.focused() && !focus.ring_visible());
    focus.key_input();
    assert!(focus.ring_visible());
    focus.blur();
    focus.key_input();
    assert!(!focus.focused() && !focus.ring_visible());
    focus.focus(false);
    assert!(!focus.ring_visible());
}

#[test]
fn tab_return_and_shift_return_move_the_selection_with_keys_only() {
    let fixture = nested();
    let mut session = CanvasSelectSession::new();
    let mut history = WindowHistory::new(window(11));
    let mut doc = DocumentState::new(fixture.model);
    let root = fixture.root;
    let mut key = |history: &mut WindowHistory, doc: &mut DocumentState, k, m| {
        let KeyboardCommand::Select(key, modifiers) = keyboard_command(k, m).unwrap() else {
            panic!()
        };
        select_key(&mut session, history, doc, root, key, modifiers).unwrap()
    };
    // 선택이 없으면 Tab은 범위의 첫 자식을 고릅니다.
    assert!(matches!(
        key(&mut history, &mut doc, KeyboardKey::Tab, Modifiers::NONE),
        SessionOutput::Selected(_)
    ));
    assert_eq!(history.view().selection().ids(), [sid(11)]);
    key(&mut history, &mut doc, KeyboardKey::Tab, Modifiers::NONE);
    assert_eq!(history.view().selection().ids(), [sid(12)]);
    key(&mut history, &mut doc, KeyboardKey::Tab, Modifiers::SHIFT);
    assert_eq!(history.view().selection().ids(), [sid(11)]);
    // Return: 컨테이너 안으로(선택 가능한 자식 전체), Shift-Return: 부모로.
    key(&mut history, &mut doc, KeyboardKey::Return, Modifiers::NONE);
    assert_eq!(history.view().selection().ids(), [sid(13), sid(14)]);
    key(
        &mut history,
        &mut doc,
        KeyboardKey::Return,
        Modifiers::SHIFT,
    );
    assert_eq!(history.view().selection().ids(), [sid(11)]);
    // Escape: View root 범위에서 선택 해제.
    key(&mut history, &mut doc, KeyboardKey::Escape, Modifiers::NONE);
    assert!(history.view().selection().is_empty());
    // 문서는 그대로이고 선택 기록만 남습니다.
    assert!(history.history().entries().len() >= 5);
}

#[test]
fn arrows_move_and_command_arrows_resize_through_the_keyboard_transform_path() {
    let (_dir, mut w) = open("keyboard-path", &canvas_document());
    select(&mut w, &[13]);
    let clock = ManualClock::new(0);
    let before = history_len(&w);
    let run = |w: &mut ProjectWindowSession, k, m: Modifiers, request| {
        let s = scene(w);
        let command = keyboard_command(k, m).unwrap();
        transform_key(w, &s, command, m.shift, RequestId(request), &clock).unwrap()
    };
    assert_eq!(
        run(
            &mut w,
            KeyboardKey::Arrow(Arrow::Right),
            Modifiers::SHIFT,
            1
        ),
        KeyOutcome::Moved
    );
    assert!(close(matrix(&w, 13), (1., 0., 0., 1., 40., 100.)));
    assert_eq!(
        run(
            &mut w,
            KeyboardKey::Arrow(Arrow::Down),
            Modifiers::SHIFT_COMMAND,
            2
        ),
        KeyOutcome::Moved
    );
    assert!(
        matches!(model(&w).payload(sid(13)), Some(NodePayload::Shape(ShapeGeometry::Rectangle(r))) if r.size.height() == 30.0)
    );
    assert_eq!(history_len(&w), before + 2);
    // 선택이 없으면 camera만 움직입니다.
    select(&mut w, &[]);
    let revision = w.revision();
    assert_eq!(
        run(&mut w, KeyboardKey::Arrow(Arrow::Left), Modifiers::NONE, 3),
        KeyOutcome::Panned
    );
    assert_eq!(w.revision(), revision);
    let s = scene(&w);
    assert!(transform_key(
        &mut w,
        &s,
        KeyboardCommand::OpenContextMenu,
        false,
        RequestId(4),
        &clock
    )
    .is_err());
}

#[test]
fn insert_plans_center_a_default_size_box_in_parent_space() {
    // 화면 800×600, 2배 확대에 world (100, 50)을 화면 원점에 둔 camera. 화면 가운데 = world (300, 200).
    let view = Transform2D::new(2., 0., 0., 2., -200., -100.).unwrap();
    let viewport = Viewport::new(Size2::new(800., 600.).unwrap(), view).unwrap();
    // 부모 local 원점이 world (100, 100).
    let parent = Transform2D::new(1., 0., 0., 1., 100., 100.).unwrap();
    let plan = |kind| plan_insert_at_center(kind, sid(10), parent, &viewport).unwrap();
    let v = |x: f32, y: f32| Vec2::new(x, y).unwrap();
    assert_eq!(
        plan(InsertKind::Frame),
        InsertPlan::Frame(Rect::new(v(150., 50.), Size2::new(100., 100.).unwrap()))
    );
    assert_eq!(
        plan(InsertKind::Shape(ShapeKind::Ellipse)),
        InsertPlan::Shape(ShapeKind::Ellipse, [v(150., 50.), v(250., 150.)])
    );
    assert_eq!(
        plan(InsertKind::Shape(ShapeKind::Line)),
        InsertPlan::Shape(ShapeKind::Line, [v(150., 100.), v(250., 100.)])
    );
    assert_eq!(
        plan(InsertKind::Text),
        InsertPlan::Text(TextPlacement {
            parent: sid(10),
            origin: v(200., 100.),
            width: TextWidth::Auto,
        })
    );
}

#[test]
fn context_menu_opens_at_the_selection_or_the_view_center() {
    use crate::canvas::select_fixtures::scene_of;
    let fixture = nested();
    let built = scene_of(&fixture.model, fixture.root);
    let geometry = CanvasGeometry::from_snapshot(&built.snapshot);
    let tree = SelectTree::new(&fixture.model, fixture.root);
    // Frame 11 (100,100) 200×200의 가운데.
    assert_eq!(
        context_menu_anchor(&tree, &geometry, &[sid(11)], &built.viewport),
        Some(view_point(200., 200.))
    );
    assert_eq!(
        context_menu_anchor(&tree, &geometry, &[], &built.viewport),
        Some(view_point(400., 300.))
    );
}

#[test]
fn every_handle_action_has_a_keyboard_or_inspector_equivalent() {
    let corners = [
        Corner::TopLeft,
        Corner::TopRight,
        Corner::BottomRight,
        Corner::BottomLeft,
    ];
    let edges = [Edge::Top, Edge::Right, Edge::Bottom, Edge::Left];
    let handles = corners
        .iter()
        .flat_map(|&c| {
            [
                CanvasHandle::Radius(c),
                CanvasHandle::ResizeCorner(c),
                CanvasHandle::Rotate(c),
            ]
        })
        .chain(edges.iter().map(|&e| CanvasHandle::ResizeEdge(e)))
        .chain([CanvasHandle::Body]);
    for handle in handles {
        let equivalents = handle_equivalents(handle);
        assert!(!equivalents.is_empty(), "{handle:?}");
        for equivalent in equivalents {
            match *equivalent {
                Equivalent::Key(key, modifiers) => {
                    let command = keyboard_command(key, modifiers);
                    let expected_resize = matches!(
                        handle,
                        CanvasHandle::ResizeCorner(_) | CanvasHandle::ResizeEdge(_)
                    );
                    assert!(
                        match command {
                            Some(KeyboardCommand::Move(..)) => handle == CanvasHandle::Body,
                            Some(KeyboardCommand::Resize(..)) => expected_resize,
                            _ => false,
                        },
                        "{handle:?} {equivalent:?}"
                    );
                }
                Equivalent::InspectorField(name) => assert!(
                    !entries_named(name).is_empty() || LayoutLonghand::from_name(name).is_some(),
                    "인스펙터 속성 {name}"
                ),
            }
        }
    }
}
