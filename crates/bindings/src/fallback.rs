//! orphan binding 기본값 fallback (VIII.8.2).
//!
//! orphan binding(VIII.8.1)의 target 속성 값은 그 속성(대상 Component Property)의 **선언된 기본값**으로 평가합니다
//! (아키텍처 §14.4). 마지막으로 계산했던 값은 저장하지도 쓰지도 않습니다: [`OrphanedBinding`]에는 값 필드가 없고,
//! 평가할 때마다 선언 기본값 표현식을 다시 평가합니다. 따라서 다시 실행해도 같은 기본값이 나옵니다.
//!
//! 기본값은 표현식이라 토큰 등을 참조할 수 있으며, scheduler(VIII.6.4)에서 [`Producer::OrphanDefault`] producer로
//! 평가됩니다(위상 순서·증분 재평가·provenance 그대로). 같은 target에 live binding이 다시 생기면 live binding이
//! 우선하고 기본값 producer는 만들지 않습니다.

use std::collections::BTreeMap;

use stellar_core_types::property::PropertyPath;
use stellar_expressions::ast::ExpressionAst;
use stellar_expressions::types::TypeSpec;
use stellar_expressions::vm::EvalLimits;

use crate::graph::{DependencyNode, Producer};
use crate::orphan::{BindingStore, OrphanedBinding};
use crate::registry::TokenRegistry;
use crate::scheduler::{token_specs, BindingEvaluator, ProducerSpec, SchedulerError, SourceInputs};

/// 속성(Component Property)의 선언 기본값.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredDefault {
    /// 속성의 현재 선언 타입.
    pub declared_type: TypeSpec,
    /// 기본값 표현식.
    pub expression: ExpressionAst,
}

/// target 속성 → 선언 기본값 제공자.
pub trait DeclaredDefaults {
    /// target 속성의 선언 기본값. 선언되지 않았으면 `None`.
    fn declared_default(&self, target: &PropertyPath) -> Option<DeclaredDefault>;
}

impl DeclaredDefaults for BTreeMap<PropertyPath, DeclaredDefault> {
    fn declared_default(&self, target: &PropertyPath) -> Option<DeclaredDefault> {
        self.get(target).cloned()
    }
}

/// orphan의 기본값 producer 명세. 선언 기본값이 없으면 `None`.
pub fn orphan_default_spec(
    orphan: &OrphanedBinding,
    defaults: &dyn DeclaredDefaults,
) -> Option<ProducerSpec> {
    let default = defaults.declared_default(orphan.target())?;
    Some(ProducerSpec {
        node: DependencyNode::Property(orphan.target().clone()),
        producer: Producer::OrphanDefault(orphan.id()),
        value_type: Some(default.declared_type),
        expression: default.expression,
    })
}

/// 토큰·live binding·orphan 기본값을 모두 producer로 하는 명세. live binding이 있는 target의 orphan은 제외합니다.
pub fn store_specs(
    tokens: &TokenRegistry,
    store: &BindingStore,
    defaults: &dyn DeclaredDefaults,
) -> Vec<ProducerSpec> {
    let mut specs = token_specs(tokens);
    specs.extend(store.live().iter().map(|binding| ProducerSpec {
        node: DependencyNode::Property(binding.target().clone()),
        producer: Producer::Binding(binding.id()),
        value_type: Some(binding.expected_type()),
        expression: binding.source().clone(),
    }));
    specs.extend(
        store
            .orphans()
            .filter(|orphan| store.live().by_target(orphan.target()).is_none())
            .filter_map(|orphan| orphan_default_spec(orphan, defaults)),
    );
    specs
}

impl BindingEvaluator {
    /// live binding과 orphan binding을 함께 평가합니다. orphan target은 선언 기본값으로 평가합니다(기본 평가 한도).
    pub fn with_orphans(
        tokens: &TokenRegistry,
        store: &BindingStore,
        defaults: &dyn DeclaredDefaults,
        sources: &dyn SourceInputs,
    ) -> Result<BindingEvaluator, SchedulerError> {
        BindingEvaluator::with_orphans_and_limits(
            tokens,
            store,
            defaults,
            sources,
            EvalLimits::DEFAULT,
        )
    }

