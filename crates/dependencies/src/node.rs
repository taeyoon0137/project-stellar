//! 형식 중립 선언 트리 [`DeclNode`]와 스키마 오류 (XV.1.1).
//!
//! `stellar.yaml`·`stellar.lock.yaml`의 텍스트 파싱과 canonical 기록은 project crate의 제한
//! YAML loader/writer(IV.1.2·IV.1.5)가 담당합니다. 이 crate는 core-types에만 의존하므로 두 파일의
//! 의존성 부분을 이 트리로 주고받습니다. project는 `YamlValue` ↔ [`DeclNode`]를 1:1로 변환합니다
//! (스칼라는 문자열 그대로, 매핑 키는 스칼라만 — 제한 YAML과 같은 규칙).

use std::fmt;

/// 형식 중립 선언 값(제한 YAML의 매핑/시퀀스/스칼라와 같은 모양).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclNode {
    /// 스칼라(문자열 그대로 보존, 암시적 타입 추론 없음).
    Scalar(String),
    /// 시퀀스(순서 보존).
    Sequence(Vec<DeclNode>),
    /// 매핑(키 순서 보존, 키는 스칼라 문자열).
    Mapping(Vec<(String, DeclNode)>),
}

impl DeclNode {
    /// 스칼라를 만듭니다.
    pub fn scalar(text: impl Into<String>) -> DeclNode {
        DeclNode::Scalar(text.into())
    }

    /// 스칼라이면 문자열을 반환합니다.
    pub fn as_scalar(&self) -> Option<&str> {
        match self {
            DeclNode::Scalar(text) => Some(text),
            _ => None,
        }
    }

    /// 시퀀스이면 슬라이스를 반환합니다.
    pub fn as_sequence(&self) -> Option<&[DeclNode]> {
        match self {
            DeclNode::Sequence(items) => Some(items),
            _ => None,
        }
    }

    /// 매핑이면 키-값 쌍을 반환합니다.
    pub fn as_mapping(&self) -> Option<&[(String, DeclNode)]> {
        match self {
            DeclNode::Mapping(pairs) => Some(pairs),
            _ => None,
        }
    }

    /// 매핑에서 키로 값을 찾습니다(매핑이 아니거나 키가 없으면 `None`).
    pub fn get(&self, key: &str) -> Option<&DeclNode> {
        self.as_mapping()?
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }
}

/// 스키마 위반. 오류 메시지에 입력 값을 복사하지 않습니다(URL·경로 노출 방지, XV.1.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaError {
    /// 필수 필드 누락.
    MissingField {
        /// 누락된 필드 경로(예: `dependencies.design-system.library_id`).
        path: String,
    },
    /// 타입 불일치.
    WrongType {
        /// 필드 경로.
        path: String,
        /// 기대한 종류(`mapping`/`sequence`/`scalar`).
        expected: &'static str,
    },
    /// 알 수 없는 키(제한 스키마).
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
    /// 허용하지 않는 문자(제어 문자·공백·금지 기호).
    InvalidCharacter,
    /// `-`로 시작(Git 명령 option으로 해석될 수 있음).
    LeadingDash,
    /// 명령을 실행하는 Git remote helper transport(`ext::`, `fd::`).
    ForbiddenTransport,
    /// 알 수 없는 source 종류.
    UnknownSource,
    /// UUID 형식이 아닌 library id.
    InvalidLibraryId,
    /// commit hash 길이가 40(SHA-1)·64(SHA-256)가 아님.
    InvalidCommitLength,
    /// 16진수가 아닌 문자.
    InvalidHexDigit,
    /// Git ref 이름 규칙 위반.
    InvalidRefName,
    /// version 선택자는 `tag`·`branch`·`commit` 중 정확히 하나여야 함.
    InvalidSelector,
    /// content hash 형식(`sha256:<64자리 소문자 hex>`) 위반.
    InvalidContentHash,
    /// 음이 아닌 10진 정수가 아님.
    InvalidInteger,
    /// 지원하지 않는 schema version.
    UnsupportedVersion,
    /// 다른 필드와 모순(예: `logical_id`가 dependency path 마지막 이름과 다름).
    Inconsistent,
    /// local override 경로 형식 위반(빈 component, 끝 `/`, `.` component).
    InvalidOverridePath,
    /// local override가 사용 프로젝트 root 자신 또는 그 내부를 가리킴.
    OverrideInsideProject,
}

