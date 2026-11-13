//! 자격 증명 위임 (XIV.9.6).
//!
//! HTTPS 자격 증명은 시스템 Git credential helper(macOS Keychain 등), SSH는 사용자 SSH agent와 Git
//! 설정에 **위임**합니다(아키텍처 §24.5·§37.3). Stellar는 token·암호·개인 키를 저장하지 않습니다.
//!
//! - [`CredentialDelegation`]은 remote 명령 실행기에 필요한 환경만 더합니다: helper·ssh·agent가 쓰는
//!   `PATH`·`HOME`·`USER`·`LOGNAME`·`TMPDIR`·`SSH_AUTH_SOCK`(allowlist, 값은 사용자 환경 그대로)과
//!   `GIT_TERMINAL_PROMPT=0`(GUI에는 terminal이 없으므로 Git이 terminal prompt로 멈추지 않게 함).
//!   `credential.helper` 같은 Git config는 설정하지 않으며 token 환경 변수를 만들지 않습니다.
//! - askpass bridge가 필요할 때만 `GIT_ASKPASS`·`SSH_ASKPASS`(+`SSH_ASKPASS_REQUIRE=force`)로 bridge
//!   실행 파일을 지정합니다. bridge가 사용자 입력을 Git에 넘길 때는 [`AskpassSecret`]을 써서 한 번 쓰고
//!   즉시 메모리를 0으로 덮어씁니다(`Debug`·`Display`로 값이 나가지 않음).
//! - remote URL에 자격 증명이 든 경우는 연결 단계(XIV.9.2)에서 거부하고, 오류 텍스트는
//!   [`redact_credentials`](super::redact_credentials)로 가립니다.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::runner::GitRunner;

/// credential helper·SSH agent가 쓰는 환경 변수(값은 사용자 환경에서 그대로 전달).
pub const DELEGATED_ENV: [&str; 6] = ["PATH", "HOME", "USER", "LOGNAME", "TMPDIR", "SSH_AUTH_SOCK"];

/// 자격 증명 위임 정책.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CredentialDelegation {
    askpass_bridge: Option<PathBuf>,
}

impl CredentialDelegation {
    /// 시스템 Git(credential helper·SSH agent)에만 위임합니다.
    pub fn system_git() -> CredentialDelegation {
        CredentialDelegation::default()
    }

    /// prompt가 필요할 때 Git·ssh가 호출할 askpass bridge 실행 파일을 지정합니다.
    pub fn with_askpass_bridge(path: PathBuf) -> CredentialDelegation {
        CredentialDelegation {
            askpass_bridge: Some(path),
        }
    }

    /// 실행기에 더할 allowlist 이름.
    pub fn env_allowlist(&self) -> Vec<OsString> {
        DELEGATED_ENV.iter().map(OsString::from).collect()
    }

    /// 실행기에 더할 명시 환경.
    pub fn env_overrides(&self) -> Vec<(OsString, OsString)> {
        let mut env = vec![(OsString::from("GIT_TERMINAL_PROMPT"), OsString::from("0"))];
        if let Some(bridge) = &self.askpass_bridge {
            for key in ["GIT_ASKPASS", "SSH_ASKPASS"] {
                env.push((OsString::from(key), bridge.as_os_str().to_os_string()));
            }
            env.push((
                OsString::from("SSH_ASKPASS_REQUIRE"),
                OsString::from("force"),
            ));
        }
        env
    }

    /// remote 명령용 실행기를 만듭니다(spawn 계측 카운터 공유).
    pub fn apply(&self, runner: &GitRunner) -> GitRunner {
        runner
            .clone()
            .with_allowed_env(self.env_allowlist())
            .with_added_env(self.env_overrides())
    }
}

/// askpass bridge가 Git에 넘길 비밀값(한 번 쓰고 즉시 0으로 덮어씀).
pub struct AskpassSecret {
    bytes: Vec<u8>,
}

impl AskpassSecret {
    /// 비밀값을 받습니다(호출자는 원본 버퍼를 따로 남기지 않아야 합니다).
    pub fn new(bytes: Vec<u8>) -> AskpassSecret {
        AskpassSecret { bytes }
    }