    /// 평가 한도를 지정한 [`BindingEvaluator::with_orphans`].
    pub fn with_orphans_and_limits(
        tokens: &TokenRegistry,
        store: &BindingStore,
        defaults: &dyn DeclaredDefaults,
        sources: &dyn SourceInputs,
        limits: EvalLimits,
    ) -> Result<BindingEvaluator, SchedulerError> {
        let specs = store_specs(tokens, store, defaults);
        let sources = crate::scheduler::TokenAwareSources {
            tokens,
            inner: sources,
        };
        BindingEvaluator::from_specs(specs, &sources, limits)
    }
}

#[cfg(test)]
mod tests {
    use stellar_core_types::ids::StableId;
    use stellar_core_types::property::PropertyKey;
    use stellar_core_types::reference::{NodeRef, TokenRef};
    use stellar_core_types::typed_value::TypedValue;
    use stellar_core_types::value::{FiniteF32, Length};
    use stellar_expressions::ast::{ExprNode, ExprNodeId, ReferenceTarget};
    use stellar_expressions::ops::BinaryOp;
    use stellar_expressions::types::ExpressionType;

    use super::*;
    use crate::binding::{BindingId, BindingSet, ScopeOwner, ValueBinding};
    use crate::orphan::BindingWorld;
    use crate::registry::TokenScope;
    use crate::scheduler::{NodeValue, StaticSources};
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

    fn px(v: f32) -> TypedValue {
        TypedValue::Length(Length::new(v).expect("유한"))
    }

    fn length() -> TypeSpec {
        TypeSpec::required(ExpressionType::Length)
    }

    fn literal(base: u64, value: TypedValue) -> ExpressionAst {
        ExpressionAst::new(x(base), vec![ExprNode::literal(x(base), value)]).expect("유효")
    }

    /// `target (op) literal`.
    fn apply(base: u64, op: BinaryOp, target: ReferenceTarget, value: TypedValue) -> ExpressionAst {
        ExpressionAst::new(
            x(base),
            vec![
                ExprNode::binary(x(base), op, x(base + 1), x(base + 2)),
                ExprNode::reference(x(base + 1), target),
                ExprNode::literal(x(base + 2), value),
            ],
        )
        .expect("유효")
    }

    fn two() -> TypedValue {
        TypedValue::Number(FiniteF32::new(2.0).expect("유한"))
    }

    fn registry(with_t1: bool) -> TokenRegistry {
        let mut set = TokenSet::new(TokenSetId::new(sid(1)));
        if with_t1 {
            set.insert(TokenDefinition::new(
                token(1),
                TokenName::new("t1").expect("이름"),
                TokenType::new(ExpressionType::Length),
                literal(10, px(10.0)),
            ))
            .expect("t1");
        }
        set.insert(TokenDefinition::new(
            token(2),
            TokenName::new("t2").expect("이름"),
            TokenType::new(ExpressionType::Length),
            literal(20, px(3.0)),
        ))
        .expect("t2");
        let mut registry = TokenRegistry::new();
        registry.add_scope(TokenScope::Project, set).expect("scope");
        registry
    }

    fn bind(n: u64, target: u64, source: ExpressionAst) -> ValueBinding {
        ValueBinding::new(
            BindingId::new(sid(4_000 + n)),
            prop(target),
            source,
            length(),
            ScopeOwner::Node(NodeRef::new(sid(5_000 + n))),
        )
    }

    /// b1: p1 ← t1 + 1px, b2: p2 ← p1 × 2.
    fn live() -> BindingSet {
        let mut set = BindingSet::new();
        set.insert(bind(
            1,
            1,
            apply(
                100,
                BinaryOp::Add,
                ReferenceTarget::Token(token(1)),
                px(1.0),
            ),
        ))
        .expect("b1");
        set.insert(bind(
            2,
            2,
            apply(
                200,
                BinaryOp::Multiply,
                ReferenceTarget::Property(prop(1)),
                two(),
            ),
        ))
        .expect("b2");
        set
    }

