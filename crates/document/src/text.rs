//! Text payload (VI.7).
//!
//! **text index 정책**: run·paragraph 경계는 UTF-8 byte offset이 아니라
//! **Unicode scalar value 단위 offset**([`TextIndex`])입니다. 따라서 경계가
//! multi-byte 문자 중간에 놓일 수 없고, surrogate는 Rust `char`가 표현할 수 없어
//! 존재 자체가 불가능합니다. grapheme cluster(한글 조합형 자모, emoji ZWJ 시퀀스)
//! 단위의 커서·선택 이동은 text layout(Phase X)의 정책이며, 저장 경계는 scalar
//! 단위로 고정합니다. 내용은 정규화하지 않고 그대로 보존합니다(III.3.4 `UserText`).
//!
//! paragraph는 `'\n'`으로 구분합니다. paragraph 목록 길이는 항상 `'\n'` 개수 + 1입니다.

use std::fmt;

use stellar_core_types::color::Color;
use stellar_core_types::ids::StableId;
use stellar_core_types::text::UserText;
use stellar_core_types::value::FiniteF32;

/// Unicode scalar value 단위 text offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextIndex(pub u32);

/// `[start, end)` scalar 범위.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextRange {
    /// 시작(포함).
    pub start: TextIndex,
    /// 끝(제외).
    pub end: TextIndex,
}

impl TextRange {
    /// 범위를 만듭니다. `start > end`이면 `None`.
    pub fn new(start: u32, end: u32) -> Option<TextRange> {
        (start <= end).then_some(TextRange {
            start: TextIndex(start),
            end: TextIndex(end),
        })
    }

    /// scalar 길이.
    pub fn len(&self) -> u32 {
        self.end.0 - self.start.0
    }

    /// 빈 범위인지.
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// style run: 범위와 style 표 index(style 정의는 VI.7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StyleRun {
    /// 적용 범위.
    pub range: TextRange,
    /// style 표 index.
    pub style: u32,
}

/// paragraph: paragraph style 표 index(속성 정의는 VI.7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Paragraph {
    /// paragraph style 표 index.
    pub style: u32,
}

/// Text payload 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextError {
    /// 범위가 내용 길이를 넘습니다.
    RangeOutOfBounds {
        /// 문제 범위.
        range: TextRange,
        /// 내용 scalar 길이.
        len: u32,
    },
    /// paragraph 수가 `'\n'` 개수 + 1과 다릅니다.
    ParagraphCountMismatch {
        /// 기대 개수.
        expected: usize,
        /// 실제 개수.
        actual: usize,
    },
    /// 내용이 UTF-8이 아닙니다(인코딩된 surrogate 포함).
    InvalidUtf8,
    /// 입력이 부족하거나 형식이 아닙니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
}

impl fmt::Display for TextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TextError::RangeOutOfBounds { range, len } => write!(
                f,
                "text 범위 [{}, {})가 길이 {len}를 넘습니다",
                range.start.0, range.end.0
            ),
            TextError::ParagraphCountMismatch { expected, actual } => {
                write!(f, "paragraph 수가 {expected}이어야 하는데 {actual}입니다")
            }
            TextError::InvalidUtf8 => f.write_str("text 내용이 UTF-8이 아닙니다"),
            TextError::Malformed { reason } => write!(f, "Text binary 형식 위반: {reason}"),
        }
    }
}

impl std::error::Error for TextError {}

/// Text node payload (VI.7.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextPayload {
    content: UserText,
    paragraphs: Vec<Paragraph>,
    style_runs: Vec<StyleRun>,
}

/// 내용의 scalar 길이.
pub fn scalar_len(text: &str) -> u32 {
    text.chars().count() as u32
}

/// scalar offset을 byte offset으로 변환합니다. 범위 밖이면 `None`.
pub fn byte_offset(text: &str, index: TextIndex) -> Option<usize> {
    if index.0 == scalar_len(text) {
        return Some(text.len());
    }
    text.char_indices().nth(index.0 as usize).map(|(b, _)| b)
}

