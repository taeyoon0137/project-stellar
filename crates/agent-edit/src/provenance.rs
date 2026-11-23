//! 에이전트 변경 출처 색인 `AgentChangeProvenance`와 출처 표시 상태 전이 (XIX.6.7, XIX.6.10, ADR-0100).
//!
//! 창 세션(WindowSessionId) 메모리에만 있는 색인입니다. 적용된 batch마다 다음을 둡니다(ADR-0005 §4.2).
//!
//! - AgentSessionId, CLI가 보고한 세션 표시 이름, batch 순번, lease(편집 세션), history sequence(그룹 확정 뒤).
//! - [`AgentBatchRecord`]: 정·역 transaction(되돌리기 입력, XIX.6.1), change set의 StableId·PropertyPath, 생성·삭제
//!   node, StableId+PropertyPath별 적용 직후 값 지문.
//! - 결과(진행 중·완료·중단)와 표시 상태.
//!
//! 직렬화 경로가 없습니다. 프로젝트 파일, Git, generation metadata, 복구 snapshot, 진단, 텔레메트리에 들어가지
//! 않으며, 창을 닫으면 [`AgentChangeProvenance::close_window`]로 WindowHistory와 함께 버립니다.
//!
//! ## 표시 상태(XIX.6.10)
//!
//! | 상태 | 뜻 | 들어오는 경로 |
//! |---|---|---|
//! | `PendingReview` | 검토 대기(표시 중) | batch 적용 |
//! | `Kept` | 확인됨: 사용자가 "유지"를 골라 표시만 지움 | [`AgentChangeProvenance::keep`] |
//! | `ChangedLater` | 표시된 속성을 사용자·다른 세션·외부 변경이 다시 바꿈 | [`AgentChangeProvenance::observe`] |
//! | `Undone` | 창 Undo로 에이전트 작업 그룹 entry를 취소함 | [`AgentChangeProvenance::on_history`] |
//! | `Reverted` | 되돌리기를 확정함 | [`AgentChangeProvenance::mark_reverted`] |
//!
//! history entry가 만든 전이(`Undone`, `Reverted`)는 그 entry의 Undo·Redo에 따라 이전 상태로 돌아갑니다. 그룹
//! entry의 Undo는 `Undone`으로, Redo는 이전 상태로 갑니다. 되돌리기 entry의 Undo는 이전 상태로, Redo는 다시
//! `Reverted`로 갑니다. `ChangedLater`는 같은 속성의 값이 적용 직후 지문과 다시 같아지면(그 변경의 Undo)
//! `PendingReview`로 돌아갑니다. "유지"와 되돌리기 확정은 앱 사용자 UI 경로 전용입니다([`UserUiAction`]). MCP
//! tool·`stellar` 명령에는 이 색인을 바꾸는 수단이 없습니다(ADR-0005 §3).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_ops::commands::agent_batch::{AgentBatchId, AgentBatchRecord};
use stellar_ops::commands::change::ChangeSet;
use stellar_ops::commands::document::model::DocumentModel;
use stellar_ops::commands::id::CommandOrigin;
use stellar_ops::commands::lock::lease::LeaseId;
use stellar_ops::commands::transaction::{AppliedTransaction, CommandTransaction};
use stellar_ops::core_types::ids::StableId;
use stellar_ops::core_types::property::PropertyPath;

use crate::permission::AgentSessionId;

/// batch 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BatchOutcome {
    /// 작업 그룹이 아직 확정되지 않았습니다.
    InProgress,
    /// 정상 완료(CompleteAgentEdit)로 확정됨.
    Completed,
    /// 세션 종료 절차로 확정됨(중단).
    Interrupted,
}

/// 출처 표시 상태(모듈 문서 표).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DisplayState {
    /// 검토 대기(표시 중).
    PendingReview,
    /// 확인됨(유지, 표시 지움).
    Kept,
    /// 이후 변경됨.
    ChangedLater,
    /// 실행 취소됨.
    Undone,
    /// 되돌림.
    Reverted,
}

impl DisplayState {
    /// 고정 이름.
    pub fn name(self) -> &'static str {
        match self {
            DisplayState::PendingReview => "pending-review",
            DisplayState::Kept => "kept",
            DisplayState::ChangedLater => "changed-later",
            DisplayState::Undone => "undone",
            DisplayState::Reverted => "reverted",
        }
    }

    /// 표시(레이어·인스펙터·overlay 표지)가 있는 상태인지.
    pub fn is_shown(self) -> bool {
        matches!(
            self,
            DisplayState::PendingReview | DisplayState::ChangedLater
        )
    }
}

