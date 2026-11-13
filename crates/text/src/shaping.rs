//! 공통 shaping 엔진 wrapper (X.9.1).
//!
//! 아키텍처 11.1의 "HarfBuzz 기반 shaping"을 HarfBuzz shaping 알고리즘의 순수 Rust 포트인
//! `harfrust`(ADR-0016)로 구현합니다.
//!
//! - **안전 경계**: 이 crate는 `#![forbid(unsafe_code)]`이고 `harfrust`도 `forbid(unsafe_code)`인
//!   순수 Rust라 C 라이브러리·raw pointer가 없습니다. 공개 API는 `harfrust` 타입을 노출하지 않으므로
//!   엔진을 교체해도 호출자는 바뀌지 않습니다.
//! - **결정론**: 결과는 font unit 정수(advance·offset)와 glyph id뿐이며 OS API에 의존하지 않습니다.
//!   같은 폰트 bytes·같은 요청은 항상 같은 glyph 열을 냅니다. px 변환은 레이아웃 단계가 정수
//!   산술로 합니다.
//! - **cluster**: glyph의 `cluster`는 요청 `text`(paragraph 전체) 안의 UTF-8 byte offset입니다.
//!   부분 범위를 shaping해도 paragraph 좌표를 유지해 줄바꿈·caret 계층이 그대로 씁니다.
//! - **glyph 순서**: HarfBuzz와 같이 **visual 순서**(왼쪽→오른쪽)입니다. RTL run은 첫 glyph가
//!   논리상 마지막 문자에 해당합니다.
//! - **OpenType feature·variation (X.9.6)**: [`ShapeRequest::features`]는 run 전체에 적용할 feature
//!   설정(예: `liga=0`)이고, variable font axis 값은 [`FontFace::with_variations`]로 만든 instance에
//!   담깁니다. 둘 다 canonical(tag 오름차순, tag당 하나) 형식으로 다룹니다.

use std::collections::BTreeMap;
use std::fmt;
use std::ops::Range;

use harfrust::font::{Font, Kind, Variation};
use harfrust::{
    Buffer, BufferFlags, ClusterLevel, Direction, Feature, Language, Script, ShapeOptions,
    ShaperFont, Tag,
};

/// shaping 문맥으로 run 앞뒤에서 넘기는 최대 문자 수(HarfBuzz context 길이 5 이상).
const CONTEXT_CHARS: usize = 8;

use crate::font::{FontVariation, OpenTypeTag};

/// OpenType feature 설정 하나(예: `liga` = 0이면 끔, `ss01` = 1이면 켬).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FontFeature {
    /// feature tag.
    pub tag: OpenTypeTag,
    /// 값(0 = 끔, 1 = 켬, 그 외 = 대체 glyph 번호 등 feature별 의미).
    pub value: u32,
}

/// feature 목록을 canonical 형식(tag 오름차순, 같은 tag는 마지막 값 우선)으로 만듭니다.
pub fn canonical_features(features: impl IntoIterator<Item = FontFeature>) -> Vec<FontFeature> {
    let mut map = BTreeMap::new();
    for feature in features {
        map.insert(feature.tag, feature.value);
    }
    map.into_iter()
        .map(|(tag, value)| FontFeature { tag, value })
        .collect()
}

/// variation 목록을 canonical 형식(tag 오름차순, 같은 tag는 마지막 값 우선)으로 만듭니다.
pub fn canonical_variations(
    variations: impl IntoIterator<Item = FontVariation>,
) -> Vec<FontVariation> {
    let mut map = BTreeMap::new();
    for variation in variations {
        map.insert(variation.tag, variation.value);
    }
    map.into_iter()
        .map(|(tag, value)| FontVariation { tag, value })
        .collect()
}

/// 폰트 안 glyph 번호.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GlyphId(u32);

impl GlyphId {
    /// `.notdef` glyph(폰트가 문자를 갖지 않을 때).
    pub const NOTDEF: GlyphId = GlyphId(0);

    /// raw glyph 번호에서 만듭니다.
    pub const fn new(raw: u32) -> GlyphId {
        GlyphId(raw)
    }

    /// raw glyph 번호.
    pub const fn raw(&self) -> u32 {
        self.0
    }
}

/// 쓰기 방향.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TextDirection {
    /// 왼쪽에서 오른쪽.
    LeftToRight,
    /// 오른쪽에서 왼쪽.
    RightToLeft,
}

impl TextDirection {
    /// 진단용 안정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            TextDirection::LeftToRight => "ltr",
            TextDirection::RightToLeft => "rtl",
        }
    }
}

/// ISO 15924 script tag(예: `Latn`, `Arab`, `Hang`). 대문자 1개 + 소문자 3개만 허용합니다.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScriptTag([u8; 4]);

