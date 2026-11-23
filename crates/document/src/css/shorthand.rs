//! CSS shorthand → longhand 확장 (IX.11.14, 속성 모델 11장, ADR-0024 결정 4).
//!
//! 입력은 항상 CSS 확장 규칙으로 longhand 선언 목록으로 펼칩니다. 생략한 longhand는 CSS처럼 initial value로
//! 되돌립니다(예: `border: 1px`의 style은 `none`, color는 `currentcolor`). 펼친 각 값이 Stellar에서 받을 수
//! 있는지는 longhand 값 parser([`super::value`])가 정합니다. 문법은 재귀가 없고 입력 길이 상한
//! ([`super::value::MAX_INPUT_LEN`])을 지킵니다.

use super::mapping::shorthand_longhands;
use super::value::{CssValueError, MAX_INPUT_LEN};

/// 펼친 longhand 선언 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Longhand {
    /// longhand 이름.
    pub name: &'static str,
    /// 값 텍스트(공백 정규화, keyword는 소문자).
    pub value: String,
}

fn invalid(message: impl Into<String>) -> CssValueError {
    CssValueError::new("invalid-value", message)
}

fn unsupported(message: impl Into<String>) -> CssValueError {
    CssValueError::new("unsupported-value", message)
}

/// 괄호·따옴표 밖의 공백으로 나눕니다. 괄호 깊이는 상한이 있고 재귀하지 않습니다.
fn tokens(text: &str) -> Result<Vec<String>, CssValueError> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0u32;
    let mut quote: Option<char> = None;
    for c in text.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => {
                quote = None;
                current.push(c);
            }
            (Some(_), c) => current.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                current.push(c);
            }
            (None, '(') => {
                depth += 1;
                if depth > 8 {
                    return Err(invalid("괄호가 너무 깊습니다"));
                }
                current.push(c);
            }
            (None, ')') => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| invalid("닫는 괄호가 남습니다"))?;
                current.push(c);
            }
            (None, c) if c.is_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            (None, c) => current.push(c),
        }
    }
    if quote.is_some() || depth != 0 {
        return Err(invalid("따옴표나 괄호가 닫히지 않았습니다"));
    }
    if !current.is_empty() {
        out.push(current);
    }
    Ok(out)
}

fn lower(token: &str) -> String {
    token.to_ascii_lowercase()
}

/// 1–4값 상자 규칙(top, right, bottom, left 또는 TL, TR, BR, BL).
fn four(values: &[String], name: &str) -> Result<[String; 4], CssValueError> {
    Ok(match values {
        [a] => [a.clone(), a.clone(), a.clone(), a.clone()],
        [a, b] => [a.clone(), b.clone(), a.clone(), b.clone()],
        [a, b, c] => [a.clone(), b.clone(), c.clone(), b.clone()],
        [a, b, c, d] => [a.clone(), b.clone(), c.clone(), d.clone()],
        _ => return Err(invalid(format!("{name}에는 값을 1–4개 넣으세요"))),
    })
}

fn is_length_like(token: &str) -> bool {
    let t = token.as_bytes();
    matches!(t.first(), Some(b'0'..=b'9' | b'.' | b'+' | b'-')) && !token.starts_with("--")
        || token.starts_with("calc(")
}

const BORDER_STYLES: [&str; 10] = [
    "none", "hidden", "dotted", "dashed", "solid", "double", "groove", "ridge", "inset", "outset",
];
const BORDER_WIDTH_KEYWORDS: [&str; 3] = ["thin", "medium", "thick"];

/// `border` 계열 한 변 값: width, style, color(순서 무관, 생략은 initial).
fn border_parts(values: &[String]) -> Result<(String, String, String), CssValueError> {
    let (mut width, mut style, mut color) = (None, None, None);
    for token in values {
        let key = lower(token);
        if BORDER_STYLES.contains(&key.as_str()) {
            if style.replace(key).is_some() {
                return Err(invalid("border style이 두 번 있습니다"));
            }
        } else if BORDER_WIDTH_KEYWORDS.contains(&key.as_str()) || is_length_like(&key) {
            if width.replace(key).is_some() {
                return Err(invalid("border 두께가 두 번 있습니다"));
            }
        } else if color.replace(key).is_some() {
            return Err(invalid("border 색이 두 번 있습니다"));
        }
    }
    Ok((
        width.unwrap_or_else(|| "medium".into()),
        style.unwrap_or_else(|| "none".into()),
        color.unwrap_or_else(|| "currentcolor".into()),
    ))
}

