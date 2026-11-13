//! 터미널 child 환경 변수 허용 목록 (XVIII.1.1).
//!
//! 터미널 child는 앱 프로세스 환경을 그대로 물려받지 않습니다. 실행할 때 환경을 모두 지우고
//! (`env_clear`) 다음 두 가지만 전달합니다(ADR-0019 보안 절 "환경 변수 허용 목록 원칙").
//!
//! 1. **상속 허용 목록**([`EnvironmentPolicy::inherited`]): 이름만 기록하고 값은 실행 시점의 앱
//!    프로세스 환경에서 읽습니다. 앱 환경에 없으면 전달하지 않습니다.
//! 2. **명시 지정**([`EnvironmentPolicy::assignments`]): 사용자가 profile에 적은 이름=값. 같은 이름이
//!    상속 목록에도 있으면 명시 지정이 이깁니다.
//!
//! 이름은 POSIX 이식 문자 집합(`[A-Za-z_][A-Za-z0-9_]*`)만 허용하고, 개수·길이에 상한을 둡니다.
//! 결과는 이름 순으로 정렬되어 결정론적입니다.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt;

/// 상속 목록과 명시 지정을 합친 항목 수 상한.
pub const MAX_ENV_ENTRIES: usize = 256;
/// 이름 길이 상한(byte).
pub const MAX_ENV_NAME_BYTES: usize = 256;
/// 명시 지정 값 길이 상한(byte).
pub const MAX_ENV_VALUE_BYTES: usize = 32 * 1024;

/// 기본 상속 목록. 셸이 사용자 환경에서 정상 동작하는 데 필요한 최소 변수입니다.
pub const DEFAULT_INHERITED_ENV: [&str; 9] = [
    "HOME", "LANG", "LC_ALL", "LC_CTYPE", "LOGNAME", "PATH", "TMPDIR", "TZ", "USER",
];

/// 검증된 환경 변수 이름.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EnvName(String);

impl EnvName {
    /// 이름을 검증합니다.
    pub fn new(name: &str) -> Result<EnvName, EnvError> {
        if name.is_empty() || name.len() > MAX_ENV_NAME_BYTES {
            return Err(EnvError::InvalidName {
                name: truncate_for_report(name),
            });
        }
        let mut bytes = name.bytes();
        let first_ok = bytes
            .next()
            .is_some_and(|b| b == b'_' || b.is_ascii_alphabetic());
        let rest_ok = bytes.all(|b| b == b'_' || b.is_ascii_alphanumeric());
        if !first_ok || !rest_ok {
            return Err(EnvError::InvalidName {
                name: truncate_for_report(name),
            });
        }
        Ok(EnvName(name.to_owned()))
    }

    /// 이름 문자열.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EnvName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 환경 정책 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvError {
    /// 이름이 비었거나 허용 문자 집합 밖이거나 너무 깁니다(보고용으로 앞부분만 보관).
    InvalidName {
        /// 거부된 이름(최대 64 byte).
        name: String,
    },
    /// 명시 지정 값이 [`MAX_ENV_VALUE_BYTES`]를 넘습니다.
    ValueTooLong {
        /// 변수 이름.
        name: EnvName,
        /// 값 길이(byte).
        len: usize,
    },
    /// 명시 지정 값에 NUL byte가 있습니다(exec 환경에 넣을 수 없음).
    ValueContainsNul {
        /// 변수 이름.
        name: EnvName,
    },
    /// 항목 수가 [`MAX_ENV_ENTRIES`]를 넘습니다.
    TooManyEntries {
        /// 상한.
        limit: usize,
    },
}

impl fmt::Display for EnvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EnvError::InvalidName { name } => {
                write!(f, "환경 변수 이름이 올바르지 않습니다: {name:?}")
            }
            EnvError::ValueTooLong { name, len } => write!(
                f,
                "환경 변수 {name}의 값이 너무 깁니다({len} byte, 상한 {MAX_ENV_VALUE_BYTES})"
            ),
            EnvError::ValueContainsNul { name } => {
                write!(f, "환경 변수 {name}의 값에 NUL 문자가 있습니다")
            }
            EnvError::TooManyEntries { limit } => {
                write!(f, "환경 변수 항목이 너무 많습니다(상한 {limit})")
            }
        }
    }
}

