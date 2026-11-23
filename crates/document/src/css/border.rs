//! Frame 테두리 4변 CSS longhand (IX.11.10, ADR-0075 개정 결정 10).
//!
//! CSS `border`는 변마다 `border-*-width`·`border-*-style`·`border-*-color` 세 longhand, 모두 열두 개입니다. 이 module이
//! 그 열두 longhand의 [`PropertyKey`](ADR-0024 결정 14), 이름·순서, 값 모델([`BorderSides`]), CSS 값 텍스트 해석·출력,
//! 레이아웃 입력(변마다 computed `border-*-width`)을 한 곳에 둡니다. 텍스트 원본(ADR-0077 v4)·migration step·layout
//! 입력이 모두 이 함수를 씁니다(이중 구현 없음).
//!
//! # 값의 뜻(CSS Backgrounds 3)
//!
//! - initial value는 변마다 `medium none currentcolor`입니다.
//! - computed `border-*-width`는 그 변 style이 `none`이면 0입니다. 레이아웃은 이 computed 값으로 padding box를 줄입니다
//!   (border box 안쪽이 padding box).
//! - style은 Stellar가 그리는 `none`·`solid`만 받습니다(`subset`). 다른 CSS keyword는 `unsupported-value`입니다.
//!
//! # 지금 저장 위치와 묶음 key 0x0105
//!
//! 제품 schema 1의 Frame payload는 테두리를 묶음 값 `Border{color, thickness}` 하나로 저장합니다(`FramePayload::border`,
//! key 0x0105). 이 값의 longhand 뜻은 [`BorderSides::from_payload`]입니다: 있으면 네 변 같은 두께·`solid`·색, 없으면
//! 네 변 initial value입니다. 그래서 옛 bytes는 값 변환 없이 열두 longhand로 읽히고, 묶음 값으로 표현되는 longhand
//! 조합(네 변 같은 `solid` 두께·명시 색, 또는 네 변 initial)은 [`BorderSides::to_payload`]로 다시 묶음 값이 됩니다.
//! 변마다 다른 값은 Frame payload schema 2(변마다 값을 저장하는 형식, [`BorderSides::encode`])가 들어올 때 저장됩니다.
//! 그 전까지 변마다 다른 값은 [`BorderSidesError`]로 거부합니다(조용히 잃지 않음).

use std::fmt;

use stellar_core_types::color::Color;
use stellar_core_types::property::PropertyKey;
use stellar_core_types::value::FiniteF32;

use super::grammar::{CssGrammar, LENGTH_NON_NEGATIVE};
use super::mapping::CssMapping;
use super::registry::{PropertyScope, RegistryEntry};
use super::value::{self, CssValue, CssValueError};
use crate::frame::Border;

/// 열두 longhand의 key(Frame 대역 `0x01xx`의 빈 번호, ADR-0024 결정 14). 순서는 변(top, right, bottom, left)마다
/// width·style·color이고, 이 순서가 registry·텍스트 원본 줄 순서입니다.
pub mod keys {
    use stellar_core_types::property::PropertyKey;

    /// `border-top-width`.
    pub const BORDER_TOP_WIDTH: PropertyKey = PropertyKey(0x0122);
    /// `border-top-style`.
    pub const BORDER_TOP_STYLE: PropertyKey = PropertyKey(0x0123);
    /// `border-top-color`.
    pub const BORDER_TOP_COLOR: PropertyKey = PropertyKey(0x0124);
    /// `border-right-width`.
    pub const BORDER_RIGHT_WIDTH: PropertyKey = PropertyKey(0x0125);
    /// `border-right-style`.
    pub const BORDER_RIGHT_STYLE: PropertyKey = PropertyKey(0x0126);
    /// `border-right-color`.
    pub const BORDER_RIGHT_COLOR: PropertyKey = PropertyKey(0x0127);
    /// `border-bottom-width`.
    pub const BORDER_BOTTOM_WIDTH: PropertyKey = PropertyKey(0x0128);
    /// `border-bottom-style`.
    pub const BORDER_BOTTOM_STYLE: PropertyKey = PropertyKey(0x0129);
    /// `border-bottom-color`.
    pub const BORDER_BOTTOM_COLOR: PropertyKey = PropertyKey(0x012a);
    /// `border-left-width`.
    pub const BORDER_LEFT_WIDTH: PropertyKey = PropertyKey(0x012b);
    /// `border-left-style`.
    pub const BORDER_LEFT_STYLE: PropertyKey = PropertyKey(0x012c);
    /// `border-left-color`.
    pub const BORDER_LEFT_COLOR: PropertyKey = PropertyKey(0x012d);

