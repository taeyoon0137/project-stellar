//! Git LFS 설치 감지 (XIII.7.1).
//!
//! 시스템 Git 환경에서 `git lfs version`을 실행해 Git LFS 설치 상태와 버전을 읽습니다. Git이
//! 실제로 찾는 `git-lfs`(exec-path·PATH의 dashed external command)를 그대로 쓰므로, Stellar가 LFS
//! executable 경로를 직접 추측하지 않습니다.
//!
//! - 종료 코드 0 + `git-lfs/<major>.<minor>[.<patch>] ...` → [`LfsInstallation::Installed`].
//! - 종료 코드 0이지만 형식이 다름 → [`LfsInstallation::Unrecognized`](raw 보존).
//! - 비영 종료(예: `git: 'lfs' is not a git command`) → [`LfsInstallation::NotInstalled`]
//!   (종료 코드·stderr 보존). stderr 문구는 locale에 따라 달라지므로 해석하지 않습니다.
//!
//! 저장소 안에서 실행하면 LFS 미설치 시 저장소 config의 `alias.lfs`가 실행될 수 있으므로, probe는
//! 저장소가 아닌 중립 디렉터리(시스템 임시 디렉터리)에서 실행합니다.

use std::path::Path;

/// `git check-attr`로 경로 속성 읽기(XIII.7.2~).
pub mod attr;

/// import 후 LFS 권고 경고(XIII.7.2).
pub mod advice;

/// 사용자 승인 후 `git lfs track` 적용(XIII.7.3).
pub mod track;

/// 기존 이력 LFS 변환 미지원(XIII.7.4).
pub mod migrate;

use crate::runner::{argv, GitOutput, GitRunError, GitRunner};

/// Git LFS 버전과 raw output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LfsVersion {
    /// major.
    pub major: u32,
    /// minor.
    pub minor: u32,
    /// patch(없으면 0).
    pub patch: u32,
    /// `git lfs version`의 raw stdout(trim).
    pub raw: String,
}

/// Git LFS 설치 상태.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LfsInstallation {
    /// 설치되어 있고 버전을 읽음.
    Installed {
        /// 확인된 버전.
        version: LfsVersion,
    },
    /// 설치되어 있지 않거나 실행할 수 없음(`git lfs version` 비영 종료).
    NotInstalled {
        /// 종료 코드(시그널 종료면 `None`).
        exit_code: Option<i32>,
        /// stderr 원시 바이트(진단용 보존).
        stderr: Vec<u8>,
    },
    /// 종료 코드 0이지만 `git-lfs/<version>` 형식이 아님.
    Unrecognized {
        /// raw stdout(trim, lossy).
        raw: String,
    },
}

impl LfsInstallation {
    /// 설치되어 버전을 읽었는지.
    pub fn is_installed(&self) -> bool {
        matches!(self, LfsInstallation::Installed { .. })
    }
}

/// `git lfs version` 출력에서 major.minor.patch를 파싱합니다(형식 불일치는 `None`).
///
/// 예: `git-lfs/3.8.1 (GitHub; darwin arm64; go 1.27.3)` → `(3, 8, 1)`,
/// `git-lfs/3.5` → `(3, 5, 0)`.
pub fn parse_lfs_version_output(raw: &str) -> Option<(u32, u32, u32)> {
    let rest = raw.trim().strip_prefix("git-lfs/")?;
    let token = rest.split_whitespace().next()?;
    let mut parts = token.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = match parts.next() {
        Some(p) => p.parse().ok()?,
        None => 0,
    };
    Some((major, minor, patch))
}

/// `git lfs version` 실행 결과를 설치 상태로 분류합니다.
pub fn classify_lfs_output(output: &GitOutput) -> LfsInstallation {
    if !output.success() {
        return LfsInstallation::NotInstalled {
            exit_code: output.exit_code,
            stderr: output.stderr.clone(),
        };
    }
    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    match parse_lfs_version_output(&raw) {
        Some((major, minor, patch)) => LfsInstallation::Installed {
            version: LfsVersion {
                major,
                minor,
                patch,
                raw,
            },
        },
        None => LfsInstallation::Unrecognized { raw },
    }
}

/// 시스템 Git 환경에서 `git lfs version`을 실행해 LFS 설치 상태를 읽습니다(중립 cwd).
pub fn probe_lfs(runner: &GitRunner) -> Result<LfsInstallation, GitRunError> {
    probe_lfs_in(runner, &std::env::temp_dir())
}

