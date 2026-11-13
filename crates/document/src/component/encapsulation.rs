//! nested instance property 직접 수정 금지 (IX.4.2).
//!
//! 컴포넌트 경계를 넘는 값은 공개 Component Property와 명시적 drilling으로만 전달합니다(기술 아키텍처
//! 14.3). 이 모듈은 그 경계를 두 겹으로 지킵니다:
//!
//! 1. **API 미제공**: 편집 대상 해석 [`resolve_property_edit`]은 scene 인스턴스(또는 인스턴스가 소유한
//!    실제 scene node)만 대상으로 돌려줍니다. 인스턴스 가상 subtree 안의 nested instance(정의 tree node)를
//!    고르면 nested 경로 대신 [`EncapsulationError::RequiresDrilling`]을 돌려줍니다 — 상위 정의에
//!    Property를 만들고 drilling해야 한다는 뜻입니다(IX.4.3 `DrillingPlan`의 입력). override command
//!    (IX.2.4)도 인스턴스 정의의 property id만 받습니다.
//! 2. **validator 거부**: 저장 형식이 표현할 수 있는 직접 경로를 모두 거부합니다.
//!    - PropertyOverride map의 nested 경로 key(IX.2.3 [`PropertyOverrideError::NestedAccess`]).
//!    - root가 컴포넌트 정의 tree node인 [`PropertyPath`](variant patch·binding target 등): 인스턴스
//!      내부 node를 직접 가리키는 경로입니다([`EncapsulationError::DefinitionInternalTarget`]).
//!    - 인스턴스 override 경로([`instance_override_path`])의 key가 둘 이상이거나 tail이 property id 하나가
//!      아닌 경우(경로를 이어 붙인 nested 접근, [`EncapsulationError::NestedOverridePath`]).
//!    - 인스턴스의 `COMPONENT_REF`·`SLOT_OVERRIDES` key를 property patch로 바꾸는 경로(교체는 구조
//!      patch, slot은 slot command·slot change의 몫).

use std::fmt;
use std::str::FromStr;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PathSegment, PropertyPath};
use stellar_core_types::reference::ComponentRef;

use super::definition::{ComponentLibrary, ComponentPropertyId};
use super::expansion::{ExpandedNodeId, ExpansionError, InstanceExpansion, SceneIndex};
use super::instance::keys;
use super::override_rules::{validate_scene_property_overrides, PropertyOverrideError};
use crate::node_kind::NodeKind;

/// 캡슐화 위반.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncapsulationError {
    /// 인스턴스·expansion을 해석할 수 없습니다.
    Expansion(ExpansionError),
    /// override 규칙 위반(IX.2.3).
    Override(PropertyOverrideError),
    /// nested instance의 property는 상위 정의의 Property와 drilling으로만 바꿀 수 있습니다.
    RequiresDrilling {
        /// scene 인스턴스부터 대상 nested instance까지의 chain(바깥 → 안쪽, 마지막이 대상).
        chain: Vec<StableId>,
        /// 대상 nested instance가 참조하는 컴포넌트.
        component: ComponentRef,
    },
    /// 선택한 node는 ComponentInstance가 아니라 property 편집 대상이 아닙니다.
    NotAnInstance {
        /// node.
        node: ExpandedNodeId,
    },
    /// 경로 root가 컴포넌트 정의 tree 안 node입니다(인스턴스 내부 직접 접근).
    DefinitionInternalTarget {
        /// root node.
        node: StableId,
        /// 그 node를 소유한 컴포넌트.
        component: ComponentRef,
    },
    /// 인스턴스 override 경로가 property 하나를 직접 가리키는 형식이 아닙니다(nested 접근).
    NestedOverridePath {
        /// 거부된 경로.
        path: PropertyPath,
    },
    /// 인스턴스의 컴포넌트 참조·slot override는 property patch로 바꿀 수 없습니다.
    ProtectedInstanceKey {
        /// 거부된 경로.
        path: PropertyPath,
    },
}

