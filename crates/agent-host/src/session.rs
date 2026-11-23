//! AgentSession 상태 기계 (XXIX.2.8·XXIX.2.13, ADR-0005 §4.2·§4.4·§4.5, 아키텍처 31.5, XIX.6.4, ADR-0092 결정 10·11).
//!
//! 앱이 시작한 세션의 lease와 권한을 소유하고, 끝날 때 XIX.6.4 종료 절차를 [`SessionEffect`] 목록으로
//! 냅니다. 이 crate는 history·commands·document에 도달하지 않으므로 효과의 실행(중단됨 history entry 확정,
//! lease 해제, 승인 화면 닫기)은 앱 runtime이 합니다(XXIX.3.1). 상태 기계는 시각을 입력(`now_ms`, 단조
//! 시각)으로 받아 결정적입니다.
//!
//! ## 상태
//!
//! | 상태 | 뜻 |
//! |---|---|
//! | `Starting` | 사용자가 시작함. CLI 프로세스 실행, bridge 결합 전 |
//! | `Running` | 결합된 bridge가 연결됨 |
//! | `Reconnecting { deadline_ms }` | bridge 연결이 끊김. 유예 안에 다시 결합되면 같은 세션으로 돌아감. lease·권한 유지 |
//! | `Ending { reason }` | 끝내는 중. 적용 중인 CommandTransaction이 끝나기를 기다림(VII.1.6 원자성) |
//! | `Ended { reason }` | 끝남. 다시 활성 상태로 돌아가지 않음 |
//!
//! ## 전이(표에 없는 조합은 [`TransitionError`])
//!
//! | 현재 | event | 다음 | 효과 |
//! |---|---|---|---|
//! | Starting | BridgeBound | Running | - |
//! | Running | BridgeDisconnected | Reconnecting(now + 유예) | - |
//! | Reconnecting | BridgeBound | Running | - (같은 세션, lease 유지) |
//! | Reconnecting | Tick(now ≥ deadline) | 종료 절차(ReconnectGraceExpired) | 종료 효과 |
//! | Starting·Running·Reconnecting | Tick(그 밖) | 그대로 | - |
//! | Running | LeaseGranted·PermissionGranted·RequestOpened·ProposalQueued·TransactionStarted | Running | - (권한은 모드 상한 안만) |
//! | 활성·Ending | RequestAnswered·ProposalResolved | 그대로 | - |
//! | Running·Reconnecting·Ending | TransactionSettled | 그대로 / Ending이면 종료 절차 마무리 | 종료 효과 |
//! | Running | EditCompleted | Running | ConfirmUndoEntry, ReleaseLeases |
//! | 활성 | UserStop·ProcessExited·TrustRevoked·LeaseConflict·UserReleasedLease·Output(Cli) | 종료 절차 | 종료 효과 |
//! | 활성·Ending | Output(DecodeFailed) | 그대로 | - (권한·lease·문서 무변경) |
//! | Ending | 종료 trigger | Ending | - (이미 끝내는 중) |
//! | Ended | 모든 event | Ended | `TransitionError::Ended` |
//!
//! ## 종료 절차(XIX.6.4, 자동 rollback 없음)
//!
//! 1. 적용 중인 transaction이 있으면 `Ending`에서 `TransactionSettled`를 기다립니다(전부 적용 또는 전부 미적용).
//! 2. 확정되지 않은 적용 batch가 있으면 `FinalizeInterruptedHistoryEntry` 하나(origin AgentSessionId).
//!    적용 batch와 출처 기록은 지우지 않습니다([`AgentSession::applied_batches`] 유지).
//! 3. `ReleaseLeases`, `RevokeSessionPermissions`.
//! 4. `RejectPendingRequests`, `DiscardProposals`, `CloseApprovalUi`.
//! 5. `EndBridgeBinding`, `CleanupExecutionTraces`(CLI 프로세스 group·scratch 정리), 필요하면 `TerminateProcess`.
//! 6. `RecordDiagnostic`(session id, duration, error code만).
//!
//! ## 사용자 주도
//!
//! 세션은 [`AgentSession::start_by_user`]로만 만들고, 끝난 세션을 다시 시작·재개하거나 메시지를 다시
//! 보내는 수단이 없습니다. CLI가 보고한 사용 한도·인증 오류는 채팅 item으로 그대로 보이고(디코더) 세션을
//! 멈춥니다.

use std::collections::BTreeSet;
use std::fmt;

use stellar_agent_protocol::permission::Permission;

use crate::decoder::CliSignal;
use crate::output::OutputSignal;

