//! 문서 Context 해석 → binding graph 입력 (XXX.7.3·XXX.7.6 제품 경로, ADR-0099 결정 3, ADR-0114 결정 5).
//!
//! 해석기(`stellar_document::context::resolve`)와 binding graph(`stellar_bindings::context`)는 서로 의존하지 않으므로
//! 해석 결과를 StableId 경로표([`ContextGraph`])로 옮기는 일은 두 crate를 함께 쓰는 이 crate가 합니다. 프리뷰·그래프
//! 진단(XXX.2.5)과 종단 시험이 같은 함수를 씁니다.
//!
//! - [`binding_consumers`]: binding 모음에서 `Context(K)`를 읽는 binding을 소비([`ContextConsumer`])로 고릅니다. 소비
//!   지점은 호출자가 정합니다(scene node면 [`ContextSite::scene`], 인스턴스 안이면 chain).
//! - [`context_graph`]: key default, 펼친 트리의 site마다 provision 값 producer(자기 값 식의 Context 읽기 경로표 포함),
//!   소비 producer의 key별 해석을 모읍니다. 증분 갱신은 `RouteChange::between`입니다.

use std::collections::{BTreeMap, BTreeSet};

use stellar_bindings::binding::{BindingSet, ValueBinding};
use stellar_bindings::context::{
    ContextGraph, ContextHopInfo, ContextKeyInput, ContextSource, ProvisionInput,
};
use stellar_bindings::graph::{ContextProvisionNode, DependencyNode};
use stellar_core_types::ids::StableId;
use stellar_document::context::invalidate::{affected_consumers, ContextChange};
use stellar_document::context::provision::context_reads;
use stellar_document::context::resolve::{
    ContextOrigin, ContextResolution, ContextResolver, ContextSite, ResolveError,
};
use stellar_document::context::set::ContextKeys;
use stellar_document::context::validate::{all_sites, ContextConsumer};