    /// 열두 key(registry 순서). 묶음 key 0x0105가 나뉘는 longhand입니다.
    pub const LONGHANDS: [PropertyKey; 12] = [
        BORDER_TOP_WIDTH,
        BORDER_TOP_STYLE,
        BORDER_TOP_COLOR,
        BORDER_RIGHT_WIDTH,
        BORDER_RIGHT_STYLE,
        BORDER_RIGHT_COLOR,
        BORDER_BOTTOM_WIDTH,
        BORDER_BOTTOM_STYLE,
        BORDER_BOTTOM_COLOR,
        BORDER_LEFT_WIDTH,
        BORDER_LEFT_STYLE,
        BORDER_LEFT_COLOR,
    ];
}

/// 열두 longhand 이름([`keys::LONGHANDS`] 순서).
pub const LONGHAND_NAMES: [&str; 12] = [
    "border-top-width",
    "border-top-style",
    "border-top-color",
    "border-right-width",
    "border-right-style",
    "border-right-color",
    "border-bottom-width",
    "border-bottom-style",
    "border-bottom-color",
    "border-left-width",
    "border-left-style",
    "border-left-color",
];

/// 변.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BorderSide {
    /// 위.
    Top,
    /// 오른쪽.
    Right,
    /// 아래.
    Bottom,
    /// 왼쪽.
    Left,
}

impl BorderSide {
    /// CSS 순서(top, right, bottom, left).
    pub const ALL: [BorderSide; 4] = [
        BorderSide::Top,
        BorderSide::Right,
        BorderSide::Bottom,
        BorderSide::Left,
    ];

    /// CSS 이름 조각.
    pub fn name(self) -> &'static str {
        match self {
            BorderSide::Top => "top",
            BorderSide::Right => "right",
            BorderSide::Bottom => "bottom",
            BorderSide::Left => "left",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// longhand가 담는 부분.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BorderPart {
    /// `border-*-width`.
    Width,
    /// `border-*-style`.
    Style,
    /// `border-*-color`.
    Color,
}

/// 열두 longhand 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BorderLonghand {
    /// 변.
    pub side: BorderSide,
    /// 부분.
    pub part: BorderPart,
}

impl BorderLonghand {
    /// registry 순서 열두 개.
    pub const ALL: [BorderLonghand; 12] = {
        let parts = [BorderPart::Width, BorderPart::Style, BorderPart::Color];
        let mut out = [BorderLonghand {
            side: BorderSide::Top,
            part: BorderPart::Width,
        }; 12];
        let mut i = 0;
        while i < 12 {
            out[i] = BorderLonghand {
                side: BorderSide::ALL[i / 3],
                part: parts[i % 3],
            };
            i += 1;
        }
        out
    };

    /// [`Self::ALL`] 안 순번.
    pub fn index(self) -> usize {
        self.side.index() * 3 + self.part as usize
    }

    /// key.
    pub fn key(self) -> PropertyKey {
        keys::LONGHANDS[self.index()]
    }

    /// CSS 이름.
    pub fn name(self) -> &'static str {
        LONGHAND_NAMES[self.index()]
    }

    /// 이름으로 찾습니다.
    pub fn from_name(name: &str) -> Option<BorderLonghand> {
        Self::ALL.into_iter().find(|l| l.name() == name)
    }

    /// key로 찾습니다.
    pub fn from_key(key: PropertyKey) -> Option<BorderLonghand> {
        Self::ALL.into_iter().find(|l| l.key() == key)
    }

    /// 값 문법.
    pub fn grammar(self) -> CssGrammar {
        match self.part {
            BorderPart::Width => LENGTH_NON_NEGATIVE,
            BorderPart::Style => CssGrammar::Keyword(&STYLE_KEYWORDS),
            BorderPart::Color => CssGrammar::Color { allow_none: false },
        }
    }
}

impl fmt::Display for BorderLonghand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Stellar가 받는 `border-*-style` keyword(저장 tag 순서).
pub const STYLE_KEYWORDS: [&str; 2] = ["none", "solid"];

