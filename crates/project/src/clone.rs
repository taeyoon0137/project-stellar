//! 프로젝트 clone 요청과 민감 URL (IV.6.1).
//!
//! [`CloneRequest`]는 clone 입력(Git URL·부모 디렉터리·폴더 이름)을 담습니다. Git URL은
//! 자격증명·비공개 경로를 포함할 수 있으므로 **민감값**([`SensitiveUrl`])으로 취급합니다.
//! `Debug`/`Display`는 원문을 노출하지 않고 host 힌트만 보여주며, 원문은 `expose()`로만
//! 접근합니다(실제 git 실행 시에만). 따라서 로그·telemetry·보고서에 URL 원문이 새지 않습니다.

use std::fmt;
use std::path::PathBuf;

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
