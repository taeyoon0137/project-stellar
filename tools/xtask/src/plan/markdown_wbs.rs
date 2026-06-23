//! 계획서 Markdown에서 WBS 추출 (I.2.10).
//!
//! task 정의 표 행에서 WBS ID·title·status·선행 exact ID를 추출합니다.
//! fenced code block, prose 예시, range·Phase·Gate 표기는 WBS 정의로 세지 않습니다.

use crate::task_status::model::TaskState;
use crate::task_status::wbs_id::WbsId;

/// 계획서에서 추출한 WBS 항목.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanWbsEntry {
    pub id: String,
    pub title: String,
    pub status: TaskState,
    pub depends_on: Vec<String>,
}

/// Markdown 본문에서 WBS task 행을 추출합니다.
pub fn extract_wbs(markdown: &str) -> Vec<PlanWbsEntry> {
    let mut entries = Vec::new();
    let mut in_fence = false;

    for raw in markdown.lines() {
        let trimmed_start = raw.trim_start();
        if trimmed_start.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let line = raw.trim();
        if !line.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() < 5 {
            continue;
        }

        let id = cells[0].trim_matches('`').trim();
        if WbsId::parse(id).is_none() {
            continue;
        }
        let Some(status) = status_from_marker(cells[1]) else {
            continue;
        };
        let title = cells[2].to_string();
        let depends_on = parse_predecessors(cells[cells.len() - 1]);

        entries.push(PlanWbsEntry {
            id: id.to_string(),
            title,
            status,
            depends_on,
        });
    }

    entries
}

/// 상태 marker(`[x]`/`[~]`/`[ ]`/`[!]`)를 정확히 일치할 때만 변환합니다.
fn status_from_marker(cell: &str) -> Option<TaskState> {
    let marker = cell.trim().trim_matches('`').trim();
    match marker {
        "[x]" => Some(TaskState::Verified),
        "[~]" => Some(TaskState::ReportedDone),
        "[ ]" => Some(TaskState::NotStarted),
        "[!]" => Some(TaskState::Blocked),
        _ => None,
    }
}

/// 선행 cell에서 exact WBS ID만 추출합니다(없음·range·Gate·Phase 문자열 무시).
fn parse_predecessors(cell: &str) -> Vec<String> {
    cell.split(',')
        .map(|token| token.trim().trim_matches('`').trim())
        .filter(|token| WbsId::parse(token).is_some())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_task_rows_and_ignores_fence_and_prose() {
        let md = "\
### I.1 표

| ID | 상태 | 구현 지시 | 검증 | 선행 |
|---|---|---|---|---|
| I.1.1 | `[x]` | 기준선 검증 | ok | 없음 |
| I.1.2 | `[ ]` | 브랜치 기록 | ok | I.1.1 |

예: `Task(I.1.1)` 형태로 작성합니다.

```text
| I.9.9 | `[x]` | 가짜 행 | ok | 없음 |
```
";
        let entries = extract_wbs(md);
        let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["I.1.1", "I.1.2"]);
        assert_eq!(entries[0].status, TaskState::Verified);
        assert_eq!(entries[1].status, TaskState::NotStarted);
        assert_eq!(entries[1].depends_on, vec!["I.1.1".to_string()]);
        assert!(!ids.contains(&"I.9.9"));
    }

    #[test]
    fn ignores_summary_and_gate_rows() {
        let md = "\
| `I.1.1`~`I.1.5` | `[x]` 검증 완료 | 요약 |
| G0 저장소 기준선 | 조건 | x |
| DEV-MAC | macOS | x | y | z |
";
        assert!(extract_wbs(md).is_empty());
    }

    #[test]
    fn extracts_multiple_predecessors_and_skips_ranges() {
        let md = "\
| ID | 상태 | 지시 | 검증 | 선행 |
|---|---|---|---|---|
| I.4.1 | `[ ]` | 명령 | ok | I.2.12, I.3.3 |
| I.5.1 | `[ ]` | 범위 | ok | I.2.2~I.2.7 |
";
        let entries = extract_wbs(md);
        assert_eq!(
            entries[0].depends_on,
            vec!["I.2.12".to_string(), "I.3.3".to_string()]
        );
        assert!(entries[1].depends_on.is_empty());
    }

    #[test]
    fn extracts_real_plan_uniquely() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md"
        );
        let content = std::fs::read_to_string(path).expect("plan readable");
        let entries = extract_wbs(&content);

        let mut ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "WBS ID가 중복 없이 추출되어야 함");

        assert!(ids.contains(&"I.1.1"));
        assert!(ids.contains(&"I.2.10"));
        assert!(!ids.contains(&"I.9.9"));
        assert!(count > 30, "충분한 수의 WBS가 추출되어야 함: {count}");
    }
}