impl ScriptTag {
    /// Common(`Zyyy`): 여러 script가 공유하는 문자(공백·숫자·구두점).
    pub const COMMON: ScriptTag = ScriptTag(*b"Zyyy");
    /// Inherited(`Zinh`): 앞 문자의 script를 따르는 문자(결합 부호).
    pub const INHERITED: ScriptTag = ScriptTag(*b"Zinh");
    /// Unknown(`Zzzz`): 미할당 문자.
    pub const UNKNOWN: ScriptTag = ScriptTag(*b"Zzzz");
    /// Latin.
    pub const LATIN: ScriptTag = ScriptTag(*b"Latn");
    /// Arabic.
    pub const ARABIC: ScriptTag = ScriptTag(*b"Arab");
    /// Hebrew.
    pub const HEBREW: ScriptTag = ScriptTag(*b"Hebr");
    /// Hangul.
    pub const HANGUL: ScriptTag = ScriptTag(*b"Hang");
    /// Han.
    pub const HAN: ScriptTag = ScriptTag(*b"Hani");
    /// Hiragana.
    pub const HIRAGANA: ScriptTag = ScriptTag(*b"Hira");
    /// Katakana.
    pub const KATAKANA: ScriptTag = ScriptTag(*b"Kana");

    /// 4바이트 tag를 검증해 만듭니다.
    pub fn new(tag: [u8; 4]) -> Option<ScriptTag> {
        (tag[0].is_ascii_uppercase() && tag[1..].iter().all(u8::is_ascii_lowercase))
            .then_some(ScriptTag(tag))
    }

    /// 4글자 문자열에서 만듭니다(예: `"Latn"`).
    pub fn parse(tag: &str) -> Option<ScriptTag> {
        ScriptTag::new(tag.as_bytes().try_into().ok()?)
    }

    /// 원본 bytes.
    pub const fn bytes(&self) -> [u8; 4] {
        self.0
    }

    /// 실제 script(Common·Inherited·Unknown이 아님)인지.
    pub fn is_real(&self) -> bool {
        !matches!(
            *self,
            ScriptTag::COMMON | ScriptTag::INHERITED | ScriptTag::UNKNOWN
        )
    }

    fn to_engine(self) -> Script {
        // ScriptTag는 생성 시 ISO 15924 형식만 허용하므로 엔진 변환은 항상 Some입니다. 방어적으로
        // Unknown을 씁니다.
        Script::from_iso15924_tag(Tag::new(&self.0)).unwrap_or(Script::UNKNOWN)
    }
}

impl fmt::Display for ScriptTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{}", char::from(byte))?;
        }
        Ok(())
    }
}

impl fmt::Debug for ScriptTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ScriptTag({self})")
    }
}

/// BCP 47 language tag 형식 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanguageTagError {
    /// 빈 문자열이거나 빈 subtag가 있습니다.
    EmptySubtag,
    /// subtag가 ASCII 영숫자 1~8자가 아닙니다.
    InvalidSubtag {
        /// 위반 subtag.
        subtag: String,
    },
    /// 첫 subtag(언어)가 영문자 2~8자가 아닙니다.
    InvalidPrimaryLanguage,
    /// 전체 길이가 상한을 넘습니다.
    TooLong {
        /// 입력 byte 길이.
        len: usize,
    },
}

impl fmt::Display for LanguageTagError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LanguageTagError::EmptySubtag => f.write_str("language tag에 빈 subtag가 있습니다"),
            LanguageTagError::InvalidSubtag { subtag } => {
                write!(
                    f,
                    "language subtag가 ASCII 영숫자 1~8자가 아닙니다: {subtag}"
                )
            }
            LanguageTagError::InvalidPrimaryLanguage => {
                f.write_str("언어 subtag는 영문자 2~8자여야 합니다")
            }
            LanguageTagError::TooLong { len } => write!(
                f,
                "language tag가 너무 깁니다: {len} bytes(최대 {})",
                LanguageTag::MAX_LEN
            ),
        }
    }
}

impl std::error::Error for LanguageTagError {}

/// BCP 47 language tag(예: `ko`, `ja-JP`, `zh-Hant-TW`). 실행 환경 locale을 암묵적으로 쓰지 않도록
/// shaping·줄바꿈에 명시적으로 넘기는 값입니다.
///
/// canonical 형식: `-` 구분, 언어 subtag 소문자, 4자 영문 subtag(script) Title case, 2자 영문
/// subtag(region) 대문자, 나머지 소문자. 한 글자 singleton(`u`, `x` 등) 뒤의 확장 subtag는 모두
/// 소문자입니다. 같은 언어는 항상 같은 문자열(캐시 key 안정).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LanguageTag(String);

impl LanguageTag {
    /// 허용 최대 길이(bytes).
    pub const MAX_LEN: usize = 64;

