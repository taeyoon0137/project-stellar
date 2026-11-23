//! 한 제스처 한 history (XII.10.40, ADR-0104 결정 5).
//!
//! pointer gesture 하나는 history 항목을 많아야 하나 만듭니다.
//!
//! - 제스처가 문서 항목을 만들면 그 항목의 `SelectionRestore`가 제스처 전·후 선택을 담고 Selection 항목은 만들지
//!   않습니다(선택하지 않은 node를 눌러 바로 끌기: undo 한 번이 편집과 선택을 함께 되돌림).
//! - 편집 없이 끝나고 선택이 바뀌었으면 Selection 항목 하나를 기록합니다(움직이지 않은 클릭).
//! - 취소하면 문서와 선택을 제스처 전으로 돌리고 기록하지 않습니다.
//!
//! [`GestureRecorder`]는 `WindowHistory` 공개 API(`begin_pointer_gesture`, `update_preview`, `end_preview`,
//! `cancel_preview`, `record_selection`)와 `WindowViewState::restore_selection`으로 이 규칙을 냅니다. history의
//! `HistoryEntry` 타입은 바꾸지 않습니다. 제스처 중 선택 반영은 이 기록기만 합니다. preview 중
//! `record_selection`은 `PreviewActive`로 거부되므로, 제스처 중 선택은 항목 없이 view에만 반영하고 끝에서 정합니다.

use stellar_commands::change::ChangeSet;
use stellar_commands::selection::{Selection, SetSelection};
use stellar_commands::state::DocumentState;
use stellar_commands::transaction::CommandTransaction;
use stellar_history::entry::{HistoryError, HistorySequence};
use stellar_history::window::WindowHistory;

/// 제스처가 남긴 history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureRecord {
    /// 문서 항목 하나(제스처 전·후 선택 포함).
    Document(HistorySequence),
    /// 선택 항목 하나.
    Selection(HistorySequence),
    /// 기록 없음(바뀐 것이 없음).
    Nothing,
}

/// 진행 중인 pointer gesture의 history 기록기.
#[derive(Debug)]
#[must_use = "제스처는 finish 또는 cancel로 끝내야 합니다"]
pub struct GestureRecorder {
    before: Selection,
}

impl GestureRecorder {
    /// pointer down: history preview를 시작하고 제스처 전 선택을 잡습니다.
    pub fn begin(
        history: &mut WindowHistory,
        gesture: u64,
    ) -> Result<GestureRecorder, HistoryError> {
        let before = history.view().selection().clone();
        history.begin_pointer_gesture(gesture)?;
        Ok(GestureRecorder { before })
    }

    /// 제스처 전 선택.
    pub fn before(&self) -> &Selection {
        &self.before
    }

    /// 제스처 중 선택 변경(누름에서 고른 node 등): 항목 없이 view에 반영합니다. 숨김 정책·문서에 없는 id는
    /// `SetSelection`과 같은 규칙으로 거릅니다.
    pub fn select(&self, history: &mut WindowHistory, doc: &DocumentState, selection: &Selection) {
        let filtered =
            SetSelection::new(selection.clone()).filtered(doc.model(), history.selection_policy());
        history.view_mut().restore_selection(&filtered, doc.model());
    }

    /// pointer move의 문서 preview(직전 preview를 대신함).
    pub fn preview(
        &self,
        history: &mut WindowHistory,
        doc: &mut DocumentState,
        transaction: &CommandTransaction,
    ) -> Result<ChangeSet, HistoryError> {
        history.update_preview(doc, transaction)
    }

    /// pointer up: 규칙대로 항목 하나 또는 없음을 기록합니다.
    pub fn finish(
        self,
        history: &mut WindowHistory,
        doc: &DocumentState,
    ) -> Result<GestureRecord, HistoryError> {
        if let Some(sequence) = history.end_preview()? {
            return Ok(GestureRecord::Document(sequence));
        }
        let after = history.view().selection().clone();
        if after == self.before {
            return Ok(GestureRecord::Nothing);
        }
        // 편집 없이 선택만 바뀜: 제스처 전 선택에서 기록해 undo가 그 선택으로 돌아가게 합니다.
        history
            .view_mut()
            .restore_selection(&self.before, doc.model());
        Ok(
            match history.record_selection(doc, &SetSelection::new(after))? {
                Some(sequence) => GestureRecord::Selection(sequence),
                None => GestureRecord::Nothing,
            },
        )
    }

