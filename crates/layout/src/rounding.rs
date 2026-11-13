//! 레이아웃 수치 rounding 정책 (X.1.5).
//!
//! # 정책
//!
//! 1. **logical subpixel 고정**: 레이아웃 수치는 logical px의 `1/64`([`SUBPIXELS_PER_PX`]) 단위
//!    정수 [`LayoutUnit`]으로 계산·확정합니다. 레이아웃 결과는 이 격자 위의 값만 가집니다.
//! 2. **float 입력 변환**: [`LayoutUnit::from_px`]는 가장 가까운 subpixel로 반올림하고, 정확히
//!    중간이면 0에서 먼 쪽으로 보냅니다(ties away from zero). 양수·음수에 대칭입니다.
//! 3. **정수 산술**: 덧셈·뺄셈·정수배·비율·나눗셈은 정수(`i128` 중간값)로 수행합니다. 비율과
//!    나눗셈의 반올림도 2와 같은 규칙이며, 범위를 넘으면 오류입니다(조용히 saturate하지 않음).
//! 4. **pixel snapping 분리**: 레이아웃은 device scale(디스플레이 배율)을 입력으로 받지 않으며
//!    device pixel에 맞추지 않습니다. device pixel snapping은 scene 값을 받은 렌더 단계
//!    (`renderer-core`)의 책임입니다. 같은 문서는 배율 1.0·1.5·2.0 환경에서 같은 레이아웃 값을
//!    가집니다.
//! 5. **플랫폼 독립 계산**: 변환은 IEEE-754 정확 연산(2의 거듭제곱 곱셈·뺄셈·비교)과 Rust가
//!    정의한 정수 캐스트만 사용하고, 플랫폼 수학 라이브러리 함수(`round` 등)에 의존하지
//!    않습니다. 따라서 macOS·Windows에서 bit 단위로 같은 결과를 냅니다.
//!
//! `1/64`은 2의 거듭제곱이라 [`LayoutUnit::to_px_f32`] 값(`raw/64`)에 기약 dyadic 배율 `m/2^k`
//! (`m` 홀수)를 곱한 값은 `|raw|·m ≤ 2^24`, 즉 `|px| ≤ 2^18/m`이면 f32에서 반올림 오차 없이
//! 정확합니다(충분 조건). 경계는 배율의 홀수 인자 `m`에 따라 다릅니다: 1.5(= 3/2, `m = 3`)는 약
//! 87,381 px, 1.25(= 5/4, `m = 5`)는 약 52,428 px, 2(`m = 1`)는 262,144 px입니다. 경계를 넘어도
//! f32 곱은 IEEE-754 기본 연산이라 플랫폼 간 결정적이지만, 정확한 유리수 값과는 다를 수 있습니다.

use std::fmt;

use stellar_core_types::value::{FiniteF32, Length};

/// logical px 하나당 subpixel 수(2의 거듭제곱). 레이아웃 rounding 격자입니다.
pub const SUBPIXELS_PER_PX: i32 = 64;

/// `SUBPIXELS_PER_PX`의 f32 값(정확히 표현됨).
const SUBPIXELS_PER_PX_F32: f32 = 64.0;

/// `i32` 범위의 f32 경계(2^31). f32로 정확히 표현됩니다.
const I32_RANGE_F32: f32 = 2_147_483_648.0;

/// 레이아웃 수치 오류 (X.1.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutUnitError {
    /// 결과가 `LayoutUnit`(`i32` subpixel) 범위를 벗어났습니다.
    OutOfRange,
    /// 0으로 나누려 했습니다.
    DivisionByZero,
}

impl fmt::Display for LayoutUnitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LayoutUnitError::OutOfRange => {
                f.write_str("레이아웃 수치가 subpixel 범위를 벗어났습니다")
            }
            LayoutUnitError::DivisionByZero => {
                f.write_str("레이아웃 수치를 0으로 나눌 수 없습니다")
            }
        }
    }
}

impl std::error::Error for LayoutUnitError {}

/// logical subpixel(`1/64` px) 단위 고정소수점 레이아웃 수치 (X.1.5).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LayoutUnit(i32);

