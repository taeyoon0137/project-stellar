//! IX.3.2 검증: slot override는 인스턴스 소유 node를 투영할 뿐 부모를 바꾸지 않으며,
//! save/load 후에도 소유 관계와 투영이 유지됩니다.

use std::collections::BTreeMap;

use stellar_core_types::ids::{RuntimeEntityId, StableId};
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::{ComponentRef, NodeRef};
use stellar_document::component::definition::{
    ComponentDefinition, ComponentLibrary, ComponentSlotId, DefinitionNode, DefinitionTree,
};
use stellar_document::component::expansion::{
    ExpandedNodeId, InstanceExpansion, SceneIndex, ScenePayloads,
};
use stellar_document::component::instance::{ComponentInstancePayload, SlotOverride};
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

const SLOT: u64 = 600;
const INSTANCE: u64 = 1001;

fn card() -> ComponentRef {
    ComponentRef::new(stable(1))
}

/// Card(1): 100 ← 101(Text), 102(body host) ← 103(Text default).
fn library() -> ComponentLibrary {
    let mut tree = DefinitionTree::new(card());
    for (id, kind, parent, order) in [
        (100, NodeKind::Frame, None, 0x80),
        (101, NodeKind::Text, Some(100), 0x20),
        (102, NodeKind::Frame, Some(100), 0x40),
        (103, NodeKind::Text, Some(102), 0x40),
    ] {
        tree.insert(DefinitionNode {
            id: stable(id),
            kind,
            parent: parent.map(stable),
            order_key: key(order),
            visible: true,
            locked: false,
            display_name: format!("def-{id}"),
            payload: vec![id as u8],
        })
        .unwrap();
    }
    let mut definition =
        ComponentDefinition::new(stable(1), "Card".into(), NodeRef::new(stable(100)));
    definition
        .add_slot(ComponentSlotId::new(stable(SLOT)))
        .unwrap();
    let mut library = ComponentLibrary::new();
    library.insert(definition, tree).unwrap();
    library
        .add_slot(
            card(),
            ComponentSlot::new(
                ComponentSlotId::new(stable(SLOT)),
                NodeRef::new(stable(102)),
                [NodeKind::Text, NodeKind::Image, NodeKind::Frame],
                SlotCardinality::new(1, Some(4)).unwrap(),
                vec![NodeRef::new(stable(103))],
            ),
        )
        .unwrap();
    library
}

/// 저장용 scene node record(런타임 핸들 대신 부모 StableId).
#[derive(Debug, Clone, PartialEq, Eq)]
struct SceneRecord {
    id: StableId,
    kind: NodeKind,
    parent: Option<StableId>,
    order_key: OrderKey,
}

fn build_scene(records: &[SceneRecord]) -> NodeColumns {
    let mut columns = NodeColumns::new();
    let mut handles: BTreeMap<StableId, RuntimeEntityId> = BTreeMap::new();
    let mut pending: Vec<&SceneRecord> = records.iter().collect();
    while !pending.is_empty() {
        let before = pending.len();
        pending.retain(|record| {
            let parent = match record.parent {
                None => None,
                Some(p) => match handles.get(&p) {
                    Some(handle) => Some(*handle),
                    None => return true,
                },
            };
            let handle = columns.insert(NodeFields {
                stable_id: record.id,
                kind: record.kind,
                parent,
                order_key: record.order_key.clone(),
                visible: true,
                locked: false,
                display_name: String::new(),
            });
            handles.insert(record.id, handle);
            false
        });
        assert!(pending.len() < before, "부모 없는 record");
    }
    columns
}

/// scene record를 bytes로 저장(테스트 전용 단순 형식: 문서 serializer는 VI.10.2).
fn save_scene(records: &[SceneRecord], payloads: &ScenePayloads) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(records.len() as u32).to_le_bytes());
    for record in records {
        out.extend_from_slice(&record.id.to_bytes());
        out.extend_from_slice(&record.kind.as_u16().to_le_bytes());
        match record.parent {
            Some(p) => {
                out.push(1);
                out.extend_from_slice(&p.to_bytes());
            }
            None => out.push(0),
        }
        let k = record.order_key.as_bytes();
        out.extend_from_slice(&(k.len() as u32).to_le_bytes());
        out.extend_from_slice(k);
    }
    out.extend_from_slice(&(payloads.len() as u32).to_le_bytes());
    for (id, bytes) in payloads {
        out.extend_from_slice(&id.to_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(bytes);
    }
    out
}

fn load_scene(bytes: &[u8]) -> (Vec<SceneRecord>, ScenePayloads) {
    let mut pos = 0usize;
    let mut take = |n: usize| {
        let slice = &bytes[pos..pos + n];
        pos += n;
        slice
    };
    let id_of = |s: &[u8]| StableId::from_bytes(s.try_into().unwrap());
    let count = u32::from_le_bytes(take(4).try_into().unwrap()) as usize;
    let mut records = Vec::new();
    for _ in 0..count {
        let id = id_of(take(16));
        let kind = NodeKind::from_u16(u16::from_le_bytes(take(2).try_into().unwrap())).unwrap();
        let parent = match take(1)[0] {
            1 => Some(id_of(take(16))),
            _ => None,
        };
        let len = u32::from_le_bytes(take(4).try_into().unwrap()) as usize;
        let order_key = OrderKey::from_bytes(take(len).to_vec()).unwrap();
        records.push(SceneRecord {
            id,
            kind,
            parent,
            order_key,
        });
    }
    let payload_count = u32::from_le_bytes(take(4).try_into().unwrap()) as usize;
    let mut payloads = ScenePayloads::new();
    for _ in 0..payload_count {
        let id = id_of(take(16));
        let len = u32::from_le_bytes(take(4).try_into().unwrap()) as usize;
        payloads.insert(id, take(len).to_vec());
    }
    (records, payloads)
}

