//! Object ▸ Flip Horizontal·Flip Vertical (`object.flip_*`, XII.10.44, ADR-0105 결정 5).
//!
//! # 결정
//!
//! - **축 기준: world 축.** 선택 경계(world) 중심을 지나는 세로선(가로 뒤집기)·가로선(세로 뒤집기)에 대한 거울상입니다.
//!   단일 선택이면 그 node 경계 중심입니다. 회전한 node(또는 migration 노드의 선형 `transform`)도 화면에 보이는 그대로
//!   좌우·상하가 바뀌며, 45도 회전 node를 가로로 뒤집으면 -45도 회전에 local 좌우 반전이 더해진 결과입니다. 크기 핸들로
//!   반대편을 넘어 뒤집는 경우(XII.10.9)는 핸들이 local 좌표이므로 local 축 기준이며, 이 명령과 다릅니다.
//! - **표현: 선형부 반전.** 지금 저장 형식에는 `scale` longhand가 없으므로(IX.11.9 남은 범위) 반전은 node의 위치 변환
//!   (Frame `transform`, 그 밖은 위치 Transform modifier)의 선형부 부호 반전으로 씁니다([`crate::arrange::place`]). CSS로는
//!   `transform: matrix(-1, 0, 0, 1, tx, ty)`와 같습니다. IX.11.9가 `scale` longhand를 열면 Frame은 그 축 `scale` 부호를
//!   바꾸는 쓰기로 바뀌며 이 module의 계산은 그대로입니다. 저장된 `transform-origin`은 없으므로 바꾸지 않습니다.
//! - **대상: 선형 변환을 받는 모든 상자 node.** Frame·Rectangle·Ellipse·Line·Polygon·Path·Text·Image는 같은 규칙입니다
//!   (Path·Polygon geometry를 따로 반전하지 않음). 배치한 ComponentInstance는 배치 Transform이 이동만 허용해 비활성이고,
//!   Fragment는 상자가 없어 비활성입니다([`can_flip`]).
//! - 명령 하나가 CommandTransaction 하나입니다(호출자가 묶음).

use stellar_core_types::ids::StableId;
use stellar_core_types::value::Transform2D;
use stellar_document::model::DocumentModel;

use crate::arrange::place::{about, accepts_linear, apply_affine, PlaceError};
use crate::command::Command;
use crate::lock::{edit_permission, EditIntent};

/// 뒤집는 방향.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlipAxis {
    /// 좌우(세로 축에 대한 거울상, `object.flip_horizontal`).
    Horizontal,
    /// 상하(가로 축에 대한 거울상, `object.flip_vertical`).
    Vertical,
}

impl FlipAxis {
    /// 명령 ID(`docs/design/ui/tools_menus.md`의 이름 규칙, 지금은 계획서 문구).
    pub fn command_id(self) -> &'static str {
        match self {
            FlipAxis::Horizontal => "object.flip_horizontal",
            FlipAxis::Vertical => "object.flip_vertical",
        }
    }
}

/// world 점 `center`를 지나는 축에 대한 거울상 행렬.
pub fn reflection(axis: FlipAxis, center: [f32; 2]) -> Option<Transform2D> {
    let linear = match axis {
        FlipAxis::Horizontal => [-1.0, 0.0, 0.0, 1.0],
        FlipAxis::Vertical => [1.0, 0.0, 0.0, -1.0],
    };
    about(linear, center[0], center[1])
}

/// world 변경 `change`를 부모 공간(`parent_world`: 부모 local → world) 변경으로 바꿉니다: `P⁻¹ ∘ change ∘ P`.
pub fn in_parent_space(change: Transform2D, parent_world: Transform2D) -> Option<Transform2D> {
    let inverse = parent_world.inverse()?;
    parent_world.then(&change).ok()?.then(&inverse).ok()
}

/// node를 뒤집을 수 있는지(선형 변경을 받는 종류이고 이동이 잠기지 않음).
pub fn can_flip(model: &DocumentModel, node: StableId) -> bool {
    accepts_linear(model, node) && edit_permission(model, node, EditIntent::Move).is_ok()
}

