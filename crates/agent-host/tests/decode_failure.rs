//! 디코드 실패 판정의 세션 처리 연결 (XXIX.2.13).
//!
//! 보고서 정의: 실패하면 채팅 디코드만 닫고(`DecodeStopped` item 하나), 원문 수신·보존과 CLI 프로세스는 계속되며,
//! 세션 상태·권한·lease·대기 요청·적용 변경은 그대로입니다(효과 0). 세션을 끝낼지는 사용자 동작과 다른 종료
//! 조건이 정합니다.

mod support;

use std::time::Duration;

use stellar_agent_host::chat::AgentChatItem;
use stellar_agent_host::output::OutputSignal;
use stellar_agent_host::session::*;
use stellar_agent_protocol::permission::Permission;
use support::*;

#[test]
fn unparseable_output_closes_chat_only_and_changes_no_authority() {
    let fixture = format!(
        "{}/tests/fixtures/decoder/claude-stream-json/fail-invalid-json.jsonl",
        env!("CARGO_MANIFEST_DIR")
    );
    let s = setup(
        "decode-fail",
        "claude",
        &format!(
            "{OK_PROBES}[session]\nread-line\nchunk 9\noutfile {fixture}\nout {{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false}}\\n\nsleep 100\nout after-failure\\n\nexit 0\n"
        ),
    );
    let v = FixtureVerifier::accepting();
    let pass = run_preflight(&s.profile, &s.target, &v, &s.home, &s.scratch).unwrap();
    let output = output_for(&s.profile);

    let mut agent = AgentSession::start_by_user(UserStartRequest {
        session: AgentSessionId(3),
        mode: SessionMode::ApplyInScope,
        now_ms: 0,
        reconnect_grace_ms: DEFAULT_RECONNECT_GRACE_MS,
    });
    agent.apply(SessionEvent::BridgeBound, 1).unwrap();
    agent
        .apply(SessionEvent::LeaseGranted(LeaseId(1)), 1)
        .unwrap();
    agent
        .apply(SessionEvent::PermissionGranted(Permission::Read), 1)
        .unwrap();
    agent
        .apply(SessionEvent::PermissionGranted(Permission::Apply), 1)
        .unwrap();
    agent
        .apply(SessionEvent::RequestOpened(RequestId(2)), 1)
        .unwrap();
    agent.apply(SessionEvent::TransactionStarted, 1).unwrap();
    agent
        .apply(
            SessionEvent::TransactionSettled(Some(AppliedBatch {
                batch_seq: 0,
                history_seq: 9,
            })),
            1,
        )
        .unwrap();
    let before = agent.clone();

    let process = pass
        .launch("x".into(), Some(mcp()), output.clone())
        .unwrap();
    let mut signals = Vec::new();
    assert!(wait_until(Duration::from_secs(5), || {
        signals.extend(output.take_signals());
        signals
            .iter()
            .any(|s| matches!(s, OutputSignal::DecodeFailed(_)))
    }));
    for signal in &signals {
        let effects = agent.apply(SessionEvent::Output(*signal), 2).unwrap();
        assert!(effects.is_empty(), "디코드 실패 효과 0: {effects:?}");
    }
    // 권한·lease 확대 0(그리고 축소도 0), 적용 변경 자동 삭제 0, 상태 유지.
    assert_eq!(agent.state(), SessionState::Running);
    assert_eq!(
        agent.leases().collect::<Vec<_>>(),
        before.leases().collect::<Vec<_>>()
    );
    assert_eq!(
        agent.permissions().collect::<Vec<_>>(),
        before.permissions().collect::<Vec<_>>()
    );
    assert_eq!(agent.pending_requests(), 1);
    assert_eq!(agent.applied_batches(), before.applied_batches());
    assert!(agent.decode_failed());

    let end = process.wait();
    assert_eq!(
        end.reason,
        stellar_agent_host::process::ProcessEndReason::Exited
    );
    // 채팅: 실패 전 item, DecodeStopped 하나, 그 뒤 item 0.
    let items = output.chat_items();
    let stop = items
        .iter()
        .position(|i| {
            matches!(
                i,
                AgentChatItem::DecodeStopped {
                    code: "invalid-json"
                }
            )
        })
        .unwrap();
    assert_eq!(stop, items.len() - 1);
    assert!(matches!(items[0], AgentChatItem::AssistantText { .. }));
    // 원문은 실패 뒤 출력까지 보존.
    assert!(output.raw_display_text().contains("after-failure"));
    // CLI 프로세스 종료는 일반 종료 절차로 이어집니다(디코드 실패와 무관).
    let effects = agent.apply(SessionEvent::ProcessExited, 3).unwrap();
    assert!(
        effects.contains(&SessionEffect::FinalizeInterruptedHistoryEntry {
            session: AgentSessionId(3),
            batches: 1
        })
    );
    assert_eq!(agent.applied_batches().len(), 1);
}
