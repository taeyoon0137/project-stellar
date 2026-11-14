//! modifier stack command (VII.2.6).
//!
//! node의 비파괴 modifier stack(VI.9)을 바꾸는 네 command입니다.
//!
//! - [`InsertModifier`]: 항목 하나를 넣습니다(caller가 id·OrderKey·payload를 정함).
//! - [`RemoveModifier`]: 항목을 뺍니다. 역은 원래 항목을 다시 넣는 InsertModifier입니다.
//! - [`MoveModifier`]: stack 안 순서 key를 바꿉니다.
//! - [`SetModifierProperty`]: 공통 `ENABLED` 또는 종류별 payload 속성 하나를 바꿉니다. 허용 key·
//!   값 타입·무효화 class는 modifier metadata registry(VI.9.7)를 따릅니다. 순서(`ORDER`)는
//!   MoveModifier 전용입니다.
//!
//! 공통 검증: 소유 node 존재, Fragment 소유 금지(VI.9.6), 종류·payload 일치와 값 범위, id 유일
//! (문서 영속 id 공간), stack 안 OrderKey 유일, 참조 대상이 있으면 node여야 함, 문서 전체 modifier
//! 참조 cycle 금지(VI.9.5). change set은 modifier id를 root로 한 속성 경로와 registry 무효화입니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{
    find_modifier_cycle, validate_modifier_owner, ModifierEntry, ModifierError, ModifierId,
    ModifierPayload, ModifierStack,
};
use stellar_document::modifier_metadata::{
    invalidation_of, keys as modifier_keys, kind_invalidation, modifier_property,
    ModifierPropertyType,
};
use stellar_document::records::NodePayload;

use crate::codec::{CommandCodecError, Reader, Writer};
use crate::command::CommandError;
use crate::edit::EditContext;
use crate::node_ops::{decode_order_key, existing_stacks};
use crate::property::{PropertyValue, PropertyValueType};

fn modifier_error(owner: StableId, error: ModifierError) -> CommandError {
    CommandError::Modifier { owner, error }
}

/// node의 현재 stack(없으면 빈 stack).
fn current_stack(model: &DocumentModel, node: StableId) -> Result<ModifierStack, CommandError> {
    if !model.contains(node) {
        return Err(CommandError::UnknownNode { id: node });
    }
    Ok(model
        .modifier_stack(node)
        .cloned()
        .unwrap_or_else(|| ModifierStack::new(node)))
}

fn find_entry(stack: &ModifierStack, id: ModifierId) -> Result<&ModifierEntry, CommandError> {
    stack
        .entries()
        .iter()
        .find(|entry| entry.id == id)
        .ok_or(CommandError::Modifier {
            owner: stack.node,
            error: ModifierError::NotFound { id },
        })
}

/// 바뀐 stack이 문서 규칙을 지키는지 검사합니다(소유 종류·참조 종류·cycle).
fn validate_stack(model: &DocumentModel, stack: &ModifierStack) -> Result<(), CommandError> {
    let owner = stack.node;
    let kind = model
        .payload(owner)
        .map(NodePayload::kind)
        .ok_or(CommandError::UnknownNode { id: owner })?;
    let mut has_references = false;
    for entry in stack.entries() {
        validate_modifier_owner(kind, entry.kind).map_err(|error| modifier_error(owner, error))?;
        entry
            .payload
            .validate()
            .map_err(|error| modifier_error(owner, error))?;
        for target in entry.payload.referenced_nodes() {
            has_references = true;
            if model.contains_persistent_id(target) && !model.contains(target) {
                return Err(CommandError::ReferenceTypeMismatch { owner, target });
            }
        }
    }
    // 소유 node의 outbound edge가 없으면 이 편집으로 새 순환을 만들 수 없습니다.
    if !has_references {
        return Ok(());
    }
    let mut stacks: Vec<ModifierStack> = existing_stacks(model)
        .into_iter()
        .filter(|existing| existing.node != owner)
        .collect();
    stacks.push(stack.clone());
    match find_modifier_cycle(&stacks) {
        Some(cycle) => Err(CommandError::ModifierCycle(cycle)),
        None => Ok(()),
    }
}

/// modifier 항목 하나를 단일 항목 stack bytes로 기록합니다(document canonical 형식 재사용).
fn encode_entry(out: &mut Writer, node: StableId, entry: &ModifierEntry) {
    let mut stack = ModifierStack::new(node);
    // 단일 항목 stack이라 id·OrderKey 충돌이 없고, entry는 검증된 stack에서 왔거나 decode가
    // 검증한 값입니다. 실패하면(불가능) 빈 stack을 기록해 디코딩이 거부하게 합니다.
    let _ = stack.insert(entry.clone());
    out.bytes(&stack.encode());
}

