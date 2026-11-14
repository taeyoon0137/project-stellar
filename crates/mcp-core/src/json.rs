//! 엄격한 JSON reader·writer (XIX.1.1).
//!
//! JSON-RPC 메시지용으로 외부 crate 없이 RFC 8259 JSON을 다룹니다.
//!
//! ## 의도적 사본
//!
//! 이 파일은 `tools/stellar-cli/src/json.rs`와 module 문서(`//!`)만 다르고 본문이 같은 사본입니다.
//! ADR-0005 §8 허용 의존에서 L1(`stellar-cli`)과 L2(`stellar-mcp`)가 함께 의존할 수 있는 crate는
//! `stellar-ops`·`stellar-diagnostics`뿐이고 JSON codec은 둘 중 어느 책임도 아닙니다. 공용 crate를
//! 새로 두려면 두 도구의 허용 edge를 바꾸는 ADR 개정이 필요하므로(외부 JSON crate 도입은
//! XXIX.1.2), 그때까지 사본을 유지합니다. 한쪽만 고치는 일이 없도록 두 crate의 `json_copy_sync`
//! 테스트가 본문 byte 일치를 검사합니다. 수정은 두 파일에 같이 합니다.
//!
//! reader는 다음을 거부합니다.
//!
//! - 입력 크기 제한([`JsonLimits::max_bytes`]) 초과, 중첩 깊이 제한([`JsonLimits::max_depth`]) 초과
//! - 값 개수 제한([`JsonLimits::max_nodes`]) 초과: 작은 값이 많은 입력은 값마다 node·원문 할당이
//!   생겨 메모리가 입력 크기의 수십 배로 늘므로, 크기 제한과 별도로 값 수를 제한합니다
//! - 문법 오류(후행 쉼표, 따옴표 없는 키, 주석, `NaN`·`Infinity`, 선행 0, BOM 등)
//! - 문자열 안의 이스케이프되지 않은 제어 문자, 잘못된 이스케이프, 짝 없는 surrogate
//! - 같은 object 안의 중복 key
//! - 값 뒤의 공백 외 후행 데이터
//!
//! 숫자는 [`JsonNumber`]가 검증된 원문 표기를 그대로 보존합니다(부동소수 변환 없음).
//! writer는 공백 없는 결정론적 표기를 만들며, 모든 개행을 이스케이프하므로 출력에 개행 문자가
//! 없습니다.

use std::collections::HashSet;
use std::fmt;

/// reader 제한.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsonLimits {
    /// 입력 최대 byte 수.
    pub max_bytes: usize,
    /// array·object 최대 중첩 깊이(최상위 컨테이너가 1).
    pub max_depth: usize,
    /// 값(null·bool·숫자·문자열·array·object) 최대 개수. 최상위 값과 컨테이너도 하나씩 세며, object
    /// 항목은 key와 값 한 쌍을 하나로 셉니다.
    pub max_nodes: usize,
}

impl Default for JsonLimits {
    /// 기본 제한: 1 MiB, 깊이 64, 값 65,536개.
    fn default() -> Self {
        JsonLimits {
            max_bytes: 1024 * 1024,
            max_depth: 64,
            max_nodes: 65_536,
        }
    }
}

/// 검증된 JSON 숫자 원문.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonNumber(String);

impl JsonNumber {
    /// 정수에서 만듭니다.
    pub fn from_i64(value: i64) -> JsonNumber {
        JsonNumber(value.to_string())
    }

    /// 부호 없는 정수에서 만듭니다.
    pub fn from_u64(value: u64) -> JsonNumber {
        JsonNumber(value.to_string())
    }

    /// 원문 표기.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 소수점·지수 없는 정수 표기인지 확인합니다(범위 무관).
    pub fn is_integer(&self) -> bool {
        !self.0.contains(['.', 'e', 'E'])
    }

    /// 소수점·지수 없는 정수 표기이고 `i64` 범위이면 값을 반환합니다.
    pub fn as_i64(&self) -> Option<i64> {
        if self.0.contains(['.', 'e', 'E']) {
            return None;
        }
        self.0.parse().ok()
    }
}

