//! chunk 크기 예산 config reader (V.6.4, ADR-0012).
//!
//! 승인된 chunk 분할 임계치는 코드에 하드코딩하지 않고
//! `config/performance-budgets.yaml`의 `storage:` 절에서 읽습니다:
//!
//! ```yaml
//! storage:
//!   chunk_split_threshold_bytes: 262144
//! ```
//!
//! 이 파일은 다른 예산(color, 추후 T0~T5)과 공유하므로 **다른 최상위 절은
//! 건너뛰고 `storage:` 절만 엄격하게** 검사합니다:
//!
//! - `storage:` 절은 정확히 한 번 나타나야 하고 같은 줄에 값을 둘 수 없습니다.
//! - 절 안의 항목은 공백 2칸 들여쓰기 `key: value` 한 줄씩입니다(tab·중첩 금지).
//! - 알 수 없는 key·중복 key·누락 key를 거부합니다.
//! - 값은 10진 양의 정수입니다. 부호·밑줄·소수점·단위·선행 0(YAML 1.1 8진수와
//!   혼동)을 거부하고 `u64` 범위를 넘으면 거부합니다.
//! - 주석 줄과 줄 끝 주석(공백 뒤 `#`)은 허용합니다.

use std::fmt;
use std::path::Path;

/// 예산 파일에서 storage 항목을 담는 최상위 절 이름.
pub const STORAGE_BUDGET_SECTION: &str = "storage";

/// chunk 분할 임계치 key(`storage.chunk_split_threshold_bytes`).
pub const SPLIT_THRESHOLD_KEY: &str = "chunk_split_threshold_bytes";

/// `storage:` 절 항목의 들여쓰기 폭(공백 수).
const ENTRY_INDENT: usize = 2;

/// chunk 크기 예산.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkBudget {
    /// chunk 분할 임계치(bytes). envelope 비압축 payload 길이(ADR-0010) 기준이며,
    /// payload가 이 값을 **넘으면** 분할 대상입니다(같으면 분할하지 않음).
    pub split_threshold_bytes: u64,
}

/// 예산 config 파싱 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkBudgetError {
    /// `storage:` 절이 없습니다.
    MissingSection,
    /// `storage:` 절이 두 번 이상 나타납니다.
    DuplicateSection {
        /// 두 번째 절의 줄 번호(1부터).
        line: usize,
    },
    /// 필수 key가 없습니다.
    MissingKey {
        /// 없는 key.
        key: &'static str,
    },
    /// 같은 key가 두 번 나타납니다.
    DuplicateKey {
        /// 중복 key.
        key: String,
        /// 두 번째 key의 줄 번호(1부터).
        line: usize,
    },
    /// `storage:` 절에 알 수 없는 key가 있습니다.
    UnknownKey {
        /// 해당 key.
        key: String,
        /// 줄 번호(1부터).
        line: usize,
    },
    /// 값이 10진 양의 정수가 아닙니다.
    InvalidValue {
        /// 해당 key.
        key: &'static str,
        /// 원본 값.
        value: String,
    },
    /// `storage:` 절 안의 줄 형식이 잘못되었습니다(들여쓰기, tab, `:` 누락, 중첩 등).
    MalformedLine {
        /// 줄 번호(1부터).
        line: usize,
        /// 원본 줄.
        text: String,
    },
    /// 파일을 읽을 수 없습니다.
    Io {
        /// 원인 메시지.
        message: String,
    },
}

impl fmt::Display for ChunkBudgetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChunkBudgetError::MissingSection => {
                write!(f, "performance-budgets에 {STORAGE_BUDGET_SECTION}: 절이 없습니다")
            }
            ChunkBudgetError::DuplicateSection { line } => write!(
                f,
                "performance-budgets {line}행: {STORAGE_BUDGET_SECTION}: 절이 중복됩니다"
            ),
            ChunkBudgetError::MissingKey { key } => write!(
                f,
                "performance-budgets {STORAGE_BUDGET_SECTION}.{key}가 없습니다"
            ),
            ChunkBudgetError::DuplicateKey { key, line } => write!(
                f,
                "performance-budgets {line}행: {STORAGE_BUDGET_SECTION}.{key}가 중복됩니다"
            ),
            ChunkBudgetError::UnknownKey { key, line } => write!(
                f,
                "performance-budgets {line}행: {STORAGE_BUDGET_SECTION}에 알 수 없는 key: {key}"
            ),
            ChunkBudgetError::InvalidValue { key, value } => write!(
                f,
                "performance-budgets {STORAGE_BUDGET_SECTION}.{key} 값이 양의 10진 정수가 아닙니다: {value:?}"
            ),
            ChunkBudgetError::MalformedLine { line, text } => write!(
                f,
                "performance-budgets {line}행: {STORAGE_BUDGET_SECTION} 절 형식이 잘못되었습니다: {text:?}"
            ),
            ChunkBudgetError::Io { message } => write!(f, "config 읽기 실패: {message}"),
        }
    }
}

