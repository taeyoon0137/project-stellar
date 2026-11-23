//! 이동 드래그 (XII.10.13, ADR-0105 결정 2).
//!
//! - Shift는 축 고정(시작점에서 더 많이 움직인 축만), Option으로 시작하면 복제 후 복제본 이동입니다.
//! - 스냅을 켜면 update마다 [`crate::canvas::snap::snap`]으로 선택 경계를 맞춥니다(후보는 시작 때 한 번).
//! - 자유 배치 컨테이너 사이로 끌어 놓으면 world 위치를 지킨 채 `MoveNode`로 부모를 바꿉니다([`DropTarget`]).
//!   흐름 컨테이너(flex·grid) 안으로 놓기는 XII.10.24 몫이라 이 행에서는 거부하고, ComponentInstance 투영 행 안으로는
//!   놓지 않습니다(공개 Slot 배치는 Slot 편집 행이 열 때까지 거부).
//! - 같은 부모 안 이동·복제는 기존 캔버스 이동 경로(`CanvasEdit::Move`, ADR-0071 UX.3.7) 하나로 확정합니다. 부모를 바꾸는
//!   확정만 이 module이 command를 만들며, 위치 쓰기는 같은 `stellar_commands::arrange::place`입니다.
//! - update는 문서를 바꾸지 않으며 확정은 history 한 항목입니다.

use std::collections::BTreeMap;

use stellar_commands::arrange::align::{flow_parent, model_style_set};
use stellar_commands::arrange::place::{self, PlaceError};
use stellar_commands::command::Command;
use stellar_commands::duplicate::DuplicateSubtree;
use stellar_commands::node_ops::MoveNode;
use stellar_commands::selection::Selection;
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::value::Vec2;
use stellar_document::layout_record::{LayoutLonghand, LayoutValue};
use stellar_document::model::DocumentModel;
use stellar_document::modifier::ModifierId;
use stellar_document::records::NodePayload;

use super::{geometries, selection_roots, union, NodeGeometry, TransformError};
use crate::app::canvas_gesture::{apply_edit, CanvasEdit};
use crate::app::RequestId;
use crate::canvas::snap::{
    self, CollectOptions, SnapCandidates, SnapMode, SnapResult, SnapSettings,
};
use crate::canvas::CanvasScene;
use crate::window_session::{history_label, ProjectWindowSession};

/// 놓을 자리.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropTarget {
    /// 지금 부모에 그대로 둡니다.
    Stay,
    /// 이 자유 배치 Frame으로 부모를 바꿉니다.
    Parent(StableId),
    /// 이 자리에는 놓을 수 없어 지금 부모에 둡니다(`flow-container`, `instance-slot`).
    Rejected(&'static str),
}

/// update 결과.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DragUpdate {
    /// world 이동량(축 고정·스냅 반영).
    pub delta: [f32; 2],
    /// 스냅 결과(부모 local, 표시용).
    pub snap: Option<SnapResult>,
}

/// 이동 드래그 상태.
#[derive(Clone, Debug)]
pub struct MoveDrag {
    view: StableId,
    anchor: Option<StableId>,
    start: [f32; 2],
    duplicate: bool,
    targets: Vec<NodeGeometry>,
    candidates: Option<SnapCandidates>,
}

impl MoveDrag {
    /// 지금 선택으로 시작합니다. `anchor`는 누른 행(선택이 덮어야 함), `pointer`는 누른 world 점, `duplicate`는 Option.
    /// 스냅 후보는 대상이 한 부모를 공유할 때 시작 때 한 번 모읍니다(`visible`은 가시 world 영역).
    pub fn begin(
        window: &ProjectWindowSession,
        scene: &CanvasScene,
        anchor: Option<StableId>,
        pointer: [f32; 2],
        duplicate: bool,
        visible: Option<[f32; 4]>,
    ) -> Result<Self, TransformError> {
        let view = window.tabs().active_view().ok_or(TransformError::Stale)?;
        let (model, _root, roots) = selection_roots(window)?;
        if roots.is_empty() {
            return Err(TransformError::Stale);
        }
        let targets = geometries(scene, &roots)?;
        let set = model_style_set(model);
        let options = CollectOptions {
            visible,
            flow_child: roots.iter().any(|&n| flow_parent(model, &set, n).is_flow()),
            ..CollectOptions::default()
        };
        let candidates = snap::collect_in(scene, &roots, &options);
        Ok(MoveDrag {
            view,
            anchor,
            start: pointer,
            duplicate,
            targets,
            candidates,
        })
    }

