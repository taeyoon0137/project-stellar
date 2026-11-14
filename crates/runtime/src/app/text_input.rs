//! 한 IME 조합의 미확정 본문과 확정 요청. 중간 입력은 창 문서와 history 밖에 둡니다.
use super::frame_move::{command_id, trusted_canvas_window};
use super::*;
use crate::document_thread::JobTicket;
use stellar_commands::{
    command::Command, id::CommandOrigin, text_edit::ReplaceText, transaction::CommandTransaction,
};
use stellar_document::{
    records::NodePayload,
    text::{edit_text, TextError, TextPayload, TextRange},
};

#[derive(Debug, Clone, Copy)]
struct Target {
    session: WindowSessionHandle,
    view: StableId,
    revision: DocumentRevision,
    node: StableId,
}

/// 표시된 Text 선택에서 시작한 조합 하나. 취소할 때는 이 값만 버립니다.
pub struct CanvasTextInput {
    target: Target,
    window: WindowSessionId,
    original: TextPayload,
    replacement: TextRange,
    marked: Option<(String, TextRange)>,
}

impl CanvasTextInput {
    /// 조합 시작 시점의 문서 본문과 문단·스타일입니다.
    pub fn original(&self) -> &TextPayload {
        &self.original
    }

    /// IME가 문서에서 대체할 scalar 범위입니다.
    pub fn replacement(&self) -> TextRange {
        self.replacement
    }

    /// 조합 본문과 조합 안의 상대 선택. 잘못된 갱신은 직전 조합을 보존합니다.
    pub fn set_marked(&mut self, text: String, selection: TextRange) -> bool {
        if selection.start > selection.end || (selection.end.0 as usize) > text.chars().count() {
            return false;
        }
        self.marked = Some((text, selection));
        true
    }

    /// 조합 안의 selection은 표시할 문자열의 Unicode scalar 좌표입니다.
    pub fn marked(&self) -> Option<(&str, TextRange)> {
        self.marked
            .as_ref()
            .map(|(text, range)| (text.as_str(), *range))
    }

    /// Phase X 레이아웃 입력에 쓰는 미확정 payload. 저장에 쓰는 편집 규칙과 동일합니다.
    pub fn preview(&self) -> Result<std::borrow::Cow<'_, TextPayload>, TextError> {
        let Some((text, _)) = self.marked() else {
            return Ok(std::borrow::Cow::Borrowed(&self.original));
        };
        if self.original.slice(self.replacement) == Some(text) {
            return Ok(std::borrow::Cow::Borrowed(&self.original));
        }
        edit_text(&self.original, self.replacement, text, &[]).map(std::borrow::Cow::Owned)
    }
}

impl AppRuntime {
    /// UI는 ticket을 비동기로 받습니다. 문서 스레드에서 신뢰·선택·View·revision을 확인합니다.
    pub fn begin_canvas_text(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
        node: StableId,
        replacement: TextRange,
    ) -> Result<JobTicket<Option<CanvasTextInput>>, SubmitError> {
        let target = Target {
            session,
            view,
            revision,
            node,
        };
        self.sender.submit(move |ctx| {
            let window = trusted_canvas_window(ctx, session).ok()?;
            begin(window, target, replacement)
        })
    }

    /// 플랫폼 입력기가 최종 확정한 본문을 단일 사용자 transaction으로 보냅니다.
    /// 즉시 재시도할 수 있도록 큐가 가득 차도 호출자의 미확정 조합을 소비하지 않습니다.
    pub fn commit_canvas_text(
        &self,
        input: &CanvasTextInput,
        text: String,
    ) -> Result<RequestId, RequestError> {
        let target = input.target;
        let range = input.replacement;
        let owner = input.window;
        self.submit(move |ctx, request| {
            let window = trusted_canvas_window(ctx, target.session)?;
            if window.window() != owner {
                return Err(Failure::new(
                    RequestFailure::UnknownWindowSession,
                    "Text 입력이 시작된 창이 닫혔습니다",
                ));
            }
            commit(window, target, range, text, request)?;
            Ok(RuntimeEvent::WindowViewChanged {
                request,
                session: target.session,
            })
        })
    }
}

