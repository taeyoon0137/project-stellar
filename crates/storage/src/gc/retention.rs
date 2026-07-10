//! 보존 generation 정책 (V.7.1).
//!
//! 기본값: **current + 직전 정상 세대 + 복구에 필요한 세대**를 보존합니다.
//! 수치(최근 세대 수, sweep 유예 시간)는 코드에 하드코딩하지 않고
//! `config/storage-gc.yaml`에서 읽습니다.

use std::fmt;
use std::path::Path;

use std::collections::BTreeSet;

/// 정책 config 파싱 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetentionConfigError {
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

impl fmt::Display for RetentionConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RetentionConfigError::MissingKey { key } => {
                write!(f, "storage-gc config에 {key}가 없습니다")
            }
            RetentionConfigError::InvalidValue { key, value } => {
                write!(f, "storage-gc config {key} 값이 잘못되었습니다: {value:?}")
            }
            RetentionConfigError::UnknownKey { key } => {
                write!(f, "storage-gc config에 알 수 없는 key: {key}")
            }
            RetentionConfigError::Io { message } => write!(f, "config 읽기 실패: {message}"),
        }
    }
}

impl std::error::Error for RetentionConfigError {}

/// 보존 정책 수치.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// 항상 보존할 최근 정상 세대 수(current 포함, 1 이상).
    pub keep_recent_generations: u64,
    /// unreferenced chunk sweep 유예 시간(초, V.7.3).
    pub grace_period_seconds: u64,
}

impl RetentionPolicy {
    /// `key: value` 한 줄 형식의 config 텍스트에서 정책을 읽습니다.
    pub fn from_config_str(text: &str) -> Result<RetentionPolicy, RetentionConfigError> {
        let mut keep: Option<u64> = None;
        let mut grace: Option<u64> = None;
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once(':') else {
                return Err(RetentionConfigError::UnknownKey {
                    key: line.to_string(),
                });
            };
            let key = key.trim();
            let value = value.trim();
            match key {
                "keep_recent_generations" => {
                    let parsed: u64 = value.parse().ok().filter(|v| *v >= 1).ok_or_else(|| {
                        RetentionConfigError::InvalidValue {
                            key: "keep_recent_generations",
                            value: value.to_string(),
                        }
                    })?;
                    keep = Some(parsed);
                }
                "grace_period_seconds" => {
                    let parsed: u64 =
                        value
                            .parse()
                            .map_err(|_| RetentionConfigError::InvalidValue {
                                key: "grace_period_seconds",
                                value: value.to_string(),
                            })?;
                    grace = Some(parsed);
                }
                other => {
                    return Err(RetentionConfigError::UnknownKey {
                        key: other.to_string(),
                    })
                }
            }
        }
        Ok(RetentionPolicy {
            keep_recent_generations: keep.ok_or(RetentionConfigError::MissingKey {
                key: "keep_recent_generations",
            })?,
            grace_period_seconds: grace.ok_or(RetentionConfigError::MissingKey {
                key: "grace_period_seconds",
            })?,
        })
    }

    /// config 파일에서 정책을 읽습니다.
    pub fn from_config_file(path: &Path) -> Result<RetentionPolicy, RetentionConfigError> {
        let text = std::fs::read_to_string(path).map_err(|e| RetentionConfigError::Io {
            message: format!("{}: {e}", path.display()),
        })?;
        RetentionPolicy::from_config_str(&text)
    }
}

/// 보존 판정 입력.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionInput {
    /// current pointer가 가리키는 세대(읽기 실패 시 `None`).
    pub current: Option<u64>,
    /// 완전 검증을 통과한 세대 번호들(V.4.5 scanner 결과).
    pub valid_generations: Vec<u64>,
    /// 복구 시 선택될 세대(V.4.5). current가 정상이면 보통 `None`.
    pub recovery_generation: Option<u64>,
}

/// 보존할 세대 집합을 판정합니다.
///
/// - current 세대는 항상 보존합니다(손상 여부와 무관 — 진단 자료).
/// - 정상 세대 중 최신 `keep_recent_generations`개를 보존합니다.
/// - 복구에 필요한 세대를 보존합니다.
pub fn retained_generations(policy: &RetentionPolicy, input: &RetentionInput) -> BTreeSet<u64> {
    let mut retained = BTreeSet::new();
    if let Some(current) = input.current {
        retained.insert(current);
    }
    let mut valid = input.valid_generations.clone();
    valid.sort_unstable();
    for generation in valid
        .iter()
        .rev()
        .take(policy.keep_recent_generations as usize)
    {
        retained.insert(*generation);
    }
    if let Some(recovery) = input.recovery_generation {
        retained.insert(recovery);
    }
    retained
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_policy() -> RetentionPolicy {
        // 저장소 config 파일 자체를 fixture로 사용합니다.
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/storage-gc.yaml");
        RetentionPolicy::from_config_file(Path::new(path)).unwrap()
    }

    #[test]
    fn repo_config_parses_with_expected_defaults() {
        let policy = default_policy();
        assert_eq!(policy.keep_recent_generations, 2);
        assert_eq!(policy.grace_period_seconds, 86_400);
    }

    #[test]
    fn default_keeps_current_and_previous() {
        // 검증 기준(V.7.1): policy fixture — current + 직전 정상 세대.
        let policy = default_policy();
        let input = RetentionInput {
            current: Some(5),
            valid_generations: vec![1, 2, 3, 4, 5],
            recovery_generation: None,
        };
        let retained = retained_generations(&policy, &input);
        assert_eq!(retained, [4, 5].into_iter().collect());
    }

    #[test]
    fn recovery_generation_is_always_kept() {
        // current 손상: fallback 세대(2)가 복구에 필요 → 보존.
        let policy = default_policy();
        let input = RetentionInput {
            current: Some(5),
            valid_generations: vec![1, 2],
            recovery_generation: Some(2),
        };
        let retained = retained_generations(&policy, &input);
        // current(5, 손상이어도 진단 자료) + 최신 정상 2개(1,2) + 복구(2).
        assert_eq!(retained, [1, 2, 5].into_iter().collect());
    }

    #[test]
    fn larger_keep_count_retains_more() {
        let policy = RetentionPolicy {
            keep_recent_generations: 4,
            grace_period_seconds: 0,
        };
        let input = RetentionInput {
            current: Some(6),
            valid_generations: vec![1, 2, 3, 4, 5, 6],
            recovery_generation: None,
        };
        let retained = retained_generations(&policy, &input);
        assert_eq!(retained, [3, 4, 5, 6].into_iter().collect());
    }

    #[test]
    fn missing_current_pointer_still_keeps_valid_generations() {
        let policy = default_policy();
        let input = RetentionInput {
            current: None,
            valid_generations: vec![7, 8],
            recovery_generation: Some(8),
        };
        let retained = retained_generations(&policy, &input);
        assert_eq!(retained, [7, 8].into_iter().collect());
    }

    #[test]
    fn malformed_configs_are_rejected() {
        let cases = [
            (
                "keep_recent_generations: 0\ngrace_period_seconds: 1",
                "0 거부",
            ),
            (
                "keep_recent_generations: abc\ngrace_period_seconds: 1",
                "비정수",
            ),
            ("grace_period_seconds: 1", "keep 누락"),
            ("keep_recent_generations: 2", "grace 누락"),
            (
                "keep_recent_generations: 2\ngrace_period_seconds: 1\nunknown_key: 3",
                "unknown key",
            ),
        ];
        for (text, name) in cases {
            assert!(
                RetentionPolicy::from_config_str(text).is_err(),
                "{name}이 거부되어야 함"
            );
        }
    }
}
