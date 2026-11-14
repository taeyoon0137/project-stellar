//! glyph outline/bitmap atlas interface와 key (XI.6.4).
//!
//! 공통 glyph outline·atlas pipeline(아키텍처 §11.1)의 renderer 쪽 계약입니다. text engine(X.9~X.10)이
//! shaping으로 얻은 glyph를 [`GlyphKey`]로 식별하고, worker의 [`GlyphRasterizer`]가 outline 또는
//! bitmap([`GlyphImage`])을 만들며, [`GlyphAtlas`]가 bitmap을 atlas page에 배치합니다.
//!
//! ## key
//!
//! **[`GlyphKey`] = [`FontHash`] + [`GlyphId`] + [`GlyphSize`] + [`VariationCoords`] + [`GlyphRenderMode`]**
//!
//! - **font hash**: 폰트 파일 bytes의 SHA-256 digest와 collection 안 face index. text crate의
//!   `FontFileHash`·face index(X.8.1)와 같은 값입니다. crate 의존 정책(II.2.8)상 renderer-core는 text를
//!   import할 수 없어 같은 의미의 값 타입을 둡니다. 파일 경로·family 이름은 key가 아닙니다.
//! - **glyph id**: 폰트 안 glyph 번호(text `GlyphId`와 같은 `u32`). 문자(code point)가 아닙니다.
//! - **size**: 화면 pixel 단위 em 크기(device scale 포함)를 1/64 px(26.6 고정소수점)로 양자화한 값입니다.
//!   1/64 px 미만 차이는 같은 key입니다.
//! - **variation**: variable font axis 좌표. tag 오름차순으로 정렬하고 같은 tag는 마지막 값을 씁니다
//!   (text `canonical_variations`와 같은 규칙). 순서만 다른 같은 좌표는 같은 key입니다.
//! - **rendering mode**: outline(벡터 path, 큰 글자), coverage bitmap(A8), color bitmap(RGBA8, emoji 등).
//!
//! [`GlyphKey::canonical_bytes`]는 field마다 고정 길이 또는 길이 prefix를 둬서 서로 다른 key가 같은 bytes가
//! 되지 않습니다(단사). 동등성 판정은 field 값 비교이며 [`GlyphKey::digest`]는 로그·fixture용입니다.
//!
//! ## 신뢰되지 않은 입력
//!
//! 폰트 데이터는 외부 입력입니다. variation axis 수([`MAX_VARIATION_AXES`]), glyph 크기
//! ([`MAX_GLYPH_PX`]), bitmap 변 길이([`MAX_GLYPH_BITMAP_DIM`])에 상한을 두고, 생성자가 길이·유한성을
//! 검사해 잘못된 값이 atlas·upload로 넘어가지 않게 합니다.

use std::fmt;
use std::sync::Arc;

use stellar_scene::geometry::PathGeometry;

use crate::digest::fnv1a64;

/// atlas page 할당·eviction (XI.6.5).
pub mod atlas;

pub use atlas::{AtlasConfig, AtlasStats, ShelfGlyphAtlas, TrimOutcome};

/// variation axis 최대 수.
pub const MAX_VARIATION_AXES: usize = 64;

/// glyph em 크기 상한(px).
pub const MAX_GLYPH_PX: f32 = 4096.0;

/// glyph bitmap 한 변 상한(px).
pub const MAX_GLYPH_BITMAP_DIM: u32 = 1024;

/// glyph 계층 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphError {
    /// 크기가 유한한 `(0, MAX_GLYPH_PX]`가 아니거나 1/64 px로 양자화하면 0입니다.
    InvalidSize,
    /// OpenType tag가 출력 가능한 ASCII 4바이트가 아닙니다.
    InvalidTag,
    /// variation 값이 유한하지 않습니다.
    NonFiniteVariation,
    /// variation axis가 너무 많습니다.
    TooManyAxes {
        /// 입력 axis 수.
        count: usize,
    },
    /// bitmap 크기·길이가 잘못되었습니다.
    InvalidBitmap {
        /// 원인.
        reason: &'static str,
    },
    /// rasterizer가 glyph를 만들지 못했습니다.
    Rasterize {
        /// 원인.
        reason: &'static str,
    },
}

