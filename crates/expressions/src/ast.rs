//! Expression AST 공통 node 구조 (VIII.2.1).
//!
//! 표현식의 원본은 타입이 지정된 AST입니다(아키텍처 §14.1). 모든 node는
//!
//! - 영속 식별자 [`ExprNodeId`](`StableId`) — Semantic Diff·Merge 단위,
//! - node 종류 [`NodeKind`](닫힌 7개, VIII.1.5),
//! - typed child 참조 [`ChildRef`] — 어느 slot([`ChildSlot`])에 어느 node가 연결되는지
//!
//! 를 가집니다. child는 node를 직접 소유하지 않고 ID로 가리키므로, 한 node를 여러 부모가 공유하는
//! **DAG**를 표현할 수 있습니다. [`ExpressionAst::new`]는 ID 유일성, root 존재, child 참조 해석,
//! kind별 slot 구성, root에서의 도달 가능성을 검증합니다. 모든 검증은 재귀 없이 명시적 stack으로
//! 수행합니다.
//!
//! kind별 slot 구성:
//!
//! | kind | slot |
//! |---|---|
//! | Literal, Reference | 없음 |
//! | Unary | `Operand` |
//! | Binary | `Lhs`, `Rhs` |
//! | Call | `Argument(0)`, …, `Argument(n-1)` (연속) |
//! | If | `Condition`, `Then`, `Else` |
//! | Match | `Scrutinee`, `Arm(0)`, …, `Arm(n-1)` (연속, n ≥ 1), `Default` |

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::ids::StableId;

use crate::ops::NodeKind;

/// 표현식 AST node 식별자. node의 [`StableId`]를 감싼 typed ID입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExprNodeId(StableId);

impl ExprNodeId {
    /// `StableId`로부터 만듭니다.
    pub const fn new(id: StableId) -> ExprNodeId {
        ExprNodeId(id)
    }

    /// 내부 `StableId`.
    pub const fn stable_id(&self) -> StableId {
        self.0
    }
}

impl fmt::Display for ExprNodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// child가 연결되는 위치. 선언 순서(같은 종류는 index 순)가 canonical slot 순서입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChildSlot {
    /// 단항 연산의 피연산자.
    Operand,
    /// 이항 연산의 왼쪽.
    Lhs,
    /// 이항 연산의 오른쪽.
    Rhs,
    /// 함수 호출의 `index`번째 인자.
    Argument(u16),
    /// `if` 조건.
    Condition,
    /// `if` 참 branch.
    Then,
    /// `if` 거짓 branch.
    Else,
    /// `match` 검사 대상.
    Scrutinee,
    /// `match`의 `index`번째 arm 값.
    Arm(u16),
    /// `match` 기본 branch.
    Default,
}

impl ChildSlot {
    /// 안정 numeric tag(직렬화 포맷 계약).
    pub const fn tag(&self) -> u8 {
        match self {
            ChildSlot::Operand => 0x01,
            ChildSlot::Lhs => 0x02,
            ChildSlot::Rhs => 0x03,
            ChildSlot::Argument(_) => 0x04,
            ChildSlot::Condition => 0x05,
            ChildSlot::Then => 0x06,
            ChildSlot::Else => 0x07,
            ChildSlot::Scrutinee => 0x08,
            ChildSlot::Arm(_) => 0x09,
            ChildSlot::Default => 0x0A,
        }
    }

    /// 순번 slot(`Argument`, `Arm`)의 index. 그 외는 0.
    pub const fn index(&self) -> u16 {
        match self {
            ChildSlot::Argument(index) | ChildSlot::Arm(index) => *index,
            _ => 0,
        }
    }

    /// tag와 index로 복원합니다. 미등록 tag나 순번 없는 slot의 0이 아닌 index는 `None`.
    pub const fn from_parts(tag: u8, index: u16) -> Option<ChildSlot> {
        let slot = match tag {
            0x01 => ChildSlot::Operand,
            0x02 => ChildSlot::Lhs,
            0x03 => ChildSlot::Rhs,
            0x04 => return Some(ChildSlot::Argument(index)),
            0x05 => ChildSlot::Condition,
            0x06 => ChildSlot::Then,
            0x07 => ChildSlot::Else,
            0x08 => ChildSlot::Scrutinee,
            0x09 => return Some(ChildSlot::Arm(index)),
            0x0A => ChildSlot::Default,
            _ => return None,
        };
        if index == 0 {
            Some(slot)
        } else {
            None
        }
    }
}

