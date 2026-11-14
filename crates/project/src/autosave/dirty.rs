//! 자동 저장 dirty 상태 (XIII.1.2).
//!
//! 기술 아키텍처 22.1·23.2: 자동 저장은 **완료된 document CommandTransaction**만 저장 대상으로 봅니다.
//! 선택 변경은 프로젝트 파일과 Git 변경에 넣지 않고, viewport(scroll·zoom)는 창 상태이며, 진행 중인
//! preview(drag·scrub)는 아직 완료되지 않은 편집입니다. 셋 모두 dirty 상태를 만들지 않습니다.
//!
//! [`DirtyTrackedHistory`]는 창 history([`WindowHistory`])를 **소유**하고, 문서·선택을 바꾸는 모든 경로를
//! 자신의 method로만 열어 둡니다. 그래서 change set의 종류를 호출자가 따로 알려 주지 않습니다(preview
//! change set을 완료 편집으로, 완료 편집을 빠뜨리는 실수를 API가 막음). 분류는 실행한 history 경로가
//! 정합니다:
//!
//! | 경로 | dirty |
//! |---|---|
//! | `record_document`·`record_grouped`(완료 transaction) | 효과가 있으면 dirty |
//! | `end_preview`가 항목을 기록(drag·scrub 확정) | dirty |
//! | `undo`·`redo`의 문서 항목 | 효과가 있으면 dirty |
//! | `record_selection`, `undo`·`redo`의 선택 항목 | 없음 |
//! | `begin_*`·`update_preview`(진행 중), `cancel_preview` | 없음 |
//! | viewport | 없음(창 view 상태라 history·문서 상태 밖이며 이 type에 들어오는 경로가 없음) |
//!
//! dirty는 저장 대상인 **마지막 완료 revision**([`DocumentRevision`])으로 기록합니다. 저장이 그 revision
//! 이상을 저장했다고 알리면([`DirtyTrackedHistory::mark_saved`]) clean이 됩니다. 저장 계획
//! ([`DirtyTrackedHistory::plan_save`])은 dirty가 아니면 문서를 직렬화하지도 않고
//! [`AutosavePlan::Clean`]입니다. preview가 진행 중이면 문서 메모리에 미완료 변경이 섞여 있으므로
//! [`AutosavePlan::PreviewInProgress`]로 캡처를 미룹니다(캡처 시점 정책은 XIII.1.3).

use std::fmt;

use stellar_commands::change::ChangeSet;
use stellar_commands::selection::SetSelection;
use stellar_commands::state::DocumentState;
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::clock::MonotonicClock;
use stellar_core_types::ids::StableId;
use stellar_core_types::revision::DocumentRevision;
use stellar_document::model::DocumentModel;
use stellar_history::entry::{GroupingKey, HistoryError, HistorySequence};
use stellar_history::history::UndoOutcome;
use stellar_history::window::WindowHistory;
use stellar_storage::generation::loader::LoadedGeneration;
use stellar_storage::save::plan::{build_save_plan, SavePlan, SavePlanError};

use crate::document_io::serialize::{serialize_document, SerializeError};

/// dirty 판정과 함께 쓰는 창 history.
#[derive(Debug, Clone)]
pub struct DirtyTrackedHistory {
    history: WindowHistory,
    /// 마지막으로 저장(또는 로드)한 문서 revision.
    saved: DocumentRevision,
    /// 저장되지 않은 마지막 완료 revision. clean이면 `None`.
    dirty: Option<DocumentRevision>,
}

/// 자동 저장 계획 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutosavePlan {
    /// 저장할 완료 편집이 없음(문서를 직렬화하지 않음).
    Clean,
    /// 완료 편집이 있지만 preview가 진행 중이라 캡처를 미룸.
    PreviewInProgress,
    /// 저장 계획.
    Ready {
        /// 이 계획이 담은 문서 revision(저장 성공 후 [`DirtyTrackedHistory::mark_saved`]에 넘김).
        revision: DocumentRevision,
        /// 저장 계획(V.5.1).
        plan: SavePlan,
    },
}

