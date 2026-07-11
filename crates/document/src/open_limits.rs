//! 열린 문서 수 제한 (VI.1.5).
//!
//! 최대 열린 문서 수는 코드에 하드코딩하지 않고 `config/document-limits.yaml`
//! 에서 읽습니다. 제한 초과 시 **기존 문서를 자동으로 닫지 않고** 명확한
//! 오류를 반환합니다 — 닫을 문서의 선택은 사용자 몫입니다.

use std::fmt;
use std::path::Path;

use stellar_core_types::ids::StableId;

use crate::open_set::{OpenDocumentSet, OpenOutcome};

/// 제한 config 파싱 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LimitsConfigError {
    /// 필수 key가 없습니다.
    MissingKey {
        /// 없는 key.
        key: &'static str,
    },
    /// 값이 양의 정수가 아닙니다.
    InvalidValue {
        /// 해당 key.
        key: &'static str,
        /// 원본 값.
        value: String,
    },
    /// 알 수 없는 key입니다.
    UnknownKey {
        /// 해당 key.
        key: String,
    },
    /// 파일을 읽을 수 없습니다.
    Io {
        /// 원인 메시지.
        message: String,
    },
}

impl fmt::Display for LimitsConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LimitsConfigError::MissingKey { key } => {
                write!(f, "document-limits config에 {key}가 없습니다")
            }
            LimitsConfigError::InvalidValue { key, value } => {
                write!(
                    f,
                    "document-limits config {key} 값이 잘못되었습니다: {value:?}"
                )
            }
            LimitsConfigError::UnknownKey { key } => {
                write!(f, "document-limits config에 알 수 없는 key: {key}")
            }
            LimitsConfigError::Io { message } => write!(f, "config 읽기 실패: {message}"),
        }
    }
}

impl std::error::Error for LimitsConfigError {}

/// 문서 런타임 제한 수치.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenDocumentLimits {
    /// 동시에 열 수 있는 최대 문서 수(1 이상).
    pub max_open_documents: usize,
}

impl OpenDocumentLimits {
    /// `key: value` 한 줄 형식의 config 텍스트에서 제한을 읽습니다.
    pub fn from_config_str(text: &str) -> Result<OpenDocumentLimits, LimitsConfigError> {
        let mut max_open: Option<usize> = None;
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once(':') else {
                return Err(LimitsConfigError::UnknownKey {
                    key: line.to_string(),
                });
            };
            let key = key.trim();
            let value = value.trim();
            match key {
                "max_open_documents" => {
                    let parsed: usize =
                        value.parse().ok().filter(|v| *v >= 1).ok_or_else(|| {
                            LimitsConfigError::InvalidValue {
                                key: "max_open_documents",
                                value: value.to_string(),
                            }
                        })?;
                    max_open = Some(parsed);
                }
                other => {
                    return Err(LimitsConfigError::UnknownKey {
                        key: other.to_string(),
                    })
                }
            }
        }
        Ok(OpenDocumentLimits {
            max_open_documents: max_open.ok_or(LimitsConfigError::MissingKey {
                key: "max_open_documents",
            })?,
        })
    }

    /// config 파일에서 제한을 읽습니다.
    pub fn from_config_file(path: &Path) -> Result<OpenDocumentLimits, LimitsConfigError> {
        let text = std::fs::read_to_string(path).map_err(|e| LimitsConfigError::Io {
            message: format!("{}: {e}", path.display()),
        })?;
        OpenDocumentLimits::from_config_str(&text)
    }
}

/// 제한 초과 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TooManyOpenDocuments {
    /// 제한 값.
    pub limit: usize,
}

impl fmt::Display for TooManyOpenDocuments {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "열린 문서가 최대 {}개입니다. 다른 문서를 닫은 뒤 다시 시도하세요",
            self.limit
        )
    }
}

impl std::error::Error for TooManyOpenDocuments {}

/// 제한을 적용해 문서를 엽니다.
///
/// - 이미 열린 문서는 제한과 무관하게 기존 handle을 반환합니다(새 자원 없음).
/// - 새 문서가 제한을 초과하면 오류를 반환하고 **어떤 문서도 자동으로 닫지
///   않습니다** — 집합은 그대로입니다.
pub fn open_with_limit(
    set: &mut OpenDocumentSet,
    document_id: StableId,
    limits: &OpenDocumentLimits,
) -> Result<OpenOutcome, TooManyOpenDocuments> {
    if !set.is_open(document_id) && set.len() >= limits.max_open_documents {
        return Err(TooManyOpenDocuments {
            limit: limits.max_open_documents,
        });
    }
    Ok(set.open(document_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    fn repo_limits() -> OpenDocumentLimits {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../config/document-limits.yaml"
        );
        OpenDocumentLimits::from_config_file(Path::new(path)).unwrap()
    }

    #[test]
    fn repo_config_parses_with_limit_ten() {
        assert_eq!(repo_limits().max_open_documents, 10);
    }

    #[test]
    fn ten_documents_succeed_and_eleventh_fails_clearly() {
        // 검증 기준(VI.1.5): 10개 성공, 11번째 명확한 오류.
        let limits = repo_limits();
        let mut set = OpenDocumentSet::new();
        for i in 0..10 {
            open_with_limit(&mut set, stable(i), &limits).unwrap();
        }
        assert_eq!(set.len(), 10);

        let err = open_with_limit(&mut set, stable(10), &limits).unwrap_err();
        assert_eq!(err, TooManyOpenDocuments { limit: 10 });
        assert!(err.to_string().contains("10"), "명확한 오류 메시지");
        // 기존 문서를 자동으로 닫지 않음.
        assert_eq!(set.len(), 10);
        for i in 0..10 {
            assert!(set.is_open(stable(i)), "기존 문서 {i} 유지");
        }
    }

    #[test]
    fn duplicate_open_at_limit_still_returns_existing() {
        let limits = repo_limits();
        let mut set = OpenDocumentSet::new();
        for i in 0..10 {
            open_with_limit(&mut set, stable(i), &limits).unwrap();
        }
        // 제한에 도달했어도 이미 열린 문서는 기존 handle 반환.
        let outcome = open_with_limit(&mut set, stable(3), &limits).unwrap();
        assert!(matches!(outcome, OpenOutcome::Existing(_)));
    }

    #[test]
    fn close_frees_capacity() {
        let limits = OpenDocumentLimits {
            max_open_documents: 2,
        };
        let mut set = OpenDocumentSet::new();
        let first = open_with_limit(&mut set, stable(1), &limits)
            .unwrap()
            .handle();
        open_with_limit(&mut set, stable(2), &limits).unwrap();
        assert!(open_with_limit(&mut set, stable(3), &limits).is_err());
        set.close(first);
        open_with_limit(&mut set, stable(3), &limits).unwrap();
    }

    #[test]
    fn malformed_configs_are_rejected() {
        for (text, name) in [
            ("max_open_documents: 0", "0 거부"),
            ("max_open_documents: abc", "비정수"),
            ("", "누락"),
            ("max_open_documents: 10\nunknown: 1", "unknown key"),
        ] {
            assert!(
                OpenDocumentLimits::from_config_str(text).is_err(),
                "{name}이 거부되어야 함"
            );
        }
    }
}