/// 창 history 재생 방향.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryDirection {
    /// Undo.
    Undo,
    /// Redo.
    Redo,
}

/// 앱 사용자 UI 동작 표식. 앱의 사용자 UI 처리기만 만듭니다(MCP·CLI 경로에는 만드는 곳이 없음).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserUiAction(());

impl UserUiAction {
    /// 사용자 UI 동작에서 만듭니다(앱 사용자 UI 전용).
    pub fn from_user_interface() -> UserUiAction {
        UserUiAction(())
    }
}

/// 출처 색인 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvenanceError {
    /// 모르는 batch.
    UnknownBatch(AgentBatchId),
    /// 그 세션의 batch가 없습니다.
    UnknownSession(AgentSessionId),
    /// 지금 상태에서는 할 수 없습니다(이미 유지·실행 취소·되돌림).
    NotShown(AgentBatchId, DisplayState),
    /// 세션 batch 순번이 한계입니다.
    SequenceOverflow,
}

impl fmt::Display for ProvenanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProvenanceError::UnknownBatch(id) => {
                write!(f, "batch {}#{}이 없습니다", id.session, id.seq)
            }
            ProvenanceError::UnknownSession(id) => write!(f, "세션 {}의 변경이 없습니다", id.0),
            ProvenanceError::NotShown(id, state) => {
                write!(
                    f,
                    "batch {}#{}는 {} 상태입니다",
                    id.session,
                    id.seq,
                    state.name()
                )
            }
            ProvenanceError::SequenceOverflow => f.write_str("batch 순번이 한계에 도달했습니다"),
        }
    }
}

impl std::error::Error for ProvenanceError {}

/// batch 하나의 출처.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchProvenance {
    /// 에이전트 세션.
    pub session: AgentSessionId,
    /// CLI가 보고한 세션 표시 이름.
    pub session_name: String,
    /// 편집 lease(그룹 단위).
    pub lease: LeaseId,
    /// 그룹 확정 history entry sequence(확정 전 `None`).
    pub history_sequence: Option<u64>,
    /// batch 기록(정·역 transaction, change set, 지문).
    pub record: AgentBatchRecord,
    /// 결과.
    pub outcome: BatchOutcome,
    display: DisplayState,
    changed_later: BTreeSet<PropertyPath>,
    /// history entry가 만든 전이: (entry sequence, 이전 상태).
    transitions: Vec<(u64, DisplayState)>,
}

impl BatchProvenance {
    /// batch id.
    pub fn id(&self) -> AgentBatchId {
        self.record.id
    }

    /// 표시 상태.
    pub fn display(&self) -> DisplayState {
        self.display
    }

    /// 이후 변경된 지문 key.
    pub fn changed_later(&self) -> &BTreeSet<PropertyPath> {
        &self.changed_later
    }

    /// 바뀐 StableId(삭제 포함).
    pub fn changed_nodes(&self) -> BTreeSet<StableId> {
        self.record.touched_nodes()
    }

    /// 바뀐 PropertyPath(VII.1.4 결과).
    pub fn changed_properties(&self) -> Vec<PropertyPath> {
        self.record.changes.changed_properties()
    }

    fn push(&mut self, sequence: u64, next: DisplayState) {
        self.transitions.push((sequence, self.display));
        self.display = next;
    }

    fn pop(&mut self, sequence: u64) {
        if self.transitions.last().is_some_and(|(s, _)| *s == sequence) {
            if let Some((_, previous)) = self.transitions.pop() {
                self.display = previous;
            }
        }
    }
}

/// 되돌리기·유지 범위.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvenanceScope {
    /// 변경(batch) 하나.
    Batch(AgentBatchId),
    /// 세션의 남은 변경 전부.
    Session(AgentSessionId),
}

/// 창 하나의 출처 색인.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AgentChangeProvenance {
    batches: Vec<BatchProvenance>,
    next_seq: BTreeMap<AgentSessionId, u32>,
    reverts: BTreeMap<u64, Vec<AgentBatchId>>,
}

impl AgentChangeProvenance {
    /// 빈 색인(창을 열 때).
    pub fn new() -> AgentChangeProvenance {
        AgentChangeProvenance::default()
    }

    /// 기록된 batch 수.
    pub fn len(&self) -> usize {
        self.batches.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.batches.is_empty()
    }

    /// 모든 batch(적용 순서).
    pub fn batches(&self) -> &[BatchProvenance] {
        &self.batches
    }

