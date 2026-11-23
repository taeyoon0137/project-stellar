//! 메뉴 배치 명령의 계획: 정렬·분배·행열 정리·뒤집기·비례 확대 (XII.10.21, XII.10.44, XII.10.12, ADR-0105 결정 1).
//!
//! 상자를 읽는 곳만 호출자마다 다르고 계획은 하나입니다. 앱 runtime은 게시된 캔버스 scene(Text layout 결과 포함)으로,
//! CLI·MCP는 문서 값만으로([`ModelGeometry`]) [`Geometry`]를 채웁니다. 계획은 기존 command만 만들며 호출자가
//! CommandTransaction 하나로 적용합니다(Undo 한 번).

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{Transform2D, Vec2};
use stellar_document::css::style_defaults::FlatSet;
use stellar_document::model::DocumentModel;
use stellar_document::records::NodePayload;

use super::align::{
    align_deltas, distribute_deltas, flow_align, flow_parent, grid_deltas, union, Align,
    Distribute, FlowAlign, Reference,
};
use super::place::{self, PlaceError};
use crate::command::Command;
use crate::flip::{flip_commands, in_parent_space, reflection, FlipAxis};
use crate::scale::{scale_commands, ScalePlan, ScaleTarget};

/// 화면 상자 원천.
pub trait Geometry {
    /// node의 world 경계 상자(회전 반영). 모르면 `None`.
    fn world_bounds(&self, node: StableId) -> Option<[f32; 4]>;
    /// node 부모 local → world. 부모가 없으면 항등.
    fn parent_world(&self, node: StableId) -> Option<Transform2D>;
}

/// 문서 값만으로 정하는 상자(CLI·MCP). Text·Path·인스턴스·Fragment는 layout·정의가 필요해 `None`입니다.
pub struct ModelGeometry<'a> {
    model: &'a DocumentModel,
}

impl<'a> ModelGeometry<'a> {
    /// 모델 위의 상자 원천.
    pub fn new(model: &'a DocumentModel) -> ModelGeometry<'a> {
        ModelGeometry { model }
    }

    fn world_of(&self, node: StableId) -> Option<Transform2D> {
        let mut world = place::local_transform(self.model, node)?;
        let mut parent = crate::tree::parent_of(self.model, node).flatten();
        while let Some(id) = parent {
            if !matches!(self.model.payload(id), Some(NodePayload::Fragment(_))) {
                world = world.then(&place::local_transform(self.model, id)?).ok()?;
            }
            parent = crate::tree::parent_of(self.model, id).flatten();
        }
        Some(world)
    }
}

impl Geometry for ModelGeometry<'_> {
    fn world_bounds(&self, node: StableId) -> Option<[f32; 4]> {
        let local = place::local_box(self.model, node)?;
        bounds_of(&self.world_of(node)?, local)
    }

    fn parent_world(&self, node: StableId) -> Option<Transform2D> {
        let mut parent = crate::tree::parent_of(self.model, node).flatten();
        while let Some(id) = parent {
            if !matches!(self.model.payload(id), Some(NodePayload::Fragment(_))) {
                return self.world_of(id);
            }
            parent = crate::tree::parent_of(self.model, id).flatten();
        }
        Some(Transform2D::identity())
    }
}

/// 변환한 상자의 축 정렬 경계.
pub fn bounds_of(transform: &Transform2D, rect: [f32; 4]) -> Option<[f32; 4]> {
    let [x, y, w, h] = rect;
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for (px, py) in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)] {
        let p = transform.apply(Vec2::new(px, py).ok()?).ok()?;
        min = [min[0].min(p.x()), min[1].min(p.y())];
        max = [max[0].max(p.x()), max[1].max(p.y())];
    }
    Some([min[0], min[1], max[0] - min[0], max[1] - min[1]])
}

/// 계획을 만들 수 없는 이유.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanError {
    /// 선택이 비었습니다.
    Empty,
    /// 상자를 모르는 node(layout이 필요한 종류 또는 없는 node).
    NoGeometry(StableId),
    /// 비활성 조합(도움말 코드: `justify-content`, `flow-child`, `needs-three`, `needs-two`, `no-parent` 등).
    Disabled(&'static str),
    /// 배치 쓰기 오류.
    Place(PlaceError),
}

impl From<PlaceError> for PlanError {
    fn from(error: PlaceError) -> PlanError {
        PlanError::Place(error)
    }
}

fn boxes(geometry: &dyn Geometry, nodes: &[StableId]) -> Result<Vec<[f32; 4]>, PlanError> {
    nodes
        .iter()
        .map(|&n| geometry.world_bounds(n).ok_or(PlanError::NoGeometry(n)))
        .collect()
}

