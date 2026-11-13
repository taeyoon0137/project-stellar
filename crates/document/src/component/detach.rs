//! 명시적 detach command (IX.2.6).
//!
//! 인스턴스의 가상 subtree를 실제 scene node로 복제하고 `ComponentRef`를 제거합니다. 자유
//! 구조 편집이 필요할 때 사용자가 명시적으로 실행하는 command입니다(기술 아키텍처 15.1).
//!
//! - 인스턴스 node 자신은 정의 root Frame의 복제본으로 바뀝니다. 외부 참조가 깨지지 않도록
//!   인스턴스 StableId를 그대로 쓰고, 나머지 복제 node에는 [`IdRemapTable`]로 새 StableId를
//!   발급합니다(III.1.5). nested 인스턴스 payload의 내부 참조(slot content)도 새 id로
//!   재매핑합니다.
//! - override된 slot의 default content는 복제하지 않고, 인스턴스 소유 slot content를 복제된
//!   slot host 아래로 실제 reparent합니다(투영 순서 = 자식 순서).
//! - nested slot host(IX.3.1): 바깥 slot이 override되지 않았으면 그 default content와 안쪽
//!   slot host가 복제되고 안쪽 slot content가 그 복제 host 아래로 옮겨집니다. 바깥 slot이
//!   override되면 안쪽 host는 가려져 복제되지 않습니다 — 가려진 slot의 content는 투영
//!   validator가 거부하고, 적용 전에 인스턴스의 모든 소유 node가 옮겨질 자리를 갖는지 다시
//!   확인해 부모 핸들이 끊긴 node를 남기지 않습니다.
//! - nested 인스턴스는 한 단계만 풀립니다 — 정의 tree 안 nested 인스턴스는 실제 인스턴스
//!   node로 복제되어 계속 자기 정의를 참조합니다.
//! - [`detach_instance`]는 되돌리기 정보([`DetachRecord`])를 돌려주고 [`undo_detach`]가 그
//!   정보로 원래 상태를 복원합니다.
//!
//! Phase VII command 계층(closed enum, revision, history)이 생기면 이 함수 쌍을 그 command의
//! apply/invert로 등록합니다.

use std::collections::{HashMap, HashSet};
use std::fmt;

use stellar_core_types::clock::MonotonicClock;
use stellar_core_types::ids::{IdRemapTable, RuntimeEntityId, StableId, StableIdGenerator};
use stellar_core_types::order_key::OrderKey;

use super::definition::{ComponentLibrary, DefinitionNode};
use super::expansion::{ExpansionError, SceneIndex, ScenePayloads};
use super::instance::{ComponentInstancePayload, InstancePayloadError, SlotOverride};
use super::slot_override::{validate_scene_slot_overrides, SlotOverrideError};
use crate::node_columns::{NodeColumns, NodeFields};
use crate::node_kind::NodeKind;

/// detach·undo 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetachError {
    /// 인스턴스를 해석할 수 없습니다.
    Expansion(ExpansionError),
    /// 인스턴스 slot override가 투영 규칙을 어깁니다.
    Slots(SlotOverrideError),
    /// 정의 tree 안 nested 인스턴스 payload가 잘못됐습니다.
    Payload {
        /// node.
        node: StableId,
        /// 원인.
        error: InstancePayloadError,
    },
    /// 인스턴스 소유 node가 복제 subtree 안에 옮겨질 자리(보이는 slot host)가 없습니다.
    UnplacedContent {
        /// node.
        node: StableId,
    },
    /// undo 시점 scene이 detach 직후 상태와 다릅니다.
    StateMismatch {
        /// 기대와 다른 node.
        node: StableId,
    },
}

impl fmt::Display for DetachError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DetachError::Expansion(err) => write!(f, "detach 대상 해석 실패: {err}"),
            DetachError::Slots(err) => write!(f, "detach 대상 slot 오류: {err}"),
            DetachError::Payload { node, error } => {
                write!(f, "nested 인스턴스 {node} payload 오류: {error}")
            }
            DetachError::UnplacedContent { node } => write!(
                f,
                "인스턴스 소유 node {node}를 옮길 slot host가 detach 결과에 없습니다"
            ),
            DetachError::StateMismatch { node } => write!(
                f,
                "undo할 수 없습니다: node {node}가 detach 직후 상태가 아닙니다"
            ),
        }
    }
}

impl std::error::Error for DetachError {}

