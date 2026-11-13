//! 축별 크기 결정 방식 (X.6.1).
//!
//! [`SizingMode`]는 node 한 축의 border-box 크기를 정하는 방식입니다.
//!
//! | mode | 의미 | containing block 필요 |
//! |---|---|---|
//! | `Fixed(len)` | 고정 크기 | 아니요 |
//! | `Fill` | 부모가 정한 영역(Free/Overlay는 content box, anchor 사이 등)을 채움 | 예 |
//! | `HugContent` | 자식 배치 extent 또는 intrinsic 콘텐츠 크기 + padding | 아니요 |
//! | `Percentage(p)` | containing block content 크기의 `p`% | 예 |
//!
//! # 거부하는 조합
//!
//! - root node의 `Fill`·`Percentage`: root에는 containing block이 없어 크기를 정할 수 없습니다.
//! - 음수 `Fixed`·`Percentage` 값.
//!
//! 부모가 `HugContent`인 축에서 자식의 `Fill`·`Percentage`는 거부하지 않습니다. 이때 자식은
//! 부모 크기 계산(측정)에는 자신의 콘텐츠 크기로 기여하고, 부모 크기가 정해진 뒤 그 크기에
//! 대해 다시 해석됩니다(순환 없음). 다른 속성과의 조합 규칙(anchor 과다 제약, flex basis 충돌,
//! 비율 등)은 해당 속성 작업이 추가합니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{FiniteF32, Length, Percentage};

use crate::error::LayoutError;
use crate::geometry::Axis;
use crate::input::length_unit;
use crate::property::LayoutProperty;

/// 한 축의 크기 결정 방식 (X.6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SizingMode {
    /// 고정 크기(비음수).
    Fixed(Length),
    /// 부모가 정한 영역을 채움.
    Fill,
    /// 콘텐츠(자식 extent 또는 intrinsic 크기) + padding.
    HugContent,
    /// containing block content 크기의 백분율(비음수, `50` = 50%).
    Percentage(Percentage),
}

impl Default for SizingMode {
    fn default() -> SizingMode {
        SizingMode::Fixed(Length::from_raw(FiniteF32::ZERO))
    }
}

impl SizingMode {
    /// containing block 크기에 의존하는 mode(`Fill`, `Percentage`)인지 여부.
    pub const fn depends_on_containing_block(&self) -> bool {
        matches!(self, SizingMode::Fill | SizingMode::Percentage(_))
    }

    /// canonical tag.
    pub(crate) const fn tag(&self) -> u8 {
        match self {
            SizingMode::Fixed(_) => SIZING_FIXED,
            SizingMode::Fill => SIZING_FILL,
            SizingMode::HugContent => SIZING_HUG,
            SizingMode::Percentage(_) => SIZING_PERCENTAGE,
        }
    }
}

/// canonical tag: Fixed.
pub(crate) const SIZING_FIXED: u8 = 0;
/// canonical tag: Fill.
pub(crate) const SIZING_FILL: u8 = 1;
/// canonical tag: HugContent.
pub(crate) const SIZING_HUG: u8 = 2;
/// canonical tag: Percentage.
pub(crate) const SIZING_PERCENTAGE: u8 = 3;

/// 축 속성.
pub(crate) const fn axis_property(axis: Axis) -> LayoutProperty {
    match axis {
        Axis::Horizontal => LayoutProperty::Width,
        Axis::Vertical => LayoutProperty::Height,
    }
}

