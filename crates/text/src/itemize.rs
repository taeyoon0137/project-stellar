//! script·language·direction itemization (X.9.2).
//!
//! paragraph 텍스트와 style span을 shaping run([`TextItem`]) 목록으로 나눕니다. 한 item 안에서는
//! style, script, BiDi level, 언어가 모두 같으므로 [`crate::shaping::shape`] 요청 하나가 됩니다.
//!
//! - **style span**: [`StyleSpan`]은 paragraph를 빈틈·겹침 없이 앞에서부터 덮어야 합니다(문서 모델의
//!   canonical style run을 byte 범위로 옮긴 값). 범위는 UTF-8 byte offset이며 문자 경계여야 합니다.
//!   문서 모델의 scalar index(VI.7.1)를 byte offset으로 바꾸는 일은 호출자 몫입니다.
//! - **script 해석**: 문자 script는 shaping 엔진과 같은 Unicode 데이터(`harfrust::unicode::script`)를
//!   씁니다. Inherited(결합 부호)는 앞 문자를, Common(공백·숫자·구두점)은 앞의 실제 script를
//!   따릅니다. 짝 괄호(UAX #9 BidiBrackets)의 닫는 괄호는 짝 여는 괄호의 script를 따릅니다. 앞에
//!   실제 script가 없는 Common·Inherited는 뒤의 첫 실제 script를, 텍스트 전체에 실제 script가 없으면
//!   Common으로 남습니다.
//! - **방향**: [`BidiParagraph`] resolved level이 바뀌는 곳에서 나눕니다(level이 홀수면 RTL).
//! - **언어**: span의 언어가 바뀌는 곳에서 나눕니다. 언어는 명시 입력이며 실행 환경 locale을 쓰지
//!   않습니다.
//!
//! 결과는 입력만으로 정해지는 결정론적 목록이고, 논리 순서(텍스트 순서)입니다.

use std::fmt;
use std::ops::Range;

use unicode_bidi::{BidiDataSource, HardcodedBidiData};

use crate::bidi::{BaseDirection, BidiLevel, BidiParagraph};
use crate::shaping::{LanguageTag, ScriptTag, TextDirection};

/// 짝 괄호 추적 stack 깊이 상한(UAX #9 BD16과 같은 63). 넘으면 더 이상 짝을 추적하지 않습니다.
const MAX_BRACKET_DEPTH: usize = 63;

/// style 구간 하나.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StyleSpan {
    /// paragraph 안 byte 범위.
    pub range: Range<usize>,
    /// style 표 index.
    pub style: u32,
    /// 이 구간의 언어(없으면 언어 미지정).
    pub language: Option<LanguageTag>,
}

/// shaping run 하나.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextItem {
    /// paragraph 안 byte 범위.
    pub range: Range<usize>,
    /// style 표 index.
    pub style: u32,
    /// 해석된 script.
    pub script: ScriptTag,
    /// BiDi resolved level.
    pub level: BidiLevel,
    /// 언어.
    pub language: Option<LanguageTag>,
}

impl TextItem {
    /// 쓰기 방향(level 홀짝).
    pub fn direction(&self) -> TextDirection {
        self.level.direction()
    }
}

/// itemization 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Itemization {
    /// 논리 순서 item.
    pub items: Vec<TextItem>,
    /// BiDi 해석(줄 재배치·caret이 다시 씁니다).
    pub bidi: BidiParagraph,
}

/// itemization 입력 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemizeError {
    /// 텍스트가 있는데 span이 없습니다.
    MissingSpans,
    /// span이 앞 span 끝에서 시작하지 않습니다(빈틈·겹침·역순).
    NotContiguous {
        /// span index.
        index: usize,
        /// 기대한 시작.
        expected_start: usize,
        /// 실제 시작.
        found_start: usize,
    },
    /// 빈 span이 있습니다(빈 텍스트의 단일 빈 span만 허용).
    EmptySpan {
        /// span index.
        index: usize,
    },
    /// span 경계가 문자 경계가 아니거나 텍스트를 벗어납니다.
    InvalidBoundary {
        /// 위반 offset.
        offset: usize,
    },
    /// span이 텍스트 끝까지 덮지 않습니다.
    UncoveredTail {
        /// 마지막 span 끝.
        covered_end: usize,
        /// 텍스트 byte 길이.
        len: usize,
    },
    /// byte offset을 `u32` cluster로 표현할 수 없을 만큼 깁니다.
    TextTooLong {
        /// 텍스트 byte 길이.
        len: usize,
    },
}

