//! 인스턴스 PropertyOverride validator (IX.2.3).
//!
//! 인스턴스는 자기가 참조하는 정의의 **공개(Public)** Property만 override할 수 있습니다(기술 아키텍처
//! 15.1). 다음은 모두 거부합니다:
//!
//! - **nested property 직접 접근**: 경로가 있는 [`OverrideKey`](nested instance chain을 거치는 key).
//!   nested 인스턴스의 값은 상위 정의가 Property를 만들고 명시적으로 drilling해야 바꿀 수
//!   있습니다(14.3). 저장 형식은 이 key를 표현할 수 있지만 validator가 받아들이지 않습니다.
//! - **private property**: 정의 내부 전용 property.
//! - **선언되지 않은 property**: 정의에 record가 없는 property id.
//! - **타입 불일치**: override 값의 타입이 property 선언 타입과 다름.
//!
//! scene 인스턴스는 [`validate_scene_property_overrides`], 정의 tree 안 nested 인스턴스는
//! [`validate_definition_property_overrides`]로 검사합니다. 참조 컴포넌트가 library에 없으면
//! (미해결 참조, III.2.4) scene 검사는 [`PropertyOverrideError::UnresolvedComponent`]를 돌려주고,
//! library 디코딩 경로는 그 인스턴스를 건너뜁니다(slot 투영 검사와 같은 규칙).

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::ComponentRef;
use stellar_core_types::typed_value::TypedValue;
use stellar_expressions::types::{ExpressionType, TypeSpec};

use super::definition::{ComponentEntry, ComponentLibrary, ComponentPropertyId};
use super::expansion::{ExpansionError, SceneIndex};
use super::instance::{ComponentInstancePayload, OverrideKey};
use crate::node_kind::NodeKind;

/// PropertyOverride 규칙 위반.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyOverrideError {
    /// 인스턴스·payload를 해석할 수 없습니다.
    Expansion(ExpansionError),
    /// 참조 컴포넌트가 library에 없습니다(미해결 참조, III.2.4).
    UnresolvedComponent {
        /// 인스턴스.
        instance: StableId,
        /// 참조 컴포넌트.
        component: ComponentRef,
    },
    /// nested instance 경로를 거치는 직접 접근입니다(drilling 필요).
    NestedAccess {
        /// 인스턴스.
        instance: StableId,
        /// 거부된 key.
        key: OverrideKey,
    },
    /// 정의에 없는 property입니다.
    UnknownProperty {
        /// 인스턴스.
        instance: StableId,
        /// property.
        property: ComponentPropertyId,
    },
    /// private property는 인스턴스에서 override할 수 없습니다.
    PrivateProperty {
        /// 인스턴스.
        instance: StableId,
        /// property.
        property: ComponentPropertyId,
    },
    /// override 값 타입이 property 선언 타입과 다릅니다.
    ValueTypeMismatch {
        /// 인스턴스.
        instance: StableId,
        /// property.
        property: ComponentPropertyId,
        /// 선언 타입.
        expected: TypeSpec,
        /// 값 타입.
        found: ExpressionType,
    },
}

impl fmt::Display for PropertyOverrideError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PropertyOverrideError::Expansion(err) => write!(f, "{err}"),
            PropertyOverrideError::UnresolvedComponent {
                instance,
                component,
            } => write!(
                f,
                "인스턴스 {instance}의 컴포넌트 {}를 찾을 수 없습니다",
                component.id()
            ),
            PropertyOverrideError::NestedAccess { instance, key } => write!(
                f,
                "인스턴스 {instance}가 nested instance property {}에 직접 접근합니다(drilling 필요)",
                key.property().stable_id()
            ),
            PropertyOverrideError::UnknownProperty { instance, property } => write!(
                f,
                "인스턴스 {instance}의 컴포넌트에 property {}가 없습니다",
                property.stable_id()
            ),
            PropertyOverrideError::PrivateProperty { instance, property } => write!(
                f,
                "인스턴스 {instance}는 private property {}를 override할 수 없습니다",
                property.stable_id()
            ),
            PropertyOverrideError::ValueTypeMismatch {
                instance,
                property,
                expected,
                found,
            } => write!(
                f,
                "인스턴스 {instance}의 property {} override 타입 {found}가 선언 타입 {expected}와 다릅니다",
                property.stable_id()
            ),
        }
    }
}

