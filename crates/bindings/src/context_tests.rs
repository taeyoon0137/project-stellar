//! Context binding graph 시험(XXX.7.3·XXX.7.6·XXX.7.7·XXX.7.8).

use std::collections::BTreeMap;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_core_types::reference::NodeRef;
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::FiniteF32;
use stellar_expressions::ast::{ExprNode, ExprNodeId, ExpressionAst, ReferenceTarget};
use stellar_expressions::ops::BinaryOp;
use stellar_expressions::types::{ExpressionType, TypeSpec};
use stellar_expressions::vm::EvalLimits;

use super::*;
use crate::binding::{BindingId, BindingSet, ScopeOwner, ValueBinding};
use crate::graph::{ContextProvisionNode, DependencyKind, DependencyNode, Producer};
use crate::orphan::{BindingStore, BindingWorld, OrphanCause};
use crate::registry::TokenRegistry;
use crate::scheduler::{BindingEvaluator, RouteChange, SchedulerError, StaticSources};

fn sid(n: u64) -> StableId {
    StableId::new_v7(n, 0x0c0c, n)
}

fn num(v: f32) -> TypedValue {
    TypedValue::Number(FiniteF32::new(v).unwrap())
}

fn number() -> TypeSpec {
    TypeSpec::required(ExpressionType::Number)
}

fn literal(seed: u64, v: f32) -> ExpressionAst {
    let id = ExprNodeId::new(sid(seed));
    ExpressionAst::new(id, vec![ExprNode::literal(id, num(v))]).unwrap()
}

fn read(seed: u64, key: StableId) -> ExpressionAst {
    let id = ExprNodeId::new(sid(seed));
    ExpressionAst::new(
        id,
        vec![ExprNode::reference(id, ReferenceTarget::Context(key))],
    )
    .unwrap()
}

/// `Context(key) + add`.
fn read_plus(seed: u64, key: StableId, add: f32) -> ExpressionAst {
    let r = ExprNodeId::new(sid(seed));
    let l = ExprNodeId::new(sid(seed + 1));
    let root = ExprNodeId::new(sid(seed + 2));
    ExpressionAst::new(
        root,
        vec![
            ExprNode::reference(r, ReferenceTarget::Context(key)),
            ExprNode::literal(l, num(add)),
            ExprNode::binary(root, BinaryOp::Add, r, l),
        ],
    )
    .unwrap()
}

fn target(n: u64) -> PropertyPath {
    PropertyPath::new(sid(n)).push(PropertyKey(1))
}

fn binding(n: u64, source: ExpressionAst) -> ValueBinding {
    ValueBinding::new(
        BindingId::new(sid(5_000 + n)),
        target(n),
        source,
        number(),
        ScopeOwner::Node(NodeRef::new(sid(n))),
    )
}

const K: u64 = 900;
const K2: u64 = 901;

fn pnode(provision: u64, instances: &[u64]) -> ContextProvisionNode {
    ContextProvisionNode {
        provision: sid(provision),
        instances: instances.iter().map(|n| sid(*n)).collect(),
    }
}

fn hop_to(provision: u64, provider: u64) -> ContextHopInfo {
    ContextHopInfo {
        source: ContextSource::Provision {
            provision: sid(provision),
            provider: sid(provider),
            instances: Vec::new(),
        },
        via_slots: Vec::new(),
    }
}

fn hop_default() -> ContextHopInfo {
    ContextHopInfo {
        source: ContextSource::Default,
        via_slots: Vec::new(),
    }
}

fn provision(
    id: u64,
    provider: u64,
    key: u64,
    value: ExpressionAst,
    hops: &[(u64, ContextHopInfo)],
) -> ProvisionInput {
    ProvisionInput {
        node: pnode(id, &[]),
        provider: sid(provider),
        key: sid(key),
        value_type: number(),
        value,
        hops: hops.iter().map(|(k, h)| (sid(*k), h.clone())).collect(),
    }
}

