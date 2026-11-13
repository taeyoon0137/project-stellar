//! IX.2.6 검증: detach 후 시각 의미 동일, undo 복원, 새 ID remap.

use std::collections::BTreeSet;

use stellar_core_types::clock::ManualClock;
use stellar_core_types::ids::{StableId, StableIdGenerator};
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::{ComponentRef, NodeRef};
use stellar_core_types::typed_value::TypedValue;
use stellar_document::component::definition::{
    ComponentDefinition, ComponentLibrary, ComponentPropertyId, ComponentSlotId, DefinitionNode,
    DefinitionTree,
};
use stellar_document::component::detach::{
    detach_instance, undo_detach, DetachError, DetachRecord,
};
use stellar_document::component::expansion::{
    ExpandedNodeId, ExpansionError, InstanceExpansion, SceneIndex, ScenePayloads,
};
use stellar_document::component::instance::{ComponentInstancePayload, OverrideKey, SlotOverride};
use stellar_document::component::slot::{ComponentSlot, SlotCardinality};
use stellar_document::component::slot_override::{
    validate_scene_slot_overrides, SlotOverrideError,
};
use stellar_document::node_columns::{NodeColumns, NodeFields};
use stellar_document::node_kind::NodeKind;

fn stable(ts: u64) -> StableId {
    StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
}

fn key(byte: u8) -> OrderKey {
    OrderKey::from_bytes(vec![byte]).unwrap()
}

fn slot(ts: u64) -> ComponentSlotId {
    ComponentSlotId::new(stable(ts))
}

fn component(ts: u64) -> ComponentRef {
    ComponentRef::new(stable(ts))
}

fn def_node(
    id: u64,
    kind: NodeKind,
    parent: Option<u64>,
    order: u8,
    payload: Vec<u8>,
) -> DefinitionNode {
    DefinitionNode {
        id: stable(id),
        kind,
        parent: parent.map(stable),
        order_key: key(order),
        visible: id != 107, // 107은 숨김 default content.
        locked: false,
        display_name: format!("def-{id}"),
        payload,
    }
}

/// Icon(2): 200 ← 201(Shape), 202(badge host, slot 650: Text 0..=1).
/// Card(1): 100 ← 101(Text), 102(body host, slot 600) ← 103(Text default),
///          105(footer host, slot 601) ← 107(Text default, 숨김),
///          106(Icon 인스턴스: badge = [108], property override) ← 108(Text).
fn library() -> ComponentLibrary {
    let mut library = ComponentLibrary::new();
    let mut icon = DefinitionTree::new(component(2));
    icon.insert(def_node(200, NodeKind::Frame, None, 0x80, vec![20]))
        .unwrap();
    icon.insert(def_node(201, NodeKind::Shape, Some(200), 0x40, vec![21]))
        .unwrap();
    icon.insert(def_node(202, NodeKind::Frame, Some(200), 0x60, vec![22]))
        .unwrap();
    let mut icon_def =
        ComponentDefinition::new(stable(2), "Icon".into(), NodeRef::new(stable(200)));
    icon_def.add_slot(slot(650)).unwrap();
    library.insert(icon_def, icon).unwrap();
    library
        .add_slot(
            component(2),
            ComponentSlot::new(
                slot(650),
                NodeRef::new(stable(202)),
                [NodeKind::Text],
                SlotCardinality::at_most(1),
                vec![],
            ),
        )
        .unwrap();

    let mut nested = ComponentInstancePayload::new(component(2));
    nested.set_slot_override(slot(650), SlotOverride::new(vec![stable(108)]).unwrap());
    nested.set_property_override(
        OverrideKey::direct(ComponentPropertyId::new(stable(900))),
        TypedValue::String("배지".into()),
    );
    let mut card = DefinitionTree::new(component(1));
    card.insert(def_node(100, NodeKind::Frame, None, 0x80, vec![10]))
        .unwrap();
    card.insert(def_node(101, NodeKind::Text, Some(100), 0x20, vec![11]))
        .unwrap();
    card.insert(def_node(102, NodeKind::Frame, Some(100), 0x40, vec![12]))
        .unwrap();
    card.insert(def_node(103, NodeKind::Text, Some(102), 0x40, vec![13]))
        .unwrap();
    card.insert(def_node(105, NodeKind::Frame, Some(100), 0x60, vec![15]))
        .unwrap();
    card.insert(def_node(107, NodeKind::Text, Some(105), 0x40, vec![17]))
        .unwrap();
    card.insert(def_node(
        106,
        NodeKind::ComponentInstance,
        Some(100),
        0x70,
        nested.encode(),
    ))
    .unwrap();
    card.insert(def_node(108, NodeKind::Text, Some(106), 0x40, vec![18]))
        .unwrap();
    let mut card_def =
        ComponentDefinition::new(stable(1), "Card".into(), NodeRef::new(stable(100)));
    card_def.add_slot(slot(600)).unwrap();
    card_def.add_slot(slot(601)).unwrap();
    library.insert(card_def, card).unwrap();
    library
        .add_slot(
            component(1),
            ComponentSlot::new(
                slot(600),
                NodeRef::new(stable(102)),
                [NodeKind::Text, NodeKind::Frame, NodeKind::ComponentInstance],
                SlotCardinality::new(1, Some(4)).unwrap(),
                vec![NodeRef::new(stable(103))],
            ),
        )
        .unwrap();
    library
        .add_slot(
            component(1),
            ComponentSlot::new(
                slot(601),
                NodeRef::new(stable(105)),
                [NodeKind::Text],
                SlotCardinality::at_most(1),
                vec![NodeRef::new(stable(107))],
            ),
        )
        .unwrap();
    library
}

