//! 프로젝트 신뢰 gate (XIV.1.1).
//!
//! 저장소 내부에서의 Git 실행은 hooks·filters·fsmonitor·외부 driver 같은 **저장소가 지정한
//! 프로그램**을 실행할 수 있습니다. 그래서 **신뢰된(trusted) 프로젝트에서만** [`GitRunner`]를
//! 활성화하고, 신뢰되지 않은 상태에서는 모든 Git command request를 프로세스 생성 **전에**
//! [`GatedRunError::ProjectNotTrusted`]로 거부합니다.
//!
//! 신뢰 상태의 원본은 project crate의 trust store(IV.5.2)입니다. git-porcelain은 project crate에
//! 의존할 수 없으므로, 호출자가 그 상태를 [`GitTrust`]로 변환해 공급합니다. 저장소 단위 Git
//! 연산(XIV.*)은 모두 [`GatedGitRunner`]를 받으므로 gate를 우회한 실행 경로가 없습니다.
//!
//! 신뢰 등록은 Git 전역 설정을 변경하지 않습니다(gate는 메모리 상태만 가집니다).

use std::ffi::OsString;
use std::path::Path;

use crate::cancel::GitCancelToken;
use crate::runner::{GitOutput, GitRunError, GitRunner};

/// Git 실행 관점의 프로젝트 신뢰 상태(호출자가 project trust store에서 변환해 공급).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitTrust {
    /// 사용자가 신뢰한 프로젝트: Git 실행 허용.
    Trusted,
    /// 신뢰되지 않은 프로젝트(기본값): 모든 Git 실행 거부.
    NotTrusted,
}

/// gate를 거친 Git 실행 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatedRunError {
    /// 신뢰되지 않은 프로젝트라 Git을 실행하지 않음(프로세스 생성 0).
    ProjectNotTrusted,
    /// 신뢰된 상태에서 Git 실행 자체가 실패함.
    Run(GitRunError),
}

