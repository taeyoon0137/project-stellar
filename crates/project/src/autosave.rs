//! 지연 자동 저장 정책 (XIII.1.1).
//!
//! 기술 아키텍처 22.1: 완료된 편집은 마지막 입력 뒤 짧은 debounce 후 비동기로 저장하고, `Save Now`·창
//! 종료·앱 비활성화 때 flush합니다. 이 모듈은 그 수치를 담는 [`AutosavePolicy`]를 정의하고 값을
//! `config/performance-budgets.yaml`의 `autosave` 절에서 읽습니다.
//!
//! ```yaml
//! autosave:
//!   debounce_ms: <양의 정수>                    # 마지막 입력 뒤 대기
//!   max_delay_ms: <debounce_ms 이상의 정수>     # 입력이 이어져도 캡처하는 최대 지연
//!   app_deactivate_flush_timeout_ms: <양의 정수> # 앱 비활성화 flush 제한 시간
//!   window_close_flush_timeout_ms: <양의 정수>   # 창 닫기 flush 제한 시간
//! ```
//!
//! - 값이 없으면 **기본값을 만들지 않고** 거부합니다([`AutosaveConfigError::MissingSection`]·
//!   [`AutosaveConfigError::MissingKey`]). 수치는 벤치마크로 확정하는 값이기 때문입니다(기술 아키텍처
//!   42장 2번).
//! - 범위 밖 값: 0 이하의 시간, `max_delay_ms < debounce_ms`, 정수가 아니거나 `u32`를 넘는 값, 알 수 없는
//!   키는 거부합니다.
//! - 앱 비활성화·창 닫기 flush는 아키텍처가 요구하는 동작이라 끌 수 없고, 제한 시간만 설정합니다.
//!
//! 무엇이 저장 대상(dirty)인지는 [`dirty`](XIII.1.2)가, 언제 저장할지는 [`schedule`]이 정합니다. 제품 값은
//! `config/performance-budgets.yaml`의 `autosave` 절(ADR-0072)에 있습니다.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::yaml::{load_restricted, scalar, LoaderError, YamlValue};

/// 완료된 document transaction만 만드는 자동 저장 dirty 상태 (XIII.1.2).
pub mod dirty;
/// debounce·최대 지연·제스처·실패를 반영한 창별 자동 저장 시점 (XIII.1.3·XIII.1.5).
pub mod schedule;

/// `performance-budgets.yaml`에서 자동 저장 값을 담는 절 이름.
pub const AUTOSAVE_SECTION: &str = "autosave";

/// 자동 저장 config 키.
const KEYS: [&str; 4] = [
    "debounce_ms",
    "max_delay_ms",
    "app_deactivate_flush_timeout_ms",
    "window_close_flush_timeout_ms",
];

/// flush 정책(항상 켜짐, 제한 시간만 설정).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlushPolicy {
    /// flush 제한 시간. 넘으면 셸에 결정 상태를 돌려줍니다(XIII.1.6).
    pub timeout: Duration,
}

/// 지연 자동 저장 정책.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutosavePolicy {
    /// 마지막 document command 뒤 snapshot 캡처까지 기다리는 시간.
    pub debounce: Duration,
    /// 입력이 계속 이어져도 이 시간이 지나면 캡처합니다(`debounce` 이상).
    pub max_delay: Duration,
    /// 앱 비활성화 flush.
    pub app_deactivate_flush: FlushPolicy,
    /// 창 닫기 flush.
    pub window_close_flush: FlushPolicy,
}

/// 자동 저장 config 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutosaveConfigError {
    /// config 파일을 읽지 못함.
    Io {
        /// 경로.
        path: PathBuf,
        /// io 오류 종류.
        kind: io::ErrorKind,
    },
    /// YAML 문법 오류.
    Syntax(LoaderError),
    /// 최상위가 매핑이 아니거나 `autosave` 절이 없음.
    MissingSection,
    /// `autosave` 절이 매핑이 아님.
    SectionNotMapping,
    /// 필수 키 누락.
    MissingKey {
        /// 키.
        key: &'static str,
    },
    /// 알 수 없는 키.
    UnknownKey {
        /// 키.
        key: String,
    },
    /// 정수(밀리초)로 해석할 수 없는 값.
    InvalidValue {
        /// 키.
        key: &'static str,
        /// 원문 값.
        value: String,
    },
    /// 범위 밖 값.
    OutOfRange {
        /// 키.
        key: &'static str,
        /// 사유.
        reason: &'static str,
    },
}

