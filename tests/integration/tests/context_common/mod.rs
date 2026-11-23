//! Context 해석(document)을 binding graph 입력(bindings)으로 옮기는 시험 공용 도우미.
//!
//! 변환 자체는 제품 경로 `stellar_prototype::context`(ADR-0114 결정 5)를 그대로 부르고, 이 도우미는 시험용 binding
//! target(binding StableId를 root로 쓰는 경로)과 평가기 조립만 둡니다.

#![allow(dead_code)]

use std::collections::BTreeMap;

use stellar_bindings::binding::{BindingId, BindingSet, ScopeOwner, ValueBinding};
use stellar_bindings::context::{ContextGraph, ContextHopInfo};
use stellar_bindings::graph::DependencyNode;
use stellar_bindings::registry::TokenRegistry;
use stellar_bindings::scheduler::{BindingEvaluator, RouteChange, StaticSources};
use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_core_types::reference::NodeRef;
use stellar_core_types::typed_value::TypedValue;
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::context::resolve::{ContextResolution, ContextResolver};
use stellar_document::context::set::ContextKeys;
use stellar_document::context::validate::ContextConsumer;
use stellar_document::model::DocumentModel;
use stellar_expressions::vm::EvalLimits;

/// 해석 출처 → binding graph 출처(제품 경로).
pub fn source(resolution: &ContextResolution) -> ContextHopInfo {
    stellar_prototype::context::hop_info(resolution)
}

/// 소비 binding의 graph target(binding StableId를 root로 쓰는 시험용 경로).
pub fn target(consumer: &ContextConsumer) -> PropertyPath {
    PropertyPath::new(consumer.binding).push(PropertyKey(1))
}

/// 소비 binding의 graph node.
pub fn consumer_node(consumer: &ContextConsumer) -> DependencyNode {
    DependencyNode::Property(target(consumer))
}

/// 소비 binding 모음.
pub fn bindings(consumers: &[ContextConsumer]) -> BindingSet {
    let mut set = BindingSet::new();
    for consumer in consumers {
        set.insert(ValueBinding::new(
            BindingId::new(consumer.binding),
            target(consumer),
            consumer.source.clone(),
            consumer.expected,
            ScopeOwner::Node(NodeRef::new(consumer.site.node)),
        ))
        .expect("binding");
    }
    set
}

/// 소비 하나의 key별 해석(제품 경로).
pub fn consumer_hops(
    resolver: &ContextResolver<'_>,
    consumer: &ContextConsumer,
) -> BTreeMap<StableId, ContextHopInfo> {
    stellar_prototype::context::consumer_hops(resolver, consumer).expect("해석")
}

/// 문서 Context와 소비 binding으로 graph 입력을 만듭니다(제품 경로 `stellar_prototype::context::context_graph`).
pub fn context_graph(
    model: &DocumentModel,
    library: &ComponentLibrary,
    extra_keys: &[&stellar_document::context::set::ContextSet],
    consumers: &[ContextConsumer],
) -> ContextGraph {
    let resolver = ContextResolver::new(model, library);
    let mut registry = ContextKeys::new(model.context());
    for set in extra_keys {
        registry = registry.with(set);
    }
    let consumers: Vec<(DependencyNode, ContextConsumer)> = consumers
        .iter()
        .map(|consumer| (consumer_node(consumer), consumer.clone()))
        .collect();
    stellar_prototype::context::context_graph(&resolver, &registry, &consumers).expect("graph 입력")
}

/// 전체 평가기.
pub fn evaluator(graph: &ContextGraph, consumers: &[ContextConsumer]) -> BindingEvaluator {
    BindingEvaluator::with_context(
        &TokenRegistry::new(),
        &bindings(consumers),
        graph,
        &StaticSources::new(),
        EvalLimits::DEFAULT,
    )
    .expect("평가기")
}

/// 소비 binding 값.
pub fn value(evaluator: &BindingEvaluator, consumer: &ContextConsumer) -> Option<TypedValue> {
    evaluator
        .value(&consumer_node(consumer))
        .expect("graph node")
        .clone()
        .expect("값")
}

/// graph 입력 두 개의 차이를 증분 변경으로 만듭니다(제품 경로 `RouteChange::between`). `affected`는 무효화 조건이 고른
/// 소비 binding StableId입니다.
pub fn route_change(
    before: &ContextGraph,
    after: &ContextGraph,
    affected: &[StableId],
) -> RouteChange {
    let nodes = after
        .consumers
        .keys()
        .filter(|node| matches!(node, DependencyNode::Property(path) if affected.contains(&path.root())))
        .cloned()
        .collect();
    RouteChange::between(before, after, &nodes)
}
