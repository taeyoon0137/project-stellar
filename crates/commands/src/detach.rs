//! 인스턴스 구조 command: Detach 실행과 Slot content 넣기·꺼내기 (XII.10.34, IX.2.6, IX.3.3, ADR-0129 결정 4).
//!
//! 두 동작 모두 "인스턴스 node 하나의 레코드를 바꾸고, 그 아래에 node를 만들고, 기존 node의 부모·순서를 바꾸고,
//! node를 지우는" 구조 변경입니다. 같은 payload [`InstanceRestructure`]를 두 command 종류가 씁니다.
//!
//! - `detach-instance`(0x0094): 인스턴스를 정의 root의 복제 Frame으로 바꾸고 정의 tree를 실제 node로 복제하며, 인스턴스
//!   소유 Slot content를 복제된 slot host 아래로 옮깁니다. 계획은 [`plan_detach`]가 IX.2.6 함수
//!   (`component::detach::detach_instance`)를 인스턴스 subtree 사본에 실행해 만듭니다(규칙 원본 하나).
//! - `place-slot-content`(0x0095): node를 인스턴스 아래로 옮기거나 새로 만들고 slot override에 투영합니다(끌어 놓기),
//!   또는 반대로 투영을 빼고 밖으로 옮깁니다. 계획은 runtime `canvas::instance`가 컴포넌트 library로 slot 규칙을
//!   확인한 뒤 만듭니다.
//!
//! - Slot content 지우기(ADR-0133 결정 2): 구조 command `delete-subtree`는 Slot content를 지우지 않습니다(slot override를
//!   바꾸지 않으므로 투영이 깨짐). [`plan_slot_content_delete`]가 content를 slot override에서 빼고 subtree를 지우는
//!   `place-slot-content` 하나를 만듭니다.
//!
//! 역 command는 같은 종류의 [`InstanceRestructure`]입니다(현재 레코드, 지운 node 복원, 원래 부모·순서, 만든 node
//! 지우기). 그래서 undo·redo가 정확합니다.
//!
//! **검증.** 적용 전에 대상·id·부모 관계를 검사하고(문서 무변경), 적용 뒤 결과 문서 전체를 문서 validator(VI.10.1)로
//! 다시 검사해 이 command가 새로 만든 위반(투영되지 않은 인스턴스 자식, OrderKey 충돌, layout record 위반 등)이 있으면
//! 되돌립니다. 드문 사용자 동작이라 문서 크기에 선형인 검사를 허용합니다(정의 편집 command와 같은 정책).

use std::collections::{BTreeSet, HashMap};
use std::fmt;

use stellar_core_types::clock::MonotonicClock;
use stellar_core_types::ids::{RuntimeEntityId, StableId, StableIdGenerator};
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::value::Size2;
use stellar_core_types::value::Transform2D;
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::component::detach::{detach_instance, DetachError};
use stellar_document::component::expansion::ScenePayloads;
use stellar_document::component::instance::{frame_instance_transform, SlotOverride};
use stellar_document::component::slot::SlotError;
use stellar_document::layout_record::{LayoutLonghand, LayoutRecord, LayoutValue};
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{ModifierKind, ModifierStack};
use stellar_document::node_columns::{NodeColumns, NodeFields};
use stellar_document::node_kind::NodeKind;
use stellar_document::record_codec::{
    decode_node_record, decode_payload, encode_node_record, encode_payload,
};
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_document::tree_ops::validate_reparent;
use stellar_document::validate::{
    can_have_children, can_own_slot_content, validate_document, DocumentValidationError,
};

use crate::change::{node_field_invalidation, node_keys, STRUCTURE_INVALIDATION};
use crate::codec::{CommandCodecError, Reader, Writer};
use crate::command::CommandError;
use crate::edit::EditContext;
use crate::lock::{edit_permission, EditIntent};
use crate::node_ops::{
    check_external_inbound, decode_layout_records, encode_layout_records, slot_content_owner,
};
use crate::tree::{parent_of, ChildIndex};