impl std::error::Error for EnvError {}

/// 터미널 child 환경 정책: 상속 허용 목록 + 명시 지정.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvironmentPolicy {
    inherited: Vec<EnvName>,
    assignments: BTreeMap<EnvName, OsString>,
}

impl EnvironmentPolicy {
    /// 아무것도 전달하지 않는 정책.
    pub fn empty() -> EnvironmentPolicy {
        EnvironmentPolicy::default()
    }

    /// [`DEFAULT_INHERITED_ENV`]를 상속하는 정책.
    pub fn with_defaults() -> EnvironmentPolicy {
        let mut policy = EnvironmentPolicy::empty();
        for name in DEFAULT_INHERITED_ENV {
            // 상수 목록은 모두 올바른 이름이고 상한보다 짧습니다.
            if let Ok(name) = EnvName::new(name) {
                policy.insert_inherited(name);
            }
        }
        policy
    }

    /// 상속 허용 목록에 이름을 더합니다(이미 있으면 그대로).
    pub fn inherit(&mut self, name: &str) -> Result<(), EnvError> {
        let name = EnvName::new(name)?;
        if self.inherited.contains(&name) {
            return Ok(());
        }
        self.ensure_capacity()?;
        self.insert_inherited(name);
        Ok(())
    }

    /// 이름=값을 명시 지정합니다(같은 이름이 있으면 값을 바꿉니다).
    pub fn set(&mut self, name: &str, value: impl Into<OsString>) -> Result<(), EnvError> {
        let name = EnvName::new(name)?;
        let value = value.into();
        let bytes = value.as_encoded_bytes();
        if bytes.len() > MAX_ENV_VALUE_BYTES {
            return Err(EnvError::ValueTooLong {
                name,
                len: bytes.len(),
            });
        }
        if bytes.contains(&0) {
            return Err(EnvError::ValueContainsNul { name });
        }
        if !self.assignments.contains_key(&name) {
            self.ensure_capacity()?;
        }
        self.assignments.insert(name, value);
        Ok(())
    }

    /// 상속 허용 목록(이름 순).
    pub fn inherited(&self) -> &[EnvName] {
        &self.inherited
    }

    /// 명시 지정(이름 순).
    pub fn assignments(&self) -> impl Iterator<Item = (&EnvName, &OsStr)> {
        self.assignments
            .iter()
            .map(|(name, value)| (name, value.as_os_str()))
    }

    /// 명시 지정 값.
    pub fn assignment(&self, name: &str) -> Option<&OsStr> {
        self.assignments
            .iter()
            .find(|(key, _)| key.as_str() == name)
            .map(|(_, value)| value.as_os_str())
    }

    /// child에 전달할 최종 환경(이름 순). `lookup`은 상속 변수의 현재 값을 돌려줍니다.
    pub fn resolve(&self, lookup: impl Fn(&str) -> Option<OsString>) -> Vec<(OsString, OsString)> {
        let mut resolved: BTreeMap<&str, OsString> = BTreeMap::new();
        for name in &self.inherited {
            if let Some(value) = lookup(name.as_str()) {
                resolved.insert(name.as_str(), value);
            }
        }
        for (name, value) in &self.assignments {
            resolved.insert(name.as_str(), value.clone());
        }
        resolved
            .into_iter()
            .map(|(name, value)| (OsString::from(name), value))
            .collect()
    }

    /// 현재 앱 프로세스 환경에서 상속 값을 읽어 최종 환경을 만듭니다.
    pub fn resolve_from_process(&self) -> Vec<(OsString, OsString)> {
        self.resolve(|name| std::env::var_os(name))
    }

    fn ensure_capacity(&self) -> Result<(), EnvError> {
        if self.inherited.len() + self.assignments.len() >= MAX_ENV_ENTRIES {
            return Err(EnvError::TooManyEntries {
                limit: MAX_ENV_ENTRIES,
            });
        }
        Ok(())
    }

