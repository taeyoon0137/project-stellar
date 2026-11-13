//! Stack 자식의 grow·shrink·basis (X.3.3).
//!
//! # 입력
//!
//! [`FlexItem`]은 Stack 부모의 main 축에서만 의미가 있습니다.
//!
//! - `basis`: flex base size. `Auto`는 자식 main sizing(`Fixed` 값, `HugContent` 콘텐츠 크기,
//!   `Percentage`)을 씁니다. `Length`·`Percentage`(content main 크기 기준)는 그 값을 씁니다.
//! - `grow`·`shrink`: 비음수 factor. 기본값은 둘 다 0(고정 크기 자식은 늘거나 줄지 않음)입니다.
//!   factor는 계산 전에 `1/64` 단위로 반올림합니다(레이아웃 subpixel 격자와 같은 규칙).
//! - main 축 `SizingMode::Fill`은 `grow 1, shrink 1, basis 0`과 같습니다. main 축 `Fill`과 기본값이
//!   아닌 `FlexItem`을 함께 쓰면 의미가 겹치므로 [`LayoutError::InvalidSizing`]으로 거부합니다.
//!
//! # 분배 (reference flex 알고리즘 축약)
//!
//! 1. hypothetical main = base를 [min, max]로 clamp(min 기본 0, max 없음).
//! 2. content main − gap 합 − hypothetical 합 > 0이면 grow, 아니면 shrink 모드입니다. factor가 0이거나
//!    base가 이미 clamp된 방향으로 벗어난 자식은 hypothetical 크기로 고정(frozen)합니다.
//! 3. 남은 공간(사용 가능 − frozen 크기 − 비고정 base 합)을 grow 모드는 grow factor, shrink 모드는
//!    `shrink × base`(scaled shrink factor) 비례로 정수 subpixel 분배합니다. 내림 몫 뒤 잔여 subpixel은
//!    StableId가 아니라 **child visual order** 앞에서부터 1씩 줍니다.
//! 4. min/max를 위반한 자식을 위반 방향에 따라 고정하고 2~3을 반복합니다(위반 합 0이면 종료).
//!
//! main 크기가 indefinite(부모 main Hug)이면 분배하지 않고 hypothetical 크기를 씁니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{FiniteF32, Percentage, Scalar};

use crate::codec::{Reader, Writer};
use crate::distribute::distribute_weighted;
use crate::error::LayoutError;
use crate::geometry::Axis;
use crate::input::{length_unit, LayoutNode};
use crate::property::LayoutProperty;
use crate::rounding::{LayoutUnit, LayoutUnitError};
use crate::sizing::SizingMode;

use stellar_core_types::value::Length;

/// flex base size (X.3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FlexBasis {
    /// 자식 main sizing을 사용.
    #[default]
    Auto,
    /// 길이(비음수).
    Length(Length),
    /// content main 크기의 백분율(비음수).
    Percentage(Percentage),
}

/// Stack 자식의 grow·shrink·basis (X.3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FlexItem {
    /// 늘림 factor(비음수).
    pub grow: Scalar,
    /// 줄임 factor(비음수).
    pub shrink: Scalar,
    /// base size.
    pub basis: FlexBasis,
}

impl Default for FlexItem {
    fn default() -> FlexItem {
        FlexItem {
            grow: Scalar::from_raw(FiniteF32::ZERO),
            shrink: Scalar::from_raw(FiniteF32::ZERO),
            basis: FlexBasis::Auto,
        }
    }
}

impl FlexItem {
    /// `grow`, `shrink`, `basis`로 만듭니다(factor 생성은 호출자가 finite로 보장).
    pub fn new(grow: Scalar, shrink: Scalar, basis: FlexBasis) -> FlexItem {
        FlexItem {
            grow,
            shrink,
            basis,
        }
    }

    /// 기본값(grow 0, shrink 0, Auto)인지 여부.
    pub fn is_default(&self) -> bool {
        *self == FlexItem::default()
    }
}

/// 변환된 basis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreparedBasis {
    Auto,
    Length(LayoutUnit),
    Percentage(FiniteF32),
}

/// 변환된 flex 입력(factor는 1/64 정수).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PreparedFlex {
    pub(crate) grow: u64,
    pub(crate) shrink: u64,
    pub(crate) basis: PreparedBasis,
}

/// main 축 Fill과 같은 factor(1.0 = 64).
pub(crate) const FILL_FACTOR: u64 = 64;

fn factor(node: StableId, value: Scalar) -> Result<u64, LayoutError> {
    let property = LayoutProperty::Flex;
    if value.get() < 0.0 {
        return Err(LayoutError::NegativeValue { node, property });
    }
    let unit =
        LayoutUnit::from_px(value.raw()).map_err(|_| LayoutError::OutOfRange { node, property })?;
    u64::try_from(unit.raw()).map_err(|_| LayoutError::NegativeValue { node, property })
}

