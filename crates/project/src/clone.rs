//! 프로젝트 clone 요청과 민감 URL (IV.6.1).
//!
//! [`CloneRequest`]는 clone 입력(Git URL·부모 디렉터리·폴더 이름)을 담습니다. Git URL은
//! 자격증명·비공개 경로를 포함할 수 있으므로 **민감값**([`SensitiveUrl`])으로 취급합니다.
//! `Debug`/`Display`는 원문을 노출하지 않고 host 힌트만 보여주며, 원문은 `expose()`로만
//! 접근합니다(실제 git 실행 시에만). 따라서 로그·telemetry·보고서에 URL 원문이 새지 않습니다.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use stellar_git_porcelain::repo::{git_clone, RepoError};
use stellar_git_porcelain::runner::GitRunner;

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
}

impl fmt::Display for CloneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CloneError::TargetExists => f.write_str("대상 경로가 이미 존재합니다"),
            CloneError::Clone(e) => write!(f, "git clone 실패: {e}"),
            CloneError::Rename { kind } => write!(f, "최종 폴더 rename 실패: {kind:?}"),
        }
    }
}

impl std::error::Error for CloneError {}

/// 임시 폴더로 clone한 뒤 성공 시 최종 폴더로 rename 합니다(IV.6.2).
///
/// `git clone -- <url> <temp>`를 부모 디렉터리 아래 임시 폴더로 실행하고, **성공하면** 최종
/// 폴더로 rename 합니다. **실패·취소 시** 임시 폴더를 제거하므로 **최종 폴더는 존재하지
/// 않습니다.**
pub fn clone_project(request: &CloneRequest, runner: &GitRunner) -> Result<PathBuf, CloneError> {
    let target = request.target_path();
    if target.exists() {
        return Err(CloneError::TargetExists);
    }
    let temp = unique_temp(&request.parent_dir);

    match git_clone(runner, request.url.expose(), &temp) {
        Ok(()) => {
            if let Err(e) = std::fs::rename(&temp, &target) {
                // rename 실패 시 임시 폴더 정리(최종 폴더는 생기지 않음).
                let _ = std::fs::remove_dir_all(&temp);
                return Err(CloneError::Rename { kind: e.kind() });
            }
            Ok(target)
        }
        Err(e) => {
            // clone 실패: git이 만든 부분 임시 폴더를 제거.
            let _ = std::fs::remove_dir_all(&temp);
            Err(CloneError::Clone(e))
        }
    }
}

/// 부모 아래 고유한 임시 폴더 경로(아직 생성하지 않음; git clone이 생성).
fn unique_temp(parent: &Path) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(".stellar-clone-tmp-{}-{n}", std::process::id()))
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
}