impl TextPayload {
    /// 검증 후 만듭니다.
    pub fn new(
        content: UserText,
        paragraphs: Vec<Paragraph>,
        style_runs: Vec<StyleRun>,
    ) -> Result<TextPayload, TextError> {
        let expected = content.as_str().matches('\n').count() + 1;
        if paragraphs.len() != expected {
            return Err(TextError::ParagraphCountMismatch {
                expected,
                actual: paragraphs.len(),
            });
        }
        let len = scalar_len(content.as_str());
        for run in &style_runs {
            if run.range.end.0 > len {
                return Err(TextError::RangeOutOfBounds {
                    range: run.range,
                    len,
                });
            }
        }
        Ok(TextPayload {
            content,
            paragraphs,
            style_runs,
        })
    }

    /// 내용(정규화 없이 보존).
    pub fn content(&self) -> &str {
        self.content.as_str()
    }

    /// paragraph 목록.
    pub fn paragraphs(&self) -> &[Paragraph] {
        &self.paragraphs
    }

    /// style run 목록.
    pub fn style_runs(&self) -> &[StyleRun] {
        &self.style_runs
    }

    /// 범위의 실제 문자열 slice.
    pub fn slice(&self, range: TextRange) -> Option<&str> {
        let text = self.content.as_str();
        let start = byte_offset(text, range.start)?;
        let end = byte_offset(text, range.end)?;
        text.get(start..end)
    }

    /// canonical bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        let bytes = self.content.as_str().as_bytes();
        buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(bytes);
        buf.extend_from_slice(&(self.paragraphs.len() as u32).to_le_bytes());
        for paragraph in &self.paragraphs {
            buf.extend_from_slice(&paragraph.style.to_le_bytes());
        }
        buf.extend_from_slice(&(self.style_runs.len() as u32).to_le_bytes());
        for run in &self.style_runs {
            buf.extend_from_slice(&run.range.start.0.to_le_bytes());
            buf.extend_from_slice(&run.range.end.0.to_le_bytes());
            buf.extend_from_slice(&run.style.to_le_bytes());
        }
        buf
    }

    /// bytes에서 복원하고 검증합니다.
    pub fn decode(bytes: &[u8]) -> Result<TextPayload, TextError> {
        let mut pos = 0usize;
        let mut take = |n: usize| -> Result<&[u8], TextError> {
            if bytes.len() - pos < n {
                return Err(TextError::Malformed {
                    reason: "입력 부족",
                });
            }
            let slice = &bytes[pos..pos + n];
            pos += n;
            Ok(slice)
        };
        let u32_of = |s: &[u8]| u32::from_le_bytes(s.try_into().expect("4바이트"));

        let len = u32_of(take(4)?) as usize;
        let content = std::str::from_utf8(take(len)?).map_err(|_| TextError::InvalidUtf8)?;
        let content = UserText::new(content);

        let paragraph_count = u32_of(take(4)?) as usize;
        let mut paragraphs = Vec::with_capacity(paragraph_count.min(4096));
        for _ in 0..paragraph_count {
            paragraphs.push(Paragraph {
                style: u32_of(take(4)?),
            });
        }
        let run_count = u32_of(take(4)?) as usize;
        let mut style_runs = Vec::with_capacity(run_count.min(4096));
        for _ in 0..run_count {
            let start = u32_of(take(4)?);
            let end = u32_of(take(4)?);
            let style = u32_of(take(4)?);
            let range = TextRange::new(start, end).ok_or(TextError::Malformed {
                reason: "start > end",
            })?;
            style_runs.push(StyleRun { range, style });
        }
        if pos != bytes.len() {
            return Err(TextError::Malformed {
                reason: "trailing bytes",
            });
        }
        TextPayload::new(content, paragraphs, style_runs)
    }
}

// --- style run 속성 (VI.7.2) ---

/// OpenType 4글자 tag(예: `liga`, `wght`). 출력 가능한 ASCII(0x20..=0x7E)만 허용합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OpenTypeTag([u8; 4]);

impl OpenTypeTag {
    /// 4바이트 tag를 만듭니다. 비ASCII·제어 문자는 거부합니다.
    pub fn new(tag: [u8; 4]) -> Option<OpenTypeTag> {
        tag.iter()
            .all(|b| (0x20..=0x7E).contains(b))
            .then_some(OpenTypeTag(tag))
    }

    /// 원본 bytes.
    pub fn bytes(&self) -> [u8; 4] {
        self.0
    }
}