impl From<ExpansionError> for DetachError {
    fn from(err: ExpansionError) -> DetachError {
        DetachError::Expansion(err)
    }
}

impl From<SlotOverrideError> for DetachError {
    fn from(err: SlotOverrideError) -> DetachError {
        DetachError::Slots(err)
    }
}

/// StableId 기준 scene node 공통 필드(런타임 핸들 없이 저장 가능한 형태).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneNodeRecord {
    /// 안정 식별자.
    pub stable_id: StableId,
    /// 종류.
    pub kind: NodeKind,
    /// 부모(`None`이면 root).
    pub parent: Option<StableId>,
    /// 형제 순서 키.
    pub order_key: OrderKey,
    /// 표시 여부.
    pub visible: bool,
    /// 잠금 여부.
    pub locked: bool,
    /// 표시 이름.
    pub display_name: String,
}

/// slot content를 옮기기 전 위치.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovedContent {
    /// content node.
    pub node: StableId,
    /// 원래 부모(인스턴스).
    pub old_parent: StableId,
    /// 원래 순서 키.
    pub old_order_key: OrderKey,
}

/// detach 되돌리기 정보.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetachRecord {
    instance: SceneNodeRecord,
    instance_payload: Vec<u8>,
    created: Vec<StableId>,
    moved: Vec<MovedContent>,
    remap: Vec<(StableId, StableId)>,
}

impl DetachRecord {
    /// detach 전 인스턴스 node.
    pub fn instance(&self) -> &SceneNodeRecord {
        &self.instance
    }

    /// 새로 만든 복제 node(전위 순서, 인스턴스 자리의 root 복제본 제외).
    pub fn created(&self) -> &[StableId] {
        &self.created
    }

    /// 복제된 slot host 아래로 옮긴 slot content.
    pub fn moved(&self) -> &[MovedContent] {
        &self.moved
    }

    /// 정의 node → 복제 node id 대응(정의 root → 인스턴스 id 포함, 정의 node id 순).
    pub fn remap(&self) -> &[(StableId, StableId)] {
        &self.remap
    }
}

fn handles(columns: &NodeColumns) -> HashMap<StableId, RuntimeEntityId> {
    columns
        .iter_live()
        .filter_map(|handle| columns.get(handle).map(|f| (f.stable_id, handle)))
        .collect()
}

/// 정의 node의 payload를 복제합니다. nested 인스턴스면 slot content 참조를 재매핑합니다.
fn copy_payload(node: &DefinitionNode, remap: &IdRemapTable) -> Result<Vec<u8>, DetachError> {
    if node.kind != NodeKind::ComponentInstance {
        return Ok(node.payload.clone());
    }
    let payload =
        ComponentInstancePayload::decode(&node.payload).map_err(|error| DetachError::Payload {
            node: node.id,
            error,
        })?;
    let mut copied = ComponentInstancePayload::new(payload.component());
    for (key, value) in payload.property_overrides() {
        copied.set_property_override(key.clone(), value.clone());
    }
    for (slot, content) in payload.slot_overrides() {
        let remapped =
            SlotOverride::new(content.content().iter().map(|c| remap.remap(*c)).collect())
                .map_err(|error| DetachError::Payload {
                    node: node.id,
                    error,
                })?;
        copied.set_slot_override(slot, remapped);
    }
    Ok(copied.encode())
}

