//! 선행(depends_on) graph 검증 (I.2.8).
//!
//! depends_on 대상 존재, self dependency, 중복 edge, cycle을 검사합니다.
//! 범위 표기(`I.1~I.5`)와 Phase/Gate 문자열은 exact dependency가 아니므로 거부합니다.

use std::collections::{HashMap, HashSet};

use crate::task_status::model::TaskStatusRecord;
use crate::task_status::wbs_id::WbsId;

/// graph 검증 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphError {
    pub code: &'static str,
    pub message: String,
}

impl GraphError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        GraphError {
            code,
            message: message.into(),
        }
    }
}

/// depends_on graph를 검증하고 발견된 모든 오류를 반환합니다(빈 Vec = 통과).
pub fn validate_graph(records: &[TaskStatusRecord]) -> Vec<GraphError> {
    let mut errors = Vec::new();
    let ids: HashSet<&str> = records.iter().map(|r| r.id.as_str()).collect();

    // 인접 리스트(존재하는 노드 간 edge만, self-loop 제외)로 cycle 검사.
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();

    for record in records {
        let mut seen_edges: Vec<&str> = Vec::new();
        for dep in &record.depends_on {
            let dep = dep.as_str();
            if dep == record.id {
                errors.push(GraphError::new(
                    "self-dependency",
                    format!("{}는 자기 자신을 선행으로 둘 수 없습니다", record.id),
                ));
                continue;
            }
            if seen_edges.contains(&dep) {
                errors.push(GraphError::new(
                    "duplicate-edge",
                    format!("{}의 중복 선행 edge: {dep}", record.id),
                ));
                continue;
            }
            seen_edges.push(dep);

            if WbsId::parse(dep).is_none() {
                errors.push(GraphError::new(
                    "invalid-dependency",
                    format!("{}의 선행은 exact WBS ID여야 합니다: {dep}", record.id),
                ));
                continue;
            }
            if !ids.contains(dep) {
                errors.push(GraphError::new(
                    "missing-dependency",
                    format!("{}의 선행 대상이 없습니다: {dep}", record.id),
                ));
                continue;
            }
            adjacency.entry(record.id.as_str()).or_default().push(dep);
        }
    }

    if let Some(cycle) = find_cycle(
        &records.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        &adjacency,
    ) {
        errors.push(GraphError::new(
            "cycle",
            format!("선행 cycle 발견: {}", cycle.join(" -> ")),
        ));
    }

    errors
}

/// DFS 3-color로 cycle을 찾고, 발견 시 cycle 경로를 반환합니다.
fn find_cycle<'a>(
    nodes: &[&'a str],
    adjacency: &HashMap<&'a str, Vec<&'a str>>,
) -> Option<Vec<&'a str>> {
    let mut color: HashMap<&str, u8> = nodes.iter().map(|n| (*n, 0u8)).collect();
    let mut path: Vec<&str> = Vec::new();
    for &node in nodes {
        if color.get(node).copied() == Some(0) {
            if let Some(cycle) = visit(node, adjacency, &mut color, &mut path) {
                return Some(cycle);
            }
        }
    }
    None
}

fn visit<'a>(
    node: &'a str,
    adjacency: &HashMap<&'a str, Vec<&'a str>>,
    color: &mut HashMap<&'a str, u8>,
    path: &mut Vec<&'a str>,
) -> Option<Vec<&'a str>> {
    color.insert(node, 1);
    path.push(node);
    if let Some(neighbors) = adjacency.get(node) {
        for &next in neighbors {
            match color.get(next).copied() {
                Some(1) => {
                    // back edge: cycle
                    let start = path.iter().position(|n| *n == next).unwrap_or(0);
                    let mut cycle: Vec<&str> = path[start..].to_vec();
                    cycle.push(next);
                    return Some(cycle);
                }
                Some(0) | None => {
                    if let Some(cycle) = visit(next, adjacency, color, path) {
                        return Some(cycle);
                    }
                }
                _ => {}
            }
        }
    }
    path.pop();
    color.insert(node, 2);
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_status::model::TaskState;

    fn rec(id: &str, deps: &[&str]) -> TaskStatusRecord {
        let mut r = TaskStatusRecord::new(id.to_string(), id.to_string(), TaskState::NotStarted);
        r.depends_on = deps.iter().map(|d| d.to_string()).collect();
        r
    }

    fn codes(records: &[TaskStatusRecord]) -> Vec<&'static str> {
        validate_graph(records).iter().map(|e| e.code).collect()
    }

    #[test]
    fn dag_passes() {
        let records = [
            rec("I.1.1", &[]),
            rec("I.1.2", &["I.1.1"]),
            rec("I.1.3", &["I.1.2"]),
        ];
        assert!(validate_graph(&records).is_empty());
    }

    #[test]
    fn detects_missing_dependency() {
        let records = [rec("I.1.1", &["I.9.9"])];
        assert!(codes(&records).contains(&"missing-dependency"));
    }

    #[test]
    fn detects_self_dependency() {
        let records = [rec("I.1.1", &["I.1.1"])];
        assert!(codes(&records).contains(&"self-dependency"));
    }

    #[test]
    fn detects_duplicate_edge() {
        let records = [rec("I.1.2", &["I.1.1", "I.1.1"]), rec("I.1.1", &[])];
        assert!(codes(&records).contains(&"duplicate-edge"));
    }

    #[test]
    fn detects_invalid_dependency() {
        let range = [rec("I.1.1", &["I.1~I.5"])];
        assert!(codes(&range).contains(&"invalid-dependency"));
        let gate = [rec("I.1.1", &["G1"])];
        assert!(codes(&gate).contains(&"invalid-dependency"));
    }

    #[test]
    fn detects_two_node_cycle() {
        let records = [rec("I.1.1", &["I.1.2"]), rec("I.1.2", &["I.1.1"])];
        assert!(codes(&records).contains(&"cycle"));
    }

    #[test]
    fn detects_ten_node_cycle() {
        let mut records = Vec::new();
        for i in 1..=10 {
            let next = if i == 10 { 1 } else { i + 1 };
            let mut r =
                TaskStatusRecord::new(format!("I.1.{i}"), format!("t{i}"), TaskState::NotStarted);
            r.depends_on = vec![format!("I.1.{next}")];
            records.push(r);
        }
        assert!(codes(&records).contains(&"cycle"));
    }
}
