//! 앱이 시작한 에이전트 세션(`stellar-agent-host` AgentSession)과 창 에이전트 편집 세션의 연결 (XXIX.3.1 엔진 부분,
//! ADR-0124 결정 1).
//!
//! agent-host의 AgentSession 상태 기계는 lease·권한 수명과 종료 절차를 [`SessionEffect`] 목록으로 내지만, 문서·history에
//! 닿지 않으므로 실행은 runtime이 합니다. 이 module은 그 효과를 창의 세션 관리자(`AgentEditSessions`)·권한 저장소에
//! 실행합니다.
//!
//! - **id 대응**: agent-host `AgentSessionId(u64)`(프로세스·CLI 관리 handle)와 에이전트 편집 `AgentSessionId(StableId)`
//!   (권한 key·출처 기록)는 [`HostedSessions`] 한 표가 1:1로 잇습니다. 편집 쪽 id는 [`new_edit_session_id`]가 만듭니다.
//! - **시작과 첫 연결 승인**: [`AgentWindow::start_hosted`]가 시작 확인(`StartedSession`)을 받아 편집 세션을 등록하고,
//!   `FirstConnectionApproval`을 권한 저장소의 첫 연결 응답(승인)으로 기록합니다(ADR-0101, XXIX.3.5).
//! - **효과 실행**: [`AgentWindow::host_event`]가 event를 AgentSession에 넣고 효과를 실행합니다. 종료 효과 묶음(lease
//!   해제·권한 폐기·대기 요청 거부·제안 폐기·"중단됨" 확정)은 세션 관리자의 종료 절차 하나(`AgentWindow::end`)로,
//!   정상 완료 효과(Undo 항목 확정·lease 해제)는 `AgentWindow::complete` 하나로, 모드 낮춤의 권한 폐기는
//!   `PermissionStore::revoke`로 실행합니다. 효과에 실린 batch 수는 쓰지 않습니다. 확정할 batch는 출처 색인이 정합니다.
//! - 프로세스 종료·실행 흔적 정리·승인 화면 닫기·bridge 결합 해제·진단은 문서 밖 효과라 창의 대기열
//!   ([`AgentWindow::take_host_effects`])에 넘깁니다. 프로세스 관리자와 UI가 꺼내 실행합니다.
//! - 편집 쪽이 먼저 끝낸 세션(외부 변경 lease 겹침 등)은 AgentSession에 같은 이유의 event를 넣어 상태를 맞춥니다.
//!   이미 끝난 편집 세션에는 종료 절차를 다시 실행하지 않습니다.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use stellar_agent_edit::agent_history::{
    CompletedEdit, EndReason, SessionEndReport, SessionOrigin,
};
use stellar_agent_edit::permission::{AgentSessionId, PermissionKey};
use stellar_agent_edit::provenance::UserUiAction;
use stellar_agent_host::session::{
    AgentSession, AgentSessionId as HostSessionId, EndReason as HostEndReason, SessionEffect,
    SessionEvent, SessionMode, SessionState, TransitionError,
};
use stellar_agent_host::start::StartedSession;
use stellar_commands::id::CommandId;
use stellar_core_types::ids::StableId;

use crate::agent::project_key;
use crate::agent_edit::{AgentEditError, AgentWindow};

/// 앱 시작 세션의 종료 이유를 에이전트 편집 종료 이유로 바꿉니다. 진단 코드는 양쪽이 같습니다.
pub fn end_reason(reason: HostEndReason) -> EndReason {
    match reason {
        HostEndReason::UserStopped => EndReason::UserStopped,
        HostEndReason::ProcessExited => EndReason::ProcessExited,
        HostEndReason::ReconnectGraceExpired => EndReason::ReconnectGraceExpired,
        HostEndReason::TrustRevoked => EndReason::TrustRevoked,
        HostEndReason::LeaseConflict => EndReason::LeaseConflict,
        HostEndReason::UserReleasedLease => EndReason::UserReleasedLease,
        HostEndReason::UsageLimitReported => EndReason::UsageLimitReported,
        HostEndReason::AuthErrorReported => EndReason::AuthErrorReported,
    }
}

