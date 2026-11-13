//! 영향받는 binding만 위상 순서로 재평가하는 scheduler (VIII.6.4).
//!
//! [`BindingEvaluator`]는 통합 graph(VIII.6.2)의 node마다 현재 값을 보관합니다.
//!
//! - **source node**(binding 없는 속성·Component Property·환경 입력·정의 없는 토큰)의 값은 외부
//!   ([`SourceInputs`], [`BindingEvaluator::update_sources`])에서 주어집니다.
//! - **producer node**(토큰·binding target 속성)의 값은 그 표현식을 bytecode(VIII.4)로 평가해 얻습니다.
//!
//! 생성 시 graph가 비순환인지 확인하고(VIII.6.3) 전체 위상 순서(같은 단계는 node 번호 순, Kahn)를 한 번 계산합니다.
//! 변경이 들어오면
//!
//! 1. 바뀐 node에서 역방향 간선(dependents)을 따라 영향받는 producer 집합을 구하고(**affected**, 순회한 node 수 =
//!    **visited**),
//! 2. affected를 위상 순위 순으로 정렬해, 읽는 node 중 값이 실제로 바뀐 것이 있는 producer만 평가합니다
//!    (**evaluated**; 결과가 같으면 그 위로 전파하지 않는 early cutoff).
//!
//! 무관한 subtree의 producer는 affected에 들지 않으므로 평가하지 않으며, 평가 순서는 위상 순위로 고정되어
//! 결정적입니다. 모든 순회는 명시적 queue로 재귀 없이 진행합니다.

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::fmt;

use stellar_core_types::typed_value::TypedValue;
use stellar_expressions::ast::{ExpressionAst, ReferenceTarget};
use stellar_expressions::bytecode::{CompileError, Program};
use stellar_expressions::eval::{EvalError, EvalErrorKind, ReferenceValues};
use stellar_expressions::typecheck::ReferenceTypes;
use stellar_expressions::types::TypeSpec;
use stellar_expressions::vm::EvalLimits;

use crate::binding::BindingSet;
use crate::graph::{
    BindingCycle, DependencyGraph, DependencyNode, GraphBuildError, NodeIndex, Producer,
    ProducerRecord,
};
use crate::registry::TokenRegistry;
use crate::token::TokenTypes;

/// source node의 타입과 값 제공자.
pub trait SourceInputs {
    /// source node 타입. 모르면 `None`.
    fn source_type(&self, node: &DependencyNode) -> Option<TypeSpec>;
    /// source node 값. `None` = 값이 주어지지 않음, `Some(None)` = 값 없음(null).
    fn source_value(&self, node: &DependencyNode) -> Option<Option<TypedValue>>;
}

/// 고정 source 입력(node → 타입·값). 테스트·도구용 간단한 [`SourceInputs`]입니다.
pub type StaticSources = BTreeMap<DependencyNode, (TypeSpec, Option<TypedValue>)>;

impl SourceInputs for StaticSources {
    fn source_type(&self, node: &DependencyNode) -> Option<TypeSpec> {
        self.get(node).map(|(spec, _)| *spec)
    }

    fn source_value(&self, node: &DependencyNode) -> Option<Option<TypedValue>> {
        self.get(node).map(|(_, value)| value.clone())
    }
}

/// node 평가 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeEvalError {
    /// source node에 값이 주어지지 않았습니다.
    MissingSource,
    /// producer 표현식 컴파일 실패(type 오류).
    Compile(CompileError),
    /// 평가 실패.
    Eval(EvalError),
    /// 실제로 읽은 node의 값이 오류입니다.
    Dependency(DependencyNode),
}

impl fmt::Display for NodeEvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodeEvalError::MissingSource => f.write_str("source 값이 주어지지 않았습니다"),
            NodeEvalError::Compile(error) => write!(f, "{error}"),
            NodeEvalError::Eval(error) => write!(f, "{error}"),
            NodeEvalError::Dependency(node) => write!(f, "읽은 node {node}의 값이 오류입니다"),
        }
    }
}

impl std::error::Error for NodeEvalError {}