impl LayoutUnit {
    /// 0.
    pub const ZERO: LayoutUnit = LayoutUnit(0);
    /// 표현 가능한 최댓값.
    pub const MAX: LayoutUnit = LayoutUnit(i32::MAX);
    /// 표현 가능한 최솟값.
    pub const MIN: LayoutUnit = LayoutUnit(i32::MIN);

    /// raw subpixel 수로부터 만듭니다.
    pub const fn from_raw(subpixels: i32) -> LayoutUnit {
        LayoutUnit(subpixels)
    }

    /// raw subpixel 수.
    pub const fn raw(&self) -> i32 {
        self.0
    }

    /// 정수 px 값으로부터 만듭니다(정확).
    pub fn from_px_int(px: i32) -> Result<LayoutUnit, LayoutUnitError> {
        px.checked_mul(SUBPIXELS_PER_PX)
            .map(LayoutUnit)
            .ok_or(LayoutUnitError::OutOfRange)
    }

    /// 유한 float px 값을 가장 가까운 subpixel로 반올림합니다(ties away from zero).
    pub fn from_px(px: FiniteF32) -> Result<LayoutUnit, LayoutUnitError> {
        // 2의 거듭제곱 곱셈은 overflow가 없으면 정확합니다. overflow(±무한대)는 아래 범위
        // 검사에서 거부됩니다.
        let scaled = px.get() * SUBPIXELS_PER_PX_F32;
        let rounded = round_half_away_from_zero(scaled).ok_or(LayoutUnitError::OutOfRange)?;
        i32::try_from(rounded)
            .map(LayoutUnit)
            .map_err(|_| LayoutUnitError::OutOfRange)
    }

    /// 길이 값을 가장 가까운 subpixel로 반올림합니다([`LayoutUnit::from_px`]와 같은 규칙).
    pub fn from_length(length: Length) -> Result<LayoutUnit, LayoutUnitError> {
        LayoutUnit::from_px(length.raw())
    }

    /// 유리수 px 값 `numerator / denominator`를 가장 가까운 subpixel로 반올림합니다.
    ///
    /// float을 거치지 않으므로 `1/3` px처럼 2진 float으로 정확히 표현할 수 없는 값도 정확한
    /// 유리수에서 한 번만 반올림합니다.
    pub fn from_ratio_px(numerator: i64, denominator: i64) -> Result<LayoutUnit, LayoutUnitError> {
        let scaled = i128::from(numerator) * i128::from(SUBPIXELS_PER_PX);
        div_round_half_away_from_zero(scaled, i128::from(denominator)).and_then(to_unit)
    }

    /// logical px f32 값. `|raw| ≤ 2^24`이면 정확하고, 그보다 크면 Rust 정의 캐스트(최근접
    /// 짝수 반올림)로 결정적으로 변환됩니다. 결과는 항상 유한합니다.
    pub fn to_px_f32(&self) -> FiniteF32 {
        // i32 → f32 캐스트는 유한하고, 2의 거듭제곱 나눗셈은 정확하므로 항상 유한합니다.
        FiniteF32::new(self.0 as f32 / SUBPIXELS_PER_PX_F32).expect("i32/64는 항상 유한")
    }

    /// logical px f64 값(모든 `i32` raw에 대해 정확).
    pub fn to_px_f64(&self) -> f64 {
        f64::from(self.0) / f64::from(SUBPIXELS_PER_PX)
    }

    /// 길이 값([`LayoutUnit::to_px_f32`]와 같은 값).
    pub fn to_length(&self) -> Length {
        Length::from_raw(self.to_px_f32())
    }

    /// 덧셈. 범위를 넘으면 오류입니다.
    pub fn checked_add(self, rhs: LayoutUnit) -> Result<LayoutUnit, LayoutUnitError> {
        self.0
            .checked_add(rhs.0)
            .map(LayoutUnit)
            .ok_or(LayoutUnitError::OutOfRange)
    }

    /// 뺄셈. 범위를 넘으면 오류입니다.
    pub fn checked_sub(self, rhs: LayoutUnit) -> Result<LayoutUnit, LayoutUnitError> {
        self.0
            .checked_sub(rhs.0)
            .map(LayoutUnit)
            .ok_or(LayoutUnitError::OutOfRange)
    }

