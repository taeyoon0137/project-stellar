//! 흐름 컨테이너(flex·grid) 안 드래그: 삽입 위치, 순서 preview, OrderKey 확정 (XII.10.24, ADR-0117 결정 5).
//!
//! - 삽입 위치([`insertion_index`])는 끄는 node를 뺀 형제의 화면 상자(게시된 scene)를 흐름 순서로 보고, 교차축으로
//!   겹치는 상자를 한 줄로 묶어 pointer가 있는 줄에서 주축 중심이 pointer보다 앞인 형제 수를 셉니다. `flex-wrap`의
//!   여러 줄과 Grid 자동 배치(읽는 순서)를 같은 규칙으로 다룹니다.
//! - preview([`reflow_preview`])는 흐름 순서 permutation reflow입니다. runtime은 layout crate를 부를 수 없어 같은 크기
//!   항목에서 정확하고 크기가 다르면 근사입니다. 놓은 뒤 화면은 실제 layout이 정합니다.
//! - 확정([`FlowDrag::commit`])은 같은 부모 안이면 OrderKey만 바꾸는 `MoveNode` 하나(history 한 항목)입니다. 밖으로
//!   끌어내면 다른 흐름 컨테이너에는 그 삽입 index로, 자유 배치 Frame에는 world 위치를 지키는 재부모화(ADR-0105)로
//!   옮깁니다.
//! - 거부: ComponentInstance 투영 행 안(`instance-slot`), 옛 `layout_mode` 부모(`legacy-layout`), Grid 명시 배치 자식
//!   (`grid-explicit-placement`, C6), 흐름 자식이 아님(`not-flow-child`).

use stellar_commands::arrange::align::{flow_parent, model_style_set, FlowParent};
use stellar_commands::command::Command;
use stellar_commands::lock::{edit_permission, EditIntent};
use stellar_commands::node_ops::MoveNode;
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_document::css::style_defaults::{resolve, FlatSet};
use stellar_document::layout_record::{LayoutLonghand, LayoutValue};
use stellar_document::model::DocumentModel;
use stellar_document::records::NodePayload;

use crate::app::canvas_gesture::parent_of;
use crate::app::RequestId;
use crate::canvas::transform::drag::{DragUpdate, DropTarget, MoveDrag};
use crate::canvas::transform::{node_geometry, selection_roots, TransformError};
use crate::canvas::CanvasScene;
use crate::window_session::{history_label, ProjectWindowSession};

/// 상자(x, y, width, height).
pub type Box4 = [f32; 4];

/// 흐름 방향: 주축 index(0 가로, 1 세로)와 주축 뒤집힘.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlowAxes {
    /// 주축 index.
    pub main: usize,
    /// 주축 화면 방향이 흐름 순서와 반대(`*-reverse`).
    pub reverse: bool,
}

impl FlowAxes {
    /// 배치 문맥의 축. 흐름이 아니면 `None`.
    pub fn of(flow: FlowParent) -> Option<FlowAxes> {
        match flow {
            FlowParent::Flex { row, reverse, .. } => Some(FlowAxes {
                main: if row { 0 } else { 1 },
                reverse,
            }),
            FlowParent::Grid => Some(FlowAxes {
                main: 0,
                reverse: false,
            }),
            FlowParent::Free | FlowParent::Legacy => None,
        }
    }
}

fn overlaps(a: (f32, f32), b: (f32, f32)) -> bool {
    a.0 < b.1 && b.0 < a.1
}

/// 흐름 순서 상자를 줄로 묶습니다. 결과는 줄마다 (첫 index, 끝 index 다음, 교차축 범위)입니다.
fn lines(items: &[Box4], axes: FlowAxes) -> Vec<(usize, usize, (f32, f32))> {
    let cross = 1 - axes.main;
    let mut out: Vec<(usize, usize, (f32, f32))> = Vec::new();
    for (i, r) in items.iter().enumerate() {
        let range = (r[cross], r[cross] + r[cross + 2].max(f32::EPSILON));
        match out.last_mut() {
            Some((_, end, line)) if overlaps(*line, range) => {
                *end = i + 1;
                *line = (line.0.min(range.0), line.1.max(range.1));
            }
            _ => out.push((i, i + 1, range)),
        }
    }
    out
}

