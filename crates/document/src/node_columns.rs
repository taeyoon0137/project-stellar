//! 공통 node 필드의 SoA 저장 (III.6.2).
//!
//! [`NodeColumns`]는 모든 node가 공통으로 갖는 필드를 **Structure-of-Arrays(SoA)** column으로
//! 저장합니다. 각 필드는 별도 `Vec`이며 slot 인덱스로 정렬됩니다. 종류별(payload) 필드는 아직
//! 없습니다(후속 Phase VI).
//!
//! 핸들 수명은 [`RuntimeEntityAllocator`]가 관리하고, [`RuntimeEntityAllocator::live_slot`]으로
//! 살아 있는 핸들의 column 인덱스를 얻습니다. 모든 column은 항상 같은 길이를 유지합니다
//! (새 slot은 모든 column에 함께 push, 재사용 slot은 모든 column을 덮어쓰며, 길이는 줄이지
//! 않습니다).
//!
//! 파생 column `child_count`는 slot별로 **이 핸들을 부모로 가리키는 live node 수**를 유지합니다.
//! 삽입·부모 변경·제거가 갱신하므로 자식 유무를 전체 순회 없이 O(1)로 답합니다(leaf 삭제의 자식 확인,
//! posthoc CMD-DELETE-QUADRATIC). 부모 핸들이 그 시점에 live가 아니면(stale) 세지 않습니다 — stale
//! 핸들은 generation 때문에 다시 live가 되지 않으므로 순회 결과와 항상 같습니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::ids::{RuntimeEntityAllocator, RuntimeEntityId};
use stellar_core_types::order_key::OrderKey;

use crate::node_kind::NodeKind;

/// 한 node의 공통 필드 스냅샷.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeFields {
    /// 안정 식별자.
    pub stable_id: StableId,
    /// node 종류 (VI.2.1).
    pub kind: NodeKind,
    /// 부모 node 핸들(`None`이면 root).
    pub parent: Option<RuntimeEntityId>,
    /// 형제 순서 키.
    pub order_key: OrderKey,
    /// 표시 여부 (VI.2.2).
    pub visible: bool,
    /// 잠금 여부 (VI.2.2).
    pub locked: bool,
    /// 표시 이름(식별에 사용 금지, VI.1.3과 동일 정책).
    pub display_name: String,
}

/// 공통 node 필드를 SoA column으로 저장하는 최소 저장소(III.6.2).
#[derive(Debug, Default)]
pub struct NodeColumns {
    alloc: RuntimeEntityAllocator,
    // --- SoA columns (slot 인덱스 정렬, 항상 같은 길이) ---
    stable_id: Vec<StableId>,
    kind: Vec<NodeKind>,
    parent: Vec<Option<RuntimeEntityId>>,
    order_key: Vec<OrderKey>,
    visible: Vec<bool>,
    locked: Vec<bool>,
    display_name: Vec<String>,
    /// slot별 live 자식 수(파생 column, 모듈 문서 참조).
    child_count: Vec<usize>,
    /// slot별 현재 live 핸들(순회용, column과 같은 길이).
    handle_by_slot: Vec<Option<RuntimeEntityId>>,
    len: usize,
}

impl NodeColumns {
    /// 빈 저장소를 만듭니다.
    pub fn new() -> NodeColumns {
        NodeColumns {
            alloc: RuntimeEntityAllocator::new(),
            stable_id: Vec::new(),
            kind: Vec::new(),
            parent: Vec::new(),
            order_key: Vec::new(),
            visible: Vec::new(),
            locked: Vec::new(),
            display_name: Vec::new(),
            child_count: Vec::new(),
            handle_by_slot: Vec::new(),
            len: 0,
        }
    }

    /// `parent`가 live면 그 자식 수를 하나 늘리거나(`increment`) 줄입니다.
    fn adjust_child_count(&mut self, parent: Option<RuntimeEntityId>, increment: bool) {
        let Some(slot) = parent.and_then(|p| self.alloc.live_slot(p)) else {
            return;
        };
        let count = &mut self.child_count[slot];
        if increment {
            *count += 1;
        } else {
            debug_assert!(*count > 0, "자식 수 음수");
            *count = count.saturating_sub(1);
        }
    }

