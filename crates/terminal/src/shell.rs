//! 사용자 셸 설정과 login/non-login 실행 (XVIII.2.2).
//!
//! 설정([`ShellSettings`])은 앱 사용자 설정 값입니다(프로젝트 파일에 넣지 않음).
//!
//! - **셸 선택**: [`ShellChoice::UserDefault`]는 계정 데이터베이스의 로그인 셸(macOS
//!   `getpwuid_r`의 `pw_shell`)을 먼저 쓰고, 없거나 쓸 수 없으면 앱 환경의 `SHELL`을 씁니다. 둘 다
//!   쓸 수 없으면 [`ShellError::NoDefaultShell`]입니다. [`ShellChoice::Custom`]은 지정 경로만 씁니다.
//! - **검증**: 절대 경로, NUL 없음, 길이 상한, 존재, 일반 파일, 실행 권한. 실패하면 child를 만들지
//!   않고 경로와 이유를 담은 오류를 돌려줍니다.
//! - **login/non-login**: login 셸은 관례대로 `argv[0]`을 `-` + 파일 이름(예: `-zsh`)으로 줍니다.
//!   non-login 셸은 파일 이름 그대로입니다. 셸 문자열이나 `-c`를 쓰지 않습니다.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::profile::MAX_ARG_BYTES;

/// 셸 선택.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellChoice {
    /// 사용자 기본 셸(계정 로그인 셸, 없으면 `SHELL`).
    UserDefault,
    /// 지정 경로.
    Custom(PathBuf),
}

/// login 여부.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellMode {
    /// login 셸(`argv[0]`이 `-`로 시작, profile 파일을 읽음).
    Login,
    /// non-login 대화형 셸.
    NonLogin,
}

/// 터미널 셸 설정. 기본값은 사용자 기본 셸 + login(macOS 터미널 관례).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellSettings {
    /// 셸 선택.
    pub choice: ShellChoice,
    /// login 여부.
    pub mode: ShellMode,
}

impl Default for ShellSettings {
    fn default() -> Self {
        ShellSettings {
            choice: ShellChoice::UserDefault,
            mode: ShellMode::Login,
        }
    }
}

/// 셸 경로가 쓸 수 없는 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellPathProblem {
    /// 절대 경로가 아님.
    NotAbsolute,
    /// NUL byte가 있거나 너무 김.
    Malformed,
    /// 경로가 없음.
    NotFound,
    /// 일반 파일이 아님(디렉터리 등).
    NotAFile,
    /// 실행 권한이 없음.
    NotExecutable,
}

impl ShellPathProblem {
    /// 고정 코드 문자열.
    pub fn code(self) -> &'static str {
        match self {
            ShellPathProblem::NotAbsolute => "not-absolute",
            ShellPathProblem::Malformed => "malformed",
            ShellPathProblem::NotFound => "not-found",
            ShellPathProblem::NotAFile => "not-a-file",
            ShellPathProblem::NotExecutable => "not-executable",
        }
    }
}

/// 셸 해석 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellError {
    /// 지정한 셸 경로를 쓸 수 없습니다.
    InvalidShell {
        /// 지정 경로.
        path: PathBuf,
        /// 이유.
        problem: ShellPathProblem,
    },
    /// 사용자 기본 셸을 찾지 못했습니다(계정 셸·`SHELL` 모두 없거나 쓸 수 없음).
    NoDefaultShell,
}

impl fmt::Display for ShellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShellError::InvalidShell { path, problem } => {
                let reason = match problem {
                    ShellPathProblem::NotAbsolute => "절대 경로가 아닙니다",
                    ShellPathProblem::Malformed => "경로 형식이 올바르지 않습니다",
                    ShellPathProblem::NotFound => "파일이 없습니다",
                    ShellPathProblem::NotAFile => "일반 파일이 아닙니다",
                    ShellPathProblem::NotExecutable => "실행 권한이 없습니다",
                };
                write!(f, "셸을 실행할 수 없습니다({reason}): {}", path.display())
            }
            ShellError::NoDefaultShell => f.write_str("사용자 기본 셸을 찾지 못했습니다"),
        }
    }
}

impl std::error::Error for ShellError {}

/// 실행할 셸.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedShell {
    program: PathBuf,
    mode: ShellMode,
}

impl ResolvedShell {
    /// 셸 실행 파일 절대 경로.
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// login 여부.
    pub fn mode(&self) -> ShellMode {
        self.mode
    }

    /// `argv[0]`: login이면 `-` + 파일 이름, 아니면 파일 이름.
    pub fn arg0(&self) -> OsString {
        let name = self.program.file_name().unwrap_or(self.program.as_os_str());
        match self.mode {
            ShellMode::Login => {
                let mut arg0 = OsString::from("-");
                arg0.push(name);
                arg0
            }
            ShellMode::NonLogin => name.to_os_string(),
        }
    }
}