/// JSON 값. object는 key 순서를 보존합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonValue {
    /// `null`.
    Null,
    /// `true`·`false`.
    Bool(bool),
    /// 숫자.
    Number(JsonNumber),
    /// 문자열.
    String(String),
    /// 배열.
    Array(Vec<JsonValue>),
    /// object(key 순서 보존, 중복 key 없음).
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    /// 문자열 값을 만듭니다.
    pub fn string(text: impl Into<String>) -> JsonValue {
        JsonValue::String(text.into())
    }

    /// 정수 값을 만듭니다.
    pub fn int(value: i64) -> JsonValue {
        JsonValue::Number(JsonNumber::from_i64(value))
    }

    /// `(key, value)` 목록으로 object를 만듭니다.
    pub fn object(pairs: Vec<(&str, JsonValue)>) -> JsonValue {
        JsonValue::Object(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    /// 있으면 문자열, 없으면 `null`.
    pub fn opt_string(text: Option<&str>) -> JsonValue {
        text.map_or(JsonValue::Null, JsonValue::string)
    }

    /// object의 key 값을 찾습니다.
    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        match self {
            JsonValue::Object(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// 문자열이면 내용을 반환합니다.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            JsonValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// 정수이면 값을 반환합니다.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            JsonValue::Number(n) => n.as_i64(),
            _ => None,
        }
    }

    /// bool이면 값을 반환합니다.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            JsonValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// 배열이면 항목을 반환합니다.
    pub fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            JsonValue::Array(items) => Some(items),
            _ => None,
        }
    }

    /// object이면 `(key, value)` 목록을 반환합니다.
    pub fn as_object(&self) -> Option<&[(String, JsonValue)]> {
        match self {
            JsonValue::Object(pairs) => Some(pairs),
            _ => None,
        }
    }
}

/// reader 오류 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonErrorKind {
    /// 입력이 [`JsonLimits::max_bytes`]를 넘습니다.
    TooLarge,
    /// 중첩이 [`JsonLimits::max_depth`]를 넘습니다.
    TooDeep,
    /// 값 개수가 [`JsonLimits::max_nodes`]를 넘습니다.
    TooManyNodes,
    /// UTF-8이 아닌 입력.
    InvalidUtf8,
    /// 값이 끝나기 전에 입력이 끝났습니다.
    UnexpectedEnd,
    /// 예상하지 못한 문자.
    UnexpectedChar,
    /// 숫자 문법 오류.
    InvalidNumber,
    /// 잘못된 escape.
    InvalidEscape,
    /// 짝 없는 surrogate escape.
    InvalidSurrogate,
    /// 문자열 안의 이스케이프되지 않은 제어 문자.
    ControlCharacter,
    /// 같은 object 안의 중복 key.
    DuplicateKey,
    /// 값 뒤의 후행 데이터.
    TrailingData,
}

impl JsonErrorKind {
    /// 고유 오류 코드 문자열.
    pub fn code(self) -> &'static str {
        match self {
            JsonErrorKind::TooLarge => "too-large",
            JsonErrorKind::TooDeep => "too-deep",
            JsonErrorKind::TooManyNodes => "too-many-nodes",
            JsonErrorKind::InvalidUtf8 => "invalid-utf8",
            JsonErrorKind::UnexpectedEnd => "unexpected-end",
            JsonErrorKind::UnexpectedChar => "unexpected-char",
            JsonErrorKind::InvalidNumber => "invalid-number",
            JsonErrorKind::InvalidEscape => "invalid-escape",
            JsonErrorKind::InvalidSurrogate => "invalid-surrogate",
            JsonErrorKind::ControlCharacter => "control-character",
            JsonErrorKind::DuplicateKey => "duplicate-key",
            JsonErrorKind::TrailingData => "trailing-data",
        }
    }
}

/// 위치를 포함한 reader 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsonError {
    /// 오류 종류.
    pub kind: JsonErrorKind,
    /// 0-기반 byte offset.
    pub offset: usize,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "JSON 오류({}): byte offset {}",
            self.kind.code(),
            self.offset
        )
    }
}

impl std::error::Error for JsonError {}

/// byte 입력을 UTF-8로 확인한 뒤 파싱합니다.
pub fn parse_bytes(input: &[u8], limits: JsonLimits) -> Result<JsonValue, JsonError> {
    if input.len() > limits.max_bytes {
        return Err(JsonError {
            kind: JsonErrorKind::TooLarge,
            offset: limits.max_bytes,
        });
    }
    let text = std::str::from_utf8(input).map_err(|e| JsonError {
        kind: JsonErrorKind::InvalidUtf8,
        offset: e.valid_up_to(),
    })?;
    parse(text, limits)
}