    /// 정수배. 범위를 넘으면 오류입니다.
    pub fn checked_mul_int(self, factor: i32) -> Result<LayoutUnit, LayoutUnitError> {
        self.0
            .checked_mul(factor)
            .map(LayoutUnit)
            .ok_or(LayoutUnitError::OutOfRange)
    }

    /// 비율 `numerator / denominator`을 곱하고 가장 가까운 subpixel로 반올림합니다
    /// (ties away from zero).
    pub fn checked_mul_ratio(
        self,
        numerator: i64,
        denominator: i64,
    ) -> Result<LayoutUnit, LayoutUnitError> {
        let product = i128::from(self.0) * i128::from(numerator);
        div_round_half_away_from_zero(product, i128::from(denominator)).and_then(to_unit)
    }

    /// 정수로 나누고 가장 가까운 subpixel로 반올림합니다(ties away from zero).
    ///
    /// 나머지 subpixel의 분배(예: flex 잔여값)는 호출자의 배치 정책 소관입니다.
    pub fn checked_div_round(self, divisor: i32) -> Result<LayoutUnit, LayoutUnitError> {
        div_round_half_away_from_zero(i128::from(self.0), i128::from(divisor)).and_then(to_unit)
    }

    /// 백분율 `percent`%(f32, `50` = 50%)를 곱하고 가장 가까운 subpixel로 한 번만 반올림합니다
    /// (ties away from zero, X.2.3).
    ///
    /// f32 백분율을 정확한 유리수 `m·2^e`(비트 분해)로 바꿔 `raw·m·2^e / 100`을 `i128` 정수로
    /// 계산하므로 float 곱의 중간 반올림이 없고 플랫폼 간 같은 값입니다.
    pub fn checked_mul_percent(self, percent: FiniteF32) -> Result<LayoutUnit, LayoutUnitError> {
        let (mantissa, exponent) = decompose_f32(percent.get());
        let numerator = i128::from(self.0) * i128::from(mantissa);
        if numerator == 0 {
            return Ok(LayoutUnit::ZERO);
        }
        if exponent >= 0 {
            // |numerator| < 2^55. 지수가 60을 넘으면 결과가 2^115/100 이상이라 범위 밖입니다.
            if exponent > 60 {
                return Err(LayoutUnitError::OutOfRange);
            }
            div_round_half_away_from_zero(numerator << exponent, 100).and_then(to_unit)
        } else {
            let shift = -exponent;
            // |numerator| < 2^55 < 100·2^100 / 2 이면 결과는 0으로 반올림됩니다.
            if shift > 100 {
                return Ok(LayoutUnit::ZERO);
            }
            div_round_half_away_from_zero(numerator, 100_i128 << shift).and_then(to_unit)
        }
    }
}

/// 유한 f32를 정확한 `(mantissa, exponent)`로 분해합니다(`value = mantissa · 2^exponent`).
fn decompose_f32(value: f32) -> (i64, i32) {
    let bits = value.to_bits();
    let negative = bits >> 31 == 1;
    let biased = ((bits >> 23) & 0xff) as i32;
    let fraction = i64::from(bits & 0x007f_ffff);
    let (mantissa, exponent) = if biased == 0 {
        // 0과 subnormal: fraction · 2^-149.
        (fraction, -149)
    } else {
        (fraction | 0x0080_0000, biased - 150)
    };
    (if negative { -mantissa } else { mantissa }, exponent)
}

impl fmt::Display for LayoutUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // raw/64는 유한 2진 소수라 f64 표현이 정확하고 서식도 플랫폼 독립입니다.
        write!(f, "{}px", self.to_px_f64())
    }
}

/// 정수 나눗셈 `numerator / denominator`를 가장 가까운 subpixel로 반올림합니다(ties away from
/// zero, crate 내부). `|numerator|`와 `|denominator|`는 `2^125` 미만이어야 합니다.
pub(crate) fn div_round(numerator: i128, denominator: i128) -> Result<LayoutUnit, LayoutUnitError> {
    div_round_half_away_from_zero(numerator, denominator).and_then(to_unit)
}

