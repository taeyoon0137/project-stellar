//! 프로젝트 clone 요청과 민감 URL (IV.6.1).
//!
//! [`CloneRequest`]는 clone 입력(Git URL·부모 디렉터리·폴더 이름)을 담습니다. Git URL은
//! 자격증명·비공개 경로를 포함할 수 있으므로 **민감값**([`SensitiveUrl`])으로 취급합니다.
//! `Debug`/`Display`는 원문을 노출하지 않고 host 힌트만 보여주며, 원문은 `expose()`로만
//! 접근합니다(실제 git 실행 시에만). 따라서 로그·telemetry·보고서에 URL 원문이 새지 않습니다.

use std::fmt;
use std::path::PathBuf;

use stellar_git_porcelain::cancel::GitCancelToken;
use stellar_git_porcelain::repo::{git_clone_cancellable, RepoError};
use stellar_git_porcelain::runner::{GitRunError, GitRunner};

use crate::private_temp::{sweep_stale_private_temps, PrivateTempDir, PrivateTempError};
use crate::probe::{probe_project, ProbeError, ProjectProbe};
use crate::root::MANIFEST_FILE;

/// 로그·보고서에 원문을 노출하지 않는 Git URL.
#[derive(Clone, PartialEq, Eq)]
pub struct SensitiveUrl(String);

impl SensitiveUrl {
    /// 원문 URL로 민감 URL을 만듭니다.
    pub fn new(url: impl Into<String>) -> SensitiveUrl {
        SensitiveUrl(url.into())
    }

    /// **원문 URL**을 노출합니다. git 실행 등 꼭 필요한 곳에서만 호출합니다.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// 로그용 redaction 문자열(원문·자격증명·경로 미포함, host 힌트만).
    pub fn redacted(&self) -> String {
        match extract_host(&self.0) {
            Some(host) => format!("[redacted Git URL @ {host}]"),
            None => "[redacted Git URL]".to_string(),
        }
    }
}

impl fmt::Debug for SensitiveUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.redacted())
    }
}

impl fmt::Display for SensitiveUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.redacted())
    }
}

/// URL에서 host만 추출합니다(scheme·userinfo·경로 제외). 추출 실패 시 `None`.
fn extract_host(url: &str) -> Option<&str> {
    // scheme:// 이후, 없으면 scp 유사(git@host:path).
    let after_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    // userinfo(user[:pass]@) 제거.
    let after_userinfo = after_scheme
        .split_once('@')
        .map_or(after_scheme, |(_, rest)| rest);
    // host는 '/' 또는 ':' 전까지.
    let host_end = after_userinfo
        .find(['/', ':'])
        .unwrap_or(after_userinfo.len());
    let host = &after_userinfo[..host_end];
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

/// 프로젝트 clone 요청.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneRequest {
    /// 민감 Git URL.
    pub url: SensitiveUrl,
    /// 부모 디렉터리.
    pub parent_dir: PathBuf,
    /// 최종 폴더 이름.
    pub folder_name: String,
}

impl CloneRequest {
    /// 최종 대상 경로(부모/폴더).
    pub fn target_path(&self) -> PathBuf {
        self.parent_dir.join(&self.folder_name)
    }
}

/// clone 실패.
#[derive(Debug)]
pub enum CloneError {
    /// 최종 대상 경로가 이미 존재함.
    TargetExists,
    /// `git clone` 실패(임시 폴더 정리됨).
    Clone(RepoError),
    /// 임시 → 최종 rename 실패.
    Rename {
        /// io 오류 종류.
        kind: std::io::ErrorKind,
    },
    /// clone된 Stellar repo의 ProjectProbe 실패(손상 매니페스트 등, IV.6.4).
    Probe(ProbeError),
    /// 비공개 staging 디렉터리를 만들지 못함(XVI.7.5, git 미실행).
    Staging(PrivateTempError),
}