/// 재연결 기본 유예(ms). 근거는 XXIX.2.8 보고서.
pub const DEFAULT_RECONNECT_GRACE_MS: u64 = 15_000;

/// 에이전트 세션 id(AgentSessionId).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AgentSessionId(pub u64);

/// 앱 lease handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LeaseId(pub u64);

/// 사용자 응답을 기다리는 승인 요청 handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestId(pub u64);

/// 적용 전 제안 handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProposalId(pub u64);

/// 세션 모드(권한 상한, ADR-0005 §4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionMode {
    /// 읽기 전용: Read.
    ReadOnly,
    /// 검토 후 적용(기본): Read, Propose.
    ReviewThenApply,
    /// 승인 범위 내 바로 적용: Read, Propose, Apply.
    ApplyInScope,
}

impl SessionMode {
    /// 권한 상한.
    pub fn cap(self) -> &'static [Permission] {
        match self {
            SessionMode::ReadOnly => &[Permission::Read],
            SessionMode::ReviewThenApply => &[Permission::Read, Permission::Propose],
            SessionMode::ApplyInScope => {
                &[Permission::Read, Permission::Propose, Permission::Apply]
            }
        }
    }
}

/// 세션이 끝난 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    /// 사용자 중지.
    UserStopped,
    /// CLI 프로세스 종료.
    ProcessExited,
    /// 재연결 유예 초과.
    ReconnectGraceExpired,
    /// 신뢰 철회.
    TrustRevoked,
    /// lease와 겹치는 외부 변경.
    LeaseConflict,
    /// 사용자의 lease 해제.
    UserReleasedLease,
    /// CLI가 보고한 사용 한도.
    UsageLimitReported,
    /// CLI가 보고한 인증 오류.
    AuthErrorReported,
}

impl EndReason {
    /// 진단용 고정 오류 코드.
    pub fn code(self) -> &'static str {
        match self {
            EndReason::UserStopped => "user-stopped",
            EndReason::ProcessExited => "process-exited",
            EndReason::ReconnectGraceExpired => "reconnect-grace-expired",
            EndReason::TrustRevoked => "trust-revoked",
            EndReason::LeaseConflict => "lease-conflict",
            EndReason::UserReleasedLease => "user-released-lease",
            EndReason::UsageLimitReported => "usage-limit",
            EndReason::AuthErrorReported => "auth-error",
        }
    }
}

/// 세션 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// 시작함(bridge 결합 전).
    Starting,
    /// 실행 중.
    Running,
    /// bridge 재연결 대기.
    Reconnecting {
        /// 유예 끝 시각(ms).
        deadline_ms: u64,
    },
    /// 끝내는 중(적용 중인 transaction 대기).
    Ending {
        /// 이유.
        reason: EndReason,
    },
    /// 끝남.
    Ended {
        /// 이유.
        reason: EndReason,
    },
}

/// 적용된 batch 하나의 출처 기록 참조(상세 출처는 창 메모리의 provenance가 가짐, XIX.6.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppliedBatch {
    /// 세션 안 batch 순번.
    pub batch_seq: u32,
    /// history sequence.
    pub history_seq: u64,
}

/// 상태 기계 입력.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEvent {
    /// 결합된 bridge가 연결됨(XIX.1.8).
    BridgeBound,
    /// bridge 연결이 끊김.
    BridgeDisconnected,
    /// 시각 진행(유예 확인).
    Tick,
    /// 앱이 lease를 줌.
    LeaseGranted(LeaseId),
    /// 앱이 권한을 승인함(XIX.3).
    PermissionGranted(Permission),
    /// 사용자 응답을 기다리는 요청이 열림.
    RequestOpened(RequestId),
    /// 요청에 사용자가 응답함.
    RequestAnswered(RequestId),
    /// 적용 전 제안이 생김.
    ProposalQueued(ProposalId),
    /// 제안이 적용·거부로 끝남.
    ProposalResolved(ProposalId),
    /// CommandTransaction 적용 시작.
    TransactionStarted,
    /// 적용 끝(`Some`이면 적용됨, `None`이면 미적용).
    TransactionSettled(Option<AppliedBatch>),
    /// CompleteAgentEdit 정상 완료.
    EditCompleted,
    /// 사용자 중지.
    UserStop,
    /// CLI 프로세스 종료.
    ProcessExited,
    /// 신뢰 철회.
    TrustRevoked,
    /// lease와 겹치는 외부 변경.
    LeaseConflict,
    /// 사용자의 lease 해제.
    UserReleasedLease,
    /// 출력 신호([`crate::output`]).
    Output(OutputSignal),
}

