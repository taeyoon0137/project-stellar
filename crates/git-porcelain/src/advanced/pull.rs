//! pull 전략 선택 (XXVI.1.3).
//!
//! pull은 사용자가 **명시적으로 고른 전략**으로만 실행합니다. 전략은 명령 인자로 전달하므로 사용자 Git
//! config(`pull.rebase`, `pull.ff` 등)를 바꾸지 않고, config 기본값에 기대지도 않습니다.
//!
//! | 전략 | 인자 |
//! |---|---|
//! | [`PullStrategy::Merge`] | `--no-rebase --ff --no-edit`(가능하면 fast-forward, 아니면 merge commit, 편집기 미실행) |
//! | [`PullStrategy::Rebase`] | `--rebase` |
//! | [`PullStrategy::FastForwardOnly`] | `--ff-only`(갈라졌으면 Git이 거부) |
//!
//! 등록된 remote만 받고 submodule 재귀를 끕니다(`--no-recurse-submodules`). conflict·거부는 Git 결과를
//! 그대로 돌려주며(stderr는 자격 증명을 가린 텍스트), 자동 continue·abort·reset을 하지 않습니다.

use std::path::Path;

use crate::cancel::GitCancelToken;
use crate::object::ObjectId;
use crate::refname::is_valid_ref_body;
use crate::remote::{list_remotes, redacted_failure, RemoteError};
use crate::runner::{argv, GitOutput};
use crate::trust::GatedGitRunner;

/// pull 전략(사용자 선택).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PullStrategy {
    /// merge(가능하면 fast-forward).
    Merge,
    /// rebase.
    Rebase,
    /// fast-forward만.
    FastForwardOnly,
}

impl PullStrategy {
    /// 전략 인자.
    pub fn args(self) -> &'static [&'static str] {
        match self {
            PullStrategy::Merge => &["--no-rebase", "--ff", "--no-edit"],
            PullStrategy::Rebase => &["--rebase"],
            PullStrategy::FastForwardOnly => &["--ff-only"],
        }
    }
}

/// pull 요청.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    /// 등록된 remote 이름.
    pub remote: String,
    /// 가져올 remote branch 이름.
    pub branch: String,
    /// 사용자가 고른 전략.
    pub strategy: PullStrategy,
}

/// pull 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullOutcome {
    /// pull 전 HEAD(unborn이면 `None`).
    pub head_before: Option<ObjectId>,
    /// pull 후 HEAD.
    pub head_after: Option<ObjectId>,
}

/// pull 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullError {
    /// 등록되지 않은 remote 또는 잘못된 branch 이름(Git pull 미실행).
    InvalidRequest(&'static str),
    /// Git 실행·취소·비영 종료(fast-forward 불가, conflict 등).
    Remote(RemoteError),
}

