//! 참조 inbound/outbound index (VI.2.5).
//!
//! node 삭제 전 "누가 이 node를 참조하는가"(inbound)를 조회하는 인터페이스입니다.
//! Boolean/Mask/Clip modifier(VI.9.4), 컴포넌트 인스턴스 등 참조를 만드는
//! 주체가 edge를 등록·해제합니다.
//!
//! **삭제 정책은 여기서 처리하지 않습니다** — index는 사실만 보고하고,
//! 참조가 남은 node의 삭제 허용·차단·정리는 command 단계(Phase VII)의 몫입니다.

use std::collections::{BTreeMap, BTreeSet};

use stellar_core_types::ids::StableId;

/// 참조 edge의 정방향/역방향 index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReferenceIndex {
    /// from → to 집합(outbound).
    outbound: BTreeMap<StableId, BTreeSet<StableId>>,
    /// to → from 집합(inbound).
    inbound: BTreeMap<StableId, BTreeSet<StableId>>,
}

impl ReferenceIndex {
    /// 빈 index.
    pub fn new() -> ReferenceIndex {
        ReferenceIndex::default()
    }

    /// 참조 edge를 등록합니다. 이미 있으면 `false`.
    pub fn add_reference(&mut self, from: StableId, to: StableId) -> bool {
        let inserted = self.outbound.entry(from).or_default().insert(to);
        if inserted {
            self.inbound.entry(to).or_default().insert(from);
        }
        inserted
    }

    /// 참조 edge를 해제합니다. 없으면 `false`.
    pub fn remove_reference(&mut self, from: StableId, to: StableId) -> bool {
        let removed = self
            .outbound
            .get_mut(&from)
            .is_some_and(|set| set.remove(&to));
        if removed {
            if let Some(set) = self.inbound.get_mut(&to) {
                set.remove(&from);
                if set.is_empty() {
                    self.inbound.remove(&to);
                }
            }
            if self.outbound.get(&from).is_some_and(BTreeSet::is_empty) {
                self.outbound.remove(&from);
            }
        }
        removed
    }

    /// node가 만든 모든 outbound 참조를 해제합니다(node 삭제 준비).
    /// 해제된 대상 목록을 반환합니다.
    pub fn remove_all_from(&mut self, from: StableId) -> Vec<StableId> {
        let Some(targets) = self.outbound.remove(&from) else {
            return Vec::new();
        };
        for to in &targets {
            if let Some(set) = self.inbound.get_mut(to) {
                set.remove(&from);
                if set.is_empty() {
                    self.inbound.remove(to);
                }
            }
        }
        targets.into_iter().collect()
    }

    /// `to`를 참조하는 node 목록(inbound). 삭제 전 조회용입니다.
    pub fn inbound(&self, to: StableId) -> Vec<StableId> {
        self.inbound
            .get(&to)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// `from`이 참조하는 node 목록(outbound).
    pub fn outbound(&self, from: StableId) -> Vec<StableId> {
        self.outbound
            .get(&from)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// `to`로 들어오는 참조가 하나라도 있는지.
    pub fn has_inbound(&self, to: StableId) -> bool {
        self.inbound.contains_key(&to)
    }

    /// 전체 edge 수.
    pub fn edge_count(&self) -> usize {
        self.outbound.values().map(BTreeSet::len).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    #[test]
    fn inbound_and_outbound_fixtures_are_accurate() {
        // 검증 기준(VI.2.5): inbound/outbound fixture 정확.
        // mask(1)→target(3), boolean(2)→target(3), mask(1)→other(4).
        let mut index = ReferenceIndex::new();
        assert!(index.add_reference(stable(1), stable(3)));
        assert!(index.add_reference(stable(2), stable(3)));
        assert!(index.add_reference(stable(1), stable(4)));

        assert_eq!(index.inbound(stable(3)), vec![stable(1), stable(2)]);
        assert_eq!(index.inbound(stable(4)), vec![stable(1)]);
        assert_eq!(index.outbound(stable(1)), vec![stable(3), stable(4)]);
        assert_eq!(index.outbound(stable(2)), vec![stable(3)]);
        assert!(index.has_inbound(stable(3)));
        assert!(!index.has_inbound(stable(1)));
        assert_eq!(index.edge_count(), 3);
    }

    #[test]
    fn duplicate_edges_are_idempotent() {
        let mut index = ReferenceIndex::new();
        assert!(index.add_reference(stable(1), stable(2)));
        assert!(!index.add_reference(stable(1), stable(2)), "중복 등록");
        assert_eq!(index.edge_count(), 1);
        assert_eq!(index.inbound(stable(2)).len(), 1);
    }

    #[test]
    fn remove_reference_updates_both_directions() {
        let mut index = ReferenceIndex::new();
        index.add_reference(stable(1), stable(3));
        index.add_reference(stable(2), stable(3));

        assert!(index.remove_reference(stable(1), stable(3)));
        assert_eq!(index.inbound(stable(3)), vec![stable(2)]);
        assert!(index.outbound(stable(1)).is_empty());
        // 없는 edge 해제는 false.
        assert!(!index.remove_reference(stable(1), stable(3)));

        assert!(index.remove_reference(stable(2), stable(3)));
        assert!(!index.has_inbound(stable(3)));
        assert_eq!(index.edge_count(), 0);
    }

    #[test]
    fn remove_all_from_clears_outbound_only() {
        let mut index = ReferenceIndex::new();
        index.add_reference(stable(1), stable(3));
        index.add_reference(stable(1), stable(4));
        index.add_reference(stable(2), stable(3));

        // node 1 삭제 준비: 1의 outbound만 정리.
        let removed = index.remove_all_from(stable(1));
        assert_eq!(removed, vec![stable(3), stable(4)]);
        assert!(index.outbound(stable(1)).is_empty());
        // 남은 참조(2→3)는 유지.
        assert_eq!(index.inbound(stable(3)), vec![stable(2)]);
        assert!(!index.has_inbound(stable(4)));
    }

    #[test]
    fn deletion_decision_stays_with_caller() {
        // index는 사실만 보고 — 참조가 남아 있어도 아무 것도 막지 않습니다.
        let mut index = ReferenceIndex::new();
        index.add_reference(stable(1), stable(3));
        let inbound = index.inbound(stable(3));
        assert_eq!(inbound, vec![stable(1)]);
        // command 단계가 이 목록으로 정책(차단/정리/무시)을 결정합니다.
    }
}
