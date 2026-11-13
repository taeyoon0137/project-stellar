//! 창별 history (VII.5).
//!
//! 같은 프로젝트를 여러 창에서 열면 각 창은 독립된 인메모리 문서 상태와 undo/redo stack을 가집니다
//! (기술 아키텍처 §22.2·§23.2). [`WindowHistory`]는 창 하나([`WindowSessionId`])의 history와 그 창의
//! view 상태(선택)를 함께 소유하며, 다른 창의 항목을 담지 않습니다. [`WindowHistories`]는 열린 창마다
//! 하나씩 만들고 창을 닫으면 폐기합니다.

use std::collections::BTreeMap;

use stellar_commands::change::ChangeSet;
use stellar_commands::id::{CommandOrigin, WindowSessionId};
use stellar_commands::selection::{Selection, SelectionPolicy, SetSelection, WindowViewState};
use stellar_commands::state::DocumentState;
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::clock::MonotonicClock;

use crate::entry::{GroupingKey, HistoryError, HistorySequence};
use crate::grouping::HistoryGroupingPolicy;
use crate::history::{History, UndoOutcome};

/// 창 하나의 history와 view 상태.
#[derive(Debug, Clone)]
pub struct WindowHistory {
    window: WindowSessionId,
    history: History,
    view: WindowViewState,
    selection_policy: SelectionPolicy,
    /// preview 시작 시점 선택(확정 항목의 선택 복원 정보).
    preview_selection: Option<Selection>,
}

impl WindowHistory {
    /// 빈 history와 빈 선택으로 창을 엽니다.
    pub fn new(window: WindowSessionId) -> WindowHistory {
        WindowHistory::with_policy(window, HistoryGroupingPolicy::default())
    }

    /// grouping 정책을 지정해 엽니다.
    pub fn with_policy(window: WindowSessionId, policy: HistoryGroupingPolicy) -> WindowHistory {
        WindowHistory {
            window,
            history: History::with_policy(policy),
            view: WindowViewState::new(window),
            selection_policy: SelectionPolicy::default(),
            preview_selection: None,
        }
    }

    /// 창 id.
    pub fn window(&self) -> WindowSessionId {
        self.window
    }

    /// 이 창의 history.
    pub fn history(&self) -> &History {
        &self.history
    }

    /// 이 창의 view 상태(선택).
    pub fn view(&self) -> &WindowViewState {
        &self.view
    }

    /// 선택 정책.
    pub fn selection_policy(&self) -> SelectionPolicy {
        self.selection_policy
    }

    /// 문서 transaction을 실행·기록하고 삭제된 node를 이 창 선택에서 정리합니다(system event).
    pub fn record_document(
        &mut self,
        doc: &mut DocumentState,
        transaction: &CommandTransaction,
        grouping: Option<GroupingKey>,
    ) -> Result<ChangeSet, HistoryError> {
        let before = self.view.selection().clone();
        let changes = self.history.record_document(doc, transaction, grouping)?;
        self.view.prune_removed(&changes);
        self.history
            .note_selection(before, self.view.selection().clone());
        Ok(changes)
    }

    /// 연속 입력 grouping 기록(VII.4.3·VII.4.4).
    pub fn record_grouped<C: MonotonicClock>(
        &mut self,
        doc: &mut DocumentState,
        transaction: &CommandTransaction,
        key: GroupingKey,
        clock: &C,
    ) -> Result<ChangeSet, HistoryError> {
        let before = self.view.selection().clone();
        let changes = self.history.record_grouped(doc, transaction, key, clock)?;
        self.view.prune_removed(&changes);
        self.history
            .note_selection(before, self.view.selection().clone());
        Ok(changes)
    }

    /// 선택 변경을 기록합니다(VII.4.6).
    pub fn record_selection(
        &mut self,
        doc: &DocumentState,
        event: &SetSelection,
    ) -> Result<Option<HistorySequence>, HistoryError> {
        self.history.record_selection(
            &mut self.view,
            doc,
            event,
            self.selection_policy,
            CommandOrigin::User,
        )
    }

    /// pointer gesture 시작(VII.4.2).
    pub fn begin_pointer_gesture(&mut self, gesture: u64) -> Result<(), HistoryError> {
        self.history.begin_pointer_gesture(gesture)?;
        self.preview_selection = Some(self.view.selection().clone());
        Ok(())
    }

