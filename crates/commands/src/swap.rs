//! 인스턴스 교체(swap)와 인스턴스 payload 교체 command (XII.10.35, ADR-0079 결정 1·6, ADR-0118 결정 4·5).
//!
//! - [`SwapInstance`]는 인스턴스 payload(참조 `ComponentRef`, Property override, Slot override)를 통째로 바꾸는
//!   command입니다. 교체(swap)뿐 아니라 캔버스의 override Set·Reset·전체 Reset과 Slot 투영 갱신(XII.10.34)도 이
//!   command 하나로 씁니다. 역은 원래 payload를 담은 같은 command입니다.
//! - [`plan_instance_swap`]은 적용 전 preview입니다. 이름과 선언 타입이 같은 공개 Property의 override만 옮기고,
//!   옮기지 못한 override 목록을 이유와 함께 돌려줍니다.
//! - 크기([`SwapSizing`]): 인스턴스 크기는 인스턴스 layout record의 `width`·`height`(ADR-0086 결정 2)입니다.
//!   기본(새 정의 기본 크기)은 명시 `width`·`height`를 지워 새 정의 root 크기를 따르게 하고, 현재 크기 유지는
//!   명시 값이 없는 축에 지금 보이는 크기(현재 정의 root 크기)를 `<length>`로 둡니다. 크기 command는 계획의
//!   [`InstanceSwapPlan::commands`]에서 교체 command 뒤에 오며 한 CommandTransaction으로 씁니다.

use std::collections::BTreeSet;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::ComponentRef;
use stellar_core_types::value::Size2;
use stellar_document::component::definition::{
    ComponentEntry, ComponentPropertyId, ComponentSlotId,
};
use stellar_document::component::instance::{ComponentInstancePayload, OverrideKey};
use stellar_document::component::slot::ComponentSlot;
use stellar_document::layout_record::{payload_size, LayoutLonghand, LayoutValue};
use stellar_document::model::DocumentModel;
use stellar_document::node_kind::NodeKind;
use stellar_document::record_codec::decode_payload;
use stellar_document::records::NodePayload;

use crate::codec::{CommandCodecError, Reader, Writer};
use crate::command::{Command, CommandError};
use crate::edit::EditContext;
use crate::style_defaults::{SetLayoutProperty, UnsetProperty};
use crate::tree::{kind_of, parent_of};

/// 인스턴스 payload를 바꿉니다: 참조 컴포넌트 교체(swap), override Set·Reset, Slot 투영 갱신. 교체 payload는
/// [`plan_instance_swap`]이, override 편집 payload는 runtime 캔버스 인스턴스 편집(XII.10.34)이 만듭니다.
///
/// command는 결과 payload를 그대로 담아 컴포넌트 library 없이 재실행할 수 있습니다. 역은 원래 payload로 되돌리는
/// 같은 command입니다(원래 `ComponentRef`·override 복원).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwapInstance {
    /// 대상 ComponentInstance node.
    pub node: StableId,
    /// 교체 뒤 payload.
    pub payload: ComponentInstancePayload,
}

