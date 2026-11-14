//! 터미널 profile (XVIII.1.1).
//!
//! profile은 "무엇을, 어디서, 어떤 환경으로" 실행할지를 정합니다(기술 아키텍처 32 "셸, 환경 변수,
//! 시작 명령, 작업 디렉터리 프로필").
//!
//! - 명령은 **argv 배열**입니다([`ArgvCommand`]). 셸 문자열을 저장하거나 셸로 해석하지 않으므로
//!   공백·따옴표가 든 인자도 그대로 전달됩니다. 프로그램은 절대 경로만 허용합니다(`PATH` 검색으로
//!   다른 실행 파일이 선택되지 않게).
//! - profile은 사용자 설정 값입니다. 프로젝트 파일·복구 snapshot에 넣지 않습니다.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::cwd::CwdSelection;
use crate::env::EnvironmentPolicy;

/// profile 이름 길이 상한(문자).
pub const MAX_PROFILE_NAME_CHARS: usize = 64;
/// 인자 개수 상한(프로그램 제외).
pub const MAX_ARGS: usize = 256;
/// 인자 하나·프로그램 경로의 길이 상한(byte).
pub const MAX_ARG_BYTES: usize = 32 * 1024;

/// profile 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    /// 이름이 비었습니다(공백만 있는 경우 포함).
    EmptyName,
    /// 이름이 [`MAX_PROFILE_NAME_CHARS`]를 넘습니다.
    NameTooLong {
        /// 문자 수.
        chars: usize,
    },
    /// 이름에 제어 문자가 있습니다.
    NameHasControl,
    /// 프로그램 경로가 절대 경로가 아닙니다.
    ProgramNotAbsolute {
        /// 입력 경로.
        program: PathBuf,
    },
    /// 프로그램 경로 또는 인자에 NUL byte가 있습니다(`index`: `None`이면 프로그램).
    ContainsNul {
        /// 인자 위치.
        index: Option<usize>,
    },
    /// 인자가 [`MAX_ARGS`]개를 넘습니다.
    TooManyArgs {
        /// 인자 수.
        count: usize,
    },
    /// 프로그램 경로 또는 인자가 [`MAX_ARG_BYTES`]를 넘습니다(`index`: `None`이면 프로그램).
    ArgTooLong {
        /// 인자 위치.
        index: Option<usize>,
        /// 길이(byte).
        len: usize,
    },
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProfileError::EmptyName => f.write_str("profile 이름이 비었습니다"),
            ProfileError::NameTooLong { chars } => write!(
                f,
                "profile 이름이 너무 깁니다({chars}자, 상한 {MAX_PROFILE_NAME_CHARS})"
            ),
            ProfileError::NameHasControl => f.write_str("profile 이름에 제어 문자가 있습니다"),
            ProfileError::ProgramNotAbsolute { program } => {
                write!(f, "실행 파일은 절대 경로여야 합니다: {}", program.display())
            }
            ProfileError::ContainsNul { index: None } => {
                f.write_str("실행 파일 경로에 NUL 문자가 있습니다")
            }
            ProfileError::ContainsNul { index: Some(index) } => {
                write!(f, "{index}번째 인자에 NUL 문자가 있습니다")
            }
            ProfileError::TooManyArgs { count } => {
                write!(f, "인자가 너무 많습니다({count}개, 상한 {MAX_ARGS})")
            }
            ProfileError::ArgTooLong { index: None, len } => {
                write!(f, "실행 파일 경로가 너무 깁니다({len} byte)")
            }
            ProfileError::ArgTooLong {
                index: Some(index),
                len,
            } => write!(f, "{index}번째 인자가 너무 깁니다({len} byte)"),
        }
    }
}

impl std::error::Error for ProfileError {}

/// 검증된 profile 이름(앞뒤 공백 제거).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProfileName(String);

impl ProfileName {
    /// 이름을 검증합니다.
    pub fn new(name: &str) -> Result<ProfileName, ProfileError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(ProfileError::EmptyName);
        }
        let chars = trimmed.chars().count();
        if chars > MAX_PROFILE_NAME_CHARS {
            return Err(ProfileError::NameTooLong { chars });
        }
        if trimmed.chars().any(char::is_control) {
            return Err(ProfileError::NameHasControl);
        }
        Ok(ProfileName(trimmed.to_owned()))
    }

    /// 이름 문자열.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProfileName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// argv 배열 명령: 절대 경로 프로그램 + 인자(`argv[1..]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgvCommand {
    program: PathBuf,
    args: Vec<OsString>,
}

impl ArgvCommand {
    /// 프로그램과 인자를 검증합니다.
    pub fn new<I, S>(program: impl Into<PathBuf>, args: I) -> Result<ArgvCommand, ProfileError>
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        let program = program.into();
        check_component(program.as_os_str(), None)?;
        if !program.is_absolute() {
            return Err(ProfileError::ProgramNotAbsolute { program });
        }
        let mut collected = Vec::new();
        for (index, arg) in args.into_iter().enumerate() {
            if index >= MAX_ARGS {
                return Err(ProfileError::TooManyArgs { count: index + 1 });
            }
            let arg = arg.into();
            check_component(&arg, Some(index))?;
            collected.push(arg);
        }
        Ok(ArgvCommand {
            program,
            args: collected,
        })
    }

    /// 실행 파일 절대 경로.
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// 인자(`argv[1..]`).
    pub fn args(&self) -> &[OsString] {
        &self.args
    }
}