/// 편집 쪽이 먼저 끝낸 세션의 이유를 AgentSession event로 바꿉니다. 앱 시작 세션에 없는 이유(MCP 연결 종료·비정상
/// 종료)는 프로세스 종료로 알립니다.
pub fn end_event(reason: EndReason) -> SessionEvent {
    match reason {
        EndReason::UserStopped => SessionEvent::UserStop,
        EndReason::LeaseConflict => SessionEvent::LeaseConflict,
        EndReason::UserReleasedLease => SessionEvent::UserReleasedLease,
        EndReason::TrustRevoked => SessionEvent::TrustRevoked,
        EndReason::McpDisconnected
        | EndReason::ProcessExited
        | EndReason::AbnormalExit
        | EndReason::ReconnectGraceExpired
        | EndReason::UsageLimitReported
        | EndReason::AuthErrorReported => SessionEvent::ProcessExited,
    }
}

/// 새 에이전트 편집 세션 id(UUIDv7: unix ms 시각 + 프로세스 안 순번). 같은 프로세스 안에서 겹치지 않습니다.
pub fn new_edit_session_id() -> AgentSessionId {
    static ISSUED: AtomicU64 = AtomicU64::new(0);
    let issued = ISSUED.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    let unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    let rand_b = (u64::from(std::process::id()) << 32) | (issued & 0xFFFF_FFFF);
    AgentSessionId(StableId::new_v7(unix_ms, 0x0a6e, rand_b))
}

#[derive(Debug)]
struct Hosted {
    session: AgentSession,
    edit: AgentSessionId,
}

/// 창의 앱 시작 세션 표(agent-host id ↔ 편집 id, AgentSession 상태).
#[derive(Debug, Default)]
pub struct HostedSessions {
    by_host: BTreeMap<HostSessionId, Hosted>,
    by_edit: BTreeMap<AgentSessionId, HostSessionId>,
    effects: Vec<(HostSessionId, SessionEffect)>,
}

impl HostedSessions {
    /// agent-host id의 편집 세션 id.
    pub fn edit_of(&self, host: HostSessionId) -> Option<AgentSessionId> {
        self.by_host.get(&host).map(|h| h.edit)
    }

    /// 편집 세션 id의 agent-host id.
    pub fn host_of(&self, edit: AgentSessionId) -> Option<HostSessionId> {
        self.by_edit.get(&edit).copied()
    }

    /// AgentSession(읽기).
    pub fn session(&self, host: HostSessionId) -> Option<&AgentSession> {
        self.by_host.get(&host).map(|h| &h.session)
    }

    /// 편집 세션의 모드(앱 시작 세션만, 외부 클라이언트 세션은 `None`).
    pub fn mode_of(&self, edit: AgentSessionId) -> Option<SessionMode> {
        self.host_of(edit)
            .and_then(|host| self.session(host))
            .map(AgentSession::mode)
    }

    /// 앱 시작 세션 수.
    pub fn len(&self) -> usize {
        self.by_host.len()
    }

    /// 앱 시작 세션이 없는지.
    pub fn is_empty(&self) -> bool {
        self.by_host.is_empty()
    }
}

