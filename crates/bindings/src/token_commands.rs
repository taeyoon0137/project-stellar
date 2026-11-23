//! 토큰 생성·삭제·rename·expression 변경 command (VIII.5.5).
//!
//! [`TokenCommand`]는 직렬화 가능한 닫힌 enum이며 `validate`·`apply`를 제공합니다. `apply`는 먼저 검증하고, 성공하면
//! 적용 직후 상태를 되돌리는 **inverse command**를 돌려줍니다(`apply(forward)` 후 `apply(inverse)`는 원래 registry와
//! 같은 상태). 검증에 실패하면 registry를 바꾸지 않습니다.
//!
//! **참조 정책**
//!
//! - 새 default expression(생성·expression 변경)은 type 검사를 통과하고, 참조하는 토큰이 모두 존재하며 그 scope에서
//!   보여야 하고(VIII.5.2), cycle을 만들지 않아야 합니다(VIII.5.3). 삭제된(orphan) 토큰을 새로 참조할 수 없습니다.
//! - inbound 참조가 있는 토큰의 삭제는 **orphan plan** 없이는 거부합니다. inbound 참조는 그 토큰을 default
//!   expression에서 읽는 다른 토큰과, source에서 읽는 binding(live·orphan 모두, [`BindingStore`])입니다. plan은 현재
//!   inbound 참조 토큰·binding 목록과 정확히 같아야 합니다(오래된 plan 거부, [`TokenOrphanPlan::current`]). plan으로
//!   삭제하면 참조 표현식과 binding source는 바꾸지 않고, 삭제된 토큰의 ID·scope·이름·마지막 타입을
//!   [`TokenTombstone`]으로 보존합니다(§14.4, 자동 삭제 금지). 그 토큰을 읽던 live binding은 binding validator(VIII.8.1
//!   `orphan_broken`)가 orphan으로 옮깁니다.
//! - 라이브러리 scope 토큰은 사용 프로젝트에서 읽기 전용입니다(외부 라이브러리 수정 정책).
//!
//! 이 command는 Phase VII command 기반(CommandId·origin·base revision·transaction)이 생기기 전의 토큰 전용 형태입니다.
//! Phase VII가 구현되면 그 command enum의 variant로 옮깁니다.

use std::collections::BTreeSet;
use std::fmt;

use stellar_core_types::reference::TokenRef;
use stellar_expressions::ast::ExpressionAst;
use stellar_expressions::typecheck::ReferenceTypes;

use crate::binding::BindingId;
use crate::graph_view::TokenGraphView;
use crate::orphan::BindingStore;
use crate::registry::{ReferenceIssue, RegistryError, TokenRegistry, TokenScope, TokenTombstone};
use crate::token::{
    check_expression, TokenDefinition, TokenName, TokenReferenceTypes, TokenSetError, TokenType,
    TokenTypeError,
};
use crate::token_graph::TokenCycle;

/// inbound 참조가 있는 토큰 삭제를 허용하는 명시 계획. 참조를 끊지 않고 orphan 상태로 둡니다.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TokenOrphanPlan {
    /// 삭제로 orphan 참조가 될 토큰(현재 inbound 참조와 정확히 같아야 함).
    pub referrers: BTreeSet<TokenRef>,
    /// 삭제로 orphan 참조가 될 binding(live·orphan, 현재 inbound binding 참조와 정확히 같아야 함).
    pub bindings: BTreeSet<BindingId>,
}

impl TokenOrphanPlan {
    /// `token`의 현재 inbound 참조(토큰 default expression과 live·orphan binding source)로 만든 plan.
    pub fn current(
        registry: &TokenRegistry,
        bindings: &BindingStore,
        token: TokenRef,
    ) -> TokenOrphanPlan {
        // binding 쪽은 저장소의 토큰 역참조 index로 그 토큰을 읽는 binding만 봅니다(XVI.4.8).
        TokenOrphanPlan {
            referrers: registry.referrers(token),
            bindings: bindings.bindings_reading(token),
        }
    }

    /// inbound 참조가 하나도 없으면 `true`.
    pub fn is_empty(&self) -> bool {
        self.referrers.is_empty() && self.bindings.is_empty()
    }
}