/// 진단 기록(session id, duration, error code만).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionDiagnostic {
    /// 세션 id.
    pub session: AgentSessionId,
    /// 지속 시간(ms).
    pub duration_ms: u64,
    /// 오류 코드.
    pub error_code: &'static str,
}

impl fmt::Display for SessionDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "agent-session={} duration_ms={} error={}",
            self.session.0, self.duration_ms, self.error_code
        )
    }
}

/// 앱 runtime이 실행할 효과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEffect {
    /// CLI process group 종료(SIGTERM → 유예 → SIGKILL).
    TerminateProcess,
    /// 확정되지 않은 적용 batch를 "중단됨" history entry 하나로 확정(origin AgentSessionId).
    FinalizeInterruptedHistoryEntry {
        /// 세션.
        session: AgentSessionId,
        /// 묶을 batch 수.
        batches: u32,
    },
    /// 정상 완료: 승인된 batch를 Undo entry 하나로 확정.
    ConfirmUndoEntry {
        /// 세션.
        session: AgentSessionId,
        /// 묶을 batch 수.
        batches: u32,
    },
    /// lease 해제.
    ReleaseLeases(Vec<LeaseId>),
    /// 세션 권한과 세션 한정 승인 폐기.
    RevokeSessionPermissions,
    /// 응답 대기 요청을 거부로 닫음.
    RejectPendingRequests(Vec<RequestId>),
    /// 적용 전 제안 폐기.
    DiscardProposals(Vec<ProposalId>),
    /// 열린 승인 화면 닫기.
    CloseApprovalUi,
    /// bridge 결합 token 무효화(XIX.1.8).
    EndBridgeBinding,
    /// 실행 흔적 정리(ADR-0005 §4.3 불변식 2).
    CleanupExecutionTraces,
    /// 진단 기록.
    RecordDiagnostic(SessionDiagnostic),
}

/// 허용되지 않는 전이.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionError {
    /// 끝난 세션(다시 시작·재개하지 않음).
    Ended,
    /// 이 상태에서 받을 수 없는 event.
    Invalid {
        /// 상태 이름.
        state: &'static str,
        /// event 이름.
        event: &'static str,
    },
    /// 모드 상한 밖 권한.
    AboveModeCap(Permission),
}

/// 사용자가 명시적으로 세션 시작을 확인한 요청.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserStartRequest {
    /// 새 세션 id.
    pub session: AgentSessionId,
    /// 사용자가 고른 모드.
    pub mode: SessionMode,
    /// 시작 시각(ms).
    pub now_ms: u64,
    /// 재연결 유예(ms).
    pub reconnect_grace_ms: u64,
}

/// AgentSession.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSession {
    id: AgentSessionId,
    mode: SessionMode,
    started_ms: u64,
    grace_ms: u64,
    state: SessionState,
    leases: BTreeSet<LeaseId>,
    permissions: BTreeSet<Permission>,
    requests: BTreeSet<RequestId>,
    proposals: BTreeSet<ProposalId>,
    in_flight: bool,
    applied: Vec<AppliedBatch>,
    unconfirmed: u32,
    decode_failed: bool,
}

fn state_name(state: SessionState) -> &'static str {
    match state {
        SessionState::Starting => "starting",
        SessionState::Running => "running",
        SessionState::Reconnecting { .. } => "reconnecting",
        SessionState::Ending { .. } => "ending",
        SessionState::Ended { .. } => "ended",
    }
}

fn event_name(event: &SessionEvent) -> &'static str {
    match event {
        SessionEvent::BridgeBound => "bridge-bound",
        SessionEvent::BridgeDisconnected => "bridge-disconnected",
        SessionEvent::Tick => "tick",
        SessionEvent::LeaseGranted(_) => "lease-granted",
        SessionEvent::PermissionGranted(_) => "permission-granted",
        SessionEvent::RequestOpened(_) => "request-opened",
        SessionEvent::RequestAnswered(_) => "request-answered",
        SessionEvent::ProposalQueued(_) => "proposal-queued",
        SessionEvent::ProposalResolved(_) => "proposal-resolved",
        SessionEvent::TransactionStarted => "transaction-started",
        SessionEvent::TransactionSettled(_) => "transaction-settled",
        SessionEvent::EditCompleted => "edit-completed",
        SessionEvent::UserStop => "user-stop",
        SessionEvent::ProcessExited => "process-exited",
        SessionEvent::TrustRevoked => "trust-revoked",
        SessionEvent::LeaseConflict => "lease-conflict",
        SessionEvent::UserReleasedLease => "user-released-lease",
        SessionEvent::Output(OutputSignal::Cli(_)) => "cli-signal",
        SessionEvent::Output(OutputSignal::DecodeFailed(_)) => "decode-failed",
    }
}

