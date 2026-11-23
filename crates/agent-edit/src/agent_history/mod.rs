//! 에이전트 편집 세션의 완료·중지·종료 절차와 외부 변경 처리 (XIX.6.2~XIX.6.4, XIX.6.6, XIX.7.1~XIX.7.3,
//! ADR-0100).
//!
//! [`AgentEditSessions`]는 한 창(문서)에 붙은 에이전트 편집 세션을 관리합니다. lease 요청·보유는
//! [`AgentEditLeases`], 권한은 [`PermissionStore`], 출처 색인은 [`AgentChangeProvenance`], 창 history는
//! [`AgentHistorySink`](commands crate의 연결 지점, history crate가 구현)를 그대로 씁니다.
//!
//! ## 정상 완료(CompleteAgentEdit, XIX.6.2)
//!
//! 확정되지 않은 batch를 "에이전트 편집" history entry 하나로 확정하고(다시 실행하지 않음) lease를 해제합니다.
//! 세션과 권한은 남아 다음 `BeginAgentEdit`를 받을 수 있습니다. 완료 뒤 그 범위는 사용자 편집이 가능합니다.
//!
//! ## 종료 절차(XIX.6.4, 자동 rollback 없음)
//!
//! 종료 이유는 [`EndReason`]입니다. 문서 변경은 DocumentThread에서 transaction 단위로 원자 실행되므로(VII.1.6) 종료
//! 절차는 transaction 경계에서만 실행되고, 적용 중인 transaction은 이미 전부 적용 또는 전부 미적용입니다.
//!
//! 1. 확정되지 않은 적용 batch를 "중단됨" history entry 하나로 확정합니다. batch와 출처 표시는 그대로 둡니다.
//! 2. lease(대기 요청 포함)를 해제하고 세션 권한과 세션 한정 승인을 버립니다.
//! 3. 응답 대기 확인 요청은 거부로 닫고(보고서에 id), 적용 전 제안은 버립니다.
//! 4. 진단은 [`InterruptedAgentTask`](session id, duration, error code)만 남깁니다.
//!
//! history 확정이 실패해도(예: 창 history가 gesture preview 중) 2~4는 반드시 실행하고 실패를 보고서에 적습니다.
//! 확정되지 않은 batch는 출처 색인에 진행 중으로 남아 다음 확정 기회에 같은 경로로 확정합니다.
//!
//! 외부 클라이언트 세션은 bridge 연결 종료가 곧 종료입니다. 앱이 시작한 세션의 재연결 유예는 AgentSession
//! 상태 기계(`stellar-agent-host`, XXIX.2.8)가 정하고, 유예를 넘기면 [`EndReason::ReconnectGraceExpired`]로 이
//! 절차를 부릅니다. 그래서 [`AgentEditSessions::on_bridge_disconnected`]는 앱 시작 세션을 유지합니다.
//!
//! 세션 하나는 한 번에 lease 하나(작업 그룹 하나)만 진행합니다. 그래서 "중단됨" entry는 세션마다 많아야 하나입니다.

use std::collections::BTreeMap;
use std::fmt;

use stellar_ops::commands::agent_batch::AgentBatchId;
use stellar_ops::commands::agent_revert::RevertItem;
use stellar_ops::commands::agent_sink::{
    AgentGroup, AgentGroupOutcome, AgentHistorySink, AgentSinkError,
};
use stellar_ops::commands::id::{AgentProvenance, CommandId, CommandOrigin};
use stellar_ops::commands::lock::lease::{LeaseId, LeaseTable};
use stellar_ops::commands::state::DocumentState;
use stellar_ops::commands::transaction::CommandTransaction;
use stellar_ops::validate::Definitions;

use crate::agent_protocol::permission::Permission;
use crate::attached::edit::{apply_approved, AttachedEditError};
use crate::attached::revert::{preview_revert, RevertFlowError, RevertPreview};
use crate::lease::{AgentEditLeases, LeaseServiceError};
use crate::permission::{
    AgentSessionId, Confirmation, ConnectionApproval, PermissionKey, PermissionStore, RequestId,
};
use crate::provenance::{AgentChangeProvenance, BatchOutcome, ProvenanceError, ProvenanceScope};