    /// 검증하고 canonical 형식으로 만듭니다.
    pub fn parse(tag: &str) -> Result<LanguageTag, LanguageTagError> {
        if tag.len() > LanguageTag::MAX_LEN {
            return Err(LanguageTagError::TooLong { len: tag.len() });
        }
        let mut out = String::with_capacity(tag.len());
        let mut in_extension = false;
        for (i, subtag) in tag.split('-').enumerate() {
            if subtag.is_empty() {
                return Err(LanguageTagError::EmptySubtag);
            }
            if subtag.len() > 8 || !subtag.bytes().all(|b| b.is_ascii_alphanumeric()) {
                return Err(LanguageTagError::InvalidSubtag {
                    subtag: subtag.to_string(),
                });
            }
            let alpha = subtag.bytes().all(|b| b.is_ascii_alphabetic());
            if i == 0 {
                if !alpha || subtag.len() < 2 {
                    return Err(LanguageTagError::InvalidPrimaryLanguage);
                }
                out.push_str(&subtag.to_ascii_lowercase());
                continue;
            }
            out.push('-');
            if subtag.len() == 1 {
                in_extension = true;
            }
            if in_extension {
                out.push_str(&subtag.to_ascii_lowercase());
            } else if alpha && subtag.len() == 4 {
                let lower = subtag.to_ascii_lowercase();
                out.push_str(&lower[..1].to_ascii_uppercase());
                out.push_str(&lower[1..]);
            } else if alpha && subtag.len() == 2 {
                out.push_str(&subtag.to_ascii_uppercase());
            } else {
                out.push_str(&subtag.to_ascii_lowercase());
            }
        }
        Ok(LanguageTag(out))
    }

    /// canonical 문자열.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 언어 subtag(예: `ja-JP` → `ja`).
    pub fn primary_language(&self) -> &str {
        self.0.split('-').next().unwrap_or("")
    }
}

impl fmt::Display for LanguageTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for LanguageTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LanguageTag({})", self.0)
    }
}

/// 폰트 로드 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontLoadError {
    /// 입력이 지원하는 폰트 형식이 아니거나 `face_index`의 face가 없습니다.
    Unsupported {
        /// 요청 face index.
        face_index: u32,
    },
    /// SFNT(TrueType·OpenType) 기반이 아닌 폰트입니다(OpenType layout shaping 불가).
    NotSfnt,
}

impl fmt::Display for FontLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FontLoadError::Unsupported { face_index } => write!(
                f,
                "폰트 데이터를 읽을 수 없거나 face {face_index}이(가) 없습니다"
            ),
            FontLoadError::NotSfnt => f.write_str("SFNT 기반 폰트가 아닙니다"),
        }
    }
}

impl std::error::Error for FontLoadError {}

/// face 전체 줄 metric(font unit). descender는 음수(baseline 아래)입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineMetrics {
    /// baseline 위 높이(≥ 0).
    pub ascender: i32,
    /// baseline 아래 깊이(≤ 0).
    pub descender: i32,
    /// 줄 사이 추가 간격(≥ 0).
    pub line_gap: i32,
}

/// shaping에 쓰는 로드된 face (X.9.1). 폰트 bytes를 소유하며 clone은 저렴합니다(공유).
///
/// face 정체성(hash+index)은 이 타입이 계산하지 않습니다. 호출자가 registry(X.8.2)에 등록한
/// handle과 짝지어 관리합니다.
#[derive(Clone)]
pub struct FontFace {
    font: Font,
    face_index: u32,
    units_per_em: u16,
    glyph_count: u32,
    line_metrics: LineMetrics,
    variations: Vec<FontVariation>,
}

impl fmt::Debug for FontFace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FontFace")
            .field("face_index", &self.face_index)
            .field("units_per_em", &self.units_per_em)
            .field("glyph_count", &self.glyph_count)
            .field("line_metrics", &self.line_metrics)
            .field("variations", &self.variations)
            .finish_non_exhaustive()
    }
}

impl FontFace {
    /// 폰트 파일 bytes와 face index로 로드합니다. 형식이 잘못된 입력은 오류이며 panic하지 않습니다.
    pub fn from_bytes(bytes: Vec<u8>, face_index: u32) -> Result<FontFace, FontLoadError> {
        let font = Font::new(bytes, face_index).ok_or(FontLoadError::Unsupported { face_index })?;
        FontFace::from_font(font, face_index)
    }

    fn from_font(font: Font, face_index: u32) -> Result<FontFace, FontLoadError> {
        if !matches!(font.kind(), Kind::Sfnt(..)) {
            return Err(FontLoadError::NotSfnt);
        }
        let metrics = font.metrics();
        let units_per_em = metrics.units_per_em;
        let line_metrics = select_line_metrics(metrics, units_per_em);
        Ok(FontFace {
            units_per_em,
            glyph_count: metrics.num_glyphs,
            line_metrics,
            face_index,
            font,
            variations: Vec::new(),
        })
    }

    /// variable font axis 값을 적용한 instance를 만듭니다 (X.9.6). 폰트 bytes는 공유합니다.
    ///
    /// 값은 canonical 형식으로 정리되고(tag 오름차순, 같은 tag는 마지막 값 우선), 폰트에 없는 axis는
    /// 무시되며 axis 범위 밖 값은 범위로 제한됩니다(OpenType 정규화 규칙). 빈 목록은 기본 instance입니다.
    /// 줄 metric은 instance 위치에서 다시 읽습니다.
    pub fn with_variations(&self, variations: &[FontVariation]) -> FontFace {
        let variations = canonical_variations(variations.iter().copied());
        let font = self
            .font
            .instance_builder()
            .variations(
                variations
                    .iter()
                    .map(|v| Variation::new(Tag::new(&v.tag.bytes()), v.value.get())),
            )
            .build();
        let line_metrics = select_line_metrics(font.metrics(), self.units_per_em);
        FontFace {
            font,
            face_index: self.face_index,
            units_per_em: self.units_per_em,
            glyph_count: self.glyph_count,
            line_metrics,
            variations,
        }
    }

