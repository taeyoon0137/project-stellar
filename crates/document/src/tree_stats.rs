//! node count·depth incremental metadata 훅 (VI.2.6).
//!
//! [`TreeStats`]는 insert/move/delete 훅으로 node 수와 depth 정보를 유지합니다:
//!
//! - node count: O(1) 증감.
//! - depth: node별 depth map + depth 히스토그램. 최대 depth 조회는 히스토그램
//!   끝을 읽어 O(log d)이고, reparent 시에는 이동한 subtree만 재계산합니다
//!   (incremental).
//!
//! **실제 성능 최적화는 후속 작업입니다** — 이 모듈은 정확한 값을 유지하는 훅
//! 구조를 고정하는 것이 목적이고, 훅 계약(어느 연산 후 무엇을 호출하는가)은
//! 유지한 채 내부 자료구조를 교체할 수 있습니다.

use std::collections::{BTreeMap, HashMap};

use stellar_core_types::ids::RuntimeEntityId;

use crate::node_columns::NodeColumns;

/// tree 통계 incremental metadata.
#[derive(Debug, Default)]
pub struct TreeStats {
    node_count: usize,
    depth_of: HashMap<RuntimeEntityId, u32>,
    /// depth → 그 depth의 node 수.
    histogram: BTreeMap<u32, usize>,
}

impl TreeStats {
    /// 빈 통계.
    pub fn new() -> TreeStats {
        TreeStats::default()
    }

    /// 현재 node 수(O(1)).
    pub fn node_count(&self) -> usize {
        self.node_count
    }

    /// 최대 depth(root=0). 비어 있으면 `None`.
    pub fn max_depth(&self) -> Option<u32> {
        self.histogram.keys().next_back().copied()
    }

    /// node의 depth.
    pub fn depth_of(&self, id: RuntimeEntityId) -> Option<u32> {
        self.depth_of.get(&id).copied()
    }

    fn set_depth(&mut self, id: RuntimeEntityId, depth: u32) {
        if let Some(old) = self.depth_of.insert(id, depth) {
            self.decrement_histogram(old);
        }
        *self.histogram.entry(depth).or_default() += 1;
    }

    fn decrement_histogram(&mut self, depth: u32) {
        if let Some(count) = self.histogram.get_mut(&depth) {
            *count -= 1;
            if *count == 0 {
                self.histogram.remove(&depth);
            }
        }
    }

    /// insert 직후 훅. 부모의 depth + 1(root는 0)로 기록합니다.
    pub fn on_insert(&mut self, columns: &NodeColumns, id: RuntimeEntityId) {
        let depth = columns
            .get(id)
            .and_then(|f| f.parent)
            .and_then(|p| self.depth_of.get(&p).copied())
            .map_or(0, |d| d + 1);
        self.node_count += 1;
        self.set_depth(id, depth);
    }

    /// remove 직전 훅(leaf 전제 — 자식 있는 subtree 삭제는 자식부터 훅 호출).
    pub fn on_remove(&mut self, id: RuntimeEntityId) {
        if let Some(depth) = self.depth_of.remove(&id) {
            self.node_count -= 1;
            self.decrement_histogram(depth);
        }
    }

    /// reparent 직후 훅. 이동한 subtree의 depth만 재계산합니다(incremental).
    pub fn on_reparent(&mut self, columns: &NodeColumns, id: RuntimeEntityId) {
        let Some(fields) = columns.get(id) else {
            return;
        };
        let new_depth = fields
            .parent
            .and_then(|p| self.depth_of.get(&p).copied())
            .map_or(0, |d| d + 1);
        // subtree BFS: 자신 + 자손의 depth 갱신.
        let mut queue = vec![(id, new_depth)];
        while let Some((node, depth)) = queue.pop() {
            self.set_depth(node, depth);
            for child in crate::tree_ops::children_in_order(columns, node) {
                queue.push((child, depth + 1));
            }
        }
    }
}

