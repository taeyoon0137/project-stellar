//! 테스트 대역 기반 E2E 시나리오 공통 fixture (XXIX.6.1).
//!
//! 검증 기준: 시나리오 정의 golden, fixture 로더 test 통과. 이 파일은 fixture를 읽어 정규 표기를 golden과 비교하고,
//! headless 실행기로 세션 수준 단계와 확인을 실제 테스트 대역 CLI 프로세스와 함께 실행합니다. 문서·history가
//! 필요한 `app` 단계와 `check.app` 확인은 macOS 앱 실행기(XXIX.6.2)가 실행합니다.
//!
//! golden 갱신: `STELLAR_BLESS_DECODER_GOLDEN=1 cargo test -p stellar-agent-host --test e2e_scenarios`

mod support;

use std::path::PathBuf;
use std::time::Duration;

use stellar_agent_host::approval::*;
use stellar_agent_host::assembly::{BatchRecord, ChatAssembly, TranscriptEntry};
use stellar_agent_host::chat::AgentChatItem;
use stellar_agent_host::output::OutputSignal;
use stellar_agent_host::scenario::*;
use stellar_agent_host::session::*;
use stellar_agent_host::start::*;
use stellar_agent_protocol::core_types::ids::StableId;
use support::*;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn load_all() -> Vec<(String, Scenario)> {
    let mut names: Vec<String> = std::fs::read_dir(fixture_dir().join("e2e"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".json"))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|n| {
            let text = std::fs::read_to_string(fixture_dir().join("e2e").join(&n)).unwrap();
            let scenario = Scenario::parse(&text).unwrap_or_else(|e| panic!("{n}: {e}"));
            (n, scenario)
        })
        .collect()
}

#[test]
fn scenario_definitions_match_golden() {
    let all = load_all();
    let ids: Vec<&str> = all.iter().map(|(_, s)| s.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "complete-undo",
            "decode-failure",
            "reconnect-grace",
            "stop-then-revert"
        ],
        "계획서 XXIX.6.1의 네 시나리오"
    );
    for (name, s) in &all {
        assert_eq!(format!("{}.json", s.id), *name, "파일 이름 = id");
    }
    let text: String = all.iter().map(|(_, s)| s.to_golden()).collect();
    let golden = fixture_dir().join("e2e/INDEX.golden");
    if std::env::var_os("STELLAR_BLESS_DECODER_GOLDEN").is_some() {
        std::fs::write(&golden, &text).unwrap();
    }
    assert_eq!(text, std::fs::read_to_string(&golden).unwrap());
    // 같은 원문은 같은 정의입니다.
    for (name, s) in &all {
        let again = Scenario::parse(
            &std::fs::read_to_string(fixture_dir().join("e2e").join(name)).unwrap(),
        )
        .unwrap();
        assert_eq!(&again, s);
    }
}

#[test]
fn loader_rejects_malformed_scenarios() {
    let base = std::fs::read_to_string(fixture_dir().join("e2e/complete-undo.json")).unwrap();
    let cases: [(&str, String, &str); 8] = [
        ("format", base.replace("scenario/1", "scenario/2"), "format"),
        (
            "unknown top key",
            base.replacen("\"id\"", "\"extra\": 1, \"id\"", 1),
            "unknown-key",
        ),
        (
            "unknown step",
            base.replace("\"do\": \"complete\"", "\"do\": \"auto-rollback\""),
            "unknown-step",
        ),
        (
            "unknown step key",
            base.replace(
                "\"do\": \"complete\"",
                "\"do\": \"complete\", \"force\": true",
            ),
            "unknown-key",
        ),
        (
            "time goes back",
            base.replace(
                "{\"at\": 12, \"do\": \"complete\"}",
                "{\"at\": 1, \"do\": \"complete\"}",
            ),
            "time",
        ),
        (
            "first step",
            base.replace("{\"at\": 0, \"do\": \"start\"},", ""),
            "first-step",
        ),
        (
            "unopened request",
            base.replace("\"request\": 2", "\"request\": 9"),
            "unknown-request",
        ),
        (
            "unknown app expectation",
            base.replace("\"undo-restores-baseline\"", "\"looks-fine\""),
            "unknown-name",
        ),
    ];
    for (what, text, code) in cases {
        assert_ne!(text, base, "{what}: 변형이 적용되어야 함");
        let e = Scenario::parse(&text).unwrap_err();
        assert_eq!(e.code, code, "{what}: {e}");
    }
}

