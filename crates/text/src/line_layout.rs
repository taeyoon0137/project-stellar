//! shaping 결과의 줄 나누기 (X.10.1).
//!
//! [`break_lines`]는 [`ShapedParagraph`]의 원자 segment(shaping cluster)를 줄바꿈 기회(X.9.4)와 너비
//! 제약에 맞춰 줄로 나눕니다.
//!
//! - **cluster 보존**: 줄 경계는 segment 경계에서만 생깁니다. segment 안에 있는 줄바꿈 기회(예:
//!   ligature 안)는 쓰지 않습니다.
//! - **greedy**: 줄에 들어가는 마지막 기회에서 끊습니다. 줄 끝 hanging 공백은 폭에 넣지 않습니다(CSS
//!   `white-space: normal`). 강제 줄바꿈(Mandatory)은 항상 줄을 끝냅니다.
//! - **긴 단어**: 기회 하나 사이 구간이 너비보다 넓으면 segment 경계에서 비상 줄바꿈합니다(CSS
//!   `overflow-wrap: break-word`). 줄에는 항상 segment가 하나 이상 들어가므로 너비가 아주 좁아도 무한
//!   반복이 없습니다.
//! - [`WrapMode::NoWrap`] 또는 너비 없음: 강제 줄바꿈에서만 나눕니다.
//!
//! 계산은 정수 subpixel 합이라 결정론적입니다.

use std::ops::Range;

use crate::line_break::{BreakKind, BreakOpportunity};
use crate::paragraph::{ShapedParagraph, TextLayoutError};
use crate::units::TextUnit;

/// 줄바꿈 방식(문서 `WrappingMode`, VI.7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WrapMode {
    /// 너비에서 줄바꿈.
    #[default]
    Wrap,
    /// 강제 줄바꿈에서만.
    NoWrap,
}

/// 줄 하나.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LineBreak {
    /// paragraph byte 범위.
    pub range: Range<usize>,
    /// 줄 폭(끝 hanging 공백 제외).
    pub width: TextUnit,
    /// 끝 hanging 공백 폭.
    pub trailing_whitespace: TextUnit,
    /// 강제 줄바꿈(텍스트 끝 제외)으로 끝났는지.
    pub hard_break: bool,
}

/// 줄을 나눕니다 (X.10.1). `breaks`는 같은 텍스트의 줄바꿈 기회(X.9.4)입니다.
pub fn break_lines(
    paragraph: &ShapedParagraph,
    breaks: &[BreakOpportunity],
    max_width: Option<TextUnit>,
    wrap: WrapMode,
) -> Result<Vec<LineBreak>, TextLayoutError> {
    if max_width.is_some_and(|w| w < TextUnit::ZERO) {
        return Err(TextLayoutError::NegativeWidth);
    }
    let segments = paragraph.segments();
    let text_len = paragraph.text().len();
    if segments.is_empty() {
        return Ok(vec![LineBreak {
            range: 0..0,
            width: TextUnit::ZERO,
            trailing_whitespace: TextUnit::ZERO,
            hard_break: false,
        }]);
    }
    let limit = match (wrap, max_width) {
        (WrapMode::Wrap, Some(width)) => Some(i64::from(width.raw())),
        _ => None,
    };
    // segment 경계에 놓인 기회만 씁니다. offset → segment index(그 앞에서 끊음).
    let mut candidates: Vec<(usize, BreakKind)> = Vec::new();
    for b in breaks {
        let index = if b.offset >= text_len {
            segments.len()
        } else {
            match segments.binary_search_by_key(&b.offset, |s| s.range.start) {
                Ok(index) => index,
                Err(_) => continue,
            }
        };
        if index == 0 {
            continue;
        }
        match candidates.last_mut() {
            Some(last) if last.0 == index => {
                if b.kind == BreakKind::Mandatory {
                    last.1 = BreakKind::Mandatory;
                }
            }
            _ => candidates.push((index, b.kind)),
        }
    }
    if candidates.last().map(|c| c.0) != Some(segments.len()) {
        candidates.push((segments.len(), BreakKind::Mandatory));
    }

    let measure = Measure::new(paragraph);
    let mut lines = Vec::new();
    let mut start = 0usize;
    let mut cursor = 0usize; // candidates index
    while start < segments.len() {
        let mut last_fit: Option<(usize, BreakKind)> = None;
        let mut chosen: Option<(usize, bool)> = None;
        while cursor < candidates.len() {
            let (end, kind) = candidates[cursor];
            if end <= start {
                cursor += 1;
                continue;
            }
            let (width, _) = measure.span(start, end);
            let fits = limit.is_none_or(|l| width <= l);
            if fits {
                last_fit = Some((end, kind));
                cursor += 1;
                if kind == BreakKind::Mandatory {
                    chosen = Some((end, end != segments.len()));
                    break;
                }
                continue;
            }
            chosen = Some(match last_fit {
                Some((fit_end, fit_kind)) => (
                    fit_end,
                    fit_kind == BreakKind::Mandatory && fit_end != segments.len(),
                ),
                None => (
                    measure.emergency_end(start, end, limit.unwrap_or(i64::MAX)),
                    false,
                ),
            });
            break;
        }
        let (end, hard_break) = match chosen
            .or(last_fit.map(|(e, k)| (e, k == BreakKind::Mandatory && e != segments.len())))
        {
            Some(choice) => choice,
            None => (segments.len(), false),
        };
        let (width, trailing) = measure.span(start, end);
        lines.push(LineBreak {
            range: segments[start].range.start..segments[end - 1].range.end,
            width: TextUnit::from_i64(width)?,
            trailing_whitespace: TextUnit::from_i64(trailing)?,
            hard_break,
        });
        start = end;
    }
    Ok(lines)
}