    /// slider·scrub 시작(VII.4.5).
    pub fn begin_scrub(&mut self, control: u64) -> Result<(), HistoryError> {
        self.history.begin_scrub(control)?;
        self.preview_selection = Some(self.view.selection().clone());
        Ok(())
    }

    /// preview 교체(VII.4.2).
    pub fn update_preview(
        &mut self,
        doc: &mut DocumentState,
        transaction: &CommandTransaction,
    ) -> Result<ChangeSet, HistoryError> {
        let changes = self.history.update_preview(doc, transaction)?;
        self.view.prune_removed(&changes);
        Ok(changes)
    }

    /// preview 확정(VII.4.2).
    pub fn end_preview(&mut self) -> Result<Option<HistorySequence>, HistoryError> {
        let sequence = self.history.end_preview()?;
        let before = self.preview_selection.take();
        if let (Some(_), Some(before)) = (sequence, before) {
            self.history
                .note_selection(before, self.view.selection().clone());
        }
        Ok(sequence)
    }

    /// preview 취소(VII.4.5).
    pub fn cancel_preview(&mut self, doc: &mut DocumentState) -> Result<(), HistoryError> {
        self.history.cancel_preview(doc)?;
        self.preview_selection = None;
        Ok(())
    }

    /// 이 창에서 마지막으로 되돌린 항목을 다시 적용합니다(VII.5.3).
    pub fn redo(&mut self, doc: &mut DocumentState) -> Result<Option<UndoOutcome>, HistoryError> {
        self.history
            .redo(doc, &mut self.view, self.selection_policy)
    }

    /// undo하고, 선택 복원 직후 `on_selection_restored`(UI 선택 변경 알림)를 reentrancy guard 안에서
    /// 부릅니다(VII.5.5).
    pub fn undo_with<F>(
        &mut self,
        doc: &mut DocumentState,
        on_selection_restored: F,
    ) -> Result<Option<UndoOutcome>, HistoryError>
    where
        F: FnMut(&mut History, &mut WindowViewState, &DocumentState),
    {
        self.history.undo_with(
            doc,
            &mut self.view,
            self.selection_policy,
            on_selection_restored,
        )
    }

    /// redo하고 선택 복원 직후 hook을 reentrancy guard 안에서 부릅니다(VII.5.5).
    pub fn redo_with<F>(
        &mut self,
        doc: &mut DocumentState,
        on_selection_restored: F,
    ) -> Result<Option<UndoOutcome>, HistoryError>
    where
        F: FnMut(&mut History, &mut WindowViewState, &DocumentState),
    {
        self.history.redo_with(
            doc,
            &mut self.view,
            self.selection_policy,
            on_selection_restored,
        )
    }

    /// 이 창의 마지막 항목 하나를 이 창의 문서·선택에 되돌립니다(VII.5.2).
    ///
    /// 역 transaction은 **이 창의 인메모리 문서 상태**(`doc`)에만 적용합니다. 창끼리 문서 모델을 공유하지
    /// 않으므로(§22.2) 다른 창의 문서·revision과 무관하며, 역 transaction은 이 창 문서의 현재 revision으로
    /// rebase해 실행합니다.
    pub fn undo(&mut self, doc: &mut DocumentState) -> Result<Option<UndoOutcome>, HistoryError> {
        self.history
            .undo(doc, &mut self.view, self.selection_policy)
    }
}

/// 열린 창별 history 모음.
#[derive(Debug, Clone, Default)]
pub struct WindowHistories {
    windows: BTreeMap<WindowSessionId, WindowHistory>,
}

impl WindowHistories {
    /// 빈 모음.
    pub fn new() -> WindowHistories {
        WindowHistories::default()
    }

    /// 창의 history를 엽니다. 이미 열려 있으면 기존 것을 돌려줍니다.
    pub fn open(&mut self, window: WindowSessionId) -> &mut WindowHistory {
        self.windows
            .entry(window)
            .or_insert_with(|| WindowHistory::new(window))
    }

    /// 열린 창의 history.
    pub fn get(&self, window: WindowSessionId) -> Option<&WindowHistory> {
        self.windows.get(&window)
    }

    /// 열린 창의 history(변경).
    pub fn get_mut(&mut self, window: WindowSessionId) -> Option<&mut WindowHistory> {
        self.windows.get_mut(&window)
    }

