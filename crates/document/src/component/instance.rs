//! ComponentInstance node payload (IX.2.1).
//!
//! 인스턴스는 정의를 복제하지 않고 [`ComponentRef`]로 참조합니다. 인스턴스가 바꾼 것만
//! **희소(sparse)** 하게 저장합니다(기술 아키텍처 15.1).
//!
//! - **PropertyOverride map**: [`OverrideKey`] → [`TypedValue`]. key는 Component Property
//!   StableId와, nested instance를 거치는 경우의 instance 경로로 구성됩니다. 경로가 빈 key가
//!   이 인스턴스 정의의 Property(직접 override)입니다. 경로가 있는 key(nested property 직접
//!   접근)는 표현만 가능하며, 허용 여부는 validator(IX.2.3·IX.4.2)가 판정합니다.
//! - **SlotOverride map**: [`ComponentSlotId`] → [`SlotOverride`]. slot에 투영할 인스턴스 소유
//!   node 목록입니다(투영 규칙은 IX.3.2).
//!
//! 값은 core-types `TypedValue`만 담습니다. 표현식 값은 Phase VIII 이후의 확장입니다.

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::ComponentRef;
use stellar_core_types::typed_value::{TypedValue, ValueDecodeError};

use super::definition::DefinitionNode;
use crate::frame::{Clipping, FramePayload};
use crate::modifier::{ModifierPayload, ModifierStack};
use crate::record_codec::decode_payload;
use crate::records::NodePayload;
use stellar_core_types::value::Transform2D;

use super::codec::{put_bytes, put_count, put_id, Malformed, Reader};
use super::definition::{ComponentPropertyId, ComponentSlotId};
use crate::node_kind::NodeKind;
use crate::reference_index::ReferenceIndex;

/// ComponentInstance 속성 key(0x0Axx). 숫자 값은 저장·진단 경로의 일부입니다.
pub mod keys {
    use stellar_core_types::property::PropertyKey;

    /// 참조하는 컴포넌트 정의(`ComponentRef`).
    pub const COMPONENT_REF: PropertyKey = PropertyKey(0x0A01);
    /// PropertyOverride map.
    pub const PROPERTY_OVERRIDES: PropertyKey = PropertyKey(0x0A02);
    /// SlotOverride map.
    pub const SLOT_OVERRIDES: PropertyKey = PropertyKey(0x0A03);
}

/// ComponentInstance payload 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstancePayloadError {
    /// slot content 목록에 같은 node가 두 번 있습니다.
    DuplicateSlotContent {
        /// 중복 node.
        node: StableId,
    },
    /// ComponentInstance가 아닌 node에 instance payload를 해석하려 했습니다.
    KindMismatch {
        /// 실제 종류.
        kind: NodeKind,
    },
    /// override 값 bytes가 유효한 `TypedValue`가 아닙니다.
    Value(ValueDecodeError),
    /// 입력이 부족하거나 형식이 아닙니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
}

impl fmt::Display for InstancePayloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstancePayloadError::DuplicateSlotContent { node } => {
                write!(f, "slot content에 node {node}가 중복됩니다")
            }
            InstancePayloadError::KindMismatch { kind } => write!(
                f,
                "{} node에는 ComponentInstance payload가 없습니다",
                kind.name()
            ),
            InstancePayloadError::Value(err) => write!(f, "override 값 디코드 실패: {err}"),
            InstancePayloadError::Malformed { reason } => {
                write!(f, "ComponentInstance binary 형식 위반: {reason}")
            }
        }
    }
}

impl std::error::Error for InstancePayloadError {}

impl From<Malformed> for InstancePayloadError {
    fn from(err: Malformed) -> InstancePayloadError {
        InstancePayloadError::Malformed { reason: err.0 }
    }
}

/// PropertyOverride map의 key.
///
/// `path`는 정의 tree 안 nested ComponentInstance node StableId의 바깥→안쪽 chain입니다.
/// 빈 경로면 이 인스턴스가 참조하는 정의의 Property를 직접 가리킵니다. 정렬은
/// `(path, property)` 사전식이며 canonical 직렬화 순서입니다.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OverrideKey {
    path: Vec<StableId>,
    property: ComponentPropertyId,
}

impl OverrideKey {
    /// 이 인스턴스 정의의 Property를 가리키는 key.
    pub fn direct(property: ComponentPropertyId) -> OverrideKey {
        OverrideKey {
            path: Vec::new(),
            property,
        }
    }