impl SwapInstance {
    pub(crate) fn current<'a>(
        &self,
        model: &'a DocumentModel,
    ) -> Result<&'a ComponentInstancePayload, CommandError> {
        match model.payload(self.node) {
            Some(NodePayload::ComponentInstance(payload)) => Ok(payload),
            None => Err(CommandError::UnknownNode { id: self.node }),
            Some(other) => Err(CommandError::ConversionKind {
                node: self.node,
                expected: NodeKind::ComponentInstance,
                found: other.kind(),
            }),
        }
    }

    /// 새 payload의 slot content가 이 인스턴스 소유 node(부모 = 인스턴스)이고 한 번씩만 투영되는지, 지금 투영된
    /// content가 교체 뒤에도 모두 투영되는지 확인합니다(IX.3.2: 인스턴스 자식은 모두 slot에 투영돼야 함). 교체가
    /// 소유 node를 투영할 곳 없이 남기지 않으므로 역(원래 payload)도 항상 적용됩니다. slot 종류·개수 규칙은 컴포넌트
    /// library가 있어야 알 수 있어 [`plan_instance_swap`]이 봅니다.
    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        let current = self.current(model)?;
        if self.payload.component().id() == StableId::NIL {
            return Err(CommandError::NilId);
        }
        let mut projected = BTreeSet::new();
        for (_, content) in self.payload.slot_overrides() {
            for node in content.content() {
                let parent =
                    parent_of(model, *node).ok_or(CommandError::UnknownNode { id: *node })?;
                if parent != Some(self.node) {
                    return Err(CommandError::InvalidParent {
                        node: *node,
                        parent,
                    });
                }
                if !projected.insert(*node) {
                    return Err(CommandError::DuplicateId { id: *node });
                }
            }
        }
        if current
            .slot_overrides()
            .flat_map(|(_, content)| content.content().iter())
            .any(|node| !projected.contains(node))
        {
            return Err(CommandError::InvalidRequest {
                reason: "교체 뒤 투영할 slot이 없는 인스턴스 소유 node가 남습니다",
            });
        }
        Ok(())
    }

    pub(crate) fn invert(&self, model: &DocumentModel) -> Result<SwapInstance, CommandError> {
        Ok(SwapInstance {
            node: self.node,
            payload: self.current(model)?.clone(),
        })
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        if self.current(ctx.model())? == &self.payload {
            return Ok(());
        }
        ctx.set_payload(
            self.node,
            NodePayload::ComponentInstance(self.payload.clone()),
        )?;
        ctx.changes().structure(self.node);
        Ok(())
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.id(self.node);
        out.bytes(&self.payload.encode());
    }

    pub(crate) fn decode_from(input: &mut Reader<'_>) -> Result<SwapInstance, CommandCodecError> {
        let node = input.id()?;
        let payload = ComponentInstancePayload::decode(input.bytes()?).map_err(|_| {
            CommandCodecError::Malformed {
                reason: "잘못된 인스턴스 payload",
            }
        })?;
        Ok(SwapInstance { node, payload })
    }
}

/// 교체 뒤 인스턴스 크기.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SwapSizing {
    /// 새 정의 root의 기본 크기(기본). 인스턴스의 명시 `width`·`height`를 지웁니다.
    #[default]
    DefinitionDefault,
    /// 지금 보이는 크기 유지. 명시 값이 없는 축에 현재 정의 root 크기를 `<length>`로 둡니다(명시 값은 그대로).
    KeepCurrent,
}

/// 교체 계획에 쓰는 정의 Property 요약. `T`는 선언 타입(`TypeSpec`)이며 같음만 비교합니다.
#[derive(Debug, Clone, PartialEq)]
pub struct SwapProperty<T> {
    /// Property id.
    pub id: ComponentPropertyId,
    /// 이름.
    pub name: String,
    /// 선언 타입.
    pub value_type: T,
    /// 공개 Property인지.
    pub public: bool,
}

/// 보존하지 못한 override.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SwapOverride {
    /// Property override.
    Property(OverrideKey),
    /// Slot override.
    Slot(ComponentSlotId),
}

/// override를 보존하지 못한 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SwapDropReason {
    /// 새 정의에 같은 이름의 Property가 없습니다.
    NoMatchingProperty,
    /// 같은 이름이지만 선언 타입이 다릅니다.
    TypeMismatch,
    /// 같은 이름·타입이지만 새 정의에서 공개가 아닙니다.
    NotPublic,
    /// nested instance 경로 override(옛 정의 tree를 가리킴)입니다.
    NestedPath,
    /// 현재 정의에 없는 Property를 가리킵니다.
    UnknownProperty,
    /// 새 정의에 같은 Slot이 없습니다.
    NoMatchingSlot,
    /// 같은 Slot이 있지만 content가 새 Slot의 종류·개수 규칙을 어깁니다.
    SlotRuleMismatch,
}

/// 보존한 Property override(옛 Property → 새 Property).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreservedOverride {
    /// 옛 정의 Property.
    pub from: ComponentPropertyId,
    /// 새 정의 Property.
    pub to: ComponentPropertyId,
    /// 공통 이름.
    pub name: String,
}