/// 외부 변경 batch 판정과 반영 (XIX.7.1~XIX.7.3).
pub mod external;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod inventory_tests;

/// 세션 출처.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionOrigin {
    /// 외부 MCP 클라이언트(bridge 연결 하나).
    ExternalClient,
    /// 앱이 시작한 내장 세션(AgentSession, XXIX.2.8).
    AppStarted,
}

/// 세션이 끝난 이유(ADR-0005 §4.2 종료 조건).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EndReason {
    /// 외부 클라이언트의 MCP 연결 종료.
    McpDisconnected,
    /// agent process 종료.
    ProcessExited,
    /// 사용자 중지(CancelAgentEdit).
    UserStopped,
    /// 재연결 유예 초과(앱 시작 세션).
    ReconnectGraceExpired,
    /// 비정상 종료.
    AbnormalExit,
    /// 프로젝트 신뢰 철회.
    TrustRevoked,
    /// lease와 겹치는 외부 변경(XIX.7.2).
    LeaseConflict,
    /// 사용자의 lease 해제.
    UserReleasedLease,
    /// CLI가 보고한 사용 한도(앱 시작 세션, ADR-0124 결정 1).
    UsageLimitReported,
    /// CLI가 보고한 인증 오류(앱 시작 세션, ADR-0124 결정 1).
    AuthErrorReported,
}

impl EndReason {
    /// 진단용 고정 오류 코드. 앱 시작 세션과 같은 이유는 `stellar-agent-host` AgentSession과 같은 문자열입니다.
    pub fn code(self) -> &'static str {
        match self {
            EndReason::McpDisconnected => "mcp-disconnected",
            EndReason::ProcessExited => "process-exited",
            EndReason::UserStopped => "user-stopped",
            EndReason::ReconnectGraceExpired => "reconnect-grace-expired",
            EndReason::AbnormalExit => "abnormal-exit",
            EndReason::TrustRevoked => "trust-revoked",
            EndReason::LeaseConflict => "lease-conflict",
            EndReason::UserReleasedLease => "user-released-lease",
            EndReason::UsageLimitReported => "usage-limit",
            EndReason::AuthErrorReported => "auth-error",
        }
    }
}

/// Interrupted Agent Task local diagnostic(XIX.6.6). 내용(프로젝트·문서·명령·출력)을 담는 필드가 없습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterruptedAgentTask {
    /// 세션 id.
    pub session: AgentSessionId,
    /// 지속 시간(ms).
    pub duration_ms: u64,
    /// 오류 코드([`EndReason::code`]).
    pub error_code: &'static str,
}

impl fmt::Display for InterruptedAgentTask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "interrupted-agent-task session={} duration_ms={} error={}",
            self.session.0, self.duration_ms, self.error_code
        )
    }
}

/// 적용 전 제안 id(세션 안에서 고유).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProposalId(pub u64);