const INSTANCE: u64 = 1001;

/// scene: 1000 ← 1001(Card, body = [1003, 1005, 1002]) ← 1002(Text), 1003(Frame) ← 1004(Text),
///        1005(Icon 인스턴스); 1000 ← 1006(Text 형제).
fn scene() -> (NodeColumns, ScenePayloads) {
    let mut columns = NodeColumns::new();
    let insert = |columns: &mut NodeColumns, id: u64, kind: NodeKind, parent, order: u8| {
        columns.insert(NodeFields {
            stable_id: stable(id),
            kind,
            parent,
            order_key: key(order),
            visible: true,
            locked: false,
            display_name: if id == INSTANCE {
                "내 카드".into()
            } else {
                String::new()
            },
        })
    };
    let root = insert(&mut columns, 1000, NodeKind::Frame, None, 0x80);
    let instance = insert(
        &mut columns,
        INSTANCE,
        NodeKind::ComponentInstance,
        Some(root),
        0x40,
    );
    insert(&mut columns, 1002, NodeKind::Text, Some(instance), 0x20);
    let frame = insert(&mut columns, 1003, NodeKind::Frame, Some(instance), 0x40);
    insert(&mut columns, 1004, NodeKind::Text, Some(frame), 0x40);
    insert(
        &mut columns,
        1005,
        NodeKind::ComponentInstance,
        Some(instance),
        0x60,
    );
    insert(&mut columns, 1006, NodeKind::Text, Some(root), 0x60);
    let mut payload = ComponentInstancePayload::new(component(1));
    payload.set_slot_override(
        slot(600),
        SlotOverride::new(vec![stable(1003), stable(1005), stable(1002)]).unwrap(),
    );
    let mut payloads = ScenePayloads::new();
    payloads.insert(stable(INSTANCE), payload.encode());
    payloads.insert(stable(1002), vec![0xA2]);
    payloads.insert(stable(1003), vec![0xA3]);
    payloads.insert(stable(1004), vec![0xA4]);
    payloads.insert(
        stable(1005),
        ComponentInstancePayload::new(component(2)).encode(),
    );
    (columns, payloads)
}

