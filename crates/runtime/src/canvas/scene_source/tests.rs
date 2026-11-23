//! 문서 → scene source 변환 테스트 (XI.2.1, XI.2.4, XI.2.5, XII.10.30).
#![cfg(test)]

use super::*;
use crate::window_session::test_fixtures::{sample_document, sid};
use stellar_commands::command::Command;
use stellar_commands::id::{CommandId, CommandOrigin};
use stellar_commands::property::{PropertyValue, SetProperty};
use stellar_commands::request::CommandRequest;
use stellar_commands::state::DocumentState;
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::clock::ManualClock;
use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::{ComponentRef, NodeRef};
use stellar_core_types::revision::DocumentRevision;
use stellar_core_types::value::{Size2, Transform2D, Vec2};
use stellar_document::component::definition::{
    ComponentDefinition, ComponentLibrary, DefinitionNode, DefinitionTree,
};
use stellar_document::component::instance::ComponentInstancePayload;
use stellar_document::frame::{keys as frame_keys, FramePayload};
use stellar_document::modifier::{ModifierEntry, ModifierId, ModifierStack};
use stellar_document::node_kind::NodeKind;
use stellar_document::records::NodeRecord;
use stellar_document::shape::{RectangleParams, ShapeGeometry};
use stellar_scene::boolean::signed_area;
use stellar_scene::dirty::{SceneDirtyKind, SceneDirtySet};
use stellar_scene::incremental::IncrementalScene;
use stellar_scene::node::SceneContent;

fn color(r: f32, g: f32, b: f32) -> Color {
    Color::new(ColorSpace::Srgb, r, g, b, 1.0).unwrap()
}

fn frame(w: f32, h: f32, x: f32, y: f32) -> FramePayload {
    FramePayload {
        size: Size2::new(w, h).unwrap(),
        transform: Transform2D::new(1., 0., 0., 1., x, y).unwrap(),
        background: Some(color(1.0, 1.0, 1.0)),
        ..FramePayload::default()
    }
}

fn rectangle(w: f32, h: f32) -> NodePayload {
    NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
        size: Size2::new(w, h).unwrap(),
        corner_radius: Default::default(),
    }))
}

fn record(n: u64, parent: u64, key: u8, visible: bool, payload: NodePayload) -> NodeRecord {
    NodeRecord {
        id: sid(n),
        parent: Some(sid(parent)),
        order_key: OrderKey::from_bytes(vec![key]).unwrap(),
        visible,
        locked: false,
        display_name: format!("n{n}"),
        payload,
    }
}

/// root 10 아래 Frame 11(Shape 21·22), 숨김 Frame 12(Shape 23), Frame 13.
fn document() -> DocumentModel {
    let mut model = sample_document();
    model
        .set_payload(sid(10), NodePayload::Frame(frame(800., 600., 0., 0.)))
        .unwrap();
    model
        .set_payload(sid(11), NodePayload::Frame(frame(100., 100., 10., 10.)))
        .unwrap();
    model
        .set_payload(sid(12), NodePayload::Frame(frame(100., 100., 200., 10.)))
        .unwrap();
    for record in [
        record(
            13,
            10,
            0x30,
            true,
            NodePayload::Frame(frame(50., 50., 400., 10.)),
        ),
        record(21, 11, 0x10, true, rectangle(10., 10.)),
        record(22, 11, 0x20, true, rectangle(20., 20.)),
        record(23, 12, 0x10, true, rectangle(30., 30.)),
    ] {
        model.insert_node(record).unwrap();
    }
    model.replace_flags(sid(12), false, false).unwrap();
    model
}

fn options<'a>() -> SceneSourceOptions<'a> {
    SceneSourceOptions::default()
}

fn clock() -> ManualClock {
    ManualClock::new(0)
}

