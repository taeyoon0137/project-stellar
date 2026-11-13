//! Git 통합 fixture harness (XIV.14.1).
//!
//! 임시 디렉터리에 **local bare 저장소**(`remote.git`)와 그것을 `origin`으로 둔 **working 저장소**
//! (`work`)를 시스템 Git으로 만듭니다. Git 통합 corpus(XIV.14.*)와 상위 crate 테스트가 공유합니다.
//!
//! - **네트워크 없음**: `GIT_ALLOW_PROTOCOL=file`로 Git이 local file 전송 외 프로토콜(https·ssh·git)을
//!   거부합니다. 원격은 항상 fixture 안의 bare 저장소 경로입니다.
//! - **hermetic**: `GIT_CONFIG_GLOBAL`·`GIT_CONFIG_SYSTEM`을 null device로, `GIT_CONFIG_NOSYSTEM=1`,
//!   `GIT_ATTR_NOSYSTEM=1`, `GIT_TERMINAL_PROMPT=0`, `HOME`을 fixture root로 둡니다. 머신 전역 설정
//!   (alias·hook·filter·`init.defaultBranch`)이 결과를 바꾸지 않습니다. 환경 allowlist는 `PATH`뿐입니다.
//! - **결정론**: object format `sha1`, branch `main`, `core.autocrlf=false`, 고정 author/committer와
//!   commit·tag마다 1분씩 증가하는 고정 시각을 씁니다. 같은 scenario는 어느 장비에서 만들어도 같은
//!   commit ID가 나옵니다([`StandardScenario`] golden 테스트로 고정).
//!
//! 기본값이 꺼진 `test-support` feature(또는 이 crate의 unit test)에서만 빌드됩니다. 다른 crate는
//! `[dev-dependencies]`에서만 켭니다.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::discovery::{discover_git, GitDiscoveryConfig};
use crate::object::ObjectId;
use crate::runner::{argv, GitOutput, GitRunError, GitRunner, NULL_DEVICE};
use crate::trust::{GatedGitRunner, GitTrust};

/// 첫 commit 시각(2026-01-01T00:00:00Z, Unix seconds).
pub const BASE_EPOCH: u64 = 1_767_225_600;

/// 고정 author·committer 이름.
pub const FIXTURE_NAME: &str = "Stellar Fixture";

/// 고정 author·committer email.
pub const FIXTURE_EMAIL: &str = "fixture@stellar.invalid";

/// fixture 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixtureError {
    /// 파일 시스템 작업 실패.
    Io {
        /// 작업 이름.
        op: &'static str,
        /// io 오류 종류.
        kind: std::io::ErrorKind,
    },
    /// Git 실행 실패(spawn·timeout).
    Run(GitRunError),
    /// Git 비영 종료.
    Git {
        /// 실행한 argv(lossy).
        command: String,
        /// 종료 코드.
        exit_code: Option<i32>,
        /// stderr(lossy).
        stderr: String,
    },
    /// Git 출력 형식 오류.
    Parse(&'static str),
    /// 잘못된 fixture 입력(빈 경로·절대 경로·`..`).
    InvalidInput(&'static str),
}

impl std::fmt::Display for FixtureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FixtureError::Io { op, kind } => write!(f, "fixture {op} 실패: {kind:?}"),
            FixtureError::Run(e) => write!(f, "{e}"),
            FixtureError::Git {
                command,
                exit_code,
                stderr,
            } => write!(f, "fixture git {command} 실패({exit_code:?}): {stderr}"),
            FixtureError::Parse(what) => write!(f, "fixture 출력 해석 실패: {what}"),
            FixtureError::InvalidInput(what) => write!(f, "잘못된 fixture 입력: {what}"),
        }
    }
}

impl std::error::Error for FixtureError {}

fn io(op: &'static str) -> impl Fn(std::io::Error) -> FixtureError {
    move |e| FixtureError::Io { op, kind: e.kind() }
}

/// 임시 local bare 저장소 + working 저장소 fixture(drop 시 삭제).
#[derive(Debug)]
pub struct GitFixture {
    root: PathBuf,
    bare: PathBuf,
    work: PathBuf,
    runner: GitRunner,
    clock: AtomicU64,
    keep: bool,
}