    /// pointer(world) update. `snap`은 (켜진 스냅, 부모 local px 허용 오차)입니다. 문서를 바꾸지 않습니다.
    pub fn update(
        &self,
        pointer: [f32; 2],
        shift: bool,
        snapping: Option<(SnapSettings, f32)>,
    ) -> Option<DragUpdate> {
        let mut delta = [pointer[0] - self.start[0], pointer[1] - self.start[1]];
        if !delta.iter().all(|v| v.is_finite()) {
            return None;
        }
        let lock = shift.then(|| {
            if delta[0].abs() >= delta[1].abs() {
                1
            } else {
                0
            }
        });
        if let Some(axis) = lock {
            delta[axis] = 0.0;
        }
        let mut result = None;
        if let (Some((settings, tolerance)), Some(candidates)) = (snapping, &self.candidates) {
            let parent = self.targets[0].parent_world;
            let (a, b, c, d, _, _) = parent.components();
            let inverse = parent.inverse()?;
            let (ia, ib, ic, id, _, _) = inverse.components();
            let local = [ia * delta[0] + ic * delta[1], ib * delta[0] + id * delta[1]];
            let bounds = union(self.targets.iter().filter_map(NodeGeometry::parent_bounds))?;
            let moved = [
                bounds[0] + local[0],
                bounds[1] + local[1],
                bounds[2],
                bounds[3],
            ];
            let snapped = snap::snap(moved, SnapMode::Move, tolerance, settings, candidates)?;
            let mut adjust = [snapped.rect[0] - moved[0], snapped.rect[1] - moved[1]];
            if let Some(axis) = lock {
                // 축 고정 축은 스냅하지 않습니다(부모가 회전하지 않은 보통 경우의 축).
                adjust[axis] = 0.0;
            }
            delta[0] += a * adjust[0] + c * adjust[1];
            delta[1] += b * adjust[0] + d * adjust[1];
            if let Some(axis) = lock {
                delta[axis] = 0.0;
            }
            result = Some(snapped);
        }
        Some(DragUpdate {
            delta,
            snap: result,
        })
    }

    /// pointer 아래 놓을 자리. 움직이는 node와 그 자손, 잠기거나 숨은 행, 다른 View의 행은 건너뜁니다.
    pub fn drop_target(
        &self,
        window: &ProjectWindowSession,
        scene: &CanvasScene,
        pointer: [f32; 2],
    ) -> DropTarget {
        let Some(document) = window.document() else {
            return DropTarget::Stay;
        };
        let model = document.model();
        let set = model_style_set(model);
        let current = crate::app::canvas_gesture::parent_of(model, self.targets[0].id);
        let moving: Vec<StableId> = self.targets.iter().map(|t| t.id).collect();
        let Ok(point) = Vec2::new(pointer[0], pointer[1]) else {
            return DropTarget::Stay;
        };
        for row in scene.frames.iter().rev() {
            if row.rectangle_shape
                || row.path_shape
                || row.text_node
                || row.image_asset.is_some()
                || !row.hit_target
            {
                continue;
            }
            let Some(local) = row.world.inverse().and_then(|inv| inv.apply(point).ok()) else {
                continue;
            };
            let r = row.geometry.rect;
            let inside = local.x() >= r.origin().x()
                && local.y() >= r.origin().y()
                && local.x() <= r.origin().x() + r.size().width()
                && local.y() <= r.origin().y() + r.size().height();
            if !inside {
                continue;
            }
            if row.instance_frame || !matches!(model.payload(row.id), Some(NodePayload::Frame(_))) {
                return DropTarget::Rejected("instance-slot");
            }
            if within(model, row.id, &moving) {
                continue;
            }
            if Some(row.id) == current {
                return DropTarget::Stay;
            }
            let Some(NodePayload::Frame(frame)) = model.payload(row.id) else {
                continue;
            };
            let display = stellar_document::css::style_defaults::resolve(
                &set,
                model.payload(row.id).expect("frame"),
                model.layout_record(row.id),
            )
            .map(|l| l.get(LayoutLonghand::Display).value.clone());
            if frame.layout_mode.is_some()
                || matches!(display, Some(LayoutValue::Keyword("flex" | "grid")))
            {
                return DropTarget::Rejected("flow-container");
            }
            return DropTarget::Parent(row.id);
        }
        DropTarget::Stay
    }

