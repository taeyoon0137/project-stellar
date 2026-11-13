//! 새 GitHub remote 최초 연결 검사 (XIV.9.3).
//!
//! 기존 빈 GitHub 저장소만 최초 업로드 대상으로 연결합니다(아키텍처 §25.3). 연결 전에
//! `git ls-remote <url>`로 원격 refs를 확인하고, ref가 **하나라도** 있으면 최초 연결을 거부하고
//! Clone 안내 상태([`InitialConnectionCheck::UseCloneInstead`])를 돌려줍니다. 원격 저장소를 만들거나
//! 바꾸지 않으며 GitHub API를 호출하지 않습니다.
//!
//! - URL은 먼저 [`classify_github_url`](XIV.9.2)로 canonical GitHub 형식인지 확인합니다. 아니면 Git을
//!   실행하지 않습니다.
//! - `-`로 시작하는 URL은 option으로 해석될 수 있으므로 거부하고, `--end-of-options` 뒤에 넘깁니다.
//! - 인증은 Git(credential helper·SSH agent)에 위임합니다. 실패 stderr는 자격 증명을 가린 텍스트만
//!   보존합니다.

use std::path::Path;

use super::github::{classify_github_url, GitHubRepo, GitHubUrlCheck, GitHubUrlProblem};
use super::{redacted_failure, RemoteError, RemoteUrl};
use crate::runner::argv;
use crate::trust::GatedGitRunner;

/// 원격 refs 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteRefsState {
    /// ref 없음(빈 저장소).
    Empty,
    /// ref가 있음.
    HasRefs {
        /// `ls-remote`가 보고한 ref 수(HEAD 포함).
        ref_count: usize,
    },
}

/// 최초 연결 검사 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitialConnectionCheck {
    /// 빈 GitHub 저장소: 최초 업로드 연결 가능.
    ReadyForFirstUpload {
        /// 검증된 저장소.
        repo: GitHubRepo,
    },
    /// 기존 이력이 있음: 최초 연결 거부, Clone 안내.
    UseCloneInstead {
        /// 검증된 저장소.
        repo: GitHubRepo,
        /// 원격 ref 수.
        ref_count: usize,
    },
}

