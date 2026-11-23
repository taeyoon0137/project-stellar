//! 에이전트 proposal 미리보기와 승인된 적용 (XIX.5.2, XIX.5.3).

use std::fmt;

use stellar_ops::apply::decode_commands;
use stellar_ops::commands::change::ChangeSet;
use stellar_ops::commands::command::Command;
use stellar_ops::commands::id::{AgentProvenance, CommandId, CommandOrigin};
use stellar_ops::commands::lock::lease::{
    self, LeaseId, LeaseTable, LeaseViolation, LeasedExecutionError, PreviewError,
};
use stellar_ops::commands::request::CommandRequest;
use stellar_ops::commands::state::DocumentState;
use stellar_ops::commands::transaction::{AppliedTransaction, CommandTransaction};
use stellar_ops::core_types::ids::StableId;
use stellar_ops::core_types::revision::DocumentRevision;
use stellar_ops::diff::{diff_records, DiffReport};
use stellar_ops::validate::{validate_records, Definitions, Finding};

use crate::agent_protocol::outcome::ResultKind;
use crate::json::JsonValue;
use crate::proposal::parse_transaction;

/// attached 편집 실패. 어느 경우든 문서 상태는 호출 전과 같습니다.
#[derive(Debug, Clone, PartialEq)]
pub enum AttachedEditError {
    /// proposal 입력 형식 오류·모르는 command(XIX.5.1).
    Input {
        /// 고정 코드.
        code: &'static str,
        /// 원인.
        message: String,
    },
    /// proposal 문서가 이 창 세션의 문서가 아닙니다.
    ForeignDocument(StableId),
    /// command validator가 거부했습니다.
    Rejected(String),
    /// lease 범위 밖이거나 다른 lease 안을 바꿉니다(XIX.5.2).
    OutOfScope(LeaseViolation),
    /// diff·문서 사본을 만들 수 없습니다.
    Internal(String),
}

impl AttachedEditError {
    /// MCP 결과 종류.
    pub fn kind(&self) -> ResultKind {
        match self {
            AttachedEditError::Input { .. } => ResultKind::InvalidArguments,
            AttachedEditError::ForeignDocument(_) => ResultKind::NotFound,
            AttachedEditError::Rejected(_) => ResultKind::Rejected,
            AttachedEditError::OutOfScope(_) => ResultKind::OutOfScope,
            AttachedEditError::Internal(_) => ResultKind::Internal,
        }
    }
}

impl fmt::Display for AttachedEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AttachedEditError::Input { message, .. } => f.write_str(message),
            AttachedEditError::ForeignDocument(id) => {
                write!(f, "문서 {id}는 이 창 세션의 문서가 아닙니다")
            }
            AttachedEditError::Rejected(reason) => write!(f, "command가 거부되었습니다: {reason}"),
            AttachedEditError::OutOfScope(v) => write!(f, "{v}"),
            AttachedEditError::Internal(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for AttachedEditError {}

/// MCP proposal 인자(`{base_revision, document, commands}`)를 이 창 세션 문서의 에이전트 transaction으로 만듭니다.
///
/// command는 headless dry-run과 같은 해석기와 같은 닫힌 enum 변환(`stellar_ops::apply::decode_commands`)을 씁니다.
/// 출처는 lease의 편집 세션(`AgentProvenance`)이라 판정 주체가 그 lease입니다. `base_revision`은 저장 generation
/// 기준이므로 창 세션 revision과의 대조는 호출자(앱)가 합니다. transaction의 base는 지금 문서 revision입니다.
pub fn proposal_transaction(
    args: &JsonValue,
    state: &DocumentState,
    lease: LeaseId,
    id: CommandId,
    max_commands: usize,
) -> Result<CommandTransaction, AttachedEditError> {
    let request = parse_transaction(args, max_commands).map_err(|e| AttachedEditError::Input {
        code: e.code,
        message: e.message,
    })?;
    if request.document != state.model().document_id() {
        return Err(AttachedEditError::ForeignDocument(request.document));
    }
    let commands = decode_commands(&request.commands).map_err(|e| AttachedEditError::Input {
        code: "invalid-transaction",
        message: e.to_string(),
    })?;
    Ok(agent_transaction(state.revision(), lease, id, commands))
}

/// lease 편집 세션 출처의 transaction.
pub fn agent_transaction(
    base: DocumentRevision,
    lease: LeaseId,
    id: CommandId,
    commands: Vec<Command>,
) -> CommandTransaction {
    CommandTransaction {
        id,
        origin: CommandOrigin::FutureAgent(AgentProvenance::new(lease.0)),
        requests: commands
            .into_iter()
            .map(|command| CommandRequest::new(base, command))
            .collect(),
    }
}

/// 승인 전 미리보기(XIX.5.3).
#[derive(Debug, Clone)]
pub struct AgentPreview {
    /// 미리보기를 만든 문서 revision(적용 전과 같음).
    pub revision: DocumentRevision,
    /// 적용 전후 semantic diff(XIV.3).
    pub diff: DiffReport,
    /// 적용했을 때의 문서 검증 진단(`stellar validate`와 같은 함수).
    pub findings: Vec<Finding>,
    /// 적용했을 때의 change set.
    pub changes: ChangeSet,
    /// lease 범위 판정. 위반이면 승인해도 적용되지 않습니다.
    pub scope: Result<(), LeaseViolation>,
}

/// proposal을 문서 사본에 적용해 semantic diff·validation·lease 판정을 만듭니다. `state`는 읽기만 하므로 문서와
/// revision이 바뀌지 않습니다. `definitions`는 인스턴스 참조를 대조할 컴포넌트 정의입니다.
pub fn preview<'a>(
    state: &DocumentState,
    transaction: &CommandTransaction,
    table: &LeaseTable,
    definitions: impl FnOnce() -> Definitions<'a>,
) -> Result<AgentPreview, AttachedEditError> {
    let preview = lease::preview(state, transaction, table).map_err(|e| match e {
        PreviewError::Transaction(e) => AttachedEditError::Rejected(e.to_string()),
        PreviewError::Copy(e) => AttachedEditError::Internal(e),
    })?;
    let diff = diff_records(&[&preview.before], &[&preview.after])
        .map_err(|e| AttachedEditError::Internal(e.0))?;
    let findings = validate_records(state.model().document_id(), &preview.after, definitions);
    Ok(AgentPreview {
        revision: state.revision(),
        diff,
        findings,
        changes: preview.changes,
        scope: preview.scope,
    })
}

/// 승인된 proposal을 lease 판정과 함께 적용합니다(XIX.5.2). 범위 밖이면 문서를 바꾸지 않고 거부합니다.
pub fn apply_approved(
    state: &mut DocumentState,
    transaction: &CommandTransaction,
    table: &LeaseTable,
) -> Result<AppliedTransaction, AttachedEditError> {
    lease::execute_leased(state, transaction, table).map_err(|e| match e {
        LeasedExecutionError::Transaction(e) => AttachedEditError::Rejected(e.to_string()),
        LeasedExecutionError::Lease(violation) => AttachedEditError::OutOfScope(violation),
        LeasedExecutionError::Restore { violation, error } => {
            AttachedEditError::Internal(format!("{violation}; 되돌리기 실패: {error}"))
        }
    })
}

#[cfg(test)]
#[path = "edit_tests.rs"]
mod tests;
