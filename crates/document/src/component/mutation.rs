//! 인스턴스 가상 subtree 구조 편집 validator (IX.2.5).
//!
//! 인스턴스 안에서는 **자유 구조 편집을 금지**합니다(기술 아키텍처 15.1). 허용되는 구조
//! 변경은 정의가 공개한 Slot에 대한 것뿐입니다:
//!
//! - 정의에서 온 가상 node(`ExpandedNodeId::Virtual`)는 삭제·이동할 수 없습니다. nested
//!   인스턴스 내부(그 slot 포함)는 바깥 인스턴스에서 직접 편집할 수 없습니다.
//! - 새 node 삽입·이동 목적지는 이 인스턴스(또는 slot에 놓인 실제 인스턴스)의 **공개 slot
//!   host**이거나, 이미 인스턴스가 소유한 실제 node(자유 구조) 아래여야 합니다.
//! - slot 목적지에는 허용 종류가, 출발지·목적지 모두에는 **적용 후 투영**의 개수 범위
//!   (최소·최대)가 적용됩니다. default content를 보이던 slot에 처음 넣으면 1개짜리 override가
//!   되므로 최소 개수가 2 이상이면 거부됩니다.
//! - 적용 후 투영은 가림 규칙(nested slot host, IX.3.1)도 지켜야 합니다: override된 바깥
//!   slot에 가려진 slot에 넣거나, 안쪽 slot에 content가 있는데 바깥 slot에 넣는 편집은
//!   거부합니다.
//!
//! 자유 편집이 필요하면 명시적 detach(IX.2.6)를 사용합니다. 이 validator는 적용 전 검사만
//! 하며 상태를 바꾸지 않습니다.

use std::fmt;

use stellar_core_types::ids::StableId;

use super::definition::{ComponentEntry, ComponentSlotId};
use super::expansion::{ExpandedNodeId, ExpansionError, InstanceExpansion};
use super::instance::{ComponentInstancePayload, InstancePayloadError, SlotOverride};
use super::slot::{ComponentSlot, SlotError};
use super::slot_override::{check_hidden_slot_content, SlotOverrideError};
use crate::node_kind::NodeKind;

/// 가상 subtree에 대한 구조 편집 요청.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VirtualMutation {
    /// node(와 subtree)를 삭제합니다.
    Delete {
        /// 대상.
        target: ExpandedNodeId,
    },
    /// node를 새 부모 아래로 옮깁니다(같은 부모면 재정렬).
    Move {
        /// 대상.
        target: ExpandedNodeId,
        /// 새 부모.
        new_parent: ExpandedNodeId,
    },
    /// 새 node를 삽입합니다.
    Insert {
        /// 부모.
        parent: ExpandedNodeId,
        /// 새 node 종류.
        kind: NodeKind,
    },
}

/// 구조 편집 거부 사유.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VirtualMutationError {
    /// 정의에서 온 node는 구조가 고정되어 삭제·이동할 수 없습니다.
    DefinitionNodeImmutable {
        /// 대상.
        target: ExpandedNodeId,
    },
    /// nested 인스턴스 내부는 바깥 인스턴스에서 직접 편집할 수 없습니다.
    NestedInstanceInternal {
        /// 대상 또는 목적지.
        node: ExpandedNodeId,
    },
    /// 목적지가 공개 Slot도, 인스턴스 소유 node도 아닙니다.
    NotASlot {
        /// 목적지.
        parent: ExpandedNodeId,
    },
    /// 자기 자신이나 자손 아래로 옮기려 합니다.
    CycleMove,
    /// slot 종류·개수 규칙 위반.
    Slot(SlotError),
    /// 적용 후 투영이 slot override 규칙(가려진 slot content 등)을 어깁니다.
    Projection(SlotOverrideError),
    /// 대상·목적지를 해석할 수 없습니다.
    Expansion(ExpansionError),
}

