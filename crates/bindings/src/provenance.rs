//! 평가 값 provenance chain (VIII.6.5).
//!
//! 최종 값이 어떤 토큰·표현식 node·Component Property·환경 입력 branch를 거쳤는지를 기록합니다(아키텍처 §14.2
//! "출처 경로 표시"). [`BindingEvaluator::provenance`]는 요청한 node에서 시작해
//!
//! 1. producer 표현식을 현재 값으로 다시 평가하며 **실제로 읽은** 참조(표현식 node StableId + 대상 node)와 선택한
//!    branch(`if`/`match`/`and`/`or`)를 기록하고([`evaluate_ast_traced`]),
//! 2. 읽은 node를 같은 방식으로 따라가
//!
//! source node(환경 입력·Component Property·binding 없는 속성)에 닿을 때까지의 chain을 너비 우선 순서로 만듭니다.
//! 선택되지 않은 branch가 참조하는 node는 chain에 들지 않습니다. 각 node는 한 번만 나옵니다. provenance는 요청 시에만
//! 계산하며 평가 경로(VIII.6.4)에 비용을 더하지 않습니다.

use std::collections::{BTreeSet, VecDeque};

use stellar_core_types::typed_value::TypedValue;
use stellar_expressions::ast::{ExprNodeId, ReferenceTarget};
use stellar_expressions::ast_eval::{evaluate_ast_traced, BranchDecision};
use stellar_expressions::eval::ReferenceValues;
use stellar_expressions::typecheck::ReferenceTypes;
use stellar_expressions::types::TypeSpec;

use crate::graph::{DependencyNode, NodeIndex, Producer};
use crate::scheduler::{BindingEvaluator, NodeValue};

/// provenance chain의 한 단계.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvenanceStep {
    /// 값을 가진 node.
    pub node: DependencyNode,
    /// 값을 만든 정의(source node는 `None`).
    pub producer: Option<Producer>,
    /// 현재 값.
    pub value: NodeValue,
    /// producer 표현식에서 실제로 읽은 참조: (참조 표현식 node, 읽은 node), 평가 순서.
    pub reads: Vec<(ExprNodeId, DependencyNode)>,
    /// producer 표현식에서 선택한 branch(평가 순서).
    pub branches: Vec<BranchDecision>,
}

/// 최종 값 → 원본 provenance chain (너비 우선, 첫 단계 = 요청 node).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvenanceChain {
    /// 단계 목록.
    pub steps: Vec<ProvenanceStep>,
}

impl ProvenanceChain {
    /// node의 단계.
    pub fn step(&self, node: &DependencyNode) -> Option<&ProvenanceStep> {
        self.steps.iter().find(|step| &step.node == node)
    }

    /// chain에 든 node(순서대로).
    pub fn nodes(&self) -> Vec<&DependencyNode> {
        self.steps.iter().map(|step| &step.node).collect()
    }

    /// 원본(source) node(순서대로).
    pub fn origins(&self) -> Vec<&DependencyNode> {
        self.steps
            .iter()
            .filter(|step| step.producer.is_none())
            .map(|step| &step.node)
            .collect()
    }
}

struct Values<'a>(&'a BindingEvaluator);

impl ReferenceValues for Values<'_> {
    fn reference_value(&self, target: &ReferenceTarget) -> Option<Option<TypedValue>> {
        let graph = self.0.graph();
        let index = graph.index_of(&DependencyNode::from_target(target))?;
        self.0.value_at(index).as_ref().ok().cloned()
    }
}

struct Types<'a>(&'a BindingEvaluator);

impl ReferenceTypes for Types<'_> {
    fn reference_type(&self, target: &ReferenceTarget) -> Option<TypeSpec> {
        let graph = self.0.graph();
        let index = graph.index_of(&DependencyNode::from_target(target))?;
        self.0.type_at(index)
    }
}