fn decode_entry(input: &mut Reader<'_>) -> Result<(StableId, ModifierEntry), CommandCodecError> {
    let stack =
        ModifierStack::decode(input.bytes()?).map_err(|_| CommandCodecError::Malformed {
            reason: "잘못된 modifier 항목",
        })?;
    match stack.entries() {
        [entry] => Ok((stack.node, entry.clone())),
        _ => Err(CommandCodecError::Malformed {
            reason: "modifier 항목은 정확히 하나여야 합니다",
        }),
    }
}

fn decode_modifier_id(input: &mut Reader<'_>) -> Result<ModifierId, CommandCodecError> {
    Ok(ModifierId::new(input.id()?))
}

/// modifier 삽입.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertModifier {
    /// 소유 node.
    pub node: StableId,
    /// 넣을 항목.
    pub entry: ModifierEntry,
}

impl InsertModifier {
    fn next_stack(&self, model: &DocumentModel) -> Result<ModifierStack, CommandError> {
        let mut stack = current_stack(model, self.node)?;
        let kind = model
            .payload(self.node)
            .map(NodePayload::kind)
            .ok_or(CommandError::UnknownNode { id: self.node })?;
        stack
            .insert_on(kind, self.entry.clone())
            .map_err(|error| modifier_error(self.node, error))?;
        Ok(stack)
    }

    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        let id = self.entry.id.stable_id();
        if id == StableId::NIL {
            return Err(CommandError::NilId);
        }
        if model.contains_persistent_id(id) {
            return Err(CommandError::DuplicateId { id });
        }
        let stack = self.next_stack(model)?;
        validate_stack(model, &stack)
    }

    pub(crate) fn invert(&self) -> RemoveModifier {
        RemoveModifier {
            node: self.node,
            modifier: self.entry.id,
        }
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        let stack = self.next_stack(ctx.model())?;
        ctx.set_modifier_stack(stack)?;
        ctx.changes().property(
            self.node,
            PropertyPath::new(self.entry.id.stable_id()),
            kind_invalidation(self.entry.kind),
        );
        Ok(())
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        encode_entry(out, self.node, &self.entry);
    }

    pub(crate) fn decode_from(input: &mut Reader<'_>) -> Result<InsertModifier, CommandCodecError> {
        let (node, entry) = decode_entry(input)?;
        Ok(InsertModifier { node, entry })
    }
}

/// modifier 제거.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoveModifier {
    /// 소유 node.
    pub node: StableId,
    /// 뺄 modifier.
    pub modifier: ModifierId,
}

impl RemoveModifier {
    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        let stack = current_stack(model, self.node)?;
        find_entry(&stack, self.modifier).map(|_| ())
    }

    pub(crate) fn invert(&self, model: &DocumentModel) -> Result<InsertModifier, CommandError> {
        let stack = current_stack(model, self.node)?;
        Ok(InsertModifier {
            node: self.node,
            entry: find_entry(&stack, self.modifier)?.clone(),
        })
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        let mut stack = current_stack(ctx.model(), self.node)?;
        let removed = stack
            .remove(self.modifier)
            .map_err(|error| modifier_error(self.node, error))?;
        ctx.set_modifier_stack(stack)?;
        ctx.changes().property(
            self.node,
            PropertyPath::new(self.modifier.stable_id()),
            kind_invalidation(removed.kind),
        );
        Ok(())
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.id(self.node);
        out.id(self.modifier.stable_id());
    }

    pub(crate) fn decode_from(input: &mut Reader<'_>) -> Result<RemoveModifier, CommandCodecError> {
        Ok(RemoveModifier {
            node: input.id()?,
            modifier: decode_modifier_id(input)?,
        })
    }
}

/// stack 안 순서 변경.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveModifier {
    /// 소유 node.
    pub node: StableId,
    /// 옮길 modifier.
    pub modifier: ModifierId,
    /// 새 순서 key.
    pub order: OrderKey,
}