/// 기존 node 하나의 새 부모·순서.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodePlacement {
    /// 옮길 node.
    pub node: StableId,
    /// 새 부모.
    pub parent: StableId,
    /// 새 형제 순서 key.
    pub order_key: OrderKey,
}

/// 인스턴스 하나의 구조 변경(`detach-instance`·`place-slot-content` 공용 payload).
///
/// 적용 순서는 레코드 교체 → 만들기 → 옮기기 → 지우기입니다.
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceRestructure {
    /// 대상 node(인스턴스, 또는 detach 결과 Frame).
    pub node: StableId,
    /// 대상 node의 바뀐 레코드. id·부모·순서는 지금과 같아야 하고 종류·payload·표시·잠금·이름이 바뀝니다.
    pub record: NodeRecord,
    /// 대상 node의 바뀐 명시 layout record(빈 record = 명시 값 없음).
    pub layout: LayoutRecord,
    /// 대상 node의 바뀐 modifier stack(`None` = 그대로, 빈 stack = 제거). Detach는 인스턴스 배치 Transform modifier를
    /// Frame payload transform으로 옮기므로 이 값을 씁니다.
    pub stack: Option<ModifierStack>,
    /// 새로 만들 node(전위 순서). 부모는 대상 node이거나 앞선 레코드입니다.
    pub insert: Vec<NodeRecord>,
    /// 만들 node의 modifier stack.
    pub insert_stacks: Vec<ModifierStack>,
    /// 만들 node의 명시 layout record(소유 node StableId 순).
    pub insert_layouts: Vec<LayoutRecord>,
    /// 부모·순서를 바꿀 기존 node.
    pub moves: Vec<NodePlacement>,
    /// 지울 node(자식부터). 지울 때 자식이 없어야 하고, 부모는 대상 node이거나 함께 지우는 node입니다.
    pub remove: Vec<StableId>,
}

fn invalid(reason: &'static str) -> CommandError {
    CommandError::InvalidRequest { reason }
}

fn handle(model: &DocumentModel, id: StableId) -> Result<RuntimeEntityId, CommandError> {
    model.runtime_id(id).ok_or(CommandError::UnknownNode { id })
}

