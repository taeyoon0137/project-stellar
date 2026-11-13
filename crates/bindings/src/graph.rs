//! 통합 binding dependency graph (VIII.6.2).
//!
//! 토큰·속성·Component Property·환경 입력을 하나의 node 타입 [`DependencyNode`]로 표현합니다(아키텍처 §14.2
//! `ValueNode`). 간선 `a → b`는 "a의 값이 b를 읽는다"(a가 b에 의존)입니다.
//!
//! - 토큰 node는 그 토큰 정의가 값을 만들고(producer = [`Producer::Token`]), default expression의 참조로 간선을 냅니다.
//! - 속성 node는 그 속성을 target으로 하는 binding이 값을 만들고(producer = [`Producer::Binding`]), binding source의
//!   참조로 간선을 냅니다. binding이 없는 속성은 외부에서 값이 주어지는 source node입니다.
//! - Component Property와 환경 입력 node는 binding graph 밖에서 값이 주어지는 source node입니다(producer 없음).
//!
//! node는 [`DependencyNode`] 정렬 순서로 [`NodeIndex`]를 받으므로 index 순서 = node 정렬 순서이고, 간선 목록도 index
//! 오름차순입니다. 같은 입력은 항상 같은 graph가 됩니다. 인접 목록은 index 기반 `Vec`이라 대규모 graph(VIII.9.1)에서도
//! 순회가 가볍습니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_core_types::reference::TokenRef;
use stellar_expressions::ast::{EnvironmentKey, ReferenceTarget};

use crate::binding::{BindingId, BindingSet};
use crate::registry::TokenRegistry;

/// 통합 graph의 node.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DependencyNode {
    /// 디자인 토큰.
    Token(TokenRef),
    /// 속성 값.
    Property(PropertyPath),
    /// Component Property(StableId).
    ComponentProperty(StableId),
    /// 환경 입력.
    Environment(EnvironmentKey),
}

impl DependencyNode {
    /// node 종류.
    pub const fn kind(&self) -> DependencyKind {
        match self {
            DependencyNode::Token(_) => DependencyKind::Token,
            DependencyNode::Property(_) => DependencyKind::Property,
            DependencyNode::ComponentProperty(_) => DependencyKind::ComponentProperty,
            DependencyNode::Environment(_) => DependencyKind::Environment,
        }
    }

    /// 표현식 참조 대상에 대응하는 node.
    pub fn from_target(target: &ReferenceTarget) -> DependencyNode {
        match target {
            ReferenceTarget::Token(token) => DependencyNode::Token(*token),
            ReferenceTarget::Property(path) => DependencyNode::Property(path.clone()),
            ReferenceTarget::ComponentProperty(id) => DependencyNode::ComponentProperty(*id),
            ReferenceTarget::Environment(key) => DependencyNode::Environment(*key),
        }
    }

    /// node에 대응하는 표현식 참조 대상.
    pub fn to_target(&self) -> ReferenceTarget {
        match self {
            DependencyNode::Token(token) => ReferenceTarget::Token(*token),
            DependencyNode::Property(path) => ReferenceTarget::Property(path.clone()),
            DependencyNode::ComponentProperty(id) => ReferenceTarget::ComponentProperty(*id),
            DependencyNode::Environment(key) => ReferenceTarget::Environment(*key),
        }
    }
}

impl fmt::Display for DependencyNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_target())
    }
}

/// node 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DependencyKind {
    /// 토큰.
    Token,
    /// 속성.
    Property,
    /// Component Property.
    ComponentProperty,
    /// 환경 입력.
    Environment,
}

/// node 값을 만드는 정의.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Producer {
    /// 토큰 정의의 default expression.
    Token(TokenRef),
    /// 속성을 target으로 하는 binding의 source.
    Binding(BindingId),
    /// orphan binding(VIII.8.1) target 속성의 선언 기본값(VIII.8.2). 값은 orphan의 ID로 구분합니다.
    OrphanDefault(BindingId),
}

/// graph 안의 node 번호. node 정렬 순서와 같습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeIndex(u32);

impl NodeIndex {
    /// 0부터 시작하는 번호.
    pub const fn get(&self) -> usize {
        self.0 as usize
    }
}

