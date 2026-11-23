//! subtree input hash와 계산 간 배치 cache (X.7.2, ADR-0076).
//!
//! # 배치는 요청의 순수 함수
//!
//! node 하나의 배치(크기·baseline·자식 배치)는 자신의 **subtree 입력**과 요청(부모가 정한 크기, 백분율 기준,
//! 가용 공간, 콘텐츠만 측정하는지 여부)만의 함수입니다. 부모·형제 문맥은 요청으로만 들어옵니다. 그래서 (node,
//! subtree input hash, 요청, 측정기 revision)이 같으면 결과가 같고, [`MeasureCache`]가 계산 사이에 재사용합니다.
//! intrinsic 측정(min-content·max-content 요청)과 최종 배치가 같은 cache를 씁니다.
//!
//! # subtree input hash
//!
//! [`subtree_hashes`]는 post-order(자식 먼저, 반복)로 node 자신의 hash(transform을 뺀 node 레이아웃 입력
//! canonical bytes의 hash) + 자식 수 + visual order 자식 subtree hash를 합성합니다. transform은 레이아웃 결과에
//! 영향을 주지 않으므로(X.2.4) 빠집니다. node 자신의 hash를 따로 두므로 증분 레이아웃(X.7.3)은 바뀐 node와 조상만
//! 다시 합성합니다. hash는 [`LayoutInputHasher`](primary·check 두 값) 기반 런타임 key이며 저장하지 않습니다.

use std::collections::HashMap;
use std::sync::Arc;

use stellar_core_types::ids::StableId;

use crate::cache::{LayoutCacheStats, LayoutInputHash, LayoutInputHasher};
use crate::engine::{NodeLayout, Request};
use crate::fast_hash::FastMap;
use crate::input::{node_layout_bytes, LayoutInput, LayoutNode, TreeIndex};

/// node 자신의 레이아웃 입력 hash(자식 subtree·transform과 무관).
pub(crate) fn node_hash(node: &LayoutNode) -> LayoutInputHash {
    let mut hasher = LayoutInputHasher::new();
    hasher.write_bytes(&node_layout_bytes(node));
    hasher.finish()
}

/// node 자신의 hash와 visual order 자식 subtree hash를 합성한 subtree hash.
pub(crate) fn combine_subtree(
    own: LayoutInputHash,
    children: impl ExactSizeIterator<Item = LayoutInputHash>,
) -> LayoutInputHash {
    let mut hasher = LayoutInputHasher::new();
    hasher.write_hash(own);
    hasher.write_u64(children.len() as u64);
    for child in children {
        hasher.write_hash(child);
    }
    hasher.finish()
}

/// 모든 node의 subtree input hash(입력 `nodes` 순서).
pub(crate) fn subtree_hashes(input: &LayoutInput, tree: &TreeIndex) -> Vec<LayoutInputHash> {
    let own: Vec<LayoutInputHash> = input.nodes.iter().map(node_hash).collect();
    subtree_hashes_from(&own, tree)
}

/// node 자신의 hash(`own`, 입력 순서)로 모든 subtree hash를 계산합니다.
pub(crate) fn subtree_hashes_from(
    own: &[LayoutInputHash],
    tree: &TreeIndex,
) -> Vec<LayoutInputHash> {
    let count = own.len();
    let mut order = Vec::with_capacity(count);
    let mut stack = vec![tree.root];
    while let Some(index) = stack.pop() {
        order.push(index);
        stack.extend(tree.children[index].iter().copied());
    }
    let mut hashes = vec![LayoutInputHash::from_parts(0, 0); count];
    for &index in order.iter().rev() {
        let children = &tree.children[index];
        hashes[index] = combine_subtree(own[index], children.iter().map(|&child| hashes[child]));
    }
    hashes
}

/// node 하나의 배치 결과(같은 subtree hash·측정기 revision).
#[derive(Debug, Clone)]
struct NodeEntries {
    subtree: LayoutInputHash,
    measurer: u64,
    layouts: FastMap<Request, Arc<NodeLayout>>,
}

/// 계산 간 배치 cache (X.7.2).
///
/// [`crate::engine::compute_layout_cached`]에 같은 cache를 넘기면 이전 계산의 배치 결과를 key가 같을 때 재사용합니다.
/// node마다 한 subtree hash·측정기 revision의 결과만 보관하며, 다른 hash로 저장하면 그 node의 이전 결과를 버립니다.
/// 계산이 성공하면 현재 입력에 없는 node·subtree hash가 바뀐 node·다른 측정기 revision의 entry를 지웁니다. 순회
/// API가 없어 내부 map 순서는 결과에 영향을 주지 않습니다.
#[derive(Debug, Clone, Default)]
pub struct MeasureCache {
    nodes: FastMap<StableId, NodeEntries>,
    len: usize,
    stats: LayoutCacheStats,
}

impl MeasureCache {
    /// 빈 cache.
    pub fn new() -> MeasureCache {
        MeasureCache::default()
    }

