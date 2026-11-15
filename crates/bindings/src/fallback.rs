//! orphan binding 기본값 fallback (VIII.8.2).
//!
//! orphan binding(VIII.8.1)의 target 속성 값은 그 속성(대상 Component Property)의 **선언된 기본값**으로 평가합니다
//! (아키텍처 §14.4). 마지막으로 계산했던 값은 저장하지도 쓰지도 않습니다: [`OrphanedBinding`]에는 값 필드가 없고,
//! 평가할 때마다 선언 기본값 표현식을 다시 평가합니다. 따라서 다시 실행해도 같은 기본값이 나옵니다.
//!
//! 기본값은 표현식이라 토큰 등을 참조할 수 있으며, scheduler(VIII.6.4)에서 [`Producer::OrphanDefault`] producer로
//! 평가됩니다(위상 순서·증분 재평가·provenance 그대로). 같은 target에 live binding이 다시 생기면 live binding이
//! 우선하고 기본값 producer는 만들지 않습니다.
//!
//! **손상된 기본값(VIII.8.3)**: 저장된 기본값([`StoredDefault`]: 선언 타입 + 저장된 canonical AST bytes)이 없거나,
//! bytes를 디코드할 수 없거나, 선언 타입과 맞지 않으면 그 타입의 **안전값**([`safe_value`])을 쓰고
//! [`CorruptionDiagnostic`]을 남깁니다. 열기는 실패하지 않으며, 저장된 bytes와 프로젝트 파일은 수정하지 않습니다
//! (입력은 읽기 전용 참조로만 받습니다).
//!
//! | 타입 | 안전값 |
//! |---|---|
//! | `Optional<T>` | 값 없음(null) |
//! | Boolean | `false` |
//! | Number·Length·Percentage·Angle·Duration | 0 |
//! | String | `""` |
//! | Enum | 같은 enum 타입의 variant 0 |
//! | Color | sRGB (0, 0, 0, 0) 투명 |
//! | Gradient | stop 없음 |
//! | Shadow | offset 0·blur 0·투명 |
//! | Typography | CSS initial record(`TypographyValue::default()`, ADR-0024) |
//! | ResourceRef | NIL `StableId` |

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::gradient::Gradient;
use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_core_types::shadow::{BlurRadius, Shadow};
use stellar_core_types::typed_value::{EnumValue, ResourceRef, TypedValue, TypographyValue};
use stellar_core_types::value::{Angle, Duration, FiniteF32, Length, Percentage, Vec2};
use stellar_expressions::ast::{ExprNode, ExprNodeId, ExpressionAst, ReferenceTarget};
use stellar_expressions::codec::CodecError;
use stellar_expressions::typecheck::ReferenceTypes;
use stellar_expressions::types::{ExpressionType, TypeSpec};
use stellar_expressions::vm::EvalLimits;

use crate::binding::{check_source, BindingId, BindingTypeError};
use crate::graph::{DependencyNode, Producer};
use crate::orphan::{BindingStore, OrphanedBinding};
use crate::registry::TokenRegistry;
use crate::scheduler::{token_specs, BindingEvaluator, ProducerSpec, SchedulerError, SourceInputs};
use crate::token::TokenTypes;

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

/// 비-nullable 타입의 안전값(모듈 문서 표). 구성값이 모두 상수라 실패하지 않으며, 실패하면 `None`입니다.
pub fn safe_value(value_type: ExpressionType) -> Option<TypedValue> {
    let zero = FiniteF32::ZERO;
    let transparent = Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 0.0).ok();
    Some(match value_type {
        ExpressionType::Boolean => TypedValue::Boolean(false),
        ExpressionType::Number => TypedValue::Number(zero),
        ExpressionType::String => TypedValue::String(String::new()),
        ExpressionType::Enum(id) => TypedValue::Enum(EnumValue::new(id.raw(), 0)),
        ExpressionType::Length => TypedValue::Length(Length::from_raw(zero)),
        ExpressionType::Percentage => TypedValue::Percentage(Percentage::from_raw(zero)),
        ExpressionType::Angle => TypedValue::Angle(Angle::from_raw(zero)),
        ExpressionType::Duration => TypedValue::Duration(Duration::from_raw(zero)),
        ExpressionType::Color => TypedValue::Color(transparent?),
        ExpressionType::Gradient => TypedValue::Gradient(Gradient::new(Vec::new())),
        ExpressionType::Shadow => {
            TypedValue::Shadow(Shadow::new(Vec2::ZERO, BlurRadius::ZERO, transparent?))
        }
        ExpressionType::Typography => TypedValue::Typography(TypographyValue::default()),
        ExpressionType::ResourceRef => TypedValue::ResourceRef(ResourceRef::new(StableId::NIL)),
    })
}

