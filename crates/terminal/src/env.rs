//! 대화형 터미널 child 환경 (XVIII.2.5, ADR-0023).
//!
//! 대화형 세션 child는 앱 프로세스 환경을 **그대로 물려받습니다**. 환경을 지우거나(`env_clear`) 상속 허용
//! 목록으로 거르지 않고, login 셸이 사용자의 rc·profile 파일로 `PATH`·prompt·alias를 만듭니다(ADR-0023 결정 3).
//! Stellar는 상속 환경 위에 다음 변경만 얹습니다([`interactive_env`], 결정 4·5).
//!
//! 1. 터미널 식별 변수 `TERM`·`COLORTERM`·`TERM_PROGRAM`·`TERM_PROGRAM_VERSION`([`TerminalIdentity`],
//!    spawn 입력). 상속값을 덮어씁니다. emulator가 24-bit 색을 지원하지 않는 identity면 `COLORTERM`을 지웁니다.
//! 2. 기본 셸 profile이면 `SHELL` = 실제로 실행하는 셸 경로. argv profile은 상속값을 그대로 둡니다.
//! 3. profile의 명시 지정([`EnvironmentPolicy`])을 마지막에 적용해 사용자 지정이 이깁니다.
//!
//! 그 밖의 변수는 더하지 않습니다. 결과는 이름 순 변경 목록([`EnvChange`])이고, 실행기가 상속 환경에 적용합니다.
//! 앱이 시작하는 에이전트 CLI 하위 프로세스의 허용 목록 환경은 이 crate가 아니라 agent host가 맡습니다
//! (ADR-0023 결정 9, XXIX.2.4).
//!
//! 이름은 POSIX 이식 문자 집합(`[A-Za-z_][A-Za-z0-9_]*`)만 허용하고, 개수·길이에 상한을 둡니다.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::Path;

use crate::emulator::{COLORTERM, TERM_NAME};

/// profile 명시 지정 항목 수 상한.
pub const MAX_ENV_ENTRIES: usize = 256;
/// 이름 길이 상한(byte).
pub const MAX_ENV_NAME_BYTES: usize = 256;
/// 명시 지정 값 길이 상한(byte).
pub const MAX_ENV_VALUE_BYTES: usize = 32 * 1024;

/// `TERM_PROGRAM`에 쓰는 Stellar 제품 식별자.
pub const TERM_PROGRAM_NAME: &str = "Stellar";

/// Stellar가 대화형 세션에 설정하는 변수 이름 전부(ADR-0023 결정 4). profile 명시 지정은 별도입니다.
pub const STELLAR_SET_VARIABLES: [&str; 5] = [
    "COLORTERM",
    "SHELL",
    "TERM",
    "TERM_PROGRAM",
    "TERM_PROGRAM_VERSION",
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
    /// 값이 [`MAX_ENV_VALUE_BYTES`]를 넘습니다.
    ValueTooLong {
        /// 변수 이름.
        name: EnvName,
        /// 값 길이(byte).
        len: usize,
    },
    /// 값에 NUL byte가 있습니다(exec 환경에 넣을 수 없음).
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

fn checked_value(name: &EnvName, value: OsString) -> Result<OsString, EnvError> {
    let bytes = value.as_encoded_bytes();
    if bytes.len() > MAX_ENV_VALUE_BYTES {
        return Err(EnvError::ValueTooLong {
            name: name.clone(),
            len: bytes.len(),
        });
    }
    if bytes.contains(&0) {
        return Err(EnvError::ValueContainsNul { name: name.clone() });
    }
    Ok(value)
}

/// profile의 환경 변수 명시 지정(아키텍처 §32 "환경 변수 프로필"). 상속 환경과 식별 변수보다 나중에 적용합니다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvironmentPolicy {
    assignments: BTreeMap<EnvName, OsString>,
}

impl EnvironmentPolicy {
    /// 명시 지정이 없는 정책(기본값). child는 상속 환경과 Stellar 식별 변수만 봅니다.
    pub fn empty() -> EnvironmentPolicy {
        EnvironmentPolicy::default()
    }

    /// 이름=값을 명시 지정합니다(같은 이름이 있으면 값을 바꿉니다).
    pub fn set(&mut self, name: &str, value: impl Into<OsString>) -> Result<(), EnvError> {
        let name = EnvName::new(name)?;
        let value = checked_value(&name, value.into())?;
        if !self.assignments.contains_key(&name) && self.assignments.len() >= MAX_ENV_ENTRIES {
            return Err(EnvError::TooManyEntries {
                limit: MAX_ENV_ENTRIES,
            });
        }
        self.assignments.insert(name, value);
        Ok(())
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
}

/// 대화형 세션의 터미널 식별 변수 값(spawn 입력, ADR-0023 결정 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalIdentity {
    term: OsString,
    colorterm: Option<OsString>,
    program: OsString,
    program_version: OsString,
}