/// 해석 결과 → graph 경로표 항목(출처와 Slot 경유).
pub fn hop_info(resolution: &ContextResolution) -> ContextHopInfo {
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

/// 소비 하나의 key별 해석.
pub fn consumer_hops(
    resolver: &ContextResolver<'_>,
    consumer: &ContextConsumer,
) -> Result<BTreeMap<StableId, ContextHopInfo>, ResolveError> {
    consumer
        .reads()
        .into_iter()
        .map(|key| Ok((key, hop_info(&resolver.resolve(key, &consumer.site)?))))
        .collect()
}

/// `bindings`에서 Context를 읽는 binding의 (graph node, 소비)입니다(binding id 순). `site_of`가 `None`이면(해석할 위치가
/// 없는 binding) 고르지 않습니다. graph node는 binding target 속성 node입니다.
pub fn binding_consumers(
    bindings: &BindingSet,
    site_of: impl Fn(&ValueBinding) -> Option<ContextSite>,
) -> Vec<(DependencyNode, ContextConsumer)> {
    let mut list: Vec<&ValueBinding> = bindings
        .iter()
        .filter(|binding| !context_reads(binding.source()).is_empty())
        .collect();
    list.sort_by_key(|binding| binding.id());
    list.into_iter()
        .filter_map(|binding| {
            let site = site_of(binding)?;
            Some((
                DependencyNode::Property(binding.target().clone()),
                ContextConsumer {
                    binding: binding.id().stable_id(),
                    site,
                    expected: binding.expected_type(),
                    source: binding.source().clone(),
                },
            ))
        })
        .collect()
}

/// 문서 Context(해석기의 set)와 소비로 graph 입력을 만듭니다. provision은 그 provider가 펼쳐지는 site마다 하나이고,
/// registry에 없는 key의 provision은 넣지 않습니다(orphan, validator 몫). 같은 입력은 같은 결과입니다.
pub fn context_graph(
    resolver: &ContextResolver<'_>,
    keys: &ContextKeys<'_>,
    consumers: &[(DependencyNode, ContextConsumer)],
) -> Result<ContextGraph, ResolveError> {
    let keys_input = keys
        .iter()
        .map(|key| ContextKeyInput {
            key: key.id(),
            value_type: key.value_type(),
            default: key.default_expression().ast().clone(),
        })
        .collect();
    let mut provisions = Vec::new();
    for site in all_sites(resolver.model(), resolver.library()) {
        for provision in resolver.context().provisions_at(site.node) {
            let Some(key) = keys.key(provision.key()) else {
                continue;
            };
            let mut hops = BTreeMap::new();
            for read in provision.reads() {
                let resolution = resolver.resolve_provision_read(provision, &site, read)?;
                hops.insert(read, hop_info(&resolution));
            }
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
    let mut graph_consumers = BTreeMap::new();
    for (node, consumer) in consumers {
        graph_consumers.insert(node.clone(), consumer_hops(resolver, consumer)?);
    }
    Ok(ContextGraph {
        keys: keys_input,
        provisions,
        consumers: graph_consumers,
    })
}

/// 문서 변경 `change`가 해석 경로에 들어가는 소비의 graph node(XXX.7.6 무효화 조건, 문서 crate
/// `affected_consumers` 판정 그대로). `before`는 변경 전 문서의 해석기입니다. `RouteChange::between`의 `affected`입니다.
pub fn affected_nodes(
    before: &ContextResolver<'_>,
    change: &ContextChange,
    consumers: &[(DependencyNode, ContextConsumer)],
) -> BTreeSet<DependencyNode> {
    let plain: Vec<ContextConsumer> = consumers.iter().map(|(_, c)| c.clone()).collect();
    let touched: BTreeSet<StableId> = affected_consumers(before, change, &plain)
        .into_iter()
        .collect();
    consumers
        .iter()
        .filter(|(_, consumer)| touched.contains(&consumer.binding))
        .map(|(node, _)| node.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use stellar_bindings::binding::{BindingId, ScopeOwner};
    use stellar_bindings::registry::TokenRegistry;
    use stellar_bindings::scheduler::{BindingEvaluator, RouteChange, StaticSources};
    use stellar_core_types::property::{PropertyKey, PropertyPath};
    use stellar_core_types::reference::NodeRef;
    use stellar_core_types::typed_value::TypedValue;
    use stellar_core_types::value::FiniteF32;
    use stellar_document::context::fixtures::world::*;
    use stellar_document::context::fixtures::{context_read, number_type, sample_world, sid};
    use stellar_expressions::vm::EvalLimits;

    use super::*;

    fn num(value: f32) -> Option<TypedValue> {
        Some(TypedValue::Number(FiniteF32::new(value).unwrap()))
    }

    #[test]
    fn document_resolution_feeds_the_binding_graph_and_its_incremental_update() {
        // 검증 기준(XXX.7.3 남은 범위, ADR-0114 결정 5): 문서 해석에서 graph 입력을 만드는 제품 경로.
        let (mut model, library) = sample_world();
        let mut bindings = BindingSet::new();
        let target = PropertyPath::new(sid(FRAME)).push(PropertyKey(0x0303));
        bindings
            .insert(ValueBinding::new(
                BindingId::new(sid(7000)),
                target.clone(),
                context_read(7001, sid(N)),
                number_type(),
                ScopeOwner::Node(NodeRef::new(sid(FRAME))),
            ))
            .unwrap();
        let site_of = |b: &ValueBinding| Some(ContextSite::scene(b.target().root()));
        let consumers = binding_consumers(&bindings, site_of);
        assert_eq!(consumers.len(), 1);
        let keys_set = model.context().clone();
        let resolver = ContextResolver::new(&model, &library);
        let before = context_graph(&resolver, &ContextKeys::new(&keys_set), &consumers).unwrap();
        let mut evaluator = BindingEvaluator::with_context(
            &TokenRegistry::new(),
            &bindings,
            &before,
            &StaticSources::new(),
            EvalLimits::DEFAULT,
        )
        .unwrap();
        let node = DependencyNode::Property(target);
        assert_eq!(evaluator.value(&node).unwrap().clone().unwrap(), num(10.0));

        // View root provision을 지우면 영향받는 소비만 다시 경로를 잡아 key default로 갑니다.
        let change = ContextChange::ProvisionRemoved {
            provision: sid(P_ROOT_N),
        };
        let affected = affected_nodes(&resolver, &change, &consumers);
        assert!(affected.contains(&node));
        let mut next = model.context().clone();
        next.remove_provision(sid(P_ROOT_N)).unwrap();
        model.replace_context(next.clone());
        let resolver = ContextResolver::new(&model, &library);
        let after = context_graph(&resolver, &ContextKeys::new(&next), &consumers).unwrap();
        evaluator
            .apply_context_change(
                RouteChange::between(&before, &after, &affected),
                &StaticSources::new(),
            )
            .unwrap();
        assert_eq!(evaluator.value(&node).unwrap().clone().unwrap(), num(1.0));
    }
}