/// CSS가 정의하지만 Stellar가 그리지 않는 `border-*-style` keyword(`unsupported-value`).
pub const UNSUPPORTED_STYLE_KEYWORDS: [&str; 8] = [
    "hidden", "dotted", "dashed", "double", "groove", "ridge", "inset", "outset",
];

/// `border-*-style`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BorderStyle {
    /// `none`(CSS initial). computed width가 0입니다.
    #[default]
    None,
    /// `solid`.
    Solid,
}

/// `border-*-width`. keyword는 입력 표기를 그대로 보존합니다(텍스트 원본 왕복). px 값은 CSS 기준 브라우저(Chrome)
/// 값입니다(`thin` 1px, `medium` 3px, `thick` 5px).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BorderWidth {
    /// `thin`.
    Thin,
    /// `medium`(CSS initial).
    Medium,
    /// `thick`.
    Thick,
    /// 비음수 `<length>`(px).
    Length(FiniteF32),
}

impl BorderWidth {
    /// px 값(keyword는 Chrome 값).
    pub fn px(self) -> f32 {
        match self {
            BorderWidth::Thin => 1.0,
            BorderWidth::Medium => 3.0,
            BorderWidth::Thick => 5.0,
            BorderWidth::Length(value) => value.get(),
        }
    }
}

/// 변 하나의 세 longhand 값.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BorderSideValue {
    /// `border-*-width`.
    pub width: BorderWidth,
    /// `border-*-style`.
    pub style: BorderStyle,
    /// `border-*-color`. `None`은 `currentcolor`(CSS initial)입니다.
    pub color: Option<Color>,
}

impl BorderSideValue {
    /// CSS initial value(`medium none currentcolor`).
    pub const INITIAL: BorderSideValue = BorderSideValue {
        width: BorderWidth::Medium,
        style: BorderStyle::None,
        color: None,
    };

    /// computed `border-*-width`(px): style `none`이면 0입니다.
    pub fn computed_width(&self) -> f32 {
        match self.style {
            BorderStyle::None => 0.0,
            BorderStyle::Solid => self.width.px(),
        }
    }
}

/// 테두리 열두 longhand 값(변 CSS 순서).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BorderSides(pub [BorderSideValue; 4]);

impl Default for BorderSides {
    fn default() -> BorderSides {
        BorderSides::INITIAL
    }
}

/// 묶음 값으로 표현할 수 없는 longhand 조합.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderSidesError {
    /// 변마다 값이 다릅니다(Frame payload schema 2 전에는 저장할 수 없음).
    NotUniform,
    /// style `none`인데 width·color가 initial이 아닙니다(묶음 값 "없음"은 initial만 뜻함).
    NoneWithValues,
    /// `solid`인데 width가 keyword이거나 color가 `currentcolor`입니다(묶음 값은 px 두께와 명시 색만 담음).
    NotRepresentable,
}

impl BorderSidesError {
    /// 진단 코드.
    pub fn code(self) -> &'static str {
        "unsupported-value"
    }
}

impl fmt::Display for BorderSidesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            BorderSidesError::NotUniform => {
                "변마다 다른 테두리는 Frame payload schema 2 전에는 저장할 수 없습니다(네 변이 같아야 합니다)"
            }
            BorderSidesError::NoneWithValues => {
                "border-style: none의 width·color는 initial(medium·currentcolor)이어야 합니다"
            }
            BorderSidesError::NotRepresentable => {
                "solid 테두리는 px 두께와 명시 색이어야 합니다(keyword 두께·currentcolor는 아직 저장할 수 없습니다)"
            }
        })
    }
}

impl std::error::Error for BorderSidesError {}

impl BorderSides {
    /// 네 변 initial value.
    pub const INITIAL: BorderSides = BorderSides([BorderSideValue::INITIAL; 4]);

    /// 네 변 같은 값.
    pub fn uniform(side: BorderSideValue) -> BorderSides {
        BorderSides([side; 4])
    }

    /// 변 값.
    pub fn side(&self, side: BorderSide) -> &BorderSideValue {
        &self.0[side.index()]
    }

    /// 변 값(수정).
    pub fn side_mut(&mut self, side: BorderSide) -> &mut BorderSideValue {
        &mut self.0[side.index()]
    }