fn parent_vector(
    geometry: &dyn Geometry,
    node: StableId,
    v: [f32; 2],
) -> Result<[f32; 2], PlanError> {
    let inverse = geometry
        .parent_world(node)
        .and_then(|p| p.inverse())
        .ok_or(PlanError::NoGeometry(node))?;
    let (a, b, c, d, _, _) = inverse.components();
    Ok([a * v[0] + c * v[1], b * v[0] + d * v[1]])
}

fn translate_all(
    model: &DocumentModel,
    geometry: &dyn Geometry,
    nodes: &[StableId],
    world_deltas: &[[f32; 2]],
    new_modifier: &mut dyn FnMut() -> StableId,
) -> Result<Vec<Command>, PlanError> {
    let mut commands = Vec::new();
    for (&node, delta) in nodes.iter().zip(world_deltas) {
        let [dx, dy] = parent_vector(geometry, node, *delta)?;
        if let Some(command) = place::translate(model, node, dx, dy, new_modifier)? {
            commands.push(command);
        }
    }
    Ok(commands)
}

fn parent_of(model: &DocumentModel, node: StableId) -> Option<StableId> {
    crate::tree::parent_of(model, node).flatten()
}

/// 정렬 6종 하나. `nodes`는 다른 선택의 자손을 뺀 선택이고 `key`는 기준 객체(primary)입니다. 단일 선택은 부모 기준,
/// 다중 선택은 `key`가 있으면 그 객체, 없으면 선택 경계 기준입니다. 흐름 자식은 `align-self`·`justify-self`만 바꾸고
/// 주축 정렬은 비활성입니다.
pub fn align(
    model: &DocumentModel,
    set: &FlatSet,
    geometry: &dyn Geometry,
    nodes: &[StableId],
    align: Align,
    key: Option<StableId>,
    new_modifier: &mut dyn FnMut() -> StableId,
) -> Result<Vec<Command>, PlanError> {
    if nodes.is_empty() {
        return Err(PlanError::Empty);
    }
    let mut commands = Vec::new();
    let mut free = Vec::new();
    for &node in nodes {
        match flow_align(model, set, node, align) {
            FlowAlign::Free => free.push(node),
            FlowAlign::Child(command) => commands.push(Command::SetLayoutProperty(command)),
            FlowAlign::Disabled { hint } => {
                return Err(PlanError::Disabled(hint.unwrap_or("flow-child")))
            }
        }
    }
    if free.is_empty() {
        return Ok(commands);
    }
    let boxes = boxes(geometry, &free)?;
    let reference = if nodes.len() == 1 {
        let parent = parent_of(model, free[0]).ok_or(PlanError::Disabled("no-parent"))?;
        // 부모가 Fragment면 그 위 상자 조상입니다.
        let mut container = Some(parent);
        while let Some(id) = container {
            if !matches!(model.payload(id), Some(NodePayload::Fragment(_))) {
                break;
            }
            container = parent_of(model, id);
        }
        let container = container.ok_or(PlanError::Disabled("no-parent"))?;
        Reference::Parent(
            geometry
                .world_bounds(container)
                .ok_or(PlanError::NoGeometry(container))?,
        )
    } else {
        match key.and_then(|k| free.iter().position(|&n| n == k)) {
            Some(index) => Reference::Key(index),
            None => Reference::Selection,
        }
    };
    let deltas = align_deltas(&boxes, align, reference).ok_or(PlanError::Disabled("empty-box"))?;
    commands.extend(translate_all(
        model,
        geometry,
        &free,
        &deltas,
        new_modifier,
    )?);
    Ok(commands)
}

fn free_boxes(
    model: &DocumentModel,
    set: &FlatSet,
    geometry: &dyn Geometry,
    nodes: &[StableId],
) -> Result<Vec<[f32; 4]>, PlanError> {
    if nodes.iter().any(|&n| flow_parent(model, set, n).is_flow()) {
        return Err(PlanError::Disabled("flow-child"));
    }
    boxes(geometry, nodes)
}

/// 분배 2종 하나(3개 이상, 가장 바깥 두 객체 고정). 흐름 자식이 있으면 비활성입니다.
pub fn distribute(
    model: &DocumentModel,
    set: &FlatSet,
    geometry: &dyn Geometry,
    nodes: &[StableId],
    distribute: Distribute,
    new_modifier: &mut dyn FnMut() -> StableId,
) -> Result<Vec<Command>, PlanError> {
    let boxes = free_boxes(model, set, geometry, nodes)?;
    let deltas = distribute_deltas(&boxes, distribute).ok_or(PlanError::Disabled("needs-three"))?;
    translate_all(model, geometry, nodes, &deltas, new_modifier)
}

