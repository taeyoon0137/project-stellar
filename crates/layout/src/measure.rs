//! 측정 cache key와 계산 간 측정 cache (X.7.2).
//!
//! # 측정과 배치의 분리
//!
//! 엔진은 두 단계를 나눕니다.
//!
//! 1. **측정(bottom-up)**: node의 content 크기·baseline은 자신의 **subtree 입력**과 요청(축별 가용 content
//!    크기, [`IntrinsicMode`])만의 함수입니다. 자식 측정 결과를 위로 합성하며 배치·진단을 만들지 않습니다.
//!    그래서 [`MeasureKey`]로 cache할 수 있습니다.
//! 2. **배치(top-down)**: 부모가 정한 rect로 자식 위치를 정하고, 필요한 크기는 측정 결과를 읽습니다. 진단
//!    (constraint·HugContent 순환)은 배치에서만 만듭니다.
//!
//! 같은 계산 안에서 같은 측정 요청은 한 번만 계산하고(in-run memo), [`MeasureCache`]를 넘기면 계산 사이에도
//! 재사용합니다. 같은 입력·같은 측정기 revision의 두 번째 계산은 측정을 하나도 다시 하지 않습니다.
//!
//! # cache key
//!
//! [`MeasureKey`] = (node [`StableId`], subtree input hash, 축별 가용 content 크기, mode, 측정기 revision).
//! subtree input hash는 [`subtree_hashes`]가 post-order(자식 먼저, 반복)로 구합니다: node 자신의 hash(입력
//! canonical bytes의 node 부분의 hash) + 자식 수 + visual order 자식 subtree hash입니다. node 자신의 hash를 따로
//! 두므로 증분 레이아웃(X.7.3)은 바뀐 node와 조상만 다시 합성합니다. 측정은 subtree 밖의 값에 의존하지 않으므로
//! (부모 문맥은 요청의 가용 크기로만 들어옴) 이 key가 같으면 결과가 같습니다. hash는 [`LayoutInputHasher`]
//! (primary·check 두 값) 기반 런타임 key이며 저장하지 않습니다.

use std::collections::HashMap;

use stellar_core_types::ids::StableId;

use crate::cache::{LayoutCacheStats, LayoutInputHash, LayoutInputHasher};
use crate::input::{node_bytes, LayoutInput, LayoutNode, TreeIndex};
use crate::intrinsic::{IntrinsicMode, IntrinsicSize};
use crate::rounding::LayoutUnit;

/// 측정 결과 cache key (X.7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MeasureKey {
    /// 측정 node.
    pub node: StableId,
    /// node subtree 입력 hash.
    pub subtree: LayoutInputHash,
    /// 축별 가용 content 크기(definite 축만 `Some`).
    pub available: [Option<LayoutUnit>; 2],
    /// indefinite 축의 측정 기준.
    pub mode: IntrinsicMode,
    /// 측정기 revision([`crate::intrinsic::ContentMeasurer::revision`]).
    pub measurer: u64,
}

