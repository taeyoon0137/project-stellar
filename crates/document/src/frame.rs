//! Frame payload (VI.3.1).
//!
//! Frame은 자식의 layout containing block과 좌표계를 만드는 유일한 컨테이너
//! (VI.3.2)입니다. payload에는 size, local transform, clipping, background,
//! border, corner radius, layout mode 참조를 담습니다.
//!
//! - size는 [`Size2`](III.3.3)라 음수가 타입 수준에서 불가능합니다(VI.3.3의
//!   "반전은 transform으로" 정책과 짝).
//! - float 필드는 전부 finite 전용 타입을 지나므로 NaN·±Infinity가 저장될 수
//!   없습니다.
//! - binary 인코딩은 canonical 규칙(V.1 R1~R3)을 따릅니다.

use std::fmt;

use stellar_core_types::color::Color;
use stellar_core_types::ids::StableId;
use stellar_core_types::value::{FiniteF32, NonFiniteError, Size2, Transform2D};

/// Frame payload 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// 값이 유한하지 않습니다.
    NonFinite,
    /// 음수가 허용되지 않는 필드입니다(corner radius, border 두께).
    Negative {
        /// 해당 필드.
        field: &'static str,
    },
    /// 입력이 부족하거나 형식이 아닙니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::NonFinite => f.write_str("값이 유한하지 않습니다"),
            FrameError::Negative { field } => write!(f, "{field}는 음수일 수 없습니다"),
            FrameError::Malformed { reason } => write!(f, "Frame binary 형식 위반: {reason}"),
        }
    }
}

impl std::error::Error for FrameError {}

impl From<NonFiniteError> for FrameError {
    fn from(_: NonFiniteError) -> FrameError {
        FrameError::NonFinite
    }
}

/// clipping 방식.
///
/// VI.3.4에서 별도 PathRef clip 형식이 추가됩니다. 숫자 tag는 저장 포맷의
/// 일부입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum Clipping {
    /// clip하지 않음.
    Disabled = 0,
    /// frame bounds로 clip(기본).
    #[default]
    FrameBounds = 1,
}

impl Clipping {
    fn from_u8(value: u8) -> Option<Clipping> {
        match value {
            0 => Some(Clipping::Disabled),
            1 => Some(Clipping::FrameBounds),
            _ => None,
        }
    }
}

/// 개별 corner radius(비음수 finite).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CornerRadius {
    top_left: FiniteF32,
    top_right: FiniteF32,
    bottom_right: FiniteF32,
    bottom_left: FiniteF32,
}

impl CornerRadius {
    /// 모두 0인 radius.
    pub const ZERO: CornerRadius = CornerRadius {
        top_left: FiniteF32::ZERO,
        top_right: FiniteF32::ZERO,
        bottom_right: FiniteF32::ZERO,
        bottom_left: FiniteF32::ZERO,
    };

    /// 비음수 finite radius를 만듭니다.
    pub fn new(
        top_left: f32,
        top_right: f32,
        bottom_right: f32,
        bottom_left: f32,
    ) -> Result<CornerRadius, FrameError> {
        let values = [top_left, top_right, bottom_right, bottom_left];
        if values.iter().any(|v| *v < 0.0) {
            return Err(FrameError::Negative {
                field: "corner radius",
            });
        }
        Ok(CornerRadius {
            top_left: FiniteF32::new(top_left)?,
            top_right: FiniteF32::new(top_right)?,
            bottom_right: FiniteF32::new(bottom_right)?,
            bottom_left: FiniteF32::new(bottom_left)?,
        })
    }

    /// (top_left, top_right, bottom_right, bottom_left).
    pub const fn components(&self) -> (f32, f32, f32, f32) {
        (
            self.top_left.get(),
            self.top_right.get(),
            self.bottom_right.get(),
            self.bottom_left.get(),
        )
    }
}

impl Default for CornerRadius {
    fn default() -> CornerRadius {
        CornerRadius::ZERO
    }
}

/// 테두리(색 + 비음수 두께).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Border {
    /// 테두리 색.
    pub color: Color,
    thickness: FiniteF32,
}

impl Border {
    /// 비음수 finite 두께의 테두리를 만듭니다.
    pub fn new(color: Color, thickness: f32) -> Result<Border, FrameError> {
        if thickness < 0.0 {
            return Err(FrameError::Negative {
                field: "border 두께",
            });
        }
        Ok(Border {
            color,
            thickness: FiniteF32::new(thickness)?,
        })
    }

    /// 두께.
    pub const fn thickness(&self) -> f32 {
        self.thickness.get()
    }
}