    /// 묶음 값 0x0105(`FramePayload::border`)의 longhand 뜻.
    pub fn from_payload(border: Option<Border>) -> BorderSides {
        match border {
            None => BorderSides::INITIAL,
            Some(border) => BorderSides::uniform(BorderSideValue {
                width: BorderWidth::Length(
                    FiniteF32::new(border.thickness()).expect("Border 두께는 finite입니다"),
                ),
                style: BorderStyle::Solid,
                color: Some(border.color),
            }),
        }
    }

    /// 묶음 값으로 되돌립니다. 표현할 수 없는 조합은 오류입니다(조용히 바꾸지 않음).
    pub fn to_payload(&self) -> Result<Option<Border>, BorderSidesError> {
        let first = self.0[0];
        if self.0.iter().any(|side| *side != first) {
            return Err(BorderSidesError::NotUniform);
        }
        match first {
            BorderSideValue::INITIAL => Ok(None),
            BorderSideValue {
                style: BorderStyle::None,
                ..
            } => Err(BorderSidesError::NoneWithValues),
            BorderSideValue {
                width: BorderWidth::Length(width),
                style: BorderStyle::Solid,
                color: Some(color),
            } => Ok(Some(
                Border::new(color, width.get()).expect("비음수 finite 두께입니다"),
            )),
            BorderSideValue { .. } => Err(BorderSidesError::NotRepresentable),
        }
    }

    /// 변마다 computed `border-*-width`(px, top·right·bottom·left). 레이아웃 입력입니다.
    pub fn computed_widths(&self) -> [f32; 4] {
        self.0.map(|side| side.computed_width())
    }

    /// longhand 하나의 정규 CSS 값 텍스트.
    pub fn format(&self, longhand: BorderLonghand) -> String {
        let side = self.side(longhand.side);
        match longhand.part {
            BorderPart::Width => match side.width {
                BorderWidth::Thin => "thin".into(),
                BorderWidth::Medium => "medium".into(),
                BorderWidth::Thick => "thick".into(),
                BorderWidth::Length(value) => value::format_length(value.get()),
            },
            BorderPart::Style => STYLE_KEYWORDS[side.style as usize].into(),
            BorderPart::Color => side
                .color
                .map_or_else(|| "currentcolor".into(), value::format_color),
        }
    }

    /// longhand 하나를 CSS 값 텍스트로 설정합니다. 값은 문서 값 parser(`css::value`)로 해석합니다.
    pub fn set(&mut self, longhand: BorderLonghand, text: &str) -> Result<(), CssValueError> {
        let parsed = parse_longhand(longhand, text)?;
        let side = self.side_mut(longhand.side);
        match parsed {
            LonghandValue::Width(width) => side.width = width,
            LonghandValue::Style(style) => side.style = style,
            LonghandValue::Color(color) => side.color = color,
        }
        Ok(())
    }

    /// 열두 longhand 선언(registry 순서, 정규 값 텍스트).
    pub fn declarations(&self) -> Vec<(&'static str, String)> {
        BorderLonghand::ALL
            .into_iter()
            .map(|l| (l.name(), self.format(l)))
            .collect()
    }

    /// 레이아웃 입력 선언: 변마다 computed `border-*-width`(px). style이 `none`인 변은 `0px`입니다.
    pub fn layout_declarations(&self) -> [(&'static str, String); 4] {
        let widths = self.computed_widths();
        BorderSide::ALL.map(|side| {
            (
                BorderLonghand {
                    side,
                    part: BorderPart::Width,
                }
                .name(),
                value::format_length(widths[side.index()]),
            )
        })
    }

    /// Frame payload schema 2의 테두리 절 canonical bytes. 변(CSS 순서)마다 width tag(0 thin, 1 medium, 2 thick,
    /// 3 길이 + f32 LE), style tag(0 none, 1 solid), color flag(0 currentcolor, 1 + [`Color`] bytes)입니다. 묶음 key
    /// 0x0105를 대체하는 저장 형식이고, schema 1 Frame payload에는 쓰지 않습니다.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 * (6 + Color::BYTE_LEN));
        for side in &self.0 {
            match side.width {
                BorderWidth::Thin => out.push(0),
                BorderWidth::Medium => out.push(1),
                BorderWidth::Thick => out.push(2),
                BorderWidth::Length(value) => {
                    out.push(3);
                    out.extend_from_slice(&value.get().to_le_bytes());
                }
            }
            out.push(side.style as u8);
            match side.color {
                None => out.push(0),
                Some(color) => {
                    out.push(1);
                    out.extend_from_slice(&color.to_bytes());
                }
            }
        }
        out
    }