/// 시각 의미 한 줄: 깊이, 종류, 실효 표시 여부, payload(인스턴스는 참조 컴포넌트).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Visual {
    depth: usize,
    kind: NodeKind,
    visible: bool,
    payload: Vec<u8>,
    component: Option<ComponentRef>,
}

fn visual_of_expansion(
    expansion: &InstanceExpansion<'_>,
    id: &ExpandedNodeId,
    depth: usize,
    parent_visible: bool,
    out: &mut Vec<Visual>,
) {
    let node = expansion.node(id).unwrap();
    let visible = parent_visible && node.visible();
    let bytes = node.payload().unwrap_or(&[]).to_vec();
    let (payload, component) = if node.kind() == NodeKind::ComponentInstance {
        (
            Vec::new(),
            Some(
                ComponentInstancePayload::decode(&bytes)
                    .unwrap()
                    .component(),
            ),
        )
    } else {
        (bytes, None)
    };
    out.push(Visual {
        depth,
        kind: node.kind(),
        visible,
        payload,
        component,
    });
    for child in expansion.children(id).unwrap() {
        visual_of_expansion(expansion, &child, depth + 1, visible, out);
    }
}

fn visual_of_scene(
    library: &ComponentLibrary,
    index: &SceneIndex<'_>,
    node: StableId,
    depth: usize,
    parent_visible: bool,
    out: &mut Vec<Visual>,
) {
    let fields = index.fields(node).unwrap();
    let visible = parent_visible && fields.visible;
    let bytes = index.payload(node).unwrap_or(&[]).to_vec();
    if fields.kind == NodeKind::ComponentInstance {
        let payload = ComponentInstancePayload::decode(&bytes).unwrap();
        out.push(Visual {
            depth,
            kind: fields.kind,
            visible,
            payload: Vec::new(),
            component: Some(payload.component()),
        });
        let expansion = InstanceExpansion::new(library, index, node).unwrap();
        visual_of_expansion(&expansion, &expansion.root(), depth + 1, visible, out);
        return;
    }
    out.push(Visual {
        depth,
        kind: fields.kind,
        visible,
        payload: bytes,
        component: None,
    });
    for child in index.children(node) {
        visual_of_scene(library, index, *child, depth + 1, visible, out);
    }
}

/// scene 전체 의미 snapshot(StableId 기준, 런타임 핸들 제외).
fn snapshot(columns: &NodeColumns, payloads: &ScenePayloads) -> Vec<String> {
    let index = SceneIndex::build(columns, payloads);
    let mut lines: Vec<String> = columns
        .iter_live()
        .map(|handle| {
            let f = columns.get(handle).unwrap();
            format!(
                "{} {:?} parent={:?} key={:?} vis={} lock={} name={} payload={:?}",
                f.stable_id,
                f.kind,
                index.parent(f.stable_id),
                f.order_key.as_bytes(),
                f.visible,
                f.locked,
                f.display_name,
                payloads.get(&f.stable_id)
            )
        })
        .collect();
    lines.sort();
    lines.push(format!("payload entries={}", payloads.len()));
    lines
}