impl MoveModifier {
    fn next_stack(&self, model: &DocumentModel) -> Result<ModifierStack, CommandError> {
        let mut stack = current_stack(model, self.node)?;
        let mut entry = stack
            .remove(self.modifier)
            .map_err(|error| modifier_error(self.node, error))?;
        entry.order = self.order.clone();
        stack
            .insert(entry)
            .map_err(|error| modifier_error(self.node, error))?;
        Ok(stack)
    }

    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        self.next_stack(model).map(|_| ())
    }

    pub(crate) fn invert(&self, model: &DocumentModel) -> Result<MoveModifier, CommandError> {
        let stack = current_stack(model, self.node)?;
        Ok(MoveModifier {
            node: self.node,
            modifier: self.modifier,
            order: find_entry(&stack, self.modifier)?.order.clone(),
        })
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        let stack = self.next_stack(ctx.model())?;
        let kind = find_entry(&stack, self.modifier)?.kind;
        ctx.set_modifier_stack(stack)?;
        ctx.changes().property(
            self.node,
            PropertyPath::with_keys(self.modifier.stable_id(), vec![modifier_keys::ORDER]),
            invalidation_of(kind, modifier_keys::ORDER).unwrap_or_else(|| kind_invalidation(kind)),
        );
        Ok(())
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.id(self.node);
        out.id(self.modifier.stable_id());
        out.bytes(self.order.as_bytes());
    }

    pub(crate) fn decode_from(input: &mut Reader<'_>) -> Result<MoveModifier, CommandCodecError> {
        Ok(MoveModifier {
            node: input.id()?,
            modifier: decode_modifier_id(input)?,
            order: decode_order_key(input)?,
        })
    }
}

/// registry 값 타입을 command 값 타입으로 옮깁니다. 순서 key는 SetModifierProperty 대상이
/// 아니라 `None`입니다.
fn modifier_value_type(value_type: ModifierPropertyType) -> Option<PropertyValueType> {
    Some(match value_type {
        ModifierPropertyType::Bool => PropertyValueType::Bool,
        ModifierPropertyType::OrderKey => return None,
        ModifierPropertyType::Transform => PropertyValueType::Transform,
        ModifierPropertyType::BooleanOp => PropertyValueType::BooleanOp,
        ModifierPropertyType::NodeRefList => PropertyValueType::NodeRefList,
        ModifierPropertyType::NodeRef => PropertyValueType::NodeRef,
        ModifierPropertyType::NonNegative => PropertyValueType::NonNegative,
        ModifierPropertyType::Color => PropertyValueType::Color,
        ModifierPropertyType::Offset => PropertyValueType::Offset,
        ModifierPropertyType::UnitInterval => PropertyValueType::UnitInterval,
        ModifierPropertyType::BlendMode => PropertyValueType::BlendMode,
    })
}

/// payload 속성 값을 읽습니다.
fn read_payload(payload: &ModifierPayload, key: PropertyKey) -> Option<PropertyValue> {
    Some(match (payload, key) {
        (ModifierPayload::Transform(m), modifier_keys::TRANSFORM_MATRIX) => {
            PropertyValue::Transform(*m)
        }
        (ModifierPayload::Boolean { op, .. }, modifier_keys::BOOLEAN_OP) => {
            PropertyValue::BooleanOp(*op)
        }
        (ModifierPayload::Boolean { operands, .. }, modifier_keys::BOOLEAN_OPERANDS) => {
            PropertyValue::NodeRefList(operands.clone())
        }
        (ModifierPayload::StrokeExpand { width }, modifier_keys::STROKE_EXPAND_WIDTH) => {
            PropertyValue::NonNegative(*width)
        }
        (ModifierPayload::Corner { radius }, modifier_keys::CORNER_RADIUS) => {
            PropertyValue::NonNegative(*radius)
        }
        (ModifierPayload::Mask { source }, modifier_keys::MASK_SOURCE)
        | (ModifierPayload::Clip { source }, modifier_keys::CLIP_SOURCE) => {
            PropertyValue::NodeRef(*source)
        }
        (ModifierPayload::Fill { color }, modifier_keys::FILL_COLOR)
        | (ModifierPayload::Stroke { color, .. }, modifier_keys::STROKE_COLOR)
        | (ModifierPayload::Shadow { color, .. }, modifier_keys::SHADOW_COLOR) => {
            PropertyValue::Color(*color)
        }
        (ModifierPayload::Stroke { width, .. }, modifier_keys::STROKE_WIDTH) => {
            PropertyValue::NonNegative(*width)
        }
        (ModifierPayload::Shadow { offset, .. }, modifier_keys::SHADOW_OFFSET) => {
            PropertyValue::Offset(*offset)
        }
        (ModifierPayload::Shadow { blur, .. }, modifier_keys::SHADOW_BLUR) => {
            PropertyValue::NonNegative(*blur)
        }
        (ModifierPayload::Blur { radius }, modifier_keys::BLUR_RADIUS) => {
            PropertyValue::NonNegative(*radius)
        }
        (ModifierPayload::Blend { mode }, modifier_keys::BLEND_MODE) => {
            PropertyValue::BlendMode(*mode)
        }
        (ModifierPayload::Opacity { value }, modifier_keys::OPACITY_VALUE) => {
            PropertyValue::UnitInterval(*value)
        }
        _ => return None,
    })
}

