//! verify-task 검증 로직 (I.4.2).
//!
//! ledger의 선행 상태, 상태별 필수 필드·증거, report, 그리고 II.1.2 이전의
//! 조기 workspace build 요구를 검사합니다.

use crate::task_status::model::{TaskState, TaskStatusRecord, VerificationEnvironment};
use crate::task_status::validate_evidence::validate_evidence;
use crate::task_status::validate_record::validate_record;

/// verify-task 문제.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyProblem {
    pub code: &'static str,
    pub message: String,
}

impl VerifyProblem {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        VerifyProblem {
            code,
            message: message.into(),
        }
    }
}

/// 한 작업을 검증합니다(빈 Vec = 통과).
///
/// - `report_exists`: 보고서 파일 존재 여부(호출자가 확인).
/// - `workspace_ready`: 제품 workspace(II.1.2) 검증 완료 여부.
pub fn verify_task(
    records: &[TaskStatusRecord],
    id: &str,
    report_exists: bool,
    workspace_ready: bool,
) -> Vec<VerifyProblem> {
    let Some(record) = records.iter().find(|r| r.id == id) else {
        return vec![VerifyProblem::new(
            "unknown-task",
            format!("ledger에 없는 작업: {id}"),
        )];
    };

    let mut problems = Vec::new();

    for dep in &record.depends_on {
        match records.iter().find(|r| &r.id == dep) {
            Some(d) if d.status == TaskState::Verified => {}
            Some(_) => problems.push(VerifyProblem::new(
                "missing-dependency",
                format!("선행 {dep}가 verified 아님"),
            )),
            None => problems.push(VerifyProblem::new(
                "missing-dependency",
                format!("선행 {dep}가 ledger에 없음"),
            )),
        }
    }

    for e in validate_record(record) {
        problems.push(VerifyProblem::new(e.code, e.message));
    }
    for e in validate_evidence(record) {
        problems.push(VerifyProblem::new(e.code, e.message));
    }

    if record.status == TaskState::Verified && !report_exists {
        problems.push(VerifyProblem::new(
            "missing-report",
            format!("{id}: 보고서 파일이 없습니다"),
        ));
    }

    if record.status == TaskState::Verified && !workspace_ready && requires_workspace_build(record)
    {
        problems.push(VerifyProblem::new(
            "premature-workspace-requirement",
            format!("{id}: II.1.2 이전에는 workspace build 환경 검증을 요구할 수 없습니다"),
        ));
    }

    problems
}

/// 제품 workspace build가 필요한 환경(CI-MAC/CI-WIN)을 요구하는지 확인합니다.
fn requires_workspace_build(record: &TaskStatusRecord) -> bool {
    record.required_environments.iter().any(|env| {
        matches!(
            env,
            VerificationEnvironment::CiMac | VerificationEnvironment::CiWin
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_status::model::VerificationEvidence;

    fn verified_dev(id: &str, deps: &[&str]) -> TaskStatusRecord {
        let mut r = TaskStatusRecord::new(id.to_string(), id.to_string(), TaskState::Verified);
        r.depends_on = deps.iter().map(|d| d.to_string()).collect();
        r.required_environments = vec![VerificationEnvironment::DevMac];
        r.evidence = vec![VerificationEvidence::new(
            VerificationEnvironment::DevMac,
            format!("docs/task-reports/{id}.md"),
        )];
        r.commit = Some("a1b2c3d".to_string());
        r.report = Some(format!("docs/task-reports/{id}.md"));
        r.verified_at = Some("2026-06-24".to_string());
        r
    }

    #[test]
    fn success_has_no_problems() {
        let records = [verified_dev("I.1.1", &[])];
        assert!(verify_task(&records, "I.1.1", true, true).is_empty());
    }

    #[test]
    fn missing_dependency_detected() {
        let records = [verified_dev("I.1.2", &["I.1.1"])];
        let codes: Vec<&str> = verify_task(&records, "I.1.2", true, true)
            .iter()
            .map(|p| p.code)
            .collect();
        assert!(codes.contains(&"missing-dependency"));
    }

    #[test]
    fn missing_report_detected() {
        let records = [verified_dev("I.1.1", &[])];
        let codes: Vec<&str> = verify_task(&records, "I.1.1", false, true)
            .iter()
            .map(|p| p.code)
            .collect();
        assert!(codes.contains(&"missing-report"));
    }

    #[test]
    fn missing_ci_win_evidence_detected() {
        let mut r = verified_dev("II.7.2", &[]);
        r.required_environments = vec![VerificationEnvironment::CiWin];
        r.evidence = vec![]; // CI-WIN 증거 없음
        let codes: Vec<&str> = verify_task(&[r], "II.7.2", true, true)
            .iter()
            .map(|p| p.code)
            .collect();
        assert!(codes.contains(&"missing-environment-evidence"));
    }

    #[test]
    fn premature_workspace_requirement_detected() {
        let mut r = verified_dev("II.7.2", &[]);
        r.required_environments = vec![VerificationEnvironment::CiWin];
        r.evidence = vec![VerificationEvidence::new(
            VerificationEnvironment::CiWin,
            "https://ci/win".to_string(),
        )];
        let codes: Vec<&str> = verify_task(&[r], "II.7.2", true, false)
            .iter()
            .map(|p| p.code)
            .collect();
        assert!(codes.contains(&"premature-workspace-requirement"));
    }
}
