//! 상태별 증거 규칙 검증 (I.2.9).
//!
//! - `verified`: commit·report·verified_at + 모든 required environment의 evidence 필요.
//! - `reported_done`: report 필요.
//! - `not_started`: commit·report·evidence가 없어야 함.
//!
//! 그 밖의 상태(in_progress, blocked, cancelled)는 본 단계에서 추가 제약을 두지 않습니다.

use crate::task_status::model::{TaskState, TaskStatusRecord};

/// 증거 규칙 위반.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceError {
    pub code: &'static str,
    pub message: String,
}

impl EvidenceError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        EvidenceError {
            code,
            message: message.into(),
        }
    }
}

/// 한 레코드의 상태별 증거 규칙을 검증합니다(빈 Vec = 통과).
pub fn validate_evidence(record: &TaskStatusRecord) -> Vec<EvidenceError> {
    let mut errors = Vec::new();
    let id = record.id.as_str();

    match record.status {
        TaskState::Verified => {
            if record.commit.is_none() {
                errors.push(EvidenceError::new(
                    "missing-commit",
                    format!("{id}: verified에 commit 필요"),
                ));
            }
            if record.report.is_none() {
                errors.push(EvidenceError::new(
                    "missing-report",
                    format!("{id}: verified에 report 필요"),
                ));
            }
            if record.verified_at.is_none() {
                errors.push(EvidenceError::new(
                    "missing-verified-at",
                    format!("{id}: verified에 verified_at 필요"),
                ));
            }
            for env in &record.required_environments {
                let covered = record.evidence.iter().any(|e| e.environment == *env);
                if !covered {
                    errors.push(EvidenceError::new(
                        "missing-environment-evidence",
                        format!("{id}: {} 환경 증거 누락", env.as_str()),
                    ));
                }
            }
        }
        TaskState::ReportedDone => {
            if record.report.is_none() {
                errors.push(EvidenceError::new(
                    "missing-report",
                    format!("{id}: reported_done에 report 필요"),
                ));
            }
        }
        TaskState::NotStarted => {
            if record.commit.is_some() {
                errors.push(EvidenceError::new(
                    "unexpected-commit",
                    format!("{id}: not_started에 commit이 있으면 안 됨"),
                ));
            }
            if record.report.is_some() {
                errors.push(EvidenceError::new(
                    "unexpected-report",
                    format!("{id}: not_started에 report가 있으면 안 됨"),
                ));
            }
            if !record.evidence.is_empty() {
                errors.push(EvidenceError::new(
                    "unexpected-evidence",
                    format!("{id}: not_started에 evidence가 있으면 안 됨"),
                ));
            }
        }
        TaskState::InProgress | TaskState::Blocked | TaskState::Cancelled => {}
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_status::model::{VerificationEnvironment, VerificationEvidence};

    fn verified() -> TaskStatusRecord {
        let mut r =
            TaskStatusRecord::new("I.1.1".to_string(), "t".to_string(), TaskState::Verified);
        r.required_environments = vec![
            VerificationEnvironment::CiMac,
            VerificationEnvironment::CiWin,
        ];
        r.evidence = vec![
            VerificationEvidence::new(VerificationEnvironment::CiMac, "url-mac".to_string()),
            VerificationEvidence::new(VerificationEnvironment::CiWin, "url-win".to_string()),
        ];
        r.commit = Some("a1b2c3d".to_string());
        r.report = Some("docs/task-reports/I.1.1.md".to_string());
        r.verified_at = Some("2026-06-24".to_string());
        r
    }

    fn codes(record: &TaskStatusRecord) -> Vec<&'static str> {
        validate_evidence(record).iter().map(|e| e.code).collect()
    }

    #[test]
    fn valid_verified_passes() {
        assert!(validate_evidence(&verified()).is_empty());
    }

    #[test]
    fn verified_missing_one_environment_fails() {
        let mut r = verified();
        r.evidence.pop(); // CI-WIN 증거 제거
        assert!(codes(&r).contains(&"missing-environment-evidence"));
    }

    #[test]
    fn verified_missing_report_fails() {
        let mut r = verified();
        r.report = None;
        assert!(codes(&r).contains(&"missing-report"));
    }

    #[test]
    fn reported_done_requires_report() {
        let mut r = TaskStatusRecord::new(
            "I.1.2".to_string(),
            "t".to_string(),
            TaskState::ReportedDone,
        );
        r.required_environments = vec![VerificationEnvironment::DevMac];
        assert!(codes(&r).contains(&"missing-report"));
        r.report = Some("docs/task-reports/I.1.2.md".to_string());
        assert!(validate_evidence(&r).is_empty());
    }

    #[test]
    fn not_started_rejects_evidence() {
        let mut r =
            TaskStatusRecord::new("I.1.3".to_string(), "t".to_string(), TaskState::NotStarted);
        r.evidence = vec![VerificationEvidence::new(
            VerificationEnvironment::DevMac,
            "x".to_string(),
        )];
        assert!(codes(&r).contains(&"unexpected-evidence"));
    }
}
