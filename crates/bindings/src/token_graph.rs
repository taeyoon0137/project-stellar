//! 토큰 dependency graph와 저장 전 cycle 거부 (VIII.5.3).
//!
//! 토큰 `A`의 default expression이 토큰 `B`를 참조하면 간선 `A → B`(A는 B에 의존)입니다. registry에 있는 토큰만
//! node가 되며, 없는 토큰 참조는 [`TokenRegistry::reference_issues`]가 따로 보고합니다.
//!
//! 토큰 cycle(`A → B → A`, 자기 참조 `A → A`)은 평가가 끝나지 않으므로 저장할 수 없습니다.
//! [`validate_for_save`]는 cycle·참조 문제·type 오류를 모아 저장 전에 거부합니다. cycle 검출은 반복(비재귀) 3색
//! DFS이며 토큰·간선을 ID 순으로 방문해 같은 registry는 항상 같은 경로를 보고합니다.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use stellar_core_types::reference::TokenRef;
use stellar_expressions::typecheck::ReferenceTypes;

use crate::registry::{ReferenceIssue, TokenRegistry};
use crate::token::{TokenReferenceTypes, TokenTypeError};

/// 토큰 cycle. 경로는 시작 토큰으로 닫힙니다(`[a, …, a]`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TokenCycle {
    /// cycle 경로.
    pub path: Vec<TokenRef>,
}

impl fmt::Display for TokenCycle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("토큰 참조 cycle: ")?;
        for (index, token) in self.path.iter().enumerate() {
            if index > 0 {
                f.write_str(" → ")?;
            }
            write!(f, "{}", token.id())?;
        }
        Ok(())
    }
}

impl std::error::Error for TokenCycle {}

/// 토큰 간 의존 graph (VIII.5.3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenDependencyGraph {
    /// 토큰별 의존 대상(ID 오름차순, 중복 없음).
    dependencies: BTreeMap<TokenRef, Vec<TokenRef>>,
    dependents: BTreeMap<TokenRef, BTreeSet<TokenRef>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    Open,
    Done,
}

impl TokenDependencyGraph {
    /// registry의 모든 토큰으로 graph를 만듭니다. registry에 없는 대상으로 가는 간선은 넣지 않습니다. 단 orphan
    /// 기록(VIII.5.5)이 있는 토큰으로 가는 간선은 넣어, 같은 ID를 다시 만들 때 생길 cycle을 미리 찾을 수 있게 합니다.
    pub fn build(registry: &TokenRegistry) -> TokenDependencyGraph {
        let mut graph = TokenDependencyGraph::default();
        for (_, definition) in registry.tokens() {
            let id = definition.id();
            let deps: Vec<TokenRef> = definition
                .referenced_tokens()
                .into_iter()
                .filter(|target| {
                    registry.scope_of(*target).is_some() || registry.tombstone(*target).is_some()
                })
                .collect();
            for dep in &deps {
                graph.dependents.entry(*dep).or_default().insert(id);
            }
            graph.dependents.entry(id).or_default();
            graph.dependencies.insert(id, deps);
        }
        graph
    }

    /// node(토큰) 수.
    pub fn len(&self) -> usize {
        self.dependencies.len()
    }

    /// node가 없으면 `true`.
    pub fn is_empty(&self) -> bool {
        self.dependencies.is_empty()
    }