/// flex 입력을 변환·검증합니다.
pub(crate) fn prepare_flex(node: &LayoutNode) -> Result<PreparedFlex, LayoutError> {
    let flex = node.flex;
    let basis = match flex.basis {
        FlexBasis::Auto => PreparedBasis::Auto,
        FlexBasis::Length(length) => {
            PreparedBasis::Length(length_unit(node.id, LayoutProperty::Flex, length, false)?)
        }
        FlexBasis::Percentage(percent) => {
            if percent.get() < 0.0 {
                return Err(LayoutError::NegativeValue {
                    node: node.id,
                    property: LayoutProperty::Flex,
                });
            }
            PreparedBasis::Percentage(percent.raw())
        }
    };
    Ok(PreparedFlex {
        grow: factor(node.id, flex.grow)?,
        shrink: factor(node.id, flex.shrink)?,
        basis,
    })
}

/// Stack 부모의 자식에서 main 축 Fill과 flex 설정 충돌을 거부합니다.
pub(crate) fn validate_flex_combination(node: &LayoutNode, main: Axis) -> Result<(), LayoutError> {
    let mode = match main {
        Axis::Horizontal => node.width,
        Axis::Vertical => node.height,
    };
    if mode == SizingMode::Fill && !node.flex.is_default() {
        return Err(LayoutError::InvalidSizing {
            node: node.id,
            axis: main,
            reason: "main축 Fill과 flex grow·shrink·basis를 함께 쓸 수 없습니다",
        });
    }
    Ok(())
}

const BASIS_AUTO: u8 = 0;
const BASIS_LENGTH: u8 = 1;
const BASIS_PERCENTAGE: u8 = 2;

/// flex canonical bytes(grow f32, shrink f32, basis tag + 값).
pub(crate) fn encode_flex(w: &mut Writer, flex: &FlexItem) {
    w.finite(flex.grow.raw());
    w.finite(flex.shrink.raw());
    match flex.basis {
        FlexBasis::Auto => w.u8(BASIS_AUTO),
        FlexBasis::Length(length) => {
            w.u8(BASIS_LENGTH);
            w.length(length);
        }
        FlexBasis::Percentage(percent) => {
            w.u8(BASIS_PERCENTAGE);
            w.finite(percent.raw());
        }
    }
}

/// flex canonical bytes를 읽습니다.
pub(crate) fn decode_flex(r: &mut Reader<'_>) -> Result<FlexItem, LayoutError> {
    let grow = Scalar::from_raw(r.finite("flex grow")?);
    let shrink = Scalar::from_raw(r.finite("flex shrink")?);
    let basis = match r.u8()? {
        BASIS_AUTO => FlexBasis::Auto,
        BASIS_LENGTH => FlexBasis::Length(r.length("flex basis")?),
        BASIS_PERCENTAGE => FlexBasis::Percentage(Percentage::from_raw(r.finite("flex basis")?)),
        _ => {
            return Err(LayoutError::Malformed {
                reason: "알 수 없는 flex basis tag",
            })
        }
    };
    Ok(FlexItem {
        grow,
        shrink,
        basis,
    })
}

/// 한 자식의 분배 입력.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FlexInput {
    pub(crate) base: LayoutUnit,
    pub(crate) grow: u64,
    pub(crate) shrink: u64,
    pub(crate) min: LayoutUnit,
    pub(crate) max: Option<LayoutUnit>,
}

impl FlexInput {
    fn clamp(&self, value: LayoutUnit) -> LayoutUnit {
        let upper = match self.max {
            Some(max) => value.min(max),
            None => value,
        };
        // min이 max보다 크면 min이 이깁니다(min > max 입력은 검증에서 거부).
        upper.max(self.min)
    }
}

