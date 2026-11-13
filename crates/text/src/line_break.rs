//! 공통 줄바꿈 기회 계산 (X.9.4).
//!
//! UAX #14 기본 알고리즘은 승인 의존성 `unicode-linebreak`(ADR-0016, Unicode 15.0)이 계산하고, 이
//! 모듈은 그 결과에 **명시한 locale과 옵션**에 따른 tailoring을 결정론적으로 더합니다. 실행 환경
//! locale은 쓰지 않으므로 [`LineBreakOptions`]는 locale 없이 만들 수 없습니다.
//!
//! 결과 [`BreakOpportunity::offset`]은 새 줄이 시작될 수 있는 UTF-8 byte offset입니다(그 앞에서 끊음).
//! offset 0은 포함하지 않고, 텍스트 끝은 항상 [`BreakKind::Mandatory`]입니다. 텍스트 안의 강제
//! 줄바꿈 문자(U+2028, U+000B, U+000C, U+0085, CR)도 Mandatory입니다.
//!
//! tailoring(CSS Text 3 `line-break`·`word-break`과 같은 의미):
//!
//! - [`LineBreakStrictness::Strict`]: UAX #14 기본(CJ 클래스 = NS). 작은 가나·장음 부호(ャ, ッ, ー 등)
//!   앞에서 끊지 않습니다.
//! - [`LineBreakStrictness::Normal`](기본): CJ를 ID로 취급해 작은 가나 앞에서도 끊을 수 있습니다.
//!   locale 언어가 일본어(`ja`)·중국어(`zh`)이면 U+301C(〜)·U+30A0(゠) 앞도 허용합니다.
//! - [`WordBreak::KeepAll`]: 글자 단위(AL·HL·NU·AI·ID·CJ·한글 음절·자모) 사이의 허용 기회를 지워 한국어
//!   어절·CJK 구절이 공백·구두점에서만 끊기게 합니다. [`WordBreak::Normal`](기본)은 UAX #14 그대로입니다.
//!
//! 끊을 수 있는지 판단할 앞 문자는 결합 부호(CM)·ZWJ를 건너뛴 base 문자입니다(LB9).

use unicode_linebreak::{break_property, linebreaks, BreakClass, BreakOpportunity as Raw};

use crate::shaping::LanguageTag;

/// 줄바꿈 기회 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BreakKind {
    /// 반드시 끊습니다.
    Mandatory,
    /// 끊을 수 있습니다.
    Allowed,
}

/// 줄바꿈 기회 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BreakOpportunity {
    /// 새 줄이 시작될 byte offset.
    pub offset: usize,
    /// 종류.
    pub kind: BreakKind,
}

/// 단어 안 줄바꿈 정책(CSS `word-break`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WordBreak {
    /// UAX #14 기본.
    #[default]
    Normal,
    /// 글자 사이 기회를 지웁니다(한국어 어절 단위 등).
    KeepAll,
}

/// CJK 줄바꿈 엄격도(CSS `line-break`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LineBreakStrictness {
    /// UAX #14 기본(작은 가나 앞 금지).
    Strict,
    /// 작은 가나 앞 허용, 일본어·중국어 locale은 〜·゠ 앞 허용.
    #[default]
    Normal,
}

/// 줄바꿈 옵션. locale은 필수입니다.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LineBreakOptions {
    /// 텍스트 locale(명시 입력).
    pub locale: LanguageTag,
    /// 단어 안 줄바꿈.
    pub word_break: WordBreak,
    /// CJK 엄격도.
    pub strictness: LineBreakStrictness,
}

impl LineBreakOptions {
    /// 기본 옵션(Normal·Normal)과 명시 locale.
    pub fn new(locale: LanguageTag) -> LineBreakOptions {
        LineBreakOptions {
            locale,
            word_break: WordBreak::Normal,
            strictness: LineBreakStrictness::Normal,
        }
    }

    fn is_chinese_or_japanese(&self) -> bool {
        matches!(self.locale.primary_language(), "ja" | "zh")
    }
}

/// 앞 문자가 이 클래스면 ID 앞 줄바꿈이 UAX #14에서 허용됩니다(LB31). OP·QU·GL·WJ·ZWJ·BB·PR·B2·SP·
/// 강제 줄바꿈·RI는 제외합니다(LB14·LB19·LB12·LB11·LB8a·LB21·LB23a·LB17·LB18·LB4~6·LB30a).
fn allows_break_before_ideograph(prev: BreakClass) -> bool {
    use BreakClass::*;
    matches!(
        prev,
        Alphabetic
            | HebrewLetter
            | Ambiguous
            | Numeric
            | Ideographic
            | ConditionalJapaneseStarter
            | HangulLvSyllable
            | HangulLvtSyllable
            | HangulLJamo
            | HangulVJamo
            | HangulTJamo
            | ClosePunctuation
            | CloseParenthesis
            | Exclamation
            | InfixSeparator
            | Symbol
            | After
            | Hyphen
            | NonStarter
            | Inseparable
            | Postfix
            | EmojiBase
            | EmojiModifier
            | ComplexContext
            | Unknown
    )
}