fn key(id: u64, default: f32) -> ContextKeyInput {
    ContextKeyInput {
        key: sid(id),
        value_type: number(),
        default: literal(id * 10, default),
    }
}

/// key K(default 1). provision P1(root 10) = 5, provision P2(frame 20) = Context(K) + 100(바깥 = P1).
/// 소비: binding 30(→ P2), binding 40(→ P1), binding 50(→ default).
fn graph() -> ContextGraph {
    let mut consumers = BTreeMap::new();
    consumers.insert(
        DependencyNode::Property(target(30)),
        BTreeMap::from([(sid(K), hop_to(2, 20))]),
    );
    consumers.insert(
        DependencyNode::Property(target(40)),
        BTreeMap::from([(sid(K), hop_to(1, 10))]),
    );
    consumers.insert(
        DependencyNode::Property(target(50)),
        BTreeMap::from([(sid(K), hop_default())]),
    );
    ContextGraph {
        keys: vec![key(K, 1.0)],
        provisions: vec![
            provision(1, 10, K, literal(11, 5.0), &[]),
            provision(
                2,
                20,
                K,
                read_plus(12, sid(K), 100.0),
                &[(K, hop_to(1, 10))],
            ),
        ],
        consumers,
    }
}

fn bindings() -> BindingSet {
    let mut set = BindingSet::new();
    for n in [30, 40, 50] {
        set.insert(binding(n, read(100 + n, sid(K)))).unwrap();
    }
    set
}

fn evaluator(context: &ContextGraph) -> BindingEvaluator {
    BindingEvaluator::with_context(
        &TokenRegistry::new(),
        &bindings(),
        context,
        &StaticSources::new(),
        EvalLimits::DEFAULT,
    )
    .unwrap()
}

fn value(evaluator: &BindingEvaluator, n: u64) -> TypedValue {
    evaluator
        .value(&DependencyNode::Property(target(n)))
        .unwrap()
        .clone()
        .unwrap()
        .unwrap()
}

#[test]
fn context_nodes_are_common_graph_nodes_and_consumers_follow_routes() {
    // 검증 기준(XXX.7.3): dependency 종류 fixture. key·provision은 공통 node이고 소비 binding은 해석된 node를 의존합니다.
    let context = graph();
    let evaluator = evaluator(&context);
    let graph = evaluator.graph();
    let kinds: Vec<DependencyKind> = graph.nodes().iter().map(DependencyNode::kind).collect();
    assert!(kinds.contains(&DependencyKind::ContextKey));
    assert!(kinds.contains(&DependencyKind::ContextProvision));
    let deps = |node: DependencyNode| -> Vec<DependencyNode> {
        let index = graph.index_of(&node).unwrap();
        graph
            .dependencies(index)
            .iter()
            .map(|d| graph.node(*d).clone())
            .collect()
    };
    assert_eq!(
        deps(DependencyNode::Property(target(30))),
        vec![DependencyNode::ContextProvision(pnode(2, &[]))]
    );
    assert_eq!(
        deps(DependencyNode::ContextProvision(pnode(2, &[]))),
        vec![DependencyNode::ContextProvision(pnode(1, &[]))]
    );
    assert_eq!(
        deps(DependencyNode::Property(target(50))),
        vec![DependencyNode::ContextKey(sid(K))]
    );
    assert_eq!(
        graph.producer(graph.index_of(&DependencyNode::ContextKey(sid(K))).unwrap()),
        Some(Producer::ContextDefault(sid(K)))
    );
    // 값: 같은 key 재읽기는 바깥(P1) 값, 경로 없는 소비는 default.
    assert_eq!(value(&evaluator, 30), num(105.0));
    assert_eq!(value(&evaluator, 40), num(5.0));
    assert_eq!(value(&evaluator, 50), num(1.0));
    // 경로표 없이 만든 graph에서는 Context(K)가 key default node입니다.
    let plain = BindingEvaluator::new(
        &TokenRegistry::new(),
        &bindings(),
        &StaticSources::from([(
            DependencyNode::ContextKey(sid(K)),
            (number(), Some(num(7.0))),
        )]),
    )
    .unwrap();
    assert_eq!(value(&plain, 30), num(7.0));
}

