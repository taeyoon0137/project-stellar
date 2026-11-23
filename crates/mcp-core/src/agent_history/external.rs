//! 외부 변경 batch의 semantic diff·lease 겹침 판정과 반영 (XIX.7.1~XIX.7.3, ADR-0100).
//!
//! 1. [`analyze_external`]: 창이 마지막으로 맞춘 원본(`base`)과 외부 원본(`incoming`)의 semantic diff(ops
//!    `diff_records`, `stellar diff`와 같은 함수)를 계산하고, 바뀐 객체를 대상([`ExternalTarget`])으로 바꿔
//!    `stellar_commands::agent_external::overlapping_leases`로 lease 겹침을 판정합니다. 같은 입력으로
//!    `partition_external`이 창 문서에 바로 적용할 레코드 변경과 reconciliation으로 넘길 레코드를 나눕니다.
//!    창(`base`→`current`)과 외부(`base`→`incoming`)가 같은 객체·속성을 서로 다르게 바꾼 항목은 semantic conflict
//!    후보입니다.
//! 2. [`respond_to_external`]: 이 창의 세션 lease와 겹치면 그 세션을 종료 절차([`EndReason::LeaseConflict`], 자동
//!    rollback 없음)로 끝냅니다(XIX.7.2). 겹치지 않으면 세션을 유지합니다(XIX.7.3). 어느 경우든 창에서 바뀌지 않은
//!    레코드의 외부 변경만 System 출처 transaction으로 돌려주고(에이전트·사용자의 저장 전 변경을 덮어쓰지 않음),
//!    창과 외부가 모두 바꾼 레코드와 semantic conflict 후보는 reconciliation 입력(XIII.3.4)으로 돌려줍니다.
//!
//! 다른 프로세스가 쥔 lease와 겹치면 그 프로세스가 같은 외부 변경을 보고 자기 세션을 끝냅니다. 이 창은 그 lease를
//! `foreign_overlaps`로 보고만 합니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_ops::commands::agent_external::{
    overlapping_leases, partition_external, ExternalPartition, ExternalTarget, RecordTarget,
};
use stellar_ops::commands::agent_sink::AgentHistorySink;
use stellar_ops::commands::document::model::DocumentModel;
use stellar_ops::commands::document::records::DocumentRecords;
use stellar_ops::commands::id::CommandId;
use stellar_ops::commands::lock::lease::{LeaseId, LeaseTable};
use stellar_ops::commands::transaction::CommandTransaction;
use stellar_ops::core_types::ids::StableId;
use stellar_ops::diff::{diff_records, DiffEntry, DiffReport};
use stellar_ops::query::OpsValue;

use super::{AgentEditSessions, EndReason, SessionContext, SessionEndReport, SessionError};

/// 외부 변경 처리 실패. 문서는 바뀌지 않습니다.
#[derive(Debug, Clone, PartialEq)]
pub enum ExternalError {
    /// semantic snapshot·문서 모델을 만들 수 없습니다.
    Projection(String),
    /// 레코드 변경 집합을 만들 수 없습니다.
    Partition(String),
    /// 세션 종료 실패.
    Session(SessionError),
}

impl fmt::Display for ExternalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExternalError::Projection(e) => write!(f, "외부 변경을 분석할 수 없습니다: {e}"),
            ExternalError::Partition(e) => write!(f, "외부 변경 집합을 만들 수 없습니다: {e}"),
            ExternalError::Session(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ExternalError {}

/// 창과 외부가 같은 객체·속성을 서로 다르게 바꾼 semantic conflict 후보.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticConflictCandidate {
    /// 객체.
    pub object: StableId,
    /// 속성 경로 이름(객체 단위 변경이면 `None`).
    pub path: Option<String>,
    /// 창 쪽 값(삭제·객체 변경이면 `None`).
    pub local: Option<OpsValue>,
    /// 외부 쪽 값.
    pub external: Option<OpsValue>,
}