    /// 이 instance에 적용된 variation(canonical). 기본 instance는 빈 목록입니다.
    pub fn variations(&self) -> &[FontVariation] {
        &self.variations
    }

    /// face index.
    pub fn face_index(&self) -> u32 {
        self.face_index
    }

    /// em당 font unit(16..=16384).
    pub fn units_per_em(&self) -> u16 {
        self.units_per_em
    }

    /// glyph 수.
    pub fn glyph_count(&self) -> u32 {
        self.glyph_count
    }

    /// 줄 metric(font unit).
    pub fn line_metrics(&self) -> LineMetrics {
        self.line_metrics
    }

    /// 문자의 기본 glyph(cmap). 폰트가 문자를 갖지 않으면 `None`.
    pub fn nominal_glyph(&self, c: char) -> Option<GlyphId> {
        self.font
            .charmap()
            .map_unicode(u32::from(c))
            .map(|g| GlyphId(g.to_u32()))
    }

    pub(crate) fn engine_font(&self) -> &Font {
        &self.font
    }
}

/// 줄 metric 선택 규칙: `OS/2` USE_TYPO_METRICS가 켜져 있고 typo 값이 있으면 typo, 아니면 `hhea`,
/// 그다음 typo, `OS/2` win 값, 마지막으로 0.8em/-0.2em/0 기본값. 값은 font unit 정수로 반올림합니다.
fn select_line_metrics(metrics: &harfrust::font::metrics::Metrics, upem: u16) -> LineMetrics {
    let usable = |line: &harfrust::font::metrics::LineBox| {
        line.ascender.to_bits() != 0 || line.descender.to_bits() != 0
    };
    let from_box = |line: &harfrust::font::metrics::LineBox| LineMetrics {
        ascender: fixed_to_units(line.ascender.to_bits()).max(0),
        descender: fixed_to_units(line.descender.to_bits()).min(0),
        line_gap: fixed_to_units(line.line_gap.to_bits()).max(0),
    };
    let typo = metrics.typo_line.as_ref().filter(|l| usable(l));
    let hhea = metrics.hhea_line.as_ref().filter(|l| usable(l));
    if metrics.use_typo_metrics {
        if let Some(line) = typo {
            return from_box(line);
        }
    }
    if let Some(line) = hhea.or(typo) {
        return from_box(line);
    }
    if let Some(win) = &metrics.win_line {
        return LineMetrics {
            ascender: fixed_to_units(win.ascender.to_bits()).max(0),
            descender: fixed_to_units(win.descender.to_bits()).min(0),
            line_gap: 0,
        };
    }
    let upem = i32::from(upem);
    LineMetrics {
        ascender: upem * 4 / 5,
        descender: -(upem / 5),
        line_gap: 0,
    }
}

/// 16.16 고정소수점 raw 값을 가장 가까운 정수로 반올림합니다(중간값은 0에서 먼 쪽). `i32` 범위로
/// 제한합니다.
fn fixed_to_units(bits: i64) -> i32 {
    let magnitude = (bits.unsigned_abs() + 0x8000) >> 16;
    let rounded = i64::try_from(magnitude).unwrap_or(i64::MAX);
    let signed = if bits < 0 { -rounded } else { rounded };
    i32::try_from(signed).unwrap_or(if signed < 0 { i32::MIN } else { i32::MAX })
}

/// shaping 요청 하나(같은 script·방향·언어의 연속 구간).
#[derive(Debug, Clone)]
pub struct ShapeRequest<'a> {
    /// paragraph 전체 텍스트. 앞뒤 문맥과 cluster 좌표의 기준입니다.
    pub text: &'a str,
    /// shaping할 byte 범위(문자 경계).
    pub range: Range<usize>,
    /// 쓰기 방향.
    pub direction: TextDirection,
    /// script.
    pub script: ScriptTag,
    /// 언어(없으면 언어별 형태 규칙을 쓰지 않음).
    pub language: Option<&'a LanguageTag>,
    /// run 전체에 적용할 OpenType feature 설정(X.9.6). 폰트 기본 feature(`liga` 등)는 명시하지 않으면
    /// 켜진 상태입니다.
    pub features: &'a [FontFeature],
}

/// shaping 결과 glyph 하나(font unit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShapedGlyph {
    /// glyph.
    pub glyph: GlyphId,
    /// paragraph `text` 안 UTF-8 byte offset(이 glyph가 속한 cluster의 시작).
    pub cluster: u32,
    /// 가로 advance.
    pub x_advance: i32,
    /// 세로 advance.
    pub y_advance: i32,
    /// 가로 offset.
    pub x_offset: i32,
    /// 세로 offset.
    pub y_offset: i32,
    /// 이 glyph의 cluster 앞에서 끊으면 다시 shaping해야 하는지(HarfBuzz `UNSAFE_TO_BREAK`).
    pub unsafe_to_break: bool,
}