impl fmt::Display for GlyphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GlyphError::InvalidSize => write!(
                f,
                "glyph 크기는 1/64 px 이상 {MAX_GLYPH_PX} px 이하의 유한한 값이어야 합니다"
            ),
            GlyphError::InvalidTag => {
                f.write_str("OpenType tag는 출력 가능한 ASCII 4바이트여야 합니다")
            }
            GlyphError::NonFiniteVariation => f.write_str("variation 값이 유한하지 않습니다"),
            GlyphError::TooManyAxes { count } => write!(
                f,
                "variation axis가 너무 많습니다: {count} (상한 {MAX_VARIATION_AXES})"
            ),
            GlyphError::InvalidBitmap { reason } => write!(f, "glyph bitmap 오류: {reason}"),
            GlyphError::Rasterize { reason } => write!(f, "glyph rasterize 실패: {reason}"),
        }
    }
}

impl std::error::Error for GlyphError {}

/// 폰트 face 식별: 파일 SHA-256 digest + collection face index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FontHash {
    file_sha256: [u8; 32],
    face_index: u32,
}

impl FontHash {
    /// 호출자(text·asset 계층)가 계산한 파일 digest와 face index로 만듭니다.
    pub const fn new(file_sha256: [u8; 32], face_index: u32) -> FontHash {
        FontHash {
            file_sha256,
            face_index,
        }
    }

    /// 파일 SHA-256 digest.
    pub const fn file_sha256(&self) -> &[u8; 32] {
        &self.file_sha256
    }

    /// face index.
    pub const fn face_index(&self) -> u32 {
        self.face_index
    }
}

/// 폰트 안 glyph 번호.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GlyphId(pub u32);

impl GlyphId {
    /// `.notdef` glyph.
    pub const NOTDEF: GlyphId = GlyphId(0);
}

/// 1/64 px로 양자화한 em 크기.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GlyphSize {
    sixty_fourths: u32,
}

impl GlyphSize {
    /// 화면 px 크기를 가장 가까운 1/64 px로 양자화합니다.
    pub fn from_px(px: f32) -> Result<GlyphSize, GlyphError> {
        if !px.is_finite() || px <= 0.0 || px > MAX_GLYPH_PX {
            return Err(GlyphError::InvalidSize);
        }
        // px ≤ 4096이므로 × 64 ≤ 262144는 f32에서 정확한 정수 범위입니다.
        let scaled = (px * 64.0).round();
        if scaled < 1.0 {
            return Err(GlyphError::InvalidSize);
        }
        Ok(GlyphSize {
            sixty_fourths: scaled as u32,
        })
    }

    /// 1/64 px 단위 정수로 만듭니다(0과 상한 초과는 거부).
    pub fn from_sixty_fourths(value: u32) -> Result<GlyphSize, GlyphError> {
        if value == 0 || value > (MAX_GLYPH_PX as u32) * 64 {
            return Err(GlyphError::InvalidSize);
        }
        Ok(GlyphSize {
            sixty_fourths: value,
        })
    }

    /// 1/64 px 단위 값.
    pub const fn sixty_fourths(&self) -> u32 {
        self.sixty_fourths
    }

    /// px 값.
    pub fn px(&self) -> f32 {
        self.sixty_fourths as f32 / 64.0
    }
}

/// OpenType tag(출력 가능한 ASCII 4바이트, text `OpenTypeTag`와 같은 규칙).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AxisTag([u8; 4]);

impl AxisTag {
    /// 4바이트 tag를 만듭니다.
    pub fn new(tag: [u8; 4]) -> Result<AxisTag, GlyphError> {
        if tag.iter().all(|b| (0x20..=0x7E).contains(b)) {
            Ok(AxisTag(tag))
        } else {
            Err(GlyphError::InvalidTag)
        }
    }

    /// 정확히 4바이트 ASCII 문자열(예: `"wght"`).
    pub fn parse(tag: &str) -> Result<AxisTag, GlyphError> {
        let bytes: [u8; 4] = tag
            .as_bytes()
            .try_into()
            .map_err(|_| GlyphError::InvalidTag)?;
        AxisTag::new(bytes)
    }

    /// 원본 bytes.
    pub const fn bytes(&self) -> [u8; 4] {
        self.0
    }
}

/// variable font axis 좌표의 정규형(tag 오름차순, tag 중복 없음, `-0.0`→`0.0`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct VariationCoords {
    /// (tag, 값 bits).
    axes: Arc<[(AxisTag, u32)]>,
}