/// 토큰 command (VIII.5.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenCommand {
    /// scope에 토큰을 만듭니다.
    Create {
        /// 대상 scope.
        scope: TokenScope,
        /// 정의.
        definition: TokenDefinition,
    },
    /// 토큰을 삭제합니다. inbound 참조가 있으면 `orphan_plan`이 필요합니다. plan이 있으면 orphan 기록을 남깁니다.
    Delete {
        /// 대상 토큰.
        token: TokenRef,
        /// orphan plan.
        orphan_plan: Option<TokenOrphanPlan>,
    },
    /// 이름을 바꿉니다(참조는 ID라 그대로).
    Rename {
        /// 대상 토큰.
        token: TokenRef,
        /// 새 이름.
        name: TokenName,
    },
    /// default expression을 바꿉니다.
    SetExpression {
        /// 대상 토큰.
        token: TokenRef,
        /// 새 default expression.
        expression: ExpressionAst,
    },
}

/// command 검증·적용 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenCommandError {
    /// scope·ID·이름 오류(registry 규칙).
    Registry(RegistryError),
    /// 라이브러리 scope는 읽기 전용입니다.
    ReadOnlyScope(TokenScope),
    /// default expression type 오류.
    Type(TokenTypeError),
    /// 새 표현식의 토큰 참조가 없거나(`Dangling`), 보이지 않거나(`NotVisible`), 삭제된 토큰(`Orphaned`)입니다.
    InvalidReference(ReferenceIssue),
    /// 표현식이 토큰 cycle을 만듭니다.
    Cycle(TokenCycle),
    /// inbound 참조가 있는데 orphan plan이 없습니다.
    InboundReferences {
        /// 삭제하려는 토큰.
        token: TokenRef,
        /// 현재 inbound 참조(토큰·binding).
        inbound: TokenOrphanPlan,
    },
    /// orphan plan의 참조 목록이 현재 inbound 참조와 다릅니다.
    StaleOrphanPlan {
        /// 삭제하려는 토큰.
        token: TokenRef,
        /// 현재 inbound 참조.
        actual: TokenOrphanPlan,
        /// plan이 가정한 참조.
        planned: TokenOrphanPlan,
    },
    /// orphan 기록이 있는 ID를 다른 scope·이름·타입으로 다시 만들려고 합니다.
    TombstoneMismatch(TokenTombstone),
}

impl fmt::Display for TokenCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenCommandError::Registry(error) => write!(f, "{error}"),
            TokenCommandError::ReadOnlyScope(scope) => write!(f, "{scope}는 읽기 전용입니다"),
            TokenCommandError::Type(error) => write!(f, "{error}"),
            TokenCommandError::InvalidReference(issue) => write!(f, "잘못된 토큰 참조: {issue:?}"),
            TokenCommandError::Cycle(cycle) => write!(f, "{cycle}"),
            TokenCommandError::InboundReferences { token, inbound } => write!(
                f,
                "토큰 {}를 토큰 {}개·binding {}개가 참조해 orphan plan 없이 삭제할 수 없습니다",
                token.id(),
                inbound.referrers.len(),
                inbound.bindings.len()
            ),
            TokenCommandError::StaleOrphanPlan { token, .. } => {
                write!(
                    f,
                    "토큰 {}의 orphan plan이 현재 참조와 다릅니다",
                    token.id()
                )
            }
            TokenCommandError::TombstoneMismatch(tombstone) => write!(
                f,
                "삭제된 토큰 {}는 같은 scope·이름·타입으로만 다시 만들 수 있습니다",
                tombstone.token.id()
            ),
        }
    }
}

impl std::error::Error for TokenCommandError {}

impl From<RegistryError> for TokenCommandError {
    fn from(error: RegistryError) -> TokenCommandError {
        TokenCommandError::Registry(error)
    }
}

fn unknown(token: TokenRef) -> TokenCommandError {
    TokenCommandError::Registry(RegistryError::Set(TokenSetError::UnknownToken(token)))
}

/// 편집 가능한 토큰의 scope(라이브러리 거부).
fn writable_scope(
    registry: &TokenRegistry,
    token: TokenRef,
) -> Result<TokenScope, TokenCommandError> {
    let scope = registry.scope_of(token).ok_or_else(|| unknown(token))?;
    if matches!(scope, TokenScope::Library(_)) {
        return Err(TokenCommandError::ReadOnlyScope(scope));
    }
    Ok(scope)
}

