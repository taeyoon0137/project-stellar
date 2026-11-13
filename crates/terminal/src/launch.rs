//! 실행 요청과 실행기 경계 (XVIII.1.4).
//!
//! registry는 신뢰 gate·cwd·명령·환경을 확인한 뒤 [`LaunchRequest`]를 만들어 [`TerminalLauncher`]에
//! 넘깁니다. 실제 PTY 생성과 child 실행은 플랫폼 실행기가 맡습니다(macOS POSIX PTY: XVIII.2.1).
//! 실행 결과는 [`TerminalProcess`] trait object로 registry 세션에 붙습니다.

use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::state::{StartFailure, TerminalExitStatus};

/// 실행기에 넘기는 검증된 실행 요청.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRequest {
    program: PathBuf,
    args: Vec<OsString>,
    arg0: Option<OsString>,
    cwd: PathBuf,
    env: Vec<(OsString, OsString)>,
}

impl LaunchRequest {
    /// 요청을 만듭니다. 값은 registry가 검증한 profile·cwd·환경에서 옵니다.
    pub fn new(
        program: PathBuf,
        args: Vec<OsString>,
        arg0: Option<OsString>,
        cwd: PathBuf,
        env: Vec<(OsString, OsString)>,
    ) -> LaunchRequest {
        LaunchRequest {
            program,
            args,
            arg0,
            cwd,
            env,
        }
    }

    /// 실행 파일 절대 경로.
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// 인자(`argv[1..]`).
    pub fn args(&self) -> &[OsString] {
        &self.args
    }

    /// `argv[0]` 지정(없으면 프로그램 경로).
    pub fn arg0(&self) -> Option<&OsString> {
        self.arg0.as_ref()
    }

    /// 검증된 canonical cwd.
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// child 환경 전체(허용 목록 적용 결과, 이름 순).
    pub fn env(&self) -> &[(OsString, OsString)] {
        &self.env
    }
}

/// OS 호출 실패(분류만 보관, `Copy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalIoError {
    /// 실패한 연산 이름(고정 문자열).
    pub operation: &'static str,
    /// I/O 오류 종류.
    pub kind: io::ErrorKind,
    /// OS 오류 번호.
    pub os_code: Option<i32>,
}

impl TerminalIoError {
    /// `io::Error`에서 분류를 옮깁니다.
    pub fn from_io(operation: &'static str, err: &io::Error) -> TerminalIoError {
        TerminalIoError {
            operation,
            kind: err.kind(),
            os_code: err.raw_os_error(),
        }
    }

    /// 마지막 OS 오류(`errno`)에서 만듭니다.
    pub fn last_os_error(operation: &'static str) -> TerminalIoError {
        TerminalIoError::from_io(operation, &io::Error::last_os_error())
    }
}

impl fmt::Display for TerminalIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.os_code {
            Some(code) => write!(f, "{} 실패({}, errno {code})", self.operation, self.kind),
            None => write!(f, "{} 실패({})", self.operation, self.kind),
        }
    }
}

impl std::error::Error for TerminalIoError {}

/// 실행 실패.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchError {
    /// PTY 생성 실패.
    Pty(TerminalIoError),
    /// child process 생성 실패.
    Spawn(TerminalIoError),
}

impl LaunchError {
    /// 상태에 기록할 실패 분류.
    pub fn failure(&self) -> StartFailure {
        match self {
            LaunchError::Pty(_) => StartFailure::Pty,
            LaunchError::Spawn(_) => StartFailure::Spawn,
        }
    }
}

impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LaunchError::Pty(err) => write!(f, "PTY를 만들지 못했습니다: {err}"),
            LaunchError::Spawn(err) => write!(f, "터미널 프로세스를 시작하지 못했습니다: {err}"),
        }
    }
}

impl std::error::Error for LaunchError {}

/// 세션 종료 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitReport {
    /// 세션 leader 종료 상태.
    pub status: TerminalExitStatus,
    /// 정리 뒤에도 남은 세션 프로세스 수(정상은 0).
    pub remaining_processes: u32,
}

/// 실행 중인 터미널 child(세션 leader와 그 프로세스 tree).
pub trait TerminalProcess: fmt::Debug + Send {
    /// 세션 leader pid.
    fn pid(&self) -> u32;

    /// 비차단 종료 확인. 끝났으면(필요한 정리까지 마쳤으면) 결과를 돌려줍니다.
    fn poll_exit(&mut self) -> Result<Option<ExitReport>, TerminalIoError>;

    /// 종료를 요청만 하고 기다리지 않습니다(여러 세션을 함께 닫을 때 먼저 모두 요청).
    fn begin_terminate(&mut self);

    /// 프로세스 tree를 종료하고 결과를 돌려줍니다(유예 시간 상한 안에서 차단).
    fn terminate(&mut self) -> Result<ExitReport, TerminalIoError>;
}

/// 터미널 child 실행기.
pub trait TerminalLauncher {
    /// 요청대로 PTY와 child를 만듭니다.
    fn launch(&mut self, request: &LaunchRequest) -> Result<Box<dyn TerminalProcess>, LaunchError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_errors_keep_classification() {
        let err = io::Error::from_raw_os_error(2);
        let classified = TerminalIoError::from_io("open", &err);
        assert_eq!(classified.kind, io::ErrorKind::NotFound);
        assert_eq!(classified.os_code, Some(2));
        assert!(classified.to_string().contains("open"));
        assert_eq!(LaunchError::Pty(classified).failure(), StartFailure::Pty);
        assert_eq!(
            LaunchError::Spawn(classified).failure(),
            StartFailure::Spawn
        );
    }
}