/// 연결 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    /// 시작 확인의 대상 프로젝트가 이 창 프로젝트가 아닙니다.
    ProjectMismatch,
    /// 시작 확인의 창이 이 창이 아닙니다.
    WindowMismatch,
    /// 이미 연결된 agent-host 세션입니다.
    DuplicateHost(HostSessionId),
    /// 연결되지 않은 agent-host 세션입니다.
    UnknownHost(HostSessionId),
    /// AgentSession이 event를 받지 않았습니다.
    Transition(TransitionError),
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkError::ProjectMismatch => {
                f.write_str("시작 확인의 프로젝트가 이 창 프로젝트가 아닙니다")
            }
            LinkError::WindowMismatch => f.write_str("시작 확인의 창이 이 창이 아닙니다"),
            LinkError::DuplicateHost(id) => {
                write!(f, "에이전트 세션 {}은 이미 연결되어 있습니다", id.0)
            }
            LinkError::UnknownHost(id) => {
                write!(f, "에이전트 세션 {}이 연결되어 있지 않습니다", id.0)
            }
            LinkError::Transition(e) => write!(f, "에이전트 세션 상태 전이 실패: {e:?}"),
        }
    }
}

/// 효과 실행 보고.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct HostEffectReport {
    /// 종료 절차를 실행했으면 그 보고서.
    pub ended: Option<Box<SessionEndReport>>,
    /// 정상 완료를 실행했으면 그 결과.
    pub completed: Option<CompletedEdit>,
    /// 모드 낮춤으로 버린 권한 확인 수.
    pub revoked: usize,
    /// 창 대기열로 넘긴 문서 밖 효과 수.
    pub forwarded: usize,
}