impl std::error::Error for PropertyOverrideError {}

impl From<ExpansionError> for PropertyOverrideError {
    fn from(err: ExpansionError) -> PropertyOverrideError {
        PropertyOverrideError::Expansion(err)
    }
}

/// override 하나(key·값)가 정의 `entry`의 규칙을 지키는지 검사합니다.
pub fn check_override(
    entry: &ComponentEntry,
    instance: StableId,
    key: &OverrideKey,
    value: &TypedValue,
) -> Result<(), PropertyOverrideError> {
    if !key.is_direct() {
        return Err(PropertyOverrideError::NestedAccess {
            instance,
            key: key.clone(),
        });
    }
    let property = key.property();
    let record = entry
        .property(property)
        .filter(|_| entry.definition().properties().contains(&property))
        .ok_or(PropertyOverrideError::UnknownProperty { instance, property })?;
    if !record.is_public() {
        return Err(PropertyOverrideError::PrivateProperty { instance, property });
    }
    if !record.value_type().accepts(Some(value)) {
        return Err(PropertyOverrideError::ValueTypeMismatch {
            instance,
            property,
            expected: record.value_type(),
            found: ExpressionType::of(value),
        });
    }
    Ok(())
}

/// payload의 모든 PropertyOverride를 정의 `entry`에 대해 검사합니다(key 오름차순, 첫 위반 반환).
pub fn check_payload_overrides(
    entry: &ComponentEntry,
    payload: &ComponentInstancePayload,
    instance: StableId,
) -> Result<(), PropertyOverrideError> {
    for (key, value) in payload.property_overrides() {
        check_override(entry, instance, key, value)?;
    }
    Ok(())
}

/// scene 인스턴스의 PropertyOverride를 검사합니다.
pub fn validate_scene_property_overrides(
    library: &ComponentLibrary,
    scene: &SceneIndex<'_>,
    instance: StableId,
) -> Result<(), PropertyOverrideError> {
    let payload = scene.instance_payload(instance)?;
    let entry =
        library
            .get(payload.component())
            .ok_or(PropertyOverrideError::UnresolvedComponent {
                instance,
                component: payload.component(),
            })?;
    check_payload_overrides(entry, &payload, instance)
}

/// 컴포넌트 정의 tree 안 nested 인스턴스들의 PropertyOverride를 검사합니다.
pub fn validate_definition_property_overrides(
    library: &ComponentLibrary,
    component: ComponentRef,
) -> Result<(), PropertyOverrideError> {
    check_definition_overrides(library, component, false)
}