/// 인스턴스 교체 계획: 적용할 command와 적용 전 preview.
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceSwapPlan {
    /// 적용할 교체 command.
    pub command: SwapInstance,
    /// 크기 command(`width`·`height` 순, [`SwapSizing`]). 교체 command 뒤에 같은 transaction으로 씁니다.
    pub sizing: Vec<Command>,
    /// 보존한 Property override(옛 Property id 순).
    pub preserved: Vec<PreservedOverride>,
    /// 보존하지 못한 override(key 순).
    pub dropped: Vec<(SwapOverride, SwapDropReason)>,
}

impl InstanceSwapPlan {
    /// 한 CommandTransaction에 넣을 command 순서: 교체, 그다음 크기.
    pub fn commands(&self) -> Vec<Command> {
        let mut commands = vec![Command::SwapInstance(self.command.clone())];
        commands.extend(self.sizing.iter().cloned());
        commands
    }
}

/// 크기 command. `current_size`는 명시 `width`·`height`가 없을 때 인스턴스가 보이는 크기(현재 정의 root 크기)입니다.
fn sizing_commands(
    model: &DocumentModel,
    node: StableId,
    sizing: SwapSizing,
    current_size: Option<Size2>,
) -> Result<Vec<Command>, CommandError> {
    let record = model.layout_record(node);
    let mut commands = Vec::new();
    for (longhand, axis) in [(LayoutLonghand::Width, 0), (LayoutLonghand::Height, 1)] {
        let explicit = record.and_then(|r| r.get(longhand)).is_some();
        match sizing {
            SwapSizing::DefinitionDefault if explicit => {
                commands.push(Command::UnsetProperty(UnsetProperty { node, longhand }));
            }
            SwapSizing::KeepCurrent if !explicit => {
                let size = current_size.ok_or(CommandError::InvalidRequest {
                    reason: "현재 정의 root 크기를 알 수 없어 현재 크기를 유지할 수 없습니다",
                })?;
                let px = if axis == 0 {
                    size.width()
                } else {
                    size.height()
                };
                commands.push(Command::SetLayoutProperty(SetLayoutProperty {
                    node,
                    longhand,
                    value: LayoutValue::Length(px),
                }));
            }
            _ => {}
        }
    }
    Ok(commands)
}

/// 정의 root의 payload 크기. Frame 등 payload 크기가 있는 root만 있습니다.
pub fn definition_root_size(entry: &ComponentEntry) -> Option<Size2> {
    let tree = entry.tree();
    let root = tree.node(tree.tree_root()?)?;
    let payload = decode_payload(root.kind, &root.payload).ok()?;
    let (width, height) = payload_size(&payload)?;
    Size2::new(width, height).ok()
}

