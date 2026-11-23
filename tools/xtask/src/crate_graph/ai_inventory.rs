//! `stellar` 명령의 AI 무관성 inventory (XXIV.4.7, ADR-0005 §1·§8, ADR-0085 결정 10).
//!
//! L1 headless 도구 `stellar`(package `stellar-cli`)는 AI·MCP·모델·에이전트와 무관해야 합니다.
//!
//! - **명령 inventory**: `stellar --help` 출력 golden(`tools/stellar-cli/tests/fixtures/help.txt`, CLI test가 실제
//!   도움말과 byte 비교)의 `명령:` 절 명령 이름에 AI 관련 낱말이 없습니다.
//! - **전이 도달 금지**: `stellar-cli`가 직접·전이로 도달하는 workspace crate 이름에 AI 관련 낱말이 없습니다.
//!   이름에 그 낱말이 들어간 crate가 새로 생겨도 이 검사가 잡습니다(ADR-0005 §8 금지 목록은 이미 있는 crate만 셈).
//!
//! 낱말 판정은 이름을 `-`·`_`로 나눈 조각 단위입니다(`stellar-mcp-core`의 `mcp`). `email`·`maintain`처럼
//! 조각 안에 글자가 들어 있는 경우는 걸리지 않습니다.

use super::manifest::WorkspaceGraph;
use super::reach::reach_paths;
use super::GraphViolation;

/// AI·MCP·모델·에이전트 관련 이름 조각.
pub const AI_TERMS: [&str; 7] = ["ai", "mcp", "agent", "agents", "model", "models", "llm"];

/// 검사 대상 L1 package.
pub const CLI_PACKAGE: &str = "stellar-cli";

/// 도움말 golden 경로(저장소 기준).
pub const HELP_FIXTURE: &str = "tools/stellar-cli/tests/fixtures/help.txt";

/// 이름 조각 중 AI 관련 낱말.
pub fn ai_term(name: &str) -> Option<&'static str> {
    name.split(['-', '_']).find_map(|part| {
        AI_TERMS
            .iter()
            .copied()
            .find(|t| part.eq_ignore_ascii_case(t))
    })
}

/// 도움말 `명령:` 절의 명령 이름(들여쓰기 4칸 뒤 첫 낱말).
pub fn help_commands(help: &str) -> Vec<&str> {
    let mut commands = Vec::new();
    let mut inside = false;
    for line in help.lines() {
        if line == "명령:" {
            inside = true;
            continue;
        }
        if inside {
            if line.trim().is_empty() {
                break;
            }
            if let Some(name) = line
                .strip_prefix("    ")
                .and_then(|l| l.split_whitespace().next())
            {
                commands.push(name);
            }
        }
    }
    commands
}

/// 도움말 명령과 `stellar-cli` 도달 crate를 검사합니다(빈 Vec = 통과).
pub fn check_cli_ai_inventory(graph: &WorkspaceGraph, help: &str) -> Vec<GraphViolation> {
    let mut violations = Vec::new();
    let commands = help_commands(help);
    if commands.is_empty() {
        violations.push(GraphViolation::new(
            "cli-help-inventory",
            format!("{HELP_FIXTURE}에서 명령 목록을 읽지 못했습니다"),
        ));
    }
    for command in commands {
        if let Some(term) = ai_term(command) {
            violations.push(GraphViolation::new(
                "cli-ai-command",
                format!(
                    "`stellar {command}` 명령 이름에 AI 관련 낱말 `{term}`이 있습니다(XXIV.4.7)"
                ),
            ));
        }
    }
    for (target, path) in reach_paths(CLI_PACKAGE, &graph.edges) {
        if let Some(term) = ai_term(&target) {
            violations.push(GraphViolation::new(
                "cli-ai-reach",
                format!(
                    "`stellar`가 AI 관련 crate에 도달합니다: {} (낱말 `{term}`, XXIV.4.7)",
                    path.join(" -> ")
                ),
            ));
        }
    }
    violations
}

#[cfg(test)]
mod tests {
    use super::super::manifest::{scan_workspace, DepKind, DepTable, ManifestEdge};
    use super::*;

    fn repo_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn edge(from: &str, to: &str, kind: DepKind) -> ManifestEdge {
        ManifestEdge {
            from: from.into(),
            to: to.into(),
            table: DepTable { kind, target: None },
        }
    }

    #[test]
    fn real_cli_has_no_ai_command_or_reach() {
        // 검증 기준(XXIV.4.7): help 명령 inventory와 전이 도달 금지 검사 통과.
        let root = repo_root();
        let graph = scan_workspace(&root).unwrap();
        let help = std::fs::read_to_string(root.join(HELP_FIXTURE)).unwrap();
        let commands = help_commands(&help);
        assert!(commands.contains(&"apply") && commands.contains(&"git"));
        assert!(commands.len() >= 15, "{commands:?}");
        assert_eq!(check_cli_ai_inventory(&graph, &help), Vec::new());
        // 실제 workspace에는 AI 관련 crate가 있고(브리지), stellar는 그중 어디에도 도달하지 않습니다.
        let ai_crates: Vec<&String> = graph
            .packages
            .iter()
            .filter(|p| ai_term(p).is_some())
            .collect();
        assert!(ai_crates.iter().any(|p| p.as_str() == "stellar-mcp-core"));
        let reached = reach_paths(CLI_PACKAGE, &graph.edges);
        assert!(reached.contains_key("stellar-ops"));
        assert!(ai_crates.iter().all(|p| !reached.contains_key(p.as_str())));
    }

    #[test]
    fn ai_command_and_transitive_ai_crate_fail() {
        let help = "명령:\n    help    도움말\n    mcp-serve  금지\n    agent  금지\n\n";
        let graph = WorkspaceGraph {
            packages: vec![],
            edges: vec![
                edge("stellar-cli", "stellar-ops", DepKind::Normal),
                edge("stellar-ops", "stellar-model-runner", DepKind::Normal),
                // dev 의존 crate의 dev 의존은 빌드되지 않으므로 세지 않습니다.
                edge("stellar-ops", "stellar-llm-fixture", DepKind::Dev),
            ],
        };
        let codes: Vec<(&str, String)> = check_cli_ai_inventory(&graph, help)
            .into_iter()
            .map(|v| (v.code, v.message))
            .collect();
        assert_eq!(codes.len(), 3, "{codes:?}");
        assert_eq!(codes[0].0, "cli-ai-command");
        assert_eq!(codes[1].0, "cli-ai-command");
        assert_eq!(codes[2].0, "cli-ai-reach");
        assert!(codes[2]
            .1
            .contains("stellar-cli -> stellar-ops -> stellar-model-runner"));
    }

    #[test]
    fn term_matching_is_by_name_segment() {
        assert_eq!(ai_term("stellar-mcp-core"), Some("mcp"));
        assert_eq!(ai_term("stellar-agent-protocol"), Some("agent"));
        assert_eq!(ai_term("text_model"), Some("model"));
        for clean in [
            "stellar-document",
            "email",
            "maintain",
            "semantic-diff",
            "stellar-terminal",
        ] {
            assert_eq!(ai_term(clean), None, "{clean}");
        }
        assert!(help_commands("명령:\n\n").is_empty());
    }
}