    /// 창 `saved_by`가 새 generation을 저장했음을 알립니다(VII.5.3).
    ///
    /// 다른 창의 history는 **즉시 폐기하지 않습니다** — 그 창들은 저장 충돌·재로드 정책(§22.2, XII.7)에서
    /// 처리하도록 stale 후보 목록만 돌려줍니다. 저장한 창 자신은 목록에 없습니다.
    pub fn on_generation_saved(&self, saved_by: WindowSessionId) -> Vec<WindowSessionId> {
        self.windows
            .keys()
            .copied()
            .filter(|window| *window != saved_by)
            .collect()
    }

    /// 창을 닫습니다(VII.5.4): 그 창의 history(undo·redo)와 선택을 폐기합니다. history는 세션 메모리
    /// 전용이라 프로젝트 파일에 아무것도 기록하지 않습니다(§23.1). 열려 있지 않았으면 `false`.
    pub fn close(&mut self, window: WindowSessionId) -> bool {
        self.windows.remove(&window).is_some()
    }

    /// 열린 창 수.
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    /// 열린 창이 없는지.
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::test_fixtures::{sid, small_state, tx};
    use std::collections::BTreeSet;
    use stellar_commands::command::Command;
    use stellar_commands::id::CommandId;
    use stellar_commands::selection::Selection;

    fn window(n: u64) -> WindowSessionId {
        WindowSessionId::from_stable_id(sid(500 + n))
    }

    fn forward_ids(history: &WindowHistory) -> BTreeSet<CommandId> {
        history
            .history()
            .entries()
            .iter()
            .filter_map(|entry| match &entry.payload {
                crate::entry::HistoryPayload::Document { forward, .. } => Some(forward.id),
                crate::entry::HistoryPayload::Selection { .. } => None,
            })
            .collect()
    }

    #[test]
    fn interleaved_edits_keep_window_stacks_separate() {
        // 검증 기준(VII.5.1): 창 A/B 교차 편집에서 stack 분리.
        let mut histories = WindowHistories::new();
        let (a, b) = (window(1), window(2));
        let mut doc_a = small_state();
        let mut doc_b = small_state();
        histories.open(a);
        histories.open(b);
        assert_eq!(histories.len(), 2);

        let mut a_ids = BTreeSet::new();
        let mut b_ids = BTreeSet::new();
        for step in 0..6u64 {
            let ta = tx(
                &doc_a,
                1_000 + step,
                Command::rename_node(sid(11), format!("A{step}")),
            );
            a_ids.insert(ta.id);
            histories
                .get_mut(a)
                .unwrap()
                .record_document(&mut doc_a, &ta, None)
                .unwrap();
            let tb = tx(
                &doc_b,
                2_000 + step,
                Command::rename_node(sid(12), format!("B{step}")),
            );
            b_ids.insert(tb.id);
            histories
                .get_mut(b)
                .unwrap()
                .record_document(&mut doc_b, &tb, None)
                .unwrap();
            if step % 2 == 0 {
                let event = SetSelection::new(Selection::new([sid(13)], None));
                histories
                    .get_mut(a)
                    .unwrap()
                    .record_selection(&doc_a, &event)
                    .unwrap();
            }
        }
        let history_a = histories.get(a).unwrap();
        let history_b = histories.get(b).unwrap();
        assert_eq!(forward_ids(history_a), a_ids, "A에는 A 항목만");
        assert_eq!(forward_ids(history_b), b_ids, "B에는 B 항목만");
        assert_eq!(
            history_a.history().len(),
            7,
            "A: 편집 6 + 선택 1(같은 선택 반복은 미기록)"
        );
        assert_eq!(history_b.history().len(), 6);
        let seq_b: Vec<u64> = history_b
            .history()
            .entries()
            .iter()
            .map(|e| e.sequence.get())
            .collect();
        assert_eq!(seq_b, (1..=6).collect::<Vec<u64>>(), "창마다 독립 sequence");
        assert_eq!(history_a.view().selection().ids(), &[sid(13)]);
        assert!(history_b.view().selection().is_empty(), "선택도 창별");

        // A의 undo는 B의 stack·문서에 영향이 없습니다.
        let b_before = doc_b.model().to_records().unwrap();
        histories.get_mut(a).unwrap().undo(&mut doc_a).unwrap();
        assert_eq!(histories.get(b).unwrap().history().len(), 6);
        assert_eq!(doc_b.model().to_records().unwrap(), b_before);
        // 이미 열린 창을 다시 열면 기존 history.
        assert_eq!(histories.open(a).history().len(), 6);
        assert_eq!(histories.open(a).window(), a);
    }

