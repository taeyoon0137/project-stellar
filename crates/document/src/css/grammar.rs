//! 속성 하나가 받는 CSS 값 문법 종류 (IX.11.1, 속성 모델 2장).
//!
//! registry 항목의 CSS 매핑([`super::mapping::CssMapping`])이 이 값을 가지며, 값 parser·formatter
//! ([`super::value`])와 속성 패널이 같은 값을 씁니다. 단위는 ADR-0075 결정 2(`Length`=`px`, `Angle`=`deg`,
//! `Duration`=`ms`, `Percentage`=`%`)를 따릅니다.

/// 속성 하나가 받는 값 문법.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CssGrammar {
    /// `<length>`(px). `non_negative`이면 음수 거부. `sizing`이면 크기 keyword(`auto`, `fit-content` …)를
    /// 알아보고 아직 저장할 수 없다고 진단합니다.
    Length {
        /// 음수 거부.
        non_negative: bool,
        /// 크기 속성(`width`·`height`).
        sizing: bool,
    },
    /// `<percentage>`(%).
    Percentage,
    /// `<angle>`(deg로 정규화).
    Angle,
    /// `<time>`(ms로 정규화).
    Duration,
    /// `<number>`(유한 수).
    Number,
    /// 0..=1 `<number>` 또는 `<percentage>`(`opacity`). 정규형은 수입니다.
    UnitInterval,
    /// 정수 개수.
    Count {
        /// 최솟값.
        min: u32,
        /// 최댓값.
        max: u32,
    },
    /// `<color>`. `none`을 받으면 `Color(None)`.
    Color {
        /// `none` 허용.
        allow_none: bool,
    },
    /// keyword 하나. 목록 index가 저장 tag입니다.
    Keyword(&'static [&'static str]),
    /// `true`·`false`(Stellar 전용 속성).
    Bool,
    /// 임의 문자열(Stellar 전용 속성).
    Text,
    /// `<transform-list>`의 2D 부분집합: `none`, `matrix()`, `translate*()`, `scale*()`.
    Transform,
    /// `border` shorthand: `none` 또는 `<length> [solid] <color>`(순서 무관).
    Border,
    /// `border-radius` shorthand: 길이 1–4개.
    Corners,
}

impl CssGrammar {
    /// 수치 조절(화살표·scrub)을 받는 문법인지.
    pub fn is_numeric(self) -> bool {
        matches!(
            self,
            CssGrammar::Length { .. }
                | CssGrammar::Percentage
                | CssGrammar::Angle
                | CssGrammar::Duration
                | CssGrammar::Number
                | CssGrammar::UnitInterval
                | CssGrammar::Count { .. }
                | CssGrammar::Corners
        )
    }

    /// 진단·snapshot용 문법 이름.
    pub fn name(self) -> &'static str {
        match self {
            CssGrammar::Length { .. } => "length",
            CssGrammar::Percentage => "percentage",
            CssGrammar::Angle => "angle",
            CssGrammar::Duration => "time",
            CssGrammar::Number => "number",
            CssGrammar::UnitInterval => "alpha-value",
            CssGrammar::Count { .. } => "integer",
            CssGrammar::Color { .. } => "color",
            CssGrammar::Keyword(_) => "keyword",
            CssGrammar::Bool => "boolean",
            CssGrammar::Text => "string",
            CssGrammar::Transform => "transform-list",
            CssGrammar::Border => "border",
            CssGrammar::Corners => "border-radius",
        }
    }
}

/// 비음수 길이.
pub const LENGTH_NON_NEGATIVE: CssGrammar = CssGrammar::Length {
    non_negative: true,
    sizing: false,
};
/// 부호 있는 길이.
pub const LENGTH_SIGNED: CssGrammar = CssGrammar::Length {
    non_negative: false,
    sizing: false,
};
/// 크기 속성 길이(`width`·`height`).
pub const LENGTH_SIZING: CssGrammar = CssGrammar::Length {
    non_negative: true,
    sizing: true,
};
