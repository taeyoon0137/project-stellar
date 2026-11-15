//! 속성 패널 값의 CSS 값 문법 parser·formatter (UX.2.2, 속성 모델 2장·8장, ADR-0024 결정 5).
//!
//! 입력은 속성 모델 2장의 형태만 받고, 그 밖은 구조화한 진단 코드(`unsupported-unit`, `unsupported-value`,
//! `unknown-token`, `forbidden-value`, `invalid-value`)로 거부합니다. 문법은 재귀가 없고 입력 길이에 상한이
//! 있습니다. 출력은 속성 모델 8.2·8.3절 정규 형식입니다(가장 짧은 왕복 10진, `px`·`deg` 단위, 소문자 hex 또는
//! `color()`).
//!
//! 문서 모델이 아직 고정 px만 저장하는 속성(`width` 등)에 `%`·`fit-content`·`var()`를 넣으면 문법은
//! 알아보되 적용하지 않고 이유를 진단합니다. 저장 형식은 IX.11.6(크기 방식)·토큰 연결이 정합니다.

use std::fmt;

use stellar_core_types::color::{Color, ColorSpace};

/// 입력 길이 상한(byte).
pub const MAX_INPUT_LEN: usize = 256;

/// 구조화한 값 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssValueError {
    /// 진단 코드(속성 모델 2장).
    pub code: &'static str,
    /// 사람이 읽는 이유.
    pub message: String,
}

impl CssValueError {
    fn new(code: &'static str, message: impl Into<String>) -> CssValueError {
        CssValueError {
            code,
            message: message.into(),
        }
    }

    fn invalid(message: impl Into<String>) -> CssValueError {
        CssValueError::new("invalid-value", message)
    }
}

impl fmt::Display for CssValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CssValueError {}

/// 속성 하나가 받는 값 문법.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CssGrammar {
    /// `<length>`(px). `non_negative`이면 음수 거부. `sizing`이면 크기 keyword(`auto`, `fit-content` …)를
    /// 알아보고 아직 저장할 수 없다고 진단합니다.
    Length {
        /// 음수 거부.
        non_negative: bool,
        /// 크기 속성(`width`·`height`).
        sizing: bool,
    },
    /// `<angle>`(deg로 정규화).
    Angle,
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
    /// keyword 하나.
    Keyword(&'static [&'static str]),
    /// `true`·`false`.
    Bool,
    /// 임의 문자열.
    Text,
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
                | CssGrammar::Angle
                | CssGrammar::Count { .. }
                | CssGrammar::Corners
        )
    }
}

/// 해석한 값.
#[derive(Debug, Clone, PartialEq)]
pub enum CssValue {
    /// 길이(px).
    Length(f32),
    /// 각도(deg).
    Angle(f32),
    /// 개수.
    Count(u32),
    /// 색(없음 = `None`).
    Color(Option<Color>),
    /// keyword의 목록 index.
    Keyword(usize),
    /// bool.
    Bool(bool),
    /// 문자열.
    Text(String),
    /// 테두리(두께 px, 색). 없음 = `None`.
    Border(Option<(f32, Color)>),
    /// 모서리 반경 TL, TR, BR, BL(px).
    Corners([f32; 4]),
}

const FORBIDDEN: [&str; 4] = ["url(", "@import", "image(", "image-set("];
const MATH: [&str; 4] = ["calc(", "min(", "max(", "clamp("];
const GLOBAL_KEYWORDS: [&str; 5] = ["inherit", "initial", "unset", "revert", "revert-layer"];
const SIZING_KEYWORDS: [&str; 5] = [
    "auto",
    "fit-content",
    "min-content",
    "max-content",
    "stretch",
];
const UNSUPPORTED_UNITS: [&str; 16] = [
    "em", "rem", "vw", "vh", "vmin", "vmax", "svw", "svh", "lvw", "lvh", "dvw", "dvh", "cqw",
    "cqh", "ch", "ex",
];