/// node 값.
pub type NodeValue = Result<Option<TypedValue>, NodeEvalError>;

/// scheduler 호출 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchedulerError {
    /// graph에 cycle이 있습니다.
    Cycle(BindingCycle),
    /// graph 구성 실패(한 node에 producer 둘 등).
    Graph(GraphBuildError),
    /// graph에 없는 node.
    UnknownNode(DependencyNode),
    /// source 값 변경 대상이 producer node입니다(표현식으로 값이 정해짐).
    NotASource(DependencyNode),
    /// 표현식 교체 대상이 producer node가 아닙니다.
    NotAProducer(DependencyNode),
    /// 새 표현식의 참조가 기존 dependency와 달라 graph 재구성이 필요합니다.
    StructureChanged(DependencyNode),
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SchedulerError::Cycle(cycle) => write!(f, "{cycle}"),
            SchedulerError::Graph(error) => write!(f, "{error}"),
            SchedulerError::UnknownNode(node) => write!(f, "graph에 없는 node: {node}"),
            SchedulerError::NotASource(node) => write!(f, "{node}는 source node가 아닙니다"),
            SchedulerError::NotAProducer(node) => write!(f, "{node}는 producer node가 아닙니다"),
            SchedulerError::StructureChanged(node) => {
                write!(
                    f,
                    "{node}의 dependency가 바뀌어 graph를 다시 만들어야 합니다"
                )
            }
        }
    }
}

impl std::error::Error for SchedulerError {}

/// 한 번의 재평가 기록.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UpdateReport {
    /// 값이 실제로 바뀐 입력 node 수(source 값 변경 또는 표현식 교체).
    pub changed_inputs: usize,
    /// 영향받는 producer 수(변경 node에서 역방향으로 도달 가능한 producer, 교체한 producer 포함).
    pub affected: usize,
    /// 영향 범위를 구하며 순회한 node 수(변경 node 포함).
    pub visited: usize,
    /// 실제로 표현식을 평가한 producer 수.
    pub evaluated: usize,
    /// 평가한 producer 순서(위상 순위 순).
    pub order: Vec<NodeIndex>,
}

/// 통합 binding graph 값 저장소와 증분 scheduler (VIII.6.4).
#[derive(Debug, Clone)]
pub struct BindingEvaluator {
    graph: DependencyGraph,
    types: Vec<Option<TypeSpec>>,
    expressions: Vec<Option<ExpressionAst>>,
    programs: Vec<Option<Result<Program, CompileError>>>,
    values: Vec<NodeValue>,
    rank: Vec<u32>,
    topological: Vec<NodeIndex>,
    limits: EvalLimits,
}

/// graph node 값을 표현식 참조 값으로 보는 view.
struct ValueView<'a> {
    graph: &'a DependencyGraph,
    values: &'a [NodeValue],
}

impl ReferenceValues for ValueView<'_> {
    fn reference_value(&self, target: &ReferenceTarget) -> Option<Option<TypedValue>> {
        let index = self.graph.index_of(&DependencyNode::from_target(target))?;
        self.values[index.get()].as_ref().ok().cloned()
    }
}

/// graph node 타입을 표현식 참조 타입으로 보는 view.
struct TypeView<'a> {
    graph: &'a DependencyGraph,
    types: &'a [Option<TypeSpec>],
}

impl ReferenceTypes for TypeView<'_> {
    fn reference_type(&self, target: &ReferenceTarget) -> Option<TypeSpec> {
        let index = self.graph.index_of(&DependencyNode::from_target(target))?;
        self.types[index.get()]
    }
}

/// producer node 하나의 명세: `node`의 값을 `producer`의 `expression`이 만들고, 결과 선언 타입은 `value_type`입니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducerSpec {
    /// 값을 받는 node.
    pub node: DependencyNode,
    /// 값을 만드는 정의.
    pub producer: Producer,
    /// node 선언 타입(다른 표현식이 이 node를 읽을 때의 타입). 모르면 `None`.
    pub value_type: Option<TypeSpec>,
    /// 값을 계산하는 표현식.
    pub expression: ExpressionAst,
}

