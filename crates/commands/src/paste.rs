//! 원본 삭제·편집 후에도 유지되는 subtree 스냅샷. 플랫폼 pasteboard 접근은 셸의 책임입니다.
use std::collections::BTreeSet;

use stellar_core_types::{
    clock::MonotonicClock,
    ids::{IdRemapTable, StableId, StableIdGenerator},
    order_key::OrderKey,
};
use stellar_document::{model::DocumentModel, modifier::ModifierStack};

use crate::{
    codec::{decode_command, encode_command, CommandCodecError},
    command::{Command, CommandError},
    duplicate::{remap_layout, remap_modifier_payload, remap_payload},
    kind::CommandKind,
    node_ops::{DeleteSubtree, InsertNode},
    tree::path_element_ids,
};

/// v1은 같은 문서의 subtree만 지원합니다. 다른 문서의 asset·Component 참조는 가져오지 않습니다.
#[derive(Debug, Clone)]
pub struct ClipboardSubtree {
    document: StableId,
    snapshot: InsertNode,
}

impl ClipboardSubtree {
    /// 문서를 변경하지 않고 subtree와 modifier, 명시 layout record를 복사합니다. View root는 거부합니다.
    pub fn capture(model: &DocumentModel, root: StableId) -> Result<Self, CommandError> {
        Ok(Self {
            document: model.document_id(),
            snapshot: DeleteSubtree { root }.invert(model)?,
        })
    }

