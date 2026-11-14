//! Platform clipboard bytes enter the owning window's ordinary document history.
use super::frame_move::{command_id, trusted_canvas_window};
use super::*;
use stellar_commands::{
    command::Command, id::CommandOrigin, paste::ClipboardSubtree, transaction::CommandTransaction,
};
use stellar_core_types::{clock::ManualClock, ids::StableIdGenerator, order_key::OrderKey};

impl AppRuntime {
    /// Shell worker query; never wait for this ticket on the UI thread.
    pub fn copy_selected_subtree(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
    ) -> Result<crate::document_thread::JobTicket<Option<Vec<u8>>>, SubmitError> {
        self.sender.submit(move |ctx| {
            let window = trusted_canvas_window(ctx, session).ok()?;
            copy_from(window, view, revision)
        })
    }

    /// Paste a same-document clipboard subtree into an explicit container in the active View.
    /// A NIL parent resolves from current selection; trust, revision and ancestor locks are checked here.
    pub fn paste_node_subtree(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
        parent: StableId,
        bytes: Vec<u8>,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            paste_into(
                trusted_canvas_window(ctx, session)?,
                request,
                view,
                revision,
                parent,
                &bytes,
            )?;
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
    }
}

fn copy_from(
    window: &ProjectWindowSession,
    view: StableId,
    revision: DocumentRevision,
) -> Option<Vec<u8>> {
    let roots = super::node_delete::editable_selected_roots(window, view, revision).ok()?;
    if roots.len() != 1 || window.view().selection().ids().len() != 1 {
        return None;
    }
    let bytes = ClipboardSubtree::capture(window.document()?.model(), roots[0])
        .ok()?
        .encode();
    (bytes.len() <= 16 * 1024 * 1024).then_some(bytes)
}