/// 행열 정리. 흐름 컨테이너를 만들지 않고 위치만 바꿉니다. 흐름 자식이 있으면 비활성입니다.
pub fn arrange_grid(
    model: &DocumentModel,
    set: &FlatSet,
    geometry: &dyn Geometry,
    nodes: &[StableId],
    gap: f32,
    new_modifier: &mut dyn FnMut() -> StableId,
) -> Result<Vec<Command>, PlanError> {
    let boxes = free_boxes(model, set, geometry, nodes)?;
    let deltas = grid_deltas(&boxes, gap).ok_or(PlanError::Disabled("needs-two"))?;
    translate_all(model, geometry, nodes, &deltas, new_modifier)
}

/// 뒤집기: 선택 world 경계 중심을 지나는 world 축의 거울상([`crate::flip`] 결정).
pub fn flip(
    model: &DocumentModel,
    geometry: &dyn Geometry,
    nodes: &[StableId],
    axis: FlipAxis,
    new_modifier: &mut dyn FnMut() -> StableId,
) -> Result<Vec<Command>, PlanError> {
    if nodes.is_empty() {
        return Err(PlanError::Empty);
    }
    if let Some(&node) = nodes.iter().find(|&&n| !place::accepts_linear(model, n)) {
        return Err(PlanError::Place(PlaceError::Unsupported(node)));
    }
    let bounds = union(&boxes(geometry, nodes)?).ok_or(PlanError::Empty)?;
    let center = [bounds[0] + bounds[2] / 2.0, bounds[1] + bounds[3] / 2.0];
    let world = reflection(axis, center).ok_or(PlanError::Disabled("non-finite"))?;
    let changes = nodes
        .iter()
        .map(|&n| {
            geometry
                .parent_world(n)
                .and_then(|p| in_parent_space(world, p))
                .map(|c| (n, c))
                .ok_or(PlanError::NoGeometry(n))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(flip_commands(model, &changes, new_modifier)?)
}

/// 비례 확대 기준점.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleOrigin {
    /// 선택 world 경계 왼쪽 위.
    TopLeft,
    /// 선택 world 경계 중심.
    Center,
}

/// 비례 확대: 선택 world 경계의 `origin`을 고정하고 `factor`배 합니다.
pub fn scale(
    model: &DocumentModel,
    geometry: &dyn Geometry,
    nodes: &[StableId],
    factor: f32,
    origin: ScaleOrigin,
    new_modifier: &mut dyn FnMut() -> StableId,
) -> Result<ScalePlan, PlanError> {
    let bounds = union(&boxes(geometry, nodes)?).ok_or(PlanError::Empty)?;
    let anchor = match origin {
        ScaleOrigin::TopLeft => [bounds[0], bounds[1]],
        ScaleOrigin::Center => [bounds[0] + bounds[2] / 2.0, bounds[1] + bounds[3] / 2.0],
    };
    let targets = nodes
        .iter()
        .map(|&node| {
            let inverse = geometry
                .parent_world(node)
                .and_then(|p| p.inverse())
                .ok_or(PlanError::NoGeometry(node))?;
            let p = inverse
                .apply(
                    Vec2::new(anchor[0], anchor[1])
                        .map_err(|_| PlanError::Disabled("non-finite"))?,
                )
                .map_err(|_| PlanError::Disabled("non-finite"))?;
            Ok(ScaleTarget {
                node,
                anchor: [p.x(), p.y()],
            })
        })
        .collect::<Result<Vec<_>, PlanError>>()?;
    Ok(scale_commands(model, &targets, factor, new_modifier)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrange::align::model_style_set;
    use crate::arrange::test_fixtures::*;

    #[test]
    fn model_geometry_reads_document_boxes_and_rejects_layout_dependent_kinds() {
        let model = shapes();
        let geometry = ModelGeometry::new(&model);
        assert_eq!(
            geometry.world_bounds(sid(11)),
            Some([10.0, 20.0, 100.0, 50.0])
        );
        assert_eq!(
            geometry.world_bounds(sid(13)),
            Some([30.0, 40.0, 40.0, 20.0])
        );
        assert_eq!(geometry.world_bounds(sid(14)), Some([5.0, 5.0, 20.0, 10.0]));
        assert_eq!(geometry.world_bounds(sid(16)), None);
        let set = model_style_set(&model);
        let mut ids = ids();
        let commands = align(
            &model,
            &set,
            &geometry,
            &[sid(11), sid(13)],
            Align::Left,
            None,
            &mut ids,
        )
        .unwrap();
        let (state, _) = apply(model, commands);
        let geometry = ModelGeometry::new(state.model());
        assert_eq!(geometry.world_bounds(sid(13)).unwrap()[0], 10.0);
        assert_eq!(
            align(
                state.model(),
                &set,
                &geometry,
                &[sid(16)],
                Align::Left,
                None,
                &mut ids
            ),
            Err(PlanError::NoGeometry(sid(16)))
        );
    }
}
