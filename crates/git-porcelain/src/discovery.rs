//! Git executable 탐색 (IV.3.1).
//!
//! 탐색 우선순위는 (1) 사용자 지정 **절대** 경로, (2) 알려진 시스템 경로, (3) `PATH` 디렉터리
//! 순서입니다. **shell을 거치지 않고** 경로를 직접 만들어 존재를 확인합니다(shell 해석·`which`
//! 호출 없음).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Git 실행 파일 이름(플랫폼별).
#[cfg(windows)]
pub const GIT_BINARY: &str = "git.exe";
/// Git 실행 파일 이름(플랫폼별).
#[cfg(not(windows))]
pub const GIT_BINARY: &str = "git";

/// 알려진 시스템 Git 경로(우선순위 순서, macOS).
#[cfg(target_os = "macos")]
const KNOWN_SYSTEM_PATHS: &[&str] = &[
    "/usr/bin/git",
    "/usr/local/bin/git",
    "/opt/homebrew/bin/git",
    "/Library/Developer/CommandLineTools/usr/bin/git",
    "/Applications/Xcode.app/Contents/Developer/usr/bin/git",
];

/// 알려진 시스템 Git 경로(우선순위 순서, Windows).
#[cfg(windows)]
const KNOWN_SYSTEM_PATHS: &[&str] = &[
    r"C:\Program Files\Git\cmd\git.exe",
    r"C:\Program Files\Git\bin\git.exe",
    r"C:\Program Files (x86)\Git\cmd\git.exe",
];

/// 알려진 시스템 Git 경로(기타 unix).
#[cfg(all(unix, not(target_os = "macos")))]
const KNOWN_SYSTEM_PATHS: &[&str] = &["/usr/bin/git", "/usr/local/bin/git"];

/// Git 탐색 설정.
#[derive(Debug, Clone, Default)]
pub struct GitDiscoveryConfig {
    /// 사용자 지정 절대 경로(있으면 최우선). 절대 경로가 아니면 무시합니다.
    pub user_path: Option<PathBuf>,
}

/// 우선순위 순서의 후보 경로 목록을 만듭니다(존재 검사 전).
///
/// 순서: 사용자 지정 절대 경로 → 알려진 시스템 경로 → `PATH` 각 디렉터리에 Git 이름을 붙인 경로.
pub fn git_candidates(config: &GitDiscoveryConfig, path_env: Option<&OsStr>) -> Vec<PathBuf> {
    let mut out = Vec::new();

    // 1. 사용자 지정 절대 경로(절대 경로일 때만).
    if let Some(user) = &config.user_path {
        if user.is_absolute() {
            out.push(user.clone());
        }
    }

    // 2. 알려진 시스템 경로.
    for known in KNOWN_SYSTEM_PATHS {
        out.push(PathBuf::from(known));
    }

    // 3. PATH 디렉터리 + Git 이름.
    if let Some(path_env) = path_env {
        for dir in std::env::split_paths(path_env) {
            out.push(dir.join(GIT_BINARY));
        }
    }

    out
}

/// 후보 중 처음으로 존재하는 경로를 고릅니다(존재 검사는 주입).
pub fn select_existing(candidates: &[PathBuf], exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    candidates.iter().find(|p| exists(p)).cloned()
}

/// 실제 파일시스템으로 Git executable을 탐색합니다(없으면 `None`).
pub fn discover_git(config: &GitDiscoveryConfig) -> Option<PathBuf> {
    let path_env = std::env::var_os("PATH");
    let candidates = git_candidates(config, path_env.as_deref());
    select_existing(&candidates, |p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn user_absolute_path_is_first() {
        let user = if cfg!(windows) {
            PathBuf::from(r"C:\custom\git.exe")
        } else {
            PathBuf::from("/custom/git")
        };
        let config = GitDiscoveryConfig {
            user_path: Some(user.clone()),
        };
        let candidates = git_candidates(&config, Some(OsStr::new("")));
        assert_eq!(candidates.first(), Some(&user));
    }

    #[test]
    fn relative_user_path_is_ignored() {
        let config = GitDiscoveryConfig {
            user_path: Some(PathBuf::from("relative/git")),
        };
        let candidates = git_candidates(&config, None);
        assert!(!candidates.contains(&PathBuf::from("relative/git")));
    }

    #[test]
    fn priority_order_user_then_known_then_path() {
        let user = if cfg!(windows) {
            PathBuf::from(r"C:\custom\git.exe")
        } else {
            PathBuf::from("/custom/git")
        };
        let config = GitDiscoveryConfig {
            user_path: Some(user.clone()),
        };
        // PATH에 두 디렉터리.
        let path_env = std::env::join_paths(["/p1", "/p2"].iter().map(PathBuf::from))
            .or_else(|_| std::env::join_paths([r"C:\p1", r"C:\p2"].iter().map(PathBuf::from)))
            .unwrap();
        let candidates = git_candidates(&config, Some(path_env.as_os_str()));

        let user_idx = candidates.iter().position(|p| p == &user).unwrap();
        let known_idx = candidates
            .iter()
            .position(|p| p.ends_with(GIT_BINARY) && p != &user && !p.starts_with("/p"))
            .unwrap();
        let path_idx = candidates
            .iter()
            .rposition(|p| p.file_name() == Some(OsStr::new(GIT_BINARY)))
            .unwrap();
        // 사용자 경로 < 알려진 경로 < PATH 경로.
        assert!(user_idx < known_idx, "user < known");
        assert!(known_idx < path_idx, "known < path");
        // PATH 디렉터리가 Git 이름과 결합되었는지.
        assert!(
            candidates.contains(&PathBuf::from("/p1").join(GIT_BINARY))
                || candidates.contains(&PathBuf::from(r"C:\p1").join(GIT_BINARY))
        );
    }

    #[test]
    fn select_first_existing_respects_priority() {
        let candidates = vec![
            PathBuf::from("/a/git"),
            PathBuf::from("/b/git"),
            PathBuf::from("/c/git"),
        ];
        // /b와 /c가 존재하면 /b(앞)가 선택됨.
        let present: HashSet<PathBuf> = [PathBuf::from("/b/git"), PathBuf::from("/c/git")]
            .into_iter()
            .collect();
        let chosen = select_existing(&candidates, |p| present.contains(p));
        assert_eq!(chosen, Some(PathBuf::from("/b/git")));
        // 아무 것도 없으면 None.
        assert_eq!(select_existing(&candidates, |_| false), None);
    }

    #[test]
    fn no_path_env_still_yields_known_paths() {
        let candidates = git_candidates(&GitDiscoveryConfig::default(), None);
        assert!(!candidates.is_empty());
        // 알려진 경로만 포함(PATH 없음).
        assert!(candidates.iter().all(|p| p.is_absolute()));
    }
}
