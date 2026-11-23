//! 문서 layout 결과로 그리는 scene 변환 테스트 (ADR-0125).

use std::cell::RefCell;
use std::time::Instant;

use super::super::*;
use super::*;
use crate::dirty::SceneDirtySet;
use crate::incremental::IncrementalScene;
use stellar_commands::command::Command;
use stellar_commands::id::{CommandId, CommandOrigin};
use stellar_commands::property::{PropertyValue, SetProperty};
use stellar_commands::request::CommandRequest;
use stellar_commands::state::DocumentState;
use stellar_commands::style_defaults::SetLayoutProperty;
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::clock::ManualClock;
use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::text::UserText;
use stellar_core_types::value::{Rect, Vec2};
use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::frame::{keys as frame_keys, FramePayload};
use stellar_document::layout_record::LayoutRecord;
use stellar_document::records::NodeRecord;
use stellar_document::shape::{RectangleParams, ShapeGeometry};
use stellar_document::text::{Paragraph, TextPayload};

fn sid(n: u64) -> StableId {
    StableId::new_v7(n, 0, n)
}

fn color() -> Color {
    Color::new(ColorSpace::Srgb, 1.0, 1.0, 1.0, 1.0).unwrap()
}

fn translate(x: f32, y: f32) -> Transform2D {
    Transform2D::new(1., 0., 0., 1., x, y).unwrap()
}

fn frame(w: f32, h: f32, x: f32, y: f32) -> NodePayload {
    NodePayload::Frame(FramePayload {
        size: Size2::new(w, h).unwrap(),
        transform: translate(x, y),
        background: Some(color()),
        ..FramePayload::default()
    })
}

fn rectangle(w: f32, h: f32) -> NodePayload {
    NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
        size: Size2::new(w, h).unwrap(),
        corner_radius: Default::default(),
    }))
}

fn node(n: u64, parent: Option<u64>, key: u8, payload: NodePayload) -> NodeRecord {
    NodeRecord {
        id: sid(n),
        parent: parent.map(sid),
        order_key: OrderKey::from_bytes(vec![key]).unwrap(),
        visible: true,
        locked: false,
        display_name: format!("n{n}"),
        payload,
    }
}

fn empty_model() -> DocumentModel {
    let mut definition = DocumentDefinition::new(sid(1), "문서".to_string());
    let mut page = PageDefinition::new(sid(2), "페이지".to_string());
    page.add_view(ViewDefinition::new(sid(3), "화면".to_string(), sid(10)))
        .unwrap();
    definition.add_page(page).unwrap();
    let mut model = DocumentModel::new(definition);
    model
        .insert_node(node(10, None, 0x10, frame(800., 600., 0., 0.)))
        .unwrap();
    model
}

fn set_layout(model: &mut DocumentModel, id: u64, values: &[(&str, &str)]) {
    let mut record = model
        .layout_record(sid(id))
        .cloned()
        .unwrap_or_else(|| LayoutRecord::new(sid(id)));
    for (name, value) in values {
        let longhand = LayoutLonghand::from_name(name).unwrap();
        record
            .set(longhand, longhand.parse(value).unwrap())
            .unwrap();
    }
    model.set_layout_record(record).unwrap();
}

/// root 10 아래 자유 배치 Frame 11(100, 50, 300×100, 가로 flex, gap 10, 왼쪽 padding 5). 11 아래 Frame 21(50×40),
/// Rectangle 22(60×30), Frame 23(70×20, 저장 transform translate(3, 4)). 자유 배치 Frame 12(500, 50).
fn flex_model() -> DocumentModel {
    let mut model = empty_model();
    for record in [
        node(11, Some(10), 0x10, frame(300., 100., 100., 50.)),
        node(12, Some(10), 0x20, frame(40., 40., 500., 50.)),
        node(21, Some(11), 0x10, frame(50., 40., 0., 0.)),
        node(22, Some(11), 0x20, rectangle(60., 30.)),
        node(23, Some(11), 0x30, frame(70., 20., 3., 4.)),
    ] {
        model.insert_node(record).unwrap();
    }
    set_layout(
        &mut model,
        11,
        &[
            ("display", "flex"),
            ("column-gap", "10px"),
            ("padding-left", "5px"),
        ],
    );
    model
}

fn transform_of(built: &DocumentSceneSource, id: u64) -> Transform2D {
    built
        .source
        .get(SceneKey::node(sid(id)))
        .and_then(|n| n.kind.node_input())
        .map(|n| n.transform)
        .unwrap_or_else(|| panic!("node {id} 없음: {:?}", built.skipped))
}

