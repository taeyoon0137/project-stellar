//! binding 저장소 index release 측정 (XVI.4.8).
//!
//! `cargo test -p stellar-bindings --release store_bench -- --ignored --nocapture`로 실행합니다. synthetic fixture
//! (VIII.9.1, 토큰 1,000 + binding 1,000·10,000)에서 토큰 Delete validate와 orphan 1,000개가 있는 open 경로를 잽니다.
//! 비교를 위해 index 이전 방식(모든 binding source를 훑는 inbound 검사, orphan 선형 검색 참조 타입)도 함께 잽니다.
#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use stellar_core_types::property::PropertyPath;
use stellar_core_types::reference::TokenRef;
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::FiniteF32;
use stellar_expressions::ast::{ExprNode, ExprNodeId, ExpressionAst, ReferenceTarget};
use stellar_expressions::typecheck::ReferenceTypes;
use stellar_expressions::types::{ExpressionType, TypeSpec};

use crate::binding::{BindingId, ValueBinding};
use crate::fallback::{resolve_stored_defaults, StoredDefault};
use crate::graph::DependencyNode;
use crate::orphan::{OrphanCause, OrphanedBinding};
use crate::registry::TokenRegistry;
use crate::scheduler::{SourceInputs, StaticSources};
use crate::store::BindingStore;
use crate::synthetic::{generate, SyntheticConfig, SyntheticGraph, SYNTHETIC_ENVIRONMENT};
use crate::token::TokenTypes;
use crate::token_commands::{TokenCommand, TokenOrphanPlan};

pub(crate) const TOKENS: u32 = 1_000;

/// 토큰 1,000개와 binding `bindings`개 synthetic fixture.
pub(crate) fn fixture(bindings: u32) -> (TokenRegistry, BindingStore) {
    let config = SyntheticConfig {
        seed: 0x5854_5649_0408,
        tokens: TOKENS,
        bindings,
        edges: u64::from(TOKENS) * 2 + u64::from(bindings) * 2,
        max_token_refs: 3,
        environment_one_in: 16,
    };
    let (registry, live) = generate(config)
        .expect("설정")
        .materialize()
        .expect("실체화");
    (registry, BindingStore::new(live))
}

/// `rounds`번 실행한 평균.
pub(crate) fn mean(rounds: u32, mut run: impl FnMut()) -> Duration {
    run();
    let started = Instant::now();
    for _ in 0..rounds {
        run();
    }
    started.elapsed() / rounds
}

/// index 이전 inbound 검사: 모든 live·orphan binding source를 훑습니다.
fn scanned_plan(
    registry: &TokenRegistry,
    store: &BindingStore,
    token: TokenRef,
) -> TokenOrphanPlan {
    let target = ReferenceTarget::Token(token);
    TokenOrphanPlan {
        referrers: registry.referrers(token),
        bindings: store
            .live()
            .iter()
            .chain(store.orphans().map(OrphanedBinding::binding))
            .filter(|binding| binding.refers_to(&target))
            .map(ValueBinding::id)
            .collect::<BTreeSet<BindingId>>(),
    }
}

#[test]
#[ignore = "release 측정: cargo test -p stellar-bindings --release store_bench -- --ignored --nocapture"]
fn measure_token_delete_validate() {
    let token = SyntheticGraph::token_ref(TOKENS / 2);
    let others = BTreeMap::<ReferenceTarget, TypeSpec>::new();
    let mut times = Vec::new();
    for bindings in [1_000u32, 10_000] {
        let (registry, store) = fixture(bindings);
        let plan = TokenOrphanPlan::current(&registry, &store, token);
        assert_eq!(
            plan,
            scanned_plan(&registry, &store, token),
            "index = 전체 순회"
        );
        let command = TokenCommand::Delete {
            token,
            orphan_plan: Some(plan.clone()),
        };
        command.validate(&registry, &store, &others).expect("유효");
        let indexed = mean(2_000, || {
            command.validate(&registry, &store, &others).expect("유효");
        });
        let scanned = mean(200, || {
            assert_eq!(scanned_plan(&registry, &store, token), plan);
        });
        println!(
            "토큰 Delete validate, 토큰 {TOKENS}·binding {bindings} (inbound binding {}): index {indexed:?}, 전체 순회(이전) {scanned:?}",
            plan.bindings.len()
        );
        times.push(indexed);
    }
    assert!(
        times[1] <= times[0] * 2,
        "binding 10배에서 시간 2배 이하: {:?} → {:?}",
        times[0],
        times[1]
    );
}