impl InstanceRestructure {
    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        let current = model
            .record(self.node)
            .ok_or(CommandError::UnknownNode { id: self.node })?;
        if self.record.id != self.node
            || self.record.parent != current.parent
            || self.record.order_key != current.order_key
        {
            return Err(invalid("대상 node의 id·부모·순서는 바꿀 수 없습니다"));
        }
        let kind = self.record.kind();
        if !can_have_children(kind) && !can_own_slot_content(kind) {
            return Err(invalid(
                "대상 node는 Frame·Fragment·ComponentInstance여야 합니다",
            ));
        }
        if self.layout.node != self.node {
            return Err(invalid("대상 layout record의 소유 node가 다릅니다"));
        }
        if self
            .stack
            .as_ref()
            .is_some_and(|stack| stack.node != self.node)
        {
            return Err(invalid("대상 modifier stack의 소유 node가 다릅니다"));
        }
        let changes_record = current != self.record
            || model
                .layout_record(self.node)
                .cloned()
                .unwrap_or_else(|| LayoutRecord::new(self.node))
                != self.layout
            || self
                .stack
                .as_ref()
                .is_some_and(|stack| !same_stack(model, stack));
        if !changes_record
            && self.insert.is_empty()
            && self.moves.is_empty()
            && self.remove.is_empty()
        {
            return Err(invalid("바꿀 구조가 없습니다"));
        }
        // 만들 node: 새 id, 부모는 대상 node 또는 앞선 레코드.
        let mut inserted = BTreeSet::new();
        for record in &self.insert {
            if record.id == StableId::NIL {
                return Err(CommandError::NilId);
            }
            if model.contains(record.id) || !inserted.insert(record.id) {
                return Err(CommandError::DuplicateId { id: record.id });
            }
            match record.parent {
                Some(parent) if parent == self.node || inserted.contains(&parent) => {}
                parent => {
                    return Err(CommandError::InvalidParent {
                        node: record.id,
                        parent,
                    })
                }
            }
        }
        let mut owners = BTreeSet::new();
        for stack in &self.insert_stacks {
            if !inserted.contains(&stack.node) || !owners.insert(stack.node) {
                return Err(invalid("만들 node의 modifier stack이 아닙니다"));
            }
        }
        let mut owners = BTreeSet::new();
        for layout in &self.insert_layouts {
            if !inserted.contains(&layout.node) || !owners.insert(layout.node) {
                return Err(invalid("만들 node의 layout record가 아닙니다"));
            }
        }
        // 옮길 node: 있는 node, 대상·만들 node가 아님, 한 번씩. 같은 View tree 안에서 cycle 없이 옮깁니다.
        let target = handle(model, self.node)?;
        let mut moved = BTreeSet::new();
        for placement in &self.moves {
            let node = handle(model, placement.node)?;
            if placement.node == self.node || inserted.contains(&placement.node) {
                return Err(invalid("대상 node나 만들 node는 옮길 수 없습니다"));
            }
            if !moved.insert(placement.node) {
                return Err(CommandError::DuplicateId { id: placement.node });
            }
            let parent = if inserted.contains(&placement.parent) {
                target
            } else {
                handle(model, placement.parent)?
            };
            validate_reparent(model.columns(), node, parent).map_err(|error| {
                CommandError::Reparent {
                    node: placement.node,
                    error,
                }
            })?;
        }
        // 지울 node: 있는 node, 대상·만들·옮길 node가 아님, 부모는 대상 node 또는 함께 지우는 node.
        let removed: BTreeSet<StableId> = self.remove.iter().copied().collect();
        if removed.len() != self.remove.len() {
            return Err(invalid("같은 node를 두 번 지울 수 없습니다"));
        }
        for id in &self.remove {
            if *id == self.node || inserted.contains(id) || moved.contains(id) {
                return Err(invalid("대상·만들·옮길 node는 지울 수 없습니다"));
            }
            match parent_of(model, *id) {
                None => return Err(CommandError::UnknownNode { id: *id }),
                Some(Some(parent)) if parent == self.node || removed.contains(&parent) => {}
                Some(parent) => return Err(CommandError::InvalidParent { node: *id, parent }),
            }
        }
        Ok(())
    }

    pub(crate) fn invert(
        &self,
        model: &DocumentModel,
    ) -> Result<InstanceRestructure, CommandError> {
        let record = model
            .record(self.node)
            .ok_or(CommandError::UnknownNode { id: self.node })?;
        let layout = model
            .layout_record(self.node)
            .cloned()
            .unwrap_or_else(|| LayoutRecord::new(self.node));
        let stack = self.stack.as_ref().map(|_| {
            model
                .modifier_stack(self.node)
                .cloned()
                .unwrap_or_else(|| ModifierStack::new(self.node))
        });
        // 지운 node를 부모부터 되살립니다.
        let mut insert = Vec::with_capacity(self.remove.len());
        let mut insert_stacks = Vec::new();
        let mut insert_layouts = Vec::new();
        for id in self.remove.iter().rev() {
            insert.push(
                model
                    .record(*id)
                    .ok_or(CommandError::UnknownNode { id: *id })?,
            );
            if let Some(stack) = model.modifier_stack(*id) {
                insert_stacks.push(stack.clone());
            }
            if let Some(layout) = model.layout_record(*id) {
                insert_layouts.push(layout.clone());
            }
        }
        insert_layouts.sort_by_key(|l| l.node);
        let mut moves = Vec::with_capacity(self.moves.len());
        for placement in self.moves.iter().rev() {
            let current = model
                .record(placement.node)
                .ok_or(CommandError::UnknownNode { id: placement.node })?;
            let parent = current.parent.ok_or(CommandError::ViewRoot {
                node: placement.node,
            })?;
            moves.push(NodePlacement {
                node: placement.node,
                parent,
                order_key: current.order_key,
            });
        }
        Ok(InstanceRestructure {
            node: self.node,
            record,
            layout,
            stack,
            insert,
            insert_stacks,
            insert_layouts,
            moves,
            remove: self.insert.iter().rev().map(|r| r.id).collect(),
        })
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        let before = violations(ctx.model())?;
        // 1. 대상 레코드와 layout record.
        let previous = ctx.replace_record(self.record.clone())?;
        if previous.kind() != self.record.kind() || previous.payload != self.record.payload {
            ctx.changes().structure(self.node);
        }
        for (key, changed) in [
            (node_keys::VISIBLE, previous.visible != self.record.visible),
            (node_keys::LOCKED, previous.locked != self.record.locked),
            (
                node_keys::DISPLAY_NAME,
                previous.display_name != self.record.display_name,
            ),
        ] {
            if changed {
                let invalidation = node_field_invalidation(key).unwrap_or_default();
                ctx.changes().node_property(self.node, key, invalidation);
            }
        }
        if ctx.model().layout_record(self.node) != Some(&self.layout)
            && !(self.layout.is_empty() && ctx.model().layout_record(self.node).is_none())
        {
            ctx.set_layout_record(self.layout.clone())?;
            ctx.changes().structure(self.node);
        }
        if let Some(stack) = &self.stack {
            if !same_stack(ctx.model(), stack) {
                ctx.set_modifier_stack(stack.clone())?;
            }
        }
        // 2. 만들기.
        for record in &self.insert {
            ctx.insert_node(record.clone())?;
            ctx.changes().inserted(record.id);
        }
        for stack in &self.insert_stacks {
            if !stack.entries().is_empty() {
                ctx.set_modifier_stack(stack.clone())?;
            }
        }
        for layout in &self.insert_layouts {
            ctx.set_layout_record(layout.clone())?;
        }
        // 3. 옮기기.
        for placement in &self.moves {
            let mut record = ctx
                .model()
                .record(placement.node)
                .ok_or(CommandError::UnknownNode { id: placement.node })?;
            let old_parent = record.parent;
            record.parent = Some(placement.parent);
            record.order_key = placement.order_key.clone();
            ctx.replace_record(record)?;
            ctx.changes()
                .node_property(placement.node, node_keys::PARENT, STRUCTURE_INVALIDATION);
            ctx.changes().node_property(
                placement.node,
                node_keys::ORDER_KEY,
                STRUCTURE_INVALIDATION,
            );
            if let Some(old_parent) = old_parent {
                ctx.changes().structure(old_parent);
            }
            ctx.changes().structure(placement.parent);
        }
        // 4. 지우기.
        for id in &self.remove {
            ctx.remove_leaf(*id)?;
            ctx.changes().removed(*id);
        }
        ctx.changes().structure(self.node);
        // 결과 문서 전체 검사: 이 command가 새로 만든 위반이 있으면 되돌립니다.
        let introduced: Vec<_> = violations(ctx.model())?
            .into_iter()
            .filter(|v| !before.contains(v))
            .collect();
        if introduced.is_empty() {
            Ok(())
        } else {
            Err(CommandError::InvalidResult(Box::new(
                DocumentValidationError {
                    violations: introduced,
                },
            )))
        }
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.id(self.node);
        out.bytes(&encode_node_record(&self.record));
        out.bytes(&self.layout.encode());
        match &self.stack {
            None => out.u8(0),
            Some(stack) => {
                out.u8(1);
                out.bytes(&stack.encode());
            }
        }
        out.len(self.insert.len());
        for record in &self.insert {
            out.bytes(&encode_node_record(record));
        }
        out.len(self.insert_stacks.len());
        for stack in &self.insert_stacks {
            out.bytes(&stack.encode());
        }
        out.len(self.moves.len());
        for placement in &self.moves {
            out.id(placement.node);
            out.id(placement.parent);
            out.bytes(placement.order_key.as_bytes());
        }
        out.len(self.remove.len());
        for id in &self.remove {
            out.id(*id);
        }
        encode_layout_records(out, &self.insert_layouts);
    }

    pub(crate) fn decode_from(
        input: &mut Reader<'_>,
    ) -> Result<InstanceRestructure, CommandCodecError> {
        let malformed = |reason| CommandCodecError::Malformed { reason };
        let node = input.id()?;
        let record =
            decode_node_record(input.bytes()?).map_err(|_| malformed("잘못된 node 레코드"))?;
        let layout =
            LayoutRecord::decode(input.bytes()?).map_err(|_| malformed("잘못된 layout record"))?;
        let stack = match input.u8()? {
            0 => None,
            1 => Some(
                ModifierStack::decode(input.bytes()?)
                    .map_err(|_| malformed("잘못된 modifier stack"))?,
            ),
            _ => return Err(malformed("잘못된 modifier stack 표지")),
        };
        let count = input.len()?;
        let mut insert = Vec::new();
        for _ in 0..count {
            insert.push(
                decode_node_record(input.bytes()?).map_err(|_| malformed("잘못된 node 레코드"))?,
            );
        }
        let count = input.len()?;
        let mut insert_stacks = Vec::new();
        for _ in 0..count {
            insert_stacks.push(
                ModifierStack::decode(input.bytes()?)
                    .map_err(|_| malformed("잘못된 modifier stack"))?,
            );
        }
        let count = input.len()?;
        let mut moves = Vec::new();
        for _ in 0..count {
            let node = input.id()?;
            let parent = input.id()?;
            let order_key = OrderKey::from_bytes(input.bytes()?.to_vec())
                .map_err(|_| malformed("잘못된 OrderKey"))?;
            moves.push(NodePlacement {
                node,
                parent,
                order_key,
            });
        }
        let count = input.len()?;
        let mut remove = Vec::new();
        for _ in 0..count {
            remove.push(input.id()?);
        }
        Ok(InstanceRestructure {
            node,
            record,
            layout,
            stack,
            insert,
            insert_stacks,
            insert_layouts: decode_layout_records(input)?,
            moves,
            remove,
        })
    }
}