#[test]
fn detach_preserves_visual_meaning_and_undo_restores() {
    // 검증 기준(IX.2.6): detach 후 시각 의미 동일, undo 복원.
    let library = library();
    let library_bytes = library.encode();
    let (mut columns, mut payloads) = scene();
    {
        let index = SceneIndex::build(&columns, &payloads);
        validate_scene_slot_overrides(&library, &index, stable(INSTANCE)).unwrap();
    }
    let before_snapshot = snapshot(&columns, &payloads);
    let before_visual = {
        let index = SceneIndex::build(&columns, &payloads);
        let expansion = InstanceExpansion::new(&library, &index, stable(INSTANCE)).unwrap();
        let instance_visible = index.fields(stable(INSTANCE)).unwrap().visible;
        let mut out = Vec::new();
        visual_of_expansion(&expansion, &expansion.root(), 0, instance_visible, &mut out);
        out
    };

    let clock = ManualClock::new(1_700_000_000_000 * 1_000_000);
    let mut generator = StableIdGenerator::new(42);
    let record = detach_instance(
        &mut columns,
        &mut payloads,
        &library,
        stable(INSTANCE),
        &mut generator,
        &clock,
    )
    .unwrap();

    // ComponentRef 제거: 인스턴스 자리는 같은 StableId의 실제 Frame.
    let index = SceneIndex::build(&columns, &payloads);
    let detached = index.fields(stable(INSTANCE)).unwrap();
    assert_eq!(detached.kind, NodeKind::Frame);
    assert_eq!(detached.display_name, "내 카드");
    assert_eq!(index.parent(stable(INSTANCE)), Some(stable(1000)));
    assert!(matches!(
        InstanceExpansion::new(&library, &index, stable(INSTANCE)),
        Err(ExpansionError::NotAnInstance { .. })
    ));

    // 시각 의미 동일: 가상 subtree와 detach 후 실제 subtree의 구조·종류·표시·payload 일치.
    let mut after_visual = Vec::new();
    visual_of_scene(
        &library,
        &index,
        stable(INSTANCE),
        0,
        true,
        &mut after_visual,
    );
    assert_eq!(after_visual, before_visual);

    // 새 ID remap: 복제 node는 모두 새 id(정의·scene 기존 id와 겹치지 않음), override된
    // slot의 default content(103)는 복제하지 않음.
    let definition_ids: BTreeSet<StableId> = library
        .iter()
        .flat_map(|entry| entry.tree().iter().map(|n| n.id))
        .collect();
    let created: BTreeSet<StableId> = record.created().iter().copied().collect();
    assert_eq!(created.len(), 6, "101, 102, 105, 107, 106, 108");
    assert!(created.is_disjoint(&definition_ids));
    assert!(!record.remap().iter().any(|(old, _)| *old == stable(103)));
    let new_of = |old: u64| {
        record
            .remap()
            .iter()
            .find(|(o, _)| *o == stable(old))
            .map(|(_, n)| *n)
            .unwrap()
    };
    assert_eq!(new_of(100), stable(INSTANCE));
    // nested 인스턴스 복제본: 내부 참조(slot content 108)는 새 id, 외부(컴포넌트·property)는 유지.
    let nested_copy =
        ComponentInstancePayload::decode(index.payload(new_of(106)).unwrap()).unwrap();
    assert_eq!(nested_copy.component(), component(2));
    assert_eq!(
        nested_copy.slot_override(slot(650)).unwrap().content(),
        &[new_of(108)]
    );
    assert_eq!(
        nested_copy.property_override(&OverrideKey::direct(ComponentPropertyId::new(stable(900)))),
        Some(&TypedValue::String("배지".into()))
    );
    validate_scene_slot_overrides(&library, &index, new_of(106)).unwrap();
    // slot content는 복제가 아니라 복제 host 아래로 이동(투영 순서 = 자식 순서).
    assert_eq!(
        index.children(new_of(102)),
        &[stable(1003), stable(1005), stable(1002)]
    );
    assert_eq!(index.parent(stable(1004)), Some(stable(1003)));
    // library(정의)는 바뀌지 않음.
    assert_eq!(library.encode(), library_bytes);
    drop(index);

    // undo 복원: scene 의미 snapshot이 detach 전과 동일.
    undo_detach(&mut columns, &mut payloads, &record).unwrap();
    assert_eq!(snapshot(&columns, &payloads), before_snapshot);
    let index = SceneIndex::build(&columns, &payloads);
    validate_scene_slot_overrides(&library, &index, stable(INSTANCE)).unwrap();
}

#[test]
fn detach_is_deterministic_for_same_seed() {
    let library = library();
    let run = || {
        let (mut columns, mut payloads) = scene();
        let clock = ManualClock::new(5_000_000);
        let mut generator = StableIdGenerator::new(9);
        let record = detach_instance(
            &mut columns,
            &mut payloads,
            &library,
            stable(INSTANCE),
            &mut generator,
            &clock,
        )
        .unwrap();
        (snapshot(&columns, &payloads), record.remap().to_vec())
    };
    assert_eq!(run(), run());
}

