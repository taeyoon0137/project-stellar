//! SVG 속성 값 표기 (XX.3.1).
//!
//! 같은 값은 항상 같은 문자열이 되도록 표기를 고정합니다.
//!
//! - 숫자: `f32` 최단 round-trip 십진 표기(지수 표기 없음). `-0`은 `0`으로 씁니다. 표기 함수는 CSS 값 formatter와
//!   같은 `stellar_core_types::number`입니다(ADR-0128 결정 4). 계산 결과가 유한하지 않으면
//!   [`SvgValueError::NonFinite`]입니다.
//! - 색: sRGB `[0, 1]`은 `rgb(R%,G%,B%)`(SVG 1.1 호환), 그 외는 CSS Color 4 `color(<space> r g b)`.
//!   알파는 별도 `*-opacity` 속성으로 씁니다(비연관 알파라 그대로 대응합니다).
//! - 변환: 항등은 생략, 그 외 `matrix(a b c d e f)`.
//! - path: `M x y`, `L x y`, `C x1 y1 x2 y2 x y`, `Z`를 공백 하나로 잇습니다.
//! - id: ASCII 영문자로 시작하고 영숫자·`-`·`_`만 쓰는 128 byte 이하 문자열만 속성에 씁니다
//!   ([`is_safe_id`]). 따라서 속성 값에 escape가 필요한 문자가 들어가지 않습니다.

use std::fmt;

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::value::{Rect, Transform2D};
use stellar_scene::geometry::{PathCommand, PathGeometry};
use stellar_scene::node::CornerRadii;

use crate::svg::mapping::is_srgb_in_gamut;

/// SVG 값 표기 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvgValueError {
    /// 계산 결과가 유한하지 않습니다.
    NonFinite,
    /// 안전하지 않은 id입니다.
    UnsafeId,
    /// 이 요소로 표현할 수 없는 값입니다(예: Outside stroke).
    Unrepresentable,
}

impl fmt::Display for SvgValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SvgValueError::NonFinite => "SVG 값 계산 결과가 유한하지 않습니다",
            SvgValueError::UnsafeId => "SVG id가 안전한 문자 규칙을 따르지 않습니다",
            SvgValueError::Unrepresentable => "이 SVG 요소로 표현할 수 없는 값입니다",
        })
    }
}

impl std::error::Error for SvgValueError {}

/// SVG id 최대 길이(byte).
pub const MAX_ID_BYTES: usize = 128;

/// 속성에 그대로 쓸 수 있는 id이면 참입니다(`[A-Za-z][A-Za-z0-9_-]*`, 1..=128 byte).
///
/// XML `ID`(NCName)와 CSS 식별자 규칙을 동시에 만족하는 부분 집합입니다.
pub fn is_safe_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    match bytes.first() {
        Some(first) if first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    bytes.len() <= MAX_ID_BYTES
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

/// 숫자를 씁니다. 표기는 CSS 값 formatter와 같은 함수(`stellar_core_types::number::write_number`, ADR-0128 결정 4)이고,
/// SVG profile은 유한하지 않은 값을 오류로 거부합니다.
pub fn write_number(out: &mut String, value: f32) -> Result<(), SvgValueError> {
    if !value.is_finite() {
        return Err(SvgValueError::NonFinite);
    }
    stellar_core_types::number::write_number(out, value);
    Ok(())
}

/// 숫자들을 공백으로 이어 씁니다.
pub fn write_numbers(out: &mut String, values: &[f32]) -> Result<(), SvgValueError> {
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        write_number(out, *value)?;
    }
    Ok(())
}

/// 색 표기.
#[derive(Debug, Clone, PartialEq)]
pub struct SvgColor {
    /// `fill`·`stroke` 값.
    pub paint: String,
    /// 알파 < 1이면 `*-opacity` 값.
    pub opacity: Option<f32>,
}

/// 색 표기를 만듭니다.
pub fn color_value(color: Color) -> Result<SvgColor, SvgValueError> {
    let mut paint = String::new();
    if is_srgb_in_gamut(color) {
        paint.push_str("rgb(");
        for (index, channel) in [color.r(), color.g(), color.b()].into_iter().enumerate() {
            if index > 0 {
                paint.push(',');
            }
            write_number(&mut paint, channel * 100.0)?;
            paint.push('%');
        }
        paint.push(')');
    } else {
        paint.push_str(match color.space() {
            ColorSpace::Srgb => "color(srgb ",
            ColorSpace::DisplayP3 => "color(display-p3 ",
        });
        write_numbers(&mut paint, &[color.r(), color.g(), color.b()])?;
        paint.push(')');
    }
    let opacity = (color.alpha() < 1.0).then_some(color.alpha());
    Ok(SvgColor { paint, opacity })
}