impl std::fmt::Display for AutosaveConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AutosaveConfigError::Io { path, kind } => {
                write!(f, "자동 저장 config를 읽지 못했습니다 {path:?}: {kind:?}")
            }
            AutosaveConfigError::Syntax(e) => write!(f, "자동 저장 config 문법 오류: {e}"),
            AutosaveConfigError::MissingSection => {
                write!(
                    f,
                    "`{AUTOSAVE_SECTION}` 절이 없습니다(기본값을 쓰지 않습니다)"
                )
            }
            AutosaveConfigError::SectionNotMapping => {
                write!(f, "`{AUTOSAVE_SECTION}` 절은 매핑이어야 합니다")
            }
            AutosaveConfigError::MissingKey { key } => {
                write!(f, "자동 저장 config 키 누락: {AUTOSAVE_SECTION}.{key}")
            }
            AutosaveConfigError::UnknownKey { key } => {
                write!(
                    f,
                    "알 수 없는 자동 저장 config 키: {AUTOSAVE_SECTION}.{key}"
                )
            }
            AutosaveConfigError::InvalidValue { key, value } => write!(
                f,
                "{AUTOSAVE_SECTION}.{key}는 밀리초 정수여야 합니다: {value:?}"
            ),
            AutosaveConfigError::OutOfRange { key, reason } => {
                write!(f, "{AUTOSAVE_SECTION}.{key} 범위 밖: {reason}")
            }
        }
    }
}

impl std::error::Error for AutosaveConfigError {}

impl AutosavePolicy {
    /// `performance-budgets.yaml` 텍스트에서 정책을 읽습니다.
    pub fn from_budgets_yaml(text: &str) -> Result<AutosavePolicy, AutosaveConfigError> {
        let root = load_restricted(text).map_err(AutosaveConfigError::Syntax)?;
        let section = root
            .as_mapping()
            .and_then(|_| root.get(AUTOSAVE_SECTION))
            .ok_or(AutosaveConfigError::MissingSection)?;
        let pairs = section
            .as_mapping()
            .ok_or(AutosaveConfigError::SectionNotMapping)?;
        for (key, _) in pairs {
            let known = key.as_scalar().is_some_and(|k| KEYS.contains(&k));
            if !known {
                return Err(AutosaveConfigError::UnknownKey {
                    key: key.as_scalar().unwrap_or("<비스칼라>").to_string(),
                });
            }
        }
        let debounce = positive_ms(section, "debounce_ms")?;
        let max_delay = positive_ms(section, "max_delay_ms")?;
        if max_delay < debounce {
            return Err(AutosaveConfigError::OutOfRange {
                key: "max_delay_ms",
                reason: "debounce_ms보다 작습니다",
            });
        }
        Ok(AutosavePolicy {
            debounce,
            max_delay,
            app_deactivate_flush: FlushPolicy {
                timeout: positive_ms(section, "app_deactivate_flush_timeout_ms")?,
            },
            window_close_flush: FlushPolicy {
                timeout: positive_ms(section, "window_close_flush_timeout_ms")?,
            },
        })
    }

    /// config 파일에서 정책을 읽습니다.
    pub fn load(path: &Path) -> Result<AutosavePolicy, AutosaveConfigError> {
        let text = std::fs::read_to_string(path).map_err(|e| AutosaveConfigError::Io {
            path: path.to_path_buf(),
            kind: e.kind(),
        })?;
        AutosavePolicy::from_budgets_yaml(&text)
    }
}

