//! Slot override 투영 규칙 (IX.3.2).
//!
//! slot override는 **인스턴스가 소유한 node**를 slot host 아래에 **투영**합니다. 정의 tree
//! 안으로 parent를 바꾸지 않습니다:
//!
//! - scene 인스턴스의 slot content는 scene에서 부모가 인스턴스 node 그대로인 node입니다.
//!   expansion(IX.2.2)에서만 [`ExpandedNodeId::Owned`](super::expansion::ExpandedNodeId)로
//!   slot host 아래에 나타납니다. 정의 tree는 바뀌지 않습니다.
//! - 정의 tree 안 nested 인스턴스의 slot content는 그 정의 tree에서 부모가 nested 인스턴스
//!   node인 node입니다.
//! - override가 없으면 slot host의 default content(정의 tree 자식)가 보입니다.
//!
//! 저장되는 것은 payload의 node 참조 목록뿐이라 save/load 후에도 소유 관계(부모 = 인스턴스)와
//! 투영이 그대로 유지됩니다. 이 모듈의 validator는 그 불변식을 검사합니다.

use std::collections::BTreeSet;
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::ComponentRef;

use super::definition::{ComponentEntry, ComponentLibrary, ComponentSlotId};
use super::expansion::{ExpansionError, SceneIndex};
use super::instance::{ComponentInstancePayload, InstancePayloadError};
use super::slot::SlotError;
use crate::node_kind::NodeKind;

/// slot override 투영 규칙 위반.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotOverrideError {
    /// 인스턴스 컴포넌트에 없는 slot을 override했습니다.
    UnknownSlot {
        /// 인스턴스.
        instance: StableId,
        /// slot.
        slot: ComponentSlotId,
    },
    /// content node가 인스턴스와 같은 tree(scene 또는 정의 tree)에 없습니다.
    ContentNotFound {
        /// 인스턴스.
        instance: StableId,
        /// node.
        node: StableId,
    },
    /// content node의 부모가 인스턴스가 아닙니다(인스턴스 소유가 아님).
    NotOwnedByInstance {
        /// 인스턴스.
        instance: StableId,
        /// node.
        node: StableId,
        /// 실제 부모.
        parent: Option<StableId>,
    },
    /// 같은 node가 두 slot(또는 한 slot에 두 번)에 투영됩니다.
    ProjectedTwice {
        /// 인스턴스.
        instance: StableId,
        /// node.
        node: StableId,
    },
    /// 인스턴스 소유 node가 어떤 slot에도 투영되지 않았습니다.
    UnprojectedOwnedNode {
        /// 인스턴스.
        instance: StableId,
        /// node.
        node: StableId,
    },
    /// slot의 종류·개수 규칙 위반.
    Rule {
        /// 인스턴스.
        instance: StableId,
        /// 원인.
        error: SlotError,
    },
    /// 인스턴스·정의를 해석할 수 없습니다.
    Expansion(ExpansionError),
}