/// 한 축 sizing 값과 조합을 검증합니다.
pub(crate) fn validate_sizing(
    node: StableId,
    axis: Axis,
    mode: SizingMode,
    is_root: bool,
) -> Result<(), LayoutError> {
    let property = axis_property(axis);
    match mode {
        SizingMode::Fixed(length) => {
            length_unit(node, property, length, false)?;
        }
        SizingMode::Percentage(percent) => {
            if percent.get() < 0.0 {
                return Err(LayoutError::NegativeValue { node, property });
            }
        }
        SizingMode::Fill | SizingMode::HugContent => {}
    }
    if is_root && mode.depends_on_containing_block() {
        return Err(LayoutError::InvalidSizing {
            node,
            axis,
            reason: "root에는 containing block이 없어 Fill·Percentage를 쓸 수 없습니다",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{LayoutInput, LayoutNode};
    use crate::test_support::{id, key, len};

    fn pct(value: f32) -> SizingMode {
        SizingMode::Percentage(Percentage::new(value).unwrap())
    }

    fn all_modes() -> [SizingMode; 4] {
        [
            SizingMode::Fixed(len(10.0)),
            SizingMode::Fill,
            SizingMode::HugContent,
            pct(50.0),
        ]
    }

    /// root(가로·세로 mode) + 자식(가로·세로 mode) 입력.
    fn tree(root: (SizingMode, SizingMode), child: (SizingMode, SizingMode)) -> LayoutInput {
        LayoutInput::new(
            id(1),
            vec![
                LayoutNode::new(id(1))
                    .with_sizing(root.0, root.1)
                    .with_child(id(2), key(1)),
                LayoutNode::new(id(2)).with_sizing(child.0, child.1),
            ],
        )
    }

    #[test]
    fn invalid_mode_combinations_are_rejected() {
        // 모든 (root 가로, root 세로) 조합: Fill·Percentage가 하나라도 있으면 거부.
        for width in all_modes() {
            for height in all_modes() {
                let result = tree((width, height), (SizingMode::Fill, SizingMode::Fill)).validate();
                let expected_axis = if width.depends_on_containing_block() {
                    Some(Axis::Horizontal)
                } else if height.depends_on_containing_block() {
                    Some(Axis::Vertical)
                } else {
                    None
                };
                match expected_axis {
                    Some(axis) => assert!(
                        matches!(
                            result,
                            Err(LayoutError::InvalidSizing { node, axis: a, .. })
                                if node == id(1) && a == axis
                        ),
                        "root ({width:?}, {height:?}) → {result:?}"
                    ),
                    None => assert_eq!(result, Ok(()), "root ({width:?}, {height:?})"),
                }
            }
        }
        // 자식은 모든 조합 허용(부모 Hug + 자식 Fill·Percentage 포함).
        for width in all_modes() {
            for height in all_modes() {
                let root = (SizingMode::HugContent, SizingMode::HugContent);
                assert_eq!(tree(root, (width, height)).validate(), Ok(()));
            }
        }
    }

    #[test]
    fn negative_values_are_rejected() {
        let fixed = (SizingMode::HugContent, SizingMode::Fixed(len(4.0)));
        let negative_pct = tree(fixed, (pct(-1.0), SizingMode::Fill));
        assert_eq!(
            negative_pct.validate(),
            Err(LayoutError::NegativeValue {
                node: id(2),
                property: LayoutProperty::Width
            })
        );
        let negative_fixed = tree(fixed, (SizingMode::Fill, SizingMode::Fixed(len(-0.5))));
        assert_eq!(
            negative_fixed.validate(),
            Err(LayoutError::NegativeValue {
                node: id(2),
                property: LayoutProperty::Height
            })
        );
        // 0%와 100% 초과는 허용.
        assert!(tree(fixed, (pct(0.0), pct(250.0))).validate().is_ok());
    }

    #[test]
    fn sizing_modes_round_trip_with_fixed_tags() {
        for width in all_modes() {
            for height in all_modes() {
                let input = tree(
                    (SizingMode::HugContent, SizingMode::HugContent),
                    (width, height),
                );
                let decoded = LayoutInput::decode(&input.encode()).unwrap();
                assert_eq!(decoded, input);
            }
        }
        // tag golden.
        let tags: Vec<u8> = all_modes().iter().map(SizingMode::tag).collect();
        assert_eq!(tags, vec![0, 1, 2, 3]);
        assert!(SizingMode::Fill.depends_on_containing_block());
        assert!(!SizingMode::HugContent.depends_on_containing_block());
        assert_eq!(SizingMode::default(), SizingMode::Fixed(len(0.0)));
    }
}
