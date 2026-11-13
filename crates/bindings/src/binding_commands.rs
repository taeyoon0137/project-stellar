//! binding 생성·교체·삭제 command (VIII.6.6).
//!
//! [`BindingCommand`]는 닫힌 enum이며 `validate`·`apply`를 제공합니다. `apply`는 먼저 검증하고 성공하면 inverse
//! command를 돌려줍니다. 검증 실패 시 binding 모음은 바뀌지 않습니다.
//!
//! 검증 규칙:
//!
//! - **type**: source 결과 타입이 target 기대 타입에 대입 가능해야 합니다(VIII.6.1 [`assignable`](crate::binding::assignable)).
//!   type mismatch는 적용 전 `validate`에서 거부합니다.
//! - **구조**: target 속성 하나에 binding 하나(VIII.6.2 [`BindingSet`]), 교체는 같은 target·owner 유지.
//! - **cycle**: 새 source가 통합 graph에 cycle을 만들면 거부합니다(VIII.6.3).
//!
//! 참조 타입은 토큰은 registry, binding이 있는 속성은 그 binding의 기대 타입, 나머지(binding 없는 속성·Component
//! Property·환경 입력)는 호출자 제공자([`BindingContext::others`])로 해석합니다.
//!
//! 토큰 command(VIII.5.5)와 같이 Phase VII command 기반이 생기기 전의 binding 전용 형태입니다.

use std::collections::BTreeSet;
use std::fmt;

use stellar_expressions::ast::{ExpressionAst, ReferenceTarget};
use stellar_expressions::typecheck::ReferenceTypes;
use stellar_expressions::types::TypeSpec;

use crate::binding::{
    check_source, BindingId, BindingSet, BindingSetError, BindingTypeError, ValueBinding,
};
use crate::graph::{BindingCycle, DependencyGraph, DependencyNode};
use crate::registry::TokenRegistry;
use crate::token::TokenTypes;

/// binding command 검증 입력.
pub struct BindingContext<'a> {
    /// 토큰 registry(토큰 참조 타입·graph).
    pub tokens: &'a TokenRegistry,
    /// 토큰·binding 있는 속성 밖의 참조 타입.
    pub others: &'a dyn ReferenceTypes,
}

/// 토큰·binding·호출자 제공자를 합친 참조 타입 view.
struct ContextTypes<'a> {
    context: &'a BindingContext<'a>,
    bindings: &'a BindingSet,
}

impl ReferenceTypes for ContextTypes<'_> {
    fn reference_type(&self, target: &ReferenceTarget) -> Option<TypeSpec> {
        match target {
            ReferenceTarget::Token(token) => self
                .context
                .tokens
                .token_type_of(*token)
                .map(|t| t.type_spec()),
            ReferenceTarget::Property(path) => match self.bindings.by_target(path) {
                Some(binding) => Some(binding.expected_type()),
                None => self.context.others.reference_type(target),
            },
            other => self.context.others.reference_type(other),
        }
    }
}

/// binding command (VIII.6.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingCommand {
    /// binding을 만듭니다.
    Create(ValueBinding),
    /// binding의 source와 기대 타입을 바꿉니다(target·owner 유지).
    Replace {
        /// 대상 binding.
        binding: BindingId,
        /// 새 source.
        source: ExpressionAst,
        /// 새 기대 타입.
        expected_type: TypeSpec,
    },
    /// binding을 삭제합니다.
    Delete(BindingId),
}

/// binding command 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingCommandError {
    /// ID·target 구조 오류.
    Set(BindingSetError),
    /// type mismatch 또는 source 표현식 type 오류.
    Type(BindingTypeError),
    /// 새 source가 cycle을 만듭니다.
    Cycle(BindingCycle),
}

impl fmt::Display for BindingCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BindingCommandError::Set(error) => write!(f, "{error}"),
            BindingCommandError::Type(error) => write!(f, "{error}"),
            BindingCommandError::Cycle(cycle) => write!(f, "{cycle}"),
        }
    }
}