#[test]
fn document_nodes_become_source_nodes_and_hidden_nodes_stay() {
    let model = document();
    let built = document_scene_source(&model, sid(10), options()).unwrap();
    assert_eq!(built.source.len(), 7);
    assert!(built.skipped.is_empty(), "{:?}", built.skipped);
    let hidden = built.source.get(SceneKey::node(sid(12))).unwrap();
    assert!(!hidden.kind.node_input().unwrap().visible);
    let (scene, _) = IncrementalScene::build(built.source, DocumentRevision(1), &clock()).unwrap();
    assert_eq!(scene.snapshot().node_count(), 7);
}

fn execute(state: &mut DocumentState, command: Command) -> stellar_commands::change::ChangeSet {
    let tx = CommandTransaction::new(CommandId::from_stable_id(sid(9_000)), CommandOrigin::User)
        .with_request(CommandRequest::new(state.revision(), command));
    state.execute_transaction(&tx).unwrap().changes
}

#[test]
fn change_set_drives_a_minimal_rebuild() {
    // XI.2.1 + XI.2.5: command change set → dirty set·source 편집 → 증분 build.
    let mut state = DocumentState::new(document());
    let mut built = document_scene_source(state.model(), sid(10), options()).unwrap();
    let (mut scene, _) =
        IncrementalScene::build(built.source.clone(), state.revision(), &clock()).unwrap();

    // 배경색: paint-only.
    let changes = execute(
        &mut state,
        Command::SetProperty(SetProperty {
            node: sid(13),
            key: frame_keys::BACKGROUND,
            value: PropertyValue::OptionalColor(Some(color(1.0, 0.0, 0.0))),
        }),
    );
    let invalidations = scene_invalidations(state.model(), &changes);
    let dirty = SceneDirtySet::from_invalidations(&invalidations, &built);
    assert_eq!(dirty.get(sid(13)), Some(SceneDirtyKind::Paint));
    let edits = scene_edits(state.model(), &mut built, &changes, options()).unwrap();
    assert_eq!(edits.len(), 1);
    let trace = scene
        .update(state.revision(), &edits, &dirty, &clock())
        .unwrap();
    assert_eq!(trace.stats.rebuilt, 1);
    assert_eq!(trace.stats.copied, 1);
    assert_eq!(trace.stats.shared, 5);

    // 이동: Frame 11과 자식 2개 재평가.
    let changes = execute(
        &mut state,
        Command::SetProperty(SetProperty {
            node: sid(11),
            key: frame_keys::TRANSFORM,
            value: PropertyValue::Transform(Transform2D::new(1., 0., 0., 1., 30., 40.).unwrap()),
        }),
    );
    let dirty =
        SceneDirtySet::from_invalidations(&scene_invalidations(state.model(), &changes), &built);
    // Frame transform의 문서 무효화 class는 paint입니다. 자손 world 값 변화는 증분 builder의 scope 비교가 잡습니다.
    assert_eq!(dirty.get(sid(11)), Some(SceneDirtyKind::Paint));
    let edits = scene_edits(state.model(), &mut built, &changes, options()).unwrap();
    let trace = scene
        .update(state.revision(), &edits, &dirty, &clock())
        .unwrap();
    assert_eq!(trace.stats.rebuilt, 3);
    // 결과는 처음부터 변환·build한 것과 같습니다.
    let fresh = document_scene_source(state.model(), sid(10), options()).unwrap();
    assert_eq!(built.source, fresh.source);
    let (full, _) = IncrementalScene::build(fresh.source, state.revision(), &clock()).unwrap();
    assert_eq!(
        scene.snapshot().semantic_hash(),
        full.snapshot().semantic_hash()
    );

    // 잠금: 문서 class는 비어 있지만 scene hit target이 바뀝니다.
    let changes = execute(
        &mut state,
        Command::SetProperty(SetProperty::locked(sid(22), true)),
    );
    let dirty =
        SceneDirtySet::from_invalidations(&scene_invalidations(state.model(), &changes), &built);
    assert_eq!(dirty.get(sid(22)), Some(SceneDirtyKind::Geometry));
}