/// registry의 모든 토큰 정의 명세(scope·ID 순).
pub fn token_specs(tokens: &TokenRegistry) -> Vec<ProducerSpec> {
    tokens
        .tokens()
        .map(|(_, definition)| ProducerSpec {
            node: DependencyNode::Token(definition.id()),
            producer: Producer::Token(definition.id()),
            value_type: Some(definition.token_type().type_spec()),
            expression: definition.default_expression().clone(),
        })
        .collect()
}

/// 정의 없는(orphan 기록만 남은) 토큰 node는 마지막 타입으로 보고, 그 밖은 `inner`에 맡기는 source 제공자.
pub(crate) struct TokenAwareSources<'a> {
    pub(crate) tokens: &'a TokenRegistry,
    pub(crate) inner: &'a dyn SourceInputs,
}

impl SourceInputs for TokenAwareSources<'_> {
    fn source_type(&self, node: &DependencyNode) -> Option<TypeSpec> {
        match node {
            DependencyNode::Token(token) => self
                .tokens
                .token_type_of(*token)
                .map(|t| t.type_spec())
                .or_else(|| self.inner.source_type(node)),
            other => self.inner.source_type(other),
        }
    }

    fn source_value(&self, node: &DependencyNode) -> Option<Option<TypedValue>> {
        self.inner.source_value(node)
    }
}

impl BindingEvaluator {
    /// graph를 만들고 비순환을 확인한 뒤 모든 node를 위상 순서로 평가합니다(기본 평가 한도).
    pub fn new(
        tokens: &TokenRegistry,
        bindings: &BindingSet,
        sources: &dyn SourceInputs,
    ) -> Result<BindingEvaluator, SchedulerError> {
        BindingEvaluator::with_limits(tokens, bindings, sources, EvalLimits::DEFAULT)
    }

    /// 평가 한도를 지정해 만듭니다.
    pub fn with_limits(
        tokens: &TokenRegistry,
        bindings: &BindingSet,
        sources: &dyn SourceInputs,
        limits: EvalLimits,
    ) -> Result<BindingEvaluator, SchedulerError> {
        let mut specs = token_specs(tokens);
        specs.extend(bindings.iter().map(|binding| ProducerSpec {
            node: DependencyNode::Property(binding.target().clone()),
            producer: Producer::Binding(binding.id()),
            value_type: Some(binding.expected_type()),
            expression: binding.source().clone(),
        }));
        let sources = TokenAwareSources {
            tokens,
            inner: sources,
        };
        BindingEvaluator::from_specs(specs, &sources, limits)
    }

    /// producer 명세로 만듭니다. graph 구성(producer 중복 거부) → 비순환 확인 → 위상 순위 → 컴파일 → 전체 평가.
    pub fn from_specs(
        specs: Vec<ProducerSpec>,
        sources: &dyn SourceInputs,
        limits: EvalLimits,
    ) -> Result<BindingEvaluator, SchedulerError> {
        let records: Vec<ProducerRecord> = specs
            .iter()
            .map(|spec| ProducerRecord {
                node: spec.node.clone(),
                producer: spec.producer,
                dependencies: spec
                    .expression
                    .referenced_targets()
                    .into_iter()
                    .map(DependencyNode::from_target)
                    .collect(),
            })
            .collect();
        let graph = DependencyGraph::from_records(records).map_err(SchedulerError::Graph)?;
        graph.ensure_acyclic().map_err(SchedulerError::Cycle)?;
        let count = graph.node_count();
        let mut types = vec![None; count];
        let mut expressions = vec![None; count];
        let mut values: Vec<NodeValue> = vec![Err(NodeEvalError::MissingSource); count];
        for spec in specs {
            if let Some(index) = graph.index_of(&spec.node) {
                types[index.get()] = spec.value_type;
                expressions[index.get()] = Some(spec.expression);
            }
        }
        for index in graph.indices() {
            if graph.producer(index).is_none() {
                let node = graph.node(index);
                types[index.get()] = sources.source_type(node);
                if let Some(value) = sources.source_value(node) {
                    values[index.get()] = Ok(value);
                }
            }
        }
        let (rank, topological) = topological_ranks(&graph);
        let mut evaluator = BindingEvaluator {
            programs: vec![None; count],
            graph,
            types,
            expressions,
            values,
            rank,
            topological,
            limits,
        };
        for index in evaluator.graph.indices() {
            evaluator.compile(index);
        }
        evaluator.evaluate_all();
        Ok(evaluator)
    }