    fn snapshot(doc: &DocumentState) -> String {
        stellar_commands::document::test_support::semantic_snapshot::model_snapshot_json(
            doc.model(),
        )
        .unwrap()
    }

    #[test]
    fn undo_applies_to_each_windows_own_document() {
        // 검증 기준(VII.5.2): 각 창 독립 snapshot fixture 통과.
        use stellar_commands::document::frame::keys as frame_keys;
        use stellar_commands::node_ops::{DeleteSubtree, MoveNode};
        use stellar_commands::property::{PropertyValue, SetProperty};
        use stellar_core_types::order_key::OrderKey;
        use stellar_core_types::value::Size2;

        let mut histories = WindowHistories::new();
        let (a, b) = (window(1), window(2));
        let mut doc_a = small_state();
        let mut doc_b = small_state();
        let original = snapshot(&doc_a);

        // A: 크기 변경 후 subtree 삭제. B: 이동.
        let resize = tx(
            &doc_a,
            1_000,
            Command::SetProperty(SetProperty {
                node: sid(12),
                key: frame_keys::SIZE,
                value: PropertyValue::Size(Size2::new(64.0, 32.0).unwrap()),
            }),
        );
        histories
            .open(a)
            .record_document(&mut doc_a, &resize, None)
            .unwrap();
        let a_resized = snapshot(&doc_a);
        let delete = tx(
            &doc_a,
            1_001,
            Command::DeleteSubtree(DeleteSubtree { root: sid(11) }),
        );
        histories
            .open(a)
            .record_document(&mut doc_a, &delete, None)
            .unwrap();
        let a_final = snapshot(&doc_a);
        let mv = tx(
            &doc_b,
            2_000,
            Command::MoveNode(MoveNode {
                node: sid(13),
                new_parent: sid(12),
                order_key: OrderKey::from_bytes(vec![0x10]).unwrap(),
            }),
        );
        histories
            .open(b)
            .record_document(&mut doc_b, &mv, None)
            .unwrap();
        let b_final = snapshot(&doc_b);
        assert_ne!(
            doc_a.revision(),
            doc_b.revision(),
            "창마다 revision이 따로 흐름"
        );

        // A undo 2회: A 문서만 단계별 원복, B 문서 그대로.
        histories
            .get_mut(a)
            .unwrap()
            .undo(&mut doc_a)
            .unwrap()
            .unwrap();
        assert_eq!(snapshot(&doc_a), a_resized);
        assert_eq!(snapshot(&doc_b), b_final);
        histories
            .get_mut(a)
            .unwrap()
            .undo(&mut doc_a)
            .unwrap()
            .unwrap();
        assert_eq!(snapshot(&doc_a), original);
        assert_eq!(snapshot(&doc_b), b_final);
        // B undo: B 문서만 원복, A 문서 그대로.
        histories
            .get_mut(b)
            .unwrap()
            .undo(&mut doc_b)
            .unwrap()
            .unwrap();
        assert_eq!(snapshot(&doc_b), original);
        assert_eq!(snapshot(&doc_a), original);
        assert_ne!(a_final, b_final);
        // 더 되돌릴 것이 없음.
        assert_eq!(
            histories.get_mut(a).unwrap().undo(&mut doc_a).unwrap(),
            None
        );
    }