    /// 취소(Escape, pointer cancel, 초점 상실): 문서·선택을 제스처 전으로 돌리고 기록하지 않습니다.
    pub fn cancel(
        self,
        history: &mut WindowHistory,
        doc: &mut DocumentState,
    ) -> Result<(), HistoryError> {
        history.cancel_preview(doc)?;
        history
            .view_mut()
            .restore_selection(&self.before, doc.model());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::hit::CanvasGeometry;
    use crate::canvas::hit::{PointStack, StackEntry};
    use crate::canvas::select::{escape, resolve_click, EditingScope, EscapeStep, SelectTree};
    use crate::canvas::select_area::Marquee;
    use crate::canvas::select_fixtures::{nested, save_bytes, scene_of, sid, view_point};
    use crate::canvas::select_hover::HoverState;
    use crate::canvas::select_navigate::{cycle_sibling, double_click, enter, Navigation};
    use crate::canvas::select_through::{plan_press, PressPlan};
    use crate::canvas::tool::{CanvasTool, CanvasToolState, Modifiers};
    use crate::window_session::test_fixtures::window;
    use stellar_commands::command::Command;
    use stellar_commands::id::{CommandId, CommandOrigin};
    use stellar_commands::property::{PropertyValue, SetProperty};
    use stellar_core_types::ids::StableId;
    use stellar_core_types::value::FiniteF32;
    use stellar_document::frame::keys as frame_keys;
    use stellar_history::entry::HistoryPayload;

    struct World {
        doc: DocumentState,
        history: WindowHistory,
        root: StableId,
        geometry: CanvasGeometry,
    }

    fn world() -> World {
        let fixture = nested();
        let scene = scene_of(&fixture.model, fixture.root);
        World {
            geometry: CanvasGeometry::from_snapshot(&scene.snapshot),
            root: fixture.root,
            doc: DocumentState::new(fixture.model),
            history: WindowHistory::new(window(9)),
        }
    }

    fn stack(ids: &[u64]) -> PointStack {
        PointStack::new(ids.iter().map(|n| StackEntry {
            node: sid(*n),
            locked: false,
        }))
    }

    /// Frame 너비를 바꾸는 preview transaction(드래그 편집 대신).
    fn width_tx(doc: &DocumentState, id: u64, node: u64, width: f32) -> CommandTransaction {
        CommandTransaction::new(CommandId::from_stable_id(sid(id)), CommandOrigin::User)
            .with_command(
                doc.revision(),
                Command::SetProperty(SetProperty {
                    node: sid(node),
                    key: frame_keys::WIDTH,
                    value: PropertyValue::NonNegative(FiniteF32::new(width).unwrap()),
                }),
            )
    }

    fn counts(history: &WindowHistory) -> (usize, usize) {
        let entries = history.history().entries();
        let docs = entries
            .iter()
            .filter(|e| matches!(e.payload, HistoryPayload::Document { .. }))
            .count();
        (docs, entries.len() - docs)
    }

    /// Move 도구 누름 → (끌기 n번) → 뗌을 실제 선택 규칙으로 재생합니다.
    fn press_drag_release(w: &mut World, gesture: u64, at: &[u64], drags: u32) -> GestureRecord {
        let tree = SelectTree::new(w.doc.model(), w.root);
        let scope = EditingScope::top(&tree);
        let current = w.history.view().selection().clone();
        let point = stack(at);
        let world_point = view_point(130., 130.);
        let plan = plan_press(
            &tree,
            scope,
            &current,
            &point,
            &w.geometry,
            world_point,
            Modifiers::NONE,
        );
        let recorder = GestureRecorder::begin(&mut w.history, gesture).unwrap();
        let mut release = None;
        match plan {
            PressPlan::SelectAndDrag(change) => {
                recorder.select(&mut w.history, &w.doc, &change.selection)
            }
            PressPlan::KeepSelection { on_click } => release = Some(on_click.selection),
            PressPlan::Marquee => {}
        }
        let target = w.history.view().selection().primary();
        for step in 0..drags {
            let tx = width_tx(
                &w.doc,
                5_000 + u64::from(step) + gesture * 10_000,
                11,
                200. + step as f32,
            );
            recorder.preview(&mut w.history, &mut w.doc, &tx).unwrap();
            assert_eq!(target, Some(sid(11)), "이 fixture는 Frame 11을 끕니다");
        }
        if drags == 0 {
            if let Some(selection) = release {
                recorder.select(&mut w.history, &w.doc, &selection);
            }
        }
        recorder.finish(&mut w.history, &w.doc).unwrap()
    }

    #[test]
    fn gesture_table_entry_counts() {
        // 검증 기준(XII.10.40): ADR-0104 결정 5 표의 제스처마다 history entry 수 기대값.
        let mut w = world();
        // 움직이지 않은 클릭(선택이 바뀜): 선택 1.
        assert!(matches!(
            press_drag_release(&mut w, 1, &[13, 11, 10], 0),
            GestureRecord::Selection(_)
        ));
        assert_eq!(counts(&w.history), (0, 1));
        // 같은 node 클릭(선택 그대로): 0.
        assert_eq!(
            press_drag_release(&mut w, 2, &[13, 11, 10], 0),
            GestureRecord::Nothing
        );
        assert_eq!(counts(&w.history), (0, 1));
        // 선택한 node 드래그: 문서 1.
        assert!(matches!(
            press_drag_release(&mut w, 3, &[13, 11, 10], 50),
            GestureRecord::Document(_)
        ));
        assert_eq!(counts(&w.history), (1, 1));
        // 마퀴(선택이 바뀜): 선택 1. 같은 선택이면 0.
        let tree = SelectTree::new(w.doc.model(), w.root);
        let top = EditingScope::top(&tree);
        let mut marquee = Marquee::begin(
            &tree,
            top,
            &w.geometry,
            w.history.view_mut(),
            view_point(290., 10.),
            Modifiers::NONE,
        )
        .unwrap();
        marquee
            .update(w.history.view_mut(), view_point(430., 150.))
            .unwrap();
        let result = marquee.finish(&tree, w.history.view_mut()).unwrap();
        w.history
            .record_selection(&w.doc, &result.event.unwrap())
            .unwrap();
        assert_eq!(counts(&w.history), (1, 2));
        // hover, camera 팬·줌, 도구 전환, 범위만 바뀌는 이동: 0.
        let before = counts(&w.history);
        let mut hover = HoverState::new();
        hover.update_canvas(&tree, top, &stack(&[17, 14, 11, 10]), Modifiers::COMMAND);
        hover.update_sidebar(&tree, Some(sid(13)));
        w.history
            .view_mut()
            .tabs_mut()
            .open(w.doc.model(), sid(3))
            .unwrap();
        let panned = w
            .history
            .view()
            .tabs()
            .active_viewport()
            .unwrap()
            .panned(40., -20.)
            .unwrap();
        w.history
            .view_mut()
            .tabs_mut()
            .set_active_viewport(panned.with_zoom(2.0).unwrap())
            .unwrap();
        let mut tools = CanvasToolState::new();
        tools.select(CanvasTool::Hand);
        tools.select(CanvasTool::Move);
        let _ = EditingScope::at(&tree, sid(11)).refit(&tree, w.history.view().selection());
        assert_eq!(counts(&w.history), before);
        // Escape 선택 해제, 더블클릭·Return·Tab 선택 이동: 각 선택 1.
        let record = |w: &mut World, selection: &Selection| {
            let event = SetSelection::new(selection.clone());
            w.history.record_selection(&w.doc, &event).unwrap()
        };
        let current = w.history.view().selection().clone();
        let EscapeStep::Deselect(change) = escape(&tree, top, &current, &mut tools) else {
            panic!("해제");
        };
        assert!(record(&mut w, &change.selection).is_some());
        let tree = SelectTree::new(w.doc.model(), w.root);
        let click = resolve_click(
            &tree,
            top,
            &Selection::empty(),
            &stack(&[17, 14, 11, 10]),
            Modifiers::NONE,
        );
        let Navigation::Changed(dbl) = double_click(
            &tree,
            click.scope,
            &click.selection,
            &stack(&[17, 14, 11, 10]),
            Modifiers::NONE,
        ) else {
            panic!("진입");
        };
        let Navigation::Changed(ret) = enter(&tree, dbl.scope, &dbl.selection) else {
            panic!("Return");
        };
        let Navigation::Changed(tab) =
            cycle_sibling(&tree, top, &Selection::new([sid(11)], None), false)
        else {
            panic!("Tab");
        };
        let start = counts(&w.history).1;
        for selection in [&dbl.selection, &ret.selection, &tab.selection] {
            assert!(record(&mut w, selection).is_some());
        }
        assert_eq!(counts(&w.history).1, start + 3);
    }

    #[test]
    fn immediate_drag_of_an_unselected_node_undoes_document_and_selection_together() {
        // 검증 기준(XII.10.40): 선택하지 않은 node 즉시 drag에서 undo 1회 후 문서와 선택이 모두 제스처 전과 같음.
        let mut w = world();
        // 처음 선택은 Fragment 12.
        w.history
            .record_selection(&w.doc, &SetSelection::new(Selection::new([sid(12)], None)))
            .unwrap();
        let before_bytes = save_bytes(&w.doc);
        let before_selection = w.history.view().selection().clone();
        let before_len = w.history.history().len();
        // 선택 밖의 11을 눌러 바로 500 update 끕니다.
        let record = press_drag_release(&mut w, 7, &[13, 11, 10], 500);
        assert!(matches!(record, GestureRecord::Document(_)));
        assert_eq!(
            w.history.history().len(),
            before_len + 1,
            "편집·선택이 항목 하나"
        );
        assert_eq!(w.history.view().selection().ids(), &[sid(11)]);
        let entry = w.history.history().entries().last().unwrap();
        let restore = entry.selection.as_ref().expect("제스처 전·후 선택");
        assert_eq!(
            (&restore.before, restore.after.ids()),
            (&before_selection, &[sid(11)][..])
        );
        assert_ne!(save_bytes(&w.doc), before_bytes);
        w.history.undo(&mut w.doc).unwrap().unwrap();
        assert_eq!(save_bytes(&w.doc), before_bytes, "문서 원복");
        assert_eq!(w.history.view().selection(), &before_selection, "선택 원복");
        w.history.redo(&mut w.doc).unwrap().unwrap();
        assert_eq!(w.history.view().selection().ids(), &[sid(11)]);
    }

    #[test]
    fn click_without_movement_records_one_selection_and_cancel_restores_everything() {
        // 검증 기준(XII.10.40): 움직이지 않은 click에서 Selection 항목 1건, 취소에서 항목 0건과 선택 원복.
        let mut w = world();
        let record = press_drag_release(&mut w, 1, &[13, 11, 10], 0);
        let GestureRecord::Selection(_) = record else {
            panic!("선택 항목");
        };
        assert_eq!(counts(&w.history), (0, 1));
        // undo는 제스처 전(빈) 선택으로 돌아갑니다.
        w.history.undo(&mut w.doc).unwrap().unwrap();
        assert!(w.history.view().selection().is_empty());
        w.history.redo(&mut w.doc).unwrap().unwrap();
        // 다른 node(12)를 눌러 선택을 바꾸고 끌다가 취소: 문서·선택 원복, 항목 0건.
        let before_bytes = save_bytes(&w.doc);
        let before_len = w.history.history().len();
        let before_selection = w.history.view().selection().clone();
        let recorder = GestureRecorder::begin(&mut w.history, 99).unwrap();
        recorder.select(&mut w.history, &w.doc, &Selection::new([sid(12)], None));
        for step in 0..20 {
            let tx = width_tx(&w.doc, 9_000 + step, 11, 300. + step as f32);
            recorder.preview(&mut w.history, &mut w.doc, &tx).unwrap();
        }
        assert_eq!(w.history.view().selection().ids(), &[sid(12)]);
        recorder.cancel(&mut w.history, &mut w.doc).unwrap();
        assert_eq!(w.history.history().len(), before_len);
        assert_eq!(w.history.view().selection(), &before_selection);
        assert_eq!(save_bytes(&w.doc), before_bytes);
        // 편집 없이 선택만 바꾸고 취소해도 원복·기록 없음.
        let recorder = GestureRecorder::begin(&mut w.history, 100).unwrap();
        recorder.select(&mut w.history, &w.doc, &Selection::empty());
        recorder.cancel(&mut w.history, &mut w.doc).unwrap();
        assert_eq!(w.history.view().selection(), &before_selection);
        assert_eq!(w.history.history().len(), before_len);
    }
}