/// keep-all에서 사이 기회를 지우는 글자 클래스.
fn is_letter_unit(class: BreakClass) -> bool {
    use BreakClass::*;
    matches!(
        class,
        Alphabetic
            | HebrewLetter
            | Ambiguous
            | Numeric
            | Ideographic
            | ConditionalJapaneseStarter
            | HangulLvSyllable
            | HangulLvtSyllable
            | HangulLJamo
            | HangulVJamo
            | HangulTJamo
            | ComplexContext
            | Unknown
    )
}

fn class_of(c: char) -> BreakClass {
    break_property(u32::from(c))
}

/// `offset` 앞의 base 문자 클래스(CM·ZWJ를 건너뜀). 없으면 `None`.
fn base_class_before(text: &str, offset: usize) -> Option<BreakClass> {
    text[..offset].chars().rev().map(class_of).find(|class| {
        !matches!(
            class,
            BreakClass::CombiningMark | BreakClass::ZeroWidthJoiner
        )
    })
}

/// 텍스트의 줄바꿈 기회를 계산합니다 (X.9.4). 결과는 offset 오름차순이고 중복이 없습니다.
pub fn line_break_opportunities(text: &str, options: &LineBreakOptions) -> Vec<BreakOpportunity> {
    let mut breaks: Vec<BreakOpportunity> = linebreaks(text)
        .filter(|(offset, _)| *offset > 0)
        .map(|(offset, kind)| BreakOpportunity {
            offset,
            kind: match kind {
                Raw::Mandatory => BreakKind::Mandatory,
                Raw::Allowed => BreakKind::Allowed,
            },
        })
        .collect();

    if options.strictness == LineBreakStrictness::Normal {
        let cjk_locale = options.is_chinese_or_japanese();
        let mut added = Vec::new();
        for (offset, c) in text.char_indices().filter(|(i, _)| *i > 0) {
            let treat_as_ideograph = class_of(c) == BreakClass::ConditionalJapaneseStarter
                || (cjk_locale && matches!(c, '\u{301C}' | '\u{30A0}'));
            if !treat_as_ideograph {
                continue;
            }
            if base_class_before(text, offset).is_some_and(allows_break_before_ideograph) {
                added.push(offset);
            }
        }
        for offset in added {
            if let Err(pos) = breaks.binary_search_by_key(&offset, |b| b.offset) {
                breaks.insert(
                    pos,
                    BreakOpportunity {
                        offset,
                        kind: BreakKind::Allowed,
                    },
                );
            }
        }
    }

    if options.word_break == WordBreak::KeepAll {
        breaks.retain(|b| {
            if b.kind == BreakKind::Mandatory {
                return true;
            }
            let next = text[b.offset..].chars().next().map(class_of);
            let prev = base_class_before(text, b.offset);
            !(prev.is_some_and(is_letter_unit) && next.is_some_and(is_letter_unit))
        });
    }
    breaks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(locale: &str) -> LineBreakOptions {
        LineBreakOptions::new(LanguageTag::parse(locale).expect("locale"))
    }

    fn offsets(text: &str, options: &LineBreakOptions) -> Vec<(usize, BreakKind)> {
        line_break_opportunities(text, options)
            .into_iter()
            .map(|b| (b.offset, b.kind))
            .collect()
    }

    fn allowed(list: &[usize], end: usize) -> Vec<(usize, BreakKind)> {
        let mut out: Vec<(usize, BreakKind)> =
            list.iter().map(|&o| (o, BreakKind::Allowed)).collect();
        out.push((end, BreakKind::Mandatory));
        out
    }

    #[test]
    fn english_and_whitespace_fixture() {
        let en = options("en");
        // 공백 뒤, 하이픈 뒤에서 끊고 공백 앞에서는 끊지 않습니다.
        assert_eq!(
            offsets("Hello world, foo-bar", &en),
            allowed(&[6, 13, 17], 20)
        );
        // 연속 공백은 마지막 공백 뒤 한 번만.
        assert_eq!(offsets("a   b", &en), allowed(&[4], 5));
        // NBSP(GL)는 끊지 않고, ZWSP(ZW) 뒤는 끊습니다.
        assert_eq!(offsets("a\u{00A0}b", &en), allowed(&[], 4));
        assert_eq!(offsets("a\u{200B}b", &en), allowed(&[4], 5));
        // 줄 끝 공백 뒤는 텍스트 끝(Mandatory)뿐입니다.
        assert_eq!(offsets("ab  ", &en), allowed(&[], 4));
        // 강제 줄바꿈 문자(U+2028)는 Mandatory.
        assert_eq!(
            offsets("a\u{2028}b", &en),
            vec![(4, BreakKind::Mandatory), (5, BreakKind::Mandatory)]
        );
        assert!(offsets("", &en).is_empty());
    }

    #[test]
    fn korean_fixture_normal_and_keep_all() {
        let text = "한국어 텍스트";
        // Normal: 음절 사이와 공백 뒤(공백 앞은 아님).
        assert_eq!(
            offsets(text, &options("ko")),
            allowed(&[3, 6, 10, 13, 16], 19)
        );
        // keep-all: 어절 경계(공백 뒤)에서만.
        let keep_all = LineBreakOptions {
            word_break: WordBreak::KeepAll,
            ..options("ko")
        };
        assert_eq!(offsets(text, &keep_all), allowed(&[10], 19));
        // keep-all도 구두점 뒤 기회는 유지합니다: "가,나" 의 쉼표 뒤.
        assert_eq!(offsets("가,나", &keep_all), allowed(&[4], 7));
    }

    #[test]
    fn japanese_fixture_small_kana_and_punctuation() {
        // キャット: キ(ID) ャ(CJ) ッ(CJ) ト(ID). 각 3 bytes.
        let text = "キャット。です";
        let strict = LineBreakOptions {
            strictness: LineBreakStrictness::Strict,
            ..options("ja")
        };
        // Strict: 작은 가나 앞 금지, 。(CL) 앞 금지, 。 뒤 허용.
        assert_eq!(offsets(text, &strict), allowed(&[9, 15, 18], 21));
        // Normal: 작은 가나 앞(3, 6) 허용.
        assert_eq!(
            offsets(text, &options("ja")),
            allowed(&[3, 6, 9, 15, 18], 21)
        );
    }

    #[test]
    fn locale_changes_cjk_hyphen_like_breaks() {
        // 東京〜大阪: 〜(U+301C, NS) 앞은 일본어·중국어 locale의 Normal에서만 허용.
        let text = "東京〜大阪";
        assert_eq!(offsets(text, &options("ja")), allowed(&[3, 6, 9, 12], 15));
        assert_eq!(
            offsets(text, &options("zh-Hant")),
            allowed(&[3, 6, 9, 12], 15)
        );
        assert_eq!(offsets(text, &options("ko")), allowed(&[3, 9, 12], 15));
        assert_eq!(offsets(text, &options("en-US")), allowed(&[3, 9, 12], 15));
        let strict = LineBreakOptions {
            strictness: LineBreakStrictness::Strict,
            ..options("ja")
        };
        assert_eq!(offsets(text, &strict), allowed(&[3, 9, 12], 15));
    }

    #[test]
    fn mixed_script_and_combining_marks() {
        let en = options("en");
        // 한글과 영문 사이: "abc한국" → 영문(AL)과 음절(H3) 사이 허용, 음절 사이 허용.
        assert_eq!(offsets("abc한국", &en), allowed(&[3, 6], 9));
        // 결합 부호(U+0301)는 앞 base에 붙어 끊지 않습니다.
        assert_eq!(offsets("e\u{0301} x", &en), allowed(&[4], 5));
        // 여는 괄호 뒤의 작은 가나 앞은 Normal이어도 끊지 않습니다(OP ×).
        assert_eq!(offsets("(ャ", &options("ja")), allowed(&[], 4));
    }

    #[test]
    fn results_are_sorted_unique_and_deterministic() {
        let text = "Stellar 스텔라 ステラ 星 שלום 123-456";
        for opts in [options("en"), options("ja"), options("ko")] {
            let a = line_break_opportunities(text, &opts);
            assert_eq!(a, line_break_opportunities(text, &opts));
            assert!(a.windows(2).all(|w| w[0].offset < w[1].offset));
            assert_eq!(
                a.last().map(|b| (b.offset, b.kind)),
                Some((text.len(), BreakKind::Mandatory))
            );
            assert!(a.iter().all(|b| text.is_char_boundary(b.offset)));
        }
    }

    #[test]
    fn unicode_data_version_is_pinned() {
        assert_eq!(unicode_linebreak::UNICODE_VERSION, (15, 0, 0));
    }
}
