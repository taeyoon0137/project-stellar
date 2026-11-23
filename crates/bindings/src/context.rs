//! Component Context 소비의 binding graph 연결 (XXX.7.3·XXX.7.6·XXX.7.7·XXX.7.8, ADR-0099 결정 3·6·7·8).
//!
//! 표현식의 `Context(K)` 참조는 소비 지점마다 다른 값으로 해석됩니다(가장 가까운 조상 provision 또는 key default). 해석기는
//! 문서 crate(`stellar_document::context::resolve`)에 있고 이 crate는 문서 crate에 의존하지 않으므로, 해석 결과를 StableId
//! 수준의 **경로표**([`ContextRoute`]: key → graph node)로 받습니다.
//!
//! - key default는 [`DependencyNode::ContextKey`] producer([`Producer::ContextDefault`])입니다.
//! - provision 하나의 인스턴스 chain별 값은 [`DependencyNode::ContextProvision`] producer([`Producer::ContextProvision`])이고
//!   자기 값 식의 `Context` 읽기도 경로표로 잇습니다(같은 key 재읽기는 바깥 조상 해석).
//! - 소비 binding(target 속성 node)은 경로표를 받아 `Context(K)` 읽기가 해석된 provision·default node를 의존합니다.
//!
//! 그래서 서로 다른 key가 서로를 읽는 cycle(K1↔K2)은 provision node 사이 간선의 cycle이 되어 graph cycle 검사(VIII.6.3)가
//! 거부합니다.

use std::collections::BTreeMap;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_expressions::ast::{ExprNodeId, ExpressionAst, NodePayload, ReferenceTarget};
use stellar_expressions::types::TypeSpec;
use stellar_expressions::vm::EvalLimits;

use crate::binding::BindingSet;
use crate::graph::{ContextProvisionNode, DependencyNode, NodeIndex, Producer};
use crate::orphan::BindingWorld;
use crate::provenance::ProvenanceChain;
use crate::registry::TokenRegistry;
use crate::scheduler::{
    token_specs, BindingEvaluator, ProducerSpec, SchedulerError, SourceInputs, TokenAwareSources,
};

/// 소비 지점 하나의 key별 해석 결과: key StableId → 읽을 graph node(provision 값 또는 key default).
pub type ContextRoute = BTreeMap<StableId, DependencyNode>;

/// 참조 대상을 경로표로 해석한 graph node. 경로표에 없는 `Context(K)`는 key default node입니다.
pub fn route_node(route: Option<&ContextRoute>, target: &ReferenceTarget) -> DependencyNode {
    if let (Some(route), ReferenceTarget::Context(key)) = (route, target) {
        if let Some(node) = route.get(key) {
            return node.clone();
        }
    }
    DependencyNode::from_target(target)
}

/// key default 입력.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextKeyInput {
    /// key StableId.
    pub key: StableId,
    /// 값 타입.
    pub value_type: TypeSpec,
    /// 닫힌 default expression.
    pub default: ExpressionAst,
}

/// 해석 결과의 출처(provenance 표시, XXX.7.7).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContextSource {
    /// key default로 해석됨.
    Default,
    /// provision으로 해석됨.
    Provision {
        /// provision StableId.
        provision: StableId,
        /// provider node StableId.
        provider: StableId,
        /// provider를 펼친 인스턴스 chain.
        instances: Vec<StableId>,
    },
}

impl ContextSource {
    /// 이 출처가 가리키는 graph node.
    pub fn node(&self, key: StableId) -> DependencyNode {
        match self {
            ContextSource::Default => DependencyNode::ContextKey(key),
            ContextSource::Provision {
                provision,
                instances,
                ..
            } => DependencyNode::ContextProvision(ContextProvisionNode {
                provision: *provision,
                instances: instances.clone(),
            }),
        }
    }
}

/// 소비 지점 하나에서 key 하나의 해석(경로표 항목 + 표시 정보).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextHopInfo {
    /// 출처.
    pub source: ContextSource,
    /// Slot 공개 목록을 거쳐 컴포넌트 내부 provision을 찾은 Slot(StableId, 지나간 순서).
    pub via_slots: Vec<StableId>,
}