/// fixture 실행기(hermetic 환경, 네트워크 프로토콜 차단).
fn fixture_runner(git: PathBuf, home: &Path) -> GitRunner {
    let pairs = [
        ("GIT_CONFIG_GLOBAL", OsStr::new(NULL_DEVICE)),
        ("GIT_CONFIG_SYSTEM", OsStr::new(NULL_DEVICE)),
        ("GIT_CONFIG_NOSYSTEM", OsStr::new("1")),
        ("GIT_ATTR_NOSYSTEM", OsStr::new("1")),
        ("GIT_TERMINAL_PROMPT", OsStr::new("0")),
        ("GIT_ALLOW_PROTOCOL", OsStr::new("file")),
        ("HOME", home.as_os_str()),
        ("GIT_AUTHOR_NAME", OsStr::new(FIXTURE_NAME)),
        ("GIT_AUTHOR_EMAIL", OsStr::new(FIXTURE_EMAIL)),
        ("GIT_COMMITTER_NAME", OsStr::new(FIXTURE_NAME)),
        ("GIT_COMMITTER_EMAIL", OsStr::new(FIXTURE_EMAIL)),
    ];
    GitRunner::new(git, vec![OsString::from("PATH")], Duration::from_secs(60)).with_env_overrides(
        pairs
            .iter()
            .map(|(k, v)| (OsString::from(k), v.to_os_string()))
            .collect(),
    )
}

/// `parent` 아래 새 고유 디렉터리를 만듭니다.
fn unique_dir(parent: &Path, tag: &str) -> Result<PathBuf, FixtureError> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    for _ in 0..64 {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = parent.join(format!("stellar-fixture-{tag}-{}-{n}", std::process::id()));
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io("임시 디렉터리 생성")(e)),
        }
    }
    Err(FixtureError::Io {
        op: "임시 디렉터리 생성",
        kind: std::io::ErrorKind::AlreadyExists,
    })
}

/// fixture 안 상대 경로 검증(빈 경로·절대 경로·`.`·`..`·`.git` 거부).
fn relative(path: &str) -> Result<&Path, FixtureError> {
    if crate::snapshot::materialize::validate_snapshot_path(path.as_bytes()).is_err() {
        return Err(FixtureError::InvalidInput("fixture 상대 경로"));
    }
    Ok(Path::new(path))
}

fn oid_of(output: &GitOutput) -> Result<ObjectId, FixtureError> {
    let raw = output.stdout.strip_suffix(b"\n").unwrap_or(&output.stdout);
    ObjectId::from_hex_bytes(raw).ok_or(FixtureError::Parse("object ID"))
}

impl GitFixture {
    /// 시스템 Git을 찾아 fixture를 만듭니다. Git이 없으면 `Ok(None)`(호출 테스트는 건너뜀).
    pub fn with_system_git(tag: &str) -> Result<Option<GitFixture>, FixtureError> {
        match discover_git(&GitDiscoveryConfig::default()) {
            Some(git) => GitFixture::new(git, tag).map(Some),
            None => Ok(None),
        }
    }

    /// `git` executable로 시스템 임시 디렉터리 아래 fixture를 만듭니다.
    ///
    /// `remote.git`(bare, `main`)과 `work`(`origin` = `remote.git`, 로컬 config 고정)를 만들며 commit은
    /// 아직 없습니다.
    pub fn new(git: PathBuf, tag: &str) -> Result<GitFixture, FixtureError> {
        let root = unique_dir(&std::env::temp_dir(), tag)?;
        let bare = root.join("remote.git");
        let work = root.join("work");
        let home = root.join("home");
        let fixture = GitFixture {
            runner: fixture_runner(git, &home),
            root,
            bare,
            work,
            clock: AtomicU64::new(0),
            keep: false,
        };
        for dir in [&fixture.bare, &fixture.work, &home] {
            std::fs::create_dir_all(dir).map_err(io("디렉터리 생성"))?;
        }
        let init = ["init", "-q", "--object-format=sha1", "-b", "main"];
        fixture.git_in(&fixture.bare, init.iter().copied().chain(["--bare"]))?;
        fixture.git_in(&fixture.work, init)?;
        for (key, value) in [
            ("core.autocrlf", "false"),
            ("core.safecrlf", "false"),
            ("commit.gpgsign", "false"),
            ("tag.gpgsign", "false"),
        ] {
            fixture.git(["config", key, value])?;
        }
        let bare = fixture.bare.as_os_str().to_os_string();
        fixture.git_os(
            &fixture.work,
            vec![
                OsString::from("remote"),
                OsString::from("add"),
                OsString::from("origin"),
                bare,
            ],
        )?;
        Ok(fixture)
    }

    /// fixture root(drop 시 삭제).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// local bare 저장소 경로(`origin`).
    pub fn bare(&self) -> &Path {
        &self.bare
    }