    /// [`Self::encode`]를 읽습니다. 형식 위반, 음수·비유한 두께, 잉여 bytes, 같은 값의 다른 bytes는 거부합니다.
    pub fn decode(bytes: &[u8]) -> Result<BorderSides, BorderDecodeError> {
        let mut pos = 0;
        let mut take = |n: usize| -> Result<&[u8], BorderDecodeError> {
            let slice = bytes.get(pos..pos + n).ok_or(BorderDecodeError)?;
            pos += n;
            Ok(slice)
        };
        let mut sides = [BorderSideValue::INITIAL; 4];
        for side in &mut sides {
            side.width = match take(1)?[0] {
                0 => BorderWidth::Thin,
                1 => BorderWidth::Medium,
                2 => BorderWidth::Thick,
                3 => {
                    let raw: [u8; 4] = take(4)?.try_into().expect("4 bytes");
                    let value = f32::from_le_bytes(raw);
                    if value < 0.0 {
                        return Err(BorderDecodeError);
                    }
                    BorderWidth::Length(FiniteF32::new(value).map_err(|_| BorderDecodeError)?)
                }
                _ => return Err(BorderDecodeError),
            };
            side.style = match take(1)?[0] {
                0 => BorderStyle::None,
                1 => BorderStyle::Solid,
                _ => return Err(BorderDecodeError),
            };
            side.color = match take(1)?[0] {
                0 => None,
                1 => {
                    let raw: [u8; Color::BYTE_LEN] =
                        take(Color::BYTE_LEN)?.try_into().expect("길이 고정");
                    Some(Color::from_bytes(raw).map_err(|_| BorderDecodeError)?)
                }
                _ => return Err(BorderDecodeError),
            };
        }
        if pos != bytes.len() {
            return Err(BorderDecodeError);
        }
        let decoded = BorderSides(sides);
        if decoded.encode() != bytes {
            return Err(BorderDecodeError);
        }
        Ok(decoded)
    }
}

/// [`BorderSides::decode`] 형식 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BorderDecodeError;

impl fmt::Display for BorderDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("테두리 절 binary 형식 위반")
    }
}

impl std::error::Error for BorderDecodeError {}

/// longhand 하나의 해석 값.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LonghandValue {
    /// `border-*-width`.
    Width(BorderWidth),
    /// `border-*-style`.
    Style(BorderStyle),
    /// `border-*-color`(`None` = `currentcolor`).
    Color(Option<Color>),
}

/// longhand 하나의 CSS 값 텍스트를 해석합니다. 길이·색은 문서 값 parser(`css::value::parse`)를 지납니다.
pub fn parse_longhand(
    longhand: BorderLonghand,
    text: &str,
) -> Result<LonghandValue, CssValueError> {
    let trimmed = text.trim();
    match longhand.part {
        BorderPart::Width => match trimmed {
            "thin" => Ok(LonghandValue::Width(BorderWidth::Thin)),
            "medium" => Ok(LonghandValue::Width(BorderWidth::Medium)),
            "thick" => Ok(LonghandValue::Width(BorderWidth::Thick)),
            _ => match value::parse(LENGTH_NON_NEGATIVE, trimmed)? {
                CssValue::Length(px) => Ok(LonghandValue::Width(BorderWidth::Length(
                    FiniteF32::new(px).map_err(|_| {
                        CssValueError::new("invalid-value", "두께가 유한하지 않습니다")
                    })?,
                ))),
                _ => Err(CssValueError::new(
                    "invalid-value",
                    format!("{longhand}: 길이가 아닙니다"),
                )),
            },
        },
        BorderPart::Style => match trimmed {
            "none" => Ok(LonghandValue::Style(BorderStyle::None)),
            "solid" => Ok(LonghandValue::Style(BorderStyle::Solid)),
            other if UNSUPPORTED_STYLE_KEYWORDS.contains(&other) => Err(CssValueError::new(
                "unsupported-value",
                format!("{longhand}: {other}는 아직 그리지 않습니다(none·solid만 받습니다)"),
            )),
            other => Err(CssValueError::new(
                "invalid-value",
                format!("{longhand}: 알 수 없는 style {other}"),
            )),
        },
        BorderPart::Color => match trimmed {
            "currentcolor" => Ok(LonghandValue::Color(None)),
            _ => match value::parse(longhand.grammar(), trimmed)? {
                CssValue::Color(Some(color)) => Ok(LonghandValue::Color(Some(color))),
                _ => Err(CssValueError::new(
                    "invalid-value",
                    format!("{longhand}: 색이 아닙니다"),
                )),
            },
        },
    }
}