impl std::error::Error for BindingCommandError {}

impl From<BindingSetError> for BindingCommandError {
    fn from(error: BindingSetError) -> BindingCommandError {
        BindingCommandError::Set(error)
    }
}

/// `binding`(적용 후 모습)을 현재 모음에서 검사합니다: cycle(자기 참조 포함) → type.
fn check_binding(
    binding: &ValueBinding,
    bindings: &BindingSet,
    context: &BindingContext<'_>,
) -> Result<(), BindingCommandError> {
    let node = DependencyNode::Property(binding.target().clone());
    let dependencies: BTreeSet<DependencyNode> = binding
        .references()
        .iter()
        .map(DependencyNode::from_target)
        .collect();
    if let Some(cycle) =
        DependencyGraph::build(context.tokens, bindings).cycle_if_dependencies(&node, &dependencies)
    {
        return Err(BindingCommandError::Cycle(cycle));
    }
    let types = ContextTypes { context, bindings };
    check_source(binding.expected_type(), binding.source(), &types)
        .map_err(BindingCommandError::Type)?;
    Ok(())
}

impl BindingCommand {
    /// 모음을 바꾸지 않고 검증합니다.
    pub fn validate(
        &self,
        bindings: &BindingSet,
        context: &BindingContext<'_>,
    ) -> Result<(), BindingCommandError> {
        match self {
            BindingCommand::Create(binding) => {
                if bindings.get(binding.id()).is_some() {
                    return Err(BindingSetError::DuplicateId(binding.id()).into());
                }
                if let Some(existing) = bindings.by_target(binding.target()) {
                    return Err(BindingSetError::DuplicateTarget {
                        target: binding.target().clone(),
                        existing: existing.id(),
                    }
                    .into());
                }
                check_binding(binding, bindings, context)
            }
            BindingCommand::Replace {
                binding,
                source,
                expected_type,
            } => {
                let current = bindings
                    .get(*binding)
                    .ok_or(BindingSetError::UnknownBinding(*binding))?;
                let next = current.with_source(source.clone(), *expected_type);
                check_binding(&next, bindings, context)
            }
            BindingCommand::Delete(binding) => {
                bindings
                    .get(*binding)
                    .ok_or(BindingSetError::UnknownBinding(*binding))?;
                Ok(())
            }
        }
    }