/// `text`를 `grammar`로 해석합니다.
pub fn parse(grammar: CssGrammar, text: &str) -> Result<CssValue, CssValueError> {
    if text.len() > MAX_INPUT_LEN {
        return Err(CssValueError::invalid("값이 너무 깁니다"));
    }
    if grammar == CssGrammar::Text {
        return Ok(CssValue::Text(text.to_string()));
    }
    let lower = text.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return Err(CssValueError::invalid("값이 비어 있습니다"));
    }
    if FORBIDDEN.iter().any(|f| lower.contains(f)) {
        return Err(CssValueError::new(
            "forbidden-value",
            "외부 리소스를 읽는 값은 받지 않습니다",
        ));
    }
    if let Some(rest) = lower.strip_prefix("var(") {
        let name = rest
            .trim_end_matches(')')
            .split(',')
            .next()
            .unwrap_or("")
            .trim();
        return Err(CssValueError::new(
            "unknown-token",
            format!("문서에 토큰 {name}이 없습니다"),
        ));
    }
    if MATH.iter().any(|f| lower.starts_with(f)) {
        return Err(CssValueError::new(
            "unsupported-value",
            "계산식은 아직 저장하지 못합니다",
        ));
    }
    if GLOBAL_KEYWORDS.contains(&lower.as_str()) {
        return Err(CssValueError::new(
            "unsupported-value",
            "문서 모델에는 cascade가 없어 전역 keyword를 받지 않습니다",
        ));
    }
    match grammar {
        CssGrammar::Length {
            non_negative,
            sizing,
        } => {
            if sizing && SIZING_KEYWORDS.contains(&lower.as_str()) {
                return Err(CssValueError::new(
                    "unsupported-value",
                    format!("{lower} 크기 방식은 아직 저장하지 못합니다. 고정 px 값을 넣으세요"),
                ));
            }
            let value = parse_length(&lower)?;
            if non_negative && value < 0.0 {
                return Err(CssValueError::invalid("음수는 받지 않습니다"));
            }
            Ok(CssValue::Length(value))
        }
        CssGrammar::Angle => parse_angle(&lower).map(CssValue::Angle),
        CssGrammar::Count { min, max } => {
            let (number, unit) = split_number(&lower)?;
            if !unit.is_empty() {
                return Err(CssValueError::new(
                    "unsupported-unit",
                    "개수에는 단위를 붙이지 않습니다",
                ));
            }
            if number.fract() != 0.0 || number < min as f32 || number > max as f32 {
                return Err(CssValueError::invalid(format!(
                    "{min}–{max} 사이의 정수를 넣으세요"
                )));
            }
            Ok(CssValue::Count(number as u32))
        }
        CssGrammar::Color { allow_none } => {
            if lower == "none" {
                return if allow_none {
                    Ok(CssValue::Color(None))
                } else {
                    Err(CssValueError::invalid("이 속성은 none을 받지 않습니다"))
                };
            }
            parse_color(&lower).map(|color| CssValue::Color(Some(color)))
        }
        CssGrammar::Keyword(choices) => choices
            .iter()
            .position(|choice| *choice == lower)
            .map(CssValue::Keyword)
            .ok_or_else(|| {
                CssValueError::new(
                    "unsupported-value",
                    format!("{} 중 하나를 넣으세요", choices.join(", ")),
                )
            }),
        CssGrammar::Bool => match lower.as_str() {
            "true" => Ok(CssValue::Bool(true)),
            "false" => Ok(CssValue::Bool(false)),
            _ => Err(CssValueError::invalid("true 또는 false를 넣으세요")),
        },
        CssGrammar::Text => unreachable!("앞에서 처리"),
        CssGrammar::Border => parse_border(&lower),
        CssGrammar::Corners => parse_corners(&lower),
    }
}