/// `token`(scope `scope`)의 새 default expression을 검사합니다: 참조 존재·가시성, cycle, type.
fn check_new_expression(
    registry: &TokenRegistry,
    scope: TokenScope,
    token: TokenRef,
    token_type: TokenType,
    expression: &ExpressionAst,
    others: &dyn ReferenceTypes,
) -> Result<(), TokenCommandError> {
    let references: BTreeSet<TokenRef> = expression
        .referenced_targets()
        .into_iter()
        .filter_map(|target| match target {
            stellar_expressions::ast::ReferenceTarget::Token(t) => Some(*t),
            _ => None,
        })
        .collect();
    if references.contains(&token) {
        return Err(TokenCommandError::Cycle(TokenCycle {
            path: vec![token, token],
        }));
    }
    for &target in &references {
        match registry.scope_of(target) {
            Some(target_scope) if registry.is_visible(scope, target_scope) => {}
            Some(target_scope) => {
                return Err(TokenCommandError::InvalidReference(
                    ReferenceIssue::NotVisible {
                        token,
                        from: scope,
                        target,
                        target_scope,
                    },
                ))
            }
            None if registry.tombstone(target).is_some() => {
                return Err(TokenCommandError::InvalidReference(
                    ReferenceIssue::Orphaned { token, target },
                ))
            }
            None => {
                return Err(TokenCommandError::InvalidReference(
                    ReferenceIssue::Dangling { token, target },
                ))
            }
        }
    }
    if let Some(cycle) = TokenGraphView::new(registry).cycle_if_references(token, &references) {
        return Err(TokenCommandError::Cycle(cycle));
    }
    let types = TokenReferenceTypes {
        tokens: registry,
        others,
    };
    check_expression(token_type, expression, &types).map_err(TokenCommandError::Type)
}

impl TokenCommand {
    /// registry를 바꾸지 않고 command를 검증합니다. `bindings`는 토큰을 읽는 binding(삭제 inbound 참조 검사),
    /// `others`는 토큰이 아닌 참조의 타입 제공자입니다.
    pub fn validate(
        &self,
        registry: &TokenRegistry,
        bindings: &BindingStore,
        others: &dyn ReferenceTypes,
    ) -> Result<(), TokenCommandError> {
        match self {
            TokenCommand::Create { scope, definition } => {
                let id = definition.id();
                if matches!(scope, TokenScope::Library(_)) {
                    return Err(TokenCommandError::ReadOnlyScope(*scope));
                }
                let set = registry
                    .set(*scope)
                    .ok_or(RegistryError::UnknownScope(*scope))?;
                if let Some(existing) = registry.scope_of(id) {
                    return Err(RegistryError::IdInOtherScope {
                        token: id,
                        existing,
                    }
                    .into());
                }
                if let Some(existing) = set.by_name(definition.name().as_str()) {
                    return Err(RegistryError::Set(TokenSetError::DuplicateName {
                        name: definition.name().clone(),
                        existing: existing.id(),
                    })
                    .into());
                }
                if let Some(tombstone) = registry.tombstone(id) {
                    if tombstone.scope != *scope
                        || &tombstone.name != definition.name()
                        || tombstone.token_type != definition.token_type()
                    {
                        return Err(TokenCommandError::TombstoneMismatch(tombstone.clone()));
                    }
                }
                check_new_expression(
                    registry,
                    *scope,
                    id,
                    definition.token_type(),
                    definition.default_expression(),
                    others,
                )
            }
            TokenCommand::Delete { token, orphan_plan } => {
                writable_scope(registry, *token)?;
                let inbound = TokenOrphanPlan::current(registry, bindings, *token);
                match orphan_plan {
                    None if !inbound.is_empty() => Err(TokenCommandError::InboundReferences {
                        token: *token,
                        inbound,
                    }),
                    Some(plan) if *plan != inbound => Err(TokenCommandError::StaleOrphanPlan {
                        token: *token,
                        actual: inbound,
                        planned: plan.clone(),
                    }),
                    _ => Ok(()),
                }
            }
            TokenCommand::Rename { token, name } => {
                let scope = writable_scope(registry, *token)?;
                match registry
                    .set(scope)
                    .and_then(|set| set.by_name(name.as_str()))
                {
                    Some(existing) if existing.id() != *token => {
                        Err(RegistryError::Set(TokenSetError::DuplicateName {
                            name: name.clone(),
                            existing: existing.id(),
                        })
                        .into())
                    }
                    _ => Ok(()),
                }
            }
            TokenCommand::SetExpression { token, expression } => {
                let scope = writable_scope(registry, *token)?;
                let token_type = registry
                    .resolve(*token)
                    .map(|resolved| resolved.definition.token_type())
                    .ok_or_else(|| unknown(*token))?;
                check_new_expression(registry, scope, *token, token_type, expression, others)
            }
        }
    }

