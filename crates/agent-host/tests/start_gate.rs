//! 내장 세션 시작 확인과 약관 고지 기록 (XXIX.3.5).
//!
//! 검증 기준: untrusted 시작 0, 확인 기록 존재, 고지 미확인 시 시작 0, 검토 결과 변경 후 재고지, 철회
//! fixture에서 lease 0·대기 승인 0·적용 변경 자동 삭제 0. 신뢰 판정과 프로젝트 identity 연결은
//! `stellar-runtime`의 `agent` 단위 test가 함께 봅니다.

mod support;

use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use stellar_agent_host::approval::{ProjectKey, WindowSessionId};
use stellar_agent_host::profile::{LaunchProfile, TermsResult, OFFICIAL_PROFILE_SOURCES};
use stellar_agent_host::session::*;
use stellar_agent_host::start::*;
use stellar_agent_protocol::core_types::ids::StableId;
use support::{double_profile, TempDir};

fn key() -> ProjectKey {
    ProjectKey::new(StableId::from_bytes([1; 16]), PathBuf::from("/tmp/p"))
}

fn window() -> WindowSessionId {
    WindowSessionId::from_stable_id(StableId::from_bytes([2; 16]))
}

fn prepare(
    profile: &LaunchProfile,
    notices: &TermsNoticeStore,
) -> Result<StartConfirmation, StartRefusal> {
    prepare_start(
        key(),
        window(),
        McpCapability::Allowed,
        profile,
        SessionMode::ReviewThenApply,
        notices,
    )
}

#[test]
fn untrusted_and_disabled_profiles_never_start() {
    let dir = TempDir::new("start-untrusted");
    let profile = double_profile("claude");
    let mut notices = TermsNoticeStore::open(dir.path()).unwrap();
    notices.acknowledge(&profile).unwrap();
    assert_eq!(
        prepare_start(
            key(),
            window(),
            McpCapability::Denied,
            &profile,
            SessionMode::ReadOnly,
            &notices
        ),
        Err(StartRefusal::Untrusted)
    );
    // 공식 profile은 약관 검토 전이라 비활성이며 시작하지 않습니다.
    for (_, text) in OFFICIAL_PROFILE_SOURCES {
        let official = LaunchProfile::parse(text).unwrap();
        assert!(!official.enabled);
        assert_eq!(
            prepare(&official, &notices),
            Err(StartRefusal::ProfileDisabled)
        );
    }
}

#[test]
fn confirmation_is_recorded_as_first_connection_approval() {
    let dir = TempDir::new("start-record");
    let profile = double_profile("claude");
    let mut notices = TermsNoticeStore::open(dir.path()).unwrap();
    let confirmation = prepare(&profile, &notices).unwrap();
    assert!(confirmation.notice_required, "처음 쓰는 CLI는 고지 필요");
    assert_eq!(confirmation.cli_display_name, "Claude Code");
    assert_eq!(confirmation.mode, SessionMode::ReviewThenApply);
    // 고지를 확인하지 않으면 시작하지 않습니다.
    assert_eq!(
        confirm_start(
            confirmation.clone(),
            McpCapability::Allowed,
            &profile,
            &notices,
            AgentSessionId(5),
            100,
            DEFAULT_RECONNECT_GRACE_MS
        )
        .unwrap_err(),
        StartRefusal::NoticeNotAcknowledged
    );
    notices.acknowledge(&profile).unwrap();
    let started = confirm_start(
        confirmation,
        McpCapability::Allowed,
        &profile,
        &notices,
        AgentSessionId(5),
        100,
        DEFAULT_RECONNECT_GRACE_MS,
    )
    .unwrap();
    assert_eq!(
        started.connection,
        FirstConnectionApproval {
            project: key(),
            window: window(),
            session: AgentSessionId(5),
            profile_id: profile.id.clone(),
            mode: SessionMode::ReviewThenApply,
            approved_at_ms: 100,
        }
    );
    assert_eq!(started.session.state(), SessionState::Starting);
    assert_eq!(started.session.permissions().count(), 0);
    assert_eq!(started.session.leases().count(), 0);
}

#[test]
fn profile_mismatch_is_refused() {
    let dir = TempDir::new("start-mismatch");
    let claude = double_profile("claude");
    let codex = double_profile("codex");
    let mut notices = TermsNoticeStore::open(dir.path()).unwrap();
    notices.acknowledge(&claude).unwrap();
    notices.acknowledge(&codex).unwrap();
    let confirmation = prepare(&claude, &notices).unwrap();
    assert_eq!(
        confirm_start(
            confirmation,
            McpCapability::Allowed,
            &codex,
            &notices,
            AgentSessionId(1),
            0,
            1
        )
        .unwrap_err(),
        StartRefusal::ProfileMismatch
    );
}

