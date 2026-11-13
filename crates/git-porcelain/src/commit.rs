//! 커밋 실행 (XIV.8.2~XIV.8.3).
//!
//! commit message는 **stdin**으로 시스템 Git에 전달합니다(`git commit --file=-`). message는 argv에
//! 들어가지 않으므로 shell escaping·quote에 의존하지 않고, `-`로 시작하는 줄이나 shell
//! metacharacter도 그대로 message 내용이 됩니다.
//!
//! `--cleanup=verbatim`을 명시해 사용자 config(`commit.cleanup`)와 무관하게 message를 바꾸지
//! 않습니다(주석처럼 보이는 `#` 줄·공백 보존). hooks는 Git이 그대로 실행하며 우회하지 않습니다.
//! 모든 실행은 trust gate(XIV.1.1)를 거칩니다.
//!
//! 커밋 전에 `user.name`/`user.email`(Git 유효 config)을 확인하고, 누락되면 `git commit`을
//! 실행하지 않고 [`CommitError::MissingIdentity`]로 누락 항목을 구조화해 반환합니다(XIV.8.3).
//! Stellar는 global·local Git config를 **자동으로 수정하지 않습니다**. 설정은 UI 안내에 따라
//! 사용자가 직접 합니다.

use std::path::Path;

use crate::command::{GitCommandFailure, GitCommandResult};
use crate::runner::argv;
use crate::trust::{GatedGitRunner, GatedRunError};

/// commit message 전달 argv(message 자체는 포함하지 않음).
pub const COMMIT_FROM_STDIN_ARGS: [&str; 3] = ["commit", "--file=-", "--cleanup=verbatim"];

/// commit identity config key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityKey {
    /// `user.name`.
    UserName,
    /// `user.email`.
    UserEmail,
}

impl IdentityKey {
    /// Git config key 문자열.
    pub fn config_key(self) -> &'static str {
        match self {
            IdentityKey::UserName => "user.name",
            IdentityKey::UserEmail => "user.email",
        }
    }
}

/// 누락된 commit identity(UI 안내용 구조). 최소 한 항목은 누락입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissingIdentity {
    /// `user.name` 누락.
    pub name_missing: bool,
    /// `user.email` 누락.
    pub email_missing: bool,
}

impl MissingIdentity {
    /// 누락된 key 목록(`user.name` → `user.email` 순서).
    pub fn missing_keys(&self) -> Vec<IdentityKey> {
        let mut keys = Vec::new();
        if self.name_missing {
            keys.push(IdentityKey::UserName);
        }
        if self.email_missing {
            keys.push(IdentityKey::UserEmail);
        }
        keys
    }
}

/// 커밋 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitError {
    /// trust gate 거부 또는 Git 실행 실패.
    Gate(GatedRunError),
    /// message에 NUL 문자가 있어 Git commit message로 전달할 수 없음(Git 미실행).
    MessageContainsNul,
    /// `user.name`/`user.email` 미설정으로 커밋을 실행하지 않음(Git config는 수정하지 않음).
    MissingIdentity(MissingIdentity),
    /// `git commit` 비영 종료(hook 실패·빈 message 등, Git 결과 보존).
    Git(GitCommandFailure),
}