fn boolean_document() -> DocumentModel {
    let mut model = sample_document();
    model.remove_leaf(sid(11)).unwrap();
    model.remove_leaf(sid(12)).unwrap();
    model
        .set_payload(sid(10), NodePayload::Frame(frame(800., 600., 0., 0.)))
        .unwrap();
    for record in [
        record(30, 10, 0x10, true, rectangle(10., 10.)),
        record(31, 10, 0x20, true, rectangle(10., 10.)),
    ] {
        model.insert_node(record).unwrap();
    }
    // operand 31은 (5, 5)로 옮긴 사각형입니다.
    let mut operand = ModifierStack::new(sid(31));
    operand
        .insert(ModifierEntry {
            id: ModifierId::new(sid(41)),
            order: OrderKey::from_bytes(vec![0x40]).unwrap(),
            kind: ModifierKind::Transform,
            enabled: true,
            payload: ModifierPayload::Transform(Transform2D::new(1., 0., 0., 1., 5., 5.).unwrap()),
        })
        .unwrap();
    model.set_modifier_stack(operand).unwrap();
    let mut host = ModifierStack::new(sid(30));
    host.insert(ModifierEntry {
        id: ModifierId::new(sid(40)),
        order: OrderKey::from_bytes(vec![0x40]).unwrap(),
        kind: ModifierKind::Boolean,
        enabled: true,
        payload: ModifierPayload::Boolean {
            op: BooleanOp::Subtract,
            operands: vec![NodeRef::new(sid(31))],
        },
    })
    .unwrap();
    model.set_modifier_stack(host).unwrap();
    model
}

#[test]
fn boolean_operands_are_absorbed_into_the_host_result() {
    // 검증 기준(XII.10.30): operand 단독 표시 없음, host 결과 path.
    let mut state = DocumentState::new(boolean_document());
    let mut built = document_scene_source(state.model(), sid(10), options()).unwrap();
    assert!(built.source.get(SceneKey::node(sid(31))).is_none());
    assert_eq!(built.operand_hosts[&sid(31)], vec![sid(30)]);
    let (mut scene, trace) =
        IncrementalScene::build(built.source.clone(), state.revision(), &clock()).unwrap();
    assert_eq!(trace.stats.boolean_failures, 0);
    assert_eq!(scene.snapshot().node_count(), 2);
    let area = |scene: &IncrementalScene| {
        let root = scene.snapshot().root(0).unwrap();
        let host = root.child(0).unwrap();
        let SceneContent::Path(path) = host.content() else {
            panic!("path");
        };
        signed_area(&path.geometry).abs()
    };
    assert_eq!(area(&scene), 75.0);
    // operand 크기 변경은 host geometry dirty이고 결과가 다시 계산됩니다.
    let changes = execute(
        &mut state,
        Command::SetProperty(SetProperty {
            node: sid(31),
            key: stellar_document::payload_metadata::shape_keys::WIDTH,
            value: PropertyValue::NonNegative(
                stellar_core_types::value::FiniteF32::new(2.).unwrap(),
            ),
        }),
    );
    let dirty =
        SceneDirtySet::from_invalidations(&scene_invalidations(state.model(), &changes), &built);
    assert_eq!(dirty.get(sid(30)), Some(SceneDirtyKind::Geometry));
    let edits = scene_edits(state.model(), &mut built, &changes, options()).unwrap();
    scene
        .update(state.revision(), &edits, &dirty, &clock())
        .unwrap();
    // A − B: B가 (5, 5)에서 폭 2가 되면 겹침은 2 × 5 = 10이므로 100 − 10 = 90입니다.
    assert_eq!(area(&scene), 90.0);
}