    /// 검증 후 적용하고 inverse command를 돌려줍니다. 검증에 실패하면 registry는 그대로입니다. binding은 읽기만
    /// 합니다(삭제로 끊긴 live binding의 orphan 전환은 binding validator의 몫).
    pub fn apply(
        self,
        registry: &mut TokenRegistry,
        bindings: &BindingStore,
        others: &dyn ReferenceTypes,
    ) -> Result<TokenCommand, TokenCommandError> {
        self.validate(registry, bindings, others)?;
        match self {
            TokenCommand::Create { scope, definition } => {
                let id = definition.id();
                let restored = registry.remove_tombstone(id);
                if let Err(error) = registry.insert(scope, definition) {
                    if let Some(tombstone) = restored {
                        registry.insert_tombstone(tombstone);
                    }
                    return Err(error.into());
                }
                let orphan_plan =
                    restored.map(|_| TokenOrphanPlan::current(registry, bindings, id));
                Ok(TokenCommand::Delete {
                    token: id,
                    orphan_plan,
                })
            }
            TokenCommand::Delete { token, orphan_plan } => {
                let (scope, definition) = registry.remove(token).ok_or_else(|| unknown(token))?;
                if orphan_plan.is_some() {
                    registry.insert_tombstone(TokenTombstone {
                        token,
                        scope,
                        name: definition.name().clone(),
                        token_type: definition.token_type(),
                    });
                }
                Ok(TokenCommand::Create { scope, definition })
            }
            TokenCommand::Rename { token, name } => {
                let previous = registry.rename(token, name)?;
                Ok(TokenCommand::Rename {
                    token,
                    name: previous,
                })
            }
            TokenCommand::SetExpression { token, expression } => {
                let previous = registry.replace_expression(token, expression)?;
                Ok(TokenCommand::SetExpression {
                    token,
                    expression: previous,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use stellar_core_types::ids::StableId;
    use stellar_core_types::typed_value::TypedValue;
    use stellar_core_types::value::{FiniteF32, Length};
    use stellar_expressions::ast::{ExprNode, ExprNodeId, ReferenceTarget};
    use stellar_expressions::ops::BinaryOp;
    use stellar_expressions::types::{ExpressionType, TypeSpec};

    use super::*;
    use crate::token::{TokenSet, TokenSetId};
    use crate::token_cache::{TokenEvalError, TokenValueCache};
    use crate::token_graph::validate_for_save;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn token(n: u64) -> TokenRef {
        TokenRef::new(sid(1_000 + n))
    }

    fn node(n: u64) -> ExprNodeId {
        ExprNodeId::new(sid(n))
    }

    fn len(v: f32) -> TypedValue {
        TypedValue::Length(Length::new(v).expect("유한"))
    }

    fn name(s: &str) -> TokenName {
        TokenName::new(s).expect("이름")
    }

    fn literal(base: u64, value: TypedValue) -> ExpressionAst {
        ExpressionAst::new(node(base), vec![ExprNode::literal(node(base), value)]).expect("유효")
    }

    /// `token(target) × 2`.
    fn doubled(base: u64, target: TokenRef) -> ExpressionAst {
        ExpressionAst::new(
            node(base),
            vec![
                ExprNode::binary(
                    node(base),
                    BinaryOp::Multiply,
                    node(base + 1),
                    node(base + 2),
                ),
                ExprNode::reference(node(base + 1), ReferenceTarget::Token(target)),
                ExprNode::literal(
                    node(base + 2),
                    TypedValue::Number(FiniteF32::new(2.0).expect("유한")),
                ),
            ],
        )
        .expect("유효")
    }

    fn define(n: u64, label: &str, expression: ExpressionAst) -> TokenDefinition {
        TokenDefinition::new(
            token(n),
            name(label),
            TokenType::new(ExpressionType::Length),
            expression,
        )
    }

    const DOC: u64 = 900;
    const LIB: u64 = 800;

    /// 라이브러리 L: `lib.base`(1). 프로젝트: `base`(2), `double = base × 2`(3). 문서: `doc.only`(4).
    fn fixture() -> TokenRegistry {
        let mut lib = TokenSet::new(TokenSetId::new(sid(1)));
        lib.insert(define(1, "lib.base", literal(100, len(4.0))))
            .expect("L");
        let mut project = TokenSet::new(TokenSetId::new(sid(2)));
        project
            .insert(define(2, "base", literal(200, len(8.0))))
            .expect("base");
        project
            .insert(define(3, "double", doubled(300, token(2))))
            .expect("double");
        let mut doc = TokenSet::new(TokenSetId::new(sid(3)));
        doc.insert(define(4, "doc.only", literal(400, len(1.0))))
            .expect("doc");
        let mut registry = TokenRegistry::new();
        registry
            .add_scope(TokenScope::Library(sid(LIB)), lib)
            .expect("L");
        registry.add_scope(TokenScope::Project, project).expect("P");
        registry
            .add_scope(TokenScope::Document(sid(DOC)), doc)
            .expect("D");
        registry
    }

    fn no_types() -> BTreeMap<ReferenceTarget, TypeSpec> {
        BTreeMap::new()
    }

    fn no_bindings() -> crate::orphan::BindingStore {
        crate::orphan::BindingStore::default()
    }

    /// forward 적용 → inverse 적용이 원래 상태로 돌아오는지 확인하고 forward 직후 상태를 돌려줍니다.
    fn round_trip(registry: &mut TokenRegistry, command: TokenCommand) -> TokenRegistry {
        let original = registry.clone();
        let inverse = command
            .apply(registry, &no_bindings(), &no_types())
            .expect("적용");
        let applied = registry.clone();
        assert_ne!(&applied, &original, "변경 발생");
        let redo = inverse
            .apply(registry, &no_bindings(), &no_types())
            .expect("inverse 적용");
        assert_eq!(registry, &original, "inverse로 원래 상태");
        // inverse의 inverse는 다시 forward 결과.
        redo.apply(registry, &no_bindings(), &no_types())
            .expect("redo");
        assert_eq!(registry, &applied, "redo로 forward 상태");
        applied
    }

    #[test]
    fn create_rename_and_set_expression_round_trip() {
        let mut registry = fixture();
        let applied = round_trip(
            &mut registry,
            TokenCommand::Create {
                scope: TokenScope::Document(sid(DOC)),
                definition: define(5, "doc.double", doubled(500, token(3))),
            },
        );
        assert!(applied.resolve(token(5)).is_some());
        round_trip(
            &mut registry,
            TokenCommand::Rename {
                token: token(2),
                name: name("spacing.base"),
            },
        );
        let applied = round_trip(
            &mut registry,
            TokenCommand::SetExpression {
                token: token(3),
                expression: doubled(310, token(1)),
            },
        );
        assert_eq!(
            applied
                .resolve(token(3))
                .map(|r| r.definition.referenced_tokens()),
            Some(BTreeSet::from([token(1)]))
        );
        assert_eq!(validate_for_save(&registry, &no_types()), Ok(()));
    }

    #[test]
    fn invalid_commands_are_rejected_without_changes() {
        let mut registry = fixture();
        let original = registry.clone();
        let reject = |registry: &mut TokenRegistry, command: TokenCommand| {
            let error = command
                .apply(registry, &no_bindings(), &no_types())
                .expect_err("거부");
            assert_eq!(registry, &original, "실패 시 registry 불변");
            error
        };
        // type mismatch(Number 리터럴을 Length 토큰에).
        assert!(matches!(
            reject(
                &mut registry,
                TokenCommand::SetExpression {
                    token: token(2),
                    expression: literal(600, TypedValue::Number(FiniteF32::ONE)),
                }
            ),
            TokenCommandError::Type(TokenTypeError::ResultMismatch { .. })
        ));
        // cycle: base = double × 2.
        assert_eq!(
            reject(
                &mut registry,
                TokenCommand::SetExpression {
                    token: token(2),
                    expression: doubled(610, token(3)),
                }
            ),
            TokenCommandError::Cycle(TokenCycle {
                path: vec![token(2), token(3), token(2)]
            })
        );
        // 자기 참조.
        assert!(matches!(
            reject(
                &mut registry,
                TokenCommand::SetExpression {
                    token: token(2),
                    expression: doubled(620, token(2)),
                }
            ),
            TokenCommandError::Cycle(_)
        ));
        // 프로젝트 토큰이 문서 토큰을 참조(보이지 않음), 없는 토큰 참조.
        assert!(matches!(
            reject(
                &mut registry,
                TokenCommand::SetExpression {
                    token: token(2),
                    expression: doubled(630, token(4)),
                }
            ),
            TokenCommandError::InvalidReference(ReferenceIssue::NotVisible { .. })
        ));
        assert!(matches!(
            reject(
                &mut registry,
                TokenCommand::Create {
                    scope: TokenScope::Project,
                    definition: define(6, "dangling", doubled(640, token(77))),
                }
            ),
            TokenCommandError::InvalidReference(ReferenceIssue::Dangling { .. })
        ));
        // 이름 중복·ID 중복·라이브러리 읽기 전용·없는 scope.
        assert!(matches!(
            reject(
                &mut registry,
                TokenCommand::Rename {
                    token: token(3),
                    name: name("base"),
                }
            ),
            TokenCommandError::Registry(RegistryError::Set(TokenSetError::DuplicateName { .. }))
        ));
        assert!(matches!(
            reject(
                &mut registry,
                TokenCommand::Create {
                    scope: TokenScope::Project,
                    definition: define(4, "dup.id", literal(650, len(1.0))),
                }
            ),
            TokenCommandError::Registry(RegistryError::IdInOtherScope { .. })
        ));
        for command in [
            TokenCommand::Rename {
                token: token(1),
                name: name("renamed"),
            },
            TokenCommand::Delete {
                token: token(1),
                orphan_plan: None,
            },
            TokenCommand::Create {
                scope: TokenScope::Library(sid(LIB)),
                definition: define(7, "lib.new", literal(660, len(1.0))),
            },
        ] {
            assert!(matches!(
                reject(&mut registry, command),
                TokenCommandError::ReadOnlyScope(TokenScope::Library(_))
            ));
        }
        assert!(matches!(
            reject(
                &mut registry,
                TokenCommand::Create {
                    scope: TokenScope::Document(sid(12345)),
                    definition: define(8, "nowhere", literal(670, len(1.0))),
                }
            ),
            TokenCommandError::Registry(RegistryError::UnknownScope(_))
        ));
    }

    #[test]
    fn delete_with_inbound_references_requires_exact_orphan_plan() {
        let mut registry = fixture();
        let original = registry.clone();
        // plan 없이 거부.
        assert_eq!(
            TokenCommand::Delete {
                token: token(2),
                orphan_plan: None,
            }
            .apply(&mut registry, &no_bindings(), &no_types()),
            Err(TokenCommandError::InboundReferences {
                token: token(2),
                inbound: TokenOrphanPlan {
                    referrers: BTreeSet::from([token(3)]),
                    bindings: BTreeSet::new(),
                },
            })
        );
        // 오래된(다른) plan 거부.
        assert!(matches!(
            TokenCommand::Delete {
                token: token(2),
                orphan_plan: Some(TokenOrphanPlan::default()),
            }
            .apply(&mut registry, &no_bindings(), &no_types()),
            Err(TokenCommandError::StaleOrphanPlan { .. })
        ));
        assert_eq!(registry, original);
        // 정확한 plan: 참조 표현식은 그대로, orphan 기록 보존, 저장 가능.
        let plan = TokenOrphanPlan {
            referrers: BTreeSet::from([token(3)]),
            ..TokenOrphanPlan::default()
        };
        let applied = round_trip(
            &mut registry,
            TokenCommand::Delete {
                token: token(2),
                orphan_plan: Some(plan),
            },
        );
        assert!(applied.resolve(token(2)).is_none());
        let tombstone = applied.tombstone(token(2)).expect("orphan 기록");
        assert_eq!(tombstone.name, name("base"));
        assert_eq!(tombstone.scope, TokenScope::Project);
        assert_eq!(
            applied
                .resolve(token(3))
                .map(|r| r.definition.referenced_tokens()),
            Some(BTreeSet::from([token(2)])),
            "참조 ID 보존"
        );
        assert_eq!(
            applied.reference_issues().into_iter().collect::<Vec<_>>(),
            vec![ReferenceIssue::Orphaned {
                token: token(3),
                target: token(2)
            }]
        );
        assert_eq!(validate_for_save(&applied, &no_types()), Ok(()));
        // orphan 참조는 평가 시 Orphaned 오류, 삭제된 토큰을 새로 참조할 수는 없습니다.
        let mut cache = TokenValueCache::new();
        let inputs = BTreeMap::new();
        assert_eq!(
            cache.get(token(3), &applied, &inputs),
            Err(TokenEvalError::Orphaned(token(2)))
        );
        let mut deleted = applied.clone();
        assert!(matches!(
            TokenCommand::Create {
                scope: TokenScope::Project,
                definition: define(9, "uses.deleted", doubled(700, token(2))),
            }
            .apply(&mut deleted, &no_bindings(), &no_types()),
            Err(TokenCommandError::InvalidReference(
                ReferenceIssue::Orphaned { .. }
            ))
        ));
        // 다른 이름으로 같은 ID를 다시 만들 수 없습니다.
        assert!(matches!(
            TokenCommand::Create {
                scope: TokenScope::Project,
                definition: define(2, "other.name", literal(710, len(8.0))),
            }
            .apply(&mut deleted, &no_bindings(), &no_types()),
            Err(TokenCommandError::TombstoneMismatch(_))
        ));
        // 참조 없는 토큰은 plan 없이 삭제할 수 있습니다.
        let applied = round_trip(
            &mut registry,
            TokenCommand::Delete {
                token: token(4),
                orphan_plan: None,
            },
        );
        assert!(applied.tombstone(token(4)).is_none());
    }

    #[test]
    fn command_sequence_undo_restores_initial_state() {
        let mut registry = fixture();
        let initial = registry.clone();
        let commands = vec![
            TokenCommand::Create {
                scope: TokenScope::Project,
                definition: define(10, "triple", doubled(800, token(3))),
            },
            TokenCommand::Rename {
                token: token(10),
                name: name("quad"),
            },
            TokenCommand::SetExpression {
                token: token(3),
                expression: doubled(810, token(1)),
            },
            TokenCommand::Delete {
                token: token(2),
                orphan_plan: None,
            },
            TokenCommand::Delete {
                token: token(3),
                orphan_plan: Some(TokenOrphanPlan {
                    referrers: BTreeSet::from([token(10)]),
                    ..TokenOrphanPlan::default()
                }),
            },
        ];
        let mut inverses = Vec::new();
        for command in commands {
            inverses.push(
                command
                    .apply(&mut registry, &no_bindings(), &no_types())
                    .expect("적용"),
            );
        }
        let finished = registry.clone();
        let mut redos = Vec::new();
        while let Some(inverse) = inverses.pop() {
            redos.push(
                inverse
                    .apply(&mut registry, &no_bindings(), &no_types())
                    .expect("undo"),
            );
        }
        assert_eq!(registry, initial, "전체 undo 후 초기 상태");
        while let Some(redo) = redos.pop() {
            redo.apply(&mut registry, &no_bindings(), &no_types())
                .expect("redo");
        }
        assert_eq!(registry, finished, "전체 redo 후 최종 상태");
    }

    /// `token(target) × 2`를 source로 하는 binding(target 속성 `2_000 + n`).
    fn binding_on(n: u64, target: TokenRef) -> crate::binding::ValueBinding {
        use stellar_core_types::property::{PropertyKey, PropertyPath};
        use stellar_core_types::reference::NodeRef;
        crate::binding::ValueBinding::new(
            crate::binding::BindingId::new(sid(4_000 + n)),
            PropertyPath::new(sid(2_000 + n)).push(PropertyKey(1)),
            doubled(5_000 + n * 10, target),
            TypeSpec::required(ExpressionType::Length),
            crate::binding::ScopeOwner::Node(NodeRef::new(sid(6_000 + n))),
        )
    }

    /// b1(live): p1 ← doc.only × 2, b2(orphan, target 없음): p2 ← doc.only × 2, b3(live): p3 ← base × 2.
    fn binding_store() -> crate::orphan::BindingStore {
        use crate::orphan::BindingWorld;
        use stellar_core_types::property::PropertyPath;

        /// b2의 target만 없는 상태.
        struct World;
        impl BindingWorld for World {
            fn target_type(&self, target: &PropertyPath) -> Option<TypeSpec> {
                (target.root() != sid(2_002)).then_some(TypeSpec::required(ExpressionType::Length))
            }
            fn reference_type(&self, _target: &ReferenceTarget) -> Option<TypeSpec> {
                Some(TypeSpec::required(ExpressionType::Length))
            }
        }

        let mut live = crate::binding::BindingSet::new();
        live.insert(binding_on(1, token(4))).expect("b1");
        live.insert(binding_on(2, token(4))).expect("b2");
        live.insert(binding_on(3, token(2))).expect("b3");
        let mut store = crate::orphan::BindingStore::new(live);
        assert_eq!(
            store.orphan_broken(&World),
            vec![crate::binding::BindingId::new(sid(4_002))]
        );
        store
    }

    #[test]
    fn delete_referenced_by_bindings_requires_exact_orphan_plan() {
        // 회귀(review EB-3): ValueBinding(live·orphan)만 참조하는 토큰도 inbound 참조가 있으므로 orphan plan 없이
        // 삭제할 수 없고, plan은 binding 목록까지 정확해야 합니다. 수정 전에는 토큰 간 참조만 보아 plan 없이 통과했습니다.
        use crate::binding::BindingId;
        let mut registry = fixture();
        let original = registry.clone();
        let store = binding_store();
        let b = |n: u64| BindingId::new(sid(4_000 + n));
        let expected = TokenOrphanPlan {
            referrers: BTreeSet::new(),
            bindings: BTreeSet::from([b(1), b(2)]),
        };
        assert_eq!(
            TokenOrphanPlan::current(&registry, &store, token(4)),
            expected,
            "live b1과 orphan b2 모두 inbound"
        );
        // plan 없음 → 거부.
        assert_eq!(
            TokenCommand::Delete {
                token: token(4),
                orphan_plan: None,
            }
            .apply(&mut registry, &store, &no_types()),
            Err(TokenCommandError::InboundReferences {
                token: token(4),
                inbound: expected.clone(),
            })
        );
        // binding 하나가 빠진(오래된) plan → 거부.
        let stale = TokenOrphanPlan {
            bindings: BTreeSet::from([b(1)]),
            ..TokenOrphanPlan::default()
        };
        assert_eq!(
            TokenCommand::Delete {
                token: token(4),
                orphan_plan: Some(stale.clone()),
            }
            .apply(&mut registry, &store, &no_types()),
            Err(TokenCommandError::StaleOrphanPlan {
                token: token(4),
                actual: expected.clone(),
                planned: stale,
            })
        );
        assert_eq!(registry, original, "거부 시 불변");
        // 정확한 plan → 삭제, tombstone 보존, binding은 읽기만(그대로).
        let store_before = store.clone();
        let undo = TokenCommand::Delete {
            token: token(4),
            orphan_plan: Some(expected.clone()),
        }
        .apply(&mut registry, &store, &no_types())
        .expect("plan 삭제");
        assert_eq!(store, store_before);
        assert!(registry.resolve(token(4)).is_none());
        assert!(registry.tombstone(token(4)).is_some());
        // undo(tombstone 복원 Create)의 inverse도 binding까지 담은 정확한 plan.
        let redo = undo
            .apply(&mut registry, &store, &no_types())
            .expect("undo");
        assert_eq!(registry, original, "undo 후 원래 상태");
        assert_eq!(
            redo,
            TokenCommand::Delete {
                token: token(4),
                orphan_plan: Some(expected),
            }
        );
        // 토큰과 binding이 함께 참조하는 base(2): plan은 둘 다 담아야 합니다.
        assert_eq!(
            TokenOrphanPlan::current(&registry, &store, token(2)),
            TokenOrphanPlan {
                referrers: BTreeSet::from([token(3)]),
                bindings: BTreeSet::from([b(3)]),
            }
        );
        // 어떤 binding도 읽지 않는 토큰은 이전처럼 plan 없이 삭제됩니다.
        let mut free = original.clone();
        TokenCommand::Delete {
            token: token(3),
            orphan_plan: None,
        }
        .apply(&mut free, &store, &no_types())
        .expect("참조 없는 삭제");
    }
}
