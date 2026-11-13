//! IX.2.2 검증: 5만 인스턴스를 펼쳐도 정의 node가 물리 복제되지 않습니다.

use std::collections::HashSet;

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::{ComponentRef, NodeRef};
use stellar_document::component::definition::{
    ComponentDefinition, ComponentLibrary, DefinitionNode, DefinitionTree,
};
use stellar_document::component::expansion::{InstanceExpansion, SceneIndex, ScenePayloads};
use stellar_document::component::instance::ComponentInstancePayload;
use stellar_document::node_columns::{NodeColumns, NodeFields};
use stellar_document::node_kind::NodeKind;

const INSTANCE_COUNT: usize = 50_000;

fn stable(ts: u64) -> StableId {
    StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
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
        order_key: OrderKey::from_bytes(vec![order]).unwrap(),
        visible: true,
        locked: false,
        display_name: format!("def-{id}"),
        payload,
    }
}

/// Icon(2): 200(Frame) ← 201(Shape), 202(Path).
/// Card(1): 100(Frame) ← 101(Text), 102(Image), 103(Frame) ← 104(Text), 105(Shape),
///          106(ComponentInstance of Icon), 107(Text).
fn library() -> ComponentLibrary {
    let mut library = ComponentLibrary::new();
    let mut icon = DefinitionTree::new(ComponentRef::new(stable(2)));
    icon.insert(def_node(200, NodeKind::Frame, None, 0x80, vec![]))
        .unwrap();
    icon.insert(def_node(201, NodeKind::Shape, Some(200), 0x20, vec![]))
        .unwrap();
    icon.insert(def_node(202, NodeKind::Path, Some(200), 0x40, vec![]))
        .unwrap();
    library
        .insert(
            ComponentDefinition::new(stable(2), "Icon".into(), NodeRef::new(stable(200))),
            icon,
        )
        .unwrap();

    let mut card = DefinitionTree::new(ComponentRef::new(stable(1)));
    card.insert(def_node(100, NodeKind::Frame, None, 0x80, vec![]))
        .unwrap();
    card.insert(def_node(101, NodeKind::Text, Some(100), 0x10, vec![]))
        .unwrap();
    card.insert(def_node(102, NodeKind::Image, Some(100), 0x20, vec![]))
        .unwrap();
    card.insert(def_node(103, NodeKind::Frame, Some(100), 0x30, vec![]))
        .unwrap();
    card.insert(def_node(104, NodeKind::Text, Some(103), 0x10, vec![]))
        .unwrap();
    card.insert(def_node(105, NodeKind::Shape, Some(103), 0x20, vec![]))
        .unwrap();
    card.insert(def_node(
        106,
        NodeKind::ComponentInstance,
        Some(103),
        0x30,
        ComponentInstancePayload::new(ComponentRef::new(stable(2))).encode(),
    ))
    .unwrap();
    card.insert(def_node(107, NodeKind::Text, Some(100), 0x40, vec![]))
        .unwrap();
    library
        .insert(
            ComponentDefinition::new(stable(1), "Card".into(), NodeRef::new(stable(100))),
            card,
        )
        .unwrap();
    library
}

fn definition_node_count(library: &ComponentLibrary) -> usize {
    library.iter().map(|entry| entry.tree().len()).sum()
}

#[test]
fn fifty_thousand_instances_share_one_definition_copy() {
    // 검증 기준(IX.2.2): 5만 instance에서 definition node count가 물리 복제되지 않습니다.
    let library = library();
    let definition_nodes = definition_node_count(&library);
    assert_eq!(definition_nodes, 11);

    let mut columns = NodeColumns::new();
    let root = columns.insert(NodeFields {
        stable_id: stable(1_000_000),
        kind: NodeKind::Frame,
        parent: None,
        order_key: OrderKey::between(None, None),
        visible: true,
        locked: false,
        display_name: "page".into(),
    });
    let mut payloads = ScenePayloads::new();
    let instance_payload = ComponentInstancePayload::new(ComponentRef::new(stable(1))).encode();
    let keys = OrderKey::evenly_spaced(INSTANCE_COUNT);
    let mut instances = Vec::with_capacity(INSTANCE_COUNT);
    for (i, key) in keys.into_iter().enumerate() {
        let id = stable(2_000_000 + i as u64);
        columns.insert(NodeFields {
            stable_id: id,
            kind: NodeKind::ComponentInstance,
            parent: Some(root),
            order_key: key,
            visible: true,
            locked: false,
            display_name: String::new(),
        });
        payloads.insert(id, instance_payload.clone());
        instances.push(id);
    }
    let scene_nodes = columns.len();
    assert_eq!(scene_nodes, INSTANCE_COUNT + 1);

    let index = SceneIndex::build(&columns, &payloads);
    let mut virtual_nodes = 0usize;
    let mut distinct_sources: HashSet<usize> = HashSet::new();
    for id in &instances {
        let expansion = InstanceExpansion::new(&library, &index, *id).unwrap();
        for node_id in expansion.walk().unwrap() {
            let node = expansion.node(&node_id).unwrap();
            let definition = node
                .definition_node()
                .expect("가상 node는 정의 node를 참조");
            distinct_sources.insert(definition as *const DefinitionNode as usize);
            virtual_nodes += 1;
        }
    }

    // 모든 인스턴스가 정의 subtree(Card 8 + nested Icon 3)를 가상으로 노출합니다.
    assert_eq!(virtual_nodes, INSTANCE_COUNT * definition_nodes);
    // 그러나 서로 다른 정의 node 저장소 주소는 정의 node 수만큼뿐 — 인스턴스별 복제 없음.
    assert_eq!(distinct_sources.len(), definition_nodes);
    // scene node 수와 정의 node 수는 펼치기 전후로 동일합니다.
    assert_eq!(columns.len(), scene_nodes);
    assert_eq!(definition_node_count(&library), definition_nodes);
    assert_eq!(payloads.len(), INSTANCE_COUNT);
}
