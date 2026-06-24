//! PR 본문 validator (I.2.16).
//!
//! PR 본문은 WBS ID, 환경 증거, 보고서 링크를 포함해야 합니다.

use crate::task_status::model::VerificationEnvironment;
use crate::task_status::wbs_id::WbsId;

/// PR 본문 검증 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrBodyError {
    pub code: &'static str,
    pub message: String,
}

impl PrBodyError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        PrBodyError {
            code,
            message: message.into(),
        }
    }
}

/// PR 본문을 검증합니다(빈 Vec = 통과).
pub fn validate_pr_body(body: &str) -> Vec<PrBodyError> {
    let mut errors = Vec::new();

    if !contains_wbs_id(body) {
        errors.push(PrBodyError::new("missing-wbs", "본문에 WBS ID가 없습니다"));
    }
    if !has_environment_evidence(body) {
        errors.push(PrBodyError::new(
            "missing-evidence",
            "본문에 검증 환경 코드가 없습니다",
        ));
    }
    if !body.contains("docs/task-reports/") {
        errors.push(PrBodyError::new(
            "missing-report",
            "본문에 보고서 링크가 없습니다",
        ));
    }

    errors
}

/// 본문에 valid WBS ID 토큰이 하나라도 있는지 확인합니다.
fn contains_wbs_id(body: &str) -> bool {
    body.split(|c: char| !(c.is_ascii_alphanumeric() || c == '.'))
        .any(|token| WbsId::parse(token).is_some())
}

fn has_environment_evidence(body: &str) -> bool {
    VerificationEnvironment::ALL
        .iter()
        .any(|env| body.contains(env.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_a() -> &'static str {
        "## WBS\n- WBS: I.1.1\n## 증거\n- 검증 환경: DEV-MAC\n## 보고서\n- docs/task-reports/I.1.1.md\n"
    }

    fn valid_b() -> &'static str {
        "WBS I.2.16 변경. 환경 CI-MAC, CI-WIN 통과. 보고서 docs/task-reports/I.2.16.md 참조.\n"
    }

    #[test]
    fn valid_bodies_pass() {
        assert!(validate_pr_body(valid_a()).is_empty());
        assert!(validate_pr_body(valid_b()).is_empty());
    }

    #[test]
    fn missing_wbs_fails() {
        let body = "환경 DEV-MAC, 보고서 docs/task-reports/x.md";
        let codes: Vec<&str> = validate_pr_body(body).iter().map(|e| e.code).collect();
        assert!(codes.contains(&"missing-wbs"));
    }

    #[test]
    fn missing_evidence_fails() {
        let body = "WBS: I.1.1, 보고서 docs/task-reports/I.1.1.md";
        let codes: Vec<&str> = validate_pr_body(body).iter().map(|e| e.code).collect();
        assert!(codes.contains(&"missing-evidence"));
    }

    #[test]
    fn missing_report_fails() {
        let body = "WBS: I.1.1, 환경 DEV-MAC";
        let codes: Vec<&str> = validate_pr_body(body).iter().map(|e| e.code).collect();
        assert!(codes.contains(&"missing-report"));
    }
}
