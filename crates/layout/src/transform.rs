//! local transform 적용 정책 (X.2.4).
//!
//! # 초기 정책
//!
//! - node의 local transform([`crate::input::LayoutNode::transform`])은 레이아웃 계산에 **참여하지
//!   않습니다**. 크기 해석, 형제 배치(flow), 부모 Hug 크기, overflow bounds는 모두 변환 전 layout
//!   rect로 계산합니다. 따라서 회전·확대된 자식도 형제 flow와 부모 크기를 바꾸지 않습니다.
//! - transform은 layout rect가 확정된 **뒤** 적용합니다. 자식 local 좌표의 점 `p`는 먼저 local
//!   transform `T`(자식 원점 = border box 좌상단 기준)를 거친 뒤 layout rect origin만큼 이동합니다:
//!   `parent = translate(rect.origin) ∘ T`. [`placement_transform`]이 이 합성 결과를 줍니다.
//! - 변환된 시각 bounds(hit test·culling용)는 scene 단계 책임이며 layout overflow bounds에 넣지
//!   않습니다.

use stellar_core_types::value::Transform2D;

use crate::error::LayoutError;
use crate::output::LayoutBox;

/// 자식 local 좌표 → 부모 local 좌표 변환(`translate(rect.origin) ∘ local`)을 만듭니다.
pub fn placement_transform(
    layout_box: &LayoutBox,
    local: &Transform2D,
) -> Result<Transform2D, LayoutError> {
    let rect = layout_box.local_rect();
    let translate = Transform2D::new(
        1.0,
        0.0,
        0.0,
        1.0,
        rect.x().to_px_f32().get(),
        rect.y().to_px_f32().get(),
    )
    .map_err(|_| LayoutError::NonFinite {
        field: "placement translate",
    })?;
    local.then(&translate).map_err(|_| LayoutError::NonFinite {
        field: "placement transform",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::compute_layout;
    use crate::geometry::Axis;
    use crate::input::{ContainerLayout, LayoutInput, LayoutNode, SizingMode};
    use crate::output::LayoutResult;
    use crate::stack::StackLayout;
    use crate::test_support::{id, key, len};
    use stellar_core_types::value::Vec2;

    /// 90° 회전 + 2배 확대.
    fn rotate_scale() -> Transform2D {
        Transform2D::new(0.0, 2.0, -2.0, 0.0, 3.0, -4.0).unwrap()
    }

    /// Hug 가로 Stack(gap 5) 안의 자식 3개. 가운데 자식의 transform만 바꿉니다.
    fn stack_with_middle_transform(
        container: ContainerLayout,
        middle: Transform2D,
    ) -> LayoutResult {
        let root = LayoutNode::new(id(1))
            .with_sizing(SizingMode::HugContent, SizingMode::HugContent)
            .with_container(container)
            .with_child(id(2), key(1))
            .with_child(id(3), key(2))
            .with_child(id(4), key(3));
        let a = LayoutNode::new(id(2)).with_fixed_size(len(40.0), len(10.0));
        let b = LayoutNode::new(id(3))
            .with_fixed_size(len(60.0), len(20.0))
            .with_position(len(45.0), len(0.0))
            .with_transform(middle);
        let c = LayoutNode::new(id(4))
            .with_fixed_size(len(30.0), len(15.0))
            .with_position(len(110.0), len(0.0));
        compute_layout(&LayoutInput::new(id(1), vec![root, a, b, c])).unwrap()
    }

    #[test]
    fn rotated_child_does_not_change_sibling_flow() {
        let stack = ContainerLayout::Stack(StackLayout {
            gap: len(5.0),
            ..StackLayout::new(Axis::Horizontal)
        });
        for container in [stack, ContainerLayout::Free] {
            let plain = stack_with_middle_transform(container.clone(), Transform2D::identity());
            let rotated = stack_with_middle_transform(container.clone(), rotate_scale());
            // 모든 box(형제 위치, 부모 Hug 크기, overflow)가 bit 단위로 같습니다.
            assert_eq!(plain.encode(), rotated.encode(), "{container:?}");
            let c = rotated.get(id(4)).unwrap().local_rect();
            assert_eq!(c.x().raw(), 110 * 64, "{container:?}");
            let root = rotated.get(id(1)).unwrap().local_rect();
            assert_eq!(root.width().raw(), 140 * 64, "{container:?}");
            assert_eq!(root.height().raw(), 20 * 64, "{container:?}");
        }
    }

    #[test]
    fn transform_applies_after_layout_rect() {
        let stack = ContainerLayout::Stack(StackLayout {
            gap: len(5.0),
            ..StackLayout::new(Axis::Horizontal)
        });
        let result = stack_with_middle_transform(stack, rotate_scale());
        let b = result.get(id(3)).unwrap();
        let placed = placement_transform(b, &rotate_scale()).unwrap();
        // local 원점 → T(0,0) = (3, −4) → + rect origin(45, 0).
        let origin = placed.apply(Vec2::ZERO).unwrap();
        assert_eq!((origin.x(), origin.y()), (48.0, -4.0));
        // local (60, 0) → T = (3, 116) → (48, 116).
        let corner = placed.apply(Vec2::new(60.0, 0.0).unwrap()).unwrap();
        assert_eq!((corner.x(), corner.y()), (48.0, 116.0));
        // identity면 rect origin 이동만.
        let plain = placement_transform(b, &Transform2D::identity()).unwrap();
        assert_eq!(plain.components(), (1.0, 0.0, 0.0, 1.0, 45.0, 0.0));
    }

    #[test]
    fn transform_round_trips_in_input_bytes() {
        let input = LayoutInput::new(
            id(1),
            vec![LayoutNode::new(id(1)).with_transform(rotate_scale())],
        );
        let decoded = LayoutInput::decode(&input.encode()).unwrap();
        assert_eq!(decoded, input);
        assert_eq!(decoded.nodes[0].transform, rotate_scale());
        assert_eq!(LayoutNode::new(id(2)).transform, Transform2D::identity());
    }
}