/// headless 실행기 상태.
struct Run {
    session: Option<AgentSession>,
    store: ApprovalStore<MarkedEvidence>,
    now: u64,
    assembly: ChatAssembly,
    output: Option<std::sync::Arc<stellar_agent_host::output::SessionOutput>>,
    process: Option<stellar_agent_host::process::SessionProcess>,
    chat_mark: u64,
    effects: Vec<&'static str>,
    opened: Vec<(RequestId, ApprovalSubject)>,
    outcomes: Vec<Resolution>,
    batches: Vec<BatchRecord>,
    app_steps: Vec<String>,
    _dir: TempDir,
}

const SESSION: AgentSessionId = AgentSessionId(1);

fn project() -> ProjectKey {
    ProjectKey::new(StableId::from_bytes([9; 16]), PathBuf::from("/tmp/e2e"))
}

fn window() -> WindowSessionId {
    WindowSessionId::from_stable_id(StableId::from_bytes([8; 16]))
}

impl Run {
    fn record_effects(&mut self, effects: &[SessionEffect], now: u64) {
        for e in effects {
            if !self.effects.contains(&e.kind()) {
                self.effects.push(e.kind());
            }
        }
        for r in self.store.apply_session_effects(SESSION, effects, now) {
            self.assembly.record_resolution(&r).unwrap();
            self.outcomes.push(r);
        }
        if effects.contains(&SessionEffect::TerminateProcess) {
            if let Some(p) = &self.process {
                p.cancel();
            }
        }
    }

    fn session(&mut self) -> &mut AgentSession {
        self.session.as_mut().expect("start 뒤")
    }

    fn event(&mut self, event: SessionEvent, now: u64) {
        let effects = self.session().apply(event, now).unwrap();
        self.record_effects(&effects, now);
    }

    fn sync_chat(&mut self) {
        if let Some(output) = &self.output {
            let (items, mark) = output.chat_since(self.chat_mark);
            self.chat_mark = mark;
            for item in items {
                self.assembly.push_chat(item);
            }
            for signal in output.take_signals() {
                let effects = self
                    .session
                    .as_mut()
                    .unwrap()
                    .apply(SessionEvent::Output(signal), self.now)
                    .unwrap();
                assert!(
                    matches!(signal, OutputSignal::DecodeFailed(_)) == effects.is_empty(),
                    "디코드 실패 신호만 효과 0"
                );
                let now = self.now;
                self.record_effects(&effects, now);
            }
        }
    }

    fn open(&mut self, subject: ApprovalSubject, now: u64) {
        let id = self
            .store
            .open(
                project(),
                ApprovalRequest {
                    session: SESSION,
                    window: window(),
                    subject,
                    evidence: MarkedEvidence,
                    deadline_ms: now + DEFAULT_RESPONSE_DEADLINE_MS,
                },
                now,
            )
            .unwrap();
        self.assembly.record_request(SESSION, id, subject).unwrap();
        self.opened.push((id, subject));
        self.event(SessionEvent::RequestOpened(id), now);
    }
}

/// headless 실행기 전용 근거 표지. headless에는 문서가 없어 코어 근거를 만들 수 없으므로 batch 승인 요청을 열
/// 수 있도록 세 근거가 있다고 표시합니다. 실제 근거(semantic diff·validation·렌더 프리뷰)는 앱 실행기가
/// 코어 결과로 채웁니다(XXIX.6.2).
#[derive(Clone, Copy)]
struct MarkedEvidence;
impl ApprovalEvidence for MarkedEvidence {
    fn parts(&self) -> EvidenceParts {
        EvidenceParts {
            semantic_diff: true,
            validation: true,
            render_preview: true,
        }
    }
}