/// 자동 저장 계획 실패.
#[derive(Debug, Clone, PartialEq)]
pub enum AutosavePlanError {
    /// 문서 직렬화 실패.
    Serialize(SerializeError),
    /// 저장 계획 실패.
    Plan(SavePlanError),
}

impl fmt::Display for AutosavePlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AutosavePlanError::Serialize(e) => write!(f, "자동 저장 직렬화 실패: {e}"),
            AutosavePlanError::Plan(e) => write!(f, "자동 저장 계획 실패: {e}"),
        }
    }
}

impl std::error::Error for AutosavePlanError {}

impl DirtyTrackedHistory {
    /// 창 history를 감쌉니다. `saved`는 이 창 문서가 마지막으로 저장(또는 로드)된 revision이고 clean으로
    /// 시작합니다.
    pub fn new(history: WindowHistory, saved: DocumentRevision) -> DirtyTrackedHistory {
        DirtyTrackedHistory {
            history,
            saved,
            dirty: None,
        }
    }

    /// 창 history(읽기 전용).
    pub fn history(&self) -> &WindowHistory {
        &self.history
    }

    /// 창 history를 꺼냅니다(창 종료).
    pub fn into_history(self) -> WindowHistory {
        self.history
    }

    /// 저장되지 않은 완료 편집이 있는지.
    pub fn is_dirty(&self) -> bool {
        self.dirty.is_some()
    }

    /// 저장되지 않은 마지막 완료 revision.
    pub fn dirty_revision(&self) -> Option<DocumentRevision> {
        self.dirty
    }

    /// 마지막으로 저장(또는 로드)한 revision.
    pub fn saved_revision(&self) -> DocumentRevision {
        self.saved
    }

    /// preview(drag·scrub)가 진행 중인지.
    pub fn preview_active(&self) -> bool {
        self.history.history().preview_active()
    }

    /// 완료 편집 하나를 기록합니다. 효과가 없는 change set은 dirty를 만들지 않습니다.
    fn note_completed(&mut self, changes: &ChangeSet) {
        if !changes.is_empty() {
            self.dirty = Some(changes.new_revision());
        }
    }

    /// 완료된 document transaction을 실행·기록합니다(dirty).
    pub fn record_document(
        &mut self,
        doc: &mut DocumentState,
        transaction: &CommandTransaction,
        grouping: Option<GroupingKey>,
    ) -> Result<ChangeSet, HistoryError> {
        let changes = self.history.record_document(doc, transaction, grouping)?;
        self.note_completed(&changes);
        Ok(changes)
    }

    /// 연속 입력 grouping으로 완료 transaction을 기록합니다(dirty).
    pub fn record_grouped<C: MonotonicClock>(
        &mut self,
        doc: &mut DocumentState,
        transaction: &CommandTransaction,
        key: GroupingKey,
        clock: &C,
    ) -> Result<ChangeSet, HistoryError> {
        let changes = self.history.record_grouped(doc, transaction, key, clock)?;
        self.note_completed(&changes);
        Ok(changes)
    }

    /// 선택 변경을 기록합니다(dirty 없음: 선택은 파일에 저장하지 않음).
    pub fn record_selection(
        &mut self,
        doc: &DocumentState,
        event: &SetSelection,
    ) -> Result<Option<HistorySequence>, HistoryError> {
        self.history.record_selection(doc, event)
    }

    /// pointer gesture preview를 시작합니다(dirty 없음).
    pub fn begin_pointer_gesture(&mut self, gesture: u64) -> Result<(), HistoryError> {
        self.history.begin_pointer_gesture(gesture)
    }

    /// slider·scrub preview를 시작합니다(dirty 없음).
    pub fn begin_scrub(&mut self, control: u64) -> Result<(), HistoryError> {
        self.history.begin_scrub(control)
    }

    /// preview를 교체합니다(dirty 없음: 완료되지 않은 편집).
    pub fn update_preview(
        &mut self,
        doc: &mut DocumentState,
        transaction: &CommandTransaction,
    ) -> Result<ChangeSet, HistoryError> {
        self.history.update_preview(doc, transaction)
    }

