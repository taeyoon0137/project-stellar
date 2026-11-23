//! 쓰기 v4의 값 문법: CSS 값 그대로(ADR-0077 결정 2).
//!
//! 모든 값은 한 줄 scalar이고 정확히 왕복합니다. float은 `f32`가 정확히 왕복하는 최단 10진 표현(지수 없음)을
//! 씁니다. 8비트 색이나 반올림한 값으로 바꾸지 않습니다.
//!
//! | 종류 | 문법 | 예 |
//! |---|---|---|
//! | 길이 | `<number>px` | `1440px`, `-0.5px` |
//! | 각도 | `<number>deg` | `45deg` |
//! | 수 | `<number>` | `0.5` |
//! | 정수 | 10진 정수 | `6` |
//! | 색 | `color(<space> r g b)`, 알파가 1이 아니면 `color(<space> r g b / a)` | `color(srgb 1 0 0)` |
//! | 행렬 | `matrix(a, b, c, d, tx, ty)` | `matrix(1, 0, 0, 1, 0, 0)` |
//! | 참조 | 하이픈 UUID 또는 `none` | |

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::value::Transform2D;

use super::super::text_source::TextSourceError;

pub(crate) fn invalid(message: impl std::fmt::Display) -> TextSourceError {
    TextSourceError::Invalid(message.to_string())
}

pub(crate) fn unsupported(message: impl std::fmt::Display) -> TextSourceError {
    TextSourceError::Unsupported(message.to_string())
}

/// 정확히 왕복하는 최단 10진 표현.
pub(crate) fn number(value: f32) -> String {
    value.to_string()
}