/// shaping 결과 run.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ShapedRun {
    /// 원본 byte 범위.
    pub range: Range<usize>,
    /// 쓰기 방향.
    pub direction: TextDirection,
    /// 결과 좌표의 em당 unit.
    pub units_per_em: u16,
    /// glyph(visual 순서).
    pub glyphs: Vec<ShapedGlyph>,
}

impl ShapedRun {
    /// 가로 advance 합(font unit).
    pub fn total_x_advance(&self) -> i64 {
        self.glyphs.iter().map(|g| i64::from(g.x_advance)).sum()
    }
}

/// shaping 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShapeError {
    /// 범위가 텍스트를 벗어나거나 거꾸로입니다.
    RangeOutOfBounds {
        /// 시작.
        start: usize,
        /// 끝.
        end: usize,
        /// 텍스트 byte 길이.
        len: usize,
    },
    /// 범위 경계가 UTF-8 문자 경계가 아닙니다.
    NotCharBoundary {
        /// 위반 offset.
        offset: usize,
    },
    /// cluster offset(`u32`)으로 표현할 수 없을 만큼 텍스트가 깁니다.
    TextTooLong {
        /// 텍스트 byte 길이.
        len: usize,
    },
    /// 엔진이 요청을 거부했습니다.
    Engine {
        /// 엔진 사유.
        reason: String,
    },
}

impl fmt::Display for ShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShapeError::RangeOutOfBounds { start, end, len } => write!(
                f,
                "shaping 범위 {start}..{end}이(가) 텍스트 길이 {len}을(를) 벗어납니다"
            ),
            ShapeError::NotCharBoundary { offset } => {
                write!(f, "shaping 범위 경계 {offset}이(가) 문자 경계가 아닙니다")
            }
            ShapeError::TextTooLong { len } => {
                write!(f, "텍스트가 너무 깁니다: {len} bytes(최대 u32)")
            }
            ShapeError::Engine { reason } => write!(f, "shaping 엔진 오류: {reason}"),
        }
    }
}

impl std::error::Error for ShapeError {}

/// 요청 범위를 검증합니다.
pub(crate) fn check_range(text: &str, range: &Range<usize>) -> Result<(), ShapeError> {
    if u32::try_from(text.len()).is_err() {
        return Err(ShapeError::TextTooLong { len: text.len() });
    }
    if range.start > range.end || range.end > text.len() {
        return Err(ShapeError::RangeOutOfBounds {
            start: range.start,
            end: range.end,
            len: text.len(),
        });
    }
    for offset in [range.start, range.end] {
        if !text.is_char_boundary(offset) {
            return Err(ShapeError::NotCharBoundary { offset });
        }
    }
    Ok(())
}

/// face로 요청 구간을 shaping합니다 (X.9.1).
pub fn shape(face: &FontFace, request: &ShapeRequest<'_>) -> Result<ShapedRun, ShapeError> {
    let ShapeRequest {
        text,
        ref range,
        direction,
        script,
        language,
        features,
    } = *request;
    check_range(text, range)?;
    let segment = &text[range.clone()];

    let mut buffer = Buffer::new();
    buffer.push_str(segment);
    buffer.set_direction(match direction {
        TextDirection::LeftToRight => Direction::LeftToRight,
        TextDirection::RightToLeft => Direction::RightToLeft,
    });
    buffer.set_script(Some(script.to_engine()));
    buffer.set_language(language.and_then(|l| Language::new(l.as_str())));
    buffer.set_cluster_level(ClusterLevel::MonotoneGraphemes);
    buffer.set_pre_context(context_before(text, range.start));
    buffer.set_post_context(context_after(text, range.end));
    let mut flags = BufferFlags::empty();
    if range.start == 0 {
        flags |= BufferFlags::BEGINNING_OF_TEXT;
    }
    if range.end == text.len() {
        flags |= BufferFlags::END_OF_TEXT;
    }
    buffer.set_flags(flags);

    let engine_features: Vec<Feature> = canonical_features(features.iter().copied())
        .iter()
        .map(|f| Feature::new(Tag::new(&f.tag.bytes()), f.value, ..))
        .collect();
    let shaper = ShaperFont::new(face.engine_font());
    let options = ShapeOptions::new().features(&engine_features);
    harfrust::shape(&shaper, &mut buffer, options).map_err(|err| ShapeError::Engine {
        reason: err.to_string(),
    })?;
    if !buffer.allocation_successful() {
        return Err(ShapeError::Engine {
            reason: "shaping buffer 할당 한도를 넘었습니다".to_string(),
        });
    }

    // check_range가 text.len() ≤ u32::MAX를 보장하므로 start와 cluster 합은 u32 범위입니다.
    let base = range.start as u32;
    let glyphs = buffer
        .glyph_infos()
        .iter()
        .zip(buffer.glyph_positions())
        .map(|(info, pos)| ShapedGlyph {
            glyph: GlyphId(info.glyph_id),
            cluster: base.saturating_add(info.cluster),
            x_advance: pos.x_advance,
            y_advance: pos.y_advance,
            x_offset: pos.x_offset,
            y_offset: pos.y_offset,
            unsafe_to_break: info.unsafe_to_break(),
        })
        .collect();
    Ok(ShapedRun {
        range: range.clone(),
        direction,
        units_per_em: face.units_per_em(),
        glyphs,
    })
}

