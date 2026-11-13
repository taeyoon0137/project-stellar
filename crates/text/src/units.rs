//! 텍스트 레이아웃 고정소수점 단위 (X.10.1).
//!
//! 레이아웃 crate의 `LayoutUnit`(X.1.5)과 같은 격자인 logical px `1/64` subpixel 정수입니다.
//! `text` crate는 `layout`에 의존할 수 없으므로 같은 규칙의 값 타입을 따로 두고, 레이아웃 연결(X.10.4)은
//! raw 값을 그대로 옮깁니다.
//!
//! - font unit → subpixel 변환은 `값 × 글꼴 크기(subpixel) ÷ units per em`을 `i128` 정수로 계산하고,
//!   가장 가까운 정수로 반올림합니다(중간값은 0에서 먼 쪽). float를 쓰지 않으므로 플랫폼 간 같은 값입니다.
//! - 범위를 넘는 결과는 조용히 saturate하지 않고 오류입니다.

use std::fmt;

/// logical px 하나당 subpixel 수(레이아웃 `SUBPIXELS_PER_PX`와 같음).
pub const SUBPIXELS_PER_PX: i32 = 64;

/// 글꼴 크기 상한(px). 이보다 큰 크기는 거부합니다.
pub const MAX_FONT_SIZE_PX: i32 = 10_000;

/// 단위 계산 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitError {
    /// 결과가 `i32` subpixel 범위를 벗어납니다.
    OutOfRange,
    /// 글꼴 크기가 0 이하이거나 상한을 넘습니다.
    InvalidFontSize,
}

impl fmt::Display for UnitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnitError::OutOfRange => {
                f.write_str("텍스트 레이아웃 수치가 subpixel 범위를 벗어났습니다")
            }
            UnitError::InvalidFontSize => write!(
                f,
                "글꼴 크기는 0보다 크고 {MAX_FONT_SIZE_PX}px 이하여야 합니다"
            ),
        }
    }
}

impl std::error::Error for UnitError {}

/// logical subpixel(`1/64` px) 단위 고정소수점 길이.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextUnit(i32);

impl TextUnit {
    /// 0.
    pub const ZERO: TextUnit = TextUnit(0);
    /// 표현 가능한 최댓값.
    pub const MAX: TextUnit = TextUnit(i32::MAX);

    /// raw subpixel 수로 만듭니다.
    pub const fn from_raw(subpixels: i32) -> TextUnit {
        TextUnit(subpixels)
    }

    /// 정수 px로 만듭니다.
    pub fn from_px(px: i32) -> Result<TextUnit, UnitError> {
        px.checked_mul(SUBPIXELS_PER_PX)
            .map(TextUnit)
            .ok_or(UnitError::OutOfRange)
    }

    /// raw subpixel 수.
    pub const fn raw(self) -> i32 {
        self.0
    }

    /// 덧셈(범위 검사).
    pub fn checked_add(self, other: TextUnit) -> Result<TextUnit, UnitError> {
        self.0
            .checked_add(other.0)
            .map(TextUnit)
            .ok_or(UnitError::OutOfRange)
    }

    /// 뺄셈(범위 검사).
    pub fn checked_sub(self, other: TextUnit) -> Result<TextUnit, UnitError> {
        self.0
            .checked_sub(other.0)
            .map(TextUnit)
            .ok_or(UnitError::OutOfRange)
    }

    /// `i64` 합계를 범위 검사해 변환합니다.
    pub fn from_i64(value: i64) -> Result<TextUnit, UnitError> {
        i32::try_from(value)
            .map(TextUnit)
            .map_err(|_| UnitError::OutOfRange)
    }
}

/// 글꼴 크기(subpixel, `0 < 크기 ≤ MAX_FONT_SIZE_PX`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FontSize(i32);

impl FontSize {
    /// raw subpixel 수로 만듭니다.
    pub fn from_raw(subpixels: i32) -> Result<FontSize, UnitError> {
        if subpixels > 0 && subpixels <= MAX_FONT_SIZE_PX * SUBPIXELS_PER_PX {
            Ok(FontSize(subpixels))
        } else {
            Err(UnitError::InvalidFontSize)
        }
    }

