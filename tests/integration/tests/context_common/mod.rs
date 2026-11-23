//! Context 해석(document)을 binding graph 입력(bindings)으로 옮기는 시험 공용 도우미.
//!
//! 두 crate는 서로 의존하지 않으므로(crate graph) 해석 결과를 StableId 경로표로 옮기는 일은 둘을 함께 쓰는 쪽이 합니다.

#![allow(dead_code)]

use std::collections::BTreeMap;

use stellar_bindings::binding::{BindingId, BindingSet, ScopeOwner, ValueBinding};
use stellar_bindings::context::{
    ContextGraph, ContextHopInfo, ContextKeyInput, ContextSource, ProvisionInput,
};
use stellar_bindings::graph::{ContextProvisionNode, DependencyNode, Producer};
use stellar_bindings::registry::TokenRegistry;
use stellar_bindings::scheduler::{BindingEvaluator, ProducerSpec, RouteChange, StaticSources};
use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_core_types::reference::NodeRef;
use stellar_core_types::typed_value::TypedValue;
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::context::resolve::{ContextOrigin, ContextResolution, ContextResolver};
use stellar_document::context::set::ContextKeys;
use stellar_document::context::validate::{all_sites, ContextConsumer};
use stellar_document::model::DocumentModel;
use stellar_expressions::vm::EvalLimits;

/// 해석 출처 → binding graph 출처.
pub fn source(resolution: &ContextResolution) -> ContextHopInfo {
    ContextHopInfo {
        source: match &resolution.origin {
            ContextOrigin::Default => ContextSource::Default,
            ContextOrigin::Provision {
                provision,
                provider,
                instances,
            } => ContextSource::Provision {
                provision: *provision,
                provider: *provider,
                instances: instances.clone(),
            },
        },
        via_slots: resolution.via_slots.clone(),
    }
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

/// 소비 하나의 key별 해석.
pub fn consumer_hops(
    resolver: &ContextResolver<'_>,
    consumer: &ContextConsumer,
) -> BTreeMap<StableId, ContextHopInfo> {
    consumer
        .reads()
        .into_iter()
        .map(|key| {
            let resolution = resolver.resolve(key, &consumer.site).expect("해석");
            (key, source(&resolution))
        })
        .collect()
}

/// 문서 Context와 소비 binding으로 graph 입력을 만듭니다. provision은 그 provider가 펼쳐지는 site마다 하나입니다.
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
    let keys = registry
        .iter()
        .map(|key| ContextKeyInput {
            key: key.id(),
            value_type: key.value_type(),
            default: key.default_expression().ast().clone(),
        })
        .collect();
    let mut provisions = Vec::new();
    for site in all_sites(model, library) {
        for provision in model.context().provisions_at(site.node) {
            let Some(key) = registry.key(provision.key()) else {
                continue;
            };
            let hops = provision
                .reads()
                .into_iter()
                .map(|read| {
                    let resolution = resolver
                        .resolve_provision_read(provision, &site, read)
                        .expect("provision 읽기 해석");
                    (read, source(&resolution))
                })
                .collect();
            provisions.push(ProvisionInput {
                node: ContextProvisionNode {
                    provision: provision.id(),
                    instances: site.instances.clone(),
                },
                provider: provision.provider(),
                key: provision.key(),
                value_type: key.value_type(),
                value: provision.value().clone(),
                hops,
            });
        }
    }
    let consumers = consumers
        .iter()
        .map(|consumer| (consumer_node(consumer), consumer_hops(&resolver, consumer)))
        .collect();
    ContextGraph {
        keys,
        provisions,
        consumers,
    }
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

fn routes(hops: &BTreeMap<StableId, ContextHopInfo>) -> BTreeMap<StableId, DependencyNode> {
    hops.iter()
        .map(|(key, hop)| (*key, hop.source.node(*key)))
        .collect()
}

/// graph 입력 두 개의 차이를 증분 변경으로 만듭니다. key·provision producer는 바뀐 것만, 소비 binding은 `affected`만
/// 경로표를 다시 씁니다(무효화 조건이 고른 소비만).
pub fn route_change(
    before: &ContextGraph,
    after: &ContextGraph,
    affected: &[StableId],
) -> RouteChange {
    let mut change = RouteChange::default();
    let old_keys: BTreeMap<StableId, &ContextKeyInput> =
        before.keys.iter().map(|k| (k.key, k)).collect();
    for key in &after.keys {
        if old_keys.get(&key.key).map(|old| *old != key) != Some(false) {
            change.upsert.push((
                ProducerSpec {
                    node: DependencyNode::ContextKey(key.key),
                    producer: Producer::ContextDefault(key.key),
                    value_type: Some(key.value_type),
                    expression: key.default.clone(),
                },
                None,
            ));
        }
    }
    let old: BTreeMap<&ContextProvisionNode, &ProvisionInput> =
        before.provisions.iter().map(|p| (&p.node, p)).collect();
    let new: BTreeMap<&ContextProvisionNode, &ProvisionInput> =
        after.provisions.iter().map(|p| (&p.node, p)).collect();
    for (node, provision) in &new {
        if old.get(node).map(|p| p == provision) != Some(true) {
            change.upsert.push((
                ProducerSpec {
                    node: DependencyNode::ContextProvision((*node).clone()),
                    producer: Producer::ContextProvision(node.provision),
                    value_type: Some(provision.value_type),
                    expression: provision.value.clone(),
                },
                Some(routes(&provision.hops)),
            ));
        }
    }
    for node in old.keys() {
        if !new.contains_key(node) {
            change
                .remove
                .push(DependencyNode::ContextProvision((*node).clone()));
        }
    }
    for (node, hops) in &after.consumers {
        let DependencyNode::Property(path) = node else {
            continue;
        };
        if affected.contains(&path.root()) {
            change.reroute.push((node.clone(), routes(hops)));
        }
    }
    change
}
