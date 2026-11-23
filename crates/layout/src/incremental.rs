//! 증분 레이아웃 (X.7.3, ADR-0076).
//!
//! [`IncrementalLayout`]은 레이아웃 입력과 마지막 결과를 node 단위로 보관하고, node 입력 edit([`LayoutEdit`])가
//! 오면 **바뀐 영향 범위만** 다시 배치합니다. 결과는 같은 입력의 전체 계산([`crate::engine::compute_layout_with`])과
//! bytes까지 같습니다.
//!
//! # 원리
//!
//! node 배치는 subtree 입력과 요청의 순수 함수이고(엔진 모듈 문서), 세션은 계산 간 cache([`MeasureCache`])를 subtree
//! input hash로 유지합니다. 그래서 다시 배치하는 node의 자식 중 subtree가 그대로이고 요청도 같은 자식은 이전 결과를
//! 그대로 씁니다(자손을 방문하지 않음). 다시 배치를 시작할 node(**relayout root**)는 다음 규칙으로 정합니다.
//!
//! 1. edit마다 node 입력 차이([`changed_properties`])로 dirty class를 구합니다(X.7.1 규칙과 같음). 레이아웃에 영향이
//!    없는 class(PaintOnly, 예: transform)만 바뀐 node는 다시 배치하지 않습니다. transform은 subtree hash에도 없습니다.
//! 2. dirty node `D`의 크기·부모 안 참여가 바뀔 수 있으면(SelfSize·ParentFlow) 부모부터, 아니면(자식에게 주는 제약만,
//!    DescendantConstraint) `D`부터 시작합니다.
//! 3. 시작 node에서 위로 올라가며 처음 만나는 **배치 경계**나 root를 relayout root로 씁니다. 배치 경계는 부모 배치가 그
//!    node에서 border-box 크기만 읽는 node입니다: `width`·`height`가 모두 `<length>`이고(`min-*`·`max-*`에 콘텐츠
//!    keyword 없음) `position: absolute`이거나 부모가 `display: block`입니다. 이때 크기와 min-content·max-content 기여가
//!    콘텐츠와 무관합니다. flex·grid 부모의 item은 자동 최소 크기·track 크기가 콘텐츠를 읽으므로 경계가 아닙니다.
//! 4. relayout root는 마지막 요청으로 다시 배치합니다. 첫 baseline이 바뀌면(부모 block이 읽음) 부모에서 3을 다시 합니다.
//!    root가 아니면 조상 overflow bounds만 다시 합칩니다.
//!
//! 숨긴(`display: none`, 조상 포함) node 안의 변경은 배치하지 않습니다. subtree hash가 바뀌었으므로 다시 보일 때 cache가
//! 맞지 않아 새로 계산합니다.
//!
//! # 구조 edit와 검증
//!
//! edit batch는 적용 전에 검증합니다(값 범위, 부분집합 규칙, 자식 참조·부모 하나·순환·도달성·깊이). 검증은 바뀐 node·자식
//! 관계와 옮겨진 subtree에 비례하며, 실패하면 상태를 바꾸지 않고 오류를 돌려줍니다. 계산 중 오류(수치 범위 초과 등)는
//! edit를 반영한 채 결과를 무효로 두고([`LayoutError::LayoutUnavailable`]), 다음 edit가 전체 계산으로 복구합니다.
//!
//! # 전체 레이아웃 fallback (X.7.6)
//!
//! cache 없이 root부터 전부 배치하는 경로는 정해진 이유([`FullLayoutReason`])에서만 씁니다: 세션 시작, 측정기 revision
//! 변경, 계산 실패 뒤 복구, 테스트·진단용 명시 호출. 일반 edit 처리 중 relayout root의 이전 배치가 없어 전체 레이아웃으로
//! 대신하는 것([`FullLayoutReason::UnexpectedFallback`])은 증분 불변식이 깨졌다는 뜻이라 metric을 남기고 debug build에서는
//! assertion으로 실패합니다(release는 결과 정확성을 위해 전체 레이아웃으로 계속).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use stellar_core_types::ids::StableId;

