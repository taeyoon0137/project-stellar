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

use stellar_core_types::ids::StableId;
use stellar_core_types::ids::{RuntimeEntityAllocator, RuntimeEntityId};
use stellar_core_types::order_key::OrderKey;

/// 한 node의 공통 필드 스냅샷.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeFields {
    /// 안정 식별자.
    pub stable_id: StableId,
    /// 부모 node 핸들(`None`이면 root).
    pub parent: Option<RuntimeEntityId>,
    /// 형제 순서 키.
    pub order_key: OrderKey,
}

/// 공통 node 필드를 SoA column으로 저장하는 최소 저장소(III.6.2).
#[derive(Debug, Default)]
pub struct NodeColumns {
    alloc: RuntimeEntityAllocator,
    // --- SoA columns (slot 인덱스 정렬, 항상 같은 길이) ---
    stable_id: Vec<StableId>,
    parent: Vec<Option<RuntimeEntityId>>,
    order_key: Vec<OrderKey>,
    len: usize,
}

impl NodeColumns {
    /// 빈 저장소를 만듭니다.
    pub fn new() -> NodeColumns {
        NodeColumns {
            alloc: RuntimeEntityAllocator::new(),
            stable_id: Vec::new(),
            parent: Vec::new(),
            order_key: Vec::new(),
            len: 0,
        }
    }

    /// node를 삽입하고 핸들을 반환합니다. 재사용 가능한 slot이 있으면 모든 column을 그 slot에
    /// 덮어쓰고, 없으면 모든 column에 함께 push 합니다(길이 일치 유지).
    pub fn insert(&mut self, fields: NodeFields) -> RuntimeEntityId {
        let id = self.alloc.allocate();
        let slot = self.alloc.live_slot(id).expect("갓 할당한 핸들은 live");
        if slot == self.stable_id.len() {
            self.stable_id.push(fields.stable_id);
            self.parent.push(fields.parent);
            self.order_key.push(fields.order_key);
        } else {
            self.stable_id[slot] = fields.stable_id;
            self.parent[slot] = fields.parent;
            self.order_key[slot] = fields.order_key;
        }
        self.len += 1;
        id
    }

    /// 핸들의 공통 필드 스냅샷을 반환합니다. stale 핸들이면 `None`.
    pub fn get(&self, id: RuntimeEntityId) -> Option<NodeFields> {
        let slot = self.alloc.live_slot(id)?;
        Some(NodeFields {
            stable_id: self.stable_id[slot],
            parent: self.parent[slot],
            order_key: self.order_key[slot].clone(),
        })
    }

    /// 부모를 갱신합니다. stale 핸들이면 `false`.
    pub fn set_parent(&mut self, id: RuntimeEntityId, parent: Option<RuntimeEntityId>) -> bool {
        match self.alloc.live_slot(id) {
            Some(slot) => {
                self.parent[slot] = parent;
                true
            }
            None => false,
        }
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

    /// 핸들이 살아 있는지 여부.
    pub fn contains(&self, id: RuntimeEntityId) -> bool {
        self.alloc.live_slot(id).is_some()
    }

    /// node를 제거합니다. column 데이터는 재사용 시 덮어쓰며 길이는 줄이지 않습니다.
    /// stale 핸들이면 `false`.
    pub fn remove(&mut self, id: RuntimeEntityId) -> bool {
        if self.alloc.live_slot(id).is_none() {
            return false;
        }
        self.alloc.deallocate(id);
        self.len -= 1;
        true
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
            parent,
            order_key: OrderKey::between(None, None),
        }
    }

    /// 모든 column 길이가 일치하는지 확인(SoA 핵심 invariant).
    fn columns_aligned(c: &NodeColumns) -> bool {
        c.stable_id.len() == c.parent.len() && c.parent.len() == c.order_key.len()
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
}
