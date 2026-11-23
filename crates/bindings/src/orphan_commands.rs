//! orphaned binding 재연결·명시 삭제 command (VIII.8.4).
//!
//! orphan(VIII.8.1)은 자동으로 다시 연결되거나 삭제되지 않습니다. 사용자가 명시적으로
//!
//! - [`OrphanCommand::Reconnect`] — 새 source(와 기대 타입)로 다시 연결해 live binding으로 되돌리거나,
//! - [`OrphanCommand::Delete`] — orphan을 삭제합니다.
//!
//! 두 command의 inverse는 각각 [`OrphanCommand::Orphanize`](재연결 전 orphan 그대로 복원)와
//! [`OrphanCommand::Restore`](삭제한 orphan 복원)입니다. `apply(forward)` 후 `apply(inverse)`는 원래 저장소와 같은
//! 상태입니다. 검증 실패 시 저장소는 바뀌지 않습니다.
//!
//! 재연결 검증: orphan이 있어야 하고, 같은 target에 live binding이 없어야 하며, 다시 연결한 binding이 현재 상태
//! ([`BindingWorld`])에서 깨지지 않아야 하고([`break_cause`]: target·참조 존재, type), cycle을 만들지 않아야 합니다.
//!
//! 토큰·binding command(VIII.5.5·VIII.6.6)와 같이 Phase VII command 기반이 생기기 전의 전용 형태입니다.

use std::collections::BTreeSet;
use std::fmt;

use stellar_expressions::ast::ExpressionAst;
use stellar_expressions::types::TypeSpec;

use crate::binding::{BindingId, BindingSetError};
use crate::graph::{BindingCycle, DependencyNode};
use crate::graph_view::BindingGraphView;
use crate::orphan::{break_cause, BindingStore, BindingWorld, OrphanCause, OrphanedBinding};
use crate::registry::TokenRegistry;

/// orphan command (VIII.8.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrphanCommand {
    /// orphan을 새 source와 기대 타입으로 다시 연결합니다(ID·target·owner 유지).
    Reconnect {
        /// orphan binding.
        binding: BindingId,
        /// 새 source.
        source: ExpressionAst,
        /// 새 기대 타입(target 속성의 현재 선언 타입).
        expected_type: TypeSpec,
    },
    /// orphan을 명시적으로 삭제합니다.
    Delete {
        /// orphan binding.
        binding: BindingId,
    },
    /// live binding을 주어진 orphan 상태로 되돌립니다(Reconnect의 inverse).
    Orphanize(OrphanedBinding),
    /// 삭제한 orphan을 복원합니다(Delete의 inverse).
    Restore(OrphanedBinding),
}

/// orphan command 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrphanCommandError {
    /// orphan이 없습니다.
    UnknownOrphan(BindingId),
    /// live binding이 없습니다.
    UnknownLive(BindingId),
    /// 같은 ID·target 규칙 위반.
    Set(BindingSetError),
    /// 다시 연결한 binding이 현재 상태에서 여전히 깨져 있습니다.
    StillBroken(OrphanCause),
    /// 다시 연결하면 cycle이 생깁니다.
    Cycle(BindingCycle),
    /// orphan으로 되돌릴 live binding과 orphan의 target·owner가 다릅니다.
    TargetMismatch(BindingId),
    /// 재연결 기대 타입이 target 속성의 현재 선언 타입과 다릅니다(target이 없으면 `declared`는 `None`).
    ExpectedTypeMismatch {
        /// 현재 선언 타입.
        declared: Option<TypeSpec>,
        /// 주어진 기대 타입.
        given: TypeSpec,
    },
}

impl fmt::Display for OrphanCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OrphanCommandError::UnknownOrphan(id) => write!(f, "orphan binding이 없습니다: {id}"),
            OrphanCommandError::UnknownLive(id) => write!(f, "live binding이 없습니다: {id}"),
            OrphanCommandError::Set(error) => write!(f, "{error}"),
            OrphanCommandError::StillBroken(cause) => {
                write!(f, "다시 연결해도 깨져 있습니다: {cause}")
            }
            OrphanCommandError::Cycle(cycle) => write!(f, "{cycle}"),
            OrphanCommandError::TargetMismatch(id) => {
                write!(f, "binding {id}의 target·owner가 orphan과 다릅니다")
            }
            OrphanCommandError::ExpectedTypeMismatch { given, .. } => {
                write!(f, "기대 타입 {given}이 target 선언 타입과 다릅니다")
            }
        }
    }
}

