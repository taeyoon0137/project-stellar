//! ADR-0005 §3 변경 불가 경로 inventory (XXIX.6.5).
//!
//! 검증 기준: 프로젝트 파일 변조 fixture에서 상태 변경 0(`stellar-runtime` `agent` 단위 test
//! `project_file_tampering_changes_no_trust_mode_or_notice_state`), agent 경로의 출처 표시 정리·되돌리기 호출 0.
//!
//! 에이전트 경로는 MCP(`crates/mcp-core`, `tools/stellar-mcp`, `crates/agent-protocol`), 로컬 IPC
//! (`crates/local-ipc`), headless `stellar` 명령(`tools/stellar-cli`)입니다. 이 경로의 제품 코드에는 다음 상태를
//! 쓰는 호출이 없어야 합니다.
//!
//! | 상태 | 쓰는 수단(앱 사용자 UI 전용) |
//! |---|---|
//! | XVI.7 프로젝트 신뢰 | `FileTrustStore::grant`·`revoke`, `TrustStore::set`·`forget` |
//! | XIX.3 권한 | `PermissionStore::respond`·`respond_connection`, `ApprovalStore::answer` |
//! | 에이전트 모드 | `AgentSession::change_mode_by_user`, `ChatAssembly::set_mode` |
//! | 약관 고지 기록 | `TermsNoticeStore::acknowledge` |
//! | 출처 표시·되돌리기 | 출처 정리·유지·되돌리기 함수(XIX.6.8~XIX.6.10, 이름 표지) |
//!
//! 에이전트 경로가 agent-host에 도달하지 않는 것(crate graph `FORBIDDEN_REACH`)과 함께, 이 test가 호출 표지
//! 자체가 없음을 봅니다. 위반 fixture로 검사기가 실제로 잡는지도 확인합니다.

mod support;

use std::path::PathBuf;

use support::scan::{code_lines, hits, source_files, sources_in};
use support::TempDir;

/// 에이전트 경로(제품 코드).
const AGENT_PATHS: [&str; 5] = [
    "crates/mcp-core/src",
    "tools/stellar-mcp/src",
    "crates/agent-protocol/src",
    "crates/local-ipc/src",
    "tools/stellar-cli/src",
];

/// 변경 불가 상태를 쓰는 호출 표지.
const WRITE_MARKERS: [&str; 22] = [
    // 신뢰
    ".grant(",
    ".revoke(",
    "TrustStore::set",
    "trust_store.set(",
    ".forget(",
    // 권한·승인 응답
    ".respond_connection(",
    "permissions.respond(",
    "store.respond(",
    "ApprovalStore",
    "UserAnswer",
    // 모드
    "change_mode_by_user",
    ".set_mode(",
    "SessionMode",
    // 약관 고지
    "TermsNoticeStore",
    ".acknowledge(",
    // 출처 표시·되돌리기
    "clear_provenance",
    "keep_provenance",
    "dismiss_provenance",
    "revert_batch",
    "revert_session",
    "revert_agent",
    "AgentChangeProvenance",
];

fn violations(files: &[PathBuf]) -> Vec<String> {
    let mut found = hits(files, &WRITE_MARKERS);
    // JSON-RPC 응답 함수 호출(`self.respond(&id, ..)`)은 권한 응답이 아닙니다. 그 줄만 빼고, 그 밖의
    // `.respond(` 호출은 어느 파일에 있든 금지입니다(파일 이름으로 통째로 빼지 않음).
    for file in files {
        for (n, line) in code_lines(file) {
            if line.contains(".respond(") && !line.contains("self.respond(&id") {
                found.push(format!("{}:{n}: .respond(", file.display()));
            }
        }
    }
    found
}

/// 앱 사용자 되돌리기·출처 module로, ADR-0100 결정 3에 따라 앱 쪽 에이전트 편집 crate가 생길 때까지 mcp-core 안에
/// 임시로 둔 파일입니다. 이 파일들은 검사에서 빼는 대신, 에이전트 경로의 다른 파일이 이 module을 부르지 않는지
/// [`agent_paths_do_not_call_the_app_revert_modules`]가 따로 봅니다. 옮기면 이 목록을 지웁니다.
const APP_SIDE_MODULES: [&str; 6] = [
    "crates/mcp-core/src/provenance.rs",
    "crates/mcp-core/src/provenance_fixture.rs",
    "crates/mcp-core/src/provenance_tests.rs",
    "crates/mcp-core/src/agent_history/",
    "crates/mcp-core/src/attached/revert.rs",
    "crates/mcp-core/src/attached/revert_tests.rs",
];

