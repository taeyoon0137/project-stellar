//! Git capability probe (XIV.1.3).
//!
//! 시스템 Git이 제공하는 기능을 [`GitCapabilities`]로 기록합니다. 초기 UI가 쓰지 않는 기능도
//! capability로만 저장하며, 기능 사용 여부 결정은 호출자가 합니다.
//!
//! - Git 내장 기능(porcelain v2, pathspec-from-file, `update-index --index-info`, worktree)은
//!   `git --version`(IV.3.2 parser)의 버전과 기능 도입 버전표로 판정합니다. 도움말 문구처럼
//!   locale·버전마다 바뀌는 출력은 해석하지 않습니다.
//! - LFS는 Git 버전과 무관한 별도 도구이므로 `git lfs version` 실행 결과로 판정합니다.
//!
//! probe는 저장소가 아닌 **중립 디렉터리**(시스템 임시 디렉터리)에서 실행합니다. 저장소 안에서
//! 실행하면 LFS 미설치 시 저장소 config의 alias가 실행될 수 있기 때문입니다.

use std::path::Path;

use crate::command::{GitCommandFailure, GitCommandResult};
use crate::runner::{argv, GitOutput, GitRunError, GitRunner};
use crate::version::{parse_git_version_output, GitVersion};

/// 기록하는 Git capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GitCapability {
    /// `git status --porcelain=v2`.
    StatusPorcelainV2,
    /// `--pathspec-from-file`(add·reset·commit·rm 등).
    PathspecFromFile,
    /// `git update-index --index-info`.
    UpdateIndexIndexInfo,
    /// `git worktree`(add·`list --porcelain`).
    Worktree,
    /// Git LFS(`git lfs`) 사용 가능.
    Lfs,
}

impl GitCapability {
    /// 정의된 모든 capability(고정 순서).
    pub const ALL: [GitCapability; 5] = [
        GitCapability::StatusPorcelainV2,
        GitCapability::PathspecFromFile,
        GitCapability::UpdateIndexIndexInfo,
        GitCapability::Worktree,
        GitCapability::Lfs,
    ];

    /// 안정적인 이름(진단·저장용).
    pub fn as_str(self) -> &'static str {
        match self {
            GitCapability::StatusPorcelainV2 => "status-porcelain-v2",
            GitCapability::PathspecFromFile => "pathspec-from-file",
            GitCapability::UpdateIndexIndexInfo => "update-index-index-info",
            GitCapability::Worktree => "worktree",
            GitCapability::Lfs => "lfs",
        }
    }

    /// Git 내장 기능의 최소 도입 버전(major, minor, patch). LFS는 별도 도구라 `None`.
    ///
    /// - porcelain v2: Git 2.11.0.
    /// - pathspec-from-file: 2.25에서 일부 명령, 2.26에서 rm·stash까지 확장 → 보수적으로 2.26.0.
    /// - `update-index --index-info`: Git 1.x부터 존재 → 보수적으로 2.0.0.
    /// - worktree: `git worktree`는 2.5, 기계 판독용 `list --porcelain`은 2.7 → 2.7.0.
    pub fn min_git_version(self) -> Option<(u32, u32, u32)> {
        match self {
            GitCapability::StatusPorcelainV2 => Some((2, 11, 0)),
            GitCapability::PathspecFromFile => Some((2, 26, 0)),
            GitCapability::UpdateIndexIndexInfo => Some((2, 0, 0)),
            GitCapability::Worktree => Some((2, 7, 0)),
            GitCapability::Lfs => None,
        }
    }
}

/// probe한 Git capability 기록.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCapabilities {
    version: GitVersion,
    supported: Vec<GitCapability>,
}

impl GitCapabilities {
    /// 버전과 LFS 사용 가능 여부로 capability를 판정합니다.
    pub fn from_version(version: GitVersion, lfs_available: bool) -> GitCapabilities {
        let current = (version.major, version.minor, version.patch);
        let supported = GitCapability::ALL
            .into_iter()
            .filter(|cap| match cap.min_git_version() {
                Some(min) => current >= min,
                None => lfs_available,
            })
            .collect();
        GitCapabilities { version, supported }
    }