/// payload 속성 값을 씁니다. key·값 조합이 맞지 않으면 `false`.
fn write_payload(payload: &mut ModifierPayload, key: PropertyKey, value: &PropertyValue) -> bool {
    match (payload, key, value) {
        (
            ModifierPayload::Transform(m),
            modifier_keys::TRANSFORM_MATRIX,
            PropertyValue::Transform(v),
        ) => *m = *v,
        (
            ModifierPayload::Boolean { op, .. },
            modifier_keys::BOOLEAN_OP,
            PropertyValue::BooleanOp(v),
        ) => *op = *v,
        (
            ModifierPayload::Boolean { operands, .. },
            modifier_keys::BOOLEAN_OPERANDS,
            PropertyValue::NodeRefList(v),
        ) => *operands = v.clone(),
        (
            ModifierPayload::StrokeExpand { width },
            modifier_keys::STROKE_EXPAND_WIDTH,
            PropertyValue::NonNegative(v),
        ) => *width = *v,
        (
            ModifierPayload::Corner { radius },
            modifier_keys::CORNER_RADIUS,
            PropertyValue::NonNegative(v),
        ) => *radius = *v,
        (
            ModifierPayload::Mask { source },
            modifier_keys::MASK_SOURCE,
            PropertyValue::NodeRef(v),
        )
        | (
            ModifierPayload::Clip { source },
            modifier_keys::CLIP_SOURCE,
            PropertyValue::NodeRef(v),
        ) => *source = *v,
        (ModifierPayload::Fill { color }, modifier_keys::FILL_COLOR, PropertyValue::Color(v))
        | (
            ModifierPayload::Stroke { color, .. },
            modifier_keys::STROKE_COLOR,
            PropertyValue::Color(v),
        )
        | (
            ModifierPayload::Shadow { color, .. },
            modifier_keys::SHADOW_COLOR,
            PropertyValue::Color(v),
        ) => *color = *v,
        (
            ModifierPayload::Stroke { width, .. },
            modifier_keys::STROKE_WIDTH,
            PropertyValue::NonNegative(v),
        ) => *width = *v,
        (
            ModifierPayload::Shadow { offset, .. },
            modifier_keys::SHADOW_OFFSET,
            PropertyValue::Offset(v),
        ) => *offset = *v,
        (
            ModifierPayload::Shadow { blur, .. },
            modifier_keys::SHADOW_BLUR,
            PropertyValue::NonNegative(v),
        ) => *blur = *v,
        (
            ModifierPayload::Blur { radius },
            modifier_keys::BLUR_RADIUS,
            PropertyValue::NonNegative(v),
        ) => *radius = *v,
        (
            ModifierPayload::Blend { mode },
            modifier_keys::BLEND_MODE,
            PropertyValue::BlendMode(v),
        ) => *mode = *v,
        (
            ModifierPayload::Opacity { value: slot },
            modifier_keys::OPACITY_VALUE,
            PropertyValue::UnitInterval(v),
        ) => *slot = *v,
        _ => return false,
    }
    true
}

/// modifier 속성 하나 변경.
#[derive(Debug, Clone, PartialEq)]
pub struct SetModifierProperty {
    /// 소유 node.
    pub node: StableId,
    /// 대상 modifier.
    pub modifier: ModifierId,
    /// 속성 key(VI.9.7 registry).
    pub key: PropertyKey,
    /// 새 값.
    pub value: PropertyValue,
}

