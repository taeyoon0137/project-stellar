//! task-status ledger 생성 (I.2.12).
//!
//! 계획서 WBS 추출(I.2.10)과 writer(I.2.11)를 호출해 `planning/task-status.yaml`을 만듭니다.
//! - 상태는 계획서 marker를 그대로 반영합니다.
//! - required_environments는 작업 행 검증 열의 `[ENV]` 태그(없으면 `DEV-MAC`)에서 가져옵니다.
//! - verified/reported_done 작업은 보고서·커밋·증거를 채워 validator를 통과시킵니다.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use crate::plan::markdown_wbs::extract_wbs;
use crate::task_status::decode::decode_collection;
use crate::task_status::model::{
    TaskState, TaskStatusRecord, VerificationEnvironment, VerificationEvidence,
};
use crate::task_status::validate_collection::validate_collection;
use crate::task_status::validate_evidence::validate_evidence;
use crate::task_status::validate_graph::validate_graph;
use crate::task_status::validate_record::validate_record;
use crate::task_status::wbs_id::WbsId;
use crate::task_status::write::write_ledger;
use crate::yaml::restricted_loader::load_restricted;

/// 한 작업의 git 커밋 정보.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    pub sha: String,
    pub committed_at: String,
}

/// WBS ID → 커밋 정보.
pub type CommitIndex = HashMap<String, CommitInfo>;