/// `available`(content main − gap 합)을 자식들에게 분배한 main 크기(visual order)를 반환합니다.
pub(crate) fn resolve_flexible_lengths(
    available: LayoutUnit,
    items: &[FlexInput],
) -> Result<Vec<LayoutUnit>, LayoutUnitError> {
    let hypothetical: Vec<LayoutUnit> = items.iter().map(|i| i.clamp(i.base)).collect();
    let mut hypothetical_sum = LayoutUnit::ZERO;
    for value in &hypothetical {
        hypothetical_sum = hypothetical_sum.checked_add(*value)?;
    }
    let growing = available > hypothetical_sum;
    let mut sizes = hypothetical.clone();
    let mut frozen: Vec<bool> = items
        .iter()
        .zip(&hypothetical)
        .map(|(item, hyp)| {
            let factor = if growing { item.grow } else { item.shrink };
            factor == 0 || (growing && item.base > *hyp) || (!growing && item.base < *hyp)
        })
        .collect();

    // 반복마다 하나 이상의 자식이 고정되므로 자식 수 + 1번 안에 끝납니다.
    for _ in 0..=items.len() {
        if frozen.iter().all(|f| *f) {
            break;
        }
        let mut remaining = available;
        for (i, item) in items.iter().enumerate() {
            let used = if frozen[i] { sizes[i] } else { item.base };
            remaining = remaining.checked_sub(used)?;
        }
        let mut target = sizes.clone();
        if growing {
            let weights: Vec<u64> = items
                .iter()
                .enumerate()
                .map(|(i, item)| if frozen[i] { 0 } else { item.grow })
                .collect();
            let shares = distribute_weighted(remaining.max(LayoutUnit::ZERO), &weights)?;
            for (i, item) in items.iter().enumerate() {
                if !frozen[i] {
                    target[i] = item.base.checked_add(shares[i])?;
                }
            }
        } else {
            let weights: Vec<u64> = items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    if frozen[i] {
                        0
                    } else {
                        // base ≥ 0이라 u64 변환은 손실이 없습니다.
                        item.shrink * u64::try_from(item.base.raw()).unwrap_or(0)
                    }
                })
                .collect();
            let deficit = LayoutUnit::ZERO
                .checked_sub(remaining)?
                .max(LayoutUnit::ZERO);
            let shares = distribute_weighted(deficit, &weights)?;
            for (i, item) in items.iter().enumerate() {
                if !frozen[i] {
                    target[i] = item.base.checked_sub(shares[i])?;
                }
            }
        }
        // min/max 위반 처리.
        let mut violation = 0i64;
        let mut clamped = target.clone();
        for (i, item) in items.iter().enumerate() {
            if !frozen[i] {
                clamped[i] = item.clamp(target[i]);
                violation += i64::from(clamped[i].raw()) - i64::from(target[i].raw());
            }
        }
        for i in 0..items.len() {
            if frozen[i] {
                continue;
            }
            let min_violation = clamped[i] > target[i];
            let max_violation = clamped[i] < target[i];
            let freeze = match violation.signum() {
                0 => true,
                1 => min_violation,
                _ => max_violation,
            };
            if freeze {
                frozen[i] = true;
                sizes[i] = clamped[i];
            }
        }
        if violation == 0 {
            break;
        }
    }
    Ok(sizes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(value: i32) -> LayoutUnit {
        LayoutUnit::from_px_int(value).unwrap()
    }

    fn item(base: i32, grow: u64, shrink: u64) -> FlexInput {
        FlexInput {
            base: px(base),
            grow: grow * 64,
            shrink: shrink * 64,
            min: LayoutUnit::ZERO,
            max: None,
        }
    }

    fn to_px(values: &[LayoutUnit]) -> Vec<f64> {
        values.iter().map(LayoutUnit::to_px_f64).collect()
    }

    #[test]
    fn grow_distributes_free_space_by_factor() {
        // reference: basis 50/100/50, grow 1/2/1, 사용 가능 300 → free 100 → 75/150/75.
        let sizes =
            resolve_flexible_lengths(px(300), &[item(50, 1, 0), item(100, 2, 0), item(50, 1, 0)])
                .unwrap();
        assert_eq!(to_px(&sizes), vec![75.0, 150.0, 75.0]);
        // grow 0 자식은 그대로.
        let sizes = resolve_flexible_lengths(px(300), &[item(50, 0, 0), item(100, 1, 0)]).unwrap();
        assert_eq!(to_px(&sizes), vec![50.0, 250.0]);
    }

    #[test]
    fn shrink_uses_scaled_factor_and_freezes_at_min() {
        // reference: basis 100/100/100, shrink 1/1/2, 사용 가능 200 → scaled 100/100/200 → 75/75/50.
        let sizes = resolve_flexible_lengths(
            px(200),
            &[item(100, 0, 1), item(100, 0, 1), item(100, 0, 2)],
        )
        .unwrap();
        assert_eq!(to_px(&sizes), vec![75.0, 75.0, 50.0]);
        // basis 10/200, shrink 10/1, 사용 가능 100: 첫 계산에서 첫 자식이 음수 → 0으로 고정 후
        // 두 번째가 남은 부족분을 모두 흡수 → 0/100.
        let sizes = resolve_flexible_lengths(px(100), &[item(10, 0, 10), item(200, 0, 1)]).unwrap();
        assert_eq!(to_px(&sizes), vec![0.0, 100.0]);
        // shrink 0이면 넘침 그대로.
        let sizes = resolve_flexible_lengths(px(100), &[item(80, 0, 0), item(80, 0, 0)]).unwrap();
        assert_eq!(to_px(&sizes), vec![80.0, 80.0]);
    }

    #[test]
    fn residual_goes_to_visual_order_front() {
        // 100 px = 6400 subpixel / grow 1·1·1 → 2134, 2133, 2133(잔여 1은 visual order 첫 자식).
        let sizes =
            resolve_flexible_lengths(px(100), &[item(0, 1, 0), item(0, 1, 0), item(0, 1, 0)])
                .unwrap();
        let raw: Vec<i32> = sizes.iter().map(LayoutUnit::raw).collect();
        assert_eq!(raw, vec![2134, 2133, 2133]);
    }

    #[test]
    fn max_violation_freezes_and_redistributes() {
        let mut capped = item(0, 1, 0);
        capped.max = Some(px(20));
        let sizes =
            resolve_flexible_lengths(px(100), &[capped, item(0, 1, 0), item(0, 1, 0)]).unwrap();
        assert_eq!(to_px(&sizes), vec![20.0, 40.0, 40.0]);
    }
}