/// `i128` 중간값을 `LayoutUnit` 범위로 좁힙니다.
fn to_unit(value: i128) -> Result<LayoutUnit, LayoutUnitError> {
    i32::try_from(value)
        .map(LayoutUnit)
        .map_err(|_| LayoutUnitError::OutOfRange)
}

/// f32를 가장 가까운 정수로 반올림합니다(ties away from zero). `i32` 범위(±2^31) 밖이거나
/// 비유한이면 `None`.
///
/// 플랫폼 수학 함수 없이 정확 연산만 씁니다: 범위 안에서 `as i64`는 0 방향 절삭으로 정의되어
/// 있고, 절삭값은 f32로 정확히 표현되며(|x| ≥ 2^23이면 x 자체가 정수), `x - 절삭값`(소수부)도
/// 정확합니다.
fn round_half_away_from_zero(x: f32) -> Option<i64> {
    // 범위 검사는 NaN·±무한대도 함께 거부합니다(포함 비교가 거짓).
    if !(-I32_RANGE_F32..I32_RANGE_F32).contains(&x) {
        return None;
    }
    let truncated = x as i64;
    let fraction = x - truncated as f32;
    let rounded = if fraction >= 0.5 {
        truncated + 1
    } else if fraction <= -0.5 {
        truncated - 1
    } else {
        truncated
    };
    Some(rounded)
}