// --- nested slot host (리뷰 수정: 가려진 안쪽 slot content) ---

const PANEL_INSTANCE: u64 = 1101;

/// Panel(5): 500 ← 501(Text), 502(outer host, slot 660: Frame|Text 0..=3)
///           ← 503(Frame default, inner host, slot 661: Text 0..=2) ← 504(Text default).
///
/// 안쪽 host 503은 바깥 slot 660의 default content 안에 있습니다(nested slot host).
fn nested_library() -> ComponentLibrary {
    let mut library = ComponentLibrary::new();
    let mut panel = DefinitionTree::new(component(5));
    for (id, kind, parent, order) in [
        (500, NodeKind::Frame, None, 0x80u8),
        (501, NodeKind::Text, Some(500), 0x20),
        (502, NodeKind::Frame, Some(500), 0x40),
        (503, NodeKind::Frame, Some(502), 0x40),
        (504, NodeKind::Text, Some(503), 0x40),
    ] {
        panel
            .insert(def_node(id, kind, parent, order, vec![(id - 450) as u8]))
            .unwrap();
    }
    let mut definition =
        ComponentDefinition::new(stable(5), "Panel".into(), NodeRef::new(stable(500)));
    definition.add_slot(slot(660)).unwrap();
    definition.add_slot(slot(661)).unwrap();
    library.insert(definition, panel).unwrap();
    library
        .add_slot(
            component(5),
            ComponentSlot::new(
                slot(660),
                NodeRef::new(stable(502)),
                [NodeKind::Frame, NodeKind::Text],
                SlotCardinality::at_most(3),
                vec![NodeRef::new(stable(503))],
            ),
        )
        .unwrap();
    library
        .add_slot(
            component(5),
            ComponentSlot::new(
                slot(661),
                NodeRef::new(stable(503)),
                [NodeKind::Text],
                SlotCardinality::at_most(2),
                vec![NodeRef::new(stable(504))],
            ),
        )
        .unwrap();
    library
}

/// scene: 1000 ← 1101(Panel) ← 1102(Text 0x20), 1103(Text 0x40). slot override는 인자로.
fn nested_scene(overrides: &[(u64, &[u64])]) -> (NodeColumns, ScenePayloads) {
    let mut columns = NodeColumns::new();
    let insert = |columns: &mut NodeColumns, id: u64, kind: NodeKind, parent, order: u8| {
        columns.insert(NodeFields {
            stable_id: stable(id),
            kind,
            parent,
            order_key: key(order),
            visible: true,
            locked: false,
            display_name: String::new(),
        })
    };
    let root = insert(&mut columns, 1000, NodeKind::Frame, None, 0x80);
    let instance = insert(
        &mut columns,
        PANEL_INSTANCE,
        NodeKind::ComponentInstance,
        Some(root),
        0x40,
    );
    insert(&mut columns, 1102, NodeKind::Text, Some(instance), 0x20);
    insert(&mut columns, 1103, NodeKind::Text, Some(instance), 0x40);
    let mut payload = ComponentInstancePayload::new(component(5));
    for (slot_id, content) in overrides {
        payload.set_slot_override(
            slot(*slot_id),
            SlotOverride::new(content.iter().copied().map(stable).collect()).unwrap(),
        );
    }
    let mut payloads = ScenePayloads::new();
    payloads.insert(stable(PANEL_INSTANCE), payload.encode());
    payloads.insert(stable(1102), vec![0xB2]);
    payloads.insert(stable(1103), vec![0xB3]);
    (columns, payloads)
}

fn expansion_visual(
    library: &ComponentLibrary,
    columns: &NodeColumns,
    payloads: &ScenePayloads,
) -> Vec<Visual> {
    let index = SceneIndex::build(columns, payloads);
    let expansion = InstanceExpansion::new(library, &index, stable(PANEL_INSTANCE)).unwrap();
    let mut out = Vec::new();
    visual_of_expansion(&expansion, &expansion.root(), 0, true, &mut out);
    out
}