/// provision 값 producer 입력.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionInput {
    /// provision 값 node.
    pub node: ContextProvisionNode,
    /// provider node StableId(provenance 표시).
    pub provider: StableId,
    /// 공급하는 key.
    pub key: StableId,
    /// key 값 타입.
    pub value_type: TypeSpec,
    /// 값 식.
    pub value: ExpressionAst,
    /// 값 식의 Context 읽기 해석.
    pub hops: BTreeMap<StableId, ContextHopInfo>,
}

/// binding graph에 넣을 Context 입력 전체.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContextGraph {
    /// key(default producer).
    pub keys: Vec<ContextKeyInput>,
    /// provision 값 producer.
    pub provisions: Vec<ProvisionInput>,
    /// 소비 producer node(binding target 속성 등) → key별 해석.
    pub consumers: BTreeMap<DependencyNode, BTreeMap<StableId, ContextHopInfo>>,
}

fn routes_of(hops: &BTreeMap<StableId, ContextHopInfo>) -> ContextRoute {
    hops.iter()
        .map(|(key, hop)| (*key, hop.source.node(*key)))
        .collect()
}

impl ContextGraph {
    /// key·provision producer 명세(key → provision 순, 각각 입력 순서).
    pub fn producer_specs(&self) -> Vec<ProducerSpec> {
        let mut specs: Vec<ProducerSpec> = self
            .keys
            .iter()
            .map(|key| ProducerSpec {
                node: DependencyNode::ContextKey(key.key),
                producer: Producer::ContextDefault(key.key),
                value_type: Some(key.value_type),
                expression: key.default.clone(),
            })
            .collect();
        specs.extend(self.provisions.iter().map(|provision| ProducerSpec {
            node: DependencyNode::ContextProvision(provision.node.clone()),
            producer: Producer::ContextProvision(provision.node.provision),
            value_type: Some(provision.value_type),
            expression: provision.value.clone(),
        }));
        specs
    }

    /// producer node별 경로표(provision 값 식 + 소비 binding).
    pub fn routes(&self) -> BTreeMap<DependencyNode, ContextRoute> {
        let mut out: BTreeMap<DependencyNode, ContextRoute> = self
            .provisions
            .iter()
            .map(|p| {
                (
                    DependencyNode::ContextProvision(p.node.clone()),
                    routes_of(&p.hops),
                )
            })
            .collect();
        for (node, hops) in &self.consumers {
            out.insert(node.clone(), routes_of(hops));
        }
        out
    }

    /// producer `node`가 key를 읽을 때의 해석 정보.
    pub fn hop(&self, node: &DependencyNode, key: StableId) -> Option<&ContextHopInfo> {
        if let DependencyNode::ContextProvision(target) = node {
            return self
                .provisions
                .iter()
                .find(|p| &p.node == target)
                .and_then(|p| p.hops.get(&key));
        }
        self.consumers.get(node).and_then(|hops| hops.get(&key))
    }

    fn provision(&self, node: &ContextProvisionNode) -> Option<&ProvisionInput> {
        self.provisions.iter().find(|p| &p.node == node)
    }
}

impl BindingEvaluator {
    /// 토큰·binding·Context 입력으로 graph를 만들고 비순환을 확인한 뒤 전체를 평가합니다(XXX.7.3).
    pub fn with_context(
        tokens: &TokenRegistry,
        bindings: &BindingSet,
        context: &ContextGraph,
        sources: &dyn SourceInputs,
        limits: EvalLimits,
    ) -> Result<BindingEvaluator, SchedulerError> {
        let mut specs = token_specs(tokens);
        specs.extend(context.producer_specs());
        specs.extend(bindings.iter().map(|binding| ProducerSpec {
            node: DependencyNode::Property(binding.target().clone()),
            producer: Producer::Binding(binding.id()),
            value_type: Some(binding.expected_type()),
            expression: binding.source().clone(),
        }));
        let sources = TokenAwareSources {
            tokens,
            inner: sources,
        };
        BindingEvaluator::from_specs_with_context(specs, context.routes(), &sources, limits)
    }

