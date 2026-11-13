//! Variant override의 ComponentInstance 캡슐화 검사 (IX.7.7).
//!
//! Variant는 일반 레이어의 추가·삭제·이동·교체를 허용하지만, **nested Component 내부 변경은 Property·Drilling·
//! Slot만** 허용합니다(기술 아키텍처 16장). [`validate_variant_encapsulation`]은 IX.7.3 `VariantOverrideSet`에
//! 인스턴스 캡슐화 validator(IX.2.3·IX.2.5·IX.3.2·IX.4.2)를 연결합니다:
//!
//! - **property patch**: 경로 분류(IX.4.2 [`classify_property_path`]) — 정의 tree node를 root로 한 경로(인스턴스
//!   내부 직접 접근), nested override 경로, 인스턴스의 컴포넌트 참조·slot override key 경로를 거부합니다. 인스턴스
//!   override 경로는 공개 property여야 하고 값 타입이 맞아야 합니다(IX.2.3).
//! - **구조 patch**: 대상·부모가 정의 tree node(인스턴스 가상 subtree 안)면 거부합니다.
//! - **적용 후 slot 투영**: 구조 patch·slot change를 적용한 최종 구조에서 모든 ComponentInstance의 소유 node(최종
//!   부모 = 인스턴스)가 slot 투영 규칙(IX.3.2)을 지켜야 합니다. 인스턴스 아래에 slot에 투영되지 않는 node를
//!   추가하거나, slot content를 삭제·이동해 투영이 깨지는 patch를 거부합니다.
//!
//! 참조 컴포넌트가 library에 없는 인스턴스(미해결 참조, III.2.4)의 투영 검사는 건너뜁니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::ComponentRef;

use crate::component::definition::ComponentLibrary;
use crate::component::encapsulation::{classify_property_path, EncapsulationError, PathTarget};
use crate::component::expansion::SceneIndex;
use crate::component::instance::{ComponentInstancePayload, OverrideKey};
use crate::component::override_rules::{check_override, PropertyOverrideError};
use crate::component::slot_override::{validate_record_slot_overrides, SlotOverrideError};
use crate::component::variant_override::{SlotChange, StructurePatch, VariantOverrideSet};
use crate::node_kind::NodeKind;

/// Variant 캡슐화 위반.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VariantEncapsulationError {
    /// property patch 경로가 캡슐화를 어깁니다(IX.4.2).
    Path(EncapsulationError),
    /// 인스턴스 override 값이 규칙을 어깁니다(IX.2.3).
    Override(PropertyOverrideError),
    /// 구조 patch의 대상·부모가 컴포넌트 정의 tree node입니다(인스턴스 내부 구조 직접 편집).
    DefinitionInternalStructure {
        /// node.
        node: StableId,
        /// 소유 컴포넌트.
        component: ComponentRef,
    },
    /// 적용 후 인스턴스 slot 투영이 깨집니다(IX.3.2).
    Projection(SlotOverrideError),
    /// 인스턴스 payload를 해석할 수 없습니다.
    Payload {
        /// 인스턴스.
        instance: StableId,
    },
}

impl fmt::Display for VariantEncapsulationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VariantEncapsulationError::Path(err) => write!(f, "{err}"),
            VariantEncapsulationError::Override(err) => write!(f, "{err}"),
            VariantEncapsulationError::DefinitionInternalStructure { node, component } => write!(
                f,
                "Variant 구조 patch가 컴포넌트 {} 내부 node {node}를 바꿉니다",
                component.id()
            ),
            VariantEncapsulationError::Projection(err) => {
                write!(f, "Variant 적용 후 slot 투영 위반: {err}")
            }
            VariantEncapsulationError::Payload { instance } => {
                write!(f, "인스턴스 {instance} payload를 해석할 수 없습니다")
            }
        }
    }
}

impl std::error::Error for VariantEncapsulationError {}