impl fmt::Display for CloneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CloneError::TargetExists => f.write_str("대상 경로가 이미 존재합니다"),
            CloneError::Clone(e) => write!(f, "git clone 실패: {e}"),
            CloneError::Rename { kind } => write!(f, "최종 폴더 rename 실패: {kind:?}"),
            CloneError::Probe(e) => write!(f, "clone된 프로젝트 probe 실패: {e}"),
            CloneError::Staging(e) => write!(f, "clone staging 디렉터리 생성 실패: {e}"),
        }
    }
}

impl std::error::Error for CloneError {}

/// clone staging 디렉터리 접두사(부모 디렉터리 아래, [`PrivateTempDir`]).
pub const CLONE_STAGING_PREFIX: &str = ".stellar-clone-tmp";

/// staging 디렉터리 안에서 git이 만드는 checkout 이름.
const STAGING_CHECKOUT: &str = "checkout";

/// 임시 폴더로 clone한 뒤 성공 시 최종 폴더로 rename 합니다(IV.6.2, XVI.7.5).
///
/// 부모 디렉터리 아래에 소유자 전용(`0700`) staging 디렉터리를 만들고 그 안으로
/// `git clone -- <url> <staging>/checkout`을 실행합니다. clone 중의 내용(비공개 저장소일 수 있음)은
/// 다른 사용자가 목록·내용 어디에도 접근할 수 없습니다. **성공하면** checkout을 최종 폴더로 rename
/// 합니다(같은 파일 시스템). **실패·취소·panic 시** staging을 강화 정리(읽기 전용 하위 디렉터리 포함)로
/// 지우므로 **최종 폴더는 존재하지 않습니다.** 시작할 때 비정상 종료로 남은 이전 staging
/// ([`sweep_stale_private_temps`])을 먼저 정리합니다.
pub fn clone_project(request: &CloneRequest, runner: &GitRunner) -> Result<PathBuf, CloneError> {
    clone_project_cancellable(request, runner, &GitCancelToken::new())
}

/// Git 종료와 최종 폴더 공개 전까지 취소할 수 있는 clone입니다.
pub fn clone_project_cancellable(
    request: &CloneRequest,
    runner: &GitRunner,
    cancel: &GitCancelToken,
) -> Result<PathBuf, CloneError> {
    let target = request.target_path();
    if std::fs::symlink_metadata(&target).is_ok() {
        return Err(CloneError::TargetExists);
    }
    // 이전 비정상 종료의 잔여 staging 정리(실패해도 clone은 계속).
    let _ = sweep_stale_private_temps(&request.parent_dir, CLONE_STAGING_PREFIX);
    let staging = PrivateTempDir::create_in(&request.parent_dir, CLONE_STAGING_PREFIX)
        .map_err(CloneError::Staging)?;
    let checkout = staging.path().join(STAGING_CHECKOUT);

    git_clone_cancellable(runner, request.url.expose(), &checkout, cancel)
        .map_err(CloneError::Clone)?;
    if cancel.is_cancelled() {
        return Err(CloneError::Clone(RepoError::Run(GitRunError::Cancelled)));
    }
    publish_checkout(&checkout, &target).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => CloneError::TargetExists,
        kind => CloneError::Rename { kind },
    })?;
    let _ = staging.cleanup();
    Ok(target)
}