    /// preview를 확정합니다. 항목이 기록되면 확정 시점 revision으로 dirty입니다.
    pub fn end_preview(
        &mut self,
        doc: &DocumentState,
    ) -> Result<Option<HistorySequence>, HistoryError> {
        let sequence = self.history.end_preview()?;
        if sequence.is_some() {
            self.dirty = Some(doc.revision());
        }
        Ok(sequence)
    }

    /// preview를 취소합니다(dirty 없음: 문서를 preview 전 상태로 되돌림).
    pub fn cancel_preview(&mut self, doc: &mut DocumentState) -> Result<(), HistoryError> {
        self.history.cancel_preview(doc)
    }

    /// 마지막 항목을 되돌립니다. 문서 항목이면 dirty, 선택 항목이면 dirty 없음.
    pub fn undo(&mut self, doc: &mut DocumentState) -> Result<Option<UndoOutcome>, HistoryError> {
        let outcome = self.history.undo(doc)?;
        self.note_outcome(outcome.as_ref());
        Ok(outcome)
    }

    /// 되돌린 항목을 다시 적용합니다. 문서 항목이면 dirty, 선택 항목이면 dirty 없음.
    pub fn redo(&mut self, doc: &mut DocumentState) -> Result<Option<UndoOutcome>, HistoryError> {
        let outcome = self.history.redo(doc)?;
        self.note_outcome(outcome.as_ref());
        Ok(outcome)
    }

    fn note_outcome(&mut self, outcome: Option<&UndoOutcome>) {
        if let Some(changes) = outcome.and_then(|outcome| outcome.changes.as_ref()) {
            self.note_completed(changes);
        }
    }

    /// `revision`까지 저장했음을 기록합니다. dirty revision이 그 이하면 clean이 됩니다(저장 중에 이어진
    /// 완료 편집은 dirty로 남음).
    pub fn mark_saved(&mut self, revision: DocumentRevision) {
        if self.dirty.is_some_and(|dirty| dirty <= revision) {
            self.dirty = None;
        }
        self.saved = self.saved.max(revision);
    }

    /// 디스크의 새 generation을 다시 불러온 뒤 clean 상태로 되돌립니다(`revision`은 다시 불러온 문서
    /// revision).
    pub fn reset_clean(&mut self, revision: DocumentRevision) {
        self.dirty = None;
        self.saved = revision;
    }

