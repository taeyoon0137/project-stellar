//! Delete selected subtrees through the owning window's document history.
use super::frame_move::{command_id, trusted_canvas_window};
use super::*;
use stellar_commands::{
    command::Command, id::CommandOrigin, node_ops::DeleteSubtree, transaction::CommandTransaction,
};

/// 창의 확정 선택에 적용할 문서 편집.
#[derive(Clone, Copy, Debug)]
pub enum SelectionEdit {
    /// 선택한 subtree를 함께 삭제합니다.
    Delete,
    /// 단일 선택 subtree를 복제합니다.
    Duplicate,
}

/// 하나의 게시 시점에 읽은 창·뷰·문서·선택 history 위치.
#[derive(Clone, Copy, Debug)]
pub struct SelectionEditTarget {
    /// 창 handle.
    pub session: WindowSessionHandle,
    /// 활성 View.
    pub view: StableId,
    /// 표시한 문서 revision.
    pub revision: DocumentRevision,
    /// 표시한 창의 Undo 스택 끝 sequence.
    pub history_sequence: u64,
}

impl AppRuntime {
    /// 신뢰한 창의 게시된 선택을 하나의 history 항목으로 삭제합니다.
    pub fn delete_selected_nodes(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
    ) -> Result<RequestId, RequestError> {
        self.edit_published_selection(session, view, revision, SelectionEdit::Delete)
    }

    /// 신뢰한 창의 게시된 단일 선택 subtree를 복제하고 복제본을 선택합니다.
    pub fn duplicate_selected_node(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
    ) -> Result<RequestId, RequestError> {
        self.edit_published_selection(session, view, revision, SelectionEdit::Duplicate)
    }

    fn edit_published_selection(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
        edit: SelectionEdit,
    ) -> Result<RequestId, RequestError> {
        // 기존 API도 접수와 실행 사이에 선택이 바뀌면 거부합니다.
        let state = self.state();
        let Some(summary) = state.window(session) else {
            return self.submit(move |_, _| Err(unknown_window(session)));
        };
        self.edit_selection(
            SelectionEditTarget {
                session,
                view,
                revision,
                history_sequence: summary.history_sequence,
            },
            edit,
            None,
        )
    }

    /// 표시된 선택을 편집합니다. Layers가 명시한 ID는 실행 시점의 선택 전체와 일치해야 합니다.
    pub fn edit_selection(
        &self,
        target: SelectionEditTarget,
        edit: SelectionEdit,
        ids: Option<Vec<StableId>>,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            let window = trusted_canvas_window(ctx, target.session)?;
            let selected = window.view().selection().ids();
            if window.history_sequence() != target.history_sequence
                || ids
                    .as_ref()
                    .is_some_and(|ids| !same_selection(ids, selected))
            {
                return Err(Failure::new(
                    RequestFailure::UnknownView,
                    "표시한 선택이 변경되었습니다",
                ));
            }
            match edit {
                SelectionEdit::Delete => {
                    delete_selected(window, request, target.view, target.revision)?
                }
                SelectionEdit::Duplicate => {
                    super::node_duplicate::duplicate_selected(
                        window,
                        request,
                        target.view,
                        target.revision,
                    )?;
                }
            }
            Ok(RuntimeEvent::WindowViewChanged {
                request,
                session: target.session,
            })
        })
    }
}

// Layers의 행 순서는 선택 순서와 다를 수 있습니다. 일부만 표시된 페이지와 중복 ID는 거부합니다.
fn same_selection(ids: &[StableId], selected: &[StableId]) -> bool {
    if ids.len() != selected.len() {
        return false;
    }
    if ids == selected {
        return true;
    }
    let targets: std::collections::HashSet<_> = ids.iter().collect();
    selected.iter().all(|id| targets.contains(id))
}

fn delete_selected(
    window: &mut ProjectWindowSession,
    request: RequestId,
    view: StableId,
    revision: DocumentRevision,
) -> Result<(), Failure> {
    let roots = editable_selected_roots(window, view, revision)?;
    let mut tx = CommandTransaction::new(command_id(window.window(), request), CommandOrigin::User);
    for node in roots {
        tx = tx.with_command(
            revision,
            Command::DeleteSubtree(DeleteSubtree { root: node }),
        );
    }
    window.apply(&tx).map_err(view_failure)?;
    Ok(())
}

