//! Commit subject validator (I.2.17).
//!
//! 허용 형식은 정확히 `Task(<WBS-ID>): <비어 있지 않은 제목>` 하나뿐입니다.
//! merge·revert·Docs·Chore·소문자 `task` 예외를 두지 않습니다.

use crate::task_status::wbs_id::WbsId;

/// commit subject를 검증합니다. 통과 시 `None`, 위반 시 오류 코드를 반환합니다.
pub fn validate_commit_subject(subject: &str) -> Option<&'static str> {
    let Some(rest) = subject.strip_prefix("Task(") else {
        return Some("not-task-prefix");
    };
    let Some(close) = rest.find(')') else {
        return Some("malformed-id");
    };
    let id = &rest[..close];
    if WbsId::parse(id).is_none() {
        return Some("malformed-id");
    }
    let after = &rest[close + 1..];
    let Some(title) = after.strip_prefix(": ") else {
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
    fn valid_roman_subjects_pass() {
        for subject in [
            "Task(I.1.1): 저장소 기준선 검증",
            "Task(XXVIII.3.3): 마지막 작업",
            "Task(X.2.5): 중간 작업",
        ] {
            assert_eq!(validate_commit_subject(subject), None, "{subject}");
        }
    }

    #[test]
    fn invalid_subjects_fail() {
        let cases = [
            ("task(I.1.1): 소문자", "not-task-prefix"),
            ("Task(1.1.1): 아라비아", "malformed-id"),
            ("Docs: 문서", "not-task-prefix"),
            ("Chore: 설정", "not-task-prefix"),
            ("Task(I.1.1): ", "empty-title"),
            ("Task(I.1.1):제목", "bad-separator"),
            ("Task(I.1): 두단계", "malformed-id"),
            ("Merge branch 'x'", "not-task-prefix"),
            ("Revert \"Task(I.1.1): x\"", "not-task-prefix"),
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
