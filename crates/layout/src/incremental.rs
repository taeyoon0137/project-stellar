//! 증분 레이아웃 (X.7.3).
//!
//! [`IncrementalLayout`]은 레이아웃 입력과 마지막 결과를 node 단위로 보관하고, node 입력 edit
//! ([`LayoutEdit`])가 오면 **바뀐 영향 범위만** 다시 배치합니다. 결과는 같은 입력의 전체 계산
//! ([`crate::engine::compute_layout_with`])과 bytes까지 같습니다.
//!
//! # dirty class와 ancestor 전파 (layout mode별 최소화)
//!
//! edit마다 node 입력 차이([`changed_properties`])와 변경 전·후 상태로 dirty class를 구합니다(X.7.1 규칙과 같음).
//! 레이아웃에 영향이 없는 class(PaintOnly, 예: transform)만 바뀐 node는 레이아웃을 전혀 다시 하지 않습니다.
//! 나머지 dirty node `D`는 다음 규칙으로 **relayout root**까지만 올라갑니다.
//!
//! 1. `D` 자신은 다시 배치합니다(stale 표시).
//! 2. `D`의 변경이 부모에 보이지 않으면(DescendantConstraint만, 부모가 baseline을 읽는 Constraint가 아님)
//!    `D`가 relayout root입니다. 크기·위치·baseline이 그대로라 부모·형제는 바뀌지 않습니다.
//! 3. 그 밖(SelfSize·ParentFlow)에는 부모 `P`가 `D`를 다시 놓아야 합니다.
//!    - `P`가 **Free·Overlay**이고 크기가 콘텐츠와 무관(두 축 `Fixed`)하면 형제 위치가 `D`와 무관하므로 `D`
//!      하나만 다시 놓습니다(`P`는 다시 배치하지 않음).
//!    - **Stack·Grid·Constraint**는 형제 위치·track이 `D`에 의존하므로 `P`의 자식 배치를 다시 합니다(`P` stale).
//!    - `P`의 크기가 콘텐츠에 의존할 수 있으면(한 축이라도 `Fixed`가 아님) `P`도 크기가 바뀔 수 있어 같은 규칙으로
//!      한 단계 더 올라갑니다. 두 축이 `Fixed`인 `P`에서 멈춥니다. root에 닿으면 root 크기를 다시 해석합니다.
//!
//! 다시 배치하는 node의 자식은 stale이 아니고 새 rect 크기·HugContent 순환 link가 이전과 같으면 **subtree를
//! 그대로 재사용**합니다(local 좌표라 위치만 바뀐 형제도 자손을 방문하지 않음). 측정은 subtree hash key의 측정
//! cache(X.7.2)를 계속 씁니다. 다시 배치한 node의 overflow bounds가 바뀌면 조상 overflow만 갱신합니다(배치는
//! 다시 하지 않음).
//!
//! 숨긴 node의 크기 변화는 부모 flow에 보이지 않으므로 전파는 숨긴 node에서 멈춥니다(표시 여부 자체의 변경은
//! 부모까지 갑니다). 숨겨진(조상 포함) node 안의 변경은 배치하지 않고 stale 표시만 가장 위 숨김 조상까지 남겨,
//! 다시 보일 때 그 경로를 따라 배치합니다.
//!
//! # 구조 edit와 검증
//!
//! edit batch는 적용 전에 검증합니다(값 범위, 부모 배치 방식 조합, 자식 참조·부모 하나·순환·도달성·깊이). 검증은
//! 바뀐 node·자식 관계와 옮겨진 subtree에 비례하며, 실패하면 상태를 바꾸지 않고 오류를 돌려줍니다. 계산 중
//! 오류(수치 범위 초과 등)는 edit를 반영한 채 결과를 무효로 두고([`LayoutError::LayoutUnavailable`]), 다음 edit가
//! 전체 계산으로 복구합니다.

use std::collections::{BTreeSet, HashMap, HashSet};

use stellar_core_types::ids::StableId;

use crate::cache::LayoutInputHash;
use crate::dirty::{changed_properties, node_dirty_classes, ALL_LAYOUT_CLASSES};
use crate::engine::{
    prepare, prepare_all, with_layout_stack, ChildConstraint, CycleLink, Engine, LayoutStats,
    Prepared,
};
use crate::error::LayoutError;
use crate::free;
use crate::geometry::LayoutRect;
use crate::input::{
    validate_in_parent, validate_node_values, ContainerLayout, LayoutInput, LayoutNode, TreeIndex,
    MAX_LAYOUT_DEPTH,
};
use crate::intrinsic::ContentMeasurer;
use crate::measure::{combine_subtree, node_hash, subtree_hashes_from, MeasureCache};
use crate::output::{LayoutBox, LayoutDiagnostic, LayoutResult};
use crate::overlay;
use crate::property::{size_may_depend_on_content, DirtyClass, DirtyClassSet};
use crate::rounding::LayoutUnit;

/// node 입력 edit 하나 (X.7.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutEdit {
    /// 같은 id의 node 입력을 바꾸거나, 없으면 새 node를 추가합니다.
    Upsert(Box<LayoutNode>),
    /// node를 제거합니다. 그 node를 자식으로 가진 부모도 같은 batch에서 자식 목록을 고쳐야 합니다. 같은 id의
    /// edit가 여러 개면 마지막 edit가 이깁니다. 적용 전 입력에 없는 node를 지우면 [`LayoutError::UnknownNode`]
    /// 입니다.
    Remove(StableId),
}

impl LayoutEdit {
    /// node 입력을 바꾸거나 추가하는 edit.
    pub fn upsert(node: LayoutNode) -> LayoutEdit {
        LayoutEdit::Upsert(Box::new(node))
    }
}

/// edit 한 번의 집계 (X.7.3). 값은 포화 증가합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UpdateStats {
    /// 레이아웃 class가 바뀐 node 수(삽입 포함).
    pub layout_dirty: u64,
    /// PaintOnly class만 바뀐 node 수(레이아웃 queue에 넣지 않음).
    pub paint_only: u64,
    /// 배치를 다시 계산한(방문한) node 수.
    pub arranged: u64,
    /// 이전 배치를 그대로 재사용한 자식 subtree 수(배치 cache hit).
    pub reused: u64,
    /// Free·Overlay 부모 안에서 자식 하나만 다시 놓은 횟수.
    pub replaced: u64,
    /// overflow bounds만 다시 합친 조상 수.
    pub overflow_updates: u64,
    /// 다시 계산한 subtree hash 수.
    pub hashed: u64,
    /// 실제로 계산한 측정 수(X.7.2).
    pub measured: u64,
    /// 같은 계산 안 측정 memo hit 수.
    pub memo_hits: u64,
    /// 계산 간 측정 cache hit 수.
    pub cache_hits: u64,
    /// 측정기 호출 수.
    pub measurer_calls: u64,
    /// 전체 레이아웃(재사용 없는 root부터의 배치) 횟수.
    pub full_layouts: u64,
}

impl UpdateStats {
    fn absorb(&mut self, engine: &LayoutStats) {
        self.measured = self.measured.saturating_add(engine.measured);
        self.memo_hits = self.memo_hits.saturating_add(engine.memo_hits);
        self.cache_hits = self.cache_hits.saturating_add(engine.cache_hits);
        self.measurer_calls = self.measurer_calls.saturating_add(engine.measurer_calls);
    }
}

/// node 하나의 마지막 배치 결과.
#[derive(Debug, Clone)]
struct NodeLayout {
    /// 부모 local 좌표 border box.
    rect: LayoutRect,
    /// 자신의 local 좌표 content box.
    content: LayoutRect,
    /// top 기준 첫 baseline.
    baseline: Option<LayoutUnit>,
    /// 자신의 local 좌표 overflow bounds.
    overflow: LayoutRect,
    /// 배치할 때 받은 HugContent 순환 link(재사용 key).
    link: CycleLink,
    /// 자식 link 계산용 자신의 콘텐츠 크기 조상.
    own: CycleLink,
    /// 이 node를 배치하며 만든 진단(자신의 순환 진단 + 자식 배치 진단, 배치 순서).
    diagnostics: Vec<LayoutDiagnostic>,
    /// 배치한 자식(배치 순서).
    placed: Vec<StableId>,
}

/// 배치 상태(엔진 입력과 분리해 동시에 빌릴 수 있게 둠).
#[derive(Debug, Clone, Default)]
struct Arrangement {
    layouts: Vec<Option<NodeLayout>>,
    stale: Vec<bool>,
    epoch: Vec<u64>,
    update: u64,
    last_arranged: Vec<StableId>,
}

/// 다시 배치할 대상.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Target {
    /// node를 이전 rect(root는 다시 해석한 크기)로 다시 배치합니다.
    Rearrange(usize),
    /// Free·Overlay 부모 안에서 자식 하나만 다시 놓습니다.
    Replace { parent: usize, child: usize },
}

/// 검증을 통과한 edit batch.
#[derive(Debug, Default)]
struct Staged {
    /// 바뀐 기존 node(index, 새 입력, 변환).
    changed: Vec<(usize, LayoutNode, Prepared)>,
    /// 새 node(입력, 변환).
    inserted: Vec<(LayoutNode, Prepared)>,
    /// 삭제할 기존 node index.
    removed: Vec<usize>,
    /// 자식 관계가 바뀌는지 여부.
    structural: bool,
    /// 새로 붙는 subtree가 만드는 최대 깊이.
    depth: usize,
}