/// `offset` 앞의 최대 [`CONTEXT_CHARS`]개 문자.
fn context_before(text: &str, offset: usize) -> &str {
    let before = &text[..offset];
    let start = before
        .char_indices()
        .rev()
        .nth(CONTEXT_CHARS - 1)
        .map_or(0, |(i, _)| i);
    &before[start..]
}

/// `offset` 뒤의 최대 [`CONTEXT_CHARS`]개 문자.
fn context_after(text: &str, offset: usize) -> &str {
    let after = &text[offset..];
    let end = after
        .char_indices()
        .nth(CONTEXT_CHARS)
        .map_or(after.len(), |(i, _)| i);
    &after[..end]
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::font::tests::{axis, tag};
    use crate::test_fonts::{
        latin_advance, latin_liga_spec, latin_rtl_spec, latin_spec, variable_latin_spec,
    };

    pub(crate) fn latin_face() -> FontFace {
        FontFace::from_bytes(latin_spec().build(), 0).expect("합성 Latin 폰트")
    }

    fn request(text: &str) -> ShapeRequest<'_> {
        ShapeRequest {
            text,
            range: 0..text.len(),
            direction: TextDirection::LeftToRight,
            script: ScriptTag::LATIN,
            language: None,
            features: &[],
        }
    }

    #[test]
    fn simple_latin_shaping_fixture() {
        let face = latin_face();
        let spec = latin_spec();
        let run = shape(&face, &request("Hello, wi")).expect("shape");
        let expected: Vec<(u32, u32, i32)> = "Hello, wi"
            .char_indices()
            .map(|(i, c)| {
                (
                    u32::from(spec.glyph_of(u32::from(c))),
                    i as u32,
                    i32::from(latin_advance(u32::from(c))),
                )
            })
            .collect();
        let actual: Vec<(u32, u32, i32)> = run
            .glyphs
            .iter()
            .map(|g| (g.glyph.raw(), g.cluster, g.x_advance))
            .collect();
        assert_eq!(actual, expected);
        assert!(run
            .glyphs
            .iter()
            .all(|g| g.y_advance == 0 && g.x_offset == 0 && g.y_offset == 0));
        assert_eq!(run.units_per_em, 1000);
        // H e o = 500, l l , i = 300, 공백 250, w 800.
        assert_eq!(run.total_x_advance(), 500 * 3 + 300 * 4 + 250 + 800);
        assert_eq!(run.direction, TextDirection::LeftToRight);
    }

    #[test]
    fn shaping_is_deterministic_across_face_loads() {
        let a = shape(&latin_face(), &request("Stellar layout")).expect("a");
        let b = shape(&latin_face(), &request("Stellar layout")).expect("b");
        assert_eq!(a, b);
    }

    #[test]
    fn sub_range_keeps_paragraph_cluster_offsets() {
        let face = latin_face();
        let text = "abc def";
        let run = shape(
            &face,
            &ShapeRequest {
                range: 4..7,
                ..request(text)
            },
        )
        .expect("shape");
        let clusters: Vec<u32> = run.glyphs.iter().map(|g| g.cluster).collect();
        assert_eq!(clusters, vec![4, 5, 6]);
        assert_eq!(run.range, 4..7);
        // 빈 범위는 glyph 0개.
        let empty = shape(
            &face,
            &ShapeRequest {
                range: 3..3,
                ..request(text)
            },
        )
        .expect("empty");
        assert!(empty.glyphs.is_empty());
    }

    #[test]
    fn rtl_run_glyphs_are_in_visual_order() {
        let face = FontFace::from_bytes(latin_rtl_spec().build(), 0).expect("폰트");
        let text = "שלום";
        let run = shape(
            &face,
            &ShapeRequest {
                direction: TextDirection::RightToLeft,
                script: ScriptTag::HEBREW,
                ..request(text)
            },
        )
        .expect("shape");
        // 히브리 문자는 2 bytes. visual 순서라 cluster가 내림차순입니다.
        let clusters: Vec<u32> = run.glyphs.iter().map(|g| g.cluster).collect();
        assert_eq!(clusters, vec![6, 4, 2, 0]);
        assert!(run.glyphs.iter().all(|g| g.x_advance == 450));
    }

    #[test]
    fn missing_characters_map_to_notdef() {
        let face = latin_face();
        let run = shape(&face, &request("a한b")).expect("shape");
        let glyphs: Vec<(u32, u32)> = run
            .glyphs
            .iter()
            .map(|g| (g.glyph.raw(), g.cluster))
            .collect();
        assert_eq!(glyphs[1], (GlyphId::NOTDEF.raw(), 1));
        assert_eq!(glyphs.len(), 3);
        assert_eq!(face.nominal_glyph('한'), None);
        assert_eq!(
            face.nominal_glyph('A').map(|g| g.raw()),
            Some(u32::from(latin_spec().glyph_of(0x41)))
        );
    }

    #[test]
    fn invalid_ranges_are_typed_errors() {
        let face = latin_face();
        let text = "a한";
        assert_eq!(
            shape(
                &face,
                &ShapeRequest {
                    range: 0..9,
                    ..request(text)
                }
            ),
            Err(ShapeError::RangeOutOfBounds {
                start: 0,
                end: 9,
                len: 4
            })
        );
        assert_eq!(
            shape(
                &face,
                &ShapeRequest {
                    range: 0..2,
                    ..request(text)
                }
            ),
            Err(ShapeError::NotCharBoundary { offset: 2 })
        );
        #[allow(clippy::reversed_empty_ranges)]
        let reversed = 3..1;
        assert!(matches!(
            shape(
                &face,
                &ShapeRequest {
                    range: reversed,
                    ..request(text)
                }
            ),
            Err(ShapeError::RangeOutOfBounds { .. })
        ));
    }

    #[test]
    fn face_loading_rejects_garbage_without_panic() {
        assert_eq!(
            FontFace::from_bytes(vec![], 0).map(|_| ()),
            Err(FontLoadError::Unsupported { face_index: 0 })
        );
        assert_eq!(
            FontFace::from_bytes(b"not a font at all".to_vec(), 0).map(|_| ()),
            Err(FontLoadError::Unsupported { face_index: 0 })
        );
        // 단일 폰트에서 face index 1은 없습니다.
        assert_eq!(
            FontFace::from_bytes(latin_spec().build(), 1).map(|_| ()),
            Err(FontLoadError::Unsupported { face_index: 1 })
        );
        // 잘린 폰트도 panic 없이 오류 또는 안전한 결과입니다.
        let bytes = latin_spec().build();
        for len in [12, 40, bytes.len() / 2] {
            if let Ok(face) = FontFace::from_bytes(bytes[..len].to_vec(), 0) {
                let _ = shape(&face, &request("abc"));
            }
        }
    }

    #[test]
    fn face_metrics_come_from_hhea() {
        let face = latin_face();
        assert_eq!(face.units_per_em(), 1000);
        assert_eq!(face.glyph_count(), 96);
        assert_eq!(face.face_index(), 0);
        assert_eq!(
            face.line_metrics(),
            LineMetrics {
                ascender: 800,
                descender: -200,
                line_gap: 0
            }
        );
        assert!(format!("{face:?}").contains("units_per_em: 1000"));
        assert_eq!(fixed_to_units(0x0001_8000), 2);
        assert_eq!(fixed_to_units(-0x0001_8000), -2);
        assert_eq!(fixed_to_units(0x0001_7FFF), 1);
        assert_eq!(fixed_to_units(i64::MAX), i32::MAX);
    }

    fn glyph_summary(run: &ShapedRun) -> Vec<(u32, u32, i32)> {
        run.glyphs
            .iter()
            .map(|g| (g.glyph.raw(), g.cluster, g.x_advance))
            .collect()
    }

    #[test]
    fn liga_on_off_fixture() {
        let spec = latin_liga_spec();
        let face = FontFace::from_bytes(spec.build(), 0).expect("liga 폰트");
        let text = "office fit";
        let glyph = |c: char| u32::from(spec.glyph_of(u32::from(c)));
        // ligature glyph id: ASCII 95개 + .notdef 뒤 첫 추가 glyph가 fi(96), 다음이 ffi(97).
        let (fi, ffi) = (96, 97);

        // 기본(liga 켜짐): "ffi" → ffi ligature(cluster 1), "fi" → fi ligature(cluster 7).
        let on = shape(&face, &request(text)).expect("liga on");
        assert_eq!(
            glyph_summary(&on),
            vec![
                (glyph('o'), 0, 500),
                (ffi, 1, 800),
                (glyph('c'), 4, 500),
                (glyph('e'), 5, 500),
                (glyph(' '), 6, 250),
                (fi, 7, 550),
                (glyph('t'), 9, 300),
            ]
        );
        // 명시적으로 켜도 같은 결과.
        let liga_on = [FontFeature {
            tag: tag("liga"),
            value: 1,
        }];
        assert_eq!(
            shape(
                &face,
                &ShapeRequest {
                    features: &liga_on,
                    ..request(text)
                }
            )
            .expect("liga=1"),
            on
        );

        // liga=0: 모든 문자가 개별 glyph.
        let liga_off = [FontFeature {
            tag: tag("liga"),
            value: 0,
        }];
        let off = shape(
            &face,
            &ShapeRequest {
                features: &liga_off,
                ..request(text)
            },
        )
        .expect("liga off");
        let expected_off: Vec<(u32, u32, i32)> = text
            .char_indices()
            .map(|(i, c)| (glyph(c), i as u32, i32::from(latin_advance(u32::from(c)))))
            .collect();
        assert_eq!(glyph_summary(&off), expected_off);
        // 예상 차이: glyph 수 7 vs 10, 폭 3400 vs 3550.
        assert_eq!((on.glyphs.len(), off.glyphs.len()), (7, 10));
        assert_eq!((on.total_x_advance(), off.total_x_advance()), (3400, 3550));

        // feature 목록은 canonical: 같은 tag는 마지막 값 우선.
        let both = [liga_on[0], liga_off[0]];
        assert_eq!(
            shape(
                &face,
                &ShapeRequest {
                    features: &both,
                    ..request(text)
                }
            )
            .expect("마지막 값 우선"),
            off
        );
    }

    #[test]
    fn variable_axis_fixture() {
        let base = FontFace::from_bytes(variable_latin_spec().build(), 0).expect("variable 폰트");
        let width = |face: &FontFace| {
            shape(face, &request("ab"))
                .expect("shape")
                .glyphs
                .iter()
                .map(|g| g.x_advance)
                .collect::<Vec<_>>()
        };
        // 기본 instance(wght 400): 500씩.
        assert_eq!(width(&base), vec![500, 500]);
        assert!(base.variations().is_empty());
        // wght 900(정규화 1.0): HVAR delta +100 → 600.
        let bold = base.with_variations(&[axis("wght", 900.0)]);
        assert_eq!(width(&bold), vec![600, 600]);
        assert_eq!(bold.variations(), &[axis("wght", 900.0)]);
        // wght 650(정규화 0.5): +50.
        assert_eq!(
            width(&base.with_variations(&[axis("wght", 650.0)])),
            vec![550, 550]
        );
        // 기본값 아래(wght 300): region이 0..1이라 변화 없음.
        assert_eq!(
            width(&base.with_variations(&[axis("wght", 300.0)])),
            vec![500, 500]
        );
        // 범위 밖(1000)은 최대값으로 제한.
        assert_eq!(
            width(&base.with_variations(&[axis("wght", 1000.0)])),
            vec![600, 600]
        );
        // 폰트에 없는 axis는 무시.
        assert_eq!(
            width(&base.with_variations(&[axis("wdth", 50.0)])),
            vec![500, 500]
        );
        // 같은 tag는 마지막 값 우선, 결과는 canonical.
        let last = base.with_variations(&[axis("wght", 900.0), axis("wght", 650.0)]);
        assert_eq!(width(&last), vec![550, 550]);
        assert_eq!(last.variations(), &[axis("wght", 650.0)]);
        // instance는 metric·glyph 수를 유지합니다(이 폰트에는 MVAR 없음).
        assert_eq!(bold.line_metrics(), base.line_metrics());
        assert_eq!(bold.glyph_count(), base.glyph_count());
    }

    #[test]
    fn canonical_feature_and_variation_lists() {
        let features = canonical_features([
            FontFeature {
                tag: tag("liga"),
                value: 1,
            },
            FontFeature {
                tag: tag("kern"),
                value: 0,
            },
            FontFeature {
                tag: tag("liga"),
                value: 0,
            },
        ]);
        assert_eq!(
            features,
            vec![
                FontFeature {
                    tag: tag("kern"),
                    value: 0
                },
                FontFeature {
                    tag: tag("liga"),
                    value: 0
                }
            ]
        );
        let variations =
            canonical_variations([axis("wght", 1.0), axis("opsz", 2.0), axis("wght", 3.0)]);
        assert_eq!(variations, vec![axis("opsz", 2.0), axis("wght", 3.0)]);
    }

    #[test]
    fn script_and_language_tags_are_validated_and_canonical() {
        assert_eq!(ScriptTag::parse("Latn"), Some(ScriptTag::LATIN));
        assert_eq!(ScriptTag::parse("latn"), None);
        assert_eq!(ScriptTag::parse("LATN"), None);
        assert_eq!(ScriptTag::parse("Lat"), None);
        assert_eq!(ScriptTag::HANGUL.to_string(), "Hang");
        assert!(ScriptTag::ARABIC.is_real());
        assert!(!ScriptTag::COMMON.is_real());
        assert_eq!(format!("{:?}", ScriptTag::HAN), "ScriptTag(Hani)");

        let tag = LanguageTag::parse("ZH-hant-tw").expect("유효");
        assert_eq!(tag.as_str(), "zh-Hant-TW");
        assert_eq!(tag.primary_language(), "zh");
        assert_eq!(LanguageTag::parse("ko").expect("ko").to_string(), "ko");
        assert_eq!(
            LanguageTag::parse("ja-JP-u-ca-japanese")
                .expect("확장")
                .as_str(),
            "ja-JP-u-ca-japanese"
        );
        assert_eq!(LanguageTag::parse(""), Err(LanguageTagError::EmptySubtag));
        assert_eq!(
            LanguageTag::parse("en--US"),
            Err(LanguageTagError::EmptySubtag)
        );
        assert_eq!(
            LanguageTag::parse("e"),
            Err(LanguageTagError::InvalidPrimaryLanguage)
        );
        assert_eq!(
            LanguageTag::parse("12"),
            Err(LanguageTagError::InvalidPrimaryLanguage)
        );
        assert!(matches!(
            LanguageTag::parse("en_US"),
            Err(LanguageTagError::InvalidSubtag { .. })
        ));
        assert!(matches!(
            LanguageTag::parse(&"a".repeat(65)),
            Err(LanguageTagError::TooLong { len: 65 })
        ));
        assert!(LanguageTagError::TooLong { len: 65 }
            .to_string()
            .contains("65"));
    }
}
