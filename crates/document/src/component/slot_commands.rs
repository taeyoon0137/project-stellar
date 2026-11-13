//! Slot content 삽입·삭제·재정렬 command (IX.3.3).
//!
//! scene 인스턴스의 slot content를 바꾸는 closed enum command입니다. 각 command는 적용 전에
//! 전부 검증하고(실패 시 scene 불변), 적용하면 정확히 되돌리는 **inverse command**를
//! 돌려줍니다.
//!
//! - [`SlotContentCommand::Insert`]: content subtree를 인스턴스 소유 node로 만들고(부모 =
//!   인스턴스) slot의 지정 위치에 투영합니다. root 종류는 slot 허용 종류여야 합니다.
//! - [`SlotContentCommand::Remove`]: content subtree를 scene에서 지우고 투영에서 뺍니다.
//!   `restore_default`면 마지막 content를 뺄 때 override entry를 지워 default content로
//!   돌아갑니다(개수 검사 대신 default가 유효함).
//! - [`SlotContentCommand::Reorder`]: 투영 순서를 바꿉니다.
//!
//! 개수(cardinality)와 가림 규칙(nested slot host, IX.3.1)은 **적용 후 투영**에 대해
//! 검사합니다. default content를 보이던 slot에 처음 삽입하면 override가 1개짜리로 바뀌므로,
//! 최소 개수가 2 이상인 slot에는 단일 삽입이 거부됩니다. override된 바깥 slot에 가려진 slot에
//! 넣거나, 안쪽 slot에 content가 있는데 바깥 slot에 넣는 것도 적용 전에 거부합니다.
//!
//! Phase VII command 계층이 생기면 이 enum을 그 closed enum의 한 갈래로 등록합니다.

use std::collections::{BTreeSet, HashMap};
use std::fmt;

use stellar_core_types::ids::{RuntimeEntityId, StableId};

use super::definition::{ComponentEntry, ComponentLibrary, ComponentSlotId};
use super::detach::SceneNodeRecord;
use super::expansion::{ExpansionError, SceneIndex, ScenePayloads};
use super::instance::{ComponentInstancePayload, InstancePayloadError, SlotOverride};
use super::slot::{ComponentSlot, SlotError};
use super::slot_override::{
    check_hidden_slot_content, validate_record_slot_overrides, SlotOverrideError,
};
use crate::node_columns::{NodeColumns, NodeFields};
use crate::node_kind::NodeKind;
use crate::validate::can_have_children;

/// 삽입·복원할 scene node 하나(필드 + payload).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotNodeRecord {
    /// 공통 필드(부모는 StableId).
    pub record: SceneNodeRecord,
    /// 종류별 payload bytes(없으면 `None`).
    pub payload: Option<Vec<u8>>,
}

/// slot content command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotContentCommand {
    /// content subtree를 만들어 slot `index` 위치에 투영합니다.
    ///
    /// `nodes[0]`이 content root(부모 = 인스턴스)이고, 나머지는 부모가 앞선 record인 자손입니다.
    /// 자손의 부모는 자식을 가질 수 있는 종류(Frame·Fragment, 또는 slot content를 소유하는
    /// ComponentInstance)여야 하고, 같은 부모의 형제끼리 OrderKey가 달라야 합니다. subtree 안
    /// ComponentInstance의 slot override는 그 subtree 안 자식만 투영 규칙(IX.3.2)대로 가리켜야
    /// 합니다 — `Remove`의 inverse가 nested 인스턴스와 그 slot content를 그대로 되살립니다.
    Insert {
        /// scene 인스턴스.
        instance: StableId,
        /// slot.
        slot: ComponentSlotId,
        /// 투영 위치(0..=현재 개수).
        index: usize,
        /// content subtree(전위 순서).
        nodes: Vec<SlotNodeRecord>,
    },
    /// content subtree를 지우고 투영에서 뺍니다.
    Remove {
        /// scene 인스턴스.
        instance: StableId,
        /// slot.
        slot: ComponentSlotId,
        /// content root.
        node: StableId,
        /// 마지막 content를 빼면 override entry를 지워 default content로 복귀.
        restore_default: bool,
    },
    /// 투영 순서를 바꿉니다.
    Reorder {
        /// scene 인스턴스.
        instance: StableId,
        /// slot.
        slot: ComponentSlotId,
        /// 옮길 content.
        node: StableId,
        /// 새 위치(0..개수).
        new_index: usize,
    },
}

/// slot content command 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotCommandError {
    /// 인스턴스를 해석할 수 없습니다.
    Expansion(ExpansionError),
    /// 인스턴스 컴포넌트에 없는 slot입니다.
    UnknownSlot {
        /// slot.
        slot: ComponentSlotId,
    },
    /// slot 종류·개수 규칙 위반.
    Rule(SlotError),
    /// 삽입할 node가 없습니다.
    EmptyInsert,
    /// record의 부모가 잘못됐습니다(content root는 인스턴스, 자손은 앞선 record).
    InvalidParent {
        /// node.
        node: StableId,
    },
    /// 이미 있는 StableId입니다.
    DuplicateId {
        /// id.
        id: StableId,
    },
    /// 삽입 record의 부모가 자식을 가질 수 없는 종류입니다(leaf: Shape·Path·Text·Image).
    ParentNotContainer {
        /// 자식 record.
        node: StableId,
        /// 부모 record.
        parent: StableId,
        /// 부모 종류.
        parent_kind: NodeKind,
    },
    /// 형제 사이 OrderKey가 중복됩니다(인스턴스 자식, 또는 삽입 subtree 안 같은 부모의 자식).
    DuplicateOrderKey {
        /// node.
        node: StableId,
    },
    /// node가 그 slot에 투영돼 있지 않습니다.
    NotInSlot {
        /// node.
        node: StableId,
    },
    /// 위치가 범위를 벗어납니다.
    IndexOutOfRange {
        /// 요청 위치.
        index: usize,
        /// 현재 개수.
        len: usize,
    },
    /// payload가 잘못됐습니다.
    Payload(InstancePayloadError),
    /// 적용 후 투영이 slot override 규칙(가려진 slot content 등)을 어깁니다.
    Projection(SlotOverrideError),
}

