//! Expression AST cycle detector (VIII.2.3).
//!
//! AST는 한 node를 여러 부모가 공유하는 DAG 참조를 허용하지만, 어떤 node도 자기 자신에게 (직접 또는
//! 다른 node를 거쳐) 되돌아가서는 안 됩니다. cycle은 무한 평가·재귀와 같으므로 표현식 모델에서
//! 금지합니다(아키텍처 §14.1 "임의 반복문과 재귀" 금지).
//!
//! 검출은 root에서 시작하는 반복(비재귀) 3색 DFS입니다. child는 slot 순서로 방문하므로 같은 AST는 항상
//! 같은 cycle 경로를 보고합니다. 경로는 `[a, …, a]`처럼 시작 node로 닫힙니다.

use std::collections::BTreeMap;

use crate::ast::{ExprNodeId, ExpressionAst};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    /// DFS stack 위에 있음.
    Open,
    /// 모든 자손 방문 완료.
    Done,
}

/// root에서 도달 가능한 첫 cycle 경로를 찾습니다. 없으면 `None`.
///
/// 경로는 cycle의 시작 node로 시작하고 끝납니다. 자기 자신을 child로 가진 node는 `[a, a]`입니다.
pub fn find_cycle(ast: &ExpressionAst) -> Option<Vec<ExprNodeId>> {
    let mut marks: BTreeMap<ExprNodeId, Mark> = BTreeMap::new();
    // (node, 다음에 볼 child 위치)
    let mut stack: Vec<(ExprNodeId, usize)> = vec![(ast.root(), 0)];
    marks.insert(ast.root(), Mark::Open);
    while let Some(&(id, next)) = stack.last() {
        let child = ast
            .node(id)
            .and_then(|node| node.children().get(next))
            .map(|child| child.target);
        let Some(child) = child else {
            marks.insert(id, Mark::Done);
            stack.pop();
            continue;
        };
        if let Some(top) = stack.last_mut() {
            top.1 += 1;
        }
        match marks.get(&child) {
            Some(Mark::Done) => {}
            Some(Mark::Open) => {
                let start = stack
                    .iter()
                    .position(|(open, _)| *open == child)
                    .unwrap_or(0);
                let mut path: Vec<ExprNodeId> =
                    stack[start..].iter().map(|(open, _)| *open).collect();
                path.push(child);
                return Some(path);
            }
            None => {
                marks.insert(child, Mark::Open);
                stack.push((child, 0));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use stellar_core_types::ids::StableId;
    use stellar_core_types::typed_value::TypedValue;
    use stellar_core_types::value::FiniteF32;

    use crate::ast::{AstError, ExprNode};
    use crate::ops::{BinaryOp, UnaryOp};

    use super::*;

    fn id(n: u64) -> ExprNodeId {
        ExprNodeId::new(StableId::new_v7(1_700_000_000_000 + n, 0, n))
    }

    fn leaf(n: u64) -> ExprNode {
        ExprNode::literal(id(n), TypedValue::Number(FiniteF32::ONE))
    }

    fn neg(n: u64, operand: u64) -> ExprNode {
        ExprNode::unary(id(n), UnaryOp::Negate, id(operand))
    }

    fn add(n: u64, lhs: u64, rhs: u64) -> ExprNode {
        ExprNode::binary(id(n), BinaryOp::Add, id(lhs), id(rhs))
    }

    fn cycle_of(result: Result<ExpressionAst, AstError>) -> Vec<ExprNodeId> {
        match result {
            Err(AstError::Cycle { path }) => path,
            other => panic!("cycle 오류 기대: {other:?}"),
        }
    }

    #[test]
    fn direct_cycle_fixtures_fail() {
        // 자기 자신을 피연산자로 가진 node.
        let path = cycle_of(ExpressionAst::new(id(1), vec![neg(1, 1)]));
        assert_eq!(path, vec![id(1), id(1)]);
        // Rhs가 자기 자신(Lhs는 정상 leaf).
        let path = cycle_of(ExpressionAst::new(id(1), vec![add(1, 2, 1), leaf(2)]));
        assert_eq!(path, vec![id(1), id(1)]);
        // 두 node가 서로를 가리킴: 1 → 2 → 1.
        let path = cycle_of(ExpressionAst::new(id(1), vec![neg(1, 2), neg(2, 1)]));
        assert_eq!(path, vec![id(1), id(2), id(1)]);
    }

    #[test]
    fn indirect_cycle_fixtures_fail() {
        // 1 → 2 → 3 → 4 → 2 (root 아래에서 닫히는 cycle).
        let path = cycle_of(ExpressionAst::new(
            id(1),
            vec![add(1, 2, 9), neg(2, 3), add(3, 9, 4), neg(4, 2), leaf(9)],
        ));
        assert_eq!(path, vec![id(2), id(3), id(4), id(2)]);
        // 공유 node를 거쳐 root로 되돌아감: 1 → 5(공유) → 1.
        let path = cycle_of(ExpressionAst::new(
            id(1),
            vec![add(1, 5, 6), neg(6, 5), neg(5, 1)],
        ));
        assert_eq!(path, vec![id(1), id(5), id(1)]);
    }

    #[test]
    fn dag_sharing_is_not_a_cycle() {
        // diamond: 1 = 2 + 3, 2 = -4, 3 = -4 (4 공유).
        let ast = ExpressionAst::new(id(1), vec![add(1, 2, 3), neg(2, 4), neg(3, 4), leaf(4)])
            .expect("DAG 허용");
        assert_eq!(find_cycle(&ast), None);
        // 같은 child를 양 slot에 연결.
        let ast = ExpressionAst::new(id(1), vec![add(1, 2, 2), leaf(2)]).expect("DAG 허용");
        assert_eq!(find_cycle(&ast), None);
    }

    #[test]
    fn deep_graphs_do_not_overflow_the_stack() {
        // 깊이 100,000 단항 chain(비순환) 통과.
        let depth = 100_000u64;
        let mut nodes: Vec<ExprNode> = (1..depth).map(|n| neg(n, n + 1)).collect();
        nodes.push(leaf(depth));
        assert!(ExpressionAst::new(id(1), nodes).is_ok());
        // 같은 깊이에서 끝이 root로 되돌아가는 cycle 검출.
        let mut nodes: Vec<ExprNode> = (1..depth).map(|n| neg(n, n + 1)).collect();
        nodes.push(neg(depth, 1));
        let path = cycle_of(ExpressionAst::new(id(1), nodes));
        assert_eq!(path.len(), depth as usize + 1);
        assert_eq!(path.first(), Some(&id(1)));
        assert_eq!(path.last(), Some(&id(1)));
        // 공유가 많은 diamond chain(경로 수 2^64 규모)도 선형 시간에 통과.
        let mut nodes = Vec::new();
        for level in 0..64u64 {
            let base = level * 3 + 1;
            nodes.push(add(base, base + 1, base + 2));
            nodes.push(neg(base + 1, base + 3));
            nodes.push(neg(base + 2, base + 3));
        }
        nodes.push(leaf(64 * 3 + 1));
        assert!(ExpressionAst::new(id(1), nodes).is_ok());
    }

    #[test]
    fn detection_is_deterministic() {
        // node 삽입 순서가 달라도 같은 경로(child slot 순서 방문).
        let a = cycle_of(ExpressionAst::new(
            id(1),
            vec![add(1, 2, 3), neg(2, 1), neg(3, 1)],
        ));
        let b = cycle_of(ExpressionAst::new(
            id(1),
            vec![neg(3, 1), neg(2, 1), add(1, 2, 3)],
        ));
        assert_eq!(a, b);
        assert_eq!(a, vec![id(1), id(2), id(1)], "Lhs 쪽 cycle을 먼저 보고");
    }
}
