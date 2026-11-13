//! 테스트 공용 도구(테스트 빌드 전용).
//!
//! 임시 디렉터리, 머신 전역 Git 설정과 무관한 hermetic 실행기, 실제 `git`으로 fixture 저장소를
//! 만드는 헬퍼를 제공합니다.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::discovery::{discover_git, GitDiscoveryConfig};
use crate::runner::{argv, GitOutput, GitRunner};

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
            std::env::temp_dir().join(format!("stellar-gp-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
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

/// 시스템 Git을 찾습니다(없으면 `None` → 호출 테스트는 건너뜀).
pub(crate) fn system_git() -> Option<PathBuf> {
    discover_git(&GitDiscoveryConfig::default())
}

/// 머신 global/system config와 무관하게 동작하는 실행기(PATH·HOME만 전달).
#[cfg(unix)]
pub(crate) fn hermetic_runner(git: PathBuf) -> GitRunner {
    hermetic_runner_with_timeout(git, Duration::from_secs(30))
}

/// 제한 시간을 지정한 hermetic 실행기.
#[cfg(unix)]
pub(crate) fn hermetic_runner_with_timeout(git: PathBuf, timeout: Duration) -> GitRunner {
    GitRunner::new(
        git,
        vec![OsString::from("PATH"), OsString::from("HOME")],
        timeout,
    )
    .with_env_overrides(vec![
        (
            OsString::from("GIT_CONFIG_GLOBAL"),
            OsString::from("/dev/null"),
        ),
        (
            OsString::from("GIT_CONFIG_SYSTEM"),
            OsString::from("/dev/null"),
        ),
        (OsString::from("GIT_TERMINAL_PROMPT"), OsString::from("0")),
    ])
}

/// GitHub URL을 로컬 디렉터리 `base` 아래 bare 저장소로 바꾸는 hermetic 실행기(네트워크 미사용).
///
/// `url.<base>/.insteadOf`로 `https://github.com/`·`git@github.com:`·`ssh://git@github.com/`를
/// `base/`로 바꾸고, `GIT_ALLOW_PROTOCOL=file`로 로컬 전송 외 프로토콜을 Git이 거부하게 합니다.
#[cfg(unix)]
pub(crate) fn github_redirect_runner(git: PathBuf, base: &Path) -> GitRunner {
    let key = format!("url.{}/.insteadOf", base.display());
    let mut env = vec![
        (OsString::from("GIT_CONFIG_COUNT"), OsString::from("3")),
        (OsString::from("GIT_ALLOW_PROTOCOL"), OsString::from("file")),
    ];
    for (i, prefix) in [
        "https://github.com/",
        "git@github.com:",
        "ssh://git@github.com/",
    ]
    .into_iter()
    .enumerate()
    {
        env.push((
            OsString::from(format!("GIT_CONFIG_KEY_{i}")),
            OsString::from(&key),
        ));
        env.push((
            OsString::from(format!("GIT_CONFIG_VALUE_{i}")),
            OsString::from(prefix),
        ));
    }
    hermetic_runner(git).with_added_env(env)
}

/// `dir`에 빈 bare 저장소(`main`)를 만듭니다.
pub(crate) fn init_bare(runner: &GitRunner, dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    git_ok(runner, dir, ["init", "-q", "--bare", "-b", "main"]);
}

/// fixture 구성용 Git 실행(성공을 강제).
pub(crate) fn git_ok<I, S>(runner: &GitRunner, dir: &Path, args: I) -> GitOutput
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let args = argv(args);
    let out = runner.run(&args, dir).expect("fixture git 실행");
    assert!(
        out.success(),
        "fixture git 실패 {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// `git init -b main` + 로컬 identity를 가진 저장소를 만듭니다.
pub(crate) fn init_repo_with_identity(runner: &GitRunner, dir: &Path) {
    git_ok(runner, dir, ["init", "-q", "-b", "main"]);
    git_ok(runner, dir, ["config", "user.name", "Test User"]);
    git_ok(runner, dir, ["config", "user.email", "test@example.com"]);
}

/// 파일 하나를 커밋합니다.
pub(crate) fn commit_file(runner: &GitRunner, dir: &Path, name: &str, contents: &[u8]) {
    if let Some(parent) = dir.join(name).parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(dir.join(name), contents).unwrap();
    git_ok(runner, dir, ["add", "--", name]);
    git_ok(runner, dir, ["commit", "-q", "-m", "fixture"]);
}

/// 바이트를 blob으로 기록하고 object ID(16진수)를 반환합니다(`hash-object -w --stdin`).
pub(crate) fn write_blob(runner: &GitRunner, dir: &Path, bytes: &[u8]) -> String {
    let out = runner
        .run_with_input(&argv(["hash-object", "-w", "--stdin"]), dir, bytes)
        .expect("hash-object 실행");
    assert!(out.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// 작업 트리 파일 없이 index에 원시 바이트 경로를 추가합니다(`update-index --cacheinfo`).
///
/// macOS APFS는 UTF-8이 아닌 파일 이름을 만들 수 없으므로 non-UTF8 경로 fixture는 이 방식으로
/// Git 경로만 등록합니다(argv는 원시 바이트 그대로 전달).
#[cfg(unix)]
pub(crate) fn add_index_path(
    runner: &GitRunner,
    dir: &Path,
    mode: &str,
    raw_path: &[u8],
    contents: &[u8],
) -> String {
    use std::os::unix::ffi::OsStrExt;
    let oid = write_blob(runner, dir, contents);
    let mut cacheinfo = format!("{mode},{oid},").into_bytes();
    cacheinfo.extend_from_slice(raw_path);
    let args: Vec<OsString> = vec![
        OsString::from("update-index"),
        OsString::from("--add"),
        OsString::from("--cacheinfo"),
        std::ffi::OsStr::from_bytes(&cacheinfo).to_os_string(),
    ];
    let out = runner.run(&args, dir).unwrap();
    assert!(out.success(), "{}", String::from_utf8_lossy(&out.stderr));
    oid
}

/// symlink를 해석한 실경로(macOS `/var`→`/private/var` 차이 흡수).
pub(crate) fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap()
}