/// 세션 관리 오류.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionError {
    /// 모르는 세션.
    UnknownSession(AgentSessionId),
    /// 이미 있는 세션.
    DuplicateSession(AgentSessionId),
    /// 이미 끝난 세션(다시 시작하지 않음).
    Ended(AgentSessionId, EndReason),
    /// 아직 끝나지 않은 세션(끝난 세션만 받는 동작).
    NotEnded(AgentSessionId),
    /// 진행 중인 편집(lease)이 있습니다.
    EditInProgress(LeaseId),
    /// 진행 중인 편집이 없습니다.
    NoEdit,
    /// 제안 출처가 세션 lease가 아닙니다.
    ForeignProposal,
    /// 모르는 제안.
    UnknownProposal(ProposalId),
    /// 승인된 적용 실패(문서는 그대로).
    Apply(AttachedEditError),
    /// 출처 색인 오류.
    Provenance(ProvenanceError),
    /// history 확정 실패(문서는 그대로, batch는 진행 중으로 남음).
    History(AgentSinkError),
    /// lease 해제 실패.
    Lease(LeaseServiceError),
    /// 되돌리기 미리보기 실패.
    Revert(RevertFlowError),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::UnknownSession(id) => write!(f, "에이전트 세션 {}이 없습니다", id.0),
            SessionError::DuplicateSession(id) => {
                write!(f, "에이전트 세션 {}이 이미 있습니다", id.0)
            }
            SessionError::Ended(id, reason) => {
                write!(f, "에이전트 세션 {}은 끝났습니다({})", id.0, reason.code())
            }
            SessionError::NotEnded(id) => {
                write!(f, "에이전트 세션 {}은 아직 끝나지 않았습니다", id.0)
            }
            SessionError::EditInProgress(id) => write!(f, "진행 중인 편집 {id}가 있습니다"),
            SessionError::NoEdit => f.write_str("진행 중인 편집이 없습니다"),
            SessionError::ForeignProposal => f.write_str("제안 출처가 이 세션의 편집이 아닙니다"),
            SessionError::UnknownProposal(id) => write!(f, "제안 {}이 없습니다", id.0),
            SessionError::Apply(e) => write!(f, "{e}"),
            SessionError::Provenance(e) => write!(f, "{e}"),
            SessionError::History(e) => write!(f, "history 확정 실패: {e}"),
            SessionError::Lease(e) => write!(f, "{e}"),
            SessionError::Revert(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SessionError {}

/// 세션 절차가 쓰는 창·프로젝트 상태.
#[derive(Debug)]
pub struct SessionContext<'a> {
    /// lease 요청·보유.
    pub leases: &'a mut AgentEditLeases,
    /// 권한 저장소.
    pub permissions: &'a mut PermissionStore,
    /// 창 출처 색인.
    pub provenance: &'a mut AgentChangeProvenance,
}

/// 정상 완료 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedEdit {
    /// 해제한 lease.
    pub lease: LeaseId,
    /// 확정한 history entry(적용 batch가 없으면 `None`).
    pub history_sequence: Option<u64>,
    /// 확정한 batch 수.
    pub batches: usize,
}

/// 종료 절차 결과.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionEndReport {
    /// 세션.
    pub session: AgentSessionId,
    /// 이유.
    pub reason: EndReason,
    /// "중단됨" history entry(적용 batch가 없거나 확정 실패면 `None`).
    pub history_sequence: Option<u64>,
    /// 확정한 batch 수.
    pub interrupted_batches: usize,
    /// history 확정 실패(있으면).
    pub history_error: Option<AgentSinkError>,
    /// 해제한 lease(대기 요청 포함).
    pub released_lease: Option<LeaseId>,
    /// lease 해제 실패(있으면).
    pub lease_error: Option<LeaseServiceError>,
    /// 거부로 닫은 확인 요청.
    pub rejected_requests: Vec<(Permission, RequestId)>,
    /// 첫 연결 승인 대기를 거부로 닫았는지.
    pub rejected_connection: bool,
    /// 버린 적용 전 제안 수.
    pub discarded_proposals: usize,
    /// 진단 기록.
    pub diagnostic: InterruptedAgentTask,
}

/// 대기 중 제안(앞부분을 나눠 적용했으면 남은 command).
#[derive(Debug)]
struct PendingProposal {
    transaction: CommandTransaction,
    /// 이미 적용한 batch 수(다음 batch transaction id에 씀).
    applied_chunks: u32,
}

/// [`AgentEditSessions::apply_proposal_chunk`] 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkApplied {
    /// 이번에 적용한 batch.
    pub batch: AgentBatchId,
    /// 제안에 남은 command 수(0이면 제안이 끝남).
    pub remaining: usize,
}

/// 나눠 적용하는 제안의 `index`번째 batch transaction id. 첫 batch는 제안 id 그대로이고, 이후는 마지막 4 byte에
/// 순번을 더합니다.
fn chunk_id(id: CommandId, index: u32) -> CommandId {
    if index == 0 {
        return id;
    }
    let mut bytes = id.to_bytes();
    let tail = u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]).wrapping_add(index);
    bytes[12..].copy_from_slice(&tail.to_be_bytes());
    CommandId::from_bytes(bytes)
}

#[derive(Debug)]
struct SessionSlot {
    key: PermissionKey,
    origin: SessionOrigin,
    name: String,
    started_ms: u64,
    lease: Option<LeaseId>,
    proposals: BTreeMap<ProposalId, PendingProposal>,
    next_proposal: u64,
}

