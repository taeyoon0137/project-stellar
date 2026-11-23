//! 에이전트 변경 되돌리기 계획: 충돌 판정과 역 command 선별 (XIX.6.8, XIX.6.9, ADR-0100).
//!
//! 되돌리기는 history 포인터 이동이 아니라 새 User 출처 transaction입니다(ADR-0005 §4.2). 이 모듈은 그
//! transaction의 command 목록을 정하고 충돌을 분류합니다. 문서 상태는 읽기만 합니다. 계산은 문서 레코드로 만든
//! 사본(승인 전 미리보기 XIX.5.3과 같은 방식)에서 합니다.
//!
//! 1. batch를 적용 역순으로 처리합니다(세션 단위는 남은 batch 전부, 변경 단위는 하나).
//! 2. batch마다 지문 key의 현재 값을 적용 직후 지문과 비교합니다. 다르면 충돌입니다.
//!    - 소유 node가 없어졌으면 [`ConflictKind::NodeDeleted`], 그 밖은 [`ConflictKind::ChangedLater`].
//!    - 에이전트가 만든 node 아래에 그 batch가 만들거나 옮기지 않은 자식이 있으면
//!      [`ConflictKind::ChildAddedLater`], 그 batch 밖 node가 참조하면 [`ConflictKind::InboundReference`]
//!      (그 node의 레코드 key 충돌).
//! 3. 충돌 key는 사용자가 [`RevertItem`]으로 명시적으로 고른 경우만 복원합니다. 고르지 않은 충돌 key가 제외
//!    대상입니다. 사용자 콘텐츠가 함께 사라지는 구조 충돌도 기본값은 현재 유지입니다.
//! 4. batch의 역 transaction을 command 하나씩 사본에 적용하고, 그 command의 change set이 닿은 key(속성 경로, 또는
//!    지문이 실제로 바뀐 node 레코드 key)에 제외 key가 있으면 사본에서 되돌리고 뺍니다. 같은 command에만 있어
//!    함께 빠진 key는 [`ConflictKind::SharedCommand`], 앞 command가 빠져 적용할 수 없게 된 command는
//!    [`ConflictKind::Dependent`]입니다.
//! 5. 결과 command 목록을 현재 revision 기준 User 출처 transaction 하나로 만듭니다([`RevertPlan::transaction`]).
//!    같은 문서 상태에 적용하면 사본과 같은 결과입니다(command는 결정적이고 빠진 command는 사본에서 역으로
//!    되돌렸음). 전부 적용되거나 전부 미적용입니다(VII.1.6).
//!
//! 되돌리지 않는 key에 대한 충돌은 보고하지 않습니다(4에서 어떤 command도 닿지 않은 key의 충돌은 목록에서
//! 뺍니다). 구조 충돌과 삭제된 대상은 늘 보고합니다. lease 판정(다른 활성 lease와 겹치면 거부)은 적용 경로(`execute_leased`)와 미리보기가 합니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_core_types::revision::DocumentRevision;
use stellar_document::model::DocumentModel;

use crate::agent_batch::{
    fingerprint, record_key, AgentBatchId, AgentBatchRecord, ValueFingerprint,
};
use crate::change::node_keys;
use crate::command::Command;
use crate::id::{CommandId, CommandOrigin};
use crate::request::CommandRequest;
use crate::state::DocumentState;
use crate::transaction::CommandTransaction;

/// 사용자가 충돌인데도 복원하겠다고 고른 항목(batch와 지문 key).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RevertItem {
    /// batch.
    pub batch: AgentBatchId,
    /// 지문 key.
    pub key: PropertyPath,
}