fn frame_size(built: &DocumentSceneSource, id: u64) -> Size2 {
    match &built.source.get(SceneKey::node(sid(id))).unwrap().kind {
        SourceKind::Frame(input) => input.size,
        other => panic!("Frame 아님: {other:?}"),
    }
}

fn execute(state: &mut DocumentState, command: Command) -> ChangeSet {
    let tx = CommandTransaction::new(CommandId::from_stable_id(sid(9_000)), CommandOrigin::User)
        .with_request(CommandRequest::new(state.revision(), command));
    state.execute_transaction(&tx).unwrap().changes
}

fn clock() -> ManualClock {
    ManualClock::new(0)
}

/// change set을 증분 경로로 반영하고 처음부터 변환·build한 결과와 같은지 확인합니다. 편집 목록을 돌려줍니다.
fn apply_and_compare(
    state: &mut DocumentState,
    built: &mut DocumentSceneSource,
    scene: &mut IncrementalScene,
    command: Command,
) -> Vec<SourceEdit> {
    let changes = execute(state, command);
    let dirty =
        SceneDirtySet::from_invalidations(&scene_invalidations(state.model(), &changes), &*built);
    let edits = scene_edits(
        state.model(),
        built,
        &changes,
        SceneSourceOptions::default(),
    )
    .unwrap();
    scene
        .update(state.revision(), &edits, &dirty, &clock())
        .unwrap();
    let fresh =
        document_scene_source(state.model(), sid(10), SceneSourceOptions::default()).unwrap();
    assert_eq!(built.source, fresh.source);
    assert_eq!(built.skipped, fresh.skipped);
    assert_eq!(built.layout, fresh.layout);
    let (full, _) = IncrementalScene::build(fresh.source, state.revision(), &clock()).unwrap();
    assert_eq!(
        scene.snapshot().semantic_hash(),
        full.snapshot().semantic_hash()
    );
    edits
}

fn set_layout_command(id: u64, name: &str, value: &str) -> Command {
    let longhand = LayoutLonghand::from_name(name).unwrap();
    Command::SetLayoutProperty(SetLayoutProperty {
        node: sid(id),
        longhand,
        value: longhand.parse(value).unwrap(),
    })
}

fn upserted(edits: &[SourceEdit]) -> BTreeSet<StableId> {
    edits
        .iter()
        .filter_map(|e| match e {
            SourceEdit::Upsert(key, _) => Some(key.id()),
            _ => None,
        })
        .collect()
}

#[test]
fn flex_children_are_drawn_at_layout_positions_and_free_nodes_keep_stored_transforms() {
    let model = flex_model();
    let built = document_scene_source(&model, sid(10), SceneSourceOptions::default()).unwrap();
    assert!(built.skipped.is_empty(), "{:?}", built.skipped);
    assert!(
        built.layout().is_available(),
        "{:?}",
        built.layout().error()
    );
    // 흐름 자식: padding 5부터 50 + 10 + 60 + 10. 저장 transform은 그 뒤 translate로 합성됩니다(23의 (3, 4)).
    assert_eq!(transform_of(&built, 21), translate(5., 0.));
    assert_eq!(transform_of(&built, 22), translate(65., 0.));
    assert_eq!(transform_of(&built, 23), translate(138., 4.));
    // 옛 자유 배치 node와 root는 저장 transform 그대로입니다.
    assert_eq!(transform_of(&built, 11), translate(100., 50.));
    assert_eq!(transform_of(&built, 12), translate(500., 50.));
    assert_eq!(transform_of(&built, 10), Transform2D::identity());
    assert_eq!(frame_size(&built, 11), Size2::new(300., 100.).unwrap());
    assert_eq!(built.layout().rule(&model, sid(21)), PlacementRule::Layout);
    assert_eq!(built.layout().rule(&model, sid(12)), PlacementRule::Stored);
    assert_eq!(built.layout().rule(&model, sid(10)), PlacementRule::Root);
}

