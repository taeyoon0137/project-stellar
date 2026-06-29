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

/// 사용자 identity(`user.name`/`user.email`) 존재 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityStatus {
    /// 이름·이메일 모두 있음.
    Present,
    /// 일부 누락.
    Missing {
        /// `user.name` 누락.
        name_missing: bool,
        /// `user.email` 누락.
        email_missing: bool,
    },
}

/// staging 후 커밋 준비 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitReadiness {
    /// 커밋 진행 가능.
    Ready,
    /// identity 누락으로 커밋 단계 중단(파일은 그대로 둠).
    HaltedMissingIdentity {
        /// `user.name` 누락.
        name_missing: bool,
        /// `user.email` 누락.
        email_missing: bool,
    },
}

/// `git add --all`로 모든 변경을 stage 합니다.
pub fn git_add_all(runner: &GitRunner, dir: &Path) -> Result<(), RepoError> {
    let output = runner
        .run(&argv(["add", "--all"]), dir)
        .map_err(RepoError::Run)?;
    GitCommandResult::from(output)
        .require_success()
        .map_err(RepoError::Git)?;
    Ok(())
}

/// `user.name`/`user.email`(유효 config) 존재를 검사합니다.
pub fn check_user_identity(runner: &GitRunner, dir: &Path) -> Result<IdentityStatus, RepoError> {
    let name_missing = !config_value_present(runner, dir, "user.name")?;
    let email_missing = !config_value_present(runner, dir, "user.email")?;
    if name_missing || email_missing {
        Ok(IdentityStatus::Missing {
            name_missing,
            email_missing,
        })
    } else {
        Ok(IdentityStatus::Present)
    }
}

/// `git config <key>`가 비어 있지 않은 값을 반환하는지 확인합니다.
fn config_value_present(runner: &GitRunner, dir: &Path, key: &str) -> Result<bool, RepoError> {
    let output = runner
        .run(&argv(["config", key]), dir)
        .map_err(RepoError::Run)?;
    // 키가 없으면 git config는 비영 종료(코드 1). 그 경우 누락으로 봅니다(오류 아님).
    if output.exit_code == Some(0) {
        Ok(!String::from_utf8_lossy(&output.stdout).trim().is_empty())
    } else {
        Ok(false)
    }
}

/// 기본 파일을 `git add --all` 한 뒤 identity를 검사해 커밋 준비 상태를 반환합니다.
///
/// identity가 누락되면 **파일은 그대로 두고**(롤백하지 않음) `HaltedMissingIdentity`를
/// 반환합니다. 실제 커밋(IV.4.6)은 `Ready`일 때만 진행합니다.
pub fn stage_and_check_identity(
    runner: &GitRunner,
    dir: &Path,
) -> Result<CommitReadiness, RepoError> {
    git_add_all(runner, dir)?;
    match check_user_identity(runner, dir)? {
        IdentityStatus::Present => Ok(CommitReadiness::Ready),
        IdentityStatus::Missing {
            name_missing,
            email_missing,
        } => Ok(CommitReadiness::HaltedMissingIdentity {
            name_missing,
            email_missing,
        }),
    }
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

    /// 머신 global/system config와 무관하게 동작하도록 GIT_CONFIG_*를 비웁니다.
    #[cfg(unix)]
    fn hermetic_runner(git: PathBuf) -> GitRunner {
        git_runner(git).with_env_overrides(vec![
            (
                OsString::from("GIT_CONFIG_GLOBAL"),
                OsString::from("/dev/null"),
            ),
            (
                OsString::from("GIT_CONFIG_SYSTEM"),
                OsString::from("/dev/null"),
            ),
        ])
    }

    #[cfg(unix)]
    #[test]
    fn identity_present_is_ready_and_keeps_files() {
        let Some(git) = discover_git(&GitDiscoveryConfig::default()) else {
            return;
        };
        let dir = temp_dir();
        let runner = hermetic_runner(git);
        runner.run(&argv(["init"]), &dir).unwrap();
        // 로컬 identity 설정.
        runner
            .run(&argv(["config", "user.name", "Test User"]), &dir)
            .unwrap();
        runner
            .run(&argv(["config", "user.email", "test@example.com"]), &dir)
            .unwrap();
        std::fs::write(dir.join("stellar.yaml"), b"schema_version: 1\n").unwrap();

        let readiness = stage_and_check_identity(&runner, &dir).unwrap();
        assert_eq!(readiness, CommitReadiness::Ready);
        // add는 파일을 지우지 않음.
        assert!(dir.join("stellar.yaml").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn identity_missing_halts_but_keeps_files() {
        let Some(git) = discover_git(&GitDiscoveryConfig::default()) else {
            return;
        };
        let dir = temp_dir();
        // identity를 설정하지 않고 hermetic(전역/시스템 config 비움) → 누락.
        let runner = hermetic_runner(git);
        runner.run(&argv(["init"]), &dir).unwrap();
        std::fs::write(dir.join("stellar.yaml"), b"schema_version: 1\n").unwrap();

        let readiness = stage_and_check_identity(&runner, &dir).unwrap();
        assert!(
            matches!(readiness, CommitReadiness::HaltedMissingIdentity { .. }),
            "identity 누락인데 중단되지 않음: {readiness:?}"
        );
        // 중단해도 파일은 남음.
        assert!(dir.join("stellar.yaml").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
