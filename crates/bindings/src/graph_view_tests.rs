//! 저장소 graph view 시험과 release 측정 (XVI.4.9).
//!
//! random command sequence(토큰 VIII.5.5, binding VIII.6.6, orphan VIII.8.4) 뒤 view의 인접 목록·producer와 cycle 판정이
//! 전체 재구성 graph([`DependencyGraph::build`], [`TokenDependencyGraph::build`])와 같은지 확인합니다. release 측정은
//! `cargo test -p stellar-bindings --release graph_view_tests -- --ignored --nocapture`입니다.
#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_core_types::reference::{NodeRef, TokenRef};
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::FiniteF32;
use stellar_expressions::ast::{ExprNode, ExprNodeId, ExpressionAst, ReferenceTarget};
use stellar_expressions::functions::FunctionId;
use stellar_expressions::typecheck::ReferenceTypes;
use stellar_expressions::types::{ExpressionType, TypeSpec};

use crate::binding::{BindingId, BindingSet, ScopeOwner, ValueBinding};
use crate::binding_commands::{BindingCommand, BindingContext, TargetCheck, TargetRejection};
use crate::graph::{DependencyGraph, DependencyNode};
use crate::graph_view::{BindingGraphView, TokenGraphView};
use crate::orphan::{BindingWorld, OrphanCause, OrphanedBinding};
use crate::orphan_commands::OrphanCommand;
use crate::registry::{TokenRegistry, TokenScope};
use crate::store::BindingStore;
use crate::store_bench::{fixture, mean, TOKENS};
use crate::synthetic::{generate, SyntheticConfig, SyntheticGraph, SYNTHETIC_ENVIRONMENT};
use crate::token::{TokenDefinition, TokenName, TokenType, TokenTypes};
use crate::token_commands::{TokenCommand, TokenOrphanPlan};
use crate::token_graph::TokenDependencyGraph;

/// 결정적 난수(SplitMix64).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

fn number() -> TypeSpec {
    TypeSpec::required(ExpressionType::Number)
}

/// 참조 0~2개의 `max`(없으면 상수, 하나면 그 참조). node id는 `base`에서 시작합니다.
fn expression(base: u64, targets: &[ReferenceTarget]) -> ExpressionAst {
    let node = |k: u64| ExprNodeId::new(StableId::new_v7(9, 0, base + k));
    let nodes = match targets {
        [] => vec![ExprNode::literal(
            node(0),
            TypedValue::Number(FiniteF32::new(1.0).expect("유한")),
        )],
        [only] => vec![ExprNode::reference(node(0), only.clone())],
        [first, second, ..] => vec![
            ExprNode::call(node(0), FunctionId::Max, &[node(1), node(2)]),
            ExprNode::reference(node(1), first.clone()),
            ExprNode::reference(node(2), second.clone()),
        ],
    };
    ExpressionAst::new(node(0), nodes).expect("유효")
}

/// 무엇이든 받는 target 검사.
struct AnyTarget;

impl TargetCheck for AnyTarget {
    fn reject_target(&self, _target: &PropertyPath) -> Option<TargetRejection> {
        None
    }
}

/// 모든 속성·환경 입력이 Number인 세계.
struct World<'a>(&'a TokenRegistry);

impl ReferenceTypes for World<'_> {
    fn reference_type(&self, target: &ReferenceTarget) -> Option<TypeSpec> {
        match target {
            ReferenceTarget::Token(token) => self.0.token_type_of(*token).map(|t| t.type_spec()),
            ReferenceTarget::Property(_) | ReferenceTarget::Environment(_) => Some(number()),
            ReferenceTarget::ComponentProperty(_) => None,
        }
    }
}

impl BindingWorld for World<'_> {
    fn target_type(&self, _target: &PropertyPath) -> Option<TypeSpec> {
        Some(number())
    }

    fn reference_type(&self, target: &ReferenceTarget) -> Option<TypeSpec> {
        ReferenceTypes::reference_type(self, target)
    }
}

/// random 참조 대상: 토큰(생성분 + 새로 만든 것), 속성, 환경 입력.
fn random_target(rng: &mut Rng, tokens: u64, properties: u32) -> ReferenceTarget {
    match rng.below(5) {
        0 | 1 => ReferenceTarget::Token(SyntheticGraph::token_ref(rng.below(tokens) as u32)),
        2 | 3 => ReferenceTarget::Property(SyntheticGraph::property(
            rng.below(u64::from(properties)) as u32,
        )),
        _ => ReferenceTarget::Environment(SYNTHETIC_ENVIRONMENT[rng.below(3) as usize]),
    }
}