/// 부호·소수·지수가 있는 수와 나머지 단위로 나눕니다.
fn split_number(text: &str) -> Result<(f32, &str), CssValueError> {
    let bytes = text.as_bytes();
    let mut end = 0;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        end = 1;
    }
    let digits_start = end;
    while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == b'.') {
        end += 1;
    }
    if end == digits_start {
        return Err(CssValueError::invalid(format!("{text}은 수가 아닙니다")));
    }
    // 지수(입력에서만): e 뒤에 숫자가 올 때만. `em` 단위와 구분합니다.
    if end + 1 < bytes.len() && matches!(bytes[end], b'e') {
        let mut exp = end + 1;
        if matches!(bytes.get(exp), Some(b'+' | b'-')) {
            exp += 1;
        }
        if bytes.get(exp).is_some_and(u8::is_ascii_digit) {
            while exp < bytes.len() && bytes[exp].is_ascii_digit() {
                exp += 1;
            }
            end = exp;
        }
    }
    let number: f32 = text[..end]
        .parse()
        .map_err(|_| CssValueError::invalid(format!("{text}은 수가 아닙니다")))?;
    if !number.is_finite() {
        return Err(CssValueError::invalid("수가 너무 큽니다"));
    }
    Ok((number, text[end..].trim()))
}

fn parse_length(text: &str) -> Result<f32, CssValueError> {
    let (number, unit) = split_number(text)?;
    match unit {
        "" | "px" => Ok(number),
        "%" => Err(CssValueError::new(
            "unsupported-value",
            "이 속성은 아직 % 값을 저장하지 못합니다",
        )),
        unit if UNSUPPORTED_UNITS.contains(&unit) => Err(CssValueError::new(
            "unsupported-unit",
            format!("{unit} 단위는 아직 받지 않습니다. px를 쓰세요"),
        )),
        unit => Err(CssValueError::new(
            "unsupported-unit",
            format!("{unit}은 길이 단위가 아닙니다"),
        )),
    }
}

fn parse_angle(text: &str) -> Result<f32, CssValueError> {
    let (number, unit) = split_number(text)?;
    let degrees = match unit {
        "" | "deg" => number,
        "rad" => number.to_degrees(),
        "grad" => number * 0.9,
        "turn" => number * 360.0,
        unit => {
            return Err(CssValueError::new(
                "unsupported-unit",
                format!("{unit}은 각도 단위가 아닙니다"),
            ))
        }
    };
    if degrees.is_finite() {
        Ok(degrees)
    } else {
        Err(CssValueError::invalid("각도가 너무 큽니다"))
    }
}

fn parse_corners(text: &str) -> Result<CssValue, CssValueError> {
    if text.contains('/') {
        return Err(CssValueError::new(
            "unsupported-value",
            "타원 모서리(/)는 받지 않습니다",
        ));
    }
    let mut values = Vec::new();
    for part in text.split_whitespace() {
        let value = parse_length(part)?;
        if value < 0.0 {
            return Err(CssValueError::invalid("음수는 받지 않습니다"));
        }
        values.push(value);
    }
    let corners = match values.as_slice() {
        [a] => [*a; 4],
        [a, b] => [*a, *b, *a, *b],
        [a, b, c] => [*a, *b, *c, *b],
        [a, b, c, d] => [*a, *b, *c, *d],
        _ => return Err(CssValueError::invalid("길이를 1–4개 넣으세요")),
    };
    Ok(CssValue::Corners(corners))
}