/// 대상의 지금 modifier stack이 `stack`과 같은지(빈 stack = stack 없음).
fn same_stack(model: &DocumentModel, stack: &ModifierStack) -> bool {
    match model.modifier_stack(stack.node) {
        Some(current) => current == stack,
        None => stack.entries().is_empty(),
    }
}

/// 문서 validator 위반 목록(위반이 없으면 빈 목록).
fn violations(
    model: &DocumentModel,
) -> Result<Vec<stellar_document::validate::DocumentViolation>, CommandError> {
    Ok(validate_document(&model.to_records()?)
        .err()
        .map(|error| error.violations)
        .unwrap_or_default())
}

/// Detach 계획 실패.
#[derive(Debug, Clone, PartialEq)]
pub enum DetachPlanError {
    /// 대상이 문서의 ComponentInstance가 아닙니다.
    NotInstance,
    /// IX.2.6 detach 규칙이 거부했습니다(미해결 정의, slot 투영 위반, 둘 곳 없는 Slot content 등).
    Detach(DetachError),
    /// 복제 결과를 문서 레코드로 바꾸지 못했습니다(정의 payload 해석 실패).
    Payload {
        /// node.
        node: StableId,
    },
}

impl fmt::Display for DetachPlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DetachPlanError::NotInstance => f.write_str("ComponentInstance가 아닙니다"),
            DetachPlanError::Detach(error) => write!(f, "{error}"),
            DetachPlanError::Payload { node } => {
                write!(f, "정의 node {node}의 payload를 해석하지 못했습니다")
            }
        }
    }
}

