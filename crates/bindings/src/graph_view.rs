//! 저장소가 편집과 함께 유지하는 dependency graph view (XVI.4.9, ADR-0108 결정 3).
//!
//! 토큰 dependency graph(VIII.5.3)와 통합 binding graph(VIII.6.2)의 간선은 producer 하나의 참조입니다. 토큰 node의
//! 간선은 그 토큰 정의의 default expression, 속성 node의 간선은 그 속성을 target으로 하는 binding의 source가 정합니다.
//! [`TokenRegistry`]는 토큰 ID로, [`BindingSet`]은 target 속성으로 producer를 바로 찾는 index를 이미 편집과 함께
//! 갱신하므로, graph의 인접 목록은 그 두 저장소 자체입니다. 편집 하나가 바꾸는 간선은 그 producer의 간선뿐이고,
//! 저장소가 갱신되는 순간 view도 갱신됩니다(별도 graph를 다시 만들거나 맞출 필요가 없음).
//!
//! command 검증(토큰 VIII.5.5, binding VIII.6.6, orphan 재연결 VIII.8.4)은 호출마다 [`DependencyGraph::build`]·
//! [`TokenDependencyGraph::build`]로 전체 graph를 다시 만드는 대신 이 view에서 새 참조가 도달하는 부분만 BFS로 훑습니다.
//! cycle 판정과 보고 경로는 전체 graph와 같습니다. 두 graph와 view가 같은 BFS([`first_cycle_path`])를 쓰고, 인접
//! 목록이 같다는 것은 property test가 확인합니다.
//!
//! [`DependencyGraph::build`]: crate::graph::DependencyGraph::build
//! [`TokenDependencyGraph::build`]: crate::token_graph::TokenDependencyGraph::build

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use stellar_core_types::reference::TokenRef;

use crate::binding::BindingSet;
use crate::graph::{BindingCycle, DependencyNode, Producer};
use crate::registry::TokenRegistry;
use crate::token_graph::TokenCycle;

/// `starts`에서 간선을 따라 BFS(시작·간선 순)로 `target`에 처음 닿는 경로 `[target, …, target]`.
///
/// 시작 node는 넣은 순서대로 한 번만 큐에 넣고, node마다 `dependencies`가 돌려준 순서로 간선을 봅니다. `target`이
/// 시작 node에 들어 있는 경우(자기 참조)는 호출자가 먼저 처리합니다. 토큰 graph·통합 graph와 그 view가 모두 이
/// 함수를 써서 같은 입력에 같은 경로를 보고합니다.
pub(crate) fn first_cycle_path<N, D>(
    target: &N,
    starts: impl IntoIterator<Item = N>,
    mut dependencies: impl FnMut(&N) -> D,
) -> Option<Vec<N>>
where
    N: Ord + Clone,
    D: IntoIterator<Item = N>,
{
    let mut parent: BTreeMap<N, Option<N>> = BTreeMap::new();
    let mut queue: VecDeque<N> = VecDeque::new();
    for start in starts {
        if let std::collections::btree_map::Entry::Vacant(slot) = parent.entry(start.clone()) {
            slot.insert(None);
            queue.push_back(start);
        }
    }
    while let Some(current) = queue.pop_front() {
        for dep in dependencies(&current) {
            if dep == *target {
                let mut chain = vec![current.clone()];
                let mut at = current.clone();
                while let Some(Some(up)) = parent.get(&at) {
                    chain.push(up.clone());
                    at = up.clone();
                }
                chain.reverse();
                let mut path = Vec::with_capacity(chain.len() + 2);
                path.push(target.clone());
                path.extend(chain);
                path.push(target.clone());
                return Some(path);
            }
            if let std::collections::btree_map::Entry::Vacant(slot) = parent.entry(dep.clone()) {
                slot.insert(Some(current.clone()));
                queue.push_back(dep);
            }
        }
    }
    None
}

/// 토큰 registry와 binding 모음 위의 통합 dependency graph view (VIII.6.2).
#[derive(Debug, Clone, Copy)]
pub struct BindingGraphView<'a> {
    tokens: &'a TokenRegistry,
    bindings: &'a BindingSet,
}