/// 사용자 기본 셸 후보 공급자(테스트에서 바꿀 수 있게 분리).
pub trait UserShellSource: fmt::Debug {
    /// 계정 데이터베이스의 로그인 셸.
    fn account_shell(&self) -> Option<PathBuf>;
    /// 앱 환경의 `SHELL`.
    fn env_shell(&self) -> Option<PathBuf>;
}

/// 실제 시스템 공급자(macOS `getpwuid_r`, 앱 환경 `SHELL`).
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemShellSource;

impl UserShellSource for SystemShellSource {
    fn account_shell(&self) -> Option<PathBuf> {
        platform::account_shell()
    }

    fn env_shell(&self) -> Option<PathBuf> {
        std::env::var_os("SHELL")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    }
}

/// 설정을 실행할 셸로 해석합니다.
pub fn resolve_shell(
    settings: &ShellSettings,
    source: &dyn UserShellSource,
) -> Result<ResolvedShell, ShellError> {
    let program = match &settings.choice {
        ShellChoice::Custom(path) => {
            check_shell_path(path).map_err(|problem| ShellError::InvalidShell {
                path: path.clone(),
                problem,
            })?;
            path.clone()
        }
        ShellChoice::UserDefault => [source.account_shell(), source.env_shell()]
            .into_iter()
            .flatten()
            .find(|path| check_shell_path(path).is_ok())
            .ok_or(ShellError::NoDefaultShell)?,
    };
    Ok(ResolvedShell {
        program,
        mode: settings.mode,
    })
}

/// 셸 경로를 검사합니다.
fn check_shell_path(path: &Path) -> Result<(), ShellPathProblem> {
    let bytes = path.as_os_str().as_encoded_bytes();
    if bytes.is_empty() || bytes.len() > MAX_ARG_BYTES || bytes.contains(&0) {
        return Err(ShellPathProblem::Malformed);
    }
    if !path.is_absolute() {
        return Err(ShellPathProblem::NotAbsolute);
    }
    let metadata = std::fs::metadata(path).map_err(|_| ShellPathProblem::NotFound)?;
    if !metadata.is_file() {
        return Err(ShellPathProblem::NotAFile);
    }
    if !is_executable(&metadata) {
        return Err(ShellPathProblem::NotExecutable);
    }
    Ok(())
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &std::fs::Metadata) -> bool {
    true
}