const fn mapping(index: usize) -> CssMapping {
    let name: &'static [&'static str] = match index {
        0 => &["border-top-width"],
        1 => &["border-top-style"],
        2 => &["border-top-color"],
        3 => &["border-right-width"],
        4 => &["border-right-style"],
        5 => &["border-right-color"],
        6 => &["border-bottom-width"],
        7 => &["border-bottom-style"],
        8 => &["border-bottom-color"],
        9 => &["border-left-width"],
        10 => &["border-left-style"],
        _ => &["border-left-color"],
    };
    match index % 3 {
        0 => CssMapping::exact(name, LENGTH_NON_NEGATIVE)
            .note("computed 값은 style none이면 0입니다. border box 안쪽이 padding box입니다. 제품 schema 1에서는 묶음 값 0x0105(네 변 같은 값)에서 풉니다"),
        1 => CssMapping::exact(name, CssGrammar::Keyword(&STYLE_KEYWORDS))
            .subset()
            .note("none·solid만 받습니다. 그 밖의 CSS keyword는 unsupported-value입니다"),
        _ => CssMapping::exact(name, CssGrammar::Color { allow_none: false })
            .note("currentcolor는 CSS initial입니다. 제품 schema 1 묶음 값은 명시 색만 담습니다"),
    }
}

/// registry 항목 열두 개(Frame 범위, [`keys::LONGHANDS`] 순서).
pub fn registry_entries() -> impl Iterator<Item = RegistryEntry> {
    (0..12).map(|index| RegistryEntry {
        scope: PropertyScope::Frame,
        key: keys::LONGHANDS[index],
        name: LONGHAND_NAMES[index],
        css: MAPPINGS[index],
        bundle: &[],
    })
}

