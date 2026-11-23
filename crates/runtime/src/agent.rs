//! 내장 에이전트 세션의 앱 runtime 연결 (XXIX.3.2·XXIX.3.5, ADR-0005 §4.5·§8, ADR-0101 결정 3·5).
//!
//! `stellar-agent-host`는 project crate에 도달하지 않으므로(ADR-0005 §8), 프로젝트 identity와 신뢰 판정은
//! 이 모듈이 넘깁니다.
//!
//! - [`project_key`]: `ProjectIdentity`(IV.2.3)의 project id와 canonical target으로 승인 요청 저장 key를
//!   만듭니다. 신뢰 저장과 같은 canonical target이라 symlink로 연 같은 프로젝트는 같은 key, 다른 worktree는
//!   다른 key입니다.
//! - [`mcp_capability`]: XVI.7.1 capability 표(`ProjectCapability::Mcp`)의 판정 결과를 그대로 넘깁니다.
//!   이 모듈이 신뢰를 따로 판정하지 않습니다.
//! - [`prepare_agent_start`]·[`confirm_agent_start`]·[`on_project_trust_change`]: 위 둘을 agent-host 시작
//!   확인과 신뢰 철회 처리에 연결합니다.
//!
//! 신뢰 상태, 에이전트 모드, 권한, 약관 고지 기록은 모두 프로젝트 폴더와 Git 밖(앱 메모리·OS 사용자
//! 데이터)에 있으므로 프로젝트 파일을 고쳐 바꿀 수 없습니다(ADR-0005 §3·Security, 아래 tamper test).

use stellar_agent_host::approval::{ProjectKey, WindowSessionId};
use stellar_agent_host::profile::LaunchProfile;
use stellar_agent_host::session::{
    AgentSession, AgentSessionId, SessionEffect, SessionMode, TransitionError,
};
use stellar_agent_host::start::{
    confirm_start, on_trust_change, prepare_start, McpCapability, StartConfirmation, StartRefusal,
    StartedSession, TermsNoticeStore,
};
use stellar_project::capability::{capability_level, CapabilityLevel, ProjectCapability};
use stellar_project::identity::ProjectIdentity;
use stellar_project::trust::TrustStatus;

/// 승인 요청 저장 key(project id + canonical target).
pub fn project_key(identity: &ProjectIdentity) -> ProjectKey {
    ProjectKey::new(
        identity.project_id(),
        identity.canonical_target().to_path_buf(),
    )
}

/// XVI.7.1 capability 표의 MCP·에이전트 판정. `Allowed`만 허용이고 제한·금지는 모두 금지입니다.
pub fn mcp_capability(trust: TrustStatus) -> McpCapability {
    match capability_level(ProjectCapability::Mcp, trust) {
        CapabilityLevel::Allowed => McpCapability::Allowed,
        CapabilityLevel::Restricted | CapabilityLevel::Denied => McpCapability::Denied,
    }
}

/// 시작 확인 데이터를 만듭니다(신뢰되지 않은 프로젝트는 거부).
pub fn prepare_agent_start(
    identity: &ProjectIdentity,
    trust: TrustStatus,
    window: WindowSessionId,
    profile: &LaunchProfile,
    mode: SessionMode,
    notices: &TermsNoticeStore,
) -> Result<StartConfirmation, StartRefusal> {
    prepare_start(
        project_key(identity),
        window,
        mcp_capability(trust),
        profile,
        mode,
        notices,
    )
}

/// 사용자가 시작을 확인했습니다. 그 시점의 신뢰 상태로 다시 확인합니다.
pub fn confirm_agent_start(
    confirmation: StartConfirmation,
    trust: TrustStatus,
    profile: &LaunchProfile,
    notices: &TermsNoticeStore,
    session: AgentSessionId,
    now_ms: u64,
    reconnect_grace_ms: u64,
) -> Result<StartedSession, StartRefusal> {
    confirm_start(
        confirmation,
        mcp_capability(trust),
        profile,
        notices,
        session,
        now_ms,
        reconnect_grace_ms,
    )
}