    /// nested instance 경로를 거치는 key(직접 nested 접근 표현).
    pub fn nested(path: Vec<StableId>, property: ComponentPropertyId) -> OverrideKey {
        OverrideKey { path, property }
    }

    /// 대상 Property.
    pub fn property(&self) -> ComponentPropertyId {
        self.property
    }

    /// nested instance 경로(빈 경로면 직접).
    pub fn path(&self) -> &[StableId] {
        &self.path
    }

    /// 경로 없이 이 정의의 Property를 직접 가리키는지.
    pub fn is_direct(&self) -> bool {
        self.path.is_empty()
    }
}

/// slot 하나의 override: slot에 투영할 인스턴스 소유 node 목록(투영 순서).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SlotOverride {
    content: Vec<StableId>,
}

impl SlotOverride {
    /// 빈 override(slot을 명시적으로 비움).
    pub fn empty() -> SlotOverride {
        SlotOverride::default()
    }

    /// content 목록으로 만듭니다. 같은 node가 두 번 있으면 거부합니다.
    pub fn new(content: Vec<StableId>) -> Result<SlotOverride, InstancePayloadError> {
        let mut seen = std::collections::BTreeSet::new();
        for node in &content {
            if !seen.insert(*node) {
                return Err(InstancePayloadError::DuplicateSlotContent { node: *node });
            }
        }
        Ok(SlotOverride { content })
    }

    /// 투영 순서의 content node 목록.
    pub fn content(&self) -> &[StableId] {
        &self.content
    }

    /// content 수.
    pub fn len(&self) -> usize {
        self.content.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    /// node의 투영 위치.
    pub fn position(&self, node: StableId) -> Option<usize> {
        self.content.iter().position(|n| *n == node)
    }
}

/// ComponentInstance node payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentInstancePayload {
    component: ComponentRef,
    property_overrides: BTreeMap<OverrideKey, TypedValue>,
    slot_overrides: BTreeMap<ComponentSlotId, SlotOverride>,
}

impl ComponentInstancePayload {
    /// override 없는 인스턴스 payload.
    pub fn new(component: ComponentRef) -> ComponentInstancePayload {
        ComponentInstancePayload {
            component,
            property_overrides: BTreeMap::new(),
            slot_overrides: BTreeMap::new(),
        }
    }

    /// 참조하는 컴포넌트 정의.
    pub fn component(&self) -> ComponentRef {
        self.component
    }

    /// key의 override 값.
    pub fn property_override(&self, key: &OverrideKey) -> Option<&TypedValue> {
        self.property_overrides.get(key)
    }