impl std::fmt::Display for GatedRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GatedRunError::ProjectNotTrusted => {
                f.write_str("신뢰되지 않은 프로젝트에서는 Git 명령을 실행하지 않습니다")
            }
            GatedRunError::Run(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for GatedRunError {}

/// 신뢰 상태에 따라 [`GitRunner`]를 활성화·비활성화하는 gate.
#[derive(Debug, Clone)]
pub struct GatedGitRunner {
    runner: GitRunner,
    trust: GitTrust,
}

impl GatedGitRunner {
    /// 실행기와 현재 신뢰 상태로 gate를 만듭니다.
    pub fn new(runner: GitRunner, trust: GitTrust) -> GatedGitRunner {
        GatedGitRunner { runner, trust }
    }

    /// 현재 신뢰 상태.
    pub fn trust(&self) -> GitTrust {
        self.trust
    }

    /// 신뢰 상태를 갱신합니다(사용자가 신뢰를 부여·철회한 뒤 호출).
    pub fn set_trust(&mut self, trust: GitTrust) {
        self.trust = trust;
    }

    /// Git 실행이 활성화되어 있는지(= trusted).
    pub fn is_enabled(&self) -> bool {
        self.trust == GitTrust::Trusted
    }

    /// trusted일 때만 내부 실행기를 빌려줍니다. untrusted면 [`GatedRunError::ProjectNotTrusted`].
    pub fn runner(&self) -> Result<&GitRunner, GatedRunError> {
        match self.trust {
            GitTrust::Trusted => Ok(&self.runner),
            GitTrust::NotTrusted => Err(GatedRunError::ProjectNotTrusted),
        }
    }

    /// trusted일 때만 Git을 실행합니다. untrusted면 **프로세스를 만들지 않고** 거부합니다.
    pub fn run(&self, args: &[OsString], cwd: &Path) -> Result<GitOutput, GatedRunError> {
        self.runner()?.run(args, cwd).map_err(GatedRunError::Run)
    }

    /// trusted일 때만 취소 가능한 실행을 합니다(XIV.1.4). untrusted면 프로세스를 만들지 않습니다.
    pub fn run_cancellable(
        &self,
        args: &[OsString],
        cwd: &Path,
        cancel: &GitCancelToken,
    ) -> Result<GitOutput, GatedRunError> {
        self.runner()?
            .run_cancellable(args, cwd, cancel)
            .map_err(GatedRunError::Run)
    }

    /// 내부 실행기의 프로세스 생성 시도 횟수(gate 거부는 세지 않음).
    pub fn spawn_count(&self) -> u64 {
        self.runner.spawn_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::argv;
    use std::path::PathBuf;
    use std::time::Duration;

    #[test]
    fn untrusted_rejects_without_spawning() {
        // 존재하지 않는 executable: 만약 spawn을 시도하면 Spawn 오류가 나고 카운터가 증가함.
        let runner = GitRunner::new(
            PathBuf::from("/nonexistent/git"),
            Vec::new(),
            Duration::from_secs(5),
        );
        let gate = GatedGitRunner::new(runner.clone(), GitTrust::NotTrusted);
        let cwd = std::env::temp_dir();
        assert_eq!(
            gate.run(&argv(["status"]), &cwd),
            Err(GatedRunError::ProjectNotTrusted)
        );
        assert_eq!(gate.runner().err(), Some(GatedRunError::ProjectNotTrusted));
        assert!(!gate.is_enabled());
        assert_eq!(runner.spawn_count(), 0);
        assert_eq!(gate.spawn_count(), 0);
    }

    #[test]
    fn trust_transition_enables_and_disables() {
        let runner = GitRunner::new(
            PathBuf::from("/nonexistent/git"),
            Vec::new(),
            Duration::from_secs(5),
        );
        let mut gate = GatedGitRunner::new(runner, GitTrust::NotTrusted);
        let cwd = std::env::temp_dir();

        gate.set_trust(GitTrust::Trusted);
        assert!(gate.is_enabled());
        // trusted면 실제 spawn을 시도(여기서는 executable이 없어 Spawn 오류).
        assert!(matches!(
            gate.run(&argv(["status"]), &cwd),
            Err(GatedRunError::Run(GitRunError::Spawn { .. }))
        ));
        assert_eq!(gate.spawn_count(), 1);

        // 신뢰 철회 후에는 다시 spawn 0으로 거부.
        gate.set_trust(GitTrust::NotTrusted);
        assert_eq!(
            gate.run(&argv(["status"]), &cwd),
            Err(GatedRunError::ProjectNotTrusted)
        );
        assert_eq!(gate.spawn_count(), 1);
    }

    #[cfg(unix)]
    mod malicious_repo {
        use super::*;
        use crate::test_support::{git_ok, hermetic_runner, init_repo_with_identity, TempDir};
        use std::os::unix::fs::PermissionsExt;

        /// 실행되면 marker 파일을 만드는 shell script를 씁니다.
        fn write_script(path: &Path, marker: &Path, tail: &str) {
            let body = format!("#!/bin/sh\ntouch '{}'\n{tail}\n", marker.display());
            std::fs::write(path, body).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        struct Fixture {
            _dir: TempDir,
            repo: PathBuf,
            markers: [PathBuf; 3],
        }

        /// fsmonitor hook, clean/smudge filter, pre-commit hook이 각각 marker를 만드는 악성 repo.
        fn malicious_repo(runner: &GitRunner) -> Fixture {
            let dir = TempDir::new("trust-evil");
            let repo = dir.path().join("repo");
            let tools = dir.path().join("tools");
            std::fs::create_dir_all(&repo).unwrap();
            std::fs::create_dir_all(&tools).unwrap();
            init_repo_with_identity(runner, &repo);

            let fsmonitor_marker = dir.path().join("PWNED_fsmonitor");
            let filter_marker = dir.path().join("PWNED_filter");
            let hook_marker = dir.path().join("PWNED_hook");

            let fsmonitor = tools.join("fsmonitor.sh");
            write_script(&fsmonitor, &fsmonitor_marker, "exit 1");
            let filter = tools.join("filter.sh");
            write_script(&filter, &filter_marker, "cat");
            write_script(&repo.join(".git/hooks/pre-commit"), &hook_marker, "exit 0");

            let fsmonitor_s = fsmonitor.to_str().unwrap().to_string();
            let filter_s = filter.to_str().unwrap().to_string();
            git_ok(runner, &repo, ["config", "core.fsmonitor", &fsmonitor_s]);
            git_ok(runner, &repo, ["config", "filter.evil.clean", &filter_s]);
            git_ok(runner, &repo, ["config", "filter.evil.smudge", &filter_s]);
            std::fs::write(repo.join(".gitattributes"), b"*.txt filter=evil\n").unwrap();
            std::fs::write(repo.join("a.txt"), b"hello\n").unwrap();

            // 구성 단계에서 marker가 생겼다면 지워 깨끗한 상태에서 시작.
            for m in [&fsmonitor_marker, &filter_marker, &hook_marker] {
                let _ = std::fs::remove_file(m);
            }
            Fixture {
                _dir: dir,
                repo,
                markers: [fsmonitor_marker, filter_marker, hook_marker],
            }
        }

        fn requests() -> Vec<Vec<OsString>> {
            vec![
                argv(["status", "--porcelain=v2", "-z"]),
                argv(["add", "--all"]),
                argv(["commit", "-m", "evil"]),
                argv(["diff"]),
                argv(["rev-parse", "--show-toplevel"]),
                argv(["log", "-1"]),
            ]
        }

        #[test]
        fn untrusted_malicious_repo_spawns_zero_processes() {
            let Some(git) = crate::test_support::system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let setup = hermetic_runner(git.clone());
            let fixture = malicious_repo(&setup);

            // gate용 실행기는 별도 카운터로 측정.
            let runner = hermetic_runner(git);
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::NotTrusted);
            for request in requests() {
                assert_eq!(
                    gate.run(&request, &fixture.repo),
                    Err(GatedRunError::ProjectNotTrusted),
                    "untrusted 요청이 거부되지 않음: {request:?}"
                );
            }
            // 핵심: 프로세스 생성 0, 저장소 지정 프로그램 실행 0.
            assert_eq!(runner.spawn_count(), 0, "untrusted에서 프로세스 생성됨");
            for marker in &fixture.markers {
                assert!(!marker.exists(), "악성 프로그램 실행됨: {marker:?}");
            }
        }

        #[test]
        fn trusted_control_shows_fixture_is_really_malicious() {
            // 대조군: trusted면 같은 요청이 실제로 Git을 실행해 저장소 지정 프로그램이 동작함.
            // (위 테스트의 marker 부재가 gate 덕분임을 보장)
            let Some(git) = crate::test_support::system_git() else {
                return;
            };
            let setup = hermetic_runner(git.clone());
            let fixture = malicious_repo(&setup);
            let runner = hermetic_runner(git);
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            for request in requests() {
                gate.run(&request, &fixture.repo).expect("trusted 실행");
            }
            assert!(runner.spawn_count() >= requests().len() as u64);
            for marker in &fixture.markers {
                assert!(marker.exists(), "대조군에서 실행되지 않음: {marker:?}");
            }
        }
    }
}
