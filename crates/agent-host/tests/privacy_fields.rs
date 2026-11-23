//! 에이전트 금지 필드 corpus와 진단 누출 검사 (XXIX.6.4).
//!
//! 검증 기준(엔진 부분): privacy scan 0건, 재시작 후 세션 0·출처 표시 0. 표지 값을 세션의 모든 입력(사용자
//! 문장, CLI 출력·표준 오류, CLI 실행 파일 경로, 결합 token, 출처 기록)에 넣고, 진단 기록·`Debug`·오류 문장·
//! OS 사용자 데이터에 쓴 파일을 [`scan`]합니다. 진단 schema key(XVI.8.6)에 에이전트 필드 이름이 없는지도 봅니다.

mod support;

use std::path::{Path, PathBuf};
use std::time::Duration;

use stellar_agent_host::approval::*;
use stellar_agent_host::assembly::{BatchRecord, ChatAssembly};
use stellar_agent_host::privacy::{scan, AGENT_FORBIDDEN_FIELDS};
use stellar_agent_host::process::McpServerLaunch;
use stellar_agent_host::session::*;
use stellar_agent_host::start::*;
use stellar_agent_protocol::core_types::ids::StableId;
use stellar_local_ipc::binding::BindingToken;
use support::*;

fn marker(name: &str) -> &'static str {
    AGENT_FORBIDDEN_FIELDS
        .iter()
        .find(|f| f.name == name)
        .unwrap()
        .marker
}