impl fmt::Display for ChildSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChildSlot::Operand => f.write_str("operand"),
            ChildSlot::Lhs => f.write_str("lhs"),
            ChildSlot::Rhs => f.write_str("rhs"),
            ChildSlot::Argument(index) => write!(f, "argument[{index}]"),
            ChildSlot::Condition => f.write_str("condition"),
            ChildSlot::Then => f.write_str("then"),
            ChildSlot::Else => f.write_str("else"),
            ChildSlot::Scrutinee => f.write_str("scrutinee"),
            ChildSlot::Arm(index) => write!(f, "arm[{index}]"),
            ChildSlot::Default => f.write_str("default"),
        }
    }
}

/// typed child 참조: `slot`에 `target` node가 연결됩니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChildRef {
    /// 연결 위치.
    pub slot: ChildSlot,
    /// 연결된 child node.
    pub target: ExprNodeId,
}

impl ChildRef {
    /// child 참조를 만듭니다.
    pub const fn new(slot: ChildSlot, target: ExprNodeId) -> ChildRef {
        ChildRef { slot, target }
    }
}

/// AST node 하나 (VIII.2.1).
///
/// child 목록은 생성 시 slot 순서로 정렬해 보관하므로, 연결 순서가 달라도 같은 node가 됩니다.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExprNode {
    id: ExprNodeId,
    kind: NodeKind,
    children: Vec<ChildRef>,
}

impl ExprNode {
    /// node를 만듭니다. `children`은 slot 순서로 정렬됩니다. 구성 검증은 [`ExpressionAst::new`]가 합니다.
    pub fn new(id: ExprNodeId, kind: NodeKind, mut children: Vec<ChildRef>) -> ExprNode {
        children.sort();
        ExprNode { id, kind, children }
    }

    /// node 식별자.
    pub const fn id(&self) -> ExprNodeId {
        self.id
    }

    /// node 종류.
    pub const fn kind(&self) -> NodeKind {
        self.kind
    }

    /// child 참조(slot 순서).
    pub fn children(&self) -> &[ChildRef] {
        &self.children
    }

    /// slot에 연결된 child.
    pub fn child(&self, slot: ChildSlot) -> Option<ExprNodeId> {
        self.children
            .iter()
            .find(|child| child.slot == slot)
            .map(|child| child.target)
    }
}

/// AST 구조 검증 오류 (VIII.2.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AstError {
    /// 같은 ID의 node가 두 번 이상 있습니다.
    DuplicateNodeId(ExprNodeId),
    /// root ID의 node가 없습니다.
    MissingRoot(ExprNodeId),
    /// child 참조가 존재하지 않는 node를 가리킵니다.
    DanglingChild {
        /// 부모 node.
        parent: ExprNodeId,
        /// 연결 위치.
        slot: ChildSlot,
        /// 존재하지 않는 대상.
        target: ExprNodeId,
    },
    /// 한 node에 같은 slot이 두 번 이상 있습니다.
    DuplicateSlot {
        /// node.
        node: ExprNodeId,
        /// 중복 slot.
        slot: ChildSlot,
    },
    /// node 종류에 허용되지 않는 slot입니다(순번 slot의 건너뛴 index 포함).
    UnexpectedSlot {
        /// node.
        node: ExprNodeId,
        /// 허용되지 않는 slot.
        slot: ChildSlot,
    },
    /// node 종류에 필요한 slot이 비어 있습니다.
    MissingSlot {
        /// node.
        node: ExprNodeId,
        /// 비어 있는 slot.
        slot: ChildSlot,
    },
    /// root에서 도달할 수 없는 node가 있습니다.
    UnreachableNode(ExprNodeId),
}