impl fmt::Display for SlotCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SlotCommandError::Expansion(err) => write!(f, "{err}"),
            SlotCommandError::UnknownSlot { slot } => {
                write!(
                    f,
                    "slot {}이 인스턴스 컴포넌트에 없습니다",
                    slot.stable_id()
                )
            }
            SlotCommandError::Rule(err) => write!(f, "{err}"),
            SlotCommandError::EmptyInsert => f.write_str("삽입할 node가 없습니다"),
            SlotCommandError::InvalidParent { node } => {
                write!(f, "삽입 record {node}의 부모가 잘못됐습니다")
            }
            SlotCommandError::DuplicateId { id } => write!(f, "이미 있는 StableId: {id}"),
            SlotCommandError::ParentNotContainer {
                node,
                parent,
                parent_kind,
            } => write!(
                f,
                "삽입 record {node}의 부모 {parent}({})는 자식을 가질 수 없습니다",
                parent_kind.name()
            ),
            SlotCommandError::DuplicateOrderKey { node } => {
                write!(f, "형제 OrderKey가 중복됩니다({node})")
            }
            SlotCommandError::NotInSlot { node } => {
                write!(f, "node {node}는 이 slot에 투영돼 있지 않습니다")
            }
            SlotCommandError::IndexOutOfRange { index, len } => {
                write!(f, "위치 {index}가 범위(개수 {len})를 벗어납니다")
            }
            SlotCommandError::Payload(err) => write!(f, "{err}"),
            SlotCommandError::Projection(err) => write!(f, "적용 후 투영 규칙 위반: {err}"),
        }
    }
}

impl std::error::Error for SlotCommandError {}

impl From<ExpansionError> for SlotCommandError {
    fn from(err: ExpansionError) -> SlotCommandError {
        SlotCommandError::Expansion(err)
    }
}

/// command 대상 해석 결과.
struct Target<'l> {
    payload: ComponentInstancePayload,
    entry: &'l ComponentEntry,
    slot: &'l ComponentSlot,
    content: Option<Vec<StableId>>,
}

impl Target<'_> {
    /// slot content를 `content`로 바꾼(`None`이면 override 제거) **적용 후** 투영이 개수·가림
    /// 규칙을 지키는지 검사합니다. override를 지우면 default content가 보이며 그 유효성은 정의
    /// 등록 때 이미 검증됐습니다.
    fn check_result(
        &self,
        instance: StableId,
        content: Option<&[StableId]>,
    ) -> Result<(), SlotCommandError> {
        let mut result = self.payload.clone();
        match content {
            Some(list) => {
                if !self.slot.cardinality.allows(list.len()) {
                    return Err(SlotCommandError::Rule(SlotError::CardinalityViolation {
                        slot: self.slot.id,
                        count: list.len(),
                    }));
                }
                let over = SlotOverride::new(list.to_vec()).map_err(SlotCommandError::Payload)?;
                result.set_slot_override(self.slot.id, over);
            }
            None => {
                result.remove_slot_override(self.slot.id);
            }
        }
        check_hidden_slot_content(self.entry, &result, instance)
            .map_err(SlotCommandError::Projection)
    }
}

fn resolve<'l>(
    library: &'l ComponentLibrary,
    index: &SceneIndex<'_>,
    instance: StableId,
    slot: ComponentSlotId,
) -> Result<Target<'l>, SlotCommandError> {
    let payload = index.instance_payload(instance)?;
    let entry = library.get(payload.component()).ok_or_else(|| {
        ExpansionError::UnresolvedComponent(payload.component().unresolved_at(
            stellar_core_types::property::PropertyPath::with_keys(
                instance,
                vec![super::instance::keys::COMPONENT_REF],
            ),
        ))
    })?;
    let slot_record = entry
        .slot(slot)
        .ok_or(SlotCommandError::UnknownSlot { slot })?;
    let content = payload.slot_override(slot).map(|o| o.content().to_vec());
    Ok(Target {
        payload,
        entry,
        slot: slot_record,
        content,
    })
}

fn handles(columns: &NodeColumns) -> HashMap<StableId, RuntimeEntityId> {
    columns
        .iter_live()
        .filter_map(|handle| columns.get(handle).map(|f| (f.stable_id, handle)))
        .collect()
}

/// node subtree를 전위 순서 record로 캡처합니다.
fn capture_subtree(index: &SceneIndex<'_>, root: StableId) -> Vec<SlotNodeRecord> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if let Some(fields) = index.fields(id) {
            out.push(SlotNodeRecord {
                record: SceneNodeRecord {
                    stable_id: id,
                    kind: fields.kind,
                    parent: index.parent(id),
                    order_key: fields.order_key,
                    visible: fields.visible,
                    locked: fields.locked,
                    display_name: fields.display_name,
                },
                payload: index.payload(id).map(<[u8]>::to_vec),
            });
        }
        stack.extend(index.children(id).iter().rev().copied());
    }
    out
}

fn store_override(
    payloads: &mut ScenePayloads,
    instance: StableId,
    mut payload: ComponentInstancePayload,
    slot: ComponentSlotId,
    content: Option<Vec<StableId>>,
) -> Result<(), SlotCommandError> {
    match content {
        Some(list) => {
            let over = SlotOverride::new(list).map_err(SlotCommandError::Payload)?;
            payload.set_slot_override(slot, over);
        }
        None => {
            payload.remove_slot_override(slot);
        }
    }
    payloads.insert(instance, payload.encode());
    Ok(())
}

