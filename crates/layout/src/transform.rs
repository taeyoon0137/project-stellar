//! local transform 적용 정책 (X.2.4).
//!
//! # 초기 정책
//!
//! - node의 local transform([`crate::input::LayoutNode::transform`])은 레이아웃 계산에 **참여하지
//!   않습니다**. 크기 해석, 형제 배치(flow), 부모 콘텐츠 기반 크기, overflow bounds는 모두 변환 전 layout
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

/// `rotate(<angle>)`의 행렬(속성 모델 15장 O13, IX.11.9 결정).
///
/// 직각 배수(0·90·180·270도와 그 ±360k)는 sin·cos를 정확한 0·±1로 둡니다. 그 밖의 각은 f64 `sin_cos`를 한 번 계산해
/// f32로 반올림합니다. 플랫폼 libm 차이는 f64 결과의 마지막 bit에만 나타나 f32 반올림 뒤 같은 값이 될 가능성이
/// 높지만 보장하지 않으므로, 저장 형식은 회전을 분해하지 않고 `matrix()`로 둡니다(ADR-0024 결정 8).
pub fn rotate_matrix(degrees: f32) -> Result<Transform2D, LayoutError> {
    let non_finite = |_| LayoutError::NonFinite { field: "rotate" };
    let turns = degrees.rem_euclid(360.0);
    let (sin, cos) = if turns == 0.0 {
        (0.0, 1.0)
    } else if turns == 90.0 {
        (1.0, 0.0)
    } else if turns == 180.0 {
        (0.0, -1.0)
    } else if turns == 270.0 {
        (-1.0, 0.0)
    } else {
        let (s, c) = f64::from(degrees).to_radians().sin_cos();
        (s as f32, c as f32)
    };
    Transform2D::new(cos, sin, -sin, cos, 0.0, 0.0).map_err(non_finite)
}