/// 한 창의 에이전트 편집 세션.
#[derive(Debug, Default)]
pub struct AgentEditSessions {
    sessions: BTreeMap<AgentSessionId, SessionSlot>,
    ended: BTreeMap<AgentSessionId, EndReason>,
}

impl AgentEditSessions {
    /// 빈 관리자.
    pub fn new() -> AgentEditSessions {
        AgentEditSessions::default()
    }

    fn slot(&mut self, id: AgentSessionId) -> Result<&mut SessionSlot, SessionError> {
        if let Some(reason) = self.ended.get(&id) {
            return Err(SessionError::Ended(id, *reason));
        }
        self.sessions
            .get_mut(&id)
            .ok_or(SessionError::UnknownSession(id))
    }

    /// 세션을 등록합니다. `name`은 CLI가 보고한 세션 표시 이름, `now_ms`는 단조 시각입니다.
    pub fn start(
        &mut self,
        id: AgentSessionId,
        key: PermissionKey,
        origin: SessionOrigin,
        name: &str,
        now_ms: u64,
    ) -> Result<(), SessionError> {
        if let Some(reason) = self.ended.get(&id) {
            return Err(SessionError::Ended(id, *reason));
        }
        if self.sessions.contains_key(&id) {
            return Err(SessionError::DuplicateSession(id));
        }
        self.sessions.insert(
            id,
            SessionSlot {
                key,
                origin,
                name: name.to_string(),
                started_ms: now_ms,
                lease: None,
                proposals: BTreeMap::new(),
                next_proposal: 1,
            },
        );
        Ok(())
    }

    /// 활성 세션인지.
    pub fn is_active(&self, id: AgentSessionId) -> bool {
        self.sessions.contains_key(&id)
    }

    /// 활성 세션 수.
    pub fn active_len(&self) -> usize {
        self.sessions.len()
    }

    /// 활성 세션 id(순서대로). 창을 닫을 때 남은 세션을 끝내는 데 씁니다.
    pub fn active_ids(&self) -> Vec<AgentSessionId> {
        self.sessions.keys().copied().collect()
    }

    /// 끝난 이유.
    pub fn end_reason(&self, id: AgentSessionId) -> Option<EndReason> {
        self.ended.get(&id).copied()
    }

    /// 세션의 진행 중 편집 lease.
    pub fn lease(&self, id: AgentSessionId) -> Option<LeaseId> {
        self.sessions.get(&id).and_then(|s| s.lease)
    }

    /// lease를 가진 활성 세션.
    pub fn session_of_lease(&self, lease: LeaseId) -> Option<AgentSessionId> {
        self.sessions
            .iter()
            .find(|(_, s)| s.lease == Some(lease))
            .map(|(id, _)| *id)
    }

    /// 대기 중인 적용 전 제안 수.
    pub fn pending_proposals(&self, id: AgentSessionId) -> usize {
        self.sessions.get(&id).map_or(0, |s| s.proposals.len())
    }

    /// `BeginAgentEdit`로 요청한 lease를 세션의 진행 중 편집으로 묶습니다(요청은 [`AgentEditLeases::submit`]).
    pub fn begin_edit(&mut self, id: AgentSessionId, lease: LeaseId) -> Result<(), SessionError> {
        let slot = self.slot(id)?;
        if let Some(current) = slot.lease {
            return Err(SessionError::EditInProgress(current));
        }
        slot.lease = Some(lease);
        Ok(())
    }

    /// 사용자 승인을 기다리는 제안을 둡니다. 출처는 세션 lease의 에이전트 출처여야 합니다.
    pub fn queue_proposal(
        &mut self,
        id: AgentSessionId,
        transaction: CommandTransaction,
    ) -> Result<ProposalId, SessionError> {
        let slot = self.slot(id)?;
        let lease = slot.lease.ok_or(SessionError::NoEdit)?;
        if transaction.origin != CommandOrigin::FutureAgent(AgentProvenance::new(lease.0)) {
            return Err(SessionError::ForeignProposal);
        }
        let proposal = ProposalId(slot.next_proposal);
        slot.next_proposal += 1;
        slot.proposals.insert(
            proposal,
            PendingProposal {
                transaction,
                applied_chunks: 0,
            },
        );
        Ok(proposal)
    }