/// 충돌 종류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictKind {
    /// 적용 뒤 다른 변경이 값을 바꿨습니다.
    ChangedLater,
    /// 바뀐 대상 node가 이미 삭제되었습니다.
    NodeDeleted {
        /// 삭제된 node.
        node: StableId,
    },
    /// 에이전트가 만든 node 아래에 이후 자식이 추가되었습니다.
    ChildAddedLater {
        /// 에이전트가 만든 node.
        parent: StableId,
        /// 이후 추가된 자식.
        child: StableId,
    },
    /// 에이전트가 만든 node를 batch 밖 node가 참조합니다(이후 참조·외부 inbound ref).
    InboundReference {
        /// 참조되는 node.
        target: StableId,
        /// 참조하는 node.
        from: StableId,
    },
    /// 충돌 key와 같은 command에 묶여 함께 되돌리지 않습니다.
    SharedCommand,
    /// 앞 command가 빠져 이 역 command를 적용할 수 없습니다.
    Dependent {
        /// 역 transaction 안 위치.
        index: usize,
        /// 거부 이유.
        reason: String,
    },
}

impl ConflictKind {
    /// 고정 코드.
    pub fn code(&self) -> &'static str {
        match self {
            ConflictKind::ChangedLater => "changed-later",
            ConflictKind::NodeDeleted { .. } => "node-deleted",
            ConflictKind::ChildAddedLater { .. } => "child-added-later",
            ConflictKind::InboundReference { .. } => "inbound-reference",
            ConflictKind::SharedCommand => "shared-command",
            ConflictKind::Dependent { .. } => "dependent",
        }
    }
}

/// 충돌 항목.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevertConflict {
    /// batch.
    pub batch: AgentBatchId,
    /// 지문 key(`Dependent`는 `None`).
    pub key: Option<PropertyPath>,
    /// 종류.
    pub kind: ConflictKind,
    /// 사용자가 복원하겠다고 골랐는지.
    pub included: bool,
}

/// 되돌리기 계획(문서를 바꾸지 않음).
#[derive(Debug, Clone, PartialEq)]
pub struct RevertPlan {
    /// 계획한 문서 revision. 적용은 이 revision에서만 합니다.
    pub revision: DocumentRevision,
    /// 대상 batch(적용 역순).
    pub batches: Vec<AgentBatchId>,
    /// 적용할 역 command(순서대로).
    pub commands: Vec<Command>,
    /// 충돌(제외·포함 모두).
    pub conflicts: Vec<RevertConflict>,
    /// 복원되는 항목.
    pub restored: Vec<RevertItem>,
}

impl RevertPlan {
    /// 적용할 command가 없는지.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// 제외된(복원하지 않는) 충돌.
    pub fn excluded(&self) -> impl Iterator<Item = &RevertConflict> + '_ {
        self.conflicts.iter().filter(|c| !c.included)
    }

    /// 계획한 revision 기준 User 출처 transaction. command가 없으면 `None`입니다.
    pub fn transaction(&self, id: CommandId) -> Option<CommandTransaction> {
        if self.commands.is_empty() {
            return None;
        }
        Some(CommandTransaction {
            id,
            origin: CommandOrigin::User,
            requests: self
                .commands
                .iter()
                .cloned()
                .map(|command| CommandRequest::new(self.revision, command))
                .collect(),
        })
    }
}

/// 계획 실패. 문서 상태는 바뀌지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevertError {
    /// 되돌릴 batch가 없습니다.
    NoBatches,
    /// 문서 사본을 만들 수 없습니다.
    Copy(String),
    /// 사본에서 뺀 command를 되돌릴 수 없습니다(내부 불일치).
    Internal(String),
}

impl fmt::Display for RevertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RevertError::NoBatches => f.write_str("되돌릴 에이전트 변경이 없습니다"),
            RevertError::Copy(e) => write!(f, "문서 사본을 만들 수 없습니다: {e}"),
            RevertError::Internal(e) => write!(f, "되돌리기 계획 내부 오류: {e}"),
        }
    }
}

impl std::error::Error for RevertError {}