use crate::cache::LayoutInputHash;
use crate::dirty::{changed_properties, node_dirty_classes, ALL_LAYOUT_CLASSES};
use crate::engine::{
    at, content_rect, prepare, prepare_all, size_of, with_layout_stack, Engine, LayoutStats,
    LengthP, NodeLayout, Prepared, Request, SizeP,
};
use crate::error::LayoutError;
use crate::geometry::LayoutRect;
use crate::input::{
    validate_in_parent, validate_node_values, LayoutInput, LayoutNode, TreeIndex, MAX_LAYOUT_DEPTH,
};
use crate::intrinsic::ContentMeasurer;
use crate::measure::{combine_subtree, node_hash, subtree_hashes_from, MeasureCache};
use crate::output::{LayoutBox, LayoutResult};
use crate::position::{place_absolute, StaticAlign, StaticRect};
use crate::property::{DirtyClass, DirtyClassSet};
use crate::rounding::LayoutUnit;
use crate::style::{Display, Position};

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
    /// 이전 배치를 그대로 재사용한 자식 subtree 수.
    pub reused: u64,
    /// 다시 배치를 시작한 relayout root 수.
    pub relayout_roots: u64,
    /// overflow bounds만 다시 합친 조상 수.
    pub overflow_updates: u64,
    /// 다시 계산한 subtree hash 수.
    pub hashed: u64,
    /// 실제로 계산한 node 배치 수(intrinsic 측정 요청 포함, X.7.2).
    pub measured: u64,
    /// 같은 계산 안 memo hit 수.
    pub memo_hits: u64,
    /// 계산 간 cache hit 수.
    pub cache_hits: u64,
    /// 측정기 호출 수.
    pub measurer_calls: u64,
    /// 전체 레이아웃 횟수.
    pub full_layouts: u64,
    /// 전체 레이아웃을 했다면 그 이유(X.7.6).
    pub full_layout_reason: Option<FullLayoutReason>,
}

/// 전체 레이아웃(cache 없는 root부터의 배치)을 한 이유 (X.7.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FullLayoutReason {
    /// 세션 시작.
    Initial,
    /// 측정기 revision이 바뀜(폰트 로드 등, 모든 측정 무효).
    MeasurerChanged,
    /// 직전 계산 실패 뒤 복구.
    Recovery,
    /// 테스트·진단용 명시 호출.
    Diagnostic,
    /// 일반 edit 처리 중 증분 경로가 대신함. 정상 경로에서는 일어나면 안 됩니다(debug assertion).
    UnexpectedFallback,
}

/// 세션 누적 metric (X.7.6). 값은 포화 증가합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayoutMetrics {
    /// 적용한 edit batch 수(검증 실패 제외).
    pub updates: u64,
    /// 전체 레이아웃 총 횟수.
    pub full_layouts: u64,
    /// 세션 시작 전체 레이아웃.
    pub initial: u64,
    /// 측정기 revision 변경 전체 레이아웃.
    pub measurer_changed: u64,
    /// 실패 뒤 복구 전체 레이아웃.
    pub recovery: u64,
    /// 테스트·진단용 전체 레이아웃.
    pub diagnostic: u64,
    /// 일반 edit 중 예상치 못한 전체 레이아웃 대체(0이어야 함).
    pub unexpected_fallback: u64,
}

impl LayoutMetrics {
    fn record(&mut self, reason: FullLayoutReason) {
        self.full_layouts = self.full_layouts.saturating_add(1);
        let counter = match reason {
            FullLayoutReason::Initial => &mut self.initial,
            FullLayoutReason::MeasurerChanged => &mut self.measurer_changed,
            FullLayoutReason::Recovery => &mut self.recovery,
            FullLayoutReason::Diagnostic => &mut self.diagnostic,
            FullLayoutReason::UnexpectedFallback => &mut self.unexpected_fallback,
        };
        *counter = counter.saturating_add(1);
    }
}

impl UpdateStats {
    fn absorb(&mut self, engine: &LayoutStats) {
        self.measured = self.measured.saturating_add(engine.measured);
        self.memo_hits = self.memo_hits.saturating_add(engine.memo_hits);
        self.cache_hits = self.cache_hits.saturating_add(engine.cache_hits);
        self.measurer_calls = self.measurer_calls.saturating_add(engine.measurer_calls);
    }
}

