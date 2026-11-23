//! 출력 표시 격리 inventory (XXIX.6.6, ADR-0005 §4.4·§8).
//!
//! 검증 기준: 네 검사 모두 0건, 위반 fixture 거부.
//!
//! 1. 터미널 경로(`crates/terminal`)와 다른 crate·셸·FFI에 공급자별 출력 parser가 없고, 터미널 출력이 제품
//!    상태로 해석되지 않습니다(터미널 crate는 문서·명령 crate에 의존하지 않고, 어느 crate도 터미널 crate에
//!    의존해 그 출력을 명령으로 바꾸지 않음).
//! 2. 디코더(`Decoder`·`decode_all`·`DecoderKind`·`AgentChatItem`)와 공급자 출력 표지는 agent-host 안에만 있습니다.
//! 3. 디코드 결과 타입(`AgentChatItem`·`DecodeOutput`·`CliSignal`·`OutputSignal`)에서 CommandTransaction·문서
//!    변경·승인 응답으로 가는 함수가 없습니다.
//! 4. agent-host의 의존 closure(manifest path 의존, 자기 dev 의존 포함)에 commands·document·history·storage·ops가
//!    없습니다.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use support::scan::{code_lines, hits, repo_dir, source_files};
use support::TempDir;

/// 공급자 출력 형식 표지(구조화 출력 type 이름과 형식 이름).
const PROVIDER_MARKERS: [&str; 10] = [
    "\"thread.started\"",
    "\"item.completed\"",
    "\"turn.completed\"",
    "\"tool_use\"",
    "\"tool_result\"",
    "stream-json",
    "exec-jsonl",
    "\"agent_message\"",
    "SDKMessage",
    "\"stream_event\"",
];

/// 디코더 타입 이름.
const DECODER_NAMES: [&str; 5] = [
    "AgentChatItem",
    "decode_all",
    "DecoderKind",
    "DecodeOutput",
    "stellar_agent_host::decoder",
];

/// 공급자 이름(터미널 경로에는 특별 처리 없음, XVIII.5.1).
const PROVIDER_NAMES: [&str; 4] = ["claude", "codex", "anthropic", "openai"];

fn non_agent_host_sources(root: &Path) -> Vec<PathBuf> {
    let host = root.join("crates/agent-host");
    ["crates", "tools", "apps"]
        .iter()
        .flat_map(|b| source_files(&root.join(b)))
        .filter(|f| !f.starts_with(&host))
        .collect()
}

fn check_parsers_outside_agent_host(root: &Path) -> Vec<String> {
    let files = non_agent_host_sources(root);
    let mut found = hits(&files, &PROVIDER_MARKERS);
    found.extend(hits(&files, &DECODER_NAMES));
    // 터미널 crate는 공급자 이름도 특별히 다루지 않습니다(대소문자 무시).
    for file in source_files(&root.join("crates/terminal/src")) {
        for (n, line) in code_lines(&file) {
            let lower = line.to_lowercase();
            for word in PROVIDER_NAMES {
                if lower.contains(word) {
                    found.push(format!("{}:{n}: {word}", file.display()));
                }
            }
        }
    }
    found
}

/// manifest의 path 의존 package 이름(표 이름별).
fn path_deps(manifest: &Path) -> BTreeMap<String, Vec<PathBuf>> {
    let text = std::fs::read_to_string(manifest).unwrap_or_default();
    let mut table = String::new();
    let mut out: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            table = line.trim_matches(|c| c == '[' || c == ']').to_string();
            continue;
        }
        if let Some(at) = line.find("path = \"") {
            let rest = &line[at + 8..];
            let rel = &rest[..rest.find('"').unwrap()];
            out.entry(table.clone())
                .or_default()
                .push(manifest.parent().unwrap().join(rel));
        }
    }
    out
}