    /// pointer up: 확정해 창 history 한 항목으로 기록합니다. `request`는 command ID를 만드는 요청 번호입니다.
    pub fn commit(
        &self,
        window: &mut ProjectWindowSession,
        scene: &CanvasScene,
        update: &DragUpdate,
        target: DropTarget,
        request: RequestId,
    ) -> Result<bool, TransformError> {
        let parent = match target {
            DropTarget::Parent(parent) => parent,
            DropTarget::Stay | DropTarget::Rejected(_) => {
                let delta = Vec2::new(update.delta[0], update.delta[1])
                    .map_err(|_| TransformError::Disabled("non-finite"))?;
                if delta == Vec2::ZERO && !self.duplicate {
                    return Ok(false);
                }
                let before = window.history().history().len();
                apply_edit(
                    window,
                    scene,
                    request,
                    self.view,
                    CanvasEdit::Move {
                        anchor: self.anchor,
                        delta,
                        parent_space: false,
                        duplicate: self.duplicate,
                    },
                )
                .map_err(|_| TransformError::Stale)?;
                return Ok(window.history().history().len() > before);
            }
        };
        let new_parent_world = super::node_geometry(scene, parent)
            .map(|g| g.local.then(&g.parent_world))
            .ok_or(TransformError::Stale)?
            .map_err(|_| TransformError::Stale)?;
        let to_new = new_parent_world.inverse().ok_or(TransformError::Stale)?;
        let shift = place::translation(update.delta[0], update.delta[1])
            .ok_or(TransformError::Disabled("non-finite"))?;
        let model = window.document().ok_or(TransformError::Stale)?.model();
        let mut ids = super::FreshIds::default();
        let mut commands = Vec::new();
        let mut copies = Vec::new();
        let mut order: Option<OrderKey> = None;
        for target in &self.targets {
            // 옛 부모 공간 → world 이동 → 새 부모 공간.
            let change = target
                .parent_world
                .then(&shift)
                .and_then(|t| t.then(&to_new))
                .map_err(|_| PlaceError::NonFinite(target.id))?;
            let key = next_key(model, parent, order.as_ref());
            order = Some(key.clone());
            if self.duplicate {
                // 복제본을 새 부모에 바로 만듭니다(원본은 그대로).
                let plan =
                    DuplicateSubtree::plan_with(model, target.id, parent, key, &mut || ids.fresh())
                        .map_err(|_| TransformError::Stale)?;
                let copy = plan.copy_root().ok_or(TransformError::Stale)?;
                let remap: BTreeMap<StableId, StableId> = plan.remap.iter().copied().collect();
                commands.push(Command::DuplicateSubtree(plan));
                copies.push(copy);
                if let Some(command) =
                    place::apply_affine(model, target.id, change, &mut || ids.fresh())?
                {
                    commands.push(retarget(command, copy, Some(&remap))?);
                }
                continue;
            }
            if let Some(command) =
                place::apply_affine(model, target.id, change, &mut || ids.fresh())?
            {
                commands.push(command);
            }
            commands.push(Command::MoveNode(MoveNode {
                node: target.id,
                new_parent: parent,
                order_key: key,
            }));
        }
        ids.finish()?;
        let tx = super::transaction(window, commands)?;
        if self.duplicate {
            let primary = copies.last().copied();
            window.apply_and_select(
                &tx,
                Selection::new(copies, primary),
                history_label::DUPLICATE,
            )
        } else {
            window.apply(&tx, history_label::MOVE)
        }
        .map_err(|error| TransformError::Edit(format!("{error:?}")))?;
        Ok(true)
    }
}