impl TerminalIdentity {
    /// 값을 검증해 만듭니다. `colorterm`이 `None`이면 child 환경에서 `COLORTERM`을 지웁니다(24-bit 색 미지원).
    pub fn new(
        term: impl Into<OsString>,
        colorterm: Option<OsString>,
        program: impl Into<OsString>,
        program_version: impl Into<OsString>,
    ) -> Result<TerminalIdentity, EnvError> {
        let check = |name: &str, value: OsString| {
            let name = EnvName::new(name)?;
            checked_value(&name, value)
        };
        Ok(TerminalIdentity {
            term: check("TERM", term.into())?,
            colorterm: colorterm.map(|v| check("COLORTERM", v)).transpose()?,
            program: check("TERM_PROGRAM", program.into())?,
            program_version: check("TERM_PROGRAM_VERSION", program_version.into())?,
        })
    }

    /// 이 crate의 emulator에 맞춘 Stellar 식별 값(XVIII.4.1: `TERM`·`COLORTERM` 상수). `version`은 앱 버전입니다.
    pub fn stellar(version: &str) -> Result<TerminalIdentity, EnvError> {
        TerminalIdentity::new(
            TERM_NAME,
            Some(COLORTERM.into()),
            TERM_PROGRAM_NAME,
            version,
        )
    }

    /// `TERM` 값.
    pub fn term(&self) -> &OsStr {
        &self.term
    }

    /// `COLORTERM` 값(`None`이면 지움).
    pub fn colorterm(&self) -> Option<&OsStr> {
        self.colorterm.as_deref()
    }

    /// `TERM_PROGRAM` 값.
    pub fn program(&self) -> &OsStr {
        &self.program
    }

    /// `TERM_PROGRAM_VERSION` 값.
    pub fn program_version(&self) -> &OsStr {
        &self.program_version
    }
}

/// 상속 환경에 대한 변경 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvChange {
    /// 값을 설정합니다(상속값을 덮어씀).
    Set(OsString),
    /// 상속값을 지웁니다.
    Remove,
}

/// 대화형 세션 child의 환경 변경 목록(이름 순). 적용 순서는 식별 변수 → `SHELL`(기본 셸 profile일 때 `shell`)
/// → profile 명시 지정이며, 뒤의 것이 이깁니다(ADR-0023 결정 4·5).
pub fn interactive_env(
    identity: &TerminalIdentity,
    shell: Option<&Path>,
    profile: &EnvironmentPolicy,
) -> Vec<(OsString, EnvChange)> {
    let mut changes: BTreeMap<OsString, EnvChange> = BTreeMap::new();
    changes.insert("TERM".into(), EnvChange::Set(identity.term.clone()));
    changes.insert(
        "COLORTERM".into(),
        identity
            .colorterm
            .clone()
            .map_or(EnvChange::Remove, EnvChange::Set),
    );
    changes.insert(
        "TERM_PROGRAM".into(),
        EnvChange::Set(identity.program.clone()),
    );
    changes.insert(
        "TERM_PROGRAM_VERSION".into(),
        EnvChange::Set(identity.program_version.clone()),
    );
    if let Some(shell) = shell {
        changes.insert("SHELL".into(), EnvChange::Set(shell.as_os_str().into()));
    }
    for (name, value) in profile.assignments() {
        changes.insert(name.as_str().into(), EnvChange::Set(value.into()));
    }
    changes.into_iter().collect()
}

