//! 저장소 bootstrap 연산 (IV.4.4~).
//!
//! 프로젝트 생성·clone 흐름의 Git 명령(`git init -b main`, `git add --all`, identity 확인, 최초
//! commit, `git clone`)을 제공합니다. 모든 함수는 [`GitExec`]를 받아 trust gate가 적용된
//! [`GatedGitRunner`](crate::trust::GatedGitRunner)로도 실행할 수 있습니다. 아직 신뢰 상태가 없는
//! 생성·clone 단계의 호출자는 [`GitRunner`](crate::runner::GitRunner)를 직접 넘기며, 이 경로는
//! gate를 거치지 않습니다([`crate::trust`] 모듈 문서의 bootstrap 경로). 기존 저장소 연산은 이
//! 모듈이 아니라 [`GatedGitRunner`](crate::trust::GatedGitRunner)만 받는 모듈을 사용합니다.
//!
//! `-b`는 Git 2.28+에서만 지원하므로, 구버전에서는 **대체 명령을 추측하지 않고** 지원 오류를
//! 반환합니다.

use std::ffi::OsString;
use std::path::Path;

use crate::command::{GitCommandFailure, GitCommandResult};
use crate::identity::{resolve_commit_identity, IdentityReadError};
use crate::runner::{argv, GitOutput, GitRunError};
use crate::trust::{GatedRunError, GitExec};
use crate::version::GitVersion;

/// 저장소 연산 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoError {
    /// Git 실행 실패(spawn·timeout 등).
    Run(GitRunError),
    /// trust gate가 적용된 실행 경로에서 신뢰되지 않은 프로젝트라 실행하지 않음(프로세스 생성 0).
    NotTrusted,
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
            RepoError::NotTrusted => write!(f, "{}", GatedRunError::ProjectNotTrusted),
            RepoError::Git(e) => write!(f, "{e}"),
            RepoError::InitBranchFlagUnsupported { major, minor } => write!(
                f,
                "이 Git 버전({major}.{minor})은 'git init -b'를 지원하지 않습니다(2.28+ 필요)"
            ),
        }
    }
}

impl std::error::Error for RepoError {}

impl From<GatedRunError> for RepoError {
    fn from(e: GatedRunError) -> RepoError {
        match e {
            GatedRunError::ProjectNotTrusted => RepoError::NotTrusted,
            GatedRunError::Run(run) => RepoError::Run(run),
        }
    }
}

/// 실행 후 성공을 요구합니다.
fn run_ok<G: GitExec + ?Sized>(
    git: &G,
    args: &[OsString],
    dir: &Path,
) -> Result<GitOutput, RepoError> {
    let output = git.exec(args, dir, None)?;
    let result = GitCommandResult::from(output)
        .require_success()
        .map_err(RepoError::Git)?;
    Ok(GitOutput {
        exit_code: result.exit_code(),
        stdout: result.stdout().to_vec(),
        stderr: result.stderr().to_vec(),
    })
}

/// `-b` 플래그를 지원하는 최소 Git 버전.
const INIT_BRANCH_MIN: (u32, u32) = (2, 28);

