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

use stellar_core_types::text::UserText;

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
}
