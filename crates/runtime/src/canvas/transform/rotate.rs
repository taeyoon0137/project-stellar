//! 회전 제스처 (XII.10.10, ADR-0105 결정 2).
//!
//! - pivot: 단일 선택은 node border box 중심, 다중 선택은 선택 world 경계 중심입니다.
//! - Shift는 15도 단위: 단일 선택은 결과 world 각도(node local x축 방향)를 15도 배수로, 다중 선택은 회전량을 15도
//!   배수로 맞춥니다.
//! - 결과는 각 node 위치 변환의 `pivot ∘ rotate ∘ pivot⁻¹` 합성입니다. 위치와 선형부가 함께 바뀌므로 pivot의 world
//!   위치가 그대로 남고, 다중 선택은 각 node의 위치와 회전이 함께 바뀝니다. 0도로 되돌리면 행렬이 원래 값입니다(오차
//!   안). 저장된 `transform-origin`이 없으므로 바꿀 origin도 없습니다(IX.11.9 남은 범위).
//! - **C3 결정: 회전 중심 이동 핸들을 두지 않습니다.** 중심을 옮기는 핸들은 `transform-origin`을 저장해야 의미가 있는데
//!   그 longhand가 아직 저장 형식에 없고(IX.11.9), 제스처마다 pivot을 상자 중심으로 고정하면 같은 회전이 CSS
//!   `rotate`(origin 50% 50%)와 같은 결과가 됩니다. `transform-origin`이 저장되면 그 핸들만 origin을 바꾸고 같은 보정으로
//!   외형을 지키는 행을 따로 엽니다.
//! - 배치한 인스턴스·Fragment는 선형 변경을 받지 못해 비활성입니다.

use stellar_commands::arrange::place::{self, PlaceError};
use stellar_commands::command::Command;
use stellar_commands::flip::in_parent_space;
use stellar_core_types::ids::StableId;
use stellar_document::model::DocumentModel;

use super::{geometries, selection_roots, union, NodeGeometry, TransformError};
use crate::canvas::CanvasScene;
use crate::window_session::{history_label, ProjectWindowSession};

/// Shift 회전 단위(도).
pub const SHIFT_STEP_DEGREES: f32 = 15.0;

/// 회전 제스처 상태.
#[derive(Clone, Debug, PartialEq)]
pub struct RotateGesture {
    pivot: [f32; 2],
    start: f32,
    /// 단일 선택이면 시작 world 각도(도).
    base: Option<f32>,
    targets: Vec<NodeGeometry>,
}

fn degrees_of(dx: f32, dy: f32) -> f32 {
    dy.atan2(dx).to_degrees()
}

impl RotateGesture {
    /// 지금 선택으로 시작합니다. `pointer`는 누른 world 점입니다.
    pub fn begin(
        window: &ProjectWindowSession,
        scene: &CanvasScene,
        pointer: [f32; 2],
    ) -> Result<Self, TransformError> {
        let (model, _root, roots) = selection_roots(window)?;
        if roots.is_empty() {
            return Err(TransformError::Stale);
        }
        if let Some(&node) = roots.iter().find(|&&n| !place::accepts_linear(model, n)) {
            return Err(TransformError::Place(PlaceError::Unsupported(node)));
        }
        let targets = geometries(scene, &roots)?;
        let (pivot, base) = if let [only] = targets.as_slice() {
            let world = only
                .local
                .then(&only.parent_world)
                .map_err(|_| TransformError::Stale)?;
            let (a, b, _, _, _, _) = world.components();
            (
                only.world_center().ok_or(TransformError::Stale)?,
                Some(degrees_of(a, b)),
            )
        } else {
            let b = union(targets.iter().map(|t| t.world_bounds)).ok_or(TransformError::Stale)?;
            ([b[0] + b[2] / 2.0, b[1] + b[3] / 2.0], None)
        };
        Ok(RotateGesture {
            pivot,
            start: degrees_of(pointer[0] - pivot[0], pointer[1] - pivot[1]),
            base,
            targets,
        })
    }

    /// pivot(world).
    pub fn pivot(&self) -> [f32; 2] {
        self.pivot
    }

    /// pointer(world) update: 회전량(도). 문서를 바꾸지 않는 순수 계산입니다.
    pub fn update(&self, pointer: [f32; 2], shift: bool) -> Option<f32> {
        let [px, py] = pointer;
        if !(px.is_finite() && py.is_finite()) {
            return None;
        }
        let raw = degrees_of(px - self.pivot[0], py - self.pivot[1]) - self.start;
        if !shift {
            return Some(raw);
        }
        let snapped = |v: f32| (v / SHIFT_STEP_DEGREES).round() * SHIFT_STEP_DEGREES;
        Some(match self.base {
            Some(base) => snapped(base + raw) - base,
            None => snapped(raw),
        })
    }