fn check(run: &mut Run, c: &Check, id: &str, at: u64) {
    run.sync_chat();
    let s = run.session.as_ref().unwrap();
    let ctx = format!("{id}@{at}");
    if let Some(state) = &c.state {
        assert_eq!(&s.state().code(), state, "{ctx}");
    }
    if let Some(n) = c.leases {
        assert_eq!(s.leases().count(), n as usize, "{ctx} leases");
    }
    if let Some(p) = &c.permissions {
        assert_eq!(&s.permissions().collect::<Vec<_>>(), p, "{ctx} permissions");
    }
    if let Some(n) = c.pending_requests {
        assert_eq!(s.pending_requests(), n as usize, "{ctx} session requests");
        assert_eq!(
            run.store.pending(&project()).count(),
            n as usize,
            "{ctx} store"
        );
    }
    if let Some(n) = c.pending_proposals {
        assert_eq!(s.pending_proposals(), n as usize, "{ctx} proposals");
    }
    if let Some(n) = c.applied {
        assert_eq!(s.applied_batches().len(), n as usize, "{ctx} applied");
    }
    if let Some(effects) = &c.effects {
        assert_eq!(&run.effects, effects, "{ctx} effects");
    }
    let kinds: Vec<&str> = run
        .assembly
        .entries()
        .filter_map(|e| match e {
            TranscriptEntry::Chat(item) => Some(item.kind()),
            TranscriptEntry::Effect(_) => None,
        })
        .collect();
    for kind in &c.chat {
        assert!(
            kinds.contains(&kind.as_str()),
            "{ctx} chat {kind}: {kinds:?}"
        );
    }
    if let Some(outcomes) = &c.outcomes {
        let got: Vec<String> = run.outcomes.iter().map(|r| r.outcome.code()).collect();
        assert_eq!(&got, outcomes, "{ctx} outcomes");
    }
    // 문서 효과 표시는 언제나 기록과 같습니다(XXIX.3.6).
    assert!(
        run.assembly
            .verify(&run.opened, &run.outcomes, &run.batches)
            .is_empty(),
        "{ctx} 효과 표시 불일치"
    );
}

