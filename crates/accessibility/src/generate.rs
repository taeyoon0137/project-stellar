//! 기본 접근성 트리 생성기 (IX.8.3).
//!
//! [`generate`]는 시각 [`SourceTree`]에서 [`AccessibilityTree`]를 만듭니다. 규칙:
//!
//! | source | 결과 |
//! |---|---|
//! | `hidden` | 자신과 subtree 전체 제외(보이지 않는 내용은 읽히지 않음, ARIA `aria-hidden`) |
//! | Fragment | flatten — node를 만들지 않고 자식을 Fragment의 접근성 부모에 순서대로 올림(VI.4.1 zero-size marker) |
//! | `decorative` | 자신만 제외하고 자식은 올림(장식은 자신의 의미만 없앰, ARIA `presentation`) |
//! | Frame·ComponentInstance | label 없으면 `Generic`, 있으면 `Group` |
//! | Text | `StaticText`, label은 작성자 label 또는 자신의 텍스트 내용(`NodeText`) |
//! | Image | `Image`, 작성자 label(없으면 진단 대상, IX.8.5) |
//! | Graphic(Shape·Path) | label이 있으면 `Image`, 없으면 장식으로 보고 제외 |
//!
//! 작성자 읽기 순서는 그대로 옮깁니다. 순회는 반복(비재귀) preorder이고 시각 자식 순서를 따르므로, flatten된
//! node의 자식도 원래 문서 순서 위치에 들어갑니다. source tree가 tree가 아니면(없는 자식, 같은 node를 두 번
//! 방문 — 공유 자식 또는 cycle) [`GenerateError`]로 거부합니다.

use std::collections::BTreeSet;
use std::fmt;

use stellar_core_types::ids::StableId;

use crate::node::{AccessibilityBinding, AccessibilityNode, AccessibilityRole};
use crate::source::{SourceKind, SourceNode, SourceTree};
use crate::tree::AccessibilityTree;

/// 생성 오류(source tree가 tree가 아님).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerateError {
    /// root 또는 자식이 source tree에 없습니다.
    MissingNode {
        /// 가리키는 부모(root 목록이면 `None`).
        parent: Option<StableId>,
        /// 없는 node.
        node: StableId,
    },
    /// 같은 node를 두 번 방문했습니다(공유 자식 또는 cycle).
    RevisitedNode {
        /// node.
        node: StableId,
    },
}

impl fmt::Display for GenerateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GenerateError::MissingNode { parent, node } => match parent {
                Some(parent) => write!(f, "source 부모 {parent}의 자식 {node}가 없습니다"),
                None => write!(f, "source root {node}가 없습니다"),
            },
            GenerateError::RevisitedNode { node } => {
                write!(
                    f,
                    "source node {node}를 두 번 방문했습니다(공유 자식 또는 cycle)"
                )
            }
        }
    }
}

impl std::error::Error for GenerateError {}

/// 시각 source tree에서 기본 접근성 트리를 만듭니다.
pub fn generate(source: &SourceTree) -> Result<AccessibilityTree, GenerateError> {
    let mut tree = AccessibilityTree::new();
    let mut visited = BTreeSet::new();
    // (source node, 그 source를 가리킨 시각 부모, 접근성 부모)
    let mut stack: Vec<(StableId, Option<StableId>, Option<StableId>)> = source
        .roots()
        .iter()
        .rev()
        .map(|root| (*root, None, None))
        .collect();
    while let Some((id, visual_parent, ax_parent)) = stack.pop() {
        let node = source.node(id).ok_or(GenerateError::MissingNode {
            parent: visual_parent,
            node: id,
        })?;
        if !visited.insert(id) {
            return Err(GenerateError::RevisitedNode { node: id });
        }
        if node.hidden {
            continue;
        }
        let mut child_parent = ax_parent;
        if let Some(ax_node) = map_node(node) {
            // source는 visited로 한 번만 방문하므로 중복 삽입이 생기지 않습니다.
            if tree.insert(ax_node).is_err() {
                return Err(GenerateError::RevisitedNode { node: id });
            }
            match ax_parent {
                Some(parent) => tree.push_child(parent, id),
                None => tree.push_root(id),
            }
            child_parent = Some(id);
        }
        stack.extend(
            node.children
                .iter()
                .rev()
                .map(|child| (*child, Some(id), child_parent)),
        );
    }
    Ok(tree)
}