    /// `node` 값의 provenance chain과 그 chain의 Context 단계(XXX.7.7). 단계마다 읽은 key, 출처(provision StableId·provider
    /// node·인스턴스 chain 또는 default), Slot 공개 목록 경유를 기록합니다.
    pub fn context_provenance(
        &self,
        node: &DependencyNode,
        context: &ContextGraph,
    ) -> Option<ContextProvenance> {
        let chain = self.provenance(node)?;
        let mut hops = Vec::new();
        for step in &chain.steps {
            let Some(index) = self.graph().index_of(&step.node) else {
                continue;
            };
            let Some(expression) = self.expression_at(index) else {
                continue;
            };
            for (expr_node, read) in &step.reads {
                let Some(key) = context_key_of(expression, *expr_node) else {
                    continue;
                };
                let (source, via_slots) = match context.hop(&step.node, key) {
                    Some(hop) => (hop.source.clone(), hop.via_slots.clone()),
                    None => (ContextSource::Default, Vec::new()),
                };
                let provider = match read {
                    DependencyNode::ContextProvision(target) => {
                        context.provision(target).map(|p| p.provider)
                    }
                    _ => None,
                };
                hops.push(ContextHop {
                    reader: step.node.clone(),
                    expression: *expr_node,
                    key,
                    read: read.clone(),
                    source,
                    provider,
                    via_slots,
                });
            }
        }
        Some(ContextProvenance { chain, hops })
    }

    /// producer 번호의 node.
    pub fn node_at(&self, index: NodeIndex) -> &DependencyNode {
        self.graph().node(index)
    }
}

fn context_key_of(expression: &ExpressionAst, node: ExprNodeId) -> Option<StableId> {
    match expression.node(node)?.payload() {
        NodePayload::Reference(ReferenceTarget::Context(key)) => Some(*key),
        _ => None,
    }
}

/// provenance chain 안 Context 읽기 한 번(XXX.7.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextHop {
    /// 읽은 producer node.
    pub reader: DependencyNode,
    /// 읽은 표현식 node.
    pub expression: ExprNodeId,
    /// 읽은 key.
    pub key: StableId,
    /// 해석된 graph node.
    pub read: DependencyNode,
    /// 출처.
    pub source: ContextSource,
    /// 값을 제공한 provider node(default면 `None`).
    pub provider: Option<StableId>,
    /// Slot 공개 목록 경유.
    pub via_slots: Vec<StableId>,
}

/// Context 단계를 붙인 provenance(XXX.7.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextProvenance {
    /// VIII.6.5 chain.
    pub chain: ProvenanceChain,
    /// chain의 Context 읽기(chain 순서, 단계 안에서는 평가 순서).
    pub hops: Vec<ContextHop>,
}

impl ContextProvenance {
    /// 최종값 → provision → … → key 순서로 지난 provider node.
    pub fn providers(&self) -> Vec<StableId> {
        self.hops.iter().filter_map(|hop| hop.provider).collect()
    }
}

/// Context key 타입을 아는 binding world(XXX.7.8). 삭제된 key를 읽는 binding은 `Context(K)`의 타입이 없어
/// [`crate::orphan::OrphanCause::SourceMissing`]으로 orphan이 됩니다.
pub struct ContextBindingWorld<'a> {
    /// key 밖 상태.
    pub inner: &'a dyn BindingWorld,
    /// 현재 key의 값 타입.
    pub keys: &'a BTreeMap<StableId, TypeSpec>,
}

impl BindingWorld for ContextBindingWorld<'_> {
    fn target_type(&self, target: &PropertyPath) -> Option<TypeSpec> {
        self.inner.target_type(target)
    }

    fn reference_type(&self, target: &ReferenceTarget) -> Option<TypeSpec> {
        match target {
            ReferenceTarget::Context(key) => self.keys.get(key).copied(),
            other => self.inner.reference_type(other),
        }
    }
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