/// 증분 레이아웃 상태 (X.7.3).
#[derive(Debug, Clone)]
pub struct IncrementalLayout {
    input: LayoutInput,
    tree: TreeIndex,
    prepared: Vec<Prepared>,
    own_hash: Vec<LayoutInputHash>,
    subtree: Vec<LayoutInputHash>,
    cache: MeasureCache,
    measurer_revision: u64,
    valid: bool,
    arrangement: Arrangement,
}

impl IncrementalLayout {
    /// 입력을 검증하고 전체 레이아웃으로 시작합니다.
    pub fn new(
        input: LayoutInput,
        measurer: &dyn ContentMeasurer,
    ) -> Result<(IncrementalLayout, UpdateStats), LayoutError> {
        let tree = input.tree()?;
        let prepared = prepare_all(&input)?;
        let own_hash: Vec<LayoutInputHash> = input.nodes.iter().map(node_hash).collect();
        let subtree = subtree_hashes_from(&own_hash, &tree);
        let count = input.nodes.len();
        let mut session = IncrementalLayout {
            input,
            tree,
            prepared,
            own_hash,
            subtree,
            cache: MeasureCache::new(),
            measurer_revision: measurer.revision(),
            valid: false,
            arrangement: Arrangement {
                layouts: vec![None; count],
                stale: vec![true; count],
                epoch: vec![0; count],
                update: 0,
                last_arranged: Vec::new(),
            },
        };
        let stats = session.relayout_all(measurer, UpdateStats::default())?;
        Ok((session, stats))
    }

    /// 현재 레이아웃 입력(node 순서는 edit에 따라 바뀔 수 있음).
    pub fn input(&self) -> &LayoutInput {
        &self.input
    }

    /// 마지막 edit에서 배치를 다시 계산한 node(배치 순서).
    pub fn last_arranged(&self) -> &[StableId] {
        &self.arrangement.last_arranged
    }

    /// 현재 결과를 만듭니다(node 수에 선형). 직전 계산이 실패했으면 [`LayoutError::LayoutUnavailable`].
    pub fn result(&self) -> Result<LayoutResult, LayoutError> {
        if !self.valid {
            return Err(LayoutError::LayoutUnavailable);
        }
        let mut boxes = Vec::with_capacity(self.input.nodes.len());
        let mut diagnostics = Vec::new();
        let mut stack = vec![self.tree.root];
        while let Some(index) = stack.pop() {
            let layout = self.arrangement.layouts[index]
                .as_ref()
                .ok_or(LayoutError::LayoutUnavailable)?;
            boxes.push(LayoutBox::new(
                self.input.nodes[index].id,
                layout.rect,
                layout.content,
                layout.baseline,
                layout.overflow,
            )?);
            diagnostics.extend(layout.diagnostics.iter().copied());
            for child in layout.placed.iter().rev() {
                let child = *self
                    .tree
                    .index
                    .get(child)
                    .ok_or(LayoutError::LayoutUnavailable)?;
                stack.push(child);
            }
        }
        LayoutResult::with_diagnostics(boxes, diagnostics)
    }

    /// 표시되는 node의 현재 box. 숨겨졌거나 없는 node는 `None`입니다.
    pub fn layout_box(&self, id: StableId) -> Option<LayoutBox> {
        if !self.valid {
            return None;
        }
        let index = *self.tree.index.get(&id)?;
        if !displayed(&self.input, &self.tree, index) {
            return None;
        }
        let layout = self.arrangement.layouts[index].as_ref()?;
        LayoutBox::new(
            id,
            layout.rect,
            layout.content,
            layout.baseline,
            layout.overflow,
        )
        .ok()
    }

    /// edit batch를 적용하고 영향 범위만 다시 배치합니다.
    ///
    /// batch가 검증에 실패하면 상태를 바꾸지 않고 오류를 돌려줍니다. 측정기 revision이 바뀌었거나 직전 계산이
    /// 실패했으면 전체 레이아웃으로 다시 계산합니다.
    pub fn apply(
        &mut self,
        edits: Vec<LayoutEdit>,
        measurer: &dyn ContentMeasurer,
    ) -> Result<UpdateStats, LayoutError> {
        let staged = self.stage(edits)?;
        let mut stats = UpdateStats::default();
        let dirty = self.commit(staged, &mut stats);
        if !self.valid || measurer.revision() != self.measurer_revision {
            return self.relayout_all(measurer, stats);
        }
        self.relayout_dirty(&dirty, measurer, stats)
    }

    // --- 검증 ---

    /// edit batch를 검증합니다(상태 변경 없음).
    fn stage(&self, edits: Vec<LayoutEdit>) -> Result<Staged, LayoutError> {
        // id별 마지막 edit(첫 등장 순서 유지).
        let mut order: Vec<StableId> = Vec::new();
        let mut last: HashMap<StableId, Option<LayoutNode>> = HashMap::new();
        for edit in edits {
            let (id, value) = match edit {
                LayoutEdit::Upsert(node) => (node.id, Some(*node)),
                LayoutEdit::Remove(id) => (id, None),
            };
            if last.insert(id, value).is_none() {
                order.push(id);
            }
        }
        let root = self.tree.root;
        let root_id = self.input.nodes[root].id;
        let mut staged = Staged::default();
        let mut overrides: HashMap<StableId, Option<&LayoutNode>> = HashMap::new();
        for id in &order {
            let value = &last[id];
            match (self.tree.index.get(id), value) {
                (Some(&index), Some(node)) => {
                    if node.children != self.input.nodes[index].children {
                        staged.structural = true;
                    }
                    overrides.insert(*id, Some(node));
                }
                (None, Some(node)) => {
                    staged.structural = true;
                    overrides.insert(*id, Some(node));
                }
                (Some(&index), None) => {
                    if index == root {
                        return Err(LayoutError::MissingRoot { root: root_id });
                    }
                    staged.structural = true;
                    overrides.insert(*id, None);
                }
                (None, None) => return Err(LayoutError::UnknownNode { node: *id }),
            }
        }
        let mut child_of: HashMap<StableId, StableId> = HashMap::new();
        for (id, value) in &overrides {
            if let Some(node) = value {
                for child in &node.children {
                    child_of.insert(child.id, *id);
                }
            }
        }
        let view = FinalView {
            session: self,
            overrides: &overrides,
            child_of: &child_of,
        };
        if staged.structural {
            staged.depth = self.validate_structure(&view, &order)?;
        }
        // 값·조합 검증.
        for id in &order {
            let Some(Some(node)) = overrides.get(id) else {
                continue;
            };
            if !node.children.is_empty() && node.content.is_leaf() {
                return Err(LayoutError::ChildrenOnContentLeaf { node: node.id });
            }
            validate_node_values(node, *id == root_id)?;
            if let Some(parent) = view.parent(*id) {
                if let Some(parent_node) = view.node(parent) {
                    validate_in_parent(node, &parent_node.container)?;
                }
            }
            for child in &node.children {
                if let Some(child_node) = view.node(child.id) {
                    validate_in_parent(child_node, &node.container)?;
                }
            }
        }
        for id in order {
            match last.remove(&id) {
                Some(Some(node)) => {
                    let prepared = prepare(&node)?;
                    match self.tree.index.get(&id) {
                        Some(&index) => staged.changed.push((index, node, prepared)),
                        None => staged.inserted.push((node, prepared)),
                    }
                }
                Some(None) => {
                    if let Some(&index) = self.tree.index.get(&id) {
                        staged.removed.push(index);
                    }
                }
                None => {}
            }
        }
        Ok(staged)
    }