/// style run 하나가 적용하는 문자 style.
///
/// canonical 형식: feature·variation은 tag 오름차순, tag당 하나(마지막 값 우선).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextStyle {
    /// typography(글꼴 family·크기 등) 정의 참조. 없으면 paragraph/문서 기본.
    pub typography: Option<StableId>,
    /// 글자 채우기 색. 없으면 상속.
    pub fill: Option<Color>,
    /// OpenType feature 설정(tag, 값).
    features: Vec<(OpenTypeTag, u32)>,
    /// variable font axis 값(tag, 값).
    variations: Vec<(OpenTypeTag, FiniteF32)>,
}

impl TextStyle {
    /// canonical style을 만듭니다(feature·variation 정렬, 중복 tag는 마지막 값 우선).
    pub fn new(
        typography: Option<StableId>,
        fill: Option<Color>,
        features: Vec<(OpenTypeTag, u32)>,
        variations: Vec<(OpenTypeTag, FiniteF32)>,
    ) -> TextStyle {
        fn canonical<V: Copy>(items: Vec<(OpenTypeTag, V)>) -> Vec<(OpenTypeTag, V)> {
            let mut map = std::collections::BTreeMap::new();
            for (tag, value) in items {
                map.insert(tag, value);
            }
            map.into_iter().collect()
        }
        TextStyle {
            typography,
            fill,
            features: canonical(features),
            variations: canonical(variations),
        }
    }

    /// OpenType feature 설정(정렬됨).
    pub fn features(&self) -> &[(OpenTypeTag, u32)] {
        &self.features
    }

    /// variation axis 값(정렬됨).
    pub fn variations(&self) -> &[(OpenTypeTag, FiniteF32)] {
        &self.variations
    }
}

/// 겹치는 style run을 canonical non-overlap run으로 정규화합니다(VI.7.2).
///
/// 규칙:
/// - 겹치는 구간은 **목록에서 나중 run이 우선**합니다(순차 적용 의미).
/// - 결과는 시작 오름차순이며 서로 겹치지 않습니다. 빈 run은 제거합니다.
/// - 인접하고 style **값**이 같은 run은 하나로 합칩니다(index가 달라도 값이 같으면
///   병합하고 앞쪽 index를 씁니다).
/// - run이 덮지 않는 구간은 run 없이 남깁니다(기본 style).
pub fn normalize_style_runs(runs: &[StyleRun], styles: &[TextStyle]) -> Vec<StyleRun> {
    let mut bounds: Vec<u32> = runs
        .iter()
        .filter(|r| !r.range.is_empty())
        .flat_map(|r| [r.range.start.0, r.range.end.0])
        .collect();
    bounds.sort_unstable();
    bounds.dedup();

    let same_style = |a: u32, b: u32| {
        a == b
            || matches!(
                (styles.get(a as usize), styles.get(b as usize)),
                (Some(x), Some(y)) if x == y
            )
    };

    let mut out: Vec<StyleRun> = Vec::new();
    for window in bounds.windows(2) {
        let (start, end) = (window[0], window[1]);
        // 이 구간을 덮는 마지막 run.
        let winner = runs
            .iter()
            .rev()
            .find(|r| r.range.start.0 <= start && end <= r.range.end.0 && !r.range.is_empty());
        let Some(winner) = winner else {
            continue;
        };
        match out.last_mut() {
            Some(last) if last.range.end.0 == start && same_style(last.style, winner.style) => {
                last.range.end = TextIndex(end);
            }
            _ => out.push(StyleRun {
                range: TextRange {
                    start: TextIndex(start),
                    end: TextIndex(end),
                },
                style: winner.style,
            }),
        }
    }
    out
}

// --- paragraph 속성 (VI.7.3) ---

/// paragraph 정렬. 기본은 Start(쓰기 방향의 시작).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum TextAlignment {
    /// 시작 정렬.
    #[default]
    Start = 0,
    /// 가운데.
    Center = 1,
    /// 끝 정렬.
    End = 2,
    /// 양쪽 정렬.
    Justify = 3,
}

/// 줄 높이. 기본은 Auto(글꼴 metric 기반, layout이 결정).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineHeight {
    /// 글꼴 metric 기반.
    #[default]
    Auto,
    /// 글꼴 크기 배수(0 < 값 ≤ [`MAX_LINE_HEIGHT_MULTIPLIER`]).
    Multiplier(FiniteF32),
    /// 고정 논리 픽셀(0 < 값 ≤ [`MAX_FIXED_LINE_HEIGHT`]).
    Fixed(FiniteF32),
}