/// `transform` 값. 항등이면 `None`.
pub fn transform_value(transform: &Transform2D) -> Result<Option<String>, SvgValueError> {
    if *transform == Transform2D::identity() {
        return Ok(None);
    }
    let (a, b, c, d, tx, ty) = transform.components();
    let mut out = String::from("matrix(");
    write_numbers(&mut out, &[a, b, c, d, tx, ty])?;
    out.push(')');
    Ok(Some(out))
}

/// path command를 `d` 값으로 씁니다. 빈 geometry는 빈 문자열입니다.
pub fn path_data(geometry: &PathGeometry) -> Result<String, SvgValueError> {
    let mut out = String::new();
    for (index, command) in geometry.commands().iter().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        match *command {
            PathCommand::MoveTo(p) => {
                out.push('M');
                write_numbers(&mut out, &[p.x(), p.y()])?;
            }
            PathCommand::LineTo(p) => {
                out.push('L');
                write_numbers(&mut out, &[p.x(), p.y()])?;
            }
            PathCommand::CubicTo { c1, c2, to } => {
                out.push('C');
                write_numbers(&mut out, &[c1.x(), c1.y(), c2.x(), c2.y(), to.x(), to.y()])?;
            }
            PathCommand::Close => out.push('Z'),
        }
    }
    Ok(out)
}

/// 둥근 모서리 사각형 `d` 값. 모서리는 원호(`A r r 0 0 1`)이며 radius 0인 모서리는 원호를 생략합니다.
///
/// radius는 scene이 이미 변 길이에 맞춘 실효 값이라 다시 줄이지 않습니다.
pub fn rounded_rect_path_data(rect: Rect, radii: CornerRadii) -> Result<String, SvgValueError> {
    let (x, y) = (rect.origin().x(), rect.origin().y());
    let (w, h) = (rect.size().width(), rect.size().height());
    let (tl, tr, br, bl) = radii.components();
    let (right, bottom) = (x + w, y + h);
    let mut out = String::from("M");
    write_numbers(&mut out, &[x + tl, y])?;
    let line = |out: &mut String, px: f32, py: f32| -> Result<(), SvgValueError> {
        out.push_str(" L");
        write_numbers(out, &[px, py])
    };
    let arc = |out: &mut String, r: f32, px: f32, py: f32| -> Result<(), SvgValueError> {
        if r > 0.0 {
            out.push_str(" A");
            write_numbers(out, &[r, r])?;
            out.push_str(" 0 0 1 ");
            write_numbers(out, &[px, py])?;
        }
        Ok(())
    };
    line(&mut out, right - tr, y)?;
    arc(&mut out, tr, right, y + tr)?;
    line(&mut out, right, bottom - br)?;
    arc(&mut out, br, right - br, bottom)?;
    line(&mut out, x + bl, bottom)?;
    arc(&mut out, bl, x, bottom - bl)?;
    line(&mut out, x, y + tl)?;
    arc(&mut out, tl, x + tl, y)?;
    out.push_str(" Z");
    Ok(out)
}

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// `len` byte의 표준 base64(padding 포함) 길이. 넘치면 `None`.
pub fn base64_len(len: usize) -> Option<usize> {
    len.div_ceil(3).checked_mul(4)
}

