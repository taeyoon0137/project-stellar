//! sync-plan 비교 로직 (I.4.3, 본문 ID 참조 검사는 XXIX.1.3).
//!
//! 계획서 WBS와 ledger를 비교해 누락·추가·중복·title·depends_on 차이를 보고합니다.
//! 파일을 수정하지 않습니다. 상태(status)는 작업 진행에 따라 자연히 달라지고
//! ledger는 주기적으로 재생성되므로 비교 대상이 아닙니다.
//!
//! 또한 task 행의 구현 지시·검증 셀과 Phase 완료 조건에 나온 WBS ID가 계획서에 정의돼 있는지
//! 검사합니다(`unknown-id-reference`, 추출 규칙은 [`crate::plan::id_refs`]).

use std::collections::{HashMap, HashSet};

use crate::plan::id_refs::{extract_id_references, RefLocation};
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

    // 본문 ID 참조: 구현 지시·검증 셀과 Phase 완료 조건의 WBS ID가 계획서에 있어야 합니다.
    let defined: HashSet<&str> = plan.iter().map(|e| e.id.as_str()).collect();
    for reference in extract_id_references(plan_markdown) {
        if defined.contains(reference.id.as_str()) {
            continue;
        }
        let place = match &reference.location {
            RefLocation::TaskRow(row) => format!("{row} 구현 지시·검증"),
            RefLocation::PhaseCompletion => "Phase 완료 조건".to_string(),
        };
        problems.push(SyncProblem::new(
            "unknown-id-reference",
            format!(
                "{}행 {place}: 계획서에 없는 WBS ID 참조: {}",
                reference.line, reference.id
            ),
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
    fn reports_references_to_deleted_ids() {
        // I.1.3은 삭제된 행이라고 가정합니다: 지시·검증 셀, 범위 끝, Phase 완료 조건의 참조를 보고합니다.
        let plan = "\
| ID | 상태 | 지시 | 검증 | 선행 |
|---|---|---|---|---|
| I.1.1 | `[x]` | 기준선, I.1.3 결과 사용 | ok | 없음 |
| I.1.2 | `[ ]` | I.1.1~I.1.3 정리 | I.1.3 fixture, 1.1.1·I.1.1.1 무시 | I.1.1 |

**Phase I 완료 조건:** I.1.2와 I.1.3 통과.
";
        let ledger = [
            record("I.1.1", "기준선, I.1.3 결과 사용", &[]),
            record("I.1.2", "I.1.1~I.1.3 정리", &["I.1.1"]),
        ];
        let problems = sync_plan(plan, &ledger);
        let codes: Vec<&str> = problems.iter().map(|p| p.code).collect();
        assert_eq!(codes, ["unknown-id-reference"; 4], "{problems:?}");
        assert!(problems[0].message.starts_with("3행 I.1.1 구현 지시·검증"));
        assert!(problems[3].message.starts_with("6행 Phase 완료 조건"));
        assert!(problems.iter().all(|p| p.message.ends_with("I.1.3")));
        // 참조가 모두 정의된 계획서는 보고하지 않습니다.
        assert!(sync_plan(
            PLAN,
            &[
                record("I.1.1", "기준선", &[]),
                record("I.1.2", "브랜치", &["I.1.1"])
            ]
        )
        .is_empty());
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