/// 타입의 안전값 literal 표현식. `Optional`이면 null literal입니다. node ID는 `node`입니다.
pub fn safe_literal(spec: TypeSpec, node: ExprNodeId) -> Option<ExpressionAst> {
    let literal = if spec.is_optional() {
        ExprNode::null(node, spec.value_type())
    } else {
        ExprNode::literal(node, safe_value(spec.value_type())?)
    };
    ExpressionAst::new(node, vec![literal]).ok()
}

/// 저장된 선언 기본값(프로젝트 파일에 기록된 그대로).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredDefault {
    /// 속성의 선언 타입.
    pub declared_type: TypeSpec,
    /// 기본값 표현식의 저장된 canonical AST bytes(VIII.2.5).
    pub expression: Vec<u8>,
}

/// target 속성 → 저장된 선언 기본값 제공자.
pub trait StoredDefaults {
    /// 저장된 선언 기본값. 없으면 `None`.
    fn stored_default(&self, target: &PropertyPath) -> Option<&StoredDefault>;
}

impl StoredDefaults for BTreeMap<PropertyPath, StoredDefault> {
    fn stored_default(&self, target: &PropertyPath) -> Option<&StoredDefault> {
        self.get(target)
    }
}

/// 기본값 손상 종류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefaultCorruption {
    /// 선언 기본값이 없습니다.
    Missing,
    /// 저장된 bytes를 표현식으로 디코드할 수 없습니다.
    Malformed(CodecError),
    /// 기본값 표현식이 선언 타입과 맞지 않습니다(또는 type 오류).
    InvalidType(BindingTypeError),
}

impl fmt::Display for DefaultCorruption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DefaultCorruption::Missing => f.write_str("선언 기본값이 없습니다"),
            DefaultCorruption::Malformed(error) => write!(f, "기본값 bytes 손상: {error}"),
            DefaultCorruption::InvalidType(error) => write!(f, "기본값 타입 오류: {error}"),
        }
    }
}

/// 기본값 손상 진단(VIII.8.3). 대체한 안전값을 함께 기록합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorruptionDiagnostic {
    /// orphan binding.
    pub binding: BindingId,
    /// target 속성.
    pub target: PropertyPath,
    /// 손상 종류.
    pub corruption: DefaultCorruption,
    /// 안전값을 만든 타입.
    pub fallback_type: TypeSpec,
    /// 대체한 안전값(`None` = null).
    pub fallback: Option<TypedValue>,
}

impl fmt::Display for CorruptionDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "binding {}: {} — {} 안전값 사용",
            self.binding, self.corruption, self.fallback_type
        )
    }
}

/// 저장된 기본값을 해석한 결과: 쓸 수 있는 선언 기본값(손상은 안전값 literal로 대체)과 진단.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedDefaults {
    defaults: BTreeMap<PropertyPath, DeclaredDefault>,
    diagnostics: Vec<CorruptionDiagnostic>,
}

impl ResolvedDefaults {
    /// 손상 진단(binding ID 순).
    pub fn diagnostics(&self) -> &[CorruptionDiagnostic] {
        &self.diagnostics
    }
}

impl DeclaredDefaults for ResolvedDefaults {
    fn declared_default(&self, target: &PropertyPath) -> Option<DeclaredDefault> {
        self.defaults.get(target).cloned()
    }
}