    /// working 저장소 경로.
    pub fn work(&self) -> &Path {
        &self.work
    }

    /// fixture 실행기(hermetic 환경).
    pub fn runner(&self) -> &GitRunner {
        &self.runner
    }

    /// trusted gate(Stellar API 호출용).
    pub fn gated(&self) -> GatedGitRunner {
        GatedGitRunner::new(self.runner.clone(), GitTrust::Trusted)
    }

    /// drop 시 삭제하지 않고 root를 돌려줍니다(호출자가 정리).
    pub fn keep(mut self) -> PathBuf {
        self.keep = true;
        self.root.clone()
    }

    /// 다음 commit·tag 시각을 고정한 실행기(1분씩 증가).
    fn dated_runner(&self) -> GitRunner {
        let tick = self.clock.fetch_add(1, Ordering::SeqCst);
        let date = OsString::from(format!("@{} +0000", BASE_EPOCH + tick * 60));
        self.runner.clone().with_added_env(vec![
            (OsString::from("GIT_AUTHOR_DATE"), date.clone()),
            (OsString::from("GIT_COMMITTER_DATE"), date),
        ])
    }

    fn check(args: &[OsString], output: GitOutput) -> Result<GitOutput, FixtureError> {
        if output.success() {
            return Ok(output);
        }
        Err(FixtureError::Git {
            command: args
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(" "),
            exit_code: output.exit_code,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    fn git_os(&self, cwd: &Path, args: Vec<OsString>) -> Result<GitOutput, FixtureError> {
        let output = self.runner.run(&args, cwd).map_err(FixtureError::Run)?;
        GitFixture::check(&args, output)
    }

    /// `cwd`에서 Git을 실행하고 성공을 요구합니다.
    pub fn git_in<I, S>(&self, cwd: &Path, args: I) -> Result<GitOutput, FixtureError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.git_os(cwd, argv(args))
    }

    /// working 저장소에서 Git을 실행하고 성공을 요구합니다.
    pub fn git<I, S>(&self, args: I) -> Result<GitOutput, FixtureError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.git_in(&self.work, args)
    }

    /// working 저장소에 파일을 씁니다(상위 디렉터리 생성).
    pub fn write(&self, path: &str, bytes: &[u8]) -> Result<(), FixtureError> {
        self.write_in(&self.work, path, bytes)
    }

    /// `dir` 아래 상대 경로에 파일을 씁니다(상위 디렉터리 생성).
    pub fn write_in(&self, dir: &Path, path: &str, bytes: &[u8]) -> Result<(), FixtureError> {
        let target = dir.join(relative(path)?);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(io("디렉터리 생성"))?;
        }
        std::fs::write(&target, bytes).map_err(io("파일 쓰기"))
    }

    /// `dir`의 모든 변경을 stage하고 고정 시각으로 commit한 뒤 HEAD commit ID를 돌려줍니다.
    pub fn commit_all_in(&self, dir: &Path, message: &str) -> Result<ObjectId, FixtureError> {
        self.git_in(dir, ["add", "-A"])?;
        let args = argv(["commit", "-q", "--allow-empty", "-m", message]);
        let output = self
            .dated_runner()
            .run(&args, dir)
            .map_err(FixtureError::Run)?;
        GitFixture::check(&args, output)?;
        self.rev_parse_in(dir, "HEAD")
    }

    /// working 저장소에서 [`GitFixture::commit_all_in`].
    pub fn commit_all(&self, message: &str) -> Result<ObjectId, FixtureError> {
        self.commit_all_in(&self.work, message)
    }

    /// 고정 시각으로 annotated tag를 만들고 tag object ID를 돌려줍니다.
    pub fn tag_annotated(
        &self,
        name: &str,
        target: &ObjectId,
        message: &str,
    ) -> Result<ObjectId, FixtureError> {
        let args = argv(["tag", "-a", "-m", message, name, target.as_str()]);
        let output = self
            .dated_runner()
            .run(&args, &self.work)
            .map_err(FixtureError::Run)?;
        GitFixture::check(&args, output)?;
        self.rev_parse(&format!("refs/tags/{name}"))
    }

    /// working 저장소에서 `rev-parse --verify <rev>`.
    pub fn rev_parse(&self, rev: &str) -> Result<ObjectId, FixtureError> {
        self.rev_parse_in(&self.work, rev)
    }

    /// `dir`에서 `rev-parse --verify <rev>`.
    pub fn rev_parse_in(&self, dir: &Path, rev: &str) -> Result<ObjectId, FixtureError> {
        oid_of(&self.git_in(dir, ["rev-parse", "--verify", rev])?)
    }