    /// 사용자가 거부한 제안을 버립니다.
    pub fn reject_proposal(
        &mut self,
        id: AgentSessionId,
        proposal: ProposalId,
    ) -> Result<(), SessionError> {
        self.slot(id)?
            .proposals
            .remove(&proposal)
            .map(|_| ())
            .ok_or(SessionError::UnknownProposal(proposal))
    }

    /// 사용자가 승인한 제안을 lease 판정과 함께 한 batch로 적용하고 출처 색인에 기록합니다(XIX.5.2, XIX.6.1, XIX.6.7).
    /// 적용은 transaction 하나라 전부 적용 또는 전부 미적용입니다. 거부되면 제안은 버리고 문서는 그대로입니다.
    /// [`Self::apply_proposal_chunk`]의 나누지 않는 경우입니다.
    pub fn apply_proposal(
        &mut self,
        id: AgentSessionId,
        proposal: ProposalId,
        state: &mut DocumentState,
        table: &LeaseTable,
        provenance: &mut AgentChangeProvenance,
    ) -> Result<AgentBatchId, SessionError> {
        self.apply_proposal_chunk(id, proposal, usize::MAX, state, table, provenance)
            .map(|chunk| chunk.batch)
    }

    /// 승인한 제안의 앞 command 최대 `max_commands`개를 한 batch로 적용합니다(XXIX.3.4 큰 batch 분할, ADR-0124 결정 6).
    ///
    /// - 한 batch는 transaction 하나라 전부 적용 또는 전부 미적용이고, batch마다 출처 기록이 따로 생깁니다. 완료·종료는
    ///   진행 중 batch 전체를 history 항목 하나로 확정하므로 Undo 한 번이 제안 전체를 되돌립니다.
    /// - 남은 command는 같은 제안 id로 대기에 남고([`ChunkApplied::remaining`]), 다음 호출이 이어서 적용합니다. 사용자가
    ///   그 사이 거부하거나 세션이 끝나면 남은 부분은 버려지고, 이미 적용한 batch는 출처 표시를 유지한 채 남습니다.
    /// - batch가 거부되면 남은 부분을 버리고 오류입니다. 그 batch는 적용되지 않았고 앞 batch는 그대로입니다.
    /// - 이어지는 batch의 transaction id는 제안 id에서 batch 순번으로 만듭니다(같은 제안 안에서 겹치지 않음).
    pub fn apply_proposal_chunk(
        &mut self,
        id: AgentSessionId,
        proposal: ProposalId,
        max_commands: usize,
        state: &mut DocumentState,
        table: &LeaseTable,
        provenance: &mut AgentChangeProvenance,
    ) -> Result<ChunkApplied, SessionError> {
        let slot = self.slot(id)?;
        let lease = slot.lease.ok_or(SessionError::NoEdit)?;
        let mut pending = slot
            .proposals
            .remove(&proposal)
            .ok_or(SessionError::UnknownProposal(proposal))?;
        let name = slot.name.clone();
        let take = max_commands.max(1).min(pending.transaction.requests.len());
        let rest = pending.transaction.requests.split_off(take);
        let chunk = CommandTransaction {
            id: chunk_id(pending.transaction.id, pending.applied_chunks),
            origin: pending.transaction.origin,
            requests: std::mem::take(&mut pending.transaction.requests),
        }
        .rebased(state.revision());
        let applied = apply_approved(state, &chunk, table).map_err(SessionError::Apply)?;
        let batch = provenance
            .record_batch(id, &name, lease, chunk, applied, state.model())
            .map_err(SessionError::Provenance)?;
        let remaining = rest.len();
        if remaining > 0 {
            pending.transaction.requests = rest;
            pending.applied_chunks = pending.applied_chunks.saturating_add(1);
            if let Some(slot) = self.sessions.get_mut(&id) {
                slot.proposals.insert(proposal, pending);
            }
        }
        Ok(ChunkApplied { batch, remaining })
    }

