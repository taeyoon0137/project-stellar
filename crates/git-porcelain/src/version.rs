//! Git 버전 probe (IV.3.2).
//!
//! 발견한 Git executable을 `--version` argv로 실행(shell 미사용)해 semantic version을 파싱하고
//! raw output을 보존합니다. **최소 버전은 강제하지 않습니다**(지원 버전 정책은 IV.3.5).

use std::path::Path;
use std::process::Command;

/// 파싱한 Git semantic version과 raw output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitVersion {
    /// major.
    pub major: u32,
    /// minor.
    pub minor: u32,
    /// patch(없으면 0).
    pub patch: u32,
    /// `git --version`의 raw stdout(trim).
    pub raw: String,
}

/// 버전 probe 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionProbeError {
    /// executable 실행 실패(없음·권한 등).
    SpawnFailed {
        /// io 오류 종류.
        kind: std::io::ErrorKind,
    },
    /// 비영 종료.
    NonZeroExit {
        /// 종료 코드(없으면 시그널 종료).
        code: Option<i32>,
        /// stderr(lossy).
        stderr: String,
    },
    /// `git version ...` 형식이 아니라 파싱 불가.
    Unparseable {
        /// raw output(보존).
        raw: String,
    },
}

impl std::fmt::Display for VersionProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VersionProbeError::SpawnFailed { kind } => {
                write!(f, "Git 실행 실패: {kind:?}")
            }
            VersionProbeError::NonZeroExit { code, .. } => {
                write!(f, "Git --version 비영 종료: code={code:?}")
            }
            VersionProbeError::Unparseable { raw } => {
                write!(f, "Git 버전 파싱 불가: {raw:?}")
            }
        }
    }
}

impl std::error::Error for VersionProbeError {}

/// `git --version` 출력에서 major.minor.patch를 파싱합니다(형식 불일치는 `None`).
///
/// 예: `git version 2.39.3 (Apple Git-145)` → `(2, 39, 3)`,
/// `git version 2.42.0.windows.2` → `(2, 42, 0)`, `git version 2.39` → `(2, 39, 0)`.
pub fn parse_git_version_output(raw: &str) -> Option<(u32, u32, u32)> {
    let rest = raw.trim().strip_prefix("git version ")?;
    let token = rest.split_whitespace().next()?;
    let mut parts = token.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    Some((major, minor, patch))
}

/// Git executable을 `--version`으로 실행해 버전을 probe합니다.
pub fn probe_git_version(git_path: &Path) -> Result<GitVersion, VersionProbeError> {
    let output = Command::new(git_path)
        .arg("--version")
        .output()
        .map_err(|e| VersionProbeError::SpawnFailed { kind: e.kind() })?;

    if !output.status.success() {
        return Err(VersionProbeError::NonZeroExit {
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    match parse_git_version_output(&raw) {
        Some((major, minor, patch)) => Ok(GitVersion {
            major,
            minor,
            patch,
            raw,
        }),
        None => Err(VersionProbeError::Unparseable { raw }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::{discover_git, GitDiscoveryConfig};
    use std::path::PathBuf;

    #[test]
    fn parses_common_version_strings() {
        assert_eq!(
            parse_git_version_output("git version 2.39.3"),
            Some((2, 39, 3))
        );
        assert_eq!(
            parse_git_version_output("git version 2.39.3 (Apple Git-145)"),
            Some((2, 39, 3))
        );
        assert_eq!(
            parse_git_version_output("git version 2.42.0.windows.2"),
            Some((2, 42, 0))
        );
        assert_eq!(
            parse_git_version_output("git version 2.39"),
            Some((2, 39, 0))
        );
        // trailing newline/공백 허용.
        assert_eq!(
            parse_git_version_output("  git version 2.40.1\n"),
            Some((2, 40, 1))
        );
    }

    #[test]
    fn rejects_unparseable_strings() {
        for raw in [
            "",
            "not git output",
            "git version x.y.z",
            "version 2.39.3",
            "git version ",
        ] {
            assert_eq!(parse_git_version_output(raw), None, "입력: {raw:?}");
        }
    }

    #[test]
    fn probe_normal_git_when_available() {
        // DEV-MAC/CI에는 git이 설치돼 있음. 발견되면 정상 probe.
        if let Some(git) = discover_git(&GitDiscoveryConfig::default()) {
            let version = probe_git_version(&git).expect("정상 git probe");
            assert!(version.major >= 1, "버전: {version:?}");
            assert!(
                version.raw.contains("git version"),
                "raw 보존: {:?}",
                version.raw
            );
        }
    }

    #[test]
    fn probe_missing_executable_is_spawn_failed() {
        let missing = PathBuf::from("/nonexistent/definitely/not/here/git");
        match probe_git_version(&missing) {
            Err(VersionProbeError::SpawnFailed { .. }) => {}
            other => panic!("예상과 다른 결과: {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn probe_non_git_executable_is_unparseable() {
        // /bin/echo --version → "--version" 출력(비-git) → Unparseable.
        let echo = PathBuf::from("/bin/echo");
        match probe_git_version(&echo) {
            Err(VersionProbeError::Unparseable { raw }) => {
                assert!(raw.contains("--version"), "raw 보존: {raw:?}");
            }
            other => panic!("예상과 다른 결과: {other:?}"),
        }
    }
}