    /// `token`이 직접 참조하는 토큰.
    pub fn dependencies(&self, token: TokenRef) -> impl Iterator<Item = TokenRef> + '_ {
        self.dependencies
            .get(&token)
            .into_iter()
            .flat_map(|deps| deps.iter().copied())
    }

    /// `token`을 직접 참조하는 토큰.
    pub fn dependents(&self, token: TokenRef) -> impl Iterator<Item = TokenRef> + '_ {
        self.dependents
            .get(&token)
            .into_iter()
            .flat_map(|deps| deps.iter().copied())
    }

    /// `token`에 (직접·간접) 의존하는 모든 토큰(자기 자신 제외, ID 순).
    pub fn transitive_dependents(&self, token: TokenRef) -> BTreeSet<TokenRef> {
        let mut seen = BTreeSet::new();
        let mut stack: Vec<TokenRef> = self.dependents(token).collect();
        while let Some(next) = stack.pop() {
            if next != token && seen.insert(next) {
                stack.extend(self.dependents(next));
            }
        }
        seen
    }

    /// 첫 cycle을 찾습니다(토큰 ID 순 DFS). 없으면 `None`.
    pub fn find_cycle(&self) -> Option<TokenCycle> {
        let mut marks: BTreeMap<TokenRef, Mark> = BTreeMap::new();
        for &start in self.dependencies.keys() {
            if marks.contains_key(&start) {
                continue;
            }
            marks.insert(start, Mark::Open);
            // (토큰, 다음에 볼 의존 iterator 위치)
            let mut stack: Vec<(TokenRef, usize)> = vec![(start, 0)];
            while let Some(&(token, next)) = stack.last() {
                let child = self
                    .dependencies
                    .get(&token)
                    .and_then(|deps| deps.get(next))
                    .copied();
                let Some(child) = child else {
                    marks.insert(token, Mark::Done);
                    stack.pop();
                    continue;
                };
                if let Some(top) = stack.last_mut() {
                    top.1 += 1;
                }
                match marks.get(&child) {
                    Some(Mark::Done) => {}
                    Some(Mark::Open) => {
                        let from = stack
                            .iter()
                            .position(|(open, _)| *open == child)
                            .unwrap_or(0);
                        let mut path: Vec<TokenRef> =
                            stack[from..].iter().map(|(open, _)| *open).collect();
                        path.push(child);
                        return Some(TokenCycle { path });
                    }
                    None => {
                        marks.insert(child, Mark::Open);
                        stack.push((child, 0));
                    }
                }
            }
        }
        None
    }

    /// `token`의 참조를 `new_references`로 바꾸면 생기는 cycle(있으면). 경로는 `[token, …, token]`입니다.
    ///
    /// 바꾸기 전 graph에서 `new_references`의 각 대상이 `token`에 도달하는지 확인합니다(반복 BFS, ID 순).
    pub fn cycle_if_references(
        &self,
        token: TokenRef,
        new_references: &BTreeSet<TokenRef>,
    ) -> Option<TokenCycle> {
        if new_references.contains(&token) {
            return Some(TokenCycle {
                path: vec![token, token],
            });
        }
        let mut parent: BTreeMap<TokenRef, TokenRef> = BTreeMap::new();
        let mut queue: VecDeque<TokenRef> = VecDeque::new();
        for &start in new_references {
            if parent.insert(start, token).is_none() {
                queue.push_back(start);
            }
        }
        while let Some(current) = queue.pop_front() {
            for dep in self.dependencies(current) {
                if dep == token {
                    // current에서 parent를 따라 새 참조 대상(start)까지 거슬러 올라간 뒤 뒤집습니다.
                    let mut chain = vec![current];
                    let mut at = current;
                    while let Some(&up) = parent.get(&at) {
                        if up == token {
                            break;
                        }
                        chain.push(up);
                        at = up;
                    }
                    chain.reverse();
                    let mut path = Vec::with_capacity(chain.len() + 2);
                    path.push(token);
                    path.extend(chain);
                    path.push(token);
                    return Some(TokenCycle { path });
                }
                if let std::collections::btree_map::Entry::Vacant(slot) = parent.entry(dep) {
                    slot.insert(current);
                    queue.push_back(dep);
                }
            }
        }
        None
    }

    /// 의존 대상이 먼저 오는 위상 순서(같은 단계는 ID 순, Kahn). cycle이 있으면 그 cycle 오류입니다.
    pub fn topological_order(&self) -> Result<Vec<TokenRef>, TokenCycle> {
        let mut remaining: BTreeMap<TokenRef, usize> = self
            .dependencies
            .iter()
            .map(|(token, deps)| (*token, deps.len()))
            .collect();
        let mut ready: BTreeSet<TokenRef> = remaining
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(token, _)| *token)
            .collect();
        let mut order = Vec::with_capacity(remaining.len());
        while let Some(token) = ready.pop_first() {
            order.push(token);
            for dependent in self.dependents(token) {
                if let Some(count) = remaining.get_mut(&dependent) {
                    *count -= 1;
                    if *count == 0 {
                        ready.insert(dependent);
                    }
                }
            }
        }
        if order.len() == self.dependencies.len() {
            Ok(order)
        } else {
            Err(self.find_cycle().unwrap_or(TokenCycle { path: Vec::new() }))
        }
    }
}

/// 저장 전 토큰 검증 오류 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenValidationError {
    /// 토큰 참조 cycle.
    Cycle(TokenCycle),
    /// 없는 토큰·보이지 않는 scope 참조.
    Reference(ReferenceIssue),
    /// default expression type 오류.
    Type {
        /// 토큰.
        token: TokenRef,
        /// 오류.
        error: TokenTypeError,
    },
}

impl fmt::Display for TokenValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenValidationError::Cycle(cycle) => write!(f, "{cycle}"),
            TokenValidationError::Reference(issue) => write!(f, "토큰 참조 문제: {issue:?}"),
            TokenValidationError::Type { token, error } => {
                write!(f, "토큰 {}: {error}", token.id())
            }
        }
    }
}

impl std::error::Error for TokenValidationError {}