    struct World<'a>(&'a TokenRegistry);

    impl BindingWorld for World<'_> {
        fn target_type(&self, _target: &PropertyPath) -> Option<TypeSpec> {
            Some(length())
        }

        fn reference_type(&self, target: &ReferenceTarget) -> Option<TypeSpec> {
            match target {
                ReferenceTarget::Token(t) => self
                    .0
                    .resolve(*t)
                    .map(|r| r.definition.token_type().type_spec()),
                ReferenceTarget::Property(_) => Some(length()),
                _ => None,
            }
        }
    }

    fn defaults(expression: ExpressionAst) -> BTreeMap<PropertyPath, DeclaredDefault> {
        BTreeMap::from([(
            prop(1),
            DeclaredDefault {
                declared_type: length(),
                expression,
            },
        )])
    }

    fn value(evaluator: &BindingEvaluator, n: u64) -> NodeValue {
        evaluator
            .value(&DependencyNode::Property(prop(n)))
            .cloned()
            .expect("node")
    }

    #[test]
    fn orphan_target_uses_declared_default_not_last_value() {
        let sources = StaticSources::new();
        // live 상태: p1 = 11px, p2 = 22px.
        let before = registry(true);
        let live_eval = BindingEvaluator::new(&before, &live(), &sources).expect("생성");
        assert_eq!(value(&live_eval, 1), Ok(Some(px(11.0))));
        assert_eq!(value(&live_eval, 2), Ok(Some(px(22.0))));
        // t1 삭제 → b1 orphan.
        let after = registry(false);
        let mut store = BindingStore::new(live());
        assert_eq!(
            store.orphan_broken(&World(&after)),
            vec![BindingId::new(sid(4_001))]
        );
        let declared = defaults(literal(300, px(4.0)));
        let evaluator =
            BindingEvaluator::with_orphans(&after, &store, &declared, &sources).expect("생성");
        // 마지막 계산값(11px)이 아니라 선언 기본값(4px).
        assert_eq!(value(&evaluator, 1), Ok(Some(px(4.0))));
        assert_eq!(
            value(&evaluator, 2),
            Ok(Some(px(8.0))),
            "dependent도 기본값 기준"
        );
        let index = evaluator
            .graph()
            .index_of(&DependencyNode::Property(prop(1)))
            .expect("node");
        assert_eq!(
            evaluator.graph().producer(index),
            Some(Producer::OrphanDefault(BindingId::new(sid(4_001))))
        );
        // 재실행(새 evaluator)해도 같은 기본값.
        let again =
            BindingEvaluator::with_orphans(&after, &store, &declared, &sources).expect("생성");
        assert_eq!(value(&again, 1), Ok(Some(px(4.0))));
        assert_eq!(value(&again, 2), Ok(Some(px(8.0))));
        // provenance도 기본값 producer를 가리킵니다.
        let chain = again
            .provenance(&DependencyNode::Property(prop(2)))
            .expect("chain");
        assert_eq!(
            chain
                .step(&DependencyNode::Property(prop(1)))
                .expect("p1")
                .producer,
            Some(Producer::OrphanDefault(BindingId::new(sid(4_001))))
        );
    }

    #[test]
    fn default_expressions_are_evaluated_and_live_binding_wins() {
        let sources = StaticSources::new();
        let after = registry(false);
        let mut store = BindingStore::new(live());
        store.orphan_broken(&World(&after));
        // 기본값이 토큰을 참조: t2 × 2 = 6px.
        let declared = defaults(apply(
            310,
            BinaryOp::Multiply,
            ReferenceTarget::Token(token(2)),
            two(),
        ));
        let mut evaluator =
            BindingEvaluator::with_orphans(&after, &store, &declared, &sources).expect("생성");
        assert_eq!(value(&evaluator, 1), Ok(Some(px(6.0))));
        // 기본값이 읽는 토큰을 바꾸면 증분 재평가.
        let report = evaluator
            .update_expression(&DependencyNode::Token(token(2)), literal(21, px(5.0)))
            .expect("교체");
        assert_eq!(report.evaluated, 3, "t2, p1(기본값), p2");
        assert_eq!(value(&evaluator, 2), Ok(Some(px(20.0))));
        // 같은 target에 live binding이 다시 생기면 live가 우선합니다.
        let mut live_again = store.clone();
        live_again
            .insert_live(bind(9, 1, literal(900, px(1.5))))
            .expect("새 b9");
        assert!(
            live_again.orphan(BindingId::new(sid(4_001))).is_some(),
            "orphan 유지"
        );
        let evaluator =
            BindingEvaluator::with_orphans(&after, &live_again, &declared, &sources).expect("생성");
        assert_eq!(value(&evaluator, 1), Ok(Some(px(1.5))));
        // orphan과 같은 ID의 live binding은 추가할 수 없습니다.
        assert!(live_again
            .insert_live(bind(1, 7, literal(910, px(1.0))))
            .is_err());
    }
}