    /// 회전량 `degrees`의 확정 command.
    pub fn commands(
        &self,
        model: &DocumentModel,
        degrees: f32,
        new_modifier: &mut dyn FnMut() -> StableId,
    ) -> Result<Vec<Command>, TransformError> {
        let linear = place::rotation(degrees).ok_or(TransformError::Disabled("non-finite"))?;
        let world = place::about(linear, self.pivot[0], self.pivot[1])
            .ok_or(TransformError::Disabled("non-finite"))?;
        let mut commands = Vec::new();
        for target in &self.targets {
            let change = in_parent_space(world, target.parent_world)
                .ok_or(PlaceError::NonFinite(target.id))?;
            if let Some(command) = place::apply_affine(model, target.id, change, new_modifier)? {
                commands.push(command);
            }
        }
        Ok(commands)
    }

    /// pointer up: 확정해 창 history 한 항목으로 기록합니다.
    pub fn commit(
        &self,
        window: &mut ProjectWindowSession,
        degrees: f32,
    ) -> Result<bool, TransformError> {
        let mut ids = super::FreshIds::default();
        let model = window.document().ok_or(TransformError::Stale)?.model();
        let commands = self.commands(model, degrees, &mut || ids.fresh())?;
        ids.finish()?;
        super::commit(window, commands, history_label::RESIZE)
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::*;
    use super::*;
    use crate::window_session::test_fixtures::sid;

    fn center(window: &ProjectWindowSession, node: u64) -> [f32; 2] {
        super::super::node_geometry(&scene(window), sid(node))
            .unwrap()
            .world_center()
            .unwrap()
    }

    #[test]
    fn single_rotation_keeps_the_pivot_and_matches_the_reference_matrix() {
        let (_dir, mut w) = open("rotate-single", &canvas_document());
        // 11: 100×50 at (10, 20), 중심 (60, 45).
        select(&mut w, &[11]);
        let gesture = RotateGesture::begin(&w, &scene(&w), [160.0, 45.0]).unwrap();
        assert_eq!(gesture.pivot(), [60.0, 45.0]);
        // pointer를 중심 아래로: +90도.
        let degrees = gesture.update([60.0, 145.0], false).unwrap();
        assert!((degrees - 90.0).abs() < 1e-4);
        let before = history_len(&w);
        gesture.commit(&mut w, 90.0).unwrap();
        assert_eq!(history_len(&w), before + 1);
        // reference: translate(60,45)·rotate(90)·translate(-60,-45)·translate(10,20) = matrix(0,1,-1,0,85,-5).
        assert!(
            close(matrix(&w, 11), (0., 1., -1., 0., 85., -5.)),
            "{:?}",
            matrix(&w, 11)
        );
        let c = center(&w, 11);
        assert!((c[0] - 60.0).abs() < 1e-3 && (c[1] - 45.0).abs() < 1e-3);
        // 0도로 되돌리면 처음 행렬입니다.
        select(&mut w, &[11]);
        let back = RotateGesture::begin(&w, &scene(&w), [160.0, 45.0]).unwrap();
        back.commit(&mut w, -90.0).unwrap();
        assert!(close(matrix(&w, 11), (1., 0., 0., 1., 10., 20.)));
    }

    #[test]
    fn shift_snaps_to_fifteen_degrees() {
        let (_dir, mut w) = open("rotate-shift", &canvas_document());
        select(&mut w, &[13]);
        let gesture = RotateGesture::begin(&w, &scene(&w), [100.0, 110.0]).unwrap();
        let pivot = gesture.pivot();
        let pointer = |deg: f32| {
            let r = deg.to_radians();
            [pivot[0] + 50.0 * r.cos(), pivot[1] + 50.0 * r.sin()]
        };
        assert!((gesture.update(pointer(25.0), true).unwrap() - 30.0).abs() < 1e-3);
        assert!((gesture.update(pointer(7.0), true).unwrap()).abs() < 1e-3);
        assert!((gesture.update(pointer(22.0), false).unwrap() - 22.0).abs() < 1e-3);
    }

    #[test]
    fn multi_rotation_moves_each_node_around_the_selection_center() {
        let (_dir, mut w) = open("rotate-multi", &canvas_document());
        // 13 (30..70, 100..120) 과 14 (100..140, 100..120): 경계 중심 (85, 110).
        select(&mut w, &[13, 14]);
        let gesture = RotateGesture::begin(&w, &scene(&w), [200.0, 110.0]).unwrap();
        assert_eq!(gesture.pivot(), [85.0, 110.0]);
        gesture.commit(&mut w, 180.0).unwrap();
        // 180도: 13의 중심 (50, 110) → (120, 110), 14의 중심 (120, 110) → (50, 110).
        let c13 = center(&w, 13);
        let c14 = center(&w, 14);
        assert!((c13[0] - 120.0).abs() < 1e-3 && (c14[0] - 50.0).abs() < 1e-3);
        assert!(close(matrix(&w, 13), (-1., 0., 0., -1., 140., 120.)));
        // 형제(11, 12)는 그대로입니다.
        assert!(close(matrix(&w, 11), (1., 0., 0., 1., 10., 20.)));
    }
}
