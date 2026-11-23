//! 테스트 fixture용 최소 JSON reader(문자열·숫자·bool·null·배열·객체).
//!
//! 외부 serializer를 layout crate 의존성으로 들이지 않으려고 둡니다. 입력은 저장소에 커밋된 생성 파일이라 오류는
//! panic으로 알립니다.

use std::collections::BTreeMap;

/// JSON 값.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(BTreeMap<String, Json>),
}

impl Json {
    pub fn get(&self, key: &str) -> &Json {
        match self {
            Json::Object(map) => map.get(key).unwrap_or(&Json::Null),
            _ => panic!("객체가 아닌 값에서 {key}를 찾았습니다"),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Json::String(s) => s,
            other => panic!("문자열이 아닙니다: {other:?}"),
        }
    }

    pub fn as_f64(&self) -> f64 {
        match self {
            Json::Number(n) => *n,
            other => panic!("숫자가 아닙니다: {other:?}"),
        }
    }

    pub fn as_array(&self) -> &[Json] {
        match self {
            Json::Array(items) => items,
            other => panic!("배열이 아닙니다: {other:?}"),
        }
    }

    pub fn as_object(&self) -> &BTreeMap<String, Json> {
        match self {
            Json::Object(map) => map,
            other => panic!("객체가 아닙니다: {other:?}"),
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Json::Null)
    }
}

/// JSON 문서를 읽습니다.
pub fn parse(text: &str) -> Json {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        pos: 0,
    };
    let value = parser.value();
    parser.skip_ws();
    assert_eq!(parser.pos, parser.bytes.len(), "JSON 뒤에 남은 문자");
    value
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn expect(&mut self, byte: u8) {
        self.skip_ws();
        assert_eq!(
            self.bytes.get(self.pos),
            Some(&byte),
            "JSON 위치 {}",
            self.pos
        );
        self.pos += 1;
    }

    fn value(&mut self) -> Json {
        self.skip_ws();
        match self.bytes[self.pos] {
            b'{' => {
                self.pos += 1;
                let mut map = BTreeMap::new();
                self.skip_ws();
                if self.bytes[self.pos] == b'}' {
                    self.pos += 1;
                    return Json::Object(map);
                }
                loop {
                    self.skip_ws();
                    let key = self.string();
                    self.expect(b':');
                    let value = self.value();
                    map.insert(key, value);
                    self.skip_ws();
                    match self.bytes[self.pos] {
                        b',' => self.pos += 1,
                        b'}' => {
                            self.pos += 1;
                            return Json::Object(map);
                        }
                        other => panic!("JSON 객체 구분자 오류: {}", other as char),
                    }
                }
            }
            b'[' => {
                self.pos += 1;
                let mut items = Vec::new();
                self.skip_ws();
                if self.bytes[self.pos] == b']' {
                    self.pos += 1;
                    return Json::Array(items);
                }
                loop {
                    items.push(self.value());
                    self.skip_ws();
                    match self.bytes[self.pos] {
                        b',' => self.pos += 1,
                        b']' => {
                            self.pos += 1;
                            return Json::Array(items);
                        }
                        other => panic!("JSON 배열 구분자 오류: {}", other as char),
                    }
                }
            }
            b'"' => Json::String(self.string()),
            b't' => {
                self.pos += 4;
                Json::Bool(true)
            }
            b'f' => {
                self.pos += 5;
                Json::Bool(false)
            }
            b'n' => {
                self.pos += 4;
                Json::Null
            }
            _ => {
                let start = self.pos;
                while self.pos < self.bytes.len()
                    && matches!(
                        self.bytes[self.pos],
                        b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'
                    )
                {
                    self.pos += 1;
                }
                let text = std::str::from_utf8(&self.bytes[start..self.pos]).unwrap();
                Json::Number(
                    text.parse()
                        .unwrap_or_else(|_| panic!("JSON 숫자 오류: {text}")),
                )
            }
        }
    }

    fn string(&mut self) -> String {
        assert_eq!(self.bytes[self.pos], b'"');
        self.pos += 1;
        let mut out = Vec::new();
        loop {
            let byte = self.bytes[self.pos];
            self.pos += 1;
            match byte {
                b'"' => break,
                b'\\' => {
                    let escaped = self.bytes[self.pos];
                    self.pos += 1;
                    match escaped {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'u' => {
                            let hex =
                                std::str::from_utf8(&self.bytes[self.pos..self.pos + 4]).unwrap();
                            self.pos += 4;
                            let code = u32::from_str_radix(hex, 16).unwrap();
                            let ch = char::from_u32(code).unwrap_or('\u{fffd}');
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        other => panic!("JSON escape 오류: {}", other as char),
                    }
                }
                other => out.push(other),
            }
        }
        String::from_utf8(out).unwrap()
    }
}