impl std::error::Error for DetachPlanError {}

/// 인스턴스 subtree(인스턴스 + 소유 Slot content와 그 자손)의 scene 사본.
fn instance_scene(
    model: &DocumentModel,
    instance: StableId,
) -> Result<(NodeColumns, ScenePayloads), DetachPlanError> {
    let index = ChildIndex::build(model);
    let mut columns = NodeColumns::new();
    let mut payloads = ScenePayloads::new();
    let mut handles: HashMap<StableId, RuntimeEntityId> = HashMap::new();
    for id in index.subtree(instance) {
        let record = model.record(id).ok_or(DetachPlanError::NotInstance)?;
        let parent = if id == instance {
            None
        } else {
            record.parent.and_then(|p| handles.get(&p).copied())
        };
        let handle = columns.insert(NodeFields {
            stable_id: id,
            kind: record.kind(),
            parent,
            order_key: record.order_key.clone(),
            visible: record.visible,
            locked: record.locked,
            display_name: record.display_name.clone(),
        });
        handles.insert(id, handle);
        payloads.insert(id, encode_payload(&record.payload));
    }
    Ok((columns, payloads))
}

/// 사본 scene의 node 하나를 문서 레코드로 바꿉니다.
fn scene_record(
    columns: &NodeColumns,
    payloads: &ScenePayloads,
    handles: &HashMap<StableId, RuntimeEntityId>,
    id: StableId,
) -> Result<NodeRecord, DetachPlanError> {
    let fields = handles
        .get(&id)
        .and_then(|h| columns.get(*h))
        .ok_or(DetachPlanError::Payload { node: id })?;
    let payload = payloads
        .get(&id)
        .and_then(|bytes| decode_payload(fields.kind, bytes).ok())
        .ok_or(DetachPlanError::Payload { node: id })?;
    let parent = fields
        .parent
        .and_then(|p| columns.get(p))
        .map(|p| p.stable_id);
    Ok(NodeRecord {
        id,
        parent,
        order_key: fields.order_key,
        visible: fields.visible,
        locked: fields.locked,
        display_name: fields.display_name,
        payload,
    })
}