impl fmt::Display for EncapsulationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EncapsulationError::Expansion(err) => write!(f, "{err}"),
            EncapsulationError::Override(err) => write!(f, "{err}"),
            EncapsulationError::RequiresDrilling { chain, component } => write!(
                f,
                "nested instance {}(컴포넌트 {})의 property는 drilling으로만 바꿀 수 있습니다",
                chain.last().copied().unwrap_or(StableId::NIL),
                component.id()
            ),
            EncapsulationError::NotAnInstance { node } => {
                write!(f, "{node:?}는 ComponentInstance가 아닙니다")
            }
            EncapsulationError::DefinitionInternalTarget { node, component } => write!(
                f,
                "경로 root {node}는 컴포넌트 {} 내부 node입니다",
                component.id()
            ),
            EncapsulationError::NestedOverridePath { path } => write!(
                f,
                "인스턴스 {} override 경로가 property 하나를 직접 가리키지 않습니다",
                path.root()
            ),
            EncapsulationError::ProtectedInstanceKey { path } => write!(
                f,
                "인스턴스 {}의 컴포넌트 참조·slot override는 property patch로 바꿀 수 없습니다",
                path.root()
            ),
        }
    }
}

impl std::error::Error for EncapsulationError {}

impl From<ExpansionError> for EncapsulationError {
    fn from(err: ExpansionError) -> EncapsulationError {
        EncapsulationError::Expansion(err)
    }
}

impl From<PropertyOverrideError> for EncapsulationError {
    fn from(err: PropertyOverrideError) -> EncapsulationError {
        EncapsulationError::Override(err)
    }
}

/// property 편집 대상: scene 인스턴스 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PropertyEditTarget {
    /// override를 저장할 scene 인스턴스.
    pub instance: StableId,
}

/// expansion에서 고른 node의 property를 편집할 대상을 해석합니다.
///
/// - 인스턴스 root(정의 root의 가상 투영)는 scene 인스턴스 자신입니다.
/// - 인스턴스가 소유한 scene node가 ComponentInstance면 그 scene 인스턴스입니다(slot content는 실제
///   scene node라 직접 편집 대상).
/// - 정의 tree 안 nested instance(가상 node)는 [`EncapsulationError::RequiresDrilling`]입니다.
pub fn resolve_property_edit(
    expansion: &InstanceExpansion<'_>,
    node: &ExpandedNodeId,
) -> Result<PropertyEditTarget, EncapsulationError> {
    let expanded = expansion.node(node)?;
    match node {
        ExpandedNodeId::Owned(id) => {
            if expanded.kind() != NodeKind::ComponentInstance {
                return Err(EncapsulationError::NotAnInstance { node: node.clone() });
            }
            Ok(PropertyEditTarget { instance: *id })
        }
        ExpandedNodeId::Virtual { instances, source } => {
            let is_root = expansion
                .library()
                .owner_of(*source)
                .and_then(|owner| expansion.library().get(owner))
                .is_some_and(|entry| entry.definition().root.id() == *source);
            if is_root {
                // chain 마지막 인스턴스의 root 투영. chain이 scene 인스턴스 하나면 그 인스턴스 자신입니다.
                return match instances.as_slice() {
                    [scene_instance] => Ok(PropertyEditTarget {
                        instance: *scene_instance,
                    }),
                    _ => Err(requires_drilling(expansion, instances, None)?),
                };
            }
            if expanded.kind() != NodeKind::ComponentInstance {
                return Err(EncapsulationError::NotAnInstance { node: node.clone() });
            }
            Err(requires_drilling(expansion, instances, Some(*source))?)
        }
    }
}

fn requires_drilling(
    expansion: &InstanceExpansion<'_>,
    instances: &[StableId],
    nested: Option<StableId>,
) -> Result<EncapsulationError, EncapsulationError> {
    let mut chain = instances.to_vec();
    if let Some(id) = nested {
        chain.push(id);
    }
    let target = chain.last().copied().unwrap_or(StableId::NIL);
    let library = expansion.library();
    let component = library
        .owner_of(target)
        .and_then(|owner| library.get(owner))
        .and_then(|entry| entry.tree().node(target))
        .and_then(|node| super::instance::ComponentInstancePayload::decode(&node.payload).ok())
        .map(|payload| payload.component())
        .ok_or(ExpansionError::NodeNotFound { id: target })?;
    Ok(EncapsulationError::RequiresDrilling { chain, component })
}

/// scene 인스턴스의 property override를 가리키는 canonical [`PropertyPath`].
///
/// 형식: root = 인스턴스, keys = `[PROPERTY_OVERRIDES]`, tail = property StableId의 소문자 hyphen UUID.
pub fn instance_override_path(instance: StableId, property: ComponentPropertyId) -> PropertyPath {
    PropertyPath::with_keys(instance, vec![keys::PROPERTY_OVERRIDES])
        .with_key_segment(&property.stable_id().to_string())
}

/// 경로 대상 분류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathTarget {
    /// scene 인스턴스의 공개 property override.
    InstanceProperty {
        /// 인스턴스.
        instance: StableId,
        /// property.
        property: ComponentPropertyId,
    },
    /// 일반 scene node 속성(인스턴스 node 자신의 공통 필드 포함).
    Node {
        /// node.
        node: StableId,
    },
}

