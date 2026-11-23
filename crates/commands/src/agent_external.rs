//! 외부 변경 batch와 에이전트 lease 겹침 판정, 에이전트 작업을 유지한 외부 변경 반영 (XIX.7.1~XIX.7.3,
//! ADR-0100).
//!
//! 다른 프로세스(`stellar apply`, 편집기, Git)가 바꾼 문서 원본(`incoming`)이 들어오면 창은 마지막으로 맞춘
//! 원본(`base`)과 지금 창 문서(`current`, 아직 저장하지 않은 에이전트·사용자 변경 포함)를 함께 봅니다.
//!
//! - **겹침**([`overlapping_leases`]): 외부 변경이 닿은 대상이 lease subtree 안이면(적용 전 창 문서나 외부 원본에서
//!   자신 또는 조상이 lease root) 그 lease와 겹칩니다. 위치가 바뀐 node가 lease root의 진조상이거나, View root가
//!   lease subtree를 품으면 겹칩니다. 소속은 [`LeaseTable::holder`], 조상 관계는 [`SubtreeLock::overlaps`]로 판정해
//!   사용자 편집 판정과 같은 규칙을 씁니다.
//! - **분할**([`partition_external`]): 외부 변경 집합(`base`→`incoming`, [`ApplyExternalSnapshot::between`]) 중 창에서
//!   바뀌지 않은 레코드의 변경만 바로 적용할 수 있습니다(`applicable`). 창에서도 바뀐 레코드(`divergent`)는
//!   덮어쓰지 않고 semantic reconciliation(XIII.3.4) 입력으로 넘깁니다. 그래서 에이전트 변경이 자동으로 지워지지
//!   않습니다.

use std::collections::{BTreeMap, BTreeSet};

use stellar_core_types::ids::StableId;
use stellar_document::model::DocumentModel;
use stellar_document::records::DocumentRecords;

use crate::command::CommandError;
use crate::lock::lease::{LeaseId, LeaseTable, SubtreeLock};
use crate::snapshot_apply::{ApplyExternalSnapshot, RecordChange};

/// 외부 변경이 닿은 대상.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExternalTarget {
    /// node의 속성·추가·삭제.
    Node(StableId),
    /// node의 부모·순서 변경.
    Moved(StableId),
    /// View(root Frame) 추가·삭제·이동.
    View {
        /// View root Frame.
        root_frame: StableId,
    },
}

/// 외부 변경이 겹친 lease와 그 대상 node.
pub fn overlapping_leases(
    table: &LeaseTable,
    current: &DocumentModel,
    incoming: &DocumentModel,
    targets: impl IntoIterator<Item = ExternalTarget>,
) -> BTreeMap<LeaseId, BTreeSet<StableId>> {
    let mut out: BTreeMap<LeaseId, BTreeSet<StableId>> = BTreeMap::new();
    let subtrees: Vec<(LeaseId, &SubtreeLock)> = table
        .iter()
        .flat_map(|lease| lease.subtrees.iter().map(move |s| (lease.id, s)))
        .filter(|(_, s)| s.document == current.document_id())
        .collect();
    let mut contains = |lease: LeaseId, node: StableId| {
        out.entry(lease).or_default().insert(node);
    };
    for target in targets {
        let (node, ancestor_check) = match target {
            ExternalTarget::Node(node) => (node, false),
            ExternalTarget::Moved(node) => (node, true),
            ExternalTarget::View { root_frame } => (root_frame, true),
        };
        for model in [current, incoming] {
            if let Some(lease) = table.holder(model, node) {
                contains(lease, node);
            }
            if !ancestor_check {
                continue;
            }
            let Some(lock) = SubtreeLock::capture(model, node) else {
                continue;
            };
            for (lease, subtree) in &subtrees {
                if lock.overlaps(subtree) {
                    contains(*lease, node);
                }
            }
        }
    }
    out
}

/// 레코드 변경의 대상 key(같은 대상끼리 짝지음).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RecordTarget {
    /// 문서 표시 이름.
    DocumentName,
    /// 페이지.
    Page(StableId),
    /// View.
    View(StableId),
    /// node 레코드.
    Node(StableId),
    /// node의 modifier stack.
    ModifierStack(StableId),
    /// node의 명시 layout record.
    Layout(StableId),
    /// 문서 Text style 표.
    TextStyles,
    /// 문서 Component Context(한 단위, ADR-0114).
    Context,
}

impl RecordTarget {
    /// 변경의 대상.
    pub fn of(change: &RecordChange) -> RecordTarget {
        match change {
            RecordChange::DocumentName { .. } => RecordTarget::DocumentName,
            RecordChange::Page { id, .. } => RecordTarget::Page(*id),
            RecordChange::View { id, .. } => RecordTarget::View(*id),
            RecordChange::Node { id, .. } => RecordTarget::Node(*id),
            RecordChange::ModifierStack { node, .. } => RecordTarget::ModifierStack(*node),
            RecordChange::Layout { node, .. } => RecordTarget::Layout(*node),
            RecordChange::TextStyles { .. } => RecordTarget::TextStyles,
            RecordChange::Context { .. } => RecordTarget::Context,
        }
    }
}

/// 외부 변경 분할 결과.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalPartition {
    /// 창에서 바뀌지 않은 레코드의 외부 변경(지금 창 문서에 그대로 적용 가능, 출처 System).
    pub applicable: ApplyExternalSnapshot,
    /// 창과 외부가 모두 바꾼 레코드(semantic reconciliation 입력, 적용하지 않음).
    pub divergent: Vec<RecordTarget>,
    /// 창에서만 바뀐 레코드(에이전트·사용자의 저장 전 변경, 그대로 유지).
    pub local_only: Vec<RecordTarget>,
}

/// 외부 변경을 창 문서에 바로 적용할 부분과 reconciliation으로 넘길 부분으로 나눕니다. `base`는 창이 마지막으로
/// 맞춘 원본, `incoming`은 외부 원본입니다. 셋은 같은 문서여야 합니다.
pub fn partition_external(
    base: &DocumentRecords,
    current: &DocumentModel,
    incoming: &DocumentRecords,
) -> Result<ExternalPartition, CommandError> {
    let mut base = base.clone();
    base.sort_canonical();
    let (base_model, _) =
        DocumentModel::from_records(base).map_err(|_| CommandError::InvalidRequest {
            reason: "기준 원본을 문서로 만들 수 없습니다",
        })?;
    let external = ApplyExternalSnapshot::between(&base_model, incoming)?;
    let local = ApplyExternalSnapshot::between(&base_model, &current.to_records()?)?;
    let local_targets: BTreeSet<RecordTarget> =
        local.changes.iter().map(RecordTarget::of).collect();
    let external_targets: BTreeSet<RecordTarget> =
        external.changes.iter().map(RecordTarget::of).collect();
    let (divergent, applicable): (Vec<RecordChange>, Vec<RecordChange>) = external
        .changes
        .into_iter()
        .partition(|change| local_targets.contains(&RecordTarget::of(change)));
    Ok(ExternalPartition {
        applicable: ApplyExternalSnapshot {
            changes: applicable,
        },
        divergent: divergent.iter().map(RecordTarget::of).collect(),
        local_only: local_targets
            .difference(&external_targets)
            .copied()
            .collect(),
    })
}

#[cfg(test)]
#[path = "agent_external_tests.rs"]
mod tests;
