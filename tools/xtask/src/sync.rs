//! sync-plan 비교 로직 (I.4.3).
//!
//! 계획서 WBS와 ledger를 비교해 누락·추가·중복·title·depends_on 차이를 보고합니다.
//! 파일을 수정하지 않습니다. 상태(status)는 작업 진행에 따라 자연히 달라지고
//! ledger는 주기적으로 재생성되므로 비교 대상이 아닙니다.

use std::collections::HashMap;

use crate::plan::markdown_wbs::extract_wbs;
use crate::task_status::model::TaskStatusRecord;

/// sync 차이.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncProblem {
    pub code: &'static str,
    pub message: String,
}

impl SyncProblem {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        SyncProblem {
            code,
            message: message.into(),
        }
    }
}

/// 계획서와 ledger의 구조적 차이를 보고합니다(빈 Vec = 일치).
pub fn sync_plan(plan_markdown: &str, ledger: &[TaskStatusRecord]) -> Vec<SyncProblem> {
    let plan = extract_wbs(plan_markdown);
    let mut problems = Vec::new();

    let mut plan_count: HashMap<&str, usize> = HashMap::new();
    for entry in &plan {
        *plan_count.entry(entry.id.as_str()).or_default() += 1;
    }
    let mut ledger_count: HashMap<&str, usize> = HashMap::new();
    for record in ledger {
        *ledger_count.entry(record.id.as_str()).or_default() += 1;
    }

    let mut dup: Vec<&str> = plan_count
        .iter()
        .chain(ledger_count.iter())
        .filter(|(_, c)| **c > 1)
        .map(|(id, _)| *id)
        .collect();
    dup.sort_unstable();
    dup.dedup();
    for id in dup {
        problems.push(SyncProblem::new("duplicate-id", format!("중복 ID: {id}")));
    }

    // 누락: 계획에 있으나 ledger에 없음
    let mut missing: Vec<&str> = plan_count
        .keys()
        .copied()
        .filter(|id| !ledger_count.contains_key(id))
        .collect();
    missing.sort_unstable();
    for id in missing {
        problems.push(SyncProblem::new(
            "missing-in-ledger",
            format!("계획에 있으나 ledger에 없음: {id}"),
        ));
    }

    // 추가: ledger에 있으나 계획에 없음
    let mut extra: Vec<&str> = ledger_count
        .keys()
        .copied()
        .filter(|id| !plan_count.contains_key(id))
        .collect();
    extra.sort_unstable();
    for id in extra {
        problems.push(SyncProblem::new(
            "extra-in-ledger",
            format!("ledger에 있으나 계획에 없음: {id}"),
        ));
    }

    // 공통 ID: title·depends_on 비교
    let ledger_map: HashMap<&str, &TaskStatusRecord> =
        ledger.iter().map(|r| (r.id.as_str(), r)).collect();
    for entry in &plan {
        if let Some(record) = ledger_map.get(entry.id.as_str()) {
            if entry.title != record.title {
                problems.push(SyncProblem::new(
                    "title-mismatch",
                    format!("{}: title 불일치", entry.id),
                ));
            }
            if entry.depends_on != record.depends_on {
                problems.push(SyncProblem::new(
                    "depends-on-mismatch",
                    format!("{}: depends_on 불일치", entry.id),
                ));
            }
        }
    }

    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_status::decode::decode_collection;
    use crate::task_status::model::TaskState;
    use crate::yaml::restricted_loader::load_restricted;

    fn record(id: &str, title: &str, deps: &[&str]) -> TaskStatusRecord {
        let mut r = TaskStatusRecord::new(id.to_string(), title.to_string(), TaskState::NotStarted);
        r.depends_on = deps.iter().map(|d| d.to_string()).collect();
        r
    }

    const PLAN: &str = "\
| ID | 상태 | 지시 | 검증 | 선행 |
|---|---|---|---|---|
| I.1.1 | `[x]` | 기준선 | ok | 없음 |
| I.1.2 | `[ ]` | 브랜치 | ok | I.1.1 |
";

    #[test]
    fn matching_plan_and_ledger_has_no_drift() {
        let ledger = [
            record("I.1.1", "기준선", &[]),
            record("I.1.2", "브랜치", &["I.1.1"]),
        ];
        assert!(sync_plan(PLAN, &ledger).is_empty());
    }

    #[test]
    fn detects_missing_extra_title_depends() {
        let ledger = [
            record("I.1.1", "다른제목", &[]),      // title mismatch
            record("I.1.2", "브랜치", &["I.9.9"]), // depends_on mismatch
            record("I.3.3", "추가", &[]),          // extra
        ];
        let codes: Vec<&str> = sync_plan(PLAN, &ledger).iter().map(|p| p.code).collect();
        assert!(codes.contains(&"title-mismatch"));
        assert!(codes.contains(&"depends-on-mismatch"));
        assert!(codes.contains(&"extra-in-ledger"));
    }

    #[test]
    fn detects_missing_in_ledger() {
        let ledger = [record("I.1.1", "기준선", &[])]; // I.1.2 누락
        let codes: Vec<&str> = sync_plan(PLAN, &ledger).iter().map(|p| p.code).collect();
        assert!(codes.contains(&"missing-in-ledger"));
    }

    #[test]
    fn real_plan_matches_real_ledger() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
        let plan = std::fs::read_to_string(format!(
            "{root}/docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md"
        ))
        .unwrap();
        let ledger_yaml =
            std::fs::read_to_string(format!("{root}/planning/task-status.yaml")).unwrap();
        let ledger = decode_collection(&load_restricted(&ledger_yaml).unwrap()).unwrap();
        let problems = sync_plan(&plan, &ledger);
        assert!(problems.is_empty(), "{problems:?}");
    }
}