fn begin(
    window: &ProjectWindowSession,
    target: Target,
    replacement: TextRange,
) -> Option<CanvasTextInput> {
    let source = editable_text(window, target)?;
    source.slice(replacement)?;
    Some(CanvasTextInput {
        target,
        window: window.window(),
        original: source.clone(),
        replacement,
        marked: None,
    })
}

fn editable_text(window: &ProjectWindowSession, target: Target) -> Option<&TextPayload> {
    if window.revision() != target.revision
        || window.tabs().active_view()? != target.view
        || window.view().selection().ids() != [target.node]
        || window.view().area_selection_active()
    {
        return None;
    }
    let model = window.document()?.model();
    let NodePayload::Text(payload) = model.payload(target.node)? else {
        return None;
    };
    let root = model
        .definition()
        .pages
        .iter()
        .flat_map(|p| &p.views)
        .find(|v| v.id == target.view)?
        .root_frame;
    let mut current = model.runtime_id(target.node)?;
    loop {
        let row = model.columns().get(current)?;
        if !row.visible || row.locked {
            return None;
        }
        if row.stable_id == root {
            return Some(payload);
        }
        current = row.parent?;
    }
}

fn commit(
    window: &mut ProjectWindowSession,
    target: Target,
    range: TextRange,
    text: String,
    request: RequestId,
) -> Result<(), Failure> {
    let original = editable_text(window, target)
        .and_then(|payload| payload.slice(range))
        .ok_or_else(|| {
            Failure::new(
                RequestFailure::UnknownView,
                "Text 편집의 문서·선택·편집 상태가 바뀌었습니다",
            )
        })?;
    if original == text {
        return Ok(());
    }
    let tx = CommandTransaction::new(command_id(window.window(), request), CommandOrigin::User)
        .with_command(
            target.revision,
            Command::ReplaceText(ReplaceText {
                node: target.node,
                range,
                replacement: text,
            }),
        );
    window.apply(&tx).map_err(view_failure)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window_session::test_fixtures::{
        identity, sample_document, saved_project_with, sid, window, TempDir,
    };
    use stellar_commands::selection::Selection;
    use stellar_core_types::text::UserText;
    use stellar_document::text::{Paragraph, StyleRun};

    fn range(start: u32, end: u32) -> TextRange {
        TextRange::new(start, end).unwrap()
    }

    fn fixture(locked: bool) -> (TempDir, ProjectWindowSession, Target) {
        let mut model = sample_document();
        let mut record = model.remove_leaf(sid(11)).unwrap();
        record.payload = NodePayload::Text(
            TextPayload::new(
                UserText::new("🎨old\n끝"),
                vec![Paragraph { style: 1 }, Paragraph { style: 2 }],
                vec![StyleRun {
                    range: range(1, 4),
                    style: 3,
                }],
            )
            .unwrap(),
        );
        model.insert_node(record).unwrap();
        model.replace_flags(sid(10), true, locked).unwrap();
        let dir = saved_project_with("text-input", &[&model]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        session.select(Selection::new([sid(11)], None)).unwrap();
        let target = Target {
            session: WindowSessionHandle(1),
            view: sid(3),
            node: sid(11),
            revision: session.revision(),
        };
        (dir, session, target)
    }

    #[test]
    fn hangul_transient_preview_is_one_undoable_saved_edit() {
        let (dir, mut session, target) = fixture(false);
        let mut input = begin(&session, target, range(1, 4)).unwrap();
        let original = input.original().clone();
        let history_before = session.history().history().len();
        for step in ["ㅎ", "하", "한"] {
            assert!(input.set_marked(step.to_owned(), range(1, 1)));
            assert_eq!(input.preview().unwrap().content(), format!("🎨{step}\n끝"));
            assert_eq!(input.preview().unwrap().paragraphs(), original.paragraphs());
            assert_eq!(editable_text(&session, target), Some(&original));
            assert_eq!(session.history().history().len(), history_before);
            assert!(!session.has_unsaved_changes());
        }
        assert!(!input.set_marked("가".into(), range(0, 2)));
        assert_eq!(input.marked(), Some(("한", range(1, 1))));
        let preview = input.preview().unwrap().into_owned();
        assert!(commit(
            &mut session,
            target,
            input.replacement(),
            "한".into(),
            RequestId(1)
        )
        .is_ok());
        assert_eq!(session.history().history().len(), history_before + 1);
        assert!(session.undo().unwrap());
        assert_eq!(
            session.document().unwrap().model().payload(target.node),
            Some(&NodePayload::Text(original))
        );
        assert!(session.redo().unwrap());
        session
            .save_with_lock_dir(|| Ok(dir.path().join("locks")))
            .unwrap();
        let reopened = ProjectWindowSession::load(identity(dir.path()), window(2)).unwrap();
        assert_eq!(
            reopened.document().unwrap().model().payload(target.node),
            Some(&NodePayload::Text(preview))
        );
    }

    #[test]
    fn cancel_stale_and_locked_targets_leave_document_alone() {
        let (_dir, mut session, target) = fixture(false);
        let mut input = begin(&session, target, range(1, 4)).unwrap();
        assert!(input.set_marked("かな".into(), range(0, 2)));
        drop(input);
        assert!(!session.has_unsaved_changes());
        assert!(begin(&session, target, range(100, 100)).is_none());
        assert!(begin(
            &session,
            Target {
                view: sid(4),
                ..target
            },
            range(0, 0)
        )
        .is_none());
        let history = session.history().history().len();
        assert!(commit(
            &mut session,
            target,
            range(1, 4),
            "old".to_owned(),
            RequestId(1)
        )
        .is_ok());
        assert_eq!(session.history().history().len(), history);
        assert!(commit(
            &mut session,
            target,
            range(1, 4),
            "新".to_owned(),
            RequestId(2)
        )
        .is_ok());
        assert!(commit(
            &mut session,
            target,
            range(1, 4),
            "古".to_owned(),
            RequestId(3)
        )
        .is_err());
        assert!(session.undo().unwrap());
        let other = Target {
            revision: session.revision(),
            ..target
        };
        session.select(Selection::new([sid(12)], None)).unwrap();
        assert!(commit(
            &mut session,
            other,
            range(1, 4),
            "別".to_owned(),
            RequestId(4)
        )
        .is_err());
        let (_dir, session, target) = fixture(true);
        assert!(begin(&session, target, range(0, 0)).is_none());
    }

    #[test]
    fn runtime_checks_window_trust_and_commits_only_when_requested() {
        use crate::app_window_tests::{next_event, open_project, open_window};
        let (dir, _, _) = fixture(false);
        let app = AppRuntime::start().unwrap();
        let project = open_project(&app, dir.path());
        let (handle, _, _) = open_window(&app, project);
        app.set_selection(handle, vec![sid(11)]).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::WindowViewChanged { .. }
        ));
        let revision = app.state().window(handle).unwrap().revision;
        let begin = || {
            app.begin_canvas_text(handle, sid(3), revision, sid(11), range(1, 4))
                .unwrap()
                .wait()
                .unwrap()
        };
        assert!(begin().is_none());
        // Drive the same document-thread trust value without writing user preferences in a Rust test.
        let trust = |trusted| {
            app.sender
                .submit(move |ctx| {
                    ctx.app_sessions_mut()
                        .projects
                        .get_mut(&project)
                        .unwrap()
                        .trust = if trusted {
                        TrustStatus::Trusted
                    } else {
                        TrustStatus::NotTrusted
                    };
                })
                .unwrap()
                .wait()
                .unwrap()
        };
        trust(true);
        let mut input = begin().unwrap();
        assert!(input.set_marked("ㅎ".into(), range(1, 1)));
        assert!(!app.state().window(handle).unwrap().has_unsaved_changes);
        trust(false);
        app.commit_canvas_text(&input, "한".to_owned()).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::RequestFailed {
                failure: RequestFailure::TrustUnavailable,
                ..
            }
        ));
        assert_eq!(app.state().window(handle).unwrap().revision, revision);
        trust(true);
        app.commit_canvas_text(&input, "한".to_owned()).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::WindowViewChanged { .. }
        ));
        assert!(app.state().window(handle).unwrap().has_unsaved_changes);
        app.commit_canvas_text(&input, "두 번".to_owned()).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::RequestFailed { .. }
        ));
    }
}
