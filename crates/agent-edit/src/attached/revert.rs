//! 에이전트 변경 되돌리기 미리보기와 적용 (XIX.6.8, XIX.6.9, ADR-0100).
//!
//! - [`preview_revert`]: 출처 색인에서 대상 batch(변경 하나 또는 세션의 남은 batch)를 고르고,
//!   `stellar_commands::agent_revert::plan_revert`로 충돌을 분류해 command를 고른 뒤, 그 User 출처 transaction을
//!   승인 전 미리보기와 같은 경로([`crate::attached::edit::preview`]: semantic diff·validation·lease 판정)로
//!   보여 줍니다. 문서와 revision은 바뀌지 않습니다.
//! - [`apply_revert`]: 사용자가 승인한 미리보기를 창 history 연결 지점([`AgentHistorySink::apply_revert`],
//!   `execute_leased` + "에이전트 변경 되돌리기" entry 하나)으로 적용하고, 복원한 batch를 "되돌림"으로 표시합니다.
//!   미리보기 뒤 문서가 바뀌었으면 적용하지 않습니다(다시 미리보기). 결과 change set은 일반 autosave 경로로
//!   넘깁니다.
//!
//! 두 함수 모두 앱 사용자 UI 경로 전용입니다. MCP tool에는 되돌리기가 없습니다(ADR-0005 §3).

use std::collections::BTreeSet;
use std::fmt;

use stellar_ops::commands::agent_batch::AgentBatchId;
use stellar_ops::commands::agent_revert::{plan_revert, RevertError, RevertItem, RevertPlan};
use stellar_ops::commands::agent_sink::{AgentHistorySink, AgentSinkError, AppliedRevert};
use stellar_ops::commands::id::CommandId;
use stellar_ops::commands::lock::lease::LeaseTable;
use stellar_ops::commands::state::DocumentState;
use stellar_ops::validate::Definitions;

use crate::attached::edit::{preview, AgentPreview, AttachedEditError};
use crate::provenance::{AgentChangeProvenance, ProvenanceError, ProvenanceScope, UserUiAction};

/// 미리보기를 만드는 transaction id(미리보기 사본 전용, 적용 id는 [`apply_revert`]가 받음).
const PREVIEW_ID: CommandId = CommandId::from_bytes([
    0x52, 0x45, 0x56, 0x45, 0x52, 0x54, 0x70, 0x00, 0x80, 0, 0, 0, 0, 0, 0, 1,
]);

/// 되돌리기 미리보기.
#[derive(Debug, Clone)]
pub struct RevertPreview {
    /// 범위.
    pub scope: ProvenanceScope,
    /// 계획(command·충돌·복원 항목).
    pub plan: RevertPlan,
    /// 승인 전 미리보기(XIX.5.3 경로). 복원할 것이 없으면 `None`.
    pub preview: Option<AgentPreview>,
}

impl RevertPreview {
    /// 적용하면 다른 활성 lease와 겹쳐 거부되는지.
    pub fn blocked_by_lease(&self) -> bool {
        self.preview.as_ref().is_some_and(|p| p.scope.is_err())
    }
}

/// 되돌리기 흐름 실패. 문서와 history는 호출 전 상태입니다.
#[derive(Debug, Clone, PartialEq)]
pub enum RevertFlowError {
    /// 출처 색인 오류(모르는 batch, 이미 유지·되돌림 등).
    Provenance(ProvenanceError),
    /// 계획 실패.
    Plan(RevertError),
    /// 미리보기 실패.
    Preview(AttachedEditError),
    /// 복원할 것이 없습니다(모두 충돌로 제외).
    NothingToRevert,
    /// 적용 실패(lease 겹침·낡은 미리보기·history 거부).
    Apply(AgentSinkError),
}

impl fmt::Display for RevertFlowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RevertFlowError::Provenance(e) => write!(f, "{e}"),
            RevertFlowError::Plan(e) => write!(f, "{e}"),
            RevertFlowError::Preview(e) => write!(f, "{e}"),
            RevertFlowError::NothingToRevert => f.write_str("복원할 변경이 없습니다"),
            RevertFlowError::Apply(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RevertFlowError {}

/// 되돌리기 미리보기를 만듭니다. `include`는 사용자가 충돌인데도 복원하겠다고 고른 항목입니다.
pub fn preview_revert<'a>(
    state: &DocumentState,
    provenance: &AgentChangeProvenance,
    scope: ProvenanceScope,
    include: &BTreeSet<RevertItem>,
    table: &LeaseTable,
    definitions: impl FnOnce() -> Definitions<'a>,
) -> Result<RevertPreview, RevertFlowError> {
    let records = provenance
        .revertible(scope)
        .map_err(RevertFlowError::Provenance)?;
    let plan = plan_revert(state, &records, include).map_err(RevertFlowError::Plan)?;
    let preview = match plan.transaction(PREVIEW_ID) {
        Some(transaction) => Some(
            preview(state, &transaction, table, definitions).map_err(RevertFlowError::Preview)?,
        ),
        None => None,
    };
    Ok(RevertPreview {
        scope,
        plan,
        preview,
    })
}

/// 승인한 미리보기를 적용합니다(앱 사용자 UI 전용). 복원 항목이 있는 batch를 "되돌림"으로 표시합니다.
pub fn apply_revert(
    approved: &RevertPreview,
    history: &mut dyn AgentHistorySink,
    table: &LeaseTable,
    provenance: &mut AgentChangeProvenance,
    id: CommandId,
    ui: UserUiAction,
) -> Result<AppliedRevert, RevertFlowError> {
    let transaction = approved
        .plan
        .transaction(id)
        .ok_or(RevertFlowError::NothingToRevert)?;
    if history.state().revision() != approved.plan.revision {
        return Err(RevertFlowError::Apply(AgentSinkError::Stale));
    }
    let applied = history
        .apply_revert(&transaction, table)
        .map_err(RevertFlowError::Apply)?;
    let restored: BTreeSet<AgentBatchId> = approved
        .plan
        .restored
        .iter()
        .map(|item| item.batch)
        .collect();
    let batches: Vec<AgentBatchId> = restored.into_iter().collect();
    provenance.mark_reverted(&batches, applied.sequence, ui);
    Ok(applied)
}

#[cfg(test)]
#[path = "revert_tests.rs"]
mod tests;