/// 문자열 입력을 파싱합니다. 값 하나와 앞뒤 공백만 허용합니다.
pub fn parse(input: &str, limits: JsonLimits) -> Result<JsonValue, JsonError> {
    if input.len() > limits.max_bytes {
        return Err(JsonError {
            kind: JsonErrorKind::TooLarge,
            offset: limits.max_bytes,
        });
    }
    let mut parser = Parser {
        bytes: input.as_bytes(),
        pos: 0,
        limits,
        nodes: 0,
    };
    parser.skip_ws();
    let value = parser.value(0)?;
    parser.skip_ws();
    if parser.pos != parser.bytes.len() {
        return Err(parser.err(JsonErrorKind::TrailingData));
    }
    Ok(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    limits: JsonLimits,
    /// 지금까지 시작한 값 수.
    nodes: usize,
}

impl Parser<'_> {
    fn err(&self, kind: JsonErrorKind) -> JsonError {
        JsonError {
            kind,
            offset: self.pos,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.peek() {
            self.pos += 1;
        }
    }

    fn expect_literal(&mut self, literal: &[u8]) -> Result<(), JsonError> {
        for &expected in literal {
            match self.peek() {
                None => return Err(self.err(JsonErrorKind::UnexpectedEnd)),
                Some(b) if b == expected => self.pos += 1,
                Some(_) => return Err(self.err(JsonErrorKind::UnexpectedChar)),
            }
        }
        Ok(())
    }

    /// `depth`는 이 값을 감싼 컨테이너 수입니다. 값을 할당하기 전에 값 개수 제한을 검사합니다.
    fn value(&mut self, depth: usize) -> Result<JsonValue, JsonError> {
        self.nodes += 1;
        if self.nodes > self.limits.max_nodes {
            return Err(self.err(JsonErrorKind::TooManyNodes));
        }
        match self.peek() {
            None => Err(self.err(JsonErrorKind::UnexpectedEnd)),
            Some(b'n') => self.expect_literal(b"null").map(|()| JsonValue::Null),
            Some(b't') => self.expect_literal(b"true").map(|()| JsonValue::Bool(true)),
            Some(b'f') => self
                .expect_literal(b"false")
                .map(|()| JsonValue::Bool(false)),
            Some(b'"') => self.string().map(JsonValue::String),
            Some(b'[') => self.array(depth + 1),
            Some(b'{') => self.object(depth + 1),
            Some(b'-' | b'0'..=b'9') => self.number().map(JsonValue::Number),
            Some(_) => Err(self.err(JsonErrorKind::UnexpectedChar)),
        }
    }

    fn enter(&self, depth: usize) -> Result<(), JsonError> {
        if depth > self.limits.max_depth {
            Err(self.err(JsonErrorKind::TooDeep))
        } else {
            Ok(())
        }
    }

    fn array(&mut self, depth: usize) -> Result<JsonValue, JsonError> {
        self.enter(depth)?;
        self.pos += 1; // '['
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(JsonValue::Array(items));
        }
        loop {
            self.skip_ws();
            items.push(self.value(depth)?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(JsonValue::Array(items));
                }
                None => return Err(self.err(JsonErrorKind::UnexpectedEnd)),
                Some(_) => return Err(self.err(JsonErrorKind::UnexpectedChar)),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<JsonValue, JsonError> {
        self.enter(depth)?;
        self.pos += 1; // '{'
        let mut pairs = Vec::new();
        let mut seen = HashSet::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(JsonValue::Object(pairs));
        }
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'"') => {}
                None => return Err(self.err(JsonErrorKind::UnexpectedEnd)),
                Some(_) => return Err(self.err(JsonErrorKind::UnexpectedChar)),
            }
            let key_offset = self.pos;
            let key = self.string()?;
            if !seen.insert(key.clone()) {
                return Err(JsonError {
                    kind: JsonErrorKind::DuplicateKey,
                    offset: key_offset,
                });
            }
            self.skip_ws();
            match self.peek() {
                Some(b':') => self.pos += 1,
                None => return Err(self.err(JsonErrorKind::UnexpectedEnd)),
                Some(_) => return Err(self.err(JsonErrorKind::UnexpectedChar)),
            }
            self.skip_ws();
            let value = self.value(depth)?;
            pairs.push((key, value));
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(JsonValue::Object(pairs));
                }
                None => return Err(self.err(JsonErrorKind::UnexpectedEnd)),
                Some(_) => return Err(self.err(JsonErrorKind::UnexpectedChar)),
            }
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.pos;
        while let Some(b'0'..=b'9') = self.peek() {
            self.pos += 1;
        }
        self.pos - start
    }

    fn number(&mut self) -> Result<JsonNumber, JsonError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.pos += 1;
                if let Some(b'0'..=b'9') = self.peek() {
                    return Err(self.err(JsonErrorKind::InvalidNumber));
                }
            }
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return Err(self.err(JsonErrorKind::InvalidNumber)),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if self.digits() == 0 {
                return Err(self.err(JsonErrorKind::InvalidNumber));
            }
        }
        if let Some(b'e' | b'E') = self.peek() {
            self.pos += 1;
            if let Some(b'+' | b'-') = self.peek() {
                self.pos += 1;
            }
            if self.digits() == 0 {
                return Err(self.err(JsonErrorKind::InvalidNumber));
            }
        }
        // 숫자 구간은 ASCII만 포함하므로 UTF-8 경계가 보장됩니다.
        let text = String::from_utf8_lossy(&self.bytes[start..self.pos]).into_owned();
        Ok(JsonNumber(text))
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let mut value = 0u32;
        for _ in 0..4 {
            let digit = match self.peek() {
                None => return Err(self.err(JsonErrorKind::UnexpectedEnd)),
                Some(b) => (b as char)
                    .to_digit(16)
                    .ok_or_else(|| self.err(JsonErrorKind::InvalidEscape))?,
            };
            value = value * 16 + digit;
            self.pos += 1;
        }
        Ok(value)
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.pos += 1; // '"'
        let mut out = String::new();
        loop {
            let run_start = self.pos;
            while let Some(b) = self.peek() {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.pos += 1;
            }
            // 입력은 &str에서 왔고 구간 경계는 ASCII byte이므로 UTF-8 경계입니다.
            out.push_str(&String::from_utf8_lossy(&self.bytes[run_start..self.pos]));
            match self.peek() {
                None => return Err(self.err(JsonErrorKind::UnexpectedEnd)),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    self.escape(&mut out)?;
                }
                Some(_) => return Err(self.err(JsonErrorKind::ControlCharacter)),
            }
        }
    }

    fn escape(&mut self, out: &mut String) -> Result<(), JsonError> {
        let Some(b) = self.peek() else {
            return Err(self.err(JsonErrorKind::UnexpectedEnd));
        };
        let simple = match b {
            b'"' => Some('"'),
            b'\\' => Some('\\'),
            b'/' => Some('/'),
            b'b' => Some('\u{8}'),
            b'f' => Some('\u{c}'),
            b'n' => Some('\n'),
            b'r' => Some('\r'),
            b't' => Some('\t'),
            b'u' => None,
            _ => return Err(self.err(JsonErrorKind::InvalidEscape)),
        };
        self.pos += 1;
        if let Some(c) = simple {
            out.push(c);
            return Ok(());
        }
        let escape_start = self.pos - 2;
        let first = self.hex4()?;
        let code = match first {
            0xD800..=0xDBFF => {
                // high surrogate 뒤에는 반드시 `\u` low surrogate가 와야 합니다.
                if self.bytes.get(self.pos..self.pos + 2) != Some(b"\\u") {
                    return Err(JsonError {
                        kind: JsonErrorKind::InvalidSurrogate,
                        offset: escape_start,
                    });
                }
                self.pos += 2;
                let second = self.hex4()?;
                if !(0xDC00..=0xDFFF).contains(&second) {
                    return Err(JsonError {
                        kind: JsonErrorKind::InvalidSurrogate,
                        offset: escape_start,
                    });
                }
                0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
            }
            0xDC00..=0xDFFF => {
                return Err(JsonError {
                    kind: JsonErrorKind::InvalidSurrogate,
                    offset: escape_start,
                })
            }
            other => other,
        };
        let c = char::from_u32(code).ok_or(JsonError {
            kind: JsonErrorKind::InvalidSurrogate,
            offset: escape_start,
        })?;
        out.push(c);
        Ok(())
    }
}

