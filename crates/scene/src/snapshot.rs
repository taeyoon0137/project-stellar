//! 불변 Scene Snapshot (XI.1.1).
//!
//! [`SceneSnapshot`]은 한 시점의 scene 전체입니다. document thread가 만들고 render thread가
//! 읽기만 합니다(아키텍처 §9.3). 최상위 node 목록([`SceneSnapshot::roots`])은 paint 순서이며,
//! 같은 subtree를 이전 snapshot과 `Arc`로 공유할 수 있습니다.
//!
//! 최상위 node 사이에서도 z-order 불변식(pre-order 엄격 증가, [`crate::node`] 문서)을
//! 검사합니다. 각 node는 생성 시 직접 자식을 검사했으므로 snapshot 전체가 불변식을 만족합니다.
//!
//! **semantic hash (XI.1.3)**: [`SceneSnapshot::semantic_hash`]는 revision 두 개를 뺀 canonical
//! 본문 bytes([`crate::codec`])의 FNV-1a 128 hash입니다. 같은 내용은 `Arc` 공유 구조·생성
//! 순서·scene revision과 무관하게 같은 hash가 됩니다. 변경 감지용이며 보안 hash가 아닙니다.

use std::collections::{BTreeSet, HashSet};
use std::fmt;
use std::sync::Arc;

use stellar_core_types::revision::{DocumentRevision, SceneRevision};

use crate::codec::{write_body, ByteSink};
use crate::node::{check_z_follows, SceneEditError, SceneNode, SceneResourceRef, SceneTreeError};
use crate::persistent::PersistentVec;

/// snapshot 내용의 semantic hash(FNV-1a 128).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SceneSemanticHash(
    /// hash 값.
    pub u128,
);

impl fmt::Display for SceneSemanticHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

/// FNV-1a 128 hasher(canonical bytes sink).
pub(crate) struct Fnv1a128(u128);

impl Fnv1a128 {
    const OFFSET_BASIS: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013B;

    pub(crate) const fn new() -> Fnv1a128 {
        Fnv1a128(Fnv1a128::OFFSET_BASIS)
    }

    pub(crate) const fn finish(&self) -> u128 {
        self.0
    }
}

impl ByteSink for Fnv1a128 {
    fn put(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u128::from(*byte);
            self.0 = self.0.wrapping_mul(Fnv1a128::PRIME);
        }
    }
}

/// 두 snapshot 사이 구조 공유 통계(XI.1.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharingStats {
    /// 새 snapshot의 전체 node 수.
    pub total_nodes: u64,
    /// 이전 snapshot과 같은 allocation(`Arc`)을 공유하는 node 수.
    pub shared_nodes: u64,
    /// 새로 할당된 node 수.
    pub new_nodes: u64,
}

/// 불변 scene snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneSnapshot {
    document_revision: DocumentRevision,
    scene_revision: SceneRevision,
    roots: PersistentVec<Arc<SceneNode>>,
    node_count: u64,
}

impl SceneSnapshot {
    /// revision과 최상위 node(paint 순서)로 snapshot을 만듭니다.
    ///
    /// 최상위 subtree 사이에서 z-order가 엄격히 증가하지 않으면 거부합니다.
    pub fn new(
        document_revision: DocumentRevision,
        scene_revision: SceneRevision,
        roots: Vec<Arc<SceneNode>>,
    ) -> Result<SceneSnapshot, SceneTreeError> {
        let mut node_count = 0u64;
        for (index, root) in roots.iter().enumerate() {
            if index > 0 {
                check_z_follows(roots[index - 1].last_z(), root.z())?;
            }
            node_count = node_count.saturating_add(root.subtree_len());
        }
        Ok(SceneSnapshot {
            document_revision,
            scene_revision,
            roots: PersistentVec::from_vec(roots),
            node_count,
        })
    }