/// 줄바꿈 방식. 기본은 Wrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum WrappingMode {
    /// 상자 너비에서 줄바꿈.
    #[default]
    Wrap = 0,
    /// 명시 줄바꿈(`'\n'`)에서만 줄을 나눔.
    NoWrap = 1,
}

/// 줄 높이 배수 상한.
pub const MAX_LINE_HEIGHT_MULTIPLIER: f32 = 10.0;
/// 고정 줄 높이 상한(논리 픽셀).
pub const MAX_FIXED_LINE_HEIGHT: f32 = 10_000.0;
/// 자간 하한(글꼴 크기 대비 비율, em).
pub const MIN_LETTER_SPACING_EM: f32 = -1.0;
/// 자간 상한(em).
pub const MAX_LETTER_SPACING_EM: f32 = 10.0;

/// paragraph 속성 검증 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParagraphStyleError {
    /// 줄 높이가 범위를 벗어납니다.
    LineHeightOutOfRange,
    /// 자간이 범위를 벗어납니다.
    LetterSpacingOutOfRange,
}

impl fmt::Display for ParagraphStyleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParagraphStyleError::LineHeightOutOfRange => {
                f.write_str("줄 높이가 허용 범위를 벗어납니다")
            }
            ParagraphStyleError::LetterSpacingOutOfRange => {
                f.write_str("자간이 허용 범위를 벗어납니다")
            }
        }
    }
}

impl std::error::Error for ParagraphStyleError {}

/// paragraph 속성. 실제 layout은 Phase X에서 구현합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParagraphStyle {
    /// 정렬.
    pub alignment: TextAlignment,
    /// 줄 높이.
    pub line_height: LineHeight,
    /// 자간(em, 글꼴 크기 대비 비율). 기본 0.
    pub letter_spacing_em: FiniteF32,
    /// 줄바꿈 방식.
    pub wrapping: WrappingMode,
}

impl Default for ParagraphStyle {
    /// Start 정렬, Auto 줄 높이, 자간 0, Wrap.
    fn default() -> ParagraphStyle {
        ParagraphStyle {
            alignment: TextAlignment::default(),
            line_height: LineHeight::default(),
            letter_spacing_em: FiniteF32::ZERO,
            wrapping: WrappingMode::default(),
        }
    }
}