fn check_component(value: &OsStr, index: Option<usize>) -> Result<(), ProfileError> {
    let bytes = value.as_encoded_bytes();
    if bytes.len() > MAX_ARG_BYTES {
        return Err(ProfileError::ArgTooLong {
            index,
            len: bytes.len(),
        });
    }
    if bytes.contains(&0) {
        return Err(ProfileError::ContainsNul { index });
    }
    Ok(())
}

/// profile이 실행할 명령.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileCommand {
    /// 설정에서 고른 사용자 셸(XVIII.2.2).
    DefaultShell,
    /// 지정 argv 명령. 외부 CLI 에이전트도 이 일반 명령으로 실행합니다(XVIII.5.1, `program` 모듈).
    Argv(ArgvCommand),
}

/// 터미널 profile. 사용자 설정 값이며 세션을 만들 때 복사됩니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalProfile {
    /// 표시 이름.
    pub name: ProfileName,
    /// 실행 명령.
    pub command: ProfileCommand,
    /// child 환경 정책.
    pub environment: EnvironmentPolicy,
    /// 작업 디렉터리 선택.
    pub cwd: CwdSelection,
}

impl TerminalProfile {
    /// 기본 셸·기본 환경·프로젝트 루트 cwd profile.
    pub fn default_shell(name: ProfileName) -> TerminalProfile {
        TerminalProfile {
            name,
            command: ProfileCommand::DefaultShell,
            environment: EnvironmentPolicy::with_defaults(),
            cwd: CwdSelection::ProjectRoot,
        }
    }

    /// argv 명령·기본 환경·프로젝트 루트 cwd profile. 외부 CLI 에이전트를 포함한 임의 명령에 쓰며
    /// 명령 종류에 따라 다르게 다루지 않습니다(XVIII.5.1).
    pub fn command(name: ProfileName, command: ArgvCommand) -> TerminalProfile {
        TerminalProfile {
            name,
            command: ProfileCommand::Argv(command),
            environment: EnvironmentPolicy::with_defaults(),
            cwd: CwdSelection::ProjectRoot,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_names_are_validated() {
        assert_eq!(ProfileName::new("  zsh  ").unwrap().as_str(), "zsh");
        assert_eq!(ProfileName::new("터미널 1").unwrap().as_str(), "터미널 1");
        assert_eq!(ProfileName::new("   "), Err(ProfileError::EmptyName));
        assert_eq!(
            ProfileName::new("a\u{7}b"),
            Err(ProfileError::NameHasControl)
        );
        let long = "가".repeat(MAX_PROFILE_NAME_CHARS + 1);
        assert_eq!(
            ProfileName::new(&long),
            Err(ProfileError::NameTooLong {
                chars: MAX_PROFILE_NAME_CHARS + 1
            })
        );
    }

    #[test]
    fn argv_keeps_arguments_verbatim_without_shell_parsing() {
        let command =
            ArgvCommand::new("/bin/echo", ["a b", "'quoted'", "$HOME", "; rm -rf /"]).unwrap();
        assert_eq!(command.program(), Path::new("/bin/echo"));
        let args: Vec<&OsStr> = command.args().iter().map(OsString::as_os_str).collect();
        assert_eq!(args, ["a b", "'quoted'", "$HOME", "; rm -rf /"]);
    }

    #[test]
    fn argv_rejects_relative_programs_nul_and_oversized_input() {
        assert!(matches!(
            ArgvCommand::new("echo", Vec::<String>::new()),
            Err(ProfileError::ProgramNotAbsolute { .. })
        ));
        assert_eq!(
            ArgvCommand::new("/bin/e\0cho", Vec::<String>::new()),
            Err(ProfileError::ContainsNul { index: None })
        );
        assert_eq!(
            ArgvCommand::new("/bin/echo", ["ok", "b\0ad"]),
            Err(ProfileError::ContainsNul { index: Some(1) })
        );
        let many = vec!["x"; MAX_ARGS + 1];
        assert_eq!(
            ArgvCommand::new("/bin/echo", many),
            Err(ProfileError::TooManyArgs {
                count: MAX_ARGS + 1
            })
        );
        assert!(ArgvCommand::new("/bin/echo", vec!["x"; MAX_ARGS]).is_ok());
        let huge = "x".repeat(MAX_ARG_BYTES + 1);
        assert_eq!(
            ArgvCommand::new("/bin/echo", [huge]),
            Err(ProfileError::ArgTooLong {
                index: Some(0),
                len: MAX_ARG_BYTES + 1
            })
        );
    }

    #[test]
    fn command_profile_is_a_plain_argv_profile() {
        let command = ArgvCommand::new("/usr/local/bin/agent", ["--flag", "two words"]).unwrap();
        let profile = TerminalProfile::command(ProfileName::new("agent").unwrap(), command.clone());
        assert_eq!(profile.command, ProfileCommand::Argv(command));
        assert_eq!(profile.cwd, CwdSelection::ProjectRoot);
        assert_eq!(profile.environment, EnvironmentPolicy::with_defaults());
    }

    #[test]
    fn default_profile_uses_shell_defaults_and_project_root() {
        let profile = TerminalProfile::default_shell(ProfileName::new("기본").unwrap());
        assert_eq!(profile.command, ProfileCommand::DefaultShell);
        assert_eq!(profile.cwd, CwdSelection::ProjectRoot);
        assert_eq!(profile.environment, EnvironmentPolicy::with_defaults());
    }
}