impl VariationCoords {
    /// 기본 instance(좌표 없음).
    pub fn none() -> VariationCoords {
        VariationCoords::default()
    }

    /// 좌표 목록으로 정규형을 만듭니다. 같은 tag는 마지막 값이 우선합니다.
    pub fn new(
        axes: impl IntoIterator<Item = (AxisTag, f32)>,
    ) -> Result<VariationCoords, GlyphError> {
        let mut sorted: Vec<(AxisTag, u32)> = Vec::new();
        let mut count = 0usize;
        for (tag, value) in axes {
            count += 1;
            if count > MAX_VARIATION_AXES * 4 {
                // 중복 tag로 무한히 긴 입력을 막습니다.
                return Err(GlyphError::TooManyAxes { count });
            }
            if !value.is_finite() {
                return Err(GlyphError::NonFiniteVariation);
            }
            let bits = if value == 0.0 { 0 } else { value.to_bits() };
            match sorted.binary_search_by(|(existing, _)| existing.cmp(&tag)) {
                Ok(index) => sorted[index].1 = bits,
                Err(index) => {
                    if sorted.len() >= MAX_VARIATION_AXES {
                        return Err(GlyphError::TooManyAxes {
                            count: sorted.len() + 1,
                        });
                    }
                    sorted.insert(index, (tag, bits));
                }
            }
        }
        Ok(VariationCoords {
            axes: sorted.into(),
        })
    }

    /// axis 수.
    pub fn len(&self) -> usize {
        self.axes.len()
    }

    /// 좌표가 없으면 참입니다.
    pub fn is_empty(&self) -> bool {
        self.axes.is_empty()
    }

    /// (tag, 값) 목록(tag 오름차순).
    pub fn axes(&self) -> impl Iterator<Item = (AxisTag, f32)> + '_ {
        self.axes
            .iter()
            .map(|(tag, bits)| (*tag, f32::from_bits(*bits)))
    }
}

/// glyph rendering 방식.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GlyphRenderMode {
    /// 벡터 outline path(큰 글자, tessellation으로 그림).
    Outline,
    /// 단색 coverage bitmap(A8, 선형 coverage).
    Coverage,
    /// color bitmap(RGBA8, emoji·COLR 등).
    Color,
}

impl GlyphRenderMode {
    /// 안정 numeric tag(Outline=0, Coverage=1, Color=2).
    pub const fn tag(self) -> u8 {
        match self {
            GlyphRenderMode::Outline => 0,
            GlyphRenderMode::Coverage => 1,
            GlyphRenderMode::Color => 2,
        }
    }

    /// 이 mode의 atlas bitmap format. Outline은 atlas를 쓰지 않습니다.
    pub const fn bitmap_format(self) -> Option<GlyphBitmapFormat> {
        match self {
            GlyphRenderMode::Outline => None,
            GlyphRenderMode::Coverage => Some(GlyphBitmapFormat::Coverage8),
            GlyphRenderMode::Color => Some(GlyphBitmapFormat::Rgba8Premultiplied),
        }
    }
}

/// glyph cache key(모듈 문서).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    /// 폰트 face.
    pub font: FontHash,
    /// glyph 번호.
    pub glyph: GlyphId,
    /// 양자화한 크기.
    pub size: GlyphSize,
    /// variation 좌표(정규형).
    pub variations: VariationCoords,
    /// rendering 방식.
    pub mode: GlyphRenderMode,
}

impl GlyphKey {
    /// key 버전 tag(canonical bytes 첫 byte).
    pub const FORMAT_VERSION: u8 = 1;