/// 최초 연결 검사 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectError {
    /// GitHub URL이 아님(외부 터미널에서 등록, Git 미실행).
    NotGitHub,
    /// GitHub URL 형식 오류(Git 미실행).
    InvalidGitHubUrl(GitHubUrlProblem),
    /// Git 실행·해석 실패.
    Remote(RemoteError),
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectError::NotGitHub => f.write_str(
                "GitHub 저장소 URL이 아닙니다. 다른 remote는 외부 터미널에서 등록하세요",
            ),
            ConnectError::InvalidGitHubUrl(p) => write!(f, "GitHub URL 형식 오류: {p:?}"),
            ConnectError::Remote(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ConnectError {}

impl From<RemoteError> for ConnectError {
    fn from(e: RemoteError) -> ConnectError {
        ConnectError::Remote(e)
    }
}

/// `git ls-remote` 출력(`<oid> TAB <ref> LF`)의 ref 수를 셉니다.
pub fn count_ls_remote_refs(stdout: &[u8]) -> Result<usize, RemoteError> {
    let bad = |detail: &str| RemoteError::Parse {
        command: "ls-remote",
        detail: detail.to_string(),
    };
    if stdout.is_empty() {
        return Ok(0);
    }
    let body = stdout
        .strip_suffix(b"\n")
        .ok_or_else(|| bad("LF로 끝나지 않음"))?;
    let mut count = 0;
    for line in body.split(|b| *b == b'\n') {
        let tab = line
            .iter()
            .position(|b| *b == b'\t')
            .ok_or_else(|| bad("TAB 없음"))?;
        if crate::object::ObjectId::from_hex_bytes(&line[..tab]).is_none() || tab + 1 == line.len()
        {
            return Err(bad("잘못된 ref 줄"));
        }
        count += 1;
    }
    Ok(count)
}

/// 원격 refs 존재 여부를 확인합니다(`git ls-remote --end-of-options <url>`, 원격 변경 없음).
pub fn probe_remote_refs(
    git: &GatedGitRunner,
    worktree: &Path,
    url: &RemoteUrl,
) -> Result<RemoteRefsState, RemoteError> {
    if url.expose().starts_with('-') || url.expose().is_empty() {
        return Err(RemoteError::Parse {
            command: "ls-remote",
            detail: "option처럼 보이거나 빈 URL".to_string(),
        });
    }
    let output = git.run(
        &argv(["ls-remote", "--end-of-options", url.expose()]),
        worktree,
    )?;
    if !output.success() {
        return Err(redacted_failure(&output));
    }
    Ok(match count_ls_remote_refs(&output.stdout)? {
        0 => RemoteRefsState::Empty,
        ref_count => RemoteRefsState::HasRefs { ref_count },
    })
}

/// URL을 canonical GitHub 형식으로 확인합니다(Git 미실행).
pub(crate) fn require_github(url: &RemoteUrl) -> Result<GitHubRepo, ConnectError> {
    match classify_github_url(url.expose()) {
        GitHubUrlCheck::GitHub(repo) => Ok(repo),
        GitHubUrlCheck::NotGitHub => Err(ConnectError::NotGitHub),
        GitHubUrlCheck::Invalid(problem) => Err(ConnectError::InvalidGitHubUrl(problem)),
    }
}

/// 새 GitHub remote를 최초 업로드용으로 연결해도 되는지 검사합니다.
pub fn check_initial_github_connection(
    git: &GatedGitRunner,
    worktree: &Path,
    url: &RemoteUrl,
) -> Result<InitialConnectionCheck, ConnectError> {
    let repo = require_github(url)?;
    Ok(match probe_remote_refs(git, worktree, url)? {
        RemoteRefsState::Empty => InitialConnectionCheck::ReadyForFirstUpload { repo },
        RemoteRefsState::HasRefs { ref_count } => {
            InitialConnectionCheck::UseCloneInstead { repo, ref_count }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_ls_remote_lines() {
        let e = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";
        assert_eq!(count_ls_remote_refs(b""), Ok(0));
        let out = format!("{e}\tHEAD\n{e}\trefs/heads/main\n{e}\trefs/tags/v1\n");
        assert_eq!(count_ls_remote_refs(out.as_bytes()), Ok(3));
        for bad in [
            format!("{e}\tHEAD"),
            format!("{e} HEAD\n"),
            "xyz\tHEAD\n".to_string(),
            format!("{e}\t\n"),
        ] {
            assert!(count_ls_remote_refs(bad.as_bytes()).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn non_github_or_invalid_urls_spawn_nothing() {
        use crate::runner::GitRunner;
        use crate::trust::GitTrust;
        let runner = GitRunner::new(
            std::path::PathBuf::from("/nonexistent/git"),
            Vec::new(),
            std::time::Duration::from_secs(5),
        );
        let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
        let dir = std::env::temp_dir();
        assert_eq!(
            check_initial_github_connection(
                &gate,
                &dir,
                &RemoteUrl::new("https://gitlab.com/a/b.git")
            ),
            Err(ConnectError::NotGitHub)
        );
        assert_eq!(
            check_initial_github_connection(
                &gate,
                &dir,
                &RemoteUrl::new("https://u:tok@github.com/a/b.git")
            ),
            Err(ConnectError::InvalidGitHubUrl(
                GitHubUrlProblem::CredentialsInUrl
            ))
        );
        assert!(probe_remote_refs(&gate, &dir, &RemoteUrl::new("--upload-pack=touch x")).is_err());
        assert_eq!(runner.spawn_count(), 0);
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::test_support::{
            commit_file, git_ok, github_redirect_runner, init_bare, init_repo_with_identity,
            system_git, TempDir,
        };
        use crate::trust::GitTrust;

        #[test]
        fn empty_and_non_empty_bare_repo_fixture() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("connect-probe");
            let remotes = dir.path().join("remotes");
            let runner = github_redirect_runner(git, &remotes);
            init_bare(&runner, &remotes.join("acme/empty.git"));
            init_bare(&runner, &remotes.join("acme/history.git"));
            let work = dir.path().join("work");
            std::fs::create_dir_all(&work).unwrap();
            init_repo_with_identity(&runner, &work);
            commit_file(&runner, &work, "a.txt", b"a\n");
            let history = remotes.join("acme/history.git");
            git_ok(
                &runner,
                &work,
                ["push", "-q", history.to_str().unwrap(), "main"],
            );

            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            // 로컬 경로 직접 probe.
            assert_eq!(
                probe_remote_refs(
                    &gate,
                    &work,
                    &RemoteUrl::new(remotes.join("acme/empty.git").to_str().unwrap())
                ),
                Ok(RemoteRefsState::Empty)
            );
            assert!(matches!(
                probe_remote_refs(&gate, &work, &RemoteUrl::new(history.to_str().unwrap())),
                Ok(RemoteRefsState::HasRefs { ref_count }) if ref_count >= 1
            ));

            // GitHub URL(insteadOf로 로컬 bare에 연결, 네트워크 미사용).
            let empty = check_initial_github_connection(
                &gate,
                &work,
                &RemoteUrl::new("https://github.com/acme/empty.git"),
            )
            .unwrap();
            assert!(matches!(
                empty,
                InitialConnectionCheck::ReadyForFirstUpload { ref repo } if repo.repo == "empty"
            ));
            let non_empty = check_initial_github_connection(
                &gate,
                &work,
                &RemoteUrl::new("git@github.com:acme/history.git"),
            )
            .unwrap();
            assert!(matches!(
                non_empty,
                InitialConnectionCheck::UseCloneInstead { ref_count, .. } if ref_count >= 1
            ));
            // 검사는 원격을 바꾸지 않음(빈 저장소는 여전히 비어 있음).
            assert_eq!(
                probe_remote_refs(
                    &gate,
                    &work,
                    &RemoteUrl::new("https://github.com/acme/empty.git")
                ),
                Ok(RemoteRefsState::Empty)
            );
            // 없는 저장소는 Git 실패(stderr redaction).
            assert!(matches!(
                check_initial_github_connection(
                    &gate,
                    &work,
                    &RemoteUrl::new("https://github.com/acme/missing.git")
                ),
                Err(ConnectError::Remote(RemoteError::Failed { .. }))
            ));
        }
    }
}
