//! ref 이름 검증(Git ref 규칙의 안전한 부분집합).
//!
//! UI 입력을 Git 명령 인자로 넘기기 전에 option 주입(`-`로 시작)과 ref 문법 위반을 걸러냅니다. 최종 검증은
//! Git이 다시 합니다(`git branch`·`git push` 등). 허용하지 않는 것: 빈 이름, `-`·`/`로 시작, `/`·`.`·
//! `.lock`으로 끝남, `..`, `//`, `@{`, `.`으로 시작하는 component, 공백·제어문자, `:` `~` `^` `?` `*`
//! `[` `\`, 단독 `@`.

/// ref 본문(예: branch 이름 `feature/x`)이 안전한지.
pub fn is_valid_ref_body(body: &str) -> bool {
    !body.is_empty()
        && body != "@"
        && !body.starts_with('-')
        && !body.starts_with('/')
        && !body.ends_with('/')
        && !body.ends_with('.')
        && !body.ends_with(".lock")
        && !body.contains("..")
        && !body.contains("//")
        && !body.contains("@{")
        && !body
            .split('/')
            .any(|c| c.starts_with('.') || c.ends_with(".lock"))
        && !body.chars().any(|c| {
            c.is_control()
                || c.is_whitespace()
                || matches!(c, ':' | '~' | '^' | '?' | '*' | '[' | '\\')
        })
}

/// `refs/heads/`·`refs/tags/` 전체 ref 이름이 안전한지.
pub fn is_valid_branch_or_tag_ref(full: &str) -> bool {
    full.strip_prefix("refs/heads/")
        .or_else(|| full.strip_prefix("refs/tags/"))
        .is_some_and(is_valid_ref_body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_body_rules() {
        for ok in ["main", "feature/디자인", "v1.0", "a-b_c", "x/y/z"] {
            assert!(is_valid_ref_body(ok), "{ok}");
        }
        for bad in [
            "",
            "@",
            "-f",
            "/a",
            "a/",
            "a.",
            "a.lock",
            "a..b",
            "a//b",
            "a@{1}",
            ".a",
            "a/.b",
            "a b",
            "a:b",
            "a~1",
            "a^",
            "a?",
            "a*",
            "a[",
            "a\\b",
            "a\tb",
            "a/b.lock/c",
        ] {
            assert!(!is_valid_ref_body(bad), "{bad:?}");
        }
        assert!(is_valid_branch_or_tag_ref("refs/heads/main"));
        assert!(is_valid_branch_or_tag_ref("refs/tags/v1"));
        assert!(!is_valid_branch_or_tag_ref("refs/remotes/origin/main"));
        assert!(!is_valid_branch_or_tag_ref("main"));
    }
}
