//! 축별 크기 결정 방식, min/max clamp, aspect ratio (X.6.1~X.6.3).
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
//!
//! # min/max clamp (X.6.2)
//!
//! [`SizeLimits`]의 축별 min/max는 **모든 배치 방식에서 같은 순서**로 최종 크기에 적용합니다:
//!
//! 1. sizing mode(또는 부모가 정한 forced 크기: Stack 분배·Stretch, Grid 영역, Constraint 해 등)로
//!    값을 정합니다.
//! 2. 그 값에 `max`를 먼저, `min`을 나중에 적용합니다: `max(min, min(값, max))`.
//! 3. `HugContent` 축은 콘텐츠 extent + padding을 구한 뒤 같은 clamp를 적용합니다(definite 축은
//!    clamp한 값으로 content box를 정해 자식을 측정).
//!
//! clamp는 엔진의 단일 크기 해석 함수에서만 하므로 Free·Stack·Grid·Overlay·Constraint가 같은 규칙을
//! 씁니다. Stack grow·shrink 분배는 같은 min/max를 freeze 반복에 넣어 분배 결과가 이미 clamp된
//! 상태가 되게 합니다(X.3.3). `min > max`는 [`LayoutError::InvalidSizing`], 음수는
//! [`LayoutError::NegativeValue`]입니다.
//!
//! # aspect ratio (X.6.3)
//!
//! [`AspectRatio`] `w : h`는 두 축 중 **한 축만 definite**일 때 적용합니다: definite 축 값(clamp 후)에서
//! 다른 축을 `× h / w` 또는 `× w / h`로 정하고(한 번 반올림) 그 축도 clamp합니다. 두 축이 모두
//! definite면 적용하지 않습니다(각 축 크기 그대로). 두 축이 모두 indefinite(`HugContent` 등)면
//! 가로를 intrinsic(콘텐츠) 크기로 정해 definite로 만든 뒤 비율로 세로를 정합니다. 순서는 sizing mode →
//! definite 축 clamp → 비율로 다른 축 → 그 축 clamp입니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{FiniteF32, Length, Percentage, Scalar};

use crate::error::LayoutError;
use crate::geometry::Axis;
use crate::input::length_unit;
use crate::property::LayoutProperty;
use crate::rounding::{LayoutUnit, LayoutUnitError};

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

// --- min/max clamp (X.6.2) ---

/// 축별 최소·최대 크기 (X.6.2). 값은 비음수이고 `min ≤ max`여야 합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SizeLimits {
    /// 최소 너비.
    pub min_width: Option<Length>,
    /// 최대 너비.
    pub max_width: Option<Length>,
    /// 최소 높이.
    pub min_height: Option<Length>,
    /// 최대 높이.
    pub max_height: Option<Length>,
}

impl SizeLimits {
    /// 축의 (min, max).
    pub const fn axis(&self, axis: Axis) -> (Option<Length>, Option<Length>) {
        match axis {
            Axis::Horizontal => (self.min_width, self.max_width),
            Axis::Vertical => (self.min_height, self.max_height),
        }
    }
}

/// 변환된 한 축 min/max.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct AxisLimits {
    pub(crate) min: LayoutUnit,
    pub(crate) max: Option<LayoutUnit>,
}

impl AxisLimits {
    /// `max(min, min(value, max))` — 최대를 먼저, 최소를 나중에 적용합니다(검증으로 min ≤ max).
    pub(crate) fn clamp(&self, value: LayoutUnit) -> LayoutUnit {
        let capped = match self.max {
            Some(max) => value.min(max),
            None => value,
        };
        capped.max(self.min)
    }
}