impl fmt::Display for ItemizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ItemizeError::MissingSpans => f.write_str("텍스트에 style span이 없습니다"),
            ItemizeError::NotContiguous {
                index,
                expected_start,
                found_start,
            } => write!(
                f,
                "style span {index}이(가) {expected_start}에서 시작해야 하지만 {found_start}에서 시작합니다"
            ),
            ItemizeError::EmptySpan { index } => write!(f, "style span {index}이(가) 비어 있습니다"),
            ItemizeError::InvalidBoundary { offset } => {
                write!(f, "style span 경계 {offset}이(가) 문자 경계가 아닙니다")
            }
            ItemizeError::UncoveredTail { covered_end, len } => write!(
                f,
                "style span이 {covered_end}까지만 덮고 텍스트 길이는 {len}입니다"
            ),
            ItemizeError::TextTooLong { len } => {
                write!(f, "텍스트가 너무 깁니다: {len} bytes(최대 u32)")
            }
        }
    }
}

impl std::error::Error for ItemizeError {}

/// span 목록이 텍스트를 정확히 덮는지 검증합니다.
pub(crate) fn validate_spans(text: &str, spans: &[StyleSpan]) -> Result<(), ItemizeError> {
    if u32::try_from(text.len()).is_err() {
        return Err(ItemizeError::TextTooLong { len: text.len() });
    }
    if text.is_empty() {
        return match spans {
            [] => Ok(()),
            [only] if only.range == (0..0) => Ok(()),
            [only] => Err(ItemizeError::InvalidBoundary {
                offset: only.range.end,
            }),
            _ => Err(ItemizeError::EmptySpan { index: 1 }),
        };
    }
    if spans.is_empty() {
        return Err(ItemizeError::MissingSpans);
    }
    let mut expected = 0;
    for (index, span) in spans.iter().enumerate() {
        if span.range.start != expected {
            return Err(ItemizeError::NotContiguous {
                index,
                expected_start: expected,
                found_start: span.range.start,
            });
        }
        if span.range.end <= span.range.start {
            return Err(ItemizeError::EmptySpan { index });
        }
        if span.range.end > text.len() || !text.is_char_boundary(span.range.end) {
            return Err(ItemizeError::InvalidBoundary {
                offset: span.range.end,
            });
        }
        expected = span.range.end;
    }
    if expected != text.len() {
        return Err(ItemizeError::UncoveredTail {
            covered_end: expected,
            len: text.len(),
        });
    }
    Ok(())
}

/// 문자 하나의 Unicode script(shaping 엔진 데이터).
pub fn char_script(c: char) -> ScriptTag {
    let bytes = harfrust::unicode::script(u32::from(c)).tag().to_be_bytes();
    ScriptTag::new(bytes).unwrap_or(ScriptTag::UNKNOWN)
}

/// 문자별 해석 script(문자 순서). 규칙은 모듈 문서 참조.
pub fn resolve_scripts(text: &str) -> Vec<ScriptTag> {
    let chars: Vec<char> = text.chars().collect();
    let mut resolved: Vec<Option<ScriptTag>> = Vec::with_capacity(chars.len());
    let mut last_real: Option<ScriptTag> = None;
    // (정규화된 여는 괄호, 그 시점 script)
    let mut brackets: Vec<(char, Option<ScriptTag>)> = Vec::new();
    for &c in &chars {
        let raw = char_script(c);
        let script = if raw.is_real() {
            last_real = Some(raw);
            Some(raw)
        } else if raw == ScriptTag::INHERITED {
            resolved.last().copied().flatten().or(last_real)
        } else {
            match HardcodedBidiData.bidi_matched_opening_bracket(c) {
                Some(pair) if pair.is_open => {
                    if brackets.len() < MAX_BRACKET_DEPTH {
                        brackets.push((pair.opening, last_real));
                    }
                    last_real
                }
                Some(pair) => {
                    // 짝이 맞는 가장 가까운 여는 괄호까지 stack을 닫습니다.
                    match brackets.iter().rposition(|(open, _)| *open == pair.opening) {
                        Some(pos) => {
                            let script = brackets[pos].1;
                            brackets.truncate(pos);
                            script.or(last_real)
                        }
                        None => last_real,
                    }
                }
                None => last_real,
            }
        };
        resolved.push(script);
    }
    // 앞에 실제 script가 없던 문자는 뒤의 첫 실제 script를 씁니다.
    let mut next_real: Option<ScriptTag> = None;
    let mut out = vec![ScriptTag::COMMON; resolved.len()];
    for i in (0..resolved.len()).rev() {
        if let Some(script) = resolved[i] {
            next_real = Some(script);
            out[i] = script;
        } else {
            out[i] = next_real.unwrap_or(ScriptTag::COMMON);
        }
    }
    out
}

