//! 시스템 Git 실행 경계 (XV.2.4).
//!
//! 이 crate는 Git 객체를 직접 파싱하지 않고 시스템 Git 명령의 출력만 해석합니다(아키텍처 24.1).
//! 실제 실행기는 project crate가 git-porcelain `GitRunner`(hermetic env·timeout·취소)로 이
//! trait을 구현합니다. 출력의 stderr에는 URL이 섞일 수 있으므로 `Debug`는 길이만 보여 줍니다.

use std::ffi::OsString;
use std::fmt;
use std::path::Path;

/// Git 명령 결과.
#[derive(Clone, PartialEq, Eq)]
pub struct GitCommandOutput {
    /// 종료 코드(시그널 종료면 `None`).
    pub exit_code: Option<i32>,
    /// stdout bytes.
    pub stdout: Vec<u8>,
    /// stderr bytes(진단 노출 금지 — URL 포함 가능).
    pub stderr: Vec<u8>,
}

impl GitCommandOutput {
    /// 종료 코드 0인지.
    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
    }
}

impl fmt::Debug for GitCommandOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitCommandOutput")
            .field("exit_code", &self.exit_code)
            .field("stdout_len", &self.stdout.len())
            .field("stderr_len", &self.stderr.len())
            .finish()
    }
}

/// Git 프로세스를 실행하지 못함.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitInvocationError {
    /// 실행 파일을 시작하지 못함.
    Spawn,
    /// 제한 시간 초과.
    TimedOut,
    /// 사용자 취소.
    Cancelled,
}

impl fmt::Display for GitInvocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GitInvocationError::Spawn => f.write_str("Git을 실행하지 못했습니다"),
            GitInvocationError::TimedOut => f.write_str("Git 실행 제한 시간을 넘었습니다"),
            GitInvocationError::Cancelled => f.write_str("Git 실행이 취소되었습니다"),
        }
    }
}

impl std::error::Error for GitInvocationError {}

/// 시스템 Git 실행기.
pub trait GitCommandRunner {
    /// `git <args>`를 `cwd`에서 실행합니다(shell을 거치지 않음).
    fn run_git(
        &mut self,
        args: &[OsString],
        cwd: &Path,
    ) -> Result<GitCommandOutput, GitInvocationError>;
}

/// 문자열 목록을 인자 vector로 바꿉니다.
pub fn git_args<I, S>(args: I) -> Vec<OsString>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    args.into_iter().map(Into::into).collect()
}
