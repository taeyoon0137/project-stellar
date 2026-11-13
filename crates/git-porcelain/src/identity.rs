//! commit identity 해석 (IV.4.5·XIV.8.3 공통).
//!
//! `git commit`이 author·committer identity를 정하는 순서를 따라, **명시적으로 설정된** 값이
//! 있는지 읽기 전용으로 확인합니다. 프로젝트 생성 흐름([`crate::repo::check_user_identity`])과
//! 커밋 흐름([`crate::commit::check_commit_identity`])이 이 한 구현을 공유합니다.
//!
//! | 항목 | 우선순위(앞이 이김) |
//! |---|---|
//! | author 이름 | `GIT_AUTHOR_NAME` → `author.name` → `user.name` |
//! | author 이메일 | `GIT_AUTHOR_EMAIL` → `author.email` → `user.email` → `EMAIL` |
//! | committer 이름 | `GIT_COMMITTER_NAME` → `committer.name` → `user.name` |
//! | committer 이메일 | `GIT_COMMITTER_EMAIL` → `committer.email` → `user.email` → `EMAIL` |
//!
//! - 환경 변수는 **자식 Git이 실제로 보는 값**([`GitExec::child_env_var`], allowlist·override
//!   적용 결과)만 셉니다. 환경 변수가 있으면 Git은 빈 문자열이어도 그 값을 쓰므로, 빈 값은 다음
//!   단계로 넘어가지 않고 미설정으로 봅니다.
//! - `author.*`/`committer.*`가 빈 값이면 Git처럼 `user.*`로 넘어갑니다. `user.*`가 (빈 값으로라도)
//!   설정되어 있으면 Git은 `EMAIL`을 보지 않으므로 여기서도 보지 않습니다.
//! - 같은 key가 여러 config 파일에 있으면 Git처럼 **마지막 값**을 씁니다.
//! - Git의 hostname·계정 정보 기반 자동 추측은 identity로 인정하지 않습니다. 사용자가 명시적으로
//!   설정하도록 UI가 안내합니다(Stellar는 config를 쓰지 않습니다).
//!
//! config는 `git config -z --get-regexp`로 이름 key와 이메일 key를 각각 한 번씩 읽습니다(실행 2회).
//! 종료 코드 1은 일치 key 없음이고, 그 밖의 비영 종료(손상된 config 등)는 누락으로 숨기지 않고
//! 오류로 반환합니다.

use std::ffi::OsString;
use std::path::Path;

use crate::command::GitCommandFailure;
use crate::decode::split_nul_terminated;
use crate::runner::argv;
use crate::trust::{GatedRunError, GitExec};

/// commit identity config key(UI 안내용).
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
///
/// `name_missing`은 author 또는 committer 이름이, `email_missing`은 author 또는 committer 이메일이
/// 해석되지 않았다는 뜻입니다. `user.name`/`user.email`을 설정하면 두 역할이 모두 해결됩니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissingIdentity {
    /// 이름 누락.
    pub name_missing: bool,
    /// 이메일 누락.
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

/// identity 확인 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityReadError {
    /// trust gate 거부 또는 Git 실행 실패.
    Gate(GatedRunError),
    /// `git config` 비영 종료(종료 코드 1 제외, Git 결과 보존).
    Git(GitCommandFailure),
}

impl std::fmt::Display for IdentityReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdentityReadError::Gate(e) => write!(f, "{e}"),
            IdentityReadError::Git(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for IdentityReadError {}

/// 이름 또는 이메일 한 종류의 identity 원천.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct FieldSources {
    /// `user.<field>`의 마지막 값(`Some(빈 값)`은 설정은 됐지만 비어 있음).
    user: Option<Vec<u8>>,
    /// `author.<field>`의 마지막 값.
    author: Option<Vec<u8>>,
    /// `committer.<field>`의 마지막 값.
    committer: Option<Vec<u8>>,
}

/// 해석 역할.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Author,
    Committer,
}