impl AgentSession {
    /// 사용자가 명시적으로 시작한 세션을 만듭니다. 상태는 `Starting`이고 권한·lease 0에서 시작합니다.
    pub fn start_by_user(request: UserStartRequest) -> AgentSession {
        AgentSession {
            id: request.session,
            mode: request.mode,
            started_ms: request.now_ms,
            grace_ms: request.reconnect_grace_ms,
            state: SessionState::Starting,
            leases: BTreeSet::new(),
            permissions: BTreeSet::new(),
            requests: BTreeSet::new(),
            proposals: BTreeSet::new(),
            in_flight: false,
            applied: Vec::new(),
            unconfirmed: 0,
            decode_failed: false,
        }
    }

    /// id.
    pub fn id(&self) -> AgentSessionId {
        self.id
    }

    /// 모드.
    pub fn mode(&self) -> SessionMode {
        self.mode
    }

    /// 상태.
    pub fn state(&self) -> SessionState {
        self.state
    }

    /// 보유 lease.
    pub fn leases(&self) -> impl Iterator<Item = LeaseId> + '_ {
        self.leases.iter().copied()
    }

    /// 보유 권한.
    pub fn permissions(&self) -> impl Iterator<Item = Permission> + '_ {
        self.permissions.iter().copied()
    }

    /// 응답 대기 요청 수.
    pub fn pending_requests(&self) -> usize {
        self.requests.len()
    }

    /// 적용 전 제안 수.
    pub fn pending_proposals(&self) -> usize {
        self.proposals.len()
    }

    /// 적용된 batch(끝난 뒤에도 유지, 출처 표시 근거).
    pub fn applied_batches(&self) -> &[AppliedBatch] {
        &self.applied
    }

    /// 디코드 실패를 받았는지(표시 상태일 뿐, 권한·lease와 무관).
    pub fn decode_failed(&self) -> bool {
        self.decode_failed
    }

    fn invalid(&self, event: &SessionEvent) -> TransitionError {
        TransitionError::Invalid {
            state: state_name(self.state),
            event: event_name(event),
        }
    }

    fn is_active(&self) -> bool {
        matches!(
            self.state,
            SessionState::Starting | SessionState::Running | SessionState::Reconnecting { .. }
        )
    }

    /// event를 적용하고 runtime이 실행할 효과를 돌려줍니다.
    pub fn apply(
        &mut self,
        event: SessionEvent,
        now_ms: u64,
    ) -> Result<Vec<SessionEffect>, TransitionError> {
        if matches!(self.state, SessionState::Ended { .. }) {
            return Err(TransitionError::Ended);
        }
        let running = self.state == SessionState::Running;
        match event {
            SessionEvent::BridgeBound => match self.state {
                SessionState::Starting | SessionState::Reconnecting { .. } => {
                    self.state = SessionState::Running;
                    Ok(Vec::new())
                }
                _ => Err(self.invalid(&event)),
            },
            SessionEvent::BridgeDisconnected => {
                if !running {
                    return Err(self.invalid(&event));
                }
                self.state = SessionState::Reconnecting {
                    deadline_ms: now_ms.saturating_add(self.grace_ms),
                };
                Ok(Vec::new())
            }
            SessionEvent::Tick => match self.state {
                SessionState::Reconnecting { deadline_ms } if now_ms >= deadline_ms => {
                    Ok(self.begin_end(EndReason::ReconnectGraceExpired, now_ms))
                }
                _ => Ok(Vec::new()),
            },
            SessionEvent::LeaseGranted(lease) if running => {
                self.leases.insert(lease);
                Ok(Vec::new())
            }
            SessionEvent::PermissionGranted(permission) if running => {
                if !self.mode.cap().contains(&permission) {
                    return Err(TransitionError::AboveModeCap(permission));
                }
                self.permissions.insert(permission);
                Ok(Vec::new())
            }
            SessionEvent::RequestOpened(id) if running => {
                self.requests.insert(id);
                Ok(Vec::new())
            }
            SessionEvent::ProposalQueued(id) if running => {
                self.proposals.insert(id);
                Ok(Vec::new())
            }
            SessionEvent::TransactionStarted if running => {
                if self.in_flight {
                    return Err(self.invalid(&event));
                }
                self.in_flight = true;
                Ok(Vec::new())
            }
            SessionEvent::EditCompleted if running => {
                let batches = std::mem::take(&mut self.unconfirmed);
                let mut effects = Vec::new();
                if batches > 0 {
                    effects.push(SessionEffect::ConfirmUndoEntry {
                        session: self.id,
                        batches,
                    });
                }
                let leases: Vec<LeaseId> = std::mem::take(&mut self.leases).into_iter().collect();
                if !leases.is_empty() {
                    effects.push(SessionEffect::ReleaseLeases(leases));
                }
                Ok(effects)
            }
            SessionEvent::LeaseGranted(_)
            | SessionEvent::PermissionGranted(_)
            | SessionEvent::RequestOpened(_)
            | SessionEvent::ProposalQueued(_)
            | SessionEvent::TransactionStarted
            | SessionEvent::EditCompleted => Err(self.invalid(&event)),
            SessionEvent::RequestAnswered(id) => {
                self.requests.remove(&id);
                Ok(Vec::new())
            }
            SessionEvent::ProposalResolved(id) => {
                self.proposals.remove(&id);
                Ok(Vec::new())
            }
            SessionEvent::TransactionSettled(applied) => {
                if !self.in_flight || self.state == SessionState::Starting {
                    return Err(self.invalid(&event));
                }
                self.in_flight = false;
                if let Some(batch) = applied {
                    self.applied.push(batch);
                    self.unconfirmed += 1;
                }
                match self.state {
                    SessionState::Ending { reason } => Ok(self.finish_end(reason, now_ms)),
                    _ => Ok(Vec::new()),
                }
            }
            SessionEvent::Output(OutputSignal::DecodeFailed(_)) => {
                // 디코드 실패는 표시 상태만 바꿉니다. 권한·lease·요청·문서를 건드리지 않습니다(XXIX.2.13).
                self.decode_failed = true;
                Ok(Vec::new())
            }
            SessionEvent::UserStop
            | SessionEvent::ProcessExited
            | SessionEvent::TrustRevoked
            | SessionEvent::LeaseConflict
            | SessionEvent::UserReleasedLease
            | SessionEvent::Output(OutputSignal::Cli(_)) => {
                if !self.is_active() {
                    return Ok(Vec::new());
                }
                let reason = match event {
                    SessionEvent::UserStop => EndReason::UserStopped,
                    SessionEvent::ProcessExited => EndReason::ProcessExited,
                    SessionEvent::TrustRevoked => EndReason::TrustRevoked,
                    SessionEvent::LeaseConflict => EndReason::LeaseConflict,
                    SessionEvent::UserReleasedLease => EndReason::UserReleasedLease,
                    SessionEvent::Output(OutputSignal::Cli(CliSignal::UsageLimit)) => {
                        EndReason::UsageLimitReported
                    }
                    _ => EndReason::AuthErrorReported,
                };
                Ok(self.begin_end(reason, now_ms))
            }
        }
    }

    fn begin_end(&mut self, reason: EndReason, now_ms: u64) -> Vec<SessionEffect> {
        let mut effects = Vec::new();
        if reason != EndReason::ProcessExited {
            effects.push(SessionEffect::TerminateProcess);
        }
        if self.in_flight {
            self.state = SessionState::Ending { reason };
            return effects;
        }
        effects.extend(self.finish_end(reason, now_ms));
        effects
    }

    fn finish_end(&mut self, reason: EndReason, now_ms: u64) -> Vec<SessionEffect> {
        let mut effects = Vec::new();
        let batches = std::mem::take(&mut self.unconfirmed);
        if batches > 0 {
            effects.push(SessionEffect::FinalizeInterruptedHistoryEntry {
                session: self.id,
                batches,
            });
        }
        let leases: Vec<LeaseId> = std::mem::take(&mut self.leases).into_iter().collect();
        effects.push(SessionEffect::ReleaseLeases(leases));
        self.permissions.clear();
        effects.push(SessionEffect::RevokeSessionPermissions);
        let requests: Vec<RequestId> = std::mem::take(&mut self.requests).into_iter().collect();
        effects.push(SessionEffect::RejectPendingRequests(requests));
        let proposals: Vec<ProposalId> = std::mem::take(&mut self.proposals).into_iter().collect();
        effects.push(SessionEffect::DiscardProposals(proposals));
        effects.push(SessionEffect::CloseApprovalUi);
        effects.push(SessionEffect::EndBridgeBinding);
        effects.push(SessionEffect::CleanupExecutionTraces);
        effects.push(SessionEffect::RecordDiagnostic(SessionDiagnostic {
            session: self.id,
            duration_ms: now_ms.saturating_sub(self.started_ms),
            error_code: reason.code(),
        }));
        self.state = SessionState::Ended { reason };
        effects
    }
}