    fn compile(&mut self, index: NodeIndex) {
        let at = index.get();
        let program = self.expressions[at].as_ref().map(|expression| {
            let types = TypeView {
                graph: &self.graph,
                types: &self.types,
            };
            Program::compile(expression, &types)
        });
        self.programs[at] = program;
    }

    /// graph.
    pub fn graph(&self) -> &DependencyGraph {
        &self.graph
    }

    /// 평가 한도.
    pub const fn limits(&self) -> EvalLimits {
        self.limits
    }

    /// node 값.
    pub fn value(&self, node: &DependencyNode) -> Option<&NodeValue> {
        self.graph
            .index_of(node)
            .map(|index| &self.values[index.get()])
    }

    /// 번호의 node 값.
    pub fn value_at(&self, index: NodeIndex) -> &NodeValue {
        &self.values[index.get()]
    }

    /// producer node의 표현식.
    pub fn expression_at(&self, index: NodeIndex) -> Option<&ExpressionAst> {
        self.expressions[index.get()].as_ref()
    }

    /// node 타입(producer는 선언 타입, source는 입력 타입).
    pub fn type_at(&self, index: NodeIndex) -> Option<TypeSpec> {
        self.types[index.get()]
    }

    /// 전체 위상 순서(dependency가 먼저, 같은 단계는 번호 순).
    pub fn topological_order(&self) -> &[NodeIndex] {
        &self.topological
    }

    /// 모든 producer를 위상 순서로 다시 평가합니다.
    pub fn evaluate_all(&mut self) -> UpdateReport {
        let mut report = UpdateReport::default();
        let order: Vec<NodeIndex> = self
            .topological
            .iter()
            .copied()
            .filter(|index| self.graph.producer(*index).is_some())
            .collect();
        report.affected = order.len();
        report.visited = self.graph.node_count();
        for index in order {
            let value = self.evaluate_node(index);
            self.values[index.get()] = value;
            report.evaluated += 1;
            report.order.push(index);
        }
        report
    }

    /// source node 값을 바꾸고 영향받는 producer만 재평가합니다. 값이 같은 변경은 무시합니다.
    pub fn update_sources(
        &mut self,
        changes: Vec<(DependencyNode, Option<TypedValue>)>,
    ) -> Result<UpdateReport, SchedulerError> {
        let mut resolved = Vec::with_capacity(changes.len());
        for (node, value) in changes {
            let index = self
                .graph
                .index_of(&node)
                .ok_or_else(|| SchedulerError::UnknownNode(node.clone()))?;
            if self.graph.producer(index).is_some() {
                return Err(SchedulerError::NotASource(node));
            }
            resolved.push((index, value));
        }
        let mut changed = Vec::new();
        for (index, value) in resolved {
            let new = Ok(value);
            if self.values[index.get()] != new {
                self.values[index.get()] = new;
                changed.push(index);
            }
        }
        Ok(self.propagate(&changed, &[]))
    }

