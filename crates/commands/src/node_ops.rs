//! node 구조 command (VII.2.1~VII.2.3).
//!
//! - [`InsertNode`] (VII.2.1): caller가 StableId를 정한 node(또는 그 node를 root로 하는
//!   pre-order subtree)를 삽입합니다. 중복 id, 잘못된 부모, 형제 OrderKey 충돌, 깊이 초과,
//!   참조 종류 불일치, modifier 소유·cycle 위반, layout record 위반을 적용 전에 거부합니다. 삽입하는 node의
//!   명시 layout longhand(ADR-0086)도 함께 넣습니다.
//! - [`MoveNode`] (VII.2.3): 부모와 OrderKey를 함께 바꿉니다. cycle·다른 View로의 이동을
//!   거부합니다.
//! - [`DeleteSubtree`] (VII.2.2): subtree를 지웁니다. 역 command는 subtree와 내부 참조를 담은
//!   InsertNode이고, 외부 inbound 참조가 있으면 정책 오류로 거부합니다. InsertNode의 역입니다. 역 InsertNode는
//!   subtree의 layout record도 담으므로 history journal을 저장했다가 다시 연 뒤의 삭제 undo도 record를
//!   되살립니다.

use std::collections::{BTreeMap, BTreeSet};

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::ComponentRef;
use stellar_document::component::definition::{
    ComponentEntry, ComponentPropertyId, ComponentSlotId,
};
use stellar_document::component::instance::{ComponentInstancePayload, OverrideKey};
use stellar_document::component::slot::ComponentSlot;
use stellar_document::frame::Clipping;
use stellar_document::layout_record::{check_records, LayoutRecord, LayoutRecordViolation};
use stellar_document::model::{DocumentModel, ModelError};
use stellar_document::modifier::{find_modifier_cycle, validate_modifier_owner, ModifierStack};
use stellar_document::node_kind::NodeKind;
use stellar_document::record_codec::{decode_node_record, encode_node_record};
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_document::tree_ops::validate_reparent;
use stellar_document::validate::{can_have_children, MAX_NODE_DEPTH};

use crate::change::{node_keys, STRUCTURE_INVALIDATION};
use crate::codec::{CommandCodecError, Reader, Writer};
use crate::command::CommandError;
use crate::edit::EditContext;
use crate::tree::{depth_of, id_owners, kind_of, parent_of, path_element_ids, ChildIndex, IdOwner};

/// node(또는 subtree) 삽입.
///
/// `nodes[0]`이 삽입 root이고 부모는 문서에 이미 있는 container여야 합니다. 나머지는
/// pre-order 순서로, 부모가 앞선 레코드여야 합니다. `modifier_stacks`는 삽입하는 node의
/// stack입니다(빈 stack은 넣지 않습니다). `layout_records`는 삽입하는 node의 명시 layout longhand입니다
/// (ADR-0086). node마다 하나 이하이고 빈 record는 담지 않습니다. record가 없는 삽입 node는 명시 값이 없는
/// 상태로 들어갑니다.
///
/// binary 형식에서 `layout_records`는 끝의 선택 절입니다(record 수 `u32` 1 이상 + 길이 접두 record). record가
/// 없으면 절을 쓰지 않으므로 그 bytes는 command codec version 3까지의 InsertNode와 같습니다.
#[derive(Debug, Clone, PartialEq)]
pub struct InsertNode {
    /// 삽입할 node(pre-order, 첫 원소가 root).
    pub nodes: Vec<NodeRecord>,
    /// 삽입하는 node의 modifier stack.
    pub modifier_stacks: Vec<ModifierStack>,
    /// 삽입하는 node의 명시 layout record(소유 node StableId 순).
    pub layout_records: Vec<LayoutRecord>,
}

impl InsertNode {
    /// node 하나를 삽입합니다.
    pub fn single(record: NodeRecord) -> InsertNode {
        InsertNode {
            nodes: vec![record],
            modifier_stacks: Vec::new(),
            layout_records: Vec::new(),
        }
    }

    /// 삽입 root.
    pub fn root(&self) -> Option<&NodeRecord> {
        self.nodes.first()
    }

    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        let Some(root) = self.nodes.first() else {
            return Err(CommandError::InvalidRequest {
                reason: "삽입할 node가 없습니다",
            });
        };

        // 1. id: NIL 금지, 요청 안 유일, 문서 안 미사용.
        let mut new_ids: BTreeMap<StableId, NodeKind> = BTreeMap::new();
        let mut claimed: BTreeSet<StableId> = BTreeSet::new();
        let mut claim = |id: StableId| -> Result<(), CommandError> {
            if id == StableId::NIL {
                return Err(CommandError::NilId);
            }
            if model.contains_persistent_id(id) || !claimed.insert(id) {
                return Err(CommandError::DuplicateId { id });
            }
            Ok(())
        };
        for record in &self.nodes {
            claim(record.id)?;
            for element in path_element_ids(&record.payload) {
                claim(element)?;
            }
            new_ids.insert(record.id, record.kind());
        }
        for stack in &self.modifier_stacks {
            for entry in stack.entries() {
                claim(entry.id.stable_id())?;
            }
        }

        // 2. 부모: root는 문서의 container, 나머지는 앞선 레코드인 container.
        let parent_depth = match root.parent {
            None => {
                return Err(CommandError::InvalidParent {
                    node: root.id,
                    parent: None,
                })
            }
            Some(parent) => {
                let kind = kind_of(model, parent).ok_or(CommandError::InvalidParent {
                    node: root.id,
                    parent: Some(parent),
                })?;
                if !can_have_children(kind) {
                    return Err(CommandError::InvalidParent {
                        node: root.id,
                        parent: Some(parent),
                    });
                }
                depth_of(model, parent).ok_or(CommandError::UnknownNode { id: parent })?
            }
        };
        let mut depth: BTreeMap<StableId, u32> = BTreeMap::new();
        let root_depth = parent_depth + 1;
        if root_depth > MAX_NODE_DEPTH {
            return Err(CommandError::DepthExceeded {
                node: root.id,
                depth: root_depth,
            });
        }
        depth.insert(root.id, root_depth);
        for record in &self.nodes[1..] {
            let parent = record.parent.ok_or(CommandError::InvalidParent {
                node: record.id,
                parent: None,
            })?;
            let (Some(&parent_depth), Some(&parent_kind)) =
                (depth.get(&parent), new_ids.get(&parent))
            else {
                return Err(CommandError::InvalidParent {
                    node: record.id,
                    parent: Some(parent),
                });
            };
            if !can_have_children(parent_kind) {
                return Err(CommandError::InvalidParent {
                    node: record.id,
                    parent: Some(parent),
                });
            }
            let node_depth = parent_depth + 1;
            if node_depth > MAX_NODE_DEPTH {
                return Err(CommandError::DepthExceeded {
                    node: record.id,
                    depth: node_depth,
                });
            }
            depth.insert(record.id, node_depth);
        }

        // 3. 형제 OrderKey 유일: root는 문서의 기존 형제와, 나머지는 요청 안 형제와.
        if let Some(parent) = root.parent {
            if model.has_child_order_key(parent, &root.order_key) {
                return Err(CommandError::DuplicateOrderKey {
                    parent,
                    node: root.id,
                });
            }
        }
        let mut keys: BTreeSet<(StableId, &OrderKey)> = BTreeSet::new();
        for record in &self.nodes[1..] {
            let parent = record.parent.unwrap_or(StableId::NIL);
            if !keys.insert((parent, &record.order_key)) {
                return Err(CommandError::DuplicateOrderKey {
                    parent,
                    node: record.id,
                });
            }
        }

        // 4. 참조 종류: 대상이 있으면 기대 종류여야 합니다(없으면 미해결 진단, III.2.4).
        let owners = std::cell::OnceCell::new();
        let target_kind = |id: StableId| -> Option<IdOwner> {
            new_ids
                .get(&id)
                .map(|kind| IdOwner::Node(*kind))
                .or_else(|| owners.get_or_init(|| id_owners(model)).get(&id).copied())
        };
        for record in &self.nodes {
            if let NodePayload::Frame(frame) = &record.payload {
                if let Clipping::Path(target) = frame.clipping {
                    match target_kind(target.id()) {
                        None | Some(IdOwner::Node(NodeKind::Path)) => {}
                        Some(_) => {
                            return Err(CommandError::ReferenceTypeMismatch {
                                owner: record.id,
                                target: target.id(),
                            })
                        }
                    }
                }
            }
        }

