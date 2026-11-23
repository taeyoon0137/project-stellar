//! Text style의 Typography CSS longhand key와 registry 항목 (IX.11.11, ADR-0075 개정 결정 11).
//!
//! Typography resource(`TypedValue::Typography`, `stellar_core_types::typography::Typography`)는 CSS longhand 일곱 개
//! (`font-family`, `font-size`, `font-weight`, `font-style`, `font-stretch`, `line-height`, `letter-spacing`)의 묶음
//! 값입니다. 문서 Text style 표(`crate::text_styles`)가 resource를 StableId로 참조하고, run·문단·문서 기본 순서로
//! 상속합니다. 이 module은 그 일곱 longhand의 [`PropertyKey`](ADR-0024 결정 14, Text 대역 `0x05xx`)와 CSS 매핑을
//! 고정합니다. semantic snapshot(Text style 경로)과 merge는 이 key로 longhand마다 경로 하나를 씁니다(속성 모델 15장 O12).
//!
//! key는 Text payload key `text-content`(0x0501) 뒤의 빈 번호 `0x0510`–`0x0516`입니다. 이전 표가 쓰던
//! `0x0501`–`0x0507`은 `text-content`와 번호가 겹쳤습니다(snapshot 경로 안에서만 쓰여 저장 bytes에는 없음).
//!
//! # `line-height` 저장 값의 뜻
//!
//! Typography `line-height`와 문단 `line_height`(`crate::text::LineHeight`)의 저장 값은 처음부터 CSS 의미입니다
//! (속성 모델 7장). 줄 상자의 각 inline box(run)는 **자기 글꼴**의 ascent·descent와 자기 `line-height`로
//! half-leading을 적용하고, 문단 root inline box의 strut(문단 typography의 첫 사용 가능 글꼴과 font-size)도 같은
//! 규칙으로 줄 상자에 들어갑니다. `<number>`는 그 inline box 자신의 font-size에 곱하고, `<length>`는 그대로 쓰며,
//! `normal`은 글꼴 metric(ascent + descent + line gap)입니다. text 엔진은 IX.11.12에서 이 의미를 따르며, 저장 bytes의
//! 뜻은 바뀌지 않습니다.

use stellar_core_types::property::PropertyKey;

use super::grammar::{CssGrammar, LENGTH_NON_NEGATIVE, LENGTH_SIGNED};
use super::mapping::{ConversionRule, CssMapping, MappingStatus};
use super::registry::{PropertyScope, RegistryEntry};

/// Typography longhand key.
pub mod keys {
    use stellar_core_types::property::PropertyKey;

    /// `font-family`.
    pub const FONT_FAMILY: PropertyKey = PropertyKey(0x0510);
    /// `font-size`.
    pub const FONT_SIZE: PropertyKey = PropertyKey(0x0511);
    /// `font-weight`.
    pub const FONT_WEIGHT: PropertyKey = PropertyKey(0x0512);
    /// `font-style`.
    pub const FONT_STYLE: PropertyKey = PropertyKey(0x0513);
    /// `font-stretch`.
    pub const FONT_STRETCH: PropertyKey = PropertyKey(0x0514);
    /// `line-height`.
    pub const LINE_HEIGHT: PropertyKey = PropertyKey(0x0515);
    /// `letter-spacing`.
    pub const LETTER_SPACING: PropertyKey = PropertyKey(0x0516);

    /// 일곱 key(Typography record 순서).
    pub const LONGHANDS: [PropertyKey; 7] = [
        FONT_FAMILY,
        FONT_SIZE,
        FONT_WEIGHT,
        FONT_STYLE,
        FONT_STRETCH,
        LINE_HEIGHT,
        LETTER_SPACING,
    ];
}

/// 일곱 longhand 이름([`keys::LONGHANDS`] 순서).
pub const LONGHAND_NAMES: [&str; 7] = [
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "font-stretch",
    "line-height",
    "letter-spacing",
];

const FONT_STYLE_KEYWORDS: [&str; 3] = ["normal", "italic", "oblique"];

const LINE_HEIGHT_RULES: [ConversionRule; 3] = [
    ConversionRule {
        variant: "normal",
        css: "line-height: normal",
        status: MappingStatus::Approximated,
    },
    ConversionRule {
        variant: "number",
        css: "line-height: <number>",
        status: MappingStatus::Exact,
    },
    ConversionRule {
        variant: "length",
        css: "line-height: <length>",
        status: MappingStatus::Exact,
    },
];

const MAPPINGS: [CssMapping; 7] = [
    CssMapping::converted(&["font-family"], Some(CssGrammar::Text))
        .note("작성자 순서의 family 목록입니다. 해석 결과는 FontFaceRef(file hash + face index)로 고정하고, 고정 face가 없거나 문서 글꼴 표에 없는 family는 missing-font로 진단합니다(임의 대체 없음)"),
    CssMapping::exact(&["font-size"], LENGTH_NON_NEGATIVE)
        .note("1/64 px 해상도로 저장합니다. medium(CSS initial)의 px 해석은 design profile이 정합니다"),
    CssMapping::exact(&["font-weight"], CssGrammar::Number)
        .subset()
        .note("1~1000 수를 저장합니다. normal·bold keyword는 입력에서만 받습니다"),
    CssMapping::exact(&["font-style"], CssGrammar::Keyword(&FONT_STYLE_KEYWORDS))
        .note("oblique는 -90~90deg 각도를 함께 저장합니다"),
    CssMapping::exact(&["font-stretch"], CssGrammar::Percentage)
        .subset()
        .note("백분율로 저장합니다(normal = 100%). keyword는 입력에서만 받습니다"),
    CssMapping::exact(&["line-height"], CssGrammar::Number)
        .note("normal | <number> | <length>. inline box마다 자기 글꼴 metric에 half-leading을 적용합니다(IX.11.12)")
        .rules(&LINE_HEIGHT_RULES),
    CssMapping::exact(&["letter-spacing"], LENGTH_SIGNED)
        .note("normal | <length> | <number>em. Typography가 기본값을 주고 run TextStyle이 override합니다"),
];

/// registry 항목 일곱 개(Text 범위, [`keys::LONGHANDS`] 순서).
pub fn registry_entries() -> impl Iterator<Item = RegistryEntry> {
    (0..7).map(|index| RegistryEntry {
        scope: PropertyScope::Text,
        key: keys::LONGHANDS[index],
        name: LONGHAND_NAMES[index],
        css: MAPPINGS[index],
        bundle: &[],
    })
}

/// key의 longhand 이름.
pub fn name_of(key: PropertyKey) -> Option<&'static str> {
    keys::LONGHANDS
        .iter()
        .position(|k| *k == key)
        .map(|i| LONGHAND_NAMES[i])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::mapping::is_known_longhand;
    use crate::payload_metadata::text_keys;

    #[test]
    fn keys_are_unique_text_band_numbers_after_text_content() {
        for (key, name) in keys::LONGHANDS.iter().zip(LONGHAND_NAMES) {
            assert_eq!(key.0 >> 8, 0x05);
            assert!(key.0 > text_keys::TEXT_CONTENT.0);
            assert!(is_known_longhand(name), "{name}");
            assert_eq!(name_of(*key), Some(name));
        }
        let mut numbers: Vec<u32> = keys::LONGHANDS.iter().map(|k| k.0).collect();
        numbers.dedup();
        assert_eq!(numbers.len(), 7);
        assert_eq!(crate::text_styles::keys::LONGHANDS, keys::LONGHANDS);
    }
}