/// 인스턴스를 detach합니다.
///
/// 인스턴스 해석·slot 투영 검증을 먼저 끝낸 뒤에만 scene을 바꿉니다(검증 실패 시 scene
/// 불변). 성공하면 [`undo_detach`]에 넘길 되돌리기 정보를 반환합니다.
pub fn detach_instance<C: MonotonicClock>(
    columns: &mut NodeColumns,
    payloads: &mut ScenePayloads,
    library: &ComponentLibrary,
    instance: StableId,
    generator: &mut StableIdGenerator,
    clock: &C,
) -> Result<DetachRecord, DetachError> {
    // --- 계획: scene을 읽기만 합니다 ---
    let (record_instance, instance_payload, payload, owned_children) = {
        let index = SceneIndex::build(columns, payloads);
        let payload = index.instance_payload(instance)?;
        validate_scene_slot_overrides(library, &index, instance)?;
        let fields = index
            .fields(instance)
            .ok_or(ExpansionError::NodeNotFound { id: instance })?;
        let record = SceneNodeRecord {
            stable_id: instance,
            kind: fields.kind,
            parent: index.parent(instance),
            order_key: fields.order_key,
            visible: fields.visible,
            locked: fields.locked,
            display_name: fields.display_name,
        };
        let bytes = index.payload(instance).unwrap_or_default().to_vec();
        (record, bytes, payload, index.children(instance).to_vec())
    };
    let entry = library.get(payload.component()).ok_or_else(|| {
        ExpansionError::UnresolvedComponent(payload.component().unresolved_at(
            stellar_core_types::property::PropertyPath::with_keys(
                instance,
                vec![super::instance::keys::COMPONENT_REF],
            ),
        ))
    })?;
    let tree = entry.tree();
    let root_id = entry.definition().root.id();
    let root = tree
        .node(root_id)
        .ok_or(ExpansionError::NodeNotFound { id: root_id })?;

    // 복제할 정의 node(전위)와 override된 slot host의 투영 목록.
    let mut copies: Vec<StableId> = Vec::new();
    let mut projections: Vec<(StableId, Vec<StableId>)> = Vec::new();
    let mut stack = vec![root_id];
    while let Some(id) = stack.pop() {
        if id != root_id {
            copies.push(id);
        }
        let overridden = entry
            .slot_for_host(id)
            .and_then(|slot| payload.slot_override(slot.id));
        match overridden {
            Some(content) => projections.push((id, content.content().to_vec())),
            None => stack.extend(tree.children(id).iter().rev().copied()),
        }
    }
    // 인스턴스의 모든 소유 node는 복제된(보이는) slot host 아래로 옮겨져야 합니다. 하나라도
    // 자리가 없으면 적용 전에 거부합니다(부모 핸들이 끊긴 node 방지).
    let placed: HashSet<StableId> = projections
        .iter()
        .flat_map(|(_, content)| content.iter().copied())
        .collect();
    if let Some(node) = owned_children.iter().find(|c| !placed.contains(*c)) {
        return Err(DetachError::UnplacedContent { node: *node });
    }
    let mut remap = IdRemapTable::clone_ids(&copies, generator, clock);
    remap.insert(root_id, instance);
    let mut copied_payloads = Vec::with_capacity(copies.len());
    for id in &copies {
        let node = tree
            .node(*id)
            .ok_or(ExpansionError::NodeNotFound { id: *id })?;
        copied_payloads.push(copy_payload(node, &remap)?);
    }

    // --- 적용 ---
    let mut map = handles(columns);
    let parent_handle = record_instance.parent.and_then(|p| map.get(&p).copied());
    if let Some(handle) = map.remove(&instance) {
        columns.remove(handle);
    }
    let display_name = if record_instance.display_name.is_empty() {
        root.display_name.clone()
    } else {
        record_instance.display_name.clone()
    };
    let root_handle = columns.insert(NodeFields {
        stable_id: instance,
        kind: root.kind,
        parent: parent_handle,
        order_key: record_instance.order_key.clone(),
        visible: record_instance.visible && root.visible,
        locked: record_instance.locked || root.locked,
        display_name,
    });
    map.insert(instance, root_handle);
    payloads.insert(instance, root.payload.clone());

    let mut created = Vec::with_capacity(copies.len());
    for (id, payload_bytes) in copies.iter().zip(copied_payloads) {
        let Some(node) = tree.node(*id) else {
            continue;
        };
        let new_id = remap.remap(*id);
        let parent = node.parent.and_then(|p| map.get(&remap.remap(p)).copied());
        let handle = columns.insert(NodeFields {
            stable_id: new_id,
            kind: node.kind,
            parent,
            order_key: node.order_key.clone(),
            visible: node.visible,
            locked: node.locked,
            display_name: node.display_name.clone(),
        });
        map.insert(new_id, handle);
        payloads.insert(new_id, payload_bytes);
        created.push(new_id);
    }

    let mut moved = Vec::new();
    for (host, content) in &projections {
        let Some(host_handle) = map.get(&remap.remap(*host)).copied() else {
            continue;
        };
        let keys = OrderKey::evenly_spaced(content.len());
        for (node, key) in content.iter().zip(keys) {
            let Some(handle) = map.get(node).copied() else {
                continue;
            };
            let old_order_key = columns
                .get(handle)
                .map(|f| f.order_key)
                .unwrap_or_else(|| key.clone());
            columns.set_parent(handle, Some(host_handle));
            columns.set_order_key(handle, key);
            moved.push(MovedContent {
                node: *node,
                old_parent: instance,
                old_order_key,
            });
        }
    }

    let mut remap_pairs: Vec<(StableId, StableId)> = copies
        .iter()
        .map(|id| (*id, remap.remap(*id)))
        .chain(std::iter::once((root_id, instance)))
        .collect();
    remap_pairs.sort();
    Ok(DetachRecord {
        instance: record_instance,
        instance_payload,
        created,
        moved,
        remap: remap_pairs,
    })
}

