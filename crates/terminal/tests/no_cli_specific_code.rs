//! XVIII.5.1: 외부 CLI 에이전트는 일반 terminal profile 명령으로만 실행합니다.
//!
//! terminal crate(소스와 생성기)에 특정 CLI·공급자를 알아보는 이름이 없는지 검사합니다. 이름이 없으면
//! 그 CLI만을 위한 분기·실행 경로·출력 parser도 둘 수 없습니다(기술 아키텍처 31.9). 공급자별 출력
//! 해석은 `stellar-agent-host` 안에만 둘 수 있고, workspace 전체 격리 검사는 XXIX.6.6이 맡습니다.

use std::path::{Path, PathBuf};

/// 찾는 이름(소문자 비교).
const CLI_SPECIFIC_NAMES: &[&str] = &[
    "claude",
    "codex",
    "gemini",
    "anthropic",
    "openai",
    "copilot",
    "aider",
    "cursor-agent",
];

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect(&path, out);
        } else if matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("rs" | "py" | "toml")
        ) {
            out.push(path);
        }
    }
}

fn names_in(text: &str) -> Vec<&'static str> {
    let lower = text.to_lowercase();
    CLI_SPECIFIC_NAMES
        .iter()
        .copied()
        .filter(|name| lower.contains(name))
        .collect()
}

#[test]
fn detector_finds_cli_specific_names() {
    // 위반 fixture: 특정 CLI 출력을 알아보는 분기.
    let violation = "if program.ends_with(\"Claude\") { parse_codex_events(output) }";
    assert_eq!(names_in(violation), ["claude", "codex"]);
    assert!(names_in("let profile = TerminalProfile::command(name, argv);").is_empty());
}

#[test]
fn terminal_crate_has_no_cli_specific_code() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for dir in ["src", "tools"] {
        collect(&root.join(dir), &mut files);
    }
    files.push(root.join("Cargo.toml"));
    assert!(files.len() >= 20, "{files:?}");
    let hits: Vec<String> = files
        .iter()
        .flat_map(|file| {
            let text = std::fs::read_to_string(file).unwrap();
            names_in(&text)
                .into_iter()
                .map(move |name| format!("{}: {name}", file.display()))
        })
        .collect();
    assert!(hits.is_empty(), "CLI 전용 이름: {hits:?}");
}
