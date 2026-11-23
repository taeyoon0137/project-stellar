//! library 의존성 엔진의 시스템 Git·SHA-256 연결 (XV.3, ADR-0119 결정 1).
//!
//! 의존성 crate(`stellar-dependencies`)는 core-types에만 의존하므로 Git 실행([`GitCommandRunner`])과 SHA-256
//! ([`Sha256Digest`])을 trait으로 받습니다. 이 모듈은 그 trait을 제품 구현에 잇습니다.
//!
//! - [`SystemGitCommands`]: git-porcelain [`GitRunner`](shell 미사용, timeout·환경 정책 포함)로 실행합니다. 실패
//!   사유에 stderr·URL을 담지 않습니다(XV.1.4).
//! - [`StorageSha256`]: 저장 content hash(ADR-0004, [`ContentHash::compute`])와 같은 SHA-256입니다. cache entry
//!   content hash(XV.3.3)와 lock content hash(XV.4.2)가 이 함수로 계산됩니다.

use std::ffi::OsString;
use std::path::Path;

use stellar_dependencies::cache_entry::Sha256Digest;
use stellar_dependencies::git::{GitCommandOutput, GitCommandRunner, GitInvocationError};
use stellar_git_porcelain::runner::{GitOutput, GitRunError, GitRunner};
use stellar_storage::hash::ContentHash;

/// 시스템 Git 실행기.
#[derive(Debug, Clone)]
pub struct SystemGitCommands(pub GitRunner);

fn output(out: GitOutput) -> GitCommandOutput {
    GitCommandOutput {
        exit_code: out.exit_code,
        stdout: out.stdout,
        stderr: out.stderr,
    }
}

fn invocation(error: GitRunError) -> GitInvocationError {
    match error {
        GitRunError::Timeout => GitInvocationError::TimedOut,
        GitRunError::Cancelled => GitInvocationError::Cancelled,
        GitRunError::Spawn { .. } | GitRunError::Wait { .. } => GitInvocationError::Spawn,
    }
}

impl GitCommandRunner for SystemGitCommands {
    fn run_git(
        &mut self,
        args: &[OsString],
        cwd: &Path,
    ) -> Result<GitCommandOutput, GitInvocationError> {
        self.0.run(args, cwd).map(output).map_err(invocation)
    }

    fn run_git_with_input(
        &mut self,
        args: &[OsString],
        cwd: &Path,
        input: &[u8],
    ) -> Result<GitCommandOutput, GitInvocationError> {
        self.0
            .run_with_input(args, cwd, input)
            .map(output)
            .map_err(invocation)
    }
}

/// 저장 content hash와 같은 SHA-256.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StorageSha256;

impl Sha256Digest for StorageSha256 {
    fn sha256(&self, bytes: &[u8]) -> [u8; 32] {
        *ContentHash::compute(bytes).digest()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_storage_content_hash() {
        let bytes = b"stellar-library-entry 1\n";
        assert_eq!(
            StorageSha256.sha256(bytes),
            *ContentHash::compute(bytes).digest()
        );
    }
}