    /// batch 하나.
    pub fn get(&self, id: AgentBatchId) -> Option<&BatchProvenance> {
        self.batches.iter().find(|b| b.record.id == id)
    }

    /// 세션의 batch(적용 순서).
    pub fn session_batches(
        &self,
        session: AgentSessionId,
    ) -> impl Iterator<Item = &BatchProvenance> + '_ {
        self.batches.iter().filter(move |b| b.session == session)
    }

    /// 표시가 있는(검토 대기·이후 변경됨) node 집합.
    pub fn shown_nodes(&self) -> BTreeSet<StableId> {
        self.batches
            .iter()
            .filter(|b| b.display.is_shown())
            .flat_map(BatchProvenance::changed_nodes)
            .collect()
    }

    /// 승인 batch 적용 직후 기록합니다. `after`는 적용 후 문서입니다.
    pub fn record_batch(
        &mut self,
        session: AgentSessionId,
        session_name: &str,
        lease: LeaseId,
        forward: CommandTransaction,
        applied: AppliedTransaction,
        after: &DocumentModel,
    ) -> Result<AgentBatchId, ProvenanceError> {
        let next = self.next_seq.entry(session).or_insert(1);
        let seq = *next;
        *next = seq
            .checked_add(1)
            .ok_or(ProvenanceError::SequenceOverflow)?;
        let id = AgentBatchId {
            session: session.0,
            seq,
        };
        self.batches.push(BatchProvenance {
            session,
            session_name: session_name.to_string(),
            lease,
            history_sequence: None,
            record: AgentBatchRecord::capture(id, forward, applied, after),
            outcome: BatchOutcome::InProgress,
            display: DisplayState::PendingReview,
            changed_later: BTreeSet::new(),
            transitions: Vec::new(),
        });
        Ok(id)
    }

    /// 세션의 확정되지 않은 batch(적용 순서).
    pub fn in_progress(&self, session: AgentSessionId) -> Vec<&AgentBatchRecord> {
        self.session_batches(session)
            .filter(|b| b.outcome == BatchOutcome::InProgress)
            .map(|b| &b.record)
            .collect()
    }

    /// 세션의 확정되지 않은 batch를 `outcome`으로 확정하고 그룹 history sequence를 적습니다. 확정한 batch 수.
    pub fn settle(
        &mut self,
        session: AgentSessionId,
        outcome: BatchOutcome,
        history_sequence: Option<u64>,
    ) -> usize {
        let mut count = 0;
        for batch in self
            .batches
            .iter_mut()
            .filter(|b| b.session == session && b.outcome == BatchOutcome::InProgress)
        {
            batch.outcome = outcome;
            batch.history_sequence = history_sequence;
            count += 1;
        }
        count
    }

    fn session_of_lease(&self, lease: LeaseId) -> Option<AgentSessionId> {
        self.batches
            .iter()
            .find(|b| b.lease == lease)
            .map(|b| b.session)
    }

    /// 에이전트 batch 적용·되돌리기 밖의 문서 변경(사용자 편집, 다른 세션 batch, 외부 반영, Undo·Redo)을 알립니다.
    /// 표시된 속성 값이 적용 직후와 달라졌으면 "이후 변경됨", 같아졌으면 "검토 대기"로 돌아갑니다. 같은 세션의 변경은
    /// 보지 않습니다.
    pub fn observe(&mut self, changes: &ChangeSet, origin: CommandOrigin, model: &DocumentModel) {
        let own = origin
            .agent_provenance()
            .and_then(|p| self.session_of_lease(LeaseId(p.edit_session())));
        let paths: BTreeSet<PropertyPath> = changes.changed_properties().into_iter().collect();
        let nodes: BTreeSet<StableId> = changes
            .changed_nodes()
            .into_iter()
            .chain(changes.removed_nodes())
            .collect();
        for batch in &mut self.batches {
            if Some(batch.session) == own || !batch.display.is_shown() {
                continue;
            }
            let related: Vec<PropertyPath> = batch
                .record
                .fingerprints
                .keys()
                .filter(|key| {
                    paths.contains(*key)
                        || (key.keys().is_empty() && nodes.contains(&key.root()))
                        || batch
                            .record
                            .owners
                            .get(*key)
                            .is_some_and(|owner| nodes.contains(owner))
                        || !model.contains(key.root()) && nodes.contains(&key.root())
                })
                .cloned()
                .collect();
            for key in related {
                if batch.record.current(model, &key) == batch.record.fingerprints[&key] {
                    batch.changed_later.remove(&key);
                } else {
                    batch.changed_later.insert(key);
                }
            }
            batch.display = if batch.changed_later.is_empty() {
                DisplayState::PendingReview
            } else {
                DisplayState::ChangedLater
            };
        }
    }

    /// 창 history의 Undo·Redo를 알립니다(`sequence`는 재생한 entry). 에이전트 그룹·되돌리기 entry이면 표시 상태를
    /// 전이합니다. 문서 값 변화는 따로 [`AgentChangeProvenance::observe`]로 알립니다.
    pub fn on_history(&mut self, sequence: u64, direction: HistoryDirection) {
        let reverted = self.reverts.get(&sequence).cloned().unwrap_or_default();
        for batch in &mut self.batches {
            if batch.history_sequence == Some(sequence) {
                match direction {
                    HistoryDirection::Undo => batch.push(sequence, DisplayState::Undone),
                    HistoryDirection::Redo => batch.pop(sequence),
                }
            }
            if reverted.contains(&batch.record.id) {
                match direction {
                    HistoryDirection::Undo => batch.pop(sequence),
                    HistoryDirection::Redo => batch.push(sequence, DisplayState::Reverted),
                }
            }
        }
    }

    fn scope_ids(&self, scope: ProvenanceScope) -> Result<Vec<AgentBatchId>, ProvenanceError> {
        match scope {
            ProvenanceScope::Batch(id) => {
                self.get(id).ok_or(ProvenanceError::UnknownBatch(id))?;
                Ok(vec![id])
            }
            ProvenanceScope::Session(session) => {
                let ids: Vec<AgentBatchId> =
                    self.session_batches(session).map(|b| b.record.id).collect();
                if ids.is_empty() {
                    return Err(ProvenanceError::UnknownSession(session));
                }
                Ok(ids)
            }
        }
    }

    /// "유지": 문서를 바꾸지 않고 표시만 지웁니다(앱 사용자 UI 전용). 범위 안 표시된 batch 수.
    pub fn keep(
        &mut self,
        scope: ProvenanceScope,
        _ui: UserUiAction,
    ) -> Result<usize, ProvenanceError> {
        let ids = self.scope_ids(scope)?;
        if let ProvenanceScope::Batch(id) = scope {
            let batch = self.get(id).ok_or(ProvenanceError::UnknownBatch(id))?;
            if !batch.display.is_shown() {
                return Err(ProvenanceError::NotShown(id, batch.display));
            }
        }
        let mut count = 0;
        for batch in &mut self.batches {
            if ids.contains(&batch.record.id) && batch.display.is_shown() {
                batch.display = DisplayState::Kept;
                count += 1;
            }
        }
        Ok(count)
    }

    /// 되돌릴 batch 기록(적용 순서). 표시된(검토 대기·이후 변경됨) batch만 대상입니다. 변경 하나를 고른 경우 그
    /// batch가 표시 중이 아니면 오류입니다. 세션 범위에서 남은 batch가 없으면 빈 목록입니다.
    pub fn revertible(
        &self,
        scope: ProvenanceScope,
    ) -> Result<Vec<&AgentBatchRecord>, ProvenanceError> {
        let ids = self.scope_ids(scope)?;
        if let ProvenanceScope::Batch(id) = scope {
            let batch = self.get(id).ok_or(ProvenanceError::UnknownBatch(id))?;
            if !batch.display.is_shown() {
                return Err(ProvenanceError::NotShown(id, batch.display));
            }
        }
        Ok(self
            .batches
            .iter()
            .filter(|b| ids.contains(&b.record.id) && b.display.is_shown())
            .map(|b| &b.record)
            .collect())
    }

    /// 되돌리기 확정을 기록합니다(`sequence`는 "에이전트 변경 되돌리기" entry, 앱 사용자 UI 전용).
    pub fn mark_reverted(&mut self, batches: &[AgentBatchId], sequence: u64, _ui: UserUiAction) {
        for batch in &mut self.batches {
            if batches.contains(&batch.record.id) {
                batch.push(sequence, DisplayState::Reverted);
            }
        }
        self.reverts.insert(sequence, batches.to_vec());
    }

    /// 창을 닫을 때 색인을 버립니다(WindowHistory와 함께). 버린 batch 수.
    pub fn close_window(&mut self) -> usize {
        let count = self.batches.len();
        *self = AgentChangeProvenance::new();
        count
    }
}

#[cfg(test)]
#[path = "provenance_tests.rs"]
mod tests;
