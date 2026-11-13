//! Commit subject validator (I.2.17).
//!
//! `AGENTS.md`의 커밋 원칙과 `.githooks/commit-msg` 훅에 맞춘 두 형식만 허용합니다.
//!
//! - 구현 작업: `Task(<WBS-ID>): <비어 있지 않은 제목>` — WBS-ID는 로마 숫자 Phase와
//!   두 단계 번호(`I.1.1`)입니다.
//! - 유지보수 작업: `<Prefix>: <비어 있지 않은 제목>` — Prefix는 `Feat`, `Docs`, `Refactor`,
//!   `Fix`, `Chore`, `Test` 중 하나입니다.
//!
//! Git이 만드는 `Revert "..."` 제목도 허용합니다. 소문자 `task(...)`, 그 밖의 prefix,
//! `fixup!`·`squash!` 임시 커밋은 거부합니다. merge commit은 CI가 `--no-merges`로 제외합니다.

use crate::task_status::wbs_id::WbsId;

/// 유지보수 작업에 허용하는 prefix(`AGENTS.md` 커밋 원칙).
pub const MAINTENANCE_PREFIXES: [&str; 6] = ["Feat", "Docs", "Refactor", "Fix", "Chore", "Test"];

/// commit subject를 검증합니다. 통과 시 `None`, 위반 시 오류 코드를 반환합니다.
pub fn validate_commit_subject(subject: &str) -> Option<&'static str> {
    if subject.starts_with("Revert \"") && subject.ends_with('"') {
        return None;
    }
    if let Some(rest) = subject.strip_prefix("Task(") {
        return validate_task_subject(rest);
    }
    for prefix in MAINTENANCE_PREFIXES {
        if let Some(after) = subject.strip_prefix(prefix) {
            if let Some(after_colon) = after.strip_prefix(':') {
                return validate_title(after_colon);
            }
        }
    }
    Some("unknown-prefix")
}

/// `Task(` 뒤의 `<WBS-ID>): <제목>`을 검증합니다.
fn validate_task_subject(rest: &str) -> Option<&'static str> {
    let Some(close) = rest.find(')') else {
        return Some("malformed-id");
    };
    if WbsId::parse(&rest[..close]).is_none() {
        return Some("malformed-id");
    }
    let Some(after_colon) = rest[close + 1..].strip_prefix(':') else {
        return Some("bad-separator");
    };
    validate_title(after_colon)
}

/// `:` 뒤가 공백 하나와 비어 있지 않은 제목인지 확인합니다.
fn validate_title(after_colon: &str) -> Option<&'static str> {
    let Some(title) = after_colon.strip_prefix(' ') else {
        return Some("bad-separator");
    };
    if title.trim().is_empty() {
        return Some("empty-title");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_roman_task_subjects_pass() {
        for subject in [
            "Task(I.1.1): 저장소 기준선 검증",
            "Task(XXVIII.3.3): 마지막 작업",
            "Task(X.2.5): 중간 작업",
        ] {
            assert_eq!(validate_commit_subject(subject), None, "{subject}");
        }
    }

    #[test]
    fn agents_md_maintenance_prefixes_pass() {
        for subject in [
            "Feat: 기능 추가",
            "Docs: 기술 아키텍처 정의서 추가",
            "Refactor: 이름 정리",
            "Fix: 버그 수정",
            "Chore: task-status 레저 재생성",
            "Test: 테스트 보강",
            "Revert \"Task(I.1.1): 저장소 기준선 검증\"",
        ] {
            assert_eq!(validate_commit_subject(subject), None, "{subject}");
        }
    }

    #[test]
    fn invalid_subjects_fail() {
        let cases = [
            ("task(I.1.1): 소문자", "unknown-prefix"),
            ("docs: 소문자 prefix", "unknown-prefix"),
            ("Style: 허용되지 않은 prefix", "unknown-prefix"),
            ("Docsy: 비슷한 prefix", "unknown-prefix"),
            ("Task(1.1.1): 아라비아", "malformed-id"),
            ("Task(I.1): 두단계", "malformed-id"),
            ("Task(I.1.1): ", "empty-title"),
            ("Task(I.1.1):제목", "bad-separator"),
            ("Docs:제목", "bad-separator"),
            ("Chore: ", "empty-title"),
            ("Merge branch 'x'", "unknown-prefix"),
            ("fixup! Task(I.1.1): x", "unknown-prefix"),
            ("squash! Docs: x", "unknown-prefix"),
        ];
        for (subject, expected) in cases {
            assert_eq!(
                validate_commit_subject(subject),
                Some(expected),
                "{subject}"
            );
        }
    }
}
