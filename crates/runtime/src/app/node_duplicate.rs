//! 단일 선택 subtree를 같은 부모 아래에 복제하고 한 history 항목으로 선택합니다.
use super::frame_move::{command_id, fresh_generator};
use super::node_delete::editable_selected_roots;
use super::*;
use stellar_commands::{
    command::Command, duplicate::DuplicateSubtree, id::CommandOrigin,
    transaction::CommandTransaction,
};
use stellar_core_types::clock::ManualClock;

pub(super) fn duplicate_selected(
    window: &mut ProjectWindowSession,
    request: RequestId,
    view: StableId,
    revision: DocumentRevision,
) -> Result<StableId, Failure> {
    let invalid = || Failure::new(RequestFailure::UnknownView, "복제할 노드 하나를 선택하세요");
    if window.view().selection().ids().len() != 1 {
        return Err(invalid());
    }
    let roots = editable_selected_roots(window, view, revision)?;
    let source = roots[0];
    let model = window.document().ok_or_else(invalid)?.model();
    let parent = model
        .record(source)
        .and_then(|r| r.parent)
        .ok_or_else(invalid)?;
    let id = command_id(window.window(), request);
    let mut generator = fresh_generator()?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_nanos()).ok())
        .ok_or_else(invalid)?;
    // 복제본은 원본 바로 위에 놓습니다. 흐름 컨테이너에서도 원본 다음 자리가 됩니다.
    let order = model.order_key_above(parent, source).ok_or_else(invalid)?;
    let command = DuplicateSubtree::plan(
        model,
        source,
        parent,
        order,
        &mut generator,
        &ManualClock::new(nanos),
    )
    .map_err(|_| invalid())?;
    let copy = command.copy_root().ok_or_else(invalid)?;
    let tx = CommandTransaction::new(id, CommandOrigin::User)
        .with_command(revision, Command::DuplicateSubtree(command));
    window.insert_and_select(&tx, copy).map_err(view_failure)?;
    Ok(copy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window_session::test_fixtures::{
        identity, sample_document, saved_project_with, sid, window,
    };
    use stellar_commands::selection::Selection;

    #[test]
    fn duplicate_subtree_history_selection_and_save_reopen() {
        let mut model = sample_document();
        let mut child = model.remove_leaf(sid(12)).unwrap();
        child.parent = Some(sid(11));
        model.insert_node(child).unwrap();
        let dir = saved_project_with("duplicate-selected", &[&model]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        session.select(Selection::new([sid(11)], None)).unwrap();
        let rev = session.revision();
        let copy = duplicate_selected(&mut session, RequestId(1), sid(3), rev)
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        let copied_child = session.document().unwrap().model().children_in_order(copy)[0];
        assert_ne!(copy, sid(11));
        assert_ne!(copied_child, sid(12));
        assert_eq!(session.view().selection().ids(), [copy]);
        assert!(duplicate_selected(&mut session, RequestId(2), sid(3), rev).is_err());
        assert!(session.undo().unwrap());
        assert_eq!(session.view().selection().ids(), [sid(11)]);
        assert!(!session.document().unwrap().model().contains(copy));
        assert!(session.redo().unwrap());
        assert_eq!(session.view().selection().ids(), [copy]);
        session
            .save_with_lock_dir(|| Ok(dir.path().join("locks")))
            .unwrap();
        let reopened = ProjectWindowSession::load(identity(dir.path()), window(2)).unwrap();
        let saved = reopened.document().unwrap().model();
        assert_eq!(saved.children_in_order(copy), [copied_child]);
        assert_eq!(saved.payload(copy), model.payload(sid(11)));
        assert_eq!(saved.payload(copied_child), model.payload(sid(12)));
        assert_eq!(saved.record(copy).unwrap().parent, Some(sid(10)));
        assert!(saved.contains(sid(11)) && saved.contains(sid(12)));
    }

    #[test]
    fn duplicate_is_placed_directly_above_the_source_through_undo_and_reopen() {
        // 형제 11(아래)·12(위) 사이에 복제본이 들어가야 흐름 컨테이너 순서도 원본 다음이 됩니다.
        let dir = saved_project_with("duplicate-order", &[&sample_document()]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        session.select(Selection::new([sid(11)], None)).unwrap();
        let rev = session.revision();
        let copy = duplicate_selected(&mut session, RequestId(1), sid(3), rev)
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        let order = |session: &ProjectWindowSession| {
            session
                .document()
                .unwrap()
                .model()
                .children_in_order(sid(10))
        };
        assert_eq!(order(&session), [sid(11), copy, sid(12)]);
        assert!(session.undo().unwrap());
        assert_eq!(order(&session), [sid(11), sid(12)]);
        assert!(session.redo().unwrap());
        assert_eq!(order(&session), [sid(11), copy, sid(12)]);
        session
            .save_with_lock_dir(|| Ok(dir.path().join("locks")))
            .unwrap();
        let reopened = ProjectWindowSession::load(identity(dir.path()), window(2)).unwrap();
        assert_eq!(order(&reopened), [sid(11), copy, sid(12)]);
    }

    #[test]
    fn duplicate_rejects_root_multiple_selection_wrong_view_and_locks() {
        for (selected, locked, view) in [
            (vec![], None, 3),
            (vec![10], None, 3),
            (vec![11, 12], None, 3),
            (vec![11], None, 4),
            (vec![11], Some(10), 3),
            (vec![11], Some(11), 3),
            (vec![11], Some(12), 3),
        ] {
            let mut model = sample_document();
            let mut child = model.remove_leaf(sid(12)).unwrap();
            child.parent = Some(sid(11));
            model.insert_node(child).unwrap();
            if let Some(locked) = locked {
                model.replace_flags(sid(locked), true, true).unwrap();
            }
            let dir = saved_project_with("duplicate-protected", &[&model]);
            let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
            session
                .select(Selection::new(selected.into_iter().map(sid), None))
                .unwrap();
            let rev = session.revision();
            assert!(duplicate_selected(&mut session, RequestId(1), sid(view), rev).is_err());
            assert_eq!(session.revision(), rev);
            assert!(!session.has_unsaved_changes());
        }
    }
}