    /// 단사 canonical bytes.
    ///
    /// `[version u8][file sha256 32][face u32 LE][glyph u32 LE][size u32 LE][mode u8]`
    /// `[axis 수 u16 LE]([tag 4][value bits u32 LE])*`.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(48 + self.variations.len() * 8);
        out.push(GlyphKey::FORMAT_VERSION);
        out.extend_from_slice(&self.font.file_sha256);
        out.extend_from_slice(&self.font.face_index.to_le_bytes());
        out.extend_from_slice(&self.glyph.0.to_le_bytes());
        out.extend_from_slice(&self.size.sixty_fourths.to_le_bytes());
        out.push(self.mode.tag());
        // MAX_VARIATION_AXES(64) ≤ u16::MAX.
        out.extend_from_slice(&(self.variations.len() as u16).to_le_bytes());
        for (tag, bits) in self.variations.axes.iter() {
            out.extend_from_slice(&tag.0);
            out.extend_from_slice(&bits.to_le_bytes());
        }
        out
    }

    /// canonical bytes의 FNV-1a 64(로그·fixture용, 동등성 판정에 쓰지 않음).
    pub fn digest(&self) -> u64 {
        fnv1a64(&self.canonical_bytes())
    }

    /// 같은 glyph 모양(폰트·glyph·variation·mode)인지 비교합니다. 크기만 다를 수 있습니다.
    pub fn same_glyph_shape(&self, other: &GlyphKey) -> bool {
        self.font == other.font
            && self.glyph == other.glyph
            && self.variations == other.variations
            && self.mode == other.mode
    }
}

/// glyph bitmap pixel format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlyphBitmapFormat {
    /// pixel당 1 byte 선형 coverage(atlas texture `R8Unorm`).
    Coverage8,
    /// pixel당 4 byte RGBA, sRGB 인코딩·premultiplied alpha(atlas texture `Rgba8UnormSrgb`).
    Rgba8Premultiplied,
}

impl GlyphBitmapFormat {
    /// pixel당 byte 수.
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            GlyphBitmapFormat::Coverage8 => 1,
            GlyphBitmapFormat::Rgba8Premultiplied => 4,
        }
    }
}

/// rasterize한 glyph bitmap.
///
/// `left`·`top`은 pen 원점에서 bitmap 좌상단까지 px offset입니다(y 아래 방향, `top`이 음수면 baseline 위).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphBitmap {
    width: u32,
    height: u32,
    left: i32,
    top: i32,
    format: GlyphBitmapFormat,
    pixels: Vec<u8>,
}

impl GlyphBitmap {
    /// 크기·길이를 검증해 만듭니다(행 padding 없음).
    pub fn new(
        width: u32,
        height: u32,
        left: i32,
        top: i32,
        format: GlyphBitmapFormat,
        pixels: Vec<u8>,
    ) -> Result<GlyphBitmap, GlyphError> {
        if width == 0 || height == 0 {
            return Err(GlyphError::InvalidBitmap {
                reason: "빈 bitmap은 GlyphImage::Empty로 표현합니다",
            });
        }
        if width > MAX_GLYPH_BITMAP_DIM || height > MAX_GLYPH_BITMAP_DIM {
            return Err(GlyphError::InvalidBitmap {
                reason: "bitmap 변 길이 상한 초과",
            });
        }
        // 두 변 ≤ 1024이므로 넘치지 않습니다.
        let expected = width as usize * height as usize * format.bytes_per_pixel();
        if pixels.len() != expected {
            return Err(GlyphError::InvalidBitmap {
                reason: "pixel 길이가 width × height × bytes per pixel과 다릅니다",
            });
        }
        Ok(GlyphBitmap {
            width,
            height,
            left,
            top,
            format,
            pixels,
        })
    }

    /// 너비.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// 높이.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// pen 원점 기준 좌상단 x offset.
    pub const fn left(&self) -> i32 {
        self.left
    }

    /// pen 원점 기준 좌상단 y offset.
    pub const fn top(&self) -> i32 {
        self.top
    }

    /// format.
    pub const fn format(&self) -> GlyphBitmapFormat {
        self.format
    }

    /// pixel bytes.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

/// rasterizer 출력.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GlyphImage {
    /// 그릴 잉크가 없는 glyph(공백 등). 정상 결과입니다.
    Empty,
    /// atlas에 올릴 bitmap(Coverage·Color mode).
    Bitmap(GlyphBitmap),
    /// pen 원점 기준 px 좌표(y 아래 방향) outline(Outline mode, tessellation 입력).
    Outline(PathGeometry),
}

/// glyph rasterizer interface(worker 전용 작업).
///
/// 구현은 같은 key에 같은 결과를 반환해야 하고(결정론), 손상된 폰트에도 panic하지 않고 오류를 반환해야
/// 합니다. 반환 image는 key의 mode와 맞아야 합니다([`check_image_matches_mode`]).
pub trait GlyphRasterizer: Send + Sync {
    /// key의 glyph를 만듭니다.
    fn rasterize(&self, key: &GlyphKey) -> Result<GlyphImage, GlyphError>;
}