    /// working 저장소의 ref를 `origin`(local bare)에 push합니다.
    pub fn push(&self, refspec: &str) -> Result<(), FixtureError> {
        self.git(["push", "-q", "origin", refspec]).map(|_| ())
    }

    /// bare 저장소를 fixture 안 `name` 디렉터리로 clone합니다(두 번째 working 저장소).
    pub fn clone_bare(&self, name: &str) -> Result<PathBuf, FixtureError> {
        let target = self.root.join(relative(name)?);
        self.git_os(
            &self.root,
            vec![
                OsString::from("clone"),
                OsString::from("-q"),
                OsString::from("--"),
                self.bare.as_os_str().to_os_string(),
                target.as_os_str().to_os_string(),
            ],
        )?;
        self.git_in(&target, ["config", "core.autocrlf", "false"])?;
        Ok(target)
    }

    /// working 저장소에서 linked worktree를 fixture 안 `name`에 만듭니다(`commit_ish` checkout).
    pub fn add_worktree(&self, name: &str, commit_ish: &str) -> Result<PathBuf, FixtureError> {
        let target = self.root.join(relative(name)?);
        self.git_os(
            &self.work,
            vec![
                OsString::from("worktree"),
                OsString::from("add"),
                OsString::from("-q"),
                OsString::from("--detach"),
                OsString::from("--"),
                target.as_os_str().to_os_string(),
                OsString::from(commit_ish),
            ],
        )?;
        Ok(target)
    }
}

impl Drop for GitFixture {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

/// 표준 scenario 결과(모든 장비에서 같은 ID).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandardScenario {
    /// 첫 commit(`main`의 첫 commit, `feature`의 분기점).
    pub initial: ObjectId,
    /// `main` 끝 commit.
    pub main: ObjectId,
    /// `feature` 끝 commit(`initial`에서 분기).
    pub feature: ObjectId,
    /// annotated tag `v1`(→ `main`) object ID.
    pub tag_v1: ObjectId,
}