    /// 자식 관계 변경을 검증하고 새로 붙는 subtree의 최대 깊이를 돌려줍니다.
    fn validate_structure(
        &self,
        view: &FinalView<'_>,
        order: &[StableId],
    ) -> Result<usize, LayoutError> {
        let root_id = self.input.nodes[self.tree.root].id;
        let mut detached: HashSet<StableId> = HashSet::new();
        let mut attached: HashMap<StableId, StableId> = HashMap::new();
        let mut attach_order: Vec<(StableId, StableId)> = Vec::new();
        let mut inserted: Vec<StableId> = Vec::new();
        for id in order {
            let old: Option<&LayoutNode> = self
                .tree
                .index
                .get(id)
                .map(|&index| &self.input.nodes[index]);
            let new = view.node(*id);
            let old_children: HashSet<StableId> = old
                .map(|node| node.children.iter().map(|c| c.id).collect())
                .unwrap_or_default();
            let mut new_children: HashSet<StableId> = HashSet::new();
            if let Some(node) = new {
                if old.is_none() {
                    inserted.push(*id);
                }
                for child in &node.children {
                    if !new_children.insert(child.id) {
                        return Err(LayoutError::MultipleParents { node: child.id });
                    }
                }
            }
            for child in &old_children {
                if !new_children.contains(child) {
                    detached.insert(*child);
                }
            }
            if let Some(node) = new {
                for child in &node.children {
                    if !old_children.contains(&child.id) {
                        attach_order.push((child.id, *id));
                    }
                }
            }
        }
        // 붙는 자식: 존재·부모 하나·root 아님.
        for &(child, parent) in &attach_order {
            if child == root_id {
                return Err(LayoutError::Cycle { node: child });
            }
            if view.node(child).is_none() {
                return Err(LayoutError::MissingChild { parent, child });
            }
            if attached.insert(child, parent).is_some() {
                return Err(LayoutError::MultipleParents { node: child });
            }
            if let Some(&index) = self.tree.index.get(&child) {
                if let Some(old_parent) = self.tree.parent[index] {
                    let old_parent_id = self.input.nodes[old_parent].id;
                    if old_parent_id != parent && !detached.contains(&child) {
                        return Err(LayoutError::MultipleParents { node: child });
                    }
                }
            }
        }
        // 지워지는 node를 여전히 자식으로 가진 부모.
        for id in order {
            if view.node(*id).is_some() {
                continue;
            }
            let Some(&index) = self.tree.index.get(id) else {
                continue;
            };
            if let Some(parent) = self.tree.parent[index] {
                let parent_id = self.input.nodes[parent].id;
                if let Some(parent_node) = view.node(parent_id) {
                    if parent_node.children.iter().any(|c| c.id == *id) {
                        return Err(LayoutError::MissingChild {
                            parent: parent_id,
                            child: *id,
                        });
                    }
                }
            }
        }
        // 최종 부모.
        let final_parent = |node: StableId| -> Option<StableId> {
            if let Some(parent) = attached.get(&node) {
                return Some(*parent);
            }
            if detached.contains(&node) {
                return None;
            }
            let index = *self.tree.index.get(&node)?;
            self.tree.parent[index].map(|parent| self.input.nodes[parent].id)
        };
        // 도달 불가: 떨어진 node·새 node 중 남아 있으면서 부모가 없는 node.
        let mut orphans: Vec<StableId> = detached
            .iter()
            .chain(inserted.iter())
            .copied()
            .filter(|node| *node != root_id && view.node(*node).is_some())
            .filter(|node| final_parent(*node).is_none())
            .collect();
        orphans.sort();
        if let Some(node) = orphans.first() {
            return Err(LayoutError::Unreachable { node: *node });
        }
        // 순환과 깊이: 붙는 자식마다 새 부모에서 root까지 올라갑니다.
        let limit = view.len().saturating_add(1);
        let mut outermost: Vec<(StableId, usize)> = Vec::new();
        for &(child, parent) in &attach_order {
            let mut current = parent;
            let mut steps = 0usize;
            let mut nested = false;
            loop {
                if current == child {
                    return Err(LayoutError::Cycle { node: child });
                }
                if attached.contains_key(&current) {
                    nested = true;
                }
                if current == root_id {
                    break;
                }
                match final_parent(current) {
                    Some(next) => current = next,
                    None => return Err(LayoutError::Unreachable { node: child }),
                }
                steps += 1;
                if steps > limit {
                    return Err(LayoutError::Cycle { node: child });
                }
            }
            if !nested {
                outermost.push((child, steps + 1));
            }
        }
        // 붙는 subtree(가장 바깥만)의 깊이.
        let mut max_depth = 0usize;
        for (child, depth) in outermost {
            let mut stack = vec![(child, depth)];
            while let Some((node, depth)) = stack.pop() {
                if depth > MAX_LAYOUT_DEPTH {
                    return Err(LayoutError::DepthLimitExceeded {
                        node,
                        limit: MAX_LAYOUT_DEPTH,
                    });
                }
                max_depth = max_depth.max(depth);
                if let Some(node) = view.node(node) {
                    stack.extend(node.children.iter().map(|c| (c.id, depth + 1)));
                }
            }
        }
        Ok(max_depth)
    }

    // --- 반영 ---

    /// 검증된 batch를 반영하고 node별 dirty class를 돌려줍니다(실패하지 않음).
    fn commit(
        &mut self,
        staged: Staged,
        stats: &mut UpdateStats,
    ) -> Vec<(StableId, DirtyClassSet)> {
        let mut dirty: Vec<(StableId, DirtyClassSet)> = Vec::new();
        let mut rehash: Vec<StableId> = Vec::new();
        let mut relinked: Vec<usize> = Vec::new();
        let mut released: Vec<usize> = Vec::new();
        // 1. 기존 node 입력 교체.
        for (index, node, prepared) in staged.changed {
            let old = &self.input.nodes[index];
            let properties = changed_properties(old, &node);
            if old.children != node.children {
                released.extend(
                    old.children
                        .iter()
                        .filter_map(|c| self.tree.index.get(&c.id)),
                );
                relinked.push(index);
            }
            let classes = node_dirty_classes(properties, Some(old), Some(&node));
            if !properties.is_empty() || old.children != node.children {
                dirty.push((node.id, classes));
            }
            rehash.push(node.id);
            self.own_hash[index] = node_hash(&node);
            self.prepared[index] = prepared;
            self.input.nodes[index] = node;
        }
        // 2. 새 node 추가.
        for (node, prepared) in staged.inserted {
            let index = self.input.nodes.len();
            self.tree.index.insert(node.id, index);
            self.tree.parent.push(None);
            self.tree.children.push(Vec::new());
            self.own_hash.push(node_hash(&node));
            self.subtree.push(LayoutInputHash::from_parts(0, 0));
            self.prepared.push(prepared);
            self.arrangement.layouts.push(None);
            self.arrangement.stale.push(true);
            self.arrangement.epoch.push(0);
            dirty.push((node.id, ALL_LAYOUT_CLASSES));
            rehash.push(node.id);
            relinked.push(index);
            self.input.nodes.push(node);
        }
        // 3. 자식 관계: 떨어지는 쪽을 먼저 끊고 새 목록으로 다시 잇습니다.
        for &index in &staged.removed {
            released.extend(
                self.input.nodes[index]
                    .children
                    .iter()
                    .filter_map(|c| self.tree.index.get(&c.id)),
            );
        }
        for index in released {
            self.tree.parent[index] = None;
        }
        for &index in &relinked {
            let mut ordered: Vec<_> = self.input.nodes[index]
                .children
                .iter()
                .filter_map(|c| {
                    self.tree
                        .index
                        .get(&c.id)
                        .map(|&child| (c.order_key.clone(), c.id, child))
                })
                .collect();
            ordered.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
            for &(_, _, child) in &ordered {
                self.tree.parent[child] = Some(index);
            }
            self.tree.children[index] = ordered.into_iter().map(|(_, _, child)| child).collect();
        }
        self.tree.depth = self.tree.depth.max(staged.depth);
        // 4. 삭제(큰 index부터 swap_remove).
        let mut removed = staged.removed;
        removed.sort_unstable_by(|a, b| b.cmp(a));
        for index in removed {
            self.remove_index(index);
        }
        // 5. 바뀐 node와 조상의 subtree hash를 깊은 것부터 다시 합성합니다.
        let mut pending: HashMap<usize, usize> = HashMap::new();
        for id in rehash {
            let Some(&start) = self.tree.index.get(&id) else {
                continue;
            };
            let mut path = vec![start];
            let mut current = start;
            while let Some(parent) = self.tree.parent[current] {
                path.push(parent);
                current = parent;
            }
            let depth = path.len() - 1;
            for (step, index) in path.into_iter().enumerate() {
                pending.entry(index).or_insert(depth - step);
            }
        }
        let mut order: Vec<(usize, usize)> = pending.into_iter().map(|(i, d)| (d, i)).collect();
        order.sort_unstable_by(|a, b| b.cmp(a));
        for (_, index) in order {
            self.subtree[index] = combine_subtree(
                self.own_hash[index],
                self.tree.children[index]
                    .iter()
                    .map(|&child| self.subtree[child]),
            );
            stats.hashed = stats.hashed.saturating_add(1);
        }
        dirty
    }

    /// index의 node를 지웁니다(마지막 node를 그 자리로 옮기고 참조를 고침).
    fn remove_index(&mut self, index: usize) {
        let id = self.input.nodes[index].id;
        self.cache.remove_node(id);
        self.tree.index.remove(&id);
        let last = self.input.nodes.len() - 1;
        self.input.nodes.swap_remove(index);
        self.prepared.swap_remove(index);
        self.own_hash.swap_remove(index);
        self.subtree.swap_remove(index);
        self.tree.parent.swap_remove(index);
        self.tree.children.swap_remove(index);
        self.arrangement.layouts.swap_remove(index);
        self.arrangement.stale.swap_remove(index);
        self.arrangement.epoch.swap_remove(index);
        if index == last {
            return;
        }
        let moved = self.input.nodes[index].id;
        self.tree.index.insert(moved, index);
        for &child in &self.tree.children[index] {
            self.tree.parent[child] = Some(index);
        }
        if let Some(parent) = self.tree.parent[index] {
            for child in &mut self.tree.children[parent] {
                if *child == last {
                    *child = index;
                }
            }
        }
        if self.tree.root == last {
            self.tree.root = index;
        }
    }

    // --- 배치 ---

    /// 재사용 없이 root부터 전부 다시 배치합니다.
    fn relayout_all(
        &mut self,
        measurer: &dyn ContentMeasurer,
        mut stats: UpdateStats,
    ) -> Result<UpdateStats, LayoutError> {
        let revision = measurer.revision();
        if revision != self.measurer_revision {
            self.cache.clear();
            self.measurer_revision = revision;
        }
        self.valid = false;
        let arrangement = &mut self.arrangement;
        arrangement.update = arrangement.update.saturating_add(1);
        arrangement.last_arranged.clear();
        for layout in &mut arrangement.layouts {
            *layout = None;
        }
        stats.full_layouts = stats.full_layouts.saturating_add(1);
        let IncrementalLayout {
            input,
            tree,
            prepared,
            subtree,
            cache,
            arrangement,
            ..
        } = self;
        let depth = tree.depth;
        let stats = with_layout_stack(depth, move || {
            let mut engine = Engine::new(input, tree, prepared, subtree, measurer, Some(cache));
            let root = tree.root;
            let size = engine.resolve_size(root, ChildConstraint::UNCONSTRAINED)?;
            rearrange(
                &mut engine,
                arrangement,
                &mut stats,
                root,
                LayoutRect::at_origin(size),
                [None, None],
            )?;
            stats.absorb(&engine.stats);
            Ok(stats)
        })?;
        self.valid = true;
        Ok(stats)
    }