impl fmt::Display for VirtualMutationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VirtualMutationError::DefinitionNodeImmutable { target } => write!(
                f,
                "정의에서 온 node {target:?}는 삭제·이동할 수 없습니다. 필요하면 detach하세요"
            ),
            VirtualMutationError::NestedInstanceInternal { node } => write!(
                f,
                "nested 인스턴스 내부 {node:?}는 직접 편집할 수 없습니다. 공개 Property·Slot을 사용하세요"
            ),
            VirtualMutationError::NotASlot { parent } => {
                write!(f, "{parent:?}는 공개 Slot이 아니라 자식을 둘 수 없습니다")
            }
            VirtualMutationError::CycleMove => {
                f.write_str("자기 자신이나 자손 아래로 옮길 수 없습니다")
            }
            VirtualMutationError::Slot(err) => write!(f, "{err}"),
            VirtualMutationError::Projection(err) => write!(f, "적용 후 투영 규칙 위반: {err}"),
            VirtualMutationError::Expansion(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for VirtualMutationError {}

impl From<ExpansionError> for VirtualMutationError {
    fn from(err: ExpansionError) -> VirtualMutationError {
        VirtualMutationError::Expansion(err)
    }
}

/// 구조 편집 위치.
enum Position<'e> {
    /// 실제 인스턴스의 공개 slot host. `payload`는 그 인스턴스의 현재 payload입니다.
    Slot {
        instance: StableId,
        entry: &'e ComponentEntry,
        slot: &'e ComponentSlot,
        payload: ComponentInstancePayload,
    },
    /// 인스턴스 소유 실제 node 아래(자유 구조).
    Free,
}

impl Position<'_> {
    fn slot_key(&self) -> Option<(StableId, ComponentSlotId)> {
        match self {
            Position::Slot { instance, slot, .. } => Some((*instance, slot.id)),
            Position::Free => None,
        }
    }
}

/// slot 위치의 현재 override content(override가 없으면 빈 목록 — default content 표시 중).
fn current_content(slot: &ComponentSlot, payload: &ComponentInstancePayload) -> Vec<StableId> {
    payload
        .slot_override(slot.id)
        .map(|content| content.content().to_vec())
        .unwrap_or_default()
}

/// `payload`의 `slot` content를 `content`로 바꾼 **적용 후** payload가 개수 범위와 가림 규칙을
/// 지키는지 검사합니다. 새 node는 아직 id가 없어 NIL 자리표시자로 셉니다.
fn check_result(
    instance: StableId,
    entry: &ComponentEntry,
    slot: &ComponentSlot,
    payload: &mut ComponentInstancePayload,
    content: Vec<StableId>,
) -> Result<(), VirtualMutationError> {
    if !slot.cardinality.allows(content.len()) {
        return Err(VirtualMutationError::Slot(
            SlotError::CardinalityViolation {
                slot: slot.id,
                count: content.len(),
            },
        ));
    }
    let over = SlotOverride::new(content).map_err(|error| match error {
        InstancePayloadError::DuplicateSlotContent { node } => {
            VirtualMutationError::Slot(SlotError::DuplicateContent {
                slot: slot.id,
                node,
            })
        }
        other => VirtualMutationError::Expansion(ExpansionError::Payload {
            node: instance,
            error: other,
        }),
    })?;
    payload.set_slot_override(slot.id, over);
    check_hidden_slot_content(entry, payload, instance).map_err(VirtualMutationError::Projection)
}

/// `parent` 아래에 자식을 둘 수 있는 위치인지 분류합니다.
fn insertion_position<'e>(
    expansion: &InstanceExpansion<'e>,
    parent: &ExpandedNodeId,
) -> Result<Position<'e>, VirtualMutationError> {
    match parent {
        ExpandedNodeId::Virtual { instances, source } => {
            if instances.len() > 1 {
                return Err(VirtualMutationError::NestedInstanceInternal {
                    node: parent.clone(),
                });
            }
            let entry = expansion.resolve_chain(instances)?;
            entry
                .tree()
                .node(*source)
                .ok_or(ExpansionError::NotInExpansion { id: parent.clone() })?;
            let slot = entry
                .slot_for_host(*source)
                .ok_or(VirtualMutationError::NotASlot {
                    parent: parent.clone(),
                })?;
            let payload = expansion.chain_payload(instances)?.into_owned();
            Ok(Position::Slot {
                instance: instances[0],
                entry,
                slot,
                payload,
            })
        }
        ExpandedNodeId::Owned(_) => {
            let node = expansion.node(parent)?;
            if node.kind() == NodeKind::ComponentInstance {
                // 실제 인스턴스 아래에는 slot을 거쳐서만 둘 수 있습니다.
                return Err(VirtualMutationError::NotASlot {
                    parent: parent.clone(),
                });
            }
            Ok(Position::Free)
        }
    }
}