struct Evidence;
impl ApprovalEvidence for Evidence {
    fn parts(&self) -> EvidenceParts {
        EvidenceParts {
            semantic_diff: true,
            validation: true,
            render_preview: true,
        }
    }
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(files_under(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}

#[test]
fn diagnostics_debug_and_user_data_hold_no_agent_fields() {
    let chat = marker("agent_chat_item");
    let raw = marker("agent_raw_output");
    let stderr = marker("agent_cli_stderr");
    let account = marker("agent_cli_account_identity");
    let peer = marker("agent_ipc_peer");
    let scenario = format!(
        "{OK_PROBES}[session]\nread-line\nerr {stderr} {account} {peer}\\n\nout {{\"type\":\"system\",\"subtype\":\"init\",\"model\":\"m\",\"session_id\":\"{account}\"}}\\n\nout {{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{chat}\"}}]}}}}\\n\nout {raw}\\n\nexit 0\n"
    );
    // CLI 실행 파일 경로에 표지를 넣습니다.
    let s = setup(marker("agent_cli_executable_path"), "claude", &scenario);
    assert!(s
        .exe
        .to_string_lossy()
        .contains(marker("agent_cli_executable_path")));
    let v = FixtureVerifier::accepting();
    let pass = run_preflight(&s.profile, &s.target, &v, &s.home, &s.scratch).unwrap();
    let output = output_for(&s.profile);
    let prompt = marker("agent_user_prompt");
    output.push_user_prompt(prompt);
    let binding = BindingToken::from_bytes([0x7f, 0x3a].repeat(16).try_into().unwrap());
    let process = pass
        .launch(
            prompt.to_string(),
            Some(McpServerLaunch {
                command: mcp().command,
                args: mcp().args,
                binding,
            }),
            output.clone(),
        )
        .unwrap();
    assert!(wait_until(Duration::from_secs(10), || output.streams_closed()));
    let end = process.wait();

    // 입력이 실제로 세션 메모리에 들어갔는지 먼저 확인합니다(검사가 비어 있지 않음).
    let raw_text = output.raw_display_text();
    assert!(raw_text.contains(raw) && raw_text.contains(stderr));
    assert!(output
        .chat_items()
        .iter()
        .any(|i| i.to_golden().contains(chat)));

    // 세션·승인·조립을 표지와 함께 끝까지 돌립니다.
    let data = s.dir.join("user-data");
    let mut notices = TermsNoticeStore::open(&data).unwrap();
    notices.acknowledge(&s.profile).unwrap();
    let project = ProjectKey::new(
        StableId::from_bytes([1; 16]),
        PathBuf::from(format!("/tmp/{}", marker("agent_provenance_index"))),
    );
    let window = WindowSessionId::from_stable_id(StableId::from_bytes([2; 16]));
    let confirmation = prepare_start(
        project.clone(),
        window,
        McpCapability::Allowed,
        &s.profile,
        SessionMode::ApplyInScope,
        &notices,
    )
    .unwrap();
    let confirmation_debug = format!("{confirmation:?}");
    let started = confirm_start(
        confirmation,
        McpCapability::Allowed,
        &s.profile,
        &notices,
        AgentSessionId(1),
        0,
        DEFAULT_RECONNECT_GRACE_MS,
    )
    .unwrap();
    let mut session = started.session;
    let mut store: ApprovalStore<Evidence> = ApprovalStore::new();
    let mut assembly = ChatAssembly::new(AgentSessionId(1), SessionMode::ApplyInScope);
    for item in output.chat_items() {
        assembly.push_chat(item);
    }
    session.apply(SessionEvent::BridgeBound, 1).unwrap();
    let id = store
        .open(
            project.clone(),
            ApprovalRequest {
                session: AgentSessionId(1),
                window,
                subject: ApprovalSubject::Batch {
                    proposal: ProposalId(1),
                },
                evidence: Evidence,
                deadline_ms: 100,
            },
            1,
        )
        .unwrap();
    assembly
        .record_request(
            AgentSessionId(1),
            id,
            ApprovalSubject::Batch {
                proposal: ProposalId(1),
            },
        )
        .unwrap();
    session.apply(SessionEvent::RequestOpened(id), 1).unwrap();
    session.apply(SessionEvent::TransactionStarted, 2).unwrap();
    session
        .apply(
            SessionEvent::TransactionSettled(Some(AppliedBatch {
                batch_seq: 0,
                history_seq: 1,
            })),
            2,
        )
        .unwrap();
    assembly
        .record_batch(&BatchRecord {
            session: AgentSessionId(1),
            batch_seq: 0,
            history_seq: 1,
            proposal: None,
            changed_nodes: 1,
            changed_properties: 1,
            created_nodes: 0,
            deleted_nodes: 0,
        })
        .unwrap();
    let effects = session.apply(SessionEvent::UserStop, 3).unwrap();
    store.apply_session_effects(AgentSessionId(1), &effects, 3);

    // 진단에 남는 것: 세션 효과의 진단 기록, 각 상태의 Debug, 오류 문장.
    let mut diagnostics = Vec::new();
    for e in &effects {
        if let SessionEffect::RecordDiagnostic(d) = e {
            diagnostics.push(d.to_string());
        }
        diagnostics.push(format!("{e:?}"));
    }
    diagnostics.extend([
        format!("{output:?}"),
        format!("{:?}", output.raw_stats()),
        format!("{session:?}"),
        format!("{store:?}"),
        format!("{assembly:?}"),
        format!("{project:?}"),
        format!("{:?}", started.connection),
        format!("{end:?}"),
        confirmation_debug,
        StartRefusal::Untrusted.code().to_string(),
        NoticeError::Corrupt("x").to_string(),
        ApprovalError::SessionEnded.to_string(),
    ]);
    for text in &diagnostics {
        assert!(scan(text).is_empty(), "누출 {:?}: {text}", scan(text));
    }
    // OS 사용자 데이터에 쓴 파일(약관 고지 기록)에도 없습니다.
    for file in files_under(&data) {
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(scan(&text).is_empty(), "{}: {text}", file.display());
    }
}

#[test]
fn restart_restores_no_session_or_provenance() {
    // 재시작 = 새 저장소. 세션·승인·채팅·출처 기록을 읽어 오는 경로가 없습니다.
    let store: ApprovalStore<Evidence> = ApprovalStore::new();
    assert_eq!(store.pending_len(), 0);
    let assembly = ChatAssembly::new(AgentSessionId(1), SessionMode::default());
    assert_eq!(assembly.entries().count(), 0);
    // 파일을 쓰는 코드는 약관 고지 기록(start.rs)과 실행 scratch(process.rs: 세션 종료 때 지움)뿐입니다.
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut writers = Vec::new();
    for file in files_under(&src) {
        let text = std::fs::read_to_string(&file).unwrap();
        let code: String = text
            .lines()
            .take_while(|l| l.trim() != "#[cfg(test)]")
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for word in ["fs::write", "File::create", "OpenOptions", "create_new"] {
            if code.contains(word) {
                let name = file.file_name().unwrap().to_string_lossy().into_owned();
                if !writers.contains(&name) {
                    writers.push(name);
                }
            }
        }
    }
    writers.sort();
    assert_eq!(writers, ["process.rs", "start.rs"]);
    // 세션·채팅·원문·승인·조립 타입에는 직렬화(serde 등) 경로가 없습니다.
    for module in [
        "session.rs",
        "chat.rs",
        "raw.rs",
        "approval.rs",
        "assembly.rs",
        "output.rs",
    ] {
        let text = std::fs::read_to_string(src.join(module)).unwrap();
        for word in [
            "Serialize",
            "Deserialize",
            "serde",
            "fn save",
            "fn persist",
            "fn restore",
            "fn load",
        ] {
            assert!(!text.contains(word), "{module}: {word}");
        }
    }
}

#[test]
fn diagnostic_schemas_have_no_agent_fields() {
    // XVI.8.6 schema inventory: 진단 결과·interaction trace JSON key에 에이전트 필드 이름이 없습니다.
    let keys: Vec<&str> = stellar_diagnostics::result::RESULT_KEYS
        .iter()
        .chain(stellar_diagnostics::interaction::TRACE_LINE_KEYS.iter())
        .copied()
        .collect();
    for key in &keys {
        assert!(scan(key).is_empty(), "{key}");
        for word in [
            "prompt",
            "chat",
            "stderr",
            "account",
            "binding",
            "peer",
            "provenance",
        ] {
            assert!(!key.contains(word), "{key}");
        }
    }
    // 세션 진단 기록 형식은 session id, duration, error code만입니다.
    let d = SessionDiagnostic {
        session: AgentSessionId(4),
        duration_ms: 10,
        error_code: EndReason::UserStopped.code(),
    };
    assert_eq!(
        d.to_string(),
        "agent-session=4 duration_ms=10 error=user-stopped"
    );
}
