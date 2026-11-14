//! 접근성 트리 (IX.8.2).
//!
//! [`AccessibilityTree`]는 시각 tree와 **별도의** parent/child 관계를 가집니다. 접근성 의미에 맞게 시각 계층을
//! 건너뛰거나 다시 묶을 수 있습니다(예: Button 합성 Radio를 radio group 아래에 직접 둠, 기술 아키텍처 §18).
//! 대신 각 접근성 node는 **정확히 하나의** source(시각 node)를 가집니다.
//!
//! - node는 source StableId를 key로 저장합니다. 한 source에서 접근성 node가 둘 생길 수 없고(삽입 시
//!   [`TreeError::DuplicateSource`]), node 하나는 source 하나만 가집니다.
//! - [`AccessibilityTree::validate`]는 source가 시각 tree에 없으면(orphan source) 오류를 냅니다. 시각 tree는
//!   [`SourceIndex`]로 조회합니다.
//! - [`AccessibilityTree::validate_structure`]는 접근성 관계가 forest인지 검사합니다: 없는 자식, 중복 자식,
//!   부모 둘 이상, 부모가 있는 root, 중복 root, cycle, root에서 도달 불가.
//!
//! 검사는 모두 반복(비재귀) 순회이며 node·간선을 StableId·목록 순으로 방문해 결과가 결정론적입니다.
//!
//! canonical binary 형식:
//!
//! ```text
//! tree := version:u16 (= 1) ++ count:u32 ++ root[16]*
//!         ++ count:u32 ++ (node ++ count:u32 ++ child[16]*)*   (node source 오름차순, 중복 없음)
//! ```
//!
//! `node`는 [`AccessibilityNode`] 본문 형식(IX.8.1)입니다. 디코더는 형식만 검사하고 관계 유효성은
//! `validate`가 맡습니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::ids::StableId;

use crate::codec::{put_count, put_id, put_u16, CodecError, Reader};
use crate::node::AccessibilityNode;

/// tree binary 형식 버전.
pub const TREE_FORMAT_VERSION: u16 = 1;

/// 시각 tree의 node 존재 조회.
pub trait SourceIndex {
    /// 시각 tree에 `id` node가 있는지.
    fn contains_source(&self, id: StableId) -> bool;
}

impl SourceIndex for BTreeSet<StableId> {
    fn contains_source(&self, id: StableId) -> bool {
        self.contains(&id)
    }
}

/// tree 변경 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeError {
    /// 같은 source의 접근성 node가 이미 있습니다.
    DuplicateSource {
        /// source.
        source: StableId,
    },
}

impl fmt::Display for TreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TreeError::DuplicateSource { source } => {
                write!(f, "source {source}의 접근성 node가 이미 있습니다")
            }
        }
    }
}

impl std::error::Error for TreeError {}

/// tree 유효성 문제.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeIssue {
    /// 접근성 node의 source가 시각 tree에 없습니다.
    OrphanSource {
        /// 접근성 node(source).
        node: StableId,
    },
    /// root 또는 자식 목록이 없는 접근성 node를 가리킵니다.
    MissingNode {
        /// 가리키는 부모(root 목록이면 `None`).
        parent: Option<StableId>,
        /// 없는 node.
        node: StableId,
    },
    /// 한 부모의 자식 목록에 같은 node가 두 번 있습니다.
    DuplicateChild {
        /// 부모.
        parent: StableId,
        /// 자식.
        child: StableId,
    },
    /// root 목록에 같은 node가 두 번 있습니다.
    DuplicateRoot {
        /// root.
        node: StableId,
    },
    /// node의 부모가 둘 이상입니다.
    MultipleParents {
        /// node.
        node: StableId,
        /// 부모(StableId 오름차순).
        parents: Vec<StableId>,
    },
    /// root가 부모를 가집니다.
    RootHasParent {
        /// root.
        node: StableId,
        /// 부모.
        parent: StableId,
    },
    /// parent/child cycle. 경로는 시작 node로 끝납니다(예: `[a, b, a]`).
    Cycle {
        /// cycle 경로.
        path: Vec<StableId>,
    },
    /// 자식 목록의 부모가 접근성 tree에 없습니다.
    MissingParent {
        /// 없는 부모.
        parent: StableId,
    },
    /// root에서 도달할 수 없는 node(cycle에 속한 node는 [`TreeIssue::Cycle`]로만 보고).
    Unreachable {
        /// node.
        node: StableId,
    },
}