/// Frame node payload.
#[derive(Debug, Clone, PartialEq)]
pub struct FramePayload {
    /// 크기(비음수, III.3.3).
    pub size: Size2,
    /// local transform(부모 좌표계 기준).
    pub transform: Transform2D,
    /// clipping 방식.
    pub clipping: Clipping,
    /// 배경 색(없으면 투명).
    pub background: Option<Color>,
    /// 테두리.
    pub border: Option<Border>,
    /// corner radius.
    pub corner_radius: CornerRadius,
    /// layout mode 참조(layout 정의는 layout crate, Phase X). 없으면 자유 배치.
    pub layout_mode: Option<StableId>,
}

impl Default for FramePayload {
    /// 기본 Frame: 크기 0, 항등 변환, bounds clip, 배경·테두리·layout 없음.
    fn default() -> FramePayload {
        FramePayload {
            size: Size2::ZERO,
            transform: Transform2D::identity(),
            clipping: Clipping::default(),
            background: None,
            border: None,
            corner_radius: CornerRadius::ZERO,
            layout_mode: None,
        }
    }
}

// --- binary 인코딩 (canonical 규칙 R1~R3) ---

fn put_f32(buf: &mut Vec<u8>, value: f32) {
    buf.extend_from_slice(&value.to_le_bytes());
}

struct Reader<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], FrameError> {
        if self.input.len() - self.pos < n {
            return Err(FrameError::Malformed {
                reason: "입력 부족",
            });
        }
        let slice = &self.input[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    fn take_u8(&mut self) -> Result<u8, FrameError> {
        Ok(self.take(1)?[0])
    }

    fn take_f32(&mut self) -> Result<f32, FrameError> {
        let bytes: [u8; 4] = self.take(4)?.try_into().expect("4바이트");
        let value = f32::from_le_bytes(bytes);
        if !value.is_finite() {
            return Err(FrameError::NonFinite);
        }
        Ok(value)
    }

    fn expect_end(&self) -> Result<(), FrameError> {
        if self.pos == self.input.len() {
            Ok(())
        } else {
            Err(FrameError::Malformed {
                reason: "trailing bytes",
            })
        }
    }
}

impl FramePayload {
    /// canonical binary bytes로 인코딩합니다.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        put_f32(&mut buf, self.size.width());
        put_f32(&mut buf, self.size.height());
        let (a, b, c, d, tx, ty) = self.transform.components();
        for component in [a, b, c, d, tx, ty] {
            put_f32(&mut buf, component);
        }
        buf.push(self.clipping as u8);
        match &self.background {
            Some(color) => {
                buf.push(1);
                buf.extend_from_slice(&color.to_bytes());
            }
            None => buf.push(0),
        }
        match &self.border {
            Some(border) => {
                buf.push(1);
                buf.extend_from_slice(&border.color.to_bytes());
                put_f32(&mut buf, border.thickness());
            }
            None => buf.push(0),
        }
        let (tl, tr, br, bl) = self.corner_radius.components();
        for radius in [tl, tr, br, bl] {
            put_f32(&mut buf, radius);
        }
        match &self.layout_mode {
            Some(id) => {
                buf.push(1);
                buf.extend_from_slice(&id.to_bytes());
            }
            None => buf.push(0),
        }
        buf
    }

