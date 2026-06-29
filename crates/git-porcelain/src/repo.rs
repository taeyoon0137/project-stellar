//! 저장소 porcelain 연산 (IV.4.4~).
//!
//! [`GitRunner`]로 저장소 수준 Git 명령을 실행합니다. 현재는 `git init -b main`을 제공합니다.
//! `-b`는 Git 2.28+에서만 지원하므로, 구버전에서는 **대체 명령을 추측하지 않고** 지원 오류를
//! 반환합니다.

use std::path::Path;

use crate::command::{GitCommandFailure, GitCommandResult};
use crate::runner::{argv, GitRunError, GitRunner};
use crate::version::GitVersion;

/// 저장소 연산 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoError {
    /// Git 실행 실패(spawn·timeout 등).
    Run(GitRunError),
    /// Git 명령 비영 종료.
    Git(GitCommandFailure),
    /// `git init -b`를 지원하지 않는 버전(2.28 미만).
    InitBranchFlagUnsupported {
        /// major.
        major: u32,
        /// minor.
        minor: u32,
    },
}

impl std::fmt::Display for RepoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RepoError::Run(e) => write!(f, "Git 실행 실패: {e}"),
            RepoError::Git(e) => write!(f, "{e}"),
            RepoError::InitBranchFlagUnsupported { major, minor } => write!(
                f,
                "이 Git 버전({major}.{minor})은 'git init -b'를 지원하지 않습니다(2.28+ 필요)"
            ),
        }
    }
}

impl std::error::Error for RepoError {}

/// `-b` 플래그를 지원하는 최소 Git 버전.
const INIT_BRANCH_MIN: (u32, u32) = (2, 28);

/// 주어진 디렉터리에서 `git init -b main`을 실행합니다.
///
/// Git 버전이 2.28 미만이면 `-b`를 지원하지 않으므로 **대체 명령을 추측하지 않고**
/// [`RepoError::InitBranchFlagUnsupported`]를 반환합니다(Git을 실행하지도 않음).
pub fn git_init_main(
    runner: &GitRunner,
    version: &GitVersion,
    dir: &Path,
) -> Result<(), RepoError> {
    if (version.major, version.minor) < INIT_BRANCH_MIN {
        return Err(RepoError::InitBranchFlagUnsupported {
            major: version.major,
            minor: version.minor,
        });
    }
    let output = runner
        .run(&argv(["init", "-b", "main"]), dir)
        .map_err(RepoError::Run)?;
    GitCommandResult::from(output)
        .require_success()
        .map_err(RepoError::Git)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::{discover_git, GitDiscoveryConfig};
    use crate::version::probe_git_version;
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn temp_dir() -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("stellar-repo-test-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn git_runner(git: PathBuf) -> GitRunner {
        GitRunner::new(
            git,
            vec![OsString::from("PATH"), OsString::from("HOME")],
            Duration::from_secs(30),
        )
    }

    #[test]
    fn git_init_creates_main_branch() {
        let Some(git) = discover_git(&GitDiscoveryConfig::default()) else {
            return; // git 미설치 환경 건너뜀.
        };
        let version = probe_git_version(&git).unwrap();
        if (version.major, version.minor) < INIT_BRANCH_MIN {
            return; // 구버전 git 환경 건너뜀.
        }
        let dir = temp_dir();
        let runner = git_runner(git);

        git_init_main(&runner, &version, &dir).expect("git init -b main");

        // .git이 생성되고 HEAD가 정확히 main을 가리킴.
        assert!(dir.join(".git").exists());
        let head = runner
            .run(&argv(["symbolic-ref", "--short", "HEAD"]), &dir)
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&head.stdout).trim(), "main");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_version_is_unsupported_without_running_git() {
        // 가짜 구버전 → -b 미지원 오류. Git을 실행하지 않으므로 대체 명령 추측도 없음.
        let dir = temp_dir();
        let runner = git_runner(PathBuf::from("/nonexistent/git"));
        let old = GitVersion {
            major: 2,
            minor: 27,
            patch: 0,
            raw: "git version 2.27.0".to_string(),
        };
        assert_eq!(
            git_init_main(&runner, &old, &dir),
            Err(RepoError::InitBranchFlagUnsupported {
                major: 2,
                minor: 27
            })
        );
        // Git을 실행하지 않았으므로 .git이 생기지 않음.
        assert!(!dir.join(".git").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
