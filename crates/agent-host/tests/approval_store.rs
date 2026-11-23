//! 승인 요청 저장 (XXIX.3.2).
//!
//! 검증 기준: 중복 응답 무시, 창 닫힘·기한 초과·세션 종료 거부 fixture, 동시 요청 순서 유지. 근거 갖춤 검사와
//! 프로젝트 identity 분리도 함께 봅니다.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use stellar_agent_host::approval::*;
use stellar_agent_host::session::*;
use stellar_agent_protocol::core_types::ids::StableId;
use stellar_agent_protocol::permission::Permission;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Evidence(EvidenceParts);

impl ApprovalEvidence for Evidence {
    fn parts(&self) -> EvidenceParts {
        self.0
    }
}

const FULL: Evidence = Evidence(EvidenceParts {
    semantic_diff: true,
    validation: true,
    render_preview: true,
});

fn key(n: u8) -> ProjectKey {
    ProjectKey::new(
        StableId::from_bytes([n; 16]),
        PathBuf::from(format!("/tmp/project-{n}")),
    )
}

fn window(n: u8) -> WindowSessionId {
    WindowSessionId::from_stable_id(StableId::from_bytes([0x40 + n; 16]))
}

fn request(
    session: u64,
    win: u8,
    subject: ApprovalSubject,
    deadline: u64,
) -> ApprovalRequest<Evidence> {
    ApprovalRequest {
        session: AgentSessionId(session),
        window: window(win),
        subject,
        evidence: FULL,
        deadline_ms: deadline,
    }
}

fn batch(p: u64) -> ApprovalSubject {
    ApprovalSubject::Batch {
        proposal: ProposalId(p),
    }
}

#[test]
fn first_answer_wins_and_duplicates_are_ignored() {
    let mut store = ApprovalStore::new();
    let id = store.open(key(1), request(1, 1, batch(1), 100), 0).unwrap();
    let first = store.answer(&key(1), id, UserAnswer::Approve, 10).unwrap();
    let Answered::Resolved(resolution) = first else {
        panic!("{first:?}");
    };
    assert_eq!(resolution.outcome, Outcome::Approved);
    assert_eq!(resolution.at_ms, 10);
    for answer in [UserAnswer::Deny, UserAnswer::Approve] {
        assert_eq!(
            store.answer(&key(1), id, answer, 11).unwrap(),
            Answered::Ignored {
                prior: Outcome::Approved
            }
        );
    }
    // 만료·창 닫힘·세션 종료도 확정된 결과를 바꾸지 않습니다.
    assert!(store.expire(1_000).is_empty());
    assert!(store.close_window(window(1), 1_000).is_empty());
    assert!(store.end_session(AgentSessionId(1), 1_000).is_empty());
    assert_eq!(
        store.answer(&key(1), id, UserAnswer::Deny, 1_001).unwrap(),
        Answered::Ignored {
            prior: Outcome::Approved
        }
    );
}

#[test]
fn deadline_window_close_and_session_end_deny_by_default() {
    let mut store = ApprovalStore::new();
    let expiring = store.open(key(1), request(1, 1, batch(1), 50), 0).unwrap();
    let late = store.open(key(1), request(1, 1, batch(2), 60), 0).unwrap();
    let in_window_2 = store
        .open(
            key(1),
            request(
                2,
                2,
                ApprovalSubject::Permission(Permission::GitCommit),
                1_000,
            ),
            0,
        )
        .unwrap();
    let in_session_3 = store
        .open(
            key(2),
            request(
                3,
                3,
                ApprovalSubject::Scope {
                    targets: 2,
                    externals: 1,
                },
                1_000,
            ),
            0,
        )
        .unwrap();

    // 기한 초과.
    let expired = store.expire(50);
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].id, expiring);
    assert_eq!(expired[0].outcome, Outcome::Denied(Denial::Expired));
    // 기한 뒤 도착한 승인은 승인하지 않습니다.
    assert_eq!(
        store
            .answer(&key(1), late, UserAnswer::Approve, 60)
            .unwrap(),
        Answered::Resolved(Resolution {
            id: late,
            session: AgentSessionId(1),
            window: window(1),
            subject: batch(2),
            outcome: Outcome::Denied(Denial::Expired),
            at_ms: 60,
        })
    );

    // 창 닫힘.
    let closed = store.close_window(window(2), 70);
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].id, in_window_2);
    assert_eq!(closed[0].outcome, Outcome::Denied(Denial::WindowClosed));
    assert_eq!(
        store.open(key(1), request(2, 2, batch(9), 1_000), 71),
        Err(ApprovalError::WindowClosed)
    );

    // 세션 종료(상태 기계 효과로).
    let mut session = AgentSession::start_by_user(UserStartRequest {
        session: AgentSessionId(3),
        mode: SessionMode::ReviewThenApply,
        now_ms: 0,
        reconnect_grace_ms: DEFAULT_RECONNECT_GRACE_MS,
    });
    session.apply(SessionEvent::BridgeBound, 1).unwrap();
    session
        .apply(SessionEvent::RequestOpened(in_session_3), 1)
        .unwrap();
    let effects = session.apply(SessionEvent::UserStop, 80).unwrap();
    let ended = store.apply_session_effects(AgentSessionId(3), &effects, 80);
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].id, in_session_3);
    assert_eq!(ended[0].outcome, Outcome::Denied(Denial::SessionEnded));
    assert!(effects.contains(&SessionEffect::RejectPendingRequests(vec![in_session_3])));
    assert_eq!(
        store.open(key(2), request(3, 4, batch(10), 1_000), 81),
        Err(ApprovalError::SessionEnded)
    );
    assert_eq!(store.pending_len(), 0);
    // 기본 거부로 닫힌 요청에 온 늦은 응답은 무시됩니다.
    assert_eq!(
        store
            .answer(&key(2), in_session_3, UserAnswer::Approve, 90)
            .unwrap(),
        Answered::Ignored {
            prior: Outcome::Denied(Denial::SessionEnded)
        }
    );
}

