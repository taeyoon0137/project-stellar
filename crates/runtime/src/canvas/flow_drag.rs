//! 흐름 컨테이너(flex·grid) 안 드래그: 삽입 위치, 순서 preview, OrderKey 확정 (XII.10.24, ADR-0117 결정 5).
//!
//! - 삽입 위치([`insertion_index`])는 끄는 node를 뺀 형제의 화면 상자(게시된 scene)를 흐름 순서로 보고, 교차축으로
//!   겹치는 상자를 한 줄로 묶어 pointer가 있는 줄에서 주축 중심이 pointer보다 앞인 형제 수를 셉니다. `flex-wrap`의
//!   여러 줄과 Grid 자동 배치(읽는 순서)를 같은 규칙으로 다룹니다.
//! - 상자는 layout 결과로 그린 scene source([`CanvasLayout`], ADR-0125)에서 읽습니다. 흐름 자식의 화면 위치는 layout
//!   결과이므로 저장 transform이 아니라 이 상자로 셉니다.
//! - preview는 실제 layout을 다시 배치한 결과입니다([`CanvasLayout::reorder_preview`]): 흐름 순서를 바꾼 입력을 증분
//!   layout에 넣고, 상자가 바뀐 형제의 scene 대체 입력(`ScenePreview`)과 world 상자를 냅니다. 크기가 다른 항목,
//!   `justify-content` 분배, wrap도 놓은 뒤 화면과 같습니다.
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
use crate::canvas::scene_layout::CanvasLayout;
use crate::canvas::transform::drag::{DragUpdate, DropTarget, MoveDrag};
use crate::canvas::transform::{selection_roots, TransformError};
use crate::canvas::CanvasScene;
use crate::window_session::{history_label, ProjectWindowSession};
use stellar_scene::incremental::ScenePreview;

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
        layout: &CanvasLayout,
        parent: StableId,
        axes: FlowAxes,
        skip: StableId,
    ) -> FlowSiblings {
        let all = model
            .children_in_order(parent)
            .into_iter()
            .filter(|&id| id != skip)
            .filter_map(|id| {
                let rect = layout.world_box(id)?;
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
        /// 새 흐름 순서의 (node, preview world 상자). 끄는 node도 들어 있습니다.
        preview: Vec<(StableId, Box4)>,
        /// 상자가 바뀐 node의 scene 대체 입력(`IncrementalScene::set_preview`에 넘김).
        scene: ScenePreview,
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
    /// 끄는 node의 원래 자동 배치 흐름 index(끄는 node를 넣은 목록 기준).
    from: usize,
    /// 상자를 가진 흐름 컨테이너(Fragment 부모면 그 위 첫 상자).
    container: StableId,
    container_bounds: Box4,
    outer: MoveDrag,
}

fn inside(r: Box4, p: [f32; 2]) -> bool {
    p[0] >= r[0] && p[1] >= r[1] && p[0] <= r[0] + r[2] && p[1] <= r[1] + r[3]
}

impl FlowDrag {
    /// 지금 선택(흐름 자식 하나)으로 시작합니다. `pointer`는 누른 world 점, `visible`은 가시 world 영역입니다.
    /// 상자는 `layout`(layout 결과로 그린 source)에서, 인스턴스 투영 행과 끌어내기 대상은 `scene`에서 읽습니다.
    pub fn begin(
        window: &ProjectWindowSession,
        scene: &CanvasScene,
        layout: &CanvasLayout,
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
        let container_bounds = layout.world_box(container).ok_or(TransformError::Stale)?;
        let own = FlowSiblings::read(model, &set, layout, parent, axes, node);
        layout.world_box(node).ok_or(TransformError::Stale)?;
        let key = order_key(model, node).ok_or(TransformError::Stale)?;
        let from = own
            .all
            .iter()
            .filter(|s| !s.3)
            .take_while(|s| s.1 < key)
            .count();
        let outer = MoveDrag::begin(window, scene, None, pointer, false, visible)?;
        Ok(FlowDrag {
            node,
            start: pointer,
            own,
            from,
            container,
            container_bounds,
            outer,
        })
    }

    /// 자동 형제 index `index` 자리에 끄는 node를 둔 layout 자식 순서. 다음 자동 형제 바로 앞, 없으면 끝입니다.
    fn reordered(&self, layout: &CanvasLayout, index: usize) -> Vec<StableId> {
        let mut order: Vec<StableId> = layout
            .layout()
            .layout_children(self.container)
            .into_iter()
            .filter(|id| *id != self.node)
            .collect();
        let auto = self.own.auto();
        let at = auto
            .get(index)
            .and_then(|(i, _)| order.iter().position(|id| *id == self.own.all[*i].0))
            .unwrap_or(order.len());
        order.insert(at, self.node);
        order
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
        layout: &CanvasLayout,
        pointer: [f32; 2],
    ) -> FlowUpdate {
        let model = window.document().map(|d| d.model());
        if inside(self.container_bounds, pointer) {
            let (index, _) = self.own.slot(pointer);
            let order = self.reordered(layout, index);
            let flow = model.and_then(|m| layout.reorder_preview(m, self.container, &order));
            let auto: std::collections::HashSet<StableId> = self
                .own
                .auto()
                .into_iter()
                .map(|(i, _)| self.own.all[i].0)
                .chain(std::iter::once(self.node))
                .collect();
            let preview = order
                .iter()
                .filter(|id| auto.contains(id))
                .filter_map(|id| {
                    let rect = flow
                        .as_ref()
                        .and_then(|f| layout.preview_box(f, *id))
                        .or_else(|| layout.world_box(*id))?;
                    Some((*id, rect))
                })
                .collect();
            return FlowUpdate::Inside {
                index,
                marker: self.marker(index),
                preview,
                scene: flow.map(|f| f.preview).unwrap_or_default(),
            };
        }
        if let Some(model) = model {
            if let Some((parent, siblings)) =
                other_flow_at(model, scene, layout, pointer, self.node)
            {
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
        layout: &CanvasLayout,
        pointer: [f32; 2],
        request: RequestId,
    ) -> Result<bool, TransformError> {
        match self.update(window, scene, layout, pointer) {
            FlowUpdate::Inside { index, .. } => {
                if index == self.from {
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
                let (_, siblings) = other_flow_at(model, scene, layout, pointer, self.node)
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
    layout: &CanvasLayout,
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
        let Some(bounds) = layout.world_box(row.id) else {
            continue;
        };
        if !inside(bounds, pointer) {
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
            FlowSiblings::read(model, &set, layout, row.id, axes, moving),
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
    _scene: &CanvasScene,
    container: StableId,
    pointer: [f32; 2],
) -> Option<OrderKey> {
    let set = model_style_set(model);
    let axes = container_axes(model, &set, container)?;
    // 형제 상자는 layout 결과 위치입니다. 이 한 번의 삽입을 위해 컨테이너가 든 View를 변환합니다.
    let mut root = container;
    while let Some(up) = parent_of(model, root) {
        root = up;
    }
    let layout = CanvasLayout::for_model(model, root, None).ok()?;
    let siblings = FlowSiblings::read(model, &set, &layout, container, axes, StableId::NIL);
    Some(siblings.slot(pointer).1)
}

/// `container`가 흐름 컨테이너인지.
pub(crate) fn is_flow_container(model: &DocumentModel, container: StableId) -> bool {
    container_axes(model, &model_style_set(model), container).is_some()
}

#[cfg(test)]
mod tests;
