//! 색상과 색공간 (III.4.1).
//!
//! 색상은 [`ColorSpace`](sRGB·Display P3)와 함께 **비연관 알파(unassociated/straight alpha)**로
//! 저장합니다. 즉 RGB 채널은 알파로 곱해지지 않은 원본 값입니다. 알파는 `[0, 1]` 범위로
//! 강제하고, 모든 채널은 유한([`FiniteF32`])해야 합니다. RGB 채널 자체는 wide-gamut·확장
//! 범위 저작을 막지 않도록 `[0, 1]`로 제한하지 않습니다(감마/감만 매핑은 렌더 단계 소관).
//!
//! [`Color::to_bytes`] ↔ [`Color::from_bytes`]는 고정 17바이트로 정확히 round-trip 합니다.

use std::fmt;

use crate::value::FiniteF32;

/// 색공간. 저장 tag(`tag()`)는 포맷 계약이므로 값을 변경·재사용하지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ColorSpace {
    /// sRGB.
    Srgb,
    /// Display P3.
    DisplayP3,
}

impl ColorSpace {
    const TAG_SRGB: u8 = 0x01;
    const TAG_DISPLAY_P3: u8 = 0x02;

    /// 안정 numeric tag.
    pub const fn tag(self) -> u8 {
        match self {
            ColorSpace::Srgb => ColorSpace::TAG_SRGB,
            ColorSpace::DisplayP3 => ColorSpace::TAG_DISPLAY_P3,
        }
    }

    /// tag로부터 색공간을 복원합니다. 미지 tag는 `None`.
    pub const fn from_tag(tag: u8) -> Option<ColorSpace> {
        match tag {
            ColorSpace::TAG_SRGB => Some(ColorSpace::Srgb),
            ColorSpace::TAG_DISPLAY_P3 => Some(ColorSpace::DisplayP3),
            _ => None,
        }
    }
}

/// 색상 생성·디코드 오류(III.4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorError {
    /// 채널이 비유한(NaN·±무한대).
    NonFiniteChannel,
    /// 알파가 `[0, 1]` 밖.
    AlphaOutOfRange,
    /// 미지 색공간 tag.
    UnknownColorSpace(u8),
}

impl fmt::Display for ColorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ColorError::NonFiniteChannel => f.write_str("색상 채널이 유한하지 않습니다"),
            ColorError::AlphaOutOfRange => f.write_str("알파는 [0, 1] 범위여야 합니다"),
            ColorError::UnknownColorSpace(tag) => {
                write!(f, "알 수 없는 색공간 tag: 0x{tag:02X}")
            }
        }
    }
}

impl std::error::Error for ColorError {}

/// 색상 값. 색공간 + 비연관 알파 RGBA(각 채널 [`FiniteF32`]). (III.4.1)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Color {
    space: ColorSpace,
    r: FiniteF32,
    g: FiniteF32,
    b: FiniteF32,
    a: FiniteF32,
}

impl Color {
    /// 직렬화 길이(바이트): 색공간 tag 1 + 채널 4×4.
    pub const BYTE_LEN: usize = 1 + 4 * 4;

    /// 색공간과 비연관 알파 RGBA로 색상을 만듭니다.
    ///
    /// 모든 채널은 유한해야 하고, 알파는 `[0, 1]` 범위여야 합니다.
    pub fn new(space: ColorSpace, r: f32, g: f32, b: f32, a: f32) -> Result<Color, ColorError> {
        let r = FiniteF32::new(r).map_err(|_| ColorError::NonFiniteChannel)?;
        let g = FiniteF32::new(g).map_err(|_| ColorError::NonFiniteChannel)?;
        let b = FiniteF32::new(b).map_err(|_| ColorError::NonFiniteChannel)?;
        let a = FiniteF32::new(a).map_err(|_| ColorError::NonFiniteChannel)?;
        if a.get() < 0.0 || a.get() > 1.0 {
            return Err(ColorError::AlphaOutOfRange);
        }
        Ok(Color { space, r, g, b, a })
    }

    /// 색공간.
    pub const fn space(&self) -> ColorSpace {
        self.space
    }

    /// 빨강 채널(비연관).
    pub const fn r(&self) -> f32 {
        self.r.get()
    }

    /// 초록 채널(비연관).
    pub const fn g(&self) -> f32 {
        self.g.get()
    }

    /// 파랑 채널(비연관).
    pub const fn b(&self) -> f32 {
        self.b.get()
    }

    /// 알파(`[0, 1]`).
    pub const fn alpha(&self) -> f32 {
        self.a.get()
    }

    /// 고정 17바이트 직렬화: 색공간 tag ++ r ++ g ++ b ++ a (각 float은 little-endian).
    pub fn to_bytes(&self) -> [u8; Color::BYTE_LEN] {
        let mut out = [0u8; Color::BYTE_LEN];
        out[0] = self.space.tag();
        out[1..5].copy_from_slice(&self.r.to_le_bytes());
        out[5..9].copy_from_slice(&self.g.to_le_bytes());
        out[9..13].copy_from_slice(&self.b.to_le_bytes());
        out[13..17].copy_from_slice(&self.a.to_le_bytes());
        out
    }

