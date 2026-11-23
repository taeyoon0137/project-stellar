//! AgentSession 상태 기계 (XXIX.2.8): 전이 matrix, 중지 후 상태, 재연결, 유예 초과, 한도 오류 재전송 0.

mod support;

use stellar_agent_host::decoder::{CliSignal, DecodeFailure};
use stellar_agent_host::output::OutputSignal;
use stellar_agent_host::session::*;
use stellar_agent_protocol::permission::Permission;
use support::*;

const GRACE: u64 = 1_000;

fn session(mode: SessionMode) -> AgentSession {
    AgentSession::start_by_user(UserStartRequest {
        session: AgentSessionId(42),
        mode,
        now_ms: 0,
        reconnect_grace_ms: GRACE,
    })
}

/// lease 2개, 권한, 요청·제안 대기, 적용 batch 2개를 가진 실행 중 세션.
fn busy() -> AgentSession {
    let mut s = session(SessionMode::ApplyInScope);
    s.apply(SessionEvent::BridgeBound, 1).unwrap();
    for e in [
        SessionEvent::LeaseGranted(LeaseId(1)),
        SessionEvent::LeaseGranted(LeaseId(2)),
        SessionEvent::PermissionGranted(Permission::Read),
        SessionEvent::PermissionGranted(Permission::Apply),
        SessionEvent::RequestOpened(RequestId(7)),
        SessionEvent::ProposalQueued(ProposalId(9)),
    ] {
        s.apply(e, 2).unwrap();
    }
    for seq in 0..2 {
        s.apply(SessionEvent::TransactionStarted, 3).unwrap();
        s.apply(
            SessionEvent::TransactionSettled(Some(AppliedBatch {
                batch_seq: seq,
                history_seq: 100 + u64::from(seq),
            })),
            4,
        )
        .unwrap();
    }
    s
}

fn kinds(effects: &[SessionEffect]) -> Vec<&'static str> {
    effects
        .iter()
        .map(|e| match e {
            SessionEffect::TerminateProcess => "terminate",
            SessionEffect::FinalizeInterruptedHistoryEntry { .. } => "interrupted-entry",
            SessionEffect::ConfirmUndoEntry { .. } => "undo-entry",
            SessionEffect::ReleaseLeases(_) => "release-leases",
            SessionEffect::RevokeSessionPermissions => "revoke",
            SessionEffect::RejectPendingRequests(_) => "reject-requests",
            SessionEffect::DiscardProposals(_) => "discard-proposals",
            SessionEffect::CloseApprovalUi => "close-ui",
            SessionEffect::EndBridgeBinding => "end-binding",
            SessionEffect::CleanupExecutionTraces => "cleanup",
            SessionEffect::RecordDiagnostic(_) => "diagnostic",
        })
        .collect()
}

const END: [&str; 9] = [
    "interrupted-entry",
    "release-leases",
    "revoke",
    "reject-requests",
    "discard-proposals",
    "close-ui",
    "end-binding",
    "cleanup",
    "diagnostic",
];

#[test]
fn stop_keeps_applied_batches_and_clears_lease_permission_pending() {
    let mut s = busy();
    let effects = s.apply(SessionEvent::UserStop, 10).unwrap();
    let mut expected = vec!["terminate"];
    expected.extend(END);
    assert_eq!(kinds(&effects), expected);
    assert_eq!(
        effects[1],
        SessionEffect::FinalizeInterruptedHistoryEntry {
            session: AgentSessionId(42),
            batches: 2
        }
    );
    assert_eq!(
        effects[2],
        SessionEffect::ReleaseLeases(vec![LeaseId(1), LeaseId(2)])
    );
    assert_eq!(
        effects[4],
        SessionEffect::RejectPendingRequests(vec![RequestId(7)])
    );
    assert_eq!(
        effects[5],
        SessionEffect::DiscardProposals(vec![ProposalId(9)])
    );
    // 적용 batch 유지(출처 표시 근거), lease 0, 권한 0, 대기 제안·요청 0, 자동 rollback 효과 없음.
    assert_eq!(s.applied_batches().len(), 2);
    assert_eq!(s.leases().count(), 0);
    assert_eq!(s.permissions().count(), 0);
    assert_eq!(s.pending_proposals(), 0);
    assert_eq!(s.pending_requests(), 0);
    assert_eq!(
        s.state(),
        SessionState::Ended {
            reason: EndReason::UserStopped
        }
    );
    // 진단은 id·duration·code만.
    let SessionEffect::RecordDiagnostic(d) = effects.last().unwrap() else {
        panic!()
    };
    assert_eq!(
        d.to_string(),
        "agent-session=42 duration_ms=10 error=user-stopped"
    );
    // 끝난 세션은 다시 시작·재개하지 않습니다.
    for e in [
        SessionEvent::BridgeBound,
        SessionEvent::UserStop,
        SessionEvent::Tick,
    ] {
        assert_eq!(s.apply(e, 11), Err(TransitionError::Ended));
    }
}