    /// 자동 저장 계획을 만듭니다.
    ///
    /// clean이면 문서를 직렬화하지 않고 [`AutosavePlan::Clean`], preview 중이면
    /// [`AutosavePlan::PreviewInProgress`]입니다. 그 밖에는 `documents`(이 창 문서를 포함한 프로젝트의
    /// 메모리 문서 전체)를 직렬화해 `base` 위의 저장 계획을 만듭니다. `doc`은 이 창의 문서 상태이며 계획이
    /// 담는 revision을 정합니다.
    pub fn plan_save(
        &self,
        doc: &DocumentState,
        documents: &[&DocumentModel],
        base: Option<&LoadedGeneration>,
        project_id: StableId,
        schema_version: u32,
    ) -> Result<AutosavePlan, AutosavePlanError> {
        if !self.is_dirty() {
            return Ok(AutosavePlan::Clean);
        }
        if self.preview_active() {
            return Ok(AutosavePlan::PreviewInProgress);
        }
        let mut states = Vec::new();
        let mut roots = Vec::with_capacity(documents.len());
        for model in documents {
            let serialized = serialize_document(model).map_err(AutosavePlanError::Serialize)?;
            states.extend(serialized.to_dirty_states());
            roots.push(serialized.document_id);
        }
        let plan = build_save_plan(base, project_id, schema_version, &states, roots)
            .map_err(AutosavePlanError::Plan)?;
        Ok(AutosavePlan::Ready {
            revision: doc.revision(),
            plan,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use stellar_commands::command::Command;
    use stellar_commands::id::{CommandId, CommandOrigin, WindowSessionId};
    use stellar_commands::selection::Selection;
    use stellar_core_types::order_key::OrderKey;
    use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
    use stellar_document::frame::FramePayload;
    use stellar_document::records::{NodePayload, NodeRecord};

    fn project() -> StableId {
        StableId::new_v7(9, 9, 9)
    }

    /// 결정론적 id.
    pub(crate) fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn frame(id: u64, parent: Option<u64>, key: u8) -> NodeRecord {
        NodeRecord {
            id: sid(id),
            parent: parent.map(sid),
            order_key: OrderKey::from_bytes(vec![key]).unwrap(),
            visible: true,
            locked: false,
            display_name: format!("n{id}"),
            payload: NodePayload::Frame(FramePayload::default()),
        }
    }

    /// 문서 id `sid(base + 1)`, root `sid(base + 10)` 아래 `base + 11`·`base + 12` Frame.
    pub(crate) fn document(base: u64) -> DocumentModel {
        let mut definition = DocumentDefinition::new(sid(base + 1), String::new());
        let mut page = PageDefinition::new(sid(base + 2), String::new());
        page.add_view(ViewDefinition::new(
            sid(base + 3),
            String::new(),
            sid(base + 10),
        ))
        .unwrap();
        definition.add_page(page).unwrap();
        let mut model = DocumentModel::new(definition);
        model.insert_node(frame(base + 10, None, 0x10)).unwrap();
        model
            .insert_node(frame(base + 11, Some(base + 10), 0x10))
            .unwrap();
        model
            .insert_node(frame(base + 12, Some(base + 10), 0x20))
            .unwrap();
        model
    }

    /// root(10) 아래 11·12.
    fn state() -> DocumentState {
        DocumentState::new(document(0))
    }

    /// 현재 revision 기준 command 하나짜리 사용자 transaction.
    pub(crate) fn tx(doc: &DocumentState, id: u64, command: Command) -> CommandTransaction {
        CommandTransaction::new(CommandId::from_stable_id(sid(id)), CommandOrigin::User)
            .with_command(doc.revision(), command)
    }

    fn tracked(doc: &DocumentState) -> DirtyTrackedHistory {
        let window = WindowSessionId::from_stable_id(sid(500));
        DirtyTrackedHistory::new(WindowHistory::new(window), doc.revision())
    }

    fn autosave_plan(tracked: &DirtyTrackedHistory, doc: &DocumentState) -> AutosavePlan {
        tracked
            .plan_save(doc, &[doc.model()], None, project(), 1)
            .unwrap()
    }

    /// `ids` 선택 event(첫 id가 primary).
    pub(crate) fn select(ids: &[u64]) -> SetSelection {
        SetSelection {
            selection: Selection::new(ids.iter().map(|n| sid(*n)), ids.first().map(|n| sid(*n))),
        }
    }

    #[test]
    fn a_thousand_selection_changes_create_no_save_plan() {
        let doc = state();
        let mut tracked = tracked(&doc);
        let mut recorded = 0;
        for i in 0..1_000u64 {
            let ids: &[u64] = match i % 3 {
                0 => &[11],
                1 => &[12],
                _ => &[11, 12],
            };
            if tracked
                .record_selection(&doc, &select(ids))
                .unwrap()
                .is_some()
            {
                recorded += 1;
            }
        }
        // 선택은 history 항목으로 기록되지만(undo 대상) dirty·저장 계획은 만들지 않습니다.
        assert_eq!(recorded, 1_000);
        assert_eq!(tracked.history().history().len(), 1_000);
        assert!(!tracked.is_dirty());
        assert_eq!(autosave_plan(&tracked, &doc), AutosavePlan::Clean);
        assert_eq!(doc.revision(), DocumentRevision(0));
    }

    #[test]
    fn undo_and_redo_of_selection_entries_create_no_save_plan() {
        let mut doc = state();
        let mut tracked = tracked(&doc);
        for ids in [[11u64], [12]] {
            tracked.record_selection(&doc, &select(&ids)).unwrap();
        }
        assert!(tracked.undo(&mut doc).unwrap().is_some());
        assert!(tracked.redo(&mut doc).unwrap().is_some());
        assert_eq!(autosave_plan(&tracked, &doc), AutosavePlan::Clean);
    }

    #[test]
    fn preview_updates_and_cancel_create_no_save_plan_but_commit_does() {
        let mut doc = state();
        let mut tracked = tracked(&doc);
        // 진행 중인 drag: 문서 메모리는 바뀌지만 완료되지 않은 편집.
        tracked.begin_pointer_gesture(1).unwrap();
        for step in 0..100u64 {
            let t = tx(
                &doc,
                1_000 + step,
                Command::rename_node(sid(11), format!("drag {step}")),
            );
            tracked.update_preview(&mut doc, &t).unwrap();
            assert!(!tracked.is_dirty());
            assert_eq!(autosave_plan(&tracked, &doc), AutosavePlan::Clean);
        }
        tracked.cancel_preview(&mut doc).unwrap();
        assert!(!tracked.is_dirty());
        assert_eq!(autosave_plan(&tracked, &doc), AutosavePlan::Clean);

        // 확정한 drag는 완료 편집입니다.
        tracked.begin_pointer_gesture(2).unwrap();
        let t = tx(&doc, 2_000, Command::rename_node(sid(11), "dragged"));
        tracked.update_preview(&mut doc, &t).unwrap();
        assert!(tracked.end_preview(&doc).unwrap().is_some());
        assert_eq!(tracked.dirty_revision(), Some(doc.revision()));
        assert!(matches!(
            autosave_plan(&tracked, &doc),
            AutosavePlan::Ready { revision, .. } if revision == doc.revision()
        ));
    }

    #[test]
    fn completed_transactions_create_a_save_plan_and_saving_clears_it() {
        let mut doc = state();
        let mut tracked = tracked(&doc);
        let t = tx(&doc, 3_000, Command::rename_node(sid(12), "renamed"));
        let changes = tracked.record_document(&mut doc, &t, None).unwrap();
        assert_eq!(tracked.dirty_revision(), Some(changes.new_revision()));
        let AutosavePlan::Ready { revision, plan } = autosave_plan(&tracked, &doc) else {
            panic!("완료 편집은 저장 계획을 만들어야 함");
        };
        assert_eq!(revision, doc.revision());
        assert!(!plan.dirty_chunks.is_empty());
        assert_eq!(plan.target_generation, 1);

        // 저장 중 이어진 완료 편집은 dirty로 남습니다.
        let t = tx(&doc, 3_001, Command::rename_node(sid(12), "again"));
        tracked.record_document(&mut doc, &t, None).unwrap();
        tracked.mark_saved(revision);
        assert!(tracked.is_dirty());
        tracked.mark_saved(doc.revision());
        assert!(!tracked.is_dirty());
        assert_eq!(tracked.saved_revision(), doc.revision());
        assert_eq!(autosave_plan(&tracked, &doc), AutosavePlan::Clean);

        // undo한 문서 항목은 완료 편집(디스크와 다른 메모리 상태)이라 dirty입니다.
        assert!(tracked.undo(&mut doc).unwrap().is_some());
        assert_eq!(tracked.dirty_revision(), Some(doc.revision()));
    }

    #[test]
    fn preview_in_progress_defers_capture_of_earlier_completed_edits() {
        let mut doc = state();
        let mut tracked = tracked(&doc);
        let t = tx(&doc, 4_000, Command::rename_node(sid(11), "done"));
        tracked.record_document(&mut doc, &t, None).unwrap();
        tracked.begin_scrub(7).unwrap();
        let t = tx(&doc, 4_001, Command::rename_node(sid(11), "scrubbing"));
        tracked.update_preview(&mut doc, &t).unwrap();
        assert_eq!(
            autosave_plan(&tracked, &doc),
            AutosavePlan::PreviewInProgress
        );
        tracked.cancel_preview(&mut doc).unwrap();
        assert!(matches!(
            autosave_plan(&tracked, &doc),
            AutosavePlan::Ready { .. }
        ));
    }

    #[test]
    fn failed_transactions_create_no_dirty_state() {
        let mut doc = state();
        let mut tracked = tracked(&doc);
        // 없는 node 이름 변경은 실패하고 문서·dirty가 그대로입니다.
        let t = tx(&doc, 5_000, Command::rename_node(sid(99), "missing"));
        assert!(tracked.record_document(&mut doc, &t, None).is_err());
        assert!(!tracked.is_dirty());
        assert_eq!(autosave_plan(&tracked, &doc), AutosavePlan::Clean);
    }
}