#[test]
fn flex_grow_changes_the_drawn_size() {
    let mut model = flex_model();
    set_layout(&mut model, 21, &[("flex-grow", "1")]);
    let built = document_scene_source(&model, sid(10), SceneSourceOptions::default()).unwrap();
    // css 세트는 content-box라 payload 너비 300이 content box입니다. 남는 공간: 300 - (50 + 60 + 70) - 20 = 100. 21만 늘어납니다.
    assert_eq!(frame_size(&built, 21), Size2::new(150., 40.).unwrap());
    assert_eq!(transform_of(&built, 22), translate(165., 0.));
    // Rectangle도 결과 크기로 그립니다(flex-grow).
    set_layout(&mut model, 21, &[("flex-grow", "0")]);
    set_layout(&mut model, 22, &[("flex-grow", "1")]);
    let built = document_scene_source(&model, sid(10), SceneSourceOptions::default()).unwrap();
    match &built.source.get(SceneKey::node(sid(22))).unwrap().kind {
        SourceKind::Shape(input) => assert_eq!(
            input.layout_rect,
            Rect::new(Vec2::ZERO, Size2::new(160., 30.).unwrap())
        ),
        other => panic!("{other:?}"),
    }
}

#[test]
fn explicit_absolute_position_is_drawn_at_its_inset_like_the_migrated_transform() {
    // IX.11.9 migration 결과 모양: 이동 성분이 left·top으로 옮겨지고 선형 부분만 transform에 남은 node는 옮기기 전
    // transform과 같은 화면 위치에 그려집니다.
    let rotate = Transform2D::new(0., 1., -1., 0., 0., 0.).unwrap();
    let before = rotate.then(&translate(40., 25.)).unwrap();
    let mut old = empty_model();
    old.insert_node(node(11, Some(10), 0x10, frame(300., 100., 10., 10.)))
        .unwrap();
    let mut stored = match frame(30., 20., 0., 0.) {
        NodePayload::Frame(frame) => frame,
        _ => unreachable!(),
    };
    stored.transform = before;
    old.insert_node(node(21, Some(11), 0x10, NodePayload::Frame(stored.clone())))
        .unwrap();
    let old_built = document_scene_source(&old, sid(10), SceneSourceOptions::default()).unwrap();
    let mut migrated = old;
    stored.transform = rotate;
    migrated
        .set_payload(sid(21), NodePayload::Frame(stored))
        .unwrap();
    set_layout(
        &mut migrated,
        21,
        &[("position", "absolute"), ("left", "40px"), ("top", "25px")],
    );
    let built = document_scene_source(&migrated, sid(10), SceneSourceOptions::default()).unwrap();
    assert_eq!(
        built.layout().rule(&migrated, sid(21)),
        PlacementRule::Layout
    );
    assert_eq!(transform_of(&built, 21), transform_of(&old_built, 21));
    assert_eq!(transform_of(&built, 21), before);
    // right·bottom anchor도 layout 결과(부모 300×100 기준)로 풉니다.
    set_layout(
        &mut migrated,
        21,
        &[
            ("left", "auto"),
            ("top", "auto"),
            ("right", "10px"),
            ("bottom", "5px"),
        ],
    );
    let built = document_scene_source(&migrated, sid(10), SceneSourceOptions::default()).unwrap();
    assert_eq!(
        transform_of(&built, 21),
        rotate.then(&translate(260., 75.)).unwrap()
    );
}

#[test]
fn width_change_moves_following_siblings_through_incremental_layout_and_scene() {
    let mut state = DocumentState::new(flex_model());
    let mut built =
        document_scene_source(state.model(), sid(10), SceneSourceOptions::default()).unwrap();
    let (mut scene, _) =
        IncrementalScene::build(built.source.clone(), state.revision(), &clock()).unwrap();
    // 21의 width(payload 크기)를 바꾸면 22·23이 비켜납니다. change set에는 21만 있습니다.
    let edits = apply_and_compare(
        &mut state,
        &mut built,
        &mut scene,
        Command::SetProperty(SetProperty {
            node: sid(21),
            key: frame_keys::WIDTH,
            value: PropertyValue::NonNegative(
                stellar_core_types::value::FiniteF32::new(80.0).unwrap(),
            ),
        }),
    );
    let moved = upserted(&edits);
    assert!(moved.contains(&sid(21)));
    assert!(moved.contains(&sid(22)), "{moved:?}");
    assert!(moved.contains(&sid(23)), "{moved:?}");
    assert!(!moved.contains(&sid(12)), "{moved:?}");
    assert_eq!(transform_of(&built, 22), translate(95., 0.));
    // 증분 layout은 컨테이너 안쪽만 다시 배치했습니다.
    let stats = built.layout().last_stats();
    assert_eq!(stats.full_layouts, 0);
    assert_eq!(built.layout().full_rebuilds(), 0);
    // container longhand(gap) 변경은 모든 자식을 옮깁니다.
    let edits = apply_and_compare(
        &mut state,
        &mut built,
        &mut scene,
        set_layout_command(11, "column-gap", "20px"),
    );
    assert!(upserted(&edits).contains(&sid(23)));
    // 흐름 컨테이너를 block으로 바꾸면 자식은 옛 자유 배치(저장 transform)로 돌아갑니다.
    apply_and_compare(
        &mut state,
        &mut built,
        &mut scene,
        set_layout_command(11, "display", "block"),
    );
    assert_eq!(transform_of(&built, 23), translate(3., 4.));
    assert_eq!(
        built.layout().rule(state.model(), sid(23)),
        PlacementRule::Stored
    );
}

