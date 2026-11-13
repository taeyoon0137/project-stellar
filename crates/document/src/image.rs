//! Image payload (VI.8).
//!
//! 이미지 node는 asset을 [`AssetRef`](III.2.3)로 참조하고, 배치 상자(size), fit
//! 방식, 정규화 crop 영역, sampling 방식, 선택적 색공간 override를 가집니다.

use std::fmt;

use stellar_core_types::color::ColorSpace;
use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_core_types::reference::{AssetRef, UnresolvedRef};
use stellar_core_types::value::{FiniteF32, Rect, Size2, Vec2};

/// Image payload 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageError {
    /// 값이 유한하지 않습니다.
    NonFinite,
    /// 입력이 부족하거나 형식이 아닙니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageError::NonFinite => f.write_str("이미지 값이 유한하지 않습니다"),
            ImageError::Malformed { reason } => write!(f, "Image binary 형식 위반: {reason}"),
        }
    }
}

impl std::error::Error for ImageError {}

/// 상자 안 배치 방식. tag는 저장 포맷의 일부입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum FitMode {
    /// 비율 무시하고 상자를 채움.
    Fill = 0,
    /// 비율 유지, 전체가 보이게(기본).
    #[default]
    Contain = 1,
    /// 비율 유지, 상자를 덮게(넘치는 부분 잘림).
    Cover = 2,
    /// 원본 크기.
    None = 3,
}

/// 확대·축소 sampling. tag는 저장 포맷의 일부입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum SamplingMode {
    /// 선형 보간(기본).
    #[default]
    Linear = 0,
    /// 최근접(픽셀 아트).
    Nearest = 1,
}

/// 원본 이미지 기준 정규화(0..1) crop 영역 `(x, y, width, height)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CropRect {
    x: FiniteF32,
    y: FiniteF32,
    width: FiniteF32,
    height: FiniteF32,
}

impl CropRect {
    /// 원본 전체(crop 없음).
    pub const FULL: CropRect = CropRect {
        x: FiniteF32::ZERO,
        y: FiniteF32::ZERO,
        width: FiniteF32::ONE,
        height: FiniteF32::ONE,
    };

    /// 유한 값으로 만듭니다(범위 검증은 VI.8.3).
    pub fn from_components(
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Result<CropRect, ImageError> {
        let f = |v: f32| FiniteF32::new(v).map_err(|_| ImageError::NonFinite);
        Ok(CropRect {
            x: f(x)?,
            y: f(y)?,
            width: f(width)?,
            height: f(height)?,
        })
    }

    /// `(x, y, width, height)`.
    pub fn components(&self) -> (f32, f32, f32, f32) {
        (
            self.x.get(),
            self.y.get(),
            self.width.get(),
            self.height.get(),
        )
    }
}

/// Image node payload (VI.8.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePayload {
    /// 참조하는 asset.
    pub asset: AssetRef,
    /// 배치 상자 크기. asset이 없어도 유지되는 placeholder bounds입니다(VI.8.2).
    pub size: Size2,
    /// fit 방식.
    pub fit: FitMode,
    /// crop 영역.
    pub crop: CropRect,
    /// sampling 방식.
    pub sampling: SamplingMode,
    /// asset 고유 색공간 대신 사용할 색공간. 없으면 asset metadata를 따름.
    pub color_space_override: Option<ColorSpace>,
}

const COLOR_SPACE_NONE: u8 = 0;

impl ImagePayload {
    /// 새 asset에 대한 기본 payload(Contain, 전체, Linear, override 없음).
    pub fn new(asset: AssetRef, size: Size2) -> ImagePayload {
        ImagePayload {
            asset,
            size,
            fit: FitMode::default(),
            crop: CropRect::FULL,
            sampling: SamplingMode::default(),
            color_space_override: None,
        }
    }

    /// canonical bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&self.asset.id().to_bytes());
        buf.extend_from_slice(&self.size.width().to_le_bytes());
        buf.extend_from_slice(&self.size.height().to_le_bytes());
        buf.push(self.fit as u8);
        let (x, y, w, h) = self.crop.components();
        for v in [x, y, w, h] {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        buf.push(self.sampling as u8);
        buf.push(
            self.color_space_override
                .map_or(COLOR_SPACE_NONE, |s| s.tag()),
        );
        buf
    }

    /// bytes에서 복원합니다.
    pub fn decode(bytes: &[u8]) -> Result<ImagePayload, ImageError> {
        const LEN: usize = 16 + 8 + 1 + 16 + 1 + 1;
        if bytes.len() != LEN {
            return Err(ImageError::Malformed {
                reason: "길이 불일치",
            });
        }
        let f = |i: usize| f32::from_le_bytes(bytes[i..i + 4].try_into().expect("4바이트"));
        let mut id = [0u8; 16];
        id.copy_from_slice(&bytes[..16]);
        let size = Size2::new(f(16), f(20)).map_err(|_| ImageError::Malformed {
            reason: "size 음수·비유한",
        })?;
        let fit = match bytes[24] {
            0 => FitMode::Fill,
            1 => FitMode::Contain,
            2 => FitMode::Cover,
            3 => FitMode::None,
            _ => {
                return Err(ImageError::Malformed {
                    reason: "알 수 없는 fit tag",
                })
            }
        };
        let crop = CropRect::from_components(f(25), f(29), f(33), f(37))?;
        let sampling = match bytes[41] {
            0 => SamplingMode::Linear,
            1 => SamplingMode::Nearest,
            _ => {
                return Err(ImageError::Malformed {
                    reason: "알 수 없는 sampling tag",
                })
            }
        };
        let color_space_override = match bytes[42] {
            COLOR_SPACE_NONE => None,
            tag => Some(ColorSpace::from_tag(tag).ok_or(ImageError::Malformed {
                reason: "알 수 없는 색공간 tag",
            })?),
        };
        Ok(ImagePayload {
            asset: AssetRef::new(StableId::from_bytes(id)),
            size,
            fit,
            crop,
            sampling,
            color_space_override,
        })
    }
}