fn projection_snapshot(
    library: &ComponentLibrary,
    columns: &NodeColumns,
    payloads: &ScenePayloads,
) -> Vec<(ExpandedNodeId, NodeKind, Option<ExpandedNodeId>)> {
    let index = SceneIndex::build(columns, payloads);
    let expansion = InstanceExpansion::new(library, &index, stable(INSTANCE)).unwrap();
    expansion
        .walk()
        .unwrap()
        .into_iter()
        .map(|id| {
            let kind = expansion.node(&id).unwrap().kind();
            let parent = expansion.parent(&id).unwrap();
            (id, kind, parent)
        })
        .collect()
}

#[test]
fn ownership_survives_save_and_load() {
    // 검증 기준(IX.3.2): save/load 후 ownership 유지.
    let library = library();
    // scene: 1000(root) ← 1001(Card 인스턴스) ← 1002(Text), 1003(Frame) ← 1004(Image).
    let records = vec![
        SceneRecord {
            id: stable(1000),
            kind: NodeKind::Frame,
            parent: None,
            order_key: key(0x80),
        },
        SceneRecord {
            id: stable(INSTANCE),
            kind: NodeKind::ComponentInstance,
            parent: Some(stable(1000)),
            order_key: key(0x40),
        },
        SceneRecord {
            id: stable(1002),
            kind: NodeKind::Text,
            parent: Some(stable(INSTANCE)),
            order_key: key(0x20),
        },
        SceneRecord {
            id: stable(1003),
            kind: NodeKind::Frame,
            parent: Some(stable(INSTANCE)),
            order_key: key(0x40),
        },
        SceneRecord {
            id: stable(1004),
            kind: NodeKind::Image,
            parent: Some(stable(1003)),
            order_key: key(0x40),
        },
    ];
    let mut payload = ComponentInstancePayload::new(card());
    // 투영 순서(1003, 1002)는 scene OrderKey 순서(1002, 1003)와 다르게 둡니다.
    payload.set_slot_override(
        ComponentSlotId::new(stable(SLOT)),
        SlotOverride::new(vec![stable(1003), stable(1002)]).unwrap(),
    );
    let mut payloads = ScenePayloads::new();
    payloads.insert(stable(INSTANCE), payload.encode());

    let columns = build_scene(&records);
    let before = projection_snapshot(&library, &columns, &payloads);
    {
        let index = SceneIndex::build(&columns, &payloads);
        validate_scene_slot_overrides(&library, &index, stable(INSTANCE)).unwrap();
    }

    // save: library chunk bytes + scene bytes(instance payload 포함).
    let library_bytes = library.encode();
    let scene_bytes = save_scene(&records, &payloads);

    // load: 역순 삽입으로 런타임 핸들 배치가 달라져도 결과는 같아야 합니다.
    let loaded_library = ComponentLibrary::decode(&library_bytes).unwrap();
    let (mut loaded_records, loaded_payloads) = load_scene(&scene_bytes);
    loaded_records.reverse();
    let loaded_columns = build_scene(&loaded_records);
    let index = SceneIndex::build(&loaded_columns, &loaded_payloads);

    // 소유 관계: content node의 scene 부모는 여전히 인스턴스(정의 host로 바뀌지 않음).
    assert_eq!(index.parent(stable(1002)), Some(stable(INSTANCE)));
    assert_eq!(index.parent(stable(1003)), Some(stable(INSTANCE)));
    assert_eq!(index.parent(stable(1004)), Some(stable(1003)));
    validate_scene_slot_overrides(&loaded_library, &index, stable(INSTANCE)).unwrap();
    // 정의 tree에도 끼어들지 않음: host 102의 자식은 default content 103뿐.
    let tree = loaded_library.get(card()).unwrap().tree();
    assert_eq!(tree.children(stable(102)), &[stable(103)]);
    assert!(!tree.contains(stable(1002)));
    // 저장된 payload는 투영 목록 그대로.
    let decoded = ComponentInstancePayload::decode(&loaded_payloads[&stable(INSTANCE)]).unwrap();
    assert_eq!(
        decoded
            .slot_override(ComponentSlotId::new(stable(SLOT)))
            .unwrap()
            .content(),
        &[stable(1003), stable(1002)]
    );

    // 투영(가상 subtree)과 expansion 부모 관계도 save/load 전후 동일.
    let after = projection_snapshot(&loaded_library, &loaded_columns, &loaded_payloads);
    assert_eq!(after, before);
    assert!(after.contains(&(
        ExpandedNodeId::Owned(stable(1003)),
        NodeKind::Frame,
        Some(ExpandedNodeId::virtual_node(
            vec![stable(INSTANCE)],
            stable(102)
        )),
    )));
    // library와 scene bytes 재저장 byte-identical.
    assert_eq!(loaded_library.encode(), library_bytes);
    loaded_records.reverse();
    assert_eq!(save_scene(&loaded_records, &loaded_payloads), scene_bytes);
}
