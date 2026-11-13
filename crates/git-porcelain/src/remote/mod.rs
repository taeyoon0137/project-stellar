//! Remote 목록과 민감 URL (XIV.9.1~).
//!
//! 기존 remote 목록은 `git remote -v` 출력의 안정 형식(`<name> TAB <url> SP (fetch|push) LF`)으로
//! 읽고 fetch URL과 push URL을 구분합니다. remote 이름은 공백·TAB을 포함할 수 없으므로 첫 TAB에서
//! 나누고, 줄 끝의 고정 접미사 ` (fetch)`/` (push)`만 떼어 URL을 보존합니다. push URL이 여러 개면
//! 모두 보존합니다.
//!
//! URL은 **민감값**입니다([`RemoteUrl`]). 사용자 이름·token이 들어 있을 수 있으므로 `Debug`는 값을
//! 출력하지 않고, 표시용 [`RemoteUrl::redacted`]는 userinfo(`user:token@`)를 가립니다. 실제 값은
//! Git 명령 인자로 넘길 때만 [`RemoteUrl::expose`]로 꺼냅니다.

use std::path::Path;

use crate::ops::{run_ok, GitOpError, ParseFailure};
use crate::runner::argv;
use crate::trust::GatedGitRunner;

/// GitHub URL 형식 검증(XIV.9.2).
pub mod github;

/// 민감값으로 다루는 remote URL.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct RemoteUrl(String);

impl RemoteUrl {
    /// URL 문자열로 만듭니다.
    pub fn new(url: impl Into<String>) -> RemoteUrl {
        RemoteUrl(url.into())
    }

    /// 실제 URL(Git 인자 전달 전용, 로그·표시에 쓰지 않음).
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// userinfo를 가린 표시용 문자열.
    pub fn redacted(&self) -> String {
        redact_credentials(&self.0)
    }
}

impl std::fmt::Debug for RemoteUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RemoteUrl(<redacted>)")
    }
}

/// 텍스트 안의 모든 `scheme://userinfo@host` userinfo를 `***`로 가립니다.
///
/// Git stderr처럼 URL이 섞인 임의 텍스트에도 씁니다. `scheme://` 뒤 authority(다음 `/`·공백·따옴표
/// 전까지)에 `@`가 있으면 마지막 `@` 앞 전체를 가립니다. scp 형식(`git@host:path`)의 사용자 이름은
/// 비밀이 아니므로 그대로 둡니다.
pub fn redact_credentials(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| c == '/' || c.is_whitespace() || matches!(c, '\'' | '"' | '>' | ')'))
            .unwrap_or(tail.len());
        let authority = &tail[..end];
        match authority.rfind('@') {
            Some(at) => {
                out.push_str("***");
                out.push_str(&authority[at..]);
            }
            None => out.push_str(authority),
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// remote 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteEntry {
    /// remote 이름.
    pub name: String,
    /// fetch URL.
    pub fetch_url: Option<RemoteUrl>,
    /// push URL(설정 순서).
    pub push_urls: Vec<RemoteUrl>,
}

const COMMAND: &str = "remote -v";

/// `git remote -v` 출력을 해석합니다(remote 첫 등장 순서 유지).
pub fn parse_remote_verbose(stdout: &[u8]) -> Result<Vec<RemoteEntry>, ParseFailure> {
    let text =
        std::str::from_utf8(stdout).map_err(|_| ParseFailure::new(COMMAND, "UTF-8 아님", b""))?;
    let mut remotes: Vec<RemoteEntry> = Vec::new();
    if text.is_empty() {
        return Ok(remotes);
    }
    let body = text
        .strip_suffix('\n')
        .ok_or_else(|| ParseFailure::new(COMMAND, "LF로 끝나지 않음", b""))?;
    for line in body.split('\n') {
        // 줄 내용(URL)은 민감값이므로 실패 원시값에 넣지 않습니다.
        let bad = |detail: &str| ParseFailure::new(COMMAND, detail, b"");
        let (name, rest) = line
            .split_once('\t')
            .ok_or_else(|| bad("TAB 구분자 없음"))?;
        if name.is_empty() || name.chars().any(char::is_whitespace) {
            return Err(bad("잘못된 remote 이름"));
        }
        let (url, is_push) = if let Some(url) = rest.strip_suffix(" (fetch)") {
            (url, false)
        } else if let Some(url) = rest.strip_suffix(" (push)") {
            (url, true)
        } else {
            return Err(bad("(fetch)/(push) 접미사 없음"));
        };
        if url.is_empty() {
            return Err(bad("빈 URL"));
        }
        let index = match remotes.iter().position(|r| r.name == name) {
            Some(i) => i,
            None => {
                remotes.push(RemoteEntry {
                    name: name.to_string(),
                    fetch_url: None,
                    push_urls: Vec::new(),
                });
                remotes.len() - 1
            }
        };
        let entry = &mut remotes[index];
        if is_push {
            entry.push_urls.push(RemoteUrl::new(url));
        } else if entry.fetch_url.replace(RemoteUrl::new(url)).is_some() {
            return Err(bad("fetch URL 중복"));
        }
    }
    Ok(remotes)
}