    /// 검증 후 적용하고 inverse command를 돌려줍니다.
    pub fn apply(
        self,
        bindings: &mut BindingSet,
        context: &BindingContext<'_>,
    ) -> Result<BindingCommand, BindingCommandError> {
        self.validate(bindings, context)?;
        match self {
            BindingCommand::Create(binding) => {
                let id = binding.id();
                bindings.insert(binding)?;
                Ok(BindingCommand::Delete(id))
            }
            BindingCommand::Replace {
                binding,
                source,
                expected_type,
            } => {
                let current = bindings
                    .get(binding)
                    .ok_or(BindingSetError::UnknownBinding(binding))?;
                let next = current.with_source(source, expected_type);
                let previous = bindings.replace(next)?;
                Ok(BindingCommand::Replace {
                    binding,
                    source: previous.source().clone(),
                    expected_type: previous.expected_type(),
                })
            }
            BindingCommand::Delete(binding) => {
                let removed = bindings
                    .remove(binding)
                    .ok_or(BindingSetError::UnknownBinding(binding))?;
                Ok(BindingCommand::Create(removed))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::ids::StableId;
    use stellar_core_types::property::{PropertyKey, PropertyPath};
    use stellar_core_types::reference::{NodeRef, TokenRef};
    use stellar_core_types::typed_value::TypedValue;
    use stellar_expressions::ast::{ExprNode, ExprNodeId};
    use stellar_expressions::types::ExpressionType;

    use super::*;
    use crate::binding::ScopeOwner;
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

    fn x(n: u64) -> ExprNodeId {
        ExprNodeId::new(sid(n))
    }

    fn color_type() -> TypeSpec {
        TypeSpec::required(ExpressionType::Color)
    }

    fn reference(base: u64, target: ReferenceTarget) -> ExpressionAst {
        ExpressionAst::new(x(base), vec![ExprNode::reference(x(base), target)]).expect("유효")
    }

    fn literal(base: u64, value: TypedValue) -> ExpressionAst {
        ExpressionAst::new(x(base), vec![ExprNode::literal(x(base), value)]).expect("유효")
    }

    fn binding(n: u64, target: u64, source: ExpressionAst, expected: TypeSpec) -> ValueBinding {
        ValueBinding::new(
            BindingId::new(sid(4_000 + n)),
            prop(target),
            source,
            expected,
            ScopeOwner::Node(NodeRef::new(sid(5_000 + n))),
        )
    }

    fn registry() -> TokenRegistry {
        let mut set = TokenSet::new(TokenSetId::new(sid(1)));
        set.insert(TokenDefinition::new(
            token(1),
            TokenName::new("brand").expect("이름"),
            TokenType::new(ExpressionType::Color),
            literal(
                10,
                TypedValue::Color(Color::new(ColorSpace::Srgb, 0.2, 0.4, 0.6, 1.0).expect("유효")),
            ),
        ))
        .expect("t1");
        let mut registry = TokenRegistry::new();
        registry.add_scope(TokenScope::Project, set).expect("scope");
        registry
    }

    /// p1 ← brand(토큰). p9는 binding 없는 Number 속성(source).
    fn fixture() -> BindingSet {
        let mut set = BindingSet::new();
        set.insert(binding(
            1,
            1,
            reference(100, ReferenceTarget::Token(token(1))),
            color_type(),
        ))
        .expect("b1");
        set
    }

    fn others() -> BTreeMap<ReferenceTarget, TypeSpec> {
        BTreeMap::from([(
            ReferenceTarget::Property(prop(9)),
            TypeSpec::required(ExpressionType::Number),
        )])
    }

    fn round_trip(set: &mut BindingSet, context: &BindingContext<'_>, command: BindingCommand) {
        let original = set.clone();
        let inverse = command.apply(set, context).expect("적용");
        let applied = set.clone();
        assert_ne!(&applied, &original);
        let redo = inverse.apply(set, context).expect("inverse");
        assert_eq!(set, &original, "inverse로 원래 상태");
        redo.apply(set, context).expect("redo");
        assert_eq!(set, &applied, "redo로 forward 상태");
    }

    #[test]
    fn create_replace_delete_round_trip() {
        let tokens = registry();
        let others = others();
        let context = BindingContext {
            tokens: &tokens,
            others: &others,
        };
        let mut set = fixture();
        // p2 ← p1(binding 있는 속성은 그 기대 타입으로 type 검사).
        round_trip(
            &mut set,
            &context,
            BindingCommand::Create(binding(
                2,
                2,
                reference(200, ReferenceTarget::Property(prop(1))),
                color_type(),
            )),
        );
        // p1을 nullable 속성으로 바꾸며 source 교체.
        round_trip(
            &mut set,
            &context,
            BindingCommand::Replace {
                binding: BindingId::new(sid(4_001)),
                source: literal(
                    300,
                    TypedValue::Color(
                        Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 1.0).expect("유효"),
                    ),
                ),
                expected_type: TypeSpec::optional(ExpressionType::Color),
            },
        );
        round_trip(
            &mut set,
            &context,
            BindingCommand::Delete(BindingId::new(sid(4_001))),
        );
    }

    #[test]
    fn type_mismatch_is_rejected_in_validate() {
        let tokens = registry();
        let others = others();
        let context = BindingContext {
            tokens: &tokens,
            others: &others,
        };
        let mut set = fixture();
        let original = set.clone();
        // Color 속성에 Number source(p9).
        let wrong = BindingCommand::Create(binding(
            3,
            3,
            reference(400, ReferenceTarget::Property(prop(9))),
            color_type(),
        ));
        let mismatch = BindingCommandError::Type(BindingTypeError::Mismatch {
            expected: color_type(),
            actual: TypeSpec::required(ExpressionType::Number),
        });
        assert_eq!(wrong.validate(&set, &context), Err(mismatch.clone()));
        assert_eq!(wrong.apply(&mut set, &context), Err(mismatch));
        assert_eq!(set, original, "실패 시 불변");
        // 교체도 validate에서 거부: Required 속성에 nullable source.
        let nullable_source =
            ExpressionAst::new(x(410), vec![ExprNode::null(x(410), ExpressionType::Color)])
                .expect("유효");
        let replace = BindingCommand::Replace {
            binding: BindingId::new(sid(4_001)),
            source: nullable_source,
            expected_type: color_type(),
        };
        assert!(matches!(
            replace.validate(&set, &context),
            Err(BindingCommandError::Type(BindingTypeError::Mismatch { .. }))
        ));
        assert!(replace.apply(&mut set, &context).is_err());
        assert_eq!(set, original);
        // 모르는 참조는 표현식 type 오류.
        assert!(matches!(
            BindingCommand::Create(binding(
                4,
                4,
                reference(420, ReferenceTarget::ComponentProperty(sid(3_000))),
                color_type(),
            ))
            .validate(&set, &context),
            Err(BindingCommandError::Type(BindingTypeError::Expression(_)))
        ));
    }

    #[test]
    fn structure_and_cycle_violations_are_rejected() {
        let tokens = registry();
        let others = others();
        let context = BindingContext {
            tokens: &tokens,
            others: &others,
        };
        let mut set = fixture();
        set.insert(binding(
            2,
            2,
            reference(500, ReferenceTarget::Property(prop(1))),
            color_type(),
        ))
        .expect("b2");
        let original = set.clone();
        // p1이 p2를 읽게 바꾸면 p1 → p2 → p1.
        assert_eq!(
            BindingCommand::Replace {
                binding: BindingId::new(sid(4_001)),
                source: reference(510, ReferenceTarget::Property(prop(2))),
                expected_type: color_type(),
            }
            .apply(&mut set, &context),
            Err(BindingCommandError::Cycle(BindingCycle {
                path: vec![
                    DependencyNode::Property(prop(1)),
                    DependencyNode::Property(prop(2)),
                    DependencyNode::Property(prop(1)),
                ]
            }))
        );
        // 자기 참조.
        assert!(matches!(
            BindingCommand::Create(binding(
                5,
                5,
                reference(520, ReferenceTarget::Property(prop(5))),
                color_type(),
            ))
            .apply(&mut set, &context),
            Err(BindingCommandError::Cycle(_))
        ));
        // 같은 target·같은 ID·없는 binding.
        assert!(matches!(
            BindingCommand::Create(binding(
                6,
                1,
                reference(530, ReferenceTarget::Token(token(1))),
                color_type(),
            ))
            .apply(&mut set, &context),
            Err(BindingCommandError::Set(
                BindingSetError::DuplicateTarget { .. }
            ))
        ));
        assert!(matches!(
            BindingCommand::Create(binding(
                1,
                7,
                reference(540, ReferenceTarget::Token(token(1))),
                color_type(),
            ))
            .apply(&mut set, &context),
            Err(BindingCommandError::Set(BindingSetError::DuplicateId(_)))
        ));
        assert_eq!(
            BindingCommand::Delete(BindingId::new(sid(4_099))).apply(&mut set, &context),
            Err(BindingCommandError::Set(BindingSetError::UnknownBinding(
                BindingId::new(sid(4_099))
            )))
        );
        assert_eq!(set, original, "모든 실패에서 불변");
    }
}