    /// producer node의 표현식을 같은 dependency의 새 표현식으로 바꾸고 영향받는 node만 재평가합니다.
    ///
    /// 새 표현식의 참조가 기존 dependency와 다르면 graph 구조가 바뀌므로 [`SchedulerError::StructureChanged`]입니다.
    pub fn update_expression(
        &mut self,
        node: &DependencyNode,
        expression: ExpressionAst,
    ) -> Result<UpdateReport, SchedulerError> {
        let index = self
            .graph
            .index_of(node)
            .ok_or_else(|| SchedulerError::UnknownNode(node.clone()))?;
        if self.graph.producer(index).is_none() {
            return Err(SchedulerError::NotAProducer(node.clone()));
        }
        let references: BTreeSet<DependencyNode> = expression
            .referenced_targets()
            .into_iter()
            .map(DependencyNode::from_target)
            .collect();
        let current: BTreeSet<DependencyNode> = self
            .graph
            .dependencies(index)
            .iter()
            .map(|dep| self.graph.node(*dep).clone())
            .collect();
        if references != current {
            return Err(SchedulerError::StructureChanged(node.clone()));
        }
        self.expressions[index.get()] = Some(expression);
        self.compile(index);
        Ok(self.propagate(&[], &[index]))
    }

    /// 값이 바뀐 node(`changed`)와 표현식이 바뀐 producer(`rewritten`)에서 시작해 영향받는 producer를 재평가합니다.
    ///
    /// 방문 기록은 영향 범위 크기에 비례하는 집합으로 두어, graph 전체 크기가 아니라 영향 범위에 비례해 동작합니다.
    fn propagate(&mut self, changed: &[NodeIndex], rewritten: &[NodeIndex]) -> UpdateReport {
        let mut report = UpdateReport {
            changed_inputs: changed.len() + rewritten.len(),
            ..UpdateReport::default()
        };
        let changed_set: HashSet<NodeIndex> = changed.iter().copied().collect();
        let rewritten_set: HashSet<NodeIndex> = rewritten.iter().copied().collect();
        // 1. 영향 범위: 역방향 BFS.
        let mut seen: HashSet<NodeIndex> = HashSet::new();
        let mut queue: VecDeque<NodeIndex> = VecDeque::new();
        for &start in changed.iter().chain(rewritten) {
            if seen.insert(start) {
                queue.push_back(start);
            }
        }
        let mut affected: Vec<NodeIndex> = Vec::new();
        while let Some(index) = queue.pop_front() {
            report.visited += 1;
            if self.graph.producer(index).is_some()
                && (rewritten_set.contains(&index) || !changed_set.contains(&index))
            {
                affected.push(index);
            }
            for &dependent in self.graph.dependents(index) {
                if seen.insert(dependent) {
                    queue.push_back(dependent);
                }
            }
        }
        affected.sort_unstable_by_key(|index| self.rank[index.get()]);
        report.affected = affected.len();
        // 2. 위상 순위 순 평가(early cutoff). dirty = 이번 갱신에서 값이 바뀐 node.
        let mut dirty: HashSet<NodeIndex> = changed_set;
        dirty.extend(rewritten.iter().copied());
        for index in affected {
            let must = rewritten_set.contains(&index)
                || self
                    .graph
                    .dependencies(index)
                    .iter()
                    .any(|dep| dirty.contains(dep));
            if !must {
                continue;
            }
            let value = self.evaluate_node(index);
            report.evaluated += 1;
            report.order.push(index);
            let at = index.get();
            // 값이 같으면(표현식을 바꾼 producer 포함) 위로 전파하지 않습니다.
            if self.values[at] != value {
                dirty.insert(index);
            } else {
                dirty.remove(&index);
            }
            self.values[at] = value;
        }
        report
    }

    /// producer 하나를 현재 값들로 평가합니다.
    fn evaluate_node(&self, index: NodeIndex) -> NodeValue {
        let at = index.get();
        let program = match &self.programs[at] {
            Some(Ok(program)) => program,
            Some(Err(error)) => return Err(NodeEvalError::Compile(error.clone())),
            None => return Err(NodeEvalError::MissingSource),
        };
        let view = ValueView {
            graph: &self.graph,
            values: &self.values,
        };
        program
            .evaluate_with_limits(&view, self.limits)
            .map(|(value, _)| value)
            .map_err(|error| self.classify(error))
    }

