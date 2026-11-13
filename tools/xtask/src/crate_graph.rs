//! crate 의존 방향 정책 (II.2.8).
//!
//! 제품 crate 간 허용된 직접 의존 edge를 정의하고, 실제 edge가 정책을 벗어나면(역방향·금지)
//! 보고합니다. `docs/architecture/crate-dependency-graph.md`와 동일한 방향을 인코딩합니다.

use std::collections::HashMap;

/// 의존 그래프 위반.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphViolation {
    pub code: &'static str,
    pub message: String,
}

impl GraphViolation {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        GraphViolation {
            code,
            message: message.into(),
        }
    }
}

/// 각 제품 crate가 직접 의존할 수 있는 crate 집합.
pub fn allowed_edges() -> HashMap<&'static str, Vec<&'static str>> {
    let core = "stellar-core-types";
    let entries: &[(&str, &[&str])] = &[
        (core, &[]),
        ("stellar-document", &[core]),
        ("stellar-commands", &["stellar-document", core]),
        ("stellar-history", &["stellar-commands", core]),
        ("stellar-expressions", &[core]),
        ("stellar-bindings", &[core]),
        ("stellar-layout", &[core]),
        ("stellar-text", &[core]),
        ("stellar-scene", &[core]),
        ("stellar-renderer-core", &["stellar-scene"]),
        ("stellar-renderer-metal", &["stellar-renderer-core"]),
        ("stellar-renderer-d3d12", &["stellar-renderer-core"]),
        ("stellar-storage", &[core]),
        ("stellar-assets", &["stellar-storage", core]),
        ("stellar-dependencies", &[core]),
        (
            "stellar-project",
            &[
                core,
                "stellar-document",
                "stellar-storage",
                "stellar-assets",
                "stellar-dependencies",
                "stellar-git-porcelain",
            ],
        ),
        ("stellar-semantic-diff", &["stellar-document", core]),
        ("stellar-semantic-merge", &["stellar-semantic-diff"]),
        ("stellar-git-porcelain", &[core]),
        ("stellar-target-profile", &[core]),
        ("stellar-accessibility", &[core]),
        ("stellar-diagnostics", &[core]),
        ("stellar-ffi", &[core]),
        // headless 도구(XXIV.1.1, `tools/stellar-cli`): core crate만 사용합니다. MCP·agent·FFI·
        // GUI(renderer) crate에 의존하지 않습니다.
        (
            "stellar-cli",
            &[
                core,
                "stellar-project",
                "stellar-document",
                "stellar-storage",
                "stellar-diagnostics",
            ],
        ),
        // MCP stdio 브리지(XIX.1.1, `tools/stellar-mcp`): FFI·GUI crate에 의존하지 않습니다.
        // headless 도구(stellar-cli)는 이 브리지에 의존할 수 없습니다(위 항목에 없음).
        ("stellar-mcp", &[core, "stellar-diagnostics"]),
    ];
    entries.iter().map(|(k, v)| (*k, v.to_vec())).collect()
}

/// 실제 edge((crate, dep))가 정책을 따르는지 검사합니다(빈 Vec = 통과).
pub fn check_crate_graph(edges: &[(String, String)]) -> Vec<GraphViolation> {
    let allowed = allowed_edges();
    let mut violations = Vec::new();
    for (from, to) in edges {
        match allowed.get(from.as_str()) {
            None => violations.push(GraphViolation::new(
                "unknown-crate",
                format!("정책에 없는 crate: {from}"),
            )),
            Some(deps) => {
                if !deps.contains(&to.as_str()) {
                    violations.push(GraphViolation::new(
                        "forbidden-edge",
                        format!("허용되지 않은 의존 edge: {from} -> {to}"),
                    ));
                }
            }
        }
    }
    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(a: &str, b: &str) -> (String, String) {
        (a.to_string(), b.to_string())
    }

    #[test]
    fn allowed_edges_pass() {
        let edges = [
            edge("stellar-history", "stellar-commands"),
            edge("stellar-renderer-metal", "stellar-renderer-core"),
            edge("stellar-project", "stellar-assets"),
            edge("stellar-git-porcelain", "stellar-core-types"),
        ];
        assert!(check_crate_graph(&edges).is_empty());
    }

    #[test]
    fn reverse_edge_fails() {
        // core-types -> document 는 역방향(금지).
        let edges = [edge("stellar-core-types", "stellar-document")];
        let codes: Vec<&str> = check_crate_graph(&edges).iter().map(|v| v.code).collect();
        assert!(codes.contains(&"forbidden-edge"));
    }

    /// `tools/<dir>/Cargo.toml`의 `[dependencies]`에서 내부(stellar-*) 의존 edge를 읽습니다.
    fn tool_edges(dir: &str) -> Vec<(String, String)> {
        let path = format!("{}/../{dir}/Cargo.toml", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path).unwrap();
        let name = text
            .lines()
            .find_map(|l| l.trim().strip_prefix("name = "))
            .map(|n| n.trim().trim_matches('"').to_string())
            .unwrap();
        let mut inside = false;
        let mut edges = Vec::new();
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                inside = trimmed == "[dependencies]";
                continue;
            }
            if let (true, Some((key, _))) = (inside, trimmed.split_once('=')) {
                let key = key.trim();
                if key.starts_with("stellar-") {
                    edges.push((name.clone(), key.to_string()));
                }
            }
        }
        edges
    }

    #[test]
    fn stellar_cli_uses_only_core_crates() {
        // XXIV.1.1: 실제 manifest의 내부 의존이 정책을 따릅니다.
        assert!(check_crate_graph(&tool_edges("stellar-cli")).is_empty());
        // L1 headless 도구는 MCP·FFI·GUI(renderer) crate에 의존할 수 없습니다.
        for forbidden in [
            "stellar-mcp",
            "stellar-ffi",
            "stellar-renderer-core",
            "stellar-renderer-metal",
        ] {
            let codes: Vec<&str> = check_crate_graph(&[edge("stellar-cli", forbidden)])
                .iter()
                .map(|v| v.code)
                .collect();
            assert_eq!(codes, vec!["forbidden-edge"], "{forbidden}");
        }
    }

    #[test]
    fn stellar_mcp_never_depends_on_ffi_or_gui() {
        // XIX.1.1: 실제 manifest의 내부 의존이 정책을 따릅니다.
        assert!(check_crate_graph(&tool_edges("stellar-mcp")).is_empty());
        for forbidden in [
            "stellar-ffi",
            "stellar-renderer-core",
            "stellar-renderer-metal",
        ] {
            let codes: Vec<&str> = check_crate_graph(&[edge("stellar-mcp", forbidden)])
                .iter()
                .map(|v| v.code)
                .collect();
            assert_eq!(codes, vec!["forbidden-edge"], "{forbidden}");
        }
    }

    #[test]
    fn cross_layer_forbidden_edge_fails() {
        // git-porcelain -> document 는 정책에 없음(금지).
        let edges = [edge("stellar-git-porcelain", "stellar-document")];
        let codes: Vec<&str> = check_crate_graph(&edges).iter().map(|v| v.code).collect();
        assert!(codes.contains(&"forbidden-edge"));
    }
}