/// 확인할 필드.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Email,
}

impl Field {
    fn suffix(self) -> &'static str {
        match self {
            Field::Name => "name",
            Field::Email => "email",
        }
    }

    fn env_name(self, role: Role) -> &'static str {
        match (role, self) {
            (Role::Author, Field::Name) => "GIT_AUTHOR_NAME",
            (Role::Author, Field::Email) => "GIT_AUTHOR_EMAIL",
            (Role::Committer, Field::Name) => "GIT_COMMITTER_NAME",
            (Role::Committer, Field::Email) => "GIT_COMMITTER_EMAIL",
        }
    }
}

/// `git config -z --get-regexp` 출력(`key\nvalue\0` 또는 값 없는 `key\0`)을 원천으로 모읍니다.
fn parse_field_sources(stdout: &[u8], field: Field) -> FieldSources {
    let mut sources = FieldSources::default();
    // 잘린 출력은 NUL로 끝난 완결 레코드만 봅니다(덜 읽은 값은 누락 쪽으로 판정됩니다).
    let complete = match stdout.iter().rposition(|b| *b == 0) {
        Some(last) => &stdout[..=last],
        None => &[][..],
    };
    for record in split_nul_terminated(complete).unwrap_or_default() {
        let (key, value) = match record.iter().position(|b| *b == b'\n') {
            Some(i) => (&record[..i], record[i + 1..].to_vec()),
            // 값 없는 boolean 형식(`[user] name`)은 Git이 빈 이름으로 취급합니다.
            None => (record, Vec::new()),
        };
        let section = match key.strip_suffix(format!(".{}", field.suffix()).as_bytes()) {
            Some(section) => section,
            None => continue,
        };
        // 마지막 값이 이깁니다.
        match section {
            b"user" => sources.user = Some(value),
            b"author" => sources.author = Some(value),
            b"committer" => sources.committer = Some(value),
            _ => {}
        }
    }
    sources
}