fn component_library() -> (ComponentLibrary, ComponentRef) {
    let component = ComponentRef::new(sid(500));
    let mut tree = DefinitionTree::new(component);
    let node = |id: u64, kind, parent: Option<u64>, key: u8, payload: NodePayload| DefinitionNode {
        id: sid(id),
        kind,
        parent: parent.map(sid),
        order_key: OrderKey::from_bytes(vec![key]).unwrap(),
        visible: true,
        locked: false,
        display_name: format!("d{id}"),
        payload: encode_payload(&payload),
    };
    tree.insert(node(
        501,
        NodeKind::Frame,
        None,
        0x40,
        NodePayload::Frame(frame(60., 30., 999., 999.)),
    ))
    .unwrap();
    tree.insert(node(
        502,
        NodeKind::Shape,
        Some(501),
        0x40,
        rectangle(8., 8.),
    ))
    .unwrap();
    tree.insert(node(
        503,
        NodeKind::Frame,
        Some(501),
        0x50,
        NodePayload::Frame(frame(20., 10., 30., 10.)),
    ))
    .unwrap();
    let mut library = ComponentLibrary::new();
    library
        .insert(
            ComponentDefinition::new(sid(500), "버튼".to_string(), NodeRef::new(sid(501))),
            tree,
        )
        .unwrap();
    (library, component)
}

fn instance_document(component: ComponentRef) -> DocumentModel {
    let mut model = sample_document();
    model.remove_leaf(sid(11)).unwrap();
    model.remove_leaf(sid(12)).unwrap();
    model
        .set_payload(sid(10), NodePayload::Frame(frame(800., 600., 0., 0.)))
        .unwrap();
    for (n, key) in [(60u64, 0x10u8), (61, 0x20)] {
        model
            .insert_node(record(
                n,
                10,
                key,
                true,
                NodePayload::ComponentInstance(ComponentInstancePayload::new(component)),
            ))
            .unwrap();
        let mut stack = ModifierStack::new(sid(n));
        stack
            .insert(ModifierEntry {
                id: ModifierId::new(sid(n + 100)),
                order: OrderKey::from_bytes(vec![0x40]).unwrap(),
                kind: ModifierKind::Transform,
                enabled: true,
                payload: ModifierPayload::Transform(
                    Transform2D::new(1., 0., 0., 1., n as f32, 100.).unwrap(),
                ),
            })
            .unwrap();
        model.set_modifier_stack(stack).unwrap();
    }
    model
}

#[test]
fn component_instances_expand_to_virtual_scene_nodes() {
    // XI.2.4: 인스턴스 가상 subtree를 scene으로 만듭니다(문서 복제 없음).
    let (library, component) = component_library();
    let model = instance_document(component);
    let built = document_scene_source(
        &model,
        sid(10),
        SceneSourceOptions {
            library: Some(&library),
            ..SceneSourceOptions::default()
        },
    )
    .unwrap();
    assert!(built.skipped.is_empty(), "{:?}", built.skipped);
    // root + 인스턴스 2개(Fragment) + 인스턴스마다 가상 node 3개.
    assert_eq!(built.source.len(), 1 + 2 * (1 + 3));
    let a = SceneKey::virtual_node(&[sid(60)], sid(501));
    let b = SceneKey::virtual_node(&[sid(61)], sid(501));
    assert!(built.source.get(a).is_some() && built.source.get(b).is_some());
    let (scene, _) = IncrementalScene::build(built.source, DocumentRevision(1), &clock()).unwrap();
    // Fragment는 scene node가 아닙니다: root + 2 × 3.
    assert_eq!(scene.snapshot().node_count(), 7);
    let root = scene.snapshot().root(0).unwrap();
    let first = root.child(0).unwrap();
    // 가상 node는 바깥 인스턴스를 source로 쓰고, 정의 root 원점은 인스턴스 배치로 바뀝니다.
    assert_eq!(first.source(), sid(60));
    assert_eq!(first.child(0).unwrap().source(), sid(60));
    assert_eq!(
        first.world_transform().apply(Vec2::ZERO).unwrap(),
        Vec2::new(60., 100.).unwrap()
    );
    let second = root.child(1).unwrap();
    assert_eq!(second.source(), sid(61));
    assert_eq!(
        second
            .child(1)
            .unwrap()
            .world_transform()
            .apply(Vec2::ZERO)
            .unwrap(),
        Vec2::new(91., 110.).unwrap()
    );
}