    /// node를 삽입하고 핸들을 반환합니다. 재사용 가능한 slot이 있으면 모든 column을 그 slot에
    /// 덮어쓰고, 없으면 모든 column에 함께 push 합니다(길이 일치 유지).
    pub fn insert(&mut self, fields: NodeFields) -> RuntimeEntityId {
        let id = self.alloc.allocate();
        let slot = self.alloc.live_slot(id).expect("갓 할당한 핸들은 live");
        let parent = fields.parent;
        if slot == self.stable_id.len() {
            self.stable_id.push(fields.stable_id);
            self.kind.push(fields.kind);
            self.parent.push(fields.parent);
            self.order_key.push(fields.order_key);
            self.visible.push(fields.visible);
            self.locked.push(fields.locked);
            self.display_name.push(fields.display_name);
            self.child_count.push(0);
            self.handle_by_slot.push(Some(id));
        } else {
            self.stable_id[slot] = fields.stable_id;
            self.kind[slot] = fields.kind;
            self.parent[slot] = fields.parent;
            self.order_key[slot] = fields.order_key;
            self.visible[slot] = fields.visible;
            self.locked[slot] = fields.locked;
            self.display_name[slot] = fields.display_name;
            self.child_count[slot] = 0;
            self.handle_by_slot[slot] = Some(id);
        }
        self.len += 1;
        self.adjust_child_count(parent, true);
        id
    }

    /// 핸들의 공통 필드 스냅샷을 반환합니다. stale 핸들이면 `None`.
    pub fn get(&self, id: RuntimeEntityId) -> Option<NodeFields> {
        let slot = self.alloc.live_slot(id)?;
        Some(NodeFields {
            stable_id: self.stable_id[slot],
            kind: self.kind[slot],
            parent: self.parent[slot],
            order_key: self.order_key[slot].clone(),
            visible: self.visible[slot],
            locked: self.locked[slot],
            display_name: self.display_name[slot].clone(),
        })
    }

    /// 부모를 갱신합니다. stale 핸들이면 `false`.
    pub fn set_parent(&mut self, id: RuntimeEntityId, parent: Option<RuntimeEntityId>) -> bool {
        match self.alloc.live_slot(id) {
            Some(slot) => {
                let previous = std::mem::replace(&mut self.parent[slot], parent);
                self.adjust_child_count(previous, false);
                self.adjust_child_count(parent, true);
                true
            }
            None => false,
        }
    }

    /// 이 핸들을 부모로 가리키는 live node 수(O(1)). stale 핸들이면 `None`.
    pub fn child_count(&self, id: RuntimeEntityId) -> Option<usize> {
        self.alloc.live_slot(id).map(|slot| self.child_count[slot])
    }

    /// 순서 키를 갱신합니다. stale 핸들이면 `false`.
    pub fn set_order_key(&mut self, id: RuntimeEntityId, order_key: OrderKey) -> bool {
        match self.alloc.live_slot(id) {
            Some(slot) => {
                self.order_key[slot] = order_key;
                true
            }
            None => false,
        }
    }

    /// 표시 여부를 갱신합니다. stale 핸들이면 `false`.
    pub fn set_visible(&mut self, id: RuntimeEntityId, visible: bool) -> bool {
        match self.alloc.live_slot(id) {
            Some(slot) => {
                self.visible[slot] = visible;
                true
            }
            None => false,
        }
    }

    /// 잠금 여부를 갱신합니다. stale 핸들이면 `false`.
    pub fn set_locked(&mut self, id: RuntimeEntityId, locked: bool) -> bool {
        match self.alloc.live_slot(id) {
            Some(slot) => {
                self.locked[slot] = locked;
                true
            }
            None => false,
        }
    }

    /// 표시·잠금 필드를 한 slot에서 함께 교체하고 이전 값을 돌려줍니다.
    pub(crate) fn replace_flags(
        &mut self,
        id: RuntimeEntityId,
        visible: bool,
        locked: bool,
    ) -> Option<(bool, bool)> {
        let slot = self.alloc.live_slot(id)?;
        let previous = (self.visible[slot], self.locked[slot]);
        self.visible[slot] = visible;
        self.locked[slot] = locked;
        Some(previous)
    }

    /// 표시 이름을 갱신합니다. stale 핸들이면 `false`.
    pub fn set_display_name(&mut self, id: RuntimeEntityId, display_name: String) -> bool {
        match self.alloc.live_slot(id) {
            Some(slot) => {
                self.display_name[slot] = display_name;
                true
            }
            None => false,
        }
    }