/// min/max를 검증·변환합니다.
pub(crate) fn prepare_limits(
    node: StableId,
    limits: &SizeLimits,
) -> Result<[AxisLimits; 2], LayoutError> {
    let mut out = [AxisLimits::default(); 2];
    for axis in Axis::BOTH {
        let property = axis_property(axis);
        let (min, max) = limits.axis(axis);
        let min = match min {
            Some(length) => length_unit(node, property, length, false)?,
            None => LayoutUnit::ZERO,
        };
        let max = match max {
            Some(length) => Some(length_unit(node, property, length, false)?),
            None => None,
        };
        if max.is_some_and(|max| min > max) {
            return Err(LayoutError::InvalidSizing {
                node,
                axis,
                reason: "min이 max보다 큽니다",
            });
        }
        out[axis.index()] = AxisLimits { min, max };
    }
    Ok(out)
}

/// min/max canonical bytes(min_width, max_width, min_height, max_height 순, flag + f32).
pub(crate) fn encode_limits(w: &mut crate::codec::Writer, limits: &SizeLimits) {
    for value in [
        limits.min_width,
        limits.max_width,
        limits.min_height,
        limits.max_height,
    ] {
        match value {
            Some(length) => {
                w.bool(true);
                w.length(length);
            }
            None => w.bool(false),
        }
    }
}

/// min/max canonical bytes를 읽습니다.
pub(crate) fn decode_limits(r: &mut crate::codec::Reader<'_>) -> Result<SizeLimits, LayoutError> {
    let mut values = [None; 4];
    for value in &mut values {
        if r.bool("size limit flag")? {
            *value = Some(r.length("size limit")?);
        }
    }
    let [min_width, max_width, min_height, max_height] = values;
    Ok(SizeLimits {
        min_width,
        max_width,
        min_height,
        max_height,
    })
}

// --- aspect ratio (X.6.3) ---

/// 너비 : 높이 비율 (X.6.3). 두 값 모두 양수입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AspectRatio {
    /// 너비 쪽 값.
    pub width: Scalar,
    /// 높이 쪽 값.
    pub height: Scalar,
}

impl AspectRatio {
    /// `width : height` 비율.
    pub const fn new(width: Scalar, height: Scalar) -> AspectRatio {
        AspectRatio { width, height }
    }
}

/// 변환된 비율(1/64 정수 `width : height`, 둘 다 양수).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PreparedRatio {
    pub(crate) width: i64,
    pub(crate) height: i64,
}

impl PreparedRatio {
    /// 너비로부터 높이(`width × h / w`, 한 번 반올림).
    pub(crate) fn height_for(&self, width: LayoutUnit) -> Result<LayoutUnit, LayoutUnitError> {
        width.checked_mul_ratio(self.height, self.width)
    }

    /// 높이로부터 너비(`height × w / h`, 한 번 반올림).
    pub(crate) fn width_for(&self, height: LayoutUnit) -> Result<LayoutUnit, LayoutUnitError> {
        height.checked_mul_ratio(self.width, self.height)
    }
}

/// 1/64 격자로 양자화하는 비음수 가중치(flex grow·shrink, grid fr, aspect ratio 성분)를 변환합니다.
///
/// 모든 가중치 입력이 같은 규칙을 씁니다.
///
/// - 음수: [`LayoutError::NegativeValue`].
/// - `LayoutUnit` 범위 초과: [`LayoutError::OutOfRange`].
/// - 0이 아닌데 1/64 격자에서 0으로 반올림되는 값(1/128 미만): [`LayoutError::BelowResolution`].
///   예를 들어 grow `0.001`을 조용히 grow 0(늘지 않음)으로 바꾸지 않습니다.
/// - 0은 0입니다.
///
/// 길이(크기·padding·gap 등)는 가중치가 아니므로 이 규칙을 쓰지 않고 1/64로 반올림합니다(0.001 px →
/// 0 px는 같은 의미).
pub(crate) fn quantize_weight(
    node: StableId,
    property: LayoutProperty,
    value: Scalar,
) -> Result<u64, LayoutError> {
    if value.get() < 0.0 {
        return Err(LayoutError::NegativeValue { node, property });
    }
    let unit =
        LayoutUnit::from_px(value.raw()).map_err(|_| LayoutError::OutOfRange { node, property })?;
    if unit.raw() == 0 && value.get() != 0.0 {
        return Err(LayoutError::BelowResolution { node, property });
    }
    u64::try_from(unit.raw()).map_err(|_| LayoutError::NegativeValue { node, property })
}