/// 지수·`inf`·`NaN`·앞뒤 공백 없이 쓴 유한 10진수만 받습니다.
pub(crate) fn parse_number(text: &str) -> Result<f32, TextSourceError> {
    let body = text.strip_prefix('-').unwrap_or(text);
    let (whole, fraction) = body.split_once('.').unwrap_or((body, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || (body.contains('.') && fraction.is_empty())
    {
        return Err(invalid(format!("숫자 형식이 잘못되었습니다: {text}")));
    }
    let value: f32 = text
        .parse()
        .map_err(|_| invalid(format!("숫자 형식이 잘못되었습니다: {text}")))?;
    if !value.is_finite() {
        return Err(invalid("숫자는 유한해야 합니다"));
    }
    // 같은 값의 다른 표기(`1.0`, `01`)는 canonical이 아니므로 거부합니다(결정론적 bytes).
    if number(value) != text {
        return Err(invalid(format!(
            "숫자는 최단 표기 {}이어야 합니다: {text}",
            number(value)
        )));
    }
    Ok(value)
}

fn with_unit(value: f32, unit: &str) -> String {
    format!("{}{unit}", number(value))
}

fn parse_unit(text: &str, unit: &str) -> Result<f32, TextSourceError> {
    let body = text
        .strip_suffix(unit)
        .ok_or_else(|| invalid(format!("{unit} 단위가 필요합니다: {text}")))?;
    parse_number(body)
}

/// `<number>px`.
pub(crate) fn px(value: f32) -> String {
    with_unit(value, "px")
}

/// `<number>px`를 읽습니다.
pub(crate) fn parse_px(text: &str) -> Result<f32, TextSourceError> {
    parse_unit(text, "px")
}

/// `<number>deg`.
pub(crate) fn deg(value: f32) -> String {
    with_unit(value, "deg")
}

/// `<number>deg`를 읽습니다.
pub(crate) fn parse_deg(text: &str) -> Result<f32, TextSourceError> {
    parse_unit(text, "deg")
}

/// `<number>%`.
pub(crate) fn percent(value: f32) -> String {
    with_unit(value, "%")
}

/// `<number>%`를 읽습니다.
pub(crate) fn parse_percent(text: &str) -> Result<f32, TextSourceError> {
    parse_unit(text, "%")
}

/// `<number>ms`.
pub(crate) fn ms(value: f32) -> String {
    with_unit(value, "ms")
}

/// `<number>ms`를 읽습니다.
pub(crate) fn parse_ms(text: &str) -> Result<f32, TextSourceError> {
    parse_unit(text, "ms")
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

fn space_name(space: ColorSpace) -> &'static str {
    match space {
        ColorSpace::Srgb => "srgb",
        ColorSpace::DisplayP3 => "display-p3",
    }
}

/// CSS Color 4 `color()` 함수. 비연관 알파이며 알파가 정확히 1이면 생략합니다.
pub(crate) fn color(value: Color) -> String {
    let rgb = format!(
        "{} {} {} {}",
        space_name(value.space()),
        number(value.r()),
        number(value.g()),
        number(value.b())
    );
    if value.alpha() == 1.0 {
        format!("color({rgb})")
    } else {
        format!("color({rgb} / {})", number(value.alpha()))
    }
}

/// [`color`] 문법을 읽습니다.
pub(crate) fn parse_color(text: &str) -> Result<Color, TextSourceError> {
    let body = text
        .strip_prefix("color(")
        .and_then(|rest| rest.strip_suffix(')'))
        .ok_or_else(|| invalid(format!("색은 color() 함수여야 합니다: {text}")))?;
    let (channels, alpha) = match body.split_once(" / ") {
        Some((channels, alpha)) => (channels, Some(parse_number(alpha)?)),
        None => (body, None),
    };
    let mut parts = channels.split(' ');
    let space = match parts.next() {
        Some("srgb") => ColorSpace::Srgb,
        Some("display-p3") => ColorSpace::DisplayP3,
        _ => return Err(unsupported(format!("알 수 없는 색 공간: {text}"))),
    };
    let mut rgb = [0.0_f32; 3];
    for slot in &mut rgb {
        *slot = parse_number(
            parts
                .next()
                .ok_or_else(|| invalid("색 채널이 부족합니다"))?,
        )?;
    }
    if parts.next().is_some() {
        return Err(invalid("색 채널이 너무 많습니다"));
    }
    if alpha == Some(1.0) {
        return Err(invalid("알파 1은 생략합니다"));
    }
    Color::new(space, rgb[0], rgb[1], rgb[2], alpha.unwrap_or(1.0)).map_err(invalid)
}

/// CSS `matrix(a, b, c, d, tx, ty)`.
pub(crate) fn matrix(value: &Transform2D) -> String {
    let (a, b, c, d, tx, ty) = value.components();
    format!(
        "matrix({}, {}, {}, {}, {}, {})",
        number(a),
        number(b),
        number(c),
        number(d),
        number(tx),
        number(ty)
    )
}

/// [`matrix`] 문법을 읽습니다.
pub(crate) fn parse_matrix(text: &str) -> Result<Transform2D, TextSourceError> {
    let body = text
        .strip_prefix("matrix(")
        .and_then(|rest| rest.strip_suffix(')'))
        .ok_or_else(|| invalid(format!("행렬은 matrix() 함수여야 합니다: {text}")))?;
    let values = body
        .split(", ")
        .map(parse_number)
        .collect::<Result<Vec<_>, _>>()?;
    let [a, b, c, d, tx, ty] = values.as_slice() else {
        return Err(invalid("matrix()에는 값 여섯 개가 필요합니다"));
    };
    Transform2D::new(*a, *b, *c, *d, *tx, *ty).map_err(invalid)
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
    use super::*;

    #[test]
    fn numbers_are_shortest_round_trip_and_canonical() {
        for value in [
            0.0,
            -0.0,
            0.1,
            -12.5,
            f32::MIN_POSITIVE,
            f32::MAX,
            1e-7,
            1440.0,
        ] {
            let text = number(value);
            assert!(!text.contains('e'), "{text}");
            assert_eq!(parse_number(&text).unwrap().to_bits(), value.to_bits());
        }
        for bad in ["1.0", "01", "+1", "1e3", "inf", "NaN", ".5", "5.", " 1", ""] {
            assert!(parse_number(bad).is_err(), "{bad}");
        }
        assert_eq!(parse_px("-0.5px").unwrap(), -0.5);
        assert!(parse_px("3").is_err());
        assert_eq!(parse_u32("10").unwrap(), 10);
        assert!(parse_u32("010").is_err());
    }

    #[test]
    fn colors_and_matrices_round_trip_exactly() {
        for value in [
            Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 1.0).unwrap(),
            Color::new(ColorSpace::DisplayP3, -0.125, 1.25, 0.1, 0.5).unwrap(),
            Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 0.0).unwrap(),
        ] {
            let text = color(value);
            assert_eq!(parse_color(&text).unwrap(), value, "{text}");
        }
        assert_eq!(
            color(Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 1.0).unwrap()),
            "color(srgb 1 0 0)"
        );
        for bad in [
            "color(srgb 1 0 0 / 1)",
            "color(oklch 1 0 0)",
            "color(srgb 1 0)",
            "#ff0000",
            "rgb(255, 0, 0)",
        ] {
            assert!(parse_color(bad).is_err(), "{bad}");
        }
        let transform = Transform2D::new(-1.0, 0.25, -0.75, 1.0, 10.0, -20.0).unwrap();
        let text = matrix(&transform);
        assert_eq!(text, "matrix(-1, 0.25, -0.75, 1, 10, -20)");
        assert_eq!(parse_matrix(&text).unwrap(), transform);
        assert!(parse_matrix("matrix(1,0,0,1,0,0)").is_err());
    }
}