        // 5. modifier stack: 소유 node는 이 요청의 node, Fragment 금지, 소유자당 하나.
        let mut stack_owners = BTreeSet::new();
        for stack in &self.modifier_stacks {
            let Some(&kind) = new_ids.get(&stack.node) else {
                return Err(CommandError::InvalidRequest {
                    reason: "modifier stack 소유 node가 삽입 대상이 아닙니다",
                });
            };
            if !stack_owners.insert(stack.node) {
                return Err(CommandError::InvalidRequest {
                    reason: "같은 node의 modifier stack이 둘 이상입니다",
                });
            }
            for entry in stack.entries() {
                validate_modifier_owner(kind, entry.kind).map_err(|error| {
                    CommandError::Modifier {
                        owner: stack.node,
                        error,
                    }
                })?;
                for target in entry.payload.referenced_nodes() {
                    match target_kind(target) {
                        None | Some(IdOwner::Node(_)) => {}
                        Some(_) => {
                            return Err(CommandError::ReferenceTypeMismatch {
                                owner: stack.node,
                                target,
                            })
                        }
                    }
                }
            }
        }
        if !self.modifier_stacks.is_empty() {
            let mut stacks = existing_stacks(model);
            stacks.extend(self.modifier_stacks.iter().cloned());
            if let Some(cycle) = find_modifier_cycle(&stacks) {
                return Err(CommandError::ModifierCycle(cycle));
            }
        }

        // 6. layout record: 소유 node는 이 요청의 node, 소유자당 하나, 빈 record 금지, node 종류에 붙는
        //    longhand만(문서 validator와 같은 검사, ADR-0086 결정 1). 목록은 소유 node 오름차순이어야 합니다
        //    (codec의 canonical 절과 같은 규칙. 어긋난 command는 history journal에 쓴 뒤 다시 읽을 수 없습니다).
        if !self.layout_records.is_empty() {
            if let Some(pair) = self
                .layout_records
                .windows(2)
                .find(|pair| pair[0].node >= pair[1].node)
            {
                return Err(CommandError::Model(ModelError::Layout {
                    id: pair[1].node,
                    reason: "layout record 소유 node가 오름차순이 아닙니다".to_string(),
                }));
            }
            let payloads: BTreeMap<StableId, &NodePayload> = self
                .nodes
                .iter()
                .map(|record| (record.id, &record.payload))
                .collect();
            if let Some(violation) =
                check_records(&self.layout_records, |id| payloads.get(&id).copied())
                    .into_iter()
                    .next()
            {
                return Err(CommandError::Model(ModelError::Layout {
                    id: violation_node(&violation),
                    reason: violation.to_string(),
                }));
            }
        }
        Ok(())
    }

    pub(crate) fn invert(&self) -> Result<DeleteSubtree, CommandError> {
        let root = self.root().ok_or(CommandError::InvalidRequest {
            reason: "삽입할 node가 없습니다",
        })?;
        Ok(DeleteSubtree { root: root.id })
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        for record in &self.nodes {
            let id = record.id;
            ctx.insert_node(record.clone())?;
            ctx.changes().inserted(id);
        }
        for stack in &self.modifier_stacks {
            if !stack.entries().is_empty() {
                ctx.set_modifier_stack(stack.clone())?;
            }
        }
        for record in &self.layout_records {
            ctx.set_layout_record(record.clone())?;
        }
        // 같은 StableId의 node를 이 session 안에서 지운 적이 있으면 모델에 그 record가 남아 있습니다(ADR-0086
        // 결정 5). 삽입한 node의 명시 값은 이 command가 정하므로 담지 않은 node의 남은 record는 지웁니다.
        for record in &self.nodes {
            if ctx.model().layout_record(record.id).is_some()
                && self
                    .layout_records
                    .binary_search_by_key(&record.id, |l| l.node)
                    .is_err()
            {
                ctx.set_layout_record(LayoutRecord::new(record.id))?;
            }
        }
        if let Some(parent) = self.root().and_then(|root| root.parent) {
            ctx.changes().structure(parent);
        }
        Ok(())
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.len(self.nodes.len());
        for record in &self.nodes {
            out.bytes(&encode_node_record(record));
        }
        out.len(self.modifier_stacks.len());
        for stack in &self.modifier_stacks {
            out.bytes(&stack.encode());
        }
        encode_layout_records(out, &self.layout_records);
    }

    pub(crate) fn decode_from(input: &mut Reader<'_>) -> Result<InsertNode, CommandCodecError> {
        let count = input.len()?;
        let mut nodes = Vec::new();
        for _ in 0..count {
            nodes.push(decode_node_record(input.bytes()?).map_err(|_| {
                CommandCodecError::Malformed {
                    reason: "잘못된 node 레코드",
                }
            })?);
        }
        let count = input.len()?;
        let mut modifier_stacks = Vec::new();
        for _ in 0..count {
            modifier_stacks.push(ModifierStack::decode(input.bytes()?).map_err(|_| {
                CommandCodecError::Malformed {
                    reason: "잘못된 modifier stack",
                }
            })?);
        }
        Ok(InsertNode {
            nodes,
            modifier_stacks,
            layout_records: decode_layout_records(input)?,
        })
    }
}

/// 위반한 record의 소유 node.
fn violation_node(violation: &LayoutRecordViolation) -> StableId {
    match violation {
        LayoutRecordViolation::OwnerMissing { node }
        | LayoutRecordViolation::Duplicate { node }
        | LayoutRecordViolation::Empty { node }
        | LayoutRecordViolation::NotApplicable { node, .. }
        | LayoutRecordViolation::SizeInPayload { node, .. } => *node,
    }
}

/// layout record 선택 절을 씁니다. record가 없으면 아무것도 쓰지 않습니다(command codec version 3까지와 같은
/// bytes).
pub(crate) fn encode_layout_records(out: &mut Writer, records: &[LayoutRecord]) {
    if records.is_empty() {
        return;
    }
    out.len(records.len());
    for record in records {
        out.bytes(&record.encode());
    }
}

/// layout record 선택 절을 읽습니다. 남은 입력이 없으면 record가 없습니다. 절이 있으면 record 수는 1 이상이고
/// 소유 node가 오름차순이어야 합니다(canonical 형식 하나).
pub(crate) fn decode_layout_records(
    input: &mut Reader<'_>,
) -> Result<Vec<LayoutRecord>, CommandCodecError> {
    if input.remaining() == 0 {
        return Ok(Vec::new());
    }
    let count = input.len()?;
    if count == 0 {
        return Err(CommandCodecError::Malformed {
            reason: "빈 layout record 절",
        });
    }
    let mut records: Vec<LayoutRecord> = Vec::new();
    for _ in 0..count {
        let record =
            LayoutRecord::decode(input.bytes()?).map_err(|_| CommandCodecError::Malformed {
                reason: "잘못된 layout record",
            })?;
        if records.last().is_some_and(|last| last.node >= record.node) {
            return Err(CommandCodecError::Malformed {
                reason: "layout record 소유 node가 오름차순이 아닙니다",
            });
        }
        records.push(record);
    }
    Ok(records)
}

/// 문서의 모든 modifier stack(소유 node StableId 순).
pub(crate) fn existing_stacks(model: &DocumentModel) -> Vec<ModifierStack> {
    crate::tree::node_ids(model)
        .into_iter()
        .filter_map(|id| model.modifier_stack(id).cloned())
        .collect()
}

/// node subtree 삭제 (VII.2.2).
///
/// root와 모든 자손을 지웁니다. 역 command는 삭제 전 subtree 레코드(pre-order)와 그
/// modifier stack을 담은 [`InsertNode`]라, subtree **안**의 참조(clip·modifier 참조)가 그대로
/// 보존·복원됩니다. subtree **밖**의 node가 subtree 안 node를 참조하면 삭제하지 않고
/// [`CommandError::ExternalReference`] 정책 오류를 반환합니다 — 참조를 먼저 정리해야 합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeleteSubtree {
    /// 지울 subtree root.
    pub root: StableId,
}

impl DeleteSubtree {
    /// 삭제 대상 subtree(pre-order)를 검증과 함께 구합니다.
    fn subtree(&self, model: &DocumentModel) -> Result<Vec<StableId>, CommandError> {
        match parent_of(model, self.root) {
            None => Err(CommandError::UnknownNode { id: self.root }),
            Some(None) => Err(CommandError::ViewRoot { node: self.root }),
            Some(Some(_)) => Ok(ChildIndex::build(model).subtree(self.root)),
        }
    }

    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        let subtree = self.subtree(model)?;
        let members: BTreeSet<StableId> = subtree.iter().copied().collect();
        let references = model.references();
        for target in &subtree {
            if let Some(from) = references
                .inbound(*target)
                .into_iter()
                .find(|from| !members.contains(from))
            {
                return Err(CommandError::ExternalReference {
                    target: *target,
                    from,
                });
            }
        }
        Ok(())
    }

    pub(crate) fn invert(&self, model: &DocumentModel) -> Result<InsertNode, CommandError> {
        let subtree = self.subtree(model)?;
        let mut nodes = Vec::with_capacity(subtree.len());
        let mut modifier_stacks = Vec::new();
        let mut layout_records = Vec::new();
        for id in subtree {
            nodes.push(model.record(id).ok_or(CommandError::UnknownNode { id })?);
            if let Some(stack) = model.modifier_stack(id) {
                modifier_stacks.push(stack.clone());
            }
            if let Some(record) = model.layout_record(id) {
                layout_records.push(record.clone());
            }
        }
        layout_records.sort_by_key(|record: &LayoutRecord| record.node);
        Ok(InsertNode {
            nodes,
            modifier_stacks,
            layout_records,
        })
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        let subtree = self.subtree(ctx.model())?;
        let parent = parent_of(ctx.model(), self.root).flatten();
        // 자손부터(역 pre-order) leaf로 지웁니다.
        for id in subtree.into_iter().rev() {
            ctx.remove_leaf(id)?;
            ctx.changes().removed(id);
        }
        if let Some(parent) = parent {
            ctx.changes().structure(parent);
        }
        Ok(())
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.id(self.root);
    }

    pub(crate) fn decode_from(input: &mut Reader<'_>) -> Result<DeleteSubtree, CommandCodecError> {
        Ok(DeleteSubtree { root: input.id()? })
    }
}

