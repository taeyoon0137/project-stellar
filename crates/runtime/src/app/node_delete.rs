//! Delete selected subtrees through the owning window's document history.
use super::frame_move::{command_id, trusted_canvas_window};
use super::*;
use stellar_commands::{
    command::Command, id::CommandOrigin, node_ops::DeleteSubtree, transaction::CommandTransaction,
};

impl AppRuntime {
    /// 신뢰한 창의 현재 선택을 하나의 history 항목으로 삭제합니다.
    pub fn delete_selected_nodes(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            delete_selected(
                trusted_canvas_window(ctx, session)?,
                request,
                view,
                revision,
            )?;
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
    }
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