/// node 하나의 마지막 최종 배치.
#[derive(Debug, Clone)]
struct Final {
    /// 부모 local 좌표 원점.
    x: LayoutUnit,
    y: LayoutUnit,
    /// 부모가 준 요청(root는 root 요청).
    request: Request,
    /// 배치 결과(자식 배치 포함).
    layout: Arc<NodeLayout>,
    /// 자신의 local 좌표 overflow bounds.
    overflow: LayoutRect,
}

/// 검증을 통과한 edit batch.
#[derive(Debug, Default)]
struct Staged {
    changed: Vec<(usize, LayoutNode, Prepared)>,
    inserted: Vec<(LayoutNode, Prepared)>,
    removed: Vec<usize>,
    structural: bool,
    depths: Vec<(StableId, usize)>,
}

/// node별 root로부터의 깊이와 깊이별 node 수.
#[derive(Debug, Clone)]
struct DepthIndex {
    of: Vec<Option<usize>>,
    count: Vec<usize>,
}

impl DepthIndex {
    fn new(tree: &TreeIndex) -> DepthIndex {
        let mut depths = DepthIndex {
            of: vec![None; tree.parent.len()],
            count: vec![0; MAX_LAYOUT_DEPTH + 1],
        };
        let mut stack = vec![(tree.root, 0usize)];
        while let Some((index, depth)) = stack.pop() {
            depths.set(index, depth);
            stack.extend(tree.children[index].iter().map(|&child| (child, depth + 1)));
        }
        depths
    }

    fn set(&mut self, index: usize, depth: usize) {
        self.forget(index);
        self.of[index] = Some(depth);
        self.count[depth] += 1;
    }

    fn forget(&mut self, index: usize) {
        if let Some(old) = self.of[index].take() {
            self.count[old] -= 1;
        }
    }

    fn max(&self) -> usize {
        self.count.iter().rposition(|&n| n > 0).unwrap_or(0)
    }
}

/// 증분 레이아웃 상태 (X.7.3).
#[derive(Debug, Clone)]
pub struct IncrementalLayout {
    input: LayoutInput,
    tree: TreeIndex,
    depths: DepthIndex,
    prepared: Vec<Prepared>,
    own_hash: Vec<LayoutInputHash>,
    subtree: Vec<LayoutInputHash>,
    cache: MeasureCache,
    measurer_revision: u64,
    valid: bool,
    finals: Vec<Option<Final>>,
    last_arranged: Vec<StableId>,
    metrics: LayoutMetrics,
    /// 이번 edit 앞뒤 모두 inset이 정해진 absolute node(표시 상태 유지). 부모를 다시 배치하지 않는 대상입니다.
    stable_absolute: HashSet<StableId>,
}

impl IncrementalLayout {
    /// 입력을 검증하고 전체 레이아웃으로 시작합니다.
    pub fn new(
        input: LayoutInput,
        measurer: &dyn ContentMeasurer,
    ) -> Result<(IncrementalLayout, UpdateStats), LayoutError> {
        let tree = input.tree()?;
        let depths = DepthIndex::new(&tree);
        let prepared = prepare_all(&input)?;
        let own_hash: Vec<LayoutInputHash> = input.nodes.iter().map(node_hash).collect();
        let subtree = subtree_hashes_from(&own_hash, &tree);
        let count = input.nodes.len();
        let mut session = IncrementalLayout {
            input,
            tree,
            depths,
            prepared,
            own_hash,
            subtree,
            cache: MeasureCache::new(),
            measurer_revision: measurer.revision(),
            valid: false,
            finals: vec![None; count],
            last_arranged: Vec::new(),
            metrics: LayoutMetrics::default(),
            stable_absolute: HashSet::new(),
        };
        let stats =
            session.relayout_all(measurer, UpdateStats::default(), FullLayoutReason::Initial)?;
        Ok((session, stats))
    }