    /// 읽은 node의 값이 오류여서 난 평가 오류는 dependency 오류로 바꿉니다.
    fn classify(&self, error: EvalError) -> NodeEvalError {
        if let EvalErrorKind::MissingReference(target) = &error.kind {
            let node = DependencyNode::from_target(target);
            if let Some(index) = self.graph.index_of(&node) {
                if self.values[index.get()].is_err() {
                    return NodeEvalError::Dependency(node);
                }
            }
        }
        NodeEvalError::Eval(error)
    }
}

/// 위상 순위(Kahn, 같은 단계는 번호 순)와 순서. graph는 비순환이어야 합니다.
fn topological_ranks(graph: &DependencyGraph) -> (Vec<u32>, Vec<NodeIndex>) {
    let count = graph.node_count();
    let mut remaining: Vec<usize> = graph
        .indices()
        .map(|index| graph.dependencies(index).len())
        .collect();
    let mut ready: BTreeSet<NodeIndex> = graph
        .indices()
        .filter(|index| remaining[index.get()] == 0)
        .collect();
    let mut rank = vec![u32::MAX; count];
    let mut order = Vec::with_capacity(count);
    while let Some(index) = ready.pop_first() {
        rank[index.get()] = order.len() as u32;
        order.push(index);
        for &dependent in graph.dependents(index) {
            let left = &mut remaining[dependent.get()];
            *left -= 1;
            if *left == 0 {
                ready.insert(dependent);
            }
        }
    }
    (rank, order)
}

#[cfg(test)]
mod tests {
    use stellar_core_types::ids::StableId;
    use stellar_core_types::property::{PropertyKey, PropertyPath};
    use stellar_core_types::reference::{NodeRef, TokenRef};
    use stellar_core_types::value::FiniteF32;
    use stellar_expressions::ast::{EnvironmentKey, ExprNode, ExprNodeId};
    use stellar_expressions::ops::BinaryOp;
    use stellar_expressions::types::ExpressionType;

    use super::*;
    use crate::binding::{BindingId, ScopeOwner, ValueBinding};
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

    fn num(v: f32) -> TypedValue {
        TypedValue::Number(FiniteF32::new(v).expect("유한"))
    }

    fn number() -> TypeSpec {
        TypeSpec::required(ExpressionType::Number)
    }