/// 값을 공백 없는 결정론적 JSON 문자열로 씁니다. 출력에는 개행 문자가 없습니다.
pub fn write(value: &JsonValue) -> String {
    let mut out = String::new();
    write_into(value, &mut out);
    out
}

fn write_into(value: &JsonValue, out: &mut String) {
    match value {
        JsonValue::Null => out.push_str("null"),
        JsonValue::Bool(true) => out.push_str("true"),
        JsonValue::Bool(false) => out.push_str("false"),
        JsonValue::Number(n) => out.push_str(n.as_str()),
        JsonValue::String(s) => write_string(s, out),
        JsonValue::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_into(item, out);
            }
            out.push(']');
        }
        JsonValue::Object(pairs) => {
            out.push('{');
            for (i, (key, item)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(key, out);
                out.push(':');
                write_into(item, out);
            }
            out.push('}');
        }
    }
}

fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || c == '\u{7f}' || c == '\u{2028}' || c == '\u{2029}' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(text: &str) -> Result<JsonValue, JsonError> {
        parse(text, JsonLimits::default())
    }

    fn kind(text: &str) -> JsonErrorKind {
        match p(text) {
            Ok(v) => panic!("거부되어야 함: {text:?} → {v:?}"),
            Err(e) => e.kind,
        }
    }

    #[test]
    fn parses_all_value_kinds() {
        let v = p(r#" {"a":null,"b":[true,false],"c":-12.5e+3,"d":"x\ty","e":{}} "#).unwrap();
        assert_eq!(v.get("a"), Some(&JsonValue::Null));
        assert_eq!(
            v.get("b").and_then(JsonValue::as_array).map(<[_]>::len),
            Some(2)
        );
        assert_eq!(
            v.get("c"),
            Some(&JsonValue::Number(JsonNumber("-12.5e+3".into())))
        );
        assert_eq!(v.get("d").and_then(JsonValue::as_str), Some("x\ty"));
        assert_eq!(v.get("e"), Some(&JsonValue::Object(Vec::new())));
        assert_eq!(p("0").unwrap().as_i64(), Some(0));
        assert_eq!(p("-0").unwrap().as_i64(), Some(0));
        assert_eq!(
            p("9007199254740993").unwrap().as_i64(),
            Some(9007199254740993)
        );
        assert_eq!(p("1.0").unwrap().as_i64(), None, "소수 표기는 정수 아님");
        assert!(JsonNumber("123456789012345678901234567890".into()).is_integer());
        assert!(!JsonNumber("1e3".into()).is_integer());
    }

    #[test]
    fn unicode_escapes_and_surrogate_pairs() {
        assert_eq!(p(r#""éA""#).unwrap().as_str(), Some("éA"));
        assert_eq!(p(r#""😀""#).unwrap().as_str(), Some("😀"));
        assert_eq!(p("\"한글😀\"").unwrap().as_str(), Some("한글😀"));
        assert_eq!(
            p(r#""\/\b\f\n\r""#).unwrap().as_str(),
            Some("/\u{8}\u{c}\n\r")
        );
    }

    #[test]
    fn malformed_inputs_are_rejected_with_kind() {
        use JsonErrorKind::*;
        let cases: &[(&str, JsonErrorKind)] = &[
            ("", UnexpectedEnd),
            ("   ", UnexpectedEnd),
            ("nul", UnexpectedEnd),
            ("nulx", UnexpectedChar),
            ("[1,]", UnexpectedChar),
            ("[1 2]", UnexpectedChar),
            ("{\"a\":1,}", UnexpectedChar),
            ("{\"a\" 1}", UnexpectedChar),
            ("{a:1}", UnexpectedChar),
            ("{'a':1}", UnexpectedChar),
            ("{\"a\":1", UnexpectedEnd),
            ("[", UnexpectedEnd),
            ("01", InvalidNumber),
            ("-", InvalidNumber),
            ("1.", InvalidNumber),
            (".5", UnexpectedChar),
            ("1e", InvalidNumber),
            ("1e+", InvalidNumber),
            ("+1", UnexpectedChar),
            ("NaN", UnexpectedChar),
            ("Infinity", UnexpectedChar),
            ("\"abc", UnexpectedEnd),
            ("\"a\nb\"", ControlCharacter),
            ("\"a\u{1}b\"", ControlCharacter),
            (r#""\x""#, InvalidEscape),
            (r#""\u12""#, InvalidEscape),
            (r#""\u12zz""#, InvalidEscape),
            (r#""\ud800""#, InvalidSurrogate),
            (r#""\ud800A""#, InvalidSurrogate),
            (r#""\udc00""#, InvalidSurrogate),
            ("{\"a\":1,\"a\":2}", DuplicateKey),
            ("[1] 2", TrailingData),
            ("{} {}", TrailingData),
            ("\u{feff}{}", UnexpectedChar),
            ("// c\n{}", UnexpectedChar),
            ("[1,/*c*/2]", UnexpectedChar),
        ];
        for (text, expected) in cases {
            assert_eq!(kind(text), *expected, "{text:?}");
        }
    }

    #[test]
    fn depth_and_size_limits_are_enforced() {
        let limits = JsonLimits {
            max_bytes: 64,
            max_depth: 3,
            ..JsonLimits::default()
        };
        assert!(parse("[[[1]]]", limits).is_ok());
        assert_eq!(
            parse("[[[[1]]]]", limits).unwrap_err().kind,
            JsonErrorKind::TooDeep
        );
        assert_eq!(
            parse(r#"{"a":{"b":{"c":{}}}}"#, limits).unwrap_err().kind,
            JsonErrorKind::TooDeep
        );
        let big = format!("\"{}\"", "a".repeat(63));
        assert_eq!(
            parse(&big, limits).unwrap_err().kind,
            JsonErrorKind::TooLarge
        );
        // 기본 깊이 제한을 넘는 깊은 중첩도 stack overflow 없이 거부합니다.
        let deep = "[".repeat(100_000);
        assert_eq!(
            parse(&deep, JsonLimits::default()).unwrap_err().kind,
            JsonErrorKind::TooDeep
        );
    }

    #[test]
    fn node_count_limit_is_enforced_before_allocation() {
        let limits = JsonLimits {
            max_nodes: 4,
            ..JsonLimits::default()
        };
        // 최상위 array 1개 + 원소 3개 = 4개.
        assert!(parse("[0,0,0]", limits).is_ok());
        let err = parse("[0,0,0,0]", limits).unwrap_err();
        assert_eq!(err.kind, JsonErrorKind::TooManyNodes);
        assert_eq!(err.offset, 7, "다섯 번째 값의 위치");
        // object 항목은 key·값 한 쌍을 하나로 셉니다. 중첩 컨테이너도 값입니다.
        assert!(parse(r#"{"a":1,"b":[2]}"#, limits).is_ok());
        assert_eq!(
            parse(r#"{"a":1,"b":[2,3]}"#, limits).unwrap_err().kind,
            JsonErrorKind::TooManyNodes
        );
        assert_eq!(JsonErrorKind::TooManyNodes.code(), "too-many-nodes");

        // 기본 제한: 1 MiB 안에 들어가는 `[0,0,…]`(약 52만 값)은 거부합니다.
        let count = JsonLimits::default().max_nodes;
        let ok = format!("[{}]", vec!["0"; count - 1].join(","));
        assert!(parse(&ok, JsonLimits::default()).is_ok());
        let flood = format!("[{}]", vec!["0"; 500_000].join(","));
        assert!(flood.len() <= JsonLimits::default().max_bytes);
        assert_eq!(
            parse(&flood, JsonLimits::default()).unwrap_err().kind,
            JsonErrorKind::TooManyNodes
        );
    }

    #[test]
    fn invalid_utf8_bytes_are_rejected() {
        let err = parse_bytes(b"\"a\xffb\"", JsonLimits::default()).unwrap_err();
        assert_eq!(err.kind, JsonErrorKind::InvalidUtf8);
        assert_eq!(err.offset, 2);
        assert!(parse_bytes(b"[1]", JsonLimits::default()).is_ok());
    }

    #[test]
    fn writer_is_compact_deterministic_and_newline_free() {
        let value = JsonValue::object(vec![
            (
                "s",
                JsonValue::string("줄1\n줄2\t\"q\"\\\u{1}\u{7f}\u{2028}"),
            ),
            ("n", JsonValue::int(-3)),
            (
                "a",
                JsonValue::Array(vec![JsonValue::Null, JsonValue::Bool(true)]),
            ),
        ]);
        let text = write(&value);
        assert_eq!(
            text,
            r#"{"s":"줄1\n줄2\t\"q\"\\\u0001\u007f\u2028","n":-3,"a":[null,true]}"#
        );
        assert!(!text.contains('\n') && !text.contains('\r'));
        // write → parse round-trip.
        assert_eq!(p(&text).unwrap(), value);
    }

    #[test]
    fn number_text_is_preserved() {
        for text in [
            "0",
            "-0",
            "12",
            "1.25",
            "1e10",
            "-2.5E-3",
            "123456789012345678901234567890",
        ] {
            assert_eq!(write(&p(text).unwrap()), text);
        }
    }
}