/// graph 구성 입력 하나: `node`의 값을 `producer`가 `dependencies`를 읽어 만듭니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducerRecord {
    /// 값을 받는 node.
    pub node: DependencyNode,
    /// 값을 만드는 정의.
    pub producer: Producer,
    /// 읽는 node.
    pub dependencies: Vec<DependencyNode>,
}

/// graph 구성 오류.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GraphBuildError {
    /// 같은 node에 producer가 둘 이상 있습니다.
    DuplicateProducer(DependencyNode),
    /// node 수가 `u32` 범위를 넘습니다.
    TooManyNodes,
}

impl fmt::Display for GraphBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GraphBuildError::DuplicateProducer(node) => {
                write!(f, "node {node}에 producer가 둘 이상 있습니다")
            }
            GraphBuildError::TooManyNodes => f.write_str("node 수가 u32 범위를 넘습니다"),
        }
    }
}

impl std::error::Error for GraphBuildError {}

/// 통합 dependency graph (VIII.6.2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DependencyGraph {
    nodes: Vec<DependencyNode>,
    index: BTreeMap<DependencyNode, NodeIndex>,
    dependencies: Vec<Vec<NodeIndex>>,
    dependents: Vec<Vec<NodeIndex>>,
    producers: Vec<Option<Producer>>,
    edge_count: usize,
}

impl DependencyGraph {
    /// 토큰 registry와 binding 모음으로 graph를 만듭니다.
    pub fn build(tokens: &TokenRegistry, bindings: &BindingSet) -> DependencyGraph {
        let mut records = Vec::with_capacity(tokens.len() + bindings.len());
        for (_, definition) in tokens.tokens() {
            records.push(ProducerRecord {
                node: DependencyNode::Token(definition.id()),
                producer: Producer::Token(definition.id()),
                dependencies: definition
                    .references()
                    .iter()
                    .map(DependencyNode::from_target)
                    .collect(),
            });
        }
        for binding in bindings.iter() {
            records.push(ProducerRecord {
                node: DependencyNode::Property(binding.target().clone()),
                producer: Producer::Binding(binding.id()),
                dependencies: binding
                    .references()
                    .iter()
                    .map(DependencyNode::from_target)
                    .collect(),
            });
        }
        // 토큰 ID는 registry에서 유일하고 binding target은 BindingSet에서 유일하므로 producer 중복은 없습니다.
        DependencyGraph::from_records(records).unwrap_or_default()
    }

    /// 구성 입력으로 graph를 만듭니다. 한 node에 producer가 둘이면 거부합니다.
    pub fn from_records(records: Vec<ProducerRecord>) -> Result<DependencyGraph, GraphBuildError> {
        let mut all: BTreeSet<&DependencyNode> = BTreeSet::new();
        for record in &records {
            all.insert(&record.node);
            all.extend(record.dependencies.iter());
        }
        if u32::try_from(all.len()).is_err() {
            return Err(GraphBuildError::TooManyNodes);
        }
        let nodes: Vec<DependencyNode> = all.into_iter().cloned().collect();
        let index: BTreeMap<DependencyNode, NodeIndex> = nodes
            .iter()
            .enumerate()
            .map(|(i, node)| (node.clone(), NodeIndex(i as u32)))
            .collect();
        let count = nodes.len();
        let mut dependencies: Vec<Vec<NodeIndex>> = vec![Vec::new(); count];
        let mut producers: Vec<Option<Producer>> = vec![None; count];
        for record in &records {
            let at = index[&record.node].get();
            if producers[at].is_some() {
                return Err(GraphBuildError::DuplicateProducer(record.node.clone()));
            }
            producers[at] = Some(record.producer);
            let mut deps: Vec<NodeIndex> =
                record.dependencies.iter().map(|dep| index[dep]).collect();
            deps.sort_unstable();
            deps.dedup();
            dependencies[at] = deps;
        }
        let mut dependents: Vec<Vec<NodeIndex>> = vec![Vec::new(); count];
        let mut edge_count = 0;
        for (from, deps) in dependencies.iter().enumerate() {
            edge_count += deps.len();
            for dep in deps {
                dependents[dep.get()].push(NodeIndex(from as u32));
            }
        }
        // from을 오름차순으로 돌았으므로 dependents도 오름차순입니다.
        Ok(DependencyGraph {
            nodes,
            index,
            dependencies,
            dependents,
            producers,
            edge_count,
        })
    }