fn os_str_from_bytes(bytes: &[u8]) -> Option<&OsStr> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Some(OsStr::from_bytes(bytes))
    }
    #[cfg(not(unix))]
    {
        std::str::from_utf8(bytes).ok().map(OsStr::new)
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::{c_char, c_int, CStr};
    use std::path::PathBuf;

    /// `struct passwd`(macOS `pwd.h`).
    #[repr(C)]
    struct Passwd {
        pw_name: *mut c_char,
        pw_passwd: *mut c_char,
        pw_uid: u32,
        pw_gid: u32,
        pw_change: i64,
        pw_class: *mut c_char,
        pw_gecos: *mut c_char,
        pw_dir: *mut c_char,
        pw_shell: *mut c_char,
        pw_expire: i64,
    }

    extern "C" {
        fn getuid() -> u32;
        fn getpwuid_r(
            uid: u32,
            pwd: *mut Passwd,
            buffer: *mut c_char,
            bufsize: usize,
            result: *mut *mut Passwd,
        ) -> c_int;
    }

    /// `ERANGE`(macOS).
    const ERANGE: c_int = 34;
    /// 문자열 buffer 상한.
    const MAX_BUFFER: usize = 64 * 1024;

    pub(super) fn account_shell() -> Option<PathBuf> {
        let mut size = 1024;
        loop {
            let mut buffer = vec![0 as c_char; size];
            // SAFETY: Passwd는 포인터와 정수만 가진 repr(C) 구조체라 0으로 채운 값이 유효합니다.
            let mut passwd: Passwd = unsafe { std::mem::zeroed() };
            let mut result: *mut Passwd = std::ptr::null_mut();
            // SAFETY: passwd·buffer·result는 호출 동안 살아 있고 buffer 길이를 함께 넘깁니다.
            // getpwuid_r는 문자열을 buffer 안에 쓰고 passwd의 포인터가 그곳을 가리키게 합니다.
            let rc = unsafe {
                getpwuid_r(
                    getuid(),
                    &mut passwd,
                    buffer.as_mut_ptr(),
                    buffer.len(),
                    &mut result,
                )
            };
            if rc == ERANGE && size < MAX_BUFFER {
                size *= 4;
                continue;
            }
            if rc != 0 || result.is_null() || passwd.pw_shell.is_null() {
                return None;
            }
            // SAFETY: 성공 시 pw_shell은 buffer 안의 NUL로 끝나는 문자열을 가리키고, buffer는 아직
            // 살아 있습니다. 바로 복사합니다.
            let shell = unsafe { CStr::from_ptr(passwd.pw_shell) };
            let shell = super::os_str_from_bytes(shell.to_bytes())?;
            if shell.is_empty() {
                return None;
            }
            return Some(PathBuf::from(shell));
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use std::path::PathBuf;

    pub(super) fn account_shell() -> Option<PathBuf> {
        None
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::test_support::TempDir;

    /// 고정 후보를 돌려주는 공급자.
    #[derive(Debug, Default)]
    pub(crate) struct FixedShellSource {
        pub(crate) account: Option<PathBuf>,
        pub(crate) env: Option<PathBuf>,
    }

    impl UserShellSource for FixedShellSource {
        fn account_shell(&self) -> Option<PathBuf> {
            self.account.clone()
        }

        fn env_shell(&self) -> Option<PathBuf> {
            self.env.clone()
        }
    }

    fn custom(path: impl Into<PathBuf>, mode: ShellMode) -> ShellSettings {
        ShellSettings {
            choice: ShellChoice::Custom(path.into()),
            mode,
        }
    }

    #[test]
    fn login_mode_prefixes_argv0_with_dash() {
        let source = FixedShellSource::default();
        let login = resolve_shell(&custom("/bin/sh", ShellMode::Login), &source).unwrap();
        assert_eq!(login.program(), Path::new("/bin/sh"));
        assert_eq!(login.arg0(), OsString::from("-sh"));
        let plain = resolve_shell(&custom("/bin/sh", ShellMode::NonLogin), &source).unwrap();
        assert_eq!(plain.arg0(), OsString::from("sh"));
        assert_eq!(plain.mode(), ShellMode::NonLogin);
        assert_eq!(ShellSettings::default().mode, ShellMode::Login);
    }

    #[test]
    fn missing_or_unusable_custom_shell_is_reported() {
        let dir = TempDir::new();
        let source = FixedShellSource::default();
        let missing = dir.path().join("no-such-shell");
        let cases = [
            (missing.clone(), ShellPathProblem::NotFound),
            (PathBuf::from("bin/zsh"), ShellPathProblem::NotAbsolute),
            (PathBuf::from("/bin/z\0sh"), ShellPathProblem::Malformed),
            (dir.path().to_path_buf(), ShellPathProblem::NotAFile),
        ];
        for (path, problem) in cases {
            assert_eq!(
                resolve_shell(&custom(&path, ShellMode::Login), &source),
                Err(ShellError::InvalidShell {
                    path: path.clone(),
                    problem
                }),
                "{path:?}"
            );
        }
        let not_executable = dir.path().join("plain-file");
        std::fs::write(&not_executable, b"#!/bin/sh\n").unwrap();
        assert_eq!(
            resolve_shell(&custom(&not_executable, ShellMode::Login), &source),
            Err(ShellError::InvalidShell {
                path: not_executable,
                problem: ShellPathProblem::NotExecutable
            })
        );
        let err = resolve_shell(&custom(&missing, ShellMode::Login), &source).unwrap_err();
        assert!(err.to_string().contains("파일이 없습니다"), "{err}");
        assert_eq!(ShellPathProblem::NotFound.code(), "not-found");
    }

    #[test]
    fn user_default_prefers_account_shell_then_env_shell() {
        let settings = ShellSettings::default();
        let both = FixedShellSource {
            account: Some(PathBuf::from("/bin/sh")),
            env: Some(PathBuf::from("/bin/zsh")),
        };
        assert_eq!(
            resolve_shell(&settings, &both).unwrap().program(),
            Path::new("/bin/sh")
        );
        // 계정 셸이 없거나 쓸 수 없으면 SHELL을 씁니다.
        let broken_account = FixedShellSource {
            account: Some(PathBuf::from("/nonexistent/shell")),
            env: Some(PathBuf::from("/bin/zsh")),
        };
        assert_eq!(
            resolve_shell(&settings, &broken_account).unwrap().program(),
            Path::new("/bin/zsh")
        );
        // 둘 다 없거나 쓸 수 없으면 오류입니다.
        for source in [
            FixedShellSource::default(),
            FixedShellSource {
                account: Some(PathBuf::from("/nonexistent/a")),
                env: Some(PathBuf::from("relative/b")),
            },
        ] {
            assert_eq!(
                resolve_shell(&settings, &source),
                Err(ShellError::NoDefaultShell)
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn system_source_reads_account_login_shell() {
        // DEV-MAC: 로그인 계정에는 절대 경로 셸이 있습니다.
        let shell = SystemShellSource.account_shell().unwrap();
        assert!(shell.is_absolute(), "{shell:?}");
        assert!(resolve_shell(&ShellSettings::default(), &SystemShellSource).is_ok());
    }
}
