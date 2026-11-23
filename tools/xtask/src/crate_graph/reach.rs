//! workspace manifest 전체 검사와 전이 도달 금지 규칙 (XXIX.1.1).
//!
//! - 직접 edge: `[dependencies]`뿐 아니라 build·dev·target 한정 표의 내부 의존도
//!   [`super::allowed_edges`] 정책을 따라야 합니다.
//! - 전이 도달: ADR-0005 §8 "전이적으로도 도달 금지" 열을 [`FORBIDDEN_REACH`]로 인코딩합니다.
//!   crate X의 도달 집합은 X 자신의 모든 표(dev 포함) 의존에서 시작해, 그 뒤로는 의존 crate를
//!   빌드할 때 함께 빌드되는 normal·build 의존(target 한정 포함)만 따라갑니다. 의존 crate의
//!   dev 의존은 X를 빌드할 때 빌드되지 않으므로 따라가지 않습니다.
//!
//! 탐색은 방문 집합을 쓰는 반복 BFS이며, 결과는 이름 순으로 정렬해 결정적입니다.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use super::manifest::{ManifestEdge, WorkspaceGraph};
use super::{allowed_edges, GraphViolation};

/// 한 crate가 전이적으로도 도달하면 안 되는 crate 목록.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReachBan {
    pub from: &'static str,
    pub never_reach: &'static [&'static str],
}

/// ADR-0005 §8 crate 배치 표의 "전이적으로도 도달 금지" 열.
///
/// 아직 workspace에 없는 crate의 규칙은 그 crate가 추가되는 순간부터 적용됩니다.
/// `stellar-ffi`는 이 열 대신 타입 변환 금지 inventory로 보완하므로 규칙이 없습니다.
pub const FORBIDDEN_REACH: &[ReachBan] = &[
    ReachBan {
        from: "stellar-mcp-core",
        never_reach: &["stellar-agent-host", "stellar-runtime", "stellar-ffi"],
    },
    ReachBan {
        from: "stellar-ops",
        never_reach: &[
            "stellar-local-ipc",
            "stellar-agent-protocol",
            "stellar-mcp-core",
            "stellar-mcp",
            "stellar-agent-host",
            "stellar-ffi",
        ],
    },
    ReachBan {
        from: "stellar-cli",
        never_reach: &[
            "stellar-local-ipc",
            "stellar-agent-protocol",
            "stellar-mcp-core",
            "stellar-mcp",
            "stellar-agent-host",
        ],
    },
    ReachBan {
        from: "stellar-local-ipc",
        never_reach: &["stellar-document", "stellar-commands", "stellar-ops"],
    },
    ReachBan {
        from: "stellar-agent-protocol",
        never_reach: &[
            "stellar-commands",
            "stellar-document",
            "stellar-ops",
            "stellar-agent-host",
        ],
    },
    ReachBan {
        from: "stellar-mcp",
        never_reach: &["stellar-agent-host", "stellar-ffi"],
    },
    ReachBan {
        from: "stellar-runtime",
        never_reach: &["stellar-mcp-core"],
    },
    ReachBan {
        from: "stellar-agent-host",
        never_reach: &[
            "stellar-document",
            "stellar-commands",
            "stellar-history",
            "stellar-storage",
            "stellar-ops",
        ],
    },
];

/// 저장소 정책(허용 edge + ADR-0005 §8 전이 금지)으로 workspace graph를 검사합니다.
pub fn check_workspace_graph(graph: &WorkspaceGraph) -> Vec<GraphViolation> {
    check_workspace_graph_with(&allowed_edges(), FORBIDDEN_REACH, graph)
}

