//! 채팅 보기 조립 (XXIX.3.6).
//!
//! 검증 기준: 기록 fixture로 만든 item 목록 golden 일치, 승인·출처 기록과 어긋나는 문서 효과 표시 0.
//!
//! golden 갱신: `STELLAR_BLESS_DECODER_GOLDEN=1 cargo test -p stellar-agent-host --test chat_assembly`

use std::path::PathBuf;

use stellar_agent_host::approval::*;
use stellar_agent_host::assembly::*;
use stellar_agent_host::chat::{AgentChatItem, DisplayText};
use stellar_agent_host::decoder::{decode_all, DecoderKind};
use stellar_agent_host::session::*;
use stellar_agent_protocol::core_types::ids::StableId;
use stellar_agent_protocol::permission::Permission;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn key() -> ProjectKey {
    ProjectKey::new(
        StableId::from_bytes([3; 16]),
        PathBuf::from("/tmp/assembly"),
    )
}

fn window() -> WindowSessionId {
    WindowSessionId::from_stable_id(StableId::from_bytes([4; 16]))
}

struct Full;
impl ApprovalEvidence for Full {
    fn parts(&self) -> EvidenceParts {
        EvidenceParts {
            semantic_diff: true,
            validation: true,
            render_preview: true,
        }
    }
}

/// 기록 fixture를 실행한 결과.
struct Replayed {
    assembly: ChatAssembly,
    opened: Vec<(RequestId, ApprovalSubject)>,
    resolutions: Vec<Resolution>,
    batches: Vec<BatchRecord>,
}

fn mode_named(name: &str) -> SessionMode {
    SessionMode::ALL
        .into_iter()
        .find(|m| m.name() == name)
        .unwrap()
}

fn replay(name: &str) -> Replayed {
    let text = std::fs::read_to_string(fixtures().join("assembly").join(name)).unwrap();
    let mut store: ApprovalStore<Full> = ApprovalStore::new();
    let mut assembly: Option<ChatAssembly> = None;
    let mut session = AgentSessionId(0);
    let mut opened = Vec::new();
    let mut resolutions: Vec<Resolution> = Vec::new();
    let mut batches = Vec::new();
    let mut now = 0u64;
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        now += 1_000;
        let (head, rest) = line.split_once(' ').unwrap_or((line, ""));
        let args: Vec<&str> = rest.split(' ').collect();
        match head {
            "session" => {
                session = AgentSessionId(args[0].parse().unwrap());
                assembly = Some(ChatAssembly::new(session, mode_named(args[1])));
            }
            "prompt" => assembly
                .as_mut()
                .unwrap()
                .push_chat(AgentChatItem::UserPrompt {
                    text: DisplayText::new(rest),
                }),
            "cli" => {
                let (kind, file) = rest.split_once('/').unwrap();
                let decoder = match kind {
                    "claude-stream-json" => DecoderKind::ClaudeStreamJsonV1,
                    _ => DecoderKind::CodexExecJsonlV1,
                };
                let raw = std::fs::read(fixtures().join("decoder").join(kind).join(file)).unwrap();
                let (items, _, failure) = decode_all(decoder, &raw);
                assert!(failure.is_none());
                for item in items {
                    assembly.as_mut().unwrap().push_chat(item);
                }
            }
            "open" => {
                let subject = match args[0] {
                    "scope" => ApprovalSubject::Scope {
                        targets: args[1].parse().unwrap(),
                        externals: args[2].parse().unwrap(),
                    },
                    "batch" => ApprovalSubject::Batch {
                        proposal: ProposalId(args[1].parse().unwrap()),
                    },
                    _ => ApprovalSubject::Permission(Permission::from_name(args[1]).unwrap()),
                };
                let id = store
                    .open(
                        key(),
                        ApprovalRequest {
                            session,
                            window: window(),
                            subject,
                            evidence: Full,
                            deadline_ms: now + DEFAULT_RESPONSE_DEADLINE_MS,
                        },
                        now,
                    )
                    .unwrap();
                assembly
                    .as_mut()
                    .unwrap()
                    .record_request(session, id, subject)
                    .unwrap();
                opened.push((id, subject));
            }
            "answer" => {
                let id = RequestId(args[0].parse().unwrap());
                let answer = if args[1] == "approve" {
                    UserAnswer::Approve
                } else {
                    UserAnswer::Deny
                };
                let Answered::Resolved(r) = store.answer(&key(), id, answer, now).unwrap() else {
                    panic!("중복 응답")
                };
                assembly.as_mut().unwrap().record_resolution(&r).unwrap();
                resolutions.push(r);
            }
            "expire" => {
                now = args[0].parse().unwrap();
                for r in store.expire(now) {
                    assembly.as_mut().unwrap().record_resolution(&r).unwrap();
                    resolutions.push(r);
                }
            }
            "batch" => {
                let n = |i: usize| -> u32 { args[i].parse().unwrap() };
                let record = BatchRecord {
                    session,
                    batch_seq: n(0),
                    history_seq: args[1].parse().unwrap(),
                    proposal: (args[2] != "-").then(|| ProposalId(args[2].parse().unwrap())),
                    changed_nodes: n(3),
                    changed_properties: n(4),
                    created_nodes: n(5),
                    deleted_nodes: n(6),
                };
                assembly.as_mut().unwrap().record_batch(&record).unwrap();
                batches.push(record);
            }
            "end" => {
                for r in store.end_session(session, now) {
                    assembly.as_mut().unwrap().record_resolution(&r).unwrap();
                    resolutions.push(r);
                }
            }
            other => panic!("모르는 기록 {other}"),
        }
    }
    Replayed {
        assembly: assembly.unwrap(),
        opened,
        resolutions,
        batches,
    }
}

