//! node 잠금 판정 (D8).
//!
//! 자신이나 조상이 잠긴 node는 바꾸지 않습니다. `locked` 자체는 조상이 잠기지 않았을 때만 바꿀 수 있어, 잠금
//! 해제는 그 node에 대한 명시 명령으로만 합니다. 숨김은 편집을 막지 않습니다.
//!
//! 잠금 규칙은 [`EditIntent::blocked_by`] 하나입니다. 앱(속성 탭·캔버스·Layers)은 [`edit_permission`] 또는
//! 조상 정보를 미리 계산한 [`NodeAccess::permits`]를, CLI apply는 적용 전 레코드로 [`NodeLock::resolve`]를 계산해
//! 같은 함수를 씁니다.
//!
//! 에이전트 편집 lease(XIX.4, ADR-0093)는 이 D8 잠금과 별개인 세션 한정 잠금입니다. 판정은 [`lease`], 같은 프로젝트의
//! 창·프로세스 사이 공유는 [`coordinator`]가 맡습니다.

/// 에이전트 편집 lease와 판정 (XIX.4.2~XIX.4.4, XIX.4.6, XIX.5.2).
pub mod lease;

/// 같은 프로젝트의 창·프로세스가 공유하는 lease 조정자 (XIX.4.5, XIX.4.7).
pub mod coordinator;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyKey;
use stellar_document::model::DocumentModel;
use stellar_document::records::NodePayload;

use crate::change::node_keys;

/// 사용자 편집의 잠금·root 판정 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditIntent {
    /// 영속 상태를 바꾸는 일반 편집(어느 속성인지 정하지 않음). `locked` 변경은 [`EditIntent::ToggleLock`]입니다.
    Edit,
    /// 영속 속성 하나를 변경합니다. 위치 속성(transform)은 흐름 부모 검사도 합니다.
    Modify(PropertyKey),
    /// 자신의 잠금 값을 변경합니다.
    ToggleLock,
    /// subtree를 삭제합니다.
    Delete,
    /// 위치를 변경합니다.
    Move,
    /// subtree를 복제합니다.
    Duplicate,
}

impl EditIntent {
    /// 이 편집을 자신·조상 잠금이 막는지(D8). `locked` 변경은 조상 잠금만 막습니다.
    pub fn blocked_by(self, lock: NodeLock) -> bool {
        match self {
            EditIntent::ToggleLock => lock.ancestor,
            EditIntent::Modify(key) => lock.blocks(key),
            EditIntent::Edit | EditIntent::Delete | EditIntent::Move | EditIntent::Duplicate => {
                lock.any()
            }
        }
    }

    fn positions(self) -> bool {
        match self {
            EditIntent::Move => true,
            EditIntent::Modify(key) => {
                key == stellar_document::frame::keys::TRANSFORM
                    || key == stellar_document::modifier_metadata::keys::TRANSFORM_MATRIX
            }
            _ => false,
        }
    }
}

/// 편집을 막는 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocked {
    /// 대상이 없습니다.
    Missing,
    /// 자신이나 조상이 잠겼습니다.
    Locked,
    /// root Frame에는 허용하지 않는 작업입니다.
    Root,
    /// 삭제할 subtree 안에 잠긴 노드가 있습니다.
    LockedDescendant,
    /// 흐름 레이아웃의 자식은 직접 배치할 수 없습니다.
    FlowParent,
}

/// node와 조상의 편집 권한. 숨김 상태는 검사하지 않습니다.
pub fn edit_permission(
    model: &DocumentModel,
    id: StableId,
    intent: EditIntent,
) -> Result<(), Blocked> {
    NodeAccess::of(model, id)
        .ok_or(Blocked::Missing)?
        .permits(intent)?;
    if matches!(intent, EditIntent::Delete) {
        let columns = model.columns();
        let mut pending = model.children_in_order(id);
        while let Some(child) = pending.pop() {
            let locked = model
                .runtime_id(child)
                .and_then(|handle| columns.get(handle))
                .is_some_and(|fields| fields.locked);
            if locked {
                return Err(Blocked::LockedDescendant);
            }
            pending.extend(model.children_in_order(child));
        }
    }
    Ok(())
}

