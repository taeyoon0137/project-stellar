//! 그림자와 blur parameter (III.4.4).
//!
//! [`BlurRadius`]는 blur·확산 등에 쓰이는 radius parameter로, **음수와 비유한 값을 거부**하고
//! `0`은 허용합니다(`-0.0`은 `0.0`으로 정규화). [`Shadow`]는 offset([`Vec2`], 음수 허용) +
//! [`BlurRadius`] + [`Color`]로 구성되며 고정 29바이트로 round-trip 합니다.

use std::fmt;

use crate::color::{Color, ColorError};
use crate::value::{FiniteF32, Vec2};

/// blur radius parameter 생성 오류(III.4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlurError {
    /// 비유한(NaN·±무한대).
    NonFinite,
    /// 음수 radius(정책상 불허).
    Negative,
}

impl fmt::Display for BlurError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BlurError::NonFinite => f.write_str("radius가 유한하지 않습니다"),
            BlurError::Negative => f.write_str("radius는 음수일 수 없습니다"),
        }
    }
}

impl std::error::Error for BlurError {}

/// blur·확산 radius parameter. 유한·비음수만 허용합니다. (III.4.4)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlurRadius(FiniteF32);

impl BlurRadius {
    /// 0 radius.
    pub const ZERO: BlurRadius = BlurRadius(FiniteF32::ZERO);

    /// 유한·비음수 radius를 만듭니다. 비유한은 `NonFinite`, 음수는 `Negative`로 거부합니다.
    pub fn new(value: f32) -> Result<BlurRadius, BlurError> {
        let v = FiniteF32::new(value).map_err(|_| BlurError::NonFinite)?;
        if v.get() < 0.0 {
            return Err(BlurError::Negative);
        }
        Ok(BlurRadius(v))
    }

    /// radius 값.
    pub const fn get(&self) -> f32 {
        self.0.get()
    }

    /// little-endian 4바이트.
    pub fn to_le_bytes(&self) -> [u8; 4] {
        self.0.to_le_bytes()
    }
}

/// 그림자 생성·디코드 오류(III.4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowError {
    /// blur radius 오류.
    Blur(BlurError),
    /// offset이 비유한.
    NonFiniteOffset,
    /// 색상 디코드 실패.
    Color(ColorError),
}

impl fmt::Display for ShadowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShadowError::Blur(err) => write!(f, "blur radius 오류: {err}"),
            ShadowError::NonFiniteOffset => f.write_str("offset이 유한하지 않습니다"),
            ShadowError::Color(err) => write!(f, "그림자 색상 디코드 실패: {err}"),
        }
    }
}

impl std::error::Error for ShadowError {}

/// 그림자: offset + blur radius + 색상. (III.4.4)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Shadow {
    offset: Vec2,
    blur: BlurRadius,
    color: Color,
}

impl Shadow {
    /// 직렬화 길이(바이트): offset 8 + blur 4 + color 17.
    pub const BYTE_LEN: usize = 8 + 4 + Color::BYTE_LEN;

    /// 검증된 구성요소로 그림자를 만듭니다. offset은 음수 허용, blur는 비음수입니다.
    pub const fn new(offset: Vec2, blur: BlurRadius, color: Color) -> Shadow {
        Shadow {
            offset,
            blur,
            color,
        }
    }

    /// offset(음수 허용).
    pub const fn offset(&self) -> Vec2 {
        self.offset
    }

    /// blur radius.
    pub const fn blur(&self) -> BlurRadius {
        self.blur
    }

    /// 색상.
    pub const fn color(&self) -> Color {
        self.color
    }

    /// 고정 29바이트 직렬화: offset.x(LE 4) ++ offset.y(LE 4) ++ blur(LE 4) ++ color(17).
    pub fn to_bytes(&self) -> [u8; Shadow::BYTE_LEN] {
        let mut out = [0u8; Shadow::BYTE_LEN];
        out[0..4].copy_from_slice(&self.offset.x().to_le_bytes());
        out[4..8].copy_from_slice(&self.offset.y().to_le_bytes());
        out[8..12].copy_from_slice(&self.blur.to_le_bytes());
        out[12..29].copy_from_slice(&self.color.to_bytes());
        out
    }