/// segment 구간 폭을 O(1)로 재는 prefix 합.
struct Measure {
    /// `prefix[i]` = segment `[0, i)` advance 합.
    prefix: Vec<i64>,
    /// `content_end[i]` = `[0, i)` 안 마지막 비공백 segment 다음 index(없으면 0).
    content_end: Vec<usize>,
}

impl Measure {
    fn new(paragraph: &ShapedParagraph) -> Measure {
        let segments = paragraph.segments();
        let mut prefix = Vec::with_capacity(segments.len() + 1);
        let mut content_end = Vec::with_capacity(segments.len() + 1);
        prefix.push(0i64);
        content_end.push(0usize);
        for (i, segment) in segments.iter().enumerate() {
            prefix.push(prefix[i] + i64::from(segment.advance.raw()));
            content_end.push(if segment.is_whitespace {
                content_end[i]
            } else {
                i + 1
            });
        }
        Measure {
            prefix,
            content_end,
        }
    }

    /// segment `[start, end)`의 (끝 공백 제외 폭, 끝 공백 폭).
    fn span(&self, start: usize, end: usize) -> (i64, i64) {
        let content = self.content_end[end].max(start);
        (
            self.prefix[content] - self.prefix[start],
            self.prefix[end] - self.prefix[content],
        )
    }

