//! 크기 조절 제스처 (XII.10.9, ADR-0105 결정 2).
//!
//! - 상자 계산은 핸들 미리보기와 같은 `gesture_math::preview`입니다(Shift 비율 유지, Option 중심 기준, 최소 1px).
//! - **비율 고정**: node에 명시 `aspect-ratio`가 있으면 비율 유지가 기본이고 Shift가 일시 해제합니다.
//! - **뒤집기**: pointer가 고정된 반대편(Option이면 중심)을 넘으면 그 축을 뒤집습니다. 단일 선택은 핸들이 local 좌표라
//!   node local 축 기준이며, 고정된 반대편 변의 world 위치가 그대로 남습니다. 표현은 위치 변환 선형부의 그 축 부호
//!   반전입니다(`scale` longhand가 없음, `stellar_commands::flip` 결정).
//! - **다중 선택**: 선택 경계(공통 부모 공간)를 같은 비율로 늘리고, node마다 자기 축 방향 길이 비율로 크기를 바꿉니다.
//!   회전한 node도 기울지 않습니다(회전은 유지, 뒤집으면 거울상). 크기가 없는 node(Text·인스턴스)는 중심만 옮깁니다.
//! - **흐름 자식**: 조절한 축의 `width`·`height`만 씁니다(위치·`flex-*`·`align-self` 불변, ADR-0074 결정 4).
//! - 확정은 pointer up 한 번(CommandTransaction 하나)입니다. update는 문서를 바꾸지 않습니다.

use stellar_commands::arrange::align::{flow_parent, model_style_set};
use stellar_commands::arrange::place::{self, PlaceError};
use stellar_commands::command::Command;
use stellar_core_types::ids::StableId;
use stellar_core_types::value::Transform2D;
use stellar_document::layout_record::{LayoutLonghand, LayoutValue};
use stellar_document::model::DocumentModel;
use stellar_document::records::NodePayload;

use super::{geometries, selection_roots, union, NodeGeometry, TransformError};
use crate::app::gesture_math::{preview, Gesture, GestureOutput, Modifiers, ResizeHandle};
use crate::canvas::CanvasScene;
use crate::window_session::{history_label, ProjectWindowSession};

/// 상자를 계산하는 좌표계.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeSpace {
    /// 단일 선택: node local(핸들 좌표).
    Local,
    /// 다중 선택: 공통 부모 공간.
    Parent,
}

/// 크기 조절 제스처 상태(시작 때 한 번 만듦).
#[derive(Clone, Debug, PartialEq)]
pub struct ResizeGesture {
    handle: ResizeHandle,
    space: ResizeSpace,
    start: [f32; 4],
    ratio_locked: bool,
    targets: Vec<NodeGeometry>,
}

/// update 결과(미리보기와 확정이 같은 값을 씁니다).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResizeUpdate {
    /// 새 상자(시작 상자와 같은 좌표계, 양수 크기).
    pub rect: [f32; 4],
    /// 축마다 뒤집혔는지.
    pub flip: [bool; 2],
}

/// node에 명시 `aspect-ratio`(auto 아님)가 있는지.
fn ratio_locked(model: &DocumentModel, node: StableId) -> bool {
    model
        .layout_record(node)
        .and_then(|r| r.get(LayoutLonghand::AspectRatio).cloned())
        .is_some_and(|v| !matches!(v, LayoutValue::Keyword("auto")))
}

impl ResizeGesture {
    /// 지금 선택(다른 선택의 자손 제외)으로 시작합니다. 단일 선택은 크기를 저장하는 종류여야 하고, 다중 선택은 한
    /// 부모를 공유해야 합니다.
    pub fn begin(
        window: &ProjectWindowSession,
        scene: &CanvasScene,
        handle: ResizeHandle,
    ) -> Result<Self, TransformError> {
        let (model, _root, roots) = selection_roots(window)?;
        if roots.is_empty() {
            return Err(TransformError::Stale);
        }
        let targets = geometries(scene, &roots)?;
        let (space, start, locked) = if let [only] = targets.as_slice() {
            if !place::has_size(model, only.id) {
                return Err(TransformError::Disabled("no-stored-size"));
            }
            (ResizeSpace::Local, only.rect, ratio_locked(model, only.id))
        } else {
            let parent = crate::app::canvas_gesture::parent_of(model, targets[0].id);
            if targets
                .iter()
                .any(|t| crate::app::canvas_gesture::parent_of(model, t.id) != parent)
            {
                return Err(TransformError::Disabled("mixed-parents"));
            }
            let bounds = union(targets.iter().filter_map(NodeGeometry::parent_bounds))
                .ok_or(TransformError::Stale)?;
            (ResizeSpace::Parent, bounds, false)
        };
        if !(start[2] > 0.0 && start[3] > 0.0) {
            return Err(TransformError::Disabled("empty-box"));
        }
        Ok(ResizeGesture {
            handle,
            space,
            start,
            ratio_locked: locked,
            targets,
        })
    }

