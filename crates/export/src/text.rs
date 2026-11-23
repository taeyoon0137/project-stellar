//! 내보내기 텍스트 glyph 입력 (XX.3.2).
//!
//! scene의 Text는 text layout cache key([`TextLayoutKey`])만 들고 있습니다(XI.1). 내보내기는 그 key를
//! **고정 font revision**([`crate::capture::FontRevision`])에서 host에게 물어 glyph run([`TextGlyphLayout`])을
//! 얻고, glyph마다 outline([`GlyphOutline`])을 받아 벡터로 씁니다. host는 [`ExportTextSource`]를 구현합니다.
//!
//! ## glyph outline key
//!
//! [`GlyphOutlineKey`] = 폰트 face + glyph id + 크기 + variation 좌표입니다. renderer glyph key(XI.6.4)의
//! Outline mode와 **같은 의미의 값**입니다(export는 crate 의존 정책상 renderer-core에 의존하지 않습니다,
//! ADR-0019). 그래서 host는 renderer의 outline rasterizer(`GlyphRenderMode::Outline`)에 그대로 위임할 수 있습니다.
//!
//! - 폰트 face: 파일 SHA-256 + collection face index(text `FontFileHash`·face index, renderer `FontHash`).
//! - 크기: em 크기를 1/64 px(26.6 고정소수점)로 양자화한 값, `(0, 4096]` px.
//! - variation: OpenType axis 좌표. tag 오름차순, 같은 tag는 마지막 값, `-0.0`→`0.0`(text
//!   `canonical_variations`, renderer `VariationCoords`와 같은 규칙). shaping에 쓴 좌표(X.9.6)를 run에 그대로
//!   두므로 variable font의 outline이 shaping 결과와 같은 instance입니다.
//!
//! ## 좌표
//!
//! glyph pen 원점([`PlacedGlyph`])은 text local 좌표(layout 상자 좌상단 기준, y 아래 방향)의 1/64 px 정수입니다
//! (text `TextUnit`과 같은 격자). outline은 pen 원점 기준 px 좌표(y 아래 방향)이며 renderer outline
//! (`GlyphImage::Outline`)과 같은 규약입니다.

use std::fmt;
use std::sync::Arc;

use stellar_core_types::color::Color;
use stellar_scene::geometry::PathGeometry;
use stellar_scene::node::TextLayoutKey;

use crate::capture::FontRevision;

/// glyph em 크기 상한(px, renderer XI.6.4와 같음).
pub const MAX_GLYPH_PX: f32 = 4096.0;

/// variation axis 최대 수(renderer XI.6.4와 같음).
pub const MAX_VARIATION_AXES: usize = 64;

/// glyph 입력 값 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphValueError {
    /// 크기가 유한한 `(0, MAX_GLYPH_PX]`가 아니거나 1/64 px로 양자화하면 0입니다.
    InvalidSize,
    /// OpenType tag가 출력 가능한 ASCII 4바이트가 아닙니다.
    InvalidTag,
    /// variation 값이 유한하지 않습니다.
    NonFiniteVariation,
    /// variation axis가 너무 많습니다.
    TooManyAxes,
}

impl fmt::Display for GlyphValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GlyphValueError::InvalidSize => write!(
                f,
                "glyph 크기는 1/64 px 이상 {MAX_GLYPH_PX} px 이하의 유한한 값이어야 합니다"
            ),
            GlyphValueError::InvalidTag => {
                f.write_str("OpenType tag는 출력 가능한 ASCII 4바이트여야 합니다")
            }
            GlyphValueError::NonFiniteVariation => f.write_str("variation 값이 유한하지 않습니다"),
            GlyphValueError::TooManyAxes => write!(
                f,
                "variation axis가 너무 많습니다(상한 {MAX_VARIATION_AXES})"
            ),
        }
    }
}

impl std::error::Error for GlyphValueError {}

/// 폰트 face 식별: 파일 SHA-256 digest + collection face index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExportFontFace {
    file_sha256: [u8; 32],
    face_index: u32,
}

impl ExportFontFace {
    /// host(text·asset 계층)가 계산한 파일 digest와 face index로 만듭니다.
    pub const fn new(file_sha256: [u8; 32], face_index: u32) -> ExportFontFace {
        ExportFontFace {
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

impl fmt::Display for ExportFontFace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("sha256:")?;
        for byte in &self.file_sha256 {
            write!(f, "{byte:02x}")?;
        }
        write!(f, "#{}", self.face_index)
    }
}

/// 1/64 px로 양자화한 em 크기.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GlyphSize {
    sixty_fourths: u32,
}