/// 정수 나눗셈을 가장 가까운 정수로 반올림합니다(ties away from zero).
fn div_round_half_away_from_zero(
    numerator: i128,
    denominator: i128,
) -> Result<i128, LayoutUnitError> {
    if denominator == 0 {
        return Err(LayoutUnitError::DivisionByZero);
    }
    // 입력은 i64 곱 범위라 i128 몫·나머지·2배 나머지가 overflow하지 않습니다.
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    if 2 * remainder.abs() >= denominator.abs() {
        // 나머지가 절반 이상이면 0에서 먼 쪽(몫의 부호 방향)으로 한 칸 이동.
        let away = if (numerator < 0) == (denominator < 0) {
            1
        } else {
            -1
        };
        Ok(quotient + away)
    } else {
        Ok(quotient)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finite(value: f32) -> FiniteF32 {
        FiniteF32::new(value).unwrap()
    }

    /// 플랫폼 간 동일성 fixture 한 줄: f32 입력 bits → 기대 subpixel → 기대 출력 bits
    /// → 배율 1.5 렌더 입력(snapping 전) 기대 bits.
    struct RoundingCase {
        name: &'static str,
        input_bits: u32,
        expected_raw: i32,
        expected_px_bits: u32,
        expected_device_1_5_bits: u32,
    }

    /// 1/3·배율 1.5 golden fixture. 값은 정확 연산만으로 정해지므로 macOS·Windows에서 같아야
    /// 합니다. 이 표가 바뀌면 플랫폼 간 레이아웃 결과 계약이 깨진 것입니다.
    const ROUNDING_FIXTURE: &[RoundingCase] = &[
        // 1/3 px(f32 0.33333334) → 21.333… subpixel → 21 → 0.328125 px.
        RoundingCase {
            name: "one_third",
            input_bits: 0x3eaa_aaab,
            expected_raw: 21,
            expected_px_bits: 0x3ea8_0000,
            expected_device_1_5_bits: 0x3efc_0000,
        },
        // -1/3 px → 대칭으로 -21.
        RoundingCase {
            name: "minus_one_third",
            input_bits: 0xbeaa_aaab,
            expected_raw: -21,
            expected_px_bits: 0xbea8_0000,
            expected_device_1_5_bits: 0xbefc_0000,
        },
        // 2/3 px = 배율 1.5에서 device px 1개(1/1.5) → 42.666… → 43 → 0.671875 px.
        RoundingCase {
            name: "two_thirds_one_device_px_at_1_5",
            input_bits: 0x3f2a_aaab,
            expected_raw: 43,
            expected_px_bits: 0x3f2c_0000,
            expected_device_1_5_bits: 0x3f81_0000,
        },
        // 100/3 px(33.333332) → 2133.333… → 2133 → 33.328125 px.
        RoundingCase {
            name: "hundred_thirds",
            input_bits: 0x4205_5555,
            expected_raw: 2133,
            expected_px_bits: 0x4205_5000,
            expected_device_1_5_bits: 0x4247_f800,
        },
        // 0.1 px(0.1f32) → 6.4 → 6 → 0.09375 px.
        RoundingCase {
            name: "one_tenth",
            input_bits: 0x3dcc_cccd,
            expected_raw: 6,
            expected_px_bits: 0x3dc0_0000,
            expected_device_1_5_bits: 0x3e10_0000,
        },
        // 정확히 중간(1/128 px = 0.5 subpixel) → 0에서 먼 쪽 1.
        RoundingCase {
            name: "half_subpixel_tie",
            input_bits: 0x3c00_0000,
            expected_raw: 1,
            expected_px_bits: 0x3c80_0000,
            expected_device_1_5_bits: 0x3cc0_0000,
        },
        // 2.5 subpixel(5/128 px) → 3(최근접 짝수 규칙이면 2가 됨).
        RoundingCase {
            name: "two_and_half_subpixel_tie",
            input_bits: 0x3d20_0000,
            expected_raw: 3,
            expected_px_bits: 0x3d40_0000,
            expected_device_1_5_bits: 0x3d90_0000,
        },
        // -2.5 subpixel → -3(대칭).
        RoundingCase {
            name: "minus_two_and_half_subpixel_tie",
            input_bits: 0xbd20_0000,
            expected_raw: -3,
            expected_px_bits: 0xbd40_0000,
            expected_device_1_5_bits: 0xbd90_0000,
        },
    ];

    #[test]
    fn one_third_and_scale_1_5_fixture_is_bit_exact() {
        for case in ROUNDING_FIXTURE {
            let input = finite(f32::from_bits(case.input_bits));
            let unit = LayoutUnit::from_px(input).unwrap();
            assert_eq!(unit.raw(), case.expected_raw, "{}: subpixel", case.name);
            let px = unit.to_px_f32().get();
            assert_eq!(
                px.to_bits(),
                case.expected_px_bits,
                "{}: px bits",
                case.name
            );

            // 렌더 단계가 배율 1.5를 곱한 값(snapping 전). f32 곱이 정확한 유리수 raw·3/128과
            // 같아야 반올림 오차 없이 플랫폼 간 동일합니다.
            let device = px * 1.5;
            assert_eq!(
                device.to_bits(),
                case.expected_device_1_5_bits,
                "{}: device bits",
                case.name
            );
            let exact = f64::from(case.expected_raw) * 3.0 / 128.0;
            assert_eq!(f64::from(device), exact, "{}: device exact", case.name);
        }
        assert_eq!(ROUNDING_FIXTURE.len(), 8);
    }

    #[test]
    fn dyadic_scale_exactness_bound_depends_on_odd_factor() {
        // (배율 분자 m, 분모 2^k): |raw|·m ≤ 2^24이면 to_px_f32 × 배율이 정확한 유리수와 같습니다.
        for (m, k) in [(3_i64, 1_u32), (5, 2), (1, 1)] {
            let scale = m as f32 / (1_u32 << k) as f32;
            let bound = (1_i64 << 24) / m;
            let exact = |raw: i64| raw as f64 * m as f64 / f64::from(64_u32 << k);
            for raw in [1, bound - 1, bound, -bound] {
                let unit = LayoutUnit::from_raw(i32::try_from(raw).unwrap());
                let device = unit.to_px_f32().get() * scale;
                assert_eq!(
                    f64::from(device),
                    exact(raw),
                    "m={m}, raw={raw}: 경계 안 정확"
                );
            }
            // 경계 바로 위의 홀수 raw는 유효 비트가 24를 넘어 정확하지 않습니다(충분 조건이 최적).
            let over = if (bound + 1) % 2 == 1 {
                bound + 1
            } else {
                bound + 2
            };
            let unit = LayoutUnit::from_raw(i32::try_from(over).unwrap());
            let device = unit.to_px_f32().get() * scale;
            assert_ne!(f64::from(device), exact(over), "m={m}, raw={over}: 경계 밖");
        }
        // px 경계: 1.5 → 2^18/3 ≈ 87,381 px, 1.25 → 2^18/5 ≈ 52,428 px, 2 → 262,144 px.
        assert_eq!((1_i64 << 24) / 3 / 64, 87_381);
        assert_eq!((1_i64 << 24) / 5 / 64, 52_428);
        assert_eq!((1_i64 << 24) / 64, 262_144);
    }

    #[test]
    fn exact_rational_one_third_rounds_once() {
        // float을 거치지 않은 정확한 유리수 입력도 같은 격자·같은 규칙.
        assert_eq!(LayoutUnit::from_ratio_px(1, 3).unwrap().raw(), 21);
        assert_eq!(LayoutUnit::from_ratio_px(-1, 3).unwrap().raw(), -21);
        assert_eq!(LayoutUnit::from_ratio_px(2, 3).unwrap().raw(), 43);
        // 배율 1.5의 device px 1개 = 1/1.5 = 2/3 px.
        assert_eq!(
            LayoutUnit::from_ratio_px(2, 3),
            LayoutUnit::from_ratio_px(10, 15)
        );
        assert_eq!(LayoutUnit::from_ratio_px(100, 3).unwrap().raw(), 2133);
        // 정확히 중간: 1/128 px = 0.5 subpixel → 1, 5/128 → 3, -5/128 → -3.
        assert_eq!(LayoutUnit::from_ratio_px(1, 128).unwrap().raw(), 1);
        assert_eq!(LayoutUnit::from_ratio_px(5, 128).unwrap().raw(), 3);
        assert_eq!(LayoutUnit::from_ratio_px(5, -128).unwrap().raw(), -3);
        assert_eq!(
            LayoutUnit::from_ratio_px(1, 0),
            Err(LayoutUnitError::DivisionByZero)
        );
        // float 경로와 유리수 경로가 1/3에서 같은 값.
        assert_eq!(
            LayoutUnit::from_px(finite(1.0 / 3.0)),
            LayoutUnit::from_ratio_px(1, 3)
        );
    }

    #[test]
    fn layout_values_are_not_snapped_to_device_pixels() {
        // 배율 1.5의 device pixel 격자는 2/3 px 간격입니다. 1/3 px는 그 격자에 맞추지 않고
        // logical subpixel(21/64 px)로 남아야 합니다(snapping은 렌더 단계).
        let unit = LayoutUnit::from_ratio_px(1, 3).unwrap();
        assert_eq!(unit.to_px_f64(), 0.328125);
        let device = unit.to_px_f64() * 1.5;
        assert_eq!(device, 0.4921875);
        assert_ne!(
            device,
            device.trunc(),
            "레이아웃 값은 device 정수 격자 밖일 수 있음"
        );
        // 같은 값을 배율과 무관하게 얻습니다(API가 배율을 받지 않음).
        assert_eq!(LayoutUnit::from_px(finite(1.0 / 3.0)).unwrap(), unit);
    }

    #[test]
    fn integer_arithmetic_rounding_policy() {
        let hundred = LayoutUnit::from_px_int(100).unwrap();
        assert_eq!(hundred.raw(), 6400);
        // 100 px / 3 = 2133.33… subpixel → 2133.
        assert_eq!(hundred.checked_div_round(3).unwrap().raw(), 2133);
        // 비율 1/3도 같은 결과.
        assert_eq!(hundred.checked_mul_ratio(1, 3).unwrap().raw(), 2133);
        // 21 subpixel × 1.5(3/2) = 31.5 → 32(ties away).
        assert_eq!(
            LayoutUnit::from_raw(21)
                .checked_mul_ratio(3, 2)
                .unwrap()
                .raw(),
            32
        );
        assert_eq!(
            LayoutUnit::from_raw(-21)
                .checked_mul_ratio(3, 2)
                .unwrap()
                .raw(),
            -32
        );
        // 5/2 = 2.5 → 3, -5/2 → -3, 5/(-2) → -3.
        assert_eq!(
            LayoutUnit::from_raw(5).checked_div_round(2).unwrap().raw(),
            3
        );
        assert_eq!(
            LayoutUnit::from_raw(-5).checked_div_round(2).unwrap().raw(),
            -3
        );
        assert_eq!(
            LayoutUnit::from_raw(5).checked_div_round(-2).unwrap().raw(),
            -3
        );
        // 2.4 → 2.
        assert_eq!(
            LayoutUnit::from_raw(12).checked_div_round(5).unwrap().raw(),
            2
        );
        assert_eq!(
            hundred.checked_div_round(0),
            Err(LayoutUnitError::DivisionByZero)
        );
        assert_eq!(
            hundred.checked_mul_ratio(1, 0),
            Err(LayoutUnitError::DivisionByZero)
        );
        // 덧셈·뺄셈·정수배는 정확.
        let third = LayoutUnit::from_raw(21);
        assert_eq!(third.checked_add(third).unwrap().raw(), 42);
        assert_eq!(third.checked_sub(hundred).unwrap().raw(), 21 - 6400);
        assert_eq!(third.checked_mul_int(3).unwrap().raw(), 63);
    }

    #[test]
    fn out_of_range_is_error_not_saturation() {
        // i32::MAX / 64 ≈ 33,554,431.98 px. 2^24 초과 구간의 f32는 짝수만 표현하므로 33,554,430을
        // 경계 안쪽 값으로 씁니다.
        assert_eq!(
            LayoutUnit::from_px(finite(33_554_430.0)).unwrap().raw(),
            33_554_430 * 64
        );
        assert_eq!(
            LayoutUnit::from_px(finite(33_554_432.0)),
            Err(LayoutUnitError::OutOfRange)
        );
        assert_eq!(
            LayoutUnit::from_px(finite(f32::MAX)),
            Err(LayoutUnitError::OutOfRange)
        );
        assert_eq!(
            LayoutUnit::from_px(finite(-33_554_432.0)).unwrap(),
            LayoutUnit::MIN
        );
        assert_eq!(
            LayoutUnit::from_px_int(33_554_432),
            Err(LayoutUnitError::OutOfRange)
        );
        assert_eq!(
            LayoutUnit::MAX.checked_add(LayoutUnit::from_raw(1)),
            Err(LayoutUnitError::OutOfRange)
        );
        assert_eq!(
            LayoutUnit::MIN.checked_sub(LayoutUnit::from_raw(1)),
            Err(LayoutUnitError::OutOfRange)
        );
        assert_eq!(
            LayoutUnit::MAX.checked_mul_int(2),
            Err(LayoutUnitError::OutOfRange)
        );
        assert_eq!(
            LayoutUnit::MAX.checked_mul_ratio(3, 2),
            Err(LayoutUnitError::OutOfRange)
        );
        assert_eq!(
            LayoutUnit::from_ratio_px(i64::MAX, 1),
            Err(LayoutUnitError::OutOfRange)
        );
        assert_eq!(
            LayoutUnit::MIN.checked_div_round(-1),
            Err(LayoutUnitError::OutOfRange)
        );
    }

    #[test]
    fn negative_zero_and_length_conversion() {
        assert_eq!(LayoutUnit::from_px(finite(-0.0)).unwrap(), LayoutUnit::ZERO);
        let length = Length::new(12.25).unwrap();
        let unit = LayoutUnit::from_length(length).unwrap();
        assert_eq!(unit.raw(), 784);
        assert_eq!(unit.to_length(), length);
        // 아주 작은 값(subpixel 절반 미만)은 0.
        assert_eq!(
            LayoutUnit::from_px(finite(0.007)).unwrap(),
            LayoutUnit::ZERO
        );
        assert_eq!(
            LayoutUnit::from_px(finite(-0.007)).unwrap(),
            LayoutUnit::ZERO
        );
    }

    #[test]
    fn round_trip_on_grid_is_exact() {
        // 격자 위 값은 f32 왕복이 정확합니다(|raw| ≤ 2^24).
        for raw in [
            -(1 << 24),
            -6400,
            -21,
            -1,
            0,
            1,
            21,
            43,
            2133,
            6400,
            1 << 24,
        ] {
            let unit = LayoutUnit::from_raw(raw);
            assert_eq!(LayoutUnit::from_px(unit.to_px_f32()).unwrap(), unit);
            assert_eq!(unit.to_px_f64() * 64.0, f64::from(raw));
        }
        // 큰 raw도 f64 변환은 정확.
        assert_eq!(LayoutUnit::MAX.to_px_f64() * 64.0, f64::from(i32::MAX));
    }

    #[test]
    fn percent_multiplication_is_exact_and_rounds_once() {
        let hundred = LayoutUnit::from_px_int(100).unwrap();
        // 50% of 100 px = 3200 subpixel.
        assert_eq!(
            hundred.checked_mul_percent(finite(50.0)).unwrap().raw(),
            3200
        );
        // f32 33.333332% of 100 px = 2133.33… → 2133(1/3 px 정책과 같음).
        assert_eq!(
            hundred
                .checked_mul_percent(finite(100.0 / 3.0))
                .unwrap()
                .raw(),
            2133
        );
        // 12.5% of 21 subpixel = 2.625 → 3, 음수 백분율 대칭.
        let third = LayoutUnit::from_raw(21);
        assert_eq!(third.checked_mul_percent(finite(12.5)).unwrap().raw(), 3);
        assert_eq!(third.checked_mul_percent(finite(-12.5)).unwrap().raw(), -3);
        // 정확히 중간: 50% of 1 subpixel = 0.5 → 1, -50% → -1.
        let one = LayoutUnit::from_raw(1);
        assert_eq!(one.checked_mul_percent(finite(50.0)).unwrap().raw(), 1);
        assert_eq!(one.checked_mul_percent(finite(-50.0)).unwrap().raw(), -1);
        // 0.1%(f32 0.1은 0.1보다 약간 큼) of 6400 → 6.4000001 → 6.
        assert_eq!(hundred.checked_mul_percent(finite(0.1)).unwrap().raw(), 6);
        // 0%, 아주 작은 값, subnormal은 0. 큰 값은 범위 오류.
        assert_eq!(
            hundred.checked_mul_percent(finite(0.0)).unwrap(),
            LayoutUnit::ZERO
        );
        assert_eq!(
            hundred.checked_mul_percent(finite(1.0e-30)).unwrap(),
            LayoutUnit::ZERO
        );
        assert_eq!(
            LayoutUnit::MAX
                .checked_mul_percent(finite(f32::from_bits(1)))
                .unwrap(),
            LayoutUnit::ZERO
        );
        assert_eq!(
            one.checked_mul_percent(finite(1.0e30)),
            Err(LayoutUnitError::OutOfRange)
        );
        assert_eq!(
            LayoutUnit::MAX.checked_mul_percent(finite(200.0)),
            Err(LayoutUnitError::OutOfRange)
        );
        assert_eq!(
            LayoutUnit::MAX.checked_mul_percent(finite(100.0)).unwrap(),
            LayoutUnit::MAX
        );
        // f64 기준값과 비교(정확 계산이 float 왕복과 일치하는 구간).
        for (raw, percent) in [(6400, 37.5_f32), (-777, 66.0), (123_456, 0.75)] {
            let exact = f64::from(raw) * f64::from(percent) / 100.0;
            let got = LayoutUnit::from_raw(raw)
                .checked_mul_percent(finite(percent))
                .unwrap()
                .raw();
            assert!((f64::from(got) - exact).abs() <= 0.5, "{raw} × {percent}%");
        }
    }

    #[test]
    fn display_and_error_messages() {
        assert_eq!(LayoutUnit::from_raw(21).to_string(), "0.328125px");
        assert_eq!(LayoutUnit::from_raw(-64).to_string(), "-1px");
        assert!(LayoutUnitError::OutOfRange.to_string().contains("범위"));
        assert!(LayoutUnitError::DivisionByZero
            .to_string()
            .contains("0으로"));
        assert_eq!(SUBPIXELS_PER_PX, 64);
    }
}
