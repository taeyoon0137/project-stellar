//! 에이전트 작업 그룹 확정과 되돌리기 기록 (XIX.6.2, XIX.6.4, XIX.6.8, ADR-0100).
//!
//! [`WindowAgentHistory`]가 `stellar_commands::agent_sink::AgentHistorySink`를 창 history로 구현합니다.
//!
//! - 작업 그룹(정상 완료, 종료 절차의 "중단됨"): 이미 적용된 batch를 다시 실행하지 않고 정방향·역방향을 항목
//!   하나로 넣습니다. 출처는 그룹 lease의 에이전트 출처(`FutureAgent`)입니다. 그 항목의 Undo는 그룹 전체를
//!   되돌립니다.
//! - 되돌리기: User 출처 transaction을 `execute_leased`(다른 활성 lease와 겹치면 거부, XIX.4.3)로 실행하고 항목
//!   하나로 넣습니다. 일반 Undo/Redo 대상입니다.
//!
//! 항목 이름은 [`AgentEntryLabels`]가 sequence별로 기억합니다(창 메모리, 창을 닫으면 history와 함께 버림).

use std::collections::BTreeMap;

use stellar_commands::agent_sink::{
    AgentGroup, AgentGroupOutcome, AgentHistorySink, AgentSinkError, AppliedRevert,
};
use stellar_commands::lock::lease::{execute_leased, LeaseTable, LeasedExecutionError};
use stellar_commands::state::DocumentState;
use stellar_commands::transaction::CommandTransaction;

use crate::entry::{HistoryEntry, HistoryError, HistorySequence};
use crate::window::WindowHistory;

/// 에이전트 관련 history 항목 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentEntryKind {
    /// 정상 완료된 에이전트 작업 그룹.
    Completed,
    /// 세션 종료 절차로 확정된 작업 그룹.
    Interrupted,
    /// 에이전트 변경 되돌리기.
    Revert,
}

impl AgentEntryKind {
    /// Undo·Redo 메뉴 동작 이름.
    pub fn action_name(self) -> &'static str {
        match self {
            AgentEntryKind::Completed => "에이전트 편집",
            AgentEntryKind::Interrupted => "에이전트 편집(중단됨)",
            AgentEntryKind::Revert => "에이전트 변경 되돌리기",
        }
    }
}

/// 창 하나의 에이전트 항목 이름표(창 메모리 전용).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentEntryLabels {
    kinds: BTreeMap<HistorySequence, AgentEntryKind>,
}

impl AgentEntryLabels {
    /// 빈 이름표.
    pub fn new() -> AgentEntryLabels {
        AgentEntryLabels::default()
    }

    /// 항목 종류.
    pub fn kind(&self, sequence: HistorySequence) -> Option<AgentEntryKind> {
        self.kinds.get(&sequence).copied()
    }

    /// 항목의 동작 이름: 에이전트 항목이면 그 이름, 아니면 일반 이름([`HistoryEntry::action_name`]).
    pub fn action_name(&self, entry: &HistoryEntry) -> &'static str {
        self.kind(entry.sequence)
            .map_or_else(|| entry.action_name(), AgentEntryKind::action_name)
    }

    /// 기록된 항목 수.
    pub fn len(&self) -> usize {
        self.kinds.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }

    fn insert(&mut self, sequence: HistorySequence, kind: AgentEntryKind) {
        self.kinds.insert(sequence, kind);
    }
}

/// 창 history·문서 상태·이름표를 묶은 연결 지점 구현.
#[derive(Debug)]
pub struct WindowAgentHistory<'a> {
    /// 창 history.
    pub window: &'a mut WindowHistory,
    /// 창 문서 상태.
    pub doc: &'a mut DocumentState,
    /// 에이전트 항목 이름표.
    pub labels: &'a mut AgentEntryLabels,
}

fn history_error(error: HistoryError) -> AgentSinkError {
    AgentSinkError::History(error.to_string())
}

impl AgentHistorySink for WindowAgentHistory<'_> {
    fn state(&self) -> &DocumentState {
        self.doc
    }

    fn confirm_group(
        &mut self,
        group: AgentGroup,
        outcome: AgentGroupOutcome,
    ) -> Result<u64, AgentSinkError> {
        let sequence = self
            .window
            .history_mut()
            .push_applied_document(group.origin, group.forward, group.inverse)
            .map_err(history_error)?;
        self.labels.insert(
            sequence,
            match outcome {
                AgentGroupOutcome::Completed => AgentEntryKind::Completed,
                AgentGroupOutcome::Interrupted => AgentEntryKind::Interrupted,
            },
        );
        Ok(sequence.get())
    }

    fn apply_revert(
        &mut self,
        transaction: &CommandTransaction,
        table: &LeaseTable,
    ) -> Result<AppliedRevert, AgentSinkError> {
        let history = self.window.history();
        if history.is_replaying() {
            return Err(history_error(HistoryError::Replaying));
        }
        if history.preview_active() {
            return Err(history_error(HistoryError::PreviewActive));
        }
        if transaction
            .requests
            .iter()
            .any(|r| r.base_revision != self.doc.revision())
        {
            return Err(AgentSinkError::Stale);
        }
        let applied = execute_leased(self.doc, transaction, table).map_err(|e| match e {
            LeasedExecutionError::Transaction(e) => AgentSinkError::Transaction(e),
            LeasedExecutionError::Lease(v) => AgentSinkError::Lease(v),
            LeasedExecutionError::Restore { violation, error } => {
                AgentSinkError::History(format!("{violation}; 되돌리기 실패: {error}"))
            }
        })?;
        let sequence = match self.window.history_mut().push_applied_document(
            transaction.origin,
            transaction.clone(),
            applied.inverse.clone(),
        ) {
            Ok(sequence) => sequence,
            Err(error) => {
                // 기록할 수 없으면 적용도 거둡니다(전부 미적용).
                let _ = self
                    .doc
                    .execute_transaction(&applied.inverse.rebased(self.doc.revision()));
                return Err(history_error(error));
            }
        };
        self.window.view_mut().prune_removed(&applied.changes);
        self.labels.insert(sequence, AgentEntryKind::Revert);
        Ok(AppliedRevert {
            sequence: sequence.get(),
            changes: applied.changes,
        })
    }
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