    /// node 수.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// 간선 수(중복 제거 후).
    pub fn edge_count(&self) -> usize {
        self.edge_count
    }

    /// node의 번호.
    pub fn index_of(&self, node: &DependencyNode) -> Option<NodeIndex> {
        self.index.get(node).copied()
    }

    /// 번호의 node.
    pub fn node(&self, index: NodeIndex) -> &DependencyNode {
        &self.nodes[index.get()]
    }

    /// 모든 node(정렬 순서 = 번호 순).
    pub fn nodes(&self) -> &[DependencyNode] {
        &self.nodes
    }

    /// `index`가 읽는 node(번호 오름차순).
    pub fn dependencies(&self, index: NodeIndex) -> &[NodeIndex] {
        &self.dependencies[index.get()]
    }

    /// `index`를 읽는 node(번호 오름차순).
    pub fn dependents(&self, index: NodeIndex) -> &[NodeIndex] {
        &self.dependents[index.get()]
    }

    /// `index`의 값을 만드는 정의. source node는 `None`.
    pub fn producer(&self, index: NodeIndex) -> Option<Producer> {
        self.producers[index.get()]
    }

    /// 모든 번호(오름차순).
    pub fn indices(&self) -> impl Iterator<Item = NodeIndex> {
        (0..self.nodes.len() as u32).map(NodeIndex)
    }
}

/// 통합 graph의 cycle(VIII.6.3). 경로는 시작 node로 닫힙니다(`[a, …, a]`, 자기 참조는 `[a, a]`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BindingCycle {
    /// cycle 경로.
    pub path: Vec<DependencyNode>,
}

impl fmt::Display for BindingCycle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("binding cycle: ")?;
        for (index, node) in self.path.iter().enumerate() {
            if index > 0 {
                f.write_str(" → ")?;
            }
            write!(f, "{node}")?;
        }
        Ok(())
    }
}

impl std::error::Error for BindingCycle {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    Unvisited,
    Open,
    Done,
}

impl DependencyGraph {
    /// 첫 cycle을 찾습니다(VIII.6.3). 토큰·속성·Component Property·환경 입력이 섞인 cycle도 같은 방식으로 찾습니다.
    ///
    /// 반복(비재귀) 3색 DFS이며 node·간선을 번호(= node 정렬) 순으로 방문해 같은 graph는 같은 경로를 보고합니다.
    pub fn find_cycle(&self) -> Option<BindingCycle> {
        let mut marks = vec![Mark::Unvisited; self.nodes.len()];
        for start in 0..self.nodes.len() {
            if marks[start] != Mark::Unvisited {
                continue;
            }
            marks[start] = Mark::Open;
            // (node, 다음에 볼 dependency 위치)
            let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
            while let Some(&(at, next)) = stack.last() {
                let Some(child) = self.dependencies[at].get(next).map(NodeIndex::get) else {
                    marks[at] = Mark::Done;
                    stack.pop();
                    continue;
                };
                if let Some(top) = stack.last_mut() {
                    top.1 += 1;
                }
                match marks[child] {
                    Mark::Done => {}
                    Mark::Open => {
                        let from = stack
                            .iter()
                            .position(|(open, _)| *open == child)
                            .unwrap_or(0);
                        let mut path: Vec<DependencyNode> = stack[from..]
                            .iter()
                            .map(|(open, _)| self.nodes[*open].clone())
                            .collect();
                        path.push(self.nodes[child].clone());
                        return Some(BindingCycle { path });
                    }
                    Mark::Unvisited => {
                        marks[child] = Mark::Open;
                        stack.push((child, 0));
                    }
                }
            }
        }
        None
    }

