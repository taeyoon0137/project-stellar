//! 결정론적 JSON 값과 출력기 (테스트 전용, VI.10.4).
//!
//! - object key는 `BTreeMap`이라 항상 byte 사전순으로 출력됩니다(삽입 순서 무관).
//! - 출력은 **ASCII만** 씁니다: `"`·`\`·제어 문자는 escape, U+007F 이상은 `\uXXXX`
//!   (BMP 밖은 UTF-16 surrogate pair)로 씁니다. 편집기·OS 인코딩과 무관하게 같은 bytes입니다.
//! - 숫자는 정수 또는 f32의 Rust `Display`(최단 왕복 10진 표기, 지수 없음)입니다. 이 표기는
//!   정수·IEEE 연산만으로 정해져 플랫폼과 무관합니다. `-0`은 `0`으로 정규화합니다.
//! - 들여쓰기 2칸, 줄바꿈 `\n`, 마지막 줄바꿈 1개.

use std::collections::BTreeMap;

/// JSON 값.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Json {
    /// null.
    Null,
    /// bool.
    Bool(bool),
    /// 이미 canonical 표기로 만든 숫자.
    Number(String),
    /// 문자열.
    String(String),
    /// 배열.
    Array(Vec<Json>),
    /// object(key 정렬).
    Object(BTreeMap<String, Json>),
}

impl Json {
    /// 정수.
    pub fn int(value: i64) -> Json {
        Json::Number(value.to_string())
    }

    /// 부호 없는 정수.
    pub fn uint(value: u64) -> Json {
        Json::Number(value.to_string())
    }

    /// 유한 f32(`-0`은 `0`).
    pub fn float(value: f32) -> Json {
        let normalized = if value == 0.0 { 0.0f32 } else { value };
        Json::Number(format!("{normalized}"))
    }

    /// 문자열.
    pub fn str(value: &str) -> Json {
        Json::String(value.to_string())
    }

    /// f32 배열.
    pub fn floats(values: &[f32]) -> Json {
        Json::Array(values.iter().map(|v| Json::float(*v)).collect())
    }

    /// `(key, value)` 목록으로 object를 만듭니다. 같은 key가 두 번이면 panic(테스트 코드 버그).
    pub fn object<const N: usize>(entries: [(&str, Json); N]) -> Json {
        let mut map = BTreeMap::new();
        for (key, value) in entries {
            let previous = map.insert(key.to_string(), value);
            assert!(previous.is_none(), "JSON key 중복: {key}");
        }
        Json::Object(map)
    }

    /// pretty 출력(마지막 줄바꿈 포함).
    pub fn to_pretty_string(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out.push('\n');
        out
    }

    fn write(&self, out: &mut String, depth: usize) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(true) => out.push_str("true"),
            Json::Bool(false) => out.push_str("false"),
            Json::Number(text) => out.push_str(text),
            Json::String(text) => write_string(out, text),
            Json::Array(items) => {
                if items.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    newline(out, depth + 1);
                    item.write(out, depth + 1);
                }
                newline(out, depth);
                out.push(']');
            }
            Json::Object(map) => {
                if map.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push('{');
                for (i, (key, value)) in map.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    newline(out, depth + 1);
                    write_string(out, key);
                    out.push_str(": ");
                    value.write(out, depth + 1);
                }
                newline(out, depth);
                out.push('}');
            }
        }
    }
}

fn newline(out: &mut String, depth: usize) {
    out.push('\n');
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn push_u16_escape(out: &mut String, unit: u16) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.push_str("\\u");
    for shift in [12, 8, 4, 0] {
        out.push(HEX[((unit >> shift) & 0xF) as usize] as char);
    }
}

/// JSON 문자열 literal을 ASCII로 씁니다.
fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) >= 0x7F => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    push_u16_escape(out, *unit);
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_sorted_and_output_is_stable() {
        let a = Json::object([
            ("b", Json::int(1)),
            ("a", Json::Null),
            ("c", Json::Bool(true)),
        ]);
        let b = Json::object([
            ("c", Json::Bool(true)),
            ("a", Json::Null),
            ("b", Json::int(1)),
        ]);
        assert_eq!(a.to_pretty_string(), b.to_pretty_string());
        assert_eq!(
            a.to_pretty_string(),
            "{\n  \"a\": null,\n  \"b\": 1,\n  \"c\": true\n}\n"
        );
    }

    #[test]
    fn strings_are_escaped_to_ascii() {
        let text = "따옴표\"\\\n\t\u{01}🎨\u{7F}";
        let out = Json::str(text).to_pretty_string();
        assert_eq!(
            out,
            "\"\\ub530\\uc634\\ud45c\\\"\\\\\\n\\t\\u0001\\ud83c\\udfa8\\u007f\"\n"
        );
        assert!(out.is_ascii());
    }

    #[test]
    fn floats_use_shortest_round_trip_without_negative_zero() {
        assert_eq!(Json::float(-0.0).to_pretty_string(), "0\n");
        assert_eq!(Json::float(1.0).to_pretty_string(), "1\n");
        assert_eq!(Json::float(-3.25).to_pretty_string(), "-3.25\n");
        assert_eq!(Json::float(0.1).to_pretty_string(), "0.1\n");
        assert_eq!(Json::float(1e-7).to_pretty_string(), "0.0000001\n");
        assert_eq!(Json::Array(vec![]).to_pretty_string(), "[]\n", "빈 배열");
        assert_eq!(
            Json::floats(&[0.5, 2.0]).to_pretty_string(),
            "[\n  0.5,\n  2\n]\n"
        );
    }
}