fn paste_into(
    window: &mut ProjectWindowSession,
    request: RequestId,
    view: StableId,
    revision: DocumentRevision,
    parent: StableId,
    bytes: &[u8],
) -> Result<StableId, Failure> {
    let invalid = || {
        Failure::new(
            RequestFailure::UnknownView,
            "붙여넣을 View·부모·잠금 상태를 확인하세요",
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
        .flat_map(|page| &page.views)
        .find(|candidate| candidate.id == view)
        .ok_or_else(invalid)?
        .root_frame;
    // NIL asks for the ordinary app destination, resolved against current document state.
    let parent = if parent == StableId::NIL {
        match window.view().selection().ids() {
            [] => root,
            [selected] => {
                let record = model.record(*selected).ok_or_else(invalid)?;
                if stellar_document::validate::can_have_children(record.kind()) {
                    *selected
                } else {
                    record.parent.ok_or_else(invalid)?
                }
            }
            _ => return Err(invalid()),
        }
    } else {
        parent
    };
    let target = model.record(parent).ok_or_else(invalid)?;
    if !stellar_document::validate::can_have_children(target.kind()) {
        return Err(invalid());
    }
    let mut current = parent;
    loop {
        let record = model.record(current).ok_or_else(invalid)?;
        if record.locked {
            return Err(invalid());
        }
        if current == root {
            break;
        }
        current = record.parent.ok_or_else(invalid)?;
    }
    let clipboard = ClipboardSubtree::decode(bytes).map_err(|_| invalid())?;
    let id = command_id(window.window(), request);
    let mut generator =
        StableIdGenerator::new(u64::from_le_bytes(id.to_bytes()[..8].try_into().unwrap()));
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_nanos()).ok())
        .ok_or_else(invalid)?;
    let last = model
        .children_in_order(parent)
        .last()
        .and_then(|id| model.record(*id));
    let order = OrderKey::between(last.as_ref().map(|record| &record.order_key), None);
    let insert = clipboard
        .plan(
            model,
            parent,
            order,
            &mut generator,
            &ManualClock::new(nanos),
        )
        .map_err(|error| Failure::new(RequestFailure::UnknownView, error.to_string()))?;
    let inserted = insert.nodes[0].id;
    let transaction = CommandTransaction::new(id, CommandOrigin::User)
        .with_command(revision, Command::InsertNode(insert));
    window
        .insert_and_select(&transaction, inserted)
        .map_err(view_failure)?;
    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window_session::test_fixtures::{
        identity, sample_document, saved_project_with, sid, window,
    };
    use stellar_commands::{node_ops::DeleteSubtree, selection::Selection};

    #[test]
    fn copy_selection_then_automatic_paste_preserves_history_and_destination() {
        let model = sample_document();
        let dir = saved_project_with("clipboard-auto", &[&model]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        assert!(copy_from(&session, sid(3), session.revision()).is_none());
        session.select(Selection::new([sid(11)], None)).unwrap();
        let revision = session.revision();
        let history = session.history().history().len();
        let bytes = copy_from(&session, sid(3), revision).unwrap();
        assert_eq!(session.revision(), revision);
        assert_eq!(session.history().history().len(), history);
        assert!(!session.has_unsaved_changes());
        let pasted = paste_into(
            &mut session,
            RequestId(1),
            sid(3),
            revision,
            StableId::NIL,
            &bytes,
        )
        .unwrap_or_else(|e| panic!("{}", e.message));
        assert_eq!(
            session
                .document()
                .unwrap()
                .model()
                .record(pasted)
                .unwrap()
                .parent,
            Some(sid(11))
        );
        assert!(session.undo().unwrap());
        assert_eq!(session.view().selection().ids(), [sid(11)]);
        assert!(session.redo().unwrap());
        session.select(Selection::default()).unwrap();
        let revision = session.revision();
        let at_root = paste_into(
            &mut session,
            RequestId(2),
            sid(3),
            revision,
            StableId::NIL,
            &bytes,
        )
        .unwrap_or_else(|e| panic!("{}", e.message));
        assert_eq!(
            session
                .document()
                .unwrap()
                .model()
                .record(at_root)
                .unwrap()
                .parent,
            Some(sid(10))
        );
        session
            .save_with_lock_dir(|| Ok(dir.path().join("locks")))
            .unwrap();
        let reopened = ProjectWindowSession::load(identity(dir.path()), window(2)).unwrap();
        assert_eq!(
            reopened.document().unwrap().model().payload(pasted),
            model.payload(sid(11))
        );
        assert_eq!(
            reopened.document().unwrap().model().payload(at_root),
            model.payload(sid(11))
        );
    }

    #[test]
    fn automatic_paste_uses_leaf_parent_and_rejects_multiple_or_locked_destination() {
        let mut model = sample_document();
        let mut leaf = model.remove_leaf(sid(12)).unwrap();
        leaf.payload = stellar_document::records::NodePayload::Text(
            stellar_document::text::TextPayload::empty(0),
        );
        model.insert_node(leaf).unwrap();
        let bytes = ClipboardSubtree::capture(&model, sid(12)).unwrap().encode();
        let dir = saved_project_with("clipboard-leaf-parent", &[&model]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        session.select(Selection::new([sid(12)], None)).unwrap();
        let revision = session.revision();
        let pasted = paste_into(
            &mut session,
            RequestId(1),
            sid(3),
            revision,
            StableId::NIL,
            &bytes,
        )
        .unwrap_or_else(|e| panic!("{}", e.message));
        assert_eq!(
            session
                .document()
                .unwrap()
                .model()
                .record(pasted)
                .unwrap()
                .parent,
            Some(sid(10))
        );
        session
            .select(Selection::new([sid(11), sid(12)], None))
            .unwrap();
        let revision = session.revision();
        let history = session.history().history().len();
        assert!(paste_into(
            &mut session,
            RequestId(2),
            sid(3),
            revision,
            StableId::NIL,
            &bytes
        )
        .is_err());
        assert_eq!(session.history().history().len(), history);
        model.replace_flags(sid(10), true, true).unwrap();
        let dir = saved_project_with("clipboard-auto-locked", &[&model]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(2)).unwrap();
        session.select(Selection::new([sid(12)], None)).unwrap();
        let revision = session.revision();
        assert!(paste_into(
            &mut session,
            RequestId(3),
            sid(3),
            revision,
            StableId::NIL,
            &bytes
        )
        .is_err());
        assert!(!session.has_unsaved_changes());
    }

    #[test]
    fn fragment_destination_preserves_history_save_and_view_lock_guards() {
        use stellar_document::{fragment::FragmentPayload, records::NodePayload};
        for (destination, locked) in [(11, None), (11, Some(11)), (11, Some(10)), (20, None)] {
            let mut model = sample_document();
            // The other View keeps its required Frame root; its child is a Fragment.
            let fragment = if destination == 20 { 21 } else { 11 };
            let mut record = model.remove_leaf(sid(11)).unwrap();
            record.id = sid(fragment);
            record.parent = Some(sid(if destination == 20 { 20 } else { 10 }));
            record.payload = NodePayload::Fragment(FragmentPayload);
            model.insert_node(record).unwrap();
            let bytes = ClipboardSubtree::capture(&model, sid(12)).unwrap().encode();
            if let Some(id) = locked {
                model.replace_flags(sid(id), true, true).unwrap();
            }
            let dir = saved_project_with("clipboard-fragment", &[&model]);
            let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
            session
                .select(Selection::new([sid(fragment)], None))
                .unwrap();
            let revision = session.revision();
            let history = session.history().history().len();
            let result = paste_into(
                &mut session,
                RequestId(1),
                sid(3),
                revision,
                StableId::NIL,
                &bytes,
            );
            if locked.is_some() || destination == 20 {
                assert!(result.is_err());
                // Explicit destinations obey the same locks and View boundary.
                assert!(paste_into(
                    &mut session,
                    RequestId(2),
                    sid(3),
                    revision,
                    sid(fragment),
                    &bytes,
                )
                .is_err());
                assert_eq!(session.revision(), revision);
                assert_eq!(session.history().history().len(), history);
                assert_eq!(session.view().selection().ids(), [sid(fragment)]);
                assert!(!session.has_unsaved_changes());
                continue;
            }
            let pasted = result.unwrap_or_else(|error| panic!("{}", error.message));
            assert_eq!(
                session
                    .document()
                    .unwrap()
                    .model()
                    .record(pasted)
                    .unwrap()
                    .parent,
                Some(sid(fragment))
            );
            assert_eq!(session.view().selection().ids(), [pasted]);
            assert_eq!(session.history().history().len(), history + 1);
            assert!(session.undo().unwrap());
            assert_eq!(session.view().selection().ids(), [sid(fragment)]);
            assert!(!session.document().unwrap().model().contains(pasted));
            assert!(session.redo().unwrap());
            assert_eq!(session.view().selection().ids(), [pasted]);
            let revision = session.revision();
            let explicit = paste_into(
                &mut session,
                RequestId(2),
                sid(3),
                revision,
                sid(fragment),
                &bytes,
            )
            .unwrap_or_else(|error| panic!("{}", error.message));
            session
                .save_with_lock_dir(|| Ok(dir.path().join("locks")))
                .unwrap();
            let reopened = ProjectWindowSession::load(identity(dir.path()), window(2)).unwrap();
            for id in [pasted, explicit] {
                let saved = reopened.document().unwrap().model();
                assert_eq!(saved.record(id).unwrap().parent, Some(sid(fragment)));
                assert_eq!(saved.payload(id), model.payload(sid(12)));
            }
        }
    }

    #[test]
    fn copy_rejects_stale_root_locked_and_multiple_selections() {
        for (ids, locked, stale) in [
            (vec![10], None, false),
            (vec![11, 12], None, false),
            (vec![11], Some(11), false),
            (vec![11], Some(10), false),
            (vec![11], None, true),
        ] {
            let mut model = sample_document();
            if let Some(id) = locked {
                model.replace_flags(sid(id), true, true).unwrap();
            }
            let dir = saved_project_with("clipboard-copy-reject", &[&model]);
            let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
            session
                .select(Selection::new(ids.into_iter().map(sid), None))
                .unwrap();
            let revision = if stale {
                DocumentRevision(session.revision().0 + 1)
            } else {
                session.revision()
            };
            assert!(copy_from(&session, sid(3), revision).is_none());
            assert!(!session.has_unsaved_changes());
        }
    }

    #[test]
    fn clipboard_survives_source_deletion_and_paste_history_save_reopen() {
        let mut model = sample_document();
        let mut child = model.remove_leaf(sid(12)).unwrap();
        child.parent = Some(sid(11));
        model.insert_node(child).unwrap();
        let bytes = ClipboardSubtree::capture(&model, sid(11)).unwrap().encode();
        let dir = saved_project_with("clipboard-history", &[&model]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        session.select(Selection::new([sid(11)], None)).unwrap();
        let tx = CommandTransaction::new(
            command_id(session.window(), RequestId(1)),
            CommandOrigin::User,
        )
        .with_command(
            session.revision(),
            Command::DeleteSubtree(DeleteSubtree { root: sid(11) }),
        );
        session.apply(&tx).unwrap();
        let selection = session.view().selection().clone();
        let revision = session.revision();
        let copy = paste_into(
            &mut session,
            RequestId(2),
            sid(3),
            revision,
            sid(10),
            &bytes,
        )
        .unwrap_or_else(|error| panic!("{}", error.message));
        let child = session.document().unwrap().model().children_in_order(copy)[0];
        assert_ne!(child, sid(12));
        assert_ne!(copy, sid(11));
        assert_eq!(session.view().selection().ids(), [copy]);
        assert!(session.undo().unwrap());
        assert_eq!(session.view().selection(), &selection);
        assert!(!session.document().unwrap().model().contains(copy));
        assert!(session.redo().unwrap());
        assert_eq!(session.view().selection().ids(), [copy]);
        let revision = session.revision();
        let second = paste_into(&mut session, RequestId(3), sid(3), revision, copy, &bytes)
            .unwrap_or_else(|error| panic!("{}", error.message));
        assert_ne!(copy, second);
        session
            .save_with_lock_dir(|| Ok(dir.path().join("locks")))
            .unwrap();
        let reopened = ProjectWindowSession::load(identity(dir.path()), window(2)).unwrap();
        let saved = reopened.document().unwrap().model();
        assert_eq!(saved.payload(copy), model.payload(sid(11)));
        assert_eq!(saved.payload(child), model.payload(sid(12)));
        assert_eq!(saved.record(second).unwrap().parent, Some(copy));
        assert!(!saved.contains(sid(11)));
    }

    #[test]
    fn invalid_destination_and_bytes_leave_history_unchanged() {
        for (parent, view, locked, malformed) in [
            (11, 3, Some(10), false),
            (11, 3, Some(11), false),
            (11, 4, None, false),
            (999, 3, None, false),
            (10, 3, None, true),
        ] {
            let mut model = sample_document();
            let mut bytes = ClipboardSubtree::capture(&model, sid(11)).unwrap().encode();
            if let Some(id) = locked {
                model.replace_flags(sid(id), true, true).unwrap();
            }
            if malformed {
                bytes = vec![0];
            }
            let dir = saved_project_with("clipboard-reject", &[&model]);
            let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
            let revision = session.revision();
            let history = session.history().history().len();
            assert!(paste_into(
                &mut session,
                RequestId(1),
                sid(view),
                revision,
                sid(parent),
                &bytes
            )
            .is_err());
            assert_eq!(session.revision(), revision);
            assert_eq!(session.history().history().len(), history);
            assert!(!session.has_unsaved_changes());
        }
    }
    #[test]
    fn stale_paste_is_rejected_after_first_paste() {
        let model = sample_document();
        let bytes = ClipboardSubtree::capture(&model, sid(11)).unwrap().encode();
        let dir = saved_project_with("clipboard-stale", &[&model]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        let revision = session.revision();
        paste_into(
            &mut session,
            RequestId(1),
            sid(3),
            revision,
            sid(10),
            &bytes,
        )
        .unwrap_or_else(|error| panic!("{}", error.message));
        let selection = session.view().selection().clone();
        let history = session.history().history().len();
        assert!(paste_into(
            &mut session,
            RequestId(2),
            sid(3),
            revision,
            sid(10),
            &bytes
        )
        .is_err());
        assert_eq!(session.view().selection(), &selection);
        assert_eq!(session.history().history().len(), history);
    }

    #[test]
    fn untrusted_paste_request_is_rejected() {
        use crate::app_window_tests::{next_event, open_project, open_window};
        let model = sample_document();
        let bytes = ClipboardSubtree::capture(&model, sid(11)).unwrap().encode();
        let dir = saved_project_with("clipboard-untrusted", &[&model]);
        let app = AppRuntime::start().unwrap();
        let owner = open_project(&app, dir.path());
        let (session, _, _) = open_window(&app, owner);
        let revision = app.state().window(session).unwrap().revision;
        assert!(app
            .copy_selected_subtree(session, sid(3), revision)
            .unwrap()
            .wait()
            .unwrap()
            .is_none());
        let request = app
            .paste_node_subtree(session, sid(3), revision, sid(10), bytes)
            .unwrap();
        assert!(matches!(next_event(&app), RuntimeEvent::RequestFailed {
            request: got, failure: RequestFailure::TrustUnavailable, ..
        } if got == request));
        assert_eq!(app.state().window(session).unwrap().revision, revision);
    }
}
