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
use stellar_document::component::detach::{detach_instance, undo_detach};
use stellar_document::component::expansion::{
    ExpandedNodeId, ExpansionError, InstanceExpansion, SceneIndex, ScenePayloads,
};
use stellar_document::component::instance::{ComponentInstancePayload, OverrideKey, SlotOverride};
use stellar_document::component::slot::{ComponentSlot, SlotCardinality};
use stellar_document::component::slot_override::validate_scene_slot_overrides;
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