impl fmt::Display for AstError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AstError::DuplicateNodeId(id) => write!(f, "node ID가 중복됩니다: {id}"),
            AstError::MissingRoot(id) => write!(f, "root node가 없습니다: {id}"),
            AstError::DanglingChild {
                parent,
                slot,
                target,
            } => write!(f, "{parent}의 {slot}이 없는 node {target}를 가리킵니다"),
            AstError::DuplicateSlot { node, slot } => {
                write!(f, "{node}에 {slot} slot이 중복됩니다")
            }
            AstError::UnexpectedSlot { node, slot } => {
                write!(f, "{node}의 종류에 {slot} slot은 허용되지 않습니다")
            }
            AstError::MissingSlot { node, slot } => {
                write!(f, "{node}의 {slot} slot이 비어 있습니다")
            }
            AstError::UnreachableNode(id) => write!(f, "root에서 도달할 수 없는 node: {id}"),
        }
    }
}

impl std::error::Error for AstError {}

/// 검증된 표현식 AST (VIII.2.1).
///
/// node는 삽입 순서로 보관하고 ID 색인을 함께 둡니다. 동등성(`PartialEq`)은 삽입 순서와 무관하게
/// root와 node 집합으로 판단합니다.
#[derive(Debug, Clone)]
pub struct ExpressionAst {
    root: ExprNodeId,
    nodes: Vec<ExprNode>,
    index: BTreeMap<ExprNodeId, usize>,
}

impl ExpressionAst {
    /// node 목록과 root로 AST를 만들고 구조를 검증합니다.
    ///
    /// 검사 순서: ID 유일성 → root 존재 → node별 slot 구성 → child 참조 해석 → 도달 가능성.
    pub fn new(root: ExprNodeId, nodes: Vec<ExprNode>) -> Result<ExpressionAst, AstError> {
        let mut index = BTreeMap::new();
        for (position, node) in nodes.iter().enumerate() {
            if index.insert(node.id, position).is_some() {
                return Err(AstError::DuplicateNodeId(node.id));
            }
        }
        if !index.contains_key(&root) {
            return Err(AstError::MissingRoot(root));
        }
        for node in &nodes {
            validate_slots(node)?;
            for child in &node.children {
                if !index.contains_key(&child.target) {
                    return Err(AstError::DanglingChild {
                        parent: node.id,
                        slot: child.slot,
                        target: child.target,
                    });
                }
            }
        }
        let ast = ExpressionAst { root, nodes, index };
        let reachable = ast.reachable_from_root();
        if let Some(node) = ast.nodes.iter().find(|node| !reachable.contains(&node.id)) {
            return Err(AstError::UnreachableNode(node.id));
        }
        Ok(ast)
    }

    /// root node ID.
    pub const fn root(&self) -> ExprNodeId {
        self.root
    }

    /// ID로 node를 찾습니다.
    pub fn node(&self, id: ExprNodeId) -> Option<&ExprNode> {
        self.index
            .get(&id)
            .and_then(|&position| self.nodes.get(position))
    }

    /// 모든 node(삽입 순서).
    pub fn nodes(&self) -> &[ExprNode] {
        &self.nodes
    }

    /// node 수.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// node가 없으면 `true`. 검증된 AST는 root가 있으므로 항상 `false`입니다.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// node별 부모 수(공유 DAG node는 2 이상). root는 0입니다.
    pub fn parent_counts(&self) -> BTreeMap<ExprNodeId, usize> {
        let mut counts: BTreeMap<ExprNodeId, usize> =
            self.nodes.iter().map(|node| (node.id, 0)).collect();
        for node in &self.nodes {
            for child in &node.children {
                if let Some(count) = counts.get_mut(&child.target) {
                    *count += 1;
                }
            }
        }
        counts
    }

    /// root에서 도달 가능한 node 집합(명시적 stack, 공유·순환 참조에서도 종료).
    fn reachable_from_root(&self) -> BTreeSet<ExprNodeId> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![self.root];
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            if let Some(node) = self.node(id) {
                stack.extend(node.children.iter().map(|child| child.target));
            }
        }
        seen
    }
}