/// 주어진 정책으로 workspace graph를 검사합니다(빈 Vec = 통과).
///
/// 보고 순서: 정책에 없는 package(`unknown-crate`), 허용되지 않은 직접 edge(`forbidden-edge`,
/// 선언된 표 표기 포함), 전이 도달 위반(`forbidden-reach`, 경로 포함).
pub fn check_workspace_graph_with(
    allowed: &HashMap<&'static str, Vec<&'static str>>,
    bans: &[ReachBan],
    graph: &WorkspaceGraph,
) -> Vec<GraphViolation> {
    let mut violations = Vec::new();
    for package in &graph.packages {
        if !allowed.contains_key(package.as_str()) {
            violations.push(GraphViolation::new(
                "unknown-crate",
                format!("정책에 없는 crate: {package}"),
            ));
        }
    }
    for edge in &graph.edges {
        let Some(deps) = allowed.get(edge.from.as_str()) else {
            if !graph.packages.contains(&edge.from) {
                violations.push(GraphViolation::new(
                    "unknown-crate",
                    format!("정책에 없는 crate: {}", edge.from),
                ));
            }
            continue;
        };
        if !deps.contains(&edge.to.as_str()) {
            violations.push(GraphViolation::new(
                "forbidden-edge",
                format!(
                    "허용되지 않은 의존 edge: {} -> {} ({})",
                    edge.from,
                    edge.to,
                    edge.table.label()
                ),
            ));
        }
    }
    violations.extend(check_reach(bans, &graph.edges));
    violations
}

/// 전이 도달 금지 규칙을 검사합니다.
pub fn check_reach(bans: &[ReachBan], edges: &[ManifestEdge]) -> Vec<GraphViolation> {
    let mut first_hop: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut propagating: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for edge in edges {
        first_hop
            .entry(edge.from.as_str())
            .or_default()
            .insert(edge.to.as_str());
        if edge.table.kind.propagates() {
            propagating
                .entry(edge.from.as_str())
                .or_default()
                .insert(edge.to.as_str());
        }
    }
    let mut violations = Vec::new();
    for ban in bans {
        let Some(start) = first_hop.get(ban.from) else {
            continue;
        };
        let parents = bfs(ban.from, start, &propagating);
        let mut targets: Vec<&str> = ban.never_reach.to_vec();
        targets.sort_unstable();
        for target in targets {
            if parents.contains_key(target) {
                let path = path_to(ban.from, target, &parents);
                violations.push(GraphViolation::new(
                    "forbidden-reach",
                    format!(
                        "전이 도달 금지 위반: {} (ADR-0005 §8: {}는 {}에 도달할 수 없음)",
                        path.join(" -> "),
                        ban.from,
                        target
                    ),
                ));
            }
        }
    }
    violations
}

/// `from`에서 시작해 도달하는 crate마다 BFS 부모를 기록합니다(`from` 자신은 제외).
fn bfs<'a>(
    from: &'a str,
    start: &BTreeSet<&'a str>,
    propagating: &BTreeMap<&'a str, BTreeSet<&'a str>>,
) -> BTreeMap<&'a str, &'a str> {
    let mut parents: BTreeMap<&str, &str> = BTreeMap::new();
    let mut queue: VecDeque<&str> = VecDeque::new();
    for &dep in start {
        if dep != from && !parents.contains_key(dep) {
            parents.insert(dep, from);
            queue.push_back(dep);
        }
    }
    while let Some(node) = queue.pop_front() {
        let Some(nexts) = propagating.get(node) else {
            continue;
        };
        for &next in nexts {
            if next != from && !parents.contains_key(next) {
                parents.insert(next, node);
                queue.push_back(next);
            }
        }
    }
    parents
}

/// `from`이 실제 graph에서 도달하는 crate와 그 최단 경로(`from -> ... -> crate`). 시작은 `from`의 모든 표,
/// 그 뒤는 normal·build 의존만 따라갑니다([`check_reach`]와 같은 규칙, XXIV.4.7).
pub fn reach_paths(from: &str, edges: &[ManifestEdge]) -> BTreeMap<String, Vec<String>> {
    let mut start: BTreeSet<&str> = BTreeSet::new();
    let mut propagating: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for edge in edges {
        if edge.from == from {
            start.insert(edge.to.as_str());
        }
        if edge.table.kind.propagates() {
            propagating
                .entry(edge.from.as_str())
                .or_default()
                .insert(edge.to.as_str());
        }
    }
    let parents = bfs(from, &start, &propagating);
    parents
        .keys()
        .map(|target| {
            (
                (*target).to_string(),
                path_to(from, target, &parents)
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            )
        })
        .collect()
}