    /// binary bytes에서 디코딩하고 검증합니다.
    pub fn decode(bytes: &[u8]) -> Result<FramePayload, FrameError> {
        let mut reader = Reader {
            input: bytes,
            pos: 0,
        };
        let width = reader.take_f32()?;
        let height = reader.take_f32()?;
        let size = Size2::new(width, height).map_err(|_| FrameError::Negative { field: "size" })?;

        let a = reader.take_f32()?;
        let b = reader.take_f32()?;
        let c = reader.take_f32()?;
        let d = reader.take_f32()?;
        let tx = reader.take_f32()?;
        let ty = reader.take_f32()?;
        let transform = Transform2D::new(a, b, c, d, tx, ty)?;

        let clipping = Clipping::from_u8(reader.take_u8()?).ok_or(FrameError::Malformed {
            reason: "알 수 없는 clipping tag",
        })?;

        let background = match reader.take_u8()? {
            0 => None,
            1 => {
                let bytes: [u8; Color::BYTE_LEN] =
                    reader.take(Color::BYTE_LEN)?.try_into().expect("길이 고정");
                Some(Color::from_bytes(bytes).map_err(|_| FrameError::Malformed {
                    reason: "background 색",
                })?)
            }
            _ => {
                return Err(FrameError::Malformed {
                    reason: "background flag",
                })
            }
        };

        let border = match reader.take_u8()? {
            0 => None,
            1 => {
                let bytes: [u8; Color::BYTE_LEN] =
                    reader.take(Color::BYTE_LEN)?.try_into().expect("길이 고정");
                let color = Color::from_bytes(bytes).map_err(|_| FrameError::Malformed {
                    reason: "border 색",
                })?;
                let thickness = reader.take_f32()?;
                Some(Border::new(color, thickness)?)
            }
            _ => {
                return Err(FrameError::Malformed {
                    reason: "border flag",
                })
            }
        };

        let tl = reader.take_f32()?;
        let tr = reader.take_f32()?;
        let br = reader.take_f32()?;
        let bl = reader.take_f32()?;
        let corner_radius = CornerRadius::new(tl, tr, br, bl)?;

        let layout_mode = match reader.take_u8()? {
            0 => None,
            1 => {
                let mut id = [0u8; 16];
                id.copy_from_slice(reader.take(16)?);
                Some(StableId::from_bytes(id))
            }
            _ => {
                return Err(FrameError::Malformed {
                    reason: "layout flag",
                })
            }
        };

        reader.expect_end()?;
        Ok(FramePayload {
            size,
            transform,
            clipping,
            background,
            border,
            corner_radius,
            layout_mode,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::color::ColorSpace;

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    #[test]
    fn default_payload_round_trips() {
        // 검증 기준(VI.3.1): 기본값 round-trip.
        let payload = FramePayload::default();
        let bytes = payload.encode();
        let decoded = FramePayload::decode(&bytes).unwrap();
        assert_eq!(decoded, payload);
        assert_eq!(decoded.encode(), bytes, "재인코딩 byte-identical");
        // 기본값 확인.
        assert_eq!(decoded.size, Size2::ZERO);
        assert_eq!(decoded.transform, Transform2D::identity());
        assert_eq!(decoded.clipping, Clipping::FrameBounds);
        assert!(decoded.background.is_none());
        assert!(decoded.border.is_none());
        assert_eq!(decoded.corner_radius, CornerRadius::ZERO);
        assert!(decoded.layout_mode.is_none());
    }

    #[test]
    fn full_payload_round_trips() {
        let payload = FramePayload {
            size: Size2::new(320.0, 240.0).unwrap(),
            transform: Transform2D::new(0.0, 1.0, -1.0, 0.0, 10.0, -5.5).unwrap(),
            clipping: Clipping::Disabled,
            background: Some(Color::new(ColorSpace::Srgb, 0.1, 0.2, 0.3, 1.0).unwrap()),
            border: Some(
                Border::new(
                    Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 0.5).unwrap(),
                    2.0,
                )
                .unwrap(),
            ),
            corner_radius: CornerRadius::new(4.0, 8.0, 12.0, 16.0).unwrap(),
            layout_mode: Some(stable(9)),
        };
        let bytes = payload.encode();
        assert_eq!(FramePayload::decode(&bytes).unwrap(), payload);
    }

    #[test]
    fn negative_radius_and_thickness_are_rejected() {
        assert_eq!(
            CornerRadius::new(-1.0, 0.0, 0.0, 0.0).unwrap_err(),
            FrameError::Negative {
                field: "corner radius"
            }
        );
        let color = Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0).unwrap();
        assert_eq!(
            Border::new(color, -0.5).unwrap_err(),
            FrameError::Negative {
                field: "border 두께"
            }
        );
    }

    #[test]
    fn malformed_bytes_are_rejected() {
        let bytes = FramePayload::default().encode();
        for cut in [0, 4, 8, 32, bytes.len() - 1] {
            assert!(FramePayload::decode(&bytes[..cut]).is_err(), "절단 {cut}");
        }
        let mut extended = bytes.clone();
        extended.push(0);
        assert!(FramePayload::decode(&extended).is_err());
        // NaN size.
        let mut nan = bytes.clone();
        nan[..4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(
            FramePayload::decode(&nan).unwrap_err(),
            FrameError::NonFinite
        );
        // 음수 size.
        let mut negative = bytes.clone();
        negative[..4].copy_from_slice(&(-1.0f32).to_le_bytes());
        assert!(matches!(
            FramePayload::decode(&negative).unwrap_err(),
            FrameError::Negative { .. }
        ));
        // 알 수 없는 clipping tag.
        let mut bad_clip = bytes;
        bad_clip[32] = 9;
        assert!(FramePayload::decode(&bad_clip).is_err());
    }
}