    /// 정상 완료(CompleteAgentEdit): 확정되지 않은 batch를 history entry 하나로 확정하고 lease를 해제합니다.
    /// history 확정이 실패하면 lease를 풀지 않고 오류입니다(다시 완료할 수 있음).
    pub fn complete(
        &mut self,
        id: AgentSessionId,
        ctx: &mut SessionContext<'_>,
        history: &mut dyn AgentHistorySink,
        entry_id: CommandId,
    ) -> Result<CompletedEdit, SessionError> {
        let slot = self.slot(id)?;
        let lease = slot.lease.ok_or(SessionError::NoEdit)?;
        let (sequence, batches) = confirm(
            id,
            lease,
            ctx.provenance,
            history,
            entry_id,
            AgentGroupOutcome::Completed,
        )
        .map_err(SessionError::History)?;
        ctx.leases.release(lease).map_err(SessionError::Lease)?;
        slot.lease = None;
        Ok(CompletedEdit {
            lease,
            history_sequence: sequence,
            batches,
        })
    }

    /// 사용자 중지(CancelAgentEdit, XIX.6.3): rollback하지 않고 종료 절차를 따릅니다.
    pub fn cancel(
        &mut self,
        id: AgentSessionId,
        ctx: &mut SessionContext<'_>,
        history: &mut dyn AgentHistorySink,
        entry_id: CommandId,
        now_ms: u64,
    ) -> Result<SessionEndReport, SessionError> {
        self.end(id, EndReason::UserStopped, ctx, history, entry_id, now_ms)
    }