/// live binding이 없는 orphan target의 저장된 기본값을 해석합니다(orphan ID 순). 입력은 읽기만 합니다.
pub fn resolve_stored_defaults(
    store: &BindingStore,
    stored: &dyn StoredDefaults,
    references: &dyn ReferenceTypes,
) -> ResolvedDefaults {
    let mut resolved = ResolvedDefaults::default();
    for orphan in store.orphans() {
        if store.live().by_target(orphan.target()).is_some() {
            continue;
        }
        let (declared_type, outcome) = match stored.stored_default(orphan.target()) {
            None => (orphan.last_valid_type(), Err(DefaultCorruption::Missing)),
            Some(default) => (
                default.declared_type,
                ExpressionAst::from_canonical_bytes(&default.expression)
                    .map_err(DefaultCorruption::Malformed)
                    .and_then(|ast| {
                        check_source(default.declared_type, &ast, references)
                            .map(|_| ast)
                            .map_err(DefaultCorruption::InvalidType)
                    }),
            ),
        };
        let expression = match outcome {
            Ok(ast) => ast,
            Err(corruption) => {
                let node = ExprNodeId::new(orphan.id().stable_id());
                let Some(literal) = safe_literal(declared_type, node) else {
                    continue;
                };
                let fallback = if declared_type.is_optional() {
                    None
                } else {
                    safe_value(declared_type.value_type())
                };
                resolved.diagnostics.push(CorruptionDiagnostic {
                    binding: orphan.id(),
                    target: orphan.target().clone(),
                    corruption,
                    fallback_type: declared_type,
                    fallback,
                });
                literal
            }
        };
        resolved.defaults.insert(
            orphan.target().clone(),
            DeclaredDefault {
                declared_type,
                expression,
            },
        );
    }
    resolved
}

/// 저장소 기준 참조 타입: 토큰 = registry, 속성 = live binding 기대 타입 → orphan 마지막 유효 타입 → source,
/// 그 밖 = source.
struct StoreTypes<'a> {
    tokens: &'a TokenRegistry,
    store: &'a BindingStore,
    sources: &'a dyn SourceInputs,
}

impl ReferenceTypes for StoreTypes<'_> {
    fn reference_type(&self, target: &ReferenceTarget) -> Option<TypeSpec> {
        let source = || {
            self.sources
                .source_type(&DependencyNode::from_target(target))
        };
        match target {
            ReferenceTarget::Token(token) => self
                .tokens
                .token_type_of(*token)
                .map(|t| t.type_spec())
                .or_else(source),
            ReferenceTarget::Property(path) => {
                if let Some(binding) = self.store.live().by_target(path) {
                    return Some(binding.expected_type());
                }
                self.store
                    .orphans()
                    .find(|orphan| orphan.target() == path)
                    .map(OrphanedBinding::last_valid_type)
                    .or_else(source)
            }
            _ => source(),
        }
    }
}