    /// override 항목(key 오름차순).
    pub fn property_overrides(&self) -> impl Iterator<Item = (&OverrideKey, &TypedValue)> + '_ {
        self.property_overrides.iter()
    }

    /// override 값을 설정하고 이전 값을 반환합니다.
    pub fn set_property_override(
        &mut self,
        key: OverrideKey,
        value: TypedValue,
    ) -> Option<TypedValue> {
        self.property_overrides.insert(key, value)
    }

    /// override를 지워(default로 복귀) 이전 값을 반환합니다. map entry 자체가 사라집니다.
    pub fn remove_property_override(&mut self, key: &OverrideKey) -> Option<TypedValue> {
        self.property_overrides.remove(key)
    }

    /// slot의 override.
    pub fn slot_override(&self, slot: ComponentSlotId) -> Option<&SlotOverride> {
        self.slot_overrides.get(&slot)
    }

    /// slot override 항목(slot id 오름차순).
    pub fn slot_overrides(&self) -> impl Iterator<Item = (ComponentSlotId, &SlotOverride)> + '_ {
        self.slot_overrides.iter().map(|(slot, o)| (*slot, o))
    }

    /// slot override를 설정하고 이전 값을 반환합니다.
    pub fn set_slot_override(
        &mut self,
        slot: ComponentSlotId,
        content: SlotOverride,
    ) -> Option<SlotOverride> {
        self.slot_overrides.insert(slot, content)
    }

    /// slot override를 지워(default content로 복귀) 이전 값을 반환합니다.
    pub fn remove_slot_override(&mut self, slot: ComponentSlotId) -> Option<SlotOverride> {
        self.slot_overrides.remove(&slot)
    }

    /// override가 하나도 없는지(희소 저장의 빈 상태).
    pub fn has_no_overrides(&self) -> bool {
        self.property_overrides.is_empty() && self.slot_overrides.is_empty()
    }

    /// 인스턴스 node의 참조(인스턴스 → 컴포넌트 정의)를 ReferenceIndex에 맞춥니다.
    ///
    /// 정의 삭제 전 inbound 인스턴스를 조회할 수 있게 합니다(VI.2.5).
    pub fn sync_references(&self, instance: StableId, index: &mut ReferenceIndex) {
        index.remove_all_from(instance);
        index.add_reference(instance, self.component.id());
    }

    /// canonical binary bytes로 인코딩합니다.
    ///
    /// 레이아웃: component id(16) + override 수(u32) + (경로 수 u32 + 경로 id* + property id
    /// 16 + 값 길이 u32 + `TypedValue` bytes)* + slot 수(u32) + (slot id 16 + content 수 u32 +
    /// node id*)*. map은 key 오름차순입니다.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        put_id(&mut buf, self.component.id());
        put_count(&mut buf, self.property_overrides.len());
        for (key, value) in &self.property_overrides {
            put_count(&mut buf, key.path.len());
            for id in &key.path {
                put_id(&mut buf, *id);
            }
            put_id(&mut buf, key.property.stable_id());
            put_bytes(&mut buf, &value.to_bytes());
        }
        put_count(&mut buf, self.slot_overrides.len());
        for (slot, content) in &self.slot_overrides {
            put_id(&mut buf, slot.stable_id());
            put_count(&mut buf, content.content.len());
            for node in &content.content {
                put_id(&mut buf, *node);
            }
        }
        buf
    }

    /// binary bytes에서 디코딩합니다. 오름차순 위반, 중복, trailing bytes는 거부합니다.
    pub fn decode(bytes: &[u8]) -> Result<ComponentInstancePayload, InstancePayloadError> {
        let mut reader = Reader::new(bytes);
        let mut payload = ComponentInstancePayload::new(ComponentRef::new(reader.id()?));
        let override_count = reader.count()?;
        let mut previous_key: Option<OverrideKey> = None;
        for _ in 0..override_count {
            let path_len = reader.count()?;
            let mut path = Vec::with_capacity(super::codec::capacity_hint(path_len));
            for _ in 0..path_len {
                path.push(reader.id()?);
            }
            let key = OverrideKey::nested(path, ComponentPropertyId::new(reader.id()?));
            if previous_key.as_ref().is_some_and(|p| *p >= key) {
                return Err(InstancePayloadError::Malformed {
                    reason: "PropertyOverride key가 오름차순이 아님",
                });
            }
            let value =
                TypedValue::from_bytes(reader.bytes()?).map_err(InstancePayloadError::Value)?;
            previous_key = Some(key.clone());
            payload.property_overrides.insert(key, value);
        }
        let slot_count = reader.count()?;
        let mut previous_slot: Option<ComponentSlotId> = None;
        for _ in 0..slot_count {
            let slot = ComponentSlotId::new(reader.id()?);
            if previous_slot.is_some_and(|p| p >= slot) {
                return Err(InstancePayloadError::Malformed {
                    reason: "SlotOverride key가 오름차순이 아님",
                });
            }
            previous_slot = Some(slot);
            let content_len = reader.count()?;
            let mut content = Vec::with_capacity(super::codec::capacity_hint(content_len));
            for _ in 0..content_len {
                content.push(reader.id()?);
            }
            payload
                .slot_overrides
                .insert(slot, SlotOverride::new(content)?);
        }
        reader.expect_end()?;
        Ok(payload)
    }

    /// node 종류를 확인한 뒤 디코딩합니다. ComponentInstance가 아니면 거부합니다.
    pub fn decode_for(
        kind: NodeKind,
        bytes: &[u8],
    ) -> Result<ComponentInstancePayload, InstancePayloadError> {
        if kind != NodeKind::ComponentInstance {
            return Err(InstancePayloadError::KindMismatch { kind });
        }
        ComponentInstancePayload::decode(bytes)
    }
}

/// 고정 Frame 정의를 배치하는 인스턴스의 활성 affine을 저장 순서로 합성합니다.
pub fn frame_instance_transform(stack: Option<&ModifierStack>) -> Option<Transform2D> {
    let mut position = Transform2D::identity();
    for modifier in stack
        .into_iter()
        .flat_map(ModifierStack::entries)
        .filter(|m| m.enabled)
    {
        let ModifierPayload::Transform(next) = modifier.payload else {
            return None;
        };
        position = position.then(&next).ok()?;
    }
    Some(position)
}