    /// 테스트 전용: node의 이전 배치를 잊게 해 증분 불변식 위반(X.7.6 fallback)을 재현합니다.
    #[cfg(test)]
    fn forget_layout(&mut self, id: StableId) {
        if let Some(&index) = self.tree.index.get(&id) {
            self.finals[index] = None;
        }
    }

    /// 세션 누적 metric(X.7.6).
    pub const fn metrics(&self) -> LayoutMetrics {
        self.metrics
    }

    /// 테스트·진단용 전체 레이아웃(cache 없이 root부터 다시 배치, X.7.6).
    pub fn full_layout_for_diagnostics(
        &mut self,
        measurer: &dyn ContentMeasurer,
    ) -> Result<UpdateStats, LayoutError> {
        self.relayout_all(
            measurer,
            UpdateStats::default(),
            FullLayoutReason::Diagnostic,
        )
    }

    /// id의 현재 node 입력.
    pub fn node(&self, id: StableId) -> Option<&LayoutNode> {
        self.tree
            .index
            .get(&id)
            .map(|&index| &self.input.nodes[index])
    }

    /// 현재 레이아웃 입력(node 순서는 edit에 따라 바뀔 수 있음).
    pub fn input(&self) -> &LayoutInput {
        &self.input
    }

    /// 마지막 edit에서 배치를 다시 계산한 node(배치 순서).
    pub fn last_arranged(&self) -> &[StableId] {
        &self.last_arranged
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
            let layout_box = self.box_of(index)?;
            boxes.push(layout_box);
            let fin = self.finals[index]
                .as_ref()
                .ok_or(LayoutError::LayoutUnavailable)?;
            diagnostics.extend(fin.layout.diagnostics.iter().copied());
            for placement in fin.layout.placements.iter().rev() {
                let child = *self
                    .tree
                    .index
                    .get(&placement.child)
                    .ok_or(LayoutError::LayoutUnavailable)?;
                stack.push(child);
            }
        }
        LayoutResult::with_diagnostics(boxes, diagnostics)
    }

    fn box_of(&self, index: usize) -> Result<LayoutBox, LayoutError> {
        let fin = self.finals[index]
            .as_ref()
            .ok_or(LayoutError::LayoutUnavailable)?;
        let id = self.input.nodes[index].id;
        let size = size_of(id, fin.layout.size)?;
        let rect = LayoutRect::from_origin_size(fin.x, fin.y, size);
        let content = content_rect(id, &self.prepared[index].padding, size)?;
        LayoutBox::new(id, rect, content, fin.layout.baseline, fin.overflow)
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
        self.box_of(index).ok()
    }

    /// edit batch를 적용하고 영향 범위만 다시 배치합니다.
    pub fn apply(
        &mut self,
        edits: Vec<LayoutEdit>,
        measurer: &dyn ContentMeasurer,
    ) -> Result<UpdateStats, LayoutError> {
        let staged = self.stage(edits)?;
        let mut stats = UpdateStats::default();
        let dirty = self.commit(staged, &mut stats);
        self.metrics.updates = self.metrics.updates.saturating_add(1);
        if !self.valid {
            return self.relayout_all(measurer, stats, FullLayoutReason::Recovery);
        }
        if measurer.revision() != self.measurer_revision {
            return self.relayout_all(measurer, stats, FullLayoutReason::MeasurerChanged);
        }
        self.relayout_dirty(&dirty, measurer, stats)
    }

    // --- 검증 ---

    fn stage(&self, edits: Vec<LayoutEdit>) -> Result<Staged, LayoutError> {
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
            staged.depths = self.validate_structure(&view, &order)?;
        }
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
                    validate_in_parent(node, parent_node)?;
                }
            }
            for child in &node.children {
                if let Some(child_node) = view.node(child.id) {
                    validate_in_parent(child_node, node)?;
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

    fn validate_structure(
        &self,
        view: &FinalView<'_>,
        order: &[StableId],
    ) -> Result<Vec<(StableId, usize)>, LayoutError> {
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
        let mut depths: Vec<(StableId, usize)> = Vec::new();
        for (child, depth) in outermost {
            let mut stack = vec![(child, depth)];
            while let Some((node, depth)) = stack.pop() {
                if depth > MAX_LAYOUT_DEPTH {
                    return Err(LayoutError::DepthLimitExceeded {
                        node,
                        limit: MAX_LAYOUT_DEPTH,
                    });
                }
                depths.push((node, depth));
                if let Some(node) = view.node(node) {
                    stack.extend(node.children.iter().map(|c| (c.id, depth + 1)));
                }
            }
        }
        Ok(depths)
    }

    // --- 반영 ---

    fn commit(
        &mut self,
        staged: Staged,
        stats: &mut UpdateStats,
    ) -> Vec<(StableId, DirtyClassSet)> {
        let mut dirty: Vec<(StableId, DirtyClassSet)> = Vec::new();
        let mut rehash: Vec<StableId> = Vec::new();
        let mut relinked: Vec<usize> = Vec::new();
        let mut released: Vec<usize> = Vec::new();
        self.stable_absolute.clear();
        for (index, node, prepared) in staged.changed {
            let old = &self.input.nodes[index];
            if fixed_absolute(&self.prepared[index])
                && fixed_absolute(&prepared)
                && self.prepared[index].display == prepared.display
            {
                self.stable_absolute.insert(node.id);
            }
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
        for (node, prepared) in staged.inserted {
            let index = self.input.nodes.len();
            self.tree.index.insert(node.id, index);
            self.tree.parent.push(None);
            self.tree.children.push(Vec::new());
            self.own_hash.push(node_hash(&node));
            self.subtree.push(LayoutInputHash::from_parts(0, 0));
            self.prepared.push(prepared);
            self.depths.of.push(None);
            self.finals.push(None);
            dirty.push((node.id, ALL_LAYOUT_CLASSES));
            rehash.push(node.id);
            relinked.push(index);
            self.input.nodes.push(node);
        }
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
        if staged.structural {
            for &index in &staged.removed {
                self.depths.forget(index);
            }
            for (id, depth) in staged.depths {
                if let Some(&index) = self.tree.index.get(&id) {
                    self.depths.set(index, depth);
                }
            }
            self.tree.depth = self.depths.max();
        }
        let mut removed = staged.removed;
        removed.sort_unstable_by(|a, b| b.cmp(a));
        for index in removed {
            self.remove_index(index);
        }
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
        self.depths.of.swap_remove(index);
        self.finals.swap_remove(index);
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

    /// cache 없이 root부터 전부 다시 배치합니다.
    fn relayout_all(
        &mut self,
        measurer: &dyn ContentMeasurer,
        mut stats: UpdateStats,
        reason: FullLayoutReason,
    ) -> Result<UpdateStats, LayoutError> {
        self.metrics.record(reason);
        stats.full_layout_reason = Some(reason);
        stats.full_layouts = stats.full_layouts.saturating_add(1);
        self.measurer_revision = measurer.revision();
        self.cache.clear();
        self.valid = false;
        self.last_arranged.clear();
        for fin in &mut self.finals {
            *fin = None;
        }
        let IncrementalLayout {
            input,
            tree,
            prepared,
            subtree,
            cache,
            finals,
            last_arranged,
            ..
        } = self;
        let depth = tree.depth;
        let stats = with_layout_stack(depth, move || {
            let mut engine = Engine::new(input, tree, prepared, subtree, measurer, Some(cache));
            let root = tree.root;
            let request = Request::root();
            let layout = engine.layout(root, request)?;
            write_subtree(
                &mut engine,
                finals,
                last_arranged,
                &mut stats,
                root,
                (LayoutUnit::ZERO, LayoutUnit::ZERO),
                request,
                layout,
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
        self.last_arranged.clear();
        let mut starts: BTreeSet<(usize, usize)> = BTreeSet::new();
        // inset이 정해진 absolute node: 부모 배치는 그 node의 위치만 읽으므로 부모를 다시 배치하지 않습니다.
        let mut absolutes: Vec<usize> = Vec::new();
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
            if let Some(parent) = self.tree.parent[index] {
                if self.stable_absolute.contains(&id)
                    && self.finals[index].is_some()
                    && self.finals[parent].is_some()
                {
                    absolutes.push(index);
                    continue;
                }
            }
            let outer =
                classes.contains(DirtyClass::SelfSize) || classes.contains(DirtyClass::ParentFlow);
            let start = match (outer, self.tree.parent[index]) {
                (true, Some(parent)) => parent,
                _ => index,
            };
            starts.insert((depth_of(&self.tree, start), start));
        }
        if starts.is_empty() && absolutes.is_empty() {
            return Ok(stats);
        }
        self.valid = false;
        let IncrementalLayout {
            input,
            tree,
            prepared,
            subtree,
            cache,
            finals,
            last_arranged,
            ..
        } = self;
        let depth = tree.depth;
        let outcome = with_layout_stack(depth, move || {
            let mut engine =
                Engine::new_partial(input, tree, prepared, subtree, measurer, Some(cache));
            let mut done: HashSet<usize> = HashSet::new();
            // 얕은 시작 node부터 처리합니다. 앞선 relayout이 이미 다시 배치한 node는 건너뜁니다.
            let mut written: HashSet<StableId> = HashSet::new();
            while let Some(entry) = starts.pop_first() {
                let start = entry.1;
                if !displayed(input, tree, start) || written.contains(&input.nodes[start].id) {
                    continue;
                }
                let Some(root) = relayout_root(tree, prepared, finals, start) else {
                    return Ok(None);
                };
                if !done.insert(root) {
                    continue;
                }
                let Some(previous) = finals[root].clone() else {
                    return Ok(None);
                };
                stats.relayout_roots = stats.relayout_roots.saturating_add(1);
                let first = last_arranged.len();
                let layout = engine.layout(root, previous.request)?;
                write_subtree(
                    &mut engine,
                    finals,
                    last_arranged,
                    &mut stats,
                    root,
                    (previous.x, previous.y),
                    previous.request,
                    layout.clone(),
                )?;
                written.extend(last_arranged[first..].iter().copied());
                let Some(parent) = tree.parent[root] else {
                    continue;
                };
                if layout.size != previous.layout.size
                    || layout.baseline != previous.layout.baseline
                {
                    // 부모가 읽는 값이 바뀌었습니다. 부모에서 다시 relayout root를 찾습니다.
                    done.remove(&root);
                    written.remove(&input.nodes[parent].id);
                    starts.insert((depth_of(tree, parent), parent));
                    continue;
                }
                refresh_overflow(input, tree, finals, &mut stats, parent)?;
            }
            for index in absolutes {
                if !displayed(input, tree, index) || written.contains(&input.nodes[index].id) {
                    continue;
                }
                let Some(parent) = tree.parent[index] else {
                    return Ok(None);
                };
                let Some(previous) = finals[parent].clone() else {
                    return Ok(None);
                };
                // 부모가 쓰는 containing block은 자기 border-box 크기입니다(block·flex·grid 모두). inset이 정해진 축은
                // static position을 읽지 않습니다.
                let rect = StaticRect {
                    start: [LayoutUnit::ZERO; 2],
                    size: [LayoutUnit::ZERO; 2],
                    align: [StaticAlign::Start; 2],
                };
                let placement = place_absolute(&mut engine, index, previous.layout.size, rect)?;
                stats.relayout_roots = stats.relayout_roots.saturating_add(1);
                let layout = engine.layout(index, placement.request)?;
                let first = last_arranged.len();
                write_subtree(
                    &mut engine,
                    finals,
                    last_arranged,
                    &mut stats,
                    index,
                    (placement.x, placement.y),
                    placement.request,
                    layout,
                )?;
                written.extend(last_arranged[first..].iter().copied());
                let mut patched = (*previous.layout).clone();
                let Some(slot) = patched
                    .placements
                    .iter_mut()
                    .find(|p| p.child == placement.child)
                else {
                    return Ok(None);
                };
                *slot = placement;
                if let Some(fin) = finals[parent].as_mut() {
                    fin.layout = Arc::new(patched);
                }
                refresh_overflow(input, tree, finals, &mut stats, parent)?;
            }
            stats.absorb(&engine.stats);
            Ok(Some(stats))
        })?;
        match outcome {
            Some(stats) => {
                self.valid = true;
                Ok(stats)
            }
            None => {
                let stats =
                    self.relayout_all(measurer, stats, FullLayoutReason::UnexpectedFallback)?;
                if cfg!(debug_assertions) {
                    panic!("증분 레이아웃이 일반 edit에서 전체 레이아웃으로 대체되었습니다(X.7.6)");
                }
                Ok(stats)
            }
        }
    }
}

/// `start`에서 위로 올라가 처음 만나는 배치 경계나 root(모듈 문서 규칙 3).
///
/// 지나는 node는 모두 표시되는 node라 이전 최종 배치가 있어야 합니다. 없으면 증분 불변식이 깨진 것이라 `None`입니다.
fn relayout_root(
    tree: &TreeIndex,
    prepared: &[Prepared],
    finals: &[Option<Final>],
    start: usize,
) -> Option<usize> {
    let mut current = start;
    loop {
        finals[current].as_ref()?;
        let Some(parent) = tree.parent[current] else {
            return Some(current);
        };
        if is_boundary(prepared, parent, current) {
            return Some(current);
        }
        current = parent;
    }
}

/// 두 축 모두 시작·끝 inset 중 하나가 정해진 absolute node인지 여부. 부모 배치는 이런 node의 크기·baseline을 읽지 않고
/// static position도 쓰지 않으므로, node가 바뀌어도 부모 안의 그 node 위치만 다시 구하면 됩니다(ADR-0084 결정 1).
fn fixed_absolute(node: &Prepared) -> bool {
    use crate::engine::InsetP;
    node.position == Position::Absolute
        && node
            .inset
            .iter()
            .all(|axis| axis.iter().any(|inset| matches!(inset, InsetP::Length(_))))
}

/// 부모 배치가 `index`에서 border-box 크기·baseline만 읽는지 여부.
fn is_boundary(prepared: &[Prepared], parent: usize, index: usize) -> bool {
    let node = &prepared[index];
    let parent_block = prepared[parent].display == Display::Block;
    let absolute = node.position == Position::Absolute;
    let px = |size: SizeP| matches!(size, SizeP::Length(LengthP::Unit(_)));
    let not_keyword = |size: SizeP| {
        !matches!(
            size,
            SizeP::MinContent | SizeP::MaxContent | SizeP::FitContent
        )
    };
    let max_not_keyword = |max: crate::engine::MaxP| {
        matches!(
            max,
            crate::engine::MaxP::None | crate::engine::MaxP::Length(_)
        )
    };
    (absolute || parent_block)
        && px(node.size[0])
        && px(node.size[1])
        && not_keyword(node.min[0])
        && not_keyword(node.min[1])
        && max_not_keyword(node.max[0])
        && max_not_keyword(node.max[1])
}

/// 새 배치를 `index` subtree의 최종 배치로 씁니다. 같은 요청·같은 결과(cache 재사용)인 subtree는 방문하지 않습니다.
#[allow(clippy::too_many_arguments)]
fn write_subtree(
    engine: &mut Engine<'_>,
    finals: &mut Vec<Option<Final>>,
    arranged: &mut Vec<StableId>,
    stats: &mut UpdateStats,
    index: usize,
    origin: (LayoutUnit, LayoutUnit),
    request: Request,
    layout: Arc<NodeLayout>,
) -> Result<LayoutRect, LayoutError> {
    let id = engine.nodes[index].id;
    if let Some(previous) = finals[index].as_mut() {
        if Arc::ptr_eq(&previous.layout, &layout) {
            previous.request = request;
            previous.x = origin.0;
            previous.y = origin.1;
            stats.reused = stats.reused.saturating_add(1);
            return Ok(previous.overflow);
        }
    }
    stats.arranged = stats.arranged.saturating_add(1);
    arranged.push(id);
    let size = size_of(id, layout.size)?;
    let mut overflow = LayoutRect::at_origin(size);
    let mut placed: HashSet<StableId> = HashSet::with_capacity(layout.placements.len());
    for placement in &layout.placements {
        let child = engine.index_of(placement.child)?;
        placed.insert(placement.child);
        let child_layout = engine.layout(child, placement.request)?;
        let child_overflow = write_subtree(
            engine,
            finals,
            arranged,
            stats,
            child,
            (placement.x, placement.y),
            placement.request,
            child_layout,
        )?;
        overflow = overflow
            .union(
                &child_overflow
                    .translated(placement.x, placement.y)
                    .map_err(at(id))?,
            )
            .map_err(at(id))?;
    }
    // 이전에 놓였지만 이제 놓이지 않는 자식(숨김)은 최종 배치를 지웁니다. 다른 부모로 옮겨진 자식은 새 부모가 씁니다.
    if let Some(previous) = finals[index].take() {
        for placement in &previous.layout.placements {
            if placed.contains(&placement.child) {
                continue;
            }
            if let Some(&child) = engine.tree.index.get(&placement.child) {
                if engine.tree.parent[child] == Some(index) {
                    clear_subtree(engine.tree, finals, child);
                }
            }
        }
    }
    finals[index] = Some(Final {
        x: origin.0,
        y: origin.1,
        request,
        layout,
        overflow,
    });
    Ok(overflow)
}

/// node subtree의 최종 배치를 지웁니다(반복).
fn clear_subtree(tree: &TreeIndex, finals: &mut [Option<Final>], start: usize) {
    let mut stack = vec![start];
    while let Some(index) = stack.pop() {
        if let Some(fin) = finals[index].take() {
            for placement in &fin.layout.placements {
                if let Some(&child) = tree.index.get(&placement.child) {
                    if tree.parent[child] == Some(index) {
                        stack.push(child);
                    }
                }
            }
        }
    }
}

/// `start`부터 root까지 overflow bounds만 다시 합칩니다(값이 같아지면 멈춤).
fn refresh_overflow(
    input: &LayoutInput,
    tree: &TreeIndex,
    finals: &mut [Option<Final>],
    stats: &mut UpdateStats,
    start: usize,
) -> Result<(), LayoutError> {
    let mut current = Some(start);
    while let Some(index) = current {
        let id = input.nodes[index].id;
        let Some(fin) = finals[index].as_ref() else {
            return Ok(());
        };
        let size = size_of(id, fin.layout.size)?;
        let mut overflow = LayoutRect::at_origin(size);
        for placement in &fin.layout.placements {
            let Some(&child) = tree.index.get(&placement.child) else {
                continue;
            };
            if let Some(child_fin) = finals[child].as_ref() {
                overflow = overflow
                    .union(
                        &child_fin
                            .overflow
                            .translated(child_fin.x, child_fin.y)
                            .map_err(at(id))?,
                    )
                    .map_err(at(id))?;
            }
        }
        stats.overflow_updates = stats.overflow_updates.saturating_add(1);
        let fin = finals[index].as_mut().expect("위에서 확인");
        if fin.overflow == overflow {
            return Ok(());
        }
        fin.overflow = overflow;
        current = tree.parent[index];
    }
    Ok(())
}

/// 검증 중 최종 상태 조회(기존 입력 + batch override).
struct FinalView<'s> {
    session: &'s IncrementalLayout,
    overrides: &'s HashMap<StableId, Option<&'s LayoutNode>>,
    child_of: &'s HashMap<StableId, StableId>,
}

impl FinalView<'_> {
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

    fn parent(&self, id: StableId) -> Option<StableId> {
        if let Some(parent) = self.child_of.get(&id) {
            return Some(*parent);
        }
        let index = *self.session.tree.index.get(&id)?;
        let parent = self.session.tree.parent[index]?;
        let parent_id = self.session.input.nodes[parent].id;
        if self.overrides.contains_key(&parent_id) {
            return None;
        }
        Some(parent_id)
    }

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

/// node가 결과에 나타나는지(root가 아닌 자신·조상이 모두 `display: none`이 아님).
fn displayed(input: &LayoutInput, tree: &TreeIndex, index: usize) -> bool {
    let mut current = index;
    loop {
        if current == tree.root {
            return true;
        }
        if input.nodes[current].style.display == Display::None {
            return false;
        }
        match tree.parent[current] {
            Some(parent) => current = parent,
            None => return false,
        }
    }
}

#[cfg(test)]
mod tests;