#[test]
fn reconnect_within_grace_keeps_leases_and_rebinds_same_session() {
    let mut s = busy();
    assert!(s
        .apply(SessionEvent::BridgeDisconnected, 100)
        .unwrap()
        .is_empty());
    assert_eq!(
        s.state(),
        SessionState::Reconnecting {
            deadline_ms: 100 + GRACE
        }
    );
    assert!(s
        .apply(SessionEvent::Tick, 100 + GRACE - 1)
        .unwrap()
        .is_empty());
    // 재연결 중에는 새 lease·권한을 받지 않습니다.
    assert!(s
        .apply(SessionEvent::LeaseGranted(LeaseId(3)), 150)
        .is_err());
    assert!(s.apply(SessionEvent::BridgeBound, 200).unwrap().is_empty());
    assert_eq!(s.state(), SessionState::Running);
    assert_eq!(s.id(), AgentSessionId(42));
    assert_eq!(s.leases().collect::<Vec<_>>(), vec![LeaseId(1), LeaseId(2)]);
    assert_eq!(s.permissions().count(), 2);
}

#[test]
fn grace_expiry_finalizes_interrupted_entry() {
    let mut s = busy();
    s.apply(SessionEvent::BridgeDisconnected, 100).unwrap();
    let effects = s.apply(SessionEvent::Tick, 100 + GRACE).unwrap();
    assert!(
        effects.contains(&SessionEffect::FinalizeInterruptedHistoryEntry {
            session: AgentSessionId(42),
            batches: 2
        })
    );
    assert_eq!(
        s.state(),
        SessionState::Ended {
            reason: EndReason::ReconnectGraceExpired
        }
    );
    assert_eq!(s.leases().count(), 0);
}

#[test]
fn late_rebind_after_grace_ends_instead_of_resuming() {
    // 유예가 지난 뒤 Tick보다 재결합이 먼저 도착해도 같은 세션으로 돌아가지 않습니다.
    let mut s = busy();
    s.apply(SessionEvent::BridgeDisconnected, 100).unwrap();
    let effects = s.apply(SessionEvent::BridgeBound, 100 + GRACE).unwrap();
    assert!(
        effects.contains(&SessionEffect::FinalizeInterruptedHistoryEntry {
            session: AgentSessionId(42),
            batches: 2
        })
    );
    assert!(effects.contains(&SessionEffect::EndBridgeBinding));
    assert_eq!(
        s.state(),
        SessionState::Ended {
            reason: EndReason::ReconnectGraceExpired
        }
    );
    assert_eq!(s.leases().count(), 0);
    assert_eq!(s.permissions().count(), 0);
}

#[test]
fn in_flight_transaction_settles_before_end_procedure() {
    let mut s = busy();
    s.apply(SessionEvent::TransactionStarted, 5).unwrap();
    let effects = s.apply(SessionEvent::TrustRevoked, 6).unwrap();
    assert_eq!(kinds(&effects), vec!["terminate"]);
    assert_eq!(
        s.state(),
        SessionState::Ending {
            reason: EndReason::TrustRevoked
        }
    );
    // 끝내는 중에 다른 종료 trigger는 무시, 디코드 실패도 효과 없음.
    assert!(s.apply(SessionEvent::UserStop, 7).unwrap().is_empty());
    let effects = s
        .apply(
            SessionEvent::TransactionSettled(Some(AppliedBatch {
                batch_seq: 2,
                history_seq: 102,
            })),
            8,
        )
        .unwrap();
    assert_eq!(kinds(&effects), END.to_vec());
    assert_eq!(
        effects[0],
        SessionEffect::FinalizeInterruptedHistoryEntry {
            session: AgentSessionId(42),
            batches: 3
        }
    );
    assert_eq!(s.applied_batches().len(), 3);
}

