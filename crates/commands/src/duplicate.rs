//! `DuplicateSubtree` command (VII.2.8).
//!
//! subtree를 복제해 새 부모 아래에 넣습니다. 복제본의 모든 영속 id(node, modifier, path point·
//! handle)는 [`IdRemapTable`](III.1.5)로 새 id를 받고, **내부 참조**(subtree 안을 가리키는 Frame
//! clip·modifier 참조)는 새 id로, **외부 참조**는 원래 대상 그대로 재매핑합니다.
//!
//! 재실행(redo) 결정론을 위해 command는 old → new id 쌍을 그대로 담습니다. 쌍은 subtree의 영속 id를
//! 정확히 덮어야 하며(누락·여분 거부), 새 id는 NIL이 아니고 서로 다르며 문서에서 쓰이지 않아야
//! 합니다. 쌍은 [`DuplicateSubtree::plan`]이 결정적 id 생성기(III.1.2)로 만들 수 있습니다.
//!
//! 적용은 재매핑한 레코드로 만든 [`InsertNode`]와 같은 검증·적용 경로를 지나며(부모·OrderKey·깊이·
//! 참조·modifier 규칙), 역 command는 복제본 root의 [`DeleteSubtree`]입니다.

use std::collections::BTreeSet;

use stellar_core_types::clock::MonotonicClock;
use stellar_core_types::ids::{IdRemapTable, StableId, StableIdGenerator};
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::NodeRef;
use stellar_document::frame::Clipping;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{ModifierEntry, ModifierId, ModifierPayload, ModifierStack};
use stellar_document::path::{CommandIds, IdentifiedPath, PathHandleId, PathPointId};
use stellar_document::records::{NodePayload, NodeRecord};

use crate::codec::{CommandCodecError, Reader, Writer};
use crate::command::CommandError;
use crate::edit::EditContext;
use crate::node_ops::{decode_order_key, DeleteSubtree, InsertNode};
use crate::tree::{path_element_ids, ChildIndex};

/// subtree 복제.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateSubtree {
    /// 복제할 subtree root.
    pub source: StableId,
    /// 복제본을 넣을 부모.
    pub new_parent: StableId,
    /// 복제본 root의 형제 순서 key.
    pub order_key: OrderKey,
    /// old → new id 쌍(old id 오름차순).
    pub remap: Vec<(StableId, StableId)>,
}

/// subtree의 모든 영속 id(node, modifier, path element), 오름차순.
fn subtree_ids(model: &DocumentModel, nodes: &[StableId]) -> Vec<StableId> {
    let mut ids = BTreeSet::new();
    for id in nodes {
        ids.insert(*id);
        if let Some(payload) = model.payload(*id) {
            ids.extend(path_element_ids(payload));
        }
        if let Some(stack) = model.modifier_stack(*id) {
            ids.extend(stack.entries().iter().map(|e| e.id.stable_id()));
        }
    }
    ids.into_iter().collect()
}

fn remap_ref(table: &IdRemapTable, target: NodeRef) -> NodeRef {
    NodeRef::new(table.remap(target.id()))
}

fn remap_payload(table: &IdRemapTable, payload: &NodePayload) -> Result<NodePayload, CommandError> {
    Ok(match payload {
        NodePayload::Frame(frame) => {
            let mut frame = frame.clone();
            if let Clipping::Path(target) = frame.clipping {
                frame.clipping = Clipping::Path(remap_ref(table, target));
            }
            NodePayload::Frame(frame)
        }
        NodePayload::Path(path) => {
            let ids = path
                .ids()
                .iter()
                .map(|ids| CommandIds {
                    point: ids.point.map(|p| PathPointId(table.remap(p.0))),
                    handles: ids.handles.map(|(h1, h2)| {
                        (
                            PathHandleId(table.remap(h1.0)),
                            PathHandleId(table.remap(h2.0)),
                        )
                    }),
                })
                .collect();
            NodePayload::Path(IdentifiedPath::new(path.data().clone(), ids).map_err(|_| {
                CommandError::InvalidRequest {
                    reason: "복제 path id가 path 불변식을 깨뜨립니다",
                }
            })?)
        }
        other => other.clone(),
    })
}