fn parse_border(text: &str) -> Result<CssValue, CssValueError> {
    if text == "none" {
        return Ok(CssValue::Border(None));
    }
    let mut width = None;
    let mut color = None;
    // 색 함수 안의 공백을 보존하려고 괄호 깊이로 나눕니다.
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (index, ch) in text.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            c if c.is_whitespace() && depth == 0 => {
                if start < index {
                    parts.push(&text[start..index]);
                }
                start = index + c.len_utf8();
            }
            _ => {}
        }
    }
    if start < text.len() {
        parts.push(&text[start..]);
    }
    for part in parts {
        match part {
            "solid" => {}
            "none" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset"
            | "hidden" => {
                return Err(CssValueError::new(
                    "unsupported-value",
                    "테두리 모양은 solid만 받습니다",
                ))
            }
            part if part
                .starts_with(|c: char| c.is_ascii_digit() || c == '.' || c == '-' || c == '+') =>
            {
                let value = parse_length(part)?;
                if value < 0.0 {
                    return Err(CssValueError::invalid("음수는 받지 않습니다"));
                }
                width = Some(value);
            }
            part => color = Some(parse_color(part)?),
        }
    }
    let black = Color::new(ColorSpace::Srgb, 0., 0., 0., 1.).expect("black");
    match (width, color) {
        (None, None) => Err(CssValueError::invalid("두께나 색을 넣으세요")),
        (width, color) => Ok(CssValue::Border(Some((
            width.unwrap_or(1.0),
            color.unwrap_or(black),
        )))),
    }
}

const NAMED: [(&str, [u8; 3]); 18] = [
    ("black", [0, 0, 0]),
    ("white", [255, 255, 255]),
    ("red", [255, 0, 0]),
    ("green", [0, 128, 0]),
    ("blue", [0, 0, 255]),
    ("yellow", [255, 255, 0]),
    ("cyan", [0, 255, 255]),
    ("magenta", [255, 0, 255]),
    ("gray", [128, 128, 128]),
    ("grey", [128, 128, 128]),
    ("silver", [192, 192, 192]),
    ("orange", [255, 165, 0]),
    ("purple", [128, 0, 128]),
    ("pink", [255, 192, 203]),
    ("brown", [165, 42, 42]),
    ("navy", [0, 0, 128]),
    ("teal", [0, 128, 128]),
    ("lime", [0, 255, 0]),
];

fn srgb(r: f32, g: f32, b: f32, a: f32) -> Result<Color, CssValueError> {
    Color::new(ColorSpace::Srgb, r, g, b, a)
        .map_err(|_| CssValueError::invalid("색 값이 범위를 벗어났습니다"))
}

/// CSS 색을 해석합니다(속성 모델 2장: hex, `rgb()`, `color(srgb …)`, `color(display-p3 …)`, `transparent`,
/// 이름 색).
pub fn parse_color(text: &str) -> Result<Color, CssValueError> {
    let text = text.trim();
    if text == "transparent" {
        return srgb(0., 0., 0., 0.);
    }
    if let Some((_, [r, g, b])) = NAMED.iter().find(|(name, _)| *name == text) {
        return srgb(
            f32::from(*r) / 255.,
            f32::from(*g) / 255.,
            f32::from(*b) / 255.,
            1.,
        );
    }
    if let Some(hex) = text.strip_prefix('#') {
        return parse_hex(hex);
    }
    if let Some(body) = text
        .strip_prefix("rgba(")
        .or_else(|| text.strip_prefix("rgb("))
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let (channels, alpha) = split_alpha(body)?;
        let [r, g, b] = channels.map(rgb_channel);
        return srgb(r?, g?, b?, alpha);
    }
    if let Some(body) = text
        .strip_prefix("color(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let body = body.trim();
        let (space, rest) = if let Some(rest) = body.strip_prefix("srgb") {
            (ColorSpace::Srgb, rest)
        } else if let Some(rest) = body.strip_prefix("display-p3") {
            (ColorSpace::DisplayP3, rest)
        } else {
            return Err(CssValueError::new(
                "unsupported-value",
                "color()는 srgb와 display-p3만 받습니다",
            ));
        };
        let (channels, alpha) = split_alpha(rest)?;
        let [r, g, b] = channels.map(unit_channel);
        return Color::new(space, r?, g?, b?, alpha)
            .map_err(|_| CssValueError::invalid("색 값이 범위를 벗어났습니다"));
    }
    if text.starts_with("hsl")
        || text.starts_with("hwb")
        || text.starts_with("lab")
        || text.starts_with("lch")
        || text.starts_with("oklab")
        || text.starts_with("oklch")
    {
        return Err(CssValueError::new(
            "unsupported-value",
            "hex, rgb(), color(srgb …), color(display-p3 …)를 쓰세요",
        ));
    }
    Err(CssValueError::invalid(format!("{text}은 색이 아닙니다")))
}