/// `path`가 캡슐화를 지키는지 검사하고 대상을 분류합니다.
///
/// scene에 없는 root는 정의 tree node면 [`EncapsulationError::DefinitionInternalTarget`], 아니면
/// `NodeNotFound`입니다.
pub fn classify_property_path(
    library: &ComponentLibrary,
    scene: &SceneIndex<'_>,
    path: &PropertyPath,
) -> Result<PathTarget, EncapsulationError> {
    let root = path.root();
    let Some(fields) = scene.fields(root) else {
        if let Some(component) = library.owner_of(root) {
            return Err(EncapsulationError::DefinitionInternalTarget {
                node: root,
                component,
            });
        }
        return Err(ExpansionError::NodeNotFound { id: root }.into());
    };
    if fields.kind != NodeKind::ComponentInstance {
        return Ok(PathTarget::Node { node: root });
    }
    match path.keys().first() {
        Some(&k) if k == keys::PROPERTY_OVERRIDES => {
            let property = parse_override_tail(path)
                .ok_or_else(|| EncapsulationError::NestedOverridePath { path: path.clone() })?;
            let payload = scene.instance_payload(root)?;
            let entry = library.get(payload.component()).ok_or(
                PropertyOverrideError::UnresolvedComponent {
                    instance: root,
                    component: payload.component(),
                },
            )?;
            let record =
                entry
                    .property(property)
                    .ok_or(PropertyOverrideError::UnknownProperty {
                        instance: root,
                        property,
                    })?;
            if !record.is_public() {
                return Err(PropertyOverrideError::PrivateProperty {
                    instance: root,
                    property,
                }
                .into());
            }
            Ok(PathTarget::InstanceProperty {
                instance: root,
                property,
            })
        }
        Some(&k) if k == keys::COMPONENT_REF || k == keys::SLOT_OVERRIDES => {
            Err(EncapsulationError::ProtectedInstanceKey { path: path.clone() })
        }
        _ => Ok(PathTarget::Node { node: root }),
    }
}

/// 직접 override 경로의 tail에서 property id를 읽습니다. 형식이 아니면 `None`.
fn parse_override_tail(path: &PropertyPath) -> Option<ComponentPropertyId> {
    if path.keys().len() != 1 {
        return None;
    }
    let Some(PathSegment::Key(text)) = path.tail() else {
        return None;
    };
    let id = StableId::from_str(text).ok()?;
    // canonical 표기(소문자)만 허용합니다 — 같은 대상이 두 경로로 표현되지 않게 합니다.
    (id.to_string() == *text).then_some(ComponentPropertyId::new(id))
}