/// 표준 base64(RFC 4648, padding 포함)로 씁니다.
pub fn push_base64(out: &mut String, bytes: &[u8]) {
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let triple = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        let symbol = |shift: u32| char::from(BASE64_ALPHABET[((triple >> shift) & 0x3F) as usize]);
        out.push(symbol(18));
        out.push(symbol(12));
        out.push(if chunk.len() > 1 { symbol(6) } else { '=' });
        out.push(if chunk.len() > 2 { symbol(0) } else { '=' });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures as fx;

    fn number(value: f32) -> Result<String, SvgValueError> {
        let mut out = String::new();
        write_number(&mut out, value).map(|()| out)
    }

    #[test]
    fn numbers_are_shortest_round_trip_without_exponent() {
        assert_eq!(number(0.0).unwrap(), "0");
        assert_eq!(number(-0.0).unwrap(), "0");
        assert_eq!(number(1.0).unwrap(), "1");
        assert_eq!(number(-2.5).unwrap(), "-2.5");
        assert_eq!(number(0.1).unwrap(), "0.1");
        assert_eq!(number(1.0e-7).unwrap(), "0.0000001");
        assert_eq!(number(1.0e20).unwrap(), "100000000000000000000");
        for value in [
            0.1f32,
            1.0 / 3.0,
            123.456,
            f32::MAX,
            f32::MIN_POSITIVE,
            1.0e-45,
        ] {
            let text = number(value).unwrap();
            assert!(!text.contains('e') && !text.contains('E'), "{text}");
            assert_eq!(text.parse::<f32>().unwrap(), value);
        }
        assert_eq!(number(f32::INFINITY), Err(SvgValueError::NonFinite));
        assert_eq!(number(f32::NAN), Err(SvgValueError::NonFinite));
    }

    #[test]
    fn colors_use_svg11_rgb_or_css_color4() {
        let c = color_value(fx::srgb(1.0, 0.5, 0.0, 1.0)).unwrap();
        assert_eq!(c.paint, "rgb(100%,50%,0%)");
        assert_eq!(c.opacity, None);
        let translucent = color_value(fx::srgb(0.0, 0.0, 0.0, 0.25)).unwrap();
        assert_eq!(translucent.paint, "rgb(0%,0%,0%)");
        assert_eq!(translucent.opacity, Some(0.25));
        let p3 =
            color_value(Color::new(ColorSpace::DisplayP3, 1.0, 0.5, 0.0, 1.0).unwrap()).unwrap();
        assert_eq!(p3.paint, "color(display-p3 1 0.5 0)");
        let extended = color_value(fx::srgb(1.25, -0.5, 0.0, 1.0)).unwrap();
        assert_eq!(extended.paint, "color(srgb 1.25 -0.5 0)");
    }

    #[test]
    fn transforms_and_paths_are_canonical() {
        assert_eq!(transform_value(&Transform2D::identity()).unwrap(), None);
        assert_eq!(
            transform_value(&Transform2D::new(0.0, 1.0, -1.0, 0.0, 20.0, -0.0).unwrap()).unwrap(),
            Some("matrix(0 1 -1 0 20 0)".to_string())
        );
        let curve = PathGeometry::new(vec![
            PathCommand::MoveTo(fx::p(0.0, 0.0)),
            PathCommand::LineTo(fx::p(10.0, 0.0)),
            PathCommand::CubicTo {
                c1: fx::p(10.0, 5.0),
                c2: fx::p(5.0, 10.0),
                to: fx::p(0.0, 10.0),
            },
            PathCommand::Close,
        ])
        .unwrap();
        assert_eq!(path_data(&curve).unwrap(), "M0 0 L10 0 C10 5 5 10 0 10 Z");
        assert_eq!(path_data(&PathGeometry::empty()).unwrap(), "");
    }

    #[test]
    fn rounded_rect_uses_clockwise_arcs() {
        let d = rounded_rect_path_data(
            fx::rect(0.0, 0.0, 100.0, 80.0),
            CornerRadii::new(8.0, 8.0, 8.0, 8.0).unwrap(),
        )
        .unwrap();
        assert_eq!(
            d,
            "M8 0 L92 0 A8 8 0 0 1 100 8 L100 72 A8 8 0 0 1 92 80 L8 80 A8 8 0 0 1 0 72 \
             L0 8 A8 8 0 0 1 8 0 Z"
        );
        // radius 0 모서리는 원호를 생략합니다.
        let mixed = rounded_rect_path_data(
            fx::rect(10.0, 20.0, 30.0, 40.0),
            CornerRadii::new(5.0, 0.0, 0.0, 0.0).unwrap(),
        )
        .unwrap();
        assert_eq!(
            mixed,
            "M15 20 L40 20 L40 60 L10 60 L10 25 A5 5 0 0 1 15 20 Z"
        );
        // 넘치는 계산은 오류입니다.
        assert_eq!(
            rounded_rect_path_data(fx::rect(f32::MAX, 0.0, f32::MAX, 1.0), CornerRadii::ZERO),
            Err(SvgValueError::NonFinite)
        );
    }

    #[test]
    fn base64_matches_rfc4648_vectors() {
        // RFC 4648 §10 test vectors.
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            let mut out = String::new();
            push_base64(&mut out, input.as_bytes());
            assert_eq!(out, expected);
            assert_eq!(base64_len(input.len()), Some(expected.len()));
        }
        let mut all = String::new();
        push_base64(&mut all, &[0xFF, 0xEE, 0x00]);
        assert_eq!(all, "/+4A");
        assert_eq!(base64_len(usize::MAX), None);
    }

    #[test]
    fn safe_ids_follow_ncname_and_css_subset() {
        for ok in ["n-0195c530", "a", "A_b-9"] {
            assert!(is_safe_id(ok), "{ok}");
        }
        for bad in ["", "1abc", "-a", "a b", "a\"b", "a.b", "a:b", "가"] {
            assert!(!is_safe_id(bad), "{bad}");
        }
        assert!(is_safe_id(&"a".repeat(MAX_ID_BYTES)));
        assert!(!is_safe_id(&"a".repeat(MAX_ID_BYTES + 1)));
    }
}