/// 한 역할·필드가 명시적으로 해석되는지 판정합니다.
fn field_resolves(
    sources: &FieldSources,
    field: Field,
    role: Role,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> bool {
    if let Some(value) = env(field.env_name(role)) {
        return !value.is_empty();
    }
    let role_value = match role {
        Role::Author => &sources.author,
        Role::Committer => &sources.committer,
    };
    if role_value.as_ref().is_some_and(|v| !v.is_empty()) {
        return true;
    }
    if let Some(user) = &sources.user {
        return !user.is_empty();
    }
    match field {
        Field::Name => false,
        Field::Email => env("EMAIL").is_some_and(|v| !v.is_empty()),
    }
}

/// 한 필드의 config 원천을 읽습니다(`git config -z --get-regexp`, 실행 1회).
fn read_field_sources<G: GitExec + ?Sized>(
    git: &G,
    worktree: &Path,
    field: Field,
) -> Result<FieldSources, IdentityReadError> {
    let pattern = format!("^(user|author|committer)\\.{}$", field.suffix());
    let output = git
        .exec(
            &argv(["config", "-z", "--get-regexp", pattern.as_str()]),
            worktree,
            None,
        )
        .map_err(IdentityReadError::Gate)?;
    match output.exit_code {
        Some(0) => Ok(parse_field_sources(&output.stdout, field)),
        Some(1) => Ok(FieldSources::default()),
        _ => Err(IdentityReadError::Git(GitCommandFailure {
            exit_code: output.exit_code,
            stdout: output.stdout,
            stderr: output.stderr,
        })),
    }
}

/// author·committer identity가 모두 명시적으로 해석되는지 확인합니다(읽기 전용, Git 실행 2회).
///
/// 모두 해석되면 `None`, 하나라도 없으면 누락 항목을 반환합니다. config를 쓰지 않습니다.
pub fn resolve_commit_identity<G: GitExec + ?Sized>(
    git: &G,
    worktree: &Path,
) -> Result<Option<MissingIdentity>, IdentityReadError> {
    let names = read_field_sources(git, worktree, Field::Name)?;
    let emails = read_field_sources(git, worktree, Field::Email)?;
    let env = |name: &str| git.child_env_var(name);
    let resolves = |sources: &FieldSources, field: Field| {
        [Role::Author, Role::Committer]
            .into_iter()
            .all(|role| field_resolves(sources, field, role, &env))
    };
    let name_missing = !resolves(&names, Field::Name);
    let email_missing = !resolves(&emails, Field::Email);
    if name_missing || email_missing {
        Ok(Some(MissingIdentity {
            name_missing,
            email_missing,
        }))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<OsString> {
        None
    }

    #[test]
    fn parses_regexp_records_last_value_wins() {
        let out = b"user.name\nGlobal\0author.name\n\0user.name\nLocal\0committer.name\nC\0";
        let s = parse_field_sources(out, Field::Name);
        assert_eq!(s.user, Some(b"Local".to_vec()));
        assert_eq!(s.author, Some(Vec::new()));
        assert_eq!(s.committer, Some(b"C".to_vec()));
        // 값 없는 boolean 형식은 빈 값.
        let s = parse_field_sources(b"user.email\0", Field::Email);
        assert_eq!(s.user, Some(Vec::new()));
        // 다른 필드 key는 무시.
        let s = parse_field_sources(b"user.email\na@b\0", Field::Name);
        assert_eq!(s, FieldSources::default());
    }

    #[test]
    fn precedence_matches_git_resolution() {
        let sources =
            |user: Option<&str>, author: Option<&str>, committer: Option<&str>| FieldSources {
                user: user.map(|v| v.as_bytes().to_vec()),
                author: author.map(|v| v.as_bytes().to_vec()),
                committer: committer.map(|v| v.as_bytes().to_vec()),
            };
        // user.name만 있으면 두 역할 모두 해석.
        let s = sources(Some("U"), None, None);
        assert!(field_resolves(&s, Field::Name, Role::Author, &no_env));
        assert!(field_resolves(&s, Field::Name, Role::Committer, &no_env));
        // author.name만 있으면 committer는 미해석.
        let s = sources(None, Some("A"), None);
        assert!(field_resolves(&s, Field::Name, Role::Author, &no_env));
        assert!(!field_resolves(&s, Field::Name, Role::Committer, &no_env));
        // 빈 author.name은 user.name으로 넘어감.
        let s = sources(Some("U"), Some(""), None);
        assert!(field_resolves(&s, Field::Name, Role::Author, &no_env));
        // 빈 user.name은 누락.
        let s = sources(Some(""), None, None);
        assert!(!field_resolves(&s, Field::Name, Role::Author, &no_env));

        // 환경 변수가 config보다 우선하고, 빈 환경 변수는 아래로 넘어가지 않음.
        let env_name = |name: &str| (name == "GIT_AUTHOR_NAME").then(|| OsString::from("Env"));
        let empty_env = |name: &str| (name == "GIT_AUTHOR_NAME").then(OsString::new);
        let s = sources(None, None, None);
        assert!(field_resolves(&s, Field::Name, Role::Author, &env_name));
        assert!(!field_resolves(&s, Field::Name, Role::Committer, &env_name));
        let s = sources(Some("U"), None, None);
        assert!(!field_resolves(&s, Field::Name, Role::Author, &empty_env));

        // EMAIL은 user.email이 아예 없을 때만 씀(이름에는 쓰지 않음).
        let email_env = |name: &str| (name == "EMAIL").then(|| OsString::from("e@x"));
        let s = sources(None, None, None);
        assert!(field_resolves(&s, Field::Email, Role::Author, &email_env));
        assert!(field_resolves(
            &s,
            Field::Email,
            Role::Committer,
            &email_env
        ));
        assert!(!field_resolves(&s, Field::Name, Role::Author, &email_env));
        let s = sources(Some(""), None, None);
        assert!(!field_resolves(&s, Field::Email, Role::Author, &email_env));
    }
}