#[test]
fn deadline_must_be_in_future_and_batch_needs_full_evidence() {
    let mut store = ApprovalStore::new();
    assert_eq!(
        store.open(key(1), request(1, 1, batch(1), 10), 10),
        Err(ApprovalError::DeadlineNotInFuture)
    );
    let partial = Evidence(EvidenceParts {
        semantic_diff: true,
        validation: true,
        render_preview: false,
    });
    let mut req = request(1, 1, batch(1), 100);
    req.evidence = partial;
    assert_eq!(
        store.open(key(1), req, 0),
        Err(ApprovalError::MissingEvidence(partial.0))
    );
    // 범위·권한 확인은 근거 없이 엽니다.
    let mut req = request(
        1,
        1,
        ApprovalSubject::Scope {
            targets: 1,
            externals: 0,
        },
        100,
    );
    req.evidence = Evidence(EvidenceParts::default());
    assert!(store.open(key(1), req, 0).is_ok());
}

#[test]
fn projects_are_separate_and_answers_need_the_right_project() {
    let mut store = ApprovalStore::new();
    let id = store.open(key(1), request(1, 1, batch(1), 100), 0).unwrap();
    assert_eq!(store.pending(&key(2)).count(), 0);
    assert_eq!(
        store.answer(&key(2), id, UserAnswer::Approve, 1),
        Err(ApprovalError::UnknownRequest(id))
    );
    assert_eq!(store.pending(&key(1)).count(), 1);
    assert_eq!(store.pending_for_session(AgentSessionId(1)), 1);
}

#[test]
fn concurrent_requests_keep_open_order() {
    // 여러 스레드(여러 세션)가 같은 프로젝트에 동시에 요청을 엽니다. 대기 목록은 연 순서(id 순서) 그대로이고
    // 세션별 순서도 유지됩니다.
    let store = Arc::new(Mutex::new(ApprovalStore::new()));
    let mut handles = Vec::new();
    for t in 0..8u64 {
        let store = store.clone();
        handles.push(std::thread::spawn(move || {
            let mut ids = Vec::new();
            for n in 0..50u64 {
                let id = store
                    .lock()
                    .unwrap()
                    .open(key(1), request(t, 1, batch(t * 100 + n), 10_000), 0)
                    .unwrap();
                ids.push(id);
            }
            ids
        }));
    }
    let per_thread: Vec<Vec<RequestId>> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let store = store.lock().unwrap();
    let listed: Vec<(RequestId, ApprovalSubject)> = store
        .pending(&key(1))
        .map(|(id, r)| (id, r.subject))
        .collect();
    assert_eq!(listed.len(), 400);
    assert!(listed.windows(2).all(|w| w[0].0 < w[1].0), "id 순서");
    for (t, ids) in per_thread.iter().enumerate() {
        let order: Vec<RequestId> = listed
            .iter()
            .filter(|(_, s)| matches!(s, ApprovalSubject::Batch { proposal } if proposal.0 / 100 == t as u64))
            .map(|(id, _)| *id)
            .collect();
        assert_eq!(&order, ids, "스레드 {t}의 순서");
    }
}

#[test]
fn bulk_denials_are_reported_in_open_order() {
    let mut store = ApprovalStore::new();
    let mut opened = Vec::new();
    for n in 0..5u8 {
        // 서로 다른 프로젝트에 번갈아 엽니다.
        opened.push(
            store
                .open(key(n % 2), request(9, 1, batch(u64::from(n)), 100), 0)
                .unwrap(),
        );
    }
    let ended = store.end_session(AgentSessionId(9), 5);
    let ids: Vec<RequestId> = ended.iter().map(|r| r.id).collect();
    assert_eq!(ids, opened);
}