#[test]
fn record_fixture_assembles_to_golden() {
    let replayed = replay("review-session.records");
    let lines: Vec<String> = replayed
        .assembly
        .entries()
        .map(TranscriptEntry::to_golden)
        .collect();
    let text = lines.join("\n") + "\n";
    let golden = fixtures().join("assembly/review-session.golden");
    if std::env::var_os("STELLAR_BLESS_DECODER_GOLDEN").is_some() {
        std::fs::write(&golden, &text).unwrap();
    }
    assert_eq!(text, std::fs::read_to_string(&golden).unwrap());
    // 표시된 효과는 기록과 정확히 같습니다.
    assert_eq!(
        replayed
            .assembly
            .verify(&replayed.opened, &replayed.resolutions, &replayed.batches),
        Vec::<Mismatch>::new()
    );
    // 같은 기록은 항상 같은 목록입니다.
    let again: Vec<String> = replay("review-session.records")
        .assembly
        .entries()
        .map(TranscriptEntry::to_golden)
        .collect();
    assert_eq!(again, lines);
}

#[test]
fn cli_reports_never_become_document_effects() {
    // CLI가 tool 성공을 보고해도 기록이 없으면 문서 효과는 0입니다.
    let raw = std::fs::read(fixtures().join("decoder/claude-stream-json/normal.jsonl")).unwrap();
    let (items, _, _) = decode_all(DecoderKind::ClaudeStreamJsonV1, &raw);
    let mut assembly = ChatAssembly::new(AgentSessionId(1), SessionMode::ApplyInScope);
    for item in items {
        assembly.push_chat(item);
    }
    assembly.push_chat(AgentChatItem::AssistantText {
        text: DisplayText::new("노드 3개를 바꿨습니다(적용 완료)."),
    });
    assert!(assembly
        .entries()
        .all(|e| matches!(e, TranscriptEntry::Chat(_))));
    assert!(assembly.verify(&[], &[], &[]).is_empty());
}

