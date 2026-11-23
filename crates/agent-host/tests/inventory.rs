//! 소스 inventory (XXIX.2.1·XXIX.2.3·XXIX.2.5·XXIX.2.8·XXIX.2.10·XXIX.2.11).
//!
//! 주석을 뺀 코드 줄에서 금지 낱말을 셉니다. 기대값은 모두 0입니다.

use std::path::{Path, PathBuf};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_dir() -> PathBuf {
    std::fs::canonicalize(crate_dir().join("../..")).unwrap()
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else if path
            .extension()
            .is_some_and(|e| e == "rs" || e == "swift" || e == "h" || e == "m")
        {
            out.push(path);
        }
    }
    out.sort();
    out
}

/// 주석(`//` 뒤, `//!`·`///` 포함)과 `#[cfg(test)]` 뒤 단위 테스트를 뺀 코드 줄.
fn code_lines(path: &Path) -> Vec<(usize, String)> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .take_while(|line| line.trim() != "#[cfg(test)]")
        .enumerate()
        .map(|(n, line)| {
            let code = match line.find("//") {
                Some(at) => &line[..at],
                None => line,
            };
            (n + 1, code.to_string())
        })
        .filter(|(_, l)| !l.trim().is_empty())
        .collect()
}

fn hits(files: &[PathBuf], words: &[&str], ignore_case: bool) -> Vec<String> {
    let mut out = Vec::new();
    for file in files {
        for (n, line) in code_lines(file) {
            let hay = if ignore_case {
                line.to_lowercase()
            } else {
                line.clone()
            };
            for word in words {
                let needle = if ignore_case {
                    word.to_lowercase()
                } else {
                    word.to_string()
                };
                if hay.contains(&needle) {
                    out.push(format!("{}:{n}: {word}", file.display()));
                }
            }
        }
    }
    out
}

fn agent_host_sources() -> Vec<PathBuf> {
    rust_files(&crate_dir().join("src"))
}

#[test]
fn no_provider_api_client_key_credential_routing_or_metering() {
    // XXIX.2.1: 모델 제공자 API client, API key field, 자격 증명 저장, 모델 라우팅, 사용량 계량 타입 0.
    let words = [
        "api_key",
        "apikey",
        "std::net",
        "tcpstream",
        "udpsocket",
        "tcplistener",
        "reqwest",
        "hyper::",
        "authorization",
        "bearer",
        "credentials",
        "credential_store",
        "credentialstore",
        "keychain",
        "secitem",
        "model_router",
        "route_model",
        "select_model",
        "modelrouter",
        "\"usage\"",
        "usage_meter",
        "usagemeter",
        "record_usage",
        "token_usage",
        "tokenusage",
        "token_count",
        "input_tokens",
        "output_tokens",
        "cost_usd",
        "billing",
    ];
    // `billing_error`는 CLI가 보고한 오류 이름을 사용 한도로 분류하는 문자열이라 예외입니다(계량 아님).
    let found: Vec<String> = hits(&agent_host_sources(), &words, true)
        .into_iter()
        .filter(|h| !(h.ends_with("billing") && h.contains("claude.rs")))
        .collect();
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn no_cli_credential_location_access() {
    // XXIX.2.5·XXIX.2.3: CLI 자격 증명·설정 저장 위치를 가리키는 경로·이름 0.
    let words = [
        ".claude",
        ".codex",
        "auth.json",
        ".credentials",
        "keychains",
        "codex_home",
        "claude_config_dir",
        "find-generic-password",
        "home_dir",
        "\"home\"",
    ];
    let mut files = agent_host_sources();
    files.push(crate_dir().join("src/bin/test_cli.rs"));
    let found = hits(&files, &words, true);
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn chat_and_decoder_hold_no_document_command_or_approval_types() {
    // XXIX.2.10·XXIX.2.11: 금지 타입·변환 함수 0.
    let mut files = vec![crate_dir().join("src/chat.rs")];
    files.extend(rust_files(&crate_dir().join("src/decoder")));
    files.push(crate_dir().join("src/output.rs"));
    let words = [
        "CommandTransaction",
        "stellar_document",
        "stellar_commands",
        "stellar_history",
        "stellar_storage",
        "stellar_ops",
        "Approval",
        "approve",
        "fn into_command",
        "fn to_command",
        "fn to_transaction",
        "impl From<AgentChatItem>",
        "impl From<&AgentChatItem>",
    ];
    let found = hits(&files, &words, false);
    assert!(found.is_empty(), "{found:#?}");
    // 크레이트 전체도 문서·명령·history·저장·ops crate를 이름으로 쓰지 않습니다.
    let found = hits(
        &agent_host_sources(),
        &[
            "stellar_document",
            "stellar_commands",
            "stellar_history",
            "stellar_storage",
            "stellar_ops",
        ],
        false,
    );
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn no_provider_output_parsing_outside_agent_host() {
    // XXIX.2.11: 공급자 출력 해석 표지가 agent-host 밖(다른 crate, 도구, macOS 셸)에 0.
    let markers = [
        "\"thread.started\"",
        "\"item.completed\"",
        "\"tool_use\"",
        "\"tool_result\"",
        "stream-json",
        "\"agent_message\"",
        "SDKMessage",
    ];
    let mut files = Vec::new();
    for base in ["crates", "tools", "apps"] {
        for file in rust_files(&repo_dir().join(base)) {
            if !file.starts_with(repo_dir().join("crates/agent-host")) {
                files.push(file);
            }
        }
    }
    assert!(files.len() > 50);
    let found = hits(&files, &markers, false);
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn no_session_restart_resume_or_resend_api() {
    // XXIX.2.8: 사용자 동작 없이 시작·재개·재전송하는 수단 0.
    let words = [
        "fn resume",
        "fn retry",
        "fn resend",
        "fn restart",
        "fn reconnect_process",
        "fn replay",
    ];
    let found = hits(&agent_host_sources(), &words, false);
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn test_double_uses_no_network_or_library() {
    // XXIX.2.2: 대역은 network API와 library(framework link)를 쓰지 않습니다.
    let bin = vec![crate_dir().join("src/bin/test_cli.rs")];
    let found = hits(
        &bin,
        &[
            "std::net",
            "TcpStream",
            "UdpSocket",
            "stellar_agent_host",
            "extern \"C\"",
        ],
        false,
    );
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn json_body_matches_mcp_core_copy() {
    fn body(text: &str) -> String {
        text.lines()
            .skip_while(|l| l.starts_with("//!") || l.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }
    let ours = std::fs::read_to_string(crate_dir().join("src/json.rs")).unwrap();
    let theirs = std::fs::read_to_string(repo_dir().join("crates/mcp-core/src/json.rs")).unwrap();
    // mcp-core 사본의 ops 타입 변환(From) 두 개를 뺀 본문과 같아야 합니다.
    let start = theirs.find("// CLI와 MCP는").expect("mcp-core From 시작");
    let end = theirs.find("impl JsonValue {").expect("impl JsonValue");
    let trimmed = format!("{}{}", &theirs[..start], &theirs[end..]);
    assert_eq!(body(&ours), body(&trimmed));
}
