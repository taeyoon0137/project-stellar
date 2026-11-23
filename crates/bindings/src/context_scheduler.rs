//! Context 해석이 바뀐 소비 binding만 다시 평가하는 재경로(XXX.7.6, ADR-0099 결정 6).
//!
//! provision 추가·삭제·값 변경, key default 변경, 구조 변경(reparent·instance 교체·slot 콘텐츠·Slot 공개 목록·View
//! Variant 구조 patch·Fragment 묶기)은 graph에서 보면 (1) producer 추가·삭제·표현식 교체와 (2) 소비 producer의 경로표
//! 교체입니다. [`BindingEvaluator::apply_context_change`]는 graph를 새 간선으로 다시 만들되 **값은 그대로 두고**, 바뀐
//! producer에서 시작해 위상 순서로 영향받는 producer만 평가합니다. 무관한 subtree의 소비 binding은 affected에 들지 않아
//! 평가 수가 0입니다.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    topological_ranks, BindingEvaluator, NodeEvalError, NodeValue, ProducerSpec, SchedulerError,
    SourceInputs, UpdateReport,
};
use crate::context::ContextRoute;
use crate::graph::{DependencyGraph, DependencyNode, NodeIndex, ProducerRecord};

/// graph 한 번의 Context 변경.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RouteChange {
    /// 추가하거나 표현식을 바꿀 producer와 그 경로표.
    pub upsert: Vec<(ProducerSpec, Option<ContextRoute>)>,
    /// 지울 producer node.
    pub remove: Vec<DependencyNode>,
    /// 경로표만 바꿀 producer node.
    pub reroute: Vec<(DependencyNode, ContextRoute)>,
}

impl BindingEvaluator {
    /// Context 변경을 적용하고 영향받는 producer만 다시 평가합니다(XXX.7.6). 새 graph에 cycle이 있으면 아무것도 바꾸지 않고
    /// [`SchedulerError::Cycle`]입니다. 새로 생긴 source node의 값은 `sources`에서 읽습니다.
    pub fn apply_context_change(
        &mut self,
        change: RouteChange,
        sources: &dyn SourceInputs,
    ) -> Result<UpdateReport, SchedulerError> {
        // 현재 producer 명세를 node 기준으로 모읍니다.
        let mut specs: BTreeMap<DependencyNode, (ProducerSpec, Option<ContextRoute>)> =
            BTreeMap::new();
        for index in self.graph.indices() {
            let at = index.get();
            if let (Some(producer), Some(expression)) =
                (self.graph.producer(index), self.expressions[at].clone())
            {
                let node = self.graph.node(index).clone();
                specs.insert(
                    node.clone(),
                    (
                        ProducerSpec {
                            node,
                            producer,
                            value_type: self.types[at],
                            expression,
                        },
                        self.routes[at].clone(),
                    ),
                );
            }
        }
        let mut rewritten: BTreeSet<DependencyNode> = BTreeSet::new();
        for node in &change.remove {
            if specs.remove(node).is_none() {
                return Err(SchedulerError::UnknownNode(node.clone()));
            }
            rewritten.insert(node.clone());
        }
        for (spec, route) in change.upsert {
            rewritten.insert(spec.node.clone());
            specs.insert(spec.node.clone(), (spec, route));
        }
        for (node, route) in change.reroute {
            let entry = specs
                .get_mut(&node)
                .ok_or_else(|| SchedulerError::NotAProducer(node.clone()))?;
            entry.1 = Some(route);
            rewritten.insert(node);
        }
        let records: Vec<ProducerRecord> = specs
            .values()
            .map(|(spec, route)| ProducerRecord {
                node: spec.node.clone(),
                producer: spec.producer,
                dependencies: spec
                    .expression
                    .referenced_targets()
                    .into_iter()
                    .map(|target| crate::context::route_node(route.as_ref(), target))
                    .collect(),
            })
            .collect();
        let graph = DependencyGraph::from_records(records).map_err(SchedulerError::Graph)?;
        graph.ensure_acyclic().map_err(SchedulerError::Cycle)?;
        let count = graph.node_count();
        let mut types = vec![None; count];
        let mut expressions = vec![None; count];
        let mut routes = vec![None; count];
        let mut values: Vec<NodeValue> = vec![Err(NodeEvalError::MissingSource); count];
        for index in graph.indices() {
            let node = graph.node(index);
            let at = index.get();
            if let Some((spec, route)) = specs.get(node) {
                types[at] = spec.value_type;
                expressions[at] = Some(spec.expression.clone());
                routes[at] = route.clone();
            }
            // 값은 같은 node면 그대로 둡니다(바뀐 producer는 아래 propagate가 다시 평가).
            if let Some(old) = self.graph.index_of(node) {
                values[at] = self.values[old.get()].clone();
                if graph.producer(index).is_none() {
                    types[at] = self.types[old.get()];
                }
            } else if graph.producer(index).is_none() {
                types[at] = sources.source_type(node);
                if let Some(value) = sources.source_value(node) {
                    values[at] = Ok(value);
                }
            }
        }
        let (rank, topological) = topological_ranks(&graph);
        let old_programs: BTreeMap<DependencyNode, _> = self
            .graph
            .indices()
            .map(|index| (self.graph.node(index).clone(), self.programs[index.get()].clone()))
            .collect();
        self.graph = graph;
        self.types = types;
        self.expressions = expressions;
        self.routes = routes;
        self.values = values;
        self.rank = rank;
        self.topological = topological;
        self.programs = vec![None; count];
        let mut starts: Vec<NodeIndex> = Vec::new();
        for index in self.graph.indices() {
            let node = self.graph.node(index).clone();
            if rewritten.contains(&node) {
                self.compile(index);
                if self.graph.producer(index).is_some() {
                    starts.push(index);
                }
            } else {
                match old_programs.get(&node) {
                    Some(program) if program.is_some() => {
                        self.programs[index.get()] = program.clone();
                    }
                    _ => self.compile(index),
                }
            }
        }
        // 지운 producer를 읽던 node는 이제 source node입니다. 그 값이 사라졌으니 읽는 쪽을 다시 평가합니다.
        let mut changed: Vec<NodeIndex> = Vec::new();
        for node in &change.remove {
            if let Some(index) = self.graph.index_of(node) {
                self.values[index.get()] = Err(NodeEvalError::MissingSource);
                changed.push(index);
            }
        }
        Ok(self.propagate(&changed, &starts))
    }
}
