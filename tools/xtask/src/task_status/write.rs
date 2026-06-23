//! Deterministic ledger writer (I.2.11).
//!
//! `TaskStatusRecord` 목록을 고정 key 순서, 2-space indent, LF, 끝 newline,
//! task ID 정렬로 직렬화합니다. 주석은 생성하지 않습니다.

use std::cmp::Ordering;

use crate::task_status::model::TaskStatusRecord;
use crate::task_status::wbs_id::WbsId;

/// 레코드 목록을 결정론적 YAML ledger 문자열로 직렬화합니다.
pub fn write_ledger(records: &[TaskStatusRecord]) -> String {
    let mut sorted: Vec<&TaskStatusRecord> = records.iter().collect();
    sorted.sort_by(|a, b| compare_id(&a.id, &b.id));

    let mut out = String::from("tasks:\n");
    for record in sorted {
        out.push_str(&format!("  - id: {}\n", quote(&record.id)));
        out.push_str(&format!("    title: {}\n", quote(&record.title)));
        out.push_str(&format!("    status: {}\n", record.status.as_str()));
        if let Some(owner) = &record.owner {
            out.push_str(&format!("    owner: {}\n", quote(owner)));
        }
        if !record.depends_on.is_empty() {
            out.push_str("    depends_on:\n");
            for dep in &record.depends_on {
                out.push_str(&format!("      - {}\n", quote(dep)));
            }
        }
        if !record.required_environments.is_empty() {
            out.push_str("    required_environments:\n");
            for env in &record.required_environments {
                out.push_str(&format!("      - {}\n", env.as_str()));
            }
        }
        if !record.evidence.is_empty() {
            out.push_str("    evidence:\n");
            for ev in &record.evidence {
                out.push_str(&format!(
                    "      - environment: {}\n",
                    ev.environment.as_str()
                ));
                out.push_str(&format!("        locator: {}\n", quote(&ev.locator)));
                if let Some(sha) = &ev.artifact_sha256 {
                    out.push_str(&format!("        artifact_sha256: {}\n", quote(sha)));
                }
            }
        }
        if let Some(commit) = &record.commit {
            out.push_str(&format!("    commit: {}\n", quote(commit)));
        }
        if let Some(report) = &record.report {
            out.push_str(&format!("    report: {}\n", quote(report)));
        }
        if let Some(verified_at) = &record.verified_at {
            out.push_str(&format!("    verified_at: {}\n", quote(verified_at)));
        }
    }
    out
}

fn compare_id(a: &str, b: &str) -> Ordering {
    match (WbsId::parse(a), WbsId::parse(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a.cmp(b),
    }
}

/// 모든 scalar 문자열을 double-quote로 감싸고 `\`, `"`를 escape 합니다.
fn quote(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_status::decode::decode_collection;
    use crate::task_status::model::{TaskState, VerificationEnvironment, VerificationEvidence};
    use crate::yaml::restricted_loader::load_restricted;

    fn sample() -> Vec<TaskStatusRecord> {
        let mut a = TaskStatusRecord::new(
            "I.1.2".to_string(),
            "브랜치 기록".to_string(),
            TaskState::Verified,
        );
        a.required_environments = vec![VerificationEnvironment::DevMac];
        a.evidence = vec![VerificationEvidence::new(
            VerificationEnvironment::DevMac,
            "local".to_string(),
        )];
        a.commit = Some("abc1234".to_string());
        a.report = Some("docs/task-reports/I.1.2.md".to_string());
        a.verified_at = Some("2026-06-24".to_string());

        let mut b = TaskStatusRecord::new(
            "I.1.1".to_string(),
            "기준선".to_string(),
            TaskState::Verified,
        );
        b.required_environments = vec![VerificationEnvironment::DevMac];
        b.depends_on = vec![];
        b.commit = Some("def5678".to_string());
        b.report = Some("docs/task-reports/I.1.1.md".to_string());
        b.verified_at = Some("2026-06-24".to_string());

        let c = TaskStatusRecord::new(
            "I.2.1".to_string(),
            "모델".to_string(),
            TaskState::NotStarted,
        );
        // c는 not_started: required_environments 비움, 그 외 없음
        vec![a, b, c]
    }

    #[test]
    fn output_is_sorted_and_lf_terminated() {
        let out = write_ledger(&sample());
        assert!(out.starts_with("tasks:\n"));
        assert!(out.ends_with('\n'));
        assert!(!out.contains('\r'));
        // 정렬: I.1.1이 I.1.2보다 먼저
        let i1 = out.find("\"I.1.1\"").unwrap();
        let i2 = out.find("\"I.1.2\"").unwrap();
        let i3 = out.find("\"I.2.1\"").unwrap();
        assert!(i1 < i2 && i2 < i3);
    }

    #[test]
    fn deterministic_across_100_runs() {
        let records = sample();
        let first = write_ledger(&records);
        for _ in 0..100 {
            assert_eq!(write_ledger(&records), first);
        }
    }

    #[test]
    fn parse_write_parse_is_stable() {
        let records = sample();
        let written = write_ledger(&records);
        let decoded = decode_collection(&load_restricted(&written).unwrap()).unwrap();
        let rewritten = write_ledger(&decoded);
        assert_eq!(written, rewritten);
        // 의미 동일성: 정렬 후 비교
        let mut a = records.clone();
        let mut b = decoded.clone();
        a.sort_by(|x, y| compare_id(&x.id, &y.id));
        b.sort_by(|x, y| compare_id(&x.id, &y.id));
        assert_eq!(a, b);
    }
}