const MAPPINGS: [CssMapping; 12] = [
    mapping(0),
    mapping(1),
    mapping(2),
    mapping(3),
    mapping(4),
    mapping(5),
    mapping(6),
    mapping(7),
    mapping(8),
    mapping(9),
    mapping(10),
    mapping(11),
];

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::color::ColorSpace;

    fn red() -> Color {
        Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 1.0).unwrap()
    }

    #[test]
    fn longhand_order_keys_and_names_agree() {
        // 검증 기준(IX.11.10): 열두 longhand에 key 하나씩, 이름·순서가 하나의 표.
        for (index, longhand) in BorderLonghand::ALL.into_iter().enumerate() {
            assert_eq!(longhand.index(), index);
            assert_eq!(longhand.key(), keys::LONGHANDS[index]);
            assert_eq!(
                longhand.name(),
                format!(
                    "border-{}-{}",
                    longhand.side.name(),
                    ["width", "style", "color"][longhand.part as usize]
                )
            );
            assert_eq!(BorderLonghand::from_name(longhand.name()), Some(longhand));
            assert_eq!(BorderLonghand::from_key(longhand.key()), Some(longhand));
            assert!(super::super::mapping::is_known_longhand(longhand.name()));
            // Frame 대역이고 container longhand(0x0110–0x0121) 다음 빈 번호입니다.
            assert_eq!(longhand.key().0 >> 8, 0x01);
            assert!(longhand.key().0 > 0x0121);
        }
    }

    #[test]
    fn legacy_bundle_value_reads_as_uniform_solid_or_initial() {
        // 기존 Border{color, thickness}는 4변 같은 값과 style solid로, 없음은 none으로(IX.11.10).
        let border = Border::new(red(), 2.5).unwrap();
        let sides = BorderSides::from_payload(Some(border));
        for side in BorderSide::ALL {
            assert_eq!(sides.side(side).style, BorderStyle::Solid);
            assert_eq!(sides.side(side).computed_width(), 2.5);
            assert_eq!(sides.side(side).color, Some(red()));
        }
        assert_eq!(sides.to_payload(), Ok(Some(border)));
        let none = BorderSides::from_payload(None);
        assert_eq!(none, BorderSides::INITIAL);
        assert_eq!(none.computed_widths(), [0.0; 4]);
        assert_eq!(none.to_payload(), Ok(None));
        assert_eq!(
            none.declarations()[..3],
            [
                ("border-top-width", "medium".to_string()),
                ("border-top-style", "none".to_string()),
                ("border-top-color", "currentcolor".to_string()),
            ]
        );
    }

    #[test]
    fn four_different_sides_round_trip_through_text_and_schema_two_bytes() {
        // 검증 기준(IX.11.10): 4변 다른 값 round-trip.
        let mut sides = BorderSides::INITIAL;
        let values = [
            ("border-top-width", "1px"),
            ("border-top-style", "solid"),
            ("border-top-color", "#ff0000"),
            ("border-right-width", "2.5px"),
            ("border-right-style", "solid"),
            ("border-right-color", "color(display-p3 0 1 0)"),
            ("border-bottom-width", "thick"),
            ("border-bottom-style", "none"),
            ("border-bottom-color", "currentcolor"),
            ("border-left-width", "0px"),
            ("border-left-style", "solid"),
            ("border-left-color", "#00000080"),
        ];
        for (name, text) in values {
            sides
                .set(BorderLonghand::from_name(name).unwrap(), text)
                .unwrap();
        }
        let text: Vec<(&str, String)> = sides.declarations();
        for ((name, expected), (actual_name, actual)) in values.iter().zip(&text) {
            assert_eq!(name, actual_name);
            assert_eq!(*expected, actual.as_str());
        }
        assert_eq!(sides.computed_widths(), [1.0, 2.5, 0.0, 0.0]);
        let bytes = sides.encode();
        assert_eq!(BorderSides::decode(&bytes), Ok(sides));
        for end in 0..bytes.len() {
            assert!(BorderSides::decode(&bytes[..end]).is_err());
        }
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(BorderSides::decode(&extra).is_err());
        assert_eq!(sides.to_payload(), Err(BorderSidesError::NotUniform));
        assert_eq!(
            sides.layout_declarations(),
            [
                ("border-top-width", "1px".to_string()),
                ("border-right-width", "2.5px".to_string()),
                ("border-bottom-width", "0px".to_string()),
                ("border-left-width", "0px".to_string()),
            ]
        );
    }

    #[test]
    fn values_use_the_document_parser_and_reject_unsupported_forms() {
        let width = BorderLonghand::from_name("border-left-width").unwrap();
        let style = BorderLonghand::from_name("border-left-style").unwrap();
        let color = BorderLonghand::from_name("border-left-color").unwrap();
        let mut sides = BorderSides::INITIAL;
        assert_eq!(sides.set(width, "-1px").unwrap_err().code, "invalid-value");
        assert_eq!(
            sides.set(width, "2em").unwrap_err().code,
            "unsupported-unit"
        );
        assert_eq!(
            sides.set(style, "dashed").unwrap_err().code,
            "unsupported-value"
        );
        assert_eq!(sides.set(style, "wavy").unwrap_err().code, "invalid-value");
        assert!(sides.set(color, "url(x)").is_err());
        // 같은 해석 결과는 같은 정규 텍스트입니다.
        sides.set(color, "rgb(255 0 0)").unwrap();
        assert_eq!(sides.format(color), "#ff0000");
        sides.set(width, "1.50px").unwrap();
        assert_eq!(sides.format(width), "1.5px");
        // 묶음 값으로 표현할 수 없는 조합은 오류입니다.
        let mut solid = BorderSides::uniform(BorderSideValue {
            width: BorderWidth::Thin,
            style: BorderStyle::Solid,
            color: Some(red()),
        });
        assert_eq!(solid.to_payload(), Err(BorderSidesError::NotRepresentable));
        for side in BorderSide::ALL {
            solid.side_mut(side).style = BorderStyle::None;
        }
        assert_eq!(solid.to_payload(), Err(BorderSidesError::NoneWithValues));
    }

    #[test]
    fn registry_entries_are_exact_frame_longhands() {
        let entries: Vec<RegistryEntry> = registry_entries().collect();
        assert_eq!(entries.len(), 12);
        for (entry, longhand) in entries.iter().zip(BorderLonghand::ALL) {
            assert_eq!(entry.scope, PropertyScope::Frame);
            assert_eq!(entry.key, longhand.key());
            assert_eq!(entry.css.longhands, &[longhand.name()]);
            assert_eq!(entry.css.grammar, Some(longhand.grammar()));
        }
    }
}