fn parse_hex(hex: &str) -> Result<Color, CssValueError> {
    let digit = |c: u8| (c as char).to_digit(16).map(|d| d as f32);
    let bytes = hex.as_bytes();
    let nibble =
        |i: usize| digit(bytes[i]).ok_or_else(|| CssValueError::invalid("hex 숫자가 아닙니다"));
    let pair = |i: usize| Ok::<f32, CssValueError>((nibble(i)? * 16. + nibble(i + 1)?) / 255.);
    let short = |i: usize| Ok::<f32, CssValueError>(nibble(i)? * 17. / 255.);
    match bytes.len() {
        3 => srgb(short(0)?, short(1)?, short(2)?, 1.),
        4 => srgb(short(0)?, short(1)?, short(2)?, short(3)?),
        6 => srgb(pair(0)?, pair(2)?, pair(4)?, 1.),
        8 => srgb(pair(0)?, pair(2)?, pair(4)?, pair(6)?),
        _ => Err(CssValueError::invalid("hex 색은 3·4·6·8자리입니다")),
    }
}

/// `r g b / a` 또는 `r, g, b, a`를 채널 3개와 alpha로 나눕니다.
fn split_alpha(body: &str) -> Result<([&str; 3], f32), CssValueError> {
    let (channels, alpha) = match body.split_once('/') {
        Some((channels, alpha)) => (channels, Some(alpha.trim())),
        None => (body, None),
    };
    let parts: Vec<&str> = channels
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .collect();
    let (rgb, alpha) = match (parts.as_slice(), alpha) {
        ([r, g, b], alpha) => ([*r, *g, *b], alpha),
        ([r, g, b, a], None) => ([*r, *g, *b], Some(*a)),
        _ => return Err(CssValueError::invalid("채널 3개(와 alpha)를 넣으세요")),
    };
    let alpha = match alpha {
        None => 1.0,
        Some(alpha) => unit_channel(alpha)?,
    };
    if !(0.0..=1.0).contains(&alpha) {
        return Err(CssValueError::invalid("alpha는 0–1입니다"));
    }
    Ok((rgb, alpha))
}

fn rgb_channel(text: &str) -> Result<f32, CssValueError> {
    let (number, unit) = split_number(text)?;
    match unit {
        "" => Ok(number / 255.),
        "%" => Ok(number / 100.),
        _ => Err(CssValueError::new(
            "unsupported-unit",
            "rgb() 채널은 수 또는 %입니다",
        )),
    }
}

fn unit_channel(text: &str) -> Result<f32, CssValueError> {
    let (number, unit) = split_number(text)?;
    match unit {
        "" => Ok(number),
        "%" => Ok(number / 100.),
        _ => Err(CssValueError::new(
            "unsupported-unit",
            "채널은 수 또는 %입니다",
        )),
    }
}

/// f32를 왕복하는 가장 짧은 10진 표기(지수 없음, `-0`은 `0`).
pub fn format_number(value: f32) -> String {
    if value == 0.0 {
        return "0".into();
    }
    format!("{value}")
}

/// `<n>px`.
pub fn format_length(value: f32) -> String {
    format!("{}px", format_number(value))
}

/// `<n>deg`.
pub fn format_angle(value: f32) -> String {
    format!("{}deg", format_number(value))
}