/// 인스턴스 `node`를 `target` 컴포넌트로 바꾸는 계획을 만듭니다(문서는 바꾸지 않음).
///
/// 직접 Property override는 옛 Property와 **이름과 선언 타입이 같은 새 정의의 공개 Property**로 옮깁니다.
/// nested 경로 override, 맞는 Property가 없는 override, 새 정의에 같은 id의 Slot이 없거나 content가 새
/// Slot의 종류·개수 규칙을 어기는 Slot override는 버리고 `dropped`에 이유와 함께 보고합니다. 버릴 Slot에
/// content(인스턴스 소유 node)가 있으면 투영할 곳 없이 남게 되므로 계획을 거부합니다(IX.3.2). 같은 입력은 같은
/// 계획입니다(결과는 key 순). `current_size`는 [`SwapSizing::KeepCurrent`]에서 명시 크기가 없는 축에 쓰는 지금
/// 정의 root 크기입니다.
#[allow(clippy::too_many_arguments)]
pub fn plan_instance_swap<T: PartialEq>(
    model: &DocumentModel,
    node: StableId,
    target: ComponentRef,
    current_properties: &[SwapProperty<T>],
    target_properties: &[SwapProperty<T>],
    target_slots: &[ComponentSlot],
    sizing: SwapSizing,
    current_size: Option<Size2>,
) -> Result<InstanceSwapPlan, CommandError> {
    if target.id() == StableId::NIL {
        return Err(CommandError::NilId);
    }
    let probe = SwapInstance {
        node,
        payload: ComponentInstancePayload::new(target),
    };
    let current = probe.current(model)?;
    let mut payload = ComponentInstancePayload::new(target);
    let mut preserved = Vec::new();
    let mut dropped = Vec::new();
    for (key, value) in current.property_overrides() {
        if !key.is_direct() {
            dropped.push((
                SwapOverride::Property(key.clone()),
                SwapDropReason::NestedPath,
            ));
            continue;
        }
        let Some(old) = current_properties.iter().find(|p| p.id == key.property()) else {
            dropped.push((
                SwapOverride::Property(key.clone()),
                SwapDropReason::UnknownProperty,
            ));
            continue;
        };
        let reason = match target_properties.iter().find(|p| p.name == old.name) {
            None => Some(SwapDropReason::NoMatchingProperty),
            Some(new) if new.value_type != old.value_type => Some(SwapDropReason::TypeMismatch),
            Some(new) if !new.public => Some(SwapDropReason::NotPublic),
            Some(new) => {
                payload.set_property_override(OverrideKey::direct(new.id), value.clone());
                preserved.push(PreservedOverride {
                    from: old.id,
                    to: new.id,
                    name: old.name.clone(),
                });
                None
            }
        };
        if let Some(reason) = reason {
            dropped.push((SwapOverride::Property(key.clone()), reason));
        }
    }
    for (slot, content) in current.slot_overrides() {
        let reason = match target_slots.iter().find(|s| s.id == slot) {
            None => Some(SwapDropReason::NoMatchingSlot),
            Some(rule) => {
                let kinds = content
                    .content()
                    .iter()
                    .map(|id| kind_of(model, *id).map(|kind| (*id, kind)))
                    .collect::<Option<Vec<_>>>()
                    .ok_or(CommandError::InvalidRequest {
                        reason: "slot content node가 문서에 없습니다",
                    })?;
                if rule.check_content(kinds).is_ok() {
                    payload.set_slot_override(slot, content.clone());
                    None
                } else {
                    Some(SwapDropReason::SlotRuleMismatch)
                }
            }
        };
        if let Some(reason) = reason {
            if !content.is_empty() {
                return Err(CommandError::InvalidRequest {
                    reason:
                        "새 정의에 둘 곳이 없는 slot content가 있습니다(먼저 옮기거나 지웁니다)",
                });
            }
            dropped.push((SwapOverride::Slot(slot), reason));
        }
    }
    preserved.sort_by_key(|p| p.from);
    dropped.sort();
    let sizing = sizing_commands(model, node, sizing, current_size)?;
    Ok(InstanceSwapPlan {
        command: SwapInstance { node, payload },
        sizing,
        preserved,
        dropped,
    })
}