impl std::error::Error for OrphanCommandError {}

impl OrphanCommand {
    /// 저장소를 바꾸지 않고 검증합니다.
    pub fn validate(
        &self,
        store: &BindingStore,
        tokens: &TokenRegistry,
        world: &dyn BindingWorld,
    ) -> Result<(), OrphanCommandError> {
        match self {
            OrphanCommand::Reconnect {
                binding,
                source,
                expected_type,
            } => {
                let orphan = store
                    .orphan(*binding)
                    .ok_or(OrphanCommandError::UnknownOrphan(*binding))?;
                if let Some(existing) = store.live().by_target(orphan.target()) {
                    return Err(OrphanCommandError::Set(BindingSetError::DuplicateTarget {
                        target: orphan.target().clone(),
                        existing: existing.id(),
                    }));
                }
                let next = orphan.binding().with_source(source.clone(), *expected_type);
                // 기대 타입은 target 속성의 현재 선언 타입이어야 합니다(target이 없으면 아래 판정이 TargetMissing).
                let declared = world.target_type(next.target());
                if declared.is_some_and(|declared| declared != *expected_type) {
                    return Err(OrphanCommandError::ExpectedTypeMismatch {
                        declared,
                        given: *expected_type,
                    });
                }
                let node = DependencyNode::Property(next.target().clone());
                let dependencies: BTreeSet<DependencyNode> = next
                    .references()
                    .iter()
                    .map(DependencyNode::from_target)
                    .collect();
                if let Some(cycle) = BindingGraphView::new(tokens, store.live())
                    .cycle_if_dependencies(&node, &dependencies)
                {
                    return Err(OrphanCommandError::Cycle(cycle));
                }
                match break_cause(&next, world) {
                    Some(cause) => Err(OrphanCommandError::StillBroken(cause)),
                    None => Ok(()),
                }
            }
            OrphanCommand::Delete { binding } => store
                .orphan(*binding)
                .map(|_| ())
                .ok_or(OrphanCommandError::UnknownOrphan(*binding)),
            OrphanCommand::Orphanize(orphan) => {
                let live = store
                    .live()
                    .get(orphan.id())
                    .ok_or(OrphanCommandError::UnknownLive(orphan.id()))?;
                if live.target() != orphan.target()
                    || live.scope_owner() != orphan.binding().scope_owner()
                {
                    return Err(OrphanCommandError::TargetMismatch(orphan.id()));
                }
                Ok(())
            }
            OrphanCommand::Restore(orphan) => {
                if store.orphan(orphan.id()).is_some() || store.live().get(orphan.id()).is_some() {
                    return Err(OrphanCommandError::Set(BindingSetError::DuplicateId(
                        orphan.id(),
                    )));
                }
                Ok(())
            }
        }
    }