/// scene의 모든 ComponentInstance가 override 캡슐화 규칙을 지키는지 검사합니다(StableId 순).
///
/// 미해결 컴포넌트를 참조하는 인스턴스는 진단 대상이므로 건너뜁니다(III.2.4).
pub fn validate_scene_encapsulation(
    library: &ComponentLibrary,
    scene: &SceneIndex<'_>,
    nodes: impl IntoIterator<Item = StableId>,
) -> Result<(), EncapsulationError> {
    let mut ids: Vec<StableId> = nodes.into_iter().collect();
    ids.sort();
    ids.dedup();
    for id in ids {
        if scene.fields(id).map(|f| f.kind) != Some(NodeKind::ComponentInstance) {
            continue;
        }
        match validate_scene_property_overrides(library, scene, id) {
            Ok(()) | Err(PropertyOverrideError::UnresolvedComponent { .. }) => {}
            Err(err) => return Err(err.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::instance::{ComponentInstancePayload, OverrideKey};
    use crate::component::override_rules::fixtures::*;
    use stellar_core_types::typed_value::TypedValue;

    #[test]
    fn nested_instance_property_has_no_direct_edit_api() {
        // 인스턴스 가상 subtree 안 nested Button(정의 node 106)을 고르면 drilling이 필요합니다.
        let library = library();
        let (columns, payloads) = scene(&ComponentInstancePayload::new(card()));
        let index = SceneIndex::build(&columns, &payloads);
        let expansion = InstanceExpansion::new(&library, &index, stable(1001)).unwrap();
        let nested = ExpandedNodeId::virtual_node(vec![stable(1001)], stable(NESTED_BUTTON));
        assert_eq!(
            resolve_property_edit(&expansion, &nested).unwrap_err(),
            EncapsulationError::RequiresDrilling {
                chain: vec![stable(1001), stable(NESTED_BUTTON)],
                component: button(),
            }
        );
        // 인스턴스 root는 scene 인스턴스 자신을 편집합니다.
        assert_eq!(
            resolve_property_edit(&expansion, &expansion.root()).unwrap(),
            PropertyEditTarget {
                instance: stable(1001)
            }
        );
    }

    #[test]
    fn prohibited_direct_path_fixtures_fail() {
        // 검증 기준(IX.4.2): prohibited direct path fixture 실패.
        let library = library();
        let (columns, payloads) = scene(&ComponentInstancePayload::new(card()));
        let index = SceneIndex::build(&columns, &payloads);
        let classify = |path: &PropertyPath| classify_property_path(&library, &index, path);

        // 1) 정의 tree 안 nested Button node를 root로 한 경로(인스턴스 내부 직접 접근).
        assert_eq!(
            classify(&PropertyPath::with_keys(
                stable(NESTED_BUTTON),
                vec![keys::PROPERTY_OVERRIDES]
            ))
            .unwrap_err(),
            EncapsulationError::DefinitionInternalTarget {
                node: stable(NESTED_BUTTON),
                component: card(),
            }
        );
        // 2) Card 인스턴스 override 경로에 nested 단계를 이어 붙인 경로.
        let nested = PropertyPath::with_keys(
            stable(1001),
            vec![keys::PROPERTY_OVERRIDES, keys::PROPERTY_OVERRIDES],
        )
        .with_key_segment(&pid(LABEL).stable_id().to_string());
        assert_eq!(
            classify(&nested).unwrap_err(),
            EncapsulationError::NestedOverridePath { path: nested }
        );
        // 3) tail이 property id가 아닌 경로(대문자 UUID·chain 문자열).
        for tail in [
            pid(TITLE).stable_id().to_string().to_uppercase(),
            format!("{}/{}", stable(NESTED_BUTTON), pid(LABEL).stable_id()),
        ] {
            let path = PropertyPath::with_keys(stable(1001), vec![keys::PROPERTY_OVERRIDES])
                .with_key_segment(&tail);
            assert!(matches!(
                classify(&path),
                Err(EncapsulationError::NestedOverridePath { .. })
            ));
        }
        // 4) 컴포넌트 참조·slot override key.
        for key in [keys::COMPONENT_REF, keys::SLOT_OVERRIDES] {
            assert!(matches!(
                classify(&PropertyPath::with_keys(stable(1001), vec![key])),
                Err(EncapsulationError::ProtectedInstanceKey { .. })
            ));
        }
        // 5) payload의 nested 경로 key도 scene validator가 거부합니다.
        let mut payload = ComponentInstancePayload::new(card());
        payload.set_property_override(
            OverrideKey::nested(vec![stable(NESTED_BUTTON)], pid(LABEL)),
            TypedValue::String("직접".into()),
        );
        let (columns, payloads) = scene(&payload);
        let index = SceneIndex::build(&columns, &payloads);
        assert!(matches!(
            validate_scene_encapsulation(&library, &index, [stable(1000), stable(1001)]),
            Err(EncapsulationError::Override(
                PropertyOverrideError::NestedAccess { .. }
            ))
        ));
    }

    #[test]
    fn canonical_direct_paths_are_accepted() {
        let library = library();
        let (columns, payloads) = scene(&ComponentInstancePayload::new(card()));
        let index = SceneIndex::build(&columns, &payloads);
        let path = instance_override_path(stable(1001), pid(TITLE));
        assert_eq!(
            classify_property_path(&library, &index, &path).unwrap(),
            PathTarget::InstanceProperty {
                instance: stable(1001),
                property: pid(TITLE),
            }
        );
        // 인스턴스 node 자신의 공통 필드와 일반 node는 Node 대상.
        assert_eq!(
            classify_property_path(&library, &index, &PropertyPath::new(stable(1001))).unwrap(),
            PathTarget::Node { node: stable(1001) }
        );
        assert_eq!(
            classify_property_path(&library, &index, &PropertyPath::new(stable(1000))).unwrap(),
            PathTarget::Node { node: stable(1000) }
        );
        // private·없는 property는 override 규칙 오류.
        assert!(matches!(
            classify_property_path(
                &library,
                &index,
                &instance_override_path(stable(1001), pid(PRIVATE_TINT))
            ),
            Err(EncapsulationError::Override(
                PropertyOverrideError::UnknownProperty { .. }
            ))
        ));
        validate_scene_encapsulation(&library, &index, [stable(1001)]).unwrap();
    }
}
