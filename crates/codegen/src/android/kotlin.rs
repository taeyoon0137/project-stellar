//! Kotlin 소스 조각 도우미: 문자열 literal escape, 식별자, 수 literal.

use crate::text::{num, unique_identifiers};

/// Kotlin 예약어와 생성 코드가 쓰는 이름(식별자 충돌 방지). 상태 변수·token 이름이 이 이름과 같으면 생성 코드가
/// 참조하는 Compose 이름을 가려 컴파일되지 않습니다.
pub const RESERVED: &[&str] = &[
    "Alignment",
    "Arrangement",
    "BasicText",
    "BasicTextField",
    "Box",
    "Color",
    "ColorSpaces",
    "Column",
    "Composable",
    "FlowColumn",
    "FlowRow",
    "IntrinsicSize",
    "Matrix",
    "Modifier",
    "Role",
    "Row",
    "Spacer",
    "String",
    "Boolean",
    "Double",
    "Unit",
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
    "null",
    "object",
    "onAction",
    "package",
    "return",
    "state",
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

/// Kotlin 문자열 literal(따옴표 포함). `\`·`"`·`$`·제어 문자를 escape합니다.
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

/// `Float` literal(`1f`, `0.5f`).
pub fn float(value: f32) -> String {
    format!("{}f", num(value))
}

/// `Dp` 식(`16.dp`, `(-8).dp`).
pub fn dp(value: f32) -> String {
    let text = num(value);
    if value < 0.0 {
        format!("({text}).dp")
    } else {
        format!("{text}.dp")
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
    fn literals_escape_kotlin_templates_and_controls() {
        assert_eq!(string_literal("a\"b\\c$d"), "\"a\\\"b\\\\c\\$d\"");
        assert_eq!(string_literal("줄\n\u{1}"), "\"줄\\n\\u0001\"");
        assert_eq!(float(0.5), "0.5f");
        assert_eq!(float(1.0), "1f");
        assert_eq!(dp(16.0), "16.dp");
        assert_eq!(dp(-8.0), "(-8).dp");
        assert_eq!(
            identifiers(["val", "Row", "a b"], "state"),
            vec!["val_2", "Row_2", "a_b"]
        );
    }
}