    /// 검증 후 적용하고 inverse command를 돌려줍니다.
    pub fn apply(
        self,
        store: &mut BindingStore,
        tokens: &TokenRegistry,
        world: &dyn BindingWorld,
    ) -> Result<OrphanCommand, OrphanCommandError> {
        self.validate(store, tokens, world)?;
        match self {
            OrphanCommand::Reconnect {
                binding,
                source,
                expected_type,
            } => {
                let orphan = store
                    .remove_orphan(binding)
                    .ok_or(OrphanCommandError::UnknownOrphan(binding))?;
                let next = orphan.binding().with_source(source, expected_type);
                if let Err(error) = store.insert_live(next) {
                    store.insert_orphan(orphan);
                    return Err(OrphanCommandError::Set(error));
                }
                Ok(OrphanCommand::Orphanize(orphan))
            }
            OrphanCommand::Delete { binding } => {
                let orphan = store
                    .remove_orphan(binding)
                    .ok_or(OrphanCommandError::UnknownOrphan(binding))?;
                Ok(OrphanCommand::Restore(orphan))
            }
            OrphanCommand::Orphanize(orphan) => {
                let live = store
                    .remove_live(orphan.id())
                    .ok_or(OrphanCommandError::UnknownLive(orphan.id()))?;
                let inverse = OrphanCommand::Reconnect {
                    binding: orphan.id(),
                    source: live.source().clone(),
                    expected_type: live.expected_type(),
                };
                store.insert_orphan(orphan);
                Ok(inverse)
            }
            OrphanCommand::Restore(orphan) => {
                let id = orphan.id();
                store.insert_orphan(orphan);
                Ok(OrphanCommand::Delete { binding: id })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use stellar_core_types::ids::StableId;
    use stellar_core_types::property::{PropertyKey, PropertyPath};
    use stellar_core_types::reference::{NodeRef, TokenRef};
    use stellar_core_types::typed_value::TypedValue;
    use stellar_core_types::value::Length;
    use stellar_expressions::ast::{ExprNode, ExprNodeId, ReferenceTarget};
    use stellar_expressions::ops::BinaryOp;
    use stellar_expressions::types::ExpressionType;

    use super::*;
    use crate::binding::{BindingSet, ScopeOwner, ValueBinding};
    use crate::fallback::DeclaredDefault;
    use crate::registry::TokenScope;
    use crate::scheduler::{BindingEvaluator, NodeValue, StaticSources};
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

    /// `target + 1px`.
    fn plus_one(base: u64, target: ReferenceTarget) -> ExpressionAst {
        ExpressionAst::new(
            x(base),
            vec![
                ExprNode::binary(x(base), BinaryOp::Add, x(base + 1), x(base + 2)),
                ExprNode::reference(x(base + 1), target),
                ExprNode::literal(x(base + 2), px(1.0)),
            ],
        )
        .expect("유효")
    }

    /// 토큰 t2 = 7px(t1은 삭제된 상태).
    fn tokens() -> TokenRegistry {
        let mut set = TokenSet::new(TokenSetId::new(sid(1)));
        set.insert(TokenDefinition::new(
            token(2),
            TokenName::new("t2").expect("이름"),
            TokenType::new(ExpressionType::Length),
            literal(20, px(7.0)),
        ))
        .expect("t2");
        let mut registry = TokenRegistry::new();
        registry.add_scope(TokenScope::Project, set).expect("scope");
        registry
    }

    struct World<'a>(&'a TokenRegistry);

    impl BindingWorld for World<'_> {
        fn target_type(&self, target: &PropertyPath) -> Option<TypeSpec> {
            (target != &prop(99)).then(length)
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

    fn bind(n: u64, target: u64, source: ExpressionAst) -> ValueBinding {
        ValueBinding::new(
            BindingId::new(sid(4_000 + n)),
            prop(target),
            source,
            length(),
            ScopeOwner::Node(NodeRef::new(sid(5_000 + n))),
        )
    }

    /// b1: p1 ← t1 + 1px(t1 삭제로 orphan), b2: p2 ← p1 + 1px(live).
    fn store(tokens: &TokenRegistry) -> BindingStore {
        let mut live = BindingSet::new();
        live.insert(bind(1, 1, plus_one(100, ReferenceTarget::Token(token(1)))))
            .expect("b1");
        live.insert(bind(
            2,
            2,
            plus_one(200, ReferenceTarget::Property(prop(1))),
        ))
        .expect("b2");
        let mut store = BindingStore::new(live);
        assert_eq!(store.orphan_broken(&World(tokens)).len(), 1);
        store
    }

    fn evaluate(tokens: &TokenRegistry, store: &BindingStore) -> (NodeValue, NodeValue) {
        let defaults = BTreeMap::from([(
            prop(1),
            DeclaredDefault {
                declared_type: length(),
                expression: literal(300, px(4.0)),
            },
        )]);
        let evaluator =
            BindingEvaluator::with_orphans(tokens, store, &defaults, &StaticSources::new())
                .expect("생성");
        let get = |n| {
            evaluator
                .value(&DependencyNode::Property(prop(n)))
                .cloned()
                .expect("node")
        };
        (get(1), get(2))
    }

    fn b1() -> BindingId {
        BindingId::new(sid(4_001))
    }

    #[test]
    fn reconnect_gives_live_value_and_undo_restores_orphan() {
        let tokens = tokens();
        let world = World(&tokens);
        let mut store = store(&tokens);
        let orphaned = store.clone();
        assert_eq!(
            evaluate(&tokens, &store),
            (Ok(Some(px(4.0))), Ok(Some(px(5.0))))
        );
        // t2 + 1px로 재연결 → live 값 8px.
        let inverse = OrphanCommand::Reconnect {
            binding: b1(),
            source: plus_one(400, ReferenceTarget::Token(token(2))),
            expected_type: length(),
        }
        .apply(&mut store, &tokens, &world)
        .expect("재연결");
        assert!(store.orphan(b1()).is_none());
        assert!(store.live().get(b1()).is_some());
        assert_eq!(
            evaluate(&tokens, &store),
            (Ok(Some(px(8.0))), Ok(Some(px(9.0))))
        );
        let reconnected = store.clone();
        // undo → orphan 상태(원래 source·원인 그대로), 기본값 4px.
        let redo = inverse.apply(&mut store, &tokens, &world).expect("undo");
        assert_eq!(store, orphaned);
        assert_eq!(
            evaluate(&tokens, &store),
            (Ok(Some(px(4.0))), Ok(Some(px(5.0))))
        );
        // redo → 다시 live.
        redo.apply(&mut store, &tokens, &world).expect("redo");
        assert_eq!(store, reconnected);
    }

    #[test]
    fn explicit_delete_and_undo() {
        let tokens = tokens();
        let world = World(&tokens);
        let mut store = store(&tokens);
        let orphaned = store.clone();
        let inverse = OrphanCommand::Delete { binding: b1() }
            .apply(&mut store, &tokens, &world)
            .expect("삭제");
        assert!(store.orphan(b1()).is_none());
        assert_eq!(store.len(), 1);
        // 삭제 후 p1은 producer가 없는 source(값 없음).
        assert!(evaluate(&tokens, &store).0.is_err());
        let redo = inverse.apply(&mut store, &tokens, &world).expect("undo");
        assert_eq!(store, orphaned);
        redo.apply(&mut store, &tokens, &world).expect("redo");
        assert!(store.orphan(b1()).is_none());
    }

    #[test]
    fn invalid_reconnects_are_rejected_without_changes() {
        let tokens = tokens();
        let world = World(&tokens);
        let mut store = store(&tokens);
        let original = store.clone();
        let reconnect = |source: ExpressionAst, expected_type: TypeSpec| OrphanCommand::Reconnect {
            binding: b1(),
            source,
            expected_type,
        };
        // 여전히 없는 토큰.
        assert!(matches!(
            reconnect(plus_one(500, ReferenceTarget::Token(token(1))), length())
                .apply(&mut store, &tokens, &world),
            Err(OrphanCommandError::StillBroken(
                OrphanCause::SourceMissing { .. }
            ))
        ));
        // 기대 타입 불일치(Color는 현재 선언 Length와 다름).
        assert_eq!(
            reconnect(
                plus_one(510, ReferenceTarget::Token(token(2))),
                TypeSpec::required(ExpressionType::Color)
            )
            .apply(&mut store, &tokens, &world),
            Err(OrphanCommandError::ExpectedTypeMismatch {
                declared: Some(length()),
                given: TypeSpec::required(ExpressionType::Color)
            })
        );
        // source 타입 불일치(Boolean 결과를 Length 속성에).
        assert!(matches!(
            reconnect(literal(515, TypedValue::Boolean(true)), length())
                .apply(&mut store, &tokens, &world),
            Err(OrphanCommandError::StillBroken(
                OrphanCause::TargetTypeChanged { .. }
            ))
        ));
        // cycle: p1이 p2를 읽으면 p2 → p1 → p2.
        assert!(matches!(
            reconnect(plus_one(520, ReferenceTarget::Property(prop(2))), length())
                .apply(&mut store, &tokens, &world),
            Err(OrphanCommandError::Cycle(_))
        ));
        // 없는 orphan·live binding 되돌리기.
        assert_eq!(
            OrphanCommand::Delete {
                binding: BindingId::new(sid(4_002))
            }
            .apply(&mut store, &tokens, &world),
            Err(OrphanCommandError::UnknownOrphan(BindingId::new(sid(
                4_002
            ))))
        );
        let orphan = store.orphan(b1()).cloned().expect("orphan");
        assert_eq!(
            OrphanCommand::Orphanize(orphan.clone()).apply(&mut store, &tokens, &world),
            Err(OrphanCommandError::UnknownLive(b1()))
        );
        assert!(matches!(
            OrphanCommand::Restore(orphan).apply(&mut store, &tokens, &world),
            Err(OrphanCommandError::Set(BindingSetError::DuplicateId(_)))
        ));
        assert_eq!(store, original, "모든 실패에서 불변");
        // 같은 target에 live binding이 생기면 재연결할 수 없습니다.
        store
            .insert_live(bind(9, 1, literal(600, px(2.0))))
            .expect("b9");
        assert!(matches!(
            reconnect(plus_one(530, ReferenceTarget::Token(token(2))), length())
                .apply(&mut store, &tokens, &world),
            Err(OrphanCommandError::Set(
                BindingSetError::DuplicateTarget { .. }
            ))
        ));
    }
}