/// 끄는 node를 뺀 형제 상자(흐름 순서)에서 pointer(world)의 삽입 index(0..=len).
pub fn insertion_index(items: &[Box4], axes: FlowAxes, pointer: [f32; 2]) -> usize {
    if items.is_empty() {
        return 0;
    }
    let cross = 1 - axes.main;
    let all = lines(items, axes);
    // pointer가 든 줄, 없으면 교차축으로 가장 가까운 줄(같으면 앞 줄).
    let (first, end, _) = *all
        .iter()
        .min_by(|a, b| {
            let distance = |r: (f32, f32)| {
                if pointer[cross] < r.0 {
                    r.0 - pointer[cross]
                } else if pointer[cross] > r.1 {
                    pointer[cross] - r.1
                } else {
                    0.0
                }
            };
            distance(a.2).total_cmp(&distance(b.2))
        })
        .expect("비어 있지 않음");
    let before = items[first..end]
        .iter()
        .filter(|r| {
            let center = r[axes.main] + r[axes.main + 2] / 2.0;
            if axes.reverse {
                center > pointer[axes.main]
            } else {
                center < pointer[axes.main]
            }
        })
        .count();
    first + before
}

/// 순서를 바꾼 흐름의 preview 상자. `items`는 끄는 node를 포함한 원래 흐름 순서 상자, `from`은 끄는 node index,
/// `to`는 끄는 node를 뺀 목록의 삽입 index입니다. 결과는 새 흐름 순서의 (원래 index, preview 상자)입니다.
///
/// 원래 줄마다 항목 수·주축 시작점(뒤집힘이면 끝점)·관측 간격(이웃 사이 평균)을 지키고, 새 순서 항목을 앞 줄부터
/// 같은 수만큼 채워 크기대로 다시 늘어놓습니다. 교차축은 항목의 원래 줄 안 위치를 새 줄에 옮깁니다.
pub fn reflow_preview(
    items: &[Box4],
    axes: FlowAxes,
    from: usize,
    to: usize,
) -> Vec<(usize, Box4)> {
    if from >= items.len() {
        return items.iter().copied().enumerate().collect();
    }
    let (m, c) = (axes.main, 1 - axes.main);
    let original = lines(items, axes);
    let line_of = |i: usize| {
        original
            .iter()
            .position(|(a, b, _)| (*a..*b).contains(&i))
            .expect("모든 항목이 줄에 있음")
    };
    let mut order: Vec<usize> = (0..items.len()).filter(|&i| i != from).collect();
    order.insert(to.min(order.len()), from);
    let mut out = Vec::with_capacity(items.len());
    let mut cursor = 0;
    for (start, end, cross_range) in &original {
        let slots = &items[*start..*end];
        let count = end - start;
        let gap = if count > 1 {
            let mut sorted: Vec<&Box4> = slots.iter().collect();
            sorted.sort_by(|a, b| a[m].total_cmp(&b[m]));
            sorted
                .windows(2)
                .map(|w| w[1][m] - (w[0][m] + w[0][m + 2]))
                .sum::<f32>()
                / (count - 1) as f32
        } else {
            0.0
        };
        let line_start = slots.iter().map(|r| r[m]).fold(f32::INFINITY, f32::min);
        let line_end = slots
            .iter()
            .map(|r| r[m] + r[m + 2])
            .fold(f32::NEG_INFINITY, f32::max);
        let mut along = if axes.reverse { line_end } else { line_start };
        for &index in &order[cursor..cursor + count] {
            let r = items[index];
            let home = original[line_of(index)].2;
            let mut placed = r;
            placed[c] = cross_range.0 + (r[c] - home.0);
            if axes.reverse {
                along -= r[m + 2];
                placed[m] = along;
                along -= gap;
            } else {
                placed[m] = along;
                along += r[m + 2] + gap;
            }
            out.push((index, placed));
        }
        cursor += count;
    }
    out
}