    /// [`Color::to_bytes`]의 역. 미지 색공간·비유한 채널·알파 범위 위반을 거부합니다.
    pub fn from_bytes(bytes: [u8; Color::BYTE_LEN]) -> Result<Color, ColorError> {
        let space =
            ColorSpace::from_tag(bytes[0]).ok_or(ColorError::UnknownColorSpace(bytes[0]))?;
        let r = read_channel(&bytes[1..5])?;
        let g = read_channel(&bytes[5..9])?;
        let b = read_channel(&bytes[9..13])?;
        let a = read_channel(&bytes[13..17])?;
        Color::new(space, r, g, b, a)
    }
}

/// little-endian 4바이트를 유한 채널 값으로 읽습니다.
fn read_channel(slice: &[u8]) -> Result<f32, ColorError> {
    let arr = [slice[0], slice[1], slice[2], slice[3]];
    FiniteF32::from_le_bytes(arr)
        .map(|v| v.get())
        .map_err(|_| ColorError::NonFiniteChannel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_space_tag_round_trip() {
        for space in [ColorSpace::Srgb, ColorSpace::DisplayP3] {
            assert_eq!(ColorSpace::from_tag(space.tag()), Some(space));
        }
        assert_eq!(ColorSpace::from_tag(0x00), None);
        assert_eq!(ColorSpace::from_tag(0xFF), None);
    }

    #[test]
    fn color_space_tags_are_stable_golden() {
        // 저장 포맷 계약. 변경 시 기존 파일 호환이 깨집니다.
        assert_eq!(ColorSpace::Srgb.tag(), 0x01);
        assert_eq!(ColorSpace::DisplayP3.tag(), 0x02);
    }

    #[test]
    fn alpha_boundary_fixtures_round_trip() {
        // alpha 0과 1 경계 fixture round-trip.
        for a in [0.0, 1.0] {
            let c = Color::new(ColorSpace::Srgb, 0.5, 0.25, 0.75, a).unwrap();
            assert_eq!(Color::from_bytes(c.to_bytes()).unwrap(), c);
            assert_eq!(c.alpha(), a);
        }
    }

    #[test]
    fn gamut_edge_fixtures_round_trip() {
        // gamut 모서리(채널 0/1) fixture round-trip, sRGB와 P3 모두.
        let edges = [
            (0.0, 0.0, 0.0),
            (1.0, 1.0, 1.0),
            (1.0, 0.0, 0.0),
            (0.0, 1.0, 0.0),
            (0.0, 0.0, 1.0),
        ];
        for space in [ColorSpace::Srgb, ColorSpace::DisplayP3] {
            for (r, g, b) in edges {
                let c = Color::new(space, r, g, b, 1.0).unwrap();
                let restored = Color::from_bytes(c.to_bytes()).unwrap();
                assert_eq!(restored, c);
                assert_eq!((restored.r(), restored.g(), restored.b()), (r, g, b));
            }
        }
    }

    #[test]
    fn extended_range_channels_preserved() {
        // RGB는 [0,1] 밖(확장 범위) 저작도 보존(클램프하지 않음).
        let c = Color::new(ColorSpace::DisplayP3, 1.2, -0.1, 0.0, 0.5).unwrap();
        let restored = Color::from_bytes(c.to_bytes()).unwrap();
        assert_eq!(restored, c);
        assert_eq!(restored.r(), 1.2);
        assert_eq!(restored.g(), -0.1);
    }

    #[test]
    fn rejects_non_finite_channel() {
        assert_eq!(
            Color::new(ColorSpace::Srgb, f32::NAN, 0.0, 0.0, 1.0),
            Err(ColorError::NonFiniteChannel)
        );
        assert_eq!(
            Color::new(ColorSpace::Srgb, 0.0, f32::INFINITY, 0.0, 1.0),
            Err(ColorError::NonFiniteChannel)
        );
    }

    #[test]
    fn rejects_alpha_out_of_range() {
        assert_eq!(
            Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.5),
            Err(ColorError::AlphaOutOfRange)
        );
        assert_eq!(
            Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, -0.001),
            Err(ColorError::AlphaOutOfRange)
        );
    }

    #[test]
    fn from_bytes_rejects_unknown_color_space() {
        let mut bytes = Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0)
            .unwrap()
            .to_bytes();
        bytes[0] = 0x7F; // 미지 색공간 tag.
        assert_eq!(
            Color::from_bytes(bytes),
            Err(ColorError::UnknownColorSpace(0x7F))
        );
    }

    #[test]
    fn from_bytes_rejects_non_finite_bytes() {
        let mut bytes = Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0)
            .unwrap()
            .to_bytes();
        // r 채널을 +Inf 비트 패턴으로 덮어씀.
        bytes[1..5].copy_from_slice(&f32::INFINITY.to_le_bytes());
        assert_eq!(Color::from_bytes(bytes), Err(ColorError::NonFiniteChannel));
    }

    #[test]
    fn negative_zero_channel_normalizes() {
        // -0.0 채널은 +0.0으로 정규화(FiniteF32 계약).
        let c = Color::new(ColorSpace::Srgb, -0.0, 0.0, 0.0, 0.0).unwrap();
        assert_eq!(c.r().to_bits(), 0u32);
    }
}