fn scene_visual(
    library: &ComponentLibrary,
    columns: &NodeColumns,
    payloads: &ScenePayloads,
) -> Vec<Visual> {
    let index = SceneIndex::build(columns, payloads);
    let mut out = Vec::new();
    visual_of_scene(library, &index, stable(PANEL_INSTANCE), 0, true, &mut out);
    out
}

fn detach_panel(
    library: &ComponentLibrary,
    columns: &mut NodeColumns,
    payloads: &mut ScenePayloads,
) -> Result<DetachRecord, DetachError> {
    let clock = ManualClock::new(1_700_000_000_000 * 1_000_000);
    let mut generator = StableIdGenerator::new(77);
    detach_instance(
        columns,
        payloads,
        library,
        stable(PANEL_INSTANCE),
        &mut generator,
        &clock,
    )
}

fn copy_of(record: &DetachRecord, old: u64) -> Option<StableId> {
    record
        .remap()
        .iter()
        .find(|(o, _)| *o == stable(old))
        .map(|(_, n)| *n)
}

#[test]
fn nested_slot_host_content_is_reparented_and_undo_restores() {
    // 바깥 slot(660)은 default, 안쪽 slot(661)만 override: 안쪽 host 복제본 아래로 content 이동.
    let library = nested_library();
    let (mut columns, mut payloads) = nested_scene(&[(661, &[1103, 1102])]);
    {
        let index = SceneIndex::build(&columns, &payloads);
        validate_scene_slot_overrides(&library, &index, stable(PANEL_INSTANCE)).unwrap();
    }
    let before_snapshot = snapshot(&columns, &payloads);
    let before_visual = expansion_visual(&library, &columns, &payloads);

    let record = detach_panel(&library, &mut columns, &mut payloads).unwrap();
    assert_eq!(
        scene_visual(&library, &columns, &payloads),
        before_visual,
        "detach 후 시각 의미 동일"
    );
    let inner_host = copy_of(&record, 503).unwrap();
    let index = SceneIndex::build(&columns, &payloads);
    assert_eq!(
        index.children(inner_host),
        &[stable(1103), stable(1102)],
        "안쪽 host 복제본 아래 투영 순서대로"
    );
    assert_eq!(index.parent(inner_host), copy_of(&record, 502));
    assert!(
        copy_of(&record, 504).is_none(),
        "override된 slot의 default 미복제"
    );
    drop(index);

    undo_detach(&mut columns, &mut payloads, &record).unwrap();
    assert_eq!(snapshot(&columns, &payloads), before_snapshot);
    let index = SceneIndex::build(&columns, &payloads);
    validate_scene_slot_overrides(&library, &index, stable(PANEL_INSTANCE)).unwrap();
}

#[test]
fn hidden_nested_slot_content_is_rejected_before_detach() {
    // 바깥 slot(660)을 override하면 안쪽 host(503)는 가려집니다. 가려진 slot(661)에 content가
    // 있으면 투영 validator가 거부하고 detach는 scene을 바꾸지 않습니다(수정 전에는 1103이 끊긴
    // 부모 핸들로 남고 undo가 복원하지 못했습니다).
    let library = nested_library();
    let (mut columns, mut payloads) = nested_scene(&[(660, &[1102]), (661, &[1103])]);
    let expected = SlotOverrideError::HiddenSlotContent {
        instance: stable(PANEL_INSTANCE),
        slot: slot(661),
        hidden_by: slot(660),
    };
    {
        let index = SceneIndex::build(&columns, &payloads);
        assert_eq!(
            validate_scene_slot_overrides(&library, &index, stable(PANEL_INSTANCE)).unwrap_err(),
            expected
        );
    }
    let before = snapshot(&columns, &payloads);
    assert_eq!(
        detach_panel(&library, &mut columns, &mut payloads).unwrap_err(),
        DetachError::Slots(expected)
    );
    assert_eq!(snapshot(&columns, &payloads), before, "거부 시 scene 불변");
}