#[test]
fn normal_completion_confirms_one_undo_entry_and_releases_lease() {
    let mut s = busy();
    let effects = s.apply(SessionEvent::EditCompleted, 9).unwrap();
    assert_eq!(
        effects,
        vec![
            SessionEffect::ConfirmUndoEntry {
                session: AgentSessionId(42),
                batches: 2
            },
            SessionEffect::ReleaseLeases(vec![LeaseId(1), LeaseId(2)]),
        ]
    );
    assert_eq!(s.state(), SessionState::Running);
    // 이미 확정된 batch는 중단됨 entry로 다시 묶지 않습니다.
    let effects = s.apply(SessionEvent::ProcessExited, 10).unwrap();
    assert!(!kinds(&effects).contains(&"interrupted-entry"));
    assert!(
        !kinds(&effects).contains(&"terminate"),
        "이미 끝난 프로세스"
    );
}

#[test]
fn mode_caps_permissions_and_excludes_sensitive_ones() {
    let mut s = session(SessionMode::ReviewThenApply);
    s.apply(SessionEvent::BridgeBound, 1).unwrap();
    s.apply(SessionEvent::PermissionGranted(Permission::Propose), 1)
        .unwrap();
    for p in [
        Permission::Apply,
        Permission::Terminal,
        Permission::FileDelete,
        Permission::GitPush,
    ] {
        assert_eq!(
            s.apply(SessionEvent::PermissionGranted(p), 1),
            Err(TransitionError::AboveModeCap(p))
        );
    }
    let mut r = session(SessionMode::ReadOnly);
    r.apply(SessionEvent::BridgeBound, 1).unwrap();
    assert!(r
        .apply(SessionEvent::PermissionGranted(Permission::Propose), 1)
        .is_err());
}

#[test]
fn transition_matrix_matches_report_definition() {
    // (시작 상태 준비, event, 기대: Ok(상태 이름) | Err)
    fn name(state: SessionState) -> &'static str {
        match state {
            SessionState::Starting => "starting",
            SessionState::Running => "running",
            SessionState::Reconnecting { .. } => "reconnecting",
            SessionState::Ending { .. } => "ending",
            SessionState::Ended { .. } => "ended",
        }
    }
    let starting = || session(SessionMode::ApplyInScope);
    let running = || {
        let mut s = starting();
        s.apply(SessionEvent::BridgeBound, 0).unwrap();
        s
    };
    let reconnecting = || {
        let mut s = running();
        s.apply(SessionEvent::BridgeDisconnected, 0).unwrap();
        s
    };
    let ending = || {
        let mut s = running();
        s.apply(SessionEvent::TransactionStarted, 0).unwrap();
        s.apply(SessionEvent::UserStop, 0).unwrap();
        s
    };
    let decode = SessionEvent::Output(OutputSignal::DecodeFailed(DecodeFailure {
        code: "invalid-json",
        line: 1,
    }));
    let limit = SessionEvent::Output(OutputSignal::Cli(CliSignal::UsageLimit));
    type Row<'a> = (
        &'static str,
        Box<dyn Fn() -> AgentSession + 'a>,
        SessionEvent,
        Result<&'static str, ()>,
    );
    let table: Vec<Row<'_>> = vec![
        (
            "starting",
            Box::new(starting),
            SessionEvent::BridgeBound,
            Ok("running"),
        ),
        (
            "starting",
            Box::new(starting),
            SessionEvent::BridgeDisconnected,
            Err(()),
        ),
        (
            "starting",
            Box::new(starting),
            SessionEvent::LeaseGranted(LeaseId(1)),
            Err(()),
        ),
        (
            "starting",
            Box::new(starting),
            SessionEvent::UserStop,
            Ok("ended"),
        ),
        (
            "starting",
            Box::new(starting),
            SessionEvent::ProcessExited,
            Ok("ended"),
        ),
        ("starting", Box::new(starting), decode, Ok("starting")),
        (
            "starting",
            Box::new(starting),
            SessionEvent::Tick,
            Ok("starting"),
        ),
        (
            "running",
            Box::new(running),
            SessionEvent::BridgeBound,
            Err(()),
        ),
        (
            "running",
            Box::new(running),
            SessionEvent::BridgeDisconnected,
            Ok("reconnecting"),
        ),
        (
            "running",
            Box::new(running),
            SessionEvent::LeaseGranted(LeaseId(1)),
            Ok("running"),
        ),
        (
            "running",
            Box::new(running),
            SessionEvent::TransactionSettled(None),
            Err(()),
        ),
        (
            "running",
            Box::new(running),
            SessionEvent::EditCompleted,
            Ok("running"),
        ),
        (
            "running",
            Box::new(running),
            SessionEvent::TrustRevoked,
            Ok("ended"),
        ),
        (
            "running",
            Box::new(running),
            SessionEvent::LeaseConflict,
            Ok("ended"),
        ),
        (
            "running",
            Box::new(running),
            SessionEvent::UserReleasedLease,
            Ok("ended"),
        ),
        ("running", Box::new(running), limit, Ok("ended")),
        ("running", Box::new(running), decode, Ok("running")),
        (
            "reconnecting",
            Box::new(reconnecting),
            SessionEvent::BridgeBound,
            Ok("running"),
        ),
        (
            "reconnecting",
            Box::new(reconnecting),
            SessionEvent::BridgeDisconnected,
            Err(()),
        ),
        (
            "reconnecting",
            Box::new(reconnecting),
            SessionEvent::ProposalQueued(ProposalId(1)),
            Err(()),
        ),
        (
            "reconnecting",
            Box::new(reconnecting),
            SessionEvent::RequestAnswered(RequestId(1)),
            Ok("reconnecting"),
        ),
        (
            "reconnecting",
            Box::new(reconnecting),
            SessionEvent::UserStop,
            Ok("ended"),
        ),
        (
            "ending",
            Box::new(ending),
            SessionEvent::UserStop,
            Ok("ending"),
        ),
        (
            "ending",
            Box::new(ending),
            SessionEvent::BridgeBound,
            Err(()),
        ),
        ("ending", Box::new(ending), decode, Ok("ending")),
        (
            "ending",
            Box::new(ending),
            SessionEvent::TransactionSettled(None),
            Ok("ended"),
        ),
    ];
    for (from, make, event, expected) in table {
        let mut s = make();
        assert_eq!(name(s.state()), from);
        let got = s.apply(event, 1).map(|_| name(s.state())).map_err(|_| ());
        assert_eq!(got, expected, "{from} + {event:?}");
    }
}