/// source node 하나의 기본 접근성 node. 접근성 node를 만들지 않으면(flatten·제외) `None`.
fn map_node(node: &SourceNode) -> Option<AccessibilityNode> {
    if node.decorative {
        return None;
    }
    let mut mapped = match node.kind {
        SourceKind::Fragment => return None,
        SourceKind::Frame | SourceKind::ComponentInstance => {
            let role = if node.label.is_some() {
                AccessibilityRole::Group
            } else {
                AccessibilityRole::Generic
            };
            AccessibilityNode::new(node.id, role)
        }
        SourceKind::Text => AccessibilityNode::new(node.id, AccessibilityRole::StaticText)
            .with_label(AccessibilityBinding::NodeText(node.id)),
        SourceKind::Image => AccessibilityNode::new(node.id, AccessibilityRole::Image),
        SourceKind::Graphic => {
            node.label.as_ref()?;
            AccessibilityNode::new(node.id, AccessibilityRole::Image)
        }
    };
    if let Some(label) = &node.label {
        mapped.label = Some(label.clone());
    }
    mapped.reading_order = node.reading_order;
    Some(mapped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{id, outline};

    fn insert(tree: &mut SourceTree, node: SourceNode) {
        tree.insert(node).expect("source node");
    }

    /// ```text
    /// 1 Frame
    /// ├ 2 Text
    /// ├ 3 Fragment ─ 4 Image("로고"), 5 Text
    /// ├ 6 Frame(decorative) ─ 7 Graphic, 8 Text
    /// ├ 9 Frame(hidden) ─ 10 Text
    /// ├ 11 Frame("카드", reading order 2) ─ 12 Text
    /// └ 13 Graphic("차트")
    /// ```
    fn fixture() -> SourceTree {
        let mut tree = SourceTree::new();
        insert(
            &mut tree,
            SourceNode::new(id(1), SourceKind::Frame).with_children([
                id(2),
                id(3),
                id(6),
                id(9),
                id(11),
                id(13),
            ]),
        );
        insert(&mut tree, SourceNode::new(id(2), SourceKind::Text));
        insert(
            &mut tree,
            SourceNode::new(id(3), SourceKind::Fragment).with_children([id(4), id(5)]),
        );
        insert(
            &mut tree,
            SourceNode::new(id(4), SourceKind::Image)
                .with_label(AccessibilityBinding::text("로고")),
        );
        insert(&mut tree, SourceNode::new(id(5), SourceKind::Text));
        insert(
            &mut tree,
            SourceNode::new(id(6), SourceKind::Frame)
                .decorative()
                .with_children([id(7), id(8)]),
        );
        insert(&mut tree, SourceNode::new(id(7), SourceKind::Graphic));
        insert(&mut tree, SourceNode::new(id(8), SourceKind::Text));
        insert(
            &mut tree,
            SourceNode::new(id(9), SourceKind::Frame)
                .hidden()
                .with_children([id(10)]),
        );
        insert(&mut tree, SourceNode::new(id(10), SourceKind::Text));
        insert(
            &mut tree,
            SourceNode::new(id(11), SourceKind::Frame)
                .with_label(AccessibilityBinding::text("카드"))
                .with_reading_order(2)
                .with_children([id(12)]),
        );
        insert(&mut tree, SourceNode::new(id(12), SourceKind::Text));
        insert(
            &mut tree,
            SourceNode::new(id(13), SourceKind::Graphic)
                .with_label(AccessibilityBinding::text("차트")),
        );
        tree.push_root(id(1));
        tree
    }

    #[test]
    fn expected_tree_fixture_passes() {
        let source = fixture();
        let tree = generate(&source).expect("생성");

        let mut expected = AccessibilityTree::new();
        let text = |n: u8| {
            AccessibilityNode::new(id(n), AccessibilityRole::StaticText)
                .with_label(AccessibilityBinding::NodeText(id(n)))
        };
        for node in [
            AccessibilityNode::new(id(1), AccessibilityRole::Generic),
            text(2),
            AccessibilityNode::new(id(4), AccessibilityRole::Image)
                .with_label(AccessibilityBinding::text("로고")),
            text(5),
            text(8),
            AccessibilityNode::new(id(11), AccessibilityRole::Group)
                .with_label(AccessibilityBinding::text("카드"))
                .with_reading_order(2),
            text(12),
            AccessibilityNode::new(id(13), AccessibilityRole::Image)
                .with_label(AccessibilityBinding::text("차트")),
        ] {
            expected.insert(node).expect("expected node");
        }
        expected.push_root(id(1));
        expected.set_children(id(1), vec![id(2), id(4), id(5), id(8), id(11), id(13)]);
        expected.set_children(id(11), vec![id(12)]);
        assert_eq!(tree, expected);

        assert_eq!(
            outline(&tree),
            "\
generic #1
  staticText #2 label=text:#2
  image #4 label=\"로고\"
  staticText #5 label=text:#5
  staticText #8 label=text:#8
  group #11 label=\"카드\" order=2
    staticText #12 label=text:#12
  image #13 label=\"차트\"
"
        );
        // 생성 결과는 source 검사·구조 검사를 통과합니다.
        assert_eq!(tree.validate(&source), Vec::new());
        // 결정론.
        assert_eq!(
            generate(&source).expect("재생성").to_bytes(),
            tree.to_bytes()
        );
    }

    #[test]
    fn hidden_root_and_root_level_flatten() {
        let mut source = SourceTree::new();
        insert(
            &mut source,
            SourceNode::new(id(1), SourceKind::Fragment).with_children([id(2), id(3)]),
        );
        insert(&mut source, SourceNode::new(id(2), SourceKind::Text));
        insert(
            &mut source,
            SourceNode::new(id(3), SourceKind::Frame)
                .decorative()
                .with_children([id(4)]),
        );
        insert(&mut source, SourceNode::new(id(4), SourceKind::Image));
        insert(
            &mut source,
            SourceNode::new(id(5), SourceKind::Frame)
                .hidden()
                .with_children([id(6)]),
        );
        insert(&mut source, SourceNode::new(id(6), SourceKind::Text));
        source.push_root(id(1));
        source.push_root(id(5));
        let tree = generate(&source).expect("생성");
        // root Fragment·decorative의 자식이 root로 올라오고, hidden root는 subtree째 빠집니다.
        assert_eq!(tree.roots(), &[id(2), id(4)]);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree.validate(&source), Vec::new());
    }

    #[test]
    fn malformed_source_trees_are_rejected() {
        let mut missing = SourceTree::new();
        insert(
            &mut missing,
            SourceNode::new(id(1), SourceKind::Frame).with_children([id(2)]),
        );
        missing.push_root(id(1));
        assert_eq!(
            generate(&missing),
            Err(GenerateError::MissingNode {
                parent: Some(id(1)),
                node: id(2)
            })
        );
        let mut missing_root = SourceTree::new();
        missing_root.push_root(id(9));
        assert_eq!(
            generate(&missing_root),
            Err(GenerateError::MissingNode {
                parent: None,
                node: id(9)
            })
        );

        let mut shared = SourceTree::new();
        insert(
            &mut shared,
            SourceNode::new(id(1), SourceKind::Frame).with_children([id(2), id(2)]),
        );
        insert(&mut shared, SourceNode::new(id(2), SourceKind::Text));
        shared.push_root(id(1));
        assert_eq!(
            generate(&shared),
            Err(GenerateError::RevisitedNode { node: id(2) })
        );

        let mut cycle = SourceTree::new();
        insert(
            &mut cycle,
            SourceNode::new(id(1), SourceKind::Frame).with_children([id(2)]),
        );
        insert(
            &mut cycle,
            SourceNode::new(id(2), SourceKind::Fragment).with_children([id(1)]),
        );
        cycle.push_root(id(1));
        assert_eq!(
            generate(&cycle),
            Err(GenerateError::RevisitedNode { node: id(1) })
        );
    }

    #[test]
    fn deep_source_tree_does_not_overflow_the_stack() {
        // 반복 순회이므로 깊이 20,000 chain도 처리합니다.
        const DEPTH: u32 = 20_000;
        let node_id = |n: u32| {
            let mut bytes = [0u8; 16];
            bytes[12..].copy_from_slice(&n.to_be_bytes());
            StableId::from_bytes(bytes)
        };
        let mut source = SourceTree::new();
        for n in 0..DEPTH {
            let mut node = SourceNode::new(node_id(n), SourceKind::Frame);
            if n + 1 < DEPTH {
                node = node.with_children([node_id(n + 1)]);
            }
            insert(&mut source, node);
        }
        source.push_root(node_id(0));
        let tree = generate(&source).expect("생성");
        assert_eq!(tree.len(), DEPTH as usize);
        assert_eq!(tree.validate(&source), Vec::new());
    }
}