fn random_targets(rng: &mut Rng, tokens: u64, properties: u32) -> Vec<ReferenceTarget> {
    let mut targets: Vec<ReferenceTarget> = (0..rng.below(3))
        .map(|_| random_target(rng, tokens, properties))
        .collect();
    targets.sort();
    targets.dedup();
    targets
}

/// view와 전체 재구성 graph가 같은지 확인하고, random cycle 질의 결과도 비교합니다.
fn assert_views_match(
    registry: &TokenRegistry,
    bindings: &BindingSet,
    rng: &mut Rng,
    tokens: u64,
    properties: u32,
) {
    let built = DependencyGraph::build(registry, bindings);
    let view = BindingGraphView::new(registry, bindings);
    for index in built.indices() {
        let node = built.node(index);
        let expected: BTreeSet<DependencyNode> = built
            .dependencies(index)
            .iter()
            .map(|dep| built.node(*dep).clone())
            .collect();
        assert_eq!(view.dependencies(node), expected, "{node} 간선");
        assert_eq!(
            view.producer(node),
            built.producer(index),
            "{node} producer"
        );
    }
    let token_built = TokenDependencyGraph::build(registry);
    let token_view = TokenGraphView::new(registry);
    for (_, definition) in registry.tokens() {
        let id = definition.id();
        assert_eq!(
            token_view.dependencies(id),
            token_built.dependencies(id).collect::<Vec<_>>()
        );
    }
    for _ in 0..12 {
        let node = DependencyNode::from_target(&random_target(rng, tokens + 2, properties + 4));
        let deps: BTreeSet<DependencyNode> = random_targets(rng, tokens + 2, properties + 4)
            .iter()
            .map(DependencyNode::from_target)
            .collect();
        assert_eq!(
            view.cycle_if_dependencies(&node, &deps),
            built.cycle_if_dependencies(&node, &deps)
        );
        let token = SyntheticGraph::token_ref(rng.below(tokens + 2) as u32);
        let references: BTreeSet<TokenRef> = (0..rng.below(3))
            .map(|_| SyntheticGraph::token_ref(rng.below(tokens + 2) as u32))
            .collect();
        assert_eq!(
            token_view.cycle_if_references(token, &references),
            token_built.cycle_if_references(token, &references)
        );
    }
}

fn small_fixture(seed: u64) -> (TokenRegistry, BindingSet) {
    generate(SyntheticConfig {
        seed,
        tokens: 24,
        bindings: 48,
        edges: 150,
        max_token_refs: 3,
        environment_one_in: 8,
    })
    .expect("설정")
    .materialize()
    .expect("실체화")
}

/// 토큰 command 하나를 만들어 적용합니다(검증 실패는 건너뜀).
fn random_token_step(
    rng: &mut Rng,
    registry: &mut TokenRegistry,
    store: &BindingStore,
    next_token: &mut u64,
    cycles: &mut usize,
) {
    let token_count = *next_token;
    let command = match rng.below(3) {
        0 => {
            let token = SyntheticGraph::token_ref(rng.below(token_count) as u32);
            let references: Vec<ReferenceTarget> = random_targets(rng, token_count, 0)
                .into_iter()
                .filter(|t| matches!(t, ReferenceTarget::Token(_)))
                .collect();
            TokenCommand::SetExpression {
                token,
                expression: expression(rng.next() % 1_000_000 * 4, &references),
            }
        }
        1 => {
            let id = *next_token;
            *next_token += 1;
            let references: Vec<ReferenceTarget> = random_targets(rng, token_count, 0)
                .into_iter()
                .filter(|t| matches!(t, ReferenceTarget::Token(_)))
                .collect();
            TokenCommand::Create {
                scope: TokenScope::Project,
                definition: TokenDefinition::new(
                    SyntheticGraph::token_ref(id as u32),
                    TokenName::new(&format!("random.{id}")).expect("이름"),
                    TokenType::new(ExpressionType::Number),
                    expression(rng.next() % 1_000_000 * 4, &references),
                ),
            }
        }
        _ => {
            let token = SyntheticGraph::token_ref(rng.below(token_count) as u32);
            TokenCommand::Delete {
                token,
                orphan_plan: Some(TokenOrphanPlan::current(registry, store, token)),
            }
        }
    };
    let types = BTreeMap::<ReferenceTarget, TypeSpec>::from_iter(
        SYNTHETIC_ENVIRONMENT
            .iter()
            .map(|key| (ReferenceTarget::Environment(*key), number())),
    );
    match command.clone().apply(registry, store, &types) {
        Ok(_) => {}
        Err(crate::token_commands::TokenCommandError::Cycle(_)) => *cycles += 1,
        Err(_) => {}
    }
}