    /// 버전 header·출처 문서 ID·기존 canonical InsertNode codec을 사용합니다. layout record가 없으면 bytes는
    /// layout record 절이 생기기 전과 같습니다. 절이 있는 데이터를 절을 모르는 옛 앱이 읽으면 InsertNode
    /// decode가 trailing bytes로 거부합니다(조용히 잃지 않음).
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = b"STELLAR-CLIP\x01".to_vec();
        bytes.extend_from_slice(&self.document.to_bytes());
        bytes.extend(encode_command(&Command::InsertNode(self.snapshot.clone())));
        bytes
    }

    /// 플랫폼에서 읽은 데이터는 크기 제한 후 closed command codec으로 해석합니다.
    pub fn decode(bytes: &[u8]) -> Result<Self, CommandCodecError> {
        let malformed = || CommandCodecError::Malformed {
            reason: "지원하지 않는 subtree 클립보드 형식",
        };
        if bytes.len() > 16 * 1024 * 1024
            || bytes.len() < 29
            || !bytes.starts_with(b"STELLAR-CLIP\x01")
        {
            return Err(malformed());
        }
        let document = StableId::from_bytes(bytes[13..29].try_into().map_err(|_| malformed())?);
        let command = &bytes[29..];
        if !command.starts_with(&CommandKind::InsertNode.as_u16().to_le_bytes()) {
            return Err(malformed());
        }
        let Command::InsertNode(snapshot) = decode_command(command)? else {
            return Err(malformed());
        };
        if snapshot.nodes.is_empty() {
            return Err(malformed());
        }
        Ok(Self { document, snapshot })
    }

    /// 결정적 id 생성기로 [`ClipboardSubtree::plan_with`]를 부릅니다(테스트·fuzz용).
    pub fn plan<C: MonotonicClock>(
        &self,
        model: &DocumentModel,
        parent: StableId,
        order: OrderKey,
        generator: &mut StableIdGenerator,
        clock: &C,
    ) -> Result<InsertNode, CommandError> {
        self.plan_with(model, parent, order, &mut || generator.next_id(clock))
    }

    /// 모든 영속 ID를 `new_id`에서 받아 재발급하고 목적 부모·순서를 지정합니다. 제품 경로는 UUIDv7 생성기를
    /// 넘깁니다. 반환 command는 일반 history로 실행합니다. 창의 선택·잠금 및 View 경계 정책은 호출자가
    /// 검사해야 합니다.
    pub fn plan_with(
        &self,
        model: &DocumentModel,
        parent: StableId,
        order: OrderKey,
        new_id: &mut dyn FnMut() -> StableId,
    ) -> Result<InsertNode, CommandError> {
        if self.document != model.document_id() {
            return Err(CommandError::InvalidRequest {
                reason: "다른 문서에서 복사한 subtree는 아직 지원하지 않습니다",
            });
        }
        let mut ids = BTreeSet::new();
        let mut claim = |id| {
            if id == StableId::NIL {
                return Err(CommandError::NilId);
            }
            if !ids.insert(id) {
                return Err(CommandError::DuplicateId { id });
            }
            Ok(())
        };
        for node in &self.snapshot.nodes {
            claim(node.id)?;
            for id in path_element_ids(&node.payload) {
                claim(id)?;
            }
        }
        for stack in &self.snapshot.modifier_stacks {
            for entry in stack.entries() {
                claim(entry.id.stable_id())?;
            }
        }
        let mut table = IdRemapTable::new();
        for old in ids {
            table.insert(old, new_id());
        }
        let mut insert = self.snapshot.clone();
        for (index, node) in insert.nodes.iter_mut().enumerate() {
            node.id = table.remap(node.id);
            node.parent = if index == 0 {
                Some(parent)
            } else {
                node.parent.map(|id| table.remap(id))
            };
            if index == 0 {
                node.order_key = order.clone();
            }
            node.payload = remap_payload(&table, &node.payload)?;
        }
        for stack in &mut insert.modifier_stacks {
            let mut copy = ModifierStack::new(table.remap(stack.node));
            for entry in stack.entries() {
                let mut entry = entry.clone();
                entry.id =
                    stellar_document::modifier::ModifierId::new(table.remap(entry.id.stable_id()));
                entry.payload = remap_modifier_payload(&table, &entry.payload);
                copy.insert(entry).map_err(|error| CommandError::Modifier {
                    owner: copy.node,
                    error,
                })?;
            }
            *stack = copy;
        }
        // 명시 layout longhand(ADR-0086)는 값에 id가 없어 소유 node만 새 id로 옮깁니다.
        for layout in &mut insert.layout_records {
            *layout = remap_layout(&table, layout);
        }
        insert.layout_records.sort_by_key(|layout| layout.node);
        insert.validate(model)?;
        Ok(insert)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        command::test_fixtures::{sid, small_model},
        request::CommandRequest,
        state::DocumentState,
    };
    use stellar_core_types::{clock::ManualClock, reference::NodeRef};
    use stellar_document::{frame::Clipping, records::NodePayload};

    fn execute(state: &mut DocumentState, command: Command) -> Command {
        state
            .execute(&CommandRequest::at(state, command))
            .unwrap()
            .inverse
    }

    #[test]
    fn snapshot_survives_source_delete_and_paste_undo_redo() {
        let model = small_model();
        let clipboard =
            ClipboardSubtree::decode(&ClipboardSubtree::capture(&model, sid(11)).unwrap().encode())
                .unwrap();
        let mut state = DocumentState::new(model);
        execute(
            &mut state,
            Command::DeleteSubtree(DeleteSubtree { root: sid(11) }),
        );
        let mut generator = StableIdGenerator::new(7);
        let clock = ManualClock::new(100);
        let mut previous = None;
        for order in [0x30, 0x40] {
            let insert = clipboard
                .plan(
                    state.model(),
                    sid(10),
                    OrderKey::from_bytes(vec![order]).unwrap(),
                    &mut generator,
                    &clock,
                )
                .unwrap();
            let root = insert.nodes[0].id;
            let child = insert.nodes[1].id;
            assert_ne!(root, sid(11));
            assert_ne!(child, sid(13));
            assert_ne!(Some(root), previous);
            assert_eq!(insert.nodes[0].display_name, "n11");
            assert_eq!(insert.nodes[1].parent, Some(root));
            let undo = execute(&mut state, Command::InsertNode(insert));
            assert_eq!(state.model().children_in_order(root), [child]);
            let redo = execute(&mut state, undo);
            assert!(!state.model().contains(root));
            execute(&mut state, redo);
            assert_eq!(state.model().children_in_order(root), [child]);
            previous = Some(root);
        }
    }

    #[test]
    fn path_ids_and_internal_clip_references_are_remapped() {
        use stellar_core_types::value::Vec2;
        use stellar_document::path::{IdentifiedPath, PathCommand, PathData, WindingRule};
        let mut model = small_model();
        let mut next = 100;
        let mut path_record = model.remove_leaf(sid(13)).unwrap();
        path_record.payload = NodePayload::Path(IdentifiedPath::assign(
            PathData::new(
                WindingRule::NonZero,
                vec![PathCommand::MoveTo(Vec2::new(1.0, 2.0).unwrap())],
            )
            .unwrap(),
            &mut || {
                next += 1;
                sid(next)
            },
        ));
        model.insert_node(path_record).unwrap();
        let Some(NodePayload::Frame(mut frame)) = model.payload(sid(11)).cloned() else {
            panic!("frame");
        };
        frame.clipping = Clipping::Path(NodeRef::new(sid(13)));
        model
            .set_payload(sid(11), NodePayload::Frame(frame))
            .unwrap();
        use stellar_document::modifier::{
            ModifierEntry, ModifierId, ModifierKind, ModifierPayload,
        };
        let mut stack = ModifierStack::new(sid(11));
        stack
            .insert(ModifierEntry {
                id: ModifierId::new(sid(200)),
                order: OrderKey::from_bytes(vec![0x10]).unwrap(),
                kind: ModifierKind::Mask,
                enabled: true,
                payload: ModifierPayload::Mask {
                    source: NodeRef::new(sid(13)),
                },
            })
            .unwrap();
        model.set_modifier_stack(stack).unwrap();
        let clipboard =
            ClipboardSubtree::decode(&ClipboardSubtree::capture(&model, sid(11)).unwrap().encode())
                .unwrap();
        let insert = clipboard
            .plan(
                &model,
                sid(12),
                OrderKey::from_bytes(vec![0x10]).unwrap(),
                &mut StableIdGenerator::new(5),
                &ManualClock::new(100),
            )
            .unwrap();
        let NodePayload::Frame(frame) = &insert.nodes[0].payload else {
            panic!("frame");
        };
        assert_eq!(
            frame.clipping,
            Clipping::Path(NodeRef::new(insert.nodes[1].id))
        );
        let copied_stack = &insert.modifier_stacks[0];
        assert_eq!(copied_stack.node, insert.nodes[0].id);
        assert_ne!(copied_stack.entries()[0].id.stable_id(), sid(200));
        assert_eq!(
            copied_stack.entries()[0].payload,
            ModifierPayload::Mask {
                source: NodeRef::new(insert.nodes[1].id)
            }
        );
        let original = path_element_ids(model.payload(sid(13)).unwrap());
        assert!(path_element_ids(&insert.nodes[1].payload)
            .iter()
            .all(|id| !original.contains(id)));
        let mut state = DocumentState::new(model);
        execute(&mut state, Command::InsertNode(insert));
    }

    #[test]
    fn rejects_foreign_document_invalid_parent_and_non_clipboard_data() {
        let model = small_model();
        let clipboard = ClipboardSubtree::capture(&model, sid(11)).unwrap();
        let mut foreign = crate::command::test_fixtures::definition();
        foreign.id = sid(99);
        let foreign = DocumentModel::new(foreign);
        let mut generator = StableIdGenerator::new(7);
        let clock = ManualClock::new(100);
        let order = OrderKey::from_bytes(vec![0x30]).unwrap();
        assert!(clipboard
            .plan(&foreign, sid(10), order.clone(), &mut generator, &clock)
            .is_err());
        assert!(clipboard
            .plan(&model, sid(999), order, &mut generator, &clock)
            .is_err());
        assert!(ClipboardSubtree::capture(&model, sid(10)).is_err());
        let bytes = clipboard.encode();
        for len in 0..bytes.len() {
            assert!(ClipboardSubtree::decode(&bytes[..len]).is_err());
        }
        let mut trailing = bytes;
        trailing.push(0);
        assert!(ClipboardSubtree::decode(&trailing).is_err());
        assert!(
            ClipboardSubtree::decode(&encode_command(&Command::DeleteSubtree(DeleteSubtree {
                root: sid(11)
            })))
            .is_err()
        );
    }
}

