//! Platform clipboard bytes enter the owning window's ordinary document history.
use super::frame_move::{command_id, FreshIds};
use super::*;
use stellar_commands::{
    command::Command, id::CommandOrigin, paste::ClipboardSubtree, transaction::CommandTransaction,
};

impl AppRuntime {
    /// 요청한 창이 표시한 선택만 복사합니다. 셸 worker 전용이며 UI thread에서 ticket을 기다리지 않습니다.
    pub fn copy_displayed_subtree(
        &self,
        target: SelectionEditTarget,
    ) -> Result<crate::document_thread::JobTicket<Option<Vec<u8>>>, SubmitError> {
        self.sender.submit(move |ctx| {
            let window = window_mut(ctx, target.session).ok()?;
            target.verify(window).ok()?;
            copy_from(window, target.view, target.revision)
        })
    }

    /// 같은 문서의 clipboard subtree를 붙여넣습니다. 목적지는 실행 시점의 선택에서 정합니다:
    /// 선택한 컨테이너 안, 그 외 단일 선택의 부모 아래, 선택이 없으면 활성 View root 아래입니다.
    /// 표시한 선택·revision과 조상 잠금은 document thread에서 다시 확인합니다.
    pub fn paste_displayed_subtree(
        &self,
        target: SelectionEditTarget,
        bytes: Vec<u8>,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            let window = window_mut(ctx, target.session)?;
            target.verify(window)?;
            paste_into(window, request, target.view, target.revision, &bytes)?;
            Ok(RuntimeEvent::WindowViewChanged {
                request,
                session: target.session,
            })
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
    // 목적지는 현재 문서 상태의 선택에서 정합니다.
    let parent = paste_parent(model, root, window.view().selection().ids()).ok_or_else(invalid)?;
    let clipboard = ClipboardSubtree::decode(bytes).map_err(|_| invalid())?;
    let id = command_id(window.window(), request);
    let mut ids = FreshIds::default();
    let order = model.next_child_order_key(parent);
    let insert = clipboard.plan_with(model, parent, order, &mut || ids.next());
    ids.finish()?;
    let insert =
        insert.map_err(|error| Failure::new(RequestFailure::UnknownView, error.to_string()))?;
    let inserted = insert.nodes[0].id;
    let transaction = CommandTransaction::new(id, CommandOrigin::User)
        .with_command(revision, Command::InsertNode(insert));
    window
        .insert_and_select(&transaction, inserted)
        .map_err(view_failure)?;
    Ok(inserted)
}

/// 붙여넣을 부모. 선택에서 정합니다(없으면 View root, 컨테이너 하나면 그 안, 아니면 그 부모).
/// 부모는 자식을 가질 수 있고 그부터 root까지 잠기지 않아야 합니다. 메뉴 활성(선택 능력 mask)도 이 규칙을 씁니다.
pub(super) fn paste_parent(
    model: &stellar_document::model::DocumentModel,
    root: StableId,
    selected: &[StableId],
) -> Option<StableId> {
    // 선택이 없으면 root, 자식을 가질 수 있는 노드 하나면 그 노드, 아니면 그 부모입니다.
    let parent = match selected {
        [] => root,
        [selected] => {
            let payload = model.payload(*selected)?;
            if stellar_document::validate::can_have_children(payload.kind()) {
                *selected
            } else {
                model
                    .runtime_id(*selected)
                    .and_then(|id| model.columns().parent(id))
                    .and_then(|id| model.stable_id(id))?
            }
        }
        _ => return None,
    };
    let target = model.payload(parent)?;
    if !stellar_document::validate::can_have_children(target.kind()) {
        return None;
    }
    (!super::sidebar::path_locked(model, parent, root)?).then_some(parent)
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
        let pasted = paste_into(&mut session, RequestId(1), sid(3), revision, &bytes)
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
        let at_root = paste_into(&mut session, RequestId(2), sid(3), revision, &bytes)
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
        let pasted = paste_into(&mut session, RequestId(1), sid(3), revision, &bytes)
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
        assert!(paste_into(&mut session, RequestId(2), sid(3), revision, &bytes).is_err());
        assert_eq!(session.history().history().len(), history);
        model.replace_flags(sid(10), true, true).unwrap();
        let dir = saved_project_with("clipboard-auto-locked", &[&model]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(2)).unwrap();
        session.select(Selection::new([sid(12)], None)).unwrap();
        let revision = session.revision();
        assert!(paste_into(&mut session, RequestId(3), sid(3), revision, &bytes).is_err());
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
            let result = paste_into(&mut session, RequestId(1), sid(3), revision, &bytes);
            if locked.is_some() || destination == 20 {
                assert!(result.is_err());
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
            // 목적지는 선택에서 정하므로 Fragment를 다시 선택해 그 안에 한 번 더 붙여넣습니다.
            session
                .select(Selection::new([sid(fragment)], None))
                .unwrap();
            let revision = session.revision();
            let explicit = paste_into(&mut session, RequestId(2), sid(3), revision, &bytes)
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
        // 선택이 비었으므로 활성 View root 아래에 붙여넣습니다.
        let copy = paste_into(&mut session, RequestId(2), sid(3), revision, &bytes)
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
        // 붙여넣은 Frame이 선택되어 있으므로 그 안에 붙여넣습니다.
        let second = paste_into(&mut session, RequestId(3), sid(3), revision, &bytes)
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
        for (selected, view, locked, malformed) in [
            (Some(11), 3, Some(10), false),
            (Some(11), 3, Some(11), false),
            (Some(11), 4, None, false),
            (None, 3, None, true),
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
            if let Some(id) = selected {
                session.select(Selection::new([sid(id)], None)).unwrap();
            }
            let revision = session.revision();
            let history = session.history().history().len();
            assert!(paste_into(&mut session, RequestId(1), sid(view), revision, &bytes).is_err());
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
        paste_into(&mut session, RequestId(1), sid(3), revision, &bytes)
            .unwrap_or_else(|error| panic!("{}", error.message));
        let selection = session.view().selection().clone();
        let history = session.history().history().len();
        assert!(paste_into(&mut session, RequestId(2), sid(3), revision, &bytes).is_err());
        assert_eq!(session.view().selection(), &selection);
        assert_eq!(session.history().history().len(), history);
    }

    #[test]
    fn clipboard_rejects_same_revision_selection_changes_before_queued_work() {
        use crate::app_window_tests::{next_event, open_project, open_window};
        use std::sync::mpsc;

        let dir = saved_project_with("queued-clipboard", &[&sample_document()]);
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
        let changed = |request| {
            assert!(
                matches!(next_event(&app), RuntimeEvent::WindowViewChanged { request: got, .. } if got == request)
            )
        };
        let rejected = |request| {
            assert!(
                matches!(next_event(&app), RuntimeEvent::RequestFailed { request: got, failure: RequestFailure::UnknownView, .. } if got == request)
            )
        };
        changed(app.set_selection(session, vec![sid(11)]).unwrap());
        let revision = app.state().window(session).unwrap().revision;
        let target = SelectionEditTarget {
            session,
            view: sid(3),
            revision,
            history_sequence: app.state().window(session).unwrap().history_sequence,
        };
        let bytes = app
            .copy_displayed_subtree(target)
            .unwrap()
            .wait()
            .unwrap()
            .unwrap();
        let (release, wait) = mpsc::channel();
        let _barrier = app.sender.submit(move |_| wait.recv().unwrap()).unwrap();
        let select = app.set_selection(session, vec![sid(12)]).unwrap();
        // 접수 시점에는 A가 게시되어 있지만 실행 시점에는 B가 선택되어 있습니다.
        let copy = app.copy_displayed_subtree(target).unwrap();
        let paste = app.paste_displayed_subtree(target, bytes.clone()).unwrap();
        release.send(()).unwrap();
        changed(select);
        assert!(copy.wait().unwrap().is_none());
        rejected(paste);
        assert!(app
            .copy_displayed_subtree(target)
            .unwrap()
            .wait()
            .unwrap()
            .is_none());
        rejected(app.paste_displayed_subtree(target, bytes.clone()).unwrap());
        let state = app.state();
        let window = state.window(session).unwrap();
        assert_eq!(window.revision, revision);
        assert_eq!(window.undo_depth, 2);
        assert!(!window.has_unsaved_changes);
        app.undo(session).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::HistoryReplayed { applied: true, .. }
        ));
        assert_eq!(
            app.copy_displayed_subtree(target).unwrap().wait().unwrap(),
            Some(bytes.clone())
        );
        changed(app.paste_displayed_subtree(target, bytes).unwrap());
        assert_eq!(
            app.state().window(session).unwrap().revision.0,
            revision.0 + 1
        );
    }