    fn insert_inherited(&mut self, name: EnvName) {
        match self.inherited.binary_search(&name) {
            Ok(_) => {}
            Err(index) => self.inherited.insert(index, name),
        }
    }
}

fn truncate_for_report(name: &str) -> String {
    name.char_indices()
        .take_while(|(index, _)| *index < 64)
        .map(|(_, c)| c)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(name: &str) -> Option<OsString> {
        match name {
            "HOME" => Some("/Users/example".into()),
            "PATH" => Some("/usr/bin:/bin".into()),
            "SECRET_TOKEN" => Some("must-not-leak".into()),
            "LANG" => Some("ko_KR.UTF-8".into()),
            _ => None,
        }
    }

    #[test]
    fn names_follow_portable_charset() {
        for ok in ["HOME", "_X", "a1_B2", "LC_ALL"] {
            assert!(EnvName::new(ok).is_ok(), "{ok}");
        }
        for bad in ["", "1ABC", "A-B", "A=B", "A B", "ÄBC", "A\0B"] {
            assert!(
                matches!(EnvName::new(bad), Err(EnvError::InvalidName { .. })),
                "{bad:?}"
            );
        }
        let long = "A".repeat(MAX_ENV_NAME_BYTES + 1);
        assert!(EnvName::new(&long).is_err());
        assert!(EnvName::new(&"A".repeat(MAX_ENV_NAME_BYTES)).is_ok());
    }

    #[test]
    fn only_allowlisted_and_assigned_variables_are_passed() {
        let mut policy = EnvironmentPolicy::empty();
        policy.inherit("PATH").unwrap();
        policy.inherit("HOME").unwrap();
        policy.inherit("MISSING_IN_PROCESS").unwrap();
        policy.set("TERM_PROGRAM", "Stellar").unwrap();
        let env = policy.resolve(lookup);
        assert_eq!(
            env,
            vec![
                (OsString::from("HOME"), OsString::from("/Users/example")),
                (OsString::from("PATH"), OsString::from("/usr/bin:/bin")),
                (OsString::from("TERM_PROGRAM"), OsString::from("Stellar")),
            ]
        );
        // 허용 목록에 없는 앱 환경 변수는 전달하지 않습니다.
        assert!(env.iter().all(|(name, _)| name != "SECRET_TOKEN"));
    }

    #[test]
    fn assignment_overrides_inherited_value() {
        let mut policy = EnvironmentPolicy::with_defaults();
        policy.set("LANG", "en_US.UTF-8").unwrap();
        let env = policy.resolve(lookup);
        let lang = env.iter().find(|(name, _)| name == "LANG").unwrap();
        assert_eq!(lang.1, OsString::from("en_US.UTF-8"));
        assert_eq!(policy.assignment("LANG"), Some(OsStr::new("en_US.UTF-8")));
    }

    #[test]
    fn defaults_are_sorted_and_deduplicated() {
        let mut policy = EnvironmentPolicy::with_defaults();
        policy.inherit("PATH").unwrap();
        let names: Vec<&str> = policy.inherited().iter().map(EnvName::as_str).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(names, sorted);
        assert_eq!(names.len(), DEFAULT_INHERITED_ENV.len());
    }

    #[test]
    fn values_and_counts_are_bounded() {
        let mut policy = EnvironmentPolicy::empty();
        let long = "x".repeat(MAX_ENV_VALUE_BYTES + 1);
        assert!(matches!(
            policy.set("BIG", long),
            Err(EnvError::ValueTooLong { len, .. }) if len == MAX_ENV_VALUE_BYTES + 1
        ));
        assert!(matches!(
            policy.set("NUL", "a\0b"),
            Err(EnvError::ValueContainsNul { .. })
        ));
        for index in 0..MAX_ENV_ENTRIES {
            policy.set(&format!("V{index}"), "1").unwrap();
        }
        assert_eq!(
            policy.inherit("ONE_MORE"),
            Err(EnvError::TooManyEntries {
                limit: MAX_ENV_ENTRIES
            })
        );
        // 이미 있는 이름의 값 변경은 상한과 무관합니다.
        policy.set("V0", "2").unwrap();
        assert_eq!(policy.assignment("V0"), Some(OsStr::new("2")));
    }
}