/// 속성 모델 8.2절 정규 색 표기.
pub fn format_color(color: Color) -> String {
    let channels = [color.r(), color.g(), color.b(), color.alpha()];
    let byte = |v: f32| (v * 255.0).round();
    let exact = color.space() == ColorSpace::Srgb
        && channels
            .iter()
            .all(|v| (0.0..=1.0).contains(v) && byte(*v) / 255.0 == *v);
    if exact {
        let mut out = String::from("#");
        for v in &channels[..3] {
            out.push_str(&format!("{:02x}", byte(*v) as u8));
        }
        if color.alpha() != 1.0 {
            out.push_str(&format!("{:02x}", byte(color.alpha()) as u8));
        }
        return out;
    }
    let space = match color.space() {
        ColorSpace::Srgb => "srgb",
        ColorSpace::DisplayP3 => "display-p3",
    };
    let [r, g, b, a] = channels.map(format_number);
    if color.alpha() == 1.0 {
        format!("color({space} {r} {g} {b})")
    } else {
        format!("color({space} {r} {g} {b} / {a})")
    }
}

/// `border-radius` 4값을 CSS 최소 shorthand로 씁니다.
pub fn format_corners([tl, tr, br, bl]: [f32; 4]) -> String {
    let parts: Vec<String> = if tl == tr && tr == br && br == bl {
        vec![format_length(tl)]
    } else if tl == br && tr == bl {
        vec![format_length(tl), format_length(tr)]
    } else if tr == bl {
        vec![format_length(tl), format_length(tr), format_length(br)]
    } else {
        [tl, tr, br, bl].map(format_length).to_vec()
    };
    parts.join(" ")
}