/// 작업 행 검증 열에서 `[ENV]` 태그를 추출합니다(없으면 `DEV-MAC`).
pub fn parse_required_environments(
    markdown: &str,
) -> HashMap<String, Vec<VerificationEnvironment>> {
    let mut map = HashMap::new();
    let mut in_fence = false;
    for raw in markdown.lines() {
        if raw.trim_start().starts_with("```") {
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
        if !is_status_marker(cells[1]) {
            continue;
        }
        let verify = cells[3];
        let mut envs: Vec<VerificationEnvironment> = VerificationEnvironment::ALL
            .into_iter()
            .filter(|env| verify.contains(&format!("[{}]", env.as_str())))
            .collect();
        if envs.is_empty() {
            envs.push(VerificationEnvironment::DevMac);
        }
        map.insert(id.to_string(), envs);
    }
    map
}

fn is_status_marker(cell: &str) -> bool {
    matches!(
        cell.trim().trim_matches('`').trim(),
        "[x]" | "[~]" | "[ ]" | "[!]"
    )
}

/// 계획서와 커밋 인덱스로 레코드 목록을 구성합니다(순수 함수, IO 없음).
pub fn build_ledger(markdown: &str, commits: &CommitIndex) -> Vec<TaskStatusRecord> {
    let entries = extract_wbs(markdown);
    let env_map = parse_required_environments(markdown);

    let mut records = Vec::with_capacity(entries.len());
    for entry in entries {
        let envs = env_map
            .get(&entry.id)
            .cloned()
            .unwrap_or_else(|| vec![VerificationEnvironment::DevMac]);
        let report = format!("docs/task-reports/{}.md", entry.id);

        let mut record = TaskStatusRecord::new(entry.id.clone(), entry.title.clone(), entry.status);
        record.depends_on = entry.depends_on.clone();
        record.required_environments = envs.clone();

        match entry.status {
            TaskState::Verified => {
                record.report = Some(report.clone());
                if let Some(info) = commits.get(&entry.id) {
                    record.commit = Some(info.sha.clone());
                    record.verified_at = Some(info.committed_at.clone());
                }
                record.evidence = envs
                    .iter()
                    .map(|env| VerificationEvidence::new(*env, report.clone()))
                    .collect();
            }
            TaskState::ReportedDone => {
                record.report = Some(report.clone());
            }
            _ => {}
        }
        records.push(record);
    }
    records
}

/// 빌드한 레코드에 모든 validator를 적용하고 오류 코드 목록을 반환합니다(빈 = 통과).
pub fn validate_all(records: &[TaskStatusRecord]) -> Vec<String> {
    let mut problems = Vec::new();
    for record in records {
        for e in validate_record(record) {
            problems.push(format!("{}: record/{}", record.id, e.code));
        }
        for e in validate_evidence(record) {
            problems.push(format!("{}: evidence/{}", record.id, e.code));
        }
    }
    for e in validate_collection(records) {
        problems.push(format!("collection/{}", e.code));
    }
    for e in validate_graph(records) {
        problems.push(format!("graph/{}", e.code));
    }
    problems
}

/// `git log`에서 `Task(<ID>): ...` 커밋의 SHA와 커밋 시각을 수집합니다.
pub fn collect_commits(repo_root: &Path) -> Result<CommitIndex, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["log", "--no-merges", "--format=%H%x09%cI%x09%s"])
        .output()
        .map_err(|e| format!("git log 실행 실패: {e}"))?;
    if !output.status.success() {
        return Err("git log 종료 코드 비0".to_string());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut index = CommitIndex::new();
    for line in text.lines() {
        let mut parts = line.splitn(3, '\t');
        let (Some(sha), Some(date), Some(subject)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if let Some(id) = parse_task_subject(subject) {
            // git log는 최신순. 같은 ID가 여러 번이면 첫(최신) 항목 유지.
            index.entry(id).or_insert(CommitInfo {
                sha: sha.to_string(),
                committed_at: date.to_string(),
            });
        }
    }
    Ok(index)
}

/// `Task(<ID>): ...` 제목에서 WBS ID를 추출합니다.
fn parse_task_subject(subject: &str) -> Option<String> {
    let rest = subject.strip_prefix("Task(")?;
    let close = rest.find(')')?;
    let id = &rest[..close];
    let after = &rest[close + 1..];
    if !after.starts_with(": ") {
        return None;
    }
    if WbsId::parse(id).is_some() {
        Some(id.to_string())
    } else {
        None
    }
}

/// 계획서를 읽어 ledger를 생성·검증하고 `planning/task-status.yaml`에 씁니다.
pub fn generate_ledger_file(repo_root: &Path) -> Result<usize, String> {
    let plan_path = repo_root.join("docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md");
    let markdown =
        std::fs::read_to_string(&plan_path).map_err(|e| format!("계획서 읽기 실패: {e}"))?;

    let commits = collect_commits(repo_root)?;
    let records = build_ledger(&markdown, &commits);

    let problems = validate_all(&records);
    if !problems.is_empty() {
        return Err(format!("ledger validator 실패: {}", problems.join(", ")));
    }

    let ledger = write_ledger(&records);

    // round-trip 확인: 생성물이 decoder를 통과해야 합니다.
    decode_collection(&load_restricted(&ledger).map_err(|e| format!("loader: {}", e.message))?)
        .map_err(|e| format!("decoder: {}", e.message))?;

    let planning_dir = repo_root.join("planning");
    std::fs::create_dir_all(&planning_dir)
        .map_err(|e| format!("planning 디렉터리 생성 실패: {e}"))?;
    std::fs::write(planning_dir.join("task-status.yaml"), &ledger)
        .map_err(|e| format!("ledger 쓰기 실패: {e}"))?;

    Ok(records.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_plan() -> &'static str {
        "\
| ID | 상태 | 지시 | 검증 | 선행 |
|---|---|---|---|---|
| I.1.1 | `[x]` | 기준선 | `[DEV-MAC]` ok | 없음 |
| I.1.2 | `[x]` | 브랜치 | `[DEV-MAC]` ok | I.1.1 |
| I.1.10 | `[~]` | CI | `[CI-MAC][CI-WIN]` ok | I.1.2 |
| I.2.1 | `[ ]` | 모델 | `[DEV-MAC]` ok | I.1.10 |
"
    }

    fn fixture_commits() -> CommitIndex {
        let mut index = CommitIndex::new();
        for id in ["I.1.1", "I.1.2"] {
            index.insert(
                id.to_string(),
                CommitInfo {
                    sha: "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0".to_string(),
                    committed_at: "2026-06-24T00:00:00+09:00".to_string(),
                },
            );
        }
        index
    }

    #[test]
    fn required_environments_parsed_from_verify_column() {
        let map = parse_required_environments(fixture_plan());
        assert_eq!(map["I.1.1"], vec![VerificationEnvironment::DevMac]);
        assert_eq!(
            map["I.1.10"],
            vec![
                VerificationEnvironment::CiMac,
                VerificationEnvironment::CiWin
            ]
        );
    }

    #[test]
    fn built_ledger_passes_all_validators() {
        let records = build_ledger(fixture_plan(), &fixture_commits());
        assert_eq!(records.len(), 4);
        assert!(
            validate_all(&records).is_empty(),
            "{:?}",
            validate_all(&records)
        );
    }

    #[test]
    fn parses_task_subject() {
        assert_eq!(
            parse_task_subject("Task(I.1.1): 제목").as_deref(),
            Some("I.1.1")
        );
        assert_eq!(parse_task_subject("Docs: x"), None);
        assert_eq!(parse_task_subject("Task(bad): x"), None);
    }
}