/// 최종 구조 node: (종류, 부모, payload).
struct FinalNode<'a> {
    kind: NodeKind,
    parent: Option<StableId>,
    payload: Option<&'a [u8]>,
}

/// `set`이 base View(`view_root` subtree)의 ComponentInstance 캡슐화를 지키는지 검사합니다.
///
/// 참조 무결성(대상 존재, cycle 등)은 [`VariantOverrideSet::validate_references`]의 몫이며, 이 검사는 그 위에서
/// 캡슐화 규칙만 봅니다. 검사 순서: property patch(경로 순) → 구조 patch(node 순) → 적용 후 투영(인스턴스 순).
pub fn validate_variant_encapsulation(
    set: &VariantOverrideSet,
    base: &SceneIndex<'_>,
    view_root: StableId,
    library: &ComponentLibrary,
) -> Result<(), VariantEncapsulationError> {
    // 1) property patch.
    for (path, value) in set.property_patches() {
        let target = classify_property_path(library, base, path).map_err(|err| match err {
            EncapsulationError::Override(err) => VariantEncapsulationError::Override(err),
            other => VariantEncapsulationError::Path(other),
        })?;
        if let PathTarget::InstanceProperty { instance, property } = target {
            let payload = base
                .instance_payload(instance)
                .map_err(|_| VariantEncapsulationError::Payload { instance })?;
            if let Some(entry) = library.get(payload.component()) {
                check_override(entry, instance, &OverrideKey::direct(property), value)
                    .map_err(VariantEncapsulationError::Override)?;
            }
        }
    }

    // 2) 구조 patch 대상·부모가 정의 tree node가 아닌지.
    let internal = |node: StableId| -> Result<(), VariantEncapsulationError> {
        if base.fields(node).is_none() {
            if let Some(component) = library.owner_of(node) {
                return Err(VariantEncapsulationError::DefinitionInternalStructure {
                    node,
                    component,
                });
            }
        }
        Ok(())
    };
    for (node, patch) in set.structure_patches() {
        match patch {
            StructurePatch::Add { parent, .. } => internal(*parent)?,
            StructurePatch::Move { parent, .. } => {
                internal(node)?;
                internal(*parent)?;
            }
            StructurePatch::Delete | StructurePatch::Replace { .. } => internal(node)?,
        }
    }

    // 3) 최종 구조 구성.
    let mut nodes: BTreeMap<StableId, FinalNode<'_>> = BTreeMap::new();
    let mut stack = vec![view_root];
    let mut guard = 0usize;
    while let Some(id) = stack.pop() {
        guard += 1;
        if guard > base.len() + 1 {
            break;
        }
        if let Some(fields) = base.fields(id) {
            nodes.insert(
                id,
                FinalNode {
                    kind: fields.kind,
                    parent: base.parent(id),
                    payload: base.payload(id),
                },
            );
            stack.extend(base.children(id).iter().copied());
        }
    }
    let mut removed: BTreeSet<StableId> = BTreeSet::new();
    for (node, patch) in set.structure_patches() {
        match patch {
            StructurePatch::Add {
                node: added,
                parent,
                ..
            } => {
                nodes.insert(
                    added.id,
                    FinalNode {
                        kind: added.kind,
                        parent: Some(*parent),
                        payload: Some(&added.payload),
                    },
                );
            }
            StructurePatch::Move { parent, .. } => {
                if let Some(entry) = nodes.get_mut(&node) {
                    entry.parent = Some(*parent);
                }
            }
            StructurePatch::Delete => {
                removed.insert(node);
            }
            StructurePatch::Replace { node: replacement } => {
                let parent = nodes.get(&node).and_then(|n| n.parent);
                removed.insert(node);
                nodes.insert(
                    replacement.id,
                    FinalNode {
                        kind: replacement.kind,
                        parent,
                        payload: Some(&replacement.payload),
                    },
                );
            }
        }
    }
    let limit = nodes.len() + 1;
    let live = |id: StableId| -> bool {
        let mut current = Some(id);
        let mut steps = 0usize;
        while let Some(node) = current {
            if removed.contains(&node) {
                return false;
            }
            if node == view_root {
                return true;
            }
            steps += 1;
            if steps > limit {
                return false;
            }
            current = nodes.get(&node).and_then(|n| n.parent);
        }
        false
    };
    let live_nodes: BTreeSet<StableId> = nodes.keys().copied().filter(|id| live(*id)).collect();

    // 4) 최종 구조의 인스턴스마다 slot 투영 검사.
    let mut slot_changes: BTreeMap<
        StableId,
        Vec<(crate::component::definition::ComponentSlotId, &SlotChange)>,
    > = BTreeMap::new();
    for (instance, slot, change) in set.slot_changes() {
        slot_changes
            .entry(instance)
            .or_default()
            .push((slot, change));
    }
    for instance in &live_nodes {
        let Some(node) = nodes.get(instance) else {
            continue;
        };
        if node.kind != NodeKind::ComponentInstance {
            continue;
        }
        let mut payload =
            ComponentInstancePayload::decode(node.payload.unwrap_or(&[])).map_err(|_| {
                VariantEncapsulationError::Payload {
                    instance: *instance,
                }
            })?;
        if library.get(payload.component()).is_none() {
            continue;
        }
        for (slot, change) in slot_changes.get(instance).into_iter().flatten() {
            match change {
                SlotChange::Set(content) => {
                    payload.set_slot_override(*slot, content.clone());
                }
                SlotChange::Reset => {
                    payload.remove_slot_override(*slot);
                }
            }
        }
        let owned: Vec<StableId> = live_nodes
            .iter()
            .copied()
            .filter(|id| nodes.get(id).and_then(|n| n.parent) == Some(*instance))
            .collect();
        validate_record_slot_overrides(
            library,
            &payload,
            *instance,
            |id| {
                live_nodes
                    .contains(&id)
                    .then(|| nodes.get(&id).map(|n| (n.kind, n.parent)))
                    .flatten()
            },
            &owned,
        )
        .map_err(VariantEncapsulationError::Projection)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::encapsulation::instance_override_path;
    use crate::component::instance::keys;
    use crate::component::override_rules::fixtures::*;
    use crate::component::variant_override::VariantNode;
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::property::{PropertyKey, PropertyPath};
    use stellar_core_types::typed_value::TypedValue;

    fn check(set: &VariantOverrideSet) -> Result<(), VariantEncapsulationError> {
        let library = library();
        let (columns, payloads) = scene(&ComponentInstancePayload::new(card()));
        let index = SceneIndex::build(&columns, &payloads);
        validate_variant_encapsulation(set, &index, stable(1000), &library)
    }

    fn text_node(id: u64) -> VariantNode {
        VariantNode {
            id: stable(id),
            kind: NodeKind::Text,
            visible: true,
            locked: false,
            display_name: String::new(),
            payload: Vec::new(),
        }
    }

    #[test]
    fn nested_direct_override_fixtures_fail() {
        // 검증 기준(IX.7.7): nested direct override fixture 실패.
        // 1) Card 정의 안 nested Button node(106)의 속성을 직접 patch.
        let mut set = VariantOverrideSet::new();
        set.set_property(
            PropertyPath::with_keys(stable(NESTED_BUTTON), vec![PropertyKey(0x0101)]),
            TypedValue::Boolean(false),
        );
        assert_eq!(
            check(&set).unwrap_err(),
            VariantEncapsulationError::Path(EncapsulationError::DefinitionInternalTarget {
                node: stable(NESTED_BUTTON),
                component: card(),
            })
        );
        // 2) 인스턴스 override 경로에 nested 단계를 이어 붙인 patch.
        let mut set = VariantOverrideSet::new();
        set.set_property(
            PropertyPath::with_keys(
                stable(1001),
                vec![keys::PROPERTY_OVERRIDES, keys::PROPERTY_OVERRIDES],
            )
            .with_key_segment(&pid(LABEL).stable_id().to_string()),
            TypedValue::String("직접".into()),
        );
        assert!(matches!(
            check(&set),
            Err(VariantEncapsulationError::Path(
                EncapsulationError::NestedOverridePath { .. }
            ))
        ));
        // 3) nested Button node를 구조 patch로 삭제·이동, 그 아래에 node 추가.
        for patch in [
            StructurePatch::Delete,
            StructurePatch::Move {
                parent: stable(1000),
                order_key: OrderKey::from_bytes(vec![0x20]).unwrap(),
            },
        ] {
            let mut set = VariantOverrideSet::new();
            set.set_structure(stable(NESTED_BUTTON), patch);
            assert_eq!(
                check(&set).unwrap_err(),
                VariantEncapsulationError::DefinitionInternalStructure {
                    node: stable(NESTED_BUTTON),
                    component: card(),
                }
            );
        }
        let mut set = VariantOverrideSet::new();
        set.set_structure(
            stable(2001),
            StructurePatch::Add {
                node: text_node(2001),
                parent: stable(NESTED_BUTTON),
                order_key: OrderKey::from_bytes(vec![0x20]).unwrap(),
            },
        );
        assert!(matches!(
            check(&set),
            Err(VariantEncapsulationError::DefinitionInternalStructure { .. })
        ));
        // 4) 인스턴스 아래에 slot에 투영되지 않는 node 추가(Card에는 slot 없음).
        let mut set = VariantOverrideSet::new();
        set.set_structure(
            stable(2002),
            StructurePatch::Add {
                node: text_node(2002),
                parent: stable(1001),
                order_key: OrderKey::from_bytes(vec![0x20]).unwrap(),
            },
        );
        assert_eq!(
            check(&set).unwrap_err(),
            VariantEncapsulationError::Projection(SlotOverrideError::UnprojectedOwnedNode {
                instance: stable(1001),
                node: stable(2002),
            })
        );
        // 5) 인스턴스 컴포넌트 참조 key patch, 타입이 틀린 공개 property patch.
        let mut set = VariantOverrideSet::new();
        set.set_property(
            PropertyPath::with_keys(stable(1001), vec![keys::COMPONENT_REF]),
            TypedValue::Boolean(true),
        );
        assert!(matches!(
            check(&set),
            Err(VariantEncapsulationError::Path(
                EncapsulationError::ProtectedInstanceKey { .. }
            ))
        ));
        let mut set = VariantOverrideSet::new();
        set.set_property(
            instance_override_path(stable(1001), pid(TITLE)),
            TypedValue::Boolean(true),
        );
        assert!(matches!(
            check(&set),
            Err(VariantEncapsulationError::Override(
                PropertyOverrideError::ValueTypeMismatch { .. }
            ))
        ));
    }

    #[test]
    fn property_drilling_and_general_layer_edits_are_allowed() {
        let mut set = VariantOverrideSet::new();
        // 인스턴스 공개 property override.
        set.set_property(
            instance_override_path(stable(1001), pid(TITLE)),
            TypedValue::String("결제 실패".into()),
        );
        // 일반 레이어 속성·구조 변경.
        set.set_property(
            PropertyPath::with_keys(stable(1000), vec![PropertyKey(0x0101)]),
            TypedValue::Boolean(false),
        );
        set.set_structure(
            stable(2003),
            StructurePatch::Add {
                node: text_node(2003),
                parent: stable(1000),
                order_key: OrderKey::from_bytes(vec![0x10]).unwrap(),
            },
        );
        check(&set).unwrap();
        // 인스턴스 node 자체의 삭제(일반 레이어 편집)도 허용됩니다.
        let mut set = VariantOverrideSet::new();
        set.set_structure(stable(1001), StructurePatch::Delete);
        check(&set).unwrap();
    }
}
