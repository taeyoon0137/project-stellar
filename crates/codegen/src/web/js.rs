//! 생성 JavaScript의 값·문자열·식별자 표기 (XXII.2.3, XXII.5.1, ADR-0106 결정 8).
//!
//! - 문자열은 JSON 문자열 escape입니다. 추가로 U+2028·U+2029와 `</`를 escape해 inline script로 넣어도 안전합니다.
//! - 수는 f32 최단 왕복 10진 표기(지수 없음, `-0` → `0`)입니다.
//! - 식별자는 ASCII 문자·숫자·`_`만 남기고 나머지를 `_`로 바꾼 뒤, 숫자로 시작하거나 예약어면 앞에 `_`를 붙입니다.
//!   이미 쓴 식별자와 겹치면 `_<StableId 32자리 hex>`를 붙입니다.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use stellar_core_types::color::ColorSpace;
use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::TypedValue;

use crate::ir::snapshot::id_hex;
use crate::text::num;

/// JSON 문자열 표기(따옴표 포함).
pub fn js_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    let mut previous = '\0';
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            '/' if previous == '<' => out.push_str("\\/"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
        previous = ch;
    }
    out.push('"');
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 값의 JavaScript literal 표기(객체 key는 이름 오름차순).
pub fn js_value(value: &TypedValue) -> String {
    match value {
        TypedValue::Boolean(flag) => flag.to_string(),
        TypedValue::Number(n) => num(n.get()),
        TypedValue::String(text) => js_string(text),
        TypedValue::Enum(e) => format!(
            "{{ \"enumType\": {}, \"type\": \"enum\", \"variant\": {} }}",
            e.enum_type(),
            e.variant()
        ),
        TypedValue::Length(v) => format!("{{ \"type\": \"length\", \"value\": {} }}", num(v.get())),
        TypedValue::Percentage(v) => format!(
            "{{ \"type\": \"percentage\", \"value\": {} }}",
            num(v.get())
        ),
        TypedValue::Angle(v) => format!("{{ \"type\": \"angle\", \"value\": {} }}", num(v.get())),
        TypedValue::Duration(v) => {
            format!("{{ \"type\": \"duration\", \"value\": {} }}", num(v.get()))
        }
        TypedValue::Color(c) => {
            let space = match c.space() {
                ColorSpace::Srgb => "srgb",
                ColorSpace::DisplayP3 => "display-p3",
            };
            format!(
                "{{ \"alpha\": {}, \"b\": {}, \"g\": {}, \"r\": {}, \"space\": \"{space}\", \"type\": \"color\" }}",
                num(c.alpha()),
                num(c.b()),
                num(c.g()),
                num(c.r())
            )
        }
        TypedValue::ResourceRef(r) => format!(
            "{{ \"id\": {}, \"type\": \"resource\" }}",
            js_string(&r.id().to_string())
        ),
        TypedValue::Gradient(_) | TypedValue::Shadow(_) | TypedValue::Typography(_) => {
            // 구조 분해는 하지 않고 정규 저장 byte를 그대로 넘깁니다(손실 없음).
            format!(
                "{{ \"bytes\": \"{}\", \"tag\": {}, \"type\": \"encoded\" }}",
                hex(&value.to_bytes()),
                value.type_tag()
            )
        }
    }
}

/// JavaScript 예약어(ES2023 예약어와 strict mode 예약어).
const RESERVED: &[&str] = &[
    "arguments",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "eval",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

/// 이름의 기본 JavaScript 식별자.
pub fn base_identifier(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty()
        || out.starts_with(|c: char| c.is_ascii_digit())
        || RESERVED.contains(&out.as_str())
    {
        out.insert(0, '_');
    }
    out
}

/// 겹치지 않는 식별자 집합. 같은 입력 순서는 같은 결과입니다.
#[derive(Debug, Clone, Default)]
pub struct Identifiers {
    used: BTreeSet<String>,
}

impl Identifiers {
    /// 빈 집합.
    pub fn new() -> Identifiers {
        Identifiers::default()
    }

    /// `name`의 식별자를 정합니다. 겹치면 `_<id hex>`를 붙입니다.
    pub fn assign(&mut self, name: &str, id: StableId) -> String {
        let base = base_identifier(name);
        let chosen = if self.used.contains(&base) {
            format!("{base}_{}", id_hex(id))
        } else {
            base
        };
        self.used.insert(chosen.clone());
        chosen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_are_json_escaped_and_safe_in_inline_script() {
        assert_eq!(
            js_string("a\"b\\c\n</script>\u{2028}\u{1}"),
            "\"a\\\"b\\\\c\\n<\\/script>\\u2028\\u0001\""
        );
    }

    #[test]
    fn identifiers_are_ascii_unique_and_avoid_reserved_words() {
        let id =
            |n: u8| StableId::from_bytes([0, 0, 0, 0, 0, 0, 0x70, 0, 0x80, 0, 0, 0, 0, 0, 0, n]);
        let mut ids = Identifiers::new();
        assert_eq!(ids.assign("is saving", id(1)), "is_saving");
        assert_eq!(ids.assign("class", id(2)), "_class");
        assert_eq!(ids.assign("1st", id(3)), "_1st");
        assert_eq!(ids.assign("메모", id(4)), "__");
        assert_eq!(
            ids.assign("is-saving", id(5)),
            "is_saving_00000000000070008000000000000005"
        );
    }
}
