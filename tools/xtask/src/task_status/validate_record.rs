//! 단일 레코드 검증 (I.2.6).
//!
//! 한 `TaskStatusRecord`의 필수값·nullable·형식 규칙만 검사합니다.
//! 다른 레코드 존재 여부, 상태별 증거 규칙, 선행 graph는 검사하지 않습니다.

use crate::task_status::model::TaskStatusRecord;

/// 레코드 단위 검증 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordError {
    pub code: &'static str,
    pub message: String,
}

impl RecordError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        RecordError {
            code,
            message: message.into(),
        }
    }
}

/// 레코드를 검증하고 발견된 모든 오류를 반환합니다(빈 Vec = 통과).
pub fn validate_record(record: &TaskStatusRecord) -> Vec<RecordError> {
    let mut errors = Vec::new();

    if record.title.trim().is_empty() {
        errors.push(RecordError::new(
            "empty-title",
            format!("{}: title이 비어 있습니다", record.id),
        ));
    }

    if record.required_environments.is_empty() {
        errors.push(RecordError::new(
            "empty-required-environments",
            format!("{}: required_environments가 비어 있습니다", record.id),
        ));
    } else {
        let mut seen = Vec::new();
        for env in &record.required_environments {
            if seen.contains(env) {
                errors.push(RecordError::new(
                    "duplicate-environment",
                    format!("{}: 중복 환경 {}", record.id, env.as_str()),
                ));
            } else {
                seen.push(*env);
            }
        }
    }

    if let Some(report) = &record.report {
        if !is_valid_relative_path(report) {
            errors.push(RecordError::new(
                "invalid-report-path",
                format!("{}: 잘못된 report 상대 경로: {report}", record.id),
            ));
        }
    }

    if let Some(commit) = &record.commit {
        if !is_valid_commit_sha(commit) {
            errors.push(RecordError::new(
                "invalid-commit-sha",
                format!("{}: 잘못된 commit SHA: {commit}", record.id),
            ));
        }
    }

    errors
}

/// 상대 경로 규칙: 비어 있지 않고, 절대 경로가 아니며, `..`·빈 segment가 없어야 합니다.
fn is_valid_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && path
            .split('/')
            .all(|segment| segment != ".." && !segment.is_empty())
}

/// git SHA 형식: 7~40자의 16진수.
fn is_valid_commit_sha(sha: &str) -> bool {
    (7..=40).contains(&sha.len()) && sha.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_status::model::{TaskState, VerificationEnvironment};

    fn base() -> TaskStatusRecord {
        let mut record = TaskStatusRecord::new(
            "I.1.1".to_string(),
            "title".to_string(),
            TaskState::Verified,
        );
        record.required_environments = vec![VerificationEnvironment::DevMac];
        record.report = Some("docs/task-reports/I.1.1.md".to_string());
        record.commit = Some("a1b2c3d".to_string());
        record
    }

    #[test]
    fn valid_record_passes() {
        assert!(validate_record(&base()).is_empty());
    }

    #[test]
    fn table_driven_field_errors() {
        struct Case {
            mutate: fn(&mut TaskStatusRecord),
            expect: &'static str,
        }
        let cases = [
            Case {
                mutate: |r| r.title = "   ".to_string(),
                expect: "empty-title",
            },
            Case {
                mutate: |r| r.required_environments.clear(),
                expect: "empty-required-environments",
            },
            Case {
                mutate: |r| {
                    r.required_environments = vec![
                        VerificationEnvironment::DevMac,
                        VerificationEnvironment::DevMac,
                    ]
                },
                expect: "duplicate-environment",
            },
            Case {
                mutate: |r| r.report = Some("/abs/path.md".to_string()),
                expect: "invalid-report-path",
            },
            Case {
                mutate: |r| r.report = Some("../escape.md".to_string()),
                expect: "invalid-report-path",
            },
            Case {
                mutate: |r| r.commit = Some("nothex".to_string()),
                expect: "invalid-commit-sha",
            },
            Case {
                mutate: |r| r.commit = Some("abc".to_string()),
                expect: "invalid-commit-sha",
            },
        ];
        for case in cases {
            let mut record = base();
            (case.mutate)(&mut record);
            let codes: Vec<&str> = validate_record(&record).iter().map(|e| e.code).collect();
            assert!(
                codes.contains(&case.expect),
                "expected {} in {codes:?}",
                case.expect
            );
        }
    }
}