    /// `path`(최상위 index, 자식 index, …) 위치의 node를 `node`로 바꾼 새 snapshot(XI.1.4).
    ///
    /// root에서 대상까지의 조상 node만 새로 할당하고(path copying), 나머지 node·자식 목록
    /// chunk·clip·paint는 이 snapshot과 공유합니다. 이 snapshot은 바뀌지 않습니다. 새 subtree의
    /// z-order가 앞뒤 node 사이에 맞지 않으면 거부하며, 검사는 경로 주변만 봅니다.
    pub fn replace_node(
        &self,
        document_revision: DocumentRevision,
        scene_revision: SceneRevision,
        path: &[usize],
        node: Arc<SceneNode>,
    ) -> Result<SceneSnapshot, SceneEditError> {
        let (&root_index, rest) = path.split_first().ok_or(SceneEditError::EmptyPath)?;
        let old_root = self
            .roots
            .get(root_index)
            .ok_or(SceneEditError::PathOutOfRange {
                level: 0,
                index: root_index,
            })?;
        let new_root = replace_below(old_root, rest, node)?;
        if let Some(previous) = root_index.checked_sub(1).and_then(|i| self.roots.get(i)) {
            check_z_follows(previous.last_z(), new_root.z())?;
        }
        if let Some(next) = self.roots.get(root_index + 1) {
            check_z_follows(new_root.last_z(), next.z())?;
        }
        let node_count = self
            .node_count
            .saturating_sub(old_root.subtree_len())
            .saturating_add(new_root.subtree_len());
        let roots = self
            .roots
            .set(root_index, new_root)
            .ok_or(SceneEditError::PathOutOfRange {
                level: 0,
                index: root_index,
            })?;
        Ok(SceneSnapshot {
            document_revision,
            scene_revision,
            roots,
            node_count,
        })
    }

    /// `previous`와의 구조 공유 통계. 공유 subtree는 내려가지 않고 크기만 셉니다.
    pub fn sharing_with(&self, previous: &SceneSnapshot) -> SharingStats {
        let seen: HashSet<*const SceneNode> = previous
            .iter_preorder()
            .map(|node| node as *const SceneNode)
            .collect();
        let mut shared_nodes = 0u64;
        let mut new_nodes = 0u64;
        let mut stack: Vec<&Arc<SceneNode>> = self.roots.iter().collect();
        while let Some(node) = stack.pop() {
            if seen.contains(&Arc::as_ptr(node)) {
                shared_nodes = shared_nodes.saturating_add(node.subtree_len());
            } else {
                new_nodes += 1;
                stack.extend(node.children());
            }
        }
        SharingStats {
            total_nodes: self.node_count,
            shared_nodes,
            new_nodes,
        }
    }

    /// 최상위 node 목록(구조 공유 vector).
    pub const fn root_list(&self) -> &PersistentVec<Arc<SceneNode>> {
        &self.roots
    }

    /// 이 snapshot을 만든 document revision.
    pub const fn document_revision(&self) -> DocumentRevision {
        self.document_revision
    }

    /// scene revision(snapshot 세대 번호).
    pub const fn scene_revision(&self) -> SceneRevision {
        self.scene_revision
    }

    /// 최상위 node 수.
    pub fn root_count(&self) -> usize {
        self.roots.len()
    }

    /// `index`번째 최상위 node.
    pub fn root(&self, index: usize) -> Option<&Arc<SceneNode>> {
        self.roots.get(index)
    }

    /// 최상위 node들(paint 순서).
    pub fn roots(&self) -> crate::persistent::Iter<'_, Arc<SceneNode>> {
        self.roots.iter()
    }

    /// 전체 node 수.
    pub const fn node_count(&self) -> u64 {
        self.node_count
    }

    /// 모든 node를 pre-order(paint 순서: 부모 → 자식 순)로 순회합니다.
    ///
    /// 재귀 없이 명시적 stack을 써서 깊은 tree에서도 호출 stack을 소모하지 않습니다.
    pub fn iter_preorder(&self) -> PreOrder<'_> {
        let mut stack: Vec<&SceneNode> = self.roots.iter().map(|root| &**root).collect();
        stack.reverse();
        PreOrder { stack }
    }

    /// snapshot이 참조하는 외부 resource 집합(정렬, 중복 제거).
    pub fn resource_refs(&self) -> BTreeSet<SceneResourceRef> {
        self.iter_preorder()
            .filter_map(|node| node.content().resource())
            .collect()
    }

    /// revision을 뺀 내용의 semantic hash. 같은 내용이면 같은 값입니다.
    pub fn semantic_hash(&self) -> SceneSemanticHash {
        let mut hasher = Fnv1a128::new();
        write_body(self, &mut hasher);
        SceneSemanticHash(hasher.finish())
    }
}