#[test]
fn mutual_context_reads_are_rejected_as_a_cycle() {
    // 검증 기준(XXX.7.5): K1↔K2 cycle은 binding cycle detector로 거부.
    let context = ContextGraph {
        keys: vec![key(K, 1.0), key(K2, 2.0)],
        provisions: vec![
            provision(1, 10, K, read(11, sid(K2)), &[(K2, hop_to(2, 10))]),
            provision(2, 10, K2, read(12, sid(K)), &[(K, hop_to(1, 10))]),
        ],
        consumers: BTreeMap::new(),
    };
    let result = BindingEvaluator::with_context(
        &TokenRegistry::new(),
        &BindingSet::new(),
        &context,
        &StaticSources::new(),
        EvalLimits::DEFAULT,
    );
    assert!(matches!(result, Err(SchedulerError::Cycle(_))));
}

#[test]
fn rerouting_reevaluates_only_affected_consumers_in_topological_order() {
    // 검증 기준(XXX.7.6): 영향받는 소비 binding 값이 전체 재해석 결과와 같음, 무관 subtree 평가 0, 순서 결정적.
    let mut context = graph();
    let mut incremental = evaluator(&context);
    // 1) provision 삭제(P2): 소비 30은 P1로 다시 해석됩니다. 40·50은 영향 없음.
    context.provisions.retain(|p| p.node.provision != sid(2));
    context.consumers.insert(
        DependencyNode::Property(target(30)),
        BTreeMap::from([(sid(K), hop_to(1, 10))]),
    );
    let report = incremental
        .apply_context_change(
            RouteChange {
                remove: vec![DependencyNode::ContextProvision(pnode(2, &[]))],
                reroute: vec![(
                    DependencyNode::Property(target(30)),
                    BTreeMap::from([(sid(K), DependencyNode::ContextProvision(pnode(1, &[])))]),
                )],
                ..RouteChange::default()
            },
            &StaticSources::new(),
        )
        .unwrap();
    let full = evaluator(&context);
    for n in [30, 40, 50] {
        assert_eq!(value(&incremental, n), value(&full, n), "binding {n}");
    }
    assert_eq!(value(&incremental, 30), num(5.0));
    let evaluated: Vec<DependencyNode> = report
        .order
        .iter()
        .map(|i| incremental.node_at(*i).clone())
        .collect();
    assert_eq!(evaluated, vec![DependencyNode::Property(target(30))]);

    // 2) provision 값 변경(P1 = 9): P1을 읽는 30·40만, 위상 순서(P1 → 소비)로 평가합니다. default 소비 50은 0회.
    let report = incremental
        .apply_context_change(
            RouteChange {
                upsert: vec![(
                    crate::scheduler::ProducerSpec {
                        node: DependencyNode::ContextProvision(pnode(1, &[])),
                        producer: Producer::ContextProvision(sid(1)),
                        value_type: Some(number()),
                        expression: literal(13, 9.0),
                    },
                    Some(BTreeMap::new()),
                )],
                ..RouteChange::default()
            },
            &StaticSources::new(),
        )
        .unwrap();
    let evaluated: Vec<DependencyNode> = report
        .order
        .iter()
        .map(|i| incremental.node_at(*i).clone())
        .collect();
    assert_eq!(
        evaluated,
        vec![
            DependencyNode::ContextProvision(pnode(1, &[])),
            DependencyNode::Property(target(30)),
            DependencyNode::Property(target(40)),
        ]
    );
    assert_eq!(value(&incremental, 40), num(9.0));
    assert_eq!(value(&incremental, 50), num(1.0));

    // 3) cycle이 생기는 변경은 거부하고 상태를 바꾸지 않습니다.
    let before = value(&incremental, 30);
    let cyclic = incremental.apply_context_change(
        RouteChange {
            upsert: vec![(
                crate::scheduler::ProducerSpec {
                    node: DependencyNode::ContextProvision(pnode(1, &[])),
                    producer: Producer::ContextProvision(sid(1)),
                    value_type: Some(number()),
                    expression: read(14, sid(K)),
                },
                Some(BTreeMap::from([(
                    sid(K),
                    DependencyNode::ContextProvision(pnode(1, &[])),
                )])),
            )],
            ..RouteChange::default()
        },
        &StaticSources::new(),
    );
    assert!(matches!(cyclic, Err(SchedulerError::Cycle(_))));
    assert_eq!(value(&incremental, 30), before);
}