#[test]
fn limit_error_from_cli_stops_session_without_resend() {
    // 테스트 대역이 사용 한도 오류를 보고하면 세션이 멈추고, 사용자 동작 없는 재전송이 없습니다.
    let probe = TempDir::new("limit");
    let stdin_dump = probe.join("stdin.txt");
    let fixture = format!(
        "{}/tests/fixtures/decoder/claude-stream-json/usage-limit.jsonl",
        env!("CARGO_MANIFEST_DIR")
    );
    let s = setup(
        "limit",
        "claude",
        &format!(
            "{OK_PROBES}[session]\nread-line\nstdin-dump {}\noutfile {fixture}\nhang\n",
            stdin_dump.display()
        ),
    );
    let v = FixtureVerifier::accepting();
    let pass = run_preflight(&s.profile, &s.target, &v, &s.home, &s.scratch).unwrap();
    let output = output_for(&s.profile);
    let mut agent = AgentSession::start_by_user(UserStartRequest {
        session: AgentSessionId(1),
        mode: SessionMode::ReviewThenApply,
        now_ms: 0,
        reconnect_grace_ms: DEFAULT_RECONNECT_GRACE_MS,
    });
    output.push_user_prompt("한도 시험");
    let process = pass
        .launch("한도 시험".into(), Some(mcp()), output.clone())
        .unwrap();
    agent.apply(SessionEvent::BridgeBound, 1).unwrap();
    let mut signals = Vec::new();
    assert!(wait_until(std::time::Duration::from_secs(5), || {
        signals.extend(output.take_signals());
        !signals.is_empty()
    }));
    let mut terminate = false;
    for signal in signals {
        let effects = agent.apply(SessionEvent::Output(signal), 2).unwrap();
        terminate |= effects.contains(&SessionEffect::TerminateProcess);
    }
    assert!(terminate);
    assert_eq!(
        agent.state(),
        SessionState::Ended {
            reason: EndReason::UsageLimitReported
        }
    );
    process.cancel();
    let end = process.wait();
    assert_eq!(
        end.reason,
        stellar_agent_host::process::ProcessEndReason::Cancelled
    );
    // CLI가 받은 사용자 메시지는 사용자가 보낸 첫 한 줄뿐입니다(상태 기계에 재전송 효과가 없음).
    let sent = std::fs::read_to_string(&stdin_dump).unwrap();
    assert_eq!(sent.lines().count(), 1, "{sent}");
    assert!(sent.contains("한도 시험"));
    // 오류 문장은 그대로 채팅 item에 있습니다.
    assert!(output
        .chat_items()
        .iter()
        .any(|i| i.to_golden().contains("\"error\":\"usage-limit\"")));
    assert_eq!(
        agent.apply(SessionEvent::BridgeBound, 3),
        Err(TransitionError::Ended)
    );
}