impl fmt::Display for TreeIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TreeIssue::OrphanSource { node } => {
                write!(f, "접근성 node {node}의 source가 시각 tree에 없습니다")
            }
            TreeIssue::MissingNode { parent, node } => match parent {
                Some(parent) => write!(f, "부모 {parent}의 자식 {node}가 접근성 tree에 없습니다"),
                None => write!(f, "root {node}가 접근성 tree에 없습니다"),
            },
            TreeIssue::MissingParent { parent } => {
                write!(f, "자식 목록의 부모 {parent}가 접근성 tree에 없습니다")
            }
            TreeIssue::DuplicateChild { parent, child } => {
                write!(f, "부모 {parent}의 자식 목록에 {child}가 중복됩니다")
            }
            TreeIssue::DuplicateRoot { node } => write!(f, "root 목록에 {node}가 중복됩니다"),
            TreeIssue::MultipleParents { node, parents } => {
                write!(f, "접근성 node {node}의 부모가 {}개입니다", parents.len())
            }
            TreeIssue::RootHasParent { node, parent } => {
                write!(f, "root {node}가 부모 {parent}를 가집니다")
            }
            TreeIssue::Cycle { path } => {
                write!(
                    f,
                    "접근성 parent/child cycle(길이 {})",
                    path.len().saturating_sub(1)
                )
            }
            TreeIssue::Unreachable { node } => {
                write!(f, "접근성 node {node}는 root에서 도달할 수 없습니다")
            }
        }
    }
}

/// 접근성 트리: source StableId → node, 접근성 parent/child 관계, root 목록.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AccessibilityTree {
    roots: Vec<StableId>,
    nodes: BTreeMap<StableId, AccessibilityNode>,
    children: BTreeMap<StableId, Vec<StableId>>,
}

impl AccessibilityTree {
    /// 빈 tree.
    pub fn new() -> AccessibilityTree {
        AccessibilityTree::default()
    }

    /// node를 추가합니다. 같은 source의 node가 이미 있으면 거부합니다.
    pub fn insert(&mut self, node: AccessibilityNode) -> Result<(), TreeError> {
        if self.nodes.contains_key(&node.source) {
            return Err(TreeError::DuplicateSource {
                source: node.source,
            });
        }
        self.nodes.insert(node.source, node);
        Ok(())
    }

    /// root 목록 끝에 추가합니다(검사는 [`AccessibilityTree::validate_structure`]).
    pub fn push_root(&mut self, node: StableId) {
        self.roots.push(node);
    }

    /// `parent`의 자식 목록 끝에 추가합니다(검사는 [`AccessibilityTree::validate_structure`]).
    pub fn push_child(&mut self, parent: StableId, child: StableId) {
        self.children.entry(parent).or_default().push(child);
    }

    /// `parent`의 자식 목록을 바꿉니다.
    pub fn set_children(&mut self, parent: StableId, children: Vec<StableId>) {
        if children.is_empty() {
            self.children.remove(&parent);
        } else {
            self.children.insert(parent, children);
        }
    }

    /// root 목록.
    pub fn roots(&self) -> &[StableId] {
        &self.roots
    }

    /// node.
    pub fn node(&self, source: StableId) -> Option<&AccessibilityNode> {
        self.nodes.get(&source)
    }

    /// node(수정 가능).
    pub fn node_mut(&mut self, source: StableId) -> Option<&mut AccessibilityNode> {
        self.nodes.get_mut(&source)
    }