    /// entry(node·요청 쌍) 수.
    pub fn len(&self) -> usize {
        self.len
    }

    /// 비어 있는지 여부.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// 모든 entry를 지웁니다(집계는 유지).
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.len = 0;
    }

    /// 지금까지의 hit/miss 집계.
    pub const fn stats(&self) -> LayoutCacheStats {
        self.stats
    }

    /// 배치 결과를 찾고 hit/miss를 집계합니다.
    pub(crate) fn get(
        &mut self,
        node: StableId,
        subtree: LayoutInputHash,
        measurer: u64,
        request: &Request,
    ) -> Option<Arc<NodeLayout>> {
        let found = self
            .nodes
            .get(&node)
            .filter(|entry| entry.subtree == subtree && entry.measurer == measurer)
            .and_then(|entry| entry.layouts.get(request).cloned());
        match found {
            Some(_) => self.stats.hits = self.stats.hits.saturating_add(1),
            None => self.stats.misses = self.stats.misses.saturating_add(1),
        }
        found
    }

    /// 배치 결과를 저장합니다. 같은 node의 다른 subtree hash·revision 결과는 버립니다.
    pub(crate) fn insert(
        &mut self,
        node: StableId,
        subtree: LayoutInputHash,
        measurer: u64,
        request: Request,
        layout: Arc<NodeLayout>,
    ) {
        let entry = self.nodes.entry(node).or_insert_with(|| NodeEntries {
            subtree,
            measurer,
            layouts: FastMap::default(),
        });
        if entry.subtree != subtree || entry.measurer != measurer {
            self.len -= entry.layouts.len();
            entry.layouts.clear();
            entry.subtree = subtree;
            entry.measurer = measurer;
        }
        if entry.layouts.insert(request, layout).is_none() {
            self.len += 1;
        }
    }

    /// node의 모든 entry를 지웁니다(증분 레이아웃에서 삭제된 node, X.7.3).
    pub(crate) fn remove_node(&mut self, node: StableId) {
        if let Some(entry) = self.nodes.remove(&node) {
            self.len -= entry.layouts.len();
        }
    }

    /// 현재 입력(node → subtree hash)과 측정기 revision에 맞지 않는 entry를 지웁니다.
    pub(crate) fn retain_current(
        &mut self,
        current: &HashMap<StableId, LayoutInputHash>,
        measurer: u64,
    ) {
        let mut removed = 0;
        self.nodes.retain(|node, entry| {
            let keep = entry.measurer == measurer && current.get(node) == Some(&entry.subtree);
            if !keep {
                removed += entry.layouts.len();
            }
            keep
        });
        self.len -= removed;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;

    use super::*;
    use crate::engine::{compute_layout_cached, compute_layout_with};
    use crate::intrinsic::tests::{image, text};
    use crate::intrinsic::{
        ContentMeasurer, IntrinsicMeasure, IntrinsicSize, MeasureError, MeasureRequest,
        MeasuredContent,
    };
    use crate::style::{Display, FlexDirection, LayoutStyle, Size};
    use crate::test_support::{id, key, len};
    use stellar_core_types::value::Transform2D;

    /// 요청별 호출 수를 세고 기본 구현에 위임하는 측정기.
    #[derive(Default)]
    struct Counting {
        calls: Mutex<HashMap<MeasureRequest, u32>>,
        revision: u64,
    }

    impl Counting {
        fn with_revision(revision: u64) -> Counting {
            Counting {
                calls: Mutex::new(HashMap::new()),
                revision,
            }
        }

        fn calls(&self) -> HashMap<MeasureRequest, u32> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl ContentMeasurer for Counting {
        fn measure(
            &self,
            request: &MeasureRequest,
            content: &MeasuredContent,
        ) -> Result<IntrinsicSize, MeasureError> {
            *self.calls.lock().unwrap().entry(*request).or_insert(0) += 1;
            content.measure(request)
        }

        fn revision(&self) -> u64 {
            self.revision
        }
    }

    fn style(f: impl FnOnce(&mut LayoutStyle)) -> LayoutStyle {
        let mut style = LayoutStyle::default();
        f(&mut style);
        style
    }

    /// `width: fit-content` 세로 flex root(1) 아래 text(2), 너비 30 text(3), 너비 40 image(4),
    /// 가로 flex(5: text 6, image 7).
    fn document(first_words: &[f32]) -> LayoutInput {
        LayoutInput::new(
            id(1),
            vec![
                LayoutNode::new(id(1))
                    .with_style(style(|s| {
                        s.display = Display::Flex;
                        s.flex_direction = FlexDirection::Column;
                        s.width = Size::FitContent;
                    }))
                    .with_child(id(2), key(1))
                    .with_child(id(3), key(2))
                    .with_child(id(4), key(3))
                    .with_child(id(5), key(4)),
                LayoutNode::new(id(2)).with_content(text(first_words)),
                LayoutNode::new(id(3))
                    .with_style(style(|s| s.width = Size::Length(len(30.0))))
                    .with_content(text(&[10.0, 20.0, 15.0])),
                LayoutNode::new(id(4))
                    .with_style(style(|s| s.width = Size::Length(len(40.0))))
                    .with_content(image(80.0, 60.0)),
                LayoutNode::new(id(5))
                    .with_style(style(|s| s.display = Display::Flex))
                    .with_child(id(6), key(1))
                    .with_child(id(7), key(2)),
                LayoutNode::new(id(6)).with_content(text(&[12.0, 8.0])),
                LayoutNode::new(id(7)).with_content(image(16.0, 9.0)),
            ],
        )
    }

    #[test]
    fn same_input_never_measures_twice() {
        let input = document(&[10.0, 20.0]);
        let counting = Counting::default();
        let plain = compute_layout_with(&input, &counting).unwrap();
        let calls = counting.calls();
        assert!(!calls.is_empty());
        assert!(calls.values().all(|&n| n == 1), "{calls:?}");

        let mut cache = MeasureCache::new();
        let counting = Counting::default();
        let first = compute_layout_cached(&input, &counting, &mut cache).unwrap();
        let calls = counting.calls();
        assert!(calls.values().all(|&n| n == 1), "{calls:?}");
        assert_eq!(first.stats.measurer_calls, calls.len() as u64);
        assert!(first.stats.memo_hits > 0);
        assert_eq!(first.stats.cache_hits, 0);
        assert_eq!(first.stats.arranged, 7);
        assert_eq!(first.result, plain);
        assert_eq!(cache.len() as u64, first.stats.measured);

        // 같은 입력의 두 번째 계산: 새 배치 0, 측정기 호출 0, 결과 bytes 동일.
        let counting = Counting::default();
        let second = compute_layout_cached(&input, &counting, &mut cache).unwrap();
        assert!(counting.calls().is_empty());
        assert_eq!(second.stats.measured, 0);
        assert_eq!(second.stats.measurer_calls, 0);
        assert_eq!(second.stats.arranged, 0);
        assert!(second.stats.cache_hits > 0);
        assert_eq!(second.result.encode(), first.result.encode());
        assert_eq!(cache.stats().hits, second.stats.cache_hits);
    }

    #[test]
    fn leaf_change_remeasures_only_affected_nodes() {
        let mut cache = MeasureCache::new();
        let first =
            compute_layout_cached(&document(&[10.0, 20.0]), &Counting::default(), &mut cache)
                .unwrap();
        // text(2)만 바꾸면 측정기는 node 2만 다시 측정하고 형제 subtree는 cache를 씁니다.
        let edited = document(&[10.0, 25.0]);
        let counting = Counting::default();
        let run = compute_layout_cached(&edited, &counting, &mut cache).unwrap();
        let calls = counting.calls();
        assert!(!calls.is_empty());
        assert!(
            calls.keys().all(|request| request.node == id(2)),
            "{calls:?}"
        );
        assert!(run.stats.cache_hits > 0);
        assert!(run.stats.measured < first.stats.measured);
        assert_eq!(
            run.result,
            compute_layout_with(&edited, &Counting::default()).unwrap()
        );
        // 측정기 revision이 바뀌면 모든 측정 콘텐츠를 다시 측정합니다.
        let counting = Counting::with_revision(1);
        let run = compute_layout_cached(&edited, &counting, &mut cache).unwrap();
        let measured_nodes: HashSet<StableId> = counting.calls().keys().map(|r| r.node).collect();
        assert_eq!(measured_nodes.len(), 5);
        assert_eq!(run.stats.cache_hits, 0);
        // 다른 입력으로 계산하면 그 입력에 맞지 않는 entry가 지워집니다.
        let other = LayoutInput::new(
            id(9),
            vec![LayoutNode::new(id(9)).with_content(text(&[1.0]))],
        );
        let run = compute_layout_cached(&other, &Counting::default(), &mut cache).unwrap();
        assert_eq!(cache.len() as u64, run.stats.measured);
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn subtree_hash_changes_on_ancestor_path_only_and_ignores_transform() {
        let hashes = |input: &LayoutInput| {
            let tree = input.tree().unwrap();
            let by_id: HashMap<StableId, LayoutInputHash> = input
                .nodes
                .iter()
                .map(|n| n.id)
                .zip(subtree_hashes(input, &tree))
                .collect();
            by_id
        };
        let base = hashes(&document(&[10.0, 20.0]));
        let edited = hashes(&document(&[10.0, 25.0]));
        for n in 1..=7u64 {
            assert_eq!(base[&id(n)] != edited[&id(n)], n == 1 || n == 2, "node {n}");
        }
        assert_eq!(base, hashes(&document(&[10.0, 20.0])));
        // transform은 레이아웃 입력 hash에 들어가지 않습니다.
        let mut moved = document(&[10.0, 20.0]);
        moved.nodes[1].transform = Transform2D::new(1.0, 0.0, 0.0, 1.0, 5.0, 5.0).unwrap();
        assert_eq!(base, hashes(&moved));
    }
}