/// 외부 변경 분석.
#[derive(Debug, Clone)]
pub struct ExternalAnalysis {
    /// 외부 변경 batch(`base`→`incoming`) semantic diff.
    pub diff: DiffReport,
    /// 겹친 lease와 대상 node.
    pub overlaps: BTreeMap<LeaseId, BTreeSet<StableId>>,
    /// 레코드 변경 분할.
    pub partition: ExternalPartition,
    /// semantic conflict 후보.
    pub conflicts: Vec<SemanticConflictCandidate>,
}

impl ExternalAnalysis {
    /// lease와 겹치는지.
    pub fn overlaps_any(&self) -> bool {
        !self.overlaps.is_empty()
    }
}

fn model_of(records: &DocumentRecords) -> Result<DocumentModel, ExternalError> {
    let mut records = records.clone();
    records.sort_canonical();
    DocumentModel::from_records(records)
        .map(|(model, _)| model)
        .map_err(|e| ExternalError::Projection(format!("{e:?}")))
}

/// semantic diff 항목을 lease 판정 대상으로 바꿉니다. 문서 범위 객체(node·modifier·View·페이지)만 있습니다.
fn targets(
    entries: &[DiffEntry],
    models: [&DocumentModel; 2],
    records: [&DocumentRecords; 2],
) -> Vec<ExternalTarget> {
    let modifier_owner = |id: StableId| {
        records.iter().find_map(|r| {
            r.modifier_stacks
                .iter()
                .find(|s| s.entries().iter().any(|e| e.id.stable_id() == id))
                .map(|s| s.node)
        })
    };
    let mut out = Vec::new();
    for entry in entries {
        let id = entry.object;
        if models.iter().any(|m| m.contains(id)) {
            let moved = matches!(entry.kind, "move-object" | "reorder-object");
            out.push(if moved {
                ExternalTarget::Moved(id)
            } else {
                ExternalTarget::Node(id)
            });
            continue;
        }
        if let Some(owner) = modifier_owner(id) {
            out.push(ExternalTarget::Node(owner));
            continue;
        }
        for model in models {
            for page in &model.definition().pages {
                for view in &page.views {
                    if view.id == id || page.id == id {
                        out.push(ExternalTarget::View {
                            root_frame: view.root_frame,
                        });
                    }
                }
            }
        }
    }
    out
}

fn conflict_candidates(
    local: &DiffReport,
    external: &DiffReport,
) -> Vec<SemanticConflictCandidate> {
    let key = |e: &DiffEntry| (e.object, e.path.clone());
    let local_by: BTreeMap<(StableId, Option<String>), &DiffEntry> =
        local.entries.iter().map(|e| (key(e), e)).collect();
    let local_objects: BTreeSet<StableId> = local.entries.iter().map(|e| e.object).collect();
    let removed_in = |report: &DiffReport| -> BTreeSet<StableId> {
        report
            .entries
            .iter()
            .filter(|e| e.kind == "remove-object")
            .map(|e| e.object)
            .collect()
    };
    let (local_removed, external_removed) = (removed_in(local), removed_in(external));
    let mut out: Vec<SemanticConflictCandidate> = Vec::new();
    for entry in &external.entries {
        let same = local_by.get(&key(entry)).copied();
        let conflict = match same {
            // 같은 객체·속성을 양쪽이 다르게 바꿈.
            Some(l) => l.kind != entry.kind || l.new != entry.new || l.to != entry.to,
            // 한쪽이 지운 객체를 다른 쪽이 바꿈.
            None => {
                local_removed.contains(&entry.object)
                    || (external_removed.contains(&entry.object)
                        && local_objects.contains(&entry.object))
            }
        };
        let duplicate = out
            .last()
            .is_some_and(|c| c.object == entry.object && c.path == entry.path);
        if conflict && !duplicate {
            out.push(SemanticConflictCandidate {
                object: entry.object,
                path: entry.path.clone(),
                local: same.and_then(|l| l.new.clone()),
                external: entry.new.clone(),
            });
        }
    }
    out
}