#[test]
fn edits_under_a_legacy_layout_frame_stay_incremental() {
    // 옛 layout 정의 참조 Frame(13)의 자식(33)은 layout 입력에 없습니다. 그 자식 편집은 layout을 바꾸지 않고, 증분
    // 갱신이 실패해 전체 다시 계산으로 가지도 않습니다.
    let mut model = flex_model();
    let mut legacy = match frame(50., 50., 600., 0.) {
        NodePayload::Frame(frame) => frame,
        _ => unreachable!(),
    };
    legacy.layout_mode = Some(sid(99));
    model
        .insert_node(node(13, Some(10), 0x30, NodePayload::Frame(legacy)))
        .unwrap();
    model
        .insert_node(node(33, Some(13), 0x10, frame(10., 10., 0., 0.)))
        .unwrap();
    let mut state = DocumentState::new(model);
    let mut built =
        document_scene_source(state.model(), sid(10), SceneSourceOptions::default()).unwrap();
    assert!(built.layout().session().unwrap().node(sid(33)).is_none());
    let (mut scene, _) =
        IncrementalScene::build(built.source.clone(), state.revision(), &clock()).unwrap();
    apply_and_compare(
        &mut state,
        &mut built,
        &mut scene,
        Command::SetProperty(SetProperty {
            node: sid(33),
            key: frame_keys::WIDTH,
            value: PropertyValue::NonNegative(
                stellar_core_types::value::FiniteF32::new(20.0).unwrap(),
            ),
        }),
    );
    assert_eq!(built.layout().full_rebuilds(), 0);
    assert!(built.layout().session().unwrap().node(sid(33)).is_none());
}

#[test]
fn structure_and_visibility_edits_keep_incremental_layout_equal_to_a_full_rebuild() {
    let mut state = DocumentState::new(flex_model());
    let mut built =
        document_scene_source(state.model(), sid(10), SceneSourceOptions::default()).unwrap();
    let (mut scene, _) =
        IncrementalScene::build(built.source.clone(), state.revision(), &clock()).unwrap();
    // 숨긴 흐름 자식은 상자가 없어(display: none) 뒤 형제가 당겨집니다.
    apply_and_compare(
        &mut state,
        &mut built,
        &mut scene,
        Command::SetProperty(SetProperty {
            node: sid(21),
            key: node_keys::VISIBLE,
            value: PropertyValue::Bool(false),
        }),
    );
    assert_eq!(transform_of(&built, 22), translate(5., 0.));
    apply_and_compare(
        &mut state,
        &mut built,
        &mut scene,
        Command::SetProperty(SetProperty {
            node: sid(21),
            key: node_keys::VISIBLE,
            value: PropertyValue::Bool(true),
        }),
    );
    assert_eq!(transform_of(&built, 22), translate(65., 0.));
    // 순서 바꾸기: 23을 맨 앞으로.
    apply_and_compare(
        &mut state,
        &mut built,
        &mut scene,
        Command::MoveNode(stellar_commands::node_ops::MoveNode {
            node: sid(23),
            new_parent: sid(11),
            order_key: OrderKey::from_bytes(vec![0x08]).unwrap(),
        }),
    );
    assert_eq!(transform_of(&built, 23), translate(8., 4.));
    assert_eq!(transform_of(&built, 21), translate(85., 0.));
    // 흐름 밖(자유 Frame 12)으로 옮기면 저장 transform으로 그립니다.
    apply_and_compare(
        &mut state,
        &mut built,
        &mut scene,
        Command::MoveNode(stellar_commands::node_ops::MoveNode {
            node: sid(23),
            new_parent: sid(12),
            order_key: OrderKey::from_bytes(vec![0x10]).unwrap(),
        }),
    );
    assert_eq!(transform_of(&built, 23), translate(3., 4.));
    assert_eq!(transform_of(&built, 21), translate(5., 0.));
    // 삭제와 undo 같은 재삽입.
    apply_and_compare(
        &mut state,
        &mut built,
        &mut scene,
        Command::DeleteSubtree(stellar_commands::node_ops::DeleteSubtree { root: sid(21) }),
    );
    assert_eq!(transform_of(&built, 22), translate(5., 0.));
    apply_and_compare(
        &mut state,
        &mut built,
        &mut scene,
        Command::InsertNode(stellar_commands::node_ops::InsertNode {
            nodes: vec![node(31, Some(11), 0x05, frame(20., 20., 0., 0.))],
            modifier_stacks: Vec::new(),
            layout_records: Vec::new(),
        }),
    );
    assert_eq!(transform_of(&built, 31), translate(5., 0.));
    assert_eq!(transform_of(&built, 22), translate(35., 0.));
    assert_eq!(built.layout().full_rebuilds(), 0);
}