/// 인스턴스 Detach 계획(IX.2.6을 문서 command로).
///
/// - 인스턴스 subtree 사본에 `component::detach::detach_instance`를 실행해 복제 node, 옮길 Slot content, 바뀐 인스턴스
///   레코드를 얻습니다. 인스턴스 StableId는 그대로이고 복제 node는 `generator`·`clock`으로 새 id를 받습니다.
/// - 인스턴스 배치는 인스턴스 Transform modifier가 갖고 있고, Frame 배치는 payload transform이 가집니다(Frame은 합성
///   효과 modifier만 평가, ADR-0116 결정 2). 그래서 복제 root Frame의 payload transform은 정의의 선형 부분에 인스턴스
///   배치(`frame_instance_transform`)를 합성한 값이고(scene 투영 `placed_free_frame`과 같은 그림), Transform modifier는
///   stack에서 지웁니다. 다른 modifier는 그대로 둡니다.
/// - 인스턴스 layout record의 명시 `width`·`height`가 `<length>`이면 Frame payload 크기로 옮기고(Frame 크기는 payload에
///   저장, ADR-0086), 그 밖의 크기 값은 지웁니다. 나머지 longhand는 그대로 둡니다.
/// - Property override 값은 정의 node 속성과의 binding(VIII.6)이 문서에 없어 복제 node에 반영하지 않습니다.
pub fn plan_detach<C: MonotonicClock>(
    model: &DocumentModel,
    library: &ComponentLibrary,
    instance: StableId,
    generator: &mut StableIdGenerator,
    clock: &C,
) -> Result<InstanceRestructure, DetachPlanError> {
    let current = model.record(instance).ok_or(DetachPlanError::NotInstance)?;
    if current.kind() != NodeKind::ComponentInstance {
        return Err(DetachPlanError::NotInstance);
    }
    // 인스턴스 배치(활성 Transform modifier 합성)와 Transform modifier를 뺀 stack.
    let current_stack = model.modifier_stack(instance);
    let mut placement_stack = current_stack
        .cloned()
        .unwrap_or_else(|| ModifierStack::new(instance));
    let mut kept_stack = placement_stack.clone();
    for entry in current_stack.into_iter().flat_map(ModifierStack::entries) {
        let removed = if entry.kind == ModifierKind::Transform {
            kept_stack.remove(entry.id)
        } else {
            placement_stack.remove(entry.id)
        };
        removed.map_err(|_| DetachPlanError::Payload { node: instance })?;
    }
    let placement = frame_instance_transform(Some(&placement_stack))
        .ok_or(DetachPlanError::Payload { node: instance })?;
    let (mut columns, mut payloads) = instance_scene(model, instance)?;
    let detached = detach_instance(
        &mut columns,
        &mut payloads,
        library,
        instance,
        generator,
        clock,
    )
    .map_err(DetachPlanError::Detach)?;
    let handles: HashMap<StableId, RuntimeEntityId> = columns
        .iter_live()
        .filter_map(|h| columns.get(h).map(|f| (f.stable_id, h)))
        .collect();
    let scene_root = scene_record(&columns, &payloads, &handles, instance)?;
    let mut layout = model
        .layout_record(instance)
        .cloned()
        .unwrap_or_else(|| LayoutRecord::new(instance));
    let mut payload = scene_root.payload;
    if let NodePayload::Frame(frame) = &mut payload {
        let (a, b, c, d, _, _) = frame.transform.components();
        frame.transform = Transform2D::new(a, b, c, d, 0., 0.)
            .and_then(|linear| linear.then(&placement))
            .map_err(|_| DetachPlanError::Payload { node: instance })?;
        let width = layout.unset(LayoutLonghand::Width);
        let height = layout.unset(LayoutLonghand::Height);
        let px = |value: Option<LayoutValue>, fallback: f32| match value {
            Some(LayoutValue::Length(px)) => px,
            _ => fallback,
        };
        frame.size = Size2::new(
            px(width, frame.size.width()),
            px(height, frame.size.height()),
        )
        .map_err(|_| DetachPlanError::Payload { node: instance })?;
    }
    let record = NodeRecord {
        id: instance,
        parent: current.parent,
        order_key: current.order_key,
        visible: scene_root.visible,
        locked: scene_root.locked,
        display_name: scene_root.display_name,
        payload,
    };
    let mut insert = Vec::with_capacity(detached.created().len());
    for id in detached.created() {
        insert.push(scene_record(&columns, &payloads, &handles, *id)?);
    }
    let mut moves = Vec::with_capacity(detached.moved().len());
    for content in detached.moved() {
        let moved = scene_record(&columns, &payloads, &handles, content.node)?;
        moves.push(NodePlacement {
            node: content.node,
            parent: moved
                .parent
                .ok_or(DetachPlanError::Payload { node: content.node })?,
            order_key: moved.order_key,
        });
    }
    Ok(InstanceRestructure {
        node: instance,
        record,
        layout,
        stack: current_stack.map(|_| kept_stack),
        insert,
        insert_stacks: Vec::new(),
        insert_layouts: Vec::new(),
        moves,
        remove: Vec::new(),
    })
}