#[test]
fn views_match_full_rebuilds_after_random_command_sequences() {
    let mut steps = 0usize;
    let mut cycles = 0usize;
    for seed in 0..30u64 {
        let mut rng = Rng(seed ^ 0x4_09);
        let (mut registry, mut bindings) = small_fixture(seed);
        let mut next_token = 24u64;
        let mut next_binding = 48u32;
        // 1. 토큰·binding command.
        for _ in 0..40 {
            if rng.below(3) == 0 {
                let store = BindingStore::new(bindings.clone());
                random_token_step(
                    &mut rng,
                    &mut registry,
                    &store,
                    &mut next_token,
                    &mut cycles,
                );
            } else {
                let world = World(&registry);
                let context = BindingContext {
                    tokens: &registry,
                    others: &world,
                    targets: &AnyTarget,
                };
                let ids: Vec<BindingId> = bindings.iter().map(ValueBinding::id).collect();
                let command = match rng.below(3) {
                    0 => {
                        let j = next_binding;
                        next_binding += 1;
                        BindingCommand::Create(ValueBinding::new(
                            BindingId::new(StableId::new_v7(5, 0, u64::from(j))),
                            SyntheticGraph::property(j),
                            expression(
                                rng.next() % 1_000_000 * 4,
                                &random_targets(&mut rng, next_token, next_binding),
                            ),
                            number(),
                            ScopeOwner::Node(NodeRef::new(StableId::new_v7(6, 0, u64::from(j)))),
                        ))
                    }
                    1 if !ids.is_empty() => BindingCommand::Replace {
                        binding: ids[rng.below(ids.len() as u64) as usize],
                        source: expression(
                            rng.next() % 1_000_000 * 4,
                            &random_targets(&mut rng, next_token, next_binding),
                        ),
                        expected_type: number(),
                    },
                    _ if !ids.is_empty() => {
                        BindingCommand::Delete(ids[rng.below(ids.len() as u64) as usize])
                    }
                    _ => continue,
                };
                if let Err(crate::binding_commands::BindingCommandError::Cycle(_)) =
                    command.apply(&mut bindings, &context)
                {
                    cycles += 1;
                }
            }
            steps += 1;
            assert_views_match(&registry, &bindings, &mut rng, next_token, next_binding);
        }
        // 2. orphan command(저장소의 live 모음이 바뀜).
        let mut store = BindingStore::new(bindings);
        for _ in 0..20 {
            let world = World(&registry);
            let live: Vec<ValueBinding> = store.live().iter().cloned().collect();
            let orphans: Vec<OrphanedBinding> = store.orphans().cloned().collect();
            let command = if rng.below(2) == 0 && !live.is_empty() {
                let binding = live[rng.below(live.len() as u64) as usize].clone();
                OrphanCommand::Orphanize(OrphanedBinding::new(binding, OrphanCause::TargetMissing))
            } else if !orphans.is_empty() {
                let orphan = &orphans[rng.below(orphans.len() as u64) as usize];
                if rng.below(4) == 0 {
                    OrphanCommand::Delete {
                        binding: orphan.id(),
                    }
                } else {
                    OrphanCommand::Reconnect {
                        binding: orphan.id(),
                        source: expression(
                            rng.next() % 1_000_000 * 4,
                            &random_targets(&mut rng, next_token, next_binding),
                        ),
                        expected_type: number(),
                    }
                }
            } else {
                continue;
            };
            if let Err(crate::orphan_commands::OrphanCommandError::Cycle(_)) =
                command.apply(&mut store, &registry, &world)
            {
                cycles += 1;
            }
            steps += 1;
            assert_views_match(&registry, store.live(), &mut rng, next_token, next_binding);
        }
    }
    assert!(steps >= 1_500, "충분한 편집: {steps}");
    assert!(cycles >= 20, "cycle 거부 경로도 지남: {cycles}");
}

/// 측정 하나: view 경로와 전체 재구성 경로 평균.
fn report(label: &str, bindings: u32, view: std::time::Duration, built: std::time::Duration) {
    println!(
        "{label}, 토큰 {TOKENS}·binding {bindings}: view {view:?}, 전체 재구성(이전) {built:?}"
    );
}