impl std::error::Error for ChunkBudgetError {}

/// 줄 끝 주석(공백 또는 tab 뒤 `#`)을 제거합니다.
fn strip_trailing_comment(value: &str) -> &str {
    let bytes = value.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'#' && (i == 0 || bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') {
            return &value[..i];
        }
    }
    value
}

/// 10진 양의 정수(`u64`)를 엄격하게 파싱합니다.
fn parse_positive_decimal(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if value.len() > 1 && value.starts_with('0') {
        return None;
    }
    value.parse::<u64>().ok().filter(|v| *v >= 1)
}

impl ChunkBudget {
    /// 예산 config 텍스트에서 chunk 예산을 읽습니다.
    pub fn from_budget_str(text: &str) -> Result<ChunkBudget, ChunkBudgetError> {
        let mut section_seen = false;
        let mut in_section = false;
        let mut threshold: Option<u64> = None;

        for (index, raw) in text.lines().enumerate() {
            let line_no = index + 1;
            let line = raw.trim_end();
            let trimmed = line.trim_start();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let indent = &line[..line.len() - trimmed.len()];

            if indent.is_empty() {
                // 최상위 줄: 새 절이 시작됩니다. storage 외 절은 다른 reader의 소관입니다.
                in_section = false;
                let Some((key, rest)) = trimmed.split_once(':') else {
                    continue;
                };
                if key != STORAGE_BUDGET_SECTION {
                    continue;
                }
                if !strip_trailing_comment(rest).trim().is_empty() {
                    return Err(ChunkBudgetError::MalformedLine {
                        line: line_no,
                        text: raw.to_string(),
                    });
                }
                if section_seen {
                    return Err(ChunkBudgetError::DuplicateSection { line: line_no });
                }
                section_seen = true;
                in_section = true;
                continue;
            }

            if !in_section {
                continue;
            }

            // storage 절 항목: 정확히 공백 2칸 들여쓰기 + `key: value`.
            let malformed = || ChunkBudgetError::MalformedLine {
                line: line_no,
                text: raw.to_string(),
            };
            if indent.len() != ENTRY_INDENT || indent.bytes().any(|b| b != b' ') {
                return Err(malformed());
            }
            let Some((key, rest)) = trimmed.split_once(':') else {
                return Err(malformed());
            };
            if key.is_empty()
                || !key
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            {
                return Err(malformed());
            }
            let value = strip_trailing_comment(rest).trim();
            if value.is_empty() {
                // 중첩 mapping은 허용하지 않습니다.
                return Err(malformed());
            }
            match key {
                SPLIT_THRESHOLD_KEY => {
                    if threshold.is_some() {
                        return Err(ChunkBudgetError::DuplicateKey {
                            key: key.to_string(),
                            line: line_no,
                        });
                    }
                    let parsed = parse_positive_decimal(value).ok_or_else(|| {
                        ChunkBudgetError::InvalidValue {
                            key: SPLIT_THRESHOLD_KEY,
                            value: value.to_string(),
                        }
                    })?;
                    threshold = Some(parsed);
                }
                other => {
                    return Err(ChunkBudgetError::UnknownKey {
                        key: other.to_string(),
                        line: line_no,
                    })
                }
            }
        }

        if !section_seen {
            return Err(ChunkBudgetError::MissingSection);
        }
        Ok(ChunkBudget {
            split_threshold_bytes: threshold.ok_or(ChunkBudgetError::MissingKey {
                key: SPLIT_THRESHOLD_KEY,
            })?,
        })
    }