// std::fs::rename은 기존의 빈 디렉터리를 대체할 수 있습니다.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn publish_checkout(source: &std::path::Path, target: &std::path::Path) -> std::io::Result<()> {
    use std::ffi::{c_char, c_int, c_uint, CString};
    use std::os::unix::ffi::OsStrExt;
    let source = CString::new(source.as_os_str().as_bytes())?;
    let target = CString::new(target.as_os_str().as_bytes())?;
    #[cfg(target_os = "macos")]
    unsafe extern "C" {
        fn renamex_np(from: *const c_char, to: *const c_char, flags: c_uint) -> c_int;
    }
    #[cfg(target_os = "linux")]
    unsafe extern "C" {
        fn renameat2(
            from_dir: c_int,
            from: *const c_char,
            to_dir: c_int,
            to: *const c_char,
            flags: c_uint,
        ) -> c_int;
    }
    // SAFETY: 시스템 호출이 끝날 때까지 두 경로는 유효한 NUL 종단 문자열입니다.
    #[cfg(target_os = "macos")]
    let status = unsafe {
        renamex_np(source.as_ptr(), target.as_ptr(), 4 /* RENAME_EXCL */)
    };
    #[cfg(target_os = "linux")]
    let status = unsafe {
        renameat2(
            -100,
            source.as_ptr(),
            -100,
            target.as_ptr(),
            1, /* RENAME_NOREPLACE */
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn publish_checkout(source: &std::path::Path, target: &std::path::Path) -> std::io::Result<()> {
    std::fs::rename(source, target)
}

/// clone 후 열기 분류 결과.
#[derive(Debug)]
pub enum ClonedProject {
    /// 유효한 Stellar 프로젝트(ProjectProbe 통과, IV.6.4).
    StellarProject {
        /// clone된 경로.
        path: PathBuf,
        /// probe 결과(매니페스트 포함).
        probe: ProjectProbe,
    },
    /// `stellar.yaml`이 없어 Stellar 프로젝트가 아님(IV.6.3). clone 결과는 **보존**되며,
    /// 보존·삭제 선택은 호출자 몫입니다(자동 변환·삭제 없음).
    NotAStellarProject {
        /// clone된(보존된) 경로.
        path: PathBuf,
    },
}

/// clone한 뒤 열기를 분류합니다(IV.6.3·IV.6.4).
///
/// - clone 결과에 `stellar.yaml`이 없으면 열기를 거부하고 `NotAStellarProject`를 반환합니다
///   (clone 결과 **보존**, 보존·삭제는 호출자가 선택; **자동 변환 금지**).
/// - `stellar.yaml`이 있으면 [`probe_project`](IV.5.1)를 **통과해야만** `StellarProject`로
///   반환합니다. 손상 매니페스트는 `CloneError::Probe`로 실패합니다(IV.6.4).
pub fn clone_and_open(
    request: &CloneRequest,
    runner: &GitRunner,
) -> Result<ClonedProject, CloneError> {
    let path = clone_project(request, runner)?;

    // 자동 변환 금지: stellar.yaml이 없으면 상태로 반환하고 결과를 보존.
    if !path.join(MANIFEST_FILE).is_file() {
        return Ok(ClonedProject::NotAStellarProject { path });
    }

    // 유효 Stellar repo는 ProjectProbe 통과 후에만 성공(IV.6.4).
    match probe_project(&path) {
        Ok(probe) => Ok(ClonedProject::StellarProject { path, probe }),
        Err(e) => Err(CloneError::Probe(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_host_from_various_url_forms() {
        assert_eq!(
            extract_host("https://github.com/org/repo.git"),
            Some("github.com")
        );
        assert_eq!(
            extract_host("https://user:pass@github.com/org/secret.git"),
            Some("github.com")
        );
        assert_eq!(
            extract_host("git@github.com:org/repo.git"),
            Some("github.com")
        );
        assert_eq!(
            extract_host("ssh://git@example.com:22/x/y.git"),
            Some("example.com")
        );
    }

    #[test]
    fn debug_and_display_do_not_leak_raw_url() {
        // 비밀 경로·토큰이 든 URL.
        let raw = "https://x-token:SECRET123@github.com/private/super-secret-repo.git";
        let url = SensitiveUrl::new(raw);

        for rendered in [format!("{url:?}"), format!("{url}"), url.redacted()] {
            assert!(!rendered.contains("SECRET123"), "토큰 노출: {rendered}");
            assert!(
                !rendered.contains("super-secret-repo"),
                "경로 노출: {rendered}"
            );
            assert!(!rendered.contains(raw), "원문 노출: {rendered}");
            // host 힌트는 허용.
            assert!(rendered.contains("github.com"));
        }
    }

    #[test]
    fn clone_request_debug_does_not_leak_url() {
        let req = CloneRequest {
            url: SensitiveUrl::new("https://token:abc@host.example/p/secret.git"),
            parent_dir: PathBuf::from("/tmp/parent"),
            folder_name: "proj".to_string(),
        };
        let debug = format!("{req:?}");
        assert!(!debug.contains("token:abc"), "자격증명 노출: {debug}");
        assert!(!debug.contains("secret.git"), "경로 노출: {debug}");
    }

    #[test]
    fn expose_returns_raw_for_git_use() {
        let raw = "https://github.com/org/repo.git";
        let url = SensitiveUrl::new(raw);
        // 실제 git 실행용 원문 접근.
        assert_eq!(url.expose(), raw);
    }

    #[test]
    fn target_path_joins_parent_and_folder() {
        let req = CloneRequest {
            url: SensitiveUrl::new("https://github.com/o/r.git"),
            parent_dir: PathBuf::from("/a/b"),
            folder_name: "c".to_string(),
        };
        assert_eq!(req.target_path(), PathBuf::from("/a/b/c"));
    }
}

// --- IV.6.2: git clone + rename (실제 git, 로컬 repo, unix) ---
#[cfg(all(test, unix))]
mod git_clone_tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use stellar_git_porcelain::discovery::{discover_git, GitDiscoveryConfig};
    use stellar_git_porcelain::runner::argv;

    fn temp_dir() -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("stellar-clone-test-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn hermetic_runner(git: PathBuf) -> GitRunner {
        GitRunner::new(
            git,
            vec![OsString::from("PATH"), OsString::from("HOME")],
            Duration::from_secs(60),
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

    fn make_source_repo(runner: &GitRunner, dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        runner.run(&argv(["init"]), dir).unwrap();
        runner
            .run(&argv(["config", "user.name", "Test"]), dir)
            .unwrap();
        runner
            .run(&argv(["config", "user.email", "t@e"]), dir)
            .unwrap();
        std::fs::write(dir.join("file.txt"), b"hello\n").unwrap();
        runner.run(&argv(["add", "--all"]), dir).unwrap();
        runner.run(&argv(["commit", "-m", "init"]), dir).unwrap();
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn final_publication_preserves_an_empty_directory_created_during_clone() {
        use std::os::unix::fs::MetadataExt;
        let root = temp_dir();
        let target = root.join("target");
        let checkout = root.join("checkout");
        std::fs::create_dir(&checkout).unwrap();
        std::fs::write(checkout.join("incoming"), b"incoming").unwrap();
        std::fs::create_dir(&target).unwrap();
        let inode = std::fs::metadata(&target).unwrap().ino();
        assert_eq!(
            publish_checkout(&checkout, &target).unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::metadata(&target).unwrap().ino(), inode);
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
        assert!(checkout.join("incoming").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clone_success_renames_to_final() {
        let Some(git) = discover_git(&GitDiscoveryConfig::default()) else {
            return;
        };
        let runner = hermetic_runner(git);
        let work = temp_dir();
        let source = work.join("source");
        make_source_repo(&runner, &source);
        let parent = work.join("dest-parent");
        std::fs::create_dir_all(&parent).unwrap();

        let req = CloneRequest {
            url: SensitiveUrl::new(source.to_string_lossy().into_owned()),
            parent_dir: parent.clone(),
            folder_name: "cloned".to_string(),
        };
        let final_path = clone_project(&req, &runner).unwrap();
        assert_eq!(final_path, parent.join("cloned"));
        assert!(final_path.join(".git").exists());
        assert!(final_path.join("file.txt").is_file());
        // 임시 폴더 잔여 없음(최종 폴더만 존재).
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 1);

        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn clone_failure_leaves_no_final_folder() {
        let Some(git) = discover_git(&GitDiscoveryConfig::default()) else {
            return;
        };
        let runner = hermetic_runner(git);
        let work = temp_dir();
        let parent = work.join("dest-parent");
        std::fs::create_dir_all(&parent).unwrap();

        // 존재하지 않는 source → clone 실패.
        let req = CloneRequest {
            url: SensitiveUrl::new(work.join("nonexistent-repo").to_string_lossy().into_owned()),
            parent_dir: parent.clone(),
            folder_name: "cloned".to_string(),
        };
        let result = clone_project(&req, &runner);
        assert!(matches!(result, Err(CloneError::Clone(_))), "{result:?}");
        // 최종 폴더 없음, 임시 폴더도 정리됨(부모 비어 있음).
        assert!(!parent.join("cloned").exists());
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 0);

        let _ = std::fs::remove_dir_all(&work);
    }

    /// 실제 git 앞에서 clone 대상의 부모(staging) 권한과 목록을 기록하는 대리 실행 파일.
    fn recording_git(real_git: &Path, dir: &Path) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let record = dir.join("staging-record.txt");
        let script = dir.join("recording-git.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nfor last; do :; done\nparent=$(dirname \"$last\")\n\
                 ls -ld \"$parent\" > '{record}'\nls -a \"$parent\" >> '{record}'\n\
                 exec '{git}' \"$@\"\n",
                record = record.display(),
                git = real_git.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        (script, record)
    }

    #[test]
    fn clone_runs_inside_an_owner_only_staging_directory() {
        let Some(git) = discover_git(&GitDiscoveryConfig::default()) else {
            return;
        };
        let work = temp_dir();
        let source = work.join("source");
        make_source_repo(&hermetic_runner(git.clone()), &source);
        let (script, record) = recording_git(&git, &work);
        let runner = hermetic_runner(script);
        let parent = work.join("dest-parent");
        std::fs::create_dir_all(&parent).unwrap();

        let req = CloneRequest {
            url: SensitiveUrl::new(source.to_string_lossy().into_owned()),
            parent_dir: parent.clone(),
            folder_name: "cloned".to_string(),
        };
        let final_path = clone_project(&req, &runner).unwrap();
        // git이 실행되는 동안 clone 대상의 부모는 소유자 전용(drwx------) staging이었음.
        let observed = std::fs::read_to_string(&record).unwrap();
        assert!(observed.starts_with("drwx------"), "{observed}");
        assert!(
            observed.contains(".stellar-clone-tmp-"),
            "staging 안에서 clone: {observed}"
        );
        assert!(
            observed.contains(crate::private_temp::PRIVATE_TEMP_MARKER),
            "{observed}"
        );
        // 끝나면 최종 폴더만 남고 staging은 정리됨.
        assert!(final_path.join("file.txt").is_file());
        let names: Vec<_> = std::fs::read_dir(&parent)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("cloned")]);

        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn clone_sweeps_stale_staging_left_by_a_dead_process() {
        use std::os::unix::fs::PermissionsExt;
        let Some(git) = discover_git(&GitDiscoveryConfig::default()) else {
            return;
        };
        let runner = hermetic_runner(git);
        let work = temp_dir();
        let source = work.join("source");
        make_source_repo(&runner, &source);
        let parent = work.join("dest-parent");
        // 이 host에서 비정상 종료한 process(존재하지 않는 pid)가 남긴 staging.
        let stale = parent.join(format!("{CLONE_STAGING_PREFIX}-99999999-0-0"));
        std::fs::create_dir_all(stale.join("checkout/.git")).unwrap();
        std::fs::write(
            stale.join(crate::private_temp::PRIVATE_TEMP_MARKER),
            crate::private_temp::marker_contents().unwrap(),
        )
        .unwrap();
        std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o700)).unwrap();
        // 사용자 파일은 건드리지 않음.
        std::fs::write(parent.join("notes.txt"), b"user").unwrap();

        let req = CloneRequest {
            url: SensitiveUrl::new(source.to_string_lossy().into_owned()),
            parent_dir: parent.clone(),
            folder_name: "cloned".to_string(),
        };
        clone_project(&req, &runner).unwrap();
        assert!(!stale.exists(), "잔여 staging 정리");
        let mut names: Vec<_> = std::fs::read_dir(&parent)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                std::ffi::OsString::from("cloned"),
                std::ffi::OsString::from("notes.txt")
            ]
        );

        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn clone_plain_repo_is_not_a_stellar_project_and_preserved() {
        let Some(git) = discover_git(&GitDiscoveryConfig::default()) else {
            return;
        };
        let runner = hermetic_runner(git);
        let work = temp_dir();
        // 일반 repo(stellar.yaml 없음).
        let source = work.join("source");
        make_source_repo(&runner, &source);
        let parent = work.join("dest-parent");
        std::fs::create_dir_all(&parent).unwrap();

        let req = CloneRequest {
            url: SensitiveUrl::new(source.to_string_lossy().into_owned()),
            parent_dir: parent.clone(),
            folder_name: "cloned".to_string(),
        };
        match clone_and_open(&req, &runner).unwrap() {
            ClonedProject::NotAStellarProject { path } => {
                // clone 결과 보존(자동 삭제·변환 없음).
                assert!(path.exists());
                assert!(path.join("file.txt").is_file());
                assert!(!path.join("stellar.yaml").exists());
            }
            other => panic!("예상과 다른 결과: {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&work);
    }

    fn make_stellar_source_repo(runner: &GitRunner, dir: &Path, manifest_text: &str) {
        std::fs::create_dir_all(dir).unwrap();
        runner.run(&argv(["init"]), dir).unwrap();
        runner
            .run(&argv(["config", "user.name", "Test"]), dir)
            .unwrap();
        runner
            .run(&argv(["config", "user.email", "t@e"]), dir)
            .unwrap();
        std::fs::write(dir.join("stellar.yaml"), manifest_text).unwrap();
        runner.run(&argv(["add", "--all"]), dir).unwrap();
        runner.run(&argv(["commit", "-m", "init"]), dir).unwrap();
    }

    fn valid_manifest_text() -> String {
        use crate::manifest::{Manifest, ProjectMeta, CURRENT_SCHEMA_VERSION};
        use crate::yaml::write_canonical;
        let manifest = Manifest {
            schema_version: CURRENT_SCHEMA_VERSION,
            project: ProjectMeta {
                id: "0195c530-3acd-7bc1-8414-c213f61cb965".to_string(),
                name: "Cloned Project".to_string(),
            },
            target_profiles: Vec::new(),
            dependencies: Vec::new(),
        };
        write_canonical(&manifest.to_yaml())
    }

    #[test]
    fn valid_stellar_repo_passes_probe() {
        let Some(git) = discover_git(&GitDiscoveryConfig::default()) else {
            return;
        };
        let runner = hermetic_runner(git);
        let work = temp_dir();
        let source = work.join("source");
        make_stellar_source_repo(&runner, &source, &valid_manifest_text());
        let parent = work.join("dest-parent");
        std::fs::create_dir_all(&parent).unwrap();

        let req = CloneRequest {
            url: SensitiveUrl::new(source.to_string_lossy().into_owned()),
            parent_dir: parent.clone(),
            folder_name: "cloned".to_string(),
        };
        match clone_and_open(&req, &runner).unwrap() {
            ClonedProject::StellarProject { path, probe } => {
                assert!(path.join("stellar.yaml").is_file());
                assert_eq!(probe.manifest.project.name, "Cloned Project");
            }
            other => panic!("예상과 다른 결과: {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn corrupt_manifest_clone_fails_probe() {
        let Some(git) = discover_git(&GitDiscoveryConfig::default()) else {
            return;
        };
        let runner = hermetic_runner(git);
        let work = temp_dir();
        let source = work.join("source");
        // anchor가 든 손상 stellar.yaml.
        make_stellar_source_repo(&runner, &source, "schema_version: &a 1\nproject: *a\n");
        let parent = work.join("dest-parent");
        std::fs::create_dir_all(&parent).unwrap();

        let req = CloneRequest {
            url: SensitiveUrl::new(source.to_string_lossy().into_owned()),
            parent_dir: parent.clone(),
            folder_name: "cloned".to_string(),
        };
        let result = clone_and_open(&req, &runner);
        assert!(matches!(result, Err(CloneError::Probe(_))), "{result:?}");

        let _ = std::fs::remove_dir_all(&work);
    }
}
