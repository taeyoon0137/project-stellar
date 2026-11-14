//! 형식 중립 매니페스트 트리 [`ManifestNode`]와 스키마 오류 (IX.10.1).
//!
//! `stellar.yaml` 텍스트 파싱과 canonical 기록은 project crate의 제한 YAML loader/writer(IV.1.2·IV.1.5)가
//! 담당합니다. 이 crate는 core-types에만 의존하므로(crate graph) Target Profile 절을 이 트리로 주고받습니다.
//! project는 `YamlValue` ↔ [`ManifestNode`]를 1:1로 변환합니다(스칼라는 문자열 그대로, 매핑 키는 스칼라만 —
//! 제한 YAML과 같은 규칙, 의존성 절의 `DeclNode`와 같은 모양).
//!
//! 스키마 오류는 경로와 위반 규칙만 담고 입력 값을 복사하지 않습니다.

use std::fmt;

/// 형식 중립 값(제한 YAML의 매핑/시퀀스/스칼라와 같은 모양).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestNode {
    /// 스칼라(문자열 그대로 보존, 암시적 타입 추론 없음).
    Scalar(String),
    /// 시퀀스(순서 보존).
    Sequence(Vec<ManifestNode>),
    /// 매핑(키 순서 보존, 키는 스칼라 문자열).
    Mapping(Vec<(String, ManifestNode)>),
}

impl ManifestNode {
    /// 스칼라를 만듭니다.
    pub fn scalar(text: impl Into<String>) -> ManifestNode {
        ManifestNode::Scalar(text.into())
    }

    /// 스칼라이면 문자열을 반환합니다.
    pub fn as_scalar(&self) -> Option<&str> {
        match self {
            ManifestNode::Scalar(text) => Some(text),
            _ => None,
        }
    }

    /// 시퀀스이면 슬라이스를 반환합니다.
    pub fn as_sequence(&self) -> Option<&[ManifestNode]> {
        match self {
            ManifestNode::Sequence(items) => Some(items),
            _ => None,
        }
    }

    /// 매핑이면 키-값 쌍을 반환합니다.
    pub fn as_mapping(&self) -> Option<&[(String, ManifestNode)]> {
        match self {
            ManifestNode::Mapping(pairs) => Some(pairs),
            _ => None,
        }
    }

    /// 매핑에서 키로 값을 찾습니다(매핑이 아니거나 키가 없으면 `None`).
    pub fn get(&self, key: &str) -> Option<&ManifestNode> {
        self.as_mapping()?
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    /// 비교·지문용 canonical bytes(파일 형식 아님).
    ///
    /// 형식: 스칼라 `0x01` + 길이 `u32 LE` + UTF-8, 시퀀스 `0x02` + 개수 `u32 LE` + 항목*, 매핑 `0x03` + 개수
    /// `u32 LE` + (키 길이 `u32 LE` + 키 + 값)*. 같은 트리는 항상 같은 bytes입니다. 반복(비재귀)으로 기록해
    /// 깊은 트리에서도 스택을 쓰지 않습니다.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        enum Step<'a> {
            Node(&'a ManifestNode),
            Key(&'a str),
        }
        let mut out = Vec::new();
        let mut stack = vec![Step::Node(self)];
        while let Some(step) = stack.pop() {
            match step {
                Step::Key(key) => push_text(&mut out, key),
                Step::Node(ManifestNode::Scalar(text)) => {
                    out.push(0x01);
                    push_text(&mut out, text);
                }
                Step::Node(ManifestNode::Sequence(items)) => {
                    out.push(0x02);
                    push_len(&mut out, items.len());
                    for item in items.iter().rev() {
                        stack.push(Step::Node(item));
                    }
                }
                Step::Node(ManifestNode::Mapping(pairs)) => {
                    out.push(0x03);
                    push_len(&mut out, pairs.len());
                    for (key, value) in pairs.iter().rev() {
                        stack.push(Step::Node(value));
                        stack.push(Step::Key(key));
                    }
                }
            }
        }
        out
    }
}

fn push_len(out: &mut Vec<u8>, len: usize) {
    // 매니페스트 트리는 loader 크기 상한(수 MB) 안이므로 u32를 넘지 않습니다. 넘으면 포화시켜 기록합니다.
    let len = u32::try_from(len).unwrap_or(u32::MAX);
    out.extend_from_slice(&len.to_le_bytes());
}

fn push_text(out: &mut Vec<u8>, text: &str) {
    push_len(out, text.len());
    out.extend_from_slice(text.as_bytes());
}

/// 스키마 위반. 오류 메시지에 입력 값을 복사하지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaError {
    /// 필수 필드 누락.
    MissingField {
        /// 누락된 필드 경로(예: `target_profiles[0].id`).
        path: String,
    },
    /// 타입 불일치.
    WrongType {
        /// 필드 경로.
        path: String,
        /// 기대한 종류(`mapping`/`sequence`/`scalar`).
        expected: &'static str,
    },
    /// 알 수 없는 키(제한 스키마). profile 종류에 속하지 않는 필드도 여기에 해당합니다.
    UnknownKey {
        /// 키가 나타난 경로.
        path: String,
    },
    /// 값 규칙 위반.
    InvalidValue {
        /// 필드 경로.
        path: String,
        /// 위반한 규칙.
        error: ValueError,
    },
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SchemaError::MissingField { path } => write!(f, "필수 필드 누락: {path}"),
            SchemaError::WrongType { path, expected } => {
                write!(f, "타입 불일치: {path}는 {expected}여야 합니다")
            }
            SchemaError::UnknownKey { path } => write!(f, "알 수 없는 키: {path}"),
            SchemaError::InvalidValue { path, error } => write!(f, "잘못된 값({path}): {error}"),
        }
    }
}