    /// 정수 px로 만듭니다.
    pub fn from_px(px: i32) -> Result<FontSize, UnitError> {
        let raw = px
            .checked_mul(SUBPIXELS_PER_PX)
            .ok_or(UnitError::InvalidFontSize)?;
        FontSize::from_raw(raw)
    }

    /// raw subpixel 수.
    pub const fn raw(self) -> i32 {
        self.0
    }

    /// 크기를 길이로.
    pub const fn to_unit(self) -> TextUnit {
        TextUnit(self.0)
    }

    /// font unit 값을 이 크기의 subpixel로 변환합니다(반올림: 중간값은 0에서 먼 쪽).
    pub fn scale(self, font_units: i64, units_per_em: u16) -> Result<TextUnit, UnitError> {
        let upem = i128::from(units_per_em.max(1));
        let product = i128::from(font_units) * i128::from(self.0);
        let magnitude = (product.unsigned_abs() + (upem as u128) / 2) / upem as u128;
        let signed = if product < 0 {
            -(i128::try_from(magnitude).map_err(|_| UnitError::OutOfRange)?)
        } else {
            i128::try_from(magnitude).map_err(|_| UnitError::OutOfRange)?
        };
        i32::try_from(signed)
            .map(TextUnit)
            .map_err(|_| UnitError::OutOfRange)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_rounds_half_away_from_zero_with_integers() {
        let size = FontSize::from_px(16).expect("16px");
        assert_eq!(size.raw(), 1024);
        // 500 fu @ 16px, upem 1000 → 8px = 512 subpx.
        assert_eq!(size.scale(500, 1000), Ok(TextUnit::from_raw(512)));
        // 1 fu → 1.024 → 1.
        assert_eq!(size.scale(1, 1000), Ok(TextUnit::from_raw(1)));
        // 중간값: 0.5 → 1, -0.5 → -1 (size 1 subpx, upem 2).
        let tiny = FontSize::from_raw(1).expect("1 subpx");
        assert_eq!(tiny.scale(1, 2), Ok(TextUnit::from_raw(1)));
        assert_eq!(tiny.scale(-1, 2), Ok(TextUnit::from_raw(-1)));
        assert_eq!(tiny.scale(1, 3), Ok(TextUnit::from_raw(0)));
        // 범위 초과는 오류.
        let huge = FontSize::from_px(MAX_FONT_SIZE_PX).expect("최대");
        assert_eq!(huge.scale(i64::MAX / 2, 16), Err(UnitError::OutOfRange));
    }

    #[test]
    fn font_size_and_unit_bounds() {
        assert_eq!(FontSize::from_px(0), Err(UnitError::InvalidFontSize));
        assert_eq!(FontSize::from_px(-3), Err(UnitError::InvalidFontSize));
        assert_eq!(
            FontSize::from_px(MAX_FONT_SIZE_PX + 1),
            Err(UnitError::InvalidFontSize)
        );
        assert_eq!(FontSize::from_px(i32::MAX), Err(UnitError::InvalidFontSize));
        assert_eq!(TextUnit::from_px(2).map(TextUnit::raw), Ok(128));
        assert_eq!(TextUnit::from_px(i32::MAX), Err(UnitError::OutOfRange));
        assert_eq!(
            TextUnit::MAX.checked_add(TextUnit::from_raw(1)),
            Err(UnitError::OutOfRange)
        );
        assert_eq!(
            TextUnit::from_raw(5).checked_sub(TextUnit::from_raw(7)),
            Ok(TextUnit::from_raw(-2))
        );
        assert_eq!(
            TextUnit::from_i64(i64::from(i32::MAX) + 1),
            Err(UnitError::OutOfRange)
        );
        assert_eq!(
            FontSize::from_px(16).map(FontSize::to_unit),
            Ok(TextUnit::from_raw(1024))
        );
        assert!(UnitError::InvalidFontSize.to_string().contains("10000"));
    }
}