/// 흐름 순서의 node가 Grid 명시 배치(줄 번호)인지.
fn explicit_grid_placement(model: &DocumentModel, set: &FlatSet, node: StableId) -> bool {
    let Some(payload) = model.payload(node) else {
        return false;
    };
    let Some(layout) = resolve(set, payload, model.layout_record(node)) else {
        return false;
    };
    [
        LayoutLonghand::GridColumnStart,
        LayoutLonghand::GridColumnEnd,
        LayoutLonghand::GridRowStart,
        LayoutLonghand::GridRowEnd,
    ]
    .into_iter()
    .any(|l| matches!(layout.get(l).value, LayoutValue::Line(_)))
}

fn order_key(model: &DocumentModel, node: StableId) -> Option<OrderKey> {
    let handle = model.runtime_id(node)?;
    Some(model.columns().get(handle)?.order_key.clone())
}

/// 흐름 컨테이너 하나의 형제 목록(끄는 node 제외).
#[derive(Clone, Debug)]
struct FlowSiblings {
    parent: StableId,
    axes: FlowAxes,
    /// 흐름 순서 전체(끄는 node 제외): (id, key, 상자, 명시 배치).
    all: Vec<(StableId, OrderKey, Box4, bool)>,
}

impl FlowSiblings {
    fn read(
        model: &DocumentModel,
        set: &FlatSet,
        scene: &CanvasScene,
        parent: StableId,
        axes: FlowAxes,
        skip: StableId,
    ) -> FlowSiblings {
        let all = model
            .children_in_order(parent)
            .into_iter()
            .filter(|&id| id != skip)
            .filter_map(|id| {
                let rect = node_geometry(scene, id)?.world_bounds;
                Some((
                    id,
                    order_key(model, id)?,
                    rect,
                    explicit_grid_placement(model, set, id),
                ))
            })
            .collect();
        FlowSiblings { parent, axes, all }
    }

    /// 자동 배치 형제(명시 배치 제외)의 (전체 목록 index, 상자).
    fn auto(&self) -> Vec<(usize, Box4)> {
        self.all
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.3)
            .map(|(i, s)| (i, s.2))
            .collect()
    }

    /// pointer의 삽입 자리: (자동 형제 목록 index, 새 OrderKey).
    fn slot(&self, pointer: [f32; 2]) -> (usize, OrderKey) {
        let auto = self.auto();
        let boxes: Vec<Box4> = auto.iter().map(|a| a.1).collect();
        let index = insertion_index(&boxes, self.axes, pointer);
        // 다음 자동 형제 바로 앞(사이의 명시 배치 형제와 key가 겹치지 않게 전체 목록 이웃 사이)에 둡니다.
        let key = match auto.get(index) {
            Some(&(next, _)) => {
                let left = next.checked_sub(1).map(|i| &self.all[i].1);
                OrderKey::between(left, Some(&self.all[next].1))
            }
            None => OrderKey::between(self.all.last().map(|s| &s.1), None),
        };
        (index, key)
    }
}

/// 진행 중 흐름 drag의 update 결과.
#[derive(Clone, Debug, PartialEq)]
pub enum FlowUpdate {
    /// 같은 흐름 컨테이너 안: 삽입 index(끄는 node를 뺀 자동 형제 기준), 삽입 표시 선, 형제 preview.
    Inside {
        /// 삽입 index.
        index: usize,
        /// 삽입 표시(world, 폭 또는 높이 0인 상자).
        marker: Box4,
        /// 새 흐름 순서의 (node, preview 상자). 끄는 node도 들어 있습니다.
        preview: Vec<(StableId, Box4)>,
    },
    /// 다른 흐름 컨테이너 위: 그 컨테이너의 삽입 index.
    OtherFlow {
        /// 대상 컨테이너.
        parent: StableId,
        /// 삽입 index.
        index: usize,
    },
    /// 흐름 밖: 자유 배치 이동 경로의 놓을 자리.
    Outside(DropTarget),
}