#[test]
fn notice_is_asked_again_after_review_changes() {
    let dir = TempDir::new("start-renotice");
    let mut profile = double_profile("claude");
    let mut notices = TermsNoticeStore::open(dir.path()).unwrap();
    assert!(notices.needs_notice(&profile));
    notices.acknowledge(&profile).unwrap();
    assert!(!notices.needs_notice(&profile));
    // 다시 열어도 기록이 남습니다(OS 사용자 데이터).
    let reopened = TermsNoticeStore::open(dir.path()).unwrap();
    assert!(!reopened.needs_notice(&profile));
    // 검토 일자가 바뀌면 다시 고지합니다.
    profile.review.terms_date = "2026-12-01".to_string();
    assert!(reopened.needs_notice(&profile));
    // 검토 결과가 바뀌어도 다시 고지합니다.
    let mut profile = double_profile("claude");
    profile.review.terms_result = TermsResult::Pending;
    assert!(reopened.needs_notice(&profile));
    // 다른 CLI는 따로 고지합니다.
    assert!(reopened.needs_notice(&double_profile("codex")));
}

#[test]
fn notice_record_is_private_minimal_and_fails_closed() {
    let dir = TempDir::new("start-notice-file");
    let profile = double_profile("claude");
    let mut notices = TermsNoticeStore::open(dir.path()).unwrap();
    assert!(
        !notices.file_path().exists(),
        "열기만으로 파일을 만들지 않음"
    );
    notices.acknowledge(&profile).unwrap();
    let file = notices.file_path();
    let text = std::fs::read_to_string(&file).unwrap();
    assert_eq!(
        text,
        format!(
            "{NOTICE_HEADER}\nclaude-code {} allowed\n",
            profile.review.terms_date
        )
    );
    assert!(!text.contains('/'), "경로 0");
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&file), 0o600);
    assert_eq!(mode(file.parent().unwrap()), 0o700);

    // 손상된 기록은 덮어쓰지 않고 오류로 닫습니다(시작 0).
    std::fs::write(&file, "garbage\n").unwrap();
    assert!(matches!(
        TermsNoticeStore::open(dir.path()),
        Err(NoticeError::Corrupt(_))
    ));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "garbage\n");
    std::fs::write(&file, format!("{NOTICE_HEADER}\nclaude-code 2026-11-23\n")).unwrap();
    assert!(TermsNoticeStore::open(dir.path()).is_err());

    // symlink는 따라가지 않습니다.
    std::fs::remove_file(&file).unwrap();
    let elsewhere = dir.join("elsewhere");
    std::fs::write(&elsewhere, format!("{NOTICE_HEADER}\n")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &file).unwrap();
    assert_eq!(
        TermsNoticeStore::open(dir.path()).unwrap_err(),
        NoticeError::Symlink
    );
}

#[test]
fn trust_revocation_stops_with_termination_procedure() {
    let mut s = AgentSession::start_by_user(UserStartRequest {
        session: AgentSessionId(2),
        mode: SessionMode::ApplyInScope,
        now_ms: 0,
        reconnect_grace_ms: DEFAULT_RECONNECT_GRACE_MS,
    });
    s.apply(SessionEvent::BridgeBound, 1).unwrap();
    s.apply(SessionEvent::LeaseGranted(LeaseId(1)), 1).unwrap();
    s.apply(SessionEvent::RequestOpened(RequestId(3)), 1)
        .unwrap();
    s.apply(SessionEvent::ProposalQueued(ProposalId(4)), 1)
        .unwrap();
    s.apply(SessionEvent::TransactionStarted, 1).unwrap();
    s.apply(
        SessionEvent::TransactionSettled(Some(AppliedBatch {
            batch_seq: 0,
            history_seq: 1,
        })),
        1,
    )
    .unwrap();
    assert!(on_trust_change(&mut s, McpCapability::Allowed, 2)
        .unwrap()
        .is_empty());
    let effects = on_trust_change(&mut s, McpCapability::Denied, 3).unwrap();
    assert_eq!(
        s.state(),
        SessionState::Ended {
            reason: EndReason::TrustRevoked
        }
    );
    assert_eq!(s.leases().count(), 0);
    assert_eq!(s.pending_requests(), 0);
    assert_eq!(s.pending_proposals(), 0);
    assert_eq!(s.applied_batches().len(), 1, "적용 변경 자동 삭제 0");
    let kinds: Vec<&str> = effects.iter().map(SessionEffect::kind).collect();
    for want in [
        "terminate",
        "interrupted-entry",
        "release-leases",
        "reject-requests",
    ] {
        assert!(kinds.contains(&want), "{kinds:?}");
    }
}