fn package_name(dir: &Path) -> String {
    let text = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap_or_default();
    text.lines()
        .find_map(|l| l.trim().strip_prefix("name = \""))
        .map(|n| n.trim_end_matches('"').to_string())
        .unwrap_or_default()
}

/// `start` crate 디렉터리의 도달 집합: 자기 모든 표에서 시작하고 그 뒤로는 normal·build 의존만 따라갑니다
/// (xtask `crate_graph::reach`와 같은 규칙).
fn reach(start: &Path) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut queue: Vec<PathBuf> = path_deps(&start.join("Cargo.toml"))
        .into_values()
        .flatten()
        .collect();
    while let Some(dir) = queue.pop() {
        let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
        let name = package_name(&dir);
        if name.is_empty() || !seen.insert(name) {
            continue;
        }
        for (table, deps) in path_deps(&dir.join("Cargo.toml")) {
            if !table.contains("dev-dependencies") {
                queue.extend(deps);
            }
        }
    }
    seen
}

const NEVER_REACH: [&str; 5] = [
    "stellar-commands",
    "stellar-document",
    "stellar-history",
    "stellar-storage",
    "stellar-ops",
];

fn check_reach(start: &Path) -> Vec<String> {
    let reached = reach(start);
    NEVER_REACH
        .iter()
        .filter(|n| reached.contains(**n))
        .map(|n| n.to_string())
        .collect()
}

/// 디코드 결과 타입을 받거나 돌려주면서 명령·문서 변경·승인 응답 타입을 다루는 함수 선언.
fn check_decode_result_paths(src: &Path) -> Vec<String> {
    let decode_types = [
        "AgentChatItem",
        "DecodeOutput",
        "CliSignal",
        "OutputSignal",
        "DecodeFailure",
    ];
    let forbidden = [
        "CommandTransaction",
        "Command",
        "Resolution",
        "UserAnswer",
        "Answered",
        "Outcome",
        "Permission",
        "LeaseId",
        "ApprovalRequest",
        "EffectEntry",
        "BatchRecord",
    ];
    let mut out = Vec::new();
    for file in source_files(src) {
        let lines = code_lines(&file);
        for (i, (n, line)) in lines.iter().enumerate() {
            if !line.contains("fn ") && !line.contains("impl From<") {
                continue;
            }
            // 선언이 여러 줄이면 `{`까지 합칩니다.
            let mut decl = line.clone();
            let mut j = i + 1;
            while !decl.contains('{') && !decl.contains(';') && j < lines.len() {
                decl.push_str(&lines[j].1);
                j += 1;
            }
            let decl = decl.split('{').next().unwrap_or_default().to_string();
            let takes_decoded = decode_types.iter().any(|t| decl.contains(t));
            let words: Vec<&str> = decl
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .collect();
            let bad = forbidden.iter().find(|f| words.contains(f));
            if takes_decoded {
                if let Some(bad) = bad {
                    out.push(format!("{}:{n}: {bad}", file.display()));
                }
            }
        }
    }
    out
}