impl SlotContentCommand {
    /// 적용 가능 여부를 검증합니다. 상태를 바꾸지 않습니다.
    pub fn validate(
        &self,
        library: &ComponentLibrary,
        columns: &NodeColumns,
        payloads: &ScenePayloads,
    ) -> Result<(), SlotCommandError> {
        let index = SceneIndex::build(columns, payloads);
        match self {
            SlotContentCommand::Insert {
                instance,
                slot,
                index: at,
                nodes,
            } => {
                let target = resolve(library, &index, *instance, *slot)?;
                let root = nodes.first().ok_or(SlotCommandError::EmptyInsert)?;
                let len = target.content.as_ref().map_or(0, Vec::len);
                if *at > len {
                    return Err(SlotCommandError::IndexOutOfRange { index: *at, len });
                }
                // 앞선 record의 종류(부모 판정)와 부모별 형제 OrderKey(중복 판정).
                let mut kinds: HashMap<StableId, NodeKind> = HashMap::new();
                let mut sibling_keys: HashMap<StableId, BTreeSet<&[u8]>> = HashMap::new();
                for (i, node) in nodes.iter().enumerate() {
                    let id = node.record.stable_id;
                    if index.fields(id).is_some() || kinds.contains_key(&id) {
                        return Err(SlotCommandError::DuplicateId { id });
                    }
                    if i == 0 {
                        if node.record.parent != Some(*instance) {
                            return Err(SlotCommandError::InvalidParent { node: id });
                        }
                    } else {
                        let parent = node
                            .record
                            .parent
                            .filter(|p| *p != id)
                            .ok_or(SlotCommandError::InvalidParent { node: id })?;
                        let parent_kind = *kinds
                            .get(&parent)
                            .ok_or(SlotCommandError::InvalidParent { node: id })?;
                        if !can_have_children(parent_kind)
                            && parent_kind != NodeKind::ComponentInstance
                        {
                            return Err(SlotCommandError::ParentNotContainer {
                                node: id,
                                parent,
                                parent_kind,
                            });
                        }
                        if !sibling_keys
                            .entry(parent)
                            .or_default()
                            .insert(node.record.order_key.as_bytes())
                        {
                            return Err(SlotCommandError::DuplicateOrderKey { node: id });
                        }
                    }
                    if node.record.kind == NodeKind::ComponentInstance {
                        ComponentInstancePayload::decode(node.payload.as_deref().unwrap_or(&[]))
                            .map_err(SlotCommandError::Payload)?;
                    }
                    kinds.insert(id, node.record.kind);
                }
                // subtree 안 인스턴스: slot override가 subtree 안 자기 자식만 투영해야 합니다.
                let records: HashMap<StableId, (NodeKind, Option<StableId>)> = nodes
                    .iter()
                    .map(|n| (n.record.stable_id, (n.record.kind, n.record.parent)))
                    .collect();
                for node in nodes
                    .iter()
                    .filter(|n| n.record.kind == NodeKind::ComponentInstance)
                {
                    let id = node.record.stable_id;
                    let payload =
                        ComponentInstancePayload::decode(node.payload.as_deref().unwrap_or(&[]))
                            .map_err(SlotCommandError::Payload)?;
                    let children: Vec<StableId> = nodes
                        .iter()
                        .filter(|n| n.record.parent == Some(id))
                        .map(|n| n.record.stable_id)
                        .collect();
                    if children.is_empty() && payload.slot_overrides().next().is_none() {
                        // 투영할 것이 없습니다(참조 컴포넌트가 미해결이어도 허용, III.2.4).
                        continue;
                    }
                    validate_record_slot_overrides(
                        library,
                        &payload,
                        id,
                        |n| records.get(&n).copied(),
                        &children,
                    )
                    .map_err(SlotCommandError::Projection)?;
                }
                let root_id = root.record.stable_id;
                if index
                    .children(*instance)
                    .iter()
                    .filter_map(|c| index.fields(*c))
                    .any(|f| f.order_key == root.record.order_key)
                {
                    return Err(SlotCommandError::DuplicateOrderKey { node: root_id });
                }
                if !target.slot.allows_kind(root.record.kind) {
                    return Err(SlotCommandError::Rule(SlotError::DisallowedKind {
                        slot: *slot,
                        node: root_id,
                        kind: root.record.kind,
                    }));
                }
                // 적용 후 투영: default를 보이던 slot이면 1개짜리 override가 됩니다.
                let mut result = target.content.clone().unwrap_or_default();
                result.insert(*at, root_id);
                target.check_result(*instance, Some(&result))
            }
            SlotContentCommand::Remove {
                instance,
                slot,
                node,
                restore_default,
            } => {
                let target = resolve(library, &index, *instance, *slot)?;
                let content = target.content.clone().unwrap_or_default();
                if !content.contains(node) {
                    return Err(SlotCommandError::NotInSlot { node: *node });
                }
                let remaining: Vec<StableId> =
                    content.iter().copied().filter(|c| c != node).collect();
                let back_to_default = *restore_default && remaining.is_empty();
                target.check_result(*instance, (!back_to_default).then_some(&remaining[..]))
            }
            SlotContentCommand::Reorder {
                instance,
                slot,
                node,
                new_index,
            } => {
                let target = resolve(library, &index, *instance, *slot)?;
                let content = target.content.unwrap_or_default();
                if !content.contains(node) {
                    return Err(SlotCommandError::NotInSlot { node: *node });
                }
                if *new_index >= content.len() {
                    return Err(SlotCommandError::IndexOutOfRange {
                        index: *new_index,
                        len: content.len(),
                    });
                }
                Ok(())
            }
        }
    }