/// 상속 환경 `base`에 변경을 적용한 최종 환경(이름 순). 실행기는 OS가 같은 일을 하게 하고, 이 함수는 검사용입니다.
pub fn apply_env_changes(
    base: impl IntoIterator<Item = (OsString, OsString)>,
    changes: &[(OsString, EnvChange)],
) -> Vec<(OsString, OsString)> {
    let mut env: BTreeMap<OsString, OsString> = base.into_iter().collect();
    for (name, change) in changes {
        match change {
            EnvChange::Set(value) => {
                env.insert(name.clone(), value.clone());
            }
            EnvChange::Remove => {
                env.remove(name);
            }
        }
    }
    env.into_iter().collect()
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

    fn identity() -> TerminalIdentity {
        TerminalIdentity::stellar("1.2.3").unwrap()
    }

    fn set(value: &str) -> EnvChange {
        EnvChange::Set(value.into())
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
    fn identity_variables_override_inherited_values() {
        // 결정 4: 식별 변수 4개는 spawn 입력 값이고 상속값을 덮어씁니다. 그 밖의 상속 변수는 그대로입니다.
        let changes = interactive_env(&identity(), None, &EnvironmentPolicy::empty());
        assert_eq!(
            changes,
            vec![
                (OsString::from("COLORTERM"), set("truecolor")),
                (OsString::from("TERM"), set("xterm-256color")),
                (OsString::from("TERM_PROGRAM"), set("Stellar")),
                (OsString::from("TERM_PROGRAM_VERSION"), set("1.2.3")),
            ]
        );
        let base = [
            ("TERM", "vt100"),
            ("TERM_PROGRAM", "Apple_Terminal"),
            ("SSH_AUTH_SOCK", "/tmp/agent.sock"),
            ("SHELL", "/bin/zsh"),
        ]
        .map(|(n, v)| (OsString::from(n), OsString::from(v)));
        let env = apply_env_changes(base, &changes);
        let get = |name: &str| {
            env.iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.to_str().unwrap())
        };
        assert_eq!(get("TERM"), Some("xterm-256color"));
        assert_eq!(get("TERM_PROGRAM"), Some("Stellar"));
        assert_eq!(
            get("SSH_AUTH_SOCK"),
            Some("/tmp/agent.sock"),
            "허용 목록 밖 상속 변수"
        );
        assert_eq!(
            get("SHELL"),
            Some("/bin/zsh"),
            "argv profile은 상속 SHELL 유지"
        );
    }

    #[test]
    fn colorterm_is_removed_without_truecolor_support() {
        let plain = TerminalIdentity::new("xterm", None, "Stellar", "1").unwrap();
        let changes = interactive_env(&plain, None, &EnvironmentPolicy::empty());
        assert!(changes.contains(&(OsString::from("COLORTERM"), EnvChange::Remove)));
        let env = apply_env_changes(
            [(OsString::from("COLORTERM"), OsString::from("truecolor"))],
            &changes,
        );
        assert!(env.iter().all(|(name, _)| name != "COLORTERM"));
    }

    #[test]
    fn default_shell_sets_shell_and_profile_assignments_win_last() {
        // 결정 4·5: 기본 셸 profile은 SHELL = 실행한 셸, profile 명시 지정이 식별 변수와 SHELL보다 이깁니다.
        let mut profile = EnvironmentPolicy::empty();
        let changes = interactive_env(&identity(), Some(Path::new("/bin/bash")), &profile);
        assert!(changes.contains(&(OsString::from("SHELL"), set("/bin/bash"))));
        profile.set("TERM", "dumb").unwrap();
        profile.set("SHELL", "/custom/shell").unwrap();
        profile.set("EDITOR", "vi").unwrap();
        let changes = interactive_env(&identity(), Some(Path::new("/bin/bash")), &profile);
        let env = apply_env_changes([], &changes);
        let names: Vec<&str> = env.iter().map(|(n, _)| n.to_str().unwrap()).collect();
        assert_eq!(
            names,
            [
                "COLORTERM",
                "EDITOR",
                "SHELL",
                "TERM",
                "TERM_PROGRAM",
                "TERM_PROGRAM_VERSION"
            ]
        );
        assert!(env.contains(&(OsString::from("TERM"), OsString::from("dumb"))));
        assert!(env.contains(&(OsString::from("SHELL"), OsString::from("/custom/shell"))));
    }

    #[test]
    fn stellar_adds_only_identity_and_shell_variables() {
        // 결정 4: 명시 지정이 없으면 Stellar가 더하는 변수는 식별 변수 4개와 SHELL뿐입니다.
        let changes = interactive_env(
            &identity(),
            Some(Path::new("/bin/zsh")),
            &EnvironmentPolicy::empty(),
        );
        let names: Vec<&str> = changes.iter().map(|(n, _)| n.to_str().unwrap()).collect();
        assert_eq!(names, STELLAR_SET_VARIABLES);
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
        assert!(matches!(
            TerminalIdentity::new("x\0", None, "Stellar", "1"),
            Err(EnvError::ValueContainsNul { .. })
        ));
        for index in 0..MAX_ENV_ENTRIES {
            policy.set(&format!("V{index}"), "1").unwrap();
        }
        assert_eq!(
            policy.set("ONE_MORE", "1"),
            Err(EnvError::TooManyEntries {
                limit: MAX_ENV_ENTRIES
            })
        );
        // 이미 있는 이름의 값 변경은 상한과 무관합니다.
        policy.set("V0", "2").unwrap();
        assert_eq!(policy.assignment("V0"), Some(OsStr::new("2")));
    }
}