/// `target`을 현재 위치에서 떼어낼 수 있는지 분류합니다.
fn removal_position<'e>(
    expansion: &InstanceExpansion<'e>,
    target: &ExpandedNodeId,
) -> Result<Position<'e>, VirtualMutationError> {
    match target {
        ExpandedNodeId::Virtual { instances, .. } => {
            expansion.node(target)?;
            if instances.len() > 1 {
                Err(VirtualMutationError::NestedInstanceInternal {
                    node: target.clone(),
                })
            } else {
                Err(VirtualMutationError::DefinitionNodeImmutable {
                    target: target.clone(),
                })
            }
        }
        ExpandedNodeId::Owned(_) => {
            let parent = expansion
                .parent(target)?
                .ok_or(ExpansionError::NotInExpansion { id: target.clone() })?;
            insertion_position(expansion, &parent)
        }
    }
}

/// `node`가 `ancestor`(자신 포함)의 expansion 자손인지.
fn is_within(
    expansion: &InstanceExpansion<'_>,
    node: &ExpandedNodeId,
    ancestor: &ExpandedNodeId,
) -> Result<bool, VirtualMutationError> {
    let mut current = Some(node.clone());
    while let Some(id) = current {
        if &id == ancestor {
            return Ok(true);
        }
        current = expansion.parent(&id)?;
    }
    Ok(false)
}

fn check_slot_kind(slot: &ComponentSlot, node: StableId, kind: NodeKind) -> Result<(), SlotError> {
    if slot.allows_kind(kind) {
        Ok(())
    } else {
        Err(SlotError::DisallowedKind {
            slot: slot.id,
            node,
            kind,
        })
    }
}

/// slot에서 `node`를 뺀 적용 후 개수가 범위 안인지(빼기만으로는 가림이 생기지 않음).
fn check_leave(
    slot: &ComponentSlot,
    payload: &ComponentInstancePayload,
    node: StableId,
) -> Result<(), VirtualMutationError> {
    let remaining = current_content(slot, payload)
        .into_iter()
        .filter(|c| *c != node)
        .count();
    if slot.cardinality.allows(remaining) {
        Ok(())
    } else {
        Err(VirtualMutationError::Slot(
            SlotError::CardinalityViolation {
                slot: slot.id,
                count: remaining,
            },
        ))
    }
}

fn target_stable_id(id: &ExpandedNodeId) -> StableId {
    match id {
        ExpandedNodeId::Virtual { source, .. } => *source,
        ExpandedNodeId::Owned(node) => *node,
    }
}

