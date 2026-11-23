//! registry 값 문법별 CSS 값 formatter·parser (IX.11.13·IX.11.14, 속성 모델 2장·8장, ADR-0024 결정 5,
//! ADR-0075 결정 5).
//!
//! **formatter**([`format`])는 속성 모델 8.2·8.3절 정규 형식을 냅니다. 수는 f32를 왕복하는 가장 짧은 10진(지수
//! 없음, `-0`은 `0`), 단위는 `px`·`%`·`deg`·`ms`(0도 단위를 붙임), keyword는 소문자, 색은 sRGB 범위 안 8비트
//! 정확 값이면 소문자 `#rrggbb[aa]`, 그 밖은 `color(srgb …)`·`color(display-p3 …)`, transform은 `none` 또는
//! `matrix(a, b, c, d, e, f)`, 토큰 참조는 `var(--name)`입니다. 같은 값은 항상 byte 단위로 같은 텍스트입니다.
//!
//! **parser**([`parse`])는 속성 모델 2장의 입력 형태만 받고, 그 밖은 구조화한 진단 코드(`unsupported-unit`,
//! `unsupported-value`, `unknown-token`, `forbidden-value`, `invalid-value`)로 거부합니다. 문법은 재귀가 없고
//! 입력 길이에 상한이 있습니다.
//!
//! 문서 모델이 아직 고정 px만 저장하는 속성(`width` 등)에 `%`·`fit-content`를 넣으면 문법은 알아보되 적용하지
//! 않고 이유를 진단합니다. 저장 형식은 IX.11.6(크기 방식)이 정합니다.

use std::fmt;

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;

pub use super::grammar::CssGrammar;

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
    /// 코드와 이유로 만듭니다.
    pub fn new(code: &'static str, message: impl Into<String>) -> CssValueError {
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

/// 토큰 참조(`var(--name)`). 이름은 CSS custom property 이름(`--` 포함, IX.11.16 파생)이고 문서 안 참조는
/// `token` StableId입니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssVar {
    /// `--`로 시작하는 custom property 이름.
    pub name: String,
    /// 참조 대상 토큰.
    pub token: StableId,
}

/// 해석한 값.
#[derive(Debug, Clone, PartialEq)]
pub enum CssValue {
    /// 길이(px).
    Length(f32),
    /// 백분율(%, 50 = 50%).
    Percentage(f32),
    /// 각도(deg).
    Angle(f32),
    /// 시간(ms).
    Duration(f32),
    /// 수(`<number>`, 0..=1 `opacity` 포함).
    Number(f32),
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
    /// 두께나 색 하나만 적은 `border`(빠진 쪽은 편집 대상의 지금 테두리 값을 이어 씁니다).
    BorderParts(Option<f32>, Option<Color>),
    /// 모서리 반경 TL, TR, BR, BL(px).
    Corners([f32; 4]),
    /// 2D 아핀 변환 `(a, b, c, d, e, f)`(CSS `matrix()` 순서, 이동은 px).
    Transform([f32; 6]),
    /// 토큰 참조.
    Var(CssVar),
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
    parse_with(grammar, text, &NoTokens)
}

/// `var(--name)`의 이름을 문서 토큰으로 푸는 scope(IX.11.16 이름 파생 결과).
pub trait TokenScope {
    /// custom property 이름(`--` 포함)의 토큰. 없으면 `None`.
    fn resolve(&self, name: &str) -> Option<StableId>;
}

/// 토큰이 없는 scope.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoTokens;

impl TokenScope for NoTokens {
    fn resolve(&self, _name: &str) -> Option<StableId> {
        None
    }
}

impl TokenScope for std::collections::BTreeMap<String, StableId> {
    fn resolve(&self, name: &str) -> Option<StableId> {
        self.get(name).copied()
    }
}