fn is_app_side(file: &std::path::Path) -> bool {
    let text = file.to_string_lossy().replace('\\', "/");
    APP_SIDE_MODULES.iter().any(|m| text.contains(m))
}

#[test]
fn agent_paths_do_not_call_the_app_revert_modules() {
    let files: Vec<PathBuf> = sources_in(&AGENT_PATHS)
        .into_iter()
        .filter(|f| !is_app_side(f))
        .collect();
    let found = hits(
        &files,
        &["provenance::", "agent_history::", "attached::revert", "revert::"],
    );
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn agent_paths_cannot_write_immutable_state() {
    let files: Vec<PathBuf> = sources_in(&AGENT_PATHS)
        .into_iter()
        .filter(|f| !is_app_side(f))
        .collect();
    assert!(
        files.len() > 30,
        "검사 대상이 비어 있지 않음: {}",
        files.len()
    );
    let found = violations(&files);
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn bridge_events_and_tool_calls_have_no_mode_or_permission_write() {
    // 세션 상태 기계 event(bridge·출력·앱 효과에서 옴)에는 모드 변경 event가 없습니다. 모드는
    // `change_mode_by_user` 하나로만 바뀝니다.
    let session =
        std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/session.rs"))
            .unwrap();
    let start = session.find("pub enum SessionEvent").unwrap();
    let end = start + session[start..].find("\n}\n").unwrap();
    let body = &session[start..end];
    for word in ["Mode", "Trust(", "Grant(", "Provenance", "Revert"] {
        assert!(!body.contains(word), "SessionEvent에 `{word}`");
    }
    // 출력 디코드 결과(OutputSignal·CliSignal)는 권한·모드·신뢰를 담지 않습니다.
    let output =
        std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/output.rs"))
            .unwrap();
    for word in ["Permission", "SessionMode", "Trust"] {
        assert!(!output.contains(word), "output.rs에 `{word}`");
    }
}

#[test]
fn checker_rejects_violation_fixture() {
    // 위반 fixture: 에이전트 경로에서 상태를 쓰는 가짜 소스.
    let dir = TempDir::new("immutable-violation");
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("tool.rs"),
        "fn handle() {\n    trust.grant(&identity);\n    store.respond(key, id, true);\n    session.change_mode_by_user(m);\n    notices.acknowledge(&p);\n    index.clear_provenance(s);\n    window.revert_session(s);\n    // 주석의 .grant( 는 세지 않습니다\n}\n#[cfg(test)]\nmod tests { fn t() { x.revoke(&y); } }\n",
    )
    .unwrap();
    // 파일 이름이 server.rs여도 JSON-RPC 응답이 아닌 `.respond(`는 잡습니다.
    std::fs::write(
        src.join("server.rs"),
        "fn rpc(&self) {\n    self.respond(&id, &method, outcome, log);\n    permissions.respond(key, id, true);\n    self.store.respond(key, id, true);\n}\n",
    )
    .unwrap();
    let server_hits: Vec<String> = violations(&[src.join("server.rs")])
        .into_iter()
        .map(|h| h.rsplit(':').nth(1).unwrap().trim().to_string())
        .collect();
    let mut server_lines = server_hits.clone();
    server_lines.sort();
    server_lines.dedup();
    assert_eq!(server_lines, ["3", "4"], "{server_hits:#?}");
    std::fs::remove_file(src.join("server.rs")).unwrap();
    let files = source_files(dir.path());
    let found = violations(&files);
    // 한 줄에 표지가 둘이면 두 번 나옵니다. 줄 기준으로 2~7행(6줄)만 잡히고 주석·단위 테스트는 빠집니다.
    let mut lines: Vec<usize> = found
        .iter()
        .map(|h| h.rsplit(':').nth(1).unwrap().trim().parse().unwrap())
        .collect();
    lines.sort_unstable();
    lines.dedup();
    assert_eq!(lines, [2, 3, 4, 5, 6, 7], "{found:#?}");
}