impl std::error::Error for SchemaError {}

/// 개별 값 규칙 위반. 값 자체는 담지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueError {
    /// 빈 값.
    Empty,
    /// 길이 상한 초과.
    TooLong {
        /// 허용 최대 byte 수.
        max: usize,
    },
    /// 허용하지 않는 문자.
    InvalidCharacter,
    /// 음이 아닌 10진 정수가 아님(부호·선행 0·범위 초과 포함).
    InvalidInteger,
    /// 알 수 없는 profile 종류 이름.
    UnknownKind,
    /// 지원하지 않는 profile schema version.
    UnsupportedSchemaVersion,
}

impl fmt::Display for ValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValueError::Empty => f.write_str("빈 값입니다"),
            ValueError::TooLong { max } => write!(f, "최대 {max} byte를 넘습니다"),
            ValueError::InvalidCharacter => f.write_str("허용하지 않는 문자가 있습니다"),
            ValueError::InvalidInteger => f.write_str("음이 아닌 10진 정수가 아닙니다"),
            ValueError::UnknownKind => f.write_str("알 수 없는 profile 종류입니다"),
            ValueError::UnsupportedSchemaVersion => {
                f.write_str("지원하지 않는 profile schema version입니다")
            }
        }
    }
}

impl std::error::Error for ValueError {}

/// 경로에 키를 붙입니다.
pub(crate) fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

/// 매핑을 기대합니다.
pub(crate) fn expect_mapping<'a>(
    node: &'a ManifestNode,
    path: &str,
) -> Result<&'a [(String, ManifestNode)], SchemaError> {
    node.as_mapping().ok_or_else(|| SchemaError::WrongType {
        path: path.to_string(),
        expected: "mapping",
    })
}

/// 스칼라를 기대합니다.
pub(crate) fn expect_scalar<'a>(
    node: &'a ManifestNode,
    path: &str,
) -> Result<&'a str, SchemaError> {
    node.as_scalar().ok_or_else(|| SchemaError::WrongType {
        path: path.to_string(),
        expected: "scalar",
    })
}

/// 매핑의 필수 스칼라 필드를 읽습니다.
pub(crate) fn required_scalar<'a>(
    node: &'a ManifestNode,
    parent: &str,
    key: &str,
) -> Result<&'a str, SchemaError> {
    let path = join(parent, key);
    let value = node
        .get(key)
        .ok_or_else(|| SchemaError::MissingField { path: path.clone() })?;
    expect_scalar(value, &path)
}