    /// dirty node에서 relayout root를 정해 그 범위만 다시 배치합니다.
    fn relayout_dirty(
        &mut self,
        dirty: &[(StableId, DirtyClassSet)],
        measurer: &dyn ContentMeasurer,
        mut stats: UpdateStats,
    ) -> Result<UpdateStats, LayoutError> {
        let arrangement = &mut self.arrangement;
        arrangement.update = arrangement.update.saturating_add(1);
        arrangement.last_arranged.clear();
        let mut targets: BTreeSet<(usize, u8, Target)> = BTreeSet::new();
        for &(id, classes) in dirty {
            if !classes.affects_layout() {
                if classes.contains(DirtyClass::PaintOnly) {
                    stats.paint_only = stats.paint_only.saturating_add(1);
                }
                continue;
            }
            stats.layout_dirty = stats.layout_dirty.saturating_add(1);
            let Some(&index) = self.tree.index.get(&id) else {
                continue;
            };
            let target = propagate(&self.input, &self.tree, arrangement, index, classes);
            let (anchor, kind) = match target {
                Target::Rearrange(node) => (node, 0),
                Target::Replace { parent, .. } => (parent, 1),
            };
            targets.insert((depth_of(&self.tree, anchor), kind, target));
        }
        if targets.is_empty() {
            return Ok(stats);
        }
        self.valid = false;
        let IncrementalLayout {
            input,
            tree,
            prepared,
            subtree,
            cache,
            arrangement,
            ..
        } = self;
        let depth = tree.depth;
        let outcome = with_layout_stack(depth, move || {
            let mut engine = Engine::new(input, tree, prepared, subtree, measurer, Some(cache));
            let mut refresh: Vec<usize> = Vec::new();
            for (_, _, target) in targets {
                if !apply_target(&mut engine, arrangement, &mut stats, target, &mut refresh)? {
                    return Ok(None);
                }
            }
            for start in refresh {
                refresh_overflow(input, tree, arrangement, &mut stats, start)?;
            }
            stats.absorb(&engine.stats);
            Ok(Some(stats))
        })?;
        match outcome {
            Some(stats) => {
                self.valid = true;
                Ok(stats)
            }
            // 이전 배치가 없는 표시 node: 전체 레이아웃으로 대신합니다(X.7.6 소관).
            None => self.relayout_all(measurer, stats),
        }
    }
}

/// 검증 중 최종 상태 조회(기존 입력 + batch override).
struct FinalView<'s> {
    session: &'s IncrementalLayout,
    overrides: &'s HashMap<StableId, Option<&'s LayoutNode>>,
    /// 바뀐·새 node의 자식 → 그 node.
    child_of: &'s HashMap<StableId, StableId>,
}

impl FinalView<'_> {
    /// 최종 node 입력(지워지면 `None`).
    fn node(&self, id: StableId) -> Option<&LayoutNode> {
        match self.overrides.get(&id) {
            Some(value) => *value,
            None => self
                .session
                .tree
                .index
                .get(&id)
                .map(|&index| &self.session.input.nodes[index]),
        }
    }

    /// 최종 부모(새 node·자식 목록 변경 반영).
    fn parent(&self, id: StableId) -> Option<StableId> {
        if let Some(parent) = self.child_of.get(&id) {
            return Some(*parent);
        }
        let index = *self.session.tree.index.get(&id)?;
        let parent = self.session.tree.parent[index]?;
        let parent_id = self.session.input.nodes[parent].id;
        // 이전 부모가 이번 batch에서 바뀌었는데 id를 더 이상 갖지 않으면 부모가 없습니다.
        if self.overrides.contains_key(&parent_id) {
            return None;
        }
        Some(parent_id)
    }

    /// 최종 node 수 상한(순환 검사 반복 제한용).
    fn len(&self) -> usize {
        self.session.input.nodes.len() + self.overrides.len()
    }
}

/// root에서 node까지 깊이.
fn depth_of(tree: &TreeIndex, index: usize) -> usize {
    let mut depth = 0;
    let mut current = index;
    while let Some(parent) = tree.parent[current] {
        depth += 1;
        current = parent;
    }
    depth
}

/// node가 결과에 나타나는지(root가 아닌 자신·조상이 모두 visible).
fn displayed(input: &LayoutInput, tree: &TreeIndex, index: usize) -> bool {
    let mut current = index;
    loop {
        if current == tree.root {
            return true;
        }
        if !input.nodes[current].visible {
            return false;
        }
        match tree.parent[current] {
            Some(parent) => current = parent,
            None => return false,
        }
    }
}

/// 부모가 자식 baseline을 읽는 배치 방식인지(Constraint의 baseline anchor).
fn reads_child_baselines(container: &ContainerLayout) -> bool {
    matches!(container, ContainerLayout::Constraint)
}

/// dirty node 하나의 relayout 대상을 정하고 경로를 stale로 표시합니다(모듈 문서의 규칙).
fn propagate(
    input: &LayoutInput,
    tree: &TreeIndex,
    arrangement: &mut Arrangement,
    index: usize,
    classes: DirtyClassSet,
) -> Target {
    arrangement.stale[index] = true;
    let parent_reads_baseline = tree.parent[index]
        .is_some_and(|parent| reads_child_baselines(&input.nodes[parent].container));
    let visible_to_parent = classes.contains(DirtyClass::SelfSize)
        || classes.contains(DirtyClass::ParentFlow)
        || (classes.contains(DirtyClass::DescendantConstraint) && parent_reads_baseline);
    // 숨긴 node는 부모 flow에서 빠지므로, 표시 여부 자체가 바뀐 경우(ParentFlow)만 부모에 보입니다.
    let hidden_without_flow =
        !input.nodes[index].visible && !classes.contains(DirtyClass::ParentFlow);
    if !visible_to_parent || hidden_without_flow {
        return Target::Rearrange(index);
    }
    let mut current = index;
    loop {
        let Some(parent) = tree.parent[current] else {
            // root: 크기를 다시 해석해 배치합니다.
            return Target::Rearrange(current);
        };
        let parent_node = &input.nodes[parent];
        let size_fixed = !size_may_depend_on_content(parent_node);
        if size_fixed
            && matches!(
                parent_node.container,
                ContainerLayout::Free | ContainerLayout::Overlay(_)
            )
        {
            return Target::Replace {
                parent,
                child: current,
            };
        }
        arrangement.stale[parent] = true;
        if size_fixed {
            return Target::Rearrange(parent);
        }
        current = parent;
        // 숨긴 조상의 크기 변화는 그 부모에 보이지 않습니다(배치는 다시 보일 때까지 미룸).
        if !input.nodes[current].visible {
            return Target::Rearrange(current);
        }
    }
}

/// 숨겨진 relayout 대상: `start`부터 가장 위 숨김 조상까지 stale로 남깁니다.
fn defer_hidden(
    input: &LayoutInput,
    tree: &TreeIndex,
    arrangement: &mut Arrangement,
    start: usize,
) {
    let mut path = vec![start];
    let mut current = start;
    while let Some(parent) = tree.parent[current] {
        path.push(parent);
        current = parent;
    }
    let topmost_hidden = path
        .iter()
        .rposition(|&node| node != tree.root && !input.nodes[node].visible);
    if let Some(top) = topmost_hidden {
        for &node in &path[..=top] {
            arrangement.stale[node] = true;
        }
    }
}

/// 대상 하나를 처리합니다. 이전 배치가 없는 표시 node를 만나면 `false`(전체 레이아웃 필요).
fn apply_target(
    engine: &mut Engine<'_>,
    arrangement: &mut Arrangement,
    stats: &mut UpdateStats,
    target: Target,
    refresh: &mut Vec<usize>,
) -> Result<bool, LayoutError> {
    let input = engine.input;
    let tree = engine.tree;
    let update = arrangement.update;
    match target {
        Target::Rearrange(node) => {
            if arrangement.epoch[node] == update {
                return Ok(true);
            }
            if !displayed(input, tree, node) {
                defer_hidden(input, tree, arrangement, node);
                return Ok(true);
            }
            let (rect, link) = if node == tree.root {
                let size = engine.resolve_size(node, ChildConstraint::UNCONSTRAINED)?;
                (LayoutRect::at_origin(size), [None, None])
            } else {
                match &arrangement.layouts[node] {
                    Some(layout) => (layout.rect, layout.link),
                    None => return Ok(false),
                }
            };
            rearrange(engine, arrangement, stats, node, rect, link)?;
            if let Some(parent) = tree.parent[node] {
                refresh.push(parent);
            }
        }
        Target::Replace { parent, child } => {
            if arrangement.epoch[parent] == update || arrangement.epoch[child] == update {
                return Ok(true);
            }
            if !displayed(input, tree, parent) {
                defer_hidden(input, tree, arrangement, child);
                return Ok(true);
            }
            let Some(layout) = &arrangement.layouts[parent] else {
                return Ok(false);
            };
            let inner = layout.content.size();
            let own = layout.own;
            let placed: Vec<StableId> = engine
                .children(parent)
                .into_iter()
                .map(|index| input.nodes[index].id)
                .collect();
            if input.nodes[child].visible {
                let rect = match &input.nodes[parent].container {
                    ContainerLayout::Free => free::child_rect(engine, parent, inner, child)?,
                    ContainerLayout::Overlay(_) => {
                        overlay::child_rect(engine, parent, inner, child)?
                    }
                    _ => return Ok(false),
                };
                let link = engine.child_link(parent, child, own);
                if can_reuse(arrangement, child, rect, link) {
                    stats.reused = stats.reused.saturating_add(1);
                    if let Some(layout) = arrangement.layouts[child].as_mut() {
                        layout.rect = rect;
                    }
                } else {
                    rearrange(engine, arrangement, stats, child, rect, link)?;
                }
            }
            if let Some(layout) = arrangement.layouts[parent].as_mut() {
                layout.placed = placed;
            }
            stats.replaced = stats.replaced.saturating_add(1);
            refresh.push(parent);
        }
    }
    Ok(true)
}