fn remap_modifier_payload(table: &IdRemapTable, payload: &ModifierPayload) -> ModifierPayload {
    match payload {
        ModifierPayload::Boolean { op, operands } => ModifierPayload::Boolean {
            op: *op,
            operands: operands.iter().map(|r| remap_ref(table, *r)).collect(),
        },
        ModifierPayload::Mask { source } => ModifierPayload::Mask {
            source: remap_ref(table, *source),
        },
        ModifierPayload::Clip { source } => ModifierPayload::Clip {
            source: remap_ref(table, *source),
        },
        other => other.clone(),
    }
}

impl DuplicateSubtree {
    /// 결정적 id 생성기로 재매핑 쌍을 만들어 command를 계획합니다.
    pub fn plan<C: MonotonicClock>(
        model: &DocumentModel,
        source: StableId,
        new_parent: StableId,
        order_key: OrderKey,
        generator: &mut StableIdGenerator,
        clock: &C,
    ) -> Result<DuplicateSubtree, CommandError> {
        if !model.contains(source) {
            return Err(CommandError::UnknownNode { id: source });
        }
        let nodes = ChildIndex::build(model).subtree(source);
        let olds = subtree_ids(model, &nodes);
        let table = IdRemapTable::clone_ids(&olds, generator, clock);
        let remap = olds.iter().map(|old| (*old, table.remap(*old))).collect();
        Ok(DuplicateSubtree {
            source,
            new_parent,
            order_key,
            remap,
        })
    }

    /// 복제본 root id.
    pub fn copy_root(&self) -> Option<StableId> {
        self.remap
            .iter()
            .find(|(old, _)| *old == self.source)
            .map(|(_, new)| *new)
    }