/// 정의 root의 캔버스 원점만 배치 원점으로 대체합니다. 자식은 정의의 상대 좌표를 유지합니다.
pub fn placed_free_frame(
    node: &DefinitionNode,
    root_placement: Option<Transform2D>,
) -> Option<FramePayload> {
    let NodePayload::Frame(mut frame) = decode_payload(node.kind, &node.payload).ok()? else {
        return None;
    };
    if frame.layout_mode.is_some() || matches!(frame.clipping, Clipping::Path(_)) {
        return None;
    }
    if let Some(placement) = root_placement {
        let (a, b, c, d, _, _) = frame.transform.components();
        frame.transform = Transform2D::new(a, b, c, d, 0., 0.)
            .ok()?
            .then(&placement)
            .ok()?;
    }
    Some(frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::typed_value::EnumValue;
    use stellar_core_types::value::{FiniteF32, Length};

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    fn property(ts: u64) -> ComponentPropertyId {
        ComponentPropertyId::new(stable(ts))
    }

    fn slot(ts: u64) -> ComponentSlotId {
        ComponentSlotId::new(stable(ts))
    }

    /// 여러 값 타입·nested key·slot override를 담은 payload.
    fn sample() -> ComponentInstancePayload {
        let mut payload = ComponentInstancePayload::new(ComponentRef::new(stable(1)));
        payload.set_property_override(
            OverrideKey::direct(property(500)),
            TypedValue::String("확인".to_string()),
        );
        payload.set_property_override(
            OverrideKey::direct(property(501)),
            TypedValue::Boolean(true),
        );
        payload.set_property_override(
            OverrideKey::direct(property(502)),
            TypedValue::Color(Color::new(ColorSpace::DisplayP3, 0.1, 0.2, 0.3, 1.0).unwrap()),
        );
        payload.set_property_override(
            OverrideKey::direct(property(503)),
            TypedValue::Length(Length::new(12.5).unwrap()),
        );
        payload.set_property_override(
            OverrideKey::nested(vec![stable(300), stable(301)], property(504)),
            TypedValue::Enum(EnumValue::new(3, 1)),
        );
        payload.set_slot_override(
            slot(600),
            SlotOverride::new(vec![stable(900), stable(901)]).unwrap(),
        );
        payload.set_slot_override(slot(601), SlotOverride::empty());
        payload
    }

    #[test]
    fn payload_round_trips() {
        // 검증 기준(IX.2.1): round-trip.
        let payload = sample();
        let bytes = payload.encode();
        let decoded = ComponentInstancePayload::decode(&bytes).unwrap();
        assert_eq!(decoded, payload);
        assert_eq!(decoded.encode(), bytes, "재인코딩 byte-identical");
        // 내용 확인: nested key와 slot 순서 보존.
        assert_eq!(
            decoded.property_override(&OverrideKey::nested(
                vec![stable(300), stable(301)],
                property(504)
            )),
            Some(&TypedValue::Enum(EnumValue::new(3, 1)))
        );
        assert_eq!(
            decoded.slot_override(slot(600)).unwrap().content(),
            &[stable(900), stable(901)]
        );
        assert!(decoded.slot_override(slot(601)).unwrap().is_empty());
        assert_eq!(
            ComponentInstancePayload::decode_for(NodeKind::ComponentInstance, &bytes).unwrap(),
            payload
        );
    }

    #[test]
    fn encoding_is_independent_of_insertion_order() {
        let a = sample();
        let mut b = ComponentInstancePayload::new(ComponentRef::new(stable(1)));
        // 역순으로 설정.
        b.set_slot_override(slot(601), SlotOverride::empty());
        b.set_slot_override(
            slot(600),
            SlotOverride::new(vec![stable(900), stable(901)]).unwrap(),
        );
        b.set_property_override(
            OverrideKey::nested(vec![stable(300), stable(301)], property(504)),
            TypedValue::Enum(EnumValue::new(3, 1)),
        );
        b.set_property_override(
            OverrideKey::direct(property(503)),
            TypedValue::Length(Length::new(12.5).unwrap()),
        );
        b.set_property_override(
            OverrideKey::direct(property(502)),
            TypedValue::Color(Color::new(ColorSpace::DisplayP3, 0.1, 0.2, 0.3, 1.0).unwrap()),
        );
        b.set_property_override(
            OverrideKey::direct(property(501)),
            TypedValue::Boolean(true),
        );
        b.set_property_override(
            OverrideKey::direct(property(500)),
            TypedValue::String("확인".to_string()),
        );
        assert_eq!(a.encode(), b.encode());
    }

    #[test]
    fn storage_is_sparse() {
        // override 없는 인스턴스는 component id + 빈 map 두 개뿐.
        let empty = ComponentInstancePayload::new(ComponentRef::new(stable(1)));
        assert!(empty.has_no_overrides());
        assert_eq!(empty.encode().len(), 16 + 4 + 4);

        // override를 설정했다 지우면 map entry가 사라져 빈 payload와 같은 bytes.
        let mut payload = empty.clone();
        let key = OverrideKey::direct(property(500));
        assert_eq!(
            payload.set_property_override(key.clone(), TypedValue::Number(FiniteF32::ONE)),
            None
        );
        payload.set_slot_override(slot(600), SlotOverride::empty());
        assert_eq!(
            payload.remove_property_override(&key),
            Some(TypedValue::Number(FiniteF32::ONE))
        );
        assert!(payload.remove_slot_override(slot(600)).is_some());
        assert_eq!(payload.encode(), empty.encode());
        assert!(payload.has_no_overrides());
    }

    #[test]
    fn malformed_payloads_are_rejected() {
        let bytes = sample().encode();
        for cut in [0, 15, 16, 19, bytes.len() - 1] {
            assert!(
                ComponentInstancePayload::decode(&bytes[..cut]).is_err(),
                "절단 {cut}"
            );
        }
        let mut extended = bytes.clone();
        extended.push(0);
        assert!(ComponentInstancePayload::decode(&extended).is_err());

        // ComponentInstance가 아닌 node 종류.
        assert_eq!(
            ComponentInstancePayload::decode_for(NodeKind::Frame, &bytes).unwrap_err(),
            InstancePayloadError::KindMismatch {
                kind: NodeKind::Frame
            }
        );

        // slot content 중복.
        assert_eq!(
            SlotOverride::new(vec![stable(900), stable(900)]).unwrap_err(),
            InstancePayloadError::DuplicateSlotContent { node: stable(900) }
        );
        let mut dup = Vec::new();
        put_id(&mut dup, stable(1));
        put_count(&mut dup, 0);
        put_count(&mut dup, 1);
        put_id(&mut dup, stable(600));
        put_count(&mut dup, 2);
        put_id(&mut dup, stable(900));
        put_id(&mut dup, stable(900));
        assert!(matches!(
            ComponentInstancePayload::decode(&dup),
            Err(InstancePayloadError::DuplicateSlotContent { .. })
        ));

        // override key 내림차순(비canonical).
        let mut unordered = Vec::new();
        put_id(&mut unordered, stable(1));
        put_count(&mut unordered, 2);
        for p in [501, 500] {
            put_count(&mut unordered, 0);
            put_id(&mut unordered, stable(p));
            put_bytes(&mut unordered, &TypedValue::Boolean(true).to_bytes());
        }
        put_count(&mut unordered, 0);
        assert!(matches!(
            ComponentInstancePayload::decode(&unordered),
            Err(InstancePayloadError::Malformed { .. })
        ));

        // 잘못된 TypedValue bytes(알 수 없는 tag).
        let mut bad_value = Vec::new();
        put_id(&mut bad_value, stable(1));
        put_count(&mut bad_value, 1);
        put_count(&mut bad_value, 0);
        put_id(&mut bad_value, stable(500));
        put_bytes(&mut bad_value, &[0xFF]);
        put_count(&mut bad_value, 0);
        assert_eq!(
            ComponentInstancePayload::decode(&bad_value).unwrap_err(),
            InstancePayloadError::Value(ValueDecodeError::UnknownTag(0xFF))
        );
    }

    #[test]
    fn override_key_shape() {
        let direct = OverrideKey::direct(property(500));
        assert!(direct.is_direct());
        assert_eq!(direct.property(), property(500));
        let nested = OverrideKey::nested(vec![stable(300)], property(500));
        assert!(!nested.is_direct());
        assert_eq!(nested.path(), &[stable(300)]);
        assert_ne!(direct, nested);
    }

    #[test]
    fn instance_references_component_in_index() {
        let payload = sample();
        let mut index = ReferenceIndex::new();
        payload.sync_references(stable(42), &mut index);
        assert_eq!(index.inbound(stable(1)), vec![stable(42)]);
        // 다시 맞춰도 edge가 중복되지 않습니다.
        payload.sync_references(stable(42), &mut index);
        assert_eq!(index.edge_count(), 1);
    }

    #[test]
    fn property_keys_use_component_range() {
        // 0x0Axx 범위(다른 node 종류 key와 충돌 없음).
        for key in [
            keys::COMPONENT_REF,
            keys::PROPERTY_OVERRIDES,
            keys::SLOT_OVERRIDES,
        ] {
            assert_eq!(key.0 >> 8, 0x0A);
        }
    }
}