/// Slot content 지우기 계획 실패.
#[derive(Debug, Clone, PartialEq)]
pub enum SlotContentDeleteError {
    /// node가 인스턴스 Slot content(인스턴스의 직접 자식으로 slot override에 투영된 node)가 아닙니다.
    NotSlotContent,
    /// content나 인스턴스가 잠겼습니다(D8).
    Locked,
    /// 인스턴스가 참조하는 컴포넌트가 library에 없습니다.
    UnresolvedComponent,
    /// content가 투영된 Slot을 컴포넌트가 선언하지 않습니다.
    UnknownSlot,
    /// 남는 content가 Slot 종류·개수 규칙을 어깁니다(IX.3.1).
    Rule(SlotError),
    /// 삭제 정책 위반(subtree 밖에서 content subtree를 참조함 등).
    Command(CommandError),
}

impl fmt::Display for SlotContentDeleteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SlotContentDeleteError::NotSlotContent => {
                f.write_str("인스턴스 Slot content가 아닙니다")
            }
            SlotContentDeleteError::Locked => f.write_str("content나 인스턴스가 잠겼습니다"),
            SlotContentDeleteError::UnresolvedComponent => {
                f.write_str("인스턴스 컴포넌트가 library에 없습니다")
            }
            SlotContentDeleteError::UnknownSlot => {
                f.write_str("컴포넌트가 선언하지 않은 Slot입니다")
            }
            SlotContentDeleteError::Rule(error) => write!(f, "{error}"),
            SlotContentDeleteError::Command(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for SlotContentDeleteError {}

/// 인스턴스 Slot content 하나를 지우는 `place-slot-content` 계획(ADR-0133 결정 2).
///
/// content를 slot override 투영에서 빼고 그 subtree를 자식부터 지웁니다. 마지막 content를 지우면 `restore_default`일 때
/// override를 지워 default content로 돌아가고, 아니면 빈 override(Slot 비움)로 남깁니다(runtime `plan_slot_take_out`과
/// 같은 규칙). 남는 content는 Slot 종류·개수 규칙(`ComponentSlot::check_content`)을 지켜야 합니다. 삭제 정책은
/// `delete-subtree`와 같습니다(subtree 밖 참조 거부). 역 command는 지운 node와 그 modifier·layout 기록, 원래 slot
/// override를 되살립니다.
pub fn plan_slot_content_delete(
    model: &DocumentModel,
    library: &ComponentLibrary,
    node: StableId,
    restore_default: bool,
) -> Result<InstanceRestructure, SlotContentDeleteError> {
    let instance = slot_content_owner(model, node).ok_or(SlotContentDeleteError::NotSlotContent)?;
    if edit_permission(model, node, EditIntent::Delete).is_err()
        || edit_permission(model, instance, EditIntent::Edit).is_err()
    {
        return Err(SlotContentDeleteError::Locked);
    }
    let mut record = model
        .record(instance)
        .ok_or(SlotContentDeleteError::NotSlotContent)?;
    let NodePayload::ComponentInstance(payload) = &record.payload else {
        return Err(SlotContentDeleteError::NotSlotContent);
    };
    let entry = library
        .get(payload.component())
        .ok_or(SlotContentDeleteError::UnresolvedComponent)?;
    let (slot, list) = payload
        .slot_overrides()
        .find(|(_, content)| content.content().contains(&node))
        .map(|(slot, content)| (slot, content.content().to_vec()))
        .ok_or(SlotContentDeleteError::NotSlotContent)?;
    let declared = entry
        .slot(slot)
        .ok_or(SlotContentDeleteError::UnknownSlot)?;
    let kept: Vec<StableId> = list.into_iter().filter(|n| *n != node).collect();
    let mut next = payload.clone();
    if kept.is_empty() && restore_default {
        next.remove_slot_override(slot);
    } else {
        let kinds = kept
            .iter()
            .map(|id| model.record(*id).map(|r| (*id, r.kind())))
            .collect::<Option<Vec<_>>>()
            .ok_or(SlotContentDeleteError::NotSlotContent)?;
        declared
            .check_content(kinds)
            .map_err(SlotContentDeleteError::Rule)?;
        next.set_slot_override(
            slot,
            SlotOverride::new(kept).map_err(|_| SlotContentDeleteError::NotSlotContent)?,
        );
    }
    let subtree = ChildIndex::build(model).subtree(node);
    check_external_inbound(model, &subtree).map_err(SlotContentDeleteError::Command)?;
    record.payload = NodePayload::ComponentInstance(next);
    Ok(InstanceRestructure {
        node: instance,
        record,
        layout: model
            .layout_record(instance)
            .cloned()
            .unwrap_or_else(|| LayoutRecord::new(instance)),
        stack: None,
        insert: Vec::new(),
        insert_stacks: Vec::new(),
        insert_layouts: Vec::new(),
        moves: Vec::new(),
        remove: subtree.into_iter().rev().collect(),
    })
}

#[cfg(test)]
#[path = "detach_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "detach_integrity_tests.rs"]
mod integrity_tests;