/// 정규 값 텍스트.
pub fn format(value: &CssValue, grammar: CssGrammar) -> String {
    match value {
        CssValue::Length(v) => format_length(*v),
        CssValue::Angle(v) => format_angle(*v),
        CssValue::Count(v) => v.to_string(),
        CssValue::Color(None) => "none".into(),
        CssValue::Color(Some(color)) => format_color(*color),
        CssValue::Keyword(index) => match grammar {
            CssGrammar::Keyword(choices) => choices.get(*index).copied().unwrap_or("").into(),
            _ => String::new(),
        },
        CssValue::Bool(v) => v.to_string(),
        CssValue::Text(text) => text.clone(),
        CssValue::Border(None) => "none".into(),
        CssValue::Border(Some((width, color))) => {
            format!("{} solid {}", format_length(*width), format_color(*color))
        }
        CssValue::Corners(corners) => format_corners(*corners),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: CssGrammar = CssGrammar::Length {
        non_negative: true,
        sizing: true,
    };

    fn ok(grammar: CssGrammar, text: &str) -> String {
        format(&parse(grammar, text).unwrap(), grammar)
    }

    fn code(grammar: CssGrammar, text: &str) -> &'static str {
        parse(grammar, text).unwrap_err().code
    }

    #[test]
    fn lengths_and_angles_normalize() {
        assert_eq!(ok(WIDTH, "320"), "320px");
        assert_eq!(ok(WIDTH, " 12.50PX "), "12.5px");
        assert_eq!(ok(WIDTH, "0"), "0px");
        assert_eq!(ok(WIDTH, "1e2"), "100px");
        assert_eq!(ok(CssGrammar::Angle, "1.5turn"), "540deg");
        assert_eq!(ok(CssGrammar::Angle, "200grad"), "180deg");
        assert_eq!(ok(CssGrammar::Angle, "-0"), "0deg");
        assert_eq!(ok(CssGrammar::Count { min: 3, max: 1000 }, "8"), "8");
        assert_eq!(ok(CssGrammar::Corners, "12"), "12px");
        assert_eq!(ok(CssGrammar::Corners, "1px 2px"), "1px 2px");
        assert_eq!(
            parse(CssGrammar::Corners, "1 2 3").unwrap(),
            CssValue::Corners([1., 2., 3., 2.])
        );
    }

    #[test]
    fn unsupported_inputs_return_structured_codes() {
        assert_eq!(code(WIDTH, "50%"), "unsupported-value");
        assert_eq!(code(WIDTH, "fit-content"), "unsupported-value");
        assert_eq!(code(WIDTH, "var(--space-2)"), "unknown-token");
        assert_eq!(code(WIDTH, "2rem"), "unsupported-unit");
        assert_eq!(code(WIDTH, "2em"), "unsupported-unit");
        assert_eq!(code(WIDTH, "calc(100% - 4px)"), "unsupported-value");
        assert_eq!(code(WIDTH, "inherit"), "unsupported-value");
        assert_eq!(code(WIDTH, "-4"), "invalid-value");
        assert_eq!(code(WIDTH, "abc"), "invalid-value");
        assert_eq!(
            code(CssGrammar::Color { allow_none: true }, "url(x.png)"),
            "forbidden-value"
        );
        assert_eq!(code(CssGrammar::Corners, "4px / 2px"), "unsupported-value");
        assert_eq!(
            code(CssGrammar::Count { min: 3, max: 1000 }, "2"),
            "invalid-value"
        );
        assert_eq!(code(WIDTH, &"9".repeat(MAX_INPUT_LEN + 1)), "invalid-value");
    }

    #[test]
    fn colors_round_trip_in_canonical_form() {
        let color = CssGrammar::Color { allow_none: true };
        assert_eq!(ok(color, "#3366FF"), "#3366ff");
        assert_eq!(ok(color, "#36f"), "#3366ff");
        assert_eq!(ok(color, "rgb(51, 102, 255)"), "#3366ff");
        assert_eq!(
            ok(color, "rgb(51 102 255 / 50%)"),
            "color(srgb 0.2 0.4 1 / 0.5)"
        );
        assert_eq!(ok(color, "#00000080"), "#00000080");
        assert_eq!(ok(color, "none"), "none");
        assert_eq!(ok(color, "white"), "#ffffff");
        assert_eq!(
            ok(color, "color(display-p3 1 0 0)"),
            "color(display-p3 1 0 0)"
        );
        for text in [
            "#d9d9d9",
            "#000000",
            "color(srgb 0.1 0.2 0.3)",
            "color(display-p3 0.5 0.25 1 / 0.75)",
        ] {
            let parsed = parse(color, text).unwrap();
            assert_eq!(format(&parsed, color), text);
            assert_eq!(parse(color, &format(&parsed, color)).unwrap(), parsed);
        }
        assert_eq!(
            code(CssGrammar::Color { allow_none: false }, "none"),
            "invalid-value"
        );
        assert_eq!(code(color, "hsl(0 100% 50%)"), "unsupported-value");
    }

    #[test]
    fn border_shorthand_accepts_any_order() {
        let a = parse(CssGrammar::Border, "2px solid #ff0000").unwrap();
        let b = parse(CssGrammar::Border, "red 2").unwrap();
        assert_eq!(a, b);
        assert_eq!(format(&a, CssGrammar::Border), "2px solid #ff0000");
        assert_eq!(ok(CssGrammar::Border, "none"), "none");
        assert_eq!(
            ok(CssGrammar::Border, "color(srgb 1 0 0 / 0.5)"),
            "1px solid color(srgb 1 0 0 / 0.5)"
        );
        assert_eq!(
            code(CssGrammar::Border, "1px dashed red"),
            "unsupported-value"
        );
    }

    #[test]
    fn number_format_round_trips_f32() {
        for value in [
            0.1f32,
            1.0 / 3.0,
            123456.79,
            -0.000_123,
            16777217.0,
            f32::MIN_POSITIVE,
        ] {
            let text = format_number(value);
            assert!(!text.contains('e'), "{text}");
            assert_eq!(text.parse::<f32>().unwrap(), value);
        }
        assert_eq!(format_number(-0.0), "0");
    }

    #[test]
    fn keywords_are_case_insensitive() {
        let fit = CssGrammar::Keyword(&["fill", "contain", "cover", "none"]);
        assert_eq!(ok(fit, "COVER"), "cover");
        assert_eq!(code(fit, "scale-down"), "unsupported-value");
        assert_eq!(ok(CssGrammar::Bool, "TRUE"), "true");
        assert_eq!(ok(CssGrammar::Text, " 카드 "), " 카드 ");
    }
}
