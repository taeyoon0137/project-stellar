//! XIX.8.1: MCP·에이전트 model에 agent 전용 작업 사본 타입이 없습니다.
//!
//! 에이전트는 일반 프로젝트 identity(project id + canonical target, IV.2.3)와 시스템 Git worktree만 씁니다. 에이전트
//! 세션·권한·lease·출처 key는 모두 그 identity에 묶이고, 별도 작업 사본(ShadowWorktree 같은 것)을 만들지
//! 않습니다(아키텍처 §31.6). workspace 전체의 agent 전용 작업 사본 타입 금지는 XVIII.5.3 test
//! (`crates/terminal/tests/worktree_type_inventory.rs`)가 맡고, 이 test는 MCP·에이전트 crate가 작업 사본 타입을
//! 아예 선언하지 않는지와 세션 key가 일반 프로젝트 identity인지 봅니다.

use std::path::{Path, PathBuf};

use crate::permission::{AgentSessionId, PermissionKey};

/// MCP·에이전트 model crate(L2·L3)와 앱 쪽 에이전트 편집 crate.
const AGENT_ROOTS: &[&str] = &[
    "crates/mcp-core",
    "crates/agent-edit",
    "crates/agent-protocol",
    "crates/agent-host",
    "tools/stellar-mcp",
];

/// 작업 사본 개념 이름 조각(소문자·`_` 제거 비교).
const WORKTREE_NOUNS: &[&str] = &[
    "worktree",
    "checkout",
    "workspace",
    "workdir",
    "workingcopy",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.map(|e| e.unwrap().path()).collect();
    paths.sort();
    for path in paths {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir() {
            if !name.starts_with('.') && name != "target" {
                rust_files(&path, out);
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// 선언된 타입 이름 중 작업 사본 개념인 것.
fn worktree_types(text: &str) -> Vec<String> {
    let tokens: Vec<&str> = text
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|t| !t.is_empty())
        .collect();
    tokens
        .windows(2)
        .filter(|pair| matches!(pair[0], "struct" | "enum" | "trait" | "type" | "union"))
        .map(|pair| pair[1])
        .filter(|name| {
            let normalized: String = name
                .chars()
                .filter(|&c| c != '_')
                .flat_map(char::to_lowercase)
                .collect();
            WORKTREE_NOUNS.iter().any(|noun| normalized.contains(noun))
        })
        .map(str::to_owned)
        .collect()
}

#[test]
fn agent_crates_declare_no_worktree_types() {
    let root = workspace_root();
    let mut files = Vec::new();
    for dir in AGENT_ROOTS {
        let dir = root.join(dir);
        assert!(dir.is_dir(), "{} 없음", dir.display());
        rust_files(&dir, &mut files);
    }
    assert!(files.len() > 20, "검사 파일 수 {}", files.len());
    let mut found = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        for name in worktree_types(&text) {
            found.push(format!("{}: {name}", file.display()));
        }
    }
    assert!(
        found.is_empty(),
        "MCP·에이전트 model의 작업 사본 타입: {found:?}"
    );
}

#[test]
fn detector_rejects_shadow_worktree_fixture() {
    let fixture = format!("pub struct {}{} {{ path: PathBuf }}", "Shadow", "Worktree");
    assert_eq!(
        worktree_types(&fixture),
        vec![format!("{}{}", "Shadow", "Worktree")]
    );
    let fixture = format!("enum Agent_{} {{}}", "Checkout");
    assert_eq!(worktree_types(&fixture).len(), 1);
    assert!(worktree_types("struct PermissionKey { project_id: StableId }").is_empty());
}

#[test]
fn agent_session_key_is_general_project_identity() {
    // 세션 key는 project id + canonical target(일반 ProjectIdentity) + 세션뿐입니다.
    let key = PermissionKey::new(
        stellar_ops::core_types::ids::StableId::new_v7(1, 0, 1),
        PathBuf::from("/repo"),
        AgentSessionId(stellar_ops::core_types::ids::StableId::new_v7(2, 0, 2)),
    );
    let PermissionKey {
        project_id: _,
        canonical_target: _,
        session: _,
    } = key;
}
