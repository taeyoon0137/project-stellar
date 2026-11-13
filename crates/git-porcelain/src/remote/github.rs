//! GitHub remote URL 형식 검증 (XIV.9.2).
//!
//! 초기 연결 UI는 GitHub 전용입니다(아키텍처 §25.3). URL은 **canonical pattern**만 문자열로 확인하고
//! GitHub API는 호출하지 않습니다(저장소 존재 확인은 XIV.9.3의 `git ls-remote`).
//!
//! | 형식 | 예 |
//! |---|---|
//! | [`GitHubUrlForm::Https`] | `https://github.com/<owner>/<repo>[.git]` |
//! | [`GitHubUrlForm::SshScp`] | `git@github.com:<owner>/<repo>[.git]` |
//! | [`GitHubUrlForm::SshUrl`] | `ssh://git@github.com/<owner>/<repo>[.git]` |
//!
//! - host는 정확히 `github.com`(소문자)이어야 합니다. 다른 host는 [`GitHubUrlCheck::NotGitHub`].
//! - GitHub host인데 형식이 canonical이 아니면 [`GitHubUrlCheck::Invalid`]로 이유를 돌려줍니다:
//!   URL 안의 자격 증명(`https://user:token@github.com/...`; Git config에 token이 저장되므로 거부),
//!   `http://`, port, query·fragment, 추가 경로, 잘못된 owner·repo 이름.
//! - owner: 영숫자·`-` 1~39자, `-`로 시작·끝나지 않음. repo: 영숫자·`.`·`_`·`-` 1~100자, `.`/`..` 불가,
//!   끝의 `.git`은 한 번만 떼어 냅니다.

/// GitHub URL 형식.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitHubUrlForm {
    /// `https://github.com/<owner>/<repo>`.
    Https,
    /// `git@github.com:<owner>/<repo>`.
    SshScp,
    /// `ssh://git@github.com/<owner>/<repo>`.
    SshUrl,
}

/// 검증된 GitHub 저장소 참조.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubRepo {
    /// URL 형식.
    pub form: GitHubUrlForm,
    /// owner(사용자·조직).
    pub owner: String,
    /// 저장소 이름(`.git` 제외).
    pub repo: String,
}

/// GitHub host URL이 canonical이 아닌 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitHubUrlProblem {
    /// URL에 사용자 이름·token 등 자격 증명이 들어 있음.
    CredentialsInUrl,
    /// 암호화되지 않은 `http://`.
    InsecureScheme,
    /// port 지정.
    PortNotAllowed,
    /// query·fragment.
    QueryOrFragment,
    /// `<owner>/<repo>` 외 경로(추가 segment·끝 `/` 포함).
    UnexpectedPath,
    /// owner 이름 규칙 위반.
    InvalidOwner,
    /// repo 이름 규칙 위반.
    InvalidRepo,
    /// SSH 사용자가 `git`이 아님.
    UnexpectedSshUser,
}

/// GitHub URL 분류 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubUrlCheck {
    /// canonical GitHub URL.
    GitHub(GitHubRepo),
    /// GitHub host가 아님(외부 터미널에서 등록).
    NotGitHub,
    /// GitHub host이지만 canonical이 아님.
    Invalid(GitHubUrlProblem),
}

impl GitHubUrlCheck {
    /// canonical GitHub URL인지.
    pub fn is_github(&self) -> bool {
        matches!(self, GitHubUrlCheck::GitHub(_))
    }
}

const HOST: &str = "github.com";

fn valid_owner(owner: &str) -> bool {
    (1..=39).contains(&owner.len())
        && owner
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !owner.starts_with('-')
        && !owner.ends_with('-')
}

fn valid_repo(repo: &str) -> bool {
    (1..=100).contains(&repo.len())
        && repo
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && repo != "."
        && repo != ".."
}

/// `<owner>/<repo>[.git]` 경로를 검증합니다.
fn parse_owner_repo(path: &str, form: GitHubUrlForm) -> GitHubUrlCheck {
    if path.contains('?') || path.contains('#') {
        return GitHubUrlCheck::Invalid(GitHubUrlProblem::QueryOrFragment);
    }
    let mut parts = path.split('/');
    let (Some(owner), Some(repo), None) = (parts.next(), parts.next(), parts.next()) else {
        return GitHubUrlCheck::Invalid(GitHubUrlProblem::UnexpectedPath);
    };
    if !valid_owner(owner) {
        return GitHubUrlCheck::Invalid(GitHubUrlProblem::InvalidOwner);
    }
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    if !valid_repo(repo) {
        return GitHubUrlCheck::Invalid(GitHubUrlProblem::InvalidRepo);
    }
    GitHubUrlCheck::GitHub(GitHubRepo {
        form,
        owner: owner.to_string(),
        repo: repo.to_string(),
    })
}

/// authority(`[userinfo@]host[:port]`)를 확인해 host가 GitHub인지와 문제를 돌려줍니다.
fn check_authority(authority: &str, ssh: bool) -> Result<(), Option<GitHubUrlProblem>> {
    let (userinfo, hostport) = match authority.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, authority),
    };
    let (host, port) = match hostport.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (hostport, None),
    };
    if host != HOST {
        return Err(None);
    }
    if port.is_some() {
        return Err(Some(GitHubUrlProblem::PortNotAllowed));
    }
    match (ssh, userinfo) {
        (false, None) => Ok(()),
        (false, Some(_)) => Err(Some(GitHubUrlProblem::CredentialsInUrl)),
        (true, Some("git")) => Ok(()),
        (true, Some(u)) if u.contains(':') => Err(Some(GitHubUrlProblem::CredentialsInUrl)),
        (true, _) => Err(Some(GitHubUrlProblem::UnexpectedSshUser)),
    }
}