/// 지정한 중립 디렉터리에서 저장소 탐색을 끈 채 `git lfs version`을 실행합니다.
fn probe_lfs_in(runner: &GitRunner, neutral_cwd: &Path) -> Result<LfsInstallation, GitRunError> {
    let output = runner
        .isolated_from_repositories(neutral_cwd)
        .run(&argv(["lfs", "version"]), neutral_cwd)?;
    Ok(classify_lfs_output(&output))
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

    const NOT_A_GIT_COMMAND: &str = "git: 'lfs' is not a git command. See 'git --help'.\n";

    #[test]
    fn parses_lfs_version_strings() {
        assert_eq!(
            parse_lfs_version_output("git-lfs/3.8.1 (GitHub; darwin arm64; go 1.27.3)"),
            Some((3, 8, 1))
        );
        assert_eq!(
            parse_lfs_version_output(
                "git-lfs/2.13.3 (GitHub; windows amd64; go 1.16.2; git a5e65851)\n"
            ),
            Some((2, 13, 3))
        );
        assert_eq!(parse_lfs_version_output("git-lfs/3.5"), Some((3, 5, 0)));
        for raw in [
            "",
            "git-lfs",
            "git-lfs/",
            "git-lfs/x.y.z",
            "git-lfs/3",
            "git-lfs/3.5.x",
            "lfs/3.0.0",
            "git version 2.39.3",
        ] {
            assert_eq!(parse_lfs_version_output(raw), None, "입력: {raw:?}");
        }
    }

    #[test]
    fn installed_fixture_output_is_installed_with_version() {
        let out = output(0, "git-lfs/3.8.1 (GitHub; darwin arm64; go 1.27.3)\n", "");
        let state = classify_lfs_output(&out);
        assert_eq!(
            state,
            LfsInstallation::Installed {
                version: LfsVersion {
                    major: 3,
                    minor: 8,
                    patch: 1,
                    raw: "git-lfs/3.8.1 (GitHub; darwin arm64; go 1.27.3)".to_string(),
                }
            }
        );
        assert!(state.is_installed());
    }

    #[test]
    fn not_installed_fixture_output_preserves_exit_and_stderr() {
        let out = output(1, "", NOT_A_GIT_COMMAND);
        let state = classify_lfs_output(&out);
        assert_eq!(
            state,
            LfsInstallation::NotInstalled {
                exit_code: Some(1),
                stderr: NOT_A_GIT_COMMAND.as_bytes().to_vec(),
            }
        );
        assert!(!state.is_installed());
    }

    #[test]
    fn unrecognized_success_output_is_not_installed_state() {
        let state = classify_lfs_output(&output(0, "something else\n", ""));
        assert_eq!(
            state,
            LfsInstallation::Unrecognized {
                raw: "something else".to_string()
            }
        );
        assert!(!state.is_installed());
    }

    #[cfg(unix)]
    mod processes {
        use super::*;
        use crate::test_support::{system_git, TempDir};
        use std::ffi::OsString;
        use std::os::unix::fs::PermissionsExt;
        use std::path::PathBuf;
        use std::time::Duration;

        fn write_executable(path: &Path, body: &str) {
            std::fs::write(path, body).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        /// PATH를 `path_dir` 하나로 고정한 시스템 Git 실행기.
        fn git_with_path(git: PathBuf, path_dir: &Path) -> GitRunner {
            GitRunner::new(git, Vec::new(), Duration::from_secs(30)).with_env_overrides(vec![
                (OsString::from("PATH"), path_dir.as_os_str().to_os_string()),
                (
                    OsString::from("GIT_CONFIG_GLOBAL"),
                    OsString::from("/dev/null"),
                ),
                (
                    OsString::from("GIT_CONFIG_SYSTEM"),
                    OsString::from("/dev/null"),
                ),
            ])
        }

        /// Git exec-path에 git-lfs가 함께 설치된 배포판이면 PATH만으로 미설치를 만들 수 없습니다.
        fn exec_path_has_lfs(git: &Path) -> bool {
            let out = std::process::Command::new(git)
                .arg("--exec-path")
                .output()
                .unwrap();
            let dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
            Path::new(&dir).join("git-lfs").exists()
        }

        #[test]
        fn real_git_with_lfs_on_path_is_installed() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            if exec_path_has_lfs(&git) {
                return; // exec-path의 git-lfs가 PATH보다 먼저 쓰이는 배포판은 건너뜀.
            }
            let dir = TempDir::new("lfs-installed");
            // 실제 Git이 PATH에서 찾는 dashed external command `git-lfs` fixture.
            write_executable(
                &dir.path().join("git-lfs"),
                "#!/bin/sh\necho 'git-lfs/9.8.7 (fixture; test)'\n",
            );
            let runner = git_with_path(git, dir.path());
            let state = probe_lfs(&runner).expect("probe");
            assert_eq!(
                state,
                LfsInstallation::Installed {
                    version: LfsVersion {
                        major: 9,
                        minor: 8,
                        patch: 7,
                        raw: "git-lfs/9.8.7 (fixture; test)".to_string(),
                    }
                }
            );
        }

        #[test]
        fn real_git_without_lfs_on_path_is_not_installed() {
            let Some(git) = system_git() else {
                return;
            };
            if exec_path_has_lfs(&git) {
                return;
            }
            let dir = TempDir::new("lfs-missing");
            let empty = dir.path().join("empty-bin");
            std::fs::create_dir_all(&empty).unwrap();
            let runner = git_with_path(git, &empty);
            match probe_lfs(&runner).expect("probe") {
                LfsInstallation::NotInstalled { exit_code, stderr } => {
                    assert_ne!(exit_code, Some(0));
                    assert!(!stderr.is_empty(), "stderr 보존");
                }
                other => panic!("예상과 다른 결과: {other:?}"),
            }
            assert_eq!(runner.spawn_count(), 1);
        }

        #[test]
        fn fake_git_fixtures_for_both_states() {
            let dir = TempDir::new("lfs-fake");
            let installed = dir.path().join("git-installed");
            write_executable(
                &installed,
                "#!/bin/sh\n[ \"$1\" = \"lfs\" ] && echo 'git-lfs/3.4.1 (GitHub; linux amd64; go 1.21.1)' && exit 0\nexit 2\n",
            );
            let missing = dir.path().join("git-missing");
            write_executable(
                &missing,
                "#!/bin/sh\necho \"git: 'lfs' is not a git command. See 'git --help'.\" >&2\nexit 1\n",
            );

            let runner = GitRunner::new(installed, Vec::new(), Duration::from_secs(10));
            let state = probe_lfs(&runner).expect("probe");
            assert!(state.is_installed());
            match state {
                LfsInstallation::Installed { version } => {
                    assert_eq!((version.major, version.minor, version.patch), (3, 4, 1));
                }
                other => panic!("예상과 다른 결과: {other:?}"),
            }

            let runner = GitRunner::new(missing, Vec::new(), Duration::from_secs(10));
            assert_eq!(
                probe_lfs(&runner).expect("probe"),
                LfsInstallation::NotInstalled {
                    exit_code: Some(1),
                    stderr: b"git: 'lfs' is not a git command. See 'git --help'.\n".to_vec(),
                }
            );
        }

        /// `alias.lfs`가 실행되면 marker를 만드는 저장소(`repo/sub` 하위 디렉터리 포함).
        fn repo_with_lfs_alias(git: &Path, dir: &Path, marker: &Path) -> PathBuf {
            let repo = dir.join("repo");
            std::fs::create_dir_all(repo.join("sub")).unwrap();
            let setup = git_with_path(git.to_path_buf(), dir);
            let alias = format!("!/usr/bin/touch '{}'", marker.display());
            for args in [
                vec!["init", "-q"],
                vec!["config", "alias.lfs", alias.as_str()],
            ] {
                assert!(setup.run(&argv(args), &repo).unwrap().success());
            }
            repo
        }

        #[test]
        fn probe_inside_repository_does_not_discover_it() {
            let Some(git) = system_git() else {
                return;
            };
            if exec_path_has_lfs(&git) {
                return; // exec-path의 git-lfs가 alias보다 먼저 쓰이는 배포판은 건너뜀.
            }
            let dir = TempDir::new("lfs-guard");
            let empty = dir.path().join("empty-bin");
            std::fs::create_dir_all(&empty).unwrap();
            let marker = dir.path().join("PWNED_alias");
            let repo = repo_with_lfs_alias(&git, dir.path(), &marker);
            let runner = git_with_path(git, &empty);

            // 대조군: guard 없이 저장소 안에서 실행하면 저장소 alias가 실행됨(fixture가 실제로 악성).
            runner.run(&argv(["lfs", "version"]), &repo).unwrap();
            assert!(marker.exists(), "대조군에서 alias가 실행되지 않음");
            std::fs::remove_file(&marker).unwrap();

            // 중립 cwd가 저장소 root이거나 그 하위여도 probe는 저장소를 발견하지 않음.
            for cwd in [repo.clone(), repo.join("sub")] {
                let state = probe_lfs_in(&runner, &cwd).expect("probe");
                assert!(
                    matches!(state, LfsInstallation::NotInstalled { .. }),
                    "{state:?}"
                );
                assert!(!marker.exists(), "probe가 저장소 alias를 실행함: {cwd:?}");
            }
        }

        #[test]
        fn missing_git_executable_is_run_error_not_state() {
            let runner = GitRunner::new(
                PathBuf::from("/nonexistent/git"),
                Vec::new(),
                Duration::from_secs(5),
            );
            assert!(matches!(probe_lfs(&runner), Err(GitRunError::Spawn { .. })));
        }
    }
}