/// node 이동·재정렬 (VII.2.3).
///
/// 부모와 OrderKey를 **함께** 바꿉니다(같은 부모에 새 key면 재정렬). 자손과 modifier stack은
/// 함께 옮겨집니다. 다음을 적용 전에 거부합니다:
///
/// - View root 이동, 자기 자신·자손을 부모로 지정(cycle), 다른 View tree로의 이동
///   ([`validate_reparent`](stellar_document::tree_ops::validate_reparent), VI.2.3).
/// - 자식을 가질 수 없는 부모, 새 형제와 OrderKey 충돌, 최대 깊이 초과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveNode {
    /// 옮길 node.
    pub node: StableId,
    /// 새 부모.
    pub new_parent: StableId,
    /// 새 형제 순서 key.
    pub order_key: OrderKey,
}

impl MoveNode {
    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        let node_handle = model
            .runtime_id(self.node)
            .ok_or(CommandError::UnknownNode { id: self.node })?;
        let parent_handle =
            model
                .runtime_id(self.new_parent)
                .ok_or(CommandError::InvalidParent {
                    node: self.node,
                    parent: Some(self.new_parent),
                })?;
        if parent_of(model, self.node) == Some(None) {
            return Err(CommandError::ViewRoot { node: self.node });
        }
        if model.columns().parent(node_handle) == Some(parent_handle) {
            if crate::tree::order_key_of(model, self.node).as_ref() != Some(&self.order_key)
                && model.has_child_order_key(self.new_parent, &self.order_key)
            {
                return Err(CommandError::DuplicateOrderKey {
                    parent: self.new_parent,
                    node: self.node,
                });
            }
            return Ok(());
        }
        validate_reparent(model.columns(), node_handle, parent_handle).map_err(|error| {
            CommandError::Reparent {
                node: self.node,
                error,
            }
        })?;
        let parent_kind = kind_of(model, self.new_parent).ok_or(CommandError::UnknownNode {
            id: self.new_parent,
        })?;
        if !can_have_children(parent_kind) {
            return Err(CommandError::InvalidParent {
                node: self.node,
                parent: Some(self.new_parent),
            });
        }
        let index = ChildIndex::build(model);
        for sibling in index.children(self.new_parent) {
            if *sibling != self.node
                && crate::tree::order_key_of(model, *sibling).as_ref() == Some(&self.order_key)
            {
                return Err(CommandError::DuplicateOrderKey {
                    parent: self.new_parent,
                    node: self.node,
                });
            }
        }
        // 깊이: 새 부모 깊이 + 1 + subtree 높이.
        let parent_depth = depth_of(model, self.new_parent).ok_or(CommandError::UnknownNode {
            id: self.new_parent,
        })?;
        let height = subtree_height(&index, self.node);
        let deepest = parent_depth + 1 + height;
        if deepest > MAX_NODE_DEPTH {
            return Err(CommandError::DepthExceeded {
                node: self.node,
                depth: deepest,
            });
        }
        Ok(())
    }

    pub(crate) fn invert(&self, model: &DocumentModel) -> Result<MoveNode, CommandError> {
        let parent = parent_of(model, self.node)
            .ok_or(CommandError::UnknownNode { id: self.node })?
            .ok_or(CommandError::ViewRoot { node: self.node })?;
        let order_key = crate::tree::order_key_of(model, self.node)
            .ok_or(CommandError::UnknownNode { id: self.node })?;
        Ok(MoveNode {
            node: self.node,
            new_parent: parent,
            order_key,
        })
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        if parent_of(ctx.model(), self.node) == Some(Some(self.new_parent)) {
            if crate::tree::order_key_of(ctx.model(), self.node).as_ref() != Some(&self.order_key) {
                ctx.replace_child_order(self.node, self.order_key.clone())?;
                ctx.changes().node_property(
                    self.node,
                    node_keys::ORDER_KEY,
                    STRUCTURE_INVALIDATION,
                );
                ctx.changes().structure(self.new_parent);
            }
            return Ok(());
        }
        let mut record = ctx
            .model()
            .record(self.node)
            .ok_or(CommandError::UnknownNode { id: self.node })?;
        let old_parent = record.parent;
        record.parent = Some(self.new_parent);
        record.order_key = self.order_key.clone();
        ctx.replace_record(record)?;
        if old_parent != Some(self.new_parent) {
            ctx.changes()
                .node_property(self.node, node_keys::PARENT, STRUCTURE_INVALIDATION);
        }
        ctx.changes()
            .node_property(self.node, node_keys::ORDER_KEY, STRUCTURE_INVALIDATION);
        if let Some(old_parent) = old_parent {
            ctx.changes().structure(old_parent);
        }
        ctx.changes().structure(self.new_parent);
        Ok(())
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.id(self.node);
        out.id(self.new_parent);
        out.bytes(self.order_key.as_bytes());
    }

    pub(crate) fn decode_from(input: &mut Reader<'_>) -> Result<MoveNode, CommandCodecError> {
        let node = input.id()?;
        let new_parent = input.id()?;
        let order_key = decode_order_key(input)?;
        Ok(MoveNode {
            node,
            new_parent,
            order_key,
        })
    }
}

/// `u32` 길이 + bytes로 기록된 OrderKey(정규형 검증).
pub(crate) fn decode_order_key(input: &mut Reader<'_>) -> Result<OrderKey, CommandCodecError> {
    OrderKey::from_bytes(input.bytes()?.to_vec()).map_err(|_| CommandCodecError::Malformed {
        reason: "잘못된 OrderKey",
    })
}

/// `root` 아래 가장 깊은 자손까지의 거리(leaf = 0).
fn subtree_height(index: &ChildIndex, root: StableId) -> u32 {
    let mut height = 0u32;
    let mut stack = vec![(root, 0u32)];
    while let Some((id, depth)) = stack.pop() {
        height = height.max(depth);
        for child in index.children(id) {
            stack.push((*child, depth.saturating_add(1)));
        }
    }
    height
}

// --- 인스턴스 교체 (XII.10.35, ADR-0079) ---

/// 인스턴스의 참조 컴포넌트를 바꿉니다(swap). 새 payload는 [`plan_instance_swap`]이 만듭니다.
///
/// command는 결과 payload(새 `ComponentRef`와 보존한 override)를 그대로 담아 컴포넌트 library 없이 재실행할 수
/// 있습니다. 역은 원래 payload로 되돌리는 같은 command입니다(원래 `ComponentRef`·override 복원).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwapInstance {
    /// 대상 ComponentInstance node.
    pub node: StableId,
    /// 교체 뒤 payload.
    pub payload: ComponentInstancePayload,
}