    /// [`Shadow::to_bytes`]의 역. 비유한 offset, 음수·비유한 blur, 잘못된 색상을 거부합니다.
    pub fn from_bytes(bytes: [u8; Shadow::BYTE_LEN]) -> Result<Shadow, ShadowError> {
        let x = read_finite(&bytes[0..4]).ok_or(ShadowError::NonFiniteOffset)?;
        let y = read_finite(&bytes[4..8]).ok_or(ShadowError::NonFiniteOffset)?;
        let offset = Vec2::new(x, y).map_err(|_| ShadowError::NonFiniteOffset)?;

        let blur_value =
            read_finite(&bytes[8..12]).ok_or(ShadowError::Blur(BlurError::NonFinite))?;
        let blur = BlurRadius::new(blur_value).map_err(ShadowError::Blur)?;

        let mut color_bytes = [0u8; Color::BYTE_LEN];
        color_bytes.copy_from_slice(&bytes[12..29]);
        let color = Color::from_bytes(color_bytes).map_err(ShadowError::Color)?;

        Ok(Shadow {
            offset,
            blur,
            color,
        })
    }
}

/// little-endian 4바이트를 유한 `f32`로 읽습니다(비유한은 `None`).
fn read_finite(slice: &[u8]) -> Option<f32> {
    let arr = [slice[0], slice[1], slice[2], slice[3]];
    FiniteF32::from_le_bytes(arr).ok().map(|v| v.get())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::ColorSpace;

    fn black() -> Color {
        Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0).unwrap()
    }

    #[test]
    fn blur_radius_boundary_zero_allowed() {
        assert_eq!(BlurRadius::new(0.0).unwrap().get(), 0.0);
        // -0.0은 +0.0으로 정규화되어 허용.
        assert_eq!(BlurRadius::new(-0.0).unwrap().get(), 0.0);
        assert_eq!(BlurRadius::ZERO.get(), 0.0);
    }

    #[test]
    fn blur_radius_rejects_negative_and_non_finite() {
        assert_eq!(BlurRadius::new(-0.001), Err(BlurError::Negative));
        assert_eq!(BlurRadius::new(-1.0), Err(BlurError::Negative));
        assert_eq!(BlurRadius::new(f32::NAN), Err(BlurError::NonFinite));
        assert_eq!(BlurRadius::new(f32::INFINITY), Err(BlurError::NonFinite));
    }

    #[test]
    fn shadow_round_trips_with_negative_offset() {
        let shadow = Shadow::new(
            Vec2::new(-4.0, 3.0).unwrap(),
            BlurRadius::new(8.0).unwrap(),
            black(),
        );
        assert_eq!(Shadow::from_bytes(shadow.to_bytes()).unwrap(), shadow);
        assert_eq!((shadow.offset().x(), shadow.offset().y()), (-4.0, 3.0));
        assert_eq!(shadow.blur().get(), 8.0);
    }

    #[test]
    fn shadow_zero_blur_round_trips() {
        let shadow = Shadow::new(Vec2::ZERO, BlurRadius::ZERO, black());
        assert_eq!(Shadow::from_bytes(shadow.to_bytes()).unwrap(), shadow);
    }

    #[test]
    fn from_bytes_rejects_negative_blur() {
        let shadow = Shadow::new(Vec2::ZERO, BlurRadius::new(2.0).unwrap(), black());
        let mut bytes = shadow.to_bytes();
        // blur 바이트(8..12)를 음수로 덮어씀.
        bytes[8..12].copy_from_slice(&(-2.0f32).to_le_bytes());
        assert_eq!(
            Shadow::from_bytes(bytes),
            Err(ShadowError::Blur(BlurError::Negative))
        );
    }

    #[test]
    fn from_bytes_rejects_non_finite_blur_and_offset() {
        let shadow = Shadow::new(Vec2::ZERO, BlurRadius::new(1.0).unwrap(), black());

        let mut blur_inf = shadow.to_bytes();
        blur_inf[8..12].copy_from_slice(&f32::INFINITY.to_le_bytes());
        assert_eq!(
            Shadow::from_bytes(blur_inf),
            Err(ShadowError::Blur(BlurError::NonFinite))
        );

        let mut offset_nan = shadow.to_bytes();
        offset_nan[0..4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(
            Shadow::from_bytes(offset_nan),
            Err(ShadowError::NonFiniteOffset)
        );
    }

    #[test]
    fn from_bytes_rejects_invalid_color() {
        let shadow = Shadow::new(Vec2::ZERO, BlurRadius::ZERO, black());
        let mut bytes = shadow.to_bytes();
        // color 색공간 tag(offset 12)를 미지 값으로 덮어씀.
        bytes[12] = 0x00;
        assert_eq!(
            Shadow::from_bytes(bytes),
            Err(ShadowError::Color(ColorError::UnknownColorSpace(0x00)))
        );
    }
}