/// node의 CSS 위치와 변형을 부모 좌표 변환 하나로 합성합니다(IX.11.9).
///
/// CSS 순서는 `translate(left, top)`(layout 결과 위치) 뒤 `transform-origin` o에 대해
/// `translate(o) ∘ transform ∘ translate(-o)`입니다. `transform`은 node border box 좌상단 원점의 선형 행렬과 이동을
/// 담은 `matrix(a, b, c, d, e, f)`입니다. o가 0이면 결과는 `translate(position) ∘ transform`입니다.
pub fn css_node_transform(
    position: (f32, f32),
    origin: (f32, f32),
    transform: &Transform2D,
) -> Result<Transform2D, LayoutError> {
    let non_finite = |_| LayoutError::NonFinite {
        field: "css transform",
    };
    let translate = |x: f32, y: f32| Transform2D::new(1.0, 0.0, 0.0, 1.0, x, y).map_err(non_finite);
    let around = if origin == (0.0, 0.0) {
        *transform
    } else {
        translate(-origin.0, -origin.1)?
            .then(transform)
            .and_then(|t| t.then(&Transform2D::new(1.0, 0.0, 0.0, 1.0, origin.0, origin.1)?))
            .map_err(non_finite)?
    };
    around
        .then(&translate(position.0, position.1)?)
        .map_err(non_finite)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::compute_layout;
    use crate::input::{LayoutInput, LayoutNode};
    use crate::output::LayoutResult;
    use crate::style::{Display, LayoutStyle, LengthPercentageAuto, Position};
    use crate::test_support::{fixed, id, key, len, style};
    use stellar_core_types::value::Vec2;

    /// 90° 회전 + 2배 확대.
    fn rotate_scale() -> Transform2D {
        Transform2D::new(0.0, 2.0, -2.0, 0.0, 3.0, -4.0).unwrap()
    }

    /// 자식 3개. `flex`면 가로 flex(gap 5) 흐름, 아니면 140×20 block 안 absolute 배치입니다. 가운데 자식의
    /// transform만 바꿉니다.
    fn with_middle_transform(flex: bool, middle: Transform2D) -> LayoutResult {
        let root_style = if flex {
            style(|s| {
                s.display = Display::Flex;
                s.column_gap = len(5.0);
            })
        } else {
            fixed(140.0, 20.0)
        };
        let at = |x: f32, base: LayoutStyle| -> LayoutStyle {
            if flex {
                base
            } else {
                LayoutStyle {
                    position: Position::Absolute,
                    left: LengthPercentageAuto::Length(len(x)),
                    top: LengthPercentageAuto::Length(len(0.0)),
                    ..base
                }
            }
        };
        let root = LayoutNode::new(id(1))
            .with_style(root_style)
            .with_child(id(2), key(1))
            .with_child(id(3), key(2))
            .with_child(id(4), key(3));
        let a = LayoutNode::new(id(2)).with_style(fixed(40.0, 10.0));
        let b = LayoutNode::new(id(3))
            .with_style(at(45.0, fixed(60.0, 20.0)))
            .with_transform(middle);
        let c = LayoutNode::new(id(4)).with_style(at(110.0, fixed(30.0, 15.0)));
        compute_layout(&LayoutInput::new(id(1), vec![root, a, b, c])).unwrap()
    }

    #[test]
    fn rotated_child_does_not_change_sibling_flow() {
        for flex in [true, false] {
            let plain = with_middle_transform(flex, Transform2D::identity());
            let rotated = with_middle_transform(flex, rotate_scale());
            assert_eq!(plain.encode(), rotated.encode(), "{flex}");
            let c = rotated.get(id(4)).unwrap().local_rect();
            assert_eq!(c.x().raw(), 110 * 64, "{flex}");
            let root = rotated.get(id(1)).unwrap().local_rect();
            assert_eq!(root.width().raw(), 140 * 64, "{flex}");
            assert_eq!(root.height().raw(), 20 * 64, "{flex}");
        }
    }

    #[test]
    fn transform_applies_after_layout_rect() {
        let result = with_middle_transform(true, rotate_scale());
        let b = result.get(id(3)).unwrap();
        let placed = placement_transform(b, &rotate_scale()).unwrap();
        let origin = placed.apply(Vec2::ZERO).unwrap();
        assert_eq!((origin.x(), origin.y()), (48.0, -4.0));
        let corner = placed.apply(Vec2::new(60.0, 0.0).unwrap()).unwrap();
        assert_eq!((corner.x(), corner.y()), (48.0, 116.0));
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

    #[test]
    fn rotate_right_angles_are_exact_and_others_round_from_f64() {
        let exact = |deg: f32| rotate_matrix(deg).unwrap().components();
        assert_eq!(exact(0.0), (1.0, 0.0, -0.0, 1.0, 0.0, 0.0));
        assert_eq!(exact(90.0), (0.0, 1.0, -1.0, 0.0, 0.0, 0.0));
        assert_eq!(exact(180.0), (-1.0, 0.0, -0.0, -1.0, 0.0, 0.0));
        assert_eq!(exact(270.0), (0.0, -1.0, 1.0, 0.0, 0.0, 0.0));
        assert_eq!(exact(-90.0), exact(270.0));
        assert_eq!(exact(450.0), exact(90.0));
        let (a, b, ..) = exact(30.0);
        assert_eq!(a, (30f64.to_radians().cos()) as f32);
        assert_eq!(b, (30f64.to_radians().sin()) as f32);
    }

    #[test]
    fn css_transform_with_zero_origin_reproduces_the_original_matrix() {
        // 위치 분리 migration(IX.11.9): 이동 성분은 left·top으로, 선형 부분은 matrix(a, b, c, d, 0, 0)으로.
        let original = Transform2D::new(0.8, 0.6, -0.6, 0.8, 12.5, -4.0).unwrap();
        let (a, b, c, d, tx, ty) = original.components();
        let linear = Transform2D::new(a, b, c, d, 0.0, 0.0).unwrap();
        let composed = css_node_transform((tx, ty), (0.0, 0.0), &linear).unwrap();
        assert_eq!(composed, original);
        // origin이 있으면 그 점이 고정됩니다.
        let rotate = rotate_matrix(90.0).unwrap();
        let around = css_node_transform((0.0, 0.0), (10.0, 10.0), &rotate).unwrap();
        let p = around
            .apply(stellar_core_types::value::Vec2::new(10.0, 10.0).unwrap())
            .unwrap();
        assert_eq!((p.x(), p.y()), (10.0, 10.0));
    }
}