/// BFS 부모 기록으로 `from -> ... -> target` 경로를 복원합니다.
fn path_to<'a>(
    from: &'a str,
    target: &'a str,
    parents: &BTreeMap<&'a str, &'a str>,
) -> Vec<&'a str> {
    let mut path = vec![target];
    let mut node = target;
    // 부모 사슬 길이는 방문한 crate 수를 넘지 않습니다.
    for _ in 0..=parents.len() {
        match parents.get(node) {
            Some(&parent) if parent == from => {
                path.push(from);
                break;
            }
            Some(&parent) => {
                path.push(parent);
                node = parent;
            }
            None => break,
        }
    }
    path.reverse();
    path
}

/// 허용 정책 graph에서 `from`이 도달할 수 있는 crate 집합(정책 자체 검사용).
pub fn policy_reach(
    allowed: &HashMap<&'static str, Vec<&'static str>>,
    from: &str,
) -> BTreeSet<&'static str> {
    let mut seen = BTreeSet::new();
    let mut queue: VecDeque<&str> = VecDeque::from([from]);
    while let Some(node) = queue.pop_front() {
        for &dep in allowed.get(node).map(Vec::as_slice).unwrap_or_default() {
            if seen.insert(dep) {
                queue.push_back(dep);
            }
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::super::manifest::{build_graph, parse_manifest, scan_workspace, DepKind, DepTable};
    use super::*;
    use std::path::{Path, PathBuf};

    fn codes(violations: &[GraphViolation]) -> Vec<&'static str> {
        violations.iter().map(|v| v.code).collect()
    }

    fn graph_from(manifests: &[&str]) -> WorkspaceGraph {
        let sources: Vec<(PathBuf, _)> = manifests
            .iter()
            .enumerate()
            .map(|(i, text)| {
                (
                    PathBuf::from(format!("fixture/{i}/Cargo.toml")),
                    parse_manifest(text).expect("fixture manifest"),
                )
            })
            .collect();
        build_graph(&sources, &BTreeMap::new()).expect("fixture graph")
    }

    fn edge(from: &str, to: &str, kind: DepKind) -> ManifestEdge {
        ManifestEdge {
            from: from.to_string(),
            to: to.to_string(),
            table: DepTable { kind, target: None },
        }
    }

    #[test]
    fn current_workspace_passes() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let graph = scan_workspace(root).expect("scan");
        let violations = check_workspace_graph(&graph);
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn target_build_and_dev_table_violations_are_rejected() {
        // 이전 검사기는 `[dependencies]`만 읽어 아래 세 edge를 놓쳤습니다.
        let graph = graph_from(&[r#"
[package]
name = "stellar-document"

[dependencies]
stellar-core-types = { path = "../core-types" }

[target.'cfg(target_os = "macos")'.dependencies]
stellar-ffi = { path = "../ffi" }

[build-dependencies]
stellar-runtime = { path = "../runtime" }

[dev-dependencies]
stellar-project = { path = "../project" }
"#]);
        let violations = check_workspace_graph(&graph);
        assert_eq!(
            codes(&violations),
            ["forbidden-edge", "forbidden-edge", "forbidden-edge"],
            "{violations:?}"
        );
        let messages: Vec<&str> = violations.iter().map(|v| v.message.as_str()).collect();
        assert!(messages
            .iter()
            .any(|m| m.contains("stellar-document -> stellar-ffi")
                && m.contains("[target.'cfg(target_os = \"macos\")'.dependencies]")));
        assert!(messages
            .iter()
            .any(|m| m.contains("-> stellar-runtime ([build-dependencies])")));
        assert!(messages
            .iter()
            .any(|m| m.contains("-> stellar-project ([dev-dependencies])")));
    }

    #[test]
    fn inline_table_dependency_table_violations_are_rejected() {
        // 의존 표를 inline table로 쓴 manifest도 직접 edge와 전이 도달을 똑같이 검사합니다.
        // 이전 해석기는 세 형태 모두 의존 0개로 읽어 검사를 통과시켰습니다.
        for text in [
            "[package]\nname = \"stellar-document\"\n[target.'cfg(unix)']\ndependencies = { stellar-ffi = { path = \"../ffi\" } }\n",
            "dev-dependencies = { stellar-ffi = { path = \"../ffi\" } }\n[package]\nname = \"stellar-document\"\n",
            "[package]\nname = \"stellar-document\"\n[target]\n'cfg(unix)' = { dependencies = { stellar-ffi = { path = \"../ffi\" } } }\n",
        ] {
            let violations = check_workspace_graph(&graph_from(&[text]));
            assert_eq!(codes(&violations), ["forbidden-edge"], "{text:?}");
            assert!(violations[0]
                .message
                .contains("stellar-document -> stellar-ffi"));
        }
        let graph = graph_from(&[
            "[package]\nname = \"stellar-agent-host\"\n[dependencies]\nstellar-diagnostics = { path = \"../diagnostics\" }\n",
            "[package]\nname = \"stellar-diagnostics\"\n[target.'cfg(unix)']\ndependencies = { stellar-document = { path = \"../document\" } }\n",
        ]);
        let reach = check_reach(FORBIDDEN_REACH, &graph.edges);
        assert_eq!(codes(&reach), ["forbidden-reach"]);
        assert!(reach[0]
            .message
            .contains("stellar-agent-host -> stellar-diagnostics -> stellar-document"));
    }

    #[test]
    fn transitive_reach_violation_is_rejected_with_path() {
        // agent-host -> diagnostics(허용) -> document: diagnostics의 직접 edge 위반과 함께
        // agent-host의 전이 도달 위반(ADR-0005 §8)도 보고합니다.
        let graph = graph_from(&[
            "[package]\nname = \"stellar-agent-host\"\n\n[dependencies]\nstellar-diagnostics = { path = \"../diagnostics\" }\n",
            "[package]\nname = \"stellar-diagnostics\"\n\n[target.'cfg(unix)'.dependencies]\nstellar-document = { path = \"../document\" }\n",
        ]);
        let reach = check_reach(FORBIDDEN_REACH, &graph.edges);
        assert_eq!(codes(&reach), ["forbidden-reach"]);
        assert!(reach[0]
            .message
            .contains("stellar-agent-host -> stellar-diagnostics -> stellar-document"));
        let all = codes(&check_workspace_graph(&graph));
        assert!(all.contains(&"forbidden-edge"));
        assert!(all.contains(&"forbidden-reach"));
    }

    #[test]
    fn reach_ban_applies_even_when_every_direct_edge_is_allowed() {
        // 전이 금지 규칙은 허용 표가 바뀌어도 독립적으로 작동합니다: 직접 edge는 모두 허용인
        // 정책에서도 금지 도달은 거부됩니다.
        let mut allowed: HashMap<&'static str, Vec<&'static str>> = HashMap::new();
        allowed.insert("stellar-cli", vec!["stellar-project"]);
        allowed.insert("stellar-project", vec!["stellar-commands"]);
        allowed.insert("stellar-commands", vec![]);
        let bans = [ReachBan {
            from: "stellar-cli",
            never_reach: &["stellar-commands"],
        }];
        let graph = graph_from(&[
            "[package]\nname = \"stellar-cli\"\n[dependencies]\nstellar-project = { path = \"p\" }\n",
            "[package]\nname = \"stellar-project\"\n[dependencies]\nstellar-commands = { path = \"c\" }\n",
            "[package]\nname = \"stellar-commands\"\n",
        ]);
        let violations = check_workspace_graph_with(&allowed, &bans, &graph);
        assert_eq!(codes(&violations), ["forbidden-reach"], "{violations:?}");
    }

    #[test]
    fn dev_dependencies_count_for_the_owner_but_do_not_propagate() {
        let bans = [ReachBan {
            from: "stellar-a",
            never_reach: &["stellar-x"],
        }];
        // a -dev-> x: a 자신의 test 빌드에 x가 들어가므로 위반입니다.
        let own_dev = [edge("stellar-a", "stellar-x", DepKind::Dev)];
        assert_eq!(codes(&check_reach(&bans, &own_dev)), ["forbidden-reach"]);
        // a -> b -dev-> x: b의 dev 의존은 a를 빌드할 때 빌드되지 않습니다.
        let dep_dev = [
            edge("stellar-a", "stellar-b", DepKind::Normal),
            edge("stellar-b", "stellar-x", DepKind::Dev),
        ];
        assert!(check_reach(&bans, &dep_dev).is_empty());
        // a -> b -build-> x: build 의존은 전이됩니다.
        let dep_build = [
            edge("stellar-a", "stellar-b", DepKind::Normal),
            edge("stellar-b", "stellar-x", DepKind::Build),
        ];
        assert_eq!(codes(&check_reach(&bans, &dep_build)), ["forbidden-reach"]);
    }

    #[test]
    fn bans_cover_the_adr_0005_section_8_columns() {
        // ADR-0005 §8 "전이적으로도 도달 금지" 열의 crate가 모두 규칙에 있어야 합니다.
        // (`stellar-mcp`처럼 열보다 넓게 막는 항목은 허용합니다.)
        let columns: [(&str, &[&str]); 2] = [
            (
                "stellar-ops",
                &[
                    "stellar-local-ipc",
                    "stellar-agent-protocol",
                    "stellar-mcp-core",
                    "stellar-agent-host",
                    "stellar-ffi",
                ],
            ),
            (
                "stellar-cli",
                &[
                    "stellar-local-ipc",
                    "stellar-agent-protocol",
                    "stellar-mcp-core",
                    "stellar-agent-host",
                ],
            ),
        ];
        for (from, column) in columns {
            let ban = FORBIDDEN_REACH
                .iter()
                .find(|ban| ban.from == from)
                .unwrap_or_else(|| panic!("{from} 규칙 없음"));
            for target in column {
                assert!(ban.never_reach.contains(target), "{from} -> {target}");
            }
        }
        // runtime(ADR-0005 §8 runtime 행)은 다른 crate를 거쳐서도 mcp-core에 닿으면 안 됩니다.
        let edges = [
            edge("stellar-runtime", "stellar-scene", DepKind::Normal),
            edge("stellar-scene", "stellar-mcp-core", DepKind::Normal),
        ];
        let reach = check_reach(FORBIDDEN_REACH, &edges);
        assert_eq!(codes(&reach), ["forbidden-reach"]);
        assert!(reach[0]
            .message
            .contains("stellar-runtime -> stellar-scene -> stellar-mcp-core"));
        // mcp-core가 생기기 전이라도 diagnostics를 거친 전이 도달은 거부됩니다.
        for from in ["stellar-ops", "stellar-cli"] {
            let edges = [
                edge(from, "stellar-diagnostics", DepKind::Normal),
                edge("stellar-diagnostics", "stellar-mcp-core", DepKind::Normal),
            ];
            let reach = check_reach(FORBIDDEN_REACH, &edges);
            assert_eq!(codes(&reach), ["forbidden-reach"], "{from}");
            assert!(reach[0].message.contains(&format!(
                "{from} -> stellar-diagnostics -> stellar-mcp-core"
            )));
        }
    }

    #[test]
    fn reach_search_terminates_on_cycles() {
        let bans = [ReachBan {
            from: "stellar-a",
            never_reach: &["stellar-z"],
        }];
        let cyclic = [
            edge("stellar-a", "stellar-b", DepKind::Normal),
            edge("stellar-b", "stellar-c", DepKind::Normal),
            edge("stellar-c", "stellar-b", DepKind::Normal),
            edge("stellar-c", "stellar-a", DepKind::Normal),
        ];
        assert!(check_reach(&bans, &cyclic).is_empty());
    }

    #[test]
    fn unknown_package_without_edges_is_reported() {
        let graph = graph_from(&["[package]\nname = \"stellar-unlisted\"\n"]);
        assert_eq!(codes(&check_workspace_graph(&graph)), ["unknown-crate"]);
    }

    #[test]
    fn bans_never_contradict_the_allowed_policy() {
        // 금지 대상은 직접 허용 edge에도, 허용 정책 graph의 전이 도달에도 없어야 합니다.
        let allowed = allowed_edges();
        for ban in FORBIDDEN_REACH {
            assert!(ban.from.starts_with("stellar-"), "{}", ban.from);
            let reach = policy_reach(&allowed, ban.from);
            for target in ban.never_reach {
                assert!(target.starts_with("stellar-"), "{target}");
                assert_ne!(ban.from, *target);
                assert!(
                    !reach.contains(target),
                    "허용 정책이 금지 도달을 허용함: {} -> {target}",
                    ban.from
                );
            }
        }
    }

    #[test]
    fn allowed_policy_is_acyclic() {
        let allowed = allowed_edges();
        for from in allowed.keys() {
            assert!(
                !policy_reach(&allowed, from).contains(from),
                "허용 정책에 순환이 있습니다: {from}"
            );
        }
    }
}