/// 컴포넌트 library 항목으로 [`plan_instance_swap`]을 부릅니다. `current`는 인스턴스가 지금 참조하는 정의여야
/// 합니다.
pub fn plan_instance_swap_with_entries(
    model: &DocumentModel,
    node: StableId,
    current: &ComponentEntry,
    target: &ComponentEntry,
    sizing: SwapSizing,
) -> Result<InstanceSwapPlan, CommandError> {
    let probe = SwapInstance {
        node,
        payload: ComponentInstancePayload::new(target.definition().reference()),
    };
    if probe.current(model)?.component() != current.definition().reference() {
        return Err(CommandError::InvalidRequest {
            reason: "인스턴스가 참조하는 정의와 current 정의가 다릅니다",
        });
    }
    let summary = |entry: &ComponentEntry| {
        entry
            .property_records()
            .map(|p| SwapProperty {
                id: p.id(),
                name: p.name().to_string(),
                value_type: p.value_type(),
                public: p.is_public(),
            })
            .collect::<Vec<_>>()
    };
    let slots: Vec<ComponentSlot> = target.slots().cloned().collect();
    plan_instance_swap(
        model,
        node,
        target.definition().reference(),
        &summary(current),
        &summary(target),
        &slots,
        sizing,
        definition_root_size(current),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{decode_command, encode_command};
    use crate::command::test_fixtures::{frame, sid, small_model};
    use crate::command::Command;
    use crate::request::CommandRequest;
    use crate::state::DocumentState;
    use stellar_core_types::typed_value::TypedValue;
    use stellar_document::component::instance::SlotOverride;
    use stellar_document::records::NodePayload;
    use stellar_document::test_support::semantic_snapshot::model_snapshot_json;

    fn prop(n: u64) -> ComponentPropertyId {
        ComponentPropertyId::new(sid(n))
    }

    fn info(n: u64, name: &str, ty: &'static str, public: bool) -> SwapProperty<&'static str> {
        SwapProperty {
            id: prop(n),
            name: name.into(),
            value_type: ty,
            public,
        }
    }

    fn slot(n: u64) -> ComponentSlotId {
        ComponentSlotId::new(sid(n))
    }

    /// 새 정의의 Slot 규칙: `kinds`만 허용, content 0~`max`개.
    fn rule(n: u64, kinds: &[NodeKind], max: Option<u32>) -> ComponentSlot {
        use stellar_core_types::reference::NodeRef;
        use stellar_document::component::slot::SlotCardinality;
        ComponentSlot::new(
            slot(n),
            NodeRef::new(sid(n + 1_000)),
            kinds.iter().copied(),
            SlotCardinality::new(0, max).unwrap(),
            Vec::new(),
        )
    }

    /// root(10) 아래 인스턴스 30(컴포넌트 sid(500)): override 6개 + slot 2개.
    fn state() -> DocumentState {
        let mut model = small_model();
        let mut payload = ComponentInstancePayload::new(ComponentRef::new(sid(500)));
        payload.set_property_override(
            OverrideKey::direct(prop(1)),
            TypedValue::String("확인".into()),
        );
        payload.set_property_override(OverrideKey::direct(prop(2)), TypedValue::Boolean(true));
        payload.set_property_override(OverrideKey::direct(prop(3)), TypedValue::Boolean(false));
        payload.set_property_override(OverrideKey::direct(prop(4)), TypedValue::String("x".into()));
        payload.set_property_override(
            OverrideKey::nested(vec![sid(77)], prop(1)),
            TypedValue::String("안".into()),
        );
        payload.set_property_override(OverrideKey::direct(prop(9)), TypedValue::Boolean(true));
        payload.set_slot_override(slot(40), SlotOverride::empty());
        payload.set_slot_override(slot(41), SlotOverride::empty());
        let mut record = frame(30, Some(10), 0x30);
        record.payload = NodePayload::ComponentInstance(payload);
        model.insert_node(record).unwrap();
        DocumentState::new(model)
    }

    fn current() -> Vec<SwapProperty<&'static str>> {
        vec![
            info(1, "label", "String", true),
            info(2, "count", "Boolean", true),
            info(3, "hidden", "Boolean", true),
            info(4, "gone", "String", true),
        ]
    }

    fn target() -> Vec<SwapProperty<&'static str>> {
        vec![
            info(101, "label", "String", true),
            info(102, "count", "Number", true),
            info(103, "hidden", "Boolean", false),
            info(104, "other", "String", true),
        ]
    }

    #[test]
    fn preserve_and_drop_matrix_is_deterministic_and_undoable() {
        let mut state = state();
        let before = model_snapshot_json(state.model()).unwrap();
        let plan = |state: &DocumentState| {
            plan_instance_swap(
                state.model(),
                sid(30),
                ComponentRef::new(sid(600)),
                &current(),
                &target(),
                &[rule(40, &[NodeKind::Frame], None)],
                SwapSizing::DefinitionDefault,
                None,
            )
            .unwrap()
        };
        let first = plan(&state);
        assert_eq!(first, plan(&state), "같은 입력 같은 preview");
        assert_eq!(
            first.preserved,
            vec![PreservedOverride {
                from: prop(1),
                to: prop(101),
                name: "label".into()
            }]
        );
        let mut expected = vec![
            (
                SwapOverride::Property(OverrideKey::direct(prop(2))),
                SwapDropReason::TypeMismatch,
            ),
            (
                SwapOverride::Property(OverrideKey::direct(prop(3))),
                SwapDropReason::NotPublic,
            ),
            (
                SwapOverride::Property(OverrideKey::direct(prop(4))),
                SwapDropReason::NoMatchingProperty,
            ),
            (
                SwapOverride::Property(OverrideKey::direct(prop(9))),
                SwapDropReason::UnknownProperty,
            ),
            (
                SwapOverride::Property(OverrideKey::nested(vec![sid(77)], prop(1))),
                SwapDropReason::NestedPath,
            ),
            (SwapOverride::Slot(slot(41)), SwapDropReason::NoMatchingSlot),
        ];
        expected.sort();
        assert_eq!(first.dropped, expected);
        let payload = &first.command.payload;
        assert_eq!(payload.component(), ComponentRef::new(sid(600)));
        assert_eq!(
            payload.property_override(&OverrideKey::direct(prop(101))),
            Some(&TypedValue::String("확인".into()))
        );
        assert_eq!(payload.property_overrides().count(), 1);
        assert_eq!(payload.slot_overrides().count(), 1);

        let command = Command::SwapInstance(first.command.clone());
        let bytes = encode_command(&command);
        assert_eq!(&bytes[..2], &0x0090u16.to_le_bytes());
        assert_eq!(decode_command(&bytes).unwrap(), command);
        let outcome = state.execute(&CommandRequest::at(&state, command)).unwrap();
        assert_eq!(
            state.model().references().outbound(sid(30)),
            vec![sid(600)],
            "참조 index도 새 정의를 가리킴"
        );
        // undo: 원래 ComponentRef와 override 전체 복원.
        state
            .execute(&CommandRequest::at(&state, outcome.inverse))
            .unwrap();
        assert_eq!(model_snapshot_json(state.model()).unwrap(), before);
        let Some(NodePayload::ComponentInstance(restored)) = state.model().payload(sid(30)) else {
            panic!();
        };
        assert_eq!(restored.component(), ComponentRef::new(sid(500)));
        assert_eq!(restored.property_overrides().count(), 6);
    }

    #[test]
    fn swap_never_leaves_owned_slot_content_unprojected() {
        // IX.3.2: 인스턴스 소유 node는 모두 slot에 투영돼야 합니다. 보존할 수 있는 slot content는 옮기고, 둘
        // 곳이 없는 content가 있으면 계획·command가 거부합니다(문서 무변경).
        let mut model = small_model();
        let mut payload = ComponentInstancePayload::new(ComponentRef::new(sid(500)));
        payload.set_slot_override(slot(40), SlotOverride::new(vec![sid(31)]).unwrap());
        payload.set_slot_override(slot(41), SlotOverride::empty());
        let mut record = frame(30, Some(10), 0x30);
        record.payload = NodePayload::ComponentInstance(payload);
        model.insert_node(record).unwrap();
        model.insert_node(frame(31, Some(30), 0x10)).unwrap();
        let mut state = DocumentState::new(model);
        let before = model_snapshot_json(state.model()).unwrap();
        let plan = |slots: &[ComponentSlot]| {
            plan_instance_swap::<&str>(
                state.model(),
                sid(30),
                ComponentRef::new(sid(600)),
                &[],
                &[],
                slots,
                SwapSizing::DefinitionDefault,
                None,
            )
        };
        // 40은 Frame 허용이라 content째 보존, 빈 41은 새 정의에 없어 탈락.
        let kept = plan(&[rule(40, &[NodeKind::Frame], Some(1))]).unwrap();
        assert_eq!(
            kept.dropped,
            vec![(SwapOverride::Slot(slot(41)), SwapDropReason::NoMatchingSlot)]
        );
        assert_eq!(
            kept.command
                .payload
                .slot_overrides()
                .map(|(id, content)| (id, content.content().to_vec()))
                .collect::<Vec<_>>(),
            vec![(slot(40), vec![sid(31)])]
        );
        // content가 있는 40을 둘 곳이 없거나(새 정의에 없음) 규칙을 어기면(Text만 허용) 거부합니다.
        assert!(plan(&[]).is_err());
        assert!(plan(&[rule(40, &[NodeKind::Text], None)]).is_err());
        let outcome = state
            .execute(&CommandRequest::at(
                &state,
                Command::SwapInstance(kept.command.clone()),
            ))
            .unwrap();
        state
            .execute(&CommandRequest::at(&state, outcome.inverse))
            .unwrap();
        assert_eq!(model_snapshot_json(state.model()).unwrap(), before);

        // command 수준: 소유 node를 버리는 payload, 남의 node(11), 두 번 투영, 없는 node는 거부합니다.
        let with = |slots: &[(u64, Vec<StableId>)]| {
            let mut payload = ComponentInstancePayload::new(ComponentRef::new(sid(600)));
            for (n, content) in slots {
                payload.set_slot_override(slot(*n), SlotOverride::new(content.clone()).unwrap());
            }
            Command::SwapInstance(SwapInstance {
                node: sid(30),
                payload,
            })
        };
        for command in [
            with(&[]),
            with(&[(40, vec![sid(11)])]),
            with(&[(40, vec![sid(31)]), (41, vec![sid(31)])]),
            with(&[(40, vec![sid(31)]), (41, vec![sid(99)])]),
        ] {
            assert!(
                state
                    .execute(&CommandRequest::at(&state, command.clone()))
                    .is_err(),
                "{command:?}"
            );
        }
        assert_eq!(model_snapshot_json(state.model()).unwrap(), before);
    }

    #[test]
    fn sizing_unsets_or_keeps_the_instance_size_in_one_transaction() {
        use crate::id::{CommandId, CommandOrigin};
        use crate::transaction::CommandTransaction;
        use stellar_document::layout_record::LayoutRecord;
        let size = Size2::new(100., 80.).unwrap();
        let plan = |state: &DocumentState, sizing| {
            plan_instance_swap(
                state.model(),
                sid(30),
                ComponentRef::new(sid(600)),
                &current(),
                &target(),
                &[
                    rule(40, &[NodeKind::Frame], None),
                    rule(41, &[NodeKind::Frame], None),
                ],
                sizing,
                Some(size),
            )
            .unwrap()
        };
        let run = |state: &mut DocumentState, plan: &InstanceSwapPlan| {
            let tx = plan.commands().into_iter().fold(
                CommandTransaction::new(CommandId::from_stable_id(sid(9_999)), CommandOrigin::User),
                |tx, command| tx.with_command(state.revision(), command),
            );
            state.execute_transaction(&tx).unwrap()
        };
        let width = |state: &DocumentState| {
            state.model().layout_record(sid(30)).map(|r| {
                (
                    r.get(LayoutLonghand::Width).cloned(),
                    r.get(LayoutLonghand::Height).cloned(),
                )
            })
        };
        // 명시 크기가 없으면 기본은 크기 command가 없고, 현재 크기 유지는 지금 정의 root 크기를 둡니다.
        let mut state = state();
        assert!(plan(&state, SwapSizing::DefinitionDefault)
            .sizing
            .is_empty());
        let keep = plan(&state, SwapSizing::KeepCurrent);
        assert_eq!(keep.sizing.len(), 2);
        let before = model_snapshot_json(state.model()).unwrap();
        let outcome = run(&mut state, &keep);
        assert_eq!(
            width(&state),
            Some((
                Some(LayoutValue::Length(100.)),
                Some(LayoutValue::Length(80.))
            ))
        );
        // undo 한 번(역 transaction)이 교체와 크기를 함께 되돌립니다.
        state.execute_transaction(&outcome.inverse).unwrap();
        assert_eq!(model_snapshot_json(state.model()).unwrap(), before);
        // 명시 크기가 있으면 기본은 지우고(새 정의 크기를 따름), 현재 크기 유지는 그대로 둡니다.
        let mut model = state.into_model();
        let mut record = LayoutRecord::new(sid(30));
        record
            .set(LayoutLonghand::Width, LayoutValue::Length(240.))
            .unwrap();
        model.set_layout_record(record).unwrap();
        let mut state = DocumentState::new(model);
        assert_eq!(
            plan(&state, SwapSizing::KeepCurrent).sizing.len(),
            1,
            "높이만 둠"
        );
        let reset = plan(&state, SwapSizing::DefinitionDefault);
        assert_eq!(
            reset.sizing,
            vec![Command::UnsetProperty(UnsetProperty {
                node: sid(30),
                longhand: LayoutLonghand::Width
            })]
        );
        run(&mut state, &reset);
        assert_eq!(width(&state), None);
    }

    #[test]
    fn definition_root_size_reads_the_root_frame() {
        use stellar_core_types::order_key::OrderKey;
        use stellar_core_types::reference::NodeRef;
        use stellar_core_types::value::Transform2D;
        use stellar_document::component::definition::{
            ComponentDefinition, ComponentLibrary, DefinitionNode, DefinitionTree,
        };
        use stellar_document::frame::FramePayload;
        use stellar_document::record_codec::encode_payload;
        let component = ComponentRef::new(sid(900));
        let mut tree = DefinitionTree::new(component);
        tree.insert(DefinitionNode {
            id: sid(901),
            kind: NodeKind::Frame,
            parent: None,
            order_key: OrderKey::from_bytes(vec![0x40]).unwrap(),
            visible: true,
            locked: false,
            display_name: "root".into(),
            payload: encode_payload(&NodePayload::Frame(FramePayload {
                size: Size2::new(120., 44.).unwrap(),
                transform: Transform2D::identity(),
                ..FramePayload::default()
            })),
        })
        .unwrap();
        let mut library = ComponentLibrary::new();
        library
            .insert(
                ComponentDefinition::new(sid(900), "버튼".into(), NodeRef::new(sid(901))),
                tree,
            )
            .unwrap();
        assert_eq!(
            definition_root_size(library.get(component).unwrap()),
            Some(Size2::new(120., 44.).unwrap())
        );
    }

    #[test]
    fn keep_current_size_and_non_instances_are_rejected() {
        let state = state();
        assert!(matches!(
            plan_instance_swap(
                state.model(),
                sid(30),
                ComponentRef::new(sid(600)),
                &current(),
                &target(),
                &[],
                SwapSizing::KeepCurrent,
                None,
            ),
            Err(CommandError::InvalidRequest { .. })
        ));
        assert!(matches!(
            plan_instance_swap(
                state.model(),
                sid(11),
                ComponentRef::new(sid(600)),
                &current(),
                &target(),
                &[],
                SwapSizing::DefinitionDefault,
                None,
            ),
            Err(CommandError::ConversionKind { .. })
        ));
        let mut state = state;
        let nil = Command::SwapInstance(SwapInstance {
            node: sid(30),
            payload: ComponentInstancePayload::new(ComponentRef::new(StableId::NIL)),
        });
        assert_eq!(
            state.execute(&CommandRequest::at(&state, nil)),
            Err(CommandError::NilId)
        );
    }

    #[test]
    fn library_entries_drive_the_same_plan() {
        use stellar_document::test_support::component_fixtures::{
            button_label, button_ref, component_library_fixture, modal_ref, text_input_ref,
        };
        let fixture = component_library_fixture().unwrap();
        let library = &fixture.library;
        let mut model = small_model();
        let mut payload = ComponentInstancePayload::new(button_ref());
        payload.set_property_override(
            OverrideKey::direct(button_label()),
            TypedValue::String("저장".into()),
        );
        let mut record = frame(30, Some(10), 0x30);
        record.payload = NodePayload::ComponentInstance(payload);
        model.insert_node(record).unwrap();
        let button = library.get(button_ref()).unwrap();
        let input = library.get(text_input_ref()).unwrap();
        let plan = plan_instance_swap_with_entries(
            &model,
            sid(30),
            button,
            input,
            SwapSizing::DefinitionDefault,
        )
        .unwrap();
        // TextInput에는 label이 없어 탈락합니다.
        assert!(plan.preserved.is_empty());
        assert_eq!(
            plan.dropped,
            vec![(
                SwapOverride::Property(OverrideKey::direct(button_label())),
                SwapDropReason::NoMatchingProperty
            )]
        );
        // 같은 정의로 바꾸면 이름·타입이 같아 모두 보존합니다.
        let same = plan_instance_swap_with_entries(
            &model,
            sid(30),
            button,
            button,
            SwapSizing::DefinitionDefault,
        )
        .unwrap();
        assert_eq!(same.preserved.len(), 1);
        assert!(same.dropped.is_empty());
        // current가 실제 참조와 다르면 거부합니다.
        let modal = library.get(modal_ref()).unwrap();
        assert!(plan_instance_swap_with_entries(
            &model,
            sid(30),
            modal,
            input,
            SwapSizing::DefinitionDefault
        )
        .is_err());
    }
}
