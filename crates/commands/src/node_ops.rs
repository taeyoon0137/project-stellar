//! node 구조 command (VII.2.1~VII.2.3).
//!
//! - [`InsertNode`] (VII.2.1): caller가 StableId를 정한 node(또는 그 node를 root로 하는
//!   pre-order subtree)를 삽입합니다. 중복 id, 잘못된 부모, 형제 OrderKey 충돌, 깊이 초과,
//!   참조 종류 불일치, modifier 소유·cycle 위반을 적용 전에 거부합니다.
//! - [`DeleteSubtree`] (VII.2.2): subtree를 지웁니다. 역 command는 subtree와 내부 참조를 담은
//!   InsertNode이고, 외부 inbound 참조가 있으면 정책 오류로 거부합니다. InsertNode의 역입니다.

use std::collections::{BTreeMap, BTreeSet};

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_document::frame::Clipping;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{find_modifier_cycle, validate_modifier_owner, ModifierStack};
use stellar_document::node_kind::NodeKind;
use stellar_document::record_codec::{decode_node_record, encode_node_record};
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_document::validate::{can_have_children, MAX_NODE_DEPTH};

use crate::codec::{CommandCodecError, Reader, Writer};
use crate::command::CommandError;
use crate::edit::EditContext;
use crate::tree::{
    depth_of, id_owners, kind_of, parent_of, path_element_ids, reference_index, ChildIndex, IdOwner,
};

/// node(또는 subtree) 삽입.
///
/// `nodes[0]`이 삽입 root이고 부모는 문서에 이미 있는 container여야 합니다. 나머지는
/// pre-order 순서로, 부모가 앞선 레코드여야 합니다. `modifier_stacks`는 삽입하는 node의
/// stack입니다(빈 stack은 넣지 않습니다).
#[derive(Debug, Clone, PartialEq)]
pub struct InsertNode {
    /// 삽입할 node(pre-order, 첫 원소가 root).
    pub nodes: Vec<NodeRecord>,
    /// 삽입하는 node의 modifier stack.
    pub modifier_stacks: Vec<ModifierStack>,
}

impl InsertNode {
    /// node 하나를 삽입합니다.
    pub fn single(record: NodeRecord) -> InsertNode {
        InsertNode {
            nodes: vec![record],
            modifier_stacks: Vec::new(),
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
        let owners = id_owners(model);

        // 1. id: NIL 금지, 요청 안 유일, 문서 안 미사용.
        let mut new_ids: BTreeMap<StableId, NodeKind> = BTreeMap::new();
        let mut claimed: BTreeSet<StableId> = BTreeSet::new();
        let mut claim = |id: StableId| -> Result<(), CommandError> {
            if id == StableId::NIL {
                return Err(CommandError::NilId);
            }
            if owners.contains_key(&id) || !claimed.insert(id) {
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
            let index = ChildIndex::build(model);
            for sibling in index.children(parent) {
                if crate::tree::order_key_of(model, *sibling).as_ref() == Some(&root.order_key) {
                    return Err(CommandError::DuplicateOrderKey {
                        parent,
                        node: root.id,
                    });
                }
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
        let target_kind = |id: StableId| -> Option<IdOwner> {
            new_ids
                .get(&id)
                .map(|kind| IdOwner::Node(*kind))
                .or_else(|| owners.get(&id).copied())
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
    }

    pub(crate) fn decode_from(input: &mut Reader<'_>) -> Result<InsertNode, CommandCodecError> {
        let count = input.len()?;
        let mut nodes = Vec::with_capacity(count);
        for _ in 0..count {
            nodes.push(decode_node_record(input.bytes()?).map_err(|_| {
                CommandCodecError::Malformed {
                    reason: "잘못된 node 레코드",
                }
            })?);
        }
        let count = input.len()?;
        let mut modifier_stacks = Vec::with_capacity(count);
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
        })
    }
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
        let references = reference_index(model);
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
        for id in subtree {
            nodes.push(model.record(id).ok_or(CommandError::UnknownNode { id })?);
            if let Some(stack) = model.modifier_stack(id) {
                modifier_stacks.push(stack.clone());
            }
        }
        Ok(InsertNode {
            nodes,
            modifier_stacks,
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
}