/// URL이 canonical GitHub 형식인지 분류합니다(네트워크·API 호출 없음).
pub fn classify_github_url(url: &str) -> GitHubUrlCheck {
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return if url.contains(HOST) {
            GitHubUrlCheck::Invalid(GitHubUrlProblem::UnexpectedPath)
        } else {
            GitHubUrlCheck::NotGitHub
        };
    }
    for (scheme, form, ssh) in [
        ("https://", GitHubUrlForm::Https, false),
        ("ssh://", GitHubUrlForm::SshUrl, true),
        ("http://", GitHubUrlForm::Https, false),
    ] {
        if let Some(rest) = url.strip_prefix(scheme) {
            let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
            if let Err(problem) = check_authority(authority, ssh) {
                return problem.map_or(GitHubUrlCheck::NotGitHub, GitHubUrlCheck::Invalid);
            }
            if scheme == "http://" {
                return GitHubUrlCheck::Invalid(GitHubUrlProblem::InsecureScheme);
            }
            return parse_owner_repo(path, form);
        }
    }
    if url.contains("://") {
        return GitHubUrlCheck::NotGitHub;
    }
    // scp 형식: [user@]host:path (첫 ':' 앞에 '/'가 없어야 함).
    match url.split_once(':') {
        Some((authority, path)) if !authority.contains('/') => {
            let (user, host) = match authority.rsplit_once('@') {
                Some((u, h)) => (Some(u), h),
                None => (None, authority),
            };
            if host != HOST {
                return GitHubUrlCheck::NotGitHub;
            }
            if user != Some("git") {
                return GitHubUrlCheck::Invalid(GitHubUrlProblem::UnexpectedSshUser);
            }
            parse_owner_repo(path, GitHubUrlForm::SshScp)
        }
        _ => GitHubUrlCheck::NotGitHub,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(form: GitHubUrlForm, owner: &str, name: &str) -> GitHubUrlCheck {
        GitHubUrlCheck::GitHub(GitHubRepo {
            form,
            owner: owner.into(),
            repo: name.into(),
        })
    }

    #[test]
    fn canonical_github_urls_are_accepted() {
        use GitHubUrlForm::*;
        let cases = [
            (
                "https://github.com/acme/design",
                repo(Https, "acme", "design"),
            ),
            (
                "https://github.com/acme/design.git",
                repo(Https, "acme", "design"),
            ),
            (
                "https://github.com/a-b/x.y_z-1.git",
                repo(Https, "a-b", "x.y_z-1"),
            ),
            (
                "git@github.com:acme/design.git",
                repo(SshScp, "acme", "design"),
            ),
            (
                "git@github.com:Acme9/Design",
                repo(SshScp, "Acme9", "Design"),
            ),
            (
                "ssh://git@github.com/acme/design.git",
                repo(SshUrl, "acme", "design"),
            ),
            (
                "https://github.com/acme/repo.git.git",
                repo(Https, "acme", "repo.git"),
            ),
        ];
        for (url, expected) in cases {
            assert_eq!(classify_github_url(url), expected, "{url}");
            assert!(classify_github_url(url).is_github());
        }
    }

    #[test]
    fn non_github_urls_are_classified_not_github() {
        for url in [
            "https://gitlab.com/acme/design.git",
            "https://github.com.evil.example/acme/design.git",
            "https://GitHub.com/acme/design.git",
            "git@gitlab.com:acme/design.git",
            "ssh://git@bitbucket.org/acme/design.git",
            "/Volumes/repos/design.git",
            "../design.git",
            "file:///tmp/design.git",
            "C:/repos/design.git",
            "",
            "github.com/acme/design",
        ] {
            assert_eq!(
                classify_github_url(url),
                GitHubUrlCheck::NotGitHub,
                "{url:?}"
            );
        }
    }

    #[test]
    fn github_host_with_non_canonical_form_is_invalid_with_reason() {
        use GitHubUrlProblem::*;
        let cases = [
            (
                "https://alice:ghp_x@github.com/acme/design.git",
                CredentialsInUrl,
            ),
            (
                "https://ghp_token@github.com/acme/design.git",
                CredentialsInUrl,
            ),
            ("ssh://git:pw@github.com/acme/design.git", CredentialsInUrl),
            ("http://github.com/acme/design.git", InsecureScheme),
            ("https://github.com:443/acme/design.git", PortNotAllowed),
            ("ssh://git@github.com:22/acme/design.git", PortNotAllowed),
            ("https://github.com/acme/design.git?x=1", QueryOrFragment),
            ("https://github.com/acme/design#readme", QueryOrFragment),
            ("https://github.com/acme/design/", UnexpectedPath),
            ("https://github.com/acme/design/tree/main", UnexpectedPath),
            ("https://github.com/acme", UnexpectedPath),
            ("https://github.com/", UnexpectedPath),
            ("https://github.com/-acme/design", InvalidOwner),
            ("https://github.com/acme-/design", InvalidOwner),
            ("https://github.com/ac_me/design", InvalidOwner),
            (
                &format!("https://github.com/{}/d", "a".repeat(40)),
                InvalidOwner,
            ),
            ("https://github.com/acme/.git", InvalidRepo),
            ("https://github.com/acme/..", InvalidRepo),
            ("https://github.com/acme/de sign", UnexpectedPath),
            ("https://github.com/acme/de%20sign", InvalidRepo),
            ("alice@github.com:acme/design.git", UnexpectedSshUser),
            ("github.com:acme/design.git", UnexpectedSshUser),
            ("ssh://alice@github.com/acme/design.git", UnexpectedSshUser),
        ];
        for (url, problem) in cases {
            assert_eq!(
                classify_github_url(url),
                GitHubUrlCheck::Invalid(problem),
                "{url}"
            );
        }
    }
}
