//! 환경 입력 reverse index와 환경 변경 무효화 (VIII.7.3).
//!
//! [`EnvironmentIndex`]는 환경 key → 그 key를 직접 참조하는 표현식(producer node: 토큰·binding) 목록입니다. 환경이
//! 바뀌면 [`EvaluationEnvironment::changed_keys`](VIII.7.2)로 평가 값이 실제로 바뀐 key만 구하고, 그 key를 읽는
//! 표현식과 그 위의 dependents만 scheduler(VIII.6.4)로 재평가합니다. 다른 key만 읽는 표현식(예: color scheme
//! 변경 시 viewport만 읽는 표현식)은 평가하지 않습니다.
//!
//! [`EnvironmentSources`]는 환경 node의 타입·값을 key registry(VIII.7.1)와 [`EvaluationEnvironment`]에서, 나머지
//! source는 다른 제공자에서 가져오는 [`SourceInputs`]입니다.

use std::collections::{BTreeMap, BTreeSet};

use stellar_core_types::typed_value::TypedValue;
use stellar_expressions::ast::EnvironmentKey;
use stellar_expressions::environment::{key_info, EvaluationEnvironment};
use stellar_expressions::types::TypeSpec;

use crate::graph::{DependencyGraph, DependencyNode, NodeIndex};
use crate::scheduler::{BindingEvaluator, SchedulerError, SourceInputs, UpdateReport};

/// 환경 key → 직접 참조하는 producer node reverse index (VIII.7.3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvironmentIndex {
    readers: BTreeMap<EnvironmentKey, Vec<NodeIndex>>,
}

impl EnvironmentIndex {
    /// graph의 환경 node마다 직접 읽는 producer를 모읍니다(번호 오름차순).
    pub fn build(graph: &DependencyGraph) -> EnvironmentIndex {
        let mut readers = BTreeMap::new();
        for index in graph.indices() {
            if let DependencyNode::Environment(key) = graph.node(index) {
                let direct: Vec<NodeIndex> = graph
                    .dependents(index)
                    .iter()
                    .copied()
                    .filter(|reader| graph.producer(*reader).is_some())
                    .collect();
                readers.insert(*key, direct);
            }
        }
        EnvironmentIndex { readers }
    }

    /// `key`를 직접 참조하는 표현식(producer node).
    pub fn readers(&self, key: EnvironmentKey) -> &[NodeIndex] {
        self.readers.get(&key).map(Vec::as_slice).unwrap_or(&[])
    }

    /// 어떤 표현식이든 참조하는 key(ID 순).
    pub fn keys(&self) -> impl Iterator<Item = EnvironmentKey> + '_ {
        self.readers.keys().copied()
    }

    /// `changed` key를 직접 참조하는 표현식 전체(번호 순).
    pub fn invalidated(&self, changed: &BTreeSet<EnvironmentKey>) -> BTreeSet<NodeIndex> {
        changed
            .iter()
            .flat_map(|key| self.readers(*key).iter().copied())
            .collect()
    }
}

/// 환경 node는 registry·환경에서, 그 밖의 source는 `others`에서 타입·값을 가져오는 source 제공자.
pub struct EnvironmentSources<'a> {
    /// 평가 환경.
    pub environment: &'a EvaluationEnvironment,
    /// 환경이 아닌 source 제공자.
    pub others: &'a dyn SourceInputs,
}

impl SourceInputs for EnvironmentSources<'_> {
    fn source_type(&self, node: &DependencyNode) -> Option<TypeSpec> {
        match node {
            DependencyNode::Environment(key) => {
                key_info(*key).map(|info| TypeSpec::required(info.value_type))
            }
            other => self.others.source_type(other),
        }
    }

    fn source_value(&self, node: &DependencyNode) -> Option<Option<TypedValue>> {
        match node {
            DependencyNode::Environment(key) => self.environment.value(*key).map(Some),
            other => self.others.source_value(other),
        }
    }
}