impl BindingEvaluator {
    /// `node`의 현재 값에 대한 provenance chain. graph에 없는 node면 `None`.
    pub fn provenance(&self, node: &DependencyNode) -> Option<ProvenanceChain> {
        let graph = self.graph();
        let start = graph.index_of(node)?;
        let mut seen: BTreeSet<NodeIndex> = BTreeSet::from([start]);
        let mut queue: VecDeque<NodeIndex> = VecDeque::from([start]);
        let mut steps = Vec::new();
        while let Some(index) = queue.pop_front() {
            let producer = graph.producer(index);
            let mut reads = Vec::new();
            let mut branches = Vec::new();
            if let (Some(_), Some(expression)) = (producer, self.expression_at(index)) {
                let (_, path) =
                    evaluate_ast_traced(expression, &Types(self), &Values(self), self.limits());
                for read in path.reads {
                    let read_node = DependencyNode::from_target(&read.target);
                    if let Some(read_index) = graph.index_of(&read_node) {
                        if seen.insert(read_index) {
                            queue.push_back(read_index);
                        }
                    }
                    reads.push((read.node, read_node));
                }
                branches = path.branches;
            }
            steps.push(ProvenanceStep {
                node: graph.node(index).clone(),
                producer,
                value: self.value_at(index).clone(),
                reads,
                branches,
            });
        }
        Some(ProvenanceChain { steps })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::ids::StableId;
    use stellar_core_types::property::{PropertyKey, PropertyPath};
    use stellar_core_types::reference::{NodeRef, TokenRef};
    use stellar_core_types::value::FiniteF32;
    use stellar_expressions::ast::{ChildSlot, EnvironmentKey, ExprNode, ExpressionAst};
    use stellar_expressions::functions::FunctionId;
    use stellar_expressions::types::ExpressionType;

    use super::*;
    use crate::binding::{BindingId, BindingSet, ScopeOwner, ValueBinding};
    use crate::registry::{TokenRegistry, TokenScope};
    use crate::scheduler::StaticSources;
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

    fn x(n: u64) -> ExprNodeId {
        ExprNodeId::new(sid(n))
    }

    fn dark() -> EnvironmentKey {
        EnvironmentKey::new(4)
    }

    fn cp() -> StableId {
        sid(3_001)
    }

    fn color(v: f32) -> TypedValue {
        TypedValue::Color(Color::new(ColorSpace::Srgb, v, v, v, 1.0).expect("유효"))
    }

    fn num(v: f32) -> TypedValue {
        TypedValue::Number(FiniteF32::new(v).expect("유한"))
    }

    fn color_token(n: u64, expression: ExpressionAst) -> TokenDefinition {
        TokenDefinition::new(
            token(n),
            TokenName::new(&format!("t{n}")).expect("이름"),
            TokenType::new(ExpressionType::Color),
            expression,
        )
    }

    /// - t1(dark) = 0.1 회색, t2(light) = 0.9 회색, t3(bg) = `if dark then t1 else t2`
    /// - p1 ← `t3`
    /// - p2 ← `with_alpha(t3, cp)`(Component Property cp = 투명도)
    fn fixture(is_dark: bool) -> BindingEvaluator {
        let mut set = TokenSet::new(TokenSetId::new(sid(1)));
        set.insert(color_token(
            1,
            ExpressionAst::new(x(10), vec![ExprNode::literal(x(10), color(0.1))]).expect("유효"),
        ))
        .expect("t1");
        set.insert(color_token(
            2,
            ExpressionAst::new(x(20), vec![ExprNode::literal(x(20), color(0.9))]).expect("유효"),
        ))
        .expect("t2");
        set.insert(color_token(
            3,
            ExpressionAst::new(
                x(30),
                vec![
                    ExprNode::if_else(x(30), x(31), x(32), x(33)),
                    ExprNode::reference(x(31), ReferenceTarget::Environment(dark())),
                    ExprNode::reference(x(32), ReferenceTarget::Token(token(1))),
                    ExprNode::reference(x(33), ReferenceTarget::Token(token(2))),
                ],
            )
            .expect("유효"),
        ))
        .expect("t3");
        let mut registry = TokenRegistry::new();
        registry.add_scope(TokenScope::Project, set).expect("scope");
        let mut bindings = BindingSet::new();
        bindings
            .insert(ValueBinding::new(
                BindingId::new(sid(4_001)),
                prop(1),
                ExpressionAst::new(
                    x(40),
                    vec![ExprNode::reference(x(40), ReferenceTarget::Token(token(3)))],
                )
                .expect("유효"),
                TypeSpec::required(ExpressionType::Color),
                ScopeOwner::Node(NodeRef::new(sid(5_001))),
            ))
            .expect("b1");
        bindings
            .insert(ValueBinding::new(
                BindingId::new(sid(4_002)),
                prop(2),
                ExpressionAst::new(
                    x(50),
                    vec![
                        ExprNode::call(x(50), FunctionId::WithAlpha, &[x(51), x(52)]),
                        ExprNode::reference(x(51), ReferenceTarget::Token(token(3))),
                        ExprNode::reference(x(52), ReferenceTarget::ComponentProperty(cp())),
                    ],
                )
                .expect("유효"),
                TypeSpec::required(ExpressionType::Color),
                ScopeOwner::Node(NodeRef::new(sid(5_002))),
            ))
            .expect("b2");
        let sources: StaticSources = BTreeMap::from([
            (
                DependencyNode::Environment(dark()),
                (
                    TypeSpec::required(ExpressionType::Boolean),
                    Some(TypedValue::Boolean(is_dark)),
                ),
            ),
            (
                DependencyNode::ComponentProperty(cp()),
                (TypeSpec::required(ExpressionType::Number), Some(num(0.5))),
            ),
        ]);
        BindingEvaluator::new(&registry, &bindings, &sources).expect("생성")
    }

    #[test]
    fn final_value_chain_reaches_token_branch_and_environment() {
        let evaluator = fixture(true);
        let chain = evaluator
            .provenance(&DependencyNode::Property(prop(1)))
            .expect("chain");
        use DependencyNode as N;
        assert_eq!(
            chain.nodes(),
            vec![
                &N::Property(prop(1)),
                &N::Token(token(3)),
                &N::Environment(dark()),
                &N::Token(token(1)),
            ],
            "선택되지 않은 t2는 chain에 없음"
        );
        let first = &chain.steps[0];
        assert_eq!(
            first.producer,
            Some(Producer::Binding(BindingId::new(sid(4_001))))
        );
        assert_eq!(first.value, Ok(Some(color(0.1))));
        assert_eq!(first.reads, vec![(x(40), N::Token(token(3)))]);
        let bg = chain.step(&N::Token(token(3))).expect("t3");
        assert_eq!(bg.producer, Some(Producer::Token(token(3))));
        assert_eq!(
            bg.reads,
            vec![(x(31), N::Environment(dark())), (x(32), N::Token(token(1)))]
        );
        assert_eq!(
            bg.branches,
            vec![BranchDecision {
                node: x(30),
                taken: ChildSlot::Then
            }],
            "환경 입력 branch"
        );
        let env = chain.step(&N::Environment(dark())).expect("env");
        assert_eq!(env.producer, None);
        assert_eq!(env.value, Ok(Some(TypedValue::Boolean(true))));
        assert_eq!(chain.origins(), vec![&N::Environment(dark())]);
        // 원본 t1은 상수 토큰(읽은 것 없음).
        let leaf = chain.step(&N::Token(token(1))).expect("t1");
        assert!(leaf.reads.is_empty() && leaf.branches.is_empty());
        assert_eq!(leaf.value, Ok(Some(color(0.1))));
    }

    #[test]
    fn chain_follows_branch_change_and_component_property() {
        use DependencyNode as N;
        let light = fixture(false);
        let chain = light.provenance(&N::Property(prop(2))).expect("chain");
        assert_eq!(
            chain.nodes(),
            vec![
                &N::Property(prop(2)),
                &N::Token(token(3)),
                &N::ComponentProperty(cp()),
                &N::Environment(dark()),
                &N::Token(token(2)),
            ]
        );
        assert_eq!(
            chain.steps[0].reads,
            vec![
                (x(51), N::Token(token(3))),
                (x(52), N::ComponentProperty(cp()))
            ]
        );
        assert_eq!(
            chain.step(&N::Token(token(3))).expect("t3").branches,
            vec![BranchDecision {
                node: x(30),
                taken: ChildSlot::Else
            }]
        );
        assert_eq!(
            chain.origins(),
            vec![&N::ComponentProperty(cp()), &N::Environment(dark())]
        );
        // 같은 evaluator의 provenance는 반복해도 같습니다.
        assert_eq!(light.provenance(&N::Property(prop(2))), Some(chain));
        assert!(light.provenance(&N::Property(prop(99))).is_none());
        // 환경 변경(VIII.6.4) 후에는 새 branch로 chain이 바뀝니다.
        let mut evaluator = light;
        evaluator
            .update_sources(vec![(
                N::Environment(dark()),
                Some(TypedValue::Boolean(true)),
            )])
            .expect("변경");
        let chain = evaluator.provenance(&N::Property(prop(1))).expect("chain");
        assert!(chain.step(&N::Token(token(1))).is_some());
        assert!(chain.step(&N::Token(token(2))).is_none());
    }
}