    /// cycle이 없으면 `Ok`, 있으면 첫 cycle 오류입니다(저장·적용 전 거부용).
    pub fn ensure_acyclic(&self) -> Result<(), BindingCycle> {
        match self.find_cycle() {
            Some(cycle) => Err(cycle),
            None => Ok(()),
        }
    }

    /// `node`가 `new_dependencies`를 읽도록 바꾸면 생기는 cycle(있으면). 경로는 `[node, …, node]`입니다.
    ///
    /// 바꾸기 전 graph에서 새 dependency가 `node`에 도달하는지 반복 BFS(번호 순)로 확인합니다. graph에 없는 node는
    /// 아직 아무것도 읽지 않으므로 cycle에 들 수 없습니다.
    pub fn cycle_if_dependencies(
        &self,
        node: &DependencyNode,
        new_dependencies: &BTreeSet<DependencyNode>,
    ) -> Option<BindingCycle> {
        if new_dependencies.contains(node) {
            return Some(BindingCycle {
                path: vec![node.clone(), node.clone()],
            });
        }
        let target = self.index_of(node)?;
        let mut parent: BTreeMap<NodeIndex, Option<NodeIndex>> = BTreeMap::new();
        let mut queue: std::collections::VecDeque<NodeIndex> = std::collections::VecDeque::new();
        for dep in new_dependencies {
            if let Some(index) = self.index_of(dep) {
                if parent.insert(index, None).is_none() {
                    queue.push_back(index);
                }
            }
        }
        while let Some(current) = queue.pop_front() {
            for &dep in self.dependencies(current) {
                if dep == target {
                    let mut chain = vec![current];
                    let mut at = current;
                    while let Some(Some(up)) = parent.get(&at) {
                        chain.push(*up);
                        at = *up;
                    }
                    chain.reverse();
                    let mut path = Vec::with_capacity(chain.len() + 2);
                    path.push(node.clone());
                    path.extend(chain.into_iter().map(|i| self.node(i).clone()));
                    path.push(node.clone());
                    return Some(BindingCycle { path });
                }
                if let std::collections::btree_map::Entry::Vacant(slot) = parent.entry(dep) {
                    slot.insert(Some(current));
                    queue.push_back(dep);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use stellar_core_types::property::PropertyKey;
    use stellar_core_types::reference::NodeRef;
    use stellar_core_types::typed_value::TypedValue;
    use stellar_core_types::value::FiniteF32;
    use stellar_expressions::ast::{ExprNode, ExprNodeId, ExpressionAst};
    use stellar_expressions::ops::BinaryOp;
    use stellar_expressions::types::{ExpressionType, TypeSpec};

    use super::*;
    use crate::binding::{ScopeOwner, ValueBinding};
    use crate::registry::TokenScope;
    use crate::token::{TokenDefinition, TokenName, TokenSet, TokenSetId, TokenType};

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn token(n: u64) -> TokenRef {
        TokenRef::new(sid(1_000 + n))
    }

    fn prop(n: u64) -> PropertyPath {
        PropertyPath::new(sid(2_000 + n)).push(PropertyKey(1))
    }

    fn env(n: u32) -> EnvironmentKey {
        EnvironmentKey::new(n)
    }

    fn component_property(n: u64) -> StableId {
        sid(3_000 + n)
    }

    /// 참조 대상들의 합(대상 1개면 그 참조, 0개면 1).
    fn sum(base: u64, targets: &[ReferenceTarget]) -> ExpressionAst {
        let node = |k: u64| ExprNodeId::new(sid(base * 100 + k));
        let mut nodes: Vec<ExprNode> = targets
            .iter()
            .enumerate()
            .map(|(i, t)| ExprNode::reference(node(50 + i as u64), t.clone()))
            .collect();
        match targets.len() {
            0 => {
                nodes.push(ExprNode::literal(
                    node(0),
                    TypedValue::Number(FiniteF32::ONE),
                ));
                ExpressionAst::new(node(0), nodes).expect("유효")
            }
            1 => ExpressionAst::new(node(50), nodes).expect("유효"),
            n => {
                // node(i) = ref(i) + node(i+1), 마지막 덧셈은 ref(n-1).
                for i in 0..(n - 1) as u64 {
                    let rest = if i as usize == n - 2 {
                        node(50 + i + 1)
                    } else {
                        node(i + 1)
                    };
                    nodes.push(ExprNode::binary(node(i), BinaryOp::Add, node(50 + i), rest));
                }
                ExpressionAst::new(node(0), nodes).expect("유효")
            }
        }
    }

    fn define(n: u64, targets: &[ReferenceTarget]) -> TokenDefinition {
        TokenDefinition::new(
            token(n),
            TokenName::new(&format!("t{n}")).expect("이름"),
            TokenType::new(ExpressionType::Number),
            sum(n, targets),
        )
    }

    fn bind(n: u64, target: PropertyPath, refs: &[ReferenceTarget]) -> ValueBinding {
        ValueBinding::new(
            BindingId::new(sid(4_000 + n)),
            target,
            sum(100 + n, refs),
            TypeSpec::required(ExpressionType::Number),
            ScopeOwner::Node(NodeRef::new(sid(5_000 + n))),
        )
    }

    /// 종류별 dependency fixture.
    ///
    /// - token1: 상수, token2 = token1 + env(7)
    /// - prop(1) ← binding1 = token2
    /// - prop(2) ← binding2 = prop(1) + prop(9)(binding 없는 source 속성)
    /// - prop(3) ← binding3 = component property 1 + env(7)
    fn fixture() -> (TokenRegistry, BindingSet) {
        let mut set = TokenSet::new(TokenSetId::new(sid(1)));
        set.insert(define(1, &[])).expect("t1");
        set.insert(define(
            2,
            &[
                ReferenceTarget::Token(token(1)),
                ReferenceTarget::Environment(env(7)),
            ],
        ))
        .expect("t2");
        let mut registry = TokenRegistry::new();
        registry.add_scope(TokenScope::Project, set).expect("scope");
        let mut bindings = BindingSet::new();
        bindings
            .insert(bind(1, prop(1), &[ReferenceTarget::Token(token(2))]))
            .expect("b1");
        bindings
            .insert(bind(
                2,
                prop(2),
                &[
                    ReferenceTarget::Property(prop(1)),
                    ReferenceTarget::Property(prop(9)),
                ],
            ))
            .expect("b2");
        bindings
            .insert(bind(
                3,
                prop(3),
                &[
                    ReferenceTarget::ComponentProperty(component_property(1)),
                    ReferenceTarget::Environment(env(7)),
                ],
            ))
            .expect("b3");
        (registry, bindings)
    }

    fn deps_of(graph: &DependencyGraph, node: &DependencyNode) -> Vec<DependencyNode> {
        let index = graph.index_of(node).expect("node");
        graph
            .dependencies(index)
            .iter()
            .map(|i| graph.node(*i).clone())
            .collect()
    }

    fn dependents_of(graph: &DependencyGraph, node: &DependencyNode) -> Vec<DependencyNode> {
        let index = graph.index_of(node).expect("node");
        graph
            .dependents(index)
            .iter()
            .map(|i| graph.node(*i).clone())
            .collect()
    }

    #[test]
    fn every_dependency_kind_is_a_common_node() {
        let (registry, bindings) = fixture();
        let graph = DependencyGraph::build(&registry, &bindings);
        use DependencyNode as N;
        // 토큰 → 토큰·환경.
        assert_eq!(
            deps_of(&graph, &N::Token(token(2))),
            vec![N::Token(token(1)), N::Environment(env(7))]
        );
        // 속성 → 토큰.
        assert_eq!(
            deps_of(&graph, &N::Property(prop(1))),
            vec![N::Token(token(2))]
        );
        // 속성 → 속성(binding 있는 것·없는 source).
        assert_eq!(
            deps_of(&graph, &N::Property(prop(2))),
            vec![N::Property(prop(1)), N::Property(prop(9))]
        );
        // 속성 → Component Property·환경.
        assert_eq!(
            deps_of(&graph, &N::Property(prop(3))),
            vec![
                N::ComponentProperty(component_property(1)),
                N::Environment(env(7))
            ]
        );
        // 역방향: 환경 입력을 읽는 node.
        assert_eq!(
            dependents_of(&graph, &N::Environment(env(7))),
            vec![N::Token(token(2)), N::Property(prop(3))]
        );
        // producer: 토큰·binding, source node는 없음.
        let producer = |n: &N| graph.producer(graph.index_of(n).expect("node"));
        assert_eq!(
            producer(&N::Token(token(1))),
            Some(Producer::Token(token(1)))
        );
        assert_eq!(
            producer(&N::Property(prop(2))),
            Some(Producer::Binding(BindingId::new(sid(4_002))))
        );
        assert_eq!(producer(&N::Property(prop(9))), None);
        assert_eq!(producer(&N::Environment(env(7))), None);
        assert_eq!(producer(&N::ComponentProperty(component_property(1))), None);
        // node 8개(토큰 2, 속성 4, Component Property 1, 환경 1), 간선 7개.
        let kinds: Vec<DependencyKind> = graph.nodes().iter().map(DependencyNode::kind).collect();
        assert_eq!(
            kinds,
            vec![
                DependencyKind::Token,
                DependencyKind::Token,
                DependencyKind::Property,
                DependencyKind::Property,
                DependencyKind::Property,
                DependencyKind::Property,
                DependencyKind::ComponentProperty,
                DependencyKind::Environment,
            ]
        );
        assert_eq!(graph.edge_count(), 7);
    }

    #[test]
    fn graph_is_deterministic_and_rejects_duplicate_producers() {
        let (registry, bindings) = fixture();
        let a = DependencyGraph::build(&registry, &bindings);
        let mut records: Vec<ProducerRecord> = a
            .indices()
            .filter_map(|i| {
                a.producer(i).map(|producer| ProducerRecord {
                    node: a.node(i).clone(),
                    producer,
                    dependencies: a
                        .dependencies(i)
                        .iter()
                        .rev()
                        .map(|d| a.node(*d).clone())
                        .collect(),
                })
            })
            .collect();
        records.reverse();
        let b = DependencyGraph::from_records(records.clone()).expect("구성");
        assert_eq!(a, b, "입력 순서와 무관");
        for i in a.indices() {
            assert!(a.dependencies(i).windows(2).all(|w| w[0] < w[1]));
            assert!(a.dependents(i).windows(2).all(|w| w[0] < w[1]));
        }
        let duplicate = records[0].clone();
        records.push(ProducerRecord {
            producer: Producer::Token(token(99)),
            ..duplicate.clone()
        });
        assert_eq!(
            DependencyGraph::from_records(records),
            Err(GraphBuildError::DuplicateProducer(duplicate.node))
        );
        // 참조 대상 ↔ node 변환은 왕복합니다.
        for node in a.nodes() {
            assert_eq!(&DependencyNode::from_target(&node.to_target()), node);
        }
    }

    /// 순수 구조 fixture: (node, producer 번호, 읽는 node) 목록으로 graph를 만듭니다.
    fn graph_of(edges: &[(DependencyNode, Vec<DependencyNode>)]) -> DependencyGraph {
        let records = edges
            .iter()
            .enumerate()
            .map(|(i, (node, deps))| ProducerRecord {
                node: node.clone(),
                producer: match node {
                    DependencyNode::Token(t) => Producer::Token(*t),
                    _ => Producer::Binding(BindingId::new(sid(9_000 + i as u64))),
                },
                dependencies: deps.clone(),
            })
            .collect();
        DependencyGraph::from_records(records).expect("구성")
    }

    #[test]
    fn property_and_token_property_cycles_are_detected() {
        use DependencyNode as N;
        let a = N::Property(prop(1));
        let b = N::Property(prop(2));
        let t = N::Token(token(1));
        // Property A ↔ B.
        let graph = graph_of(&[(a.clone(), vec![b.clone()]), (b.clone(), vec![a.clone()])]);
        assert_eq!(
            graph.ensure_acyclic(),
            Err(BindingCycle {
                path: vec![a.clone(), b.clone(), a.clone()]
            })
        );
        // token ↔ property: 토큰 T가 속성 A를 읽고, A의 binding이 T를 읽습니다.
        let graph = graph_of(&[(t.clone(), vec![a.clone()]), (a.clone(), vec![t.clone()])]);
        assert_eq!(
            graph.find_cycle().map(|c| c.path),
            Some(vec![t.clone(), a.clone(), t.clone()])
        );
        // 자기 참조 속성.
        let graph = graph_of(&[(a.clone(), vec![a.clone()])]);
        assert_eq!(
            graph.find_cycle().map(|c| c.path),
            Some(vec![a.clone(), a.clone()])
        );
    }

    #[test]
    fn mixed_cycle_fixture_fails_and_acyclic_passes() {
        use DependencyNode as N;
        let p1 = N::Property(prop(1));
        let p2 = N::Property(prop(2));
        let t1 = N::Token(token(1));
        let t2 = N::Token(token(2));
        let env7 = N::Environment(env(7));
        let cp = N::ComponentProperty(component_property(1));
        // p1 → t1 → t2 → p2 → p1 (환경·Component Property는 cycle 밖의 source).
        let edges: Vec<(DependencyNode, Vec<DependencyNode>)> = vec![
            (p1.clone(), vec![t1.clone(), cp.clone()]),
            (t1.clone(), vec![t2.clone(), env7.clone()]),
            (t2.clone(), vec![p2.clone()]),
            (p2.clone(), vec![p1.clone(), env7.clone()]),
        ];
        let graph = graph_of(&edges);
        assert_eq!(
            graph.find_cycle().map(|c| c.path),
            Some(vec![
                t1.clone(),
                t2.clone(),
                p2.clone(),
                p1.clone(),
                t1.clone()
            ])
        );
        // 실제 registry·binding fixture(VIII.6.2)는 비순환입니다.
        let (registry, bindings) = fixture();
        assert_eq!(
            DependencyGraph::build(&registry, &bindings).ensure_acyclic(),
            Ok(())
        );
        // 변경 전 예측: prop(1)의 binding이 prop(2)를 읽게 하면 prop1 → prop2 → prop1.
        let graph = DependencyGraph::build(&registry, &bindings);
        assert_eq!(
            graph
                .cycle_if_dependencies(&p1, &BTreeSet::from([p2.clone()]))
                .map(|c| c.path),
            Some(vec![p1.clone(), p2.clone(), p1.clone()])
        );
        // token1이 prop(3)를 읽어도 prop(3)는 token1에 도달하지 않습니다.
        assert!(graph
            .cycle_if_dependencies(&t1, &BTreeSet::from([N::Property(prop(3))]))
            .is_none());
        // token2가 prop(1)을 읽으면 token2 → prop1 → token2.
        assert_eq!(
            graph
                .cycle_if_dependencies(&t2, &BTreeSet::from([p1.clone()]))
                .map(|c| c.path),
            Some(vec![t2.clone(), p1.clone(), t2])
        );
        assert!(graph
            .cycle_if_dependencies(&p1, &BTreeSet::from([p1.clone()]))
            .is_some());
    }

    #[test]
    fn long_mixed_chain_is_checked_without_recursion() {
        use DependencyNode as N;
        let n = 50_000u64;
        let node = |i: u64| {
            if i.is_multiple_of(2) {
                N::Token(token(i))
            } else {
                N::Property(prop(i))
            }
        };
        let mut edges: Vec<(DependencyNode, Vec<DependencyNode>)> =
            (0..n).map(|i| (node(i), vec![node(i + 1)])).collect();
        edges.push((node(n), vec![]));
        let records: Vec<ProducerRecord> = edges
            .iter()
            .enumerate()
            .map(|(i, (node, deps))| ProducerRecord {
                node: node.clone(),
                producer: Producer::Binding(BindingId::new(sid(10_000_000 + i as u64))),
                dependencies: deps.clone(),
            })
            .collect();
        let graph = DependencyGraph::from_records(records.clone()).expect("구성");
        assert!(graph.find_cycle().is_none());
        let mut cyclic = records;
        if let Some(last) = cyclic.last_mut() {
            last.dependencies = vec![node(0)];
        }
        let cycle = DependencyGraph::from_records(cyclic)
            .expect("구성")
            .find_cycle()
            .expect("cycle");
        assert_eq!(cycle.path.len(), n as usize + 2);
    }
}