#[test]
fn variant_visibility_patches_apply_and_others_are_counted() {
    // XI.2.4: View Variant 결과 중 표시 여부 patch를 적용합니다.
    use stellar_core_types::property::PropertyPath;
    let model = document();
    let mut variant = VariantOverrideSet::new();
    variant.set_property(
        PropertyPath::with_keys(sid(13), vec![node_keys::VISIBLE]),
        TypedValue::Boolean(false),
    );
    variant.set_property(
        PropertyPath::with_keys(sid(13), vec![frame_keys::BACKGROUND]),
        TypedValue::Boolean(true),
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
    let node = built.source.get(SceneKey::node(sid(13))).unwrap();
    assert!(!node.kind.node_input().unwrap().visible);
    assert_eq!(built.unapplied_variant_patches, 1);
}

#[test]
fn ix9_component_fixture_expands_structurally() {
    // IX.9.1 fixture: 정의 payload bytes가 비어 있어 scene 내용은 만들 수 없지만, 가상 key·slot 투영·nested 인스턴스
    // 펼침과 건너뜀 이유를 확인합니다.
    use stellar_document::test_support::component_fixtures::{
        component_library_fixture, fixture_scene,
    };
    let fixture = component_library_fixture().unwrap();
    let (columns, payloads) = fixture_scene().unwrap();
    let index = SceneIndex::build(&columns, &payloads);
    let mut keys = 0;
    let mut reasons = BTreeSet::new();
    let root = stellar_document::test_support::component_fixtures::sid(
        stellar_document::test_support::component_fixtures::SCENE_ROOT,
    );
    for instance in index.children(root) {
        let expansion = InstanceExpansion::new(&fixture.library, &index, *instance).unwrap();
        let walk = expansion.walk().unwrap();
        assert!(!walk.is_empty());
        let mut out = Vec::new();
        let mut skipped = Vec::new();
        expand(
            &expansion,
            &expansion.root(),
            *instance,
            None,
            &mut out,
            &mut skipped,
        );
        keys += walk.len();
        reasons.extend(skipped.into_iter().map(|(_, reason)| reason));
    }
    assert!(keys > 5);
    assert_eq!(reasons, BTreeSet::from([SkipReason::Payload]));
}

/// change set 하나를 증분 경로로 반영하고, 처음부터 변환·build한 결과와 같은지 확인합니다.
fn apply_and_compare(
    state: &mut DocumentState,
    built: &mut DocumentSceneSource,
    scene: &mut IncrementalScene,
    command: Command,
) {
    let changes = execute(state, command);
    let dirty =
        SceneDirtySet::from_invalidations(&scene_invalidations(state.model(), &changes), &*built);
    let edits = scene_edits(state.model(), built, &changes, options()).unwrap();
    scene
        .update(state.revision(), &edits, &dirty, &clock())
        .unwrap();
    let fresh = document_scene_source(state.model(), sid(10), options()).unwrap();
    assert_eq!(built.source, fresh.source);
    assert_eq!(built.operand_hosts, fresh.operand_hosts);
    assert_eq!(built.skipped, fresh.skipped);
    let (full, _) = IncrementalScene::build(fresh.source, state.revision(), &clock()).unwrap();
    assert_eq!(
        scene.snapshot().semantic_hash(),
        full.snapshot().semantic_hash()
    );
}

fn modifier(id: u64, kind: ModifierKind, payload: ModifierPayload) -> ModifierEntry {
    ModifierEntry {
        id: ModifierId::new(sid(id)),
        order: OrderKey::from_bytes(vec![0x50]).unwrap(),
        kind,
        enabled: true,
        payload,
    }
}

#[test]
fn operand_moved_by_an_ancestor_frame_updates_the_boolean_host() {
    // operand 31을 Frame 11 아래로 옮긴 문서. Frame 11의 이동은 operand world 위치를 바꾸므로 host 결과가 바뀝니다.
    let mut model = boolean_document();
    model
        .insert_node(record(
            11,
            10,
            0x30,
            true,
            NodePayload::Frame(frame(100., 100., 0., 0.)),
        ))
        .unwrap();
    let mut state = DocumentState::new(model);
    execute(
        &mut state,
        Command::MoveNode(stellar_commands::node_ops::MoveNode {
            node: sid(31),
            new_parent: sid(11),
            order_key: OrderKey::from_bytes(vec![0x10]).unwrap(),
        }),
    );
    let mut built = document_scene_source(state.model(), sid(10), options()).unwrap();
    let (mut scene, _) =
        IncrementalScene::build(built.source.clone(), state.revision(), &clock()).unwrap();
    apply_and_compare(
        &mut state,
        &mut built,
        &mut scene,
        Command::SetProperty(SetProperty {
            node: sid(11),
            key: frame_keys::TRANSFORM,
            value: PropertyValue::Transform(Transform2D::new(1., 0., 0., 1., 2., 0.).unwrap()),
        }),
    );
}

#[test]
fn adding_a_boolean_modifier_absorbs_an_existing_node() {
    // Shape 21에 Shape 22를 operand로 둔 Boolean을 더하면 22는 단독 표시에서 빠집니다.
    let mut state = DocumentState::new(document());
    let mut built = document_scene_source(state.model(), sid(10), options()).unwrap();
    let (mut scene, _) =
        IncrementalScene::build(built.source.clone(), state.revision(), &clock()).unwrap();
    apply_and_compare(
        &mut state,
        &mut built,
        &mut scene,
        Command::InsertModifier(stellar_commands::modifier_ops::InsertModifier {
            node: sid(21),
            entry: modifier(
                60,
                ModifierKind::Boolean,
                ModifierPayload::Boolean {
                    op: BooleanOp::Union,
                    operands: vec![NodeRef::new(sid(22))],
                },
            ),
        }),
    );
    assert!(built.source.get(SceneKey::node(sid(22))).is_none());
    // 지우면 다시 나타납니다.
    apply_and_compare(
        &mut state,
        &mut built,
        &mut scene,
        Command::RemoveModifier(stellar_commands::modifier_ops::RemoveModifier {
            node: sid(21),
            modifier: ModifierId::new(sid(60)),
        }),
    );
    assert!(built.source.get(SceneKey::node(sid(22))).is_some());
}

#[test]
fn a_node_that_becomes_unsupported_leaves_its_parent_list() {
    // Frame 13에 modifier를 더하면 source에서 빠지고(UnsupportedModifiers), 부모 목록도 함께 바뀝니다.
    let mut state = DocumentState::new(document());
    let mut built = document_scene_source(state.model(), sid(10), options()).unwrap();
    let (mut scene, _) =
        IncrementalScene::build(built.source.clone(), state.revision(), &clock()).unwrap();
    apply_and_compare(
        &mut state,
        &mut built,
        &mut scene,
        Command::InsertModifier(stellar_commands::modifier_ops::InsertModifier {
            node: sid(13),
            entry: modifier(
                61,
                ModifierKind::Transform,
                ModifierPayload::Transform(Transform2D::identity()),
            ),
        }),
    );
    assert_eq!(
        built.skipped.get(&SceneKey::node(sid(13))),
        Some(&SkipReason::UnsupportedModifiers)
    );
    apply_and_compare(
        &mut state,
        &mut built,
        &mut scene,
        Command::RemoveModifier(stellar_commands::modifier_ops::RemoveModifier {
            node: sid(13),
            modifier: ModifierId::new(sid(61)),
        }),
    );
    assert!(built.skipped.is_empty());
}

#[test]
fn ancestor_move_of_an_operand_puts_the_host_in_the_dirty_set() {
    // XI.2.1: operand 31의 조상 Frame 11이 움직이면 host 30도 dirty set에 들어갑니다(SceneDependents::descendant_dependents).
    let mut model = boolean_document();
    model
        .insert_node(record(
            11,
            10,
            0x30,
            true,
            NodePayload::Frame(frame(100., 100., 0., 0.)),
        ))
        .unwrap();
    let mut state = DocumentState::new(model);
    execute(
        &mut state,
        Command::MoveNode(stellar_commands::node_ops::MoveNode {
            node: sid(31),
            new_parent: sid(11),
            order_key: OrderKey::from_bytes(vec![0x10]).unwrap(),
        }),
    );
    let built = document_scene_source(state.model(), sid(10), options()).unwrap();
    assert_eq!(built.descendant_dependents(sid(11)), vec![sid(30)]);
    assert_eq!(built.descendant_dependents(sid(10)), vec![sid(30)]);
    assert_eq!(built.descendant_dependents(sid(30)), Vec::<StableId>::new());
    let changes = execute(
        &mut state,
        Command::SetProperty(SetProperty {
            node: sid(11),
            key: frame_keys::TRANSFORM,
            value: PropertyValue::Transform(Transform2D::new(1., 0., 0., 1., 2., 0.).unwrap()),
        }),
    );
    let dirty =
        SceneDirtySet::from_invalidations(&scene_invalidations(state.model(), &changes), &built);
    // Frame transform은 문서 class가 paint입니다. 그래도 자손 operand의 host는 geometry dirty입니다.
    assert_eq!(dirty.get(sid(11)), Some(SceneDirtyKind::Paint));
    assert_eq!(dirty.get(sid(30)), Some(SceneDirtyKind::Geometry));
}

#[test]
fn operands_of_a_skipped_host_stay_absorbed_and_are_recorded() {
    // XII.10.30(ADR-0084 결정 7): host가 skip되면 operand는 흡수된 채 그리지 않고, 이유를 BooleanHostSkipped로 남깁니다.
    let mut state = DocumentState::new(boolean_document());
    let mut built = document_scene_source(state.model(), sid(10), options()).unwrap();
    let (mut scene, _) =
        IncrementalScene::build(built.source.clone(), state.revision(), &clock()).unwrap();
    assert!(built.skipped.is_empty(), "{:?}", built.skipped);
    apply_and_compare(
        &mut state,
        &mut built,
        &mut scene,
        Command::InsertModifier(stellar_commands::modifier_ops::InsertModifier {
            node: sid(30),
            entry: modifier(
                62,
                ModifierKind::StrokeExpand,
                ModifierPayload::StrokeExpand {
                    width: stellar_core_types::value::FiniteF32::new(1.).unwrap(),
                },
            ),
        }),
    );
    assert_eq!(
        built.skipped.get(&SceneKey::node(sid(30))),
        Some(&SkipReason::UnsupportedModifiers)
    );
    assert!(built.source.get(SceneKey::node(sid(31))).is_none());
    assert_eq!(
        built.skipped.get(&SceneKey::node(sid(31))),
        Some(&SkipReason::BooleanHostSkipped)
    );
    // root만 남습니다(operand를 단독으로 그리지 않음).
    assert_eq!(scene.snapshot().node_count(), 1);
    // host가 돌아오면 operand 기록도 사라집니다.
    apply_and_compare(
        &mut state,
        &mut built,
        &mut scene,
        Command::RemoveModifier(stellar_commands::modifier_ops::RemoveModifier {
            node: sid(30),
            modifier: ModifierId::new(sid(62)),
        }),
    );
    assert!(built.skipped.is_empty(), "{:?}", built.skipped);
    assert_eq!(scene.snapshot().node_count(), 2);
}