impl PartialEq for ExpressionAst {
    fn eq(&self, other: &ExpressionAst) -> bool {
        self.root == other.root
            && self.nodes.len() == other.nodes.len()
            && self
                .nodes
                .iter()
                .all(|node| other.node(node.id) == Some(node))
    }
}

impl Eq for ExpressionAst {}

/// kind별 slot 구성을 검사합니다. child는 slot 순서로 정렬되어 있습니다.
fn validate_slots(node: &ExprNode) -> Result<(), AstError> {
    let id = node.id;
    for pair in node.children.windows(2) {
        if pair[0].slot == pair[1].slot {
            return Err(AstError::DuplicateSlot {
                node: id,
                slot: pair[0].slot,
            });
        }
    }
    let fixed: &[ChildSlot] = match node.kind {
        NodeKind::Literal | NodeKind::Reference => &[],
        NodeKind::Unary => &[ChildSlot::Operand],
        NodeKind::Binary => &[ChildSlot::Lhs, ChildSlot::Rhs],
        NodeKind::If => &[ChildSlot::Condition, ChildSlot::Then, ChildSlot::Else],
        NodeKind::Call => return validate_sequence(node, SequenceSlot::Argument, 0),
        NodeKind::Match => {
            require(node, ChildSlot::Scrutinee)?;
            require(node, ChildSlot::Default)?;
            return validate_sequence(node, SequenceSlot::Arm, 1);
        }
    };
    for child in &node.children {
        if !fixed.contains(&child.slot) {
            return Err(AstError::UnexpectedSlot {
                node: id,
                slot: child.slot,
            });
        }
    }
    for slot in fixed {
        require(node, *slot)?;
    }
    Ok(())
}

/// 순번 slot 종류.
#[derive(Clone, Copy)]
enum SequenceSlot {
    Argument,
    Arm,
}

impl SequenceSlot {
    fn slot(self, index: u16) -> ChildSlot {
        match self {
            SequenceSlot::Argument => ChildSlot::Argument(index),
            SequenceSlot::Arm => ChildSlot::Arm(index),
        }
    }

    fn index_of(self, slot: ChildSlot) -> Option<u16> {
        match (self, slot) {
            (SequenceSlot::Argument, ChildSlot::Argument(index))
            | (SequenceSlot::Arm, ChildSlot::Arm(index)) => Some(index),
            _ => None,
        }
    }
}

/// 순번 slot이 0부터 빈틈없이 이어지고, 그 외 slot은 Match의 고정 slot뿐인지 검사합니다.
fn validate_sequence(node: &ExprNode, kind: SequenceSlot, minimum: usize) -> Result<(), AstError> {
    let mut expected: u16 = 0;
    let mut count = 0usize;
    for child in &node.children {
        match kind.index_of(child.slot) {
            Some(index) if index == expected => {
                expected = expected.wrapping_add(1);
                count += 1;
            }
            Some(_) => {
                return Err(AstError::UnexpectedSlot {
                    node: node.id,
                    slot: child.slot,
                })
            }
            None => {
                let allowed_fixed = matches!(kind, SequenceSlot::Arm)
                    && matches!(child.slot, ChildSlot::Scrutinee | ChildSlot::Default);
                if !allowed_fixed {
                    return Err(AstError::UnexpectedSlot {
                        node: node.id,
                        slot: child.slot,
                    });
                }
            }
        }
    }
    if count < minimum {
        return Err(AstError::MissingSlot {
            node: node.id,
            slot: kind.slot(0),
        });
    }
    Ok(())
}