/// 선택한 subtree와 조상의 편집 가능 여부를 document thread에서 확인합니다.
pub(super) fn editable_selected_roots(
    window: &ProjectWindowSession,
    view: StableId,
    revision: DocumentRevision,
) -> Result<Vec<StableId>, Failure> {
    let invalid = || {
        Failure::new(
            RequestFailure::UnknownView,
            "편집할 선택·뷰·잠금 상태가 변경되었습니다",
        )
    };
    if window.revision() != revision || window.tabs().active_view() != Some(view) {
        return Err(invalid());
    }
    let model = window.document().ok_or_else(invalid)?.model();
    let root = model
        .definition()
        .pages
        .iter()
        .flat_map(|p| &p.views)
        .find(|v| v.id == view)
        .ok_or_else(invalid)?
        .root_frame;
    let selected = window.view().selection().ids();
    if selected.is_empty() {
        return Err(invalid());
    }
    let mut roots = Vec::new();
    for &node in selected {
        if node == root {
            return Err(invalid());
        }
        let mut current = node;
        let mut covered = false;
        loop {
            let record = model.record(current).ok_or_else(invalid)?;
            if record.locked {
                return Err(invalid());
            }
            if current == root {
                break;
            }
            current = record.parent.ok_or_else(invalid)?;
            covered |= selected.contains(&current);
        }
        if !covered {
            roots.push(node);
        }
    }
    // A locked descendant must not disappear with an unlocked parent.
    let mut pending = roots.clone();
    while let Some(node) = pending.pop() {
        if model.record(node).ok_or_else(invalid)?.locked {
            return Err(invalid());
        }
        pending.extend(model.children_in_order(node));
    }
    Ok(roots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window_session::test_fixtures::{
        identity, sample_document, saved_project_with, sid, window,
    };
    use stellar_commands::selection::Selection;

    #[test]
    fn deletion_history_and_save_reopen() {
        let dir = saved_project_with("delete-selected", &[&sample_document()]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        session
            .select(Selection::new([sid(11), sid(12)], None))
            .unwrap();
        let rev = session.revision();
        assert!(delete_selected(&mut session, RequestId(1), sid(4), rev).is_err());
        assert!(delete_selected(&mut session, RequestId(2), sid(3), rev).is_ok());
        assert!(session.view().selection().ids().is_empty());
        assert!(!session.document().unwrap().model().contains(sid(11)));
        assert!(delete_selected(&mut session, RequestId(3), sid(3), rev).is_err());
        assert!(session.undo().unwrap());
        assert_eq!(session.view().selection().ids(), [sid(11), sid(12)]);
        assert!(session.document().unwrap().model().contains(sid(11)));
        assert!(session.redo().unwrap());
        session
            .save_with_lock_dir(|| Ok(dir.path().join("locks")))
            .unwrap();
        let reopened = ProjectWindowSession::load(identity(dir.path()), window(2)).unwrap();
        assert!(!reopened.document().unwrap().model().contains(sid(11)));
        assert!(!reopened.document().unwrap().model().contains(sid(12)));
        assert!(reopened.document().unwrap().model().contains(sid(10)));
    }

    #[test]
    fn selected_parent_deduplicates_child_and_protects_locked_descendants() {
        for locked in [false, true] {
            let mut model = sample_document();
            let mut child = model.remove_leaf(sid(12)).unwrap();
            child.parent = Some(sid(11));
            child.locked = locked;
            model.insert_node(child).unwrap();
            let dir = saved_project_with("delete-subtree", &[&model]);
            let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
            session
                .select(Selection::new(
                    if locked {
                        vec![sid(11)]
                    } else {
                        vec![sid(11), sid(12)]
                    },
                    None,
                ))
                .unwrap();
            let rev = session.revision();
            let result = delete_selected(&mut session, RequestId(1), sid(3), rev);
            if locked {
                assert!(result.is_err());
                assert_eq!(session.revision(), rev);
                assert!(session.document().unwrap().model().contains(sid(11)));
            } else {
                assert!(result.is_ok());
                assert!(!session.document().unwrap().model().contains(sid(12)));
                assert!(session.undo().unwrap());
                assert_eq!(session.view().selection().ids(), [sid(11), sid(12)]);
            }
        }
    }

    #[test]
    fn untrusted_delete_is_rejected_without_mutation() {
        use crate::app_window_tests::{next_event, open_project, open_window};
        let dir = saved_project_with("delete-untrusted", &[&sample_document()]);
        let app = AppRuntime::start().unwrap();
        let owner = open_project(&app, dir.path());
        let (session, _, _) = open_window(&app, owner);
        app.set_selection(session, vec![sid(11)]).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::WindowViewChanged { .. }
        ));
        let rev = app.state().window(session).unwrap().revision;
        app.delete_selected_nodes(session, sid(3), rev).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::RequestFailed {
                failure: RequestFailure::TrustUnavailable,
                ..
            }
        ));
        assert_eq!(app.state().window(session).unwrap().revision, rev);
        assert!(!app.state().window(session).unwrap().has_unsaved_changes);
    }

    #[test]
    fn selection_edits_check_display_and_targets_after_queued_changes_and_replay() {
        use crate::app_window_tests::{next_event, open_project, open_window};
        use std::sync::mpsc;

        let dir = saved_project_with("selection-target", &[&sample_document()]);
        let app = AppRuntime::start().unwrap();
        let owner = open_project(&app, dir.path());
        let (session, _, _) = open_window(&app, owner);
        app.sender
            .submit(move |ctx| {
                ctx.app_sessions_mut()
                    .projects
                    .get_mut(&owner)
                    .unwrap()
                    .trust = TrustStatus::Trusted;
            })
            .unwrap()
            .wait()
            .unwrap();
        let take_target = || {
            let state = app.state();
            let window = state.window(session).unwrap();
            SelectionEditTarget {
                session,
                view: window.active_view.unwrap(),
                revision: window.revision,
                history_sequence: window.history_sequence,
            }
        };
        let changed = |request| {
            assert!(
                matches!(next_event(&app), RuntimeEvent::WindowViewChanged { request: got, .. } if got == request)
            );
        };
        let rejected = |request| {
            assert!(matches!(next_event(&app), RuntimeEvent::RequestFailed {
                request: got, failure: RequestFailure::UnknownView, ..
            } if got == request));
        };
        changed(app.set_selection(session, vec![sid(11)]).unwrap());
        let first = take_target();
        // worker를 막아 B 선택 → 기존 ABI 명령 순으로 넣습니다. 접수 시점의 게시 선택은 여전히 A입니다.
        let (release, wait) = mpsc::channel();
        let _blocked = app.sender.submit(move |_| wait.recv().unwrap()).unwrap();
        let next = app.set_selection(session, vec![sid(12)]).unwrap();
        let delete = app
            .delete_selected_nodes(session, first.view, first.revision)
            .unwrap();
        let duplicate = app
            .duplicate_selected_node(session, first.view, first.revision)
            .unwrap();
        release.send(()).unwrap();
        changed(next);
        rejected(delete);
        rejected(duplicate);
        let second = take_target();
        assert_eq!(first.revision, second.revision);
        assert_ne!(first.history_sequence, second.history_sequence);
        for edit in [SelectionEdit::Delete, SelectionEdit::Duplicate] {
            rejected(app.edit_selection(first, edit, None).unwrap());
            rejected(
                app.edit_selection(second, edit, Some(vec![sid(11)]))
                    .unwrap(),
            );
        }
        assert_eq!(app.state().window(session).unwrap().undo_depth, 2);
        assert!(!app.state().window(session).unwrap().has_unsaved_changes);
        // Undo·Redo는 문서 revision을 바꾸지 않아도 원래 선택 위치를 정확히 복원합니다.
        app.undo(session).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::HistoryReplayed { applied: true, .. }
        ));
        assert_eq!(take_target().history_sequence, first.history_sequence);
        rejected(
            app.edit_selection(second, SelectionEdit::Delete, None)
                .unwrap(),
        );
        app.redo(session).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::HistoryReplayed { applied: true, .. }
        ));
        assert_eq!(take_target().history_sequence, second.history_sequence);
        rejected(
            app.edit_selection(first, SelectionEdit::Duplicate, None)
                .unwrap(),
        );
        changed(app.set_selection(session, vec![sid(11), sid(12)]).unwrap());
        let both = take_target();
        for ids in [
            vec![sid(11)],
            vec![sid(11), sid(11)],
            vec![sid(11), sid(20)],
        ] {
            rejected(
                app.edit_selection(both, SelectionEdit::Delete, Some(ids))
                    .unwrap(),
            );
        }
        let reversed = vec![sid(12), sid(11)];
        rejected(
            app.edit_selection(both, SelectionEdit::Duplicate, Some(reversed.clone()))
                .unwrap(),
        );
        changed(
            app.edit_selection(both, SelectionEdit::Delete, Some(reversed))
                .unwrap(),
        );
        assert_eq!(app.state().window(session).unwrap().selection_len, 0);
        app.undo(session).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::HistoryReplayed { applied: true, .. }
        ));
        assert_eq!(app.state().window(session).unwrap().selection_len, 2);
    }

    #[test]
    fn root_ancestor_and_descendant_locks_are_atomic() {
        for (locked, selected) in [(10, 11), (11, 11), (11, 10), (12, 11)] {
            let mut model = sample_document();
            model.replace_flags(sid(locked), true, true).unwrap();
            let dir = saved_project_with("delete-locked", &[&model]);
            let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
            session
                .select(Selection::new([sid(selected)], None))
                .unwrap();
            let rev = session.revision();
            let result = delete_selected(&mut session, RequestId(1), sid(3), rev);
            if locked == 12 {
                assert!(result.is_ok());
            } else {
                assert!(result.is_err());
                assert_eq!(session.revision(), rev);
                assert!(!session.has_unsaved_changes());
            }
        }
    }
}