// --- 누락 asset (VI.8.2) ---

/// Image 속성 key: asset 참조(진단 위치).
pub const IMAGE_PROP_ASSET: PropertyKey = PropertyKey(0x0601);

/// asset 해석 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageResolution {
    /// asset이 있어 그릴 수 있음.
    Ready(AssetRef),
    /// asset이 없음 — node는 **유효**하며, 이는 missing resource 진단입니다.
    /// 렌더러는 placeholder를 같은 bounds에 그립니다.
    MissingResource(UnresolvedRef),
}

impl ImagePayload {
    /// asset 존재 여부를 해석합니다. `asset_exists`는 asset store 조회입니다.
    ///
    /// 누락은 invalid node가 아니라 `PropertyPath(image, ASSET)` 위치의 미해결 참조
    /// 진단(III.2.4)입니다.
    pub fn resolve_asset(
        &self,
        image_id: StableId,
        asset_exists: impl Fn(StableId) -> bool,
    ) -> ImageResolution {
        if asset_exists(self.asset.id()) {
            ImageResolution::Ready(self.asset)
        } else {
            ImageResolution::MissingResource(
                self.asset
                    .unresolved_at(PropertyPath::with_keys(image_id, vec![IMAGE_PROP_ASSET])),
            )
        }
    }

    /// layout bounds(shape local 원점 기준). asset 존재 여부와 **무관**하게 payload의
    /// 배치 상자 크기에서만 계산하므로 asset이 사라져도 layout이 흔들리지 않습니다.
    pub fn layout_bounds(&self) -> Rect {
        Rect::new(Vec2::ZERO, self.size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(seed: u64) -> AssetRef {
        AssetRef::new(StableId::new_v7(seed, 0, seed))
    }

    #[test]
    fn default_and_full_payloads_round_trip() {
        // 검증 기준(VI.8.1): round-trip.
        let default = ImagePayload::new(asset(1), Size2::new(320.0, 200.0).unwrap());
        assert_eq!(ImagePayload::decode(&default.encode()).unwrap(), default);
        assert_eq!(default.fit, FitMode::Contain);
        assert_eq!(default.crop, CropRect::FULL);

        let full = ImagePayload {
            asset: asset(2),
            size: Size2::new(64.0, 64.0).unwrap(),
            fit: FitMode::Cover,
            crop: CropRect::from_components(0.25, 0.1, 0.5, 0.8).unwrap(),
            sampling: SamplingMode::Nearest,
            color_space_override: Some(ColorSpace::DisplayP3),
        };
        let bytes = full.encode();
        assert_eq!(ImagePayload::decode(&bytes).unwrap(), full);
        assert_eq!(ImagePayload::decode(&bytes).unwrap().encode(), bytes);
    }

    #[test]
    fn malformed_bytes_are_rejected() {
        let bytes = ImagePayload::new(asset(1), Size2::new(1.0, 1.0).unwrap()).encode();
        assert!(ImagePayload::decode(&bytes[..bytes.len() - 1]).is_err());
        let mut bad_fit = bytes.clone();
        bad_fit[24] = 9;
        assert!(ImagePayload::decode(&bad_fit).is_err());
        let mut bad_space = bytes.clone();
        bad_space[42] = 0x7F;
        assert!(ImagePayload::decode(&bad_space).is_err());
        let mut nan = bytes;
        nan[25..29].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(
            ImagePayload::decode(&nan).unwrap_err(),
            ImageError::NonFinite
        );
    }

    #[test]
    fn missing_asset_keeps_layout_bounds() {
        // 검증 기준(VI.8.2): 에셋 누락 전후 layout bounds 동일.
        use stellar_core_types::reference::ExpectedRefType;
        let image_id = StableId::new_v7(9, 0, 9);
        let payload = ImagePayload::new(asset(1), Size2::new(320.0, 180.0).unwrap());

        let present = payload.resolve_asset(image_id, |_| true);
        assert_eq!(present, ImageResolution::Ready(asset(1)));
        let bounds_present = payload.layout_bounds();

        let missing = payload.resolve_asset(image_id, |_| false);
        let ImageResolution::MissingResource(unresolved) = missing else {
            panic!("missing resource 진단이어야 함");
        };
        assert_eq!(unresolved.original_id(), asset(1).id());
        assert_eq!(unresolved.expected(), ExpectedRefType::Asset);
        assert_eq!(
            unresolved.location(),
            &PropertyPath::with_keys(image_id, vec![IMAGE_PROP_ASSET])
        );
        // node는 유효하게 남고 bounds는 그대로.
        assert_eq!(payload.layout_bounds(), bounds_present);
        assert_eq!(bounds_present.size(), Size2::new(320.0, 180.0).unwrap());
        assert!(ImagePayload::decode(&payload.encode()).is_ok());
    }
}
