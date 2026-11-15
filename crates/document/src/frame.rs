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
use stellar_core_types::property::PropertyPath;
use stellar_core_types::reference::{NodeRef, UnresolvedRef};
use stellar_core_types::value::{FiniteF32, NonFiniteError, Size2, Transform2D};

use crate::node_kind::NodeKind;

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

/// clipping 방식 (VI.3.1, VI.3.4).
///
/// 숫자 tag는 저장 포맷의 일부입니다: 0 = 없음, 1 = frame bounds, 2 = PathRef.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Clipping {
    /// clip하지 않음.
    Disabled,
    /// clip path가 없을 때의 기본: frame bounds로 clip.
    #[default]
    FrameBounds,
    /// 별도 Path node의 geometry로 clip. 대상이 없으면 미해결 진단이 됩니다.
    Path(NodeRef),
}

const CLIP_TAG_DISABLED: u8 = 0;
const CLIP_TAG_FRAME_BOUNDS: u8 = 1;
const CLIP_TAG_PATH: u8 = 2;

/// Frame 속성 key (VI.3.4·VI.3.5 공용). 숫자 값은 저장·진단 경로의 일부입니다.
pub mod keys {
    use stellar_core_types::property::PropertyKey;

    /// 이전 SetProperty와 journal의 폭·높이 묶음.
    pub const SIZE: PropertyKey = PropertyKey(0x0101);
    /// local transform.
    pub const TRANSFORM: PropertyKey = PropertyKey(0x0102);
    /// clipping.
    pub const CLIPPING: PropertyKey = PropertyKey(0x0103);
    /// background.
    pub const BACKGROUND: PropertyKey = PropertyKey(0x0104);
    /// border.
    pub const BORDER: PropertyKey = PropertyKey(0x0105);
    /// 이전 SetProperty와 journal의 네 모서리 묶음.
    pub const CORNER_RADIUS: PropertyKey = PropertyKey(0x0106);
    /// layout mode 참조.
    pub const LAYOUT_MODE: PropertyKey = PropertyKey(0x0107);
    /// 고정 폭 (CSS width).
    pub const WIDTH: PropertyKey = PropertyKey(0x0108);
    /// 고정 높이 (CSS height).
    pub const HEIGHT: PropertyKey = PropertyKey(0x0109);
    /// CSS border-top-left-radius.
    pub const BORDER_TOP_LEFT_RADIUS: PropertyKey = PropertyKey(0x010a);
    /// CSS border-top-right-radius.
    pub const BORDER_TOP_RIGHT_RADIUS: PropertyKey = PropertyKey(0x010b);
    /// CSS border-bottom-right-radius.
    pub const BORDER_BOTTOM_RIGHT_RADIUS: PropertyKey = PropertyKey(0x010c);
    /// CSS border-bottom-left-radius.
    pub const BORDER_BOTTOM_LEFT_RADIUS: PropertyKey = PropertyKey(0x010d);
    /// CSS 순서(좌상, 우상, 우하, 좌하)의 독립 속성.
    pub const BORDER_RADIUS_LONGHANDS: [PropertyKey; 4] = [
        BORDER_TOP_LEFT_RADIUS,
        BORDER_TOP_RIGHT_RADIUS,
        BORDER_BOTTOM_RIGHT_RADIUS,
        BORDER_BOTTOM_LEFT_RADIUS,
    ];
}

/// clip 해석 결과 (VI.3.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipResolution {
    /// clip 없음.
    None,
    /// frame bounds clip.
    FrameBounds,
    /// 해석된 Path node clip.
    Path(NodeRef),
    /// clip 대상이 없음 — invalid node가 아니라 미해결 진단입니다(III.2.4).
    Unresolved(UnresolvedRef),
    /// clip 대상이 Path node가 아닙니다.
    NotAPath {
        /// 참조된 node.
        target: NodeRef,
        /// 실제 종류.
        kind: NodeKind,
    },
}