/// 외부 변경 batch를 분석합니다(문서를 바꾸지 않음). `current`는 지금 창 문서입니다.
pub fn analyze_external(
    current: &DocumentModel,
    base: &DocumentRecords,
    incoming: &DocumentRecords,
    table: &LeaseTable,
) -> Result<ExternalAnalysis, ExternalError> {
    let diff = diff_records(&[base], &[incoming]).map_err(|e| ExternalError::Projection(e.0))?;
    let incoming_model = model_of(incoming)?;
    let overlaps = overlapping_leases(
        table,
        current,
        &incoming_model,
        targets(&diff.entries, [current, &incoming_model], [base, incoming]),
    );
    let partition = partition_external(base, current, incoming)
        .map_err(|e| ExternalError::Partition(e.to_string()))?;
    let current_records = current
        .to_records()
        .map_err(|e| ExternalError::Projection(e.to_string()))?;
    let local =
        diff_records(&[base], &[&current_records]).map_err(|e| ExternalError::Projection(e.0))?;
    let conflicts = conflict_candidates(&local, &diff);
    Ok(ExternalAnalysis {
        diff,
        overlaps,
        partition,
        conflicts,
    })
}

/// 외부 변경 처리 결정.
#[derive(Debug, Clone)]
pub struct ExternalDecision {
    /// 분석.
    pub analysis: ExternalAnalysis,
    /// 겹쳐서 끝낸 이 창의 세션.
    pub ended: Vec<SessionEndReport>,
    /// 다른 프로세스가 쥔 겹친 lease(그 프로세스가 끝냄).
    pub foreign_overlaps: Vec<LeaseId>,
    /// 창에 적용할 외부 변경(System 출처, 창에서 바뀌지 않은 레코드만). 없으면 `None`.
    pub apply: Option<CommandTransaction>,
    /// reconciliation(XIII.3.4)으로 넘길, 창과 외부가 모두 바꾼 레코드.
    pub divergent: Vec<RecordTarget>,
}

/// 외부 변경에 대응합니다(모듈 문서 2). 문서는 바꾸지 않고, 반영할 transaction을 돌려줍니다. 호출자는 그것을 창
/// history의 일반 외부 반영 경로로 적용하고 출처 색인에 `observe`로 알립니다.
#[allow(clippy::too_many_arguments)]
pub fn respond_to_external(
    sessions: &mut AgentEditSessions,
    ctx: &mut SessionContext<'_>,
    history: &mut dyn AgentHistorySink,
    base: &DocumentRecords,
    incoming: &DocumentRecords,
    table: &LeaseTable,
    entry_id: CommandId,
    apply_id: CommandId,
    now_ms: u64,
) -> Result<ExternalDecision, ExternalError> {
    let analysis = analyze_external(history.state().model(), base, incoming, table)?;
    let mut ended = Vec::new();
    let mut foreign_overlaps = Vec::new();
    for lease in analysis.overlaps.keys() {
        match sessions.session_of_lease(*lease) {
            Some(session) => ended.push(
                sessions
                    .end(
                        session,
                        EndReason::LeaseConflict,
                        ctx,
                        history,
                        entry_id,
                        now_ms,
                    )
                    .map_err(ExternalError::Session)?,
            ),
            None => foreign_overlaps.push(*lease),
        }
    }
    let revision = history.state().revision();
    let apply = (!analysis.partition.applicable.is_empty()).then(|| {
        analysis
            .partition
            .applicable
            .clone()
            .into_transaction(apply_id, revision)
    });
    let divergent = analysis.partition.divergent.clone();
    Ok(ExternalDecision {
        analysis,
        ended,
        foreign_overlaps,
        apply,
        divergent,
    })
}

#[cfg(test)]
#[path = "external_tests.rs"]
mod tests;