/// 저장 전에 registry 전체를 검증합니다: cycle, 참조 문제(orphan 참조 제외), default expression type.
///
/// `others`는 토큰이 아닌 참조(환경 입력 등)의 타입 제공자입니다. 오류가 하나라도 있으면 저장을 거부해야 하며,
/// 오류 순서는 cycle → 참조 문제(정렬) → type 오류(scope·ID 순)로 결정적입니다.
pub fn validate_for_save(
    registry: &TokenRegistry,
    others: &dyn ReferenceTypes,
) -> Result<(), Vec<TokenValidationError>> {
    let mut errors = Vec::new();
    if let Some(cycle) = TokenDependencyGraph::build(registry).find_cycle() {
        errors.push(TokenValidationError::Cycle(cycle));
    }
    errors.extend(
        registry
            .reference_issues()
            .into_iter()
            // orphan 참조는 명시 계획으로 만든 의도된 상태라 저장을 막지 않습니다(§14.4).
            .filter(|issue| !matches!(issue, ReferenceIssue::Orphaned { .. }))
            .map(TokenValidationError::Reference),
    );
    let types = TokenReferenceTypes {
        tokens: registry,
        others,
    };
    for (_, definition) in registry.tokens() {
        if let Err(error) = definition.type_check(&types) {
            errors.push(TokenValidationError::Type {
                token: definition.id(),
                error,
            });
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use stellar_core_types::ids::StableId;
    use stellar_core_types::typed_value::TypedValue;
    use stellar_core_types::value::FiniteF32;
    use stellar_expressions::ast::{ExprNode, ExprNodeId, ExpressionAst, ReferenceTarget};
    use stellar_expressions::ops::BinaryOp;
    use stellar_expressions::types::{ExpressionType, TypeSpec};

    use super::*;
    use crate::registry::TokenScope;
    use crate::token::{TokenDefinition, TokenName, TokenSet, TokenSetId, TokenType};

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn token(n: u64) -> TokenRef {
        TokenRef::new(sid(1_000 + n))
    }

    /// 참조 토큰들의 합(참조가 없으면 1). node ID는 토큰 번호로 구분합니다.
    fn sum_token(n: u64, refs: &[u64]) -> TokenDefinition {
        let base = 1_000_000 * (n + 1);
        let node = |k: u64| ExprNodeId::new(sid(base + k));
        let mut nodes = Vec::new();
        if refs.is_empty() {
            nodes.push(ExprNode::literal(
                node(0),
                TypedValue::Number(FiniteF32::ONE),
            ));
        } else {
            // node(0) = r0 + (r1 + (… + r_last))
            for (i, r) in refs.iter().enumerate() {
                let i = i as u64;
                nodes.push(ExprNode::reference(
                    node(1_000 + i),
                    ReferenceTarget::Token(token(*r)),
                ));
                if (i as usize) + 1 < refs.len() {
                    let rest = if (i as usize) + 2 == refs.len() {
                        node(1_000 + i + 1)
                    } else {
                        node(i + 1)
                    };
                    nodes.push(ExprNode::binary(
                        node(i),
                        BinaryOp::Add,
                        node(1_000 + i),
                        rest,
                    ));
                }
            }
            if refs.len() == 1 {
                // 단일 참조는 참조 node가 root입니다.
                let root = node(1_000);
                return definition(n, ExpressionAst::new(root, nodes).expect("유효"));
            }
        }
        definition(n, ExpressionAst::new(node(0), nodes).expect("유효"))
    }

    fn definition(n: u64, expression: ExpressionAst) -> TokenDefinition {
        TokenDefinition::new(
            token(n),
            TokenName::new(&format!("t{n}")).expect("이름"),
            TokenType::new(ExpressionType::Number),
            expression,
        )
    }

    fn registry(tokens: Vec<TokenDefinition>) -> TokenRegistry {
        let mut set = TokenSet::new(TokenSetId::new(sid(1)));
        for definition in tokens {
            set.insert(definition).expect("추가");
        }
        let mut registry = TokenRegistry::new();
        registry.add_scope(TokenScope::Project, set).expect("scope");
        registry
    }

    fn no_types() -> BTreeMap<ReferenceTarget, TypeSpec> {
        BTreeMap::new()
    }

    #[test]
    fn two_token_cycle_is_rejected_before_save() {
        // A → B → A
        let registry = registry(vec![sum_token(1, &[2]), sum_token(2, &[1])]);
        let graph = TokenDependencyGraph::build(&registry);
        let cycle = graph.find_cycle().expect("cycle");
        assert_eq!(cycle.path, vec![token(1), token(2), token(1)]);
        assert_eq!(graph.topological_order(), Err(cycle.clone()));
        let errors = validate_for_save(&registry, &no_types()).expect_err("저장 거부");
        assert_eq!(errors.first(), Some(&TokenValidationError::Cycle(cycle)));
    }

    #[test]
    fn self_and_indirect_cycles_are_detected_deterministically() {
        let self_cycle = registry(vec![sum_token(1, &[1])]);
        assert_eq!(
            TokenDependencyGraph::build(&self_cycle)
                .find_cycle()
                .map(|c| c.path),
            Some(vec![token(1), token(1)])
        );
        // 1 → 2 → 3 → 1, 4는 사이클 밖에서 2를 참조.
        let tokens = || {
            vec![
                sum_token(4, &[2]),
                sum_token(3, &[1]),
                sum_token(1, &[2]),
                sum_token(2, &[3]),
            ]
        };
        let a = TokenDependencyGraph::build(&registry(tokens()));
        let mut reversed = tokens();
        reversed.reverse();
        let b = TokenDependencyGraph::build(&registry(reversed));
        let path = vec![token(1), token(2), token(3), token(1)];
        assert_eq!(a.find_cycle().map(|c| c.path), Some(path.clone()));
        assert_eq!(b.find_cycle().map(|c| c.path), Some(path), "삽입 순서 무관");
    }

    #[test]
    fn acyclic_graph_orders_dependencies_first() {
        // diamond: 4 → {2, 3}, 2 → 1, 3 → 1; 5 독립.
        let registry = registry(vec![
            sum_token(1, &[]),
            sum_token(2, &[1]),
            sum_token(3, &[1]),
            sum_token(4, &[2, 3]),
            sum_token(5, &[]),
        ]);
        let graph = TokenDependencyGraph::build(&registry);
        assert!(graph.find_cycle().is_none());
        assert_eq!(
            graph.topological_order(),
            Ok(vec![token(1), token(2), token(3), token(4), token(5)])
        );
        assert_eq!(
            graph.transitive_dependents(token(1)),
            BTreeSet::from([token(2), token(3), token(4)])
        );
        assert!(graph.transitive_dependents(token(5)).is_empty());
        assert_eq!(validate_for_save(&registry, &no_types()), Ok(()));
        // 변경 전 검사: 1이 4를 참조하면 1 → 4 → 2 → 1.
        assert_eq!(
            graph
                .cycle_if_references(token(1), &BTreeSet::from([token(4)]))
                .map(|c| c.path),
            Some(vec![token(1), token(4), token(2), token(1)])
        );
        assert_eq!(
            graph
                .cycle_if_references(token(3), &BTreeSet::from([token(3)]))
                .map(|c| c.path),
            Some(vec![token(3), token(3)])
        );
        assert!(graph
            .cycle_if_references(token(5), &BTreeSet::from([token(4)]))
            .is_none());
    }

    #[test]
    fn save_validation_reports_references_and_types() {
        let mut tokens = vec![sum_token(1, &[]), sum_token(2, &[9])];
        // Number 토큰인데 Boolean 결과.
        let node = ExprNodeId::new(sid(77));
        tokens.push(TokenDefinition::new(
            token(3),
            TokenName::new("bool").expect("이름"),
            TokenType::new(ExpressionType::Number),
            ExpressionAst::new(
                node,
                vec![ExprNode::literal(node, TypedValue::Boolean(true))],
            )
            .expect("유효"),
        ));
        let errors = validate_for_save(&registry(tokens), &no_types()).expect_err("거부");
        assert!(matches!(
            errors[0],
            TokenValidationError::Reference(ReferenceIssue::Dangling { .. })
        ));
        assert!(matches!(
            &errors[1],
            TokenValidationError::Type { token: t, .. } if *t == token(2)
        ));
        assert!(matches!(
            &errors[2],
            TokenValidationError::Type { token: t, error: TokenTypeError::ResultMismatch { .. } }
                if *t == token(3)
        ));
        assert_eq!(errors.len(), 3);
    }

    #[test]
    fn long_chain_is_handled_without_recursion() {
        // 20,000단계 chain과 끝을 처음으로 잇는 cycle.
        let n = 20_000u64;
        let mut tokens: Vec<TokenDefinition> = (1..n).map(|i| sum_token(i, &[i + 1])).collect();
        tokens.push(sum_token(n, &[]));
        let graph = TokenDependencyGraph::build(&registry(tokens.clone()));
        assert!(graph.find_cycle().is_none());
        assert_eq!(graph.topological_order().map(|o| o.len()), Ok(n as usize));
        tokens.pop();
        tokens.push(sum_token(n, &[1]));
        let cycle = TokenDependencyGraph::build(&registry(tokens))
            .find_cycle()
            .expect("cycle");
        assert_eq!(cycle.path.len(), n as usize + 1);
    }
}