fn flex(values: &[String]) -> Result<[String; 3], CssValueError> {
    let lowered: Vec<String> = values.iter().map(|v| lower(v)).collect();
    let is_number = |t: &str| t.parse::<f32>().is_ok_and(f32::is_finite);
    Ok(match lowered.as_slice() {
        [one] if one == "none" => ["0".into(), "0".into(), "auto".into()],
        [one] if one == "auto" => ["1".into(), "1".into(), "auto".into()],
        [grow] if is_number(grow) => [grow.clone(), "1".into(), "0%".into()],
        [basis] => ["1".into(), "1".into(), basis.clone()],
        [grow, shrink] if is_number(grow) && is_number(shrink) => {
            [grow.clone(), shrink.clone(), "0%".into()]
        }
        [grow, basis] if is_number(grow) => [grow.clone(), "1".into(), basis.clone()],
        [grow, shrink, basis] if is_number(grow) && is_number(shrink) => {
            [grow.clone(), shrink.clone(), basis.clone()]
        }
        _ => return Err(invalid("flex 값 형식이 아닙니다")),
    })
}

/// `a / b / …`를 `/`로 나눕니다(괄호 밖).
fn slash_parts(text: &str, max: usize) -> Result<Vec<String>, CssValueError> {
    let parts: Vec<String> = text
        .split('/')
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();
    if parts.len() > max || parts.iter().any(String::is_empty) {
        return Err(invalid(format!("/로 나눈 값을 1–{max}개 넣으세요")));
    }
    Ok(parts)
}

const FONT_STYLES: [&str; 3] = ["normal", "italic", "oblique"];
const FONT_WEIGHTS: [&str; 4] = ["normal", "bold", "bolder", "lighter"];
const FONT_STRETCHES: [&str; 9] = [
    "ultra-condensed",
    "extra-condensed",
    "condensed",
    "semi-condensed",
    "semi-expanded",
    "expanded",
    "extra-expanded",
    "ultra-expanded",
    "normal",
];
const SYSTEM_FONTS: [&str; 6] = [
    "caption",
    "icon",
    "menu",
    "message-box",
    "small-caption",
    "status-bar",
];

