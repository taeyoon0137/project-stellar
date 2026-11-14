//! crate 의존 방향 정책 (II.2.8, XXIX.1.1).
//!
//! 제품 crate 간 허용된 직접 의존 edge를 정의하고, 실제 edge가 정책을 벗어나면(역방향·금지)
//! 보고합니다. `docs/architecture/crate-dependency-graph.md`와 동일한 방향을 인코딩합니다.
//!
//! XXIX.1.1 확장:
//! - [`manifest`]: `crates/*`와 `tools/stellar-*` manifest의 `[dependencies]`·`[build-dependencies]`·
//!   `[dev-dependencies]`·`[target.<spec>.*]` 표에서 내부 의존을 읽습니다.
//! - [`reach`]: 모든 표의 직접 edge를 이 정책으로 검사하고, ADR-0005 §8의 전이 도달 금지
//!   규칙([`reach::FORBIDDEN_REACH`])을 적용합니다.

pub mod manifest;
pub mod reach;

pub use reach::check_workspace_graph;

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
        ("stellar-document", &[core, "stellar-expressions"]),
        ("stellar-commands", &["stellar-document", core]),
        ("stellar-history", &["stellar-commands", core]),
        ("stellar-expressions", &[core]),
        ("stellar-bindings", &[core, "stellar-expressions"]),
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
                "stellar-commands",
                "stellar-history",
                "stellar-text",
            ],
        ),
        ("stellar-semantic-diff", &["stellar-document", core]),
        ("stellar-semantic-merge", &["stellar-semantic-diff"]),
        ("stellar-git-porcelain", &[core]),
        ("stellar-target-profile", &[core]),
        ("stellar-accessibility", &[core]),
        ("stellar-diagnostics", &[core]),
        // 로드맵 crate(ADR-0019): 내보내기(Phase XX)와 내장 터미널 코어(Phase XVIII).
        ("stellar-export", &[core, "stellar-scene"]),
        ("stellar-terminal", &[core]),
        // 앱 프로세스 runtime(ADR-0013): thread 구성과 document thread event loop.
        (
            "stellar-runtime",
            &[
                core,
                "stellar-document",
                "stellar-commands",
                "stellar-history",
                "stellar-project",
                "stellar-storage",
                "stellar-assets",
                "stellar-renderer-core",
            ],
        ),
        // 셸 경계는 runtime을 통해서만 문서·프로젝트 작업에 도달합니다(ADR-0013).
        ("stellar-ffi", &[core, "stellar-runtime"]),
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
        (
            "stellar-mcp",
            &[core, "stellar-diagnostics", "stellar-local-ipc"],
        ),
        // bridge↔app 로컬 IPC(ADR-0005 §8, `crates/local-ipc`): core-types만 허용합니다.
        // document·commands·ops에 도달하지 않습니다.
        ("stellar-local-ipc", &[core]),
        // L1 headless 조회·편집 공용 library(XXIV.4.2, ADR-0005 §8, `crates/ops`).
        // local-ipc·agent-protocol·mcp-core·agent-host·ffi는 직접 edge가 없고 전이 도달도
        // `reach::FORBIDDEN_REACH`가 막습니다.
        (
            "stellar-ops",
            &[
                "stellar-project",
                "stellar-document",
                "stellar-commands",
                "stellar-history",
                "stellar-storage",
                "stellar-semantic-diff",
                "stellar-semantic-merge",
                "stellar-diagnostics",
            ],
        ),
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

    /// `tools/<dir>/Cargo.toml`의 모든 의존 표에서 내부(stellar-*) 의존 edge를 읽습니다
    /// (XXIX.1.1 manifest 해석기 사용).
    fn tool_edges(dir: &str) -> Vec<(String, String)> {
        let path = format!("{}/../{dir}/Cargo.toml", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path).unwrap();
        let parsed = manifest::parse_manifest(&text).unwrap();
        let name = parsed.package.clone().unwrap();
        parsed
            .deps
            .iter()
            .map(|dep| dep.resolved_name(&std::collections::BTreeMap::new()))
            .filter(|dep| dep.starts_with(manifest::INTERNAL_PREFIX))
            .map(|dep| (name.clone(), dep))
            .collect()
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
    fn local_ipc_depends_only_on_core_types() {
        // ADR-0005 §8: local-ipc는 core-types만 허용하고 document·commands·ops에 도달하지 않습니다.
        let manifest = format!(
            "{}/../../crates/local-ipc/Cargo.toml",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(manifest).unwrap();
        assert!(text.contains("name = \"stellar-local-ipc\""));
        assert!(check_crate_graph(&[edge("stellar-local-ipc", "stellar-core-types")]).is_empty());
        for forbidden in [
            "stellar-document",
            "stellar-commands",
            "stellar-history",
            "stellar-storage",
            "stellar-diagnostics",
            "stellar-mcp",
        ] {
            let codes: Vec<&str> = check_crate_graph(&[edge("stellar-local-ipc", forbidden)])
                .iter()
                .map(|v| v.code)
                .collect();
            assert_eq!(codes, vec!["forbidden-edge"], "{forbidden}");
        }
        // 브리지는 local-ipc를 쓸 수 있고, headless 도구(L1)는 쓸 수 없습니다.
        assert!(check_crate_graph(&[edge("stellar-mcp", "stellar-local-ipc")]).is_empty());
        let codes: Vec<&str> = check_crate_graph(&[edge("stellar-cli", "stellar-local-ipc")])
            .iter()
            .map(|v| v.code)
            .collect();
        assert_eq!(codes, vec!["forbidden-edge"]);
    }

    #[test]
    fn ops_edges_follow_adr_0005() {
        // XXIV.4.2, ADR-0005 §8: ops의 실제 manifest 의존이 허용 목록 안에 있습니다.
        let path = format!("{}/../../crates/ops/Cargo.toml", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path).unwrap();
        let parsed = manifest::parse_manifest(&text).unwrap();
        assert_eq!(parsed.package.as_deref(), Some("stellar-ops"));
        let edges: Vec<(String, String)> = parsed
            .deps
            .iter()
            .map(|dep| dep.resolved_name(&std::collections::BTreeMap::new()))
            .filter(|dep| dep.starts_with(manifest::INTERNAL_PREFIX))
            .map(|dep| ("stellar-ops".to_string(), dep))
            .collect();
        assert!(!edges.is_empty());
        assert!(check_crate_graph(&edges).is_empty(), "{edges:?}");
        for allowed in [
            "stellar-project",
            "stellar-document",
            "stellar-commands",
            "stellar-history",
            "stellar-storage",
            "stellar-semantic-diff",
            "stellar-semantic-merge",
            "stellar-diagnostics",
        ] {
            assert!(
                check_crate_graph(&[edge("stellar-ops", allowed)]).is_empty(),
                "{allowed}"
            );
        }
        for forbidden in [
            "stellar-local-ipc",
            "stellar-agent-protocol",
            "stellar-mcp-core",
            "stellar-mcp",
            "stellar-agent-host",
            "stellar-ffi",
            "stellar-runtime",
            "stellar-cli",
        ] {
            let codes: Vec<&str> = check_crate_graph(&[edge("stellar-ops", forbidden)])
                .iter()
                .map(|v| v.code)
                .collect();
            assert_eq!(codes, vec!["forbidden-edge"], "{forbidden}");
        }
    }

    #[test]
    fn runtime_edges_pass_and_ffi_reaches_runtime() {
        // ADR-0013: ffi -> runtime -> (document, commands, history, project, storage, assets,
        // renderer-core, core-types).
        let edges = [
            edge("stellar-ffi", "stellar-runtime"),
            edge("stellar-ffi", "stellar-core-types"),
            edge("stellar-runtime", "stellar-core-types"),
            edge("stellar-runtime", "stellar-document"),
            edge("stellar-runtime", "stellar-commands"),
            edge("stellar-runtime", "stellar-history"),
            edge("stellar-runtime", "stellar-project"),
            edge("stellar-runtime", "stellar-storage"),
            edge("stellar-runtime", "stellar-assets"),
            edge("stellar-runtime", "stellar-renderer-core"),
        ];
        assert!(check_crate_graph(&edges).is_empty());
    }

    #[test]
    fn runtime_reverse_and_bypass_edges_fail() {
        // runtime -> ffi 는 역방향, 하위 crate -> runtime 은 역참조, ffi -> document 는 runtime 우회.
        for (from, to) in [
            ("stellar-runtime", "stellar-ffi"),
            ("stellar-document", "stellar-runtime"),
            ("stellar-project", "stellar-runtime"),
            ("stellar-ffi", "stellar-document"),
            ("stellar-ffi", "stellar-project"),
        ] {
            let codes: Vec<&str> = check_crate_graph(&[edge(from, to)])
                .iter()
                .map(|v| v.code)
                .collect();
            assert_eq!(codes, vec!["forbidden-edge"], "{from} -> {to}");
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
