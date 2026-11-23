//! 수 하나의 정규 10진 표기 (IX.11.20, ADR-0128 결정 4).
//!
//! CSS 값 formatter(`stellar_document::css::value`)와 SVG 값 표기(`stellar_export::svg::format`)가 같은 수 표기를
//! 씁니다. 두 crate가 서로 의존하지 않으므로(export는 Scene Snapshot만 읽음, ADR-0019) 표기 함수는 둘이 함께 쓰는
//! 이 crate에 하나만 둡니다. 단위·구분자·색·행렬 표기는 각 profile이 이 함수 위에 정합니다.
//!
//! - `f32`를 왕복하는 가장 짧은 10진 표기이고 지수 표기를 쓰지 않습니다(Rust `f32` `Display`).
//! - `-0`은 `0`으로 씁니다.
//! - 유한하지 않은 값은 표기 대상이 아닙니다. 호출자가 먼저 거부합니다(SVG profile은 오류, CSS 값은 유한 값만
//!   만듭니다).

use std::fmt::Write;

/// `value`의 정규 10진 표기를 `out` 끝에 붙입니다(모듈 문서 규칙).
pub fn write_number(out: &mut String, value: f32) {
    if value == 0.0 {
        out.push('0');
        return;
    }
    // `String`에 쓰는 `write!`는 실패하지 않습니다.
    let _ = write!(out, "{value}");
}

/// `value`의 정규 10진 표기(모듈 문서 규칙).
pub fn format_number(value: f32) -> String {
    let mut out = String::new();
    write_number(&mut out, value);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notation_is_shortest_round_trip_without_exponent() {
        for (value, text) in [
            (0.0f32, "0"),
            (-0.0, "0"),
            (1.0, "1"),
            (-2.5, "-2.5"),
            (0.1, "0.1"),
            (1440.0, "1440"),
            (1.0e-7, "0.0000001"),
        ] {
            assert_eq!(format_number(value), text, "{value:?}");
        }
        for value in [1.0f32 / 3.0, f32::MIN_POSITIVE, 123_456.79, f32::MAX] {
            let text = format_number(value);
            assert!(!text.contains(['e', 'E']), "{text}");
            assert_eq!(text.parse::<f32>().unwrap(), value);
        }
        let mut out = String::from("x=");
        write_number(&mut out, 12.25);
        assert_eq!(out, "x=12.25");
    }
}