/// 비율을 검증·변환합니다. 각 값은 양수여야 하며(0·음수는 [`LayoutError::InvalidSizing`]),
/// 양자화는 [`quantize_weight`] 규칙을 따릅니다.
pub(crate) fn prepare_ratio(
    node: StableId,
    ratio: Option<AspectRatio>,
) -> Result<Option<PreparedRatio>, LayoutError> {
    let Some(ratio) = ratio else {
        return Ok(None);
    };
    let convert = |value: Scalar| -> Result<i64, LayoutError> {
        if value.get() <= 0.0 {
            return Err(LayoutError::InvalidSizing {
                node,
                axis: Axis::Horizontal,
                reason: "aspect ratio 값은 양수여야 합니다",
            });
        }
        let units = quantize_weight(node, LayoutProperty::AspectRatio, value)?;
        i64::try_from(units).map_err(|_| LayoutError::OutOfRange {
            node,
            property: LayoutProperty::AspectRatio,
        })
    };
    Ok(Some(PreparedRatio {
        width: convert(ratio.width)?,
        height: convert(ratio.height)?,
    }))
}

/// 비율 canonical bytes(flag + width f32 + height f32).
pub(crate) fn encode_ratio(w: &mut crate::codec::Writer, ratio: Option<AspectRatio>) {
    match ratio {
        Some(ratio) => {
            w.bool(true);
            w.finite(ratio.width.raw());
            w.finite(ratio.height.raw());
        }
        None => w.bool(false),
    }
}

