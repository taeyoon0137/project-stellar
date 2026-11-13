//! 저장소 연산 공통 실행·오류 (XIV.2.1~).
//!
//! 기존 저장소에 대한 읽기·쓰기 연산은 [`GatedGitRunner`]로만 실행합니다(trust gate, argv, 환경
//! allowlist, timeout). 성공(종료 코드 0)을 요구하는 실행과 그 실패를 [`GitOpError`] 하나로
//! 표현해 모듈마다 같은 오류 구조를 씁니다. 비영 종료는 [`GitCommandFailure`]로 Git 결과를 그대로
//! 보존합니다.

use std::ffi::OsString;
use std::path::Path;

use crate::command::GitCommandFailure;
use crate::runner::GitOutput;
use crate::trust::{GatedGitRunner, GatedRunError};

/// 출력 해석 실패(원시 출력 일부 보존).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseFailure {
    /// 해석한 명령 종류(예: `status --porcelain=v2`).
    pub command: &'static str,
    /// 실패 설명.
    pub detail: String,
    /// 문제 레코드 원시 바이트.
    pub raw: Vec<u8>,
}

impl ParseFailure {
    /// 실패를 만듭니다.
    pub fn new(command: &'static str, detail: impl Into<String>, raw: &[u8]) -> ParseFailure {
        ParseFailure {
            command,
            detail: detail.into(),
            raw: raw.to_vec(),
        }
    }
}

impl std::fmt::Display for ParseFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "git {} 출력 해석 실패: {} ({} bytes)",
            self.command,
            self.detail,
            self.raw.len()
        )
    }
}

impl std::error::Error for ParseFailure {}

/// 저장소 연산 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitOpError {
    /// trust gate 거부 또는 Git 실행 실패(spawn·timeout·취소).
    Gate(GatedRunError),
    /// Git 명령 비영 종료(Git 결과 보존).
    Git(GitCommandFailure),
    /// Git 출력 해석 실패.
    Parse(ParseFailure),
}

impl std::fmt::Display for GitOpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitOpError::Gate(e) => write!(f, "{e}"),
            GitOpError::Git(e) => write!(f, "{e}"),
            GitOpError::Parse(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for GitOpError {}

impl From<GatedRunError> for GitOpError {
    fn from(e: GatedRunError) -> GitOpError {
        GitOpError::Gate(e)
    }
}

impl From<ParseFailure> for GitOpError {
    fn from(e: ParseFailure) -> GitOpError {
        GitOpError::Parse(e)
    }
}

/// gate를 거쳐 실행하고 성공 출력을 반환합니다(비영 종료는 [`GitOpError::Git`]).
pub(crate) fn run_ok(
    git: &GatedGitRunner,
    cwd: &Path,
    args: &[OsString],
) -> Result<GitOutput, GitOpError> {
    require_success(git.run(args, cwd)?)
}

/// gate를 거쳐 stdin 입력과 함께 실행하고 성공 출력을 반환합니다(비영 종료는 [`GitOpError::Git`]).
pub(crate) fn run_ok_with_input(
    git: &GatedGitRunner,
    cwd: &Path,
    args: &[OsString],
    input: &[u8],
) -> Result<GitOutput, GitOpError> {
    require_success(git.run_with_input(args, cwd, input)?)
}

/// 종료 코드 0을 요구합니다.
pub(crate) fn require_success(output: GitOutput) -> Result<GitOutput, GitOpError> {
    if output.success() {
        Ok(output)
    } else {
        Err(GitOpError::Git(failure_of(output)))
    }
}

/// 비영 종료 출력을 [`GitCommandFailure`]로 만듭니다.
pub(crate) fn failure_of(output: GitOutput) -> GitCommandFailure {
    GitCommandFailure {
        exit_code: output.exit_code,
        stdout: output.stdout,
        stderr: output.stderr,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_success_preserves_failure() {
        let out = GitOutput {
            exit_code: Some(128),
            stdout: b"o".to_vec(),
            stderr: b"fatal: x".to_vec(),
        };
        match require_success(out) {
            Err(GitOpError::Git(f)) => {
                assert_eq!(f.exit_code, Some(128));
                assert_eq!(f.stderr, b"fatal: x");
            }
            other => panic!("{other:?}"),
        }
        let ok = GitOutput {
            exit_code: Some(0),
            stdout: b"ok".to_vec(),
            stderr: Vec::new(),
        };
        assert_eq!(require_success(ok).unwrap().stdout, b"ok");
    }

    #[test]
    fn parse_failure_display_mentions_command() {
        let e = GitOpError::from(ParseFailure::new("ls-tree", "bad mode", b"x"));
        assert!(e.to_string().contains("ls-tree"));
    }
}