/// 세션 중 프로젝트 신뢰 상태가 바뀌었습니다. 철회면 즉시 XIX.6.4 종료 절차를 시작합니다.
pub fn on_project_trust_change(
    session: &mut AgentSession,
    trust: TrustStatus,
    now_ms: u64,
) -> Result<Vec<SessionEffect>, TransitionError> {
    on_trust_change(session, mcp_capability(trust), now_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use stellar_agent_host::approval::{
        ApprovalRequest, ApprovalStore, ApprovalSubject, Denial, NoEvidence, Outcome,
    };
    use stellar_agent_host::session::{LeaseId, SessionEvent, SessionState};
    use stellar_core_types::ids::StableId;
    use stellar_project::identity::parse_project_id;
    use stellar_project::probe::probe_project;
    use stellar_project::scaffold::write_default_scaffold;
    use stellar_project::trust_store::FileTrustStore;

    const PROJECT_ID: &str = "0195c530-3acd-7bc1-8414-c213f61cb965";

    struct Dir(PathBuf);
    impl Dir {
        fn new(tag: &str) -> Dir {
            static N: AtomicU64 = AtomicU64::new(0);
            let base = std::fs::canonicalize(std::env::temp_dir()).unwrap();
            let dir = base.join(format!(
                "stellar-runtime-agent-{tag}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Dir(dir)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn profile() -> LaunchProfile {
        let path = format!(
            "{}/../agent-host/tests/fixtures/profiles/test-double-claude.json",
            env!("CARGO_MANIFEST_DIR")
        );
        LaunchProfile::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn project(root: &Path) -> ProjectIdentity {
        let project = root.join("project");
        std::fs::create_dir_all(&project).unwrap();
        write_default_scaffold(&project, "에이전트", PROJECT_ID).unwrap();
        ProjectIdentity::new(parse_project_id(PROJECT_ID).unwrap(), &project).unwrap()
    }

    fn window() -> WindowSessionId {
        WindowSessionId::from_stable_id(StableId::new_v7(1, 2, 3))
    }

    fn acknowledged(data: &Path) -> TermsNoticeStore {
        let mut notices = TermsNoticeStore::open(data).unwrap();
        notices.acknowledge(&profile()).unwrap();
        notices
    }

    #[test]
    fn capability_follows_matrix() {
        assert_eq!(mcp_capability(TrustStatus::Trusted), McpCapability::Allowed);
        assert_eq!(
            mcp_capability(TrustStatus::NotTrusted),
            McpCapability::Denied
        );
    }

    #[test]
    fn untrusted_project_never_starts() {
        // 검증 기준(XXIX.3.5): untrusted 시작 0.
        let dir = Dir::new("untrusted");
        let identity = project(&dir.0);
        let data = dir.0.join("data");
        let notices = acknowledged(&data);
        let trust = FileTrustStore::open(&data).unwrap().status(&identity);
        assert_eq!(trust, TrustStatus::NotTrusted);
        let refused = prepare_agent_start(
            &identity,
            trust,
            window(),
            &profile(),
            SessionMode::default(),
            &notices,
        );
        assert_eq!(refused, Err(StartRefusal::Untrusted));

        // 확인 데이터를 만든 뒤 신뢰가 철회되면 확인 단계에서도 시작하지 않습니다.
        let confirmation = prepare_agent_start(
            &identity,
            TrustStatus::Trusted,
            window(),
            &profile(),
            SessionMode::default(),
            &notices,
        )
        .unwrap();
        let refused = confirm_agent_start(
            confirmation,
            TrustStatus::NotTrusted,
            &profile(),
            &notices,
            AgentSessionId(1),
            0,
            1_000,
        );
        assert_eq!(refused.unwrap_err(), StartRefusal::Untrusted);
    }

    #[test]
    fn trusted_start_records_first_connection_and_revocation_stops() {
        // 검증 기준(XXIX.3.5): 확인 기록 존재, 철회 fixture에서 lease 0·대기 승인 0·적용 변경 자동 삭제 0.
        let dir = Dir::new("trusted");
        let identity = project(&dir.0);
        let data = dir.0.join("data");
        let mut trust_store = FileTrustStore::open(&data).unwrap();
        trust_store.grant(&identity).unwrap();
        let notices = acknowledged(&data);
        let trust = trust_store.status(&identity);
        let confirmation = prepare_agent_start(
            &identity,
            trust,
            window(),
            &profile(),
            SessionMode::ApplyInScope,
            &notices,
        )
        .unwrap();
        assert!(!confirmation.notice_required);
        let StartedSession {
            mut session,
            connection,
        } = confirm_agent_start(
            confirmation,
            trust,
            &profile(),
            &notices,
            AgentSessionId(7),
            10,
            1_000,
        )
        .unwrap();
        assert_eq!(connection.project, project_key(&identity));
        assert_eq!(connection.session, AgentSessionId(7));
        assert_eq!(connection.mode, SessionMode::ApplyInScope);
        assert_eq!(connection.approved_at_ms, 10);
        assert_eq!(session.permissions().count(), 0, "확인은 권한을 주지 않음");

        let key = project_key(&identity);
        let mut approvals: ApprovalStore<NoEvidence> = ApprovalStore::new();
        session.apply(SessionEvent::BridgeBound, 11).unwrap();
        session
            .apply(SessionEvent::LeaseGranted(LeaseId(1)), 11)
            .unwrap();
        let request = approvals
            .open(
                key.clone(),
                ApprovalRequest {
                    session: AgentSessionId(7),
                    window: window(),
                    subject: ApprovalSubject::Scope {
                        targets: 1,
                        externals: 0,
                    },
                    evidence: NoEvidence,
                    deadline_ms: 10_000,
                },
                11,
            )
            .unwrap();
        session
            .apply(SessionEvent::RequestOpened(request), 11)
            .unwrap();
        session.apply(SessionEvent::TransactionStarted, 12).unwrap();
        session
            .apply(
                SessionEvent::TransactionSettled(Some(stellar_agent_host::session::AppliedBatch {
                    batch_seq: 0,
                    history_seq: 3,
                })),
                12,
            )
            .unwrap();

        // 신뢰 철회.
        trust_store.revoke(&identity).unwrap();
        let effects =
            on_project_trust_change(&mut session, trust_store.status(&identity), 20).unwrap();
        let resolved = approvals.apply_session_effects(AgentSessionId(7), &effects, 20);
        assert!(matches!(session.state(), SessionState::Ended { .. }));
        assert_eq!(session.leases().count(), 0);
        assert_eq!(session.pending_requests(), 0);
        assert_eq!(approvals.pending(&key).count(), 0);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].outcome, Outcome::Denied(Denial::SessionEnded));
        assert_eq!(session.applied_batches().len(), 1, "적용 변경 자동 삭제 0");
        let kinds: Vec<&str> = effects.iter().map(SessionEffect::kind).collect();
        assert!(kinds.contains(&"interrupted-entry"), "{kinds:?}");
    }

    #[test]
    fn notice_is_required_before_first_use() {
        // 검증 기준(XXIX.3.5): 고지 미확인 시 시작 0.
        let dir = Dir::new("notice");
        let identity = project(&dir.0);
        let data = dir.0.join("data");
        let notices = TermsNoticeStore::open(&data).unwrap();
        let confirmation = prepare_agent_start(
            &identity,
            TrustStatus::Trusted,
            window(),
            &profile(),
            SessionMode::default(),
            &notices,
        )
        .unwrap();
        assert!(confirmation.notice_required);
        let refused = confirm_agent_start(
            confirmation,
            TrustStatus::Trusted,
            &profile(),
            &notices,
            AgentSessionId(1),
            0,
            1_000,
        );
        assert_eq!(refused.unwrap_err(), StartRefusal::NoticeNotAcknowledged);
    }

    #[test]
    fn project_file_tampering_changes_no_trust_mode_or_notice_state() {
        // 검증 기준(XXIX.6.5): 프로젝트 파일 변조 fixture에서 신뢰·권한·모드·고지 상태 변경 0.
        let dir = Dir::new("tamper");
        let identity = project(&dir.0);
        let data = dir.0.join("data");
        let notices = TermsNoticeStore::open(&data).unwrap();
        let notice_before = std::fs::read(notices.file_path()).ok();

        // 프로젝트 안에 신뢰·모드·권한·출처를 흉내 내는 내용을 씁니다.
        let root = identity.root_path().to_path_buf();
        let mut manifest = std::fs::read_to_string(root.join("stellar.yaml")).unwrap();
        manifest.push_str(
            "trust: trusted\nagent:\n  mode: apply-in-scope\n  permissions: [apply, terminal, git-push]\n  terms_acknowledged: true\n",
        );
        std::fs::write(root.join("stellar.yaml"), manifest).unwrap();
        std::fs::create_dir_all(root.join(".stellar")).unwrap();
        std::fs::write(
            root.join(".stellar/agent-state.yaml"),
            "mode: apply-in-scope\nprovenance: cleared\ntrust: trusted\n",
        )
        .unwrap();
        // 매니페스트 loader는 모르는 key를 받아들이든 거부하든 신뢰·모드 상태를 만들지 않습니다.
        let _ = probe_project(&root);

        let trust = FileTrustStore::open(&data).unwrap().status(&identity);
        assert_eq!(trust, TrustStatus::NotTrusted, "신뢰 변경 0");
        assert_eq!(
            prepare_agent_start(
                &identity,
                trust,
                window(),
                &profile(),
                SessionMode::ReadOnly,
                &notices
            ),
            Err(StartRefusal::Untrusted)
        );
        let reopened = TermsNoticeStore::open(&data).unwrap();
        assert!(reopened.needs_notice(&profile()), "고지 기록 변경 0");
        assert_eq!(std::fs::read(reopened.file_path()).ok(), notice_before);

        // 사용자가 신뢰한 뒤에도 모드는 사용자가 고른 값이고 권한은 0에서 시작합니다.
        let confirmation = prepare_agent_start(
            &identity,
            TrustStatus::Trusted,
            window(),
            &profile(),
            SessionMode::ReadOnly,
            &acknowledged(&data),
        )
        .unwrap();
        assert_eq!(confirmation.mode, SessionMode::ReadOnly);
        let started = confirm_agent_start(
            confirmation,
            TrustStatus::Trusted,
            &profile(),
            &TermsNoticeStore::open(&data).unwrap(),
            AgentSessionId(2),
            0,
            1_000,
        )
        .unwrap();
        assert_eq!(started.session.mode(), SessionMode::ReadOnly);
        assert_eq!(started.session.permissions().count(), 0);
    }
}