#[test]
fn empty_hidden_override_detaches_and_undo_restores() {
    // 가려진 slot의 빈 override는 허용됩니다(옮길 content가 없음).
    let library = nested_library();
    let (mut columns, mut payloads) = nested_scene(&[(660, &[1102, 1103]), (661, &[])]);
    {
        let index = SceneIndex::build(&columns, &payloads);
        validate_scene_slot_overrides(&library, &index, stable(PANEL_INSTANCE)).unwrap();
    }
    let before_snapshot = snapshot(&columns, &payloads);
    let before_visual = expansion_visual(&library, &columns, &payloads);
    let record = detach_panel(&library, &mut columns, &mut payloads).unwrap();
    assert_eq!(scene_visual(&library, &columns, &payloads), before_visual);
    assert!(
        copy_of(&record, 503).is_none() && copy_of(&record, 504).is_none(),
        "가려진 안쪽 host·default는 복제하지 않음"
    );
    let index = SceneIndex::build(&columns, &payloads);
    assert_eq!(
        index.children(copy_of(&record, 502).unwrap()),
        &[stable(1102), stable(1103)]
    );
    drop(index);
    undo_detach(&mut columns, &mut payloads, &record).unwrap();
    assert_eq!(snapshot(&columns, &payloads), before_snapshot);
}

#[test]
fn undo_rejects_foreign_child_under_detached_subtree() {
    // detach 뒤 복제 subtree 아래에 다른 node가 생기면 undo가 그 node를 고아로 만들 수 있으므로
    // 아무것도 바꾸지 않고 거부합니다.
    let library = nested_library();
    let (mut columns, mut payloads) = nested_scene(&[(661, &[1102, 1103])]);
    let record = detach_panel(&library, &mut columns, &mut payloads).unwrap();
    let host_copy = copy_of(&record, 502).unwrap();
    let handle = SceneIndex::build(&columns, &payloads)
        .runtime_id(host_copy)
        .unwrap();
    columns.insert(NodeFields {
        stable_id: stable(1199),
        kind: NodeKind::Text,
        parent: Some(handle),
        order_key: key(0x70),
        visible: true,
        locked: false,
        display_name: String::new(),
    });
    let before = snapshot(&columns, &payloads);
    assert_eq!(
        undo_detach(&mut columns, &mut payloads, &record).unwrap_err(),
        DetachError::StateMismatch { node: stable(1199) }
    );
    assert_eq!(snapshot(&columns, &payloads), before);
}

#[test]
fn undo_rejects_moved_content_reparented_outside_detached_subtree() {
    // 리뷰 보강: detach로 slot host 복제본 아래로 옮긴 content(1102)가 그 뒤 복제 subtree 밖(scene
    // root 1000)으로 옮겨지면, undo가 그 node를 인스턴스 아래로 되돌릴 근거가 없으므로 아무것도
    // 바꾸지 않고 거부합니다(check_detached_shape의 moved 분기).
    let library = nested_library();
    let (mut columns, mut payloads) = nested_scene(&[(661, &[1102, 1103])]);
    let record = detach_panel(&library, &mut columns, &mut payloads).unwrap();
    let (moved, scene_root) = {
        let index = SceneIndex::build(&columns, &payloads);
        (
            index.runtime_id(stable(1102)).unwrap(),
            index.runtime_id(stable(1000)).unwrap(),
        )
    };
    columns.set_parent(moved, Some(scene_root));
    columns.set_order_key(moved, key(0x10));
    let before = snapshot(&columns, &payloads);
    assert_eq!(
        undo_detach(&mut columns, &mut payloads, &record).unwrap_err(),
        DetachError::StateMismatch { node: stable(1102) }
    );
    assert_eq!(snapshot(&columns, &payloads), before, "거부 시 scene 불변");
}