/// node 자신의 입력 hash(자식 subtree와 무관, node canonical bytes 기준).
pub(crate) fn node_hash(node: &LayoutNode) -> LayoutInputHash {
    let mut hasher = LayoutInputHasher::new();
    hasher.write_bytes(&node_bytes(node));
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

/// 모든 node의 subtree input hash(입력 `nodes` 순서). 검증된 tree에서 post-order로 반복 계산합니다.
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
    // pre-order 목록을 만든 뒤 역순으로 처리하면 자식이 항상 부모보다 먼저 계산됩니다.
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

/// node 하나의 측정 결과(같은 subtree hash·측정기 revision).
#[derive(Debug, Clone)]
struct NodeMeasures {
    subtree: LayoutInputHash,
    measurer: u64,
    sizes: HashMap<([Option<LayoutUnit>; 2], IntrinsicMode), IntrinsicSize>,
}

/// 계산 간 측정 cache (X.7.2).
///
/// [`crate::engine::compute_layout_cached`]에 같은 cache를 넘기면 이전 계산의 측정 결과를 key가 같을 때
/// 재사용합니다. node마다 한 subtree hash·측정기 revision의 결과만 보관하며, 다른 hash로 저장하면 그 node의
/// 이전 결과를 버립니다. 계산이 성공하면 현재 입력에 없는 node·subtree hash가 바뀐 node·다른 측정기 revision의
/// entry를 지워 크기를 현재 입력의 측정 수로 제한합니다. 순회 API가 없어 내부 map 순서는 결과에 영향을
/// 주지 않습니다.
#[derive(Debug, Clone, Default)]
pub struct MeasureCache {
    nodes: HashMap<StableId, NodeMeasures>,
    len: usize,
    stats: LayoutCacheStats,
}

impl MeasureCache {
    /// 빈 cache.
    pub fn new() -> MeasureCache {
        MeasureCache::default()
    }

    /// entry 수.
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

    /// key의 측정 결과를 찾고 hit/miss를 집계합니다.
    pub(crate) fn get(&mut self, key: &MeasureKey) -> Option<IntrinsicSize> {
        let found = self
            .nodes
            .get(&key.node)
            .filter(|entry| entry.subtree == key.subtree && entry.measurer == key.measurer)
            .and_then(|entry| entry.sizes.get(&(key.available, key.mode)).copied());
        match found {
            Some(_) => self.stats.hits = self.stats.hits.saturating_add(1),
            None => self.stats.misses = self.stats.misses.saturating_add(1),
        }
        found
    }

    /// 측정 결과를 저장합니다. 같은 node의 다른 subtree hash·revision 결과는 버립니다.
    pub(crate) fn insert(&mut self, key: MeasureKey, size: IntrinsicSize) {
        let entry = self.nodes.entry(key.node).or_insert_with(|| NodeMeasures {
            subtree: key.subtree,
            measurer: key.measurer,
            sizes: HashMap::new(),
        });
        if entry.subtree != key.subtree || entry.measurer != key.measurer {
            self.len -= entry.sizes.len();
            entry.sizes.clear();
            entry.subtree = key.subtree;
            entry.measurer = key.measurer;
        }
        if entry
            .sizes
            .insert((key.available, key.mode), size)
            .is_none()
        {
            self.len += 1;
        }
    }

    /// node의 모든 entry를 지웁니다(증분 레이아웃에서 삭제된 node, X.7.3).
    pub(crate) fn remove_node(&mut self, node: StableId) {
        if let Some(entry) = self.nodes.remove(&node) {
            self.len -= entry.sizes.len();
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
                removed += entry.sizes.len();
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
    use crate::geometry::Axis;
    use crate::grid::{GridItem, GridLayout, TrackSize};
    use crate::input::{ContainerLayout, LayoutNode, SizingMode};
    use crate::intrinsic::tests::{image, text};
    use crate::intrinsic::{
        ContentMeasurer, IntrinsicMeasure, MeasureError, MeasureRequest, MeasuredContent,
    };
    use crate::stack::StackLayout;
    use crate::test_support::{id, key, len};

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

    /// 세로 Hug Stack root(1) 아래 text(2), 너비 30 text(3), 너비 40 image(4),
    /// Hug Grid(5: text 6, image 7).
    fn document(first_words: &[f32]) -> LayoutInput {
        let hug = SizingMode::HugContent;
        let grid = GridLayout::new(
            vec![TrackSize::MaxContent, TrackSize::MaxContent],
            vec![TrackSize::MaxContent],
        );
        LayoutInput::new(
            id(1),
            vec![
                LayoutNode::new(id(1))
                    .with_sizing(hug, hug)
                    .with_container(ContainerLayout::Stack(StackLayout::new(Axis::Vertical)))
                    .with_child(id(2), key(1))
                    .with_child(id(3), key(2))
                    .with_child(id(4), key(3))
                    .with_child(id(5), key(4)),
                LayoutNode::new(id(2))
                    .with_sizing(hug, hug)
                    .with_content(text(first_words)),
                LayoutNode::new(id(3))
                    .with_sizing(SizingMode::Fixed(len(30.0)), hug)
                    .with_content(text(&[10.0, 20.0, 15.0])),
                LayoutNode::new(id(4))
                    .with_sizing(SizingMode::Fixed(len(40.0)), hug)
                    .with_content(image(80.0, 60.0)),
                LayoutNode::new(id(5))
                    .with_sizing(hug, hug)
                    .with_container(ContainerLayout::Grid(grid))
                    .with_child(id(6), key(1))
                    .with_child(id(7), key(2)),
                LayoutNode::new(id(6))
                    .with_grid(GridItem::at(1, 1, 1, 1))
                    .with_sizing(hug, hug)
                    .with_content(text(&[12.0, 8.0])),
                LayoutNode::new(id(7))
                    .with_grid(GridItem::at(2, 1, 1, 1))
                    .with_sizing(hug, hug)
                    .with_content(image(16.0, 9.0)),
            ],
        )
    }

    #[test]
    fn same_input_never_measures_twice() {
        let input = document(&[10.0, 20.0]);
        // cache 없이도 한 계산 안에서 같은 측정 요청은 측정기에 한 번만 갑니다.
        let counting = Counting::default();
        let plain = compute_layout_with(&input, &counting).unwrap();
        let calls = counting.calls();
        assert!(!calls.is_empty());
        assert!(calls.values().all(|&n| n == 1), "{calls:?}");

        // 첫 cached 계산: 중복 0, 같은 요청 재사용(memo hit)이 실제로 일어남.
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

        // 같은 입력의 두 번째 계산: 측정 0, 측정기 호출 0, 결과 bytes 동일.
        let counting = Counting::default();
        let second = compute_layout_cached(&input, &counting, &mut cache).unwrap();
        assert!(counting.calls().is_empty());
        assert_eq!(second.stats.measured, 0);
        assert_eq!(second.stats.measurer_calls, 0);
        assert!(second.stats.cache_hits > 0);
        assert_eq!(second.result.encode(), first.result.encode());
        assert_eq!(cache.stats().hits, second.stats.cache_hits);
    }

    #[test]
    fn leaf_change_remeasures_only_its_ancestor_path() {
        let mut cache = MeasureCache::new();
        let first =
            compute_layout_cached(&document(&[10.0, 20.0]), &Counting::default(), &mut cache)
                .unwrap();
        let entries = cache.len();
        // text(2)만 바꾸면 측정기는 node 2만 다시 측정하고 형제(3, 4, Grid 5 subtree)는 cache hit.
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
        // 결과는 cache 없는 계산과 같고, 바뀐 node의 옛 entry는 지워져 크기가 늘지 않습니다.
        assert_eq!(
            run.result,
            compute_layout_with(&edited, &Counting::default()).unwrap()
        );
        assert_eq!(cache.len(), entries);
        // 측정기 revision이 바뀌면 모든 측정 콘텐츠를 다시 측정합니다.
        let counting = Counting::with_revision(1);
        let run = compute_layout_cached(&edited, &counting, &mut cache).unwrap();
        let measured_nodes: HashSet<StableId> = counting.calls().keys().map(|r| r.node).collect();
        assert_eq!(measured_nodes.len(), 5);
        assert_eq!(run.stats.cache_hits, 0);
        // 다른 입력으로 계산하면 그 입력에 맞지 않는 entry가 지워집니다.
        let other = LayoutInput::new(
            id(9),
            vec![LayoutNode::new(id(9))
                .with_sizing(SizingMode::HugContent, SizingMode::HugContent)
                .with_content(text(&[1.0]))],
        );
        let run = compute_layout_cached(&other, &Counting::default(), &mut cache).unwrap();
        // 크기 측정(가용 없음) + baseline 측정(너비 definite) 두 entry만 남습니다.
        assert_eq!(cache.len(), 2);
        assert_eq!(run.stats.measured, 2);
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn subtree_hash_is_cache_key_for_descendant_changes_only() {
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
        // 바뀐 leaf와 그 조상(root)만 hash가 바뀝니다.
        for n in 1..=7u64 {
            assert_eq!(base[&id(n)] != edited[&id(n)], n == 1 || n == 2, "node {n}");
        }
        // 같은 입력은 같은 hash(결정적).
        assert_eq!(base, hashes(&document(&[10.0, 20.0])));
        // key는 가용 크기·mode·측정기 revision을 구분합니다.
        let key = |available: Option<LayoutUnit>, mode: IntrinsicMode, revision: u64| MeasureKey {
            node: id(2),
            subtree: base[&id(2)],
            available: [available, None],
            mode,
            measurer: revision,
        };
        let max = IntrinsicMode::MaxContent;
        assert_ne!(key(None, max, 0), key(Some(LayoutUnit::ZERO), max, 0));
        assert_ne!(key(None, max, 0), key(None, max, 1));
        assert_ne!(key(None, max, 0), key(None, IntrinsicMode::MinContent, 0));
    }
}