#[test]
fn fragments_in_a_flow_container_are_flattened() {
    let mut model = flex_model();
    model
        .insert_node(node(
            30,
            Some(11),
            0x15,
            NodePayload::Fragment(Default::default()),
        ))
        .unwrap();
    model
        .insert_node(node(32, Some(30), 0x10, frame(10., 10., 0., 0.)))
        .unwrap();
    let built = document_scene_source(&model, sid(10), SceneSourceOptions::default()).unwrap();
    // 21(50) 뒤 Fragment 30의 32(10), 그 뒤 22.
    assert_eq!(transform_of(&built, 32), translate(65., 0.));
    assert_eq!(transform_of(&built, 22), translate(85., 0.));
    // 숨긴 Fragment의 자식은 상자가 없습니다(view 규칙).
    model.replace_flags(sid(30), false, false).unwrap();
    let mut state = DocumentState::new(flex_model());
    let mut built =
        document_scene_source(state.model(), sid(10), SceneSourceOptions::default()).unwrap();
    let (mut scene, _) =
        IncrementalScene::build(built.source.clone(), state.revision(), &clock()).unwrap();
    apply_and_compare(
        &mut state,
        &mut built,
        &mut scene,
        Command::InsertNode(stellar_commands::node_ops::InsertNode {
            nodes: vec![
                node(
                    30,
                    Some(11),
                    0x15,
                    NodePayload::Fragment(Default::default()),
                ),
                node(32, Some(30), 0x10, frame(10., 10., 0., 0.)),
            ],
            modifier_stacks: Vec::new(),
            layout_records: Vec::new(),
        }),
    );
    assert_eq!(transform_of(&built, 22), translate(85., 0.));
    apply_and_compare(
        &mut state,
        &mut built,
        &mut scene,
        Command::SetProperty(SetProperty {
            node: sid(30),
            key: node_keys::VISIBLE,
            value: PropertyValue::Bool(false),
        }),
    );
    assert_eq!(transform_of(&built, 22), translate(65., 0.));
}

#[test]
fn layout_failure_skips_only_layout_placed_nodes() {
    // padding이 있는 content-box 상자의 aspect-ratio는 엔진이 받지 않습니다(입력 검증 실패).
    let mut model = flex_model();
    set_layout(
        &mut model,
        21,
        &[("padding-top", "2px"), ("aspect-ratio", "1")],
    );
    let built = document_scene_source(&model, sid(10), SceneSourceOptions::default()).unwrap();
    assert!(!built.layout().is_available());
    assert!(built.layout().error().is_some());
    for id in [21, 22, 23] {
        assert_eq!(
            built.skipped.get(&SceneKey::node(sid(id))),
            Some(&SkipReason::LayoutNotEvaluated)
        );
    }
    assert_eq!(transform_of(&built, 12), translate(500., 50.));
    assert_eq!(transform_of(&built, 11), translate(100., 50.));
}

#[test]
fn react_native_defaults_lay_out_every_frame_as_a_column() {
    let mut model = empty_model();
    for record in [
        node(11, Some(10), 0x10, frame(100., 30., 7., 0.)),
        node(12, Some(10), 0x20, frame(100., 40., 0., 0.)),
    ] {
        model.insert_node(record).unwrap();
    }
    model.set_style_defaults("react-native");
    let built = document_scene_source(&model, sid(10), SceneSourceOptions::default()).unwrap();
    // react-native: frame 구획 display: flex, flex-direction: column.
    assert_eq!(transform_of(&built, 11), translate(7., 0.));
    assert_eq!(transform_of(&built, 12), translate(0., 30.));
}