    /// 기회 사이 구간이 너무 넓을 때 너비에 들어가는 만큼(최소 segment 하나) 끊을 segment index.
    fn emergency_end(&self, start: usize, end: usize, limit: i64) -> usize {
        let mut fit = start + 1;
        for candidate in start + 2..=end {
            if self.span(start, candidate).0 <= limit {
                fit = candidate;
            } else {
                break;
            }
        }
        fit
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::bidi::BaseDirection;
    use crate::line_break::{line_break_opportunities, LineBreakOptions};
    use crate::paragraph::tests::Fixture;
    use crate::shaping::LanguageTag;

    fn lines(text: &str, width_px: Option<i32>, wrap: WrapMode, locale: &str) -> Vec<String> {
        let fx = Fixture::new();
        let para = fx.shape(text, 16, BaseDirection::LeftToRight);
        let breaks = line_break_opportunities(
            text,
            &LineBreakOptions::new(LanguageTag::parse(locale).expect("locale")),
        );
        let width = width_px.map(|px| TextUnit::from_px(px).expect("폭"));
        break_lines(&para, &breaks, width, wrap)
            .expect("줄")
            .iter()
            .map(|l| text[l.range.clone()].to_string())
            .collect()
    }

    #[test]
    fn narrow_width_fixture() {
        // 16px: 글자 8px(넓은 글자 12.8px), 공백 4px. "the quick brown fox".
        let text = "the quick brown fox";
        assert_eq!(
            lines(text, Some(48), WrapMode::Wrap, "en"),
            vec!["the ", "quick ", "brown ", "fox"]
        );
        assert_eq!(
            lines(text, Some(90), WrapMode::Wrap, "en"),
            vec!["the quick ", "brown fox"]
        );
        // 끝 공백은 폭에 들어가지 않으므로 "the quick"(정확히 맞음) 뒤 공백이 넘쳐도 같은 줄입니다.
        let fx = Fixture::new();
        let para = fx.shape(text, 16, BaseDirection::LeftToRight);
        let breaks = line_break_opportunities(
            text,
            &LineBreakOptions::new(LanguageTag::parse("en").expect("en")),
        );
        let exact = TextUnit::from_raw(para.segments()[..9].iter().map(|s| s.advance.raw()).sum());
        let result = break_lines(&para, &breaks, Some(exact), WrapMode::Wrap).expect("줄");
        assert_eq!(result[0].range, 0..10);
        assert_eq!(result[0].width, exact);
        assert_eq!(result[0].trailing_whitespace, TextUnit::from_raw(256));
        // 너비 0이어도 줄마다 segment가 하나 이상입니다(무한 반복 없음).
        let zero = lines("ab", Some(0), WrapMode::Wrap, "en");
        assert_eq!(zero, vec!["a", "b"]);
    }

    #[test]
    fn long_word_breaks_at_cluster_boundaries_without_splitting_ligature() {
        // "officeoffice"는 기회가 없는 긴 단어: 비상 줄바꿈은 segment 경계에서만, ffi ligature는 유지.
        let text = "officeoffice";
        let result = lines(text, Some(20), WrapMode::Wrap, "en");
        assert_eq!(result, vec!["o", "ffi", "ce", "o", "ffi", "ce"]);
        // 넉넉한 폭이면 한 줄.
        assert_eq!(
            lines(text, Some(400), WrapMode::Wrap, "en"),
            vec!["officeoffice"]
        );
        // 넘치는 긴 단어 뒤에 일반 단어: "abcdefgh"(64px)는 40px에서 비상 줄바꿈하고, 나머지
        // "fgh ij"(8·3 + 4 + 4.8·2 = 37.6px)는 한 줄에 들어갑니다.
        assert_eq!(
            lines("abcdefgh ij", Some(40), WrapMode::Wrap, "en"),
            vec!["abcde", "fgh ij"]
        );
    }

    #[test]
    fn cjk_fixture_breaks_between_ideographs() {
        // CJK 글자 16px. 폭 40px → 2글자씩, 。 앞에서는 끊지 않음(CL).
        let text = "日本語の文章。です";
        assert_eq!(
            lines(text, Some(40), WrapMode::Wrap, "ja"),
            vec!["日本", "語の", "文", "章。", "です"]
        );
        // 한글(Normal): 음절 사이에서 끊깁니다. 공백은 chain 첫 항목(Latin, 4px)이 덮어 "어 텍"
        // (16 + 4 + 16 = 36px)이 한 줄입니다.
        assert_eq!(
            lines("한국어 텍스트", Some(40), WrapMode::Wrap, "ko"),
            vec!["한국", "어 텍", "스트"]
        );
    }

    #[test]
    fn mandatory_breaks_and_no_wrap() {
        let text = "ab\u{2028}cd ef";
        assert_eq!(
            lines(text, None, WrapMode::Wrap, "en"),
            vec!["ab\u{2028}", "cd ef"]
        );
        assert_eq!(
            lines(text, Some(8), WrapMode::NoWrap, "en"),
            vec!["ab\u{2028}", "cd ef"]
        );
        let fx = Fixture::new();
        let para = fx.shape(text, 16, BaseDirection::LeftToRight);
        let breaks = line_break_opportunities(
            text,
            &LineBreakOptions::new(LanguageTag::parse("en").expect("en")),
        );
        let result = break_lines(&para, &breaks, None, WrapMode::Wrap).expect("줄");
        assert!(result[0].hard_break);
        assert!(!result[1].hard_break);
        assert_eq!(
            break_lines(&para, &breaks, Some(TextUnit::from_raw(-1)), WrapMode::Wrap),
            Err(TextLayoutError::NegativeWidth)
        );
    }

    #[test]
    fn empty_paragraph_has_one_empty_line() {
        let fx = Fixture::new();
        let para = fx.shape("", 16, BaseDirection::LeftToRight);
        let result =
            break_lines(&para, &[], Some(TextUnit::from_raw(100)), WrapMode::Wrap).expect("줄");
        assert_eq!(
            result,
            vec![LineBreak {
                range: 0..0,
                width: TextUnit::ZERO,
                trailing_whitespace: TextUnit::ZERO,
                hard_break: false
            }]
        );
    }

    #[test]
    fn line_breaking_is_deterministic_and_covers_text() {
        let text = "Stellar 스텔라 ステラ 星 office officer";
        for width in [0, 30, 64, 100, 1000] {
            let a = lines(text, Some(width), WrapMode::Wrap, "ja");
            assert_eq!(a, lines(text, Some(width), WrapMode::Wrap, "ja"));
            assert_eq!(a.concat(), text);
        }
    }
}