/// `font`: `[style] [weight] [stretch] size[/line-height] family`. 생략한 longhand는 initial(`normal`).
fn font(values: &[String]) -> Result<[String; 6], CssValueError> {
    if let [only] = values {
        if SYSTEM_FONTS.contains(&lower(only).as_str()) {
            return Err(unsupported("system font keyword는 받지 않습니다"));
        }
    }
    let (mut style, mut weight, mut stretch) = (None, None, None);
    let mut index = 0;
    while index < values.len() {
        let key = lower(&values[index]);
        let numeric_weight = key
            .parse::<f32>()
            .is_ok_and(|w| (1.0..=1000.0).contains(&w));
        if key == "normal" {
            // `normal`은 아직 정하지 않은 앞 longhand 하나를 채웁니다.
            if style.is_none() {
                style = Some(key);
            } else if weight.is_none() {
                weight = Some(key);
            } else if stretch.is_none() {
                stretch = Some(key);
            } else {
                return Err(invalid("font에 normal이 너무 많습니다"));
            }
        } else if FONT_STYLES.contains(&key.as_str()) && style.is_none() {
            style = Some(key);
        } else if (FONT_WEIGHTS.contains(&key.as_str()) || numeric_weight) && weight.is_none() {
            weight = Some(key);
        } else if FONT_STRETCHES.contains(&key.as_str()) && stretch.is_none() {
            stretch = Some(key);
        } else {
            break;
        }
        index += 1;
    }
    let size_token = values
        .get(index)
        .ok_or_else(|| invalid("font에는 크기와 글꼴 이름이 필요합니다"))?;
    let (size, line_height) = match size_token.split_once('/') {
        Some((size, line)) if !line.is_empty() => (lower(size), lower(line)),
        Some((size, _)) => {
            index += 1;
            let line = values
                .get(index)
                .ok_or_else(|| invalid("/ 뒤에 line-height가 필요합니다"))?;
            (lower(size), lower(line))
        }
        None => match values.get(index + 1) {
            Some(next) if next.starts_with('/') => {
                index += 1;
                let line = next.trim_start_matches('/');
                if line.is_empty() {
                    index += 1;
                    let line = values
                        .get(index)
                        .ok_or_else(|| invalid("/ 뒤에 line-height가 필요합니다"))?;
                    (lower(size_token), lower(line))
                } else {
                    (lower(size_token), lower(line))
                }
            }
            _ => (lower(size_token), "normal".into()),
        },
    };
    let family = values[index + 1..].join(" ");
    if family.is_empty() {
        return Err(invalid("font에는 글꼴 이름이 필요합니다"));
    }
    Ok([
        style.unwrap_or_else(|| "normal".into()),
        weight.unwrap_or_else(|| "normal".into()),
        stretch.unwrap_or_else(|| "normal".into()),
        size,
        line_height,
        family,
    ])
}

fn white_space(values: &[String]) -> Result<[String; 2], CssValueError> {
    let lowered: Vec<String> = values.iter().map(|v| lower(v)).collect();
    let pair = |a: &str, b: &str| Ok([a.to_string(), b.to_string()]);
    match lowered.as_slice() {
        [one] => match one.as_str() {
            "normal" => pair("collapse", "wrap"),
            "pre" => pair("preserve", "nowrap"),
            "pre-wrap" => pair("preserve", "wrap"),
            "pre-line" => pair("preserve-breaks", "wrap"),
            "nowrap" => pair("collapse", "nowrap"),
            other => two_keyword_white_space(&[other.to_string()]),
        },
        many => two_keyword_white_space(many),
    }
}

const COLLAPSE: [&str; 5] = [
    "collapse",
    "discard",
    "preserve",
    "preserve-breaks",
    "preserve-spaces",
];
const WRAP_MODES: [&str; 2] = ["wrap", "nowrap"];

fn two_keyword_white_space(values: &[String]) -> Result<[String; 2], CssValueError> {
    let (mut collapse, mut mode) = (None, None);
    for value in values {
        if COLLAPSE.contains(&value.as_str()) && collapse.is_none() {
            collapse = Some(value.clone());
        } else if WRAP_MODES.contains(&value.as_str()) && mode.is_none() {
            mode = Some(value.clone());
        } else {
            return Err(invalid(format!("{value}은 white-space 값이 아닙니다")));
        }
    }
    Ok([
        collapse.unwrap_or_else(|| "collapse".into()),
        mode.unwrap_or_else(|| "wrap".into()),
    ])
}

const WRAP_STYLES: [&str; 4] = ["auto", "balance", "stable", "pretty"];

fn text_wrap(values: &[String]) -> Result<[String; 2], CssValueError> {
    let (mut mode, mut style) = (None, None);
    for value in values.iter().map(|v| lower(v)) {
        if WRAP_MODES.contains(&value.as_str()) && mode.is_none() {
            mode = Some(value);
        } else if WRAP_STYLES.contains(&value.as_str()) && style.is_none() {
            style = Some(value);
        } else {
            return Err(invalid(format!("{value}은 text-wrap 값이 아닙니다")));
        }
    }
    Ok([
        mode.unwrap_or_else(|| "wrap".into()),
        style.unwrap_or_else(|| "auto".into()),
    ])
}

fn grid_line_pair(text: &str) -> Result<[String; 2], CssValueError> {
    let parts = slash_parts(text, 2)?;
    let start = lower(&parts[0]);
    let end = parts.get(1).map_or_else(|| "auto".into(), |p| lower(p));
    Ok([start, end])
}

