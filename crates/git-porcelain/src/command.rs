//! Git 명령 결과 (IV.3.4).
//!
//! [`GitCommandResult`]는 Git 실행의 **exit code, stdout, stderr를 모두 보존**합니다. 비영
//! 종료를 자동으로 "문자열 성공"으로 바꾸지 않습니다 — 성공이 필요한 호출자는
//! [`GitCommandResult::require_success`]로 **명시적으로** 실패를 다뤄야 합니다.
//!
//! [`GitOutput`](crate::runner::GitOutput)(원시 캡처)을 감싸 의미 계층을 더합니다.

use std::borrow::Cow;

use crate::runner::GitOutput;

/// Git 명령 결과(exit code·stdout·stderr 보존).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommandResult {
    output: GitOutput,
}

impl GitCommandResult {
    /// 원시 [`GitOutput`]에서 명령 결과를 만듭니다.
    pub fn new(output: GitOutput) -> GitCommandResult {
        GitCommandResult { output }
    }

    /// 종료 코드(시그널 종료면 `None`).
    pub fn exit_code(&self) -> Option<i32> {
        self.output.exit_code
    }

    /// stdout 원시 바이트.
    pub fn stdout(&self) -> &[u8] {
        &self.output.stdout
    }

    /// stderr 원시 바이트.
    pub fn stderr(&self) -> &[u8] {
        &self.output.stderr
    }

    /// 종료 코드가 정확히 0이면 성공.
    pub fn succeeded(&self) -> bool {
        self.output.exit_code == Some(0)
    }

    /// stdout을 UTF-8 lossy 문자열로 봅니다(편의용; 원시 바이트가 정본).
    pub fn stdout_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.output.stdout)
    }

    /// stderr를 UTF-8 lossy 문자열로 봅니다.
    pub fn stderr_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.output.stderr)
    }

    /// 성공이면 결과를, 비영 종료면 [`GitCommandFailure`]를 반환합니다.
    ///
    /// 비영 종료를 자동으로 성공으로 바꾸지 않으므로, 호출자는 실패를 무시할 수 없습니다.
    pub fn require_success(self) -> Result<GitCommandResult, GitCommandFailure> {
        if self.succeeded() {
            Ok(self)
        } else {
            Err(GitCommandFailure {
                exit_code: self.output.exit_code,
                stdout: self.output.stdout,
                stderr: self.output.stderr,
            })
        }
    }
}

impl From<GitOutput> for GitCommandResult {
    fn from(output: GitOutput) -> GitCommandResult {
        GitCommandResult::new(output)
    }
}

/// 비영 종료한 Git 명령의 보존된 실패 정보.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommandFailure {
    /// 종료 코드(시그널 종료면 `None`).
    pub exit_code: Option<i32>,
    /// stdout 원시 바이트(보존).
    pub stdout: Vec<u8>,
    /// stderr 원시 바이트(보존).
    pub stderr: Vec<u8>,
}

impl std::fmt::Display for GitCommandFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Git 명령 비영 종료(code={:?}): {}",
            self.exit_code,
            String::from_utf8_lossy(&self.stderr).trim()
        )
    }
}

impl std::error::Error for GitCommandFailure {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::{discover_git, GitDiscoveryConfig};
    use crate::runner::{argv, GitRunner};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn temp_dir() -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("stellar-cmd-test-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn result(exit_code: Option<i32>, stdout: &[u8], stderr: &[u8]) -> GitCommandResult {
        GitCommandResult::new(GitOutput {
            exit_code,
            stdout: stdout.to_vec(),
            stderr: stderr.to_vec(),
        })
    }

    #[test]
    fn preserves_all_fields() {
        let r = result(Some(3), b"out", b"err");
        assert_eq!(r.exit_code(), Some(3));
        assert_eq!(r.stdout(), b"out");
        assert_eq!(r.stderr(), b"err");
        assert_eq!(r.stdout_lossy(), "out");
        assert_eq!(r.stderr_lossy(), "err");
    }

    #[test]
    fn nonzero_exit_is_not_auto_success() {
        let r = result(Some(1), b"some output", b"fatal: boom");
        assert!(!r.succeeded());
        // require_success는 비영 종료를 실패로 반환(자동 성공 위장 없음).
        let failure = r.require_success().unwrap_err();
        assert_eq!(failure.exit_code, Some(1));
        assert_eq!(failure.stdout, b"some output");
        assert_eq!(failure.stderr, b"fatal: boom");
    }

    #[test]
    fn zero_exit_is_success() {
        let r = result(Some(0), b"ok", b"");
        assert!(r.succeeded());
        let ok = r.require_success().expect("성공");
        assert_eq!(ok.stdout(), b"ok");
    }

    #[test]
    fn signal_termination_is_not_success() {
        // 시그널 종료(code None)는 성공이 아님.
        let r = result(None, b"", b"");
        assert!(!r.succeeded());
        assert!(r.require_success().is_err());
    }

    #[test]
    fn known_failing_git_command_is_preserved_as_failure() {
        // 비-repo 디렉터리에서 `git rev-parse --verify HEAD`는 실패(코드·stderr 보존).
        let Some(git) = discover_git(&GitDiscoveryConfig::default()) else {
            return; // git 미설치 환경에서는 건너뜀.
        };
        let dir = temp_dir();
        let runner = GitRunner::new(git, Vec::new(), Duration::from_secs(10));
        let output = runner
            .run(&argv(["rev-parse", "--verify", "HEAD"]), &dir)
            .expect("실행");
        let result = GitCommandResult::from(output);
        // 자동 성공으로 바뀌지 않고 실패로 보존.
        assert!(!result.succeeded(), "비-repo에서 성공으로 위장됨");
        let failure = result.require_success().unwrap_err();
        assert_ne!(failure.exit_code, Some(0));
        assert!(
            !failure.stderr.is_empty(),
            "stderr가 보존되어야 함: {}",
            String::from_utf8_lossy(&failure.stderr)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