/// [`validate_definition_property_overrides`] 본체. `skip_unresolved`면 library에 없는 컴포넌트를
/// 참조하는 nested 인스턴스는 건너뜁니다(library 디코딩 경로).
pub(super) fn check_definition_overrides(
    library: &ComponentLibrary,
    component: ComponentRef,
    skip_unresolved: bool,
) -> Result<(), PropertyOverrideError> {
    let entry = library
        .get(component)
        .ok_or(ExpansionError::NodeNotFound { id: component.id() })?;
    for node in entry.tree().iter() {
        if node.kind != NodeKind::ComponentInstance {
            continue;
        }
        let payload = ComponentInstancePayload::decode(&node.payload).map_err(|error| {
            ExpansionError::Payload {
                node: node.id,
                error,
            }
        })?;
        let Some(nested) = library.get(payload.component()) else {
            if skip_unresolved {
                continue;
            }
            return Err(PropertyOverrideError::UnresolvedComponent {
                instance: node.id,
                component: payload.component(),
            });
        };
        check_payload_overrides(nested, &payload, node.id)?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! Button·Card property fixture(IX.2.3·IX.2.4·IX.4.2 테스트 공용).

    use super::super::typed_expr::test_util::literal;
    use super::*;
    use crate::component::definition::{ComponentDefinition, DefinitionNode, DefinitionTree};
    use crate::component::property::{ComponentProperty, PropertyVisibility};
    use crate::node_columns::{NodeColumns, NodeFields};
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::reference::NodeRef;

    pub(crate) fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    pub(crate) fn pid(ts: u64) -> ComponentPropertyId {
        ComponentPropertyId::new(stable(ts))
    }

    pub(crate) fn button() -> ComponentRef {
        ComponentRef::new(stable(2))
    }

    pub(crate) fn card() -> ComponentRef {
        ComponentRef::new(stable(1))
    }

    /// Button(2) property: 800 label(String, public), 801 pressedTint(Boolean, private).
    pub(crate) const LABEL: u64 = 800;
    pub(crate) const PRIVATE_TINT: u64 = 801;
    /// Card(1) property: 700 title(String, public).
    pub(crate) const TITLE: u64 = 700;
    /// Card 정의 tree 안 nested Button 인스턴스.
    pub(crate) const NESTED_BUTTON: u64 = 106;

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

    fn property(
        id: u64,
        name: &str,
        value: TypedValue,
        visibility: PropertyVisibility,
    ) -> ComponentProperty {
        ComponentProperty::new(
            pid(id),
            name.into(),
            TypeSpec::required(ExpressionType::of(&value)),
            literal(value),
            visibility,
        )
        .unwrap()
    }

    /// Button(2): 200 ← 201(Text). Card(1): 100 ← 106(Button 인스턴스, `nested` payload).
    pub(crate) fn library_with(nested: ComponentInstancePayload) -> ComponentLibrary {
        let mut library = ComponentLibrary::new();
        let mut tree = DefinitionTree::new(button());
        tree.insert(def_node(200, NodeKind::Frame, None, 0x80, vec![]))
            .unwrap();
        tree.insert(def_node(201, NodeKind::Text, Some(200), 0x40, vec![]))
            .unwrap();
        let mut definition =
            ComponentDefinition::new(stable(2), "Button".into(), NodeRef::new(stable(200)));
        definition.add_property(pid(LABEL)).unwrap();
        definition.add_property(pid(PRIVATE_TINT)).unwrap();
        library.insert(definition, tree).unwrap();
        library
            .add_property(
                button(),
                property(
                    LABEL,
                    "label",
                    TypedValue::String("OK".into()),
                    PropertyVisibility::Public,
                ),
            )
            .unwrap();
        library
            .add_property(
                button(),
                property(
                    PRIVATE_TINT,
                    "pressedTint",
                    TypedValue::Boolean(false),
                    PropertyVisibility::Private,
                ),
            )
            .unwrap();

        let mut tree = DefinitionTree::new(card());
        tree.insert(def_node(100, NodeKind::Frame, None, 0x80, vec![]))
            .unwrap();
        tree.insert(def_node(
            NESTED_BUTTON,
            NodeKind::ComponentInstance,
            Some(100),
            0x40,
            nested.encode(),
        ))
        .unwrap();
        let mut definition =
            ComponentDefinition::new(stable(1), "Card".into(), NodeRef::new(stable(100)));
        definition.add_property(pid(TITLE)).unwrap();
        library.insert(definition, tree).unwrap();
        library
            .add_property(
                card(),
                property(
                    TITLE,
                    "title",
                    TypedValue::String("Card".into()),
                    PropertyVisibility::Public,
                ),
            )
            .unwrap();
        library
    }

    /// 기본 library: nested Button은 override 없음.
    pub(crate) fn library() -> ComponentLibrary {
        library_with(ComponentInstancePayload::new(button()))
    }

    /// scene: 1000(Frame) ← 1001(Card 인스턴스, `payload`).
    pub(crate) fn scene(
        payload: &ComponentInstancePayload,
    ) -> (NodeColumns, crate::component::expansion::ScenePayloads) {
        let mut columns = NodeColumns::new();
        let root = columns.insert(NodeFields {
            stable_id: stable(1000),
            kind: NodeKind::Frame,
            parent: None,
            order_key: OrderKey::from_bytes(vec![0x80]).unwrap(),
            visible: true,
            locked: false,
            display_name: String::new(),
        });
        columns.insert(NodeFields {
            stable_id: stable(1001),
            kind: NodeKind::ComponentInstance,
            parent: Some(root),
            order_key: OrderKey::from_bytes(vec![0x40]).unwrap(),
            visible: true,
            locked: false,
            display_name: String::new(),
        });
        let mut payloads = crate::component::expansion::ScenePayloads::new();
        payloads.insert(stable(1001), payload.encode());
        (columns, payloads)
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    fn card_payload(entries: &[(OverrideKey, TypedValue)]) -> ComponentInstancePayload {
        let mut payload = ComponentInstancePayload::new(card());
        for (key, value) in entries {
            payload.set_property_override(key.clone(), value.clone());
        }
        payload
    }

    fn check(entries: &[(OverrideKey, TypedValue)]) -> Result<(), PropertyOverrideError> {
        let library = library();
        let (columns, payloads) = scene(&card_payload(entries));
        let index = SceneIndex::build(&columns, &payloads);
        validate_scene_property_overrides(&library, &index, stable(1001))
    }

    #[test]
    fn public_direct_override_is_accepted() {
        check(&[(
            OverrideKey::direct(pid(TITLE)),
            TypedValue::String("주문".into()),
        )])
        .unwrap();
        check(&[]).unwrap();
    }

    #[test]
    fn direct_nested_access_fixture_fails() {
        // 검증 기준(IX.2.3): 직접 nested access fixture 실패. Card 인스턴스가 정의 안 nested Button의
        // label(공개 property여도)을 경로로 직접 바꾸려 합니다.
        let key = OverrideKey::nested(vec![stable(NESTED_BUTTON)], pid(LABEL));
        assert_eq!(
            check(&[(key.clone(), TypedValue::String("직접".into()))]).unwrap_err(),
            PropertyOverrideError::NestedAccess {
                instance: stable(1001),
                key,
            }
        );
    }

    #[test]
    fn private_unknown_and_mistyped_overrides_fail() {
        // Card 정의의 property가 아닌 Button private property(경로 없이)도 Card에는 없는 property.
        assert_eq!(
            check(&[(
                OverrideKey::direct(pid(PRIVATE_TINT)),
                TypedValue::Boolean(true)
            )])
            .unwrap_err(),
            PropertyOverrideError::UnknownProperty {
                instance: stable(1001),
                property: pid(PRIVATE_TINT),
            }
        );
        assert_eq!(
            check(&[(OverrideKey::direct(pid(TITLE)), TypedValue::Boolean(true))]).unwrap_err(),
            PropertyOverrideError::ValueTypeMismatch {
                instance: stable(1001),
                property: pid(TITLE),
                expected: TypeSpec::required(ExpressionType::String),
                found: ExpressionType::Boolean,
            }
        );
        // 정의 tree 안 nested Button 인스턴스가 Button의 private property를 override → 거부.
        let mut nested = ComponentInstancePayload::new(button());
        nested.set_property_override(
            OverrideKey::direct(pid(PRIVATE_TINT)),
            TypedValue::Boolean(true),
        );
        let library = library_with(nested);
        assert_eq!(
            validate_definition_property_overrides(&library, card()).unwrap_err(),
            PropertyOverrideError::PrivateProperty {
                instance: stable(NESTED_BUTTON),
                property: pid(PRIVATE_TINT),
            }
        );
        // public property override는 정의 안에서도 허용됩니다.
        let mut nested = ComponentInstancePayload::new(button());
        nested.set_property_override(
            OverrideKey::direct(pid(LABEL)),
            TypedValue::String("구매".into()),
        );
        validate_definition_property_overrides(&library_with(nested), card()).unwrap();
    }

    #[test]
    fn library_decode_rejects_invalid_definition_overrides() {
        // 정의 tree 안 nested 인스턴스의 nested 경로 key는 library 디코딩에서 거부됩니다.
        let mut nested = ComponentInstancePayload::new(button());
        nested.set_property_override(
            OverrideKey::nested(vec![stable(9_999)], pid(LABEL)),
            TypedValue::String("x".into()),
        );
        let invalid = library_with(nested);
        assert!(matches!(
            ComponentLibrary::decode(&invalid.encode()).unwrap_err(),
            crate::component::definition::ComponentDefinitionError::PropertyOverride(
                PropertyOverrideError::NestedAccess { .. }
            )
        ));
        // 유효한 library는 round-trip.
        let library = library();
        assert_eq!(
            ComponentLibrary::decode(&library.encode()).unwrap(),
            library
        );
        // 미해결 컴포넌트를 참조하는 scene 인스턴스는 진단용 오류로 구분됩니다.
        let (columns, payloads) = scene(&ComponentInstancePayload::new(ComponentRef::new(stable(
            77,
        ))));
        let index = SceneIndex::build(&columns, &payloads);
        assert!(matches!(
            validate_scene_property_overrides(&library, &index, stable(1001)),
            Err(PropertyOverrideError::UnresolvedComponent { .. })
        ));
    }
}