fn text_payload(text: &str) -> NodePayload {
    NodePayload::Text(
        TextPayload::new(UserText::new(text), vec![Paragraph { style: 0 }], vec![]).unwrap(),
    )
}

#[test]
fn text_width_follows_the_resolved_width() {
    let mut model = flex_model();
    model
        .set_payload(sid(11), frame(400., 100., 100., 50.))
        .unwrap();
    model
        .insert_node(node(40, Some(11), 0x40, text_payload("가나다")))
        .unwrap();
    let widths: RefCell<Vec<Option<f32>>> = RefCell::new(Vec::new());
    // 가짜 text 엔진: 줄 너비가 없으면 90×12, 있으면 그 너비에 줄 수만큼 높이.
    let mut hook =
        |id: StableId, _: &TextPayload, _: Option<&ModifierStack>, _: bool, width: Option<f32>| {
            widths.borrow_mut().push(width);
            let w = width.unwrap_or(90.0);
            let lines = (90.0 / w).ceil();
            Some(TextInput {
                node: NodeInput::new(id, Transform2D::identity()),
                layout: crate::node::TextLayoutKey::from_bytes(id.to_bytes()),
                size: Size2::new(w.min(90.0), 12.0 * lines).unwrap(),
            })
        };
    let check = |model: &DocumentModel, hook: &mut TextHook<'_>| {
        let built = document_scene_source(
            model,
            sid(10),
            SceneSourceOptions {
                text: Some(hook),
                ..SceneSourceOptions::default()
            },
        )
        .unwrap();
        assert!(built.skipped.is_empty(), "{:?}", built.skipped);
        transform_of(&built, 40)
    };
    // width: auto(흐름 자식 max-content 기여 90, 남는 공간 충분) → content box 너비 90으로 그립니다.
    let at = check(&model, &mut hook);
    assert_eq!(at, translate(215., 0.));
    assert_eq!(widths.borrow().last(), Some(&Some(90.0)));
    // width: 40px → 40px 줄 너비.
    set_layout(&mut model, 40, &[("width", "40px")]);
    check(&model, &mut hook);
    assert_eq!(widths.borrow().last(), Some(&Some(40.0)));
    // width: max-content → 줄바꿈 없음(측정 결과를 다시 씀: 측정 한 번 = None 한 번).
    set_layout(&mut model, 40, &[("width", "max-content")]);
    widths.borrow_mut().clear();
    check(&model, &mut hook);
    assert_eq!(*widths.borrow(), vec![None]);
}

/// 줄바꿈을 아는 가짜 텍스트 측정: 단어 30px 셋(공백 0), 줄 높이 12.
struct Words;

impl TextLayoutHook for Words {
    fn min_content_width(
        &self,
        _: StableId,
        _: &TextIntrinsic,
    ) -> Result<stellar_layout::rounding::LayoutUnit, stellar_layout::intrinsic::MeasureError> {
        Ok(stellar_layout::rounding::LayoutUnit::from_px_int(30).unwrap())
    }

    fn layout(
        &self,
        _: StableId,
        _: &TextIntrinsic,
        width: Option<stellar_layout::rounding::LayoutUnit>,
    ) -> Result<stellar_layout::intrinsic::TextLineLayout, stellar_layout::intrinsic::MeasureError>
    {
        let unit = |px| stellar_layout::rounding::LayoutUnit::from_px_int(px).unwrap();
        let per_line = width.map_or(3, |w| (w.raw() / unit(30).raw()).clamp(1, 3));
        let lines = (3 + per_line - 1) / per_line;
        Ok(stellar_layout::intrinsic::TextLineLayout {
            width: unit(30 * per_line),
            height: unit(12 * lines),
            first_baseline: Some(unit(10)),
        })
    }
}

