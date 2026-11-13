//! 창별 history (VII.5).
//!
//! 같은 프로젝트를 여러 창에서 열면 각 창은 독립된 인메모리 문서 상태와 undo/redo stack을 가집니다
//! (기술 아키텍처 §22.2·§23.2). [`WindowHistory`]는 창 하나([`WindowSessionId`])의 history와 그 창의
//! view 상태(선택)를 함께 소유하며, 다른 창의 항목을 담지 않습니다. [`WindowHistories`]는 열린 창마다
//! 하나씩 만들고 창을 닫으면 폐기합니다.

use std::collections::BTreeMap;

use stellar_commands::change::ChangeSet;
use stellar_commands::id::{CommandOrigin, WindowSessionId};
use stellar_commands::selection::{SelectionPolicy, SetSelection, WindowViewState};
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
        let changes = self.history.record_document(doc, transaction, grouping)?;
        self.view.prune_removed(&changes);
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
        let changes = self.history.record_grouped(doc, transaction, key, clock)?;
        self.view.prune_removed(&changes);
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
        self.history.begin_pointer_gesture(gesture)
    }

    /// slider·scrub 시작(VII.4.5).
    pub fn begin_scrub(&mut self, control: u64) -> Result<(), HistoryError> {
        self.history.begin_scrub(control)
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
        self.history.end_preview()
    }

    /// preview 취소(VII.4.5).
    pub fn cancel_preview(&mut self, doc: &mut DocumentState) -> Result<(), HistoryError> {
        self.history.cancel_preview(doc)
    }

    /// 이 창의 마지막 항목 하나를 이 창의 문서·선택에 되돌립니다.
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
}