/// 흐름 자식 하나의 drag.
#[derive(Debug)]
pub struct FlowDrag {
    node: StableId,
    start: [f32; 2],
    own: FlowSiblings,
    /// 끄는 node를 포함한 원래 자동 배치 흐름(preview 기준): (id, 상자)와 끄는 node index.
    original: (Vec<(StableId, Box4)>, usize),
    container_bounds: Box4,
    outer: MoveDrag,
}

fn inside(r: Box4, p: [f32; 2]) -> bool {
    p[0] >= r[0] && p[1] >= r[1] && p[0] <= r[0] + r[2] && p[1] <= r[1] + r[3]
}

impl FlowDrag {
    /// 지금 선택(흐름 자식 하나)으로 시작합니다. `pointer`는 누른 world 점, `visible`은 가시 world 영역입니다.
    pub fn begin(
        window: &ProjectWindowSession,
        scene: &CanvasScene,
        pointer: [f32; 2],
        visible: Option<Box4>,
    ) -> Result<FlowDrag, TransformError> {
        let (model, _root, roots) = selection_roots(window)?;
        let [node] = roots.as_slice() else {
            return Err(TransformError::Disabled("single-flow-child"));
        };
        let node = *node;
        let set = model_style_set(model);
        let flow = flow_parent(model, &set, node);
        if flow == FlowParent::Legacy {
            return Err(TransformError::Disabled("legacy-layout"));
        }
        let axes = FlowAxes::of(flow).ok_or(TransformError::Disabled("not-flow-child"))?;
        if scene
            .frames
            .iter()
            .any(|row| row.id == node && row.instance_frame)
        {
            return Err(TransformError::Disabled("instance-slot"));
        }
        edit_permission(model, node, EditIntent::Edit)
            .map_err(|_| TransformError::Disabled("locked"))?;
        if flow == FlowParent::Grid && explicit_grid_placement(model, &set, node) {
            return Err(TransformError::Disabled("grid-explicit-placement"));
        }
        let parent = parent_of(model, node).ok_or(TransformError::Stale)?;
        let container = flow_container(model, parent);
        let container_bounds = node_geometry(scene, container)
            .ok_or(TransformError::Stale)?
            .world_bounds;
        let own = FlowSiblings::read(model, &set, scene, parent, axes, node);
        let dragged = node_geometry(scene, node)
            .ok_or(TransformError::Stale)?
            .world_bounds;
        let key = order_key(model, node).ok_or(TransformError::Stale)?;
        let mut original: Vec<(StableId, Box4)> = Vec::new();
        let mut from = None;
        for (id, k, rect, explicit) in &own.all {
            if from.is_none() && *k > key {
                from = Some(original.len());
                original.push((node, dragged));
            }
            if !explicit {
                original.push((*id, *rect));
            }
        }
        let from = from.unwrap_or_else(|| {
            original.push((node, dragged));
            original.len() - 1
        });
        let outer = MoveDrag::begin(window, scene, None, pointer, false, visible)?;
        Ok(FlowDrag {
            node,
            start: pointer,
            own,
            original: (original, from),
            container_bounds,
            outer,
        })
    }

    /// 끄는 node.
    pub fn node(&self) -> StableId {
        self.node
    }