impl GlyphSize {
    /// px 크기를 가장 가까운 1/64 px로 양자화합니다.
    pub fn from_px(px: f32) -> Result<GlyphSize, GlyphValueError> {
        if !px.is_finite() || px <= 0.0 || px > MAX_GLYPH_PX {
            return Err(GlyphValueError::InvalidSize);
        }
        // px ≤ 4096이므로 × 64 ≤ 262144는 f32에서 정확한 정수 범위입니다.
        let scaled = (px * 64.0).round();
        if scaled < 1.0 {
            return Err(GlyphValueError::InvalidSize);
        }
        Ok(GlyphSize {
            sixty_fourths: scaled as u32,
        })
    }

    /// 1/64 px 단위 정수로 만듭니다(0과 상한 초과는 거부). text `TextUnit` raw 값을 그대로 넘길 수 있습니다.
    pub fn from_sixty_fourths(value: u32) -> Result<GlyphSize, GlyphValueError> {
        if value == 0 || value > (MAX_GLYPH_PX as u32) * 64 {
            return Err(GlyphValueError::InvalidSize);
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

/// OpenType variation axis tag(출력 가능한 ASCII 4바이트).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AxisTag([u8; 4]);

impl AxisTag {
    /// 4바이트 tag를 만듭니다.
    pub fn new(tag: [u8; 4]) -> Result<AxisTag, GlyphValueError> {
        if tag.iter().all(|b| (0x20..=0x7E).contains(b)) {
            Ok(AxisTag(tag))
        } else {
            Err(GlyphValueError::InvalidTag)
        }
    }

    /// 정확히 4바이트 ASCII 문자열(예: `"wght"`).
    pub fn parse(tag: &str) -> Result<AxisTag, GlyphValueError> {
        let bytes: [u8; 4] = tag
            .as_bytes()
            .try_into()
            .map_err(|_| GlyphValueError::InvalidTag)?;
        AxisTag::new(bytes)
    }

    /// 원본 bytes.
    pub const fn bytes(&self) -> [u8; 4] {
        self.0
    }
}

/// variable font axis 좌표의 정규형(모듈 문서 규칙).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct GlyphVariations {
    /// (tag, 값 bits), tag 오름차순.
    axes: Arc<[(AxisTag, u32)]>,
}

impl GlyphVariations {
    /// 기본 instance(좌표 없음).
    pub fn none() -> GlyphVariations {
        GlyphVariations::default()
    }

    /// 좌표 목록으로 정규형을 만듭니다. 같은 tag는 마지막 값이 우선합니다.
    pub fn new(
        axes: impl IntoIterator<Item = (AxisTag, f32)>,
    ) -> Result<GlyphVariations, GlyphValueError> {
        let mut sorted: Vec<(AxisTag, u32)> = Vec::new();
        for (count, (tag, value)) in axes.into_iter().enumerate() {
            if count >= MAX_VARIATION_AXES * 4 {
                // 중복 tag로 무한히 긴 입력을 막습니다.
                return Err(GlyphValueError::TooManyAxes);
            }
            if !value.is_finite() {
                return Err(GlyphValueError::NonFiniteVariation);
            }
            let bits = if value == 0.0 { 0 } else { value.to_bits() };
            match sorted.binary_search_by(|(existing, _)| existing.cmp(&tag)) {
                Ok(index) => sorted[index].1 = bits,
                Err(index) => {
                    if sorted.len() >= MAX_VARIATION_AXES {
                        return Err(GlyphValueError::TooManyAxes);
                    }
                    sorted.insert(index, (tag, bits));
                }
            }
        }
        Ok(GlyphVariations {
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

    /// `tag` 좌표.
    pub fn get(&self, tag: AxisTag) -> Option<f32> {
        self.axes
            .binary_search_by(|(existing, _)| existing.cmp(&tag))
            .ok()
            .map(|index| f32::from_bits(self.axes[index].1))
    }
}

/// glyph outline 요청 key(renderer XI.6.4 `GlyphKey`의 Outline mode와 같은 의미).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GlyphOutlineKey {
    /// 폰트 face.
    pub font: ExportFontFace,
    /// 폰트 안 glyph 번호(문자가 아님).
    pub glyph: u32,
    /// 양자화한 크기.
    pub size: GlyphSize,
    /// variation 좌표(정규형).
    pub variations: GlyphVariations,
}

/// 배치한 glyph 하나: glyph 번호와 pen 원점(text local 좌표, 1/64 px, y 아래 방향).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlacedGlyph {
    /// 폰트 안 glyph 번호.
    pub glyph: u32,
    /// pen 원점 x(1/64 px).
    pub x: i32,
    /// pen 원점 y(baseline, 1/64 px).
    pub y: i32,
}

/// 같은 폰트 instance·크기·고유 색을 쓰는 glyph 묶음.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphRun {
    /// 폰트 face(fallback이 적용된 실제 face).
    pub font: ExportFontFace,
    /// em 크기.
    pub size: GlyphSize,
    /// variation 좌표(shaping에 쓴 값).
    pub variations: GlyphVariations,
    /// run 고유 색(document text style fill). `None`이면 고유 색 없이 paint layer만 칠합니다.
    pub fill: Option<Color>,
    /// glyph(그리는 순서).
    pub glyphs: Vec<PlacedGlyph>,
}

impl GlyphRun {
    /// glyph의 outline key.
    pub fn key(&self, glyph: u32) -> GlyphOutlineKey {
        GlyphOutlineKey {
            font: self.font,
            glyph,
            size: self.size,
            variations: self.variations.clone(),
        }
    }
}

/// text layout 하나의 glyph run 목록(그리는 순서).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextGlyphLayout {
    /// run.
    pub runs: Vec<GlyphRun>,
}

/// glyph outline 조회 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GlyphOutline {
    /// 잉크가 없는 glyph(공백 등). 정상 결과입니다.
    Empty,
    /// pen 원점 기준 px 좌표(y 아래 방향) outline. 채우기 규칙은 non-zero입니다(OpenType outline 규칙).
    Path(PathGeometry),
    /// outline이 없고 bitmap·color 표현만 있는 glyph(emoji bitmap 등). 벡터로 쓸 수 없습니다.
    NoOutline,
}