fn run_scenario(scenario: &Scenario) -> Run {
    let double_lines: Vec<String> = scenario
        .double
        .iter()
        .map(|l| match l.strip_prefix("outfile ") {
            Some(rel) => format!("outfile {}", fixture_dir().join(rel).display()),
            None => l.clone(),
        })
        .collect();
    let probes = if scenario.profile == "claude" {
        OK_PROBES
    } else {
        OK_PROBES_CODEX
    };
    let layout = setup(
        &format!("e2e-{}", scenario.id),
        &scenario.profile,
        &format!("{probes}[session]\n{}\n", double_lines.join("\n")),
    );
    let mut run = Run {
        session: None,
        store: ApprovalStore::new(),
        now: 0,
        assembly: ChatAssembly::new(SESSION, scenario.mode),
        output: None,
        process: None,
        chat_mark: 0,
        effects: Vec::new(),
        opened: Vec::new(),
        outcomes: Vec::new(),
        batches: Vec::new(),
        app_steps: Vec::new(),
        _dir: TempDir::new(&format!("e2e-data-{}", scenario.id)),
    };
    let data = run._dir.path().to_path_buf();
    let verifier = FixtureVerifier::accepting();
    for step in &scenario.steps {
        let now = step.at_ms;
        run.now = now;
        match &step.action {
            Action::Start => {
                let mut notices = TermsNoticeStore::open(&data).unwrap();
                notices.acknowledge(&layout.profile).unwrap();
                let confirmation = prepare_start(
                    project(),
                    window(),
                    McpCapability::Allowed,
                    &layout.profile,
                    scenario.mode,
                    &notices,
                )
                .unwrap();
                let started = confirm_start(
                    confirmation,
                    McpCapability::Allowed,
                    &layout.profile,
                    &notices,
                    SESSION,
                    now,
                    DEFAULT_RECONNECT_GRACE_MS,
                )
                .unwrap();
                assert_eq!(started.connection.session, SESSION);
                run.session = Some(started.session);
            }
            Action::Launch { prompt } => {
                let pass = run_preflight(
                    &layout.profile,
                    &layout.target,
                    &verifier,
                    &layout.home,
                    &layout.scratch,
                )
                .unwrap();
                let output = output_for(&layout.profile);
                output.push_user_prompt(prompt);
                run.process = Some(
                    pass.launch(prompt.clone(), Some(mcp()), output.clone())
                        .unwrap(),
                );
                run.output = Some(output);
            }
            Action::BridgeBound => run.event(SessionEvent::BridgeBound, now),
            Action::BridgeDisconnected => run.event(SessionEvent::BridgeDisconnected, now),
            Action::Tick => run.event(SessionEvent::Tick, now),
            Action::Grant(p) => run.event(SessionEvent::PermissionGranted(*p), now),
            Action::Lease(l) => run.event(SessionEvent::LeaseGranted(LeaseId(*l)), now),
            Action::OpenScope { targets } => run.open(
                ApprovalSubject::Scope {
                    targets: *targets,
                    externals: 0,
                },
                now,
            ),
            Action::OpenBatch { proposal } => {
                run.event(SessionEvent::ProposalQueued(ProposalId(*proposal)), now);
                run.open(
                    ApprovalSubject::Batch {
                        proposal: ProposalId(*proposal),
                    },
                    now,
                );
            }
            Action::Answer { request, approve } => {
                let (id, _) = run.opened[*request as usize - 1];
                let answer = if *approve {
                    UserAnswer::Approve
                } else {
                    UserAnswer::Deny
                };
                if let Answered::Resolved(r) =
                    run.store.answer(&project(), id, answer, now).unwrap()
                {
                    run.assembly.record_resolution(&r).unwrap();
                    run.outcomes.push(r);
                }
                run.event(SessionEvent::RequestAnswered(id), now);
            }
            Action::Apply {
                batch,
                history,
                proposal,
            } => {
                run.event(SessionEvent::TransactionStarted, now);
                run.event(
                    SessionEvent::TransactionSettled(Some(AppliedBatch {
                        batch_seq: *batch,
                        history_seq: *history,
                    })),
                    now,
                );
                if let Some(p) = proposal {
                    run.event(SessionEvent::ProposalResolved(ProposalId(*p)), now);
                }
                let record = BatchRecord {
                    session: SESSION,
                    batch_seq: *batch,
                    history_seq: *history,
                    proposal: proposal.map(ProposalId),
                    changed_nodes: 1,
                    changed_properties: 1,
                    created_nodes: 0,
                    deleted_nodes: 0,
                };
                run.assembly.record_batch(&record).unwrap();
                run.batches.push(record);
            }
            Action::Complete => run.event(SessionEvent::EditCompleted, now),
            Action::Stop => run.event(SessionEvent::UserStop, now),
            Action::ReleaseLease => run.event(SessionEvent::UserReleasedLease, now),
            Action::AwaitOutput(wait) => {
                let output = run.output.clone().unwrap();
                let reached = wait_until(Duration::from_secs(10), || match wait {
                    OutputWait::DecodeFailed => output.decode_failure().is_some(),
                    OutputWait::TurnCompleted => output
                        .chat_items()
                        .iter()
                        .any(|i| matches!(i, AgentChatItem::TurnCompleted { .. })),
                    OutputWait::Closed => output.streams_closed(),
                });
                assert!(reached, "{}: 출력 대기 시간 초과", scenario.id);
                run.sync_chat();
            }
            Action::App { action, batch } => {
                // 앱 실행기 전용(XXIX.6.2). headless는 건너뛰고 세션 상태를 바꾸지 않습니다.
                run.app_steps.push(match batch {
                    Some(b) => format!("{} {b}", action.name()),
                    None => action.name().to_string(),
                });
            }
            Action::Check(c) => check(&mut run, c, &scenario.id, now),
        }
    }
    if let Some(process) = run.process.take() {
        process.cancel();
        let _ = process.wait();
    }
    run
}

#[test]
fn headless_runner_executes_session_level_steps() {
    for (_, scenario) in load_all() {
        let run = run_scenario(&scenario);
        let app_actions = scenario
            .steps
            .iter()
            .filter(|s| matches!(s.action, Action::App { .. }))
            .count();
        assert_eq!(run.app_steps.len(), app_actions, "{}", scenario.id);
        assert!(!scenario.app_expectations().is_empty(), "{}", scenario.id);
    }
}
