//! 테스트 공용 도구(테스트 빌드 전용).
//!
//! drop 시 삭제되는 임시 디렉터리와, 머신 전역 Git 설정과 무관하게 실제 `git`을 실행하는
//! [`SystemGit`](호출 기록 포함)을 제공합니다. 시스템 Git이 없으면 관련 테스트는 건너뜁니다.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::git::{git_args, GitCommandOutput, GitCommandRunner, GitInvocationError};

/// drop 시 삭제되는 임시 디렉터리.
pub(crate) struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// `tag`를 이름에 넣은 고유 임시 디렉터리를 만듭니다.
    pub(crate) fn new(tag: &str) -> TempDir {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("stellar-deps-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("임시 디렉터리 생성");
        // macOS `/var` → `/private/var` 차이를 없애 경로 비교를 안정화합니다.
        let path = std::fs::canonicalize(&path).expect("canonicalize");
        TempDir { path }
    }

    /// 디렉터리 경로.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// PATH에서 `git` 실행 파일을 찾습니다.
fn find_git() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("git"))
        .find(|candidate| candidate.is_file())
}

/// hermetic 시스템 Git 실행기. 하위 명령별 호출 횟수를 기록합니다.
pub(crate) struct SystemGit {
    git: PathBuf,
    home: PathBuf,
    pub(crate) calls: BTreeMap<String, usize>,
}

impl SystemGit {
    /// 시스템 Git이 있으면 실행기를 만듭니다. `home`은 HOME으로 쓸 디렉터리입니다.
    pub(crate) fn find(home: &Path) -> Option<SystemGit> {
        let git = find_git();
        if git.is_none() {
            eprintln!("시스템 git 없음 — 실제 Git fixture 테스트를 건너뜁니다");
        }
        Some(SystemGit {
            git: git?,
            home: home.to_path_buf(),
            calls: BTreeMap::new(),
        })
    }

    /// 하위 명령 호출 횟수.
    pub(crate) fn count(&self, subcommand: &str) -> usize {
        self.calls.get(subcommand).copied().unwrap_or(0)
    }

    /// fixture 구성용 실행(성공 강제, 호출 기록에서 제외). stdout을 반환합니다.
    pub(crate) fn ok<I, S>(&mut self, cwd: &Path, args: I) -> String
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        let args = git_args(args);
        let out = self.spawn(&args, cwd).expect("fixture git 실행");
        assert!(
            out.success(),
            "fixture git 실패 {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("utf8 stdout")
    }

    fn spawn(&self, args: &[OsString], cwd: &Path) -> Result<GitCommandOutput, GitInvocationError> {
        let output = Command::new(&self.git)
            .args(args)
            .current_dir(cwd)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", &self.home)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .output()
            .map_err(|_| GitInvocationError::Spawn)?;
        Ok(GitCommandOutput {
            exit_code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

impl GitCommandRunner for SystemGit {
    fn run_git(
        &mut self,
        args: &[OsString],
        cwd: &Path,
    ) -> Result<GitCommandOutput, GitInvocationError> {
        // `--git-dir <dir>` 같은 전역 option 뒤의 첫 하위 명령을 기록합니다.
        let mut iter = args.iter().map(|a| a.to_string_lossy().into_owned());
        let mut subcommand = String::new();
        while let Some(arg) = iter.next() {
            if arg == "--git-dir" || arg == "-C" {
                iter.next();
                continue;
            }
            if !arg.starts_with('-') {
                subcommand = arg;
                break;
            }
        }
        *self.calls.entry(subcommand).or_default() += 1;
        self.spawn(args, cwd)
    }
}

/// 실제 Git fixture 저장소: main에 commit 두 개와 tag·branch.
pub(crate) struct FixtureRepo {
    /// 작업 저장소 경로(원격 URL로 사용).
    pub(crate) path: PathBuf,
    /// 첫 commit.
    pub(crate) first: String,
    /// 두 번째 commit(main HEAD).
    pub(crate) second: String,
}

/// `dir/name`에 fixture 저장소를 만듭니다.
///
/// - `v1`: 첫 commit을 가리키는 lightweight tag
/// - `v2`: 두 번째 commit을 가리키는 annotated tag
/// - `feature`: 첫 commit을 가리키는 branch
/// - `tree-tag`: tree object를 가리키는 lightweight tag(commit 아님)
pub(crate) fn fixture_repo(git: &mut SystemGit, dir: &Path, name: &str) -> FixtureRepo {
    let path = dir.join(name);
    std::fs::create_dir_all(&path).expect("repo dir");
    git.ok(&path, ["init", "-q", "-b", "main"]);
    git.ok(&path, ["config", "user.name", "Fixture"]);
    git.ok(&path, ["config", "user.email", "fixture@example.com"]);
    git.ok(&path, ["config", "commit.gpgsign", "false"]);
    git.ok(&path, ["config", "tag.gpgsign", "false"]);
    std::fs::write(path.join("stellar.yaml"), "schema_version: 1\n").expect("write");
    git.ok(&path, ["add", "stellar.yaml"]);
    git.ok(&path, ["commit", "-q", "-m", "first"]);
    let first = git.ok(&path, ["rev-parse", "HEAD"]).trim().to_string();
    git.ok(&path, ["tag", "v1"]);
    git.ok(&path, ["branch", "feature"]);
    std::fs::write(path.join("stellar.yaml"), "schema_version: 2\n").expect("write");
    git.ok(&path, ["commit", "-q", "-am", "second"]);
    let second = git.ok(&path, ["rev-parse", "HEAD"]).trim().to_string();
    git.ok(&path, ["tag", "-a", "v2", "-m", "release v2"]);
    git.ok(&path, ["tag", "tree-tag", "HEAD^{tree}"]);
    FixtureRepo {
        path,
        first,
        second,
    }
}