/// 양의 밀리초 정수 키를 읽습니다.
fn positive_ms(section: &YamlValue, key: &'static str) -> Result<Duration, AutosaveConfigError> {
    let value = section
        .get(key)
        .ok_or(AutosaveConfigError::MissingKey { key })?;
    let text = value
        .as_scalar()
        .ok_or_else(|| AutosaveConfigError::InvalidValue {
            key,
            value: "<비스칼라>".to_string(),
        })?;
    let ms = scalar::parse_u32(text).ok_or_else(|| AutosaveConfigError::InvalidValue {
        key,
        value: text.to_string(),
    })?;
    if ms == 0 {
        return Err(AutosaveConfigError::OutOfRange {
            key,
            reason: "0보다 커야 합니다",
        });
    }
    Ok(Duration::from_millis(u64::from(ms)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 테스트 입력 전용 값(제품 값이 아님 — 제품 값은 벤치마크로 정해 config에 기록).
    const SAMPLE: &str = "\
color:
  transfer_abs_tolerance: 1.0e-5
autosave:
  debounce_ms: 700
  max_delay_ms: 5000
  app_deactivate_flush_timeout_ms: 2000
  window_close_flush_timeout_ms: 3000
";

    #[test]
    fn valid_section_is_read() {
        let policy = AutosavePolicy::from_budgets_yaml(SAMPLE).unwrap();
        assert_eq!(
            policy,
            AutosavePolicy {
                debounce: Duration::from_millis(700),
                max_delay: Duration::from_millis(5000),
                app_deactivate_flush: FlushPolicy {
                    timeout: Duration::from_millis(2000)
                },
                window_close_flush: FlushPolicy {
                    timeout: Duration::from_millis(3000)
                },
            }
        );
        // max_delay == debounce는 허용(경계).
        let equal = SAMPLE.replace("max_delay_ms: 5000", "max_delay_ms: 700");
        assert!(AutosavePolicy::from_budgets_yaml(&equal).is_ok());
    }

    #[test]
    fn missing_config_is_rejected_without_defaults() {
        assert_eq!(
            AutosavePolicy::from_budgets_yaml("color:\n  transfer_abs_tolerance: 1.0e-5\n"),
            Err(AutosaveConfigError::MissingSection)
        );
        assert_eq!(
            AutosavePolicy::from_budgets_yaml("- a\n"),
            Err(AutosaveConfigError::MissingSection)
        );
        assert_eq!(
            AutosavePolicy::from_budgets_yaml("autosave: 5\n"),
            Err(AutosaveConfigError::SectionNotMapping)
        );
        for key in KEYS {
            let without: String = SAMPLE
                .lines()
                .filter(|line| !line.trim_start().starts_with(key))
                .map(|line| format!("{line}\n"))
                .collect();
            assert_eq!(
                AutosavePolicy::from_budgets_yaml(&without),
                Err(AutosaveConfigError::MissingKey {
                    key: KEYS.into_iter().find(|k| *k == key).unwrap()
                }),
                "{key}"
            );
        }
        assert!(matches!(
            AutosavePolicy::load(Path::new("/nonexistent/performance-budgets.yaml")),
            Err(AutosaveConfigError::Io { .. })
        ));
        assert!(matches!(
            AutosavePolicy::from_budgets_yaml("autosave: [\n"),
            Err(AutosaveConfigError::Syntax(_))
        ));
    }

    #[test]
    fn out_of_range_values_are_rejected() {
        for (from, to, expected) in [
            (
                "debounce_ms: 700",
                "debounce_ms: 0",
                AutosaveConfigError::OutOfRange {
                    key: "debounce_ms",
                    reason: "0보다 커야 합니다",
                },
            ),
            (
                "window_close_flush_timeout_ms: 3000",
                "window_close_flush_timeout_ms: 0",
                AutosaveConfigError::OutOfRange {
                    key: "window_close_flush_timeout_ms",
                    reason: "0보다 커야 합니다",
                },
            ),
            (
                "max_delay_ms: 5000",
                "max_delay_ms: 699",
                AutosaveConfigError::OutOfRange {
                    key: "max_delay_ms",
                    reason: "debounce_ms보다 작습니다",
                },
            ),
            (
                "debounce_ms: 700",
                "debounce_ms: -1",
                AutosaveConfigError::InvalidValue {
                    key: "debounce_ms",
                    value: "-1".to_string(),
                },
            ),
            (
                "debounce_ms: 700",
                "debounce_ms: 0.5",
                AutosaveConfigError::InvalidValue {
                    key: "debounce_ms",
                    value: "0.5".to_string(),
                },
            ),
            (
                "debounce_ms: 700",
                "debounce_ms: 4294967296",
                AutosaveConfigError::InvalidValue {
                    key: "debounce_ms",
                    value: "4294967296".to_string(),
                },
            ),
            (
                "max_delay_ms: 5000",
                "max_delay_ms: [1]",
                AutosaveConfigError::InvalidValue {
                    key: "max_delay_ms",
                    value: "<비스칼라>".to_string(),
                },
            ),
            (
                "max_delay_ms: 5000",
                "max_delay_ms: 5000\n  flush_on_save: false",
                AutosaveConfigError::UnknownKey {
                    key: "flush_on_save".to_string(),
                },
            ),
        ] {
            let text = SAMPLE.replace(from, to);
            assert_eq!(
                AutosavePolicy::from_budgets_yaml(&text),
                Err(expected),
                "{to}"
            );
        }
    }

    #[test]
    fn repository_budget_file_is_well_formed() {
        // 앱 runtime이 이 절을 읽어 자동 저장을 켜므로(ADR-0072) 저장소 config는 유효해야 합니다.
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("config")
            .join("performance-budgets.yaml");
        match AutosavePolicy::load(&path) {
            Ok(policy) => assert!(policy.max_delay >= policy.debounce),
            Err(other) => panic!("performance-budgets.yaml 자동 저장 절 오류: {other}"),
        }
    }
}