/// rasterizer 출력이 key mode와 맞는지 검사합니다. `Empty`는 모든 mode에 허용됩니다.
pub fn check_image_matches_mode(key: &GlyphKey, image: &GlyphImage) -> Result<(), GlyphError> {
    let ok = match image {
        GlyphImage::Empty => true,
        GlyphImage::Outline(_) => key.mode == GlyphRenderMode::Outline,
        GlyphImage::Bitmap(bitmap) => key.mode.bitmap_format() == Some(bitmap.format()),
    };
    if ok {
        Ok(())
    } else {
        Err(GlyphError::Rasterize {
            reason: "출력 형식이 rendering mode와 다릅니다",
        })
    }
}

/// atlas page 식별.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AtlasPageId(pub u32);

/// atlas 안 glyph 위치.
///
/// page를 비우면(eviction) 그 page의 세대가 올라가므로, 세대가 다른 placement는 더 이상 유효하지
/// 않습니다([`GlyphAtlas::is_valid`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AtlasPlacement {
    /// page.
    pub page: AtlasPageId,
    /// page 세대.
    pub generation: u32,
    /// page 안 x(px).
    pub x: u32,
    /// page 안 y(px).
    pub y: u32,
    /// 너비(px).
    pub width: u32,
    /// 높이(px).
    pub height: u32,
    /// pen 원점 기준 bitmap 좌상단 x offset.
    pub left: i32,
    /// pen 원점 기준 bitmap 좌상단 y offset.
    pub top: i32,
    /// bitmap format.
    pub format: GlyphBitmapFormat,
}

/// atlas 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtlasError {
    /// bitmap이 page보다 큽니다(호출자는 Outline mode로 그려야 합니다).
    TooLarge {
        /// 너비.
        width: u32,
        /// 높이.
        height: u32,
    },
    /// 공간이 없고 비울 수 있는 page도 없습니다(현재 frame·GPU 진행 중 page는 비우지 않음).
    Full,
    /// 설정 값이 잘못되었습니다.
    InvalidConfig {
        /// 원인.
        reason: &'static str,
    },
    /// 열린 frame이 없거나 frame 번호가 증가하지 않았습니다.
    FrameOrder,
    /// bitmap format이 key의 rendering mode와 다릅니다.
    FormatMismatch,
}

impl fmt::Display for AtlasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AtlasError::TooLarge { width, height } => {
                write!(f, "glyph bitmap이 atlas page보다 큽니다: {width}x{height}")
            }
            AtlasError::Full => f.write_str("glyph atlas에 공간이 없습니다"),
            AtlasError::InvalidConfig { reason } => write!(f, "glyph atlas 설정 오류: {reason}"),
            AtlasError::FrameOrder => f.write_str("glyph atlas frame 순서 오류"),
            AtlasError::FormatMismatch => {
                f.write_str("glyph bitmap format이 rendering mode와 다릅니다")
            }
        }
    }
}

impl std::error::Error for AtlasError {}

/// glyph bitmap atlas interface.
///
/// frame 수명: `begin_frame` → (`lookup` | `insert`)* . 현재 frame에서 조회·배치한 glyph는 그 frame이
/// 끝날 때까지 eviction되지 않습니다(XI.6.5).
pub trait GlyphAtlas {
    /// 새 frame을 시작합니다. frame 번호는 엄격히 증가해야 합니다.
    fn begin_frame(&mut self, frame: u64) -> Result<(), AtlasError>;

    /// GPU가 `frame`까지 실행을 끝냈음을 알립니다(그 frame까지만 쓴 page는 재사용 가능).
    fn gpu_completed(&mut self, frame: u64);

    /// key의 배치를 조회하고, 있으면 현재 frame 사용으로 표시합니다.
    fn lookup(&mut self, key: &GlyphKey) -> Option<AtlasPlacement>;

    /// bitmap을 배치하고 현재 frame 사용으로 표시합니다. 같은 key가 있으면 기존 배치를 돌려줍니다.
    fn insert(&mut self, key: GlyphKey, bitmap: &GlyphBitmap)
        -> Result<AtlasPlacement, AtlasError>;

    /// placement가 아직 유효한지(page가 비워지지 않았는지) 확인합니다.
    fn is_valid(&self, placement: &AtlasPlacement) -> bool;
}

#[cfg(test)]
mod tests;