#[test]
fn provenance_records_key_and_provision_chain() {
    // 검증 기준(XXX.7.7): 최종값 → provision → key chain, default 해석·slot 경유 표시.
    let mut context = graph();
    context.consumers.insert(
        DependencyNode::Property(target(40)),
        BTreeMap::from([(
            sid(K),
            ContextHopInfo {
                source: ContextSource::Provision {
                    provision: sid(1),
                    provider: sid(10),
                    instances: Vec::new(),
                },
                via_slots: vec![sid(77)],
            },
        )]),
    );
    let evaluator = evaluator(&context);
    let trace = evaluator
        .context_provenance(&DependencyNode::Property(target(30)), &context)
        .unwrap();
    assert_eq!(trace.providers(), vec![sid(20), sid(10)]);
    assert_eq!(trace.hops[0].key, sid(K));
    assert_eq!(
        trace.hops[0].read,
        DependencyNode::ContextProvision(pnode(2, &[]))
    );
    assert_eq!(
        trace.hops[1].reader,
        DependencyNode::ContextProvision(pnode(2, &[]))
    );
    let default = evaluator
        .context_provenance(&DependencyNode::Property(target(50)), &context)
        .unwrap();
    assert_eq!(default.hops.len(), 1);
    assert_eq!(default.hops[0].source, ContextSource::Default);
    assert_eq!(default.hops[0].provider, None);
    assert_eq!(default.hops[0].read, DependencyNode::ContextKey(sid(K)));
    let slot = evaluator
        .context_provenance(&DependencyNode::Property(target(40)), &context)
        .unwrap();
    assert_eq!(slot.hops[0].via_slots, vec![sid(77)]);
}

struct NoWorld;

impl BindingWorld for NoWorld {
    fn target_type(&self, _: &PropertyPath) -> Option<TypeSpec> {
        Some(number())
    }
    fn reference_type(&self, _: &ReferenceTarget) -> Option<TypeSpec> {
        None
    }
}

#[test]
fn deleting_a_consumed_key_orphans_consumer_bindings() {
    // 검증 기준(XXX.7.8): 소비가 있는 key 삭제는 소비 binding을 OrphanedBinding으로 바꿈.
    let mut store = BindingStore::new(bindings());
    let live = BTreeMap::from([(sid(K), number())]);
    let world = ContextBindingWorld {
        inner: &NoWorld,
        keys: &live,
    };
    assert!(store.orphan_broken(&world).is_empty());
    let deleted = BTreeMap::new();
    let world = ContextBindingWorld {
        inner: &NoWorld,
        keys: &deleted,
    };
    let orphaned = store.orphan_broken(&world);
    assert_eq!(orphaned.len(), 3);
    let orphan = store.orphan(orphaned[0]).unwrap();
    assert!(matches!(
        orphan.cause(),
        OrphanCause::SourceMissing { target: ReferenceTarget::Context(k), .. } if *k == sid(K)
    ));
    // binding은 지워지지 않고 원래 AST를 보존합니다.
    assert_eq!(store.len(), 3);
}