    /// 대상들의 합(대상 0개면 `literal`).
    fn sum(base: u64, targets: &[ReferenceTarget], literal: f32) -> ExpressionAst {
        let node = |k: u64| ExprNodeId::new(sid(base * 100 + k));
        let mut nodes: Vec<ExprNode> = targets
            .iter()
            .enumerate()
            .map(|(i, t)| ExprNode::reference(node(50 + i as u64), t.clone()))
            .collect();
        match targets.len() {
            0 => {
                nodes.push(ExprNode::literal(node(0), num(literal)));
                ExpressionAst::new(node(0), nodes).expect("유효")
            }
            1 => ExpressionAst::new(node(50), nodes).expect("유효"),
            n => {
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

    fn t(n: u64) -> ReferenceTarget {
        ReferenceTarget::Token(token(n))
    }

    fn p(n: u64) -> ReferenceTarget {
        ReferenceTarget::Property(prop(n))
    }

    fn e(n: u32) -> ReferenceTarget {
        ReferenceTarget::Environment(EnvironmentKey::new(n))
    }

    fn define(n: u64, targets: &[ReferenceTarget], literal: f32) -> TokenDefinition {
        TokenDefinition::new(
            token(n),
            TokenName::new(&format!("t{n}")).expect("이름"),
            TokenType::new(ExpressionType::Number),
            sum(n, targets, literal),
        )
    }

    fn bind(n: u64, target: u64, refs: &[ReferenceTarget]) -> ValueBinding {
        ValueBinding::new(
            BindingId::new(sid(4_000 + n)),
            prop(target),
            sum(100 + n, refs, 0.0),
            number(),
            ScopeOwner::Node(NodeRef::new(sid(5_000 + n))),
        )
    }

    /// 두 독립 subtree.
    ///
    /// - 왼쪽: t1 = 1, t2 = t1 + e1, p1 ← t2, p2 ← p1 + t1
    /// - 오른쪽: t3 = 10, p3 ← t3 + e2, p4 ← p3
    fn fixture() -> (TokenRegistry, BindingSet, StaticSources) {
        let mut set = TokenSet::new(TokenSetId::new(sid(1)));
        set.insert(define(1, &[], 1.0)).expect("t1");
        set.insert(define(2, &[t(1), e(1)], 0.0)).expect("t2");
        set.insert(define(3, &[], 10.0)).expect("t3");
        let mut registry = TokenRegistry::new();
        registry.add_scope(TokenScope::Project, set).expect("scope");
        let mut bindings = BindingSet::new();
        bindings.insert(bind(1, 1, &[t(2)])).expect("b1");
        bindings.insert(bind(2, 2, &[p(1), t(1)])).expect("b2");
        bindings.insert(bind(3, 3, &[t(3), e(2)])).expect("b3");
        bindings.insert(bind(4, 4, &[p(3)])).expect("b4");
        let sources = BTreeMap::from([
            (
                DependencyNode::Environment(EnvironmentKey::new(1)),
                (number(), Some(num(100.0))),
            ),
            (
                DependencyNode::Environment(EnvironmentKey::new(2)),
                (number(), Some(num(1000.0))),
            ),
        ]);
        (registry, bindings, sources)
    }

    fn value_of(evaluator: &BindingEvaluator, target: ReferenceTarget) -> NodeValue {
        evaluator
            .value(&DependencyNode::from_target(&target))
            .cloned()
            .expect("node")
    }

    fn nodes_of(evaluator: &BindingEvaluator, order: &[NodeIndex]) -> Vec<DependencyNode> {
        order
            .iter()
            .map(|i| evaluator.graph().node(*i).clone())
            .collect()
    }

    #[test]
    fn initial_evaluation_follows_topological_order() {
        let (registry, bindings, sources) = fixture();
        let evaluator = BindingEvaluator::new(&registry, &bindings, &sources).expect("생성");
        assert_eq!(value_of(&evaluator, t(2)), Ok(Some(num(101.0))));
        assert_eq!(value_of(&evaluator, p(1)), Ok(Some(num(101.0))));
        assert_eq!(value_of(&evaluator, p(2)), Ok(Some(num(102.0))));
        assert_eq!(value_of(&evaluator, p(4)), Ok(Some(num(1010.0))));
        // 위상 순서: dependency가 항상 먼저.
        let rank: BTreeMap<NodeIndex, usize> = evaluator
            .topological_order()
            .iter()
            .enumerate()
            .map(|(r, i)| (*i, r))
            .collect();
        for index in evaluator.graph().indices() {
            for dep in evaluator.graph().dependencies(index) {
                assert!(rank[dep] < rank[&index]);
            }
        }
    }

    #[test]
    fn unrelated_subtree_is_not_evaluated_and_order_is_deterministic() {
        let (registry, bindings, sources) = fixture();
        let mut evaluator = BindingEvaluator::new(&registry, &bindings, &sources).expect("생성");
        // e1 변경: 왼쪽 subtree(t2, p1, p2)만 평가하고 오른쪽(p3, p4)은 0.
        let report = evaluator
            .update_sources(vec![(
                DependencyNode::Environment(EnvironmentKey::new(1)),
                Some(num(200.0)),
            )])
            .expect("변경");
        let evaluated = nodes_of(&evaluator, &report.order);
        assert_eq!(
            evaluated,
            vec![
                DependencyNode::Token(token(2)),
                DependencyNode::Property(prop(1)),
                DependencyNode::Property(prop(2)),
            ]
        );
        assert_eq!(report.changed_inputs, 1);
        assert_eq!(report.affected, 3);
        assert_eq!(report.evaluated, 3);
        assert_eq!(report.visited, 4, "e1, t2, p1, p2");
        assert!(evaluated.iter().all(|n| !matches!(
            n,
            DependencyNode::Property(path) if *path == prop(3) || *path == prop(4)
        )));
        assert_eq!(value_of(&evaluator, p(2)), Ok(Some(num(202.0))));
        assert_eq!(value_of(&evaluator, p(4)), Ok(Some(num(1010.0))));
        // 같은 변경을 새 evaluator에서 반복해도 같은 순서·같은 기록.
        let mut again = BindingEvaluator::new(&registry, &bindings, &sources).expect("생성");
        let second = again
            .update_sources(vec![(
                DependencyNode::Environment(EnvironmentKey::new(1)),
                Some(num(200.0)),
            )])
            .expect("변경");
        assert_eq!(second, report);
        // 같은 값으로 다시 설정하면 아무것도 평가하지 않습니다.
        let none = evaluator
            .update_sources(vec![(
                DependencyNode::Environment(EnvironmentKey::new(1)),
                Some(num(200.0)),
            )])
            .expect("변경");
        assert_eq!(none.evaluated, 0);
        assert_eq!(none.changed_inputs, 0);
    }

    #[test]
    fn token_expression_change_reevaluates_dependents_with_cutoff() {
        let (registry, bindings, sources) = fixture();
        let mut evaluator = BindingEvaluator::new(&registry, &bindings, &sources).expect("생성");
        // t3 = 20: p3, p4만.
        let report = evaluator
            .update_expression(&DependencyNode::Token(token(3)), sum(3, &[], 20.0))
            .expect("교체");
        assert_eq!(
            nodes_of(&evaluator, &report.order),
            vec![
                DependencyNode::Token(token(3)),
                DependencyNode::Property(prop(3)),
                DependencyNode::Property(prop(4)),
            ]
        );
        assert_eq!(value_of(&evaluator, p(4)), Ok(Some(num(1020.0))));
        // t1을 같은 값(1)의 다른 표현식으로: t1만 평가하고 그 위는 cutoff.
        let report = evaluator
            .update_expression(&DependencyNode::Token(token(1)), sum(7, &[], 1.0))
            .expect("교체");
        assert_eq!(report.affected, 4, "t1, t2, p1, p2");
        assert_eq!(report.evaluated, 1, "값이 같아 전파하지 않음");
        // dependency가 바뀌는 교체는 구조 변경.
        assert_eq!(
            evaluator.update_expression(&DependencyNode::Token(token(1)), sum(8, &[t(3)], 0.0)),
            Err(SchedulerError::StructureChanged(DependencyNode::Token(
                token(1)
            )))
        );
        assert!(matches!(
            evaluator.update_sources(vec![(DependencyNode::Token(token(1)), Some(num(1.0)))]),
            Err(SchedulerError::NotASource(_))
        ));
        assert!(matches!(
            evaluator.update_expression(
                &DependencyNode::Environment(EnvironmentKey::new(1)),
                sum(9, &[], 0.0)
            ),
            Err(SchedulerError::NotAProducer(_))
        ));
    }

    #[test]
    fn missing_sources_errors_and_cycles() {
        let (registry, mut bindings, mut sources) = fixture();
        sources.remove(&DependencyNode::Environment(EnvironmentKey::new(2)));
        // e2 타입을 모르면 p3는 컴파일 오류, p4는 dependency 오류.
        let mut evaluator = BindingEvaluator::new(&registry, &bindings, &sources).expect("생성");
        assert!(matches!(
            value_of(&evaluator, p(3)),
            Err(NodeEvalError::Compile(_))
        ));
        assert_eq!(
            value_of(&evaluator, p(4)),
            Err(NodeEvalError::Dependency(DependencyNode::Property(prop(3))))
        );
        assert_eq!(
            value_of(&evaluator, e(2)),
            Err(NodeEvalError::MissingSource)
        );
        assert!(matches!(
            evaluator.update_sources(vec![(DependencyNode::Property(prop(77)), None)]),
            Err(SchedulerError::UnknownNode(_))
        ));
        // cycle graph는 만들 수 없습니다.
        bindings.insert(bind(9, 9, &[p(10)])).expect("b9");
        bindings.insert(bind(10, 10, &[p(9)])).expect("b10");
        assert!(matches!(
            BindingEvaluator::new(&registry, &bindings, &sources),
            Err(SchedulerError::Cycle(_))
        ));
    }
}
