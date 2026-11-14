//! Kotlin 소스 조각 도우미: 문자열 literal escape, 식별자, 수 literal.

use crate::text::{num, unique_identifiers};

/// Kotlin hard keyword와 생성 코드가 쓰는 이름(식별자 충돌 방지).
pub const RESERVED: &[&str] = &[
    "as",
    "break",
    "class",
    "continue",
    "do",
    "else",
    "false",
    "for",
    "fun",
    "if",
    "in",
    "interface",
    "is",
    "it",
    "modifier",
    "null",
    "object",
    "onAction",
    "package",
    "return",
    "super",
    "this",
    "throw",
    "true",
    "try",
    "typealias",
    "typeof",
    "val",
    "var",
    "when",
    "while",
];

/// Kotlin 문자열 literal(따옴표 포함). `\`·`"`·`$`(문자열 template)·제어 문자를 escape합니다.
pub fn string_literal(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `Dp` literal(`16.dp`, `0.5.dp`, `(-4).dp`).
pub fn dp(value: f32) -> String {
    let text = num(value);
    if value < 0.0 {
        format!("({text}).dp")
    } else {
        format!("{text}.dp")
    }
}

/// `Float` literal(`1f`, `0.5f`, `-0.5f`).
pub fn float(value: f32) -> String {
    format!("{}f", num(value))
}

/// `Double` literal(`12.0`, `0.5`).
pub fn double(value: f32) -> String {
    let text = num(value);
    if text.contains('.') {
        text
    } else {
        format!("{text}.0")
    }
}

/// 이름 목록의 Kotlin 식별자(ASCII 정리, 예약어·중복은 접미사).
pub fn identifiers<'a>(names: impl IntoIterator<Item = &'a str>, fallback: &str) -> Vec<String> {
    unique_identifiers(names, fallback, RESERVED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_escape_templates_and_numbers_are_typed() {
        assert_eq!(
            string_literal("가격 $5 \"x\"\\"),
            "\"가격 \\$5 \\\"x\\\"\\\\\""
        );
        assert_eq!(string_literal("a\u{1}"), "\"a\\u0001\"");
        assert_eq!(dp(16.0), "16.dp");
        assert_eq!(dp(0.5), "0.5.dp");
        assert_eq!(dp(-4.0), "(-4).dp");
        assert_eq!(float(1.0), "1f");
        assert_eq!(float(-0.5), "-0.5f");
        assert_eq!(double(12.0), "12.0");
        assert_eq!(double(0.25), "0.25");
        assert_eq!(
            identifiers(["val", "it", "x"], "state"),
            vec!["val_2", "it_2", "x"]
        );
    }
}