fn grid_area(text: &str) -> Result<[String; 4], CssValueError> {
    let parts = slash_parts(text, 4)?;
    let get = |i: usize| parts.get(i).map_or_else(|| "auto".into(), |p| lower(p));
    // 순서: row-start / column-start / row-end / column-end.
    Ok([get(0), get(1), get(2), get(3)])
}

/// `name` shorthand의 `value`를 longhand 선언 목록으로 펼칩니다. `name`이 shorthand가 아니면 `None`.
pub fn expand(name: &str, value: &str) -> Option<Result<Vec<Longhand>, CssValueError>> {
    let longhands = shorthand_longhands(name)?;
    Some(expand_known(name, longhands, value))
}

fn expand_known(
    name: &str,
    longhands: &'static [&'static str],
    value: &str,
) -> Result<Vec<Longhand>, CssValueError> {
    if value.len() > MAX_INPUT_LEN {
        return Err(invalid("값이 너무 깁니다"));
    }
    let text = value.trim();
    if text.is_empty() {
        return Err(invalid("값이 비어 있습니다"));
    }
    let global = lower(text);
    if ["inherit", "initial", "unset", "revert", "revert-layer"].contains(&global.as_str()) {
        return Err(unsupported(
            "문서 모델에는 cascade가 없어 전역 keyword를 받지 않습니다",
        ));
    }
    if global.contains("var(") {
        return Err(unsupported(
            "shorthand 안의 var()는 펼칠 수 없습니다. longhand로 넣으세요",
        ));
    }
    let values = tokens(text)?;
    let lowered: Vec<String> = values.iter().map(|v| lower(v)).collect();
    let texts: Vec<String> = match name {
        "padding" | "inset" | "border-width" | "border-style" | "border-color" => {
            four(&lowered, name)?.to_vec()
        }
        "border-radius" => {
            if text.contains('/') {
                return Err(unsupported("타원 모서리(/)는 받지 않습니다"));
            }
            four(&lowered, name)?.to_vec()
        }
        "gap" => match lowered.as_slice() {
            [a] => vec![a.clone(), a.clone()],
            [a, b] => vec![a.clone(), b.clone()],
            _ => return Err(invalid("gap에는 값을 1–2개 넣으세요")),
        },
        "place-items" | "place-content" | "place-self" => match lowered.as_slice() {
            [a] => vec![a.clone(), a.clone()],
            [a, b] => vec![a.clone(), b.clone()],
            _ => return Err(invalid(format!("{name}에는 값을 1–2개 넣으세요"))),
        },
        "overflow" => match lowered.as_slice() {
            [a] => vec![a.clone(), a.clone()],
            [a, b] => vec![a.clone(), b.clone()],
            _ => return Err(invalid("overflow에는 값을 1–2개 넣으세요")),
        },
        "border" => {
            let (width, style, color) = border_parts(&values)?;
            let mut out = Vec::with_capacity(12);
            out.extend(std::iter::repeat_n(width, 4));
            out.extend(std::iter::repeat_n(style, 4));
            out.extend(std::iter::repeat_n(color, 4));
            out
        }
        "flex" => flex(&values)?.to_vec(),
        "flex-flow" => {
            let (mut direction, mut wrap) = (None, None);
            for value in &lowered {
                if ["row", "row-reverse", "column", "column-reverse"].contains(&value.as_str())
                    && direction.is_none()
                {
                    direction = Some(value.clone());
                } else if ["nowrap", "wrap", "wrap-reverse"].contains(&value.as_str())
                    && wrap.is_none()
                {
                    wrap = Some(value.clone());
                } else {
                    return Err(invalid(format!("{value}은 flex-flow 값이 아닙니다")));
                }
            }
            vec![
                direction.unwrap_or_else(|| "row".into()),
                wrap.unwrap_or_else(|| "nowrap".into()),
            ]
        }
        "grid-row" | "grid-column" => grid_line_pair(text)?.to_vec(),
        "grid-area" => grid_area(text)?.to_vec(),
        "grid-template" => {
            if text.contains('"') || text.contains('\'') {
                return Err(unsupported("grid-template-areas는 받지 않습니다"));
            }
            if global == "none" {
                vec!["none".into(), "none".into()]
            } else {
                let parts = slash_parts(text, 2)?;
                if parts.len() != 2 {
                    return Err(invalid("grid-template은 rows / columns 형식입니다"));
                }
                vec![lower(&parts[0]), lower(&parts[1])]
            }
        }
        "background" => match lowered.as_slice() {
            [color] if !color.contains("gradient(") => vec![color.clone()],
            _ => {
                return Err(unsupported(
                    "background는 색 하나만 받습니다(이미지·위치 등 다른 성분 거부)",
                ))
            }
        },
        "font" => font(&values)?.to_vec(),
        "white-space" => white_space(&values)?.to_vec(),
        "text-wrap" => text_wrap(&values)?.to_vec(),
        other => {
            return Err(unsupported(format!(
                "{other} shorthand는 아직 펼치지 못합니다"
            )))
        }
    };
    debug_assert_eq!(texts.len(), longhands.len(), "{name}");
    Ok(longhands
        .iter()
        .zip(texts)
        .map(|(name, value)| Longhand { name, value })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::mapping::CSS_SHORTHANDS;

    fn expanded(name: &str, value: &str) -> Vec<(&'static str, String)> {
        expand(name, value)
            .expect("shorthand")
            .unwrap_or_else(|e| panic!("{name}: {value}: {e}"))
            .into_iter()
            .map(|l| (l.name, l.value))
            .collect()
    }

    fn values(name: &str, value: &str) -> Vec<String> {
        expanded(name, value).into_iter().map(|(_, v)| v).collect()
    }

    fn code(name: &str, value: &str) -> &'static str {
        expand(name, value).unwrap().unwrap_err().code
    }

    #[test]
    fn eleventh_chapter_shorthand_fixtures() {
        // 검증 기준(IX.11.14): 속성 모델 11장 shorthand 확장 fixture.
        assert_eq!(values("padding", "4px"), ["4px"; 4]);
        assert_eq!(values("padding", "1px 2px"), ["1px", "2px", "1px", "2px"]);
        assert_eq!(
            values("padding", "1px 2px 3px"),
            ["1px", "2px", "3px", "2px"]
        );
        assert_eq!(values("inset", "0 1px 2px 3PX"), ["0", "1px", "2px", "3px"]);
        assert_eq!(
            expanded("gap", "8px 16px"),
            vec![("row-gap", "8px".into()), ("column-gap", "16px".into())]
        );
        assert_eq!(values("flex", "1"), ["1", "1", "0%"]);
        assert_eq!(values("flex", "auto"), ["1", "1", "auto"]);
        assert_eq!(values("flex", "none"), ["0", "0", "auto"]);
        assert_eq!(values("flex", "2 3"), ["2", "3", "0%"]);
        assert_eq!(values("flex", "2 120px"), ["2", "1", "120px"]);
        assert_eq!(values("flex", "0 0 auto"), ["0", "0", "auto"]);
        assert_eq!(values("flex", "120px"), ["1", "1", "120px"]);
        assert_eq!(values("flex-flow", "wrap"), ["row", "wrap"]);
        assert_eq!(values("flex-flow", "column wrap"), ["column", "wrap"]);
        assert_eq!(
            expanded("place-items", "center"),
            vec![
                ("align-items", "center".into()),
                ("justify-items", "center".into())
            ]
        );
        assert_eq!(values("place-self", "start end"), ["start", "end"]);
        let border = values("border", "2px solid #FF0000");
        assert_eq!(&border[0..4], ["2px"; 4]);
        assert_eq!(&border[4..8], ["solid"; 4]);
        assert_eq!(&border[8..12], ["#ff0000"; 4]);
        let none = values("border", "none");
        assert_eq!(&none[0..4], ["medium"; 4]);
        assert_eq!(&none[8..12], ["currentcolor"; 4]);
        assert_eq!(values("border", "red 1px")[0], "1px");
        assert_eq!(values("border-width", "1px 2px")[3], "2px");
        assert_eq!(values("border-style", "solid")[2], "solid");
        assert_eq!(values("border-color", "red blue")[1], "blue");
        assert_eq!(
            values("border-radius", "1px 2px 3px"),
            ["1px", "2px", "3px", "2px"]
        );
        assert_eq!(values("grid-row", "1 / span 2"), ["1", "span 2"]);
        assert_eq!(values("grid-column", "2"), ["2", "auto"]);
        assert_eq!(values("grid-area", "1 / 2 / 3"), ["1", "2", "3", "auto"]);
        assert_eq!(
            values("grid-template", "100px 1fr / minmax(0, 1fr) 20px"),
            ["100px 1fr", "minmax(0, 1fr) 20px"]
        );
        assert_eq!(values("overflow", "clip"), ["clip", "clip"]);
        assert_eq!(values("overflow", "hidden clip"), ["hidden", "clip"]);
        assert_eq!(
            values("background", "rgb(0 0 0 / 50%)"),
            ["rgb(0 0 0 / 50%)"]
        );
        assert_eq!(
            expanded("font", "italic bold 16px/1.5 \"Pretendard\", sans-serif"),
            vec![
                ("font-style", "italic".into()),
                ("font-weight", "bold".into()),
                ("font-stretch", "normal".into()),
                ("font-size", "16px".into()),
                ("line-height", "1.5".into()),
                ("font-family", "\"Pretendard\", sans-serif".into()),
            ]
        );
        assert_eq!(
            values("font", "12px Inter"),
            ["normal", "normal", "normal", "12px", "normal", "Inter"]
        );
        assert_eq!(
            values("font", "600 condensed 14px / 20px Inter"),
            ["normal", "600", "condensed", "14px", "20px", "Inter"]
        );
        assert_eq!(values("white-space", "pre-wrap"), ["preserve", "wrap"]);
        assert_eq!(values("white-space", "pre"), ["preserve", "nowrap"]);
        assert_eq!(
            values("white-space", "nowrap preserve"),
            ["preserve", "nowrap"]
        );
        assert_eq!(values("text-wrap", "balance"), ["wrap", "balance"]);
        assert_eq!(values("text-wrap", "nowrap"), ["nowrap", "auto"]);
        // 모든 11장 shorthand가 펼쳐집니다.
        for (shorthand, longhands) in CSS_SHORTHANDS {
            assert!(expand(shorthand, "x").is_some(), "{shorthand}");
            assert!(!longhands.is_empty());
        }
        assert!(expand("width", "1px").is_none());
    }

    #[test]
    fn rejected_shorthand_inputs_return_structured_codes() {
        // 검증 기준(IX.11.14): 거부 입력 fixture.
        assert_eq!(code("border-radius", "4px / 2px"), "unsupported-value");
        assert_eq!(code("background", "url(a.png) red"), "unsupported-value");
        assert_eq!(
            code("background", "linear-gradient(red, blue)"),
            "unsupported-value"
        );
        assert_eq!(
            code("grid-template", "\"a b\" 1fr / 1fr 1fr"),
            "unsupported-value"
        );
        assert_eq!(code("font", "menu"), "unsupported-value");
        assert_eq!(code("padding", "inherit"), "unsupported-value");
        assert_eq!(code("padding", "var(--a)"), "unsupported-value");
        assert_eq!(code("padding", "1px 2px 3px 4px 5px"), "invalid-value");
        assert_eq!(code("gap", ""), "invalid-value");
        assert_eq!(code("border", "1px 2px"), "invalid-value");
        assert_eq!(code("font", "bold"), "invalid-value");
        assert_eq!(code("white-space", "loose"), "invalid-value");
        assert_eq!(code("padding", "(1px"), "invalid-value");
        assert_eq!(
            code("padding", &"1px ".repeat(MAX_INPUT_LEN)),
            "invalid-value"
        );
    }
}