    /// pointer(world) update. 문서·history를 바꾸지 않습니다.
    pub fn update(
        &self,
        window: &ProjectWindowSession,
        scene: &CanvasScene,
        pointer: [f32; 2],
    ) -> FlowUpdate {
        if inside(self.container_bounds, pointer) {
            let (index, _) = self.own.slot(pointer);
            let (items, from) = &self.original;
            let boxes: Vec<Box4> = items.iter().map(|i| i.1).collect();
            let preview = reflow_preview(&boxes, self.own.axes, *from, index)
                .into_iter()
                .map(|(i, r)| (items[i].0, r))
                .collect();
            return FlowUpdate::Inside {
                index,
                marker: self.marker(index),
                preview,
            };
        }
        if let Some(model) = window.document().map(|d| d.model()) {
            if let Some((parent, siblings)) = other_flow_at(model, scene, pointer, self.node) {
                let (index, _) = siblings.slot(pointer);
                return FlowUpdate::OtherFlow { parent, index };
            }
        }
        FlowUpdate::Outside(self.outer.drop_target(window, scene, pointer))
    }

    /// 삽입 표시: 이웃 자동 형제 사이 주축 위치에 교차축 줄 높이만큼의 선.
    fn marker(&self, index: usize) -> Box4 {
        let auto: Vec<Box4> = self.own.auto().into_iter().map(|a| a.1).collect();
        let (m, c) = (self.own.axes.main, 1 - self.own.axes.main);
        let reverse = self.own.axes.reverse;
        let edge = |r: &Box4, end: bool| {
            if end != reverse {
                r[m] + r[m + 2]
            } else {
                r[m]
            }
        };
        let (at, cross) = match (
            index.checked_sub(1).and_then(|i| auto.get(i)),
            auto.get(index),
        ) {
            (Some(prev), Some(next))
                if (prev[c] - next[c]).abs() < prev[c + 2].max(next[c + 2]) =>
            {
                ((edge(prev, true) + edge(next, false)) / 2.0, *next)
            }
            (_, Some(next)) => (edge(next, false), *next),
            (Some(prev), None) => (edge(prev, true), *prev),
            (None, None) => (self.container_bounds[m], self.container_bounds),
        };
        let mut out = [0.0; 4];
        out[m] = at;
        out[c] = cross[c];
        out[c + 2] = cross[c + 2];
        out
    }

    /// pointer up: 확정해 history 한 항목으로 기록합니다. 기록하면 참입니다.
    pub fn commit(
        &self,
        window: &mut ProjectWindowSession,
        scene: &CanvasScene,
        pointer: [f32; 2],
        request: RequestId,
    ) -> Result<bool, TransformError> {
        match self.update(window, scene, pointer) {
            FlowUpdate::Inside { index, .. } => {
                if index == self.original.1 {
                    return Ok(false);
                }
                let (_, key) = self.own.slot(pointer);
                let command = Command::MoveNode(MoveNode {
                    node: self.node,
                    new_parent: self.own.parent,
                    order_key: key,
                });
                crate::canvas::transform::commit(window, vec![command], history_label::REORDER)
            }
            FlowUpdate::OtherFlow { parent, .. } => {
                let model = window.document().ok_or(TransformError::Stale)?.model();
                let (_, siblings) = other_flow_at(model, scene, pointer, self.node)
                    .filter(|(p, _)| *p == parent)
                    .ok_or(TransformError::Stale)?;
                let (_, key) = siblings.slot(pointer);
                let command = Command::MoveNode(MoveNode {
                    node: self.node,
                    new_parent: parent,
                    order_key: key,
                });
                crate::canvas::transform::commit(window, vec![command], history_label::MOVE)
            }
            FlowUpdate::Outside(target @ DropTarget::Parent(_)) => {
                let update = DragUpdate {
                    delta: [pointer[0] - self.start[0], pointer[1] - self.start[1]],
                    snap: None,
                };
                self.outer.commit(window, scene, &update, target, request)
            }
            // 흐름 자식은 위치를 쓰지 않으므로 놓을 자리가 없으면 그대로 둡니다.
            FlowUpdate::Outside(_) => Ok(false),
        }
    }
}

/// Fragment 부모는 상자가 없으므로 그 위 첫 상자 조상이 흐름 컨테이너입니다.
fn flow_container(model: &DocumentModel, mut parent: StableId) -> StableId {
    while matches!(model.payload(parent), Some(NodePayload::Fragment(_))) {
        match parent_of(model, parent) {
            Some(up) => parent = up,
            None => break,
        }
    }
    parent
}

