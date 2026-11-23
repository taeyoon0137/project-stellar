//! 쓰기 v4의 값 문법: CSS 값 그대로(ADR-0077 결정 2).
//!
//! CSS 값은 자체 formatter·parser를 두지 않고 `stellar_document::css::value`의 공개 함수를 씁니다(ADR-0075 결정 5
//! 정규형, ADR-0077 개정 1). 쓰기는 그 formatter의 정규 텍스트이고, 읽기는 그 parser로 해석한 값을 다시 정규
//! 텍스트로 썼을 때 입력과 byte 단위로 같아야 합니다. 같은 값의 다른 표기(`1440.0px`, `#FF0000`, `rgb()`)는
//! 거부하므로 bytes가 결정론적입니다.
//!
//! | 종류 | 문법 | 예 |
//! |---|---|---|
//! | 길이 | `<number>px` | `1440px`, `-0.5px` |
//! | 각도 | `<number>deg` | `45deg` |
//! | 수 | `<number>`(최단 왕복 10진, 지수 없음, `-0`은 `0`) | `0.5` |
//! | 정수 | 10진 정수 | `6` |
//! | 색 | sRGB 8비트 정확 값은 `#rrggbb[aa]`, 그 밖은 `color(<space> r g b[ / a])` | `#ff0000` |
//! | 행렬 | 단위 행렬은 `none`, 그 밖은 `matrix(a, b, c, d, tx, ty)` | `matrix(1, 0, 0, 1, 12.5, -4)` |
//! | 참조 | 하이픈 UUID 또는 `none` | |

use stellar_core_types::color::Color;
use stellar_core_types::ids::StableId;
use stellar_core_types::value::Transform2D;
use stellar_document::css::value::{self as css_value, CssGrammar, CssValue, CssValueError};

use super::super::text_source::TextSourceError;

pub(crate) fn invalid(message: impl std::fmt::Display) -> TextSourceError {
    TextSourceError::Invalid(message.to_string())
}

pub(crate) fn unsupported(message: impl std::fmt::Display) -> TextSourceError {
    TextSourceError::Unsupported(message.to_string())
}

/// CSS 값 오류를 codec 오류로 바꿉니다. 문법은 알지만 저장하지 못하는 값(`unsupported-*`)은 `Unsupported`입니다.
fn value_error(text: &str, error: CssValueError) -> TextSourceError {
    if error.code.starts_with("unsupported") {
        unsupported(format!("{text}: {error}"))
    } else {
        invalid(format!("{text}: {error}"))
    }
}

/// `text`를 `grammar`로 해석하고, 정규 텍스트와 같은지 확인합니다.
fn canonical(grammar: CssGrammar, text: &str) -> Result<CssValue, TextSourceError> {
    let value = css_value::parse(grammar, text).map_err(|error| value_error(text, error))?;
    let expected = css_value::format(&value, grammar);
    if expected != text {
        return Err(invalid(format!(
            "정규 표기 {expected}이어야 합니다: {text}"
        )));
    }
    Ok(value)
}

const LENGTH: CssGrammar = CssGrammar::Length {
    non_negative: false,
    sizing: false,
};

/// 정확히 왕복하는 최단 10진 표현.
pub(crate) fn number(value: f32) -> String {
    css_value::format_number(value)
}

/// 정규 10진 수를 읽습니다.
pub(crate) fn parse_number(text: &str) -> Result<f32, TextSourceError> {
    match canonical(CssGrammar::Number, text)? {
        CssValue::Number(value) => Ok(value),
        _ => Err(invalid(format!("수가 아닙니다: {text}"))),
    }
}

/// `<number>px`.
pub(crate) fn px(value: f32) -> String {
    css_value::format_length(value)
}

/// `<number>px`를 읽습니다.
pub(crate) fn parse_px(text: &str) -> Result<f32, TextSourceError> {
    match canonical(LENGTH, text)? {
        CssValue::Length(value) => Ok(value),
        _ => Err(invalid(format!("길이가 아닙니다: {text}"))),
    }
}

/// `<number>deg`.
pub(crate) fn deg(value: f32) -> String {
    css_value::format_angle(value)
}

/// `<number>deg`를 읽습니다.
pub(crate) fn parse_deg(text: &str) -> Result<f32, TextSourceError> {
    match canonical(CssGrammar::Angle, text)? {
        CssValue::Angle(value) => Ok(value),
        _ => Err(invalid(format!("각도가 아닙니다: {text}"))),
    }
}