impl fmt::Display for SlotOverrideError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SlotOverrideError::UnknownSlot { instance, slot } => write!(
                f,
                "인스턴스 {instance}가 없는 slot {}을 override했습니다",
                slot.stable_id()
            ),
            SlotOverrideError::ContentNotFound { instance, node } => {
                write!(f, "인스턴스 {instance}의 slot content {node}가 없습니다")
            }
            SlotOverrideError::NotOwnedByInstance {
                instance,
                node,
                parent,
            } => match parent {
                Some(parent) => write!(
                    f,
                    "slot content {node}의 부모는 {parent}입니다(인스턴스 {instance} 소유가 아님)"
                ),
                None => write!(
                    f,
                    "slot content {node}는 root node입니다(인스턴스 {instance} 소유가 아님)"
                ),
            },
            SlotOverrideError::ProjectedTwice { instance, node } => {
                write!(f, "인스턴스 {instance}에서 node {node}가 두 번 투영됩니다")
            }
            SlotOverrideError::UnprojectedOwnedNode { instance, node } => write!(
                f,
                "인스턴스 {instance} 소유 node {node}가 어떤 slot에도 투영되지 않았습니다"
            ),
            SlotOverrideError::Rule { instance, error } => {
                write!(f, "인스턴스 {instance} slot 규칙 위반: {error}")
            }
            SlotOverrideError::Expansion(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for SlotOverrideError {}

impl From<ExpansionError> for SlotOverrideError {
    fn from(err: ExpansionError) -> SlotOverrideError {
        SlotOverrideError::Expansion(err)
    }
}

/// 공통 투영 검사. `lookup`은 인스턴스와 같은 tree에서 node의 (종류, 부모)를 찾고,
/// `owned_children`은 그 tree에서 인스턴스의 자식 전체입니다.
fn check_projection(
    entry: &ComponentEntry,
    payload: &ComponentInstancePayload,
    instance: StableId,
    lookup: impl Fn(StableId) -> Option<(NodeKind, Option<StableId>)>,
    owned_children: &[StableId],
) -> Result<(), SlotOverrideError> {
    let mut projected = BTreeSet::new();
    for (slot_id, content) in payload.slot_overrides() {
        let slot = entry.slot(slot_id).ok_or(SlotOverrideError::UnknownSlot {
            instance,
            slot: slot_id,
        })?;
        let mut checked = Vec::with_capacity(content.len());
        for node in content.content() {
            let (kind, parent) = lookup(*node).ok_or(SlotOverrideError::ContentNotFound {
                instance,
                node: *node,
            })?;
            if parent != Some(instance) {
                return Err(SlotOverrideError::NotOwnedByInstance {
                    instance,
                    node: *node,
                    parent,
                });
            }
            if !projected.insert(*node) {
                return Err(SlotOverrideError::ProjectedTwice {
                    instance,
                    node: *node,
                });
            }
            checked.push((*node, kind));
        }
        slot.check_content(checked)
            .map_err(|error| SlotOverrideError::Rule { instance, error })?;
    }
    if let Some(node) = owned_children.iter().find(|c| !projected.contains(*c)) {
        return Err(SlotOverrideError::UnprojectedOwnedNode {
            instance,
            node: *node,
        });
    }
    Ok(())
}

/// scene 인스턴스의 slot override가 투영 규칙을 지키는지 검사합니다.
///
/// content는 scene에서 부모가 인스턴스인 node여야 하고(정의 tree node나 다른 부모의 node는
/// 거부), 각 slot의 종류·개수 규칙을 지켜야 하며, 인스턴스의 자식은 모두 어떤 slot에
/// 정확히 한 번 투영돼야 합니다.
pub fn validate_scene_slot_overrides(
    library: &ComponentLibrary,
    scene: &SceneIndex<'_>,
    instance: StableId,
) -> Result<(), SlotOverrideError> {
    let payload = scene.instance_payload(instance)?;
    let entry = library
        .get(payload.component())
        .ok_or(ExpansionError::UnresolvedComponent(
            payload.component().unresolved_at(
                stellar_core_types::property::PropertyPath::with_keys(
                    instance,
                    vec![super::instance::keys::COMPONENT_REF],
                ),
            ),
        ))?;
    check_projection(
        entry,
        &payload,
        instance,
        |node| {
            scene
                .fields(node)
                .map(|fields| (fields.kind, scene.parent(node)))
        },
        scene.children(instance),
    )
}

/// 컴포넌트 정의 tree 안 nested 인스턴스들의 slot override를 검사합니다.
///
/// nested 인스턴스의 content는 같은 정의 tree에서 부모가 그 인스턴스인 node여야 합니다.
pub fn validate_definition_slot_overrides(
    library: &ComponentLibrary,
    component: ComponentRef,
) -> Result<(), SlotOverrideError> {
    let entry = library
        .get(component)
        .ok_or(ExpansionError::NodeNotFound { id: component.id() })?;
    let tree = entry.tree();
    for node in tree.iter() {
        if node.kind != NodeKind::ComponentInstance {
            continue;
        }
        let payload = ComponentInstancePayload::decode_for(node.kind, &node.payload).map_err(
            |error: InstancePayloadError| ExpansionError::Payload {
                node: node.id,
                error,
            },
        )?;
        let nested =
            library
                .get(payload.component())
                .ok_or(ExpansionError::UnresolvedComponent(
                    payload.component().unresolved_at(
                        stellar_core_types::property::PropertyPath::with_keys(
                            node.id,
                            vec![super::instance::keys::COMPONENT_REF],
                        ),
                    ),
                ))?;
        check_projection(
            nested,
            &payload,
            node.id,
            |id| tree.node(id).map(|n| (n.kind, n.parent)),
            tree.children(node.id),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::definition::{ComponentDefinition, DefinitionNode, DefinitionTree};
    use crate::component::expansion::{ExpandedNodeId, InstanceExpansion, ScenePayloads};
    use crate::component::instance::SlotOverride;
    use crate::component::slot::{ComponentSlot, SlotCardinality};
    use crate::node_columns::{NodeColumns, NodeFields};
    use stellar_core_types::ids::RuntimeEntityId;
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::reference::NodeRef;

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    fn slot_id(ts: u64) -> ComponentSlotId {
        ComponentSlotId::new(stable(ts))
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

    fn card() -> ComponentRef {
        ComponentRef::new(stable(1))
    }

    fn icon() -> ComponentRef {
        ComponentRef::new(stable(2))
    }

    fn page() -> ComponentRef {
        ComponentRef::new(stable(3))
    }

    /// Card(1): 100 ← 101(Text), 102(body host, slot 600 Text|Frame|ComponentInstance, 0..=3)
    ///          ← 103(Text default). 105(footer host, slot 601 Text, 0..=1) — default 없음.
    /// Icon(2): 200 ← 201(Shape).
    /// Page(3): 300 ← 301(Card 인스턴스, slot 600 = [302]) ← 302(Text, 301 소유).
    fn fixture_library(page_projects_child: bool) -> ComponentLibrary {
        let mut library = ComponentLibrary::new();
        let mut tree = DefinitionTree::new(card());
        tree.insert(def_node(100, NodeKind::Frame, None, 0x80, vec![]))
            .unwrap();
        tree.insert(def_node(101, NodeKind::Text, Some(100), 0x20, vec![]))
            .unwrap();
        tree.insert(def_node(102, NodeKind::Frame, Some(100), 0x40, vec![]))
            .unwrap();
        tree.insert(def_node(103, NodeKind::Text, Some(102), 0x40, vec![]))
            .unwrap();
        tree.insert(def_node(105, NodeKind::Frame, Some(100), 0x60, vec![]))
            .unwrap();
        let mut definition =
            ComponentDefinition::new(stable(1), "Card".into(), NodeRef::new(stable(100)));
        definition.add_slot(slot_id(600)).unwrap();
        definition.add_slot(slot_id(601)).unwrap();
        library.insert(definition, tree).unwrap();
        library
            .add_slot(
                card(),
                ComponentSlot::new(
                    slot_id(600),
                    NodeRef::new(stable(102)),
                    [NodeKind::Text, NodeKind::Frame, NodeKind::ComponentInstance],
                    SlotCardinality::new(0, Some(3)).unwrap(),
                    vec![NodeRef::new(stable(103))],
                ),
            )
            .unwrap();
        library
            .add_slot(
                card(),
                ComponentSlot::new(
                    slot_id(601),
                    NodeRef::new(stable(105)),
                    [NodeKind::Text],
                    SlotCardinality::at_most(1),
                    vec![],
                ),
            )
            .unwrap();

        let mut icon_tree = DefinitionTree::new(icon());
        icon_tree
            .insert(def_node(200, NodeKind::Frame, None, 0x80, vec![]))
            .unwrap();
        icon_tree
            .insert(def_node(201, NodeKind::Shape, Some(200), 0x40, vec![]))
            .unwrap();
        library
            .insert(
                ComponentDefinition::new(stable(2), "Icon".into(), NodeRef::new(stable(200))),
                icon_tree,
            )
            .unwrap();

        let mut nested = ComponentInstancePayload::new(card());
        if page_projects_child {
            nested.set_slot_override(slot_id(600), SlotOverride::new(vec![stable(302)]).unwrap());
        }
        let mut page_tree = DefinitionTree::new(page());
        page_tree
            .insert(def_node(300, NodeKind::Frame, None, 0x80, vec![]))
            .unwrap();
        page_tree
            .insert(def_node(
                301,
                NodeKind::ComponentInstance,
                Some(300),
                0x40,
                nested.encode(),
            ))
            .unwrap();
        page_tree
            .insert(def_node(302, NodeKind::Text, Some(301), 0x40, vec![]))
            .unwrap();
        library
            .insert(
                ComponentDefinition::new(stable(3), "Page".into(), NodeRef::new(stable(300))),
                page_tree,
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

    /// scene: 1000(root) ← 1001(Card 인스턴스) ← 1002(Text), 1003(Frame) ← 1004(Text),
    ///        1005(Icon 인스턴스); 1000 ← 1006(Text, 인스턴스 소유 아님).
    fn scene(override_600: Vec<u64>) -> (NodeColumns, ScenePayloads) {
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
        columns.insert(fields(1006, NodeKind::Text, Some(root), 0x60));
        let mut payload = ComponentInstancePayload::new(card());
        payload.set_slot_override(
            slot_id(600),
            SlotOverride::new(override_600.into_iter().map(stable).collect()).unwrap(),
        );
        let mut payloads = ScenePayloads::new();
        payloads.insert(stable(1001), payload.encode());
        payloads.insert(stable(1005), ComponentInstancePayload::new(icon()).encode());
        (columns, payloads)
    }

    fn owned(id: u64) -> ExpandedNodeId {
        ExpandedNodeId::Owned(stable(id))
    }

    fn virt(chain: &[u64], source: u64) -> ExpandedNodeId {
        ExpandedNodeId::virtual_node(chain.iter().copied().map(stable).collect(), stable(source))
    }

    #[test]
    fn owned_nodes_are_projected_without_reparenting() {
        let library = fixture_library(true);
        let (columns, payloads) = scene(vec![1003, 1002, 1005]);
        let index = SceneIndex::build(&columns, &payloads);
        validate_scene_slot_overrides(&library, &index, stable(1001)).unwrap();

        let expansion = InstanceExpansion::new(&library, &index, stable(1001)).unwrap();
        // slot host 102는 override 순서대로 인스턴스 소유 node를 투영.
        assert_eq!(
            expansion.children(&virt(&[1001], 102)).unwrap(),
            vec![owned(1003), owned(1002), owned(1005)]
        );
        // override 없는 slot host 105는 default content(없음), 일반 node 101은 그대로.
        assert!(expansion.children(&virt(&[1001], 105)).unwrap().is_empty());
        // 투영된 node의 expansion 부모는 slot host, scene 부모는 인스턴스 그대로.
        assert_eq!(
            expansion.parent(&owned(1003)).unwrap(),
            Some(virt(&[1001], 102))
        );
        assert_eq!(index.parent(stable(1003)), Some(stable(1001)));
        assert_eq!(expansion.parent(&owned(1004)).unwrap(), Some(owned(1003)));
        assert_eq!(expansion.children(&owned(1003)).unwrap(), vec![owned(1004)]);
        // 소유 node가 인스턴스면 그 정의를 다시 펼침.
        assert_eq!(
            expansion.children(&owned(1005)).unwrap(),
            vec![virt(&[1005], 200)]
        );
        assert_eq!(
            expansion.parent(&virt(&[1005], 200)).unwrap(),
            Some(owned(1005))
        );
        // 정의 tree는 그대로(host 102의 자식은 default content 103뿐).
        let tree = library.get(card()).unwrap().tree();
        assert_eq!(tree.children(stable(102)), &[stable(103)]);
        // 인스턴스 밖 node는 expansion 범위가 아님.
        assert!(expansion.node(&owned(1006)).is_err());
        // 전체 순회에 투영 node 포함.
        let walk = expansion.walk().unwrap();
        assert!(walk.contains(&owned(1004)));
        assert!(walk.contains(&virt(&[1005], 201)));
        assert!(
            !walk.contains(&virt(&[1001], 103)),
            "override된 slot의 default는 숨김"
        );
    }

    #[test]
    fn default_content_shows_when_not_overridden() {
        let library = fixture_library(true);
        let (columns, mut payloads) = scene(vec![]);
        payloads.insert(stable(1001), ComponentInstancePayload::new(card()).encode());
        let index = SceneIndex::build(&columns, &payloads);
        let expansion = InstanceExpansion::new(&library, &index, stable(1001)).unwrap();
        assert_eq!(
            expansion.children(&virt(&[1001], 102)).unwrap(),
            vec![virt(&[1001], 103)]
        );
    }

    #[test]
    fn projection_rule_violations_are_reported() {
        let library = fixture_library(true);
        let check = |content: Vec<u64>| {
            let (columns, payloads) = scene(content);
            let index = SceneIndex::build(&columns, &payloads);
            validate_scene_slot_overrides(&library, &index, stable(1001))
        };
        // 인스턴스 소유가 아닌 node(부모가 scene root).
        assert_eq!(
            check(vec![1006, 1002, 1003]).unwrap_err(),
            SlotOverrideError::NotOwnedByInstance {
                instance: stable(1001),
                node: stable(1006),
                parent: Some(stable(1000)),
            }
        );
        // 정의 tree node를 content로 지정(정의 안으로 parent를 바꾸는 투영 금지).
        assert_eq!(
            check(vec![103]).unwrap_err(),
            SlotOverrideError::ContentNotFound {
                instance: stable(1001),
                node: stable(103),
            }
        );
        // 소유 node의 자손(1004)은 직접 투영 불가.
        assert!(matches!(
            check(vec![1004]).unwrap_err(),
            SlotOverrideError::NotOwnedByInstance { .. }
        ));
        // 소유 자식 누락.
        assert_eq!(
            check(vec![1002, 1003]).unwrap_err(),
            SlotOverrideError::UnprojectedOwnedNode {
                instance: stable(1001),
                node: stable(1005),
            }
        );

        // 두 slot에 같은 node, 금지 종류, 개수 초과, 없는 slot.
        let (columns, mut payloads) = scene(vec![1002, 1003, 1005]);
        let mut payload = ComponentInstancePayload::new(card());
        payload.set_slot_override(
            slot_id(600),
            SlotOverride::new(vec![stable(1002), stable(1003), stable(1005)]).unwrap(),
        );
        payload.set_slot_override(slot_id(601), SlotOverride::new(vec![stable(1002)]).unwrap());
        payloads.insert(stable(1001), payload.encode());
        let index = SceneIndex::build(&columns, &payloads);
        assert!(matches!(
            validate_scene_slot_overrides(&library, &index, stable(1001)).unwrap_err(),
            SlotOverrideError::ProjectedTwice { .. }
        ));

        let mut payload = ComponentInstancePayload::new(card());
        payload.set_slot_override(slot_id(600), SlotOverride::new(vec![stable(1002)]).unwrap());
        payload.set_slot_override(slot_id(601), SlotOverride::new(vec![stable(1003)]).unwrap());
        payloads.insert(stable(1001), payload.encode());
        let index = SceneIndex::build(&columns, &payloads);
        assert!(matches!(
            validate_scene_slot_overrides(&library, &index, stable(1001)).unwrap_err(),
            SlotOverrideError::Rule {
                error: SlotError::DisallowedKind { .. },
                ..
            }
        ));

        let mut payload = ComponentInstancePayload::new(card());
        payload.set_slot_override(slot_id(699), SlotOverride::empty());
        payloads.insert(stable(1001), payload.encode());
        let index = SceneIndex::build(&columns, &payloads);
        assert!(matches!(
            validate_scene_slot_overrides(&library, &index, stable(1001)).unwrap_err(),
            SlotOverrideError::UnknownSlot { .. }
        ));

        // 개수 초과: footer slot(최대 1)에 Text 2개.
        let mut columns = NodeColumns::new();
        let root = columns.insert(fields(1000, NodeKind::Frame, None, 0x80));
        let instance = columns.insert(fields(1001, NodeKind::ComponentInstance, Some(root), 0x40));
        columns.insert(fields(1002, NodeKind::Text, Some(instance), 0x20));
        columns.insert(fields(1007, NodeKind::Text, Some(instance), 0x30));
        let mut payload = ComponentInstancePayload::new(card());
        payload.set_slot_override(
            slot_id(601),
            SlotOverride::new(vec![stable(1002), stable(1007)]).unwrap(),
        );
        let mut payloads = ScenePayloads::new();
        payloads.insert(stable(1001), payload.encode());
        let index = SceneIndex::build(&columns, &payloads);
        assert!(matches!(
            validate_scene_slot_overrides(&library, &index, stable(1001)).unwrap_err(),
            SlotOverrideError::Rule {
                error: SlotError::CardinalityViolation { count: 2, .. },
                ..
            }
        ));
    }

    #[test]
    fn nested_instance_slot_content_comes_from_outer_definition() {
        let library = fixture_library(true);
        validate_definition_slot_overrides(&library, page()).unwrap();

        let mut columns = NodeColumns::new();
        let root = columns.insert(fields(1000, NodeKind::Frame, None, 0x80));
        columns.insert(fields(2001, NodeKind::ComponentInstance, Some(root), 0x40));
        let mut payloads = ScenePayloads::new();
        payloads.insert(stable(2001), ComponentInstancePayload::new(page()).encode());
        let index = SceneIndex::build(&columns, &payloads);
        let expansion = InstanceExpansion::new(&library, &index, stable(2001)).unwrap();
        // nested Card 인스턴스(301)의 body host는 Page 정의 node 302를 투영.
        assert_eq!(
            expansion.children(&virt(&[2001, 301], 102)).unwrap(),
            vec![virt(&[2001], 302)]
        );
        assert_eq!(
            expansion.parent(&virt(&[2001], 302)).unwrap(),
            Some(virt(&[2001, 301], 102))
        );
        // 투영하지 않은 nested 소유 node는 오류.
        let unprojected = fixture_library(false);
        assert_eq!(
            validate_definition_slot_overrides(&unprojected, page()).unwrap_err(),
            SlotOverrideError::UnprojectedOwnedNode {
                instance: stable(301),
                node: stable(302),
            }
        );
    }
}