    #[test]
    fn redo_branch_is_discarded_by_new_commit_but_not_by_other_window_save() {
        // 검증 기준(VII.5.3): branch history fixture 통과.
        let mut histories = WindowHistories::new();
        let (a, b) = (window(1), window(2));
        let mut doc_a = small_state();
        let mut doc_b = small_state();
        histories.open(b);
        let original = snapshot(&doc_a);
        let mut states = vec![original.clone()];
        for (i, name) in ["e1", "e2", "e3"].iter().enumerate() {
            let t = tx(
                &doc_a,
                1_000 + i as u64,
                Command::rename_node(sid(11), *name),
            );
            histories
                .open(a)
                .record_document(&mut doc_a, &t, None)
                .unwrap();
            states.push(snapshot(&doc_a));
        }
        let wa = histories.get_mut(a).unwrap();
        wa.undo(&mut doc_a).unwrap();
        wa.undo(&mut doc_a).unwrap();
        assert_eq!(snapshot(&doc_a), states[1]);
        assert_eq!(wa.history().redo_len(), 2);

        // 다른 창(B)의 저장은 A의 history를 폐기하지 않습니다.
        let stale = histories.on_generation_saved(b);
        assert_eq!(stale, vec![a]);
        let wa = histories.get_mut(a).unwrap();
        assert_eq!(wa.history().len(), 1);
        assert_eq!(wa.history().redo_len(), 2);
        // B 자신의 편집도 A의 redo에 영향이 없습니다.
        let tb = tx(&doc_b, 2_000, Command::rename_node(sid(12), "B"));
        histories
            .get_mut(b)
            .unwrap()
            .record_document(&mut doc_b, &tb, None)
            .unwrap();
        let wa = histories.get_mut(a).unwrap();
        assert_eq!(wa.history().redo_len(), 2);

        // redo 한 번 → e2 상태, 다시 undo.
        wa.redo(&mut doc_a).unwrap().unwrap();
        assert_eq!(snapshot(&doc_a), states[2]);
        wa.undo(&mut doc_a).unwrap();
        assert_eq!(snapshot(&doc_a), states[1]);

        // 같은 창에서 새 command(e4) 확정 → redo 갈래(e2, e3) 폐기.
        let t = tx(&doc_a, 1_010, Command::rename_node(sid(12), "e4"));
        wa.record_document(&mut doc_a, &t, None).unwrap();
        assert_eq!(wa.history().redo_len(), 0);
        assert_eq!(wa.redo(&mut doc_a).unwrap(), None);
        assert_eq!(wa.history().len(), 2, "e1, e4");
        // 끝까지 undo하면 원래 문서.
        wa.undo(&mut doc_a).unwrap();
        wa.undo(&mut doc_a).unwrap();
        assert_eq!(snapshot(&doc_a), original);
        // 끝까지 redo하면 e1 → e4 순서로 재적용.
        wa.redo(&mut doc_a).unwrap();
        wa.redo(&mut doc_a).unwrap();
        assert_eq!(doc_a.model().record(sid(11)).unwrap().display_name, "e1");
        assert_eq!(doc_a.model().record(sid(12)).unwrap().display_name, "e4");
    }

    #[test]
    fn selection_entries_redo_and_new_selection_discards_branch() {
        let mut history = WindowHistory::new(window(3));
        let mut doc = small_state();
        let pick =
            |ids: &[u64]| SetSelection::new(Selection::new(ids.iter().map(|i| sid(*i)), None));
        history.record_selection(&doc, &pick(&[11])).unwrap();
        history.record_selection(&doc, &pick(&[12])).unwrap();
        history.undo(&mut doc).unwrap();
        assert_eq!(history.view().selection().ids(), &[sid(11)]);
        history.redo(&mut doc).unwrap();
        assert_eq!(history.view().selection().ids(), &[sid(12)]);
        history.undo(&mut doc).unwrap();
        history.record_selection(&doc, &pick(&[13])).unwrap();
        assert_eq!(history.history().redo_len(), 0, "새 선택도 redo 갈래 폐기");
    }

    #[test]
    fn closing_a_window_discards_history_and_selection() {
        // 검증 기준(VII.5.4): reopen 후 history 비어 있음.
        let mut histories = WindowHistories::new();
        let (a, b) = (window(1), window(2));
        let mut doc = small_state();
        let mut doc_b = small_state();
        let wa = histories.open(a);
        for i in 0..3u64 {
            let t = tx(
                &doc,
                1_000 + i,
                Command::rename_node(sid(11), format!("이름{i}")),
            );
            wa.record_document(&mut doc, &t, None).unwrap();
        }
        wa.undo(&mut doc).unwrap();
        wa.record_selection(&doc, &SetSelection::new(Selection::new([sid(12)], None)))
            .unwrap();
        let tb = tx(&doc_b, 2_000, Command::rename_node(sid(12), "B"));
        histories
            .open(b)
            .record_document(&mut doc_b, &tb, None)
            .unwrap();
        let records_before_close = doc.model().to_records().unwrap();

        assert!(histories.close(a));
        assert!(histories.get(a).is_none());
        assert!(!histories.close(a), "이미 닫힘");
        // 창 닫기는 문서(프로젝트 저장 대상)를 바꾸지 않습니다.
        assert_eq!(doc.model().to_records().unwrap(), records_before_close);
        // 다른 창은 그대로.
        assert_eq!(histories.get(b).unwrap().history().len(), 1);

        // 같은 창 id로 다시 열면 history·선택이 비어 있습니다.
        let reopened = histories.open(a);
        assert!(reopened.history().is_empty());
        assert_eq!(reopened.history().redo_len(), 0);
        assert!(reopened.view().selection().is_empty());
        assert_eq!(reopened.undo(&mut doc).unwrap(), None);
        assert_eq!(histories.len(), 2);
    }