impl ParagraphStyle {
    /// 경계값을 검증합니다.
    pub fn validate(&self) -> Result<(), ParagraphStyleError> {
        match self.line_height {
            LineHeight::Auto => {}
            LineHeight::Multiplier(v) => {
                if !(v.get() > 0.0 && v.get() <= MAX_LINE_HEIGHT_MULTIPLIER) {
                    return Err(ParagraphStyleError::LineHeightOutOfRange);
                }
            }
            LineHeight::Fixed(v) => {
                if !(v.get() > 0.0 && v.get() <= MAX_FIXED_LINE_HEIGHT) {
                    return Err(ParagraphStyleError::LineHeightOutOfRange);
                }
            }
        }
        let ls = self.letter_spacing_em.get();
        if !(MIN_LETTER_SPACING_EM..=MAX_LETTER_SPACING_EM).contains(&ls) {
            return Err(ParagraphStyleError::LetterSpacingOutOfRange);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single_paragraph(text: &str, runs: Vec<StyleRun>) -> TextPayload {
        let paragraphs = vec![Paragraph { style: 0 }; text.matches('\n').count() + 1];
        TextPayload::new(UserText::new(text), paragraphs, runs).unwrap()
    }

    fn run(start: u32, end: u32, style: u32) -> StyleRun {
        StyleRun {
            range: TextRange::new(start, end).unwrap(),
            style,
        }
    }

    #[test]
    fn hangul_composition_round_trips_without_normalization() {
        // 검증 기준(VI.7.1): 한글 조합 fixture round-trip.
        let composed = "한글"; // U+D55C U+AE00
        let decomposed = "\u{1112}\u{1161}\u{11AB}"; // 한 = ㅎ+ㅏ+ㄴ(조합형 자모)
        let payload = single_paragraph(
            &format!("{composed}{decomposed}"),
            vec![run(0, 2, 1), run(2, 5, 2)],
        );
        let decoded = TextPayload::decode(&payload.encode()).unwrap();
        assert_eq!(decoded, payload);
        // 정규화하지 않음: 조합형 자모가 그대로 3 scalar.
        assert_eq!(scalar_len(decoded.content()), 5);
        assert_eq!(
            decoded.slice(TextRange::new(2, 5).unwrap()),
            Some(decomposed)
        );
        assert_eq!(decoded.slice(TextRange::new(0, 2).unwrap()), Some(composed));
    }

    #[test]
    fn emoji_zwj_sequence_round_trips() {
        // 검증 기준(VI.7.1): emoji ZWJ fixture round-trip.
        let family = "👨\u{200D}👩\u{200D}👧"; // 5 scalar(3 emoji + 2 ZWJ)
        let text = format!("a{family}b");
        let payload = single_paragraph(&text, vec![run(1, 6, 3)]);
        let decoded = TextPayload::decode(&payload.encode()).unwrap();
        assert_eq!(decoded, payload);
        assert_eq!(scalar_len(&text), 7);
        assert_eq!(decoded.slice(TextRange::new(1, 6).unwrap()), Some(family));
    }

    #[test]
    fn encoded_surrogate_is_impossible() {
        // 검증 기준(VI.7.1): surrogate 불가 fixture.
        let payload = single_paragraph("ab", vec![]);
        let mut bytes = payload.encode();
        // 내용 "ab"(2바이트)를 길이 3의 인코딩된 surrogate U+D800(ED A0 80)으로 교체.
        bytes.splice(0..6, [3, 0, 0, 0, 0xED, 0xA0, 0x80]);
        assert_eq!(
            TextPayload::decode(&bytes).unwrap_err(),
            TextError::InvalidUtf8
        );
    }

    #[test]
    fn index_is_scalar_not_byte() {
        let text = "가a";
        assert_eq!(byte_offset(text, TextIndex(1)), Some(3), "'가'는 3바이트");
        assert_eq!(byte_offset(text, TextIndex(2)), Some(4));
        assert_eq!(byte_offset(text, TextIndex(3)), None);
    }

    #[test]
    fn validation_rejects_bad_ranges_and_paragraphs() {
        assert!(matches!(
            TextPayload::new(
                UserText::new("abc"),
                vec![Paragraph { style: 0 }],
                vec![run(0, 4, 0)]
            ),
            Err(TextError::RangeOutOfBounds { len: 3, .. })
        ));
        assert_eq!(
            TextPayload::new(UserText::new("a\nb"), vec![Paragraph { style: 0 }], vec![])
                .unwrap_err(),
            TextError::ParagraphCountMismatch {
                expected: 2,
                actual: 1
            }
        );
        assert!(TextRange::new(5, 2).is_none());
        let payload = single_paragraph("a\nb", vec![run(0, 3, 0)]);
        assert_eq!(payload.paragraphs().len(), 2);
        let mut bytes = payload.encode();
        bytes.push(0);
        assert!(TextPayload::decode(&bytes).is_err());
    }

    fn tag(s: &str) -> OpenTypeTag {
        OpenTypeTag::new(s.as_bytes().try_into().unwrap()).unwrap()
    }

    fn style(seed: u64) -> TextStyle {
        TextStyle::new(Some(StableId::new_v7(seed, 0, seed)), None, vec![], vec![])
    }

    #[test]
    fn overlapping_runs_normalize_to_canonical_non_overlap() {
        // 검증 기준(VI.7.2): overlapping run을 canonical non-overlap run으로 normalize.
        let styles = vec![style(1), style(2), style(3)];
        // 0..10 style0, 그 위에 3..6 style1, 그 위에 5..8 style2(나중 우선).
        let runs = vec![run(0, 10, 0), run(3, 6, 1), run(5, 8, 2)];
        let normalized = normalize_style_runs(&runs, &styles);
        assert_eq!(
            normalized,
            vec![run(0, 3, 0), run(3, 5, 1), run(5, 8, 2), run(8, 10, 0)]
        );
        // 결과는 겹치지 않고 정렬됨.
        for pair in normalized.windows(2) {
            assert!(pair[0].range.end <= pair[1].range.start);
        }
        // 멱등.
        assert_eq!(normalize_style_runs(&normalized, &styles), normalized);
    }

    #[test]
    fn equal_style_values_merge_and_empty_runs_drop() {
        // index 0과 1은 값이 같음 → 인접하면 병합.
        let styles = vec![style(1), style(1), style(2)];
        let runs = vec![run(0, 4, 0), run(4, 8, 1), run(8, 8, 2), run(10, 12, 2)];
        assert_eq!(
            normalize_style_runs(&runs, &styles),
            vec![run(0, 8, 0), run(10, 12, 2)],
            "값 동일 병합, 빈 run 제거, 덮지 않은 8..10은 run 없음"
        );
        assert!(normalize_style_runs(&[], &styles).is_empty());
    }

    #[test]
    fn style_attributes_are_canonical() {
        let a = TextStyle::new(
            None,
            None,
            vec![(tag("liga"), 0), (tag("kern"), 1), (tag("liga"), 1)],
            vec![
                (tag("wght"), FiniteF32::new(700.0).unwrap()),
                (tag("ital"), FiniteF32::new(1.0).unwrap()),
            ],
        );
        assert_eq!(
            a.features(),
            &[(tag("kern"), 1), (tag("liga"), 1)],
            "정렬 + 마지막 값 우선"
        );
        assert_eq!(a.variations()[0].0, tag("ital"));
        assert!(
            OpenTypeTag::new([0x00, b'a', b'b', b'c']).is_none(),
            "제어 문자 거부"
        );
        assert!(
            OpenTypeTag::new([0xC3, 0xA9, b'a', b'b']).is_none(),
            "비ASCII 거부"
        );
    }

    fn finite(v: f32) -> FiniteF32 {
        FiniteF32::new(v).unwrap()
    }

    #[test]
    fn paragraph_style_defaults_are_valid() {
        // 검증 기준(VI.7.3): 기본값 validation 통과.
        let d = ParagraphStyle::default();
        assert_eq!(d.alignment, TextAlignment::Start);
        assert_eq!(d.line_height, LineHeight::Auto);
        assert_eq!(d.letter_spacing_em.get(), 0.0);
        assert_eq!(d.wrapping, WrappingMode::Wrap);
        assert!(d.validate().is_ok());
    }

    #[test]
    fn paragraph_style_boundary_values() {
        // 검증 기준(VI.7.3): 경계값 validation 통과.
        let with = |line_height, ls: f32| ParagraphStyle {
            line_height,
            letter_spacing_em: finite(ls),
            ..ParagraphStyle::default()
        };
        // 줄 높이 배수: 0 초과 ~ 10 이하.
        assert!(with(
            LineHeight::Multiplier(finite(MAX_LINE_HEIGHT_MULTIPLIER)),
            0.0
        )
        .validate()
        .is_ok());
        assert!(with(LineHeight::Multiplier(finite(0.0001)), 0.0)
            .validate()
            .is_ok());
        for bad in [0.0, -1.0, MAX_LINE_HEIGHT_MULTIPLIER + 0.01] {
            assert_eq!(
                with(LineHeight::Multiplier(finite(bad)), 0.0)
                    .validate()
                    .unwrap_err(),
                ParagraphStyleError::LineHeightOutOfRange
            );
        }
        // 고정 줄 높이: 0 초과 ~ 10000 이하.
        assert!(with(LineHeight::Fixed(finite(MAX_FIXED_LINE_HEIGHT)), 0.0)
            .validate()
            .is_ok());
        assert!(with(LineHeight::Fixed(finite(0.0)), 0.0)
            .validate()
            .is_err());
        assert!(
            with(LineHeight::Fixed(finite(MAX_FIXED_LINE_HEIGHT + 1.0)), 0.0)
                .validate()
                .is_err()
        );
        // 자간: [-1, 10] em.
        assert!(with(LineHeight::Auto, MIN_LETTER_SPACING_EM)
            .validate()
            .is_ok());
        assert!(with(LineHeight::Auto, MAX_LETTER_SPACING_EM)
            .validate()
            .is_ok());
        assert_eq!(
            with(LineHeight::Auto, -1.01).validate().unwrap_err(),
            ParagraphStyleError::LetterSpacingOutOfRange
        );
        assert!(with(LineHeight::Auto, 10.01).validate().is_err());
    }
}