    /// 핸들이 살아 있는지 여부.
    pub fn contains(&self, id: RuntimeEntityId) -> bool {
        self.alloc.live_slot(id).is_some()
    }

    /// node를 제거합니다. column 데이터는 재사용 시 덮어쓰며 길이는 줄이지 않습니다.
    /// stale 핸들이면 `false`.
    pub fn remove(&mut self, id: RuntimeEntityId) -> bool {
        let Some(slot) = self.alloc.live_slot(id) else {
            return false;
        };
        let parent = self.parent[slot];
        self.handle_by_slot[slot] = None;
        self.alloc.deallocate(id);
        self.len -= 1;
        self.adjust_child_count(parent, false);
        true
    }

    /// 살아 있는 모든 핸들을 slot 순서로 순회합니다.
    ///
    /// slot 순서는 arena 내부 사정이므로 **문서 의미 순서가 아닙니다** —
    /// 자식 순서는 [`children_in_order`](crate::tree_ops::children_in_order)를
    /// 사용하세요(VI.2.4).
    pub fn iter_live(&self) -> impl Iterator<Item = RuntimeEntityId> + '_ {
        self.handle_by_slot.iter().filter_map(|h| *h).inspect(|_| {
            // 전체 순회 작업량 계수(테스트 빌드에서만, `op_count`).
            crate::op_count::tick(1);
        })
    }

    /// 살아 있는 node 수.
    pub fn len(&self) -> usize {
        self.len
    }

    /// 비어 있는지 여부.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(seed: u64, parent: Option<RuntimeEntityId>) -> NodeFields {
        NodeFields {
            stable_id: StableId::new_v7(seed, 0, seed),
            kind: NodeKind::Frame,
            parent,
            order_key: OrderKey::between(None, None),
            visible: true,
            locked: false,
            display_name: format!("node-{seed}"),
        }
    }

    /// 모든 column 길이가 일치하는지 확인(SoA 핵심 invariant).
    fn columns_aligned(c: &NodeColumns) -> bool {
        let n = c.stable_id.len();
        c.kind.len() == n
            && c.parent.len() == n
            && c.order_key.len() == n
            && c.visible.len() == n
            && c.locked.len() == n
            && c.display_name.len() == n
            && c.child_count.len() == n
            && c.handle_by_slot.len() == n
    }

    #[test]
    fn insert_get_remove_roundtrip() {
        let mut c = NodeColumns::new();
        assert!(c.is_empty());
        let root = c.insert(fields(1, None));
        let child = c.insert(fields(2, Some(root)));
        assert_eq!(c.len(), 2);

        let got = c.get(child).unwrap();
        assert_eq!(got.stable_id, StableId::new_v7(2, 0, 2));
        assert_eq!(got.parent, Some(root));

        assert!(c.remove(root));
        assert_eq!(c.len(), 1);
        assert!(c.get(root).is_none());
        assert!(!c.remove(root));
    }

    #[test]
    fn setters_update_and_reject_stale() {
        let mut c = NodeColumns::new();
        let a = c.insert(fields(1, None));
        let b = c.insert(fields(2, None));
        assert!(c.set_parent(b, Some(a)));
        assert_eq!(c.get(b).unwrap().parent, Some(a));

        let new_key = OrderKey::from_bytes(vec![0x40]).unwrap();
        assert!(c.set_order_key(b, new_key.clone()));
        assert_eq!(c.get(b).unwrap().order_key, new_key);

        c.remove(b);
        assert!(!c.set_parent(b, None));
        assert!(!c.set_order_key(b, new_key));
    }

    #[test]
    fn extended_common_fields_round_trip() {
        // 검증 기준(VI.2.2): column invariant와 round-trip.
        let mut c = NodeColumns::new();
        let id = c.insert(NodeFields {
            stable_id: StableId::new_v7(7, 0, 7),
            kind: NodeKind::Text,
            parent: None,
            order_key: OrderKey::between(None, None),
            visible: false,
            locked: true,
            display_name: "제목 텍스트".to_string(),
        });
        let got = c.get(id).unwrap();
        assert_eq!(got.kind, NodeKind::Text);
        assert!(!got.visible);
        assert!(got.locked);
        assert_eq!(got.display_name, "제목 텍스트");
    }

    #[test]
    fn new_setters_update_and_reject_stale() {
        let mut c = NodeColumns::new();
        let id = c.insert(fields(1, None));
        assert!(c.set_visible(id, false));
        assert!(c.set_locked(id, true));
        assert!(c.set_display_name(id, "새 이름".to_string()));
        let got = c.get(id).unwrap();
        assert!(!got.visible);
        assert!(got.locked);
        assert_eq!(got.display_name, "새 이름");
        // 이름 변경은 stable id에 영향 없음(VI.1.3 정책과 일관).
        assert_eq!(got.stable_id, StableId::new_v7(1, 0, 1));

        c.remove(id);
        assert!(!c.set_visible(id, true));
        assert!(!c.set_locked(id, false));
        assert!(!c.set_display_name(id, String::new()));
    }

    #[test]
    fn columns_aligned_after_insert_delete() {
        let mut c = NodeColumns::new();
        assert!(columns_aligned(&c));
        let mut live: Vec<RuntimeEntityId> = Vec::new();
        let mut rng: u64 = 0xDEAD_BEEF_CAFE_F00D;
        let mut seed = 0u64;

        for _ in 0..10_000 {
            // 결정론적 xorshift로 삽입/삭제 선택.
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            if live.is_empty() || rng.is_multiple_of(2) {
                seed += 1;
                let id = c.insert(fields(seed, None));
                live.push(id);
            } else {
                let idx = (rng as usize) % live.len();
                let id = live.swap_remove(idx);
                assert!(c.remove(id));
            }
            // 매 연산 후 모든 column 길이 일치.
            assert!(columns_aligned(&c), "column 길이 불일치");
        }
        assert!(columns_aligned(&c));
        assert_eq!(c.len(), live.len());
    }

    #[test]
    fn reused_slot_keeps_columns_aligned_and_invalidates_handle() {
        let mut c = NodeColumns::new();
        let a = c.insert(fields(1, None));
        c.remove(a);
        let b = c.insert(fields(2, None)); // slot 재사용.
        assert!(columns_aligned(&c));
        assert!(c.get(a).is_none(), "옛 핸들 무효");
        assert_eq!(c.get(b).unwrap().stable_id, StableId::new_v7(2, 0, 2));
    }

    /// 전체 순회로 센 자식 수(파생 column 비교 기준).
    fn scanned_children(c: &NodeColumns, parent: RuntimeEntityId) -> usize {
        c.iter_live()
            .filter(|id| c.get(*id).and_then(|f| f.parent) == Some(parent))
            .count()
    }

    #[test]
    fn child_count_matches_full_scan_under_random_edits() {
        // posthoc CMD-DELETE-QUADRATIC: 파생 자식 수는 삽입·부모 변경·제거(자식이 남은 부모 제거와 slot
        // 재사용 포함) 뒤에도 전체 순회 결과와 같아야 합니다.
        let mut c = NodeColumns::new();
        let mut live: Vec<RuntimeEntityId> = Vec::new();
        let mut rng: u64 = 0x5EED_C0DE_1234_5678;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        for seed in 0..3_000u64 {
            let r = next();
            let pick = |r: u64, live: &[RuntimeEntityId]| live[(r as usize >> 8) % live.len()];
            match r % 4 {
                0 | 1 => {
                    let parent = (!live.is_empty() && r % 8 != 0).then(|| pick(r, &live));
                    live.push(c.insert(fields(seed, parent)));
                }
                2 if !live.is_empty() => {
                    let node = pick(r, &live);
                    let parent = (r % 16 != 2).then(|| pick(next(), &live));
                    assert!(c.set_parent(node, parent));
                }
                _ if !live.is_empty() => {
                    // 자식이 있어도 column 수준에서는 지울 수 있습니다(자식의 부모는 stale).
                    let index = (r as usize >> 8) % live.len();
                    assert!(c.remove(live.swap_remove(index)));
                }
                _ => {}
            }
        }
        assert!(live.len() > 100);
        for id in &live {
            assert_eq!(c.child_count(*id), Some(scanned_children(&c, *id)));
        }
        let gone = c.insert(fields(9_999, None));
        c.remove(gone);
        assert_eq!(c.child_count(gone), None, "stale 핸들");
    }
}