fn require(node: &ExprNode, slot: ChildSlot) -> Result<(), AstError> {
    if node.child(slot).is_some() {
        Ok(())
    } else {
        Err(AstError::MissingSlot {
            node: node.id,
            slot,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u64) -> ExprNodeId {
        ExprNodeId::new(StableId::new_v7(1_700_000_000_000 + n, 0, n))
    }

    fn leaf(n: u64) -> ExprNode {
        ExprNode::new(id(n), NodeKind::Literal, vec![])
    }

    fn binary(n: u64, lhs: u64, rhs: u64) -> ExprNode {
        ExprNode::new(
            id(n),
            NodeKind::Binary,
            vec![
                ChildRef::new(ChildSlot::Lhs, id(lhs)),
                ChildRef::new(ChildSlot::Rhs, id(rhs)),
            ],
        )
    }

    #[test]
    fn tree_validates() {
        // (1 + 2) + 3 형태의 tree.
        let ast = ExpressionAst::new(
            id(10),
            vec![
                binary(10, 11, 3),
                binary(11, 1, 2),
                leaf(1),
                leaf(2),
                leaf(3),
            ],
        )
        .expect("유효한 tree");
        assert_eq!(ast.root(), id(10));
        assert_eq!(ast.len(), 5);
        assert_eq!(ast.node(id(11)).map(ExprNode::kind), Some(NodeKind::Binary));
        assert!(ast.parent_counts().values().all(|&count| count <= 1));
    }

    #[test]
    fn shared_child_dag_validates() {
        // 같은 leaf를 Lhs·Rhs 양쪽과 다른 부모가 공유(DAG).
        let ast = ExpressionAst::new(id(10), vec![binary(10, 11, 1), binary(11, 1, 1), leaf(1)])
            .expect("유효한 DAG");
        assert_eq!(ast.parent_counts()[&id(1)], 3);
        assert_eq!(ast.parent_counts()[&id(10)], 0);
    }

    #[test]
    fn id_uniqueness_is_enforced() {
        let err = ExpressionAst::new(id(10), vec![binary(10, 1, 2), leaf(1), leaf(1)])
            .expect_err("중복 ID");
        assert_eq!(err, AstError::DuplicateNodeId(id(1)));
        // root와 child ID 충돌도 중복.
        let err = ExpressionAst::new(id(1), vec![leaf(1), leaf(1)]).expect_err("중복 ID");
        assert_eq!(err, AstError::DuplicateNodeId(id(1)));
    }

    #[test]
    fn structural_errors_are_reported() {
        assert_eq!(
            ExpressionAst::new(id(9), vec![leaf(1)]),
            Err(AstError::MissingRoot(id(9)))
        );
        assert_eq!(
            ExpressionAst::new(id(10), vec![binary(10, 1, 7), leaf(1)]),
            Err(AstError::DanglingChild {
                parent: id(10),
                slot: ChildSlot::Rhs,
                target: id(7),
            })
        );
        assert_eq!(
            ExpressionAst::new(id(10), vec![binary(10, 1, 2), leaf(1), leaf(2), leaf(3)]),
            Err(AstError::UnreachableNode(id(3)))
        );
        // Binary에 Rhs 없음.
        let half = ExprNode::new(
            id(10),
            NodeKind::Binary,
            vec![ChildRef::new(ChildSlot::Lhs, id(1))],
        );
        assert_eq!(
            ExpressionAst::new(id(10), vec![half, leaf(1)]),
            Err(AstError::MissingSlot {
                node: id(10),
                slot: ChildSlot::Rhs,
            })
        );
        // Literal에 child.
        let literal_with_child = ExprNode::new(
            id(10),
            NodeKind::Literal,
            vec![ChildRef::new(ChildSlot::Operand, id(1))],
        );
        assert_eq!(
            ExpressionAst::new(id(10), vec![literal_with_child, leaf(1)]),
            Err(AstError::UnexpectedSlot {
                node: id(10),
                slot: ChildSlot::Operand,
            })
        );
        // 같은 slot 두 번.
        let twice = ExprNode::new(
            id(10),
            NodeKind::Unary,
            vec![
                ChildRef::new(ChildSlot::Operand, id(1)),
                ChildRef::new(ChildSlot::Operand, id(2)),
            ],
        );
        assert_eq!(
            ExpressionAst::new(id(10), vec![twice, leaf(1), leaf(2)]),
            Err(AstError::DuplicateSlot {
                node: id(10),
                slot: ChildSlot::Operand,
            })
        );
    }

    #[test]
    fn sequence_slots_must_be_contiguous() {
        let call = |slots: Vec<ChildSlot>| {
            ExprNode::new(
                id(10),
                NodeKind::Call,
                slots
                    .into_iter()
                    .enumerate()
                    .map(|(n, slot)| ChildRef::new(slot, id(n as u64 + 1)))
                    .collect(),
            )
        };
        let ok = call(vec![ChildSlot::Argument(0), ChildSlot::Argument(1)]);
        assert!(ExpressionAst::new(id(10), vec![ok, leaf(1), leaf(2)]).is_ok());
        let gap = call(vec![ChildSlot::Argument(0), ChildSlot::Argument(2)]);
        assert_eq!(
            ExpressionAst::new(id(10), vec![gap, leaf(1), leaf(2)]),
            Err(AstError::UnexpectedSlot {
                node: id(10),
                slot: ChildSlot::Argument(2),
            })
        );
        let wrong = call(vec![ChildSlot::Argument(0), ChildSlot::Then]);
        assert!(matches!(
            ExpressionAst::new(id(10), vec![wrong, leaf(1), leaf(2)]),
            Err(AstError::UnexpectedSlot { .. })
        ));
        // Match: Scrutinee + Arm(0..) + Default, arm 최소 1개.
        let matched = ExprNode::new(
            id(10),
            NodeKind::Match,
            vec![
                ChildRef::new(ChildSlot::Default, id(3)),
                ChildRef::new(ChildSlot::Arm(0), id(2)),
                ChildRef::new(ChildSlot::Scrutinee, id(1)),
            ],
        );
        assert!(ExpressionAst::new(id(10), vec![matched, leaf(1), leaf(2), leaf(3)]).is_ok());
        let no_arm = ExprNode::new(
            id(10),
            NodeKind::Match,
            vec![
                ChildRef::new(ChildSlot::Default, id(3)),
                ChildRef::new(ChildSlot::Scrutinee, id(1)),
            ],
        );
        assert_eq!(
            ExpressionAst::new(id(10), vec![no_arm, leaf(1), leaf(3)]),
            Err(AstError::MissingSlot {
                node: id(10),
                slot: ChildSlot::Arm(0),
            })
        );
    }

    #[test]
    fn child_order_and_insertion_order_do_not_matter() {
        let a = ExpressionAst::new(id(10), vec![binary(10, 1, 2), leaf(1), leaf(2)]).expect("유효");
        let reversed_children = ExprNode::new(
            id(10),
            NodeKind::Binary,
            vec![
                ChildRef::new(ChildSlot::Rhs, id(2)),
                ChildRef::new(ChildSlot::Lhs, id(1)),
            ],
        );
        let b =
            ExpressionAst::new(id(10), vec![leaf(2), leaf(1), reversed_children]).expect("유효");
        assert_eq!(a, b);
        assert_eq!(
            b.node(id(10)).map(|node| node.children().to_vec()),
            Some(vec![
                ChildRef::new(ChildSlot::Lhs, id(1)),
                ChildRef::new(ChildSlot::Rhs, id(2)),
            ])
        );
    }

    #[test]
    fn child_slot_tags_round_trip() {
        let slots = [
            ChildSlot::Operand,
            ChildSlot::Lhs,
            ChildSlot::Rhs,
            ChildSlot::Argument(3),
            ChildSlot::Condition,
            ChildSlot::Then,
            ChildSlot::Else,
            ChildSlot::Scrutinee,
            ChildSlot::Arm(65_535),
            ChildSlot::Default,
        ];
        for slot in slots {
            assert_eq!(ChildSlot::from_parts(slot.tag(), slot.index()), Some(slot));
        }
        assert_eq!(ChildSlot::from_parts(0x00, 0), None);
        assert_eq!(ChildSlot::from_parts(0x0B, 0), None);
        assert_eq!(ChildSlot::from_parts(0x02, 1), None, "Lhs에 index 금지");
        // canonical slot 순서.
        let mut sorted = slots.to_vec();
        sorted.sort();
        assert_eq!(sorted, slots.to_vec());
        assert!(ChildSlot::Arm(1) < ChildSlot::Arm(2));
        assert!(ChildSlot::Arm(9) < ChildSlot::Default);
    }
}