/// 전체 재계산(검증용 기준값). 훅 유지 결과와 일치해야 합니다.
pub fn compute_stats(columns: &NodeColumns) -> TreeStats {
    let mut stats = TreeStats::new();
    // depth는 parent chain으로 직접 계산.
    fn depth_by_chain(columns: &NodeColumns, id: RuntimeEntityId) -> u32 {
        let mut depth = 0;
        let mut current = columns.get(id).and_then(|f| f.parent);
        while let Some(parent) = current {
            depth += 1;
            current = columns.get(parent).and_then(|f| f.parent);
        }
        depth
    }
    for id in columns.iter_live() {
        let depth = depth_by_chain(columns, id);
        stats.node_count += 1;
        stats.set_depth(id, depth);
    }
    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node_columns::NodeFields;
    use crate::node_kind::NodeKind;
    use crate::tree_ops::reparent;
    use stellar_core_types::ids::StableId;
    use stellar_core_types::order_key::OrderKey;

    fn fields(seed: u64, parent: Option<RuntimeEntityId>) -> NodeFields {
        NodeFields {
            stable_id: StableId::new_v7(seed, 0, seed),
            kind: NodeKind::Frame,
            parent,
            order_key: OrderKey::from_bytes(vec![(seed % 250) as u8 + 1]).unwrap(),
            visible: true,
            locked: false,
            display_name: String::new(),
        }
    }

    fn assert_matches_recompute(stats: &TreeStats, columns: &NodeColumns) {
        let reference = compute_stats(columns);
        assert_eq!(stats.node_count(), reference.node_count());
        assert_eq!(stats.max_depth(), reference.max_depth());
        for id in columns.iter_live() {
            assert_eq!(stats.depth_of(id), reference.depth_of(id), "{id:?}");
        }
    }

    #[test]
    fn insert_move_delete_keep_count_and_depth_accurate() {
        // 검증 기준(VI.2.6): insert/move/delete 후 count/depth 정확.
        let mut columns = NodeColumns::new();
        let mut stats = TreeStats::new();

        // insert: root ← a ← b, root ← c.
        let root = columns.insert(fields(1, None));
        stats.on_insert(&columns, root);
        let a = columns.insert(fields(2, Some(root)));
        stats.on_insert(&columns, a);
        let b = columns.insert(fields(3, Some(a)));
        stats.on_insert(&columns, b);
        let c = columns.insert(fields(4, Some(root)));
        stats.on_insert(&columns, c);

        assert_eq!(stats.node_count(), 4);
        assert_eq!(stats.max_depth(), Some(2));
        assert_eq!(stats.depth_of(b), Some(2));
        assert_matches_recompute(&stats, &columns);

        // move: a(및 자손 b)를 c 아래로 → b의 depth 3.
        reparent(&mut columns, a, c).unwrap();
        stats.on_reparent(&columns, a);
        assert_eq!(stats.depth_of(a), Some(2));
        assert_eq!(stats.depth_of(b), Some(3));
        assert_eq!(stats.max_depth(), Some(3));
        assert_matches_recompute(&stats, &columns);

        // delete: leaf b 제거 → 최대 depth 2로 감소.
        stats.on_remove(b);
        columns.remove(b);
        assert_eq!(stats.node_count(), 3);
        assert_eq!(stats.max_depth(), Some(2));
        assert_matches_recompute(&stats, &columns);
    }

    #[test]
    fn randomized_operations_match_recompute() {
        let mut columns = NodeColumns::new();
        let mut stats = TreeStats::new();
        let root = columns.insert(fields(1, None));
        stats.on_insert(&columns, root);
        let mut live = vec![root];
        let mut rng: u64 = 0x5EED_5EED_5EED_5EED;
        let mut seed = 1u64;

        for step in 0..500 {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            match rng % 3 {
                0 => {
                    // insert: 임의 부모 아래.
                    seed += 1;
                    let parent = live[(rng >> 8) as usize % live.len()];
                    let id = columns.insert(fields(seed, Some(parent)));
                    stats.on_insert(&columns, id);
                    live.push(id);
                }
                1 if live.len() > 1 => {
                    // move: 유효한 reparent만 시도.
                    let node = live[(rng >> 8) as usize % live.len()];
                    let target = live[(rng >> 24) as usize % live.len()];
                    if reparent(&mut columns, node, target).is_ok() {
                        stats.on_reparent(&columns, node);
                    }
                }
                _ if live.len() > 1 => {
                    // delete: leaf만.
                    let candidates: Vec<_> = live
                        .iter()
                        .copied()
                        .filter(|id| {
                            *id != root
                                && crate::tree_ops::children_in_order(&columns, *id).is_empty()
                        })
                        .collect();
                    if let Some(&victim) =
                        candidates.get((rng >> 16) as usize % candidates.len().max(1))
                    {
                        stats.on_remove(victim);
                        columns.remove(victim);
                        live.retain(|id| *id != victim);
                    }
                }
                _ => {}
            }
            if step % 50 == 0 {
                assert_matches_recompute(&stats, &columns);
            }
        }
        assert_matches_recompute(&stats, &columns);
    }
}