    /// 값을 `out`(askpass stdout)에 한 번 쓰고 메모리를 덮어씁니다.
    pub fn write_once_to(mut self, out: &mut impl std::io::Write) -> std::io::Result<()> {
        let result = out.write_all(&self.bytes).and_then(|()| out.flush());
        self.wipe();
        result
    }

    /// 메모리를 0으로 덮어씁니다(최적화로 제거되지 않도록 volatile write).
    fn wipe(&mut self) {
        for b in self.bytes.iter_mut() {
            // SAFETY: `b`는 `self.bytes`가 소유한 유효한 `u8` 위치에 대한 고유 참조입니다.
            unsafe { std::ptr::write_volatile(b, 0) };
        }
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
        self.bytes.clear();
    }
}

impl Drop for AskpassSecret {
    fn drop(&mut self) {
        self.wipe();
    }
}

impl std::fmt::Debug for AskpassSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AskpassSecret(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegation_env_has_no_secrets_or_config_writes() {
        let d = CredentialDelegation::system_git();
        let overrides = d.env_overrides();
        assert_eq!(
            overrides,
            vec![(OsString::from("GIT_TERMINAL_PROMPT"), OsString::from("0"))]
        );
        let names: Vec<String> = d
            .env_allowlist()
            .into_iter()
            .map(|n| n.into_string().unwrap())
            .collect();
        assert!(names.contains(&"SSH_AUTH_SOCK".to_string()));
        for forbidden in [
            "GITHUB_TOKEN",
            "GH_TOKEN",
            "GIT_CONFIG_PARAMETERS",
            "GIT_CONFIG_COUNT",
        ] {
            assert!(!names.iter().any(|n| n == forbidden));
            assert!(!overrides.iter().any(|(k, _)| k == forbidden));
        }
        let bridge = CredentialDelegation::with_askpass_bridge(PathBuf::from("/opt/bridge"));
        let keys: Vec<String> = bridge
            .env_overrides()
            .into_iter()
            .map(|(k, _)| k.into_string().unwrap())
            .collect();
        assert_eq!(
            keys,
            [
                "GIT_TERMINAL_PROMPT",
                "GIT_ASKPASS",
                "SSH_ASKPASS",
                "SSH_ASKPASS_REQUIRE"
            ]
        );
    }

    #[test]
    fn delegation_applies_to_runner_and_shares_counter() {
        let runner = GitRunner::new(
            PathBuf::from("/nonexistent/git"),
            Vec::new(),
            std::time::Duration::from_secs(5),
        );
        let applied = CredentialDelegation::system_git().apply(&runner);
        assert_eq!(
            applied.child_env_var("GIT_TERMINAL_PROMPT"),
            Some(OsString::from("0"))
        );
        assert_eq!(applied.child_env_var("HOME"), std::env::var_os("HOME"));
        let _ = applied.run(&[], &std::env::temp_dir());
        assert_eq!(runner.spawn_count(), 1);
    }

    #[test]
    fn askpass_secret_is_written_once_wiped_and_never_formatted() {
        let mut out = Vec::new();
        let secret = AskpassSecret::new(b"ghp_ASKPASS_SECRET".to_vec());
        assert_eq!(format!("{secret:?}"), "AskpassSecret(<redacted>)");
        secret.write_once_to(&mut out).unwrap();
        assert_eq!(out, b"ghp_ASKPASS_SECRET");

        let mut s = AskpassSecret::new(b"abc".to_vec());
        let ptr = s.bytes.as_ptr();
        let cap = s.bytes.capacity();
        s.wipe();
        assert!(s.bytes.is_empty());
        // clear 후에도 같은 할당이 남아 있으며 내용은 0으로 덮였습니다.
        assert_eq!(s.bytes.capacity(), cap);
        assert_eq!(s.bytes.as_ptr(), ptr);
        // SAFETY: capacity 안의 같은 할당을 읽기만 합니다(wipe가 0으로 초기화한 바이트).
        let raw = unsafe { std::slice::from_raw_parts(ptr, 3) };
        assert_eq!(raw, &[0, 0, 0]);
    }

    #[cfg(unix)]
    mod storage_scan {
        use super::*;
        use crate::remote::connect::{
            connect_empty_github_remote, ConnectError, ConnectRemoteRequest,
        };
        use crate::remote::github::GitHubUrlProblem;
        use crate::remote::push::{push, PushRefspec, PushRequest};
        use crate::remote::RemoteUrl;
        use crate::test_support::{
            commit_file, git_ok, github_redirect_runner, init_bare, init_repo_with_identity,
            system_git, TempDir,
        };
        use crate::trust::{GatedGitRunner, GitTrust};
        use std::path::Path;

        const MARKER: &str = "S3CR3T_TOKEN_MARKER";

        /// 디렉터리 아래 모든 파일 bytes에서 marker를 찾습니다.
        fn files_containing(dir: &Path, marker: &[u8], out: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let ty = entry.file_type().unwrap();
                if ty.is_dir() {
                    files_containing(&path, marker, out);
                } else if ty.is_file() {
                    let bytes = std::fs::read(&path).unwrap();
                    if bytes.windows(marker.len()).any(|w| w == marker) {
                        out.push(path);
                    }
                }
            }
        }

        #[test]
        fn credential_redaction_and_storage_scan() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("cred-scan");
            let remotes = dir.path().join("remotes");
            let base = github_redirect_runner(git, &remotes);
            init_bare(&base, &remotes.join("acme/empty.git"));
            let work = dir.path().join("work");
            std::fs::create_dir_all(&work).unwrap();
            init_repo_with_identity(&base, &work);
            commit_file(&base, &work, "stellar.yaml", b"v: 1\n");
            // askpass bridge가 호출되면 marker를 내놓는 fixture(로컬 전송에서는 호출되지 않아야 함).
            let bridge = dir.path().join("askpass.sh");
            std::fs::write(&bridge, format!("#!/bin/sh\necho {MARKER}\n")).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bridge, std::fs::Permissions::from_mode(0o755)).unwrap();
            let runner = CredentialDelegation::with_askpass_bridge(bridge.clone()).apply(&base);
            let gate = GatedGitRunner::new(runner, GitTrust::Trusted);

            // 1) 자격 증명이 든 URL은 연결 전에 거부(Git 미실행, config 미기록).
            let with_creds =
                RemoteUrl::new(format!("https://alice:{MARKER}@github.com/acme/empty.git"));
            let err = connect_empty_github_remote(
                &gate,
                &work,
                &ConnectRemoteRequest {
                    name: None,
                    url: with_creds.clone(),
                },
            )
            .unwrap_err();
            assert_eq!(
                err,
                ConnectError::InvalidGitHubUrl(GitHubUrlProblem::CredentialsInUrl)
            );
            assert!(!format!("{err:?} {err}").contains(MARKER));

            // 2) 정상 흐름: 연결 → push(set-upstream).
            connect_empty_github_remote(
                &gate,
                &work,
                &ConnectRemoteRequest {
                    name: None,
                    url: RemoteUrl::new("https://github.com/acme/empty.git"),
                },
            )
            .unwrap();
            push(
                &gate,
                &work,
                &PushRequest {
                    remote: "origin".into(),
                    refspec: PushRefspec::branch("main").unwrap(),
                    set_upstream: true,
                },
            )
            .unwrap();

            // 3) 사용자가 직접 자격 증명 URL을 config에 넣은 remote로 push해도 Stellar 오류 표시에는
            //    비밀값이 나오지 않음(https는 이 fixture에서 Git이 거부하므로 네트워크 미사용).
            git_ok(
                gate.runner().unwrap(),
                &work,
                ["remote", "add", "userset", with_creds.expose()],
            );
            let push_err = push(
                &gate,
                &work,
                &PushRequest {
                    remote: "userset".into(),
                    refspec: PushRefspec::branch("main").unwrap(),
                    set_upstream: false,
                },
            )
            .unwrap_err();
            let shown = format!("{push_err:?} {push_err}");
            assert!(!shown.contains(MARKER), "{shown}");
            git_ok(
                gate.runner().unwrap(),
                &work,
                ["remote", "remove", "userset"],
            );

            // 4) storage scan: 작업 저장소(.git 포함)·원격 어디에도 비밀값이 저장되지 않음.
            let mut found = Vec::new();
            files_containing(&work, MARKER.as_bytes(), &mut found);
            files_containing(&remotes, MARKER.as_bytes(), &mut found);
            assert!(found.is_empty(), "비밀값이 저장됨: {found:?}");
        }
    }
}