impl fmt::Display for ValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValueError::Empty => f.write_str("빈 값입니다"),
            ValueError::TooLong { max } => write!(f, "최대 {max} byte를 넘습니다"),
            ValueError::InvalidCharacter => f.write_str("허용하지 않는 문자가 있습니다"),
            ValueError::LeadingDash => f.write_str("'-'로 시작할 수 없습니다"),
            ValueError::ForbiddenTransport => {
                f.write_str("명령을 실행하는 transport(ext::, fd::)는 허용하지 않습니다")
            }
            ValueError::UnknownSource => f.write_str("알 수 없는 source 종류입니다"),
            ValueError::InvalidLibraryId => f.write_str("library id는 UUID여야 합니다"),
            ValueError::InvalidCommitLength => {
                f.write_str("commit hash는 40자(SHA-1) 또는 64자(SHA-256)여야 합니다")
            }
            ValueError::InvalidHexDigit => f.write_str("16진수가 아닌 문자가 있습니다"),
            ValueError::InvalidRefName => f.write_str("Git ref 이름 규칙을 위반합니다"),
            ValueError::InvalidSelector => {
                f.write_str("version은 tag·branch·commit 중 정확히 하나여야 합니다")
            }
            ValueError::InvalidContentHash => {
                f.write_str("content hash는 'sha256:<64자리 소문자 16진수>'여야 합니다")
            }
            ValueError::InvalidInteger => f.write_str("음이 아닌 10진 정수가 아닙니다"),
            ValueError::UnsupportedVersion => f.write_str("지원하지 않는 schema version입니다"),
            ValueError::Inconsistent => f.write_str("다른 필드와 모순됩니다"),
            ValueError::InvalidOverridePath => f.write_str(
                "local override 경로 형식이 잘못되었습니다('/' 구분, 빈·'.' component 금지)",
            ),
            ValueError::OverrideInsideProject => f.write_str(
                "local override는 사용 프로젝트 밖의 다른 프로젝트 root를 가리켜야 합니다",
            ),
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
    node: &'a DeclNode,
    path: &str,
) -> Result<&'a [(String, DeclNode)], SchemaError> {
    node.as_mapping().ok_or_else(|| SchemaError::WrongType {
        path: path.to_string(),
        expected: "mapping",
    })
}

/// 시퀀스를 기대합니다.
pub(crate) fn expect_sequence<'a>(
    node: &'a DeclNode,
    path: &str,
) -> Result<&'a [DeclNode], SchemaError> {
    node.as_sequence().ok_or_else(|| SchemaError::WrongType {
        path: path.to_string(),
        expected: "sequence",
    })
}

/// 엄격한 음이 아닌 10진 `u32`(선행 `+`·공백·선행 0 금지, "0"은 허용).
pub(crate) fn parse_u32_strict(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if text.len() > 1 && text.starts_with('0') {
        return None;
    }
    text.parse().ok()
}

/// 스칼라를 기대합니다.
pub(crate) fn expect_scalar<'a>(node: &'a DeclNode, path: &str) -> Result<&'a str, SchemaError> {
    node.as_scalar().ok_or_else(|| SchemaError::WrongType {
        path: path.to_string(),
        expected: "scalar",
    })
}

/// 매핑에서 필수 키를 찾습니다.
pub(crate) fn require<'a>(
    pairs: &'a [(String, DeclNode)],
    key: &str,
    parent: &str,
) -> Result<&'a DeclNode, SchemaError> {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
        .ok_or_else(|| SchemaError::MissingField {
            path: join(parent, key),
        })
}

/// 매핑에서 선택 키를 찾습니다.
pub(crate) fn optional<'a>(pairs: &'a [(String, DeclNode)], key: &str) -> Option<&'a DeclNode> {
    pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// 허용 목록 밖의 키를 거부합니다.
pub(crate) fn reject_unknown(
    pairs: &[(String, DeclNode)],
    parent: &str,
    allowed: &[&str],
) -> Result<(), SchemaError> {
    for (key, _) in pairs {
        if !allowed.contains(&key.as_str()) {
            return Err(SchemaError::UnknownKey {
                path: join(parent, key),
            });
        }
    }
    Ok(())
}

/// 값 규칙 위반을 경로가 붙은 스키마 오류로 바꿉니다.
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
    fn lookup_helpers() {
        let node = DeclNode::Mapping(vec![
            ("a".to_string(), DeclNode::scalar("1")),
            (
                "b".to_string(),
                DeclNode::Sequence(vec![DeclNode::scalar("x")]),
            ),
        ]);
        assert_eq!(node.get("a").and_then(DeclNode::as_scalar), Some("1"));
        assert_eq!(
            node.get("b")
                .and_then(DeclNode::as_sequence)
                .map(<[_]>::len),
            Some(1)
        );
        assert!(node.get("c").is_none());
        assert!(DeclNode::scalar("s").get("a").is_none());
    }

    #[test]
    fn errors_do_not_echo_values() {
        let err = invalid("dependencies.x.repository", ValueError::LeadingDash);
        let text = err.to_string();
        assert!(text.contains("dependencies.x.repository"));
        assert!(text.contains("'-'"));
    }
}