/// CSS custom property 이름(`--` + identifier 문자, escape `\` 허용)인지. 파생 규칙은 IX.11.16입니다.
pub fn is_custom_property_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("--") else {
        return false;
    };
    let mut chars = rest.chars();
    let mut any = false;
    while let Some(c) = chars.next() {
        any = true;
        match c {
            '\\' => {
                if chars.next().is_none() {
                    return false;
                }
            }
            c if c.is_ascii_alphanumeric() || c == '-' || c == '_' || !c.is_ascii() => {}
            _ => return false,
        }
    }
    any
}

/// `var(--name)`을 해석합니다. 이름은 대소문자를 구분합니다. fallback(`var(--a, 1px)`)은 문서 토큰이 항상
/// 있어야 하므로 받지 않습니다.
fn parse_var(text: &str, scope: &dyn TokenScope) -> Result<CssValue, CssValueError> {
    let body = text[4..]
        .strip_suffix(')')
        .ok_or_else(|| CssValueError::invalid("var()가 닫히지 않았습니다"))?
        .trim();
    if body.contains(',') {
        return Err(CssValueError::new(
            "unsupported-value",
            "var() fallback은 받지 않습니다",
        ));
    }
    if !is_custom_property_name(body) {
        return Err(CssValueError::invalid(format!(
            "{body}은 custom property 이름이 아닙니다"
        )));
    }
    match scope.resolve(body) {
        Some(token) => Ok(CssValue::Var(CssVar {
            name: body.to_string(),
            token,
        })),
        None => Err(CssValueError::new(
            "unknown-token",
            format!("문서에 토큰 {body}이 없습니다"),
        )),
    }
}

/// `text`를 `grammar`로 해석합니다. `var(--name)`은 `scope`로 풉니다.
pub fn parse_with(
    grammar: CssGrammar,
    text: &str,
    scope: &dyn TokenScope,
) -> Result<CssValue, CssValueError> {
    if text.len() > MAX_INPUT_LEN {
        return Err(CssValueError::invalid("값이 너무 깁니다"));
    }
    if grammar == CssGrammar::Text {
        return Ok(CssValue::Text(text.to_string()));
    }
    let trimmed = text.trim();
    if trimmed
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("var("))
        && grammar != CssGrammar::Bool
    {
        return parse_var(trimmed, scope);
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.is_empty() {
        return Err(CssValueError::invalid("값이 비어 있습니다"));
    }
    if FORBIDDEN.iter().any(|f| lower.contains(f)) {
        return Err(CssValueError::new(
            "forbidden-value",
            "외부 리소스를 읽는 값은 받지 않습니다",
        ));
    }
    if lower.contains("var(") {
        return Err(CssValueError::new(
            "unsupported-value",
            "var()는 값 전체로만 받습니다",
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
        CssGrammar::Percentage => {
            let (number, unit) = split_number(&lower)?;
            match unit {
                "%" => Ok(CssValue::Percentage(number)),
                _ => Err(CssValueError::new(
                    "unsupported-unit",
                    "백분율은 %로 넣으세요",
                )),
            }
        }
        CssGrammar::Duration => {
            let (number, unit) = split_number(&lower)?;
            let ms = match unit {
                "ms" => number,
                "s" => number * 1000.,
                "" if number == 0.0 => 0.0,
                _ => {
                    return Err(CssValueError::new(
                        "unsupported-unit",
                        "시간은 ms 또는 s로 넣으세요",
                    ))
                }
            };
            finite_value(ms).map(CssValue::Duration)
        }
        CssGrammar::Number => {
            let (number, unit) = split_number(&lower)?;
            if !unit.is_empty() {
                return Err(CssValueError::new(
                    "unsupported-unit",
                    "수에는 단위를 붙이지 않습니다",
                ));
            }
            Ok(CssValue::Number(number))
        }
        CssGrammar::UnitInterval => {
            let (number, unit) = split_number(&lower)?;
            let value = match unit {
                "" => number,
                "%" => number / 100.,
                _ => {
                    return Err(CssValueError::new(
                        "unsupported-unit",
                        "0–1 수 또는 %를 넣으세요",
                    ))
                }
            };
            if !(0.0..=1.0).contains(&value) {
                return Err(CssValueError::invalid("0–1 사이 값을 넣으세요"));
            }
            Ok(CssValue::Number(value))
        }
        CssGrammar::Transform => parse_transform(&lower).map(CssValue::Transform),
        CssGrammar::Border => parse_border(&lower),
        CssGrammar::Corners => parse_corners(&lower),
    }
}

fn finite_value(value: f32) -> Result<f32, CssValueError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(CssValueError::invalid("수가 너무 큽니다"))
    }
}