impl<'a> BindingGraphView<'a> {
    /// 두 저장소의 view. 만들 때 아무것도 훑지 않습니다.
    pub fn new(tokens: &'a TokenRegistry, bindings: &'a BindingSet) -> BindingGraphView<'a> {
        BindingGraphView { tokens, bindings }
    }

    /// `node`의 값을 만드는 정의(없으면 외부 source node).
    pub fn producer(&self, node: &DependencyNode) -> Option<Producer> {
        match node {
            DependencyNode::Token(token) => {
                self.tokens.resolve(*token).map(|_| Producer::Token(*token))
            }
            DependencyNode::Property(path) => self
                .bindings
                .by_target(path)
                .map(|binding| Producer::Binding(binding.id())),
            DependencyNode::ComponentProperty(_) | DependencyNode::Environment(_) => None,
        }
    }

    /// `node`가 직접 읽는 node(정렬, 중복 없음 — 전체 graph의 번호 순서와 같음).
    pub fn dependencies(&self, node: &DependencyNode) -> BTreeSet<DependencyNode> {
        let references = match node {
            DependencyNode::Token(token) => self
                .tokens
                .resolve(*token)
                .map(|resolved| resolved.definition.references()),
            DependencyNode::Property(path) => self
                .bindings
                .by_target(path)
                .map(|binding| binding.references()),
            DependencyNode::ComponentProperty(_) | DependencyNode::Environment(_) => None,
        };
        references
            .unwrap_or_default()
            .iter()
            .map(DependencyNode::from_target)
            .collect()
    }

    /// `node`가 `new_dependencies`를 읽도록 바꾸면 생기는 cycle(있으면). 경로는 `[node, …, node]`입니다.
    ///
    /// [`DependencyGraph::cycle_if_dependencies`](crate::graph::DependencyGraph::cycle_if_dependencies)와 같은
    /// 판정·경로입니다. 아무도 읽지 않는 node에는 들어오는 간선이 없어 cycle에 들 수 없으므로, 전체 graph에 없는
    /// node도 BFS가 자연히 `None`을 돌려줍니다.
    pub fn cycle_if_dependencies(
        &self,
        node: &DependencyNode,
        new_dependencies: &BTreeSet<DependencyNode>,
    ) -> Option<BindingCycle> {
        if new_dependencies.contains(node) {
            return Some(BindingCycle {
                path: vec![node.clone(), node.clone()],
            });
        }
        first_cycle_path(node, new_dependencies.iter().cloned(), |current| {
            self.dependencies(current)
        })
        .map(|path| BindingCycle { path })
    }
}

/// 토큰 registry 위의 토큰 dependency graph view (VIII.5.3).
#[derive(Debug, Clone, Copy)]
pub struct TokenGraphView<'a> {
    registry: &'a TokenRegistry,
}

impl<'a> TokenGraphView<'a> {
    /// registry의 view. 만들 때 아무것도 훑지 않습니다.
    pub fn new(registry: &'a TokenRegistry) -> TokenGraphView<'a> {
        TokenGraphView { registry }
    }

    /// `token`이 직접 참조하는 토큰(ID 순). registry에 있거나 orphan 기록(tombstone)이 있는 대상만 넣습니다
    /// ([`TokenDependencyGraph::build`](crate::token_graph::TokenDependencyGraph::build)와 같은 규칙).
    pub fn dependencies(&self, token: TokenRef) -> Vec<TokenRef> {
        let Some(resolved) = self.registry.resolve(token) else {
            return Vec::new();
        };
        resolved
            .definition
            .referenced_tokens()
            .into_iter()
            .filter(|target| {
                self.registry.scope_of(*target).is_some()
                    || self.registry.tombstone(*target).is_some()
            })
            .collect()
    }

    /// `token`의 참조를 `new_references`로 바꾸면 생기는 cycle(있으면). 경로는 `[token, …, token]`입니다.
    ///
    /// [`TokenDependencyGraph::cycle_if_references`](crate::token_graph::TokenDependencyGraph::cycle_if_references)와
    /// 같은 판정·경로입니다.
    pub fn cycle_if_references(
        &self,
        token: TokenRef,
        new_references: &BTreeSet<TokenRef>,
    ) -> Option<TokenCycle> {
        if new_references.contains(&token) {
            return Some(TokenCycle {
                path: vec![token, token],
            });
        }
        first_cycle_path(&token, new_references.iter().copied(), |current| {
            self.dependencies(*current)
        })
        .map(|path| TokenCycle { path })
    }
}