/// 자식의 이전 배치를 그대로 쓸 수 있는지(stale 아님, 같은 크기·link).
fn can_reuse(arrangement: &Arrangement, child: usize, rect: LayoutRect, link: CycleLink) -> bool {
    !arrangement.stale[child]
        && arrangement.layouts[child]
            .as_ref()
            .is_some_and(|layout| layout.rect.size() == rect.size() && layout.link == link)
}

/// node를 `rect`에 배치하고, 재사용할 수 없는 자식만 내려가 배치합니다. overflow bounds를 돌려줍니다.
fn rearrange(
    engine: &mut Engine<'_>,
    arrangement: &mut Arrangement,
    stats: &mut UpdateStats,
    index: usize,
    rect: LayoutRect,
    link: CycleLink,
) -> Result<LayoutRect, LayoutError> {
    let id = engine.nodes[index].id;
    stats.arranged = stats.arranged.saturating_add(1);
    arrangement.last_arranged.push(id);
    let start = engine.diagnostics.len();
    let own = engine.arrange_own(index, rect.size(), link)?;
    let diagnostics = engine.diagnostics.split_off(start);
    let fail = |error| LayoutError::Arithmetic { node: id, error };
    let mut overflow = LayoutRect::at_origin(rect.size());
    let mut placed = Vec::with_capacity(own.placements.len());
    for (child, child_rect) in own.placements {
        let child_link = engine.child_link(index, child, own.own);
        let child_overflow = if can_reuse(arrangement, child, child_rect, child_link) {
            stats.reused = stats.reused.saturating_add(1);
            match arrangement.layouts[child].as_mut() {
                Some(layout) => {
                    layout.rect = child_rect;
                    layout.overflow
                }
                None => LayoutRect::at_origin(child_rect.size()),
            }
        } else {
            rearrange(engine, arrangement, stats, child, child_rect, child_link)?
        };
        let moved = child_overflow
            .translated(child_rect.x(), child_rect.y())
            .map_err(fail)?;
        overflow = overflow.union(&moved).map_err(fail)?;
        placed.push(engine.nodes[child].id);
    }
    arrangement.layouts[index] = Some(NodeLayout {
        rect,
        content: own.content,
        baseline: own.baseline,
        overflow,
        link,
        own: own.own,
        diagnostics,
        placed,
    });
    arrangement.stale[index] = false;
    arrangement.epoch[index] = arrangement.update;
    Ok(overflow)
}