    /// 검증 후 적용하고 inverse command를 반환합니다. 검증 실패 시 scene은 바뀌지 않습니다.
    pub fn apply(
        &self,
        library: &ComponentLibrary,
        columns: &mut NodeColumns,
        payloads: &mut ScenePayloads,
    ) -> Result<SlotContentCommand, SlotCommandError> {
        self.validate(library, columns, payloads)?;
        match self {
            SlotContentCommand::Insert {
                instance,
                slot,
                index: at,
                nodes,
            } => {
                let (payload, content) = {
                    let index = SceneIndex::build(columns, payloads);
                    let target = resolve(library, &index, *instance, *slot)?;
                    (target.payload, target.content)
                };
                let existed = content.is_some();
                let mut list = content.unwrap_or_default();
                let mut map = handles(columns);
                for node in nodes {
                    let parent = node.record.parent.and_then(|p| map.get(&p).copied());
                    let handle = columns.insert(NodeFields {
                        stable_id: node.record.stable_id,
                        kind: node.record.kind,
                        parent,
                        order_key: node.record.order_key.clone(),
                        visible: node.record.visible,
                        locked: node.record.locked,
                        display_name: node.record.display_name.clone(),
                    });
                    map.insert(node.record.stable_id, handle);
                    if let Some(bytes) = &node.payload {
                        payloads.insert(node.record.stable_id, bytes.clone());
                    }
                }
                let root = nodes[0].record.stable_id;
                list.insert(*at, root);
                store_override(payloads, *instance, payload, *slot, Some(list))?;
                Ok(SlotContentCommand::Remove {
                    instance: *instance,
                    slot: *slot,
                    node: root,
                    restore_default: !existed,
                })
            }
            SlotContentCommand::Remove {
                instance,
                slot,
                node,
                restore_default,
            } => {
                let (payload, content, captured) = {
                    let index = SceneIndex::build(columns, payloads);
                    let target = resolve(library, &index, *instance, *slot)?;
                    (
                        target.payload,
                        target.content,
                        capture_subtree(&index, *node),
                    )
                };
                let mut list = content.unwrap_or_default();
                let old_index = list.iter().position(|n| n == node).unwrap_or(0);
                list.retain(|n| n != node);
                let map = handles(columns);
                for record in captured.iter().rev() {
                    if let Some(handle) = map.get(&record.record.stable_id) {
                        columns.remove(*handle);
                    }
                    payloads.remove(&record.record.stable_id);
                }
                let content = if *restore_default && list.is_empty() {
                    None
                } else {
                    Some(list)
                };
                store_override(payloads, *instance, payload, *slot, content)?;
                Ok(SlotContentCommand::Insert {
                    instance: *instance,
                    slot: *slot,
                    index: old_index,
                    nodes: captured,
                })
            }
            SlotContentCommand::Reorder {
                instance,
                slot,
                node,
                new_index,
            } => {
                let (payload, content) = {
                    let index = SceneIndex::build(columns, payloads);
                    let target = resolve(library, &index, *instance, *slot)?;
                    (target.payload, target.content)
                };
                let mut list = content.unwrap_or_default();
                let old_index = list.iter().position(|n| n == node).unwrap_or(0);
                let moved = list.remove(old_index);
                list.insert(*new_index, moved);
                store_override(payloads, *instance, payload, *slot, Some(list))?;
                Ok(SlotContentCommand::Reorder {
                    instance: *instance,
                    slot: *slot,
                    node: *node,
                    new_index: old_index,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::definition::{ComponentDefinition, DefinitionNode, DefinitionTree};
    use crate::component::expansion::{ExpandedNodeId, InstanceExpansion};
    use crate::component::slot::SlotCardinality;
    use crate::component::slot_override::validate_scene_slot_overrides;
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::reference::{ComponentRef, NodeRef};

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    fn key(byte: u8) -> OrderKey {
        OrderKey::from_bytes(vec![byte]).unwrap()
    }

    const BODY: u64 = 600;
    const FOOTER: u64 = 601;
    const INSTANCE: u64 = 1001;

    fn slot(ts: u64) -> ComponentSlotId {
        ComponentSlotId::new(stable(ts))
    }

    fn card() -> ComponentRef {
        ComponentRef::new(stable(1))
    }

    /// Card(1): 100 ← 102(body host, Text|Frame, 1..=3) ← 103(Text default),
    ///          105(footer host, Text, 0..=1).
    fn library() -> ComponentLibrary {
        let mut tree = DefinitionTree::new(card());
        for (id, kind, parent, order) in [
            (100, NodeKind::Frame, None, 0x80u8),
            (102, NodeKind::Frame, Some(100), 0x40),
            (103, NodeKind::Text, Some(102), 0x40),
            (105, NodeKind::Frame, Some(100), 0x60),
        ] {
            tree.insert(DefinitionNode {
                id: stable(id),
                kind,
                parent: parent.map(stable),
                order_key: key(order),
                visible: true,
                locked: false,
                display_name: String::new(),
                payload: Vec::new(),
            })
            .unwrap();
        }
        let mut definition =
            ComponentDefinition::new(stable(1), "Card".into(), NodeRef::new(stable(100)));
        definition.add_slot(slot(BODY)).unwrap();
        definition.add_slot(slot(FOOTER)).unwrap();
        let mut library = ComponentLibrary::new();
        library.insert(definition, tree).unwrap();
        library
            .add_slot(
                card(),
                ComponentSlot::new(
                    slot(BODY),
                    NodeRef::new(stable(102)),
                    [NodeKind::Text, NodeKind::Frame],
                    SlotCardinality::new(1, Some(3)).unwrap(),
                    vec![NodeRef::new(stable(103))],
                ),
            )
            .unwrap();
        library
            .add_slot(
                card(),
                ComponentSlot::new(
                    slot(FOOTER),
                    NodeRef::new(stable(105)),
                    [NodeKind::Text],
                    SlotCardinality::at_most(1),
                    vec![],
                ),
            )
            .unwrap();
        library
    }

    /// scene: 1000 ← 1001(Card, body = `body`) ← 1002(Text 0x20), 1003(Frame 0x40) ← 1004(Text).
    fn scene(body: &[u64]) -> (NodeColumns, ScenePayloads) {
        let mut columns = NodeColumns::new();
        let mk = |id: u64, kind, parent, order: u8| NodeFields {
            stable_id: stable(id),
            kind,
            parent,
            order_key: key(order),
            visible: true,
            locked: false,
            display_name: format!("n{id}"),
        };
        let root = columns.insert(mk(1000, NodeKind::Frame, None, 0x80));
        let instance = columns.insert(mk(INSTANCE, NodeKind::ComponentInstance, Some(root), 0x40));
        if body.contains(&1002) {
            columns.insert(mk(1002, NodeKind::Text, Some(instance), 0x20));
        }
        if body.contains(&1003) {
            let frame = columns.insert(mk(1003, NodeKind::Frame, Some(instance), 0x40));
            columns.insert(mk(1004, NodeKind::Text, Some(frame), 0x40));
        }
        let mut payload = ComponentInstancePayload::new(card());
        payload.set_slot_override(
            slot(BODY),
            SlotOverride::new(body.iter().copied().map(stable).collect()).unwrap(),
        );
        let mut payloads = ScenePayloads::new();
        payloads.insert(stable(INSTANCE), payload.encode());
        payloads.insert(stable(1003), vec![0xF3]);
        (columns, payloads)
    }

    fn snapshot(columns: &NodeColumns, payloads: &ScenePayloads) -> Vec<String> {
        let index = SceneIndex::build(columns, payloads);
        let mut lines: Vec<String> = columns
            .iter_live()
            .filter_map(|h| columns.get(h))
            .map(|f| {
                format!(
                    "{} {:?} {:?} {:?} {} {} {}",
                    f.stable_id,
                    f.kind,
                    index.parent(f.stable_id),
                    f.order_key.as_bytes(),
                    f.visible,
                    f.locked,
                    f.display_name
                )
            })
            .collect();
        lines.sort();
        lines.extend(payloads.iter().map(|(id, b)| format!("{id} {b:?}")));
        lines
    }

    fn text(id: u64, parent: u64, order: u8) -> SlotNodeRecord {
        SlotNodeRecord {
            record: SceneNodeRecord {
                stable_id: stable(id),
                kind: NodeKind::Text,
                parent: Some(stable(parent)),
                order_key: key(order),
                visible: true,
                locked: false,
                display_name: format!("new{id}"),
            },
            payload: Some(vec![id as u8]),
        }
    }

    fn projected(
        library: &ComponentLibrary,
        columns: &NodeColumns,
        payloads: &ScenePayloads,
        host: u64,
    ) -> Vec<ExpandedNodeId> {
        let index = SceneIndex::build(columns, payloads);
        let expansion = InstanceExpansion::new(library, &index, stable(INSTANCE)).unwrap();
        expansion
            .children(&ExpandedNodeId::virtual_node(
                vec![stable(INSTANCE)],
                stable(host),
            ))
            .unwrap()
    }

    fn owned(id: u64) -> ExpandedNodeId {
        ExpandedNodeId::Owned(stable(id))
    }

    #[test]
    fn invalid_kind_and_overflow_are_rejected() {
        // 검증 기준(IX.3.3): invalid kind·overflow fixture 실패.
        let library = library();
        let (mut columns, mut payloads) = scene(&[1002, 1003]);
        let before = snapshot(&columns, &payloads);

        // footer는 Text만 허용 — Frame 삽입 거부.
        let mut frame = text(2001, INSTANCE, 0x70);
        frame.record.kind = NodeKind::Frame;
        let invalid_kind = SlotContentCommand::Insert {
            instance: stable(INSTANCE),
            slot: slot(FOOTER),
            index: 0,
            nodes: vec![frame],
        };
        assert!(matches!(
            invalid_kind
                .apply(&library, &mut columns, &mut payloads)
                .unwrap_err(),
            SlotCommandError::Rule(SlotError::DisallowedKind {
                kind: NodeKind::Frame,
                ..
            })
        ));
        assert_eq!(snapshot(&columns, &payloads), before);

        // body(최대 3): 하나 넣어 3개, 하나 더 넣으면 초과.
        let third = SlotContentCommand::Insert {
            instance: stable(INSTANCE),
            slot: slot(BODY),
            index: 2,
            nodes: vec![text(2001, INSTANCE, 0x70)],
        };
        third.apply(&library, &mut columns, &mut payloads).unwrap();
        let after_third = snapshot(&columns, &payloads);
        let overflow = SlotContentCommand::Insert {
            instance: stable(INSTANCE),
            slot: slot(BODY),
            index: 0,
            nodes: vec![text(2002, INSTANCE, 0x78)],
        };
        assert_eq!(
            overflow
                .apply(&library, &mut columns, &mut payloads)
                .unwrap_err(),
            SlotCommandError::Rule(SlotError::CardinalityViolation {
                slot: slot(BODY),
                count: 4,
            })
        );
        assert_eq!(snapshot(&columns, &payloads), after_third);

        // footer(최대 1)도 같은 규칙.
        let footer = SlotContentCommand::Insert {
            instance: stable(INSTANCE),
            slot: slot(FOOTER),
            index: 0,
            nodes: vec![text(2003, INSTANCE, 0x7A)],
        };
        footer.apply(&library, &mut columns, &mut payloads).unwrap();
        let footer_overflow = SlotContentCommand::Insert {
            instance: stable(INSTANCE),
            slot: slot(FOOTER),
            index: 1,
            nodes: vec![text(2004, INSTANCE, 0x7C)],
        };
        assert!(matches!(
            footer_overflow
                .validate(&library, &columns, &payloads)
                .unwrap_err(),
            SlotCommandError::Rule(SlotError::CardinalityViolation { count: 2, .. })
        ));
        let index = SceneIndex::build(&columns, &payloads);
        validate_scene_slot_overrides(&library, &index, stable(INSTANCE)).unwrap();
    }

    #[test]
    fn insert_apply_and_inverse_round_trip() {
        let library = library();
        let (mut columns, mut payloads) = scene(&[1002, 1003]);
        let before = snapshot(&columns, &payloads);
        // 자손이 있는 subtree를 가운데 삽입.
        let mut subtree_root = text(2001, INSTANCE, 0x70);
        subtree_root.record.kind = NodeKind::Frame;
        let insert = SlotContentCommand::Insert {
            instance: stable(INSTANCE),
            slot: slot(BODY),
            index: 1,
            nodes: vec![subtree_root, text(2002, 2001, 0x40)],
        };
        let inverse = insert.apply(&library, &mut columns, &mut payloads).unwrap();
        assert_eq!(
            projected(&library, &columns, &payloads, 102),
            vec![owned(1002), owned(2001), owned(1003)]
        );
        let index = SceneIndex::build(&columns, &payloads);
        assert_eq!(index.parent(stable(2001)), Some(stable(INSTANCE)));
        assert_eq!(index.parent(stable(2002)), Some(stable(2001)));
        validate_scene_slot_overrides(&library, &index, stable(INSTANCE)).unwrap();
        drop(index);

        let redo = inverse
            .apply(&library, &mut columns, &mut payloads)
            .unwrap();
        assert_eq!(snapshot(&columns, &payloads), before);
        // inverse의 inverse는 원래 command와 같은 효과.
        redo.apply(&library, &mut columns, &mut payloads).unwrap();
        assert_eq!(
            projected(&library, &columns, &payloads, 102),
            vec![owned(1002), owned(2001), owned(1003)]
        );
    }

    #[test]
    fn remove_respects_minimum_and_restores_default() {
        let library = library();
        let (mut columns, mut payloads) = scene(&[1003]);
        let before = snapshot(&columns, &payloads);
        // body 최소 1개: 마지막 content를 그냥 빼면 거부.
        let remove = SlotContentCommand::Remove {
            instance: stable(INSTANCE),
            slot: slot(BODY),
            node: stable(1003),
            restore_default: false,
        };
        assert_eq!(
            remove
                .apply(&library, &mut columns, &mut payloads)
                .unwrap_err(),
            SlotCommandError::Rule(SlotError::CardinalityViolation {
                slot: slot(BODY),
                count: 0,
            })
        );
        assert_eq!(snapshot(&columns, &payloads), before);

        // default로 복귀하며 빼기: subtree 삭제, override entry 제거 → default content 표시.
        let back = SlotContentCommand::Remove {
            instance: stable(INSTANCE),
            slot: slot(BODY),
            node: stable(1003),
            restore_default: true,
        };
        let inverse = back.apply(&library, &mut columns, &mut payloads).unwrap();
        assert_eq!(columns.len(), 2, "1003·1004 삭제");
        assert_eq!(
            projected(&library, &columns, &payloads, 102),
            vec![ExpandedNodeId::virtual_node(
                vec![stable(INSTANCE)],
                stable(103)
            )]
        );
        let payload = ComponentInstancePayload::decode(&payloads[&stable(INSTANCE)]).unwrap();
        assert!(payload.slot_override(slot(BODY)).is_none());
        // inverse로 subtree·payload·투영 전부 복원.
        inverse
            .apply(&library, &mut columns, &mut payloads)
            .unwrap();
        assert_eq!(snapshot(&columns, &payloads), before);
    }

    #[test]
    fn reorder_and_footer_insert_inverse_restore_sparse_payload() {
        let library = library();
        let (mut columns, mut payloads) = scene(&[1002, 1003]);
        let before = snapshot(&columns, &payloads);
        let reorder = SlotContentCommand::Reorder {
            instance: stable(INSTANCE),
            slot: slot(BODY),
            node: stable(1002),
            new_index: 1,
        };
        let inverse = reorder
            .apply(&library, &mut columns, &mut payloads)
            .unwrap();
        assert_eq!(
            projected(&library, &columns, &payloads, 102),
            vec![owned(1003), owned(1002)]
        );
        inverse
            .apply(&library, &mut columns, &mut payloads)
            .unwrap();
        assert_eq!(snapshot(&columns, &payloads), before);

        // override 없던 footer에 삽입 → inverse가 entry까지 지워 payload bytes 원복.
        let insert = SlotContentCommand::Insert {
            instance: stable(INSTANCE),
            slot: slot(FOOTER),
            index: 0,
            nodes: vec![text(2001, INSTANCE, 0x70)],
        };
        let inverse = insert.apply(&library, &mut columns, &mut payloads).unwrap();
        assert!(matches!(
            inverse,
            SlotContentCommand::Remove {
                restore_default: true,
                ..
            }
        ));
        inverse
            .apply(&library, &mut columns, &mut payloads)
            .unwrap();
        assert_eq!(snapshot(&columns, &payloads), before);
    }

    #[test]
    fn malformed_commands_are_rejected() {
        let library = library();
        let (columns, payloads) = scene(&[1002, 1003]);
        let insert = |slot_id: u64, index: usize, nodes: Vec<SlotNodeRecord>| {
            SlotContentCommand::Insert {
                instance: stable(INSTANCE),
                slot: slot(slot_id),
                index,
                nodes,
            }
            .validate(&library, &columns, &payloads)
        };
        assert_eq!(
            insert(699, 0, vec![text(2001, INSTANCE, 0x70)]).unwrap_err(),
            SlotCommandError::UnknownSlot { slot: slot(699) }
        );
        assert_eq!(
            insert(BODY, 0, vec![]).unwrap_err(),
            SlotCommandError::EmptyInsert
        );
        assert_eq!(
            insert(BODY, 5, vec![text(2001, INSTANCE, 0x70)]).unwrap_err(),
            SlotCommandError::IndexOutOfRange { index: 5, len: 2 }
        );
        assert_eq!(
            insert(BODY, 0, vec![text(1002, INSTANCE, 0x70)]).unwrap_err(),
            SlotCommandError::DuplicateId { id: stable(1002) }
        );
        // content root의 부모가 인스턴스가 아님 / 자손 부모가 앞선 record가 아님.
        assert_eq!(
            insert(BODY, 0, vec![text(2001, 1000, 0x70)]).unwrap_err(),
            SlotCommandError::InvalidParent { node: stable(2001) }
        );
        assert_eq!(
            insert(
                BODY,
                0,
                vec![text(2001, INSTANCE, 0x70), text(2002, 1003, 0x40)]
            )
            .unwrap_err(),
            SlotCommandError::InvalidParent { node: stable(2002) }
        );
        // 인스턴스 자식 OrderKey 중복(1002가 0x20).
        assert_eq!(
            insert(BODY, 0, vec![text(2001, INSTANCE, 0x20)]).unwrap_err(),
            SlotCommandError::DuplicateOrderKey { node: stable(2001) }
        );
        // 투영되지 않은 node 삭제·재정렬, 범위 밖 재정렬.
        let remove = SlotContentCommand::Remove {
            instance: stable(INSTANCE),
            slot: slot(BODY),
            node: stable(1004),
            restore_default: false,
        };
        assert_eq!(
            remove.validate(&library, &columns, &payloads).unwrap_err(),
            SlotCommandError::NotInSlot { node: stable(1004) }
        );
        let reorder = SlotContentCommand::Reorder {
            instance: stable(INSTANCE),
            slot: slot(BODY),
            node: stable(1002),
            new_index: 2,
        };
        assert_eq!(
            reorder.validate(&library, &columns, &payloads).unwrap_err(),
            SlotCommandError::IndexOutOfRange { index: 2, len: 2 }
        );
        // scene 인스턴스가 아닌 대상(정의 node id).
        let not_scene = SlotContentCommand::Reorder {
            instance: stable(102),
            slot: slot(BODY),
            node: stable(1002),
            new_index: 0,
        };
        assert!(matches!(
            not_scene
                .validate(&library, &columns, &payloads)
                .unwrap_err(),
            SlotCommandError::Expansion(ExpansionError::NodeNotFound { .. })
        ));
    }

    fn rec(id: u64, kind: NodeKind, parent: u64, order: u8, payload: Vec<u8>) -> SlotNodeRecord {
        SlotNodeRecord {
            record: SceneNodeRecord {
                stable_id: stable(id),
                kind,
                parent: Some(stable(parent)),
                order_key: key(order),
                visible: true,
                locked: false,
                display_name: format!("new{id}"),
            },
            payload: Some(payload),
        }
    }

    #[test]
    fn insert_subtree_parents_and_sibling_keys_are_validated() {
        let library = library();
        let (columns, payloads) = scene(&[1002, 1003]);
        let insert = |nodes: Vec<SlotNodeRecord>| {
            SlotContentCommand::Insert {
                instance: stable(INSTANCE),
                slot: slot(BODY),
                index: 0,
                nodes,
            }
            .validate(&library, &columns, &payloads)
        };
        let frame = |id, parent, order| rec(id, NodeKind::Frame, parent, order, vec![]);
        // leaf(Text) 아래 자식.
        assert_eq!(
            insert(vec![
                frame(2001, INSTANCE, 0x70),
                text(2002, 2001, 0x40),
                text(2003, 2002, 0x40),
            ])
            .unwrap_err(),
            SlotCommandError::ParentNotContainer {
                node: stable(2003),
                parent: stable(2002),
                parent_kind: NodeKind::Text,
            }
        );
        // subtree 안 같은 부모의 형제 OrderKey 중복.
        assert_eq!(
            insert(vec![
                frame(2001, INSTANCE, 0x70),
                text(2002, 2001, 0x40),
                text(2003, 2001, 0x40),
            ])
            .unwrap_err(),
            SlotCommandError::DuplicateOrderKey { node: stable(2003) }
        );
        // 다른 부모 아래 같은 key는 허용.
        insert(vec![
            frame(2001, INSTANCE, 0x70),
            frame(2002, 2001, 0x40),
            text(2003, 2002, 0x40),
        ])
        .unwrap();
    }

    #[test]
    fn nested_instance_subtree_is_validated_and_remove_inverse_restores_it() {
        let library = panel_library();
        let panel = ComponentRef::new(stable(7));
        let nested_payload = |content: &[u64]| {
            let mut payload = ComponentInstancePayload::new(panel);
            if !content.is_empty() {
                payload.set_slot_override(
                    slot(INNER),
                    SlotOverride::new(content.iter().copied().map(stable).collect()).unwrap(),
                );
            }
            payload.encode()
        };
        let subtree = |content: &[u64], with_child: bool| {
            let mut nodes = vec![
                rec(2101, NodeKind::Frame, PANEL, 0x70, vec![]),
                rec(
                    2102,
                    NodeKind::ComponentInstance,
                    2101,
                    0x40,
                    nested_payload(content),
                ),
            ];
            if with_child {
                nodes.push(text(2103, 2102, 0x40));
            }
            SlotContentCommand::Insert {
                instance: stable(PANEL),
                slot: slot(OUTER),
                index: 0,
                nodes,
            }
        };
        let (mut columns, mut payloads) = panel_scene(&[], &[]);
        // nested 인스턴스 자식이 투영되지 않음 / override가 subtree 밖 node를 가리킴.
        assert_eq!(
            subtree(&[], true)
                .validate(&library, &columns, &payloads)
                .unwrap_err(),
            SlotCommandError::Projection(SlotOverrideError::UnprojectedOwnedNode {
                instance: stable(2102),
                node: stable(2103),
            })
        );
        assert_eq!(
            subtree(&[2199], false)
                .validate(&library, &columns, &payloads)
                .unwrap_err(),
            SlotCommandError::Projection(SlotOverrideError::ContentNotFound {
                instance: stable(2102),
                node: stable(2199),
            })
        );

        // 유효한 nested 인스턴스 subtree 삽입 → 두 인스턴스 모두 투영 규칙 통과.
        subtree(&[2103], true)
            .apply(&library, &mut columns, &mut payloads)
            .unwrap();
        let inserted = snapshot(&columns, &payloads);
        let index = SceneIndex::build(&columns, &payloads);
        validate_scene_slot_overrides(&library, &index, stable(PANEL)).unwrap();
        validate_scene_slot_overrides(&library, &index, stable(2102)).unwrap();
        drop(index);
        // Remove의 inverse(Insert)가 nested 인스턴스와 그 slot content를 그대로 되살립니다.
        let inverse = SlotContentCommand::Remove {
            instance: stable(PANEL),
            slot: slot(OUTER),
            node: stable(2101),
            restore_default: true,
        }
        .apply(&library, &mut columns, &mut payloads)
        .unwrap();
        assert_eq!(columns.len(), 2);
        inverse
            .apply(&library, &mut columns, &mut payloads)
            .unwrap();
        assert_eq!(snapshot(&columns, &payloads), inserted);
    }

    const PANEL: u64 = 1201;
    const OUTER: u64 = 760;
    const INNER: u64 = 761;
    const PAIR: u64 = 762;

    /// Panel(7): 700 ← 702(outer host, slot 760: Frame|Text 0..=3) ← 703(Frame default, inner
    /// host, slot 761: Text 0..=2) ← 704(Text default); 705(pair host, slot 762: Text 2..=3)
    /// ← 706, 707(Text default).
    fn panel_library() -> ComponentLibrary {
        let panel = ComponentRef::new(stable(7));
        let mut tree = DefinitionTree::new(panel);
        for (id, kind, parent, order) in [
            (700, NodeKind::Frame, None, 0x80u8),
            (702, NodeKind::Frame, Some(700), 0x40),
            (703, NodeKind::Frame, Some(702), 0x40),
            (704, NodeKind::Text, Some(703), 0x40),
            (705, NodeKind::Frame, Some(700), 0x60),
            (706, NodeKind::Text, Some(705), 0x20),
            (707, NodeKind::Text, Some(705), 0x40),
        ] {
            tree.insert(DefinitionNode {
                id: stable(id),
                kind,
                parent: parent.map(stable),
                order_key: key(order),
                visible: true,
                locked: false,
                display_name: String::new(),
                payload: Vec::new(),
            })
            .unwrap();
        }
        let mut definition =
            ComponentDefinition::new(stable(7), "Panel".into(), NodeRef::new(stable(700)));
        for id in [OUTER, INNER, PAIR] {
            definition.add_slot(slot(id)).unwrap();
        }
        let mut library = ComponentLibrary::new();
        library.insert(definition, tree).unwrap();
        for (id, host, kinds, cardinality, default) in [
            (
                OUTER,
                702,
                vec![NodeKind::Frame, NodeKind::Text],
                SlotCardinality::at_most(3),
                vec![703],
            ),
            (
                INNER,
                703,
                vec![NodeKind::Text],
                SlotCardinality::at_most(2),
                vec![704],
            ),
            (
                PAIR,
                705,
                vec![NodeKind::Text],
                SlotCardinality::new(2, Some(3)).unwrap(),
                vec![706, 707],
            ),
        ] {
            library
                .add_slot(
                    panel,
                    ComponentSlot::new(
                        slot(id),
                        NodeRef::new(stable(host)),
                        kinds,
                        cardinality,
                        default
                            .into_iter()
                            .map(|d| NodeRef::new(stable(d)))
                            .collect(),
                    ),
                )
                .unwrap();
        }
        library
    }

    /// scene: 1000 ← 1201(Panel, override는 인자) ← `owned`(Text, 0x10부터 0x08 간격).
    fn panel_scene(overrides: &[(u64, &[u64])], owned: &[u64]) -> (NodeColumns, ScenePayloads) {
        let mut columns = NodeColumns::new();
        let mk = |id: u64, kind, parent, order: u8| NodeFields {
            stable_id: stable(id),
            kind,
            parent,
            order_key: key(order),
            visible: true,
            locked: false,
            display_name: String::new(),
        };
        let root = columns.insert(mk(1000, NodeKind::Frame, None, 0x80));
        let instance = columns.insert(mk(PANEL, NodeKind::ComponentInstance, Some(root), 0x40));
        for (i, id) in owned.iter().enumerate() {
            columns.insert(mk(*id, NodeKind::Text, Some(instance), 0x10 + 8 * i as u8));
        }
        let mut payload = ComponentInstancePayload::new(ComponentRef::new(stable(7)));
        for (id, content) in overrides {
            payload.set_slot_override(
                slot(*id),
                SlotOverride::new(content.iter().copied().map(stable).collect()).unwrap(),
            );
        }
        let mut payloads = ScenePayloads::new();
        payloads.insert(stable(PANEL), payload.encode());
        (columns, payloads)
    }

    fn panel_insert(slot_id: u64, id: u64, order: u8) -> SlotContentCommand {
        SlotContentCommand::Insert {
            instance: stable(PANEL),
            slot: slot(slot_id),
            index: 0,
            nodes: vec![text(id, PANEL, order)],
        }
    }

    #[test]
    fn first_insert_into_default_slot_checks_minimum_on_result() {
        // default 2개를 보이던 slot(최소 2)에 첫 삽입 → 1개짜리 override가 되므로 거부.
        let library = panel_library();
        let (mut columns, mut payloads) = panel_scene(&[], &[]);
        let before = snapshot(&columns, &payloads);
        let insert = panel_insert(PAIR, 2101, 0x70);
        assert_eq!(
            insert
                .apply(&library, &mut columns, &mut payloads)
                .unwrap_err(),
            SlotCommandError::Rule(SlotError::CardinalityViolation {
                slot: slot(PAIR),
                count: 1,
            })
        );
        assert_eq!(snapshot(&columns, &payloads), before, "거부 시 scene 불변");

        // 이미 2개 override면 세 번째(최대 3)는 통과하고 투영 규칙도 유지.
        let (mut columns, mut payloads) = panel_scene(&[(PAIR, &[1202, 1203])], &[1202, 1203]);
        insert.apply(&library, &mut columns, &mut payloads).unwrap();
        let index = SceneIndex::build(&columns, &payloads);
        validate_scene_slot_overrides(&library, &index, stable(PANEL)).unwrap();
    }

    #[test]
    fn inserts_creating_hidden_slot_content_are_rejected() {
        let library = panel_library();
        let hidden = SlotCommandError::Projection(SlotOverrideError::HiddenSlotContent {
            instance: stable(PANEL),
            slot: slot(INNER),
            hidden_by: slot(OUTER),
        });
        let inner = panel_insert(INNER, 2101, 0x70);
        let outer = panel_insert(OUTER, 2102, 0x78);

        // 바깥 slot이 default면 안쪽 slot 삽입은 허용.
        let (mut columns, mut payloads) = panel_scene(&[], &[]);
        inner.apply(&library, &mut columns, &mut payloads).unwrap();
        // 안쪽 content가 있는데 바깥 slot에 넣으면 안쪽 content가 가려짐 → 거부, 불변.
        let before = snapshot(&columns, &payloads);
        assert_eq!(
            outer
                .apply(&library, &mut columns, &mut payloads)
                .unwrap_err(),
            hidden
        );
        assert_eq!(snapshot(&columns, &payloads), before);

        // 바깥 slot이 override된 상태에서 가려진 안쪽 slot에 넣기 → 거부, 불변.
        let (mut columns, mut payloads) = panel_scene(&[(OUTER, &[1202])], &[1202]);
        let before = snapshot(&columns, &payloads);
        assert_eq!(
            inner
                .apply(&library, &mut columns, &mut payloads)
                .unwrap_err(),
            hidden
        );
        assert_eq!(snapshot(&columns, &payloads), before);

        // 바깥 slot을 default로 되돌리면 안쪽 slot이 다시 보여 삽입 가능.
        SlotContentCommand::Remove {
            instance: stable(PANEL),
            slot: slot(OUTER),
            node: stable(1202),
            restore_default: true,
        }
        .apply(&library, &mut columns, &mut payloads)
        .unwrap();
        inner.apply(&library, &mut columns, &mut payloads).unwrap();
        let index = SceneIndex::build(&columns, &payloads);
        validate_scene_slot_overrides(&library, &index, stable(PANEL)).unwrap();
    }
}