/// detach 직후 구조인지 확인합니다: 복제 subtree(root 복제본 + 복제 node) 아래에는 복제 node와
/// 옮긴 content만 있고, 옮긴 content는 여전히 복제 subtree 안에 있어야 합니다. 그렇지 않으면
/// undo가 node를 지우거나 다시 붙일 때 부모 핸들이 끊긴 node가 생깁니다.
fn check_detached_shape(
    columns: &NodeColumns,
    payloads: &ScenePayloads,
    record: &DetachRecord,
) -> Result<(), DetachError> {
    let index = SceneIndex::build(columns, payloads);
    let subtree: HashSet<StableId> = record
        .created
        .iter()
        .copied()
        .chain(std::iter::once(record.instance.stable_id))
        .collect();
    let moved: HashSet<StableId> = record.moved.iter().map(|m| m.node).collect();
    for id in &subtree {
        if let Some(child) = index
            .children(*id)
            .iter()
            .find(|c| !subtree.contains(*c) && !moved.contains(*c))
        {
            return Err(DetachError::StateMismatch { node: *child });
        }
    }
    for node in &moved {
        if !index.parent(*node).is_some_and(|p| subtree.contains(&p)) {
            return Err(DetachError::StateMismatch { node: *node });
        }
    }
    Ok(())
}