/// 문서 레코드 사본(승인 전 미리보기와 같은 방식, 원본은 읽기만 함).
pub fn scratch_copy(state: &DocumentState) -> Result<DocumentState, String> {
    let mut records = state.model().to_records().map_err(|e| e.to_string())?;
    records.sort_canonical();
    let (copy, _) = DocumentModel::from_records(records).map_err(|e| format!("{e:?}"))?;
    Ok(DocumentState::with_revision(copy, state.revision()))
}

/// `batches`(적용 순서)를 되돌리는 계획을 만듭니다. 모듈 문서 규칙을 따릅니다.
pub fn plan_revert(
    state: &DocumentState,
    batches: &[&AgentBatchRecord],
    include: &BTreeSet<RevertItem>,
) -> Result<RevertPlan, RevertError> {
    if batches.is_empty() {
        return Err(RevertError::NoBatches);
    }
    let mut scratch = scratch_copy(state).map_err(RevertError::Copy)?;
    let mut plan = RevertPlan {
        revision: state.revision(),
        batches: Vec::new(),
        commands: Vec::new(),
        conflicts: Vec::new(),
        restored: Vec::new(),
    };
    for batch in batches.iter().rev() {
        plan.batches.push(batch.id);
        revert_batch(&mut scratch, batch, include, &mut plan)?;
    }
    Ok(plan)
}

fn is_definition_id(model: &DocumentModel, id: StableId) -> bool {
    let definition = model.definition();
    definition.id == id
        || definition
            .pages
            .iter()
            .any(|page| page.id == id || page.views.iter().any(|view| view.id == id))
}

fn moved_by(batch: &AgentBatchRecord) -> BTreeSet<StableId> {
    batch
        .changes
        .changed_properties()
        .into_iter()
        .filter(|path| path.keys() == [node_keys::PARENT])
        .map(|path| path.root())
        .collect()
}

/// batch 하나의 충돌(제외 여부 포함)을 분류합니다.
fn classify(
    model: &DocumentModel,
    batch: &AgentBatchRecord,
    include: &BTreeSet<RevertItem>,
) -> BTreeMap<PropertyPath, (ConflictKind, bool)> {
    let mut out = BTreeMap::new();
    let included = |key: &PropertyPath| {
        include.contains(&RevertItem {
            batch: batch.id,
            key: key.clone(),
        })
    };
    for (path, recorded) in &batch.fingerprints {
        if batch.current(model, path) == *recorded {
            continue;
        }
        let owner = batch.owners.get(path).copied().unwrap_or(path.root());
        let kind = if *recorded != ValueFingerprint::Absent
            && !model.contains(owner)
            && !is_definition_id(model, owner)
        {
            ConflictKind::NodeDeleted { node: owner }
        } else {
            ConflictKind::ChangedLater
        };
        out.insert(path.clone(), (kind, included(path)));
    }
    let moved = moved_by(batch);
    for created in &batch.created {
        if !model.contains(*created) {
            continue;
        }
        let key = record_key(*created);
        if out.contains_key(&key) {
            continue;
        }
        let later_child = model
            .children_in_order(*created)
            .into_iter()
            .find(|child| !batch.created.contains(child) && !moved.contains(child));
        let inbound = model
            .references()
            .inbound(*created)
            .into_iter()
            .find(|from| !batch.created.contains(from));
        let kind = match (later_child, inbound) {
            (Some(child), _) => Some(ConflictKind::ChildAddedLater {
                parent: *created,
                child,
            }),
            (None, Some(from)) => Some(ConflictKind::InboundReference {
                target: *created,
                from,
            }),
            (None, None) => None,
        };
        if let Some(kind) = kind {
            let chosen = included(&key);
            out.insert(key, (kind, chosen));
        }
    }
    out
}