    /// 상자 좌표계.
    pub fn space(&self) -> ResizeSpace {
        self.space
    }

    /// 시작 상자.
    pub fn start(&self) -> [f32; 4] {
        self.start
    }

    /// pointer(시작 상자 좌표계) update. 문서를 바꾸지 않는 순수 계산입니다.
    pub fn update(&self, pointer: [f32; 2], modifiers: Modifiers) -> Option<ResizeUpdate> {
        let keep = Modifiers {
            shift: modifiers.shift != self.ratio_locked,
            option: modifiers.option,
        };
        let GestureOutput::Rect(rect) = preview(
            Gesture::Resize {
                handle: self.handle,
                rect: self.start,
            },
            keep,
            pointer,
        )?
        else {
            return None;
        };
        let mut flip = [false; 2];
        for (axis, flipped) in flip.iter_mut().enumerate() {
            let (moves_min, moves_max) = self.handle.moves(axis);
            if !moves_min && !moves_max {
                continue;
            }
            let (lo, size) = (self.start[axis], self.start[axis + 2]);
            let anchor = if modifiers.option {
                lo + size / 2.0
            } else if moves_min {
                lo + size
            } else {
                lo
            };
            let extent = pointer[axis] - anchor;
            *flipped = if moves_min {
                extent > 0.0
            } else {
                extent < 0.0
            };
        }
        Some(ResizeUpdate { rect, flip })
    }

    /// 확정 command(한 transaction). `new_modifier`는 새 위치 Transform modifier ID입니다.
    pub fn commands(
        &self,
        model: &DocumentModel,
        update: &ResizeUpdate,
        new_modifier: &mut dyn FnMut() -> StableId,
    ) -> Result<Vec<Command>, TransformError> {
        let set = model_style_set(model);
        let mut commands = Vec::new();
        match self.space {
            ResizeSpace::Local => {
                let target = &self.targets[0];
                let [x, y, w, h] = update.rect;
                commands.extend(place::set_size(model, target.id, w, h)?);
                if flow_parent(model, &set, target.id).is_flow() {
                    // 흐름 자식: 조절한 축의 크기만 씁니다.
                    return Ok(commands);
                }
                let origin = [target.rect[0], target.rect[1]];
                let sign = update.flip.map(|f| if f { -1.0 } else { 1.0 });
                let tx = x + if update.flip[0] { w } else { 0.0 } - sign[0] * origin[0];
                let ty = y + if update.flip[1] { h } else { 0.0 } - sign[1] * origin[1];
                let inner = Transform2D::new(sign[0], 0.0, 0.0, sign[1], tx, ty)
                    .map_err(|_| PlaceError::NonFinite(target.id))?;
                let next = inner
                    .then(&target.local)
                    .map_err(|_| PlaceError::NonFinite(target.id))?;
                push_change(model, target, next, new_modifier, &mut commands)?;
            }
            ResizeSpace::Parent => {
                let [bx, by, bw, bh] = self.start;
                let [rx, ry, rw, rh] = update.rect;
                let k = [rw / bw, rh / bh];
                let sign = update.flip.map(|f| if f { -1.0f32 } else { 1.0 });
                let map = |p: [f32; 2]| -> [f32; 2] {
                    let along = |axis: usize, value: f32, start: f32, out: f32, size: f32| {
                        let scaled = (value - start) * k[axis];
                        if update.flip[axis] {
                            out + size - scaled
                        } else {
                            out + scaled
                        }
                    };
                    [along(0, p[0], bx, rx, rw), along(1, p[1], by, ry, rh)]
                };
                for target in &self.targets {
                    let (a, b, c, d, _, _) = target.local.components();
                    let [ox, oy, w, h] = target.rect;
                    let sized = place::has_size(model, target.id);
                    let scale_of = |ux: f32, uy: f32| {
                        let len = ux.hypot(uy);
                        if len == 0.0 {
                            1.0
                        } else {
                            (ux * k[0]).hypot(uy * k[1]) / len
                        }
                    };
                    let (nw, nh) = if sized {
                        (w * scale_of(a, b), h * scale_of(c, d))
                    } else {
                        (w, h)
                    };
                    if sized {
                        commands.extend(place::set_size(model, target.id, nw, nh)?);
                    }
                    if flow_parent(model, &set, target.id).is_flow() {
                        continue;
                    }
                    let linear_ok = place::accepts_linear(model, target.id);
                    let (la, lb, lc, ld) = if linear_ok {
                        (sign[0] * a, sign[1] * b, sign[0] * c, sign[1] * d)
                    } else {
                        (a, b, c, d)
                    };
                    let center = target
                        .local
                        .apply(
                            stellar_core_types::value::Vec2::new(ox + w / 2.0, oy + h / 2.0)
                                .map_err(|_| PlaceError::NonFinite(target.id))?,
                        )
                        .map_err(|_| PlaceError::NonFinite(target.id))?;
                    let [cx, cy] = map([center.x(), center.y()]);
                    let (qx, qy) = (ox + nw / 2.0, oy + nh / 2.0);
                    let next = Transform2D::new(
                        la,
                        lb,
                        lc,
                        ld,
                        cx - (la * qx + lc * qy),
                        cy - (lb * qx + ld * qy),
                    )
                    .map_err(|_| PlaceError::NonFinite(target.id))?;
                    push_change(model, target, next, new_modifier, &mut commands)?;
                }
            }
        }
        Ok(commands)
    }