#[test]
fn text_measure_hook_lets_flex_shrink_wrap_text() {
    // 60px 흐름 컨테이너 안 Text(max-content 90). 줄바꿈 측정이 있으면 min-content(30)까지 줄어 60px로 그리고, 없으면
    // 줄바꿈 없는 크기(min = max = 90)라 줄지 않습니다.
    let mut model = empty_model();
    model
        .insert_node(node(11, Some(10), 0x10, frame(60., 100., 0., 0.)))
        .unwrap();
    model
        .insert_node(node(40, Some(11), 0x10, text_payload("aaa bbb ccc")))
        .unwrap();
    set_layout(&mut model, 11, &[("display", "flex")]);
    let widths: RefCell<Vec<Option<f32>>> = RefCell::new(Vec::new());
    let mut hook =
        |id: StableId, _: &TextPayload, _: Option<&ModifierStack>, _: bool, width: Option<f32>| {
            widths.borrow_mut().push(width);
            Some(TextInput {
                node: NodeInput::new(id, Transform2D::identity()),
                layout: crate::node::TextLayoutKey::from_bytes(id.to_bytes()),
                size: Size2::new(width.unwrap_or(90.0).min(90.0), 12.0).unwrap(),
            })
        };
    let measure = Words;
    let built = document_scene_source(
        &model,
        sid(10),
        SceneSourceOptions {
            text: Some(&mut hook),
            text_measure: Some(&measure),
            ..SceneSourceOptions::default()
        },
    )
    .unwrap();
    assert!(built.skipped.is_empty(), "{:?}", built.skipped);
    assert_eq!(widths.borrow().last(), Some(&Some(60.0)));
    let built = document_scene_source(
        &model,
        sid(10),
        SceneSourceOptions {
            text: Some(&mut hook),
            ..SceneSourceOptions::default()
        },
    )
    .unwrap();
    assert!(built.skipped.is_empty(), "{:?}", built.skipped);
    assert_eq!(widths.borrow().last(), Some(&Some(90.0)));
}

#[test]
fn variant_visibility_patch_is_a_layout_input() {
    use stellar_core_types::typed_value::TypedValue;
    use stellar_document::component::variant_override::VariantOverrideSet;
    let model = flex_model();
    let mut variant = VariantOverrideSet::new();
    variant.set_property(
        stellar_core_types::property::PropertyPath::with_keys(sid(21), vec![node_keys::VISIBLE]),
        TypedValue::Boolean(false),
    );
    let built = document_scene_source(
        &model,
        sid(10),
        SceneSourceOptions {
            variant: Some(&variant),
            ..SceneSourceOptions::default()
        },
    )
    .unwrap();
    // 21을 숨기는 변형: 22가 앞으로 옵니다.
    assert_eq!(transform_of(&built, 22), translate(5., 0.));
}

#[test]
fn reorder_preview_moves_siblings_with_the_real_layout_without_touching_the_document() {
    let model = flex_model();
    let built = document_scene_source(&model, sid(10), SceneSourceOptions::default()).unwrap();
    let preview = built
        .layout()
        .reorder_preview(sid(11), &[sid(23), sid(21), sid(22)], None)
        .unwrap();
    let changed = built.layout().changed_boxes(&preview);
    assert_eq!(changed, BTreeSet::from([sid(21), sid(22), sid(23)]));
    let x = |layout: &DocumentLayout, id: u64| {
        layout
            .layout_box(sid(id))
            .unwrap()
            .local_rect()
            .x()
            .to_px_f32()
            .get()
    };
    assert_eq!(x(&preview, 23), 5.0);
    assert_eq!(x(&preview, 21), 85.0);
    assert_eq!(x(&built.layout, 21), 5.0);
    // 순서가 같은 형제 집합이 아니면 거부합니다.
    assert!(built
        .layout()
        .reorder_preview(sid(11), &[sid(23), sid(21)], None)
        .is_err());
}