    /// `git --version`과 `git lfs version`의 원시 출력에서 capability를 판정합니다.
    pub fn from_outputs(
        version_output: &GitOutput,
        lfs_output: &GitOutput,
    ) -> Result<GitCapabilities, CapabilityProbeError> {
        let result = GitCommandResult::from(version_output.clone())
            .require_success()
            .map_err(CapabilityProbeError::VersionFailed)?;
        let raw = result.stdout_lossy().trim().to_string();
        let (major, minor, patch) = parse_git_version_output(&raw)
            .ok_or_else(|| CapabilityProbeError::VersionUnparseable { raw: raw.clone() })?;
        let version = GitVersion {
            major,
            minor,
            patch,
            raw,
        };
        Ok(GitCapabilities::from_version(
            version,
            lfs_available_from_output(lfs_output),
        ))
    }

    /// probe한 Git 버전.
    pub fn version(&self) -> &GitVersion {
        &self.version
    }

    /// capability 지원 여부.
    pub fn supports(&self, capability: GitCapability) -> bool {
        self.supported.contains(&capability)
    }

    /// 지원하는 capability 목록([`GitCapability::ALL`] 순서).
    pub fn supported(&self) -> &[GitCapability] {
        &self.supported
    }
}

/// `git lfs version` 결과가 LFS 사용 가능을 뜻하는지(종료 코드 0 + `git-lfs/` 출력).
pub fn lfs_available_from_output(output: &GitOutput) -> bool {
    output.success() && output.stdout.starts_with(b"git-lfs/")
}

/// capability probe 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityProbeError {
    /// Git 실행 실패(spawn·timeout 등).
    Run(GitRunError),
    /// `git --version` 비영 종료.
    VersionFailed(GitCommandFailure),
    /// `git --version` 출력 파싱 불가(raw 보존).
    VersionUnparseable {
        /// raw stdout(trim).
        raw: String,
    },
}

impl std::fmt::Display for CapabilityProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CapabilityProbeError::Run(e) => write!(f, "Git 실행 실패: {e}"),
            CapabilityProbeError::VersionFailed(e) => write!(f, "{e}"),
            CapabilityProbeError::VersionUnparseable { raw } => {
                write!(f, "Git 버전 파싱 불가: {raw:?}")
            }
        }
    }
}

impl std::error::Error for CapabilityProbeError {}

/// 시스템 Git의 capability를 probe합니다(중립 디렉터리에서 `--version`, `lfs version` 실행).
pub fn probe_git_capabilities(runner: &GitRunner) -> Result<GitCapabilities, CapabilityProbeError> {
    probe_git_capabilities_in(runner, &std::env::temp_dir())
}