/// node마다 부모 공간 변경(`(node, change)`, [`in_parent_space`]로 만든 값)을 적용하는 command. 하나라도 뒤집을 수
/// 없으면 아무 command도 만들지 않고 그 node의 오류를 돌려줍니다.
pub fn flip_commands(
    model: &DocumentModel,
    targets: &[(StableId, Transform2D)],
    new_modifier: &mut dyn FnMut() -> StableId,
) -> Result<Vec<Command>, PlaceError> {
    let mut commands = Vec::with_capacity(targets.len());
    for &(node, change) in targets {
        if !accepts_linear(model, node) {
            return Err(PlaceError::Unsupported(node));
        }
        if let Some(command) = apply_affine(model, node, change, new_modifier)? {
            commands.push(command);
        }
    }
    Ok(commands)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrange::place::local_transform;
    use crate::arrange::test_fixtures::*;

    fn close(a: Transform2D, b: Transform2D) -> bool {
        let (a, b) = (a.components(), b.components());
        [
            a.0 - b.0,
            a.1 - b.1,
            a.2 - b.2,
            a.3 - b.3,
            a.4 - b.4,
            a.5 - b.5,
        ]
        .iter()
        .all(|d| d.abs() < 1e-4)
    }

    #[test]
    fn flipping_twice_restores_the_transform_and_keeps_the_center() {
        let model = shapes();
        // 11: Frame 100×50, translate(10, 20). 중심 (60, 45).
        let flip = reflection(FlipAxis::Horizontal, [60.0, 45.0]).unwrap();
        let mut ids = ids();
        let commands = flip_commands(&model, &[(sid(11), flip)], &mut ids).unwrap();
        let (state, inverse) = apply(copy(&model), commands);
        let flipped = local_transform(state.model(), sid(11)).unwrap();
        // matrix(-1, 0, 0, 1, 110, 20): 중심 (50, 25) → (60, 45) 그대로.
        assert!(close(
            flipped,
            Transform2D::new(-1., 0., 0., 1., 110., 20.).unwrap()
        ));
        let center = flipped
            .apply(stellar_core_types::value::Vec2::new(50.0, 25.0).unwrap())
            .unwrap();
        assert_eq!((center.x(), center.y()), (60.0, 45.0));
        let commands = flip_commands(state.model(), &[(sid(11), flip)], &mut ids).unwrap();
        let (mut twice, _) = apply(copy(state.model()), commands);
        assert!(close(
            local_transform(twice.model(), sid(11)).unwrap(),
            local_transform(&model, sid(11)).unwrap()
        ));
        // undo 한 번으로 원복.
        let mut once = state;
        undo(&mut once, &inverse);
        assert_eq!(once.model().record(sid(11)), model.record(sid(11)));
        let _ = &mut twice;
    }

    #[test]
    fn multi_selection_mirrors_positions_about_the_shared_center_and_kinds_follow_one_rule() {
        let model = shapes();
        let flip = reflection(FlipAxis::Vertical, [0.0, 100.0]).unwrap();
        let mut ids = ids();
        let targets: Vec<_> = [11, 13, 14, 15, 16]
            .iter()
            .map(|&n| (sid(n), flip))
            .collect();
        let commands = flip_commands(&model, &targets, &mut ids).unwrap();
        assert_eq!(commands.len(), 5);
        let (state, _) = apply(model, commands);
        // Rectangle 13: 위치 modifier translate(30, 40) → matrix(1, 0, 0, -1, 30, 160).
        assert!(close(
            local_transform(state.model(), sid(13)).unwrap(),
            Transform2D::new(1., 0., 0., -1., 30., 160.).unwrap()
        ));
        // Ellipse 14: modifier가 없어 새 위치 modifier를 넣습니다.
        assert!(close(
            local_transform(state.model(), sid(14)).unwrap(),
            Transform2D::new(1., 0., 0., -1., 0., 200.).unwrap()
        ));
    }

    #[test]
    fn world_axis_through_a_rotated_parent_is_conjugated_into_parent_space() {
        // 부모가 90° 회전이면 world 가로 뒤집기는 부모 공간의 세로 뒤집기입니다.
        let parent = Transform2D::new(0., 1., -1., 0., 0., 0.).unwrap();
        let world = reflection(FlipAxis::Horizontal, [0.0, 0.0]).unwrap();
        let local = in_parent_space(world, parent).unwrap();
        assert!(close(
            local,
            reflection(FlipAxis::Vertical, [0.0, 0.0]).unwrap()
        ));
    }

    #[test]
    fn fragments_and_locked_nodes_cannot_flip() {
        let mut model = shapes();
        assert!(!can_flip(&model, sid(12)));
        model.replace_flags(sid(13), true, true).unwrap();
        assert!(!can_flip(&model, sid(13)));
        let flip = reflection(FlipAxis::Horizontal, [0.0, 0.0]).unwrap();
        let mut ids = ids();
        assert!(flip_commands(&model, &[(sid(12), flip)], &mut ids).is_err());
        assert_eq!(FlipAxis::Vertical.command_id(), "object.flip_vertical");
    }
}
