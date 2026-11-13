//! 불변 Scene Snapshot (XI.1.1).
//!
//! [`SceneSnapshot`]은 한 시점의 scene 전체입니다. document thread가 만들고 render thread가
//! 읽기만 합니다(아키텍처 §9.3). 최상위 node 목록([`SceneSnapshot::roots`])은 paint 순서이며,
//! 같은 subtree를 이전 snapshot과 `Arc`로 공유할 수 있습니다.
//!
//! 최상위 node 사이에서도 z-order 불변식(pre-order 엄격 증가, [`crate::node`] 문서)을
//! 검사합니다. 각 node는 생성 시 직접 자식을 검사했으므로 snapshot 전체가 불변식을 만족합니다.

use std::collections::BTreeSet;
use std::sync::Arc;

use stellar_core_types::revision::{DocumentRevision, SceneRevision};

use crate::node::{check_z_follows, SceneNode, SceneResourceRef, SceneTreeError};

/// 불변 scene snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneSnapshot {
    document_revision: DocumentRevision,
    scene_revision: SceneRevision,
    roots: Arc<[Arc<SceneNode>]>,
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
            roots: roots.into(),
            node_count,
        })
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
    pub fn roots(&self) -> impl DoubleEndedIterator<Item = &Arc<SceneNode>> + '_ {
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
        PreOrder {
            stack: self.roots.iter().rev().map(|root| &**root).collect(),
        }
    }

    /// snapshot이 참조하는 외부 resource 집합(정렬, 중복 제거).
    pub fn resource_refs(&self) -> BTreeSet<SceneResourceRef> {
        self.iter_preorder()
            .filter_map(|node| node.content().resource())
            .collect()
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
        self.stack
            .extend(node.children().rev().map(|child| &**child));
        Some(node)
    }
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
    fn snapshot_is_send_sync_value_type() {
        // GPU handle·document 객체·내부 가변성이 없으므로 thread 간 공유가 가능합니다.
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<SceneSnapshot>();
        assert_send_sync::<SceneNode>();
    }
}
