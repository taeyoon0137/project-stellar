//! 커밋 실행 (XIV.8.2).
//!
//! commit message는 **stdin**으로 시스템 Git에 전달합니다(`git commit --file=-`). message는 argv에
//! 들어가지 않으므로 shell escaping·quote에 의존하지 않고, `-`로 시작하는 줄이나 shell
//! metacharacter도 그대로 message 내용이 됩니다.
//!
//! `--cleanup=verbatim`을 명시해 사용자 config(`commit.cleanup`)와 무관하게 message를 바꾸지
//! 않습니다(주석처럼 보이는 `#` 줄·공백 보존). hooks는 Git이 그대로 실행하며 우회하지 않습니다.
//! 모든 실행은 trust gate(XIV.1.1)를 거칩니다.

use std::path::Path;

use crate::command::{GitCommandFailure, GitCommandResult};
use crate::runner::argv;
use crate::trust::{GatedGitRunner, GatedRunError};

/// commit message 전달 argv(message 자체는 포함하지 않음).
pub const COMMIT_FROM_STDIN_ARGS: [&str; 3] = ["commit", "--file=-", "--cleanup=verbatim"];

/// 커밋 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitError {
    /// trust gate 거부 또는 Git 실행 실패.
    Gate(GatedRunError),
    /// message에 NUL 문자가 있어 Git commit message로 전달할 수 없음(Git 미실행).
    MessageContainsNul,
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
    let output = git
        .run_with_input(&argv(COMMIT_FROM_STDIN_ARGS), worktree, message.as_bytes())
        .map_err(CommitError::Gate)?;
    GitCommandResult::from(output)
        .require_success()
        .map_err(CommitError::Git)?;
    Ok(())
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
    }
}