/// detach를 되돌립니다. scene이 detach 직후 상태가 아니면 아무것도 바꾸지 않고 거부합니다.
///
/// 사전 조건: 복제 node·root 복제본·옮긴 content·원래 부모가 모두 있고, 복제 subtree 아래에
/// 그 밖의 node가 없으며, 옮긴 content가 복제 subtree 안에 있어야 합니다.
pub fn undo_detach(
    columns: &mut NodeColumns,
    payloads: &mut ScenePayloads,
    record: &DetachRecord,
) -> Result<(), DetachError> {
    let mut map = handles(columns);
    for id in record
        .created
        .iter()
        .chain(std::iter::once(&record.instance.stable_id))
        .chain(record.moved.iter().map(|m| &m.node))
        .chain(record.instance.parent.iter())
    {
        if !map.contains_key(id) {
            return Err(DetachError::StateMismatch { node: *id });
        }
    }
    check_detached_shape(columns, payloads, record)?;
    for id in record.created.iter().rev() {
        if let Some(handle) = map.remove(id) {
            columns.remove(handle);
        }
        payloads.remove(id);
    }
    if let Some(handle) = map.remove(&record.instance.stable_id) {
        columns.remove(handle);
    }
    let parent = record.instance.parent.and_then(|p| map.get(&p).copied());
    let handle = columns.insert(NodeFields {
        stable_id: record.instance.stable_id,
        kind: record.instance.kind,
        parent,
        order_key: record.instance.order_key.clone(),
        visible: record.instance.visible,
        locked: record.instance.locked,
        display_name: record.instance.display_name.clone(),
    });
    payloads.insert(record.instance.stable_id, record.instance_payload.clone());
    for moved in &record.moved {
        if let Some(node) = map.get(&moved.node).copied() {
            columns.set_parent(node, Some(handle));
            columns.set_order_key(node, moved.old_order_key.clone());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::definition::{ComponentDefinition, DefinitionTree};
    use stellar_core_types::clock::ManualClock;
    use stellar_core_types::reference::{ComponentRef, NodeRef};

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    fn library() -> ComponentLibrary {
        let mut tree = DefinitionTree::new(ComponentRef::new(stable(1)));
        for (id, kind, parent, key) in [
            (100, NodeKind::Frame, None, 0x80u8),
            (101, NodeKind::Text, Some(100), 0x40),
        ] {
            tree.insert(DefinitionNode {
                id: stable(id),
                kind,
                parent: parent.map(stable),
                order_key: OrderKey::from_bytes(vec![key]).unwrap(),
                visible: true,
                locked: false,
                display_name: format!("def-{id}"),
                payload: vec![id as u8],
            })
            .unwrap();
        }
        let mut library = ComponentLibrary::new();
        library
            .insert(
                ComponentDefinition::new(stable(1), "Label".into(), NodeRef::new(stable(100))),
                tree,
            )
            .unwrap();
        library
    }

    fn scene(kind: NodeKind) -> (NodeColumns, ScenePayloads) {
        let mut columns = NodeColumns::new();
        let root = columns.insert(NodeFields {
            stable_id: stable(1000),
            kind: NodeKind::Frame,
            parent: None,
            order_key: OrderKey::between(None, None),
            visible: true,
            locked: false,
            display_name: String::new(),
        });
        columns.insert(NodeFields {
            stable_id: stable(1001),
            kind,
            parent: Some(root),
            order_key: OrderKey::between(None, None),
            visible: false,
            locked: true,
            display_name: String::new(),
        });
        let mut payloads = ScenePayloads::new();
        payloads.insert(
            stable(1001),
            ComponentInstancePayload::new(ComponentRef::new(stable(1))).encode(),
        );
        (columns, payloads)
    }

    #[test]
    fn root_copy_takes_instance_place_and_identity() {
        let library = library();
        let (mut columns, mut payloads) = scene(NodeKind::ComponentInstance);
        let clock = ManualClock::new(1_700_000_000_000 * 1_000_000);
        let mut generator = StableIdGenerator::new(7);
        let record = detach_instance(
            &mut columns,
            &mut payloads,
            &library,
            stable(1001),
            &mut generator,
            &clock,
        )
        .unwrap();
        let map = handles(&columns);
        let root_copy = columns.get(map[&stable(1001)]).unwrap();
        assert_eq!(root_copy.kind, NodeKind::Frame);
        // 표시·잠금은 인스턴스와 정의 root를 합성, 이름이 비어 있으면 정의 이름.
        assert!(!root_copy.visible);
        assert!(root_copy.locked);
        assert_eq!(root_copy.display_name, "def-100");
        assert_eq!(payloads[&stable(1001)], vec![100]);
        // 복제 node는 새 id.
        assert_eq!(record.created().len(), 1);
        assert_ne!(record.created()[0], stable(101));
        assert_eq!(payloads[&record.created()[0]], vec![101]);
    }

    #[test]
    fn invalid_targets_leave_scene_unchanged() {
        let library = library();
        let clock = ManualClock::new(0);
        let mut generator = StableIdGenerator::new(1);
        // 인스턴스가 아닌 node.
        let (mut columns, mut payloads) = scene(NodeKind::Frame);
        let before = payloads.clone();
        assert!(matches!(
            detach_instance(
                &mut columns,
                &mut payloads,
                &library,
                stable(1001),
                &mut generator,
                &clock
            ),
            Err(DetachError::Expansion(ExpansionError::NotAnInstance { .. }))
        ));
        assert_eq!(payloads, before);
        assert_eq!(columns.len(), 2);

        // 정의가 없는 컴포넌트.
        let (mut columns, mut payloads) = scene(NodeKind::ComponentInstance);
        payloads.insert(
            stable(1001),
            ComponentInstancePayload::new(ComponentRef::new(stable(9))).encode(),
        );
        assert!(detach_instance(
            &mut columns,
            &mut payloads,
            &library,
            stable(1001),
            &mut generator,
            &clock
        )
        .is_err());
        assert_eq!(columns.len(), 2);
    }

    #[test]
    fn undo_rejects_changed_state() {
        let library = library();
        let (mut columns, mut payloads) = scene(NodeKind::ComponentInstance);
        let clock = ManualClock::new(0);
        let mut generator = StableIdGenerator::new(3);
        let record = detach_instance(
            &mut columns,
            &mut payloads,
            &library,
            stable(1001),
            &mut generator,
            &clock,
        )
        .unwrap();
        // 복제 node를 지운 뒤에는 undo 거부(상태 불변).
        let map = handles(&columns);
        columns.remove(map[&record.created()[0]]);
        let len = columns.len();
        assert_eq!(
            undo_detach(&mut columns, &mut payloads, &record).unwrap_err(),
            DetachError::StateMismatch {
                node: record.created()[0]
            }
        );
        assert_eq!(columns.len(), len);
    }
}