/// 가상 subtree 구조 편집을 적용 전에 검증합니다. 통과해야만 적용할 수 있습니다.
pub fn validate_virtual_mutation(
    expansion: &InstanceExpansion<'_>,
    mutation: &VirtualMutation,
) -> Result<(), VirtualMutationError> {
    match mutation {
        VirtualMutation::Delete { target } => {
            if let Position::Slot { slot, payload, .. } = removal_position(expansion, target)? {
                check_leave(slot, &payload, target_stable_id(target))?;
            }
            Ok(())
        }
        VirtualMutation::Move { target, new_parent } => {
            let from = removal_position(expansion, target)?;
            let to = insertion_position(expansion, new_parent)?;
            if is_within(expansion, new_parent, target)? {
                return Err(VirtualMutationError::CycleMove);
            }
            let moved = target_stable_id(target);
            let same_slot = from.slot_key().is_some() && from.slot_key() == to.slot_key();
            if let Position::Slot {
                instance,
                entry,
                slot,
                payload,
            } = &to
            {
                let kind = expansion.node(target)?.kind();
                check_slot_kind(slot, moved, kind).map_err(VirtualMutationError::Slot)?;
                if same_slot {
                    // 같은 slot 안 재정렬: 개수·가림이 바뀌지 않습니다.
                    return Ok(());
                }
                let mut result = payload.clone();
                // 같은 인스턴스의 다른 slot에서 오면 출발 slot에서 뺀 상태까지 함께 봅니다.
                if let Position::Slot {
                    instance: from_instance,
                    slot: from_slot,
                    ..
                } = &from
                {
                    if from_instance == instance {
                        let remaining: Vec<StableId> = current_content(from_slot, &result)
                            .into_iter()
                            .filter(|c| *c != moved)
                            .collect();
                        let over = SlotOverride::new(remaining).map_err(|error| {
                            ExpansionError::Payload {
                                node: *instance,
                                error,
                            }
                        })?;
                        result.set_slot_override(from_slot.id, over);
                    }
                }
                let mut content = current_content(slot, &result);
                content.push(moved);
                check_result(*instance, entry, slot, &mut result, content)?;
            }
            if let Position::Slot { slot, payload, .. } = &from {
                if !same_slot {
                    check_leave(slot, payload, moved)?;
                }
            }
            Ok(())
        }
        VirtualMutation::Insert { parent, kind } => {
            if let Position::Slot {
                instance,
                entry,
                slot,
                mut payload,
            } = insertion_position(expansion, parent)?
            {
                check_slot_kind(slot, StableId::NIL, *kind).map_err(VirtualMutationError::Slot)?;
                // 적용 후 투영: default를 보이던 slot이면 1개짜리 override가 됩니다.
                let mut content = current_content(slot, &payload);
                content.push(StableId::NIL);
                check_result(instance, entry, slot, &mut payload, content)?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::definition::{
        ComponentDefinition, ComponentLibrary, DefinitionNode, DefinitionTree,
    };
    use crate::component::expansion::{SceneIndex, ScenePayloads};
    use crate::component::instance::{ComponentInstancePayload, SlotOverride};
    use crate::component::slot::SlotCardinality;
    use crate::node_columns::{NodeColumns, NodeFields};
    use stellar_core_types::ids::RuntimeEntityId;
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::reference::{ComponentRef, NodeRef};

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    fn slot_id(ts: u64) -> ComponentSlotId {
        ComponentSlotId::new(stable(ts))
    }

    fn component(ts: u64) -> ComponentRef {
        ComponentRef::new(stable(ts))
    }

    fn def_node(
        id: u64,
        kind: NodeKind,
        parent: Option<u64>,
        key: u8,
        payload: Vec<u8>,
    ) -> DefinitionNode {
        DefinitionNode {
            id: stable(id),
            kind,
            parent: parent.map(stable),
            order_key: OrderKey::from_bytes(vec![key]).unwrap(),
            visible: true,
            locked: false,
            display_name: String::new(),
            payload,
        }
    }

    /// Icon(2): 200 ← 201(Shape).
    /// Card(1): 100 ← 101(Text), 102(body host, slot 600: Text|Frame|ComponentInstance, 1..=3)
    ///          ← 103(Text default), 105(footer host, slot 601: Text, 0..=1), 106(Icon 인스턴스).
    /// Page(3): 300 ← 301(Card 인스턴스, slot 600 = [302]) ← 302(Text).
    fn library() -> ComponentLibrary {
        let mut library = ComponentLibrary::new();
        let mut icon = DefinitionTree::new(component(2));
        icon.insert(def_node(200, NodeKind::Frame, None, 0x80, vec![]))
            .unwrap();
        icon.insert(def_node(201, NodeKind::Shape, Some(200), 0x40, vec![]))
            .unwrap();
        library
            .insert(
                ComponentDefinition::new(stable(2), "Icon".into(), NodeRef::new(stable(200))),
                icon,
            )
            .unwrap();

        let mut card = DefinitionTree::new(component(1));
        card.insert(def_node(100, NodeKind::Frame, None, 0x80, vec![]))
            .unwrap();
        card.insert(def_node(101, NodeKind::Text, Some(100), 0x20, vec![]))
            .unwrap();
        card.insert(def_node(102, NodeKind::Frame, Some(100), 0x40, vec![]))
            .unwrap();
        card.insert(def_node(103, NodeKind::Text, Some(102), 0x40, vec![]))
            .unwrap();
        card.insert(def_node(105, NodeKind::Frame, Some(100), 0x60, vec![]))
            .unwrap();
        card.insert(def_node(
            106,
            NodeKind::ComponentInstance,
            Some(100),
            0x70,
            ComponentInstancePayload::new(component(2)).encode(),
        ))
        .unwrap();
        let mut definition =
            ComponentDefinition::new(stable(1), "Card".into(), NodeRef::new(stable(100)));
        definition.add_slot(slot_id(600)).unwrap();
        definition.add_slot(slot_id(601)).unwrap();
        library.insert(definition, card).unwrap();
        library
            .add_slot(
                component(1),
                ComponentSlot::new(
                    slot_id(600),
                    NodeRef::new(stable(102)),
                    [NodeKind::Text, NodeKind::Frame, NodeKind::ComponentInstance],
                    SlotCardinality::new(1, Some(3)).unwrap(),
                    vec![NodeRef::new(stable(103))],
                ),
            )
            .unwrap();
        library
            .add_slot(
                component(1),
                ComponentSlot::new(
                    slot_id(601),
                    NodeRef::new(stable(105)),
                    [NodeKind::Text],
                    SlotCardinality::at_most(1),
                    vec![],
                ),
            )
            .unwrap();

        let mut nested = ComponentInstancePayload::new(component(1));
        nested.set_slot_override(slot_id(600), SlotOverride::new(vec![stable(302)]).unwrap());
        let mut page = DefinitionTree::new(component(3));
        page.insert(def_node(300, NodeKind::Frame, None, 0x80, vec![]))
            .unwrap();
        page.insert(def_node(
            301,
            NodeKind::ComponentInstance,
            Some(300),
            0x40,
            nested.encode(),
        ))
        .unwrap();
        page.insert(def_node(302, NodeKind::Text, Some(301), 0x40, vec![]))
            .unwrap();
        library
            .insert(
                ComponentDefinition::new(stable(3), "Page".into(), NodeRef::new(stable(300))),
                page,
            )
            .unwrap();
        library
    }

    fn fields(id: u64, kind: NodeKind, parent: Option<RuntimeEntityId>, key: u8) -> NodeFields {
        NodeFields {
            stable_id: stable(id),
            kind,
            parent,
            order_key: OrderKey::from_bytes(vec![key]).unwrap(),
            visible: true,
            locked: false,
            display_name: String::new(),
        }
    }

    /// scene: 1000 ← 1001(Card, slot 600 = `body`) ← 1002(Text), 1003(Frame) ← 1004(Text),
    ///        1005(Icon 인스턴스); 1000 ← 2001(Page 인스턴스).
    fn scene(body: &[u64]) -> (NodeColumns, ScenePayloads) {
        let mut columns = NodeColumns::new();
        let root = columns.insert(fields(1000, NodeKind::Frame, None, 0x80));
        let instance = columns.insert(fields(1001, NodeKind::ComponentInstance, Some(root), 0x40));
        columns.insert(fields(1002, NodeKind::Text, Some(instance), 0x20));
        let frame = columns.insert(fields(1003, NodeKind::Frame, Some(instance), 0x40));
        columns.insert(fields(1004, NodeKind::Text, Some(frame), 0x40));
        columns.insert(fields(
            1005,
            NodeKind::ComponentInstance,
            Some(instance),
            0x60,
        ));
        columns.insert(fields(2001, NodeKind::ComponentInstance, Some(root), 0x60));
        let mut payload = ComponentInstancePayload::new(component(1));
        payload.set_slot_override(
            slot_id(600),
            SlotOverride::new(body.iter().copied().map(stable).collect()).unwrap(),
        );
        let mut payloads = ScenePayloads::new();
        payloads.insert(stable(1001), payload.encode());
        payloads.insert(
            stable(1005),
            ComponentInstancePayload::new(component(2)).encode(),
        );
        payloads.insert(
            stable(2001),
            ComponentInstancePayload::new(component(3)).encode(),
        );
        (columns, payloads)
    }

    fn virt(chain: &[u64], source: u64) -> ExpandedNodeId {
        ExpandedNodeId::virtual_node(chain.iter().copied().map(stable).collect(), stable(source))
    }

    fn owned(id: u64) -> ExpandedNodeId {
        ExpandedNodeId::Owned(stable(id))
    }

    fn delete(target: ExpandedNodeId) -> VirtualMutation {
        VirtualMutation::Delete { target }
    }

    fn mv(target: ExpandedNodeId, new_parent: ExpandedNodeId) -> VirtualMutation {
        VirtualMutation::Move { target, new_parent }
    }

    fn insert(parent: ExpandedNodeId, kind: NodeKind) -> VirtualMutation {
        VirtualMutation::Insert { parent, kind }
    }

    #[test]
    fn nested_child_delete_and_move_are_rejected() {
        // 검증 기준(IX.2.5): nested child delete/move 거부.
        let library = library();
        let (columns, payloads) = scene(&[1002, 1003, 1005]);
        let index = SceneIndex::build(&columns, &payloads);
        let card = InstanceExpansion::new(&library, &index, stable(1001)).unwrap();

        // 정의에서 온 자식(제목 Text, default content, nested 인스턴스 node).
        for target in [virt(&[1001], 101), virt(&[1001], 106), virt(&[1001], 102)] {
            assert_eq!(
                validate_virtual_mutation(&card, &delete(target.clone())).unwrap_err(),
                VirtualMutationError::DefinitionNodeImmutable {
                    target: target.clone()
                }
            );
            assert!(matches!(
                validate_virtual_mutation(&card, &mv(target, virt(&[1001], 105))).unwrap_err(),
                VirtualMutationError::DefinitionNodeImmutable { .. }
            ));
        }
        // nested 인스턴스(Icon) 내부 자식.
        let icon_child = virt(&[1001, 106], 201);
        assert!(matches!(
            validate_virtual_mutation(&card, &delete(icon_child.clone())).unwrap_err(),
            VirtualMutationError::NestedInstanceInternal { .. }
        ));
        assert!(matches!(
            validate_virtual_mutation(&card, &mv(icon_child, owned(1003))).unwrap_err(),
            VirtualMutationError::NestedInstanceInternal { .. }
        ));
        // slot에 놓인 실제 인스턴스(1005)의 정의 자식도 고정.
        assert!(matches!(
            validate_virtual_mutation(&card, &delete(virt(&[1005], 201))).unwrap_err(),
            VirtualMutationError::DefinitionNodeImmutable { .. }
        ));

        // Page 안 nested Card의 slot content(302)와 그 slot host는 바깥에서 편집 불가.
        let page = InstanceExpansion::new(&library, &index, stable(2001)).unwrap();
        assert!(matches!(
            validate_virtual_mutation(&page, &delete(virt(&[2001], 302))).unwrap_err(),
            VirtualMutationError::DefinitionNodeImmutable { .. }
        ));
        assert!(matches!(
            validate_virtual_mutation(&page, &insert(virt(&[2001, 301], 102), NodeKind::Text))
                .unwrap_err(),
            VirtualMutationError::NestedInstanceInternal { .. }
        ));
    }

    #[test]
    fn only_public_slots_accept_structure() {
        let library = library();
        let (columns, payloads) = scene(&[1002, 1003]);
        let index = SceneIndex::build(&columns, &payloads);
        let card = InstanceExpansion::new(&library, &index, stable(1001)).unwrap();

        // 공개 slot host에는 삽입 가능, 그 외 정의 node 아래는 금지.
        validate_virtual_mutation(&card, &insert(virt(&[1001], 102), NodeKind::Text)).unwrap();
        validate_virtual_mutation(&card, &insert(virt(&[1001], 105), NodeKind::Text)).unwrap();
        for parent in [virt(&[1001], 100), virt(&[1001], 101), virt(&[1001], 103)] {
            assert_eq!(
                validate_virtual_mutation(&card, &insert(parent.clone(), NodeKind::Text))
                    .unwrap_err(),
                VirtualMutationError::NotASlot { parent }
            );
        }
        // slot에 놓인 실제 node 아래는 자유 구조, 실제 인스턴스 바로 아래는 금지.
        validate_virtual_mutation(&card, &insert(owned(1003), NodeKind::Shape)).unwrap();
        assert!(matches!(
            validate_virtual_mutation(&card, &insert(owned(1005), NodeKind::Text)).unwrap_err(),
            VirtualMutationError::NotASlot { .. }
        ));
        // 허용되지 않은 종류.
        assert!(matches!(
            validate_virtual_mutation(&card, &insert(virt(&[1001], 102), NodeKind::Image))
                .unwrap_err(),
            VirtualMutationError::Slot(SlotError::DisallowedKind { .. })
        ));
    }

    #[test]
    fn slot_content_edits_follow_cardinality() {
        let library = library();
        let (columns, payloads) = scene(&[1002, 1003, 1005]);
        let index = SceneIndex::build(&columns, &payloads);
        let card = InstanceExpansion::new(&library, &index, stable(1001)).unwrap();

        // 가득 찬 body(최대 3)에 삽입 → 초과.
        assert_eq!(
            validate_virtual_mutation(&card, &insert(virt(&[1001], 102), NodeKind::Text))
                .unwrap_err(),
            VirtualMutationError::Slot(SlotError::CardinalityViolation {
                slot: slot_id(600),
                count: 4,
            })
        );
        // slot content 삭제·재정렬·다른 slot으로 이동·소유 node 아래로 이동은 허용.
        validate_virtual_mutation(&card, &delete(owned(1002))).unwrap();
        validate_virtual_mutation(&card, &mv(owned(1002), virt(&[1001], 102))).unwrap();
        validate_virtual_mutation(&card, &mv(owned(1002), virt(&[1001], 105))).unwrap();
        validate_virtual_mutation(&card, &mv(owned(1002), owned(1003))).unwrap();
        validate_virtual_mutation(&card, &delete(owned(1004))).unwrap();
        // footer는 Text만 허용.
        assert!(matches!(
            validate_virtual_mutation(&card, &mv(owned(1003), virt(&[1001], 105))).unwrap_err(),
            VirtualMutationError::Slot(SlotError::DisallowedKind { .. })
        ));
        // 자기 자손 아래로 이동.
        assert_eq!(
            validate_virtual_mutation(&card, &mv(owned(1003), owned(1004))).unwrap_err(),
            VirtualMutationError::CycleMove
        );
        // 소유 자손을 body slot으로 옮기면 개수 초과.
        assert!(matches!(
            validate_virtual_mutation(&card, &mv(owned(1004), virt(&[1001], 102))).unwrap_err(),
            VirtualMutationError::Slot(SlotError::CardinalityViolation { count: 4, .. })
        ));

        // body 최소 1개: 하나뿐인 content 삭제·이동은 거부.
        let (columns, payloads) = scene(&[1002]);
        let index = SceneIndex::build(&columns, &payloads);
        let card = InstanceExpansion::new(&library, &index, stable(1001)).unwrap();
        assert_eq!(
            validate_virtual_mutation(&card, &delete(owned(1002))).unwrap_err(),
            VirtualMutationError::Slot(SlotError::CardinalityViolation {
                slot: slot_id(600),
                count: 0,
            })
        );
        assert!(matches!(
            validate_virtual_mutation(&card, &mv(owned(1002), virt(&[1001], 105))).unwrap_err(),
            VirtualMutationError::Slot(SlotError::CardinalityViolation { count: 0, .. })
        ));
    }

    const PANEL: u64 = 1201;
    const OUTER: u64 = 760;
    const INNER: u64 = 761;
    const PAIR: u64 = 762;

    /// Panel(7): 700 ← 702(outer host, slot 760: Frame|Text 0..=3) ← 703(Frame default, inner
    /// host, slot 761: Text 0..=2) ← 704(Text default); 705(pair host, slot 762: Text 2..=3)
    /// ← 706, 707(Text default).
    fn panel_library() -> ComponentLibrary {
        let mut library = ComponentLibrary::new();
        let mut tree = DefinitionTree::new(component(7));
        for (id, kind, parent, order) in [
            (700, NodeKind::Frame, None, 0x80u8),
            (702, NodeKind::Frame, Some(700), 0x40),
            (703, NodeKind::Frame, Some(702), 0x40),
            (704, NodeKind::Text, Some(703), 0x40),
            (705, NodeKind::Frame, Some(700), 0x60),
            (706, NodeKind::Text, Some(705), 0x20),
            (707, NodeKind::Text, Some(705), 0x40),
        ] {
            tree.insert(def_node(id, kind, parent, order, vec![]))
                .unwrap();
        }
        let mut definition =
            ComponentDefinition::new(stable(7), "Panel".into(), NodeRef::new(stable(700)));
        for id in [OUTER, INNER, PAIR] {
            definition.add_slot(slot_id(id)).unwrap();
        }
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
                    component(7),
                    ComponentSlot::new(
                        slot_id(id),
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
        let root = columns.insert(fields(1000, NodeKind::Frame, None, 0x80));
        let instance = columns.insert(fields(PANEL, NodeKind::ComponentInstance, Some(root), 0x40));
        for (i, id) in owned.iter().enumerate() {
            columns.insert(fields(
                *id,
                NodeKind::Text,
                Some(instance),
                0x10 + 8 * i as u8,
            ));
        }
        let mut payload = ComponentInstancePayload::new(component(7));
        for (id, content) in overrides {
            payload.set_slot_override(
                slot_id(*id),
                SlotOverride::new(content.iter().copied().map(stable).collect()).unwrap(),
            );
        }
        let mut payloads = ScenePayloads::new();
        payloads.insert(stable(PANEL), payload.encode());
        (columns, payloads)
    }

    #[test]
    fn first_insert_or_move_into_default_slot_checks_minimum_on_result() {
        // default 2개를 보이던 pair slot(최소 2)에 하나 넣으면 1개짜리 override → 거부.
        let library = panel_library();
        let (columns, payloads) = panel_scene(&[(OUTER, &[1202])], &[1202]);
        let index = SceneIndex::build(&columns, &payloads);
        let panel = InstanceExpansion::new(&library, &index, stable(PANEL)).unwrap();
        let below_minimum = VirtualMutationError::Slot(SlotError::CardinalityViolation {
            slot: slot_id(PAIR),
            count: 1,
        });
        assert_eq!(
            validate_virtual_mutation(&panel, &insert(virt(&[PANEL], 705), NodeKind::Text))
                .unwrap_err(),
            below_minimum
        );
        assert_eq!(
            validate_virtual_mutation(&panel, &mv(owned(1202), virt(&[PANEL], 705))).unwrap_err(),
            below_minimum
        );

        // 이미 2개 override면 세 번째는 통과, 네 번째(최대 3 초과)는 거부.
        let (columns, payloads) = panel_scene(&[(PAIR, &[1203, 1204])], &[1203, 1204]);
        let index = SceneIndex::build(&columns, &payloads);
        let panel = InstanceExpansion::new(&library, &index, stable(PANEL)).unwrap();
        validate_virtual_mutation(&panel, &insert(virt(&[PANEL], 705), NodeKind::Text)).unwrap();
        let (columns, payloads) = panel_scene(&[(PAIR, &[1203, 1204, 1205])], &[1203, 1204, 1205]);
        let index = SceneIndex::build(&columns, &payloads);
        let panel = InstanceExpansion::new(&library, &index, stable(PANEL)).unwrap();
        assert_eq!(
            validate_virtual_mutation(&panel, &insert(virt(&[PANEL], 705), NodeKind::Text))
                .unwrap_err(),
            VirtualMutationError::Slot(SlotError::CardinalityViolation {
                slot: slot_id(PAIR),
                count: 4,
            })
        );
    }

    #[test]
    fn edits_creating_hidden_slot_content_are_rejected() {
        let library = panel_library();
        let hidden = VirtualMutationError::Projection(SlotOverrideError::HiddenSlotContent {
            instance: stable(PANEL),
            slot: slot_id(INNER),
            hidden_by: slot_id(OUTER),
        });
        let check = |overrides: &[(u64, &[u64])], owned_nodes: &[u64], mutation| {
            let (columns, payloads) = panel_scene(overrides, owned_nodes);
            let index = SceneIndex::build(&columns, &payloads);
            let panel = InstanceExpansion::new(&library, &index, stable(PANEL)).unwrap();
            validate_virtual_mutation(&panel, &mutation)
        };
        // 바깥이 default면 안쪽 host 삽입 허용, 바깥이 override되면 가려진 host 삽입 거부.
        check(&[], &[], insert(virt(&[PANEL], 703), NodeKind::Text)).unwrap();
        assert_eq!(
            check(
                &[(OUTER, &[1202])],
                &[1202],
                insert(virt(&[PANEL], 703), NodeKind::Text)
            )
            .unwrap_err(),
            hidden
        );
        // 안쪽 content가 있는데 바깥 slot에 삽입하면 안쪽이 가려짐 → 거부.
        assert_eq!(
            check(
                &[(INNER, &[1203])],
                &[1203],
                insert(virt(&[PANEL], 702), NodeKind::Text)
            )
            .unwrap_err(),
            hidden
        );
        // 안쪽의 마지막 content를 바깥으로 옮기면 안쪽이 비므로 허용(적용 후 투영 기준).
        check(
            &[(INNER, &[1203])],
            &[1203],
            mv(owned(1203), virt(&[PANEL], 702)),
        )
        .unwrap();
        // 안쪽에 content가 남으면 거부.
        assert_eq!(
            check(
                &[(INNER, &[1203, 1204])],
                &[1203, 1204],
                mv(owned(1203), virt(&[PANEL], 702))
            )
            .unwrap_err(),
            hidden
        );
    }
}