impl std::fmt::Display for PullError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PullError::InvalidRequest(what) => write!(f, "잘못된 pull 요청: {what}"),
            PullError::Remote(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for PullError {}

impl From<RemoteError> for PullError {
    fn from(e: RemoteError) -> PullError {
        PullError::Remote(e)
    }
}

fn head(git: &GatedGitRunner, worktree: &Path) -> Result<Option<ObjectId>, RemoteError> {
    let out: GitOutput = git.run(&argv(["rev-parse", "-q", "--verify", "HEAD"]), worktree)?;
    Ok(if out.success() {
        ObjectId::from_hex_bytes(out.stdout.trim_ascii())
    } else {
        None
    })
}

/// pull argv(전략·remote·branch).
pub fn pull_args(request: &PullRequest) -> Vec<std::ffi::OsString> {
    let mut args = argv(["pull", "--no-recurse-submodules"]);
    args.extend(argv(request.strategy.args().iter().copied()));
    args.extend(argv([
        "--",
        request.remote.as_str(),
        request.branch.as_str(),
    ]));
    args
}

/// 고른 전략으로 pull합니다(config 미변경, 자동 continue 없음).
pub fn pull(
    git: &GatedGitRunner,
    worktree: &Path,
    request: &PullRequest,
    cancel: &GitCancelToken,
) -> Result<PullOutcome, PullError> {
    if !is_valid_ref_body(&request.branch) {
        return Err(PullError::InvalidRequest("branch 이름"));
    }
    let remotes = list_remotes(git, worktree).map_err(RemoteError::from)?;
    if !remotes.iter().any(|r| r.name == request.remote) {
        return Err(PullError::InvalidRequest("등록되지 않은 remote"));
    }
    let head_before = head(git, worktree)?;
    let output = git
        .run_cancellable(&pull_args(request), worktree, cancel)
        .map_err(RemoteError::Gate)?;
    if !output.success() {
        return Err(PullError::Remote(redacted_failure(&output)));
    }
    Ok(PullOutcome {
        head_before,
        head_after: head(git, worktree)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strategy_args_are_explicit() {
        let request = PullRequest {
            remote: "origin".into(),
            branch: "main".into(),
            strategy: PullStrategy::FastForwardOnly,
        };
        let args: Vec<String> = pull_args(&request)
            .into_iter()
            .map(|a| a.into_string().unwrap())
            .collect();
        assert_eq!(
            args,
            [
                "pull",
                "--no-recurse-submodules",
                "--ff-only",
                "--",
                "origin",
                "main"
            ]
        );
        assert_eq!(
            PullStrategy::Merge.args(),
            ["--no-rebase", "--ff", "--no-edit"]
        );
        assert_eq!(PullStrategy::Rebase.args(), ["--rebase"]);
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::test_support::{
            commit_file, git_ok, hermetic_runner, init_bare, init_repo_with_identity, system_git,
            TempDir,
        };
        use crate::trust::GitTrust;
        use std::path::PathBuf;

        struct Fixture {
            dir: TempDir,
            bare: PathBuf,
            upstream: PathBuf,
            runner: crate::runner::GitRunner,
        }

        fn fixture() -> Option<Fixture> {
            let git = system_git()?;
            let dir = TempDir::new("adv-pull");
            let runner = hermetic_runner(git);
            let bare = dir.path().join("remote.git");
            init_bare(&runner, &bare);
            let upstream = dir.path().join("upstream");
            std::fs::create_dir_all(&upstream).unwrap();
            init_repo_with_identity(&runner, &upstream);
            commit_file(&runner, &upstream, "base.txt", b"base\n");
            git_ok(
                &runner,
                &upstream,
                ["push", "-q", bare.to_str().unwrap(), "main"],
            );
            Some(Fixture {
                dir,
                bare,
                upstream,
                runner,
            })
        }

        /// 원격을 clone하고 pull 관련 config를 일부러 반대로 설정한 작업 저장소.
        fn clone_with_conflicting_config(fx: &Fixture, name: &str) -> PathBuf {
            let work = fx.dir.path().join(name);
            git_ok(
                &fx.runner,
                fx.dir.path(),
                [
                    "clone",
                    "-q",
                    fx.bare.to_str().unwrap(),
                    work.to_str().unwrap(),
                ],
            );
            init_repo_with_identity(&fx.runner, &work);
            // 사용자 config가 무엇이든 명시 전략이 이깁니다.
            git_ok(&fx.runner, &work, ["config", "pull.ff", "only"]);
            git_ok(&fx.runner, &work, ["config", "pull.rebase", "true"]);
            work
        }

        fn advance_remote(fx: &Fixture, file: &str) {
            commit_file(&fx.runner, &fx.upstream, file, b"remote\n");
            git_ok(
                &fx.runner,
                &fx.upstream,
                ["push", "-q", fx.bare.to_str().unwrap(), "main"],
            );
        }

        fn parents(fx: &Fixture, work: &Path) -> usize {
            let out = git_ok(
                &fx.runner,
                work,
                ["rev-list", "--parents", "-n", "1", "HEAD"],
            );
            String::from_utf8_lossy(&out.stdout)
                .split_whitespace()
                .count()
                - 1
        }

        fn request(strategy: PullStrategy) -> PullRequest {
            PullRequest {
                remote: "origin".into(),
                branch: "main".into(),
                strategy,
            }
        }

        #[test]
        fn strategy_fixture_and_config_unchanged() {
            let Some(fx) = fixture() else {
                return; // git 미설치 환경 건너뜀.
            };
            let ff = clone_with_conflicting_config(&fx, "ff");
            let merge = clone_with_conflicting_config(&fx, "merge");
            let rebase = clone_with_conflicting_config(&fx, "rebase");
            // merge·rebase 저장소는 local commit으로 이력을 갈라지게 함.
            commit_file(&fx.runner, &merge, "local.txt", b"m\n");
            commit_file(&fx.runner, &rebase, "local.txt", b"r\n");
            advance_remote(&fx, "remote1.txt");
            let configs: Vec<(PathBuf, Vec<u8>)> = [&ff, &merge, &rebase]
                .iter()
                .map(|w| {
                    let p = w.join(".git/config");
                    let bytes = std::fs::read(&p).unwrap();
                    (p, bytes)
                })
                .collect();

            let gate = GatedGitRunner::new(fx.runner.clone(), GitTrust::Trusted);
            let cancel = GitCancelToken::new();
            // ff-only: fast-forward 가능하면 성공.
            let out = pull(&gate, &ff, &request(PullStrategy::FastForwardOnly), &cancel).unwrap();
            assert_ne!(out.head_before, out.head_after);
            assert_eq!(parents(&fx, &ff), 1);
            // ff-only: 갈라진 이력은 Git이 거부, HEAD 그대로.
            let before = head(&gate, &merge).unwrap();
            assert!(matches!(
                pull(
                    &gate,
                    &merge,
                    &request(PullStrategy::FastForwardOnly),
                    &cancel
                ),
                Err(PullError::Remote(RemoteError::Failed { .. }))
            ));
            assert_eq!(head(&gate, &merge).unwrap(), before);
            // merge: pull.ff=only config가 있어도 명시 전략으로 merge commit(부모 2).
            pull(&gate, &merge, &request(PullStrategy::Merge), &cancel).unwrap();
            assert_eq!(parents(&fx, &merge), 2);
            assert!(merge.join("remote1.txt").exists() && merge.join("local.txt").exists());
            // rebase: 선형 이력(부모 1), local commit이 원격 commit 위로.
            pull(&gate, &rebase, &request(PullStrategy::Rebase), &cancel).unwrap();
            assert_eq!(parents(&fx, &rebase), 1);
            let base = git_ok(&fx.runner, &rebase, ["rev-parse", "HEAD~1"]);
            let remote_main = git_ok(&fx.runner, &rebase, ["rev-parse", "origin/main"]);
            assert_eq!(base.stdout, remote_main.stdout);

            // 어떤 전략도 config를 바꾸지 않음.
            for (path, bytes) in configs {
                assert_eq!(std::fs::read(&path).unwrap(), bytes, "{path:?}");
            }
        }

        #[test]
        fn invalid_requests_do_not_pull() {
            let Some(fx) = fixture() else {
                return;
            };
            let work = clone_with_conflicting_config(&fx, "inv");
            let gate = GatedGitRunner::new(fx.runner.clone(), GitTrust::Trusted);
            let cancel = GitCancelToken::new();
            let mut bad = request(PullStrategy::Merge);
            bad.branch = "--rebase=interactive".into();
            let before = fx.runner.spawn_count();
            assert!(matches!(
                pull(&gate, &work, &bad, &cancel),
                Err(PullError::InvalidRequest(_))
            ));
            assert_eq!(fx.runner.spawn_count(), before);
            let mut unknown = request(PullStrategy::Merge);
            unknown.remote = "elsewhere".into();
            assert!(matches!(
                pull(&gate, &work, &unknown, &cancel),
                Err(PullError::InvalidRequest(_))
            ));
        }
    }
}