/// [`SceneSnapshot::iter_preorder`] 반복자.
#[derive(Debug)]
pub struct PreOrder<'a> {
    stack: Vec<&'a SceneNode>,
}

impl<'a> Iterator for PreOrder<'a> {
    type Item = &'a SceneNode;

    fn next(&mut self) -> Option<&'a SceneNode> {
        let node = self.stack.pop()?;
        // 자식을 역순으로 쌓아 paint 순서대로 꺼냅니다.
        let start = self.stack.len();
        self.stack.extend(node.children().map(|child| &**child));
        self.stack[start..].reverse();
        Some(node)
    }
}

/// `root` 아래 `path` 위치를 `replacement`로 바꾼 새 node(경로 복사).
///
/// `path[k]`는 `k + 1`단계 index입니다. 경로를 먼저 끝까지 확인한 뒤 아래에서 위로 조상을
/// 다시 만들며, 재귀하지 않아 경로가 수십만 단이어도 호출 stack을 소모하지 않습니다.
fn replace_below(
    root: &Arc<SceneNode>,
    path: &[usize],
    replacement: Arc<SceneNode>,
) -> Result<Arc<SceneNode>, SceneEditError> {
    let mut ancestors: Vec<&Arc<SceneNode>> = Vec::with_capacity(path.len());
    let mut node = root;
    for (offset, &index) in path.iter().enumerate() {
        ancestors.push(node);
        node = node.child(index).ok_or(SceneEditError::PathOutOfRange {
            level: offset + 1,
            index,
        })?;
    }
    let mut current = replacement;
    for (offset, (ancestor, &index)) in ancestors.iter().zip(path).enumerate().rev() {
        let level = offset + 1;
        current = ancestor
            .with_child_replaced(index, current)
            .map(Arc::new)
            .map_err(|error| match error {
                SceneEditError::PathOutOfRange { index, .. } => {
                    SceneEditError::PathOutOfRange { level, index }
                }
                other => other,
            })?;
    }
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{self, leaf, rect, spec};
    use crate::node::{SceneContent, SceneNodeSpec, TextLayoutKey, TextPrimitive, ZOrder};

    fn node(n: u8, z: u64, children: Vec<Arc<SceneNode>>) -> Arc<SceneNode> {
        Arc::new(
            SceneNode::new(
                spec(n as u64, fixtures::rect_content(1.0, 1.0), z),
                children,
            )
            .unwrap(),
        )
    }

    fn source_byte(node: &SceneNode) -> u8 {
        node.source().to_bytes()[15]
    }

    #[test]
    fn preorder_visits_parent_before_children_in_paint_order() {
        // 1(2, 3(4)), 5
        let tree = node(
            1,
            1,
            vec![node(2, 2, vec![]), node(3, 3, vec![node(4, 4, vec![])])],
        );
        let snapshot = SceneSnapshot::new(
            DocumentRevision(7),
            SceneRevision(3),
            vec![tree, node(5, 5, vec![])],
        )
        .unwrap();
        let order: Vec<u8> = snapshot.iter_preorder().map(source_byte).collect();
        assert_eq!(order, vec![1, 2, 3, 4, 5]);
        assert_eq!(snapshot.node_count(), 5);
        assert_eq!(snapshot.root_count(), 2);
        assert_eq!(snapshot.document_revision(), DocumentRevision(7));
        assert_eq!(snapshot.scene_revision(), SceneRevision(3));
    }

    #[test]
    fn z_order_must_increase_across_roots() {
        let a = node(1, 1, vec![node(2, 9, vec![])]);
        let b = node(3, 5, vec![]);
        assert_eq!(
            SceneSnapshot::new(DocumentRevision(1), SceneRevision(1), vec![a, b]).unwrap_err(),
            SceneTreeError::ZOrderNotIncreasing {
                previous: ZOrder(9),
                next: ZOrder(5)
            }
        );
    }

    #[test]
    fn resource_refs_are_sorted_and_deduplicated() {
        let text = |key: u8, z: u64| {
            Arc::new(SceneNode::leaf(SceneNodeSpec {
                content: SceneContent::Text(TextPrimitive {
                    layout: TextLayoutKey::from_bytes([key; 16]),
                    bounds: rect(0.0, 0.0, 0.0, 0.0),
                }),
                ..spec(key as u64, fixtures::rect_content(1.0, 1.0), z)
            }))
        };
        let snapshot = SceneSnapshot::new(
            DocumentRevision(1),
            SceneRevision(1),
            vec![text(3, 1), text(1, 2), text(3, 3), leaf(9, 4, 1.0, 1.0)],
        )
        .unwrap();
        let refs: Vec<SceneResourceRef> = snapshot.resource_refs().into_iter().collect();
        assert_eq!(
            refs,
            vec![
                SceneResourceRef::TextLayout(TextLayoutKey::from_bytes([1; 16])),
                SceneResourceRef::TextLayout(TextLayoutKey::from_bytes([3; 16])),
            ]
        );
    }

    #[test]
    fn fnv1a_128_matches_reference_vectors() {
        let hash = |bytes: &[u8]| {
            let mut h = Fnv1a128::new();
            h.put(bytes);
            h.finish()
        };
        // FNV 공개 test vector.
        assert_eq!(hash(b""), 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d);
        assert_eq!(hash(b"a"), 0xd228_cb69_6f1a_8caf_7891_2b70_4e4a_8964);
        assert_eq!(hash(b"foobar"), 0x343e_1662_793c_64bf_6f0d_3597_ba44_6f18);
        // 나눠 넣어도 같은 결과(sink 계약).
        let mut split = Fnv1a128::new();
        split.put(b"foo");
        split.put(b"bar");
        assert_eq!(split.finish(), hash(b"foobar"));
    }

    #[test]
    fn semantic_hash_ignores_revisions_only() {
        let snapshot = fixtures::small();
        let roots: Vec<Arc<SceneNode>> = snapshot.roots().cloned().collect();
        let other_revisions =
            SceneSnapshot::new(DocumentRevision(99), SceneRevision(42), roots).unwrap();
        assert_eq!(other_revisions.semantic_hash(), snapshot.semantic_hash());
        let empty = SceneSnapshot::new(DocumentRevision(1), SceneRevision(1), Vec::new()).unwrap();
        assert_ne!(empty.semantic_hash(), snapshot.semantic_hash());
        assert_eq!(format!("{}", SceneSemanticHash(0xAB)).len(), 32);
    }

    fn node_at<'a>(snapshot: &'a SceneSnapshot, path: &[usize]) -> &'a Arc<SceneNode> {
        let mut node = snapshot.root(path[0]).expect("root");
        for &index in &path[1..] {
            node = node.child(index).expect("child");
        }
        node
    }

    fn repainted(node: &SceneNode) -> SceneNodeSpec {
        let mut spec = node.spec();
        spec.paint = crate::stack::PaintStackRef::new(crate::stack::PaintStack {
            layers: vec![crate::stack::PaintLayer::Fill(fixtures::solid(
                0.1, 0.2, 0.3, 1.0,
            ))],
            ..crate::stack::PaintStack::empty()
        });
        spec
    }

    #[test]
    fn single_leaf_edit_shares_every_other_node() {
        // 검증 기준(XI.1.4): 단일 leaf 변경 시 shared node count.
        // 최상위 8 × frame 8 × frame 8 × leaf 8 = 4680 node, leaf depth 3.
        let base = fixtures::generate(fixtures::FixtureConfig {
            seed: 0x14,
            roots: 8,
            children: 8,
            depth: 3,
        });
        assert_eq!(base.node_count(), 8 * (1 + 8 + 64 + 512));
        let path = [3, 2, 5, 7];
        let old_leaf = Arc::clone(node_at(&base, &path));
        let new_leaf = Arc::new(SceneNode::leaf(repainted(&old_leaf)));
        let base_hash = base.semantic_hash();

        let mut map = crate::revision::SceneRevisionMap::new();
        map.next(DocumentRevision(1)).unwrap();
        let next = map
            .derive(&base, DocumentRevision(2), &path, Arc::clone(&new_leaf))
            .unwrap();
        assert_eq!(next.scene_revision(), SceneRevision(2));
        assert_eq!(next.node_count(), base.node_count());

        // 새 allocation은 leaf와 그 조상 3개(경로 길이 4)뿐입니다.
        let stats = next.sharing_with(&base);
        assert_eq!(
            stats,
            SharingStats {
                total_nodes: 4680,
                shared_nodes: 4676,
                new_nodes: 4
            }
        );
        // 형제 subtree는 같은 Arc를 가리킵니다.
        assert!(Arc::ptr_eq(
            node_at(&next, &[3, 2, 5, 6]),
            node_at(&base, &[3, 2, 5, 6])
        ));
        assert!(Arc::ptr_eq(
            node_at(&next, &[3, 1]),
            node_at(&base, &[3, 1])
        ));
        assert!(Arc::ptr_eq(node_at(&next, &[4]), node_at(&base, &[4])));
        assert!(Arc::ptr_eq(node_at(&next, &path), &new_leaf));
        // clip stack도 공유됩니다(leaf spec을 그대로 복사).
        assert!(node_at(&next, &path).clip().ptr_eq(old_leaf.clip()));

        // 이전 snapshot은 그대로입니다.
        assert!(Arc::ptr_eq(node_at(&base, &path), &old_leaf));
        assert_eq!(base.semantic_hash(), base_hash);
        assert_ne!(next.semantic_hash(), base_hash);
        // 같은 내용을 처음부터 다시 만든 것과 값이 같습니다(결정론).
        assert_eq!(
            crate::codec::decode(&crate::codec::encode(&next)).unwrap(),
            next
        );
    }

    #[test]
    fn frame_property_edit_keeps_children_shared() {
        let base = fixtures::generate(fixtures::FixtureConfig {
            seed: 0x15,
            roots: 2,
            children: 8,
            depth: 3,
        });
        let path = [1, 4];
        let frame = node_at(&base, &path);
        let edited = Arc::new(frame.with_spec(repainted(frame)).unwrap());
        assert_eq!(edited.subtree_len(), frame.subtree_len());
        let next = base
            .replace_node(DocumentRevision(2), SceneRevision(2), &path, edited)
            .unwrap();
        // frame과 최상위 조상만 새로 할당, frame의 자식 subtree는 모두 공유.
        assert_eq!(next.sharing_with(&base).new_nodes, 2);
        assert!(Arc::ptr_eq(
            node_at(&next, &[1, 4, 0]),
            node_at(&base, &[1, 4, 0])
        ));
    }

    #[test]
    fn wide_frame_child_edit_copies_one_chunk_path() {
        // 자식 5000개 frame에서 자식 하나를 바꾸면 자식 목록 leaf chunk 157개 중 1개만 새로 만듭니다.
        let children: Vec<Arc<SceneNode>> = (0..5_000u64)
            .map(|i| leaf(i + 10, i + 2, 1.0, 1.0))
            .collect();
        let frame = Arc::new(
            SceneNode::new(spec(1, fixtures::rect_content(10.0, 10.0), 1), children).unwrap(),
        );
        let base = SceneSnapshot::new(DocumentRevision(1), SceneRevision(1), vec![frame]).unwrap();
        let target = node_at(&base, &[0, 2_500]);
        let replacement = Arc::new(SceneNode::leaf(repainted(target)));
        let next = base
            .replace_node(
                DocumentRevision(2),
                SceneRevision(2),
                &[0, 2_500],
                replacement,
            )
            .unwrap();
        let stats = next.sharing_with(&base);
        assert_eq!((stats.new_nodes, stats.shared_nodes), (2, 4_999));
        let old_list = base.root(0).unwrap().child_list();
        let new_list = next.root(0).unwrap().child_list();
        assert_eq!(
            new_list.shared_leaf_chunks(old_list),
            5_000usize.div_ceil(32) - 1
        );
    }

    #[test]
    fn replacement_must_fit_z_range_and_path() {
        let base = fixtures::generate(fixtures::FixtureConfig {
            seed: 0x16,
            roots: 2,
            children: 3,
            depth: 2,
        });
        let path = [0, 1, 1];
        let target = node_at(&base, &path);
        let next_sibling_z = node_at(&base, &[0, 1, 2]).z();
        // 다음 형제의 z와 겹치는 leaf는 거부합니다.
        let mut clash = target.spec();
        clash.z = next_sibling_z;
        assert_eq!(
            base.replace_node(
                DocumentRevision(2),
                SceneRevision(2),
                &path,
                Arc::new(SceneNode::leaf(clash))
            ),
            Err(SceneEditError::Tree(SceneTreeError::ZOrderNotIncreasing {
                previous: next_sibling_z,
                next: next_sibling_z
            }))
        );
        let same = Arc::clone(target);
        assert_eq!(
            base.replace_node(
                DocumentRevision(2),
                SceneRevision(2),
                &[0, 1, 9],
                same.clone()
            ),
            Err(SceneEditError::PathOutOfRange { level: 2, index: 9 })
        );
        assert_eq!(
            base.replace_node(DocumentRevision(2), SceneRevision(2), &[5], same.clone()),
            Err(SceneEditError::PathOutOfRange { level: 0, index: 5 })
        );
        assert_eq!(
            base.replace_node(DocumentRevision(2), SceneRevision(2), &[], same.clone()),
            Err(SceneEditError::EmptyPath)
        );
        // 마지막 최상위 subtree의 끝이 늘어나도 다음 root와 겹치면 거부합니다.
        let last_of_first_root = node_at(&base, &[0, 2, 2]);
        let mut grow = last_of_first_root.spec();
        grow.z = ZOrder(node_at(&base, &[1]).z().0 + 1);
        assert!(matches!(
            base.replace_node(
                DocumentRevision(2),
                SceneRevision(2),
                &[0, 2, 2],
                Arc::new(SceneNode::leaf(grow))
            ),
            Err(SceneEditError::Tree(_))
        ));
    }

    #[test]
    fn deep_path_replace_and_drop_do_not_recurse() {
        use crate::fixtures::{chain_over, on_small_stack, DEEP};
        // 회귀(scene recursive-drop-stack-overflow): 10만 단 경로의 replace_node·비교·해제가
        // 작은 stack에서도 끝나야 합니다.
        on_small_stack(|| {
            let base = SceneSnapshot::new(
                DocumentRevision(1),
                SceneRevision(1),
                vec![chain_over(leaf(0, DEEP + 1, 1.0, 1.0), DEEP)],
            )
            .unwrap();
            let mut path = vec![0usize; DEEP as usize + 1];
            let target = Arc::clone(node_at(&base, &path));
            let replacement = Arc::new(SceneNode::leaf(repainted(&target)));
            let next = base
                .replace_node(
                    DocumentRevision(2),
                    SceneRevision(2),
                    &path,
                    Arc::clone(&replacement),
                )
                .unwrap();
            assert!(Arc::ptr_eq(node_at(&next, &path), &replacement));
            assert!(Arc::ptr_eq(node_at(&base, &path), &target));
            assert_eq!(next.node_count(), DEEP + 1);
            // 경로 위 node만 새로 할당됩니다.
            assert_eq!(next.sharing_with(&base).new_nodes, DEEP + 1);
            assert!(next != base);
            // 깊은 경로 끝의 범위 밖 index는 해당 단계로 보고합니다.
            path.push(3);
            assert_eq!(
                base.replace_node(DocumentRevision(2), SceneRevision(2), &path, replacement),
                Err(SceneEditError::PathOutOfRange {
                    level: DEEP as usize + 1,
                    index: 3
                })
            );
            // 같은 내용의 복제본과 값이 같고, 해제도 반복으로 끝납니다.
            let copy = base.clone();
            assert!(copy == base);
            drop((base, next, copy, target));
        });
    }

    #[test]
    fn snapshot_is_send_sync_value_type() {
        // GPU handle·document 객체·내부 가변성이 없으므로 thread 간 공유가 가능합니다.
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<SceneSnapshot>();
        assert_send_sync::<SceneNode>();
    }
}