/// index 이전 참조 타입: Property 참조마다 orphan을 선형 검색합니다(옛 `StoreTypes`).
struct ScannedTypes<'a> {
    tokens: &'a TokenRegistry,
    store: &'a BindingStore,
    sources: &'a dyn SourceInputs,
}

impl ReferenceTypes for ScannedTypes<'_> {
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

/// index를 쓰는 참조 타입(`StoreTypes`와 같은 규칙).
struct IndexedTypes<'a>(ScannedTypes<'a>);

impl ReferenceTypes for IndexedTypes<'_> {
    fn reference_type(&self, target: &ReferenceTarget) -> Option<TypeSpec> {
        if let ReferenceTarget::Property(path) = target {
            if self.0.store.live().by_target(path).is_none() {
                return self
                    .0
                    .store
                    .orphan_by_target(path)
                    .map(OrphanedBinding::last_valid_type)
                    .or_else(|| {
                        self.0
                            .sources
                            .source_type(&DependencyNode::from_target(target))
                    });
            }
        }
        self.0.reference_type(target)
    }
}

fn number() -> TypeSpec {
    TypeSpec::required(ExpressionType::Number)
}

/// binding 마지막 1,000개를 orphan으로 옮기고, 각 orphan의 저장된 기본값이 직전 orphan target을 읽게 합니다.
fn with_orphans(store: &mut BindingStore, bindings: u32) -> BTreeMap<PropertyPath, StoredDefault> {
    let mut stored = BTreeMap::new();
    let mut previous: Option<PropertyPath> = None;
    for j in bindings - 1_000..bindings {
        let id = BindingId::new(stellar_core_types::ids::StableId::new_v7(
            5,
            0,
            u64::from(j),
        ));
        let binding = store.remove_live(id).expect("live");
        let target = binding.target().clone();
        store.insert_orphan(OrphanedBinding::new(binding, OrphanCause::TargetMissing));
        let node = ExprNodeId::new(stellar_core_types::ids::StableId::new_v7(
            7,
            0,
            u64::from(j),
        ));
        let expression = match &previous {
            Some(path) => ExpressionAst::new(
                node,
                vec![ExprNode::reference(
                    node,
                    ReferenceTarget::Property(path.clone()),
                )],
            ),
            None => ExpressionAst::new(
                node,
                vec![ExprNode::literal(
                    node,
                    TypedValue::Number(FiniteF32::new(1.0).expect("유한")),
                )],
            ),
        }
        .expect("유효");
        stored.insert(
            target.clone(),
            StoredDefault {
                declared_type: number(),
                expression: expression.to_canonical_bytes().expect("직렬화"),
            },
        );
        previous = Some(target);
    }
    stored
}

#[test]
#[ignore = "release 측정: cargo test -p stellar-bindings --release store_bench -- --ignored --nocapture"]
fn measure_open_with_orphans() {
    let mut sources = StaticSources::new();
    for key in SYNTHETIC_ENVIRONMENT {
        sources.insert(
            DependencyNode::Environment(key),
            (
                number(),
                Some(TypedValue::Number(FiniteF32::new(1.0).expect("유한"))),
            ),
        );
    }
    for bindings in [1_000u32, 10_000] {
        let (registry, mut store) = fixture(bindings);
        let stored = with_orphans(&mut store, bindings);
        let scanned = ScannedTypes {
            tokens: &registry,
            store: &store,
            sources: &sources,
        };
        let indexed = IndexedTypes(ScannedTypes {
            tokens: &registry,
            store: &store,
            sources: &sources,
        });
        let a = resolve_stored_defaults(&store, &stored, &indexed);
        let b = resolve_stored_defaults(&store, &stored, &scanned);
        assert_eq!(a.diagnostics(), b.diagnostics());
        assert!(a.diagnostics().is_empty(), "저장된 기본값은 모두 유효");
        let resolve_indexed = mean(20, || {
            resolve_stored_defaults(&store, &stored, &indexed);
        });
        let resolve_scanned = mean(5, || {
            resolve_stored_defaults(&store, &stored, &scanned);
        });
        let open = mean(5, || {
            crate::scheduler::BindingEvaluator::open_with_stored_defaults(
                &registry, &store, &stored, &sources,
            )
            .expect("열기");
        });
        println!(
            "orphan 1,000개 open, 토큰 {TOKENS}·binding {bindings}: 저장 기본값 해석 index {resolve_indexed:?}, 선형 검색(이전) {resolve_scanned:?}, open 전체 {open:?}"
        );
    }
}