/// frame의 clip을 해석합니다.
///
/// `lookup`은 StableId로 node 종류를 찾습니다(없으면 `None`). PathRef 대상이
/// 없으면 frame을 거부하지 않고 `PropertyPath(frame, CLIPPING)` 위치의
/// [`UnresolvedRef`]로 보고합니다.
pub fn resolve_clip(
    frame_id: StableId,
    clipping: Clipping,
    lookup: impl Fn(StableId) -> Option<NodeKind>,
) -> ClipResolution {
    match clipping {
        Clipping::Disabled => ClipResolution::None,
        Clipping::FrameBounds => ClipResolution::FrameBounds,
        Clipping::Path(target) => match lookup(target.id()) {
            None => ClipResolution::Unresolved(
                target.unresolved_at(PropertyPath::with_keys(frame_id, vec![keys::CLIPPING])),
            ),
            Some(NodeKind::Path) => ClipResolution::Path(target),
            Some(kind) => ClipResolution::NotAPath { target, kind },
        },
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
    /// 이전 SetProperty와 journal의 네 모서리 묶음.
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
        match self.clipping {
            Clipping::Disabled => buf.push(CLIP_TAG_DISABLED),
            Clipping::FrameBounds => buf.push(CLIP_TAG_FRAME_BOUNDS),
            Clipping::Path(target) => {
                buf.push(CLIP_TAG_PATH);
                buf.extend_from_slice(&target.id().to_bytes());
            }
        }
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

        let clipping = match reader.take_u8()? {
            CLIP_TAG_DISABLED => Clipping::Disabled,
            CLIP_TAG_FRAME_BOUNDS => Clipping::FrameBounds,
            CLIP_TAG_PATH => {
                let mut id = [0u8; 16];
                id.copy_from_slice(reader.take(16)?);
                Clipping::Path(NodeRef::new(StableId::from_bytes(id)))
            }
            _ => {
                return Err(FrameError::Malformed {
                    reason: "알 수 없는 clipping tag",
                })
            }
        };

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

// --- size·반전 정책 (VI.3.3) ---
//
// width/height는 항상 비음수입니다. 시각적 반전(mirror)은 음수 size가 아니라
// 행렬식이 음수인 local transform으로만 표현합니다. 사용자가 반대 방향으로
// 끌어 만든 signed extent는 [`signed_extent_to_size_and_mirror`]로 "양수 size +
// 반전 transform"으로 정규화합니다.

/// signed extent(음수 가능)를 비음수 size와 반전 transform으로 정규화합니다.
///
/// 음수 축은 그 축의 scale -1로 표현되며, 결과 box는 원래 signed extent가
/// 덮던 영역(`[min(0,w), max(0,w)] × [min(0,h), max(0,h)]`)을 그대로 덮습니다.
pub fn signed_extent_to_size_and_mirror(
    width: f32,
    height: f32,
) -> Result<(Size2, Transform2D), FrameError> {
    if !width.is_finite() || !height.is_finite() {
        return Err(FrameError::NonFinite);
    }
    let size = Size2::new(width.abs(), height.abs()).map_err(|_| FrameError::NonFinite)?;
    let sx = if width < 0.0 { -1.0 } else { 1.0 };
    let sy = if height < 0.0 { -1.0 } else { 1.0 };
    let mirror = Transform2D::new(sx, 0.0, 0.0, sy, 0.0, 0.0)?;
    Ok((size, mirror))
}

impl FramePayload {
    /// size를 설정합니다. 음수는 거부합니다 — 반전은 transform으로 표현합니다.
    pub fn set_size(&mut self, width: f32, height: f32) -> Result<(), FrameError> {
        self.size = Size2::new(width, height).map_err(|e| match e {
            stellar_core_types::value::SizeError::NonFinite => FrameError::NonFinite,
            stellar_core_types::value::SizeError::Negative => {
                FrameError::Negative { field: "size" }
            }
        })?;
        Ok(())
    }

    /// local transform이 반전(행렬식 < 0)을 포함하는지.
    pub fn is_mirrored(&self) -> bool {
        self.transform.determinant() < 0.0
    }

    /// frame을 자기 bounds 안에서 좌우 반전합니다. size는 그대로 비음수입니다.
    ///
    /// local 좌표에서 `x → width − x` 반전을 먼저 적용한 뒤 기존 transform을
    /// 적용하므로 부모 좌표계에서 차지하는 영역은 변하지 않습니다.
    pub fn mirror_horizontally(&mut self) -> Result<(), FrameError> {
        let flip = Transform2D::new(-1.0, 0.0, 0.0, 1.0, self.size.width(), 0.0)?;
        self.transform = flip.then(&self.transform)?;
        Ok(())
    }

    /// frame을 자기 bounds 안에서 상하 반전합니다. size는 그대로 비음수입니다.
    pub fn mirror_vertically(&mut self) -> Result<(), FrameError> {
        let flip = Transform2D::new(1.0, 0.0, 0.0, -1.0, 0.0, self.size.height())?;
        self.transform = flip.then(&self.transform)?;
        Ok(())
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

    #[test]
    fn negative_size_is_rejected_but_mirror_transform_is_allowed() {
        // 검증 기준(VI.3.3): 음수 size 거부, mirror transform 허용.
        let mut frame = FramePayload::default();
        assert_eq!(
            frame.set_size(-10.0, 20.0).unwrap_err(),
            FrameError::Negative { field: "size" }
        );
        assert_eq!(
            frame.set_size(f32::NAN, 1.0).unwrap_err(),
            FrameError::NonFinite
        );
        assert_eq!(frame.size, Size2::ZERO, "거부 시 size 불변");

        frame.set_size(100.0, 50.0).unwrap();
        assert!(!frame.is_mirrored());
        frame.mirror_horizontally().unwrap();
        assert!(frame.is_mirrored(), "반전은 transform으로 허용");
        assert_eq!(
            frame.size,
            Size2::new(100.0, 50.0).unwrap(),
            "size는 비음수 유지"
        );

        // 반전 frame도 저장·복원됩니다.
        let decoded = FramePayload::decode(&frame.encode()).unwrap();
        assert!(decoded.is_mirrored());
        assert_eq!(decoded, frame);
    }

    #[test]
    fn mirror_keeps_occupied_region() {
        use stellar_core_types::value::Vec2;
        let mut frame = FramePayload::default();
        frame.set_size(100.0, 50.0).unwrap();
        frame.transform = Transform2D::new(1.0, 0.0, 0.0, 1.0, 30.0, 40.0).unwrap();
        let corners_before: Vec<(f32, f32)> = [(0.0, 0.0), (100.0, 50.0)]
            .iter()
            .map(|(x, y)| {
                let p = frame.transform.apply(Vec2::new(*x, *y).unwrap()).unwrap();
                (p.x(), p.y())
            })
            .collect();
        frame.mirror_horizontally().unwrap();
        // 반전 후 local (0,0)은 오른쪽 끝, (100,50)은 왼쪽 아래 끝으로 — 영역 동일.
        let left_top = frame
            .transform
            .apply(Vec2::new(100.0, 0.0).unwrap())
            .unwrap();
        let right_bottom = frame
            .transform
            .apply(Vec2::new(0.0, 50.0).unwrap())
            .unwrap();
        assert_eq!((left_top.x(), left_top.y()), corners_before[0]);
        assert_eq!((right_bottom.x(), right_bottom.y()), corners_before[1]);
        // 두 번 반전하면 원래 transform.
        frame.mirror_horizontally().unwrap();
        assert_eq!(
            frame.transform,
            Transform2D::new(1.0, 0.0, 0.0, 1.0, 30.0, 40.0).unwrap()
        );
        assert!(!frame.is_mirrored());
    }

    #[test]
    fn signed_extent_normalizes_to_positive_size_and_mirror() {
        let (size, mirror) = signed_extent_to_size_and_mirror(-80.0, 30.0).unwrap();
        assert_eq!(size, Size2::new(80.0, 30.0).unwrap());
        assert!(mirror.determinant() < 0.0);
        let (size, mirror) = signed_extent_to_size_and_mirror(-80.0, -30.0).unwrap();
        assert_eq!(size, Size2::new(80.0, 30.0).unwrap());
        // 두 축 반전은 180° 회전과 같아 행렬식은 양수.
        assert!(mirror.determinant() > 0.0);
        let (_, identity) = signed_extent_to_size_and_mirror(5.0, 5.0).unwrap();
        assert_eq!(identity, Transform2D::identity());
        assert_eq!(
            signed_extent_to_size_and_mirror(f32::INFINITY, 1.0).unwrap_err(),
            FrameError::NonFinite
        );
    }

    #[test]
    fn clip_forms_round_trip() {
        for clipping in [
            Clipping::Disabled,
            Clipping::FrameBounds,
            Clipping::Path(NodeRef::new(stable(77))),
        ] {
            let payload = FramePayload {
                clipping,
                ..FramePayload::default()
            };
            assert_eq!(FramePayload::decode(&payload.encode()).unwrap(), payload);
        }
    }

    #[test]
    fn missing_clip_ref_is_unresolved_diagnostic() {
        // 검증 기준(VI.3.4): missing clip ref는 unresolved diagnostic.
        use stellar_core_types::reference::ExpectedRefType;
        let frame_id = stable(1);
        let missing = NodeRef::new(stable(99));
        let resolution = resolve_clip(frame_id, Clipping::Path(missing), |_| None);
        let ClipResolution::Unresolved(unresolved) = resolution else {
            panic!("미해결 진단이어야 함: {resolution:?}");
        };
        assert_eq!(unresolved.original_id(), stable(99));
        assert_eq!(unresolved.expected(), ExpectedRefType::Node);
        assert_eq!(
            unresolved.location(),
            &PropertyPath::with_keys(frame_id, vec![keys::CLIPPING])
        );
    }

    #[test]
    fn clip_resolution_forms() {
        let frame_id = stable(1);
        let path_node = stable(10);
        let rect_node = stable(11);
        let lookup = |id: StableId| {
            if id == path_node {
                Some(NodeKind::Path)
            } else if id == rect_node {
                Some(NodeKind::Shape)
            } else {
                None
            }
        };
        // clip path 없음 → frame bounds(기본).
        assert_eq!(
            resolve_clip(frame_id, Clipping::default(), lookup),
            ClipResolution::FrameBounds
        );
        assert_eq!(
            resolve_clip(frame_id, Clipping::Disabled, lookup),
            ClipResolution::None
        );
        assert_eq!(
            resolve_clip(frame_id, Clipping::Path(NodeRef::new(path_node)), lookup),
            ClipResolution::Path(NodeRef::new(path_node))
        );
        assert_eq!(
            resolve_clip(frame_id, Clipping::Path(NodeRef::new(rect_node)), lookup),
            ClipResolution::NotAPath {
                target: NodeRef::new(rect_node),
                kind: NodeKind::Shape
            }
        );
    }
}