/// `<number>%`.
pub(crate) fn percent(value: f32) -> String {
    css_value::format_percentage(value)
}

/// `<number>%`를 읽습니다.
pub(crate) fn parse_percent(text: &str) -> Result<f32, TextSourceError> {
    match canonical(CssGrammar::Percentage, text)? {
        CssValue::Percentage(value) => Ok(value),
        _ => Err(invalid(format!("백분율이 아닙니다: {text}"))),
    }
}

/// `<number>ms`.
pub(crate) fn ms(value: f32) -> String {
    css_value::format_duration(value)
}

/// `<number>ms`를 읽습니다.
pub(crate) fn parse_ms(text: &str) -> Result<f32, TextSourceError> {
    match canonical(CssGrammar::Duration, text)? {
        CssValue::Duration(value) => Ok(value),
        _ => Err(invalid(format!("시간이 아닙니다: {text}"))),
    }
}

/// 10진 정수(부호 없음).
pub(crate) fn parse_u32(text: &str) -> Result<u32, TextSourceError> {
    if text.is_empty()
        || !text.bytes().all(|b| b.is_ascii_digit())
        || (text.len() > 1 && text.starts_with('0'))
    {
        return Err(invalid(format!("정수 형식이 잘못되었습니다: {text}")));
    }
    text.parse()
        .map_err(|_| invalid(format!("정수 범위를 벗어났습니다: {text}")))
}

/// 정규 색 표기(ADR-0075 정규형: sRGB 8비트 정확 값은 `#rrggbb[aa]`, 그 밖은 `color()`).
pub(crate) fn color(value: Color) -> String {
    css_value::format_color(value)
}

/// [`color`] 문법을 읽습니다.
pub(crate) fn parse_color(text: &str) -> Result<Color, TextSourceError> {
    match canonical(CssGrammar::Color { allow_none: false }, text)? {
        CssValue::Color(Some(value)) => Ok(value),
        _ => Err(invalid(format!("색이 아닙니다: {text}"))),
    }
}

/// CSS `transform` 정규형: 단위 행렬은 `none`, 그 밖은 `matrix(a, b, c, d, tx, ty)`.
pub(crate) fn matrix(value: &Transform2D) -> String {
    let (a, b, c, d, tx, ty) = value.components();
    css_value::format_transform([a, b, c, d, tx, ty])
}

/// [`matrix`] 문법을 읽습니다.
pub(crate) fn parse_matrix(text: &str) -> Result<Transform2D, TextSourceError> {
    match canonical(CssGrammar::Transform, text)? {
        CssValue::Transform([a, b, c, d, tx, ty]) => {
            Transform2D::new(a, b, c, d, tx, ty).map_err(invalid)
        }
        _ => Err(invalid(format!("행렬이 아닙니다: {text}"))),
    }
}

/// 하이픈 UUID 참조.
pub(crate) fn id(value: StableId) -> String {
    value.to_string()
}

/// 하이픈 UUID 참조를 읽습니다. 표기가 canonical(소문자)이어야 합니다.
pub(crate) fn parse_id(text: &str) -> Result<StableId, TextSourceError> {
    let value: StableId = text
        .parse()
        .map_err(|_| invalid(format!("StableId 형식이 잘못되었습니다: {text}")))?;
    if value.to_string() != text {
        return Err(invalid(format!(
            "StableId는 소문자 하이픈 UUID여야 합니다: {text}"
        )));
    }
    Ok(value)
}

/// `none` 또는 참조.
pub(crate) fn optional_id(value: Option<StableId>) -> String {
    value.map_or_else(|| "none".to_owned(), id)
}

/// `none` 또는 참조를 읽습니다.
pub(crate) fn parse_optional_id(text: &str) -> Result<Option<StableId>, TextSourceError> {
    if text == "none" {
        Ok(None)
    } else {
        parse_id(text).map(Some)
    }
}

/// 공백으로 구분한 참조 목록. 빈 목록은 `none`입니다.
pub(crate) fn id_list(values: impl IntoIterator<Item = StableId>) -> String {
    let joined = values.into_iter().map(id).collect::<Vec<_>>().join(" ");
    if joined.is_empty() {
        "none".to_owned()
    } else {
        joined
    }
}

/// [`id_list`] 문법을 읽습니다.
pub(crate) fn parse_id_list(text: &str) -> Result<Vec<StableId>, TextSourceError> {
    if text == "none" {
        return Ok(Vec::new());
    }
    text.split(' ').map(parse_id).collect()
}