/// 정의되지 않은 키를 거부합니다.
pub(crate) fn reject_unknown_keys(
    pairs: &[(String, ManifestNode)],
    parent: &str,
    allowed: &[&str],
) -> Result<(), SchemaError> {
    match pairs
        .iter()
        .find(|(key, _)| !allowed.contains(&key.as_str()))
    {
        Some((key, _)) => Err(SchemaError::UnknownKey {
            path: join(parent, key),
        }),
        None => Ok(()),
    }
}

/// 음이 아닌 10진 정수(`u32`)를 엄격히 읽습니다: ASCII 숫자만, 선행 0 금지(`0` 자체는 허용), 범위 검사.
pub(crate) fn parse_u32(text: &str) -> Result<u32, ValueError> {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return Err(ValueError::Empty);
    }
    if !bytes.iter().all(u8::is_ascii_digit) || (bytes.len() > 1 && bytes[0] == b'0') {
        return Err(ValueError::InvalidInteger);
    }
    if bytes.len() > 10 {
        return Err(ValueError::InvalidInteger);
    }
    text.parse::<u32>().map_err(|_| ValueError::InvalidInteger)
}

/// 값 규칙 위반을 경로가 있는 스키마 오류로 바꿉니다.
pub(crate) fn invalid(path: &str, error: ValueError) -> SchemaError {
    SchemaError::InvalidValue {
        path: path.to_string(),
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_integer_parsing() {
        assert_eq!(parse_u32("0"), Ok(0));
        assert_eq!(parse_u32("35"), Ok(35));
        assert_eq!(parse_u32("4294967295"), Ok(u32::MAX));
        assert_eq!(parse_u32(""), Err(ValueError::Empty));
        for bad in [
            "01",
            "+1",
            "-1",
            "1.0",
            " 1",
            "4294967296",
            "99999999999",
            "1e3",
        ] {
            assert_eq!(parse_u32(bad), Err(ValueError::InvalidInteger), "{bad}");
        }
    }

    #[test]
    fn canonical_bytes_are_order_sensitive_and_stable() {
        let a = ManifestNode::Mapping(vec![
            ("a".to_string(), ManifestNode::scalar("1")),
            (
                "b".to_string(),
                ManifestNode::Sequence(vec![ManifestNode::scalar("x")]),
            ),
        ]);
        let b = ManifestNode::Mapping(vec![
            (
                "b".to_string(),
                ManifestNode::Sequence(vec![ManifestNode::scalar("x")]),
            ),
            ("a".to_string(), ManifestNode::scalar("1")),
        ]);
        assert_eq!(a.canonical_bytes(), a.clone().canonical_bytes());
        assert_ne!(a.canonical_bytes(), b.canonical_bytes());
        assert_eq!(
            ManifestNode::scalar("ab").canonical_bytes(),
            vec![0x01, 2, 0, 0, 0, b'a', b'b']
        );
        // 매우 깊은 시퀀스도 스택 없이 기록합니다.
        let mut deep = ManifestNode::scalar("leaf");
        for _ in 0..100_000 {
            deep = ManifestNode::Sequence(vec![deep]);
        }
        assert_eq!(deep.canonical_bytes().len(), 100_000 * 5 + 9);
        // 깊은 트리 drop도 재귀 없이(아래 반복 해체) 처리합니다.
        let mut current = deep;
        while let ManifestNode::Sequence(mut items) = current {
            current = items.pop().unwrap_or(ManifestNode::scalar(""));
        }
    }

    #[test]
    fn unknown_keys_report_path() {
        let pairs = vec![
            ("id".to_string(), ManifestNode::scalar("a")),
            ("extra".to_string(), ManifestNode::scalar("b")),
        ];
        assert_eq!(
            reject_unknown_keys(&pairs, "target_profiles[0]", &["id"]),
            Err(SchemaError::UnknownKey {
                path: "target_profiles[0].extra".to_string()
            })
        );
        assert!(reject_unknown_keys(&pairs, "x", &["id", "extra"]).is_ok());
    }
}