#[test]
fn effects_that_disagree_with_records_are_refused() {
    let s = AgentSessionId(1);
    let mut a = ChatAssembly::new(s, SessionMode::ReviewThenApply);
    let batch = |seq: u32, proposal: Option<u64>| BatchRecord {
        session: s,
        batch_seq: seq,
        history_seq: 9,
        proposal: proposal.map(ProposalId),
        changed_nodes: 1,
        changed_properties: 1,
        created_nodes: 0,
        deleted_nodes: 0,
    };
    // 승인되지 않은 제안의 적용, 검토 후 적용 모드의 바로 적용, 다른 세션의 기록은 받지 않습니다.
    assert_eq!(
        a.record_batch(&batch(0, Some(1))),
        Err(AssemblyError::BatchNotApproved(ProposalId(1)))
    );
    assert_eq!(
        a.record_batch(&batch(0, None)),
        Err(AssemblyError::DirectApplyAboveMode)
    );
    let mut other = batch(0, None);
    other.session = AgentSessionId(2);
    assert_eq!(a.record_batch(&other), Err(AssemblyError::OtherSession));
    // 열린 적 없는 요청의 확정과 대상이 다른 확정도 받지 않습니다.
    let resolution = Resolution {
        id: RequestId(7),
        session: s,
        window: window(),
        subject: ApprovalSubject::Batch {
            proposal: ProposalId(1),
        },
        outcome: Outcome::Approved,
        at_ms: 1,
    };
    assert_eq!(
        a.record_resolution(&resolution),
        Err(AssemblyError::UnknownRequest(RequestId(7)))
    );
    a.record_request(
        s,
        RequestId(7),
        ApprovalSubject::Batch {
            proposal: ProposalId(2),
        },
    )
    .unwrap();
    assert_eq!(
        a.record_resolution(&resolution),
        Err(AssemblyError::SubjectMismatch(RequestId(7)))
    );
    // 거부된 batch 승인 뒤의 적용 표시도 받지 않습니다.
    let mut denied = resolution;
    denied.subject = ApprovalSubject::Batch {
        proposal: ProposalId(2),
    };
    denied.outcome = Outcome::Denied(Denial::User);
    a.record_resolution(&denied).unwrap();
    assert_eq!(
        a.record_resolution(&denied),
        Err(AssemblyError::AlreadyResolved(RequestId(7)))
    );
    assert_eq!(
        a.record_batch(&batch(0, Some(2))),
        Err(AssemblyError::BatchNotApproved(ProposalId(2)))
    );
    // 받은 것만 표시됩니다.
    let opened = [(
        RequestId(7),
        ApprovalSubject::Batch {
            proposal: ProposalId(2),
        },
    )];
    assert!(a.verify(&opened, &[denied], &[]).is_empty());
    // 기록에서 빠진 효과는 불일치로 잡힙니다.
    assert_eq!(a.verify(&opened, &[], &[]).len(), 1);
    assert_eq!(a.verify(&[], &[denied], &[]).len(), 1);
    let missing = a.verify(&opened, &[denied], &[batch(0, Some(2))]);
    assert!(matches!(missing.as_slice(), [Mismatch::Missing(_)]));
}

#[test]
fn batch_order_is_enforced_and_debug_has_no_content() {
    let s = AgentSessionId(1);
    let mut a = ChatAssembly::new(s, SessionMode::ApplyInScope);
    let b = |seq| BatchRecord {
        session: s,
        batch_seq: seq,
        history_seq: u64::from(seq),
        proposal: None,
        changed_nodes: 0,
        changed_properties: 0,
        created_nodes: 0,
        deleted_nodes: 0,
    };
    a.record_batch(&b(0)).unwrap();
    a.record_batch(&b(2)).unwrap();
    assert_eq!(
        a.record_batch(&b(2)),
        Err(AssemblyError::BatchOutOfOrder { batch_seq: 2 })
    );
    a.push_chat(AgentChatItem::UserPrompt {
        text: DisplayText::new("비밀 문장"),
    });
    assert!(!format!("{a:?}").contains("비밀"));
}