/// 주어진 디렉터리에서 `git init -b main`을 실행합니다.
///
/// Git 버전이 2.28 미만이면 `-b`를 지원하지 않으므로 **대체 명령을 추측하지 않고**
/// [`RepoError::InitBranchFlagUnsupported`]를 반환합니다(Git을 실행하지도 않음).
pub fn git_init_main<G: GitExec + ?Sized>(
    runner: &G,
    version: &GitVersion,
    dir: &Path,
) -> Result<(), RepoError> {
    if (version.major, version.minor) < INIT_BRANCH_MIN {
        return Err(RepoError::InitBranchFlagUnsupported {
            major: version.major,
            minor: version.minor,
        });
    }
    run_ok(runner, &argv(["init", "-b", "main"]), dir)?;
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
pub fn git_add_all<G: GitExec + ?Sized>(runner: &G, dir: &Path) -> Result<(), RepoError> {
    run_ok(runner, &argv(["add", "--all"]), dir)?;
    Ok(())
}

/// commit identity가 명시적으로 설정되어 있는지 검사합니다(읽기 전용).
///
/// 커밋 흐름의 [`check_commit_identity`](crate::commit::check_commit_identity)와 같은 해석
/// ([`crate::identity`]: `GIT_AUTHOR_*`/`GIT_COMMITTER_*` 환경 → `author.*`/`committer.*` →
/// `user.*` → `EMAIL`)을 씁니다. 손상된 config처럼 `git config`가 종료 코드 1 이외로 실패하면
/// 누락으로 숨기지 않고 [`RepoError::Git`]을 반환합니다.
pub fn check_user_identity<G: GitExec + ?Sized>(
    runner: &G,
    dir: &Path,
) -> Result<IdentityStatus, RepoError> {
    match resolve_commit_identity(runner, dir) {
        Ok(None) => Ok(IdentityStatus::Present),
        Ok(Some(missing)) => Ok(IdentityStatus::Missing {
            name_missing: missing.name_missing,
            email_missing: missing.email_missing,
        }),
        Err(IdentityReadError::Gate(e)) => Err(e.into()),
        Err(IdentityReadError::Git(failure)) => Err(RepoError::Git(failure)),
    }
}

/// `git clone -- <url> <dest>`를 실행합니다(argv, shell 없음).
///
/// `--`로 URL이 플래그로 해석되지 않게 하고, `dest`를 명시합니다. cwd는 `dest`의 부모입니다.
pub fn git_clone<G: GitExec + ?Sized>(runner: &G, url: &str, dest: &Path) -> Result<(), RepoError> {
    let cwd = dest.parent().unwrap_or(dest);
    run_ok(runner, &clone_args(url, dest), cwd)?;
    Ok(())
}

fn clone_args(url: &str, dest: &Path) -> [OsString; 4] {
    [
        OsString::from("clone"),
        OsString::from("--"),
        OsString::from(url),
        dest.as_os_str().to_os_string(),
    ]
}

/// 새 checkout을 만드는 동안 취소하면 Git 프로세스 트리까지 종료합니다.
pub fn git_clone_cancellable(
    runner: &crate::runner::GitRunner,
    url: &str,
    dest: &Path,
    cancel: &crate::cancel::GitCancelToken,
) -> Result<(), RepoError> {
    let output = runner
        .run_cancellable(
            &clone_args(url, dest),
            dest.parent().unwrap_or(dest),
            cancel,
        )
        .map_err(RepoError::Run)?;
    GitCommandResult::from(output)
        .require_success()
        .map_err(RepoError::Git)?;
    Ok(())
}

/// 최초 커밋 메시지(고정).
pub const INITIAL_COMMIT_MESSAGE: &str = "New Stellar project initialized";

/// 주어진 메시지로 커밋합니다(argv). identity는 호출 전에 보장되어야 합니다.
pub fn git_commit<G: GitExec + ?Sized>(
    runner: &G,
    dir: &Path,
    message: &str,
) -> Result<(), RepoError> {
    run_ok(runner, &argv(["commit", "-m", message]), dir)?;
    Ok(())
}

/// 최초 커밋([`INITIAL_COMMIT_MESSAGE`])을 생성합니다.
pub fn create_initial_commit<G: GitExec + ?Sized>(runner: &G, dir: &Path) -> Result<(), RepoError> {
    git_commit(runner, dir, INITIAL_COMMIT_MESSAGE)
}

/// 기본 파일을 `git add --all` 한 뒤 identity를 검사해 커밋 준비 상태를 반환합니다.
///
/// identity가 누락되면 **파일은 그대로 두고**(롤백하지 않음) `HaltedMissingIdentity`를
/// 반환합니다. 실제 커밋(IV.4.6)은 `Ready`일 때만 진행합니다.
pub fn stage_and_check_identity<G: GitExec + ?Sized>(
    runner: &G,
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
    use crate::runner::GitRunner;
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

    #[cfg(unix)]
    #[test]
    fn initial_commit_has_exact_message_and_clean_tree() {
        let Some(git) = discover_git(&GitDiscoveryConfig::default()) else {
            return;
        };
        let dir = temp_dir();
        let runner = hermetic_runner(git);
        runner.run(&argv(["init"]), &dir).unwrap();
        runner
            .run(&argv(["config", "user.name", "Test User"]), &dir)
            .unwrap();
        runner
            .run(&argv(["config", "user.email", "test@example.com"]), &dir)
            .unwrap();
        std::fs::write(dir.join("stellar.yaml"), b"schema_version: 1\n").unwrap();

        git_add_all(&runner, &dir).unwrap();
        create_initial_commit(&runner, &dir).unwrap();

        // git log -1 --format=%B가 정확히 메시지와 일치.
        let log = runner
            .run(&argv(["log", "-1", "--format=%B"]), &dir)
            .unwrap();
        let body = String::from_utf8_lossy(&log.stdout);
        assert_eq!(body.trim_end(), INITIAL_COMMIT_MESSAGE);

        // working tree clean.
        let status = runner.run(&argv(["status", "--porcelain"]), &dir).unwrap();
        assert!(
            status.stdout.is_empty(),
            "working tree가 깨끗하지 않음: {}",
            String::from_utf8_lossy(&status.stdout)
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod gated_tests {
    use super::*;
    use crate::commit::check_commit_identity;
    use crate::runner::GitRunner;
    use crate::trust::{GatedGitRunner, GitTrust};
    use std::path::PathBuf;
    use std::time::Duration;

    fn version_2_54() -> GitVersion {
        GitVersion {
            major: 2,
            minor: 54,
            patch: 0,
            raw: "git version 2.54.0".to_string(),
        }
    }

    #[test]
    fn untrusted_gate_rejects_every_bootstrap_op_without_spawning() {
        let runner = GitRunner::new(
            PathBuf::from("/nonexistent/git"),
            Vec::new(),
            Duration::from_secs(5),
        );
        let gate = GatedGitRunner::new(runner.clone(), GitTrust::NotTrusted);
        let dir = std::env::temp_dir();
        assert_eq!(
            git_init_main(&gate, &version_2_54(), &dir),
            Err(RepoError::NotTrusted)
        );
        assert_eq!(git_add_all(&gate, &dir), Err(RepoError::NotTrusted));
        assert_eq!(check_user_identity(&gate, &dir), Err(RepoError::NotTrusted));
        assert_eq!(
            stage_and_check_identity(&gate, &dir),
            Err(RepoError::NotTrusted)
        );
        assert_eq!(
            create_initial_commit(&gate, &dir),
            Err(RepoError::NotTrusted)
        );
        assert_eq!(
            git_clone(&gate, "/nonexistent/src", &dir.join("x")),
            Err(RepoError::NotTrusted)
        );
        assert_eq!(runner.spawn_count(), 0);
        assert!(RepoError::NotTrusted.to_string().contains("신뢰"));
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::test_support::{git_ok, system_git, TempDir};
        use std::ffi::OsString;

        fn runner_with_global(git: PathBuf, global: &Path) -> GitRunner {
            GitRunner::new(
                git,
                vec![OsString::from("PATH"), OsString::from("HOME")],
                Duration::from_secs(30),
            )
            .with_env_overrides(vec![
                (
                    OsString::from("GIT_CONFIG_GLOBAL"),
                    global.as_os_str().to_os_string(),
                ),
                (
                    OsString::from("GIT_CONFIG_SYSTEM"),
                    OsString::from("/dev/null"),
                ),
            ])
        }

        #[test]
        fn trusted_gate_runs_bootstrap_flow() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("repo-gated");
            let global = dir.path().join("global.gitconfig");
            std::fs::write(&global, b"[user]\n\tname = G\n\temail = g@example.com\n").unwrap();
            let repo = dir.path().join("repo");
            std::fs::create_dir_all(&repo).unwrap();
            let gate = GatedGitRunner::new(runner_with_global(git, &global), GitTrust::Trusted);
            git_init_main(&gate, &version_2_54(), &repo).expect("init");
            std::fs::write(repo.join("stellar.yaml"), b"schema_version: 1\n").unwrap();
            assert_eq!(
                stage_and_check_identity(&gate, &repo),
                Ok(CommitReadiness::Ready)
            );
            create_initial_commit(&gate, &repo).expect("commit");
            let log = git_ok(gate.runner().unwrap(), &repo, ["log", "-1", "--format=%B"]);
            assert_eq!(
                String::from_utf8_lossy(&log.stdout).trim_end(),
                INITIAL_COMMIT_MESSAGE
            );
        }

        #[test]
        fn creation_and_commit_identity_checks_agree() {
            // 생성 흐름(check_user_identity)과 커밋 흐름(check_commit_identity)이 같은 해석을 씁니다.
            let Some(git) = system_git() else {
                return;
            };
            // (global config, 기대 누락(name, email) — None이면 모두 있음)
            type Case = (&'static [u8], Option<(bool, bool)>);
            let cases: [Case; 5] = [
                (b"", Some((true, true))),
                (b"[user]\n\tname = U\n", Some((false, true))),
                (b"[user]\n\tname = U\n\temail = u@example.com\n", None),
                (
                    b"[author]\n\tname = A\n\temail = a@example.com\n\
                      [committer]\n\tname = C\n\temail = c@example.com\n",
                    None,
                ),
                (
                    b"[author]\n\tname = A\n\temail = a@example.com\n",
                    Some((true, true)),
                ),
            ];
            for (n, (config, expected)) in cases.into_iter().enumerate() {
                let dir = TempDir::new(&format!("repo-ident-{n}"));
                let global = dir.path().join("global.gitconfig");
                std::fs::write(&global, config).unwrap();
                let repo = dir.path().join("repo");
                std::fs::create_dir_all(&repo).unwrap();
                let runner = runner_with_global(git.clone(), &global);
                git_ok(&runner, &repo, ["init", "-q", "-b", "main"]);

                let creation = check_user_identity(&runner, &repo).expect("creation check");
                let gate = GatedGitRunner::new(runner, GitTrust::Trusted);
                let commit = check_commit_identity(&gate, &repo).expect("commit check");
                let expected_status = match expected {
                    None => IdentityStatus::Present,
                    Some((name_missing, email_missing)) => IdentityStatus::Missing {
                        name_missing,
                        email_missing,
                    },
                };
                assert_eq!(creation, expected_status, "case {n}");
                assert_eq!(
                    commit.map(|m| (m.name_missing, m.email_missing)),
                    expected,
                    "case {n}"
                );
            }
        }
    }
}