/// glyph outline 조회 실패.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlyphOutlineError {
    /// 고정 font revision에 폰트가 없습니다.
    FontUnavailable,
    /// 폰트 데이터가 손상되어 outline을 읽지 못했습니다.
    Malformed,
}

impl fmt::Display for GlyphOutlineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            GlyphOutlineError::FontUnavailable => "고정 font revision에 폰트가 없습니다",
            GlyphOutlineError::Malformed => "폰트 데이터에서 glyph outline을 읽지 못했습니다",
        })
    }
}

impl std::error::Error for GlyphOutlineError {}

/// text glyph 조회. host(text layout cache·font registry)가 구현합니다.
///
/// `fonts`는 capture가 고정한 revision이며, 구현은 그 revision의 결과를 돌려주거나 없으면 `None`/오류를
/// 돌려줘야 합니다(현재 revision으로 대체하지 않습니다). 같은 입력에는 같은 결과를 돌려줘야 하고(결정론),
/// 손상된 폰트에도 panic하지 않아야 합니다.
pub trait ExportTextSource {
    /// `fonts` 시점의 `layout` glyph run.
    fn glyph_layout(
        &self,
        layout: TextLayoutKey,
        fonts: FontRevision,
    ) -> Option<Arc<TextGlyphLayout>>;

    /// `fonts` 시점의 glyph outline.
    fn glyph_outline(
        &self,
        key: &GlyphOutlineKey,
        fonts: FontRevision,
    ) -> Result<GlyphOutline, GlyphOutlineError>;
}

/// text가 없는 내보내기용 source(모든 조회가 없음).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoText;

impl ExportTextSource for NoText {
    fn glyph_layout(&self, _: TextLayoutKey, _: FontRevision) -> Option<Arc<TextGlyphLayout>> {
        None
    }