/// 지정한 중립 디렉터리에서 capability를 probe합니다.
fn probe_git_capabilities_in(
    runner: &GitRunner,
    neutral_cwd: &Path,
) -> Result<GitCapabilities, CapabilityProbeError> {
    let version_output = runner
        .run(&argv(["--version"]), neutral_cwd)
        .map_err(CapabilityProbeError::Run)?;
    let lfs_output = runner
        .run(&argv(["lfs", "version"]), neutral_cwd)
        .map_err(CapabilityProbeError::Run)?;
    GitCapabilities::from_outputs(&version_output, &lfs_output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(exit_code: i32, stdout: &str, stderr: &str) -> GitOutput {
        GitOutput {
            exit_code: Some(exit_code),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    const LFS_INSTALLED: &str = "git-lfs/3.8.1 (GitHub; darwin arm64; go 1.27.3)\n";
    const LFS_MISSING_STDERR: &str = "git: 'lfs' is not a git command. See 'git --help'.\n";

    fn caps(version_line: &str, lfs: &GitOutput) -> GitCapabilities {
        GitCapabilities::from_outputs(&output(0, version_line, ""), lfs).expect("parse")
    }

    #[test]
    fn different_git_version_fixtures_parse_to_expected_capabilities() {
        use GitCapability::*;
        let no_lfs = output(1, "", LFS_MISSING_STDERR);
        // (fixture --version 출력, 기대 지원 목록)
        let fixtures: [(&str, &[GitCapability]); 9] = [
            ("git version 1.8.3.1\n", &[]),
            ("git version 2.6.7\n", &[UpdateIndexIndexInfo]),
            ("git version 2.9.5\n", &[UpdateIndexIndexInfo, Worktree]),
            (
                "git version 2.11.0\n",
                &[StatusPorcelainV2, UpdateIndexIndexInfo, Worktree],
            ),
            (
                "git version 2.25.4\n",
                &[StatusPorcelainV2, UpdateIndexIndexInfo, Worktree],
            ),
            (
                "git version 2.26.0\n",
                &[
                    StatusPorcelainV2,
                    PathspecFromFile,
                    UpdateIndexIndexInfo,
                    Worktree,
                ],
            ),
            (
                "git version 2.39.3 (Apple Git-145)\n",
                &[
                    StatusPorcelainV2,
                    PathspecFromFile,
                    UpdateIndexIndexInfo,
                    Worktree,
                ],
            ),
            (
                "git version 2.42.0.windows.2\n",
                &[
                    StatusPorcelainV2,
                    PathspecFromFile,
                    UpdateIndexIndexInfo,
                    Worktree,
                ],
            ),
            (
                "git version 2.54.0 (Apple Git-157)\n",
                &[
                    StatusPorcelainV2,
                    PathspecFromFile,
                    UpdateIndexIndexInfo,
                    Worktree,
                ],
            ),
        ];
        for (line, expected) in fixtures {
            let c = caps(line, &no_lfs);
            assert_eq!(c.supported(), expected, "fixture: {line:?}");
            assert_eq!(c.version().raw, line.trim());
        }
    }

    #[test]
    fn version_boundaries_are_inclusive() {
        let no_lfs = output(1, "", LFS_MISSING_STDERR);
        assert!(!caps("git version 2.10.9", &no_lfs).supports(GitCapability::StatusPorcelainV2));
        assert!(caps("git version 2.11.0", &no_lfs).supports(GitCapability::StatusPorcelainV2));
        assert!(!caps("git version 2.25.9", &no_lfs).supports(GitCapability::PathspecFromFile));
        assert!(caps("git version 2.26.0", &no_lfs).supports(GitCapability::PathspecFromFile));
        assert!(!caps("git version 2.6.9", &no_lfs).supports(GitCapability::Worktree));
        assert!(caps("git version 2.7.0", &no_lfs).supports(GitCapability::Worktree));
    }

    #[test]
    fn lfs_fixture_outputs_parse() {
        let installed = output(0, LFS_INSTALLED, "");
        let installed_windows = output(
            0,
            "git-lfs/2.13.3 (GitHub; windows amd64; go 1.16.2; git a5e65851)\n",
            "",
        );
        let missing = output(1, "", LFS_MISSING_STDERR);
        let weird_success = output(0, "something else\n", "");
        assert!(lfs_available_from_output(&installed));
        assert!(lfs_available_from_output(&installed_windows));
        assert!(!lfs_available_from_output(&missing));
        assert!(!lfs_available_from_output(&weird_success));

        // LFS 여부는 Git 버전과 독립적으로 기록됩니다.
        let c = caps("git version 2.9.5", &installed);
        assert!(c.supports(GitCapability::Lfs));
        assert!(!c.supports(GitCapability::StatusPorcelainV2));
        let c = caps("git version 2.54.0", &missing);
        assert!(!c.supports(GitCapability::Lfs));
    }

    #[test]
    fn version_failures_are_errors() {
        let lfs = output(0, LFS_INSTALLED, "");
        assert!(matches!(
            GitCapabilities::from_outputs(&output(0, "not git\n", ""), &lfs),
            Err(CapabilityProbeError::VersionUnparseable { .. })
        ));
        assert!(matches!(
            GitCapabilities::from_outputs(&output(128, "", "fatal\n"), &lfs),
            Err(CapabilityProbeError::VersionFailed(_))
        ));
    }

    #[test]
    fn capability_names_are_stable_and_unique() {
        let names: Vec<&str> = GitCapability::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(
            names,
            [
                "status-porcelain-v2",
                "pathspec-from-file",
                "update-index-index-info",
                "worktree",
                "lfs"
            ]
        );
    }

    #[cfg(unix)]
    mod processes {
        use super::*;
        use crate::test_support::{system_git, TempDir};
        use std::ffi::OsString;
        use std::os::unix::fs::PermissionsExt;
        use std::path::PathBuf;
        use std::time::Duration;

        /// `--version`과 `lfs version`에 고정 응답하는 가짜 Git executable fixture.
        fn fake_git(dir: &Path, version_line: &str, lfs_installed: bool) -> PathBuf {
            let lfs = if lfs_installed {
                format!("echo '{}'; exit 0", LFS_INSTALLED.trim())
            } else {
                format!("echo \"{}\" >&2; exit 1", LFS_MISSING_STDERR.trim())
            };
            let script = format!(
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo '{version_line}'; exit 0; fi\n\
                 if [ \"$1\" = \"lfs\" ]; then {lfs}; fi\nexit 2\n"
            );
            let path = dir.join("fake-git");
            std::fs::write(&path, script).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        }

        #[test]
        fn fake_git_executables_with_different_outputs() {
            let dir = TempDir::new("cap-fake");
            let old = fake_git(dir.path(), "git version 2.9.5", false);
            let runner = GitRunner::new(old, Vec::new(), Duration::from_secs(10));
            let c = probe_git_capabilities(&runner).expect("probe");
            assert_eq!(
                c.supported(),
                &[GitCapability::UpdateIndexIndexInfo, GitCapability::Worktree]
            );
            assert_eq!(runner.spawn_count(), 2);

            let modern_dir = dir.path().join("modern");
            std::fs::create_dir_all(&modern_dir).unwrap();
            let modern = fake_git(&modern_dir, "git version 2.42.0.windows.2", true);
            let runner = GitRunner::new(modern, Vec::new(), Duration::from_secs(10));
            let c = probe_git_capabilities(&runner).expect("probe");
            assert_eq!(c.supported(), &GitCapability::ALL);
            assert_eq!(c.version().raw, "git version 2.42.0.windows.2");
        }

        #[test]
        fn real_system_git_probe_matches_direct_outputs() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let runner = GitRunner::new(
                git,
                vec![OsString::from("PATH"), OsString::from("HOME")],
                Duration::from_secs(30),
            );
            let c = probe_git_capabilities(&runner).expect("probe");
            let neutral = std::env::temp_dir();
            let version = runner.run(&argv(["--version"]), &neutral).unwrap();
            let lfs = runner.run(&argv(["lfs", "version"]), &neutral).unwrap();
            assert_eq!(
                c.version().raw,
                String::from_utf8_lossy(&version.stdout).trim()
            );
            assert_eq!(
                c.supports(GitCapability::Lfs),
                lfs_available_from_output(&lfs)
            );
            // 이 환경의 최신 Git은 내장 capability를 모두 지원.
            if (c.version().major, c.version().minor) >= (2, 26) {
                for cap in [
                    GitCapability::StatusPorcelainV2,
                    GitCapability::PathspecFromFile,
                    GitCapability::UpdateIndexIndexInfo,
                    GitCapability::Worktree,
                ] {
                    assert!(c.supports(cap), "{cap:?}");
                }
            }
        }
    }
}