/// 기존 remote 목록을 읽습니다(`git remote -v`, trust gate 적용).
pub fn list_remotes(git: &GatedGitRunner, worktree: &Path) -> Result<Vec<RemoteEntry>, GitOpError> {
    let output = run_ok(git, worktree, &argv(["remote", "-v"]))?;
    Ok(parse_remote_verbose(&output.stdout)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiple_remotes_fetch_and_push_urls_are_distinguished() {
        let out = b"origin\thttps://github.com/acme/design.git (fetch)\n\
                    origin\tgit@github.com:acme/design.git (push)\n\
                    origin\tssh://mirror.example/acme/design.git (push)\n\
                    backup\t/Volumes/Backup/repos/my design.git (fetch)\n\
                    backup\t/Volumes/Backup/repos/my design.git (push)\n";
        let remotes = parse_remote_verbose(out).unwrap();
        assert_eq!(remotes.len(), 2);
        assert_eq!(remotes[0].name, "origin");
        assert_eq!(
            remotes[0].fetch_url.as_ref().unwrap().expose(),
            "https://github.com/acme/design.git"
        );
        let pushes: Vec<&str> = remotes[0].push_urls.iter().map(RemoteUrl::expose).collect();
        assert_eq!(
            pushes,
            [
                "git@github.com:acme/design.git",
                "ssh://mirror.example/acme/design.git"
            ]
        );
        assert_eq!(remotes[1].name, "backup");
        assert_eq!(
            remotes[1].fetch_url.as_ref().unwrap().expose(),
            "/Volumes/Backup/repos/my design.git"
        );
        assert!(parse_remote_verbose(b"").unwrap().is_empty());
    }

    #[test]
    fn malformed_output_is_rejected_without_echoing_urls() {
        for bad in [
            &b"origin https://x (fetch)\n"[..],
            b"origin\thttps://x\n",
            b"origin\thttps://x (fetch)",
            b"\thttps://x (fetch)\n",
            b"origin\t (fetch)\n",
            b"o\ta (fetch)\no\tb (fetch)\n",
        ] {
            let err = parse_remote_verbose(bad).unwrap_err();
            assert!(err.raw.is_empty(), "URL이 오류에 남음");
        }
    }

    #[test]
    fn urls_are_sensitive_in_debug_and_redacted_for_display() {
        let url = RemoteUrl::new("https://alice:ghp_SECRET123@github.com/acme/design.git");
        assert_eq!(format!("{url:?}"), "RemoteUrl(<redacted>)");
        assert_eq!(url.redacted(), "https://***@github.com/acme/design.git");
        assert!(!url.redacted().contains("SECRET"));
        let entry = RemoteEntry {
            name: "origin".into(),
            fetch_url: Some(url.clone()),
            push_urls: vec![url],
        };
        assert!(!format!("{entry:?}").contains("SECRET"));
    }

    #[test]
    fn redaction_handles_text_with_many_urls() {
        let text = "fatal: unable to access 'https://bob:tok@github.com/a/b.git/': x\n\
                    also http://u@h.example:8080/p and ssh://git@github.com/a/b \
                    git@github.com:a/b.git file:///tmp/x";
        let red = redact_credentials(text);
        assert!(!red.contains("bob") && !red.contains("tok"));
        assert!(red.contains("'https://***@github.com/a/b.git/'"));
        assert!(red.contains("http://***@h.example:8080/p"));
        assert!(red.contains("ssh://***@github.com/a/b"));
        assert!(red.contains("git@github.com:a/b.git"));
        assert!(red.contains("file:///tmp/x"));
        assert_eq!(redact_credentials("no urls here"), "no urls here");
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::test_support::{
            git_ok, hermetic_runner, init_repo_with_identity, system_git, TempDir,
        };
        use crate::trust::{GatedRunError, GitTrust};

        #[test]
        fn multiple_remote_fixture_from_real_git() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("remote-list");
            let runner = hermetic_runner(git);
            let repo = dir.path();
            init_repo_with_identity(&runner, repo);
            git_ok(
                &runner,
                repo,
                [
                    "remote",
                    "add",
                    "origin",
                    "https://github.com/acme/design.git",
                ],
            );
            git_ok(
                &runner,
                repo,
                [
                    "remote",
                    "set-url",
                    "--push",
                    "origin",
                    "git@github.com:acme/design.git",
                ],
            );
            git_ok(
                &runner,
                repo,
                [
                    "remote",
                    "set-url",
                    "--add",
                    "--push",
                    "origin",
                    "/srv/mirror one.git",
                ],
            );
            git_ok(
                &runner,
                repo,
                ["remote", "add", "backup", "/srv/backup.git"],
            );
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            let remotes = list_remotes(&gate, repo).unwrap();
            let names: Vec<&str> = remotes.iter().map(|r| r.name.as_str()).collect();
            assert_eq!(names, ["backup", "origin"]);
            let origin = &remotes[1];
            assert_eq!(
                origin.fetch_url.as_ref().unwrap().expose(),
                "https://github.com/acme/design.git"
            );
            let pushes: Vec<&str> = origin.push_urls.iter().map(RemoteUrl::expose).collect();
            assert_eq!(
                pushes,
                ["git@github.com:acme/design.git", "/srv/mirror one.git"]
            );
            // push URL 미설정 remote는 Git이 fetch URL을 push로도 보여 줌.
            assert_eq!(
                remotes[0].push_urls,
                vec![RemoteUrl::new("/srv/backup.git")]
            );

            let untrusted = GatedGitRunner::new(runner.clone(), GitTrust::NotTrusted);
            let before = runner.spawn_count();
            assert_eq!(
                list_remotes(&untrusted, repo),
                Err(GitOpError::Gate(GatedRunError::ProjectNotTrusted))
            );
            assert_eq!(runner.spawn_count(), before);
        }
    }
}