fn revert_batch(
    scratch: &mut DocumentState,
    batch: &AgentBatchRecord,
    include: &BTreeSet<RevertItem>,
    plan: &mut RevertPlan,
) -> Result<(), RevertError> {
    let conflicts = classify(scratch.model(), batch, include);
    let excluded: BTreeSet<&PropertyPath> = conflicts
        .iter()
        .filter(|(_, (_, chosen))| !chosen)
        .map(|(key, _)| key)
        .collect();
    let record_keys: BTreeMap<StableId, PropertyPath> = batch
        .fingerprints
        .keys()
        .filter(|path| path.keys().is_empty() && path.tail().is_none())
        .filter(|path| !batch.owners.contains_key(*path))
        .filter(|path| !is_definition_id(scratch.model(), path.root()))
        .map(|path| (path.root(), path.clone()))
        .collect();
    let mut record_cache: BTreeMap<StableId, ValueFingerprint> = record_keys
        .iter()
        .map(|(node, key)| (*node, fingerprint(scratch.model(), key, None)))
        .collect();
    let mut touched_any: BTreeSet<PropertyPath> = BTreeSet::new();
    let mut shared: BTreeSet<PropertyPath> = BTreeSet::new();
    let mut dependent = Vec::new();
    for (index, request) in batch.inverse.requests.iter().enumerate() {
        let command = request.command.clone();
        let outcome = match scratch.execute(&CommandRequest::at(scratch, command.clone())) {
            Ok(outcome) => outcome,
            Err(error) => {
                dependent.push(RevertConflict {
                    batch: batch.id,
                    key: None,
                    kind: ConflictKind::Dependent {
                        index,
                        reason: error.to_string(),
                    },
                    included: false,
                });
                continue;
            }
        };
        let mut touched: BTreeSet<PropertyPath> = outcome
            .changes
            .changed_properties()
            .into_iter()
            .filter(|path| batch.fingerprints.contains_key(path))
            .collect();
        let nodes: BTreeSet<StableId> = outcome
            .changes
            .changed_nodes()
            .into_iter()
            .chain(outcome.changes.removed_nodes())
            .collect();
        for node in nodes {
            let Some(key) = record_keys.get(&node) else {
                continue;
            };
            let now = fingerprint(scratch.model(), key, None);
            if record_cache.insert(node, now) != Some(now) {
                touched.insert(key.clone());
            }
        }
        touched_any.extend(touched.iter().cloned());
        if touched.iter().any(|key| excluded.contains(key)) {
            scratch
                .execute(&CommandRequest::at(scratch, outcome.inverse))
                .map_err(|e| RevertError::Internal(e.to_string()))?;
            for key in &touched {
                if let Some(record) = record_keys.get(&key.root()).filter(|r| *r == key) {
                    record_cache.insert(key.root(), fingerprint(scratch.model(), record, None));
                }
            }
            shared.extend(
                touched
                    .into_iter()
                    .filter(|key| !conflicts.contains_key(key)),
            );
            continue;
        }
        plan.commands.push(command);
        plan.restored
            .extend(touched.into_iter().map(|key| RevertItem {
                batch: batch.id,
                key,
            }));
    }
    for (key, (kind, chosen)) in conflicts {
        if touched_any.contains(&key) || always_reported(&kind) {
            plan.conflicts.push(RevertConflict {
                batch: batch.id,
                key: Some(key),
                kind,
                included: chosen,
            });
        }
    }
    plan.conflicts
        .extend(shared.into_iter().map(|key| RevertConflict {
            batch: batch.id,
            key: Some(key),
            kind: ConflictKind::SharedCommand,
            included: false,
        }));
    plan.conflicts.extend(dependent);
    Ok(())
}

/// 구조 충돌과 삭제된 대상은 되돌리기 command가 닿지 않았어도(대상이 없어 적용할 수 없는 경우 포함) 목록에
/// 남깁니다.
fn always_reported(kind: &ConflictKind) -> bool {
    matches!(
        kind,
        ConflictKind::ChildAddedLater { .. }
            | ConflictKind::InboundReference { .. }
            | ConflictKind::NodeDeleted { .. }
    )
}

#[cfg(test)]
#[path = "agent_revert_tests.rs"]
mod tests;