/// pointer 아래 다른 흐름 컨테이너(끄는 node·그 자손·인스턴스 투영 행 제외)와 그 형제.
fn other_flow_at(
    model: &DocumentModel,
    scene: &CanvasScene,
    pointer: [f32; 2],
    moving: StableId,
) -> Option<(StableId, FlowSiblings)> {
    let set = model_style_set(model);
    let own_parent = parent_of(model, moving);
    for row in scene.frames.iter().rev() {
        if !row.hit_target || row.instance_frame {
            continue;
        }
        if !matches!(model.payload(row.id), Some(NodePayload::Frame(_))) {
            continue;
        }
        let Some(geometry) = node_geometry(scene, row.id) else {
            continue;
        };
        if !inside(geometry.world_bounds, pointer) {
            continue;
        }
        let mut current = Some(row.id);
        let mut descends = false;
        while let Some(id) = current {
            if id == moving {
                descends = true;
                break;
            }
            current = parent_of(model, id);
        }
        if descends || Some(row.id) == own_parent {
            return None;
        }
        let axes = container_axes(model, &set, row.id)?;
        return Some((
            row.id,
            FlowSiblings::read(model, &set, scene, row.id, axes, moving),
        ));
    }
    None
}

/// Frame `container`의 자식이 될 때의 흐름 축. 흐름 컨테이너가 아니면 `None`입니다.
///
/// 자식이 있으면 그 자식의 배치 문맥(`flow_parent`, 기본값 세트 포함)을 그대로 쓰고, 빈 컨테이너면 컨테이너의
/// `display`·`flex-direction`을 풉니다.
fn container_axes(model: &DocumentModel, set: &FlatSet, container: StableId) -> Option<FlowAxes> {
    let layout = resolve(
        set,
        model.payload(container)?,
        model.layout_record(container),
    )?;
    let keyword = |longhand| match &layout.get(longhand).value {
        LayoutValue::Keyword(k) => *k,
        _ => "",
    };
    if let Some(NodePayload::Frame(frame)) = model.payload(container) {
        if frame.layout_mode.is_some() {
            return None;
        }
    }
    let display = keyword(LayoutLonghand::Display);
    if !matches!(display, "flex" | "grid") {
        return None;
    }
    let from_child = model
        .children_in_order(container)
        .into_iter()
        .map(|c| flow_parent(model, set, c))
        .find(|f| f.is_flow());
    let flow = from_child.unwrap_or(match display {
        "grid" => FlowParent::Grid,
        _ => FlowParent::Flex {
            row: matches!(
                keyword(LayoutLonghand::FlexDirection),
                "row" | "row-reverse"
            ),
            reverse: matches!(
                keyword(LayoutLonghand::FlexDirection),
                "row-reverse" | "column-reverse"
            ),
            wrap_reverse: keyword(LayoutLonghand::FlexWrap) == "wrap-reverse",
        },
    });
    FlowAxes::of(flow)
}

/// 흐름 컨테이너 `container`에 새 자식을 pointer(world) 자리로 넣을 OrderKey. 흐름 컨테이너가 아니면 `None`입니다
/// (생성 도구·붙여넣기가 씀).
pub(crate) fn flow_insert_key(
    model: &DocumentModel,
    scene: &CanvasScene,
    container: StableId,
    pointer: [f32; 2],
) -> Option<OrderKey> {
    let set = model_style_set(model);
    let axes = container_axes(model, &set, container)?;
    let siblings = FlowSiblings::read(model, &set, scene, container, axes, StableId::NIL);
    Some(siblings.slot(pointer).1)
}

/// `container`가 흐름 컨테이너인지.
pub(crate) fn is_flow_container(model: &DocumentModel, container: StableId) -> bool {
    container_axes(model, &model_style_set(model), container).is_some()
}

#[cfg(test)]
mod tests;