    /// pointer up: 확정해 창 history에 한 항목으로 기록합니다.
    pub fn commit(
        &self,
        window: &mut ProjectWindowSession,
        update: &ResizeUpdate,
    ) -> Result<bool, TransformError> {
        let mut ids = super::FreshIds::default();
        let model = window.document().ok_or(TransformError::Stale)?.model();
        let commands = self.commands(model, update, &mut || ids.fresh())?;
        ids.finish()?;
        super::commit(window, commands, history_label::RESIZE)
    }
}

/// local → 부모 변환을 `next`로 바꾸는 command(`change = next ∘ local⁻¹`).
fn push_change(
    model: &DocumentModel,
    target: &NodeGeometry,
    next: Transform2D,
    new_modifier: &mut dyn FnMut() -> StableId,
    commands: &mut Vec<Command>,
) -> Result<(), TransformError> {
    let inverse = target
        .local
        .inverse()
        .ok_or(PlaceError::NonFinite(target.id))?;
    let change = inverse
        .then(&next)
        .map_err(|_| PlaceError::NonFinite(target.id))?;
    if change == Transform2D::identity() {
        return Ok(());
    }
    let near_identity = {
        let (a, b, c, d, x, y) = change.components();
        [a - 1.0, b, c, d - 1.0, x, y]
            .iter()
            .all(|v| v.abs() < 1e-6)
    };
    if near_identity {
        return Ok(());
    }
    if matches!(
        model.payload(target.id),
        Some(NodePayload::ComponentInstance(_))
    ) {
        // 인스턴스는 평행 이동만 받습니다(선형부 오차를 버림).
        let (_, _, _, _, x, y) = change.components();
        if let Some(command) = place::translate(model, target.id, x, y, new_modifier)? {
            commands.push(command);
        }
        return Ok(());
    }
    if let Some(command) = place::apply_affine(model, target.id, change, new_modifier)? {
        commands.push(command);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::*;
    use super::*;
    use crate::window_session::test_fixtures::sid;
    use stellar_document::shape::ShapeGeometry;

    fn world_point(window: &ProjectWindowSession, node: u64, p: [f32; 2]) -> [f32; 2] {
        let s = scene(window);
        let g = super::super::node_geometry(&s, sid(node)).unwrap();
        let w = g.local.then(&g.parent_world).unwrap();
        let v = w
            .apply(stellar_core_types::value::Vec2::new(p[0], p[1]).unwrap())
            .unwrap();
        [v.x(), v.y()]
    }

    const NONE: Modifiers = Modifiers {
        shift: false,
        option: false,
    };

    #[test]
    fn modifier_matrix_for_a_single_frame() {
        // 11: 100×50 at (10, 20). 오른쪽 아래 핸들.
        let cases = [
            (NONE, [130.0, 70.0], [0.0, 0.0, 130.0, 70.0]),
            (
                Modifiers {
                    shift: true,
                    option: false,
                },
                [150.0, 60.0],
                [0.0, 0.0, 150.0, 75.0],
            ),
            (
                Modifiers {
                    shift: false,
                    option: true,
                },
                [110.0, 50.0],
                [-10.0, 0.0, 120.0, 50.0],
            ),
        ];
        for (modifiers, pointer, expected) in cases {
            let (_dir, mut w) = open("resize-matrix", &canvas_document());
            select(&mut w, &[11]);
            let gesture = ResizeGesture::begin(&w, &scene(&w), ResizeHandle::BottomRight).unwrap();
            assert_eq!(gesture.space(), ResizeSpace::Local);
            let update = gesture.update(pointer, modifiers).unwrap();
            assert_eq!(update.rect, expected, "{modifiers:?}");
            assert_eq!(update.flip, [false, false]);
            let before = history_len(&w);
            assert!(gesture.commit(&mut w, &update).unwrap());
            assert_eq!(history_len(&w), before + 1);
            let Some(NodePayload::Frame(f)) = model(&w).payload(sid(11)) else {
                panic!()
            };
            assert_eq!(
                (f.size.width(), f.size.height()),
                (expected[2], expected[3])
            );
            // 원점 이동은 transform에 반영(Option 중심 기준이면 왼쪽으로 10).
            assert!(close(
                matrix(&w, 11),
                (1., 0., 0., 1., 10. + expected[0], 20. + expected[1])
            ));
        }
    }

    #[test]
    fn crossing_the_opposite_side_flips_on_the_local_axis_and_keeps_the_fixed_edge() {
        let (_dir, mut w) = open("resize-flip", &canvas_document());
        // Rectangle 13 (40×20 at 30, 100). 오른쪽 핸들을 왼쪽 변 너머 -30까지.
        select(&mut w, &[13]);
        let fixed_before = world_point(&w, 13, [0.0, 0.0]);
        let gesture = ResizeGesture::begin(&w, &scene(&w), ResizeHandle::Right).unwrap();
        let update = gesture.update([-30.0, 5.0], NONE).unwrap();
        assert_eq!(update.rect, [-30.0, 0.0, 30.0, 20.0]);
        assert_eq!(update.flip, [true, false]);
        gesture.commit(&mut w, &update).unwrap();
        // 선형부 x 부호만 반전, 크기 30×20, 고정된 왼쪽 변(local x=0)은 뒤집힌 새 local x=0과 같은 world 자리.
        let (a, b, c, d, _, _) = matrix(&w, 13);
        assert_eq!((a, b, c, d), (-1., 0., 0., 1.));
        let fixed_after = world_point(&w, 13, [0.0, 0.0]);
        assert!(
            (fixed_before[0] - fixed_after[0]).abs() < 1e-3
                && (fixed_before[1] - fixed_after[1]).abs() < 1e-3
        );
        assert!(
            matches!(model(&w).payload(sid(13)), Some(NodePayload::Shape(ShapeGeometry::Rectangle(r))) if r.size.width() == 30.0)
        );
        // 같은 축으로 한 번 더 뒤집으면 선형부가 원래대로입니다.
        let gesture = ResizeGesture::begin(&w, &scene(&w), ResizeHandle::Right).unwrap();
        let update = gesture.update([-40.0, 5.0], NONE).unwrap();
        gesture.commit(&mut w, &update).unwrap();
        let (a, _, _, d, _, _) = matrix(&w, 13);
        assert_eq!((a, d), (1., 1.));
        // undo 두 번이면 처음과 같습니다.
        assert!(w.undo().unwrap() && w.undo().unwrap());
        assert!(close(matrix(&w, 13), (1., 0., 0., 1., 30., 100.)));
    }

    #[test]
    fn multi_selection_scales_positions_and_sizes_inside_the_selection_bounds() {
        let (_dir, mut w) = open("resize-multi", &canvas_document());
        // 11 (10..110, 20..70) 과 13 (30..70, 100..120): 경계 (10, 20, 100, 100).
        select(&mut w, &[11, 13]);
        let gesture = ResizeGesture::begin(&w, &scene(&w), ResizeHandle::BottomRight).unwrap();
        assert_eq!(gesture.space(), ResizeSpace::Parent);
        assert_eq!(gesture.start(), [10.0, 20.0, 100.0, 100.0]);
        let update = gesture.update([210.0, 120.0], NONE).unwrap();
        let before = history_len(&w);
        gesture.commit(&mut w, &update).unwrap();
        assert_eq!(history_len(&w), before + 1);
        // x 2배, y 1배.
        assert!(close(matrix(&w, 11), (1., 0., 0., 1., 10., 20.)));
        let Some(NodePayload::Frame(f)) = model(&w).payload(sid(11)) else {
            panic!()
        };
        assert_eq!((f.size.width(), f.size.height()), (200.0, 50.0));
        assert!(close(matrix(&w, 13), (1., 0., 0., 1., 50., 100.)));
        // 공통 부모가 아니면 비활성.
        let mut doc = canvas_document();
        add_shape(
            &mut doc,
            30,
            11,
            ShapeGeometry::Rectangle(stellar_document::shape::RectangleParams {
                size: stellar_core_types::value::Size2::new(10., 10.).unwrap(),
                corner_radius: Default::default(),
            }),
            at(5., 5.),
        );
        let (_dir2, mut w) = open("resize-mixed", &doc);
        select(&mut w, &[30, 13]);
        assert_eq!(
            ResizeGesture::begin(&w, &scene(&w), ResizeHandle::Right),
            Err(TransformError::Disabled("mixed-parents"))
        );
    }

    #[test]
    fn many_updates_then_one_commit_is_one_entry_and_cancel_is_none() {
        let (_dir, mut w) = open("resize-history", &canvas_document());
        select(&mut w, &[12]);
        let before = (history_len(&w), w.revision());
        let gesture = ResizeGesture::begin(&w, &scene(&w), ResizeHandle::Bottom).unwrap();
        let mut last = None;
        for step in 0..500 {
            last = gesture.update([30.0, 40.0 + step as f32 * 0.1], NONE);
        }
        // Escape: 확정하지 않으면 문서·history가 그대로입니다.
        assert_eq!((history_len(&w), w.revision()), before);
        gesture.commit(&mut w, &last.unwrap()).unwrap();
        assert_eq!(history_len(&w), before.0 + 1);
    }

    #[test]
    fn flow_children_write_only_the_resized_axis_size() {
        use stellar_commands::style_defaults::SetLayoutProperty;
        let mut doc = canvas_document();
        super::super::fixtures::add_shape(
            &mut doc,
            30,
            11,
            ShapeGeometry::Rectangle(stellar_document::shape::RectangleParams {
                size: stellar_core_types::value::Size2::new(10., 10.).unwrap(),
                corner_radius: Default::default(),
            }),
            at(5., 5.),
        );
        let (_dir, mut w) = open("resize-flow", &doc);
        let flex = vec![Command::SetLayoutProperty(SetLayoutProperty {
            node: sid(11),
            longhand: LayoutLonghand::Display,
            value: LayoutValue::Keyword("flex"),
        })];
        super::super::commit(&mut w, flex, 0).unwrap();
        select(&mut w, &[30]);
        let gesture = ResizeGesture::begin(&w, &scene(&w), ResizeHandle::Left).unwrap();
        let update = gesture.update([-5.0, 3.0], NONE).unwrap();
        let model_before = matrix(&w, 30);
        let records_before = model(&w).layout_record(sid(30)).cloned();
        gesture.commit(&mut w, &update).unwrap();
        assert_eq!(matrix(&w, 30), model_before);
        assert_eq!(model(&w).layout_record(sid(30)).cloned(), records_before);
        assert!(
            matches!(model(&w).payload(sid(30)), Some(NodePayload::Shape(ShapeGeometry::Rectangle(r))) if (r.size.width(), r.size.height()) == (15.0, 10.0))
        );
    }

    #[test]
    fn explicit_aspect_ratio_keeps_the_ratio_until_shift_releases_it() {
        use stellar_commands::style_defaults::SetLayoutProperty;
        let (_dir, mut w) = open("resize-ratio", &canvas_document());
        let lock = vec![Command::SetLayoutProperty(SetLayoutProperty {
            node: sid(11),
            longhand: LayoutLonghand::AspectRatio,
            value: LayoutLonghand::AspectRatio.parse("2 / 1").unwrap(),
        })];
        super::super::commit(&mut w, lock, 0).unwrap();
        select(&mut w, &[11]);
        let gesture = ResizeGesture::begin(&w, &scene(&w), ResizeHandle::BottomRight).unwrap();
        assert_eq!(
            gesture.update([200.0, 50.0], NONE).unwrap().rect,
            [0.0, 0.0, 200.0, 100.0]
        );
        let shift = Modifiers {
            shift: true,
            option: false,
        };
        assert_eq!(
            gesture.update([200.0, 50.0], shift).unwrap().rect,
            [0.0, 0.0, 200.0, 50.0]
        );
    }
}