/// 표준 scenario를 만듭니다. `main`·`feature`·`v1`을 bare 저장소에 push하고 `main`을 checkout한 상태로
/// 끝납니다.
///
/// ```text
/// initial ── main       (main: stellar.yaml, design/pages/home.yaml 수정)
///    └────── feature    (feature: design/pages/about.yaml 추가)
/// v1 → main (annotated)
/// ```
pub fn build_standard_scenario(fixture: &GitFixture) -> Result<StandardScenario, FixtureError> {
    fixture.write("stellar.yaml", b"format: 1\nname: fixture\n")?;
    fixture.write("design/pages/home.yaml", b"id: home\n")?;
    let initial = fixture.commit_all("initial")?;
    fixture.push("main")?;

    fixture.write("design/pages/home.yaml", b"id: home\ntitle: Home\n")?;
    let main = fixture.commit_all("update home")?;
    fixture.push("main")?;

    fixture.git(["switch", "-q", "-c", "feature", initial.as_str()])?;
    fixture.write("design/pages/about.yaml", b"id: about\n")?;
    let feature = fixture.commit_all("add about")?;
    fixture.push("feature")?;
    fixture.git(["switch", "-q", "main"])?;

    let tag_v1 = fixture.tag_annotated("v1", &main, "v1")?;
    fixture.push("refs/tags/v1")?;
    Ok(StandardScenario {
        initial,
        main,
        feature,
        tag_v1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 표준 scenario golden(SHA-1). 바뀌면 fixture 결정론이 깨진 것입니다.
    const GOLDEN_INITIAL: &str = "430418a2ddc33655f75f4b7dbbf6b83e378851b5";
    const GOLDEN_MAIN: &str = "6073a623e79df89edcf0ec0a8ae2b34c5415bf4e";
    const GOLDEN_FEATURE: &str = "1cf4a59f0e548d662e0ab18e7092a0008a285203";
    const GOLDEN_TAG_V1: &str = "0d5b6c898e0d38bbff4bc579a816c6c709ad0f0a";

    #[test]
    fn standard_scenario_is_identical_across_runs_and_matches_golden() {
        let (Some(a), Some(b)) = (
            GitFixture::with_system_git("std-a").unwrap(),
            GitFixture::with_system_git("std-b").unwrap(),
        ) else {
            return; // git 미설치 환경 건너뜀.
        };
        let first = build_standard_scenario(&a).unwrap();
        let second = build_standard_scenario(&b).unwrap();
        assert_eq!(first, second, "같은 scenario가 다른 ID를 만듦");
        assert_eq!(
            [
                first.initial.as_str(),
                first.main.as_str(),
                first.feature.as_str(),
                first.tag_v1.as_str()
            ],
            [GOLDEN_INITIAL, GOLDEN_MAIN, GOLDEN_FEATURE, GOLDEN_TAG_V1]
        );
        // bare 저장소에 같은 ref가 있음(local 전송).
        for (rev, expected) in [
            ("refs/heads/main", &first.main),
            ("refs/heads/feature", &first.feature),
            ("refs/tags/v1", &first.tag_v1),
        ] {
            assert_eq!(&a.rev_parse_in(a.bare(), rev).unwrap(), expected, "{rev}");
        }
        assert_eq!(a.rev_parse("HEAD").unwrap(), first.main);
        assert_eq!(a.rev_parse("v1^{commit}").unwrap(), first.main);
    }

    #[test]
    fn second_clone_and_linked_worktree_share_the_bare_history() {
        let Some(fixture) = GitFixture::with_system_git("std-clone").unwrap() else {
            return;
        };
        let scenario = build_standard_scenario(&fixture).unwrap();
        let clone = fixture.clone_bare("clone").unwrap();
        assert_eq!(fixture.rev_parse_in(&clone, "HEAD").unwrap(), scenario.main);
        assert_eq!(
            fixture
                .rev_parse_in(&clone, "refs/remotes/origin/feature")
                .unwrap(),
            scenario.feature
        );
        let wt = fixture.add_worktree("wt", "feature").unwrap();
        assert_eq!(fixture.rev_parse_in(&wt, "HEAD").unwrap(), scenario.feature);
        assert!(wt.join("design/pages/about.yaml").is_file());
        // clone에서 만든 commit도 같은 시각 규칙을 씁니다.
        fixture
            .write_in(&clone, "design/pages/contact.yaml", b"id: contact\n")
            .unwrap();
        let c = fixture.commit_all_in(&clone, "add contact").unwrap();
        let date = fixture
            .git_in(
                &clone,
                ["log", "-1", "--format=%at %ct %an <%ae>", c.as_str()],
            )
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&date.stdout).trim(),
            format!(
                "{0} {0} {FIXTURE_NAME} <{FIXTURE_EMAIL}>",
                BASE_EPOCH + 4 * 60
            )
        );
    }

    #[test]
    fn network_protocols_are_refused_and_fixture_is_hermetic() {
        let Some(fixture) = GitFixture::with_system_git("std-net").unwrap() else {
            return;
        };
        fixture.write("a.txt", b"a\n").unwrap();
        fixture.commit_all("a").unwrap();
        for url in [
            "https://example.invalid/stellar.git",
            "ssh://git@example.invalid/stellar.git",
            "git://example.invalid/stellar.git",
        ] {
            let started = std::time::Instant::now();
            let err = fixture.git(["push", url, "main"]).unwrap_err();
            assert!(matches!(err, FixtureError::Git { .. }), "{url}: {err:?}");
            // 연결을 시도하지 않고 즉시 거부합니다.
            assert!(started.elapsed() < Duration::from_secs(10), "{url}");
        }
        // 전역 설정이 보이지 않음(HOME·GIT_CONFIG_GLOBAL 격리).
        let origin = fixture
            .git(["config", "--show-origin", "--get-all", "user.name"])
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        assert!(origin.is_empty(), "{origin}");
        assert_eq!(
            fixture.runner().child_env_var("HOME"),
            Some(fixture.root().join("home").into_os_string())
        );
    }

    #[test]
    fn invalid_relative_paths_are_rejected_and_root_is_removed_on_drop() {
        let Some(fixture) = GitFixture::with_system_git("std-drop").unwrap() else {
            return;
        };
        for bad in ["", "/abs", "../escape", "a/../../b", ".git/config"] {
            assert_eq!(
                fixture.write(bad, b"x"),
                Err(FixtureError::InvalidInput("fixture 상대 경로")),
                "{bad:?}"
            );
        }
        let root = fixture.root().to_path_buf();
        assert!(root.join("remote.git").is_dir());
        drop(fixture);
        assert!(!root.exists());
        // keep이면 남습니다.
        let kept = GitFixture::with_system_git("std-keep")
            .unwrap()
            .unwrap()
            .keep();
        assert!(kept.join("work").is_dir());
        std::fs::remove_dir_all(kept).unwrap();
    }
}