/// `start`부터 root 쪽으로 overflow bounds를 다시 합칩니다. 값이 그대로인 node에서 멈춥니다.
fn refresh_overflow(
    input: &LayoutInput,
    tree: &TreeIndex,
    arrangement: &mut Arrangement,
    stats: &mut UpdateStats,
    start: usize,
) -> Result<(), LayoutError> {
    let mut current = Some(start);
    while let Some(index) = current {
        let Some(layout) = arrangement.layouts[index].as_ref() else {
            return Ok(());
        };
        let id = input.nodes[index].id;
        let fail = |error| LayoutError::Arithmetic { node: id, error };
        let mut overflow = LayoutRect::at_origin(layout.rect.size());
        for child in &layout.placed {
            let Some(&child) = tree.index.get(child) else {
                continue;
            };
            if let Some(child_layout) = arrangement.layouts[child].as_ref() {
                let moved = child_layout
                    .overflow
                    .translated(child_layout.rect.x(), child_layout.rect.y())
                    .map_err(fail)?;
                overflow = overflow.union(&moved).map_err(fail)?;
            }
        }
        if layout.overflow == overflow {
            return Ok(());
        }
        if let Some(layout) = arrangement.layouts[index].as_mut() {
            layout.overflow = overflow;
        }
        stats.overflow_updates = stats.overflow_updates.saturating_add(1);
        current = tree.parent[index];
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constraint::{AnchorEdge, AnchorTarget, Constraint};
    use crate::corpus::generate_corpus;
    use crate::engine::{compute_layout, compute_layout_with};
    use crate::flex::{FlexBasis, FlexItem};
    use crate::free::FreePosition;
    use crate::geometry::Axis;
    use crate::grid::{GridItem, GridLayout, TrackSize};
    use crate::input::{IntrinsicContent, LayoutChild, LayoutContent, SizingMode};
    use crate::intrinsic::{DefaultMeasurer, IntrinsicMeasure, IntrinsicSize, MeasureError};
    use crate::intrinsic::{MeasureRequest, MeasuredContent};
    use crate::overlay::{OverlayAlignment, OverlayLayout};
    use crate::stack::StackLayout;
    use crate::test_support::{id, key, len};
    use stellar_core_types::value::{Insets, Length, Percentage, Scalar, Transform2D};

    /// SplitMix64(테스트 결정적 난수).
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n.max(1) as u64) as usize
        }

        /// 1/4 px 배수 길이 `0..=max` px.
        fn length(&mut self, max: u32) -> Length {
            len(self.below(max as usize * 4 + 1) as f32 / 4.0)
        }
    }

    fn sorted_nodes(input: &LayoutInput) -> Vec<LayoutNode> {
        let mut nodes = input.nodes.clone();
        nodes.sort_by_key(|node| node.id);
        nodes
    }

    /// 세션 결과가 같은 입력의 전체 계산과 bytes까지 같은지 확인합니다.
    fn assert_matches_full(session: &IncrementalLayout, context: &str) {
        let expected = compute_layout(session.input()).unwrap_or_else(|e| panic!("{context}: {e}"));
        let actual = session
            .result()
            .unwrap_or_else(|e| panic!("{context}: {e}"));
        assert_eq!(actual.encode(), expected.encode(), "{context}");
    }

    /// edit를 단순 모델 입력에 적용합니다(기대값 계산용).
    fn apply_model(model: &mut LayoutInput, edits: &[LayoutEdit]) {
        for edit in edits {
            match edit {
                LayoutEdit::Upsert(node) => {
                    match model.nodes.iter_mut().find(|n| n.id == node.id) {
                        Some(slot) => *slot = (**node).clone(),
                        None => model.nodes.push((**node).clone()),
                    }
                }
                LayoutEdit::Remove(target) => model.nodes.retain(|n| n.id != *target),
            }
        }
    }

    fn random_sizing(rng: &mut Rng) -> SizingMode {
        match rng.below(5) {
            0 | 1 => SizingMode::Fixed(rng.length(120)),
            2 => SizingMode::HugContent,
            3 => SizingMode::Fill,
            _ => SizingMode::Percentage(Percentage::new(rng.below(100) as f32).unwrap()),
        }
    }

    /// 무작위 edit batch(일부는 일부러 잘못된 batch).
    fn random_edits(model: &LayoutInput, rng: &mut Rng, next_id: &mut u64) -> Vec<LayoutEdit> {
        let parents: HashMap<StableId, StableId> = model
            .nodes
            .iter()
            .flat_map(|n| n.children.iter().map(move |c| (c.id, n.id)))
            .collect();
        let by_id: HashMap<StableId, &LayoutNode> = model.nodes.iter().map(|n| (n.id, n)).collect();
        let node = &model.nodes[rng.below(model.nodes.len())];
        let mut new = node.clone();
        let mut edits = Vec::new();
        match rng.below(15) {
            0 => new.width = random_sizing(rng),
            1 => new.height = random_sizing(rng),
            2 => {
                let p = rng.length(8).get();
                new.padding = Insets::new(p, p / 2.0, p, 0.0).unwrap();
            }
            3 => new.visible = !new.visible,
            4 => {
                new.transform =
                    Transform2D::new(1.0, 0.0, 0.0, 1.0, rng.length(9).get(), 0.0).unwrap();
            }
            5 => {
                let factor = Scalar::new(rng.below(3) as f32).unwrap();
                new.flex = FlexItem::new(factor, factor, FlexBasis::Auto);
            }
            6 => new.position = FreePosition::at(rng.length(50), rng.length(50)),
            7 => {
                if let LayoutContent::Intrinsic(content) = &mut new.content {
                    content.width = rng.length(90);
                } else {
                    new.content = LayoutContent::Intrinsic(IntrinsicContent {
                        width: rng.length(40),
                        height: rng.length(20),
                        baseline: Some(len(8.0)),
                    });
                }
            }
            8 => {
                // 새 leaf 자식 추가.
                *next_id += 1;
                let child =
                    LayoutNode::new(id(*next_id)).with_fixed_size(rng.length(30), rng.length(30));
                new.children.push(LayoutChild {
                    id: child.id,
                    order_key: key(rng.below(5000) as u32),
                });
                edits.push(LayoutEdit::upsert(child));
            }
            9 => {
                // 자식 leaf 하나 삭제.
                let leaves: Vec<StableId> = new
                    .children
                    .iter()
                    .map(|c| c.id)
                    .filter(|c| by_id.get(c).is_some_and(|n| n.children.is_empty()))
                    .collect();
                if let Some(&leaf) = leaves.get(rng.below(leaves.len().max(1))) {
                    new.children.retain(|c| c.id != leaf);
                    edits.push(LayoutEdit::Remove(leaf));
                }
            }
            10 => {
                if !new.children.is_empty() {
                    let at = rng.below(new.children.len());
                    new.children[at].order_key = key(rng.below(5000) as u32);
                }
            }
            11 => {
                // 다른 곳의 leaf를 이 node 아래로 이동(조상이면 순환 오류가 기대값).
                let leaf = &model.nodes[rng.below(model.nodes.len())];
                if leaf.id != node.id && leaf.id != model.root {
                    if let Some(&old_parent) = parents.get(&leaf.id) {
                        if old_parent != node.id {
                            let mut old = by_id[&old_parent].clone();
                            old.children.retain(|c| c.id != leaf.id);
                            edits.push(LayoutEdit::upsert(old));
                            new.children.push(LayoutChild {
                                id: leaf.id,
                                order_key: key(rng.below(5000) as u32),
                            });
                        }
                    }
                }
            }
            12 => {
                new.container = match rng.below(4) {
                    0 => ContainerLayout::Free,
                    1 => ContainerLayout::Stack(StackLayout::new(Axis::Horizontal)),
                    2 => ContainerLayout::Overlay(OverlayLayout {
                        horizontal: OverlayAlignment::Center,
                        vertical: OverlayAlignment::End,
                    }),
                    _ => ContainerLayout::Constraint,
                };
            }
            13 => {
                // 잘못된 batch: 없는 자식 참조 또는 root를 자식으로.
                let child = if rng.below(2) == 0 {
                    id(999_999)
                } else {
                    model.root
                };
                new.children.push(LayoutChild {
                    id: child,
                    order_key: key(1),
                });
            }
            _ => {
                new.limits.min_width = Some(rng.length(60));
            }
        }
        edits.push(LayoutEdit::upsert(new));
        edits
    }

    #[test]
    fn initial_layout_matches_full_compute_for_corpus() {
        for fixture in generate_corpus(20) {
            let (session, stats) = IncrementalLayout::new(fixture.input.clone(), &DefaultMeasurer)
                .unwrap_or_else(|e| panic!("{}: {e}", fixture.name));
            assert_eq!(stats.full_layouts, 1);
            assert_eq!(
                session.result().unwrap(),
                compute_layout(&fixture.input).unwrap(),
                "{}",
                fixture.name
            );
        }
    }

    #[test]
    fn random_edits_match_full_compute_and_reject_invalid_batches() {
        let mut applied = 0;
        let mut rejected = 0;
        let mut reused = 0;
        for (n, fixture) in generate_corpus(12).into_iter().enumerate() {
            let mut rng = Rng(0x5eed + n as u64);
            let mut next_id = 1_000_000;
            let mut model = fixture.input.clone();
            let (mut session, _) =
                IncrementalLayout::new(fixture.input.clone(), &DefaultMeasurer).unwrap();
            for step in 0..24 {
                let context = format!("{} step {step}", fixture.name);
                let edits = random_edits(&model, &mut rng, &mut next_id);
                let mut expected = model.clone();
                apply_model(&mut expected, &edits);
                let valid = expected.validate().is_ok();
                let full = if valid {
                    compute_layout(&expected).ok()
                } else {
                    None
                };
                let before = session.result().ok();
                let recovering = before.is_none();
                let outcome = session.apply(edits, &DefaultMeasurer);
                if !valid {
                    // 검증 실패: 상태 불변.
                    assert!(outcome.is_err(), "{context}: 잘못된 batch를 받아들임");
                    assert_eq!(
                        sorted_nodes(session.input()),
                        sorted_nodes(&model),
                        "{context}"
                    );
                    assert_eq!(session.result().ok(), before, "{context}");
                    rejected += 1;
                    continue;
                }
                model = expected;
                assert_eq!(
                    sorted_nodes(session.input()),
                    sorted_nodes(&model),
                    "{context}"
                );
                match full {
                    Some(full) => {
                        let stats = outcome.unwrap_or_else(|e| panic!("{context}: {e}"));
                        // 직전 계산 실패에서 복구할 때만 전체 레이아웃을 씁니다.
                        assert_eq!(
                            stats.full_layouts,
                            u64::from(recovering),
                            "{context}: 전체 레이아웃으로 대신함"
                        );
                        assert_eq!(
                            session.result().unwrap().encode(),
                            full.encode(),
                            "{context}"
                        );
                        reused += stats.reused;
                        applied += 1;
                    }
                    None => {
                        // 계산 오류: 세션도 실패하고 결과를 내지 않습니다.
                        assert!(outcome.is_err(), "{context}");
                        assert_eq!(session.result(), Err(LayoutError::LayoutUnavailable));
                    }
                }
            }
        }
        assert!(applied > 500, "{applied}");
        assert!(rejected > 50, "{rejected}");
        assert!(reused > 0);
    }

    /// 모드별 방문 범위 fixture.
    ///
    /// ```text
    /// 1 root Fixed Free
    /// ├─ 2 A Fixed 세로 Stack ── 3 B Hug 가로 Stack ── 4 L leaf, 5 S Hug Stack(6, 7)
    /// │                       └─ 8 C leaf
    /// ├─ 9 F Fixed Free ── 10 M leaf, 11 N Hug Stack(12)
    /// ├─ 13 O Fixed Overlay ── 14 P leaf, 21 Q leaf
    /// ├─ 15 G Fixed Grid(고정 track) ── 16, 17
    /// └─ 18 K Fixed Constraint ── 19, 20(19의 오른쪽에 붙음)
    /// ```
    fn modes_document() -> LayoutInput {
        let hug = SizingMode::HugContent;
        let leaf = |n: u64, w: f32| LayoutNode::new(id(n)).with_fixed_size(len(w), len(10.0));
        let stack = |axis| ContainerLayout::Stack(StackLayout::new(axis));
        let grid = ContainerLayout::Grid(GridLayout::new(
            vec![TrackSize::Fixed(len(50.0)), TrackSize::Fixed(len(50.0))],
            vec![TrackSize::Fixed(len(20.0))],
        ));
        LayoutInput::new(
            id(1),
            vec![
                LayoutNode::new(id(1))
                    .with_fixed_size(len(1000.0), len(1000.0))
                    .with_child(id(2), key(1))
                    .with_child(id(9), key(2))
                    .with_child(id(13), key(3))
                    .with_child(id(15), key(4))
                    .with_child(id(18), key(5)),
                LayoutNode::new(id(2))
                    .with_fixed_size(len(400.0), len(400.0))
                    .with_container(stack(Axis::Vertical))
                    .with_child(id(3), key(1))
                    .with_child(id(8), key(2)),
                LayoutNode::new(id(3))
                    .with_sizing(hug, hug)
                    .with_container(stack(Axis::Horizontal))
                    .with_child(id(4), key(1))
                    .with_child(id(5), key(2)),
                leaf(4, 10.0),
                LayoutNode::new(id(5))
                    .with_sizing(hug, hug)
                    .with_container(stack(Axis::Horizontal))
                    .with_child(id(6), key(1))
                    .with_child(id(7), key(2)),
                leaf(6, 5.0),
                leaf(7, 6.0),
                leaf(8, 30.0),
                LayoutNode::new(id(9))
                    .with_fixed_size(len(300.0), len(300.0))
                    .with_position(len(0.0), len(500.0))
                    .with_child(id(10), key(1))
                    .with_child(id(11), key(2)),
                leaf(10, 10.0).with_position(len(4.0), len(4.0)),
                LayoutNode::new(id(11))
                    .with_sizing(hug, hug)
                    .with_container(stack(Axis::Horizontal))
                    .with_position(len(50.0), len(50.0))
                    .with_child(id(12), key(1)),
                leaf(12, 8.0),
                LayoutNode::new(id(13))
                    .with_fixed_size(len(100.0), len(100.0))
                    .with_position(len(500.0), len(0.0))
                    .with_container(ContainerLayout::Overlay(OverlayLayout {
                        horizontal: OverlayAlignment::Center,
                        vertical: OverlayAlignment::Center,
                    }))
                    .with_child(id(14), key(1))
                    .with_child(id(21), key(2)),
                leaf(14, 20.0),
                leaf(21, 40.0),
                LayoutNode::new(id(15))
                    .with_fixed_size(len(100.0), len(20.0))
                    .with_position(len(500.0), len(200.0))
                    .with_container(grid)
                    .with_child(id(16), key(1))
                    .with_child(id(17), key(2)),
                leaf(16, 10.0).with_grid(GridItem::at(1, 1, 1, 1)),
                leaf(17, 10.0).with_grid(GridItem::at(2, 1, 1, 1)),
                LayoutNode::new(id(18))
                    .with_fixed_size(len(200.0), len(100.0))
                    .with_position(len(500.0), len(400.0))
                    .with_container(ContainerLayout::Constraint)
                    .with_child(id(19), key(1))
                    .with_child(id(20), key(2)),
                leaf(19, 30.0)
                    .with_constraint(Constraint::new(
                        AnchorEdge::Left,
                        AnchorTarget::Parent(AnchorEdge::Left),
                        len(0.0),
                    ))
                    .with_constraint(Constraint::new(
                        AnchorEdge::Top,
                        AnchorTarget::Parent(AnchorEdge::Top),
                        len(0.0),
                    )),
                leaf(20, 30.0)
                    .with_constraint(Constraint::new(
                        AnchorEdge::Left,
                        AnchorTarget::Sibling {
                            node: id(19),
                            edge: AnchorEdge::Right,
                        },
                        len(2.0),
                    ))
                    .with_constraint(Constraint::new(
                        AnchorEdge::Top,
                        AnchorTarget::Parent(AnchorEdge::Top),
                        len(0.0),
                    )),
            ],
        )
    }

    fn edited(session: &IncrementalLayout, n: u64, f: impl FnOnce(&mut LayoutNode)) -> LayoutEdit {
        let mut node = session.input().node(id(n)).unwrap().clone();
        f(&mut node);
        LayoutEdit::upsert(node)
    }

    fn ids(list: &[u64]) -> Vec<StableId> {
        list.iter().map(|n| id(*n)).collect()
    }

    #[test]
    fn single_edits_visit_only_expected_nodes_per_layout_mode() {
        let width = |w: f32| move |node: &mut LayoutNode| node.width = SizingMode::Fixed(len(w));
        // (설명, node, edit, 기대 방문 node(배치 순서))
        type Case = (
            &'static str,
            u64,
            Box<dyn FnOnce(&mut LayoutNode)>,
            Vec<u64>,
        );
        let cases: Vec<Case> = vec![
            // Stack 안 leaf 너비: Hug Stack B를 거쳐 Fixed Stack A에서 멈춥니다(root 방문 0).
            ("stack leaf width", 4, Box::new(width(20.0)), vec![2, 3, 4]),
            // Free(Fixed) 안 leaf: 그 leaf만 다시 놓습니다.
            ("free leaf width", 10, Box::new(width(15.0)), vec![10]),
            // Free 안 Hug container의 leaf: Hug N까지 올라가 N만 다시 놓습니다.
            ("free nested leaf", 12, Box::new(width(9.0)), vec![11, 12]),
            // Overlay(Fixed) 안 leaf: 형제 Q는 방문하지 않습니다.
            ("overlay leaf width", 14, Box::new(width(30.0)), vec![14]),
            // Grid(Fixed) item: grid를 다시 배치하고 다른 item은 재사용.
            ("grid item width", 16, Box::new(width(20.0)), vec![15, 16]),
            // Constraint: container를 다시 풀고 20은 같은 크기라 위치만 갱신.
            ("constraint width", 19, Box::new(width(40.0)), vec![18, 19]),
            // Fixed leaf padding: DescendantConstraint만 → 자기 자신.
            (
                "leaf padding",
                4,
                Box::new(|node: &mut LayoutNode| {
                    node.padding = Insets::new(1.0, 1.0, 1.0, 1.0).unwrap()
                }),
                vec![4],
            ),
            // Fixed Stack A padding: A만(B·C는 같은 크기라 재사용).
            (
                "stack padding",
                2,
                Box::new(|node: &mut LayoutNode| {
                    node.padding = Insets::new(5.0, 0.0, 0.0, 5.0).unwrap()
                }),
                vec![2],
            ),
            // Free 자식 위치(ParentFlow): 그 자식만.
            (
                "free position",
                11,
                Box::new(|node: &mut LayoutNode| {
                    node.position = FreePosition::at(len(60.0), len(60.0))
                }),
                vec![11],
            ),
            // transform(PaintOnly): 레이아웃 0.
            (
                "leaf transform",
                4,
                Box::new(|node: &mut LayoutNode| {
                    node.transform = Transform2D::new(0.0, 1.0, -1.0, 0.0, 0.0, 0.0).unwrap()
                }),
                vec![],
            ),
        ];
        for (name, node, edit, expected) in cases {
            let (mut session, _) =
                IncrementalLayout::new(modes_document(), &DefaultMeasurer).unwrap();
            let change = edited(&session, node, edit);
            let stats = session.apply(vec![change], &DefaultMeasurer).unwrap();
            assert_eq!(session.last_arranged(), &ids(&expected)[..], "{name}");
            assert_eq!(stats.arranged as usize, expected.len(), "{name}");
            assert_eq!(stats.full_layouts, 0, "{name}");
            assert!(
                !session.last_arranged().contains(&id(1)),
                "{name}: root 방문"
            );
            assert_matches_full(&session, name);
            if expected.is_empty() {
                assert_eq!((stats.paint_only, stats.measured), (1, 0), "{name}");
            }
        }
    }

    #[test]
    fn leaf_color_change_does_zero_layout() {
        use crate::dirty::{
            dirty_set_from_changes, CommandInvalidation, LayoutChangeSource, LayoutInputIndex,
            LayoutPropertyMap, PropertyLayoutEffect,
        };
        use stellar_core_types::property::PropertyKey;
        struct Fill;
        impl LayoutPropertyMap for Fill {
            fn effect(&self, _: PropertyKey) -> Option<PropertyLayoutEffect> {
                Some(PropertyLayoutEffect::PaintOnly)
            }
        }
        struct ColorChange;
        impl LayoutChangeSource for ColorChange {
            fn changed_nodes(&self) -> Vec<(StableId, CommandInvalidation)> {
                vec![(id(4), CommandInvalidation::PAINT)]
            }
            fn changed_properties(&self) -> Vec<(StableId, PropertyKey)> {
                vec![(id(4), PropertyKey(0x0105))]
            }
            fn inserted_nodes(&self) -> Vec<StableId> {
                Vec::new()
            }
            fn removed_nodes(&self) -> Vec<StableId> {
                Vec::new()
            }
        }
        let (mut session, _) = IncrementalLayout::new(modes_document(), &DefaultMeasurer).unwrap();
        let before = session.result().unwrap();
        let index = LayoutInputIndex::new(session.input());
        let dirty = dirty_set_from_changes(&ColorChange, &Fill, &index, &index);
        // layout queue가 비어 있으므로 다시 읽을 node가 없고, 빈 batch는 레이아웃 0입니다.
        assert!(dirty.is_layout_clean());
        assert_eq!(dirty.paint(), vec![id(4)]);
        let edits: Vec<LayoutEdit> = dirty
            .layout_queue()
            .into_iter()
            .map(|(node, _)| LayoutEdit::upsert(session.input().node(node).unwrap().clone()))
            .collect();
        let stats = session.apply(edits, &DefaultMeasurer).unwrap();
        assert_eq!(stats, UpdateStats::default());
        assert!(session.last_arranged().is_empty());
        assert_eq!(session.result().unwrap(), before);
    }

    #[test]
    fn hug_chain_reaches_root_and_resizes_it() {
        let hug = SizingMode::HugContent;
        let input = LayoutInput::new(
            id(1),
            vec![
                LayoutNode::new(id(1))
                    .with_sizing(hug, hug)
                    .with_container(ContainerLayout::Stack(StackLayout::new(Axis::Horizontal)))
                    .with_child(id(2), key(1))
                    .with_child(id(4), key(2)),
                LayoutNode::new(id(2))
                    .with_sizing(hug, hug)
                    .with_container(ContainerLayout::Stack(StackLayout::new(Axis::Vertical)))
                    .with_child(id(3), key(1)),
                LayoutNode::new(id(3)).with_fixed_size(len(10.0), len(10.0)),
                LayoutNode::new(id(4)).with_fixed_size(len(10.0), len(10.0)),
            ],
        );
        let (mut session, _) = IncrementalLayout::new(input, &DefaultMeasurer).unwrap();
        let change = edited(&session, 3, |node| {
            node.width = SizingMode::Fixed(len(25.0))
        });
        let stats = session.apply(vec![change], &DefaultMeasurer).unwrap();
        assert_eq!(session.last_arranged(), &ids(&[1, 2, 3])[..]);
        assert_eq!(stats.reused, 1);
        assert_matches_full(&session, "hug chain");
        assert_eq!(
            session.layout_box(id(1)).unwrap().local_rect().width(),
            LayoutUnit::from_px_int(35).unwrap()
        );
    }

    #[test]
    fn hidden_subtree_edits_are_deferred_until_shown() {
        let (mut session, _) = IncrementalLayout::new(modes_document(), &DefaultMeasurer).unwrap();
        // B(3)를 숨깁니다: A가 다시 배치되고 B는 결과에서 빠집니다.
        let hide = edited(&session, 3, |node| node.visible = false);
        session.apply(vec![hide], &DefaultMeasurer).unwrap();
        assert!(session.layout_box(id(4)).is_none());
        assert_matches_full(&session, "hide");
        // 숨긴 subtree 안 변경은 배치하지 않습니다.
        let grow = edited(&session, 6, |node| {
            node.width = SizingMode::Fixed(len(70.0))
        });
        let stats = session.apply(vec![grow], &DefaultMeasurer).unwrap();
        assert_eq!(stats.arranged, 0);
        assert_matches_full(&session, "hidden edit");
        // 다시 보이면 stale 경로(B → S → 6)를 따라 배치합니다.
        let show = edited(&session, 3, |node| node.visible = true);
        session.apply(vec![show], &DefaultMeasurer).unwrap();
        assert_eq!(session.last_arranged(), &ids(&[2, 3, 5, 6])[..]);
        assert_matches_full(&session, "show");
        assert_eq!(
            session.layout_box(id(6)).unwrap().local_rect().width(),
            LayoutUnit::from_px_int(70).unwrap()
        );
    }

    #[test]
    fn structure_edits_insert_move_and_remove() {
        let (mut session, _) = IncrementalLayout::new(modes_document(), &DefaultMeasurer).unwrap();
        // 삽입: B(Hug Stack)에 새 leaf 30.
        let insert = edited(&session, 3, |node| {
            node.children.push(LayoutChild {
                id: id(40),
                order_key: key(3),
            })
        });
        let leaf = LayoutNode::new(id(40)).with_fixed_size(len(7.0), len(7.0));
        let stats = session
            .apply(vec![insert, LayoutEdit::upsert(leaf)], &DefaultMeasurer)
            .unwrap();
        assert_eq!(session.last_arranged(), &ids(&[2, 3, 40])[..]);
        assert_eq!(stats.full_layouts, 0);
        assert_matches_full(&session, "insert");
        // 이동: leaf 10을 F(Free)에서 O(Overlay)로.
        let from = edited(&session, 9, |node| node.children.retain(|c| c.id != id(10)));
        let to = edited(&session, 13, |node| {
            node.children.push(LayoutChild {
                id: id(10),
                order_key: key(9),
            })
        });
        session.apply(vec![from, to], &DefaultMeasurer).unwrap();
        assert_matches_full(&session, "move");
        // 삭제: subtree S(5: 6, 7)를 통째로.
        let parent = edited(&session, 3, |node| node.children.retain(|c| c.id != id(5)));
        session
            .apply(
                vec![
                    parent,
                    LayoutEdit::Remove(id(5)),
                    LayoutEdit::Remove(id(6)),
                    LayoutEdit::Remove(id(7)),
                ],
                &DefaultMeasurer,
            )
            .unwrap();
        assert!(session.input().node(id(6)).is_none());
        assert_matches_full(&session, "remove subtree");
        // 지운 node가 섞인 swap_remove 뒤에도 다음 edit이 정확합니다.
        let change = edited(&session, 21, |node| {
            node.width = SizingMode::Fixed(len(11.0))
        });
        session.apply(vec![change], &DefaultMeasurer).unwrap();
        assert_eq!(session.last_arranged(), &ids(&[21])[..]);
        assert_matches_full(&session, "after remove");
    }

    #[test]
    fn invalid_batches_leave_the_session_unchanged() {
        let (mut session, _) = IncrementalLayout::new(modes_document(), &DefaultMeasurer).unwrap();
        let input = sorted_nodes(session.input());
        let result = session.result().unwrap();
        let child = |n: u64| LayoutChild {
            id: id(n),
            order_key: key(99),
        };
        let cases: Vec<(Vec<LayoutEdit>, LayoutError)> = vec![
            // 조상 A를 B 아래에(A는 root에서 떼지 않아 부모 둘).
            (
                vec![edited(&session, 3, |node| node.children.push(child(2)))],
                LayoutError::MultipleParents { node: id(2) },
            ),
            // 없는 자식.
            (
                vec![edited(&session, 3, |node| node.children.push(child(77)))],
                LayoutError::MissingChild {
                    parent: id(3),
                    child: id(77),
                },
            ),
            // root를 자식으로.
            (
                vec![edited(&session, 3, |node| node.children.push(child(1)))],
                LayoutError::Cycle { node: id(1) },
            ),
            // 부모 둘: 8을 B에도 붙임(A에서 떼지 않음).
            (
                vec![edited(&session, 3, |node| node.children.push(child(8)))],
                LayoutError::MultipleParents { node: id(8) },
            ),
            // 도달 불가: A에서 8을 떼기만 함.
            (
                vec![edited(&session, 2, |node| {
                    node.children.retain(|c| c.id != id(8))
                })],
                LayoutError::Unreachable { node: id(8) },
            ),
            // 부모가 여전히 가진 node 삭제.
            (
                vec![LayoutEdit::Remove(id(8))],
                LayoutError::MissingChild {
                    parent: id(2),
                    child: id(8),
                },
            ),
            // root 삭제·없는 node 삭제.
            (
                vec![LayoutEdit::Remove(id(1))],
                LayoutError::MissingRoot { root: id(1) },
            ),
            (
                vec![LayoutEdit::Remove(id(55))],
                LayoutError::UnknownNode { node: id(55) },
            ),
            // 새 node를 어디에도 붙이지 않음.
            (
                vec![LayoutEdit::upsert(LayoutNode::new(id(60)))],
                LayoutError::Unreachable { node: id(60) },
            ),
            // 순환: 새 node 둘이 서로를 자식으로(그중 하나는 B 아래).
            (
                vec![
                    edited(&session, 3, |node| node.children.push(child(61))),
                    LayoutEdit::upsert(LayoutNode::new(id(61)).with_child(id(62), key(1))),
                    LayoutEdit::upsert(LayoutNode::new(id(62)).with_child(id(61), key(1))),
                ],
                LayoutError::MultipleParents { node: id(61) },
            ),
            // 값 오류: root를 Fill로.
            (
                vec![edited(&session, 1, |node| node.width = SizingMode::Fill)],
                LayoutError::InvalidSizing {
                    node: id(1),
                    axis: Axis::Horizontal,
                    reason: "root는 Fixed 또는 HugContent만 허용합니다",
                },
            ),
        ];
        for (edits, expected) in cases {
            let error = session.apply(edits, &DefaultMeasurer).unwrap_err();
            assert_eq!(
                std::mem::discriminant(&error),
                std::mem::discriminant(&expected),
                "{error:?} != {expected:?}"
            );
            assert_eq!(sorted_nodes(session.input()), input);
            assert_eq!(session.result().unwrap(), result);
        }
        // 깊이 상한: 깊이 MAX를 넘는 사슬을 붙이면 거부합니다.
        let mut chain = Vec::new();
        for n in 0..MAX_LAYOUT_DEPTH as u64 {
            let mut node = LayoutNode::new(id(5000 + n));
            if n + 1 < MAX_LAYOUT_DEPTH as u64 {
                node = node.with_child(id(5001 + n), key(1));
            }
            chain.push(LayoutEdit::upsert(node));
        }
        let mut edits = vec![edited(&session, 3, |node| node.children.push(child(5000)))];
        edits.extend(chain);
        assert!(matches!(
            session.apply(edits, &DefaultMeasurer),
            Err(LayoutError::DepthLimitExceeded { .. })
        ));
        assert_eq!(sorted_nodes(session.input()), input);
    }

    /// revision을 바꿀 수 있고 모든 콘텐츠를 기본 구현으로 측정하는 측정기.
    struct Revisioned(u64);

    impl ContentMeasurer for Revisioned {
        fn measure(
            &self,
            request: &MeasureRequest,
            content: &MeasuredContent,
        ) -> Result<IntrinsicSize, MeasureError> {
            content.measure(request)
        }

        fn revision(&self) -> u64 {
            self.0
        }
    }

    #[test]
    fn measurer_revision_change_relayouts_everything() {
        let (mut session, _) = IncrementalLayout::new(modes_document(), &Revisioned(0)).unwrap();
        let change = edited(&session, 4, |node| {
            node.width = SizingMode::Fixed(len(12.0))
        });
        let stats = session.apply(vec![change], &Revisioned(1)).unwrap();
        assert_eq!(stats.full_layouts, 1);
        assert_eq!(stats.arranged as usize, session.input().nodes.len());
        assert_eq!(
            session.result().unwrap(),
            compute_layout_with(session.input(), &Revisioned(1)).unwrap()
        );
    }

    #[test]
    fn deep_chain_updates_on_default_thread_stack() {
        let hug = SizingMode::HugContent;
        let mut nodes = Vec::new();
        for n in 0..=MAX_LAYOUT_DEPTH as u64 {
            let mut node = LayoutNode::new(id(n + 1))
                .with_sizing(hug, hug)
                .with_container(ContainerLayout::Stack(StackLayout::new(Axis::Vertical)));
            if n < MAX_LAYOUT_DEPTH as u64 {
                node = node.with_child(id(n + 2), key(1));
            } else {
                node = node.with_fixed_size(len(10.0), len(10.0));
            }
            nodes.push(node);
        }
        let input = LayoutInput::new(id(1), nodes);
        let leaf = MAX_LAYOUT_DEPTH as u64 + 1;
        std::thread::scope(|scope| {
            std::thread::Builder::new()
                .stack_size(2 << 20)
                .spawn_scoped(scope, || {
                    let (mut session, _) = IncrementalLayout::new(input, &DefaultMeasurer).unwrap();
                    let change = edited(&session, leaf, |node| {
                        node.width = SizingMode::Fixed(len(20.0))
                    });
                    let stats = session.apply(vec![change], &DefaultMeasurer).unwrap();
                    // Hug 사슬 전체(root 포함)가 다시 배치됩니다.
                    assert_eq!(stats.arranged, leaf);
                    assert_matches_full(&session, "deep chain");
                })
                .unwrap()
                .join()
                .unwrap();
        });
    }
}
