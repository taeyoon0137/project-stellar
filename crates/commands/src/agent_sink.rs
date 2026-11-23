//! 에이전트 작업 그룹과 창 history 연결 지점 (XIX.6.2, XIX.6.4, XIX.6.8, ADR-0100).
//!
//! 에이전트 세션 관리(mcp-core)와 창 history(history crate)는 서로 닿지 않습니다(crate graph: mcp-core는 history에,
//! runtime은 mcp-core에 의존할 수 없음). 둘이 모두 닿는 commands crate에 연결 지점 trait
//! [`AgentHistorySink`]를 둡니다. history crate가 창 history로 구현하고(`stellar_history::agent`), 세션 관리는
//! 이 trait만 부릅니다.
//!
//! - 작업 그룹 확정: 정상 완료(CompleteAgentEdit)와 종료 절차의 "중단됨"은 같은 경로입니다. 이미 적용된
//!   batch들을 다시 실행하지 않고 정방향(이어 붙임)·역방향(역순으로 이어 붙임)을 history entry 하나로 넣습니다.
//! - 되돌리기 적용: 새 User 출처 transaction을 lease 판정(`execute_leased`)과 함께 실행하고 "에이전트 변경
//!   되돌리기" entry 하나로 넣습니다.

use std::fmt;

use crate::agent_batch::AgentBatchRecord;
use crate::change::ChangeSet;
use crate::id::{AgentProvenance, CommandId, CommandOrigin};
use crate::lock::lease::{LeaseTable, LeaseViolation};
use crate::state::DocumentState;
use crate::transaction::{CommandTransaction, TransactionError};

/// 작업 그룹 확정 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentGroupOutcome {
    /// 정상 완료(CompleteAgentEdit).
    Completed,
    /// 세션 종료 절차로 확정("중단됨").
    Interrupted,
}

/// history entry 하나로 확정할 작업 그룹(이미 적용됨).
#[derive(Debug, Clone, PartialEq)]
pub struct AgentGroup {
    /// 출처(그룹 lease의 에이전트 편집 세션).
    pub origin: CommandOrigin,
    /// batch 정방향을 적용 순서로 이어 붙인 transaction.
    pub forward: CommandTransaction,
    /// batch 역방향을 적용 역순으로 이어 붙인 transaction.
    pub inverse: CommandTransaction,
}

impl AgentGroup {
    /// 적용 순서 batch 목록으로 그룹을 만듭니다. 비어 있으면 `None`입니다. 출처는 `lease` 편집 세션, transaction
    /// id는 `id`입니다.
    pub fn from_batches(
        id: CommandId,
        lease: AgentProvenance,
        batches: &[&AgentBatchRecord],
    ) -> Option<AgentGroup> {
        if batches.is_empty() {
            return None;
        }
        let origin = CommandOrigin::FutureAgent(lease);
        let mut forward = CommandTransaction::new(id, origin);
        let mut inverse = CommandTransaction::new(id, origin);
        for batch in batches {
            forward
                .requests
                .extend(batch.forward.requests.iter().cloned());
        }
        for batch in batches.iter().rev() {
            inverse
                .requests
                .extend(batch.inverse.requests.iter().cloned());
        }
        Some(AgentGroup {
            origin,
            forward,
            inverse,
        })
    }
}

/// 되돌리기 적용 결과.
#[derive(Debug, Clone, PartialEq)]
pub struct AppliedRevert {
    /// "에이전트 변경 되돌리기" history entry sequence.
    pub sequence: u64,
    /// 적용 change set(일반 autosave 경로 입력).
    pub changes: ChangeSet,
}

/// 연결 지점 실패. 문서와 history는 호출 전 상태입니다.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentSinkError {
    /// command validator가 거부했습니다.
    Transaction(TransactionError),
    /// 다른 활성 lease 범위와 겹칩니다(XIX.4.3).
    Lease(LeaseViolation),
    /// 계획 뒤 문서 revision이 바뀌었습니다(미리보기를 다시 만들어야 함).
    Stale,
    /// history가 지금 기록을 받을 수 없습니다(preview·재생 중 등).
    History(String),
}

impl fmt::Display for AgentSinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentSinkError::Transaction(e) => write!(f, "{e}"),
            AgentSinkError::Lease(v) => write!(f, "{v}"),
            AgentSinkError::Stale => f.write_str("미리보기 뒤 문서가 바뀌었습니다"),
            AgentSinkError::History(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for AgentSinkError {}

/// 창 history 연결 지점.
pub trait AgentHistorySink {
    /// 지금 문서 상태(되돌리기 계획·미리보기 입력, 읽기 전용).
    fn state(&self) -> &DocumentState;

    /// 이미 적용된 작업 그룹을 history entry 하나로 확정하고 그 sequence를 돌려줍니다. 문서는 바뀌지 않습니다.
    fn confirm_group(
        &mut self,
        group: AgentGroup,
        outcome: AgentGroupOutcome,
    ) -> Result<u64, AgentSinkError>;

    /// 되돌리기 transaction(User 출처)을 lease 판정과 함께 적용하고 history entry 하나로 기록합니다.
    fn apply_revert(
        &mut self,
        transaction: &CommandTransaction,
        table: &LeaseTable,
    ) -> Result<AppliedRevert, AgentSinkError>;
}
