//! 작업 보고서 validator (I.2.15).
//!
//! 완료/차단 보고서의 template heading, WBS 일치, 검증 환경 증거를 검사합니다.

use crate::task_status::model::VerificationEnvironment;
use crate::task_status::wbs_id::WbsId;

/// 보고서 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportKind {
    Complete,
    Blocked,
}

/// 보고서 검증 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportError {
    pub code: &'static str,
    pub message: String,
}

impl ReportError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        ReportError {
            code,
            message: message.into(),
        }
    }
}

const COMPLETE_HEADINGS: [&str; 8] = [
    "## WBS",
    "## 변경 파일",
    "## 명령과 종료 코드",
    "## 환경과 증거",
    "## 테스트",
    "## 성능",
    "## 차이",
    "## 남은 위험",
];

const BLOCKED_HEADINGS: [&str; 6] = [
    "## WBS",
    "## 실패 선행조건",
    "## 재현 명령",
    "## 필요한 결정과 환경",
    "## 변경되지 않은 파일",
    "## 재개 조건",
];

/// 보고서 내용을 검증합니다(빈 Vec = 통과).
pub fn validate_report(content: &str, kind: ReportKind, expected_id: &str) -> Vec<ReportError> {
    let mut errors = Vec::new();

    let headings: &[&str] = match kind {
        ReportKind::Complete => &COMPLETE_HEADINGS,
        ReportKind::Blocked => &BLOCKED_HEADINGS,
    };
    for heading in headings {
        let count = content
            .lines()
            .filter(|line| line.trim_end() == *heading)
            .count();
        if count == 0 {
            errors.push(ReportError::new(
                "missing-heading",
                format!("필수 heading 누락: {heading}"),
            ));
        } else if count > 1 {
            errors.push(ReportError::new(
                "duplicate-heading",
                format!("heading 중복: {heading}"),
            ));
        }
    }

    if !wbs_matches(content, expected_id) {
        errors.push(ReportError::new(
            "missing-wbs",
            format!("WBS: {expected_id} 와 일치하는 항목이 없습니다"),
        ));
    }

    if kind == ReportKind::Complete && !has_environment_evidence(content) {
        errors.push(ReportError::new(
            "missing-environment",
            "검증 환경 코드(예: DEV-MAC)가 보고서에 없습니다".to_string(),
        ));
    }

    errors
}

/// `WBS: <id>` 형태의 줄에서 expected_id와 일치하는 항목이 있는지 확인합니다.
fn wbs_matches(content: &str, expected_id: &str) -> bool {
    for line in content.lines() {
        if let Some(rest) = line.split("WBS:").nth(1) {
            let token = rest.trim().trim_matches('`').split_whitespace().next();
            if let Some(token) = token {
                if WbsId::parse(token).is_some() && token == expected_id {
                    return true;
                }
            }
        }
    }
    false
}

/// 검증 환경 코드가 본문에 하나라도 있는지 확인합니다.
fn has_environment_evidence(content: &str) -> bool {
    VerificationEnvironment::ALL
        .iter()
        .any(|env| content.contains(env.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_report() -> String {
        "\
# 작업 보고서: I.1.1 제목

## WBS

- WBS: I.1.1

## 변경 파일

- a.rs

## 명령과 종료 코드

| cmd | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 증거: docs/task-reports/I.1.1.md

## 테스트

- ok

## 성능

- 해당 없음

## 차이

- 없음

## 남은 위험

- 없음
"
        .to_string()
    }

    #[test]
    fn valid_complete_report_passes() {
        assert!(validate_report(&complete_report(), ReportKind::Complete, "I.1.1").is_empty());
    }

    #[test]
    fn missing_heading_fails() {
        let content = complete_report().replace("## 테스트\n", "");
        let codes: Vec<&str> = validate_report(&content, ReportKind::Complete, "I.1.1")
            .iter()
            .map(|e| e.code)
            .collect();
        assert!(codes.contains(&"missing-heading"));
    }

    #[test]
    fn missing_wbs_fails() {
        let content = complete_report().replace("- WBS: I.1.1", "- WBS:");
        let codes: Vec<&str> = validate_report(&content, ReportKind::Complete, "I.1.1")
            .iter()
            .map(|e| e.code)
            .collect();
        assert!(codes.contains(&"missing-wbs"));
    }

    #[test]
    fn missing_environment_evidence_fails() {
        let content = complete_report().replace("DEV-MAC", "로컬");
        let codes: Vec<&str> = validate_report(&content, ReportKind::Complete, "I.1.1")
            .iter()
            .map(|e| e.code)
            .collect();
        assert!(codes.contains(&"missing-environment"));
    }

    #[test]
    fn valid_blocked_report_passes() {
        let content = "\
# 차단 보고서: I.2.4 제목

## WBS

- WBS: I.2.4

## 실패 선행조건

- x

## 재현 명령

- y

## 필요한 결정과 환경

- CI-WIN

## 변경되지 않은 파일

- z

## 재개 조건

- w
";
        assert!(validate_report(content, ReportKind::Blocked, "I.2.4").is_empty());
    }
}