/// 2만 node 흐름 문서 편집 성능(ADR-0125). release에서 `--ignored --nocapture`로 잽니다.
#[test]
#[ignore = "성능 측정: cargo test -p stellar-scene --release -- --ignored flow_document_edit_performance"]
fn flow_document_edit_performance_with_20000_nodes() {
    let mut model = empty_model();
    // 가로 flex wrap 컨테이너 100개 × 자식 199개 + 컨테이너 = 20,000 node(root 제외).
    let rows = 100u64;
    let per_row = 199u64;
    let keys = OrderKey::evenly_spaced(per_row as usize + 1);
    for r in 0..rows {
        let container = 1_000 + r;
        model
            .insert_node(NodeRecord {
                id: sid(container),
                parent: Some(sid(10)),
                order_key: keys[(r % per_row) as usize].clone(),
                visible: true,
                locked: false,
                display_name: String::new(),
                payload: frame(800., 400., 0., (r * 400) as f32),
            })
            .unwrap();
        for c in 0..per_row {
            model
                .insert_node(NodeRecord {
                    id: sid(100_000 + r * 1_000 + c),
                    parent: Some(sid(container)),
                    order_key: keys[c as usize].clone(),
                    visible: true,
                    locked: false,
                    display_name: String::new(),
                    payload: frame(30., 20., 0., 0.),
                })
                .unwrap();
        }
    }
    for r in 0..rows {
        set_layout(
            &mut model,
            1_000 + r,
            &[
                ("display", "flex"),
                ("flex-wrap", "wrap"),
                ("column-gap", "2px"),
            ],
        );
    }
    let start = Instant::now();
    let mut built = document_scene_source(&model, sid(10), SceneSourceOptions::default()).unwrap();
    let initial = start.elapsed();
    assert!(built.skipped.is_empty());
    let mut state = DocumentState::new(model);
    let mut samples = Vec::new();
    for i in 0..20u64 {
        let changes = execute(
            &mut state,
            Command::SetProperty(SetProperty {
                node: sid(100_000 + 50 * 1_000 + 10),
                key: frame_keys::WIDTH,
                value: PropertyValue::NonNegative(
                    stellar_core_types::value::FiniteF32::new(30.0 + (i % 5) as f32).unwrap(),
                ),
            }),
        );
        let start = Instant::now();
        let edits = scene_edits(
            state.model(),
            &mut built,
            &changes,
            SceneSourceOptions::default(),
        )
        .unwrap();
        samples.push((start.elapsed(), edits.len()));
    }
    let mut times: Vec<_> = samples.iter().map(|s| s.0).collect();
    times.sort();
    println!(
        "20000 node 흐름 문서: 처음 변환 {:?}, 편집 한 번 scene_edits 중앙값 {:?}, 최대 {:?}, Upsert 수 {}",
        initial,
        times[times.len() / 2],
        times[times.len() - 1],
        samples[1].1
    );
}

#[test]
fn legacy_layout_mode_toggle_on_a_flow_container_stays_equal_to_a_full_rebuild() {
    // 흐름 컨테이너 11에 옛 `layout_mode`를 걸면 자식이 layout 입력에서 빠지고, 풀면 다시 흐름 자식으로 들어갑니다.
    let mut state = DocumentState::new(flex_model());
    let mut built =
        document_scene_source(state.model(), sid(10), SceneSourceOptions::default()).unwrap();
    let (mut scene, _) =
        IncrementalScene::build(built.source.clone(), state.revision(), &clock()).unwrap();
    let layout_mode = |value| {
        Command::SetProperty(SetProperty {
            node: sid(11),
            key: frame_keys::LAYOUT_MODE,
            value: PropertyValue::OptionalLayoutRef(value),
        })
    };
    apply_and_compare(
        &mut state,
        &mut built,
        &mut scene,
        layout_mode(Some(sid(99))),
    );
    assert!(built.layout().session().unwrap().node(sid(21)).is_none());
    apply_and_compare(&mut state, &mut built, &mut scene, layout_mode(None));
    assert!(built.layout().session().unwrap().node(sid(21)).is_some());
    assert_eq!(transform_of(&built, 22), translate(65., 0.));
    assert_eq!(built.layout().full_rebuilds(), 0);
}

#[test]
fn position_and_container_display_edits_stay_equal_to_a_full_rebuild() {
    // 흐름 자식을 absolute로 빼고 inset을 주고, 컨테이너 display를 block·grid로 바꿔도 증분 결과가 같습니다.
    let mut state = DocumentState::new(flex_model());
    let mut built =
        document_scene_source(state.model(), sid(10), SceneSourceOptions::default()).unwrap();
    let (mut scene, _) =
        IncrementalScene::build(built.source.clone(), state.revision(), &clock()).unwrap();
    for command in [
        set_layout_command(22, "position", "absolute"),
        set_layout_command(22, "left", "7px"),
    ] {
        apply_and_compare(&mut state, &mut built, &mut scene, command);
    }
    assert_eq!(
        built.layout().rule(state.model(), sid(22)),
        PlacementRule::Layout
    );
    let (_, _, _, _, x, _) = transform_of(&built, 22).components();
    assert_eq!(x, 7.0);
    // 22가 흐름에서 빠져 23이 21 바로 뒤로 당겨집니다.
    assert_eq!(transform_of(&built, 23), translate(68., 4.));
    for command in [
        set_layout_command(11, "display", "block"),
        set_layout_command(12, "position", "relative"),
        set_layout_command(11, "display", "grid"),
    ] {
        apply_and_compare(&mut state, &mut built, &mut scene, command);
    }
    assert_eq!(built.layout().full_rebuilds(), 0);
}