impl std::fmt::Display for CommitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommitError::Gate(e) => write!(f, "{e}"),
            CommitError::MessageContainsNul => {
                f.write_str("commit message에 NUL 문자를 넣을 수 없습니다")
            }
            CommitError::MissingIdentity(missing) => {
                let keys: Vec<&str> = missing
                    .missing_keys()
                    .into_iter()
                    .map(IdentityKey::config_key)
                    .collect();
                write!(
                    f,
                    "Git 사용자 identity가 설정되지 않았습니다({}). 설정 후 다시 커밋하세요",
                    keys.join(", ")
                )
            }
            CommitError::Git(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CommitError {}

/// staged 변경을 `message`로 커밋합니다(message는 stdin으로 전달).
pub fn commit_with_message(
    git: &GatedGitRunner,
    worktree: &Path,
    message: &str,
) -> Result<(), CommitError> {
    if message.contains('\0') {
        return Err(CommitError::MessageContainsNul);
    }
    if let Some(missing) = check_commit_identity(git, worktree)? {
        return Err(CommitError::MissingIdentity(missing));
    }
    let output = git
        .run_with_input(&argv(COMMIT_FROM_STDIN_ARGS), worktree, message.as_bytes())
        .map_err(CommitError::Gate)?;
    GitCommandResult::from(output)
        .require_success()
        .map_err(CommitError::Git)?;
    Ok(())
}

/// 커밋 전에 `user.name`/`user.email`이 Git 유효 config에 있는지 확인합니다(읽기 전용).
///
/// 둘 다 있으면 `None`, 하나라도 없으면 누락 항목을 반환합니다. config를 쓰지 않습니다.
pub fn check_commit_identity(
    git: &GatedGitRunner,
    worktree: &Path,
) -> Result<Option<MissingIdentity>, CommitError> {
    let name_missing = !identity_value_present(git, worktree, IdentityKey::UserName)?;
    let email_missing = !identity_value_present(git, worktree, IdentityKey::UserEmail)?;
    if name_missing || email_missing {
        Ok(Some(MissingIdentity {
            name_missing,
            email_missing,
        }))
    } else {
        Ok(None)
    }
}

/// `git config --get <key>`가 비어 있지 않은 값을 주는지 확인합니다.
///
/// 종료 코드 1은 key 없음(누락)입니다. 그 밖의 비영 종료(손상된 config 등)는 누락으로 숨기지 않고
/// [`CommitError::Git`]으로 반환합니다.
fn identity_value_present(
    git: &GatedGitRunner,
    worktree: &Path,
    key: IdentityKey,
) -> Result<bool, CommitError> {
    let output = git
        .run(&argv(["config", "--get", key.config_key()]), worktree)
        .map_err(CommitError::Gate)?;
    match output.exit_code {
        Some(0) => Ok(!String::from_utf8_lossy(&output.stdout).trim().is_empty()),
        Some(1) => Ok(false),
        _ => Err(CommitError::Git(GitCommandFailure {
            exit_code: output.exit_code,
            stdout: output.stdout,
            stderr: output.stderr,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::GitRunner;
    use crate::trust::GitTrust;
    use std::path::PathBuf;
    use std::time::Duration;

    #[test]
    fn untrusted_commit_spawns_nothing() {
        let runner = GitRunner::new(
            PathBuf::from("/nonexistent/git"),
            Vec::new(),
            Duration::from_secs(5),
        );
        let gate = GatedGitRunner::new(runner.clone(), GitTrust::NotTrusted);
        assert_eq!(
            commit_with_message(&gate, &std::env::temp_dir(), "msg\n"),
            Err(CommitError::Gate(GatedRunError::ProjectNotTrusted))
        );
        assert_eq!(runner.spawn_count(), 0);
    }

    #[test]
    fn nul_in_message_is_rejected_before_spawn() {
        let runner = GitRunner::new(
            PathBuf::from("/nonexistent/git"),
            Vec::new(),
            Duration::from_secs(5),
        );
        let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
        assert_eq!(
            commit_with_message(&gate, &std::env::temp_dir(), "a\0b"),
            Err(CommitError::MessageContainsNul)
        );
        assert_eq!(runner.spawn_count(), 0);
    }

    #[test]
    fn commit_args_do_not_carry_the_message() {
        assert_eq!(
            COMMIT_FROM_STDIN_ARGS,
            ["commit", "--file=-", "--cleanup=verbatim"]
        );
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::test_support::{
            git_ok, hermetic_runner, init_repo_with_identity, system_git, TempDir,
        };

        /// `git cat-file commit HEAD`의 header 뒤(첫 빈 줄 이후) 원시 message 바이트.
        fn head_message_bytes(gate: &GatedGitRunner, repo: &Path) -> Vec<u8> {
            let out = git_ok(gate.runner().unwrap(), repo, ["cat-file", "commit", "HEAD"]);
            let split = out
                .stdout
                .windows(2)
                .position(|w| w == b"\n\n")
                .expect("commit object header 끝");
            out.stdout[split + 2..].to_vec()
        }

        fn repo_with_staged_file(gate: &GatedGitRunner, dir: &Path, name: &str) {
            let runner = gate.runner().unwrap();
            init_repo_with_identity(runner, dir);
            std::fs::write(dir.join(name), b"contents\n").unwrap();
            git_ok(runner, dir, ["add", "--", name]);
        }

        #[test]
        fn multiline_unicode_message_is_stored_verbatim() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("commit-msg");
            let gate = GatedGitRunner::new(hermetic_runner(git), GitTrust::Trusted);
            repo_with_staged_file(&gate, dir.path(), "a.txt");

            let message = "디자인 토큰 갱신 ✨ — Ünïcödé 日本語\n\
                           \n\
                           본문 첫 줄\n\
                           # 주석처럼 보이는 줄도 보존\n\
                           --amend 로 시작하는 줄\n\
                           \x20\x20들여쓰기와 '작은따옴표' \"큰따옴표\" 보존\n";
            commit_with_message(&gate, dir.path(), message).expect("commit");
            assert_eq!(head_message_bytes(&gate, dir.path()), message.as_bytes());
        }

        #[test]
        fn shell_metacharacters_in_message_are_not_interpreted() {
            let Some(git) = system_git() else {
                return;
            };
            let dir = TempDir::new("commit-shell");
            let gate = GatedGitRunner::new(hermetic_runner(git), GitTrust::Trusted);
            repo_with_staged_file(&gate, dir.path(), "a.txt");

            let message = "$(touch PWNED1) `touch PWNED2`; touch PWNED3 && touch PWNED4 | x\n";
            commit_with_message(&gate, dir.path(), message).expect("commit");
            assert_eq!(head_message_bytes(&gate, dir.path()), message.as_bytes());
            for name in ["PWNED1", "PWNED2", "PWNED3", "PWNED4"] {
                assert!(!dir.path().join(name).exists(), "shell 해석됨: {name}");
            }
        }

        #[test]
        fn message_without_trailing_newline_is_kept_exact() {
            let Some(git) = system_git() else {
                return;
            };
            let dir = TempDir::new("commit-nonl");
            let gate = GatedGitRunner::new(hermetic_runner(git), GitTrust::Trusted);
            repo_with_staged_file(&gate, dir.path(), "a.txt");
            commit_with_message(&gate, dir.path(), "한 줄 제목 🚀").expect("commit");
            assert_eq!(
                head_message_bytes(&gate, dir.path()),
                "한 줄 제목 🚀".as_bytes()
            );
        }

        #[test]
        fn git_failure_is_returned_as_is() {
            let Some(git) = system_git() else {
                return;
            };
            let dir = TempDir::new("commit-empty");
            let gate = GatedGitRunner::new(hermetic_runner(git), GitTrust::Trusted);
            // staged 변경 없음 → Git이 커밋을 거부(결과 보존).
            init_repo_with_identity(gate.runner().unwrap(), dir.path());
            match commit_with_message(&gate, dir.path(), "nothing\n") {
                Err(CommitError::Git(failure)) => assert_ne!(failure.exit_code, Some(0)),
                other => panic!("예상과 다른 결과: {other:?}"),
            }
        }

        /// global config를 임시 파일로 지정한 실행기(system config는 비움).
        fn runner_with_global(git: PathBuf, global: &Path) -> GitRunner {
            use std::ffi::OsString;
            GitRunner::new(
                git,
                vec![OsString::from("PATH"), OsString::from("HOME")],
                Duration::from_secs(30),
            )
            .with_env_overrides(vec![
                (
                    OsString::from("GIT_CONFIG_GLOBAL"),
                    global.as_os_str().to_os_string(),
                ),
                (
                    OsString::from("GIT_CONFIG_SYSTEM"),
                    OsString::from("/dev/null"),
                ),
            ])
        }

        /// identity 없이 파일 하나를 stage한 저장소(로컬 identity 미설정).
        fn repo_without_identity(runner: &GitRunner, repo: &Path) {
            git_ok(runner, repo, ["init", "-q", "-b", "main"]);
            std::fs::write(repo.join("a.txt"), b"contents\n").unwrap();
            git_ok(runner, repo, ["add", "--", "a.txt"]);
        }

        #[test]
        fn missing_identity_is_structured_and_config_is_not_modified() {
            let Some(git) = system_git() else {
                return;
            };
            let dir = TempDir::new("commit-noident");
            let repo = dir.path().join("repo");
            std::fs::create_dir_all(&repo).unwrap();
            let global = dir.path().join("global.gitconfig");
            let global_before = b"[core]\n\tautocrlf = false\n".to_vec();
            std::fs::write(&global, &global_before).unwrap();

            let setup = runner_with_global(git.clone(), &global);
            repo_without_identity(&setup, &repo);
            let local_before = std::fs::read(repo.join(".git/config")).unwrap();

            let runner = runner_with_global(git, &global);
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            let result = commit_with_message(&gate, &repo, "first\n");
            let missing = MissingIdentity {
                name_missing: true,
                email_missing: true,
            };
            assert_eq!(result, Err(CommitError::MissingIdentity(missing)));
            assert_eq!(
                missing.missing_keys(),
                vec![IdentityKey::UserName, IdentityKey::UserEmail]
            );
            let text = CommitError::MissingIdentity(missing).to_string();
            assert!(
                text.contains("user.name") && text.contains("user.email"),
                "{text}"
            );

            // identity 확인(config 읽기 2회)만 실행하고 git commit은 실행하지 않음.
            assert_eq!(runner.spawn_count(), 2);
            let head = setup
                .run(&argv(["rev-parse", "--verify", "-q", "HEAD"]), &repo)
                .unwrap();
            assert!(!head.success(), "커밋이 생성됨");
            // Stellar가 global·local config를 자동 수정하지 않음.
            assert_eq!(std::fs::read(&global).unwrap(), global_before);
            assert_eq!(
                std::fs::read(repo.join(".git/config")).unwrap(),
                local_before
            );
        }

        #[test]
        fn only_email_missing_is_reported_precisely() {
            let Some(git) = system_git() else {
                return;
            };
            let dir = TempDir::new("commit-noemail");
            let repo = dir.path().join("repo");
            std::fs::create_dir_all(&repo).unwrap();
            let global = dir.path().join("global.gitconfig");
            std::fs::write(&global, b"[user]\n\tname = Global Name\n").unwrap();
            let runner = runner_with_global(git, &global);
            repo_without_identity(&runner, &repo);
            let gate = GatedGitRunner::new(runner, GitTrust::Trusted);

            assert_eq!(
                check_commit_identity(&gate, &repo),
                Ok(Some(MissingIdentity {
                    name_missing: false,
                    email_missing: true,
                }))
            );
            assert_eq!(
                commit_with_message(&gate, &repo, "x\n"),
                Err(CommitError::MissingIdentity(MissingIdentity {
                    name_missing: false,
                    email_missing: true,
                }))
            );
        }

        #[test]
        fn identity_from_global_config_allows_commit() {
            // 유효 config(global 포함)를 읽으므로 local 미설정이어도 global identity로 커밋됩니다.
            let Some(git) = system_git() else {
                return;
            };
            let dir = TempDir::new("commit-globalident");
            let repo = dir.path().join("repo");
            std::fs::create_dir_all(&repo).unwrap();
            let global = dir.path().join("global.gitconfig");
            std::fs::write(
                &global,
                b"[user]\n\tname = Global Name\n\temail = global@example.com\n",
            )
            .unwrap();
            let runner = runner_with_global(git, &global);
            repo_without_identity(&runner, &repo);
            let gate = GatedGitRunner::new(runner, GitTrust::Trusted);

            assert_eq!(check_commit_identity(&gate, &repo), Ok(None));
            commit_with_message(&gate, &repo, "with global identity\n").expect("commit");
            let author = git_ok(
                gate.runner().unwrap(),
                &repo,
                ["log", "-1", "--format=%an <%ae>"],
            );
            assert_eq!(
                String::from_utf8_lossy(&author.stdout).trim(),
                "Global Name <global@example.com>"
            );
        }

        #[test]
        fn corrupt_config_is_not_hidden_as_missing_identity() {
            let Some(git) = system_git() else {
                return;
            };
            let dir = TempDir::new("commit-badconfig");
            let repo = dir.path().join("repo");
            std::fs::create_dir_all(&repo).unwrap();
            let global = dir.path().join("global.gitconfig");
            std::fs::write(&global, b"").unwrap();
            let runner = runner_with_global(git, &global);
            repo_without_identity(&runner, &repo);
            // 손상된 global config: git config가 exit 1이 아닌 오류로 종료.
            std::fs::write(&global, b"[user\n\tname = broken\n").unwrap();
            let gate = GatedGitRunner::new(runner, GitTrust::Trusted);
            match check_commit_identity(&gate, &repo) {
                Err(CommitError::Git(failure)) => {
                    assert!(!matches!(failure.exit_code, Some(0) | Some(1)));
                }
                other => panic!("예상과 다른 결과: {other:?}"),
            }
        }
    }

    #[test]
    fn untrusted_identity_check_spawns_nothing() {
        let runner = GitRunner::new(
            PathBuf::from("/nonexistent/git"),
            Vec::new(),
            Duration::from_secs(5),
        );
        let gate = GatedGitRunner::new(runner.clone(), GitTrust::NotTrusted);
        assert_eq!(
            check_commit_identity(&gate, &std::env::temp_dir()),
            Err(CommitError::Gate(GatedRunError::ProjectNotTrusted))
        );
        assert_eq!(runner.spawn_count(), 0);
    }
}