#[test]
fn no_provider_parser_outside_agent_host_and_terminal_output_is_not_product_state() {
    let root = repo_dir();
    let found = check_parsers_outside_agent_host(&root);
    assert!(found.is_empty(), "{found:#?}");
    // 터미널 crate는 core-types만 쓰고, 어느 crate도 터미널 crate에 의존하지 않습니다.
    let terminal = reach(&root.join("crates/terminal"));
    assert_eq!(
        terminal.into_iter().collect::<Vec<_>>(),
        ["stellar-core-types"]
    );
    let mut dependents = Vec::new();
    for base in ["crates", "tools"] {
        for entry in std::fs::read_dir(root.join(base)).unwrap().flatten() {
            let manifest = entry.path().join("Cargo.toml");
            if entry.file_name() == "terminal" || !manifest.exists() {
                continue;
            }
            let uses = path_deps(&manifest)
                .values()
                .flatten()
                .any(|d| package_name(d) == "stellar-terminal");
            if uses {
                dependents.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    assert!(dependents.is_empty(), "{dependents:?}");
}

#[test]
fn decode_results_have_no_path_to_commands_documents_or_approvals() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let found = check_decode_result_paths(&src);
    assert!(found.is_empty(), "{found:#?}");
    // 채팅 조립은 CLI 보고 item을 받기만 하고 아무것도 돌려주지 않습니다.
    let assembly = std::fs::read_to_string(src.join("assembly.rs")).unwrap();
    assert!(assembly.contains("pub fn push_chat(&mut self, item: AgentChatItem) {"));
}

#[test]
fn agent_host_reaches_no_document_or_command_crate() {
    let host = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let reached = reach(&host);
    assert!(reached.contains("stellar-agent-protocol"), "{reached:?}");
    assert!(check_reach(&host).is_empty(), "{reached:?}");
}

#[test]
fn checkers_reject_violation_fixtures() {
    let dir = TempDir::new("isolation-violation");
    let root = dir.path();
    // (1)(2) 다른 crate의 공급자 parser와 디코더 사용, 터미널의 공급자 특별 처리.
    std::fs::create_dir_all(root.join("crates/viewer/src")).unwrap();
    std::fs::write(
        root.join("crates/viewer/src/lib.rs"),
        concat!(
            "const KIND: &str = r#\"\"tool_use\"\"#;\n",
            "fn f() { let _ = stellar_agent_host::decoder::Decoder::new; }\n",
        ),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("crates/terminal/src")).unwrap();
    std::fs::write(
        root.join("crates/terminal/src/lib.rs"),
        "fn spawn(p: &str) { if p == \"Claude\" { parse_events(); } }\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("crates/agent-host/src")).unwrap();
    std::fs::write(
        root.join("crates/agent-host/src/decoder.rs"),
        "const T: &str = \"stream-json\";\n",
    )
    .unwrap();
    let found = check_parsers_outside_agent_host(root);
    // 공급자 표지 1, 디코더 경로 1, 터미널의 공급자 이름 1. agent-host 안의 표지는 세지 않습니다.
    assert_eq!(found.len(), 3, "{found:#?}");
    for want in ["\"tool_use\"", "stellar_agent_host::decoder", "claude"] {
        assert!(
            found.iter().any(|h| h.ends_with(want)),
            "{want}: {found:#?}"
        );
    }
    assert!(found.iter().all(|h| !h.contains("agent-host")));

    // (3) 디코드 결과에서 승인 응답·명령으로 가는 함수.
    let src = root.join("crates/agent-host/src");
    std::fs::write(
        src.join("bad.rs"),
        "pub fn approve_from(item: &AgentChatItem) -> UserAnswer { todo!() }\nimpl From<AgentChatItem> for CommandTransaction {}\npub fn to_cmd(\n    s: CliSignal,\n) -> Command {\n}\n",
    )
    .unwrap();
    let found = check_decode_result_paths(&src);
    assert_eq!(found.len(), 3, "{found:#?}");

    // (4) agent-host 의존 closure에 문서 crate가 생기는 manifest.
    for (name, deps) in [
        ("agent-host", "stellar-mid = { path = \"../mid\" }\n"),
        ("mid", "stellar-document = { path = \"../document\" }\n"),
        ("document", ""),
    ] {
        let d = root.join("crates").join(name);
        std::fs::create_dir_all(&d).unwrap();
        let package = if name == "agent-host" {
            "stellar-agent-host".to_string()
        } else {
            format!("stellar-{name}")
        };
        std::fs::write(
            d.join("Cargo.toml"),
            format!("[package]\nname = \"{package}\"\n\n[dependencies]\n{deps}"),
        )
        .unwrap();
    }
    assert_eq!(
        check_reach(&root.join("crates/agent-host")),
        ["stellar-document"]
    );
}