#[cfg(test)]
mod layout_record_tests {
    //! 클립보드 subtree가 명시 layout record(ADR-0086)를 담고 붙여넣을 때 새 id로 옮기는지.
    use super::*;
    use crate::command::test_fixtures::{sid, small_model};
    use crate::request::CommandRequest;
    use crate::state::DocumentState;
    use stellar_core_types::clock::ManualClock;
    use stellar_document::layout_record::{LayoutLonghand, LayoutRecord};

    #[test]
    fn clipboard_carries_layout_records_and_remaps_their_owner() {
        let mut model = small_model();
        let mut record = LayoutRecord::new(sid(13));
        record
            .set(
                LayoutLonghand::FlexBasis,
                LayoutLonghand::FlexBasis.parse("50%").unwrap(),
            )
            .unwrap();
        model.set_layout_record(record).unwrap();
        let clipboard =
            ClipboardSubtree::decode(&ClipboardSubtree::capture(&model, sid(11)).unwrap().encode())
                .unwrap();
        let mut state = DocumentState::new(model);
        // 원본을 지운 뒤에도 클립보드의 record로 붙여넣습니다.
        state
            .execute(&CommandRequest::at(
                &state,
                Command::DeleteSubtree(DeleteSubtree { root: sid(11) }),
            ))
            .unwrap();
        let insert = clipboard
            .plan(
                state.model(),
                sid(10),
                OrderKey::from_bytes(vec![0x30]).unwrap(),
                &mut StableIdGenerator::new(3),
                &ManualClock::new(100),
            )
            .unwrap();
        let child = insert.nodes[1].id;
        assert_ne!(child, sid(13));
        assert_eq!(insert.layout_records.len(), 1);
        assert_eq!(insert.layout_records[0].node, child);
        state
            .execute(&CommandRequest::at(&state, Command::InsertNode(insert)))
            .unwrap();
        assert_eq!(
            state
                .model()
                .layout_record(child)
                .and_then(|r| r.get(LayoutLonghand::FlexBasis)),
            Some(&LayoutLonghand::FlexBasis.parse("50%").unwrap())
        );
    }
}