/// node 하나의 편집 판정에 필요한 조상 정보(잠금·root·흐름 부모). 한 번 계산해 같은 node의 여러 속성을 판정하는
/// 호출자(속성 탭 snapshot)가 다시 씁니다. 계산 비용은 조상 수에 비례하고, NodeRecord를 만들지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeAccess {
    /// 자신·조상 잠금.
    pub lock: NodeLock,
    /// 부모가 없는 root Frame.
    pub root: bool,
    /// 조상 중 흐름 레이아웃(`layout_mode`가 있는 Frame)이 있음.
    pub flow_parent: bool,
}

impl NodeAccess {
    /// 문서에 있는 node의 판정 정보. 없으면 `None`입니다.
    pub fn of(model: &DocumentModel, id: StableId) -> Option<NodeAccess> {
        let columns = model.columns();
        let handle = model.runtime_id(id)?;
        let parent = columns.parent(handle);
        let mut flow_parent = false;
        let mut current = parent;
        let mut remaining = columns.len();
        while let Some(ancestor) = current {
            if remaining == 0 {
                break;
            }
            remaining -= 1;
            let ancestor_id = model.stable_id(ancestor)?;
            if matches!(model.payload(ancestor_id), Some(NodePayload::Frame(frame)) if frame.layout_mode.is_some())
            {
                flow_parent = true;
                break;
            }
            current = columns.parent(ancestor);
        }
        Some(NodeAccess {
            lock: NodeLock::of(model, id)?,
            root: parent.is_none(),
            flow_parent,
        })
    }

    /// root·잠금·흐름 부모 판정. 삭제의 잠긴 자손 검사는 subtree를 읽어야 하므로 [`edit_permission`]이 합니다.
    pub fn permits(self, intent: EditIntent) -> Result<(), Blocked> {
        if matches!(
            intent,
            EditIntent::Delete | EditIntent::Move | EditIntent::Duplicate
        ) && self.root
        {
            return Err(Blocked::Root);
        }
        if intent.blocked_by(self.lock) {
            return Err(Blocked::Locked);
        }
        if intent.positions() && self.flow_parent {
            return Err(Blocked::FlowParent);
        }
        Ok(())
    }
}

/// node 하나의 잠금 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NodeLock {
    /// 자신이 잠김.
    pub own: bool,
    /// 조상 중 하나가 잠김(자신 제외).
    pub ancestor: bool,
}

impl NodeLock {
    /// 문서에 있는 node의 잠금 상태. 문서에 없으면 `None`입니다. 비용은 조상 수에 비례합니다.
    pub fn of(model: &DocumentModel, id: StableId) -> Option<NodeLock> {
        let columns = model.columns();
        let handle = model.runtime_id(id)?;
        NodeLock::resolve(handle, columns.len(), |handle| {
            Some((columns.get(handle)?.locked, columns.parent(handle)))
        })
    }

    /// `lookup(id) = (자신 잠금, 부모)`로 잠금 상태를 계산합니다. `id`가 없으면 `None`입니다.
    ///
    /// 모델이 없는 호출자(CLI apply의 적용 전 레코드)도 같은 규칙을 쓰게 합니다. 검증한 문서에는 순환이 없지만,
    /// 잘못된 입력에서도 끝나도록 `limit`번 넘게 올라가면 조상이 잠긴 것으로 봅니다(보수적 판정).
    pub fn resolve<K: Copy>(
        id: K,
        limit: usize,
        mut lookup: impl FnMut(K) -> Option<(bool, Option<K>)>,
    ) -> Option<NodeLock> {
        let (own, mut parent) = lookup(id)?;
        let mut remaining = limit;
        while let Some(current) = parent {
            if remaining == 0 {
                return Some(NodeLock {
                    own,
                    ancestor: true,
                });
            }
            remaining -= 1;
            let Some((locked, next)) = lookup(current) else {
                break;
            };
            if locked {
                return Some(NodeLock {
                    own,
                    ancestor: true,
                });
            }
            parent = next;
        }
        Some(NodeLock {
            own,
            ancestor: false,
        })
    }

    /// 자신 또는 조상이 잠김.
    pub fn any(self) -> bool {
        self.own || self.ancestor
    }