    /// 예산 config 파일에서 chunk 예산을 읽습니다.
    pub fn from_budget_file(path: &Path) -> Result<ChunkBudget, ChunkBudgetError> {
        let text = std::fs::read_to_string(path).map_err(|e| ChunkBudgetError::Io {
            message: format!("{}: {e}", path.display()),
        })?;
        ChunkBudget::from_budget_str(&text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 저장소의 실제 예산 파일.
    fn repo_budget() -> ChunkBudget {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../config/performance-budgets.yaml"
        );
        ChunkBudget::from_budget_file(Path::new(path)).unwrap()
    }

    #[test]
    fn repo_budget_has_adr_0012_threshold() {
        // ADR-0012: 256 KiB = 262,144 bytes.
        assert_eq!(repo_budget().split_threshold_bytes, 262_144);
    }

    #[test]
    fn other_sections_are_ignored() {
        let text = "\
# 주석
color:
  transfer_abs_tolerance: 1.0e-5
  nested:
    anything: [1, 2]
storage:
  # 절 안 주석
  chunk_split_threshold_bytes: 4096  # 줄 끝 주석
timing:
  t0: 16
";
        assert_eq!(
            ChunkBudget::from_budget_str(text).unwrap(),
            ChunkBudget {
                split_threshold_bytes: 4096
            }
        );
    }

    #[test]
    fn missing_section_and_key_are_rejected() {
        assert_eq!(
            ChunkBudget::from_budget_str("color:\n  transfer_abs_tolerance: 1.0e-5\n"),
            Err(ChunkBudgetError::MissingSection)
        );
        assert_eq!(
            ChunkBudget::from_budget_str("storage:\n"),
            Err(ChunkBudgetError::MissingKey {
                key: SPLIT_THRESHOLD_KEY
            })
        );
    }

    #[test]
    fn invalid_values_are_rejected() {
        for value in [
            "0",
            "-1",
            "+1",
            "0262144",
            "262_144",
            "256KiB",
            "2.5",
            "1e6",
            "\"1\"",
            "18446744073709551616",
        ] {
            let text = format!("storage:\n  chunk_split_threshold_bytes: {value}\n");
            assert!(
                matches!(
                    ChunkBudget::from_budget_str(&text),
                    Err(ChunkBudgetError::InvalidValue { .. })
                ),
                "값 {value:?}가 거부되지 않음"
            );
        }
        // u64 최댓값은 허용됩니다.
        let text = "storage:\n  chunk_split_threshold_bytes: 18446744073709551615\n";
        assert_eq!(
            ChunkBudget::from_budget_str(text)
                .unwrap()
                .split_threshold_bytes,
            u64::MAX
        );
    }

    #[test]
    fn structural_violations_are_rejected() {
        let malformed = |line: usize, text: &str| ChunkBudgetError::MalformedLine {
            line,
            text: text.to_string(),
        };
        let cases = [
            ("storage: 1\n", malformed(1, "storage: 1")),
            (
                "storage:\n    chunk_split_threshold_bytes: 1\n",
                malformed(2, "    chunk_split_threshold_bytes: 1"),
            ),
            (
                "storage:\n\tchunk_split_threshold_bytes: 1\n",
                malformed(2, "\tchunk_split_threshold_bytes: 1"),
            ),
            (
                "storage:\n  chunk_split_threshold_bytes\n",
                malformed(2, "  chunk_split_threshold_bytes"),
            ),
            (
                "storage:\n  chunk_split_threshold_bytes:\n    x: 1\n",
                malformed(2, "  chunk_split_threshold_bytes:"),
            ),
            (
                "storage:\n  chunk_merge_bytes: 1\n",
                ChunkBudgetError::UnknownKey {
                    key: "chunk_merge_bytes".to_string(),
                    line: 2,
                },
            ),
            (
                "storage:\n  chunk_split_threshold_bytes: 1\n  chunk_split_threshold_bytes: 2\n",
                ChunkBudgetError::DuplicateKey {
                    key: SPLIT_THRESHOLD_KEY.to_string(),
                    line: 3,
                },
            ),
            (
                "storage:\n  chunk_split_threshold_bytes: 1\nstorage:\n  chunk_split_threshold_bytes: 1\n",
                ChunkBudgetError::DuplicateSection { line: 3 },
            ),
        ];
        for (text, expected) in cases {
            let err = ChunkBudget::from_budget_str(text).unwrap_err();
            assert_eq!(err, expected, "{text:?}");
            // Display가 비어 있지 않습니다.
            assert!(!err.to_string().is_empty());
        }
    }

    #[test]
    fn missing_file_is_io_error() {
        let err = ChunkBudget::from_budget_file(Path::new("/nonexistent/budget.yaml")).unwrap_err();
        assert!(matches!(err, ChunkBudgetError::Io { .. }));
    }
}