    fn glyph_outline(
        &self,
        _: &GlyphOutlineKey,
        _: FontRevision,
    ) -> Result<GlyphOutline, GlyphOutlineError> {
        Err(GlyphOutlineError::FontUnavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(t: &str) -> AxisTag {
        AxisTag::parse(t).unwrap()
    }

    #[test]
    fn size_quantizes_like_the_renderer_key() {
        assert_eq!(GlyphSize::from_px(12.0).unwrap().sixty_fourths(), 768);
        // 1/64 px 미만 차이는 같은 크기입니다.
        assert_eq!(
            GlyphSize::from_px(12.004).unwrap(),
            GlyphSize::from_px(12.0).unwrap()
        );
        assert_eq!(GlyphSize::from_px(4096.0).unwrap().px(), 4096.0);
        for bad in [0.0, -1.0, 4096.5, f32::NAN, f32::INFINITY, 0.001] {
            assert_eq!(GlyphSize::from_px(bad), Err(GlyphValueError::InvalidSize));
        }
        assert_eq!(
            GlyphSize::from_sixty_fourths(0),
            Err(GlyphValueError::InvalidSize)
        );
        assert!(GlyphSize::from_sixty_fourths(4096 * 64).is_ok());
        assert!(GlyphSize::from_sixty_fourths(4096 * 64 + 1).is_err());
    }

    #[test]
    fn variations_are_canonical() {
        let a = GlyphVariations::new([(tag("wght"), 700.0), (tag("opsz"), 12.0)]).unwrap();
        let b = GlyphVariations::new([
            (tag("opsz"), 12.0),
            (tag("wght"), 400.0),
            (tag("wght"), 700.0),
        ])
        .unwrap();
        assert_eq!(a, b, "순서·중복과 무관");
        assert_eq!(
            a.axes().map(|(t, v)| (t.bytes(), v)).collect::<Vec<_>>(),
            vec![(*b"opsz", 12.0), (*b"wght", 700.0)]
        );
        assert_eq!(a.get(tag("wght")), Some(700.0));
        assert_eq!(a.get(tag("wdth")), None);
        assert_eq!(
            GlyphVariations::new([(tag("wght"), -0.0)]).unwrap(),
            GlyphVariations::new([(tag("wght"), 0.0)]).unwrap()
        );
        assert_eq!(
            GlyphVariations::new([(tag("wght"), f32::NAN)]),
            Err(GlyphValueError::NonFiniteVariation)
        );
        let many = (0..65u8).map(|i| {
            (
                AxisTag::new([b'a', b'a', b'a' + i / 26, b'a' + i % 26]).unwrap(),
                1.0,
            )
        });
        assert_eq!(
            GlyphVariations::new(many),
            Err(GlyphValueError::TooManyAxes)
        );
        let repeated = std::iter::repeat_n((tag("wght"), 1.0), MAX_VARIATION_AXES * 4 + 1);
        assert_eq!(
            GlyphVariations::new(repeated),
            Err(GlyphValueError::TooManyAxes)
        );
        assert_eq!(
            AxisTag::new([0, b'a', b'b', b'c']),
            Err(GlyphValueError::InvalidTag)
        );
        assert_eq!(AxisTag::parse("wg"), Err(GlyphValueError::InvalidTag));
        assert!(GlyphVariations::none().is_empty());
    }

    #[test]
    fn run_keys_carry_font_size_and_variations() {
        let run = GlyphRun {
            font: ExportFontFace::new([7; 32], 2),
            size: GlyphSize::from_px(20.0).unwrap(),
            variations: GlyphVariations::new([(tag("wght"), 650.0)]).unwrap(),
            fill: None,
            glyphs: vec![PlacedGlyph {
                glyph: 5,
                x: 0,
                y: 0,
            }],
        };
        let key = run.key(5);
        assert_eq!(key.font.face_index(), 2);
        assert_eq!(key.glyph, 5);
        assert_eq!(key.size.sixty_fourths(), 1280);
        assert_eq!(key.variations.get(tag("wght")), Some(650.0));
        assert!(ExportFontFace::new([0xAB; 32], 1)
            .to_string()
            .ends_with("abab#1"));
        for error in [
            GlyphValueError::InvalidSize,
            GlyphValueError::InvalidTag,
            GlyphValueError::NonFiniteVariation,
            GlyphValueError::TooManyAxes,
        ] {
            assert!(!error.to_string().is_empty());
        }
        assert_eq!(
            NoText.glyph_outline(&key, FontRevision(1)),
            Err(GlyphOutlineError::FontUnavailable)
        );
        assert!(NoText
            .glyph_layout(TextLayoutKey::from_bytes([0; 16]), FontRevision(1))
            .is_none());
    }
}
