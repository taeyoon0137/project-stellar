//! Swift 소스 조각 도우미: 문자열 literal escape, 식별자, 수 literal.

use crate::text::{num, unique_identifiers};

/// Swift 예약어와 생성 코드가 쓰는 멤버 이름(식별자 충돌 방지).
pub const RESERVED: &[&str] = &[
    "Any",
    "Self",
    "Type",
    "as",
    "associatedtype",
    "await",
    "body",
    "break",
    "case",
    "catch",
    "class",
    "content",
    "continue",
    "default",
    "defer",
    "deinit",
    "do",
    "else",
    "enum",
    "extension",
    "fallthrough",
    "false",
    "fileprivate",
    "for",
    "func",
    "guard",
    "if",
    "import",
    "in",
    "init",
    "inout",
    "internal",
    "is",
    "let",
    "nil",
    "onAction",
    "onStateChange",
    "open",
    "operator",
    "private",
    "precedencegroup",
    "protocol",
    "public",
    "repeat",
    "rethrows",
    "return",
    "self",
    "static",
    "struct",
    "subscript",
    "super",
    "switch",
    "throw",
    "throws",
    "true",
    "try",
    "typealias",
    "var",
    "view",
    "views",
    "where",
    "while",
];

/// Swift 문자열 literal(따옴표 포함). `\`·`"`·제어 문자를 escape합니다.
pub fn string_literal(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\u{{{:x}}}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Swift 수 literal(`Double`/`CGFloat` 문맥).
pub fn number(value: f32) -> String {
    num(value)
}

/// 이름 목록의 Swift 식별자(lowerCamel 변환 없이 ASCII 정리, 예약어·중복은 접미사).
pub fn identifiers<'a>(names: impl IntoIterator<Item = &'a str>, fallback: &str) -> Vec<String> {
    unique_identifiers(names, fallback, RESERVED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_escape_quotes_backslashes_and_controls() {
        assert_eq!(string_literal("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(string_literal("줄\n바꿈\u{1}"), "\"줄\\n바꿈\\u{1}\"");
        assert_eq!(number(0.5), "0.5");
        assert_eq!(
            identifiers(["default", "a b"], "state"),
            vec!["default_2", "a_b"]
        );
    }
}