    #[test]
    fn restricted_mode_copy_and_paste_apply_without_project_trust() {
        // 제한 모드는 문서 편집을 막지 않습니다(사용자 결정 D1).
        use crate::app_window_tests::{next_event, open_project, open_window};
        let model = sample_document();
        let dir = saved_project_with("clipboard-untrusted", &[&model]);
        let app = AppRuntime::start().unwrap();
        let owner = open_project(&app, dir.path());
        assert_ne!(
            app.state().project(owner).unwrap().trust,
            TrustStatus::Trusted
        );
        let (session, _, _) = open_window(&app, owner);
        app.set_selection(session, vec![sid(11)]).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::WindowViewChanged { .. }
        ));
        let state = app.state();
        let window = state.window(session).unwrap();
        let target = SelectionEditTarget {
            session,
            view: window.active_view.unwrap(),
            revision: window.revision,
            history_sequence: window.history_sequence,
        };
        let bytes = app
            .copy_displayed_subtree(target)
            .unwrap()
            .wait()
            .unwrap()
            .expect("restricted mode copies the displayed selection");
        let request = app.paste_displayed_subtree(target, bytes).unwrap();
        assert!(
            matches!(next_event(&app), RuntimeEvent::WindowViewChanged { request: got, .. } if got == request)
        );
        assert_eq!(
            app.state().window(session).unwrap().revision.0,
            target.revision.0 + 1
        );
    }
}