impl BindingEvaluator {
    /// 환경을 `previous`에서 `next`로 바꿉니다. 평가 값이 바뀐 key 중 표현식이 참조하는 key만 갱신하고, 그 key를
    /// 읽는 표현식과 dependents만 재평가합니다.
    pub fn apply_environment(
        &mut self,
        index: &EnvironmentIndex,
        previous: &EvaluationEnvironment,
        next: &EvaluationEnvironment,
    ) -> Result<UpdateReport, SchedulerError> {
        let changes: Vec<(DependencyNode, Option<TypedValue>)> = previous
            .changed_keys(next)
            .into_iter()
            .filter(|key| index.readers.contains_key(key))
            .map(|key| (DependencyNode::Environment(key), next.value(key)))
            .collect();
        self.update_sources(changes)
    }
}

#[cfg(test)]
mod tests {
    use stellar_core_types::ids::StableId;
    use stellar_core_types::property::{PropertyKey, PropertyPath};
    use stellar_core_types::reference::NodeRef;
    use stellar_core_types::typed_value::EnumValue;
    use stellar_core_types::value::{FiniteF32, Length};
    use stellar_expressions::ast::{ExprNode, ExprNodeId, ExpressionAst, ReferenceTarget};
    use stellar_expressions::environment::{
        color_scheme, COLOR_SCHEME, COLOR_SCHEME_TYPE, REDUCED_MOTION, VIEWPORT_WIDTH,
    };
    use stellar_expressions::ops::BinaryOp;
    use stellar_expressions::types::ExpressionType;

    use super::*;
    use crate::binding::{BindingId, BindingSet, ScopeOwner, ValueBinding};
    use crate::registry::TokenRegistry;
    use crate::scheduler::StaticSources;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn prop(n: u64) -> PropertyPath {
        PropertyPath::new(sid(2_000 + n)).push(PropertyKey(1))
    }

    fn x(n: u64) -> ExprNodeId {
        ExprNodeId::new(sid(n))
    }

    fn px(v: f32) -> TypedValue {
        TypedValue::Length(Length::new(v).expect("유한"))
    }

    fn dark() -> TypedValue {
        TypedValue::Enum(EnumValue::new(COLOR_SCHEME_TYPE.raw(), color_scheme::DARK))
    }

    fn bind(n: u64, source: ExpressionAst) -> ValueBinding {
        ValueBinding::new(
            BindingId::new(sid(4_000 + n)),
            prop(n),
            source,
            TypeSpec::required(ExpressionType::Length),
            ScopeOwner::Node(NodeRef::new(sid(5_000 + n))),
        )
    }

    /// p1 ← `if color_scheme == dark then 2px else 1px`(scheme만), p2 ← `viewport_width × 0.5`(viewport만),
    /// p3 ← `p2 + 4px`(p2의 dependent).
    fn bindings() -> BindingSet {
        let mut set = BindingSet::new();
        set.insert(bind(
            1,
            ExpressionAst::new(
                x(10),
                vec![
                    ExprNode::if_else(x(10), x(11), x(14), x(15)),
                    ExprNode::binary(x(11), BinaryOp::Equal, x(12), x(13)),
                    ExprNode::reference(x(12), ReferenceTarget::Environment(COLOR_SCHEME)),
                    ExprNode::literal(x(13), dark()),
                    ExprNode::literal(x(14), px(2.0)),
                    ExprNode::literal(x(15), px(1.0)),
                ],
            )
            .expect("유효"),
        ))
        .expect("p1");
        set.insert(bind(
            2,
            ExpressionAst::new(
                x(20),
                vec![
                    ExprNode::binary(x(20), BinaryOp::Multiply, x(21), x(22)),
                    ExprNode::reference(x(21), ReferenceTarget::Environment(VIEWPORT_WIDTH)),
                    ExprNode::literal(
                        x(22),
                        TypedValue::Number(FiniteF32::new(0.5).expect("유한")),
                    ),
                ],
            )
            .expect("유효"),
        ))
        .expect("p2");
        set.insert(bind(
            3,
            ExpressionAst::new(
                x(30),
                vec![
                    ExprNode::binary(x(30), BinaryOp::Add, x(31), x(32)),
                    ExprNode::reference(x(31), ReferenceTarget::Property(prop(2))),
                    ExprNode::literal(x(32), px(4.0)),
                ],
            )
            .expect("유효"),
        ))
        .expect("p3");
        set
    }