/// paragraph 텍스트와 style span을 shaping run으로 나눕니다 (X.9.2).
pub fn itemize(
    text: &str,
    spans: &[StyleSpan],
    base: BaseDirection,
) -> Result<Itemization, ItemizeError> {
    validate_spans(text, spans)?;
    let bidi = BidiParagraph::analyze(text, base);
    let scripts = resolve_scripts(text);
    let mut items: Vec<TextItem> = Vec::new();
    let mut span_index = 0;
    for ((offset, c), script) in text.char_indices().zip(scripts) {
        while spans[span_index].range.end <= offset {
            span_index += 1;
        }
        let span = &spans[span_index];
        let level = bidi.level_at(offset);
        let end = offset + c.len_utf8();
        match items.last_mut() {
            Some(item)
                if item.range.end == offset
                    && item.style == span.style
                    && item.script == script
                    && item.level == level
                    && item.language == span.language
                    && span.range.start < offset =>
            {
                item.range.end = end;
            }
            _ => items.push(TextItem {
                range: offset..end,
                style: span.style,
                script,
                level,
                language: span.language.clone(),
            }),
        }
    }
    Ok(Itemization { items, bidi })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn whole(text: &str, style: u32) -> Vec<StyleSpan> {
        vec![StyleSpan {
            range: 0..text.len(),
            style,
            language: None,
        }]
    }

    fn summary(items: &[TextItem]) -> Vec<(Range<usize>, String, u8)> {
        items
            .iter()
            .map(|i| (i.range.clone(), i.script.to_string(), i.level.number()))
            .collect()
    }

    #[test]
    fn latin_arabic_hangul_mixed_fixture() {
        let text = "Hello مرحبا 안녕하세요!";
        let result = itemize(text, &whole(text, 0), BaseDirection::LeftToRight).expect("itemize");
        assert_eq!(
            summary(&result.items),
            vec![
                // Common 공백은 앞 script(Latn)를 따릅니다.
                (0..6, "Latn".to_string(), 0),
                (6..16, "Arab".to_string(), 1),
                // 아랍어 뒤 공백: script는 앞(Arab)을 따르지만 L·R 사이 중립이라 level 0.
                (16..17, "Arab".to_string(), 0),
                // 끝의 '!'는 앞 script(Hang)를 따릅니다.
                (17..33, "Hang".to_string(), 0),
            ]
        );
        assert_eq!(
            result
                .items
                .iter()
                .map(TextItem::direction)
                .collect::<Vec<_>>(),
            vec![
                TextDirection::LeftToRight,
                TextDirection::RightToLeft,
                TextDirection::LeftToRight,
                TextDirection::LeftToRight
            ]
        );
        assert_eq!(&text[6..16], "مرحبا");
        assert_eq!(&text[17..33], "안녕하세요!");
    }

    #[test]
    fn rtl_paragraph_with_numbers_and_latin() {
        let text = "عدد 123 abc";
        let result = itemize(text, &whole(text, 0), BaseDirection::RightToLeft).expect("itemize");
        assert_eq!(
            summary(&result.items),
            vec![
                (0..7, "Arab".to_string(), 1),
                // 아랍 문맥의 숫자는 AN(level 2), script는 앞 Arab.
                (7..10, "Arab".to_string(), 2),
                // 숫자 뒤 공백: R(AN)·L 사이 중립 → 문단 방향 RTL(1).
                (10..11, "Arab".to_string(), 1),
                (11..14, "Latn".to_string(), 2),
            ]
        );
        assert_eq!(result.bidi.base_level(), BidiLevel::RTL);
    }

    #[test]
    fn style_and_language_boundaries_split_items() {
        let text = "ab cd";
        let ko = LanguageTag::parse("ko").expect("ko");
        let spans = vec![
            StyleSpan {
                range: 0..1,
                style: 0,
                language: None,
            },
            StyleSpan {
                range: 1..3,
                style: 1,
                language: None,
            },
            StyleSpan {
                range: 3..5,
                style: 1,
                language: Some(ko.clone()),
            },
        ];
        let result = itemize(text, &spans, BaseDirection::LeftToRight).expect("itemize");
        let shape: Vec<(Range<usize>, u32, Option<LanguageTag>)> = result
            .items
            .iter()
            .map(|i| (i.range.clone(), i.style, i.language.clone()))
            .collect();
        assert_eq!(
            shape,
            vec![(0..1, 0, None), (1..3, 1, None), (3..5, 1, Some(ko))]
        );
    }

    #[test]
    fn common_and_inherited_resolution() {
        // 앞에 실제 script가 없는 숫자·공백은 뒤 script(Hang)를 씁니다.
        let scripts = resolve_scripts("12 한");
        assert_eq!(scripts, vec![ScriptTag::HANGUL; 4]);
        // 결합 부호(U+0301)는 앞 문자 script.
        assert_eq!(
            resolve_scripts("e\u{0301}"),
            vec![ScriptTag::LATIN, ScriptTag::LATIN]
        );
        // 닫는 괄호는 짝 여는 괄호 시점 script를 따릅니다: "a(ب)" 의 ')'는 Latn.
        assert_eq!(
            resolve_scripts("a(ب)"),
            vec![
                ScriptTag::LATIN,
                ScriptTag::LATIN,
                ScriptTag::ARABIC,
                ScriptTag::LATIN
            ]
        );
        // 실제 script가 전혀 없으면 Common.
        assert_eq!(resolve_scripts("1 2"), vec![ScriptTag::COMMON; 3]);
        assert_eq!(char_script('한'), ScriptTag::HANGUL);
        assert_eq!(char_script('漢'), ScriptTag::HAN);
        assert_eq!(char_script('ア'), ScriptTag::KATAKANA);
        assert_eq!(char_script(' '), ScriptTag::COMMON);
        // 깊은 괄호 중첩도 stack 상한 안에서 처리됩니다.
        let deep = format!("a{}b{}", "(".repeat(500), ")".repeat(500));
        assert_eq!(resolve_scripts(&deep).len(), 1002);
    }

    #[test]
    fn itemization_is_deterministic() {
        let text = "Stellar 스텔라 ステラ 星 שלום";
        let a = itemize(text, &whole(text, 3), BaseDirection::Auto).expect("a");
        let b = itemize(text, &whole(text, 3), BaseDirection::Auto).expect("b");
        assert_eq!(a, b);
        let covered: usize = a.items.iter().map(|i| i.range.len()).sum();
        assert_eq!(covered, text.len());
        assert!(a
            .items
            .windows(2)
            .all(|w| w[0].range.end == w[1].range.start));
        assert!(a.items.iter().all(|i| i.style == 3));
    }

    #[test]
    fn invalid_spans_are_rejected() {
        let text = "a한b";
        let span = |range: Range<usize>| StyleSpan {
            range,
            style: 0,
            language: None,
        };
        assert_eq!(
            itemize(text, &[], BaseDirection::LeftToRight).map(|_| ()),
            Err(ItemizeError::MissingSpans)
        );
        assert_eq!(
            itemize(text, &[span(1..5)], BaseDirection::LeftToRight).map(|_| ()),
            Err(ItemizeError::NotContiguous {
                index: 0,
                expected_start: 0,
                found_start: 1
            })
        );
        assert_eq!(
            itemize(text, &[span(0..2)], BaseDirection::LeftToRight).map(|_| ()),
            Err(ItemizeError::InvalidBoundary { offset: 2 })
        );
        assert_eq!(
            itemize(text, &[span(0..1), span(1..1)], BaseDirection::LeftToRight).map(|_| ()),
            Err(ItemizeError::EmptySpan { index: 1 })
        );
        assert_eq!(
            itemize(text, &[span(0..4)], BaseDirection::LeftToRight).map(|_| ()),
            Err(ItemizeError::UncoveredTail {
                covered_end: 4,
                len: 5
            })
        );
        assert_eq!(
            itemize(text, &[span(0..1), span(0..5)], BaseDirection::LeftToRight).map(|_| ()),
            Err(ItemizeError::NotContiguous {
                index: 1,
                expected_start: 1,
                found_start: 0
            })
        );
        // 빈 텍스트: span 없음 또는 단일 빈 span만 허용, item 없음.
        assert!(itemize("", &[], BaseDirection::LeftToRight)
            .expect("빈 텍스트")
            .items
            .is_empty());
        assert!(itemize("", &[span(0..0)], BaseDirection::LeftToRight).is_ok());
        assert!(ItemizeError::UncoveredTail {
            covered_end: 4,
            len: 5
        }
        .to_string()
        .contains('5'));
    }
}