/// 행렬 곱 `a ∘ b`(b를 먼저 적용). 성분 순서는 CSS `matrix(a, b, c, d, e, f)`입니다.
fn compose(a: [f32; 6], b: [f32; 6]) -> [f32; 6] {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}

/// transform 함수 인자: `,` 또는 공백으로 나눕니다.
fn arguments(body: &str) -> Vec<&str> {
    body.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .collect()
}

fn unitless(text: &str) -> Result<f32, CssValueError> {
    let (number, unit) = split_number(text)?;
    if unit.is_empty() {
        Ok(number)
    } else {
        Err(CssValueError::new(
            "unsupported-unit",
            format!("{text}: 단위 없는 수를 넣으세요"),
        ))
    }
}

/// `<transform-list>` 2D 부분집합. 함수는 왼쪽부터 오른쪽으로 곱합니다(CSS: 오른쪽 함수가 먼저 적용).
/// 직각 배수가 아닌 `rotate()`와 `skew*()`는 플랫폼 의존 초월함수가 필요해 받지 않습니다(속성 모델 15장 O13).
fn parse_transform(text: &str) -> Result<[f32; 6], CssValueError> {
    if text == "none" {
        return Ok(IDENTITY);
    }
    let mut matrix = IDENTITY;
    let mut rest = text.trim();
    let mut count = 0;
    while !rest.is_empty() {
        count += 1;
        if count > 16 {
            return Err(CssValueError::invalid("transform 함수가 너무 많습니다"));
        }
        let open = rest
            .find('(')
            .ok_or_else(|| CssValueError::invalid("transform 함수 이름 뒤에 (가 필요합니다"))?;
        let close = rest
            .find(')')
            .ok_or_else(|| CssValueError::invalid("transform 함수가 닫히지 않았습니다"))?;
        if close < open {
            return Err(CssValueError::invalid("transform 괄호 순서가 틀렸습니다"));
        }
        let name = rest[..open].trim();
        let args = arguments(&rest[open + 1..close]);
        if args.iter().any(|a| a.contains('(')) {
            return Err(CssValueError::invalid(
                "transform 인자 안에 함수는 받지 않습니다",
            ));
        }
        let arity = |min: usize, max: usize| -> Result<(), CssValueError> {
            if (min..=max).contains(&args.len()) {
                Ok(())
            } else {
                Err(CssValueError::invalid(format!(
                    "{name}()에는 인자를 {min}–{max}개 넣으세요"
                )))
            }
        };
        let step = match name {
            "matrix" => {
                arity(6, 6)?;
                let mut m = [0f32; 6];
                for (slot, arg) in m.iter_mut().zip(&args) {
                    *slot = unitless(arg)?;
                }
                m
            }
            "translate" | "translatex" | "translatey" => {
                arity(1, if name == "translate" { 2 } else { 1 })?;
                let x = parse_length(args[0])?;
                let y = args
                    .get(1)
                    .map(|a| parse_length(a))
                    .transpose()?
                    .unwrap_or(0.);
                match name {
                    "translatey" => [1., 0., 0., 1., 0., x],
                    _ => [1., 0., 0., 1., x, y],
                }
            }
            "scale" | "scalex" | "scaley" => {
                arity(1, if name == "scale" { 2 } else { 1 })?;
                let sx = unitless(args[0])?;
                match name {
                    "scalex" => [sx, 0., 0., 1., 0., 0.],
                    "scaley" => [1., 0., 0., sx, 0., 0.],
                    _ => {
                        let sy = args.get(1).map(|a| unitless(a)).transpose()?.unwrap_or(sx);
                        [sx, 0., 0., sy, 0., 0.]
                    }
                }
            }
            "rotate" => {
                arity(1, 1)?;
                let degrees = parse_angle(args[0])?;
                let quarter = degrees / 90.;
                if quarter.fract() != 0.0 {
                    return Err(CssValueError::new(
                        "unsupported-value",
                        "직각 배수가 아닌 rotate()는 아직 받지 않습니다. matrix()를 쓰세요",
                    ));
                }
                match quarter.rem_euclid(4.) as u8 {
                    0 => IDENTITY,
                    1 => [0., 1., -1., 0., 0., 0.],
                    2 => [-1., 0., 0., -1., 0., 0.],
                    _ => [0., -1., 1., 0., 0., 0.],
                }
            }
            "skew" | "skewx" | "skewy" | "rotatez" => {
                return Err(CssValueError::new(
                    "unsupported-value",
                    format!("{name}()는 아직 받지 않습니다. matrix()를 쓰세요"),
                ))
            }
            other if other.ends_with("3d") || other.ends_with('z') || other == "perspective" => {
                return Err(CssValueError::new(
                    "unsupported-value",
                    "3D transform은 받지 않습니다",
                ))
            }
            other => {
                return Err(CssValueError::invalid(format!(
                    "{other}()는 transform 함수가 아닙니다"
                )))
            }
        };
        matrix = compose(matrix, step);
        if matrix.iter().any(|v| !v.is_finite()) {
            return Err(CssValueError::invalid("transform 값이 너무 큽니다"));
        }
        rest = rest[close + 1..].trim_start();
    }
    Ok(matrix.map(|v| v + 0.0))
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
    // 빠진 쪽의 기본값(두께·색)은 여기서 정하지 않습니다. 편집 대상의 지금 테두리 값을 이어 쓰거나 거부합니다(D7).
    match (width, color) {
        (None, None) => Err(CssValueError::invalid("두께나 색을 넣으세요")),
        (Some(width), Some(color)) => Ok(CssValue::Border(Some((width, color)))),
        (width, color) => Ok(CssValue::BorderParts(width, color)),
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

/// `<n>%`.
pub fn format_percentage(value: f32) -> String {
    format!("{}%", format_number(value))
}

/// `<n>ms`.
pub fn format_duration(value: f32) -> String {
    format!("{}ms", format_number(value))
}

/// 단위 행렬.
pub const IDENTITY: [f32; 6] = [1., 0., 0., 1., 0., 0.];

/// `none`(단위 행렬) 또는 `matrix(a, b, c, d, e, f)`. 분해(`rotate`·`scale`)는 플랫폼 의존 초월함수가 필요해
/// 정규형으로 쓰지 않습니다(속성 모델 5장 `transform` 행, 15장 O13).
pub fn format_transform(matrix: [f32; 6]) -> String {
    if matrix.map(|v| v + 0.0) == IDENTITY {
        return "none".into();
    }
    let parts = matrix.map(format_number);
    format!("matrix({})", parts.join(", "))
}

/// `var(--name)`.
pub fn format_var(var: &CssVar) -> String {
    format!("var({})", var.name)
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
        CssValue::Percentage(v) => format_percentage(*v),
        CssValue::Angle(v) => format_angle(*v),
        CssValue::Duration(v) => format_duration(*v),
        CssValue::Number(v) => format_number(*v),
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
        CssValue::BorderParts(width, color) => {
            let mut parts: Vec<String> = width.iter().map(|w| format_length(*w)).collect();
            parts.push("solid".into());
            parts.extend(color.iter().map(|c| format_color(*c)));
            parts.join(" ")
        }
        CssValue::Corners(corners) => format_corners(*corners),
        CssValue::Transform(matrix) => format_transform(*matrix),
        CssValue::Var(var) => format_var(var),
    }
}

/// **테스트 전용** CSS 값 parser fuzz target(IX.11.14). 외부 fuzz 엔진 없이 쓰는 결정론적 harness입니다. 입력
/// bytes 하나를 UTF-8(손실 허용) 텍스트로 바꿔 registry의 모든 값 문법 parser와 모든 shorthand 확장에 넣고,
/// 받아들인 값은 정규 텍스트로 다시 쓴 뒤 다시 해석해 같은 값이 나오는지 봅니다. 어떤 입력에도 panic이
/// 없어야 합니다(거부는 정상). 반환값은 받아들인 해석 수입니다.
#[cfg(any(test, feature = "test-support"))]
pub fn fuzz_one(input: &[u8]) -> usize {
    let text = String::from_utf8_lossy(input);
    let mut grammars: Vec<CssGrammar> = super::registry::entries()
        .into_iter()
        .filter_map(|e| e.css.grammar)
        .collect();
    grammars.extend([
        CssGrammar::Percentage,
        CssGrammar::Duration,
        CssGrammar::Number,
        CssGrammar::Border,
        CssGrammar::Corners,
        CssGrammar::Transform,
    ]);
    let mut accepted = 0;
    for grammar in grammars {
        if let Ok(value) = parse(grammar, &text) {
            accepted += 1;
            if grammar != CssGrammar::Text {
                let canonical = format(&value, grammar);
                let again = parse(grammar, &canonical).unwrap_or_else(|e| {
                    panic!("정규 텍스트 재해석 실패 {grammar:?} {canonical:?}: {e}")
                });
                assert_eq!(format(&again, grammar), canonical, "{grammar:?}");
            }
        }
    }
    for (shorthand, _) in super::mapping::CSS_SHORTHANDS {
        if let Some(Ok(_)) = super::shorthand::expand(shorthand, &text) {
            accepted += 1;
        }
    }
    accepted
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
            "solid color(srgb 1 0 0 / 0.5)"
        );
        assert_eq!(
            parse(CssGrammar::Border, "3px solid").unwrap(),
            CssValue::BorderParts(Some(3.0), None)
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

    fn red() -> Color {
        Color::new(ColorSpace::Srgb, 1., 0., 0., 1.).unwrap()
    }

    #[test]
    fn formatter_golden_per_registry_grammar() {
        // 검증 기준(IX.11.13): 값 문법별 golden 출력.
        use crate::css::grammar::{LENGTH_NON_NEGATIVE, LENGTH_SIGNED, LENGTH_SIZING};
        let p3 = Color::new(ColorSpace::DisplayP3, 0.5, 0.25, 1., 0.75).unwrap();
        let wide = Color::new(ColorSpace::Srgb, 1.25, 0., -0.5, 1.).unwrap();
        let var = CssVar {
            name: "--space-2".into(),
            token: StableId::new_v7(1, 0, 1),
        };
        let cases: Vec<(CssGrammar, CssValue, &str)> = vec![
            (LENGTH_SIZING, CssValue::Length(320.), "320px"),
            (LENGTH_NON_NEGATIVE, CssValue::Length(0.), "0px"),
            (LENGTH_SIGNED, CssValue::Length(-12.5), "-12.5px"),
            (LENGTH_SIGNED, CssValue::Length(-0.0), "0px"),
            (CssGrammar::Percentage, CssValue::Percentage(50.), "50%"),
            (CssGrammar::Angle, CssValue::Angle(540.), "540deg"),
            (CssGrammar::Duration, CssValue::Duration(250.), "250ms"),
            (CssGrammar::Number, CssValue::Number(0.1), "0.1"),
            (CssGrammar::UnitInterval, CssValue::Number(0.25), "0.25"),
            (
                CssGrammar::Count { min: 3, max: 1000 },
                CssValue::Count(8),
                "8",
            ),
            (
                CssGrammar::Color { allow_none: true },
                CssValue::Color(None),
                "none",
            ),
            (
                CssGrammar::Color { allow_none: false },
                CssValue::Color(Some(red())),
                "#ff0000",
            ),
            (
                CssGrammar::Color { allow_none: false },
                CssValue::Color(Some(Color::new(ColorSpace::Srgb, 0., 0., 0., 0.5).unwrap())),
                "color(srgb 0 0 0 / 0.5)",
            ),
            (
                CssGrammar::Color { allow_none: false },
                CssValue::Color(Some(p3)),
                "color(display-p3 0.5 0.25 1 / 0.75)",
            ),
            (
                CssGrammar::Color { allow_none: false },
                CssValue::Color(Some(wide)),
                "color(srgb 1.25 0 -0.5)",
            ),
            (
                CssGrammar::Keyword(&["butt", "round", "square"]),
                CssValue::Keyword(1),
                "round",
            ),
            (CssGrammar::Bool, CssValue::Bool(false), "false"),
            (CssGrammar::Text, CssValue::Text("카드".into()), "카드"),
            (CssGrammar::Transform, CssValue::Transform(IDENTITY), "none"),
            (
                CssGrammar::Transform,
                CssValue::Transform([1., 0., 0., 1., 10., -4.5]),
                "matrix(1, 0, 0, 1, 10, -4.5)",
            ),
            (
                CssGrammar::Border,
                CssValue::Border(Some((2., red()))),
                "2px solid #ff0000",
            ),
            (CssGrammar::Border, CssValue::Border(None), "none"),
            (CssGrammar::Corners, CssValue::Corners([4.; 4]), "4px"),
            (
                CssGrammar::Corners,
                CssValue::Corners([1., 2., 3., 4.]),
                "1px 2px 3px 4px",
            ),
            (
                LENGTH_NON_NEGATIVE,
                CssValue::Var(var.clone()),
                "var(--space-2)",
            ),
            (
                CssGrammar::Color { allow_none: false },
                CssValue::Var(var),
                "var(--space-2)",
            ),
        ];
        for (grammar, value, expected) in cases {
            let first = format(&value, grammar);
            assert_eq!(first, expected, "{grammar:?}");
            // 같은 값은 byte 단위로 같은 출력입니다.
            assert_eq!(format(&value.clone(), grammar).as_bytes(), first.as_bytes());
        }
    }

    #[test]
    fn same_value_from_different_spellings_formats_identically() {
        // 검증 기준(IX.11.13): 같은 값 byte-identical 출력.
        let color = CssGrammar::Color { allow_none: true };
        let spellings = [
            "#F00",
            "#ff0000",
            "rgb(255, 0, 0)",
            "rgb(100% 0% 0%)",
            "red",
            "color(srgb 1 0 0)",
        ];
        let outputs: Vec<String> = spellings.iter().map(|t| ok(color, t)).collect();
        assert!(outputs.iter().all(|o| o == "#ff0000"), "{outputs:?}");
        let angle = CssGrammar::Angle;
        let turns: Vec<String> = ["0.5turn", "180deg", "200grad", "180"]
            .iter()
            .map(|t| ok(angle, t))
            .collect();
        assert!(turns.iter().all(|o| o == "180deg"), "{turns:?}");
    }

    #[test]
    fn f32_round_trip_property() {
        // 검증 기준(IX.11.13): f32 왕복 property test. 임의 유한 f32 bit 패턴이 가장 짧은 10진 표기에서 같은
        // bit로 돌아옵니다(`-0`은 `0`).
        let mut rng = crate::test_support::random_document::SplitMix64::new(0x1113);
        let mut checked = 0;
        for _ in 0..200_000 {
            let value = f32::from_bits(rng.next_u64() as u32);
            if !value.is_finite() {
                continue;
            }
            let text = format_number(value);
            assert!(!text.contains('e') && !text.contains('E'), "{text}");
            let back: f32 = text.parse().unwrap();
            let expected = if value == 0.0 { 0.0 } else { value };
            assert_eq!(back.to_bits(), expected.to_bits(), "{value:e} → {text}");
            if value != 0.0 {
                assert_eq!(format_number(back), text, "고정점");
            }
            checked += 1;
        }
        assert!(checked > 190_000);
        for value in [f32::MAX, f32::MIN, f32::MIN_POSITIVE, 1e-45, 16_777_217.0] {
            assert_eq!(format_number(value).parse::<f32>().unwrap(), value);
        }
    }

    #[test]
    fn new_grammars_parse_property_model_input_forms() {
        // 검증 기준(IX.11.14): 속성 모델 2장의 입력 형태.
        assert_eq!(ok(CssGrammar::Percentage, "50%"), "50%");
        assert_eq!(code(CssGrammar::Percentage, "50"), "unsupported-unit");
        assert_eq!(ok(CssGrammar::Duration, "0.25s"), "250ms");
        assert_eq!(ok(CssGrammar::Duration, "120MS"), "120ms");
        assert_eq!(ok(CssGrammar::Duration, "0"), "0ms");
        assert_eq!(code(CssGrammar::Duration, "3"), "unsupported-unit");
        assert_eq!(ok(CssGrammar::Number, "1e2"), "100");
        assert_eq!(code(CssGrammar::Number, "2px"), "unsupported-unit");
        assert_eq!(ok(CssGrammar::UnitInterval, "50%"), "0.5");
        assert_eq!(ok(CssGrammar::UnitInterval, ".25"), "0.25");
        assert_eq!(code(CssGrammar::UnitInterval, "1.5"), "invalid-value");
        assert_eq!(ok(CssGrammar::Transform, "none"), "none");
        assert_eq!(
            ok(CssGrammar::Transform, "translate(10px, -4.5px)"),
            "matrix(1, 0, 0, 1, 10, -4.5)"
        );
        assert_eq!(
            ok(CssGrammar::Transform, "translate(10px) scale(2)"),
            "matrix(2, 0, 0, 2, 10, 0)"
        );
        assert_eq!(
            ok(CssGrammar::Transform, "scale(2) translate(10px)"),
            "matrix(2, 0, 0, 2, 20, 0)"
        );
        assert_eq!(
            ok(CssGrammar::Transform, "rotate(90deg)"),
            "matrix(0, 1, -1, 0, 0, 0)"
        );
        assert_eq!(
            ok(CssGrammar::Transform, "rotate(0.5turn)"),
            "matrix(-1, 0, 0, -1, 0, 0)"
        );
        assert_eq!(
            ok(CssGrammar::Transform, "matrix(1 0 0 1 3 4)"),
            "matrix(1, 0, 0, 1, 3, 4)"
        );
        assert_eq!(
            code(CssGrammar::Transform, "rotate(30deg)"),
            "unsupported-value"
        );
        assert_eq!(
            code(CssGrammar::Transform, "skew(10deg)"),
            "unsupported-value"
        );
        assert_eq!(
            code(CssGrammar::Transform, "translate3d(1px, 2px, 3px)"),
            "unsupported-value"
        );
        assert_eq!(
            code(CssGrammar::Transform, "translate(50%)"),
            "unsupported-value"
        );
        assert_eq!(code(CssGrammar::Transform, "spin(1)"), "invalid-value");
        assert_eq!(code(CssGrammar::Transform, "matrix(1, 2)"), "invalid-value");
    }

    #[test]
    fn var_resolves_through_the_token_scope_and_keeps_case() {
        use std::collections::BTreeMap;
        let token = StableId::new_v7(9, 0, 9);
        let scope: BTreeMap<String, StableId> = [("--Brand-Primary".to_string(), token)].into();
        let color = CssGrammar::Color { allow_none: false };
        let value = parse_with(color, " var(--Brand-Primary) ", &scope).unwrap();
        assert_eq!(
            value,
            CssValue::Var(CssVar {
                name: "--Brand-Primary".into(),
                token
            })
        );
        assert_eq!(format(&value, color), "var(--Brand-Primary)");
        let code_with = |text: &str| parse_with(color, text, &scope).unwrap_err().code;
        assert_eq!(
            code_with("var(--brand-primary)"),
            "unknown-token",
            "대소문자 구분"
        );
        assert_eq!(code_with("var(--Brand-Primary, red)"), "unsupported-value");
        assert_eq!(code_with("var(Brand)"), "invalid-value");
        assert_eq!(code_with("1px var(--Brand-Primary)"), "unsupported-value");
        assert!(is_custom_property_name("--a\\.b"));
        assert!(!is_custom_property_name("--"));
        assert!(!is_custom_property_name("--a b"));
    }

    #[test]
    fn rejected_inputs_cover_every_structured_code() {
        // 검증 기준(IX.11.14): 거부 입력 fixture(코드 4종 + 형식 오류).
        let length = crate::css::grammar::LENGTH_SIGNED;
        assert_eq!(code(length, "1rem"), "unsupported-unit");
        assert_eq!(code(length, "10vw"), "unsupported-unit");
        assert_eq!(code(length, "calc(50% + 4px)"), "unsupported-value");
        assert_eq!(code(length, "clamp(1px, 2px, 3px)"), "unsupported-value");
        assert_eq!(code(length, "unset"), "unsupported-value");
        assert_eq!(code(length, "var(--missing)"), "unknown-token");
        assert_eq!(code(length, "url(x.svg)"), "forbidden-value");
        assert_eq!(
            code(
                CssGrammar::Color { allow_none: false },
                "image-set(\"a.png\" 1x)"
            ),
            "forbidden-value"
        );
        assert_eq!(code(length, "@import x"), "forbidden-value");
        assert_eq!(code(length, "px"), "invalid-value");
    }

    #[test]
    fn parser_fuzz_target_never_panics() {
        // 검증 기준(IX.11.14): parser fuzz target panic 0. 무작위 bytes와 유효 입력 변조를 섞습니다.
        let corpus = [
            "320px",
            "-0",
            "1.5turn",
            "#3366FF80",
            "rgb(51 102 255 / 50%)",
            "color(display-p3 1 0 0)",
            "2px solid red",
            "1px 2px 3px 4px",
            "translate(10px) scale(2) rotate(90deg)",
            "matrix(1,0,0,1,0,0)",
            "50%",
            "0.25s",
            "var(--a)",
            "calc(1px + 2%)",
            "url(x)",
            "none",
            "1 1 0%",
            "italic 16px/1.5 Inter",
            "pre-wrap",
            "1 / span 2",
            "\"a\" 1fr / 1fr",
            "1e38px",
            "1e39",
            "((((((((((x))))))))))",
        ];
        let mut rng = crate::test_support::random_document::SplitMix64::new(0x1114);
        let mut accepted = 0;
        for round in 0..6_000u32 {
            let mut bytes: Vec<u8> = if round % 3 == 0 {
                (0..rng.below(48)).map(|_| rng.next_u64() as u8).collect()
            } else {
                corpus[rng.below(corpus.len() as u64) as usize]
                    .as_bytes()
                    .to_vec()
            };
            for _ in 0..rng.below(4) {
                if bytes.is_empty() {
                    break;
                }
                let at = rng.below(bytes.len() as u64) as usize;
                match rng.below(3) {
                    0 => bytes[at] = rng.next_u64() as u8,
                    1 => bytes.truncate(at),
                    _ => bytes.insert(at, b"()/,%-.e #"[rng.below(10) as usize]),
                }
            }
            accepted += fuzz_one(&bytes);
        }
        assert!(
            accepted > 1_000,
            "유효 입력도 충분히 지나야 합니다: {accepted}"
        );
        assert_eq!(fuzz_one(&vec![b'1'; MAX_INPUT_LEN * 4]), 0, "길이 상한");
    }
}