    fn setup(environment: &EvaluationEnvironment) -> (BindingEvaluator, EnvironmentIndex) {
        let others = StaticSources::new();
        let sources = EnvironmentSources {
            environment,
            others: &others,
        };
        let evaluator =
            BindingEvaluator::new(&TokenRegistry::new(), &bindings(), &sources).expect("생성");
        let index = EnvironmentIndex::build(evaluator.graph());
        (evaluator, index)
    }

    fn evaluated(evaluator: &BindingEvaluator, report: &UpdateReport) -> Vec<DependencyNode> {
        report
            .order
            .iter()
            .map(|i| evaluator.graph().node(*i).clone())
            .collect()
    }

    fn value(evaluator: &BindingEvaluator, n: u64) -> Option<TypedValue> {
        evaluator
            .value(&DependencyNode::Property(prop(n)))
            .and_then(|v| v.clone().ok())
            .flatten()
    }

    #[test]
    fn reverse_index_lists_direct_readers() {
        let (evaluator, index) = setup(&EvaluationEnvironment::new());
        let graph = evaluator.graph();
        let readers = |key| -> Vec<DependencyNode> {
            index
                .readers(key)
                .iter()
                .map(|i| graph.node(*i).clone())
                .collect()
        };
        assert_eq!(
            readers(COLOR_SCHEME),
            vec![DependencyNode::Property(prop(1))]
        );
        assert_eq!(
            readers(VIEWPORT_WIDTH),
            vec![DependencyNode::Property(prop(2))]
        );
        assert!(readers(REDUCED_MOTION).is_empty(), "참조 없는 key");
        assert_eq!(
            index.keys().collect::<Vec<_>>(),
            vec![COLOR_SCHEME, VIEWPORT_WIDTH]
        );
        // 기본값(light, 1280px)으로 평가.
        assert_eq!(value(&evaluator, 1), Some(px(1.0)));
        assert_eq!(value(&evaluator, 2), Some(px(640.0)));
        assert_eq!(value(&evaluator, 3), Some(px(644.0)));
    }

    #[test]
    fn color_scheme_change_does_not_evaluate_viewport_only_expressions() {
        let light = EvaluationEnvironment::new();
        let (mut evaluator, index) = setup(&light);
        let dark_env = light.with(COLOR_SCHEME, dark()).expect("유효");
        let report = evaluator
            .apply_environment(&index, &light, &dark_env)
            .expect("적용");
        assert_eq!(
            evaluated(&evaluator, &report),
            vec![DependencyNode::Property(prop(1))],
            "viewport만 읽는 p2·p3는 평가하지 않음"
        );
        assert_eq!(value(&evaluator, 1), Some(px(2.0)));
        // viewport 변경은 p2와 dependent p3만.
        let wide = dark_env.with(VIEWPORT_WIDTH, px(1920.0)).expect("유효");
        let report = evaluator
            .apply_environment(&index, &dark_env, &wide)
            .expect("적용");
        assert_eq!(
            evaluated(&evaluator, &report),
            vec![
                DependencyNode::Property(prop(2)),
                DependencyNode::Property(prop(3))
            ]
        );
        assert_eq!(value(&evaluator, 3), Some(px(964.0)));
        // 참조 없는 key·기본값과 같은 명시 값은 평가 0.
        let unrelated = wide
            .with(REDUCED_MOTION, TypedValue::Boolean(true))
            .expect("유효");
        let report = evaluator
            .apply_environment(&index, &wide, &unrelated)
            .expect("적용");
        assert_eq!((report.changed_inputs, report.evaluated), (0, 0));
        assert_eq!(
            index
                .invalidated(&BTreeSet::from([COLOR_SCHEME, VIEWPORT_WIDTH]))
                .len(),
            2
        );
    }
}
