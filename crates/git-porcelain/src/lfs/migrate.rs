//! 기존 Git 이력의 LFS 변환 미지원 (XIII.7.4).
//!
//! `git lfs migrate import/export`는 commit을 다시 써서 이력(commit ID)을 바꿉니다. Stellar는 기존 이력을
//! 자동·수동 어느 쪽으로도 마이그레이션하지 않습니다(아키텍처 29.2). 요청 경로는 타입으로 남겨 두되
//! [`migrate_lfs_history`]는 항상 [`UnsupportedOperation`]을 반환하며 Git 프로세스를 만들지 않습니다.
//! 성공 타입이 [`Infallible`]이라 성공 값을 만들 수 없습니다.
//!
//! LFS 추적은 승인 후 `git lfs track`(XIII.7.3)으로 **앞으로의 commit**에만 적용합니다.

use std::convert::Infallible;
use std::path::Path;

use crate::trust::GatedGitRunner;

/// 이력 변환 방향.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LfsMigrationMode {
    /// 기존 blob을 LFS pointer로 변환(`git lfs migrate import`).
    Import,
    /// LFS pointer를 일반 blob으로 되돌림(`git lfs migrate export`).
    Export,
}

/// 이력 변환 요청(지원하지 않음).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LfsMigrationRequest {
    /// 방향.
    pub mode: LfsMigrationMode,
    /// 대상 패턴.
    pub patterns: Vec<String>,
}

/// 지원하지 않는 연산.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsupportedOperation {
    /// 연산 이름(고정 문자열).
    pub operation: &'static str,
}

impl UnsupportedOperation {
    /// 고정 오류 코드.
    pub const CODE: &'static str = "unsupported-operation";

    /// 고정 오류 코드.
    pub fn code(&self) -> &'static str {
        Self::CODE
    }
}

impl std::fmt::Display for UnsupportedOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "지원하지 않는 연산입니다: {}. Stellar는 기존 Git 이력을 다시 쓰지 않습니다",
            self.operation
        )
    }
}

impl std::error::Error for UnsupportedOperation {}

/// 기존 이력의 LFS 변환 요청을 거부합니다(항상 [`UnsupportedOperation`], 프로세스 생성 0).
pub fn migrate_lfs_history(
    _git: &GatedGitRunner,
    _worktree_root: &Path,
    request: &LfsMigrationRequest,
) -> Result<Infallible, UnsupportedOperation> {
    Err(UnsupportedOperation {
        operation: match request.mode {
            LfsMigrationMode::Import => "git lfs migrate import",
            LfsMigrationMode::Export => "git lfs migrate export",
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::GitPath;
    use crate::lfs::track::{LfsTrackRequest, LfsTrackTarget};
    use crate::runner::GitRunner;
    use crate::trust::GitTrust;

    fn requests() -> Vec<LfsMigrationRequest> {
        vec![
            LfsMigrationRequest {
                mode: LfsMigrationMode::Import,
                patterns: vec!["*.bin".into()],
            },
            LfsMigrationRequest {
                mode: LfsMigrationMode::Export,
                patterns: Vec::new(),
            },
        ]
    }

    #[test]
    fn migration_is_unsupported_without_spawning() {
        // executable이 없는 실행기: spawn을 시도하면 카운터가 증가합니다.
        let runner = GitRunner::new(
            std::path::PathBuf::from("/nonexistent/git"),
            Vec::new(),
            std::time::Duration::from_secs(5),
        );
        for trust in [GitTrust::Trusted, GitTrust::NotTrusted] {
            let gate = GatedGitRunner::new(runner.clone(), trust);
            for request in requests() {
                let err = match migrate_lfs_history(&gate, &std::env::temp_dir(), &request) {
                    Err(e) => e,
                    Ok(never) => match never {},
                };
                assert_eq!(err.code(), "unsupported-operation");
                assert!(err.to_string().contains("git lfs migrate"));
            }
        }
        assert_eq!(runner.spawn_count(), 0);
    }

    #[test]
    fn track_requests_never_build_history_commands() {
        // 승인 후 실행 경로(XIII.7.3)의 argv는 항상 `lfs track`이며 migrate를 만들 수 없습니다.
        let p = |s: &str| GitPath::from_bytes(s.as_bytes());
        let reqs = [
            LfsTrackRequest::new(LfsTrackTarget::Extension("bin".into()), p("a.bin")).unwrap(),
            LfsTrackRequest::new(LfsTrackTarget::File(p("migrate")), p("migrate")).unwrap(),
        ];
        for req in reqs {
            assert_eq!(&req.args()[..2], &["lfs", "track"]);
            assert!(!req.args()[..req.args().len() - 1]
                .iter()
                .any(|a| a == "migrate"));
        }
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::test_support::{
            commit_file, git_ok, hermetic_runner, init_repo_with_identity, system_git, TempDir,
        };

        #[test]
        fn history_is_unchanged_after_migration_request() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("lfs-migrate");
            let runner = hermetic_runner(git);
            let repo = dir.path();
            init_repo_with_identity(&runner, repo);
            commit_file(&runner, repo, "assets/a.bin", &[0u8; 64]);
            commit_file(&runner, repo, "stellar.yaml", b"v: 1\n");
            let refs = || {
                git_ok(
                    &runner,
                    repo,
                    ["for-each-ref", "--format=%(objectname) %(refname)"],
                )
                .stdout
            };
            let head = || git_ok(&runner, repo, ["rev-parse", "HEAD"]).stdout;
            let (refs_before, head_before) = (refs(), head());

            let gate_runner = runner.clone();
            let gate = GatedGitRunner::new(gate_runner.clone(), GitTrust::Trusted);
            let before = gate_runner.spawn_count();
            for request in requests() {
                assert!(migrate_lfs_history(&gate, repo, &request).is_err());
            }
            // 이력 변경 명령 실행 0(이 호출이 만든 프로세스 0), refs·HEAD 불변.
            assert_eq!(gate_runner.spawn_count(), before);
            assert_eq!(refs(), refs_before);
            assert_eq!(head(), head_before);
        }
    }
}