    /// node 목록(source 오름차순).
    pub fn nodes(&self) -> impl Iterator<Item = &AccessibilityNode> + '_ {
        self.nodes.values()
    }

    /// 자식 목록(없으면 빈 slice).
    pub fn children(&self, parent: StableId) -> &[StableId] {
        self.children.get(&parent).map(Vec::as_slice).unwrap_or(&[])
    }

    /// node 수.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// 자식 → 첫 부모(자식 목록 순회 순서상 처음 나온 부모). 부모 둘 이상은 검사가 보고합니다.
    pub fn parents(&self) -> BTreeMap<StableId, StableId> {
        let mut parents = BTreeMap::new();
        for (parent, children) in &self.children {
            for child in children {
                parents.entry(*child).or_insert(*parent);
            }
        }
        parents
    }

    /// 관계와 source를 모두 검사합니다. 빈 목록이면 유효합니다.
    pub fn validate(&self, sources: &dyn SourceIndex) -> Vec<TreeIssue> {
        let mut issues: Vec<TreeIssue> = self
            .nodes
            .keys()
            .filter(|source| !sources.contains_source(**source))
            .map(|source| TreeIssue::OrphanSource { node: *source })
            .collect();
        issues.extend(self.validate_structure());
        issues
    }

    /// 접근성 parent/child 관계가 forest인지 검사합니다. 빈 목록이면 유효합니다.
    pub fn validate_structure(&self) -> Vec<TreeIssue> {
        let mut issues = Vec::new();

        // root 목록: 없는 node·중복.
        let mut seen_roots = BTreeSet::new();
        for root in &self.roots {
            if !self.nodes.contains_key(root) {
                issues.push(TreeIssue::MissingNode {
                    parent: None,
                    node: *root,
                });
            }
            if !seen_roots.insert(*root) {
                issues.push(TreeIssue::DuplicateRoot { node: *root });
            }
        }

        // 자식 목록: 없는 부모·자식, 중복 자식, 부모 수.
        let mut parents_of: BTreeMap<StableId, BTreeSet<StableId>> = BTreeMap::new();
        for (parent, children) in &self.children {
            if !self.nodes.contains_key(parent) {
                issues.push(TreeIssue::MissingParent { parent: *parent });
            }
            let mut seen = BTreeSet::new();
            for child in children {
                if !self.nodes.contains_key(child) {
                    issues.push(TreeIssue::MissingNode {
                        parent: Some(*parent),
                        node: *child,
                    });
                }
                if !seen.insert(*child) {
                    issues.push(TreeIssue::DuplicateChild {
                        parent: *parent,
                        child: *child,
                    });
                }
                parents_of.entry(*child).or_default().insert(*parent);
            }
        }
        for (node, parents) in &parents_of {
            if parents.len() > 1 {
                issues.push(TreeIssue::MultipleParents {
                    node: *node,
                    parents: parents.iter().copied().collect(),
                });
            }
            if seen_roots.contains(node) {
                if let Some(parent) = parents.iter().next() {
                    issues.push(TreeIssue::RootHasParent {
                        node: *node,
                        parent: *parent,
                    });
                }
            }
        }

        // cycle: 반복 3색 DFS.
        let cycles = self.find_cycles();
        let in_cycle: BTreeSet<StableId> = cycles.iter().flatten().copied().collect();
        issues.extend(cycles.into_iter().map(|path| TreeIssue::Cycle { path }));

        // 도달성: root에서 반복 순회.
        let reachable = self.reachable_from_roots();
        for node in self.nodes.keys() {
            if !reachable.contains(node) && !in_cycle.contains(node) {
                issues.push(TreeIssue::Unreachable { node: *node });
            }
        }
        issues
    }

    /// root에서 도달 가능한 node 집합(반복 순회, 방문 집합으로 cycle에도 종료).
    pub(crate) fn reachable_from_roots(&self) -> BTreeSet<StableId> {
        let mut reachable = BTreeSet::new();
        let mut stack: Vec<StableId> = self.roots.iter().rev().copied().collect();
        while let Some(node) = stack.pop() {
            if !self.nodes.contains_key(&node) || !reachable.insert(node) {
                continue;
            }
            stack.extend(self.children(node).iter().rev().copied());
        }
        reachable
    }

    /// 모든 cycle 경로(back edge마다 하나, node StableId 순 방문).
    fn find_cycles(&self) -> Vec<Vec<StableId>> {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Color {
            Gray,
            Black,
        }
        let mut color: BTreeMap<StableId, Color> = BTreeMap::new();
        let mut cycles = Vec::new();
        for start in self.nodes.keys() {
            if color.contains_key(start) {
                continue;
            }
            // (node, 다음에 볼 자식 index)
            let mut stack: Vec<(StableId, usize)> = vec![(*start, 0)];
            color.insert(*start, Color::Gray);
            while let Some(top) = stack.last_mut() {
                let (node, index) = *top;
                let children = self.children(node);
                if index >= children.len() {
                    color.insert(node, Color::Black);
                    stack.pop();
                    continue;
                }
                top.1 += 1;
                let child = children[index];
                if !self.nodes.contains_key(&child) {
                    continue;
                }
                match color.get(&child) {
                    None => {
                        color.insert(child, Color::Gray);
                        stack.push((child, 0));
                    }
                    Some(Color::Gray) => {
                        let from = stack
                            .iter()
                            .position(|(entry, _)| *entry == child)
                            .unwrap_or(0);
                        let mut path: Vec<StableId> =
                            stack[from..].iter().map(|(entry, _)| *entry).collect();
                        path.push(child);
                        cycles.push(path);
                    }
                    Some(Color::Black) => {}
                }
            }
        }
        cycles
    }

    /// canonical bytes. 자식 목록은 부모 node에 속해 기록되므로, 접근성 node가 없는 부모의 자식
    /// 목록([`TreeIssue::MissingParent`])은 기록하지 않습니다.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        put_u16(&mut buf, TREE_FORMAT_VERSION);
        put_count(&mut buf, self.roots.len());
        for root in &self.roots {
            put_id(&mut buf, *root);
        }
        put_count(&mut buf, self.nodes.len());
        for (source, node) in &self.nodes {
            node.encode_into(&mut buf);
            let children = self.children(*source);
            put_count(&mut buf, children.len());
            for child in children {
                put_id(&mut buf, *child);
            }
        }
        buf
    }

    /// [`AccessibilityTree::to_bytes`]의 역. 형식만 검사합니다(관계 유효성은 `validate`).
    pub fn from_bytes(bytes: &[u8]) -> Result<AccessibilityTree, CodecError> {
        let mut reader = Reader::new(bytes);
        if reader.u16()? != TREE_FORMAT_VERSION {
            return Err(CodecError::new("지원하지 않는 tree 형식 버전"));
        }
        let mut tree = AccessibilityTree::new();
        let root_count = reader.count(16)?;
        for _ in 0..root_count {
            tree.roots.push(reader.id()?);
        }
        let node_count = reader.count(16)?;
        let mut previous: Option<StableId> = None;
        for _ in 0..node_count {
            let node = AccessibilityNode::decode_from(&mut reader)?;
            if previous.is_some_and(|p| p >= node.source) {
                return Err(CodecError::new("node source가 오름차순이 아님"));
            }
            previous = Some(node.source);
            let source = node.source;
            let child_count = reader.count(16)?;
            let mut children = Vec::with_capacity(child_count);
            for _ in 0..child_count {
                children.push(reader.id()?);
            }
            tree.nodes.insert(source, node);
            tree.set_children(source, children);
        }
        reader.expect_end()?;
        if tree.to_bytes() != bytes {
            return Err(CodecError::new("canonical 형식이 아님"));
        }
        Ok(tree)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{AccessibilityBinding, AccessibilityRole};
    use crate::test_support::id;

    fn node(n: u8, role: AccessibilityRole) -> AccessibilityNode {
        AccessibilityNode::new(id(n), role)
    }

    /// 시각 tree: 1 > (2, 3 > 4). 접근성 tree는 시각 계층과 다르게 4를 1 아래에 직접 둡니다.
    fn separate_hierarchy() -> (AccessibilityTree, BTreeSet<StableId>) {
        let sources: BTreeSet<StableId> = [1, 2, 3, 4].into_iter().map(id).collect();
        let mut tree = AccessibilityTree::new();
        tree.insert(node(1, AccessibilityRole::RadioGroup))
            .expect("1");
        tree.insert(node(2, AccessibilityRole::Radio).with_label(AccessibilityBinding::text("가")))
            .expect("2");
        tree.insert(node(4, AccessibilityRole::Radio).with_label(AccessibilityBinding::text("나")))
            .expect("4");
        tree.push_root(id(1));
        tree.push_child(id(1), id(2));
        tree.push_child(id(1), id(4));
        (tree, sources)
    }

    #[test]
    fn separate_parent_child_relation_is_valid() {
        let (tree, sources) = separate_hierarchy();
        assert_eq!(tree.validate(&sources), Vec::new());
        // source 3(시각 중간 node)은 접근성 node가 없어도 됩니다.
        assert!(tree.node(id(3)).is_none());
        assert_eq!(tree.children(id(1)), &[id(2), id(4)]);
        assert_eq!(tree.parents().get(&id(4)), Some(&id(1)));
    }

    #[test]
    fn one_source_cannot_produce_two_nodes() {
        let (mut tree, _) = separate_hierarchy();
        assert_eq!(
            tree.insert(node(2, AccessibilityRole::Button)),
            Err(TreeError::DuplicateSource { source: id(2) })
        );
    }

    #[test]
    fn orphan_source_fixture_fails() {
        let (mut tree, sources) = separate_hierarchy();
        tree.insert(node(9, AccessibilityRole::Image)).expect("9");
        tree.push_child(id(1), id(9));
        assert_eq!(
            tree.validate(&sources),
            vec![TreeIssue::OrphanSource { node: id(9) }]
        );
        // 구조만 보면 유효합니다(orphan은 source 검사의 몫).
        assert_eq!(tree.validate_structure(), Vec::new());
    }

    #[test]
    fn cycle_fixture_fails() {
        // root 1 > 2 > 3 > 2 (back edge).
        let sources: BTreeSet<StableId> = [1, 2, 3].into_iter().map(id).collect();
        let mut tree = AccessibilityTree::new();
        for n in [1, 2, 3] {
            tree.insert(node(n, AccessibilityRole::Group))
                .expect("node");
        }
        tree.push_root(id(1));
        tree.push_child(id(1), id(2));
        tree.push_child(id(2), id(3));
        tree.push_child(id(3), id(2));
        let issues = tree.validate(&sources);
        assert!(issues.contains(&TreeIssue::Cycle {
            path: vec![id(2), id(3), id(2)]
        }));
        assert!(issues.contains(&TreeIssue::MultipleParents {
            node: id(2),
            parents: vec![id(1), id(3)],
        }));

        // root 없는 self cycle: cycle로만 보고(도달 불가 중복 보고 없음).
        let mut tree = AccessibilityTree::new();
        tree.insert(node(5, AccessibilityRole::Group)).expect("5");
        tree.push_child(id(5), id(5));
        assert_eq!(
            tree.validate_structure(),
            vec![TreeIssue::Cycle {
                path: vec![id(5), id(5)]
            }]
        );
    }

    #[test]
    fn structural_violations_are_reported() {
        let mut tree = AccessibilityTree::new();
        for n in [1, 2, 3, 4] {
            tree.insert(node(n, AccessibilityRole::Generic))
                .expect("node");
        }
        tree.push_root(id(1));
        tree.push_root(id(1));
        tree.push_root(id(2));
        tree.push_root(id(8));
        tree.push_child(id(1), id(2));
        tree.push_child(id(1), id(7));
        tree.push_child(id(1), id(3));
        tree.push_child(id(1), id(3));
        tree.push_child(id(6), id(4));
        let issues = tree.validate_structure();
        assert_eq!(
            issues,
            vec![
                TreeIssue::DuplicateRoot { node: id(1) },
                TreeIssue::MissingNode {
                    parent: None,
                    node: id(8)
                },
                TreeIssue::MissingNode {
                    parent: Some(id(1)),
                    node: id(7)
                },
                TreeIssue::DuplicateChild {
                    parent: id(1),
                    child: id(3)
                },
                TreeIssue::MissingParent { parent: id(6) },
                TreeIssue::RootHasParent {
                    node: id(2),
                    parent: id(1)
                },
                TreeIssue::Unreachable { node: id(4) },
            ]
        );
        // 결정론: 같은 입력 같은 결과.
        assert_eq!(tree.validate_structure(), issues);
    }

    #[test]
    fn tree_round_trips_and_rejects_malformed_bytes() {
        let (tree, _) = separate_hierarchy();
        let bytes = tree.to_bytes();
        assert_eq!(AccessibilityTree::from_bytes(&bytes), Ok(tree.clone()));
        assert_eq!(
            AccessibilityTree::from_bytes(&AccessibilityTree::new().to_bytes()),
            Ok(AccessibilityTree::new())
        );
        for len in 0..bytes.len() {
            assert!(
                AccessibilityTree::from_bytes(&bytes[..len]).is_err(),
                "len {len}"
            );
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(AccessibilityTree::from_bytes(&trailing).is_err());

        // node source 정렬 위반: 두 node를 역순으로 기록.
        let mut buf = Vec::new();
        put_u16(&mut buf, TREE_FORMAT_VERSION);
        put_count(&mut buf, 0);
        put_count(&mut buf, 2);
        for n in [2, 1] {
            node(n, AccessibilityRole::Generic).encode_into(&mut buf);
            put_count(&mut buf, 0);
        }
        assert_eq!(
            AccessibilityTree::from_bytes(&buf),
            Err(CodecError::new("node source가 오름차순이 아님"))
        );
    }
}