impl SwapInstance {
    fn current<'a>(
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
    /// 새 정의 root의 기본 크기(기본).
    #[default]
    DefinitionDefault,
    /// 지금 보이는 크기 유지. 인스턴스 크기 override가 문서 모델에 아직 없어 계획이 거부합니다(ADR-0079).
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceSwapPlan {
    /// 적용할 command.
    pub command: SwapInstance,
    /// 보존한 Property override(옛 Property id 순).
    pub preserved: Vec<PreservedOverride>,
    /// 보존하지 못한 override(key 순).
    pub dropped: Vec<(SwapOverride, SwapDropReason)>,
}

/// 인스턴스 `node`를 `target` 컴포넌트로 바꾸는 계획을 만듭니다(문서는 바꾸지 않음).
///
/// 직접 Property override는 옛 Property와 **이름과 선언 타입이 같은 새 정의의 공개 Property**로 옮깁니다.
/// nested 경로 override, 맞는 Property가 없는 override, 새 정의에 같은 id의 Slot이 없거나 content가 새
/// Slot의 종류·개수 규칙을 어기는 Slot override는 버리고 `dropped`에 이유와 함께 보고합니다. 버릴 Slot에
/// content(인스턴스 소유 node)가 있으면 투영할 곳 없이 남게 되므로 계획을 거부합니다(IX.3.2). 같은 입력은 같은
/// 계획입니다(결과는 key 순).
pub fn plan_instance_swap<T: PartialEq>(
    model: &DocumentModel,
    node: StableId,
    target: ComponentRef,
    current_properties: &[SwapProperty<T>],
    target_properties: &[SwapProperty<T>],
    target_slots: &[ComponentSlot],
    sizing: SwapSizing,
) -> Result<InstanceSwapPlan, CommandError> {
    if sizing == SwapSizing::KeepCurrent {
        return Err(CommandError::InvalidRequest {
            reason: "인스턴스 크기 override가 문서 모델에 없어 현재 크기를 유지할 수 없습니다",
        });
    }
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
    Ok(InstanceSwapPlan {
        command: SwapInstance { node, payload },
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
    use stellar_core_types::reference::NodeRef;
    use stellar_core_types::value::Vec2;
    use stellar_document::frame::FramePayload;
    use stellar_document::modifier::{ModifierEntry, ModifierId, ModifierKind, ModifierPayload};
    use stellar_document::path::{IdentifiedPath, PathCommand, PathData, WindingRule};
    use stellar_document::test_support::semantic_snapshot::model_snapshot_json;
    use stellar_document::text::TextPayload;

    fn snapshot(state: &DocumentState) -> String {
        model_snapshot_json(state.model()).expect("snapshot")
    }

    fn run(state: &mut DocumentState, command: Command) -> Result<Command, CommandError> {
        let request = CommandRequest::at(state, command);
        state.execute(&request).map(|outcome| outcome.inverse)
    }

    fn insert(record: NodeRecord) -> Command {
        Command::InsertNode(InsertNode::single(record))
    }

    #[test]
    #[ignore = "release에서 1천·1만·5만 leaf 문서의 단일 Frame 삽입 시간 측정"]
    fn measure_insert_single_frame() {
        for leaf_count in [1_000u64, 10_000, 50_000] {
            let build_started = std::time::Instant::now();
            let mut model = DocumentModel::new(crate::command::test_fixtures::definition());
            model.insert_node(frame(10, None, 1)).unwrap();
            let leaves_per_group = leaf_count / 100;
            let keys = OrderKey::evenly_spaced(leaves_per_group as usize + 2);
            for group in 0..100 {
                let parent = 100 + group;
                model
                    .insert_node(frame(parent, Some(10), group as u8 + 1))
                    .unwrap();
                for offset in 0..leaves_per_group {
                    let mut leaf =
                        frame(1_000 + group * leaves_per_group + offset, Some(parent), 1);
                    leaf.order_key = keys[offset as usize].clone();
                    model.insert_node(leaf).unwrap();
                }
            }
            let mut state = DocumentState::new(model);
            let build_elapsed = build_started.elapsed();
            let mut samples = Vec::with_capacity(101);
            for sequence in 0..101 {
                let group = sequence % 100;
                let mut added = frame(100_000 + sequence, Some(100 + group), 1);
                added.order_key = keys[(leaves_per_group + sequence / 100) as usize].clone();
                let request = CommandRequest::at(&state, insert(added));
                let start = std::time::Instant::now();
                let result = state.execute(&request).unwrap();
                samples.push(start.elapsed());
                assert!(matches!(result.inverse, Command::DeleteSubtree(_)));
            }
            samples.sort_unstable();
            assert_eq!(state.model().node_count(), leaf_count as usize + 202);
            println!("insert execute, {leaf_count} leaves, setup {build_elapsed:?}, 101 samples: p50 {:?}, p95 {:?}, p99 {:?}",
                samples[50], samples[95], samples[99]);
        }
    }

    #[test]
    fn insert_indexes_match_full_rebuilds_after_command_sequences() {
        use crate::test_support::{random_valid_request, FreshIds};
        use stellar_document::test_support::random_document::{random_valid_document, SplitMix64};

        let mut applied = 0;
        for seed in 0..16 {
            let mut state = DocumentState::new(random_valid_document(seed));
            let mut rng = SplitMix64::new(seed);
            let mut fresh = FreshIds::new(seed);
            let mut seen_ids = BTreeSet::new();
            let mut seen_keys = BTreeSet::new();
            let mut assert_indexes = |model: &DocumentModel| {
                let owners = id_owners(model);
                seen_ids.extend(owners.keys().copied());
                for &id in &seen_ids {
                    assert_eq!(model.contains_persistent_id(id), owners.contains_key(&id));
                }
                let rebuilt = ChildIndex::build(model);
                let mut keys = BTreeSet::new();
                for id in crate::tree::node_ids(model) {
                    assert_eq!(model.children_in_order(id), rebuilt.children(id));
                    if let Some(Some(parent)) = parent_of(model, id) {
                        keys.insert((parent, crate::tree::order_key_of(model, id).unwrap()));
                    }
                }
                seen_keys.extend(keys.iter().cloned());
                for (parent, key) in &seen_keys {
                    assert_eq!(
                        model.has_child_order_key(*parent, key),
                        keys.contains(&(*parent, key.clone()))
                    );
                }
            };
            assert_indexes(state.model());
            for step in 0..32 {
                let Some(request) = random_valid_request(&state, &mut rng, &mut fresh, 32) else {
                    continue;
                };
                applied += 1;
                let outcome = state.execute(&request).unwrap();
                assert_indexes(state.model());
                if step % 4 == 0 {
                    run(&mut state, outcome.inverse).unwrap();
                    assert_indexes(state.model());
                }
            }
        }
        assert!(applied >= 300, "충분한 command 경로 검사: {applied}");
    }

    #[test]
    fn direct_records_keep_duplicate_order_keys_and_non_node_ids_until_the_last_owner_leaves() {
        // 직접 모델 API는 전체 문서 validator 이전에 일시적으로 겹치는 ID·순서 키를 허용합니다.
        let mut model = small_model();
        let first = path_record(20, 11, 0x30);
        let mut second = first.clone();
        second.id = sid(21);
        let key = first.order_key.clone();
        model.insert_node(first).unwrap();
        model.insert_node(second).unwrap();
        for owner in [20, 21] {
            let mut stack = ModifierStack::new(sid(owner));
            stack
                .push_back(ModifierId::new(sid(2001)), ModifierKind::Opacity)
                .unwrap();
            model.set_modifier_stack(stack).unwrap();
        }
        assert!(model.has_child_order_key(sid(11), &key));
        assert!(model.contains_persistent_id(sid(2001)));
        model.remove_leaf(sid(20)).unwrap();
        assert!(model.has_child_order_key(sid(11), &key));
        assert!(model.contains_persistent_id(sid(2001)));
        model
            .set_payload(sid(21), path_record(22, 11, 0x30).payload)
            .unwrap();
        assert!(model.contains_persistent_id(sid(2001)));
        model
            .set_modifier_stack(ModifierStack::new(sid(21)))
            .unwrap();
        assert!(!model.contains_persistent_id(sid(2001)));
        assert!(model.contains_persistent_id(sid(2201)));
        model.remove_leaf(sid(21)).unwrap();
        assert!(!model.has_child_order_key(sid(11), &key));
        assert!(!model.contains_persistent_id(sid(2201)));
    }

    #[test]
    fn failed_subtree_insert_rolls_back_indexes_and_can_retry_after_reload() {
        let nodes = vec![frame(20, Some(11), 0x30), path_record(21, 20, 0x10)];
        let modifier_stacks = [(20, 60), (21, 61)]
            .map(|(owner, modifier)| {
                let mut stack = ModifierStack::new(sid(owner));
                stack
                    .push_back(ModifierId::new(sid(modifier)), ModifierKind::Opacity)
                    .unwrap();
                stack
            })
            .to_vec();
        let command = Command::InsertNode(InsertNode {
            nodes: nodes.clone(),
            modifier_stacks,
            layout_records: Vec::new(),
        });
        let mut added_ids = vec![sid(20), sid(21), sid(60), sid(61)];
        added_ids.extend(path_element_ids(&nodes[1].payload));
        for fault_at in 0..4 {
            let mut model = small_model();
            let before = model.to_records().unwrap();
            command.validate(&model).unwrap();
            let mut guard = crate::guard::TransactionGuard::begin(&mut model);
            guard.ctx().inject_fault_after(fault_at);
            assert!(command.apply(guard.ctx()).is_err());
            guard.rollback().unwrap();
            assert_eq!(model.to_records().unwrap(), before);
            for &id in &added_ids {
                assert!(
                    !model.contains_persistent_id(id),
                    "fault {fault_at}: {id:?}"
                );
            }
            for node in &nodes {
                assert!(!model.has_child_order_key(node.parent.unwrap(), &node.order_key));
            }
            // 같은 요청의 재시도와 저장 레코드에서의 재구성이 동일한 인덱스를 만듭니다.
            let mut state = DocumentState::new(model);
            run(&mut state, command.clone()).unwrap();
            let (reopened, _) = DocumentModel::from_records(state.model().to_records().unwrap())
                .expect("삽입한 subtree와 modifier를 다시 열기");
            for &id in &added_ids {
                assert!(reopened.contains_persistent_id(id));
            }
            for node in &nodes {
                let parent = node.parent.unwrap();
                assert!(reopened.has_child_order_key(parent, &node.order_key));
                assert_eq!(
                    reopened.children_in_order(parent),
                    state.model().children_in_order(parent)
                );
            }
        }
    }

    #[test]
    fn insert_apply_inverse_round_trip() {
        // 검증 기준(VII.2.1): apply/inverse round-trip.
        let mut state = DocumentState::new(small_model());
        let before = snapshot(&state);
        let mut record = frame(20, Some(11), 0x30);
        record.display_name = "새 frame".into();
        let mut stack = ModifierStack::new(sid(20));
        stack
            .push_back(ModifierId::new(sid(60)), ModifierKind::Opacity)
            .unwrap();
        let mut child = frame(21, Some(20), 0x10);
        child.payload = NodePayload::Text(TextPayload::empty(0));
        let command = Command::InsertNode(InsertNode {
            nodes: vec![record.clone(), child],
            modifier_stacks: vec![stack],
            layout_records: Vec::new(),
        });
        let inverse = run(&mut state, command.clone()).unwrap();
        assert_eq!(
            inverse,
            Command::DeleteSubtree(DeleteSubtree { root: sid(20) })
        );
        assert_eq!(state.model().record(sid(20)).unwrap(), record);
        assert_eq!(state.model().children_in_order(sid(20)), vec![sid(21)]);
        assert!(state.model().modifier_stack(sid(20)).is_some());

        // subtree 삭제는 VII.2.2 범위라 자식부터 지워 원복합니다.
        run(
            &mut state,
            Command::DeleteSubtree(DeleteSubtree { root: sid(21) }),
        )
        .unwrap();
        let reinsert = run(&mut state, inverse).unwrap();
        assert_eq!(snapshot(&state), before);
        // 삭제의 역은 원래 레코드·stack을 다시 넣습니다.
        let Command::InsertNode(restored) = &reinsert else {
            panic!("InsertNode여야 함");
        };
        assert_eq!(restored.nodes, vec![record]);
        assert_eq!(restored.modifier_stacks.len(), 1);
        run(&mut state, reinsert).unwrap();
        assert!(state.model().modifier_stack(sid(20)).is_some());
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let mut state = DocumentState::new(small_model());
        let before = snapshot(&state);
        // 기존 node·문서 정의 id·요청 안 중복.
        for id in [11, 1, 3] {
            let mut record = frame(id, Some(10), 0x90);
            record.id = sid(id);
            assert_eq!(
                run(&mut state, insert(record)),
                Err(CommandError::DuplicateId { id: sid(id) })
            );
        }
        let dup = Command::InsertNode(InsertNode {
            nodes: vec![frame(20, Some(10), 0x90), frame(20, Some(20), 0x10)],
            modifier_stacks: Vec::new(),
            layout_records: Vec::new(),
        });
        assert_eq!(
            run(&mut state, dup),
            Err(CommandError::DuplicateId { id: sid(20) })
        );
        let mut nil = frame(20, Some(10), 0x90);
        nil.id = StableId::NIL;
        assert_eq!(run(&mut state, insert(nil)), Err(CommandError::NilId));
        // path point id가 기존 node id와 겹침.
        let data = PathData::new(
            WindingRule::NonZero,
            vec![PathCommand::MoveTo(Vec2::new(0.0, 0.0).unwrap())],
        )
        .unwrap();
        let mut ids = [sid(12)].into_iter();
        let path = IdentifiedPath::assign(data, &mut || ids.next().unwrap_or(StableId::NIL));
        let mut record = frame(20, Some(10), 0x90);
        record.payload = NodePayload::Path(path);
        assert_eq!(
            run(&mut state, insert(record)),
            Err(CommandError::DuplicateId { id: sid(12) })
        );
        assert_eq!(snapshot(&state), before);
    }

    #[test]
    fn invalid_parent_is_rejected() {
        let mut state = DocumentState::new(small_model());
        let mut text = frame(30, Some(10), 0x90);
        text.payload = NodePayload::Text(TextPayload::empty(0));
        run(&mut state, insert(text)).unwrap();
        let before = snapshot(&state);
        let cases = [
            (frame(20, None, 0x90), None),
            (frame(20, Some(99), 0x90), Some(sid(99))),
            // Text는 자식을 가질 수 없습니다.
            (frame(20, Some(30), 0x10), Some(sid(30))),
        ];
        for (record, parent) in cases {
            assert_eq!(
                run(&mut state, insert(record)),
                Err(CommandError::InvalidParent {
                    node: sid(20),
                    parent
                })
            );
        }
        // subtree 안 자손의 부모가 앞선 레코드가 아님.
        let orphan = Command::InsertNode(InsertNode {
            nodes: vec![frame(20, Some(10), 0x90), frame(21, Some(12), 0x10)],
            modifier_stacks: Vec::new(),
            layout_records: Vec::new(),
        });
        assert_eq!(
            run(&mut state, orphan),
            Err(CommandError::InvalidParent {
                node: sid(21),
                parent: Some(sid(12))
            })
        );
        let empty = Command::InsertNode(InsertNode {
            nodes: Vec::new(),
            modifier_stacks: Vec::new(),
            layout_records: Vec::new(),
        });
        assert!(matches!(
            run(&mut state, empty),
            Err(CommandError::InvalidRequest { .. })
        ));
        assert_eq!(snapshot(&state), before);
    }

    #[test]
    fn invalid_order_key_and_depth_are_rejected() {
        let mut state = DocumentState::new(small_model());
        let before = snapshot(&state);
        // 11의 key 0x10과 충돌.
        assert_eq!(
            run(&mut state, insert(frame(20, Some(10), 0x10))),
            Err(CommandError::DuplicateOrderKey {
                parent: sid(10),
                node: sid(20)
            })
        );
        let siblings = Command::InsertNode(InsertNode {
            nodes: vec![
                frame(20, Some(10), 0x90),
                frame(21, Some(20), 0x10),
                frame(22, Some(20), 0x10),
            ],
            modifier_stacks: Vec::new(),
            layout_records: Vec::new(),
        });
        assert_eq!(
            run(&mut state, siblings),
            Err(CommandError::DuplicateOrderKey {
                parent: sid(20),
                node: sid(22)
            })
        );
        // depth 64 초과: 11(depth 1) 아래 64단 chain.
        let mut chain = Vec::new();
        for i in 0..64u64 {
            let parent = if i == 0 { 11 } else { 99 + i };
            chain.push(frame(100 + i, Some(parent), 0x40));
        }
        let deep = Command::InsertNode(InsertNode {
            nodes: chain,
            modifier_stacks: Vec::new(),
            layout_records: Vec::new(),
        });
        assert_eq!(
            run(&mut state, deep),
            Err(CommandError::DepthExceeded {
                node: sid(163),
                depth: 65
            })
        );
        assert_eq!(snapshot(&state), before);
    }

    #[test]
    fn references_and_modifier_rules_are_checked() {
        let mut state = DocumentState::new(small_model());
        let before = snapshot(&state);
        // Frame clip이 Path가 아닌 node(12)를 가리킴.
        let mut clipped = frame(20, Some(10), 0x90);
        clipped.payload = NodePayload::Frame(FramePayload {
            clipping: Clipping::Path(NodeRef::new(sid(12))),
            ..FramePayload::default()
        });
        assert_eq!(
            run(&mut state, insert(clipped)),
            Err(CommandError::ReferenceTypeMismatch {
                owner: sid(20),
                target: sid(12)
            })
        );
        // Fragment에 modifier.
        let mut fragment = frame(20, Some(10), 0x90);
        fragment.payload = NodePayload::Fragment(stellar_document::fragment::FragmentPayload);
        let mut stack = ModifierStack::new(sid(20));
        stack
            .push_back(ModifierId::new(sid(61)), ModifierKind::Blur)
            .unwrap();
        assert!(matches!(
            run(
                &mut state,
                Command::InsertNode(InsertNode {
                    nodes: vec![fragment],
                    modifier_stacks: vec![stack],
                    layout_records: Vec::new(),
                })
            ),
            Err(CommandError::Modifier { .. })
        ));
        // modifier 참조가 문서의 View id(정의)를 가리킴.
        let mut stack = ModifierStack::new(sid(20));
        stack
            .insert(ModifierEntry {
                id: ModifierId::new(sid(62)),
                order: OrderKey::between(None, None),
                kind: ModifierKind::Mask,
                enabled: true,
                payload: ModifierPayload::Mask {
                    source: NodeRef::new(sid(3)),
                },
            })
            .unwrap();
        assert_eq!(
            run(
                &mut state,
                Command::InsertNode(InsertNode {
                    nodes: vec![frame(20, Some(10), 0x90)],
                    modifier_stacks: vec![stack],
                    layout_records: Vec::new(),
                })
            ),
            Err(CommandError::ReferenceTypeMismatch {
                owner: sid(20),
                target: sid(3)
            })
        );
        // 자기 자신을 mask로 쓰면 cycle.
        let mut stack = ModifierStack::new(sid(20));
        stack
            .insert(ModifierEntry {
                id: ModifierId::new(sid(63)),
                order: OrderKey::between(None, None),
                kind: ModifierKind::Mask,
                enabled: true,
                payload: ModifierPayload::Mask {
                    source: NodeRef::new(sid(20)),
                },
            })
            .unwrap();
        assert!(matches!(
            run(
                &mut state,
                Command::InsertNode(InsertNode {
                    nodes: vec![frame(20, Some(10), 0x90)],
                    modifier_stacks: vec![stack],
                    layout_records: Vec::new(),
                })
            ),
            Err(CommandError::ModifierCycle(_))
        ));
        assert_eq!(snapshot(&state), before);
    }

    #[test]
    fn delete_rules_and_codec_round_trip() {
        let mut state = DocumentState::new(small_model());
        assert_eq!(
            run(
                &mut state,
                Command::DeleteSubtree(DeleteSubtree { root: sid(10) })
            ),
            Err(CommandError::ViewRoot { node: sid(10) })
        );
        assert_eq!(
            run(
                &mut state,
                Command::DeleteSubtree(DeleteSubtree { root: sid(99) })
            ),
            Err(CommandError::UnknownNode { id: sid(99) })
        );
        let mut stack = ModifierStack::new(sid(20));
        stack
            .push_back(ModifierId::new(sid(60)), ModifierKind::Opacity)
            .unwrap();
        let command = Command::InsertNode(InsertNode {
            nodes: vec![frame(20, Some(11), 0x30), frame(21, Some(20), 0x10)],
            modifier_stacks: vec![stack],
            layout_records: Vec::new(),
        });
        let bytes = encode_command(&command);
        assert_eq!(&bytes[..2], &[0x01, 0x00]);
        assert_eq!(decode_command(&bytes).unwrap(), command);
        let delete = Command::DeleteSubtree(DeleteSubtree { root: sid(20) });
        let bytes = encode_command(&delete);
        assert_eq!(&bytes[..2], &[0x02, 0x00]);
        assert_eq!(decode_command(&bytes).unwrap(), delete);
    }

    fn path_record(id: u64, parent: u64, key: u8) -> NodeRecord {
        let data = PathData::new(
            WindingRule::NonZero,
            vec![
                PathCommand::MoveTo(Vec2::new(0.0, 0.0).unwrap()),
                PathCommand::LineTo(Vec2::new(4.0, 2.0).unwrap()),
            ],
        )
        .unwrap();
        let mut next = id * 100;
        let path = IdentifiedPath::assign(data, &mut || {
            next += 1;
            sid(next)
        });
        let mut record = frame(id, Some(parent), key);
        record.payload = NodePayload::Path(path);
        record
    }

    /// root(10) → 12 아래 Frame(20)과 그 자식 `n`개를 가진 상태.
    fn wide_state(n: u64) -> DocumentState {
        let mut model = small_model();
        model.insert_node(frame(20, Some(12), 0x30)).unwrap();
        for i in 0..n {
            let mut child = frame(1_000 + i, Some(20), 0x01);
            child.order_key = OrderKey::from_bytes(vec![(i >> 8) as u8, i as u8, 1]).unwrap();
            model.insert_node(child).unwrap();
        }
        DocumentState::new(model)
    }

    #[test]
    fn delete_subtree_and_undo_work_is_linear_in_subtree_size() {
        // posthoc CMD-DELETE-QUADRATIC 회귀: node마다 `remove_leaf`가 전체 column을 훑어 자식을 확인해
        // 삭제 비용이 O(subtree × N)였습니다(자식 1.6만 개 삭제 8.7초). 작업량은 문서 전체 순회에서
        // 방문한 live node 수(`op_count`)입니다 — 순회 횟수가 상수여야 선형입니다.
        let work = |n: u64| {
            let mut state = wide_state(n);
            let before = snapshot(&state);
            let delete = Command::DeleteSubtree(DeleteSubtree { root: sid(20) });
            let (inverse, deleted) =
                stellar_document::op_count::measure(|| run(&mut state, delete));
            assert_eq!(state.model().node_count(), 4);
            // 되돌리기(InsertNode, 실패 시 rollback의 `UndoOp::Remove` 경로와 같은 primitive)도 선형.
            let (restored, undo) = stellar_document::op_count::measure(|| {
                run(&mut state, inverse.expect("삭제 성공"))
            });
            restored.expect("되돌리기 성공");
            assert_eq!(snapshot(&state), before);
            (deleted, undo)
        };
        let (small, large) = (work(500), work(2_000));
        let nodes = 2_000 + 5;
        for (name, small, large) in [("삭제", small.0, large.0), ("되돌리기", small.1, large.1)]
        {
            assert!(
                large <= 16 * nodes,
                "{name}: node 하나당 상수 번만 순회해야 합니다: 작업량 {large}"
            );
            assert!(
                large <= 5 * small,
                "{name}: 입력 4배에 작업량 {small} → {large}(선형이면 4배 남짓, 제곱이면 16배)"
            );
        }
    }

    #[test]
    fn delete_subtree_preserves_internal_references_and_rejects_external() {
        // 검증 기준(VII.2.2): delete→undo semantic snapshot 동일.
        let mut state = DocumentState::new(small_model());
        let mut clipped = frame(22, Some(20), 0x20);
        clipped.payload = NodePayload::Frame(FramePayload {
            clipping: Clipping::Path(NodeRef::new(sid(21))),
            ..FramePayload::default()
        });
        let mut stack = ModifierStack::new(sid(22));
        stack
            .insert(ModifierEntry {
                id: ModifierId::new(sid(70)),
                order: OrderKey::between(None, None),
                kind: ModifierKind::Mask,
                enabled: false,
                payload: ModifierPayload::Mask {
                    source: NodeRef::new(sid(21)),
                },
            })
            .unwrap();
        run(
            &mut state,
            Command::InsertNode(InsertNode {
                nodes: vec![
                    frame(20, Some(10), 0x90),
                    path_record(21, 20, 0x10),
                    clipped,
                ],
                modifier_stacks: vec![stack],
                layout_records: Vec::new(),
            }),
        )
        .unwrap();
        let before = snapshot(&state);

        // 21만 지우면 subtree 밖 22가 참조 → 정책 오류, 문서 불변.
        assert_eq!(
            run(
                &mut state,
                Command::DeleteSubtree(DeleteSubtree { root: sid(21) })
            ),
            Err(CommandError::ExternalReference {
                target: sid(21),
                from: sid(22)
            })
        );
        assert_eq!(snapshot(&state), before);

        // 참조가 모두 subtree 안이면 삭제되고, undo가 내부 참조까지 복원합니다.
        let request = CommandRequest::at(
            &state,
            Command::DeleteSubtree(DeleteSubtree { root: sid(20) }),
        );
        let outcome = state.execute(&request).unwrap();
        assert_eq!(
            outcome.changes.removed_nodes(),
            vec![sid(20), sid(21), sid(22)]
        );
        assert_eq!(outcome.changes.changed_nodes(), vec![sid(10)]);
        for id in [20, 21, 22] {
            assert!(!state.model().contains(sid(id)));
        }
        let Command::InsertNode(restore) = &outcome.inverse else {
            panic!("InsertNode여야 함");
        };
        assert_eq!(
            restore.nodes.iter().map(|n| n.id).collect::<Vec<_>>(),
            vec![sid(20), sid(21), sid(22)],
            "pre-order"
        );
        assert_eq!(restore.modifier_stacks.len(), 1);
        run(&mut state, outcome.inverse).unwrap();
        assert_eq!(snapshot(&state), before);
    }

    #[test]
    fn delete_undo_round_trips_on_random_documents() {
        use stellar_document::test_support::random_document::random_valid_document;
        let (mut deleted, mut rejected) = (0, 0);
        for seed in 0..150u64 {
            let mut state = DocumentState::new(random_valid_document(seed));
            let before = snapshot(&state);
            let before_records = state.model().to_records().unwrap();
            let candidates: Vec<StableId> = crate::tree::node_ids(state.model())
                .into_iter()
                .filter(|id| parent_of(state.model(), *id).flatten().is_some())
                .collect();
            for target in candidates.into_iter().step_by(3) {
                if !state.model().contains(target) {
                    continue;
                }
                let subtree = ChildIndex::build(state.model()).subtree(target);
                let request = CommandRequest::at(
                    &state,
                    Command::DeleteSubtree(DeleteSubtree { root: target }),
                );
                match state.execute(&request) {
                    Ok(outcome) => {
                        deleted += 1;
                        let mut expected = subtree.clone();
                        expected.sort();
                        assert_eq!(outcome.changes.removed_nodes(), expected);
                        run(&mut state, outcome.inverse).unwrap();
                        // 레코드가 같으면 semantic snapshot도 같습니다(snapshot은 레코드의 함수).
                        assert_eq!(
                            state.model().to_records().unwrap(),
                            before_records,
                            "seed {seed} {target}"
                        );
                    }
                    Err(CommandError::ExternalReference { target: t, from }) => {
                        rejected += 1;
                        assert!(subtree.contains(&t) && !subtree.contains(&from));
                        assert_eq!(state.model().to_records().unwrap(), before_records);
                    }
                    Err(other) => panic!("seed {seed}: {other}"),
                }
            }
            assert_eq!(snapshot(&state), before, "seed {seed}");
        }
        assert!(deleted > 300, "삭제 {deleted}");
        assert!(rejected > 20, "정책 거부 {rejected}");
    }

    fn move_to(node: u64, parent: u64, key: u8) -> Command {
        Command::MoveNode(MoveNode {
            node: sid(node),
            new_parent: sid(parent),
            order_key: OrderKey::from_bytes(vec![key]).unwrap(),
        })
    }

    #[test]
    fn move_and_reorder_round_trip() {
        // 검증 기준(VII.2.3): move→undo, reorder fixture 통과.
        let mut state = DocumentState::new(small_model());
        let before = snapshot(&state);
        // 13(11의 자식)을 12 아래로.
        let request = CommandRequest::at(&state, move_to(13, 12, 0x10));
        let outcome = state.execute(&request).unwrap();
        assert_eq!(state.model().children_in_order(sid(12)), vec![sid(13)]);
        assert!(state.model().children_in_order(sid(11)).is_empty());
        assert_eq!(
            outcome.changes.changed_properties(),
            vec![
                stellar_core_types::property::PropertyPath::with_keys(
                    sid(13),
                    vec![node_keys::PARENT]
                ),
                stellar_core_types::property::PropertyPath::with_keys(
                    sid(13),
                    vec![node_keys::ORDER_KEY]
                ),
            ]
        );
        assert_eq!(
            outcome.changes.changed_nodes(),
            vec![sid(11), sid(12), sid(13)]
        );
        assert_eq!(outcome.inverse, move_to(13, 11, 0x10));
        run(&mut state, outcome.inverse).unwrap();
        assert_eq!(snapshot(&state), before);

        // 재정렬: 같은 부모 안에서 12를 11 앞으로.
        let handles =
            [sid(10), sid(11), sid(12), sid(13), sid(14)].map(|id| state.model().runtime_id(id));
        let inverse = run(&mut state, move_to(12, 10, 0x08)).unwrap();
        assert_eq!(
            handles,
            [sid(10), sid(11), sid(12), sid(13), sid(14)].map(|id| state.model().runtime_id(id))
        );
        assert_eq!(
            state.model().children_in_order(sid(10)),
            vec![sid(12), sid(11)]
        );
        run(&mut state, inverse).unwrap();
        assert_eq!(
            state.model().children_in_order(sid(10)),
            vec![sid(11), sid(12)]
        );
        assert_eq!(snapshot(&state), before);

        // 자손·modifier stack이 함께 옮겨집니다.
        let mut stack = ModifierStack::new(sid(14));
        stack
            .push_back(ModifierId::new(sid(80)), ModifierKind::Blur)
            .unwrap();
        run(
            &mut state,
            Command::InsertNode(InsertNode {
                nodes: vec![frame(14, Some(13), 0x10)],
                modifier_stacks: vec![stack.clone()],
                layout_records: Vec::new(),
            }),
        )
        .unwrap();
        let with_child = snapshot(&state);
        let inverse = run(&mut state, move_to(11, 12, 0x10)).unwrap();
        assert_eq!(state.model().children_in_order(sid(12)), vec![sid(11)]);
        assert_eq!(state.model().children_in_order(sid(13)), vec![sid(14)]);
        assert_eq!(state.model().modifier_stack(sid(14)), Some(&stack));
        run(&mut state, inverse).unwrap();
        assert_eq!(snapshot(&state), with_child);
    }

    fn two_view_model() -> DocumentModel {
        use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
        let mut definition = DocumentDefinition::new(sid(1), String::new());
        let mut page = PageDefinition::new(sid(2), String::new());
        page.add_view(ViewDefinition::new(sid(3), String::new(), sid(10)))
            .unwrap();
        page.add_view(ViewDefinition::new(sid(4), String::new(), sid(50)))
            .unwrap();
        definition.add_page(page).unwrap();
        let mut model = DocumentModel::new(definition);
        model.insert_node(frame(10, None, 0x10)).unwrap();
        model.insert_node(frame(11, Some(10), 0x10)).unwrap();
        model.insert_node(frame(13, Some(11), 0x10)).unwrap();
        model.insert_node(frame(50, None, 0x10)).unwrap();
        model.insert_node(frame(51, Some(50), 0x10)).unwrap();
        let mut text = frame(52, Some(50), 0x20);
        text.payload = NodePayload::Text(TextPayload::empty(0));
        model.insert_node(text).unwrap();
        model
    }

    #[test]
    fn invalid_moves_are_rejected() {
        use stellar_document::tree_ops::ReparentError;
        let mut state = DocumentState::new(two_view_model());
        let before = snapshot(&state);
        let reparent = |error| CommandError::Reparent {
            node: sid(11),
            error,
        };
        assert_eq!(
            run(&mut state, move_to(11, 11, 0x30)),
            Err(reparent(ReparentError::SelfParent))
        );
        assert_eq!(
            run(&mut state, move_to(11, 13, 0x30)),
            Err(reparent(ReparentError::DescendantParent))
        );
        assert_eq!(
            run(&mut state, move_to(11, 51, 0x30)),
            Err(reparent(ReparentError::CrossViewMove))
        );
        assert_eq!(
            run(&mut state, move_to(10, 11, 0x30)),
            Err(CommandError::ViewRoot { node: sid(10) })
        );
        assert_eq!(
            run(&mut state, move_to(51, 52, 0x30)),
            Err(CommandError::InvalidParent {
                node: sid(51),
                parent: Some(sid(52))
            })
        );
        assert_eq!(
            run(&mut state, move_to(13, 10, 0x10)),
            Err(CommandError::DuplicateOrderKey {
                parent: sid(10),
                node: sid(13)
            })
        );
        assert_eq!(
            run(&mut state, move_to(99, 10, 0x30)),
            Err(CommandError::UnknownNode { id: sid(99) })
        );
        assert_eq!(snapshot(&state), before);
        // 같은 위치로의 이동은 변경 없음.
        let request = CommandRequest::at(&state, move_to(13, 11, 0x10));
        let outcome = state.execute(&request).unwrap();
        assert!(outcome.changes.is_empty());
        assert_eq!(snapshot(&state), before);
    }

    #[test]
    fn move_depth_limit_counts_subtree_height() {
        let mut state = DocumentState::new(small_model());
        // 12 아래 62단 chain → 가장 깊은 node depth 63.
        let mut chain = Vec::new();
        for i in 0..62u64 {
            let parent = if i == 0 { 12 } else { 199 + i };
            chain.push(frame(200 + i, Some(parent), 0x40));
        }
        run(
            &mut state,
            Command::InsertNode(InsertNode {
                nodes: chain,
                modifier_stacks: Vec::new(),
                layout_records: Vec::new(),
            }),
        )
        .unwrap();
        // 12(높이 62)를 13(depth 2) 아래로 → 3 + 62 = 65 > 64.
        assert_eq!(
            run(&mut state, move_to(12, 13, 0x50)),
            Err(CommandError::DepthExceeded {
                node: sid(12),
                depth: 65
            })
        );
        // 11(depth 1) 아래로는 2 + 62 = 64 허용.
        run(&mut state, move_to(12, 11, 0x50)).unwrap();
    }

    #[test]
    fn random_moves_round_trip() {
        use stellar_document::test_support::random_document::{random_valid_document, SplitMix64};
        let mut moved = 0;
        for seed in 0..120u64 {
            let mut state = DocumentState::new(random_valid_document(seed));
            let before = state.model().to_records().unwrap();
            let ids = crate::tree::node_ids(state.model());
            if ids.len() < 3 {
                continue;
            }
            let mut rng = SplitMix64::new(seed ^ 0x307E);
            for _ in 0..6 {
                let node = *rng.pick(&ids);
                let parent = *rng.pick(&ids);
                let index = ChildIndex::build(state.model());
                let siblings = index.children(parent);
                let at = rng.below(siblings.len() as u64 + 1) as usize;
                let left = at
                    .checked_sub(1)
                    .and_then(|i| siblings.get(i))
                    .and_then(|id| crate::tree::order_key_of(state.model(), *id));
                let right = siblings
                    .get(at)
                    .and_then(|id| crate::tree::order_key_of(state.model(), *id));
                let key = OrderKey::between(left.as_ref(), right.as_ref());
                let command = Command::MoveNode(MoveNode {
                    node,
                    new_parent: parent,
                    order_key: key,
                });
                let request = CommandRequest::at(&state, command);
                if let Ok(outcome) = state.execute(&request) {
                    moved += 1;
                    stellar_document::validate::validate_document(
                        &state.model().to_records().unwrap(),
                    )
                    .expect("이동 후 문서 유효");
                    run(&mut state, outcome.inverse).unwrap();
                    assert_eq!(state.model().to_records().unwrap(), before, "seed {seed}");
                }
            }
        }
        assert!(moved > 100, "이동 {moved}");
    }
}

#[cfg(test)]
mod swap_tests {
    use super::*;
    use crate::codec::{decode_command, encode_command};
    use crate::command::test_fixtures::{frame, sid, small_model};
    use crate::command::Command;
    use crate::request::CommandRequest;
    use crate::state::DocumentState;
    use stellar_core_types::typed_value::TypedValue;
    use stellar_document::component::instance::SlotOverride;
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

#[cfg(test)]
mod layout_record_tests {
    //! 삽입·삭제가 명시 layout record(ADR-0086)를 옮기는지.
    use super::*;
    use crate::codec::{decode_command, encode_command};
    use crate::command::test_fixtures::{frame, sid, small_model};
    use crate::command::Command;
    use crate::request::CommandRequest;
    use crate::state::DocumentState;
    use stellar_document::layout_record::LayoutLonghand;
    use stellar_document::text::TextPayload;

    fn run(state: &mut DocumentState, command: Command) -> Result<Command, CommandError> {
        let request = CommandRequest::at(state, command);
        state.execute(&request).map(|outcome| outcome.inverse)
    }

    fn record(node: u64, pairs: &[(LayoutLonghand, &str)]) -> LayoutRecord {
        let mut record = LayoutRecord::new(sid(node));
        for (longhand, text) in pairs {
            record
                .set(*longhand, longhand.parse(text).unwrap())
                .unwrap();
        }
        record
    }

    /// 11(Frame)과 13(Frame)에 명시 값이 있는 모델.
    fn model_with_layout() -> DocumentModel {
        let mut model = small_model();
        model
            .set_layout_record(record(
                11,
                &[
                    (LayoutLonghand::Display, "flex"),
                    (LayoutLonghand::ColumnGap, "8px"),
                ],
            ))
            .unwrap();
        model
            .set_layout_record(record(13, &[(LayoutLonghand::FlexGrow, "1")]))
            .unwrap();
        model
    }

    fn payload_bytes(insert: &InsertNode) -> Vec<u8> {
        let mut out = Writer::new();
        insert.encode_into(&mut out);
        out.into_bytes()
    }

    #[test]
    fn delete_inverse_carries_subtree_layout_records_through_the_codec() {
        let mut state = DocumentState::new(model_with_layout());
        let before = state.model().to_records().unwrap();
        let inverse = run(
            &mut state,
            Command::DeleteSubtree(DeleteSubtree { root: sid(11) }),
        )
        .unwrap();
        let Command::InsertNode(insert) = &inverse else {
            panic!("삭제의 역은 InsertNode");
        };
        assert_eq!(
            insert.layout_records,
            vec![
                record(
                    11,
                    &[
                        (LayoutLonghand::Display, "flex"),
                        (LayoutLonghand::ColumnGap, "8px")
                    ]
                ),
                record(13, &[(LayoutLonghand::FlexGrow, "1")]),
            ]
        );
        // history journal에 저장했다가 다시 연 경우: 지운 node의 record는 영속 레코드에 나오지 않습니다(살아 있는
        // node의 record만). 역 command bytes만으로 record가 돌아와야 합니다.
        let restored = decode_command(&encode_command(&inverse)).unwrap();
        assert_eq!(restored, inverse);
        let saved = state.model().to_records().unwrap();
        assert!(saved.layout_records.is_empty());
        let (reopened, _) = DocumentModel::from_records(saved).unwrap();
        let mut reopened = DocumentState::new(reopened);
        run(&mut reopened, restored).unwrap();
        assert_eq!(reopened.model().to_records().unwrap(), before);
    }

    #[test]
    fn insert_without_layout_records_keeps_the_old_bytes() {
        let insert = InsertNode::single(frame(20, Some(10), 0x30));
        let mut old = Writer::new();
        old.len(1);
        old.bytes(&encode_node_record(&insert.nodes[0]));
        old.len(0);
        assert_eq!(payload_bytes(&insert), old.into_bytes());
    }

    #[test]
    fn layout_record_section_must_be_canonical() {
        let mut insert = InsertNode::single(frame(20, Some(10), 0x30));
        insert.nodes.push(frame(21, Some(20), 0x10));
        insert.layout_records = vec![
            record(20, &[(LayoutLonghand::Display, "grid")]),
            record(21, &[(LayoutLonghand::Top, "4px")]),
        ];
        let good = Command::InsertNode(insert.clone());
        assert_eq!(decode_command(&encode_command(&good)).unwrap(), good);
        // 빈 절과 내림차순 소유 node는 canonical 형식이 아닙니다.
        let mut empty = payload_bytes(&InsertNode::single(frame(20, Some(10), 0x30)));
        empty.extend_from_slice(&0u32.to_le_bytes());
        assert!(InsertNode::decode_from(&mut Reader::new(&empty)).is_err());
        insert.layout_records.reverse();
        let reversed = payload_bytes(&insert);
        assert!(InsertNode::decode_from(&mut Reader::new(&reversed)).is_err());
    }

    #[test]
    fn insert_rejects_records_outside_the_insert_or_not_applicable() {
        let model = small_model();
        // 삽입하지 않는 node의 record.
        let mut insert = InsertNode::single(frame(20, Some(10), 0x30));
        insert.layout_records = vec![record(11, &[(LayoutLonghand::Display, "flex")])];
        assert!(insert.validate(&model).is_err());
        // Text에 container longhand.
        let mut text = frame(21, Some(10), 0x40);
        text.payload = NodePayload::Text(TextPayload::empty(0));
        let mut insert = InsertNode::single(text);
        insert.layout_records = vec![record(21, &[(LayoutLonghand::Display, "flex")])];
        assert!(insert.validate(&model).is_err());
        // Frame에 payload 크기 longhand.
        let mut insert = InsertNode::single(frame(20, Some(10), 0x30));
        insert.layout_records = vec![record(20, &[(LayoutLonghand::Width, "10px")])];
        assert!(insert.validate(&model).is_err());
        // 빈 record.
        let mut insert = InsertNode::single(frame(20, Some(10), 0x30));
        insert.layout_records = vec![LayoutRecord::new(sid(20))];
        assert!(insert.validate(&model).is_err());
        // 소유 node 내림차순 목록은 codec이 다시 읽지 못하므로 적용 전에 거부합니다. 오류는 위반 node를 가리킵니다.
        let mut insert = InsertNode::single(frame(20, Some(10), 0x30));
        insert.nodes.push(frame(21, Some(20), 0x10));
        insert.layout_records = vec![
            record(21, &[(LayoutLonghand::FlexGrow, "1")]),
            record(20, &[(LayoutLonghand::Display, "grid")]),
        ];
        assert!(matches!(
            insert.validate(&model),
            Err(CommandError::Model(ModelError::Layout { id, .. })) if id == sid(20)
        ));
        insert.layout_records.reverse();
        insert.validate(&model).unwrap();
        // 자식 node의 위반은 root가 아니라 그 node를 가리킵니다.
        insert.layout_records[1] = record(21, &[(LayoutLonghand::Width, "10px")]);
        assert!(matches!(
            insert.validate(&model),
            Err(CommandError::Model(ModelError::Layout { id, .. })) if id == sid(21)
        ));
        // Text의 item longhand와 record 크기는 받습니다.
        let mut text = frame(21, Some(10), 0x40);
        text.payload = NodePayload::Text(TextPayload::empty(0));
        let mut insert = InsertNode::single(text);
        insert.layout_records = vec![record(
            21,
            &[
                (LayoutLonghand::Width, "120px"),
                (LayoutLonghand::FlexShrink, "0"),
            ],
        )];
        insert.validate(&model).unwrap();
    }

    #[test]
    fn insert_clears_a_record_left_by_an_earlier_delete_of_the_same_id() {
        let mut state = DocumentState::new(model_with_layout());
        run(
            &mut state,
            Command::DeleteSubtree(DeleteSubtree { root: sid(11) }),
        )
        .unwrap();
        // 같은 id를 record 없이 다시 넣으면 명시 값이 없는 node입니다.
        let mut again = InsertNode::single(frame(11, Some(10), 0x10));
        again.nodes.push(frame(13, Some(11), 0x10));
        let inverse = run(&mut state, Command::InsertNode(again)).unwrap();
        assert!(state.model().layout_record(sid(11)).is_none());
        assert!(state.model().layout_record(sid(13)).is_none());
        assert!(state
            .model()
            .to_records()
            .unwrap()
            .layout_records
            .is_empty());
        // 되돌리고 다시 실행해도 같습니다.
        let redo = run(&mut state, inverse).unwrap();
        run(&mut state, redo).unwrap();
        assert!(state.model().layout_record(sid(11)).is_none());
    }
}