    /// 재매핑한 subtree를 삽입하는 InsertNode를 만듭니다(재매핑 쌍 검증 포함).
    fn to_insert(&self, model: &DocumentModel) -> Result<InsertNode, CommandError> {
        if !model.contains(self.source) {
            return Err(CommandError::UnknownNode { id: self.source });
        }
        let nodes = ChildIndex::build(model).subtree(self.source);
        let expected = subtree_ids(model, &nodes);
        let olds: Vec<StableId> = self.remap.iter().map(|(old, _)| *old).collect();
        if olds != expected {
            return Err(CommandError::InvalidRequest {
                reason: "재매핑 쌍이 subtree의 영속 id를 정확히 덮지 않습니다",
            });
        }
        let mut table = IdRemapTable::new();
        let mut news = BTreeSet::new();
        for (old, new) in &self.remap {
            if *new == StableId::NIL {
                return Err(CommandError::NilId);
            }
            if !news.insert(*new) {
                return Err(CommandError::DuplicateId { id: *new });
            }
            table.insert(*old, *new);
        }
        let mut records = Vec::with_capacity(nodes.len());
        let mut stacks = Vec::new();
        for (index, id) in nodes.iter().enumerate() {
            let record = model
                .record(*id)
                .ok_or(CommandError::UnknownNode { id: *id })?;
            let (parent, order_key) = if index == 0 {
                (Some(self.new_parent), self.order_key.clone())
            } else {
                (
                    record.parent.map(|p| table.remap(p)),
                    record.order_key.clone(),
                )
            };
            records.push(NodeRecord {
                id: table.remap(record.id),
                parent,
                order_key,
                visible: record.visible,
                locked: record.locked,
                display_name: record.display_name.clone(),
                payload: remap_payload(&table, &record.payload)?,
            });
            if let Some(stack) = model.modifier_stack(*id) {
                let mut copy = ModifierStack::new(table.remap(stack.node));
                for entry in stack.entries() {
                    copy.insert(ModifierEntry {
                        id: ModifierId::new(table.remap(entry.id.stable_id())),
                        order: entry.order.clone(),
                        kind: entry.kind,
                        enabled: entry.enabled,
                        payload: remap_modifier_payload(&table, &entry.payload),
                    })
                    .map_err(|error| CommandError::Modifier {
                        owner: copy.node,
                        error,
                    })?;
                }
                stacks.push(copy);
            }
        }
        Ok(InsertNode {
            nodes: records,
            modifier_stacks: stacks,
        })
    }

    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        self.to_insert(model)?.validate(model)
    }

    pub(crate) fn invert(&self) -> Result<DeleteSubtree, CommandError> {
        let root = self.copy_root().ok_or(CommandError::InvalidRequest {
            reason: "재매핑 쌍에 subtree root가 없습니다",
        })?;
        Ok(DeleteSubtree { root })
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        let insert = self.to_insert(ctx.model())?;
        insert.apply(ctx)
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.id(self.source);
        out.id(self.new_parent);
        out.bytes(self.order_key.as_bytes());
        out.len(self.remap.len());
        for (old, new) in &self.remap {
            out.id(*old);
            out.id(*new);
        }
    }

    pub(crate) fn decode_from(
        input: &mut Reader<'_>,
    ) -> Result<DuplicateSubtree, CommandCodecError> {
        let source = input.id()?;
        let new_parent = input.id()?;
        let order_key = decode_order_key(input)?;
        let count = input.len()?;
        let mut remap = Vec::with_capacity(count);
        let mut seen: BTreeSet<StableId> = BTreeSet::new();
        for _ in 0..count {
            let old = input.id()?;
            let new = input.id()?;
            if !seen.insert(old) {
                return Err(CommandCodecError::Malformed {
                    reason: "재매핑 old id 중복",
                });
            }
            remap.push((old, new));
        }
        Ok(DuplicateSubtree {
            source,
            new_parent,
            order_key,
            remap,
        })
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
    use crate::tree::{id_owners, node_ids};
    use std::collections::BTreeMap;
    use stellar_core_types::clock::ManualClock;
    use stellar_core_types::value::Vec2;
    use stellar_document::frame::FramePayload;
    use stellar_document::modifier::ModifierKind;
    use stellar_document::path::{PathCommand, PathData, WindingRule};
    use stellar_document::test_support::semantic_snapshot::model_snapshot_json;

    fn snapshot(state: &DocumentState) -> String {
        model_snapshot_json(state.model()).expect("snapshot")
    }

    fn run(state: &mut DocumentState, command: Command) -> Result<Command, CommandError> {
        let request = CommandRequest::at(state, command);
        state.execute(&request).map(|outcome| outcome.inverse)
    }

    fn key(byte: u8) -> OrderKey {
        OrderKey::from_bytes(vec![byte]).unwrap()
    }

    /// 11 아래 subtree 20{21(Path), 22(Frame, clip→21, mask→21, mask→13 외부)}.
    fn state_with_refs() -> DocumentState {
        let mut state = DocumentState::new(small_model());
        let data = PathData::new(
            WindingRule::NonZero,
            vec![
                PathCommand::MoveTo(Vec2::new(0.0, 0.0).unwrap()),
                PathCommand::CubicTo {
                    c1: Vec2::new(1.0, 0.0).unwrap(),
                    c2: Vec2::new(2.0, 1.0).unwrap(),
                    to: Vec2::new(3.0, 3.0).unwrap(),
                },
            ],
        )
        .unwrap();
        let mut next = 300u64;
        let path = IdentifiedPath::assign(data, &mut || {
            next += 1;
            sid(next)
        });
        let mut path_record = frame(21, Some(20), 0x10);
        path_record.payload = NodePayload::Path(path);
        let mut clipped = frame(22, Some(20), 0x20);
        clipped.payload = NodePayload::Frame(FramePayload {
            clipping: Clipping::Path(NodeRef::new(sid(21))),
            ..FramePayload::default()
        });
        let mut stack = ModifierStack::new(sid(22));
        for (id, order, target) in [(70, 0x10, 21), (71, 0x20, 13)] {
            stack
                .insert(ModifierEntry {
                    id: ModifierId::new(sid(id)),
                    order: key(order),
                    kind: ModifierKind::Mask,
                    enabled: true,
                    payload: ModifierPayload::Mask {
                        source: NodeRef::new(sid(target)),
                    },
                })
                .unwrap();
        }
        run(
            &mut state,
            Command::InsertNode(InsertNode {
                nodes: vec![frame(20, Some(11), 0x40), path_record, clipped],
                modifier_stacks: vec![stack],
            }),
        )
        .unwrap();
        state
    }

    fn plan(state: &DocumentState, parent: u64, order: u8) -> DuplicateSubtree {
        let clock = ManualClock::new(1_900_000_000_000 * 1_000_000);
        let mut generator = StableIdGenerator::new(7);
        DuplicateSubtree::plan(
            state.model(),
            sid(20),
            sid(parent),
            key(order),
            &mut generator,
            &clock,
        )
        .unwrap()
    }

    #[test]
    fn duplicate_remaps_internal_ids_and_references() {
        // 검증 기준(VII.2.8): 원본과 중복 ID 없음, 내부 참조 새 subtree.
        let mut state = state_with_refs();
        let before = snapshot(&state);
        let originals: BTreeSet<StableId> = id_owners(state.model()).into_keys().collect();
        let command = plan(&state, 12, 0x30);
        assert_eq!(
            command.remap.len(),
            3 + 4 + 2,
            "node 3 + path point·handle 4 + modifier 2"
        );
        let copy_root = command.copy_root().unwrap();
        let table: BTreeMap<StableId, StableId> = command.remap.iter().copied().collect();

        let inverse = run(&mut state, Command::DuplicateSubtree(command.clone())).unwrap();
        assert_eq!(
            inverse,
            Command::DeleteSubtree(DeleteSubtree { root: copy_root })
        );
        stellar_document::validate::validate_document(&state.model().to_records().unwrap())
            .expect("복제 후 문서 유효");
        // 새 id는 원본과 겹치지 않습니다.
        let after: BTreeSet<StableId> = id_owners(state.model()).into_keys().collect();
        let added: BTreeSet<StableId> = after.difference(&originals).copied().collect();
        assert_eq!(added, table.values().copied().collect());
        assert!(added.is_disjoint(&originals));
        // 구조: 새 부모 아래, 자식 순서 유지.
        assert_eq!(state.model().children_in_order(sid(12)), vec![copy_root]);
        let copy_21 = table[&sid(21)];
        let copy_22 = table[&sid(22)];
        assert_eq!(
            state.model().children_in_order(copy_root),
            vec![copy_21, copy_22]
        );
        // 내부 참조(clip·mask → 21)는 복제본 21로, 외부 참조(mask → 13)는 그대로.
        let NodePayload::Frame(clipped) = state.model().payload(copy_22).unwrap() else {
            panic!("Frame");
        };
        assert_eq!(clipped.clipping, Clipping::Path(NodeRef::new(copy_21)));
        let stack = state.model().modifier_stack(copy_22).unwrap();
        let sources: Vec<StableId> = stack
            .entries()
            .iter()
            .map(|e| match e.payload {
                ModifierPayload::Mask { source } => source.id(),
                _ => StableId::NIL,
            })
            .collect();
        assert_eq!(sources, vec![copy_21, sid(13)]);
        // path point·handle id도 새 id.
        let NodePayload::Path(path) = state.model().payload(copy_21).unwrap() else {
            panic!("Path");
        };
        for id in path_element_ids(&NodePayload::Path(path.clone())) {
            assert!(added.contains(&id));
        }
        // 원본은 그대로.
        let NodePayload::Frame(original) = state.model().payload(sid(22)).unwrap() else {
            panic!("Frame");
        };
        assert_eq!(original.clipping, Clipping::Path(NodeRef::new(sid(21))));

        run(&mut state, inverse).unwrap();
        assert_eq!(snapshot(&state), before);
        assert_eq!(
            decode_command(&encode_command(&Command::DuplicateSubtree(command.clone()))).unwrap(),
            Command::DuplicateSubtree(command)
        );
    }

    #[test]
    fn duplicate_rejects_bad_remap_and_placement() {
        let mut state = state_with_refs();
        let before = snapshot(&state);
        let good = plan(&state, 12, 0x30);
        // 누락.
        let mut missing = good.clone();
        missing.remap.pop();
        assert!(matches!(
            run(&mut state, Command::DuplicateSubtree(missing)),
            Err(CommandError::InvalidRequest { .. })
        ));
        // 새 id가 문서에서 사용 중(node 13).
        let mut used = good.clone();
        used.remap[1].1 = sid(13);
        assert_eq!(
            run(&mut state, Command::DuplicateSubtree(used)),
            Err(CommandError::DuplicateId { id: sid(13) })
        );
        // 새 id 중복.
        let mut dup = good.clone();
        dup.remap[2].1 = dup.remap[1].1;
        assert!(matches!(
            run(&mut state, Command::DuplicateSubtree(dup)),
            Err(CommandError::DuplicateId { .. })
        ));
        // 부모의 형제 key 충돌(12 아래에는 없음 → 10 아래 0x10 = 11).
        let mut collide = good.clone();
        collide.new_parent = sid(10);
        collide.order_key = key(0x10);
        assert!(matches!(
            run(&mut state, Command::DuplicateSubtree(collide)),
            Err(CommandError::DuplicateOrderKey { .. })
        ));
        assert_eq!(snapshot(&state), before);
        assert_eq!(node_ids(state.model()).len(), 7);
        // 자기 subtree 안으로의 복제는 새 node라 허용됩니다.
        let into_self = plan(&state, 20, 0x30);
        run(&mut state, Command::DuplicateSubtree(into_self)).unwrap();
        assert_eq!(node_ids(state.model()).len(), 10);
    }
}