#[test]
#[ignore = "release 측정: cargo test -p stellar-bindings --release graph_view_tests -- --ignored --nocapture"]
fn measure_command_validate() {
    let mut binding_times = Vec::new();
    let mut deep_times = Vec::new();
    let mut token_times = Vec::new();
    let mut orphan_times = Vec::new();
    for bindings in [1_000u32, 10_000] {
        let (registry, store) = fixture(bindings);
        let world = World(&registry);
        let context = BindingContext {
            tokens: &registry,
            others: &world,
            targets: &AnyTarget,
        };
        let live = store.live();
        // (a) binding 하나의 source를 토큰 하나를 읽도록 바꿈.
        let id = BindingId::new(StableId::new_v7(5, 0, u64::from(bindings / 2)));
        let token_source = expression(
            0,
            &[ReferenceTarget::Token(SyntheticGraph::token_ref(
                TOKENS / 2,
            ))],
        );
        let replace = BindingCommand::Replace {
            binding: id,
            source: token_source.clone(),
            expected_type: number(),
        };
        replace.validate(live, &context).expect("유효");
        let view = mean(2_000, || replace.validate(live, &context).expect("유효"));
        let node = DependencyNode::Property(SyntheticGraph::property(bindings / 2));
        let deps = BTreeSet::from([DependencyNode::Token(SyntheticGraph::token_ref(TOKENS / 2))]);
        let built = mean(20, || {
            assert!(DependencyGraph::build(&registry, live)
                .cycle_if_dependencies(&node, &deps)
                .is_none());
        });
        report("binding Replace validate(토큰 참조)", bindings, view, built);
        binding_times.push(view);
        // (b) 같은 binding을 원래 참조(앞선 속성 포함)를 그대로 둔 새 source로 바꿈: BFS가 도달 범위를 모두 훑음.
        let original = live.get(id).expect("binding").source().clone();
        let deep = BindingCommand::Replace {
            binding: id,
            source: original,
            expected_type: number(),
        };
        deep.validate(live, &context).expect("유효");
        let view_deep = mean(200, || deep.validate(live, &context).expect("유효"));
        report(
            "binding Replace validate(원래 참조 유지)",
            bindings,
            view_deep,
            built,
        );
        deep_times.push(view_deep);
        // (c) 토큰 default expression 교체.
        let set_expression = TokenCommand::SetExpression {
            token: SyntheticGraph::token_ref(TOKENS / 2),
            expression: expression(
                0,
                &[ReferenceTarget::Token(SyntheticGraph::token_ref(
                    TOKENS / 4,
                ))],
            ),
        };
        set_expression
            .validate(&registry, &store, &world)
            .expect("유효");
        let view_token = mean(2_000, || {
            set_expression
                .validate(&registry, &store, &world)
                .expect("유효")
        });
        let references = BTreeSet::from([SyntheticGraph::token_ref(TOKENS / 4)]);
        let built_token = mean(200, || {
            assert!(TokenDependencyGraph::build(&registry)
                .cycle_if_references(SyntheticGraph::token_ref(TOKENS / 2), &references)
                .is_none());
        });
        report(
            "토큰 SetExpression validate",
            bindings,
            view_token,
            built_token,
        );
        token_times.push(view_token);
        // (d) orphan 재연결.
        let mut orphaned = store.clone();
        let binding = orphaned.remove_live(id).expect("live");
        orphaned.insert_orphan(OrphanedBinding::new(binding, OrphanCause::TargetMissing));
        let reconnect = OrphanCommand::Reconnect {
            binding: id,
            source: token_source,
            expected_type: number(),
        };
        reconnect
            .validate(&orphaned, &registry, &world)
            .expect("유효");
        let view_orphan = mean(2_000, || {
            reconnect
                .validate(&orphaned, &registry, &world)
                .expect("유효")
        });
        let built_orphan = mean(20, || {
            assert!(DependencyGraph::build(&registry, orphaned.live())
                .cycle_if_dependencies(&node, &deps)
                .is_none());
        });
        report(
            "orphan Reconnect validate",
            bindings,
            view_orphan,
            built_orphan,
        );
        orphan_times.push(view_orphan);
    }
    for (label, times) in [
        ("binding Replace", &binding_times),
        ("토큰 SetExpression", &token_times),
        ("orphan Reconnect", &orphan_times),
    ] {
        assert!(
            times[1] <= times[0] * 2,
            "{label}: binding 10배에서 시간 2배 이하 ({:?} → {:?})",
            times[0],
            times[1]
        );
    }
    println!(
        "원래 참조 유지 Replace 비율: {:.2}배",
        deep_times[1].as_secs_f64() / deep_times[0].as_secs_f64()
    );
}