/// 새 부모의 맨 앞(마지막 자식 뒤) 자리. 앞서 놓은 node가 있으면 그 뒤입니다.
fn next_key(model: &DocumentModel, parent: StableId, previous: Option<&OrderKey>) -> OrderKey {
    match previous {
        Some(previous) => OrderKey::between(Some(previous), None),
        None => model.next_child_order_key(parent),
    }
}

/// `node`가 `moving` 가운데 하나이거나 그 자손인지.
fn within(model: &DocumentModel, node: StableId, moving: &[StableId]) -> bool {
    let mut current = Some(node);
    while let Some(id) = current {
        if moving.contains(&id) {
            return true;
        }
        current = crate::app::canvas_gesture::parent_of(model, id);
    }
    false
}

/// 원본 기준 command를 복제본으로 옮깁니다(modifier id는 remap).
fn retarget(
    command: Command,
    node: StableId,
    remap: Option<&BTreeMap<StableId, StableId>>,
) -> Result<Command, TransformError> {
    Ok(match command {
        Command::SetProperty(mut set) => {
            set.node = node;
            Command::SetProperty(set)
        }
        Command::SetModifierProperty(mut set) => {
            set.node = node;
            if let Some(map) = remap {
                set.modifier = ModifierId::new(
                    *map.get(&set.modifier.stable_id())
                        .ok_or(TransformError::Stale)?,
                );
            }
            Command::SetModifierProperty(set)
        }
        Command::InsertModifier(mut insert) => {
            insert.node = node;
            Command::InsertModifier(insert)
        }
        other => other,
    })
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::*;
    use super::*;
    use crate::window_session::test_fixtures::sid;

    fn world_origin(window: &ProjectWindowSession, node: u64) -> [f32; 2] {
        let g = super::super::node_geometry(&scene(window), sid(node)).unwrap();
        [g.world_bounds[0], g.world_bounds[1]]
    }

    #[test]
    fn shift_locks_the_dominant_axis_and_one_commit_is_one_entry() {
        let (_dir, mut w) = open("drag-axis", &canvas_document());
        select(&mut w, &[13]);
        let s = scene(&w);
        let drag = MoveDrag::begin(&w, &s, Some(sid(13)), [40.0, 105.0], false, None).unwrap();
        let before = (history_len(&w), w.revision());
        let mut last = None;
        for step in 0..500 {
            last = drag.update(
                [40.0 + step as f32 * 0.06, 105.0 + step as f32 * 0.01],
                true,
                None,
            );
        }
        assert_eq!((history_len(&w), w.revision()), before);
        let update = last.unwrap();
        assert_eq!(update.delta[1], 0.0);
        assert!(drag
            .commit(&mut w, &s, &update, DropTarget::Stay, RequestId(1))
            .unwrap());
        assert_eq!(history_len(&w), before.0 + 1);
        let (_, _, _, _, x, y) = matrix(&w, 13);
        assert!((x - (30.0 + update.delta[0])).abs() < 1e-4 && y == 100.0);
    }

    #[test]
    fn option_drag_duplicates_and_moves_the_copy_in_one_entry() {
        let (_dir, mut w) = open("drag-duplicate", &canvas_document());
        select(&mut w, &[13]);
        let s = scene(&w);
        let drag = MoveDrag::begin(&w, &s, Some(sid(13)), [40.0, 105.0], true, None).unwrap();
        let update = drag.update([60.0, 105.0], false, None).unwrap();
        let before = history_len(&w);
        drag.commit(&mut w, &s, &update, DropTarget::Stay, RequestId(2))
            .unwrap();
        assert_eq!(history_len(&w), before + 1);
        // 원본은 그대로, 선택은 복제본.
        assert!(close(matrix(&w, 13), (1., 0., 0., 1., 30., 100.)));
        let copy = w.view().selection().ids()[0];
        assert_ne!(copy, sid(13));
        let (_, _, _, _, x, _) = stellar_commands::arrange::place::local_transform(model(&w), copy)
            .unwrap()
            .components();
        assert_eq!(x, 50.0);
    }

    #[test]
    fn dropping_into_a_free_frame_reparents_and_keeps_the_world_position() {
        let (_dir, mut w) = open("drag-reparent", &canvas_document());
        select(&mut w, &[13]);
        let s = scene(&w);
        let before = world_origin(&w, 13);
        let drag = MoveDrag::begin(&w, &s, Some(sid(13)), [40.0, 105.0], false, None).unwrap();
        // Frame 11(10..110, 20..70) 안으로 끌어 (-10, -70): 상자 (20, 30).
        let update = drag.update([30.0, 35.0], false, None).unwrap();
        let target = drag.drop_target(&w, &s, [30.0, 35.0]);
        assert_eq!(target, DropTarget::Parent(sid(11)));
        drag.commit(&mut w, &s, &update, target, RequestId(3))
            .unwrap();
        assert_eq!(
            crate::app::canvas_gesture::parent_of(model(&w), sid(13)),
            Some(sid(11))
        );
        let after = world_origin(&w, 13);
        assert!(
            (after[0] - (before[0] - 10.0)).abs() < 1e-3
                && (after[1] - (before[1] - 70.0)).abs() < 1e-3
        );
        // 새 부모 local 위치: (20 - 10, 30 - 20).
        assert!(close(matrix(&w, 13), (1., 0., 0., 1., 10., 10.)));
        // undo 한 번으로 원래 부모·위치.
        assert!(w.undo().unwrap());
        assert_eq!(
            crate::app::canvas_gesture::parent_of(model(&w), sid(13)),
            Some(sid(10))
        );
        assert!(close(matrix(&w, 13), (1., 0., 0., 1., 30., 100.)));
    }

    #[test]
    fn flow_containers_are_rejected_as_drop_targets() {
        use stellar_commands::style_defaults::SetLayoutProperty;
        let (_dir, mut w) = open("drag-flow", &canvas_document());
        let grid = vec![Command::SetLayoutProperty(SetLayoutProperty {
            node: sid(11),
            longhand: LayoutLonghand::Display,
            value: LayoutValue::Keyword("grid"),
        })];
        super::super::commit(&mut w, grid, 0).unwrap();
        select(&mut w, &[13]);
        let s = scene(&w);
        let drag = MoveDrag::begin(&w, &s, Some(sid(13)), [40.0, 105.0], false, None).unwrap();
        assert_eq!(
            drag.drop_target(&w, &s, [30.0, 35.0]),
            DropTarget::Rejected("flow-container")
        );
        // 지금 부모 안 빈 곳이면 그대로 둡니다.
        assert_eq!(drag.drop_target(&w, &s, [300.0, 250.0]), DropTarget::Stay);
    }

    #[test]
    fn snapping_adjusts_the_world_delta_to_a_sibling_edge() {
        let (_dir, mut w) = open("drag-snap", &canvas_document());
        select(&mut w, &[13]);
        let s = scene(&w);
        let drag = MoveDrag::begin(&w, &s, Some(sid(13)), [40.0, 105.0], false, None).unwrap();
        let settings = SnapSettings {
            objects: true,
            pixels: false,
        };
        // 13 왼쪽(30)을 14 왼쪽(100) 근처 98로 끌면 100에 맞춥니다.
        let update = drag
            .update([108.0, 133.0], false, Some((settings, 4.0)))
            .unwrap();
        assert_eq!(update.delta[0], 70.0);
        assert!(update.snap.is_some());
    }
}