impl SetModifierProperty {
    /// registry 검사 후 바뀐 stack을 만듭니다.
    fn next_stack(&self, model: &DocumentModel) -> Result<ModifierStack, CommandError> {
        let mut stack = current_stack(model, self.node)?;
        let entry = find_entry(&stack, self.modifier)?.clone();
        let unknown = || CommandError::UnknownProperty {
            node: self.modifier.stable_id(),
            key: self.key,
        };
        let metadata = modifier_property(self.key).ok_or_else(unknown)?;
        invalidation_of(entry.kind, self.key).ok_or_else(unknown)?;
        let expected = modifier_value_type(metadata.value_type)
            .ok_or(CommandError::PropertyNotSettable { key: self.key })?;
        let found = self.value.value_type();
        if expected != found {
            return Err(CommandError::PropertyTypeMismatch {
                key: self.key,
                expected,
                found,
            });
        }
        if self.key == modifier_keys::ENABLED {
            let PropertyValue::Bool(enabled) = self.value else {
                return Err(CommandError::PropertyTypeMismatch {
                    key: self.key,
                    expected,
                    found,
                });
            };
            stack
                .set_enabled(self.modifier, enabled)
                .map_err(|error| modifier_error(self.node, error))?;
        } else {
            let mut payload = entry.payload.clone();
            if !write_payload(&mut payload, self.key, &self.value) {
                return Err(unknown());
            }
            stack
                .set_payload(self.modifier, payload)
                .map_err(|error| modifier_error(self.node, error))?;
        }
        Ok(stack)
    }

    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        let stack = self.next_stack(model)?;
        validate_stack(model, &stack)
    }

    pub(crate) fn invert(
        &self,
        model: &DocumentModel,
    ) -> Result<SetModifierProperty, CommandError> {
        let stack = current_stack(model, self.node)?;
        let entry = find_entry(&stack, self.modifier)?;
        let value = if self.key == modifier_keys::ENABLED {
            PropertyValue::Bool(entry.enabled)
        } else {
            read_payload(&entry.payload, self.key).ok_or(CommandError::UnknownProperty {
                node: self.modifier.stable_id(),
                key: self.key,
            })?
        };
        Ok(SetModifierProperty {
            node: self.node,
            modifier: self.modifier,
            key: self.key,
            value,
        })
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        let stack = self.next_stack(ctx.model())?;
        let kind = find_entry(&stack, self.modifier)?.kind;
        ctx.set_modifier_stack(stack)?;
        let invalidation =
            invalidation_of(kind, self.key).unwrap_or_else(|| kind_invalidation(kind));
        ctx.changes().property(
            self.node,
            PropertyPath::with_keys(self.modifier.stable_id(), vec![self.key]),
            invalidation,
        );
        Ok(())
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.id(self.node);
        out.id(self.modifier.stable_id());
        out.u32(self.key.0);
        self.value.encode_into(out);
    }

    pub(crate) fn decode_from(
        input: &mut Reader<'_>,
    ) -> Result<SetModifierProperty, CommandCodecError> {
        Ok(SetModifierProperty {
            node: input.id()?,
            modifier: decode_modifier_id(input)?,
            key: PropertyKey(input.u32()?),
            value: PropertyValue::decode_from(input)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{decode_command, encode_command};
    use crate::command::test_fixtures::{frame, sid, small_model};
    use crate::command::Command;
    use crate::node_ops::InsertNode;
    use crate::request::CommandRequest;
    use crate::state::DocumentState;
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::reference::NodeRef;
    use stellar_core_types::value::{FiniteF32, Transform2D, Vec2};
    use stellar_document::fragment::FragmentPayload;
    use stellar_document::modifier::{BlendMode, BooleanOp, ModifierKind};
    use stellar_document::modifier_metadata::{InvalidationClass, InvalidationSet};
    use stellar_document::test_support::semantic_snapshot::model_snapshot_json;

    fn snapshot(state: &DocumentState) -> String {
        model_snapshot_json(state.model()).expect("snapshot")
    }

    fn run(state: &mut DocumentState, command: Command) -> Result<Command, CommandError> {
        let request = CommandRequest::at(state, command);
        state.execute(&request).map(|outcome| outcome.inverse)
    }

    fn entry(id: u64, key: u8, kind: ModifierKind) -> ModifierEntry {
        ModifierEntry {
            id: ModifierId::new(sid(id)),
            order: OrderKey::from_bytes(vec![key]).unwrap(),
            kind,
            enabled: true,
            payload: ModifierPayload::default_for(kind),
        }
    }

    fn insert(node: u64, entry: ModifierEntry) -> Command {
        Command::InsertModifier(InsertModifier {
            node: sid(node),
            entry,
        })
    }

    fn set(node: u64, modifier: u64, key: PropertyKey, value: PropertyValue) -> Command {
        Command::SetModifierProperty(SetModifierProperty {
            node: sid(node),
            modifier: ModifierId::new(sid(modifier)),
            key,
            value,
        })
    }

    /// 11에 Opacity(60, 0x10)·Shadow(61, 0x20)·Mask(62, 0x30, 대상 12) stack.
    fn state_with_stack() -> DocumentState {
        let mut state = DocumentState::new(small_model());
        run(
            &mut state,
            insert(11, entry(60, 0x10, ModifierKind::Opacity)),
        )
        .unwrap();
        run(
            &mut state,
            insert(11, entry(61, 0x20, ModifierKind::Shadow)),
        )
        .unwrap();
        let mut mask = entry(62, 0x30, ModifierKind::Mask);
        mask.payload = ModifierPayload::Mask {
            source: NodeRef::new(sid(12)),
        };
        run(&mut state, insert(11, mask)).unwrap();
        state
    }

    fn order_of(state: &DocumentState, node: u64) -> Vec<StableId> {
        state
            .model()
            .modifier_stack(sid(node))
            .map(|s| {
                s.ids_in_order()
                    .into_iter()
                    .map(|id| id.stable_id())
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn insert_and_remove_modifier_round_trip() {
        // 검증 기준(VII.2.6): 각 apply/inverse fixture 통과.
        let mut state = DocumentState::new(small_model());
        let before = snapshot(&state);
        let request = CommandRequest::at(&state, insert(11, entry(60, 0x10, ModifierKind::Blur)));
        let outcome = state.execute(&request).unwrap();
        assert_eq!(order_of(&state, 11), vec![sid(60)]);
        assert_eq!(
            outcome.changes.changed_properties(),
            vec![PropertyPath::new(sid(60))]
        );
        assert_eq!(
            outcome.changes.invalidation(),
            kind_invalidation(ModifierKind::Blur)
        );
        assert_eq!(
            outcome.inverse,
            Command::RemoveModifier(RemoveModifier {
                node: sid(11),
                modifier: ModifierId::new(sid(60))
            })
        );
        let reinsert = run(&mut state, outcome.inverse).unwrap();
        assert_eq!(snapshot(&state), before, "빈 stack은 제거");
        assert_eq!(reinsert, insert(11, entry(60, 0x10, ModifierKind::Blur)));

        // 여러 항목 중 가운데 제거 → 역으로 같은 위치에 복원.
        let mut state = state_with_stack();
        let before = snapshot(&state);
        let inverse = run(
            &mut state,
            Command::RemoveModifier(RemoveModifier {
                node: sid(11),
                modifier: ModifierId::new(sid(61)),
            }),
        )
        .unwrap();
        assert_eq!(order_of(&state, 11), vec![sid(60), sid(62)]);
        run(&mut state, inverse).unwrap();
        assert_eq!(snapshot(&state), before);
    }

    #[test]
    fn move_modifier_round_trip() {
        let mut state = state_with_stack();
        let before = snapshot(&state);
        let request = CommandRequest::at(
            &state,
            Command::MoveModifier(MoveModifier {
                node: sid(11),
                modifier: ModifierId::new(sid(62)),
                order: OrderKey::from_bytes(vec![0x05]).unwrap(),
            }),
        );
        let outcome = state.execute(&request).unwrap();
        assert_eq!(order_of(&state, 11), vec![sid(62), sid(60), sid(61)]);
        assert_eq!(
            outcome.changes.changed_properties(),
            vec![PropertyPath::with_keys(sid(62), vec![modifier_keys::ORDER])]
        );
        run(&mut state, outcome.inverse).unwrap();
        assert_eq!(snapshot(&state), before);
        // 다른 항목의 key와 충돌.
        assert!(matches!(
            run(
                &mut state,
                Command::MoveModifier(MoveModifier {
                    node: sid(11),
                    modifier: ModifierId::new(sid(62)),
                    order: OrderKey::from_bytes(vec![0x10]).unwrap(),
                })
            ),
            Err(CommandError::Modifier {
                error: ModifierError::DuplicateOrderKey,
                ..
            })
        ));
        assert_eq!(snapshot(&state), before);
    }

    #[test]
    fn set_modifier_property_round_trip_for_every_field() {
        let red = Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 0.5).unwrap();
        let cases: Vec<(ModifierKind, PropertyKey, PropertyValue)> = vec![
            (
                ModifierKind::Transform,
                modifier_keys::TRANSFORM_MATRIX,
                PropertyValue::Transform(Transform2D::new(1.0, 0.0, 0.0, 1.0, 4.0, 2.0).unwrap()),
            ),
            (
                ModifierKind::Boolean,
                modifier_keys::BOOLEAN_OP,
                PropertyValue::BooleanOp(BooleanOp::Exclude),
            ),
            (
                ModifierKind::Boolean,
                modifier_keys::BOOLEAN_OPERANDS,
                PropertyValue::NodeRefList(vec![NodeRef::new(sid(12)), NodeRef::new(sid(13))]),
            ),
            (
                ModifierKind::StrokeExpand,
                modifier_keys::STROKE_EXPAND_WIDTH,
                PropertyValue::NonNegative(FiniteF32::new(3.0).unwrap()),
            ),
            (
                ModifierKind::Corner,
                modifier_keys::CORNER_RADIUS,
                PropertyValue::NonNegative(FiniteF32::new(6.0).unwrap()),
            ),
            (
                ModifierKind::Mask,
                modifier_keys::MASK_SOURCE,
                PropertyValue::NodeRef(NodeRef::new(sid(12))),
            ),
            (
                ModifierKind::Clip,
                modifier_keys::CLIP_SOURCE,
                PropertyValue::NodeRef(NodeRef::new(sid(13))),
            ),
            (
                ModifierKind::Fill,
                modifier_keys::FILL_COLOR,
                PropertyValue::Color(red),
            ),
            (
                ModifierKind::Stroke,
                modifier_keys::STROKE_COLOR,
                PropertyValue::Color(red),
            ),
            (
                ModifierKind::Stroke,
                modifier_keys::STROKE_WIDTH,
                PropertyValue::NonNegative(FiniteF32::new(2.5).unwrap()),
            ),
            (
                ModifierKind::Shadow,
                modifier_keys::SHADOW_COLOR,
                PropertyValue::Color(red),
            ),
            (
                ModifierKind::Shadow,
                modifier_keys::SHADOW_OFFSET,
                PropertyValue::Offset(Vec2::new(2.0, -1.0).unwrap()),
            ),
            (
                ModifierKind::Shadow,
                modifier_keys::SHADOW_BLUR,
                PropertyValue::NonNegative(FiniteF32::new(8.0).unwrap()),
            ),
            (
                ModifierKind::Blur,
                modifier_keys::BLUR_RADIUS,
                PropertyValue::NonNegative(FiniteF32::new(4.0).unwrap()),
            ),
            (
                ModifierKind::Blend,
                modifier_keys::BLEND_MODE,
                PropertyValue::BlendMode(BlendMode::Multiply),
            ),
            (
                ModifierKind::Opacity,
                modifier_keys::OPACITY_VALUE,
                PropertyValue::UnitInterval(FiniteF32::new(0.25).unwrap()),
            ),
            (
                ModifierKind::Opacity,
                modifier_keys::ENABLED,
                PropertyValue::Bool(false),
            ),
        ];
        for (kind, key, value) in cases {
            let mut state = DocumentState::new(small_model());
            run(&mut state, insert(11, entry(60, 0x10, kind))).unwrap();
            let before = snapshot(&state);
            let request = CommandRequest::at(&state, set(11, 60, key, value.clone()));
            let outcome = state.execute(&request).unwrap();
            let stack = state.model().modifier_stack(sid(11)).unwrap();
            let current = &stack.entries()[0];
            if key == modifier_keys::ENABLED {
                assert!(!current.enabled);
            } else {
                assert_eq!(read_payload(&current.payload, key), Some(value.clone()));
            }
            assert_eq!(
                outcome.changes.changed_properties(),
                vec![PropertyPath::with_keys(sid(60), vec![key])]
            );
            assert_eq!(
                Some(outcome.changes.invalidation()),
                invalidation_of(kind, key),
                "{} {key:?}",
                kind.name()
            );
            run(&mut state, outcome.inverse).unwrap();
            assert_eq!(snapshot(&state), before, "{} {key:?}", kind.name());
            // codec round-trip.
            let command = set(11, 60, key, value);
            assert_eq!(decode_command(&encode_command(&command)).unwrap(), command);
        }
        // paint 전용 속성은 geometry를 무효화하지 않습니다.
        let paint = invalidation_of(ModifierKind::Fill, modifier_keys::FILL_COLOR).unwrap();
        assert_eq!(paint, InvalidationSet::PAINT);
        assert!(!paint.contains(InvalidationClass::Layout));
    }

    #[test]
    fn modifier_commands_reject_invalid_edits() {
        let mut state = state_with_stack();
        let fragment = {
            let mut record = frame(20, Some(10), 0x90);
            record.payload = NodePayload::Fragment(FragmentPayload);
            record
        };
        run(
            &mut state,
            Command::InsertNode(InsertNode::single(fragment)),
        )
        .unwrap();
        let before = snapshot(&state);
        // Fragment 소유 금지(VI.9.6).
        assert_eq!(
            run(&mut state, insert(20, entry(70, 0x10, ModifierKind::Blur))),
            Err(CommandError::Modifier {
                owner: sid(20),
                error: ModifierError::FragmentOwner {
                    kind: ModifierKind::Blur
                }
            })
        );
        // id 중복(node id와도).
        assert_eq!(
            run(&mut state, insert(12, entry(60, 0x10, ModifierKind::Blur))),
            Err(CommandError::DuplicateId { id: sid(60) })
        );
        assert_eq!(
            run(&mut state, insert(12, entry(13, 0x10, ModifierKind::Blur))),
            Err(CommandError::DuplicateId { id: sid(13) })
        );
        // stack 안 OrderKey 충돌.
        assert!(matches!(
            run(&mut state, insert(11, entry(70, 0x10, ModifierKind::Blur))),
            Err(CommandError::Modifier {
                error: ModifierError::DuplicateOrderKey,
                ..
            })
        ));
        // 값 범위(불투명도 1.5)·타입 불일치·순서 key·종류에 없는 key.
        assert!(matches!(
            run(
                &mut state,
                set(
                    11,
                    60,
                    modifier_keys::OPACITY_VALUE,
                    PropertyValue::UnitInterval(FiniteF32::new(1.5).unwrap())
                )
            ),
            Err(CommandError::Modifier {
                error: ModifierError::InvalidPayload { .. },
                ..
            })
        ));
        assert!(matches!(
            run(
                &mut state,
                set(
                    11,
                    60,
                    modifier_keys::OPACITY_VALUE,
                    PropertyValue::Bool(true)
                )
            ),
            Err(CommandError::PropertyTypeMismatch { .. })
        ));
        assert_eq!(
            run(
                &mut state,
                set(11, 60, modifier_keys::ORDER, PropertyValue::Bool(true))
            ),
            Err(CommandError::PropertyNotSettable {
                key: modifier_keys::ORDER
            })
        );
        assert_eq!(
            run(
                &mut state,
                set(
                    11,
                    60,
                    modifier_keys::BLUR_RADIUS,
                    PropertyValue::Bool(true)
                )
            ),
            Err(CommandError::UnknownProperty {
                node: sid(60),
                key: modifier_keys::BLUR_RADIUS
            })
        );
        // mask 대상이 자기 자신(self cycle) 또는 간접 cycle.
        assert!(matches!(
            run(
                &mut state,
                set(
                    11,
                    62,
                    modifier_keys::MASK_SOURCE,
                    PropertyValue::NodeRef(NodeRef::new(sid(11)))
                )
            ),
            Err(CommandError::ModifierCycle(_))
        ));
        let mut back = entry(71, 0x10, ModifierKind::Clip);
        back.payload = ModifierPayload::Clip {
            source: NodeRef::new(sid(11)),
        };
        assert!(matches!(
            run(&mut state, insert(12, back)),
            Err(CommandError::ModifierCycle(_))
        ));
        // 참조 대상이 node가 아닌 영속 id(View).
        assert_eq!(
            run(
                &mut state,
                set(
                    11,
                    62,
                    modifier_keys::MASK_SOURCE,
                    PropertyValue::NodeRef(NodeRef::new(sid(3)))
                )
            ),
            Err(CommandError::ReferenceTypeMismatch {
                owner: sid(11),
                target: sid(3)
            })
        );
        // 없는 modifier·node.
        assert!(matches!(
            run(
                &mut state,
                Command::RemoveModifier(RemoveModifier {
                    node: sid(11),
                    modifier: ModifierId::new(sid(99)),
                })
            ),
            Err(CommandError::Modifier {
                error: ModifierError::NotFound { .. },
                ..
            })
        ));
        assert_eq!(
            run(&mut state, insert(99, entry(70, 0x10, ModifierKind::Blur))),
            Err(CommandError::UnknownNode { id: sid(99) })
        );
        assert_eq!(snapshot(&state), before);
    }

    #[test]
    fn modifier_command_codec_round_trips() {
        let mut shadow = entry(61, 0x20, ModifierKind::Shadow);
        shadow.enabled = false;
        let commands = [
            insert(11, shadow),
            Command::RemoveModifier(RemoveModifier {
                node: sid(11),
                modifier: ModifierId::new(sid(61)),
            }),
            Command::MoveModifier(MoveModifier {
                node: sid(11),
                modifier: ModifierId::new(sid(61)),
                order: OrderKey::from_bytes(vec![0x01, 0x80]).unwrap(),
            }),
        ];
        for (command, kind) in commands.iter().zip([0x0020u16, 0x0021, 0x0022]) {
            let bytes = encode_command(command);
            assert_eq!(&bytes[..2], &kind.to_le_bytes());
            assert_eq!(&decode_command(&bytes).unwrap(), command);
        }
        let bytes = encode_command(&set(
            11,
            60,
            modifier_keys::ENABLED,
            PropertyValue::Bool(true),
        ));
        assert_eq!(&bytes[..2], &0x0023u16.to_le_bytes());
    }
}