    /// 중지하고 되돌리기(XIX.6.3): 종료 절차를 마친 뒤 그 세션의 세션 단위 되돌리기 미리보기를 만듭니다. 미리보기는
    /// 문서를 바꾸지 않으며, 사용자가 승인하면 `attached::revert::apply_revert`로 적용합니다. 별도 rollback 경로는
    /// 없습니다. 되돌릴 표시 batch가 없으면 미리보기는 `None`입니다.
    #[allow(clippy::too_many_arguments)]
    pub fn cancel_and_preview_revert<'d>(
        &mut self,
        id: AgentSessionId,
        ctx: &mut SessionContext<'_>,
        history: &mut dyn AgentHistorySink,
        entry_id: CommandId,
        now_ms: u64,
        table: &LeaseTable,
        definitions: impl FnOnce() -> Definitions<'d>,
    ) -> Result<(SessionEndReport, Option<RevertPreview>), SessionError> {
        let report = self.cancel(id, ctx, history, entry_id, now_ms)?;
        let scope = ProvenanceScope::Session(id);
        if ctx
            .provenance
            .revertible(scope)
            .map_or(true, |records| records.is_empty())
        {
            return Ok((report, None));
        }
        let preview = preview_revert(
            history.state(),
            ctx.provenance,
            scope,
            &std::collections::BTreeSet::<RevertItem>::new(),
            table,
            definitions,
        )
        .map_err(SessionError::Revert)?;
        Ok((report, Some(preview)))
    }

    /// bridge 연결 종료를 알립니다. 외부 클라이언트 세션은 종료 절차(`McpDisconnected`)를 실행하고, 앱이 시작한 세션은
    /// 재연결 유예 동안 유지합니다(유예는 AgentSession이 정하고, 넘기면 `ReconnectGraceExpired`로 [`Self::end`]).
    pub fn on_bridge_disconnected(
        &mut self,
        id: AgentSessionId,
        ctx: &mut SessionContext<'_>,
        history: &mut dyn AgentHistorySink,
        entry_id: CommandId,
        now_ms: u64,
    ) -> Result<Option<SessionEndReport>, SessionError> {
        match self.slot(id)?.origin {
            SessionOrigin::AppStarted => Ok(None),
            SessionOrigin::ExternalClient => self
                .end(
                    id,
                    EndReason::McpDisconnected,
                    ctx,
                    history,
                    entry_id,
                    now_ms,
                )
                .map(Some),
        }
    }

    /// 종료 절차에서 history 확정이 실패해(예: 창 history가 gesture preview 중) 진행 중으로 남은 batch를 "중단됨"
    /// entry 하나로 확정합니다. 다음 확정 기회(preview가 끝난 뒤 등)에 앱이 부릅니다. 끝난 세션만 받고, 남은 batch가
    /// 없으면 entry를 만들지 않습니다. 실패하면 batch는 계속 진행 중으로 남습니다.
    pub fn confirm_interrupted(
        &self,
        id: AgentSessionId,
        provenance: &mut AgentChangeProvenance,
        history: &mut dyn AgentHistorySink,
        entry_id: CommandId,
    ) -> Result<(Option<u64>, usize), SessionError> {
        if self.sessions.contains_key(&id) {
            return Err(SessionError::NotEnded(id));
        }
        if !self.ended.contains_key(&id) {
            return Err(SessionError::UnknownSession(id));
        }
        let Some(lease) = provenance
            .session_batches(id)
            .find(|b| b.outcome == BatchOutcome::InProgress)
            .map(|b| b.lease)
        else {
            return Ok((None, 0));
        };
        confirm(
            id,
            lease,
            provenance,
            history,
            entry_id,
            AgentGroupOutcome::Interrupted,
        )
        .map_err(SessionError::History)
    }

    /// 종료 절차(모듈 문서). 자동 rollback하지 않습니다.
    pub fn end(
        &mut self,
        id: AgentSessionId,
        reason: EndReason,
        ctx: &mut SessionContext<'_>,
        history: &mut dyn AgentHistorySink,
        entry_id: CommandId,
        now_ms: u64,
    ) -> Result<SessionEndReport, SessionError> {
        self.slot(id)?;
        let Some(slot) = self.sessions.remove(&id) else {
            return Err(SessionError::UnknownSession(id));
        };
        self.ended.insert(id, reason);
        // 1. 적용 batch를 "중단됨" entry 하나로 확정(batch·출처 표시는 유지).
        let lease_for_group = slot.lease.or_else(|| {
            ctx.provenance
                .session_batches(id)
                .find(|b| b.outcome == BatchOutcome::InProgress)
                .map(|b| b.lease)
        });
        let (history_sequence, interrupted_batches, history_error) = match lease_for_group {
            Some(lease) => match confirm(
                id,
                lease,
                ctx.provenance,
                history,
                entry_id,
                AgentGroupOutcome::Interrupted,
            ) {
                Ok((sequence, count)) => (sequence, count, None),
                Err(error) => (None, 0, Some(error)),
            },
            None => (None, 0, None),
        };
        // 2. lease 해제, 권한 폐기.
        let (released_lease, lease_error) = match slot.lease {
            Some(lease) => (Some(lease), ctx.leases.release(lease).err()),
            None => (None, None),
        };
        let rejected_requests: Vec<(Permission, RequestId)> = Permission::ALL
            .into_iter()
            .filter_map(
                |permission| match ctx.permissions.confirmation(&slot.key, permission) {
                    Confirmation::Pending(request) => Some((permission, request)),
                    _ => None,
                },
            )
            .collect();
        let rejected_connection =
            ctx.permissions.connection(&slot.key) == Some(ConnectionApproval::Pending);
        ctx.permissions.end_session(&slot.key);
        // 3. 적용 전 제안 폐기.
        let discarded_proposals = slot.proposals.len();
        // 4. 진단(내용 없음).
        let diagnostic = InterruptedAgentTask {
            session: id,
            duration_ms: now_ms.saturating_sub(slot.started_ms),
            error_code: reason.code(),
        };
        Ok(SessionEndReport {
            session: id,
            reason,
            history_sequence,
            interrupted_batches,
            history_error,
            released_lease,
            lease_error,
            rejected_requests,
            rejected_connection,
            discarded_proposals,
            diagnostic,
        })
    }
}

/// 세션의 확정되지 않은 batch를 entry 하나로 확정합니다. batch가 없으면 entry를 만들지 않습니다.
fn confirm(
    session: AgentSessionId,
    lease: LeaseId,
    provenance: &mut AgentChangeProvenance,
    history: &mut dyn AgentHistorySink,
    entry_id: CommandId,
    outcome: AgentGroupOutcome,
) -> Result<(Option<u64>, usize), AgentSinkError> {
    let records = provenance.in_progress(session);
    let Some(group) = AgentGroup::from_batches(entry_id, AgentProvenance::new(lease.0), &records)
    else {
        return Ok((None, 0));
    };
    let sequence = history.confirm_group(group, outcome)?;
    let settled = provenance.settle(
        session,
        match outcome {
            AgentGroupOutcome::Completed => BatchOutcome::Completed,
            AgentGroupOutcome::Interrupted => BatchOutcome::Interrupted,
        },
        Some(sequence),
    );
    Ok((Some(sequence), settled))
}