    #[test]
    fn selection_restoration_during_undo_creates_no_entries() {
        // 검증 기준(VII.5.5): undo 100회 후 stack size 예상 일치.
        let mut history = WindowHistory::new(window(4));
        let mut doc = small_state();
        let pick =
            |ids: &[u64]| SetSelection::new(Selection::new(ids.iter().map(|i| sid(*i)), None));
        let targets = [11u64, 12, 13];
        let mut recorded = 0usize;
        for step in 0..100u64 {
            let node = targets[(step % 3) as usize];
            if step % 2 == 0 {
                if history
                    .record_selection(&doc, &pick(&[node]))
                    .unwrap()
                    .is_some()
                {
                    recorded += 1;
                }
            } else {
                let t = tx(
                    &doc,
                    3_000 + step,
                    Command::rename_node(sid(node), format!("이름{step}")),
                );
                history.record_document(&mut doc, &t, None).unwrap();
                recorded += 1;
            }
        }
        assert_eq!(recorded, 100);
        assert_eq!(history.history().len(), 100);

        // UI 선택 변경 알림이 복원된 선택을 다시 기록하려 해도(재진입) 항목이 생기지 않습니다.
        let mut echoes = 0usize;
        for undone in 1..=100usize {
            history
                .undo_with(&mut doc, |inner, view, doc| {
                    assert!(inner.is_replaying());
                    let echo = SetSelection::new(view.selection().clone());
                    let other = SetSelection::new(Selection::new([sid(10)], None));
                    for event in [echo, other] {
                        let result = inner
                            .record_selection(
                                view,
                                doc,
                                &event,
                                SelectionPolicy::default(),
                                CommandOrigin::User,
                            )
                            .unwrap();
                        assert_eq!(result, None);
                        echoes += 1;
                    }
                    // 재생 중 문서 편집 기록은 거부됩니다.
                    let t = tx(doc, 9_999, Command::rename_node(sid(10), "x"));
                    let mut scratch = small_state();
                    assert_eq!(
                        inner.record_document(&mut scratch, &t, None),
                        Err(HistoryError::Replaying)
                    );
                })
                .unwrap()
                .unwrap();
            assert_eq!(history.history().len(), 100 - undone);
            assert_eq!(history.history().redo_len(), undone);
            assert!(!history.history().is_replaying());
        }
        assert_eq!(echoes, 200);
        assert!(history.view().selection().is_empty());
        // redo 100회도 대칭입니다.
        for redone in 1..=100usize {
            history.redo_with(&mut doc, |_, _, _| {}).unwrap().unwrap();
            assert_eq!(history.history().len(), redone);
            assert_eq!(history.history().redo_len(), 100 - redone);
        }
    }

    #[test]
    fn undo_restores_selection_from_before_the_edit() {
        use stellar_commands::node_ops::DeleteSubtree;
        let mut history = WindowHistory::new(window(5));
        let mut doc = small_state();
        let pick =
            |ids: &[u64]| SetSelection::new(Selection::new(ids.iter().map(|i| sid(*i)), None));
        history.record_selection(&doc, &pick(&[13, 12])).unwrap();
        // 선택된 13을 포함한 subtree 삭제 → 선택에서 정리(system), 항목은 삭제 1건만.
        let t = tx(
            &doc,
            4_000,
            Command::DeleteSubtree(DeleteSubtree { root: sid(11) }),
        );
        history.record_document(&mut doc, &t, None).unwrap();
        assert_eq!(history.view().selection().ids(), &[sid(12)]);
        assert_eq!(history.history().len(), 2);
        let restore = history.history().entries()[1].selection.clone().unwrap();
        assert_eq!(restore.before.ids(), &[sid(13), sid(12)]);
        assert_eq!(restore.after.ids(), &[sid(12)]);
        // 삭제 undo → node 복원과 함께 편집 직전 선택(13 포함) 복원, 새 항목 없음.
        history.undo(&mut doc).unwrap();
        assert_eq!(history.view().selection().ids(), &[sid(13), sid(12)]);
        assert_eq!(history.history().len(), 1);
        // redo → 편집 직후 선택.
        history.redo(&mut doc).unwrap();
        assert_eq!(history.view().selection().ids(), &[sid(12)]);
    }
}