/// `true`·`false`.
pub(crate) fn parse_bool(text: &str) -> Result<bool, TextSourceError> {
    match text {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(invalid(format!("불리언은 true 또는 false입니다: {text}"))),
    }
}

/// byte당 두 자리 소문자 hex.
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// [`hex`] 문법을 읽습니다.
pub(crate) fn parse_hex(text: &str) -> Result<Vec<u8>, TextSourceError> {
    if !text.len().is_multiple_of(2)
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid("hex는 소문자 두 자리씩이어야 합니다"));
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(invalid))
        .collect()
}

#[cfg(test)]
mod tests {
    use stellar_core_types::color::ColorSpace;

    use super::*;

    #[test]
    fn numbers_are_shortest_round_trip_and_canonical() {
        for value in [0.0, 0.1, -12.5, f32::MIN_POSITIVE, f32::MAX, 1e-7, 1440.0] {
            let text = number(value);
            assert!(!text.contains('e'), "{text}");
            assert_eq!(parse_number(&text).unwrap().to_bits(), value.to_bits());
        }
        // ADR-0075 정규형: `-0`은 `0`으로 씁니다.
        assert_eq!(number(-0.0), "0");
        for bad in [
            "1.0", "01", "+1", "1e3", "inf", "NaN", ".5", "5.", " 1", "", "-0",
        ] {
            assert!(parse_number(bad).is_err(), "{bad}");
        }
        assert_eq!(parse_px("-0.5px").unwrap(), -0.5);
        for bad in ["3", "3PX", "1.50px", "1em"] {
            assert!(parse_px(bad).is_err(), "{bad}");
        }
        assert_eq!(parse_u32("10").unwrap(), 10);
        assert!(parse_u32("010").is_err());
        assert_eq!(parse_ms(&ms(250.0)).unwrap(), 250.0);
        assert!(parse_ms("0.25s").is_err());
        assert_eq!(parse_deg(&deg(45.0)).unwrap(), 45.0);
        assert_eq!(parse_percent(&percent(50.0)).unwrap(), 50.0);
    }

    #[test]
    fn values_use_the_document_css_formatter() {
        // 같은 값은 stellar_document::css::value와 byte 단위로 같은 텍스트입니다(이중 경로 없음).
        let red = Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 1.0).unwrap();
        assert_eq!(color(red), css_value::format_color(red));
        assert_eq!(color(red), "#ff0000");
        assert_eq!(px(1440.0), css_value::format_length(1440.0));
        assert_eq!(matrix(&Transform2D::identity()), "none");
    }

    #[test]
    fn colors_and_matrices_round_trip_exactly() {
        for value in [
            Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 1.0).unwrap(),
            Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 0.5).unwrap(),
            Color::new(ColorSpace::Srgb, 0.1, 0.2, 0.3, 1.0).unwrap(),
            Color::new(ColorSpace::DisplayP3, -0.125, 1.25, 0.1, 0.5).unwrap(),
            Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 0.0).unwrap(),
        ] {
            let text = color(value);
            assert_eq!(parse_color(&text).unwrap(), value, "{text}");
        }
        assert_eq!(
            color(Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 0.5).unwrap()),
            "color(srgb 1 0 0 / 0.5)"
        );
        assert_eq!(
            color(Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 0.0).unwrap()),
            "#00000000"
        );
        for bad in [
            "color(srgb 1 0 0)",
            "color(srgb 1 0 0 / 1)",
            "color(oklch 1 0 0)",
            "color(srgb 1 0)",
            "#FF0000",
            "#f00",
            "red",
            "rgb(255, 0, 0)",
        ] {
            assert!(parse_color(bad).is_err(), "{bad}");
        }
        let transform = Transform2D::new(-1.0, 0.25, -0.75, 1.0, 10.0, -20.0).unwrap();
        let text = matrix(&transform);
        assert_eq!(text, "matrix(-1, 0.25, -0.75, 1, 10, -20)");
        assert_eq!(parse_matrix(&text).unwrap(), transform);
        assert_eq!(parse_matrix("none").unwrap(), Transform2D::identity());
        for bad in [
            "matrix(1,0,0,1,0,0)",
            "matrix(1, 0, 0, 1, 0, 0)",
            "translate(10px)",
        ] {
            assert!(parse_matrix(bad).is_err(), "{bad}");
        }
    }
}