    /// `key`를 바꾸는 편집을 막는지. `locked`는 조상 잠금만 막고, 그 밖은 자신 또는 조상 잠금이 막습니다.
    pub fn blocks(self, key: PropertyKey) -> bool {
        if key == node_keys::LOCKED {
            self.ancestor
        } else {
            self.any()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_fixtures::{definition, frame, sid};
    use stellar_document::frame::FramePayload;

    /// root(10) ─ 11(잠금) ─ 13, root ─ 12.
    fn model(lock: &[u64], flow: &[u64]) -> DocumentModel {
        let mut model = DocumentModel::new(definition());
        for (id, parent, key) in [
            (10, None, 0x10),
            (11, Some(10), 0x10),
            (12, Some(10), 0x20),
            (13, Some(11), 0x10),
        ] {
            let mut record = frame(id, parent, key);
            record.locked = lock.contains(&id);
            if flow.contains(&id) {
                record.payload = NodePayload::Frame(FramePayload {
                    layout_mode: Some(sid(90)),
                    ..FramePayload::default()
                });
            }
            model.insert_node(record).expect("node");
        }
        model
    }

    #[test]
    fn own_lock_blocks_edits_but_not_its_own_toggle() {
        let model = model(&[11], &[]);
        assert_eq!(
            edit_permission(&model, sid(11), EditIntent::Edit),
            Err(Blocked::Locked)
        );
        assert_eq!(
            edit_permission(&model, sid(11), EditIntent::ToggleLock),
            Ok(())
        );
        assert_eq!(
            edit_permission(&model, sid(11), EditIntent::Modify(node_keys::LOCKED)),
            Ok(())
        );
        assert_eq!(edit_permission(&model, sid(12), EditIntent::Edit), Ok(()));
    }

    #[test]
    fn ancestor_lock_blocks_every_intent_including_toggle() {
        let model = model(&[11], &[]);
        for intent in [
            EditIntent::Edit,
            EditIntent::ToggleLock,
            EditIntent::Modify(node_keys::DISPLAY_NAME),
            EditIntent::Delete,
            EditIntent::Move,
            EditIntent::Duplicate,
        ] {
            assert_eq!(
                edit_permission(&model, sid(13), intent),
                Err(Blocked::Locked),
                "{intent:?}"
            );
        }
    }

    #[test]
    fn root_and_missing_and_locked_descendant() {
        let model = model(&[13], &[]);
        assert_eq!(
            edit_permission(&model, sid(10), EditIntent::Delete),
            Err(Blocked::Root)
        );
        assert_eq!(edit_permission(&model, sid(10), EditIntent::Edit), Ok(()));
        assert_eq!(
            edit_permission(&model, sid(99), EditIntent::Edit),
            Err(Blocked::Missing)
        );
        assert_eq!(
            edit_permission(&model, sid(11), EditIntent::Delete),
            Err(Blocked::LockedDescendant)
        );
        assert_eq!(
            edit_permission(&model, sid(11), EditIntent::Duplicate),
            Ok(())
        );
    }

    #[test]
    fn flow_ancestor_blocks_only_position_edits() {
        let model = model(&[], &[11]);
        assert_eq!(
            edit_permission(&model, sid(13), EditIntent::Move),
            Err(Blocked::FlowParent)
        );
        assert_eq!(
            edit_permission(
                &model,
                sid(13),
                EditIntent::Modify(stellar_document::frame::keys::TRANSFORM)
            ),
            Err(Blocked::FlowParent)
        );
        assert_eq!(edit_permission(&model, sid(13), EditIntent::Edit), Ok(()));
        assert_eq!(edit_permission(&model, sid(12), EditIntent::Move), Ok(()));
    }

    #[test]
    fn resolve_matches_model_lock_and_treats_cycles_as_locked() {
        let model = model(&[11], &[]);
        let records: std::collections::HashMap<u64, (bool, Option<u64>)> = [
            (10, (false, None)),
            (11, (true, Some(10))),
            (12, (false, Some(10))),
            (13, (false, Some(11))),
        ]
        .into_iter()
        .collect();
        for id in [10, 11, 12, 13] {
            assert_eq!(
                NodeLock::resolve(id, records.len(), |id| records.get(&id).copied()),
                NodeLock::of(&model, sid(id)),
                "{id}"
            );
        }
        assert_eq!(
            NodeLock::resolve(99, 4, |id| records.get(&id).copied()),
            None
        );
        let cycle = |id: u64| Some((false, Some(if id == 1 { 2 } else { 1 })));
        assert!(NodeLock::resolve(1, 2, cycle).unwrap().ancestor);
    }
}