impl AgentWindow<'_> {
    /// 사용자가 확인한 앱 시작 세션을 이 창에 등록합니다(앱 사용자 UI 전용). 편집 세션을 시작하고 첫 연결 승인을
    /// 권한 저장소에 기록한 뒤 편집 세션 id를 돌려줍니다. 대상 프로젝트·창이 다르면 아무것도 바꾸지 않습니다.
    pub fn start_hosted(
        &mut self,
        started: StartedSession,
        name: &str,
        _ui: UserUiAction,
        now_ms: u64,
    ) -> Result<AgentSessionId, AgentEditError> {
        let StartedSession {
            session,
            connection,
        } = started;
        if connection.project != project_key(self.session.identity()) {
            return Err(AgentEditError::Link(LinkError::ProjectMismatch));
        }
        if connection.window != self.session.window() {
            return Err(AgentEditError::Link(LinkError::WindowMismatch));
        }
        let host = session.id();
        if self.agent.hosted.by_host.contains_key(&host) {
            return Err(AgentEditError::Link(LinkError::DuplicateHost(host)));
        }
        let edit = new_edit_session_id();
        self.start_session(edit, SessionOrigin::AppStarted, name, now_ms)?;
        let key = self.permission_key(edit);
        // 시작 확인이 곧 첫 연결 승인입니다(ADR-0101, XXIX.3.5). 방금 등록해 대기 중이므로 실패하지 않습니다.
        let _ = self.agent.permissions.respond_connection(&key, true);
        self.agent.hosted.by_edit.insert(edit, host);
        self.agent
            .hosted
            .by_host
            .insert(host, Hosted { session, edit });
        Ok(edit)
    }

    pub(crate) fn permission_key(&self, edit: AgentSessionId) -> PermissionKey {
        let identity = self.session.identity();
        PermissionKey::new(
            identity.project_id(),
            identity.canonical_target().to_path_buf(),
            edit,
        )
    }

    /// 앱 시작 세션에 event를 넣고 나온 효과를 실행합니다(모듈 문서). `entry_id`는 효과가 확정하는 history 항목 id입니다.
    pub fn host_event(
        &mut self,
        host: HostSessionId,
        event: SessionEvent,
        entry_id: CommandId,
        now_ms: u64,
    ) -> Result<HostEffectReport, AgentEditError> {
        let hosted = self
            .agent
            .hosted
            .by_host
            .get_mut(&host)
            .ok_or(AgentEditError::Link(LinkError::UnknownHost(host)))?;
        let effects = hosted
            .session
            .apply(event, now_ms)
            .map_err(|e| AgentEditError::Link(LinkError::Transition(e)))?;
        Ok(self.run_host_effects(host, effects, entry_id, now_ms))
    }

    /// 사용자가 앱 UI에서 앱 시작 세션의 모드를 바꿉니다(XXIX.3.3, 앱 사용자 UI 전용). 낮추면 상한 밖 권한을 버립니다.
    pub fn change_hosted_mode(
        &mut self,
        host: HostSessionId,
        mode: SessionMode,
        _ui: UserUiAction,
    ) -> Result<HostEffectReport, AgentEditError> {
        let hosted = self
            .agent
            .hosted
            .by_host
            .get_mut(&host)
            .ok_or(AgentEditError::Link(LinkError::UnknownHost(host)))?;
        let effects = hosted
            .session
            .change_mode_by_user(mode)
            .map_err(|e| AgentEditError::Link(LinkError::Transition(e)))?;
        Ok(self.run_host_effects(host, effects, CommandId::from_bytes([0; 16]), 0))
    }

    /// 편집 쪽이 끝낸 세션을 AgentSession에 알립니다(이미 끝났으면 아무것도 하지 않음).
    pub(crate) fn notify_host_ended(&mut self, report: &SessionEndReport, now_ms: u64) {
        let Some(host) = self.agent.hosted.host_of(report.session) else {
            return;
        };
        let _ = self.host_event(
            host,
            end_event(report.reason),
            CommandId::from_bytes([0; 16]),
            now_ms,
        );
    }

    /// 문서 밖 효과(프로세스 종료, 실행 흔적 정리, 승인 화면 닫기, bridge 결합 해제, 진단)를 꺼냅니다.
    pub fn take_host_effects(&mut self) -> Vec<(HostSessionId, SessionEffect)> {
        std::mem::take(&mut self.agent.hosted.effects)
    }

    fn run_host_effects(
        &mut self,
        host: HostSessionId,
        effects: Vec<SessionEffect>,
        entry_id: CommandId,
        now_ms: u64,
    ) -> HostEffectReport {
        let mut report = HostEffectReport::default();
        let Some(hosted) = self.agent.hosted.by_host.get(&host) else {
            return report;
        };
        let (edit, state) = (hosted.edit, hosted.session.state());
        let ends = effects
            .iter()
            .any(|e| matches!(e, SessionEffect::RevokeSessionPermissions));
        let completes = !ends
            && effects.iter().any(|e| {
                matches!(
                    e,
                    SessionEffect::ConfirmUndoEntry { .. } | SessionEffect::ReleaseLeases(_)
                )
            });
        if ends && self.agent.sessions.is_active(edit) {
            let reason = match state {
                SessionState::Ended { reason } | SessionState::Ending { reason } => {
                    end_reason(reason)
                }
                _ => EndReason::AbnormalExit,
            };
            if let Ok(ended) = self.end(edit, reason, entry_id, now_ms) {
                report.ended = Some(Box::new(ended));
            }
        }
        if completes && self.agent.sessions.lease(edit).is_some() {
            report.completed = self.complete(edit, entry_id).ok();
        }
        for effect in effects {
            match effect {
                SessionEffect::RevokePermissions(permissions) => {
                    let key = self.permission_key(edit);
                    report.revoked += self.agent.permissions.revoke(&key, &permissions);
                }
                SessionEffect::FinalizeInterruptedHistoryEntry { .. }
                | SessionEffect::ConfirmUndoEntry { .. }
                | SessionEffect::ReleaseLeases(_)
                | SessionEffect::RevokeSessionPermissions
                | SessionEffect::RejectPendingRequests(_)
                | SessionEffect::DiscardProposals(_) => {}
                SessionEffect::TerminateProcess
                | SessionEffect::CloseApprovalUi
                | SessionEffect::EndBridgeBinding
                | SessionEffect::CleanupExecutionTraces
                | SessionEffect::RecordDiagnostic(_) => {
                    report.forwarded += 1;
                    self.agent.hosted.effects.push((host, effect));
                }
            }
        }
        report
    }
}

#[cfg(test)]
#[path = "agent_session_tests.rs"]
mod tests;