impl BindingEvaluator {
    /// 저장된 기본값으로 orphan을 포함해 엽니다(VIII.8.3). 손상된 기본값은 안전값과 진단으로 대체하며 열기는
    /// 실패하지 않습니다(cycle 등 graph 오류 제외). 입력 저장소·기본값은 수정하지 않습니다.
    pub fn open_with_stored_defaults(
        tokens: &TokenRegistry,
        store: &BindingStore,
        stored: &dyn StoredDefaults,
        sources: &dyn SourceInputs,
    ) -> Result<(BindingEvaluator, Vec<CorruptionDiagnostic>), SchedulerError> {
        let types = StoreTypes {
            tokens,
            store,
            sources,
        };
        let resolved = resolve_stored_defaults(store, stored, &types);
        let evaluator = BindingEvaluator::with_orphans(tokens, store, &resolved, sources)?;
        Ok((evaluator, resolved.diagnostics))
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

    #[test]
    fn safe_values_cover_every_type() {
        let types = [
            ExpressionType::Boolean,
            ExpressionType::Number,
            ExpressionType::String,
            ExpressionType::Enum(stellar_expressions::types::EnumTypeId::new(77)),
            ExpressionType::Length,
            ExpressionType::Percentage,
            ExpressionType::Angle,
            ExpressionType::Duration,
            ExpressionType::Color,
            ExpressionType::Gradient,
            ExpressionType::Shadow,
            ExpressionType::Typography,
            ExpressionType::ResourceRef,
        ];
        assert_eq!(types.len(), ExpressionType::VARIANT_COUNT);
        for value_type in types {
            let value = safe_value(value_type).expect("안전값");
            assert!(value_type.matches(&value), "{value_type}");
            let required = safe_literal(TypeSpec::required(value_type), x(1)).expect("literal");
            let optional = safe_literal(TypeSpec::optional(value_type), x(1)).expect("literal");
            assert!(matches!(
                required.node(x(1)).map(|n| n.payload()),
                Some(stellar_expressions::ast::NodePayload::Literal(
                    stellar_expressions::ast::Literal::Value(_)
                ))
            ));
            assert!(matches!(
                optional.node(x(1)).map(|n| n.payload()),
                Some(stellar_expressions::ast::NodePayload::Literal(
                    stellar_expressions::ast::Literal::Null(_)
                ))
            ));
        }
    }

    #[test]
    fn malformed_default_fixture_opens_with_safe_values_and_diagnostics() {
        let sources = StaticSources::new();
        let tokens = registry(true);
        // orphan 5개(o1~o5)를 만들기 위해 없는 토큰(99)을 참조하는 binding을 만듭니다.
        let mut live = BindingSet::new();
        for n in 1..=5u64 {
            live.insert(bind(
                10 + n,
                10 + n,
                apply(
                    1_000 + n * 10,
                    BinaryOp::Add,
                    ReferenceTarget::Token(token(99)),
                    px(1.0),
                ),
            ))
            .expect("binding");
        }
        let mut store = BindingStore::new(live);
        assert_eq!(store.orphan_broken(&World(&tokens)).len(), 5);
        let valid = literal(500, px(4.0)).to_canonical_bytes().expect("직렬화");
        let mut truncated = valid.clone();
        truncated.truncate(valid.len() - 3);
        let wrong_type = literal(
            510,
            TypedValue::Color(Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 1.0).expect("유효")),
        )
        .to_canonical_bytes()
        .expect("직렬화");
        let mut flipped = valid.clone();
        flipped[0] ^= 0xFF; // version 손상.
        let stored: BTreeMap<PropertyPath, StoredDefault> = BTreeMap::from([
            (
                prop(11),
                StoredDefault {
                    declared_type: length(),
                    expression: valid.clone(),
                },
            ),
            // prop(12): 기본값 없음.
            (
                prop(13),
                StoredDefault {
                    declared_type: length(),
                    expression: truncated,
                },
            ),
            (
                prop(14),
                StoredDefault {
                    declared_type: length(),
                    expression: wrong_type,
                },
            ),
            (
                prop(15),
                StoredDefault {
                    declared_type: TypeSpec::optional(ExpressionType::Color),
                    expression: flipped,
                },
            ),
        ]);
        let before = stored.clone();
        let store_before = store.clone();
        let (evaluator, diagnostics) =
            BindingEvaluator::open_with_stored_defaults(&tokens, &store, &stored, &sources)
                .expect("손상 fixture도 열림");
        // 파일(입력)은 수정하지 않습니다.
        assert_eq!(stored, before);
        assert_eq!(store, store_before);
        // 값: 정상 기본값 4px, 나머지는 타입별 안전값.
        assert_eq!(value(&evaluator, 11), Ok(Some(px(4.0))));
        assert_eq!(value(&evaluator, 12), Ok(Some(px(0.0))));
        assert_eq!(value(&evaluator, 13), Ok(Some(px(0.0))));
        assert_eq!(value(&evaluator, 14), Ok(Some(px(0.0))));
        assert_eq!(value(&evaluator, 15), Ok(None), "Optional은 null");
        // 오류 표시: 손상 4건, binding ID 순, 원인 정확.
        let summary: Vec<(u64, &str)> = diagnostics
            .iter()
            .map(|d| {
                let kind = match d.corruption {
                    DefaultCorruption::Missing => "missing",
                    DefaultCorruption::Malformed(_) => "malformed",
                    DefaultCorruption::InvalidType(_) => "invalid-type",
                };
                (d.target.root().timestamp_ms() - 2_000, kind)
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (12, "missing"),
                (13, "malformed"),
                (14, "invalid-type"),
                (15, "malformed")
            ]
        );
        assert_eq!(diagnostics[0].fallback, Some(px(0.0)));
        assert_eq!(diagnostics[3].fallback, None);
        assert_eq!(
            diagnostics[3].fallback_type,
            TypeSpec::optional(ExpressionType::Color)
        );
        assert!(diagnostics[2].to_string().contains("안전값"));
        // 다시 열어도 같은 값·같은 진단(결정적).
        let (again, again_diagnostics) =
            BindingEvaluator::open_with_stored_defaults(&tokens, &store, &stored, &sources)
                .expect("재열기");
        assert_eq!(again_diagnostics, diagnostics);
        for n in 11..=15 {
            assert_eq!(value(&again, n), value(&evaluator, n));
        }
    }
}