/// 비율 canonical bytes를 읽습니다.
pub(crate) fn decode_ratio(
    r: &mut crate::codec::Reader<'_>,
) -> Result<Option<AspectRatio>, LayoutError> {
    if !r.bool("aspect ratio flag")? {
        return Ok(None);
    }
    let width = Scalar::from_raw(r.finite("aspect ratio")?);
    let height = Scalar::from_raw(r.finite("aspect ratio")?);
    Ok(Some(AspectRatio { width, height }))
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

    use crate::constraint::{AnchorEdge, AnchorTarget, Constraint};
    use crate::engine::compute_layout;
    use crate::grid::{GridItem, GridLayout, TrackSize};
    use crate::input::ContainerLayout;
    use crate::output::LayoutResult;
    use crate::overlay::OverlayLayout;
    use crate::stack::{CrossAlignment, StackLayout};
    use crate::test_support::px;

    fn limits(
        min_w: Option<f32>,
        max_w: Option<f32>,
        min_h: Option<f32>,
        max_h: Option<f32>,
    ) -> SizeLimits {
        SizeLimits {
            min_width: min_w.map(len),
            max_width: max_w.map(len),
            min_height: min_h.map(len),
            max_height: max_h.map(len),
        }
    }

    /// 100×60 root(container)와 자식들.
    fn run(container: ContainerLayout, children: Vec<LayoutNode>) -> LayoutResult {
        let mut root = LayoutNode::new(id(1))
            .with_fixed_size(len(100.0), len(60.0))
            .with_container(container);
        for (i, child) in children.iter().enumerate() {
            root = root.with_child(child.id, key(i as u32 + 1));
        }
        let mut nodes = vec![root];
        nodes.extend(children);
        compute_layout(&LayoutInput::new(id(1), nodes)).unwrap()
    }

    fn size_px(result: &LayoutResult, n: u64) -> (f64, f64) {
        let r = result.get(id(n)).unwrap().local_rect();
        (r.width().to_px_f64(), r.height().to_px_f64())
    }

    #[test]
    fn min_greater_than_max_and_negative_limits_are_rejected() {
        let node =
            |l: SizeLimits| LayoutInput::new(id(1), vec![LayoutNode::new(id(1)).with_limits(l)]);
        assert!(matches!(
            node(limits(Some(50.0), Some(40.0), None, None)).validate(),
            Err(LayoutError::InvalidSizing {
                axis: Axis::Horizontal,
                ..
            })
        ));
        assert!(matches!(
            node(limits(None, None, Some(10.0), Some(9.5))).validate(),
            Err(LayoutError::InvalidSizing {
                axis: Axis::Vertical,
                ..
            })
        ));
        assert!(matches!(
            node(limits(Some(-1.0), None, None, None)).validate(),
            Err(LayoutError::NegativeValue { .. })
        ));
        // 경계: min == max 허용, round-trip.
        let equal = node(limits(Some(40.0), Some(40.0), Some(0.0), None));
        assert_eq!(equal.validate(), Ok(()));
        assert_eq!(LayoutInput::decode(&equal.encode()).unwrap(), equal);
    }

    #[test]
    fn boundary_fixture_in_free_and_root() {
        // root Fixed 100 + max 80 → 80. Free 자식: Fixed 50/max 40 → 40, Fill(영역 20)/min 30 → 30,
        // Hug(intrinsic 70)/max 50 → 50, 값이 경계와 같으면 그대로(min = max = 25).
        let root = LayoutNode::new(id(1))
            .with_fixed_size(len(100.0), len(60.0))
            .with_limits(limits(None, Some(80.0), None, None))
            .with_child(id(2), key(1))
            .with_child(id(3), key(2))
            .with_child(id(4), key(3))
            .with_child(id(5), key(4));
        let fixed = LayoutNode::new(id(2))
            .with_fixed_size(len(50.0), len(10.0))
            .with_limits(limits(None, Some(40.0), None, None));
        let fill = LayoutNode::new(id(3))
            .with_sizing(SizingMode::Fill, SizingMode::Fixed(len(10.0)))
            .with_position(len(60.0), len(0.0))
            .with_limits(limits(Some(30.0), None, None, None));
        let hug = LayoutNode::new(id(4))
            .with_sizing(SizingMode::HugContent, SizingMode::HugContent)
            .with_content(crate::input::LayoutContent::Intrinsic(
                crate::input::IntrinsicContent {
                    width: len(70.0),
                    height: len(5.0),
                    baseline: None,
                },
            ))
            .with_limits(limits(None, Some(50.0), Some(5.0), Some(5.0)));
        let exact = LayoutNode::new(id(5))
            .with_fixed_size(len(25.0), len(25.0))
            .with_limits(limits(Some(25.0), Some(25.0), Some(25.0), Some(25.0)));
        let result = compute_layout(&LayoutInput::new(
            id(1),
            vec![root, fixed, fill, hug, exact],
        ))
        .unwrap();
        assert_eq!(size_px(&result, 1), (80.0, 60.0));
        assert_eq!(size_px(&result, 2), (40.0, 10.0));
        // root 너비 80 → Fill 영역 80 − 60 = 20 → min 30.
        assert_eq!(size_px(&result, 3), (30.0, 10.0));
        assert_eq!(size_px(&result, 4), (50.0, 5.0));
        assert_eq!(size_px(&result, 5), (25.0, 25.0));
    }

    #[test]
    fn boundary_fixture_in_stack_grid_overlay_and_constraint() {
        let fill = || SizingMode::Fill;
        // Stack grow: Fill 3개 in 100, 첫 자식 max 20 → 20/40/40.
        let stack = ContainerLayout::Stack(StackLayout::new(Axis::Horizontal));
        let child = |n: u64, l: SizeLimits| {
            LayoutNode::new(id(n))
                .with_sizing(fill(), SizingMode::Fixed(len(10.0)))
                .with_limits(l)
        };
        let result = run(
            stack.clone(),
            vec![
                child(2, limits(None, Some(20.0), None, None)),
                child(3, SizeLimits::default()),
                child(4, SizeLimits::default()),
            ],
        );
        assert_eq!(
            [
                size_px(&result, 2).0,
                size_px(&result, 3).0,
                size_px(&result, 4).0
            ],
            [20.0, 40.0, 40.0]
        );
        // Stack shrink: Fixed 100 두 개 shrink 1 in 100 → 50/50, 첫 자식 min 70 → 70/30.
        let shrinker = |n: u64, l: SizeLimits| {
            LayoutNode::new(id(n))
                .with_fixed_size(len(100.0), len(10.0))
                .with_flex(crate::flex::FlexItem::new(
                    stellar_core_types::value::Scalar::new(0.0).unwrap(),
                    stellar_core_types::value::Scalar::new(1.0).unwrap(),
                    crate::flex::FlexBasis::Auto,
                ))
                .with_limits(l)
        };
        let result = run(
            stack,
            vec![
                shrinker(2, limits(Some(70.0), None, None, None)),
                shrinker(3, SizeLimits::default()),
            ],
        );
        assert_eq!([size_px(&result, 2).0, size_px(&result, 3).0], [70.0, 30.0]);
        // Stack Stretch cross(60) + max_height 30 → 30.
        let stretch = ContainerLayout::Stack(StackLayout {
            cross_alignment: CrossAlignment::Stretch,
            ..StackLayout::new(Axis::Horizontal)
        });
        let hug = LayoutNode::new(id(2))
            .with_sizing(SizingMode::Fixed(len(10.0)), SizingMode::HugContent)
            .with_limits(limits(None, None, None, Some(30.0)));
        assert_eq!(size_px(&run(stretch, vec![hug]), 2), (10.0, 30.0));

        // Grid: 영역 60×60(1열 1행) Fill + max 45 / min 70.
        let grid = ContainerLayout::Grid(GridLayout::new(
            vec![TrackSize::Fixed(len(60.0))],
            vec![TrackSize::Fixed(len(60.0))],
        ));
        let item = LayoutNode::new(id(2))
            .with_grid(GridItem::at(1, 1, 1, 1))
            .with_sizing(fill(), fill())
            .with_limits(limits(None, Some(45.0), Some(70.0), None));
        assert_eq!(size_px(&run(grid, vec![item]), 2), (45.0, 70.0));

        // Overlay: Fill(100×60) + max_width 80, min_height 70 → 80×70.
        let overlay = LayoutNode::new(id(2))
            .with_sizing(fill(), fill())
            .with_limits(limits(None, Some(80.0), Some(70.0), None));
        assert_eq!(
            size_px(
                &run(
                    ContainerLayout::Overlay(OverlayLayout::default()),
                    vec![overlay]
                ),
                2
            ),
            (80.0, 70.0)
        );

        // Constraint: Left = 0, Right = parent.Right → Fill 100 → max 70.
        let constrained = LayoutNode::new(id(2))
            .with_sizing(fill(), SizingMode::Fixed(len(10.0)))
            .with_constraint(Constraint::new(
                AnchorEdge::Left,
                AnchorTarget::Parent(AnchorEdge::Left),
                len(0.0),
            ))
            .with_constraint(Constraint::new(
                AnchorEdge::Right,
                AnchorTarget::Parent(AnchorEdge::Right),
                len(0.0),
            ))
            .with_constraint(Constraint::new(
                AnchorEdge::Top,
                AnchorTarget::Parent(AnchorEdge::Top),
                len(0.0),
            ))
            .with_limits(limits(None, Some(70.0), None, None));
        let result = run(ContainerLayout::Constraint, vec![constrained]);
        assert_eq!(size_px(&result, 2), (70.0, 10.0));
        assert_eq!(result.get(id(2)).unwrap().local_rect().x(), px(0));
    }

    fn ratio(w: f32, h: f32) -> AspectRatio {
        AspectRatio::new(
            stellar_core_types::value::Scalar::new(w).unwrap(),
            stellar_core_types::value::Scalar::new(h).unwrap(),
        )
    }

    /// 200×80 Free root 안의 비율 자식 하나를 계산해 (너비, 높이) px를 돌려줍니다.
    fn ratio_child(
        width: SizingMode,
        height: SizingMode,
        aspect: AspectRatio,
        limits: SizeLimits,
        content: Option<(f32, f32)>,
    ) -> (f64, f64) {
        let mut child = LayoutNode::new(id(2))
            .with_sizing(width, height)
            .with_aspect_ratio(aspect)
            .with_limits(limits);
        if let Some((w, h)) = content {
            child = child.with_content(crate::input::LayoutContent::Intrinsic(
                crate::input::IntrinsicContent {
                    width: len(w),
                    height: len(h),
                    baseline: None,
                },
            ));
        }
        let result = run_free(vec![child]);
        size_px(&result, 2)
    }

    fn run_free(children: Vec<LayoutNode>) -> LayoutResult {
        let mut root = LayoutNode::new(id(1)).with_fixed_size(len(200.0), len(80.0));
        for (i, child) in children.iter().enumerate() {
            root = root.with_child(child.id, key(i as u32 + 1));
        }
        let mut nodes = vec![root];
        nodes.extend(children);
        compute_layout(&LayoutInput::new(id(1), nodes)).unwrap()
    }

    /// (가로 sizing, 세로 sizing, intrinsic 콘텐츠, 기대 (너비, 높이)).
    type RatioCase = (SizingMode, SizingMode, Option<(f32, f32)>, (f64, f64));

    #[test]
    fn ratio_matrix_fixture() {
        let two_one = ratio(2.0, 1.0);
        let none = SizeLimits::default();
        let fixed = |v: f32| SizingMode::Fixed(len(v));
        let hug = SizingMode::HugContent;
        // (가로, 세로, 콘텐츠, 기대 크기).
        let cases: [RatioCase; 8] = [
            // 가로만 definite → 세로 = 100 / 2.
            (fixed(100.0), hug, None, (100.0, 50.0)),
            // 세로만 definite → 가로 = 30 × 2.
            (hug, fixed(30.0), None, (60.0, 30.0)),
            // Fill(200) definite → 세로 100.
            (SizingMode::Fill, hug, None, (200.0, 100.0)),
            // Percentage 25% of 200 = 50 → 세로 25.
            (pct(25.0), hug, None, (50.0, 25.0)),
            // 세로 Fill(80)만 definite → 가로 160.
            (hug, SizingMode::Fill, None, (160.0, 80.0)),
            // 두 축 definite → 비율 미적용.
            (fixed(100.0), fixed(20.0), None, (100.0, 20.0)),
            // 두 축 indefinite → 가로 intrinsic 30, 세로 = 15(콘텐츠 높이 40 무시).
            (hug, hug, Some((30.0, 40.0)), (30.0, 15.0)),
            // 두 축 indefinite, 콘텐츠 없음 → 0 × 0.
            (hug, hug, None, (0.0, 0.0)),
        ];
        for (width, height, content, expected) in cases {
            assert_eq!(
                ratio_child(width, height, two_one, none, content),
                expected,
                "{width:?} × {height:?}"
            );
        }
        // 순서: definite 축 clamp 뒤 비율, 그 다음 다른 축 clamp.
        assert_eq!(
            ratio_child(
                fixed(100.0),
                hug,
                two_one,
                limits(None, Some(60.0), None, None),
                None
            ),
            (60.0, 30.0)
        );
        assert_eq!(
            ratio_child(
                fixed(100.0),
                hug,
                two_one,
                limits(None, None, None, Some(20.0)),
                None
            ),
            (100.0, 20.0)
        );
        // 3 : 1, 너비 100 → 높이 33.33 → 2133 subpixel(한 번 반올림).
        let child = LayoutNode::new(id(2))
            .with_sizing(fixed(100.0), hug)
            .with_aspect_ratio(ratio(3.0, 1.0));
        let result = run_free(vec![child]);
        assert_eq!(result.get(id(2)).unwrap().local_rect().height().raw(), 2133);
    }

    #[test]
    fn ratio_follows_stack_main_size_and_validates() {
        // 가로 Stack에서 grow로 정해진 main 너비(forced)가 definite → 세로는 비율로.
        let root = LayoutNode::new(id(1))
            .with_fixed_size(len(120.0), len(100.0))
            .with_container(ContainerLayout::Stack(StackLayout::new(Axis::Horizontal)))
            .with_child(id(2), key(1));
        let child = LayoutNode::new(id(2))
            .with_sizing(SizingMode::Fill, SizingMode::HugContent)
            .with_aspect_ratio(ratio(2.0, 1.0));
        let input = LayoutInput::new(id(1), vec![root, child]);
        let result = compute_layout(&input).unwrap();
        assert_eq!(size_px(&result, 2), (120.0, 60.0));
        assert_eq!(LayoutInput::decode(&input.encode()).unwrap(), input);
        // 0·음수 비율 거부.
        for bad in [ratio(0.0, 1.0), ratio(1.0, -2.0)] {
            let input =
                LayoutInput::new(id(1), vec![LayoutNode::new(id(1)).with_aspect_ratio(bad)]);
            assert!(
                matches!(input.validate(), Err(LayoutError::InvalidSizing { .. })),
                "{bad:?}"
            );
        }
        // 0으로 반올림되는 양수 비율은 flex·fr과 같은 BelowResolution(리뷰 후속).
        let tiny = LayoutInput::new(
            id(1),
            vec![LayoutNode::new(id(1)).with_aspect_ratio(ratio(0.001, 1.0))],
        );
        assert_eq!(
            tiny.validate(),
            Err(LayoutError::BelowResolution {
                node: id(1),
                property: LayoutProperty::AspectRatio
            })
        );
    }

    #[test]
    fn weight_quantization_is_consistent_across_flex_fr_and_ratio() {
        // 리뷰 재현: flex grow·shrink와 grid fr의 1/128 미만 양수는 조용히 0이 되었습니다.
        // 지금은 세 가중치 입력이 같은 규칙(quantize_weight)을 씁니다.
        let scalar = |v: f32| Scalar::new(v).unwrap();
        let below = LayoutError::BelowResolution {
            node: id(7),
            property: LayoutProperty::Flex,
        };
        for (value, expected) in [
            (0.0, Ok(0)),
            // 정확히 1/128은 ties away from zero로 1 subpixel.
            (1.0 / 128.0, Ok(1)),
            (0.001, Err(below.clone())),
            (1.0 / 129.0, Err(below.clone())),
            (1.5, Ok(96)),
            (
                -0.001,
                Err(LayoutError::NegativeValue {
                    node: id(7),
                    property: LayoutProperty::Flex,
                }),
            ),
        ] {
            assert_eq!(
                quantize_weight(id(7), LayoutProperty::Flex, scalar(value)),
                expected,
                "{value}"
            );
        }
        // Stack 자식 grow·shrink 0.001 → 검증 단계 BelowResolution(이전에는 grow 0으로 동작).
        use crate::flex::{FlexBasis, FlexItem};
        for flex in [
            FlexItem::new(scalar(0.001), scalar(0.0), FlexBasis::Auto),
            FlexItem::new(scalar(0.0), scalar(0.004), FlexBasis::Auto),
        ] {
            let input = LayoutInput::new(
                id(1),
                vec![
                    LayoutNode::new(id(1))
                        .with_container(ContainerLayout::Stack(StackLayout::new(Axis::Horizontal)))
                        .with_child(id(2), key(1)),
                    LayoutNode::new(id(2)).with_flex(flex),
                ],
            );
            assert_eq!(
                input.validate(),
                Err(LayoutError::BelowResolution {
                    node: id(2),
                    property: LayoutProperty::Flex
                })
            );
        }
        // grid fr 0.005 → BelowResolution(이전에는 0fr).
        use crate::grid::{GridLayout, TrackBreadth, TrackSize};
        for track in [
            TrackSize::Fraction(scalar(0.005)),
            TrackSize::MinMax(
                TrackBreadth::Fixed(len(1.0)),
                TrackBreadth::Fraction(scalar(0.005)),
            ),
        ] {
            let input = LayoutInput::new(
                id(1),
                vec![LayoutNode::new(id(1))
                    .with_container(ContainerLayout::Grid(GridLayout::new(vec![track], vec![])))],
            );
            assert_eq!(
                input.validate(),
                Err(LayoutError::BelowResolution {
                    node: id(1),
                    property: LayoutProperty::ContainerLayout
                })
            );
        }
    }
}
