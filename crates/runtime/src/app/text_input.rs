//! 앱 창의 캔버스 Text 편집 요청(ADR-0067 §6, ADR-0129 결정 2).
//!
//! 편집 규칙은 코어 편집기 [`CanvasTextEditor`](`canvas::text::edit`, ADR-0118 결정 1·2·3) 하나가 정합니다. 이 모듈은
//! 창 session마다 진행 중 편집기 하나를 두고, 비동기 요청을 그 편집기 호출로 바꾸고, 기록을 창 session(Undo 메뉴
//! 이름표 포함)에 넘기는 일만 합니다. 조합 범위 하나를 바로 기록하던 옛 별도 구현은 지웠습니다.
//!
//! - 조합 중 본문은 문서와 history에 넣지 않고, 같은 창의 canvas scene이 미리보기로 그립니다.
//! - 확정·직접 입력만 Text 범위 편집으로 기록합니다(입력 종류별 grouping key, 새 Text는 첫 확정 때 생성).
//! - 편집 가능 여부는 `commands::lock::edit_permission`과 활성 View 소속으로만 정합니다. 프로젝트 신뢰는 보지
//!   않습니다(D1).
//! - 편집은 [`AppRuntime::end_canvas_text`]로 끝냅니다. 이 편집에서 만든 Text를 모두 지우고 끝내면 빈 Text를 지우는
//!   항목 하나를 기록합니다(ADR-0129 결정 1).
use super::frame_move::command_id;
use super::sidebar::{in_view, view_root};
use super::*;
use crate::canvas::text::edit::{
    CanvasTextEditor, CaretMove, EditEnd, TextEditError, TextEditHost, TextPlacement, TextWidth,
};
use crate::canvas::text::TextPreview;
use crate::document_thread::JobTicket;
use crate::window_session::history_label;
use stellar_commands::{
    lock::{edit_permission, EditIntent},
    selection::Selection,
    state::DocumentState,
    transaction::CommandTransaction,
};
use stellar_core_types::{clock::MonotonicClock, value::Vec2};
use stellar_document::records::NodePayload;
use stellar_document::text::scalar_len;
use stellar_history::entry::{GroupingKey, HistorySequence};
use stellar_text::edit::{EditSelection, ScalarRange};

pub use stellar_document::text::TextRange;

/// 편집을 시작할 대상.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TextTarget {
    /// 기존 Text.
    Node(StableId),
    /// 부모 Frame local 좌표의 원점에 만들 새 자동 폭 Text(Text 도구 클릭).
    New {
        /// 자유 배치 부모 Frame.
        parent: StableId,
        /// 부모 local 원점.
        origin: Vec2,
    },
    /// Text 도구 결과(`plan_text_tool`)대로 만들 새 Text(끌기면 고정 폭).
    Placement(TextPlacement),
}

/// 창 하나의 진행 중 Text 편집. document thread만 소유합니다.
#[derive(Debug)]
pub(crate) struct TextComposition {
    token: u64,
    view: StableId,
    editor: CanvasTextEditor,
    /// 마지막으로 계산한 조합 미리보기와 그때의 문서 revision.
    preview: Option<TextPreview>,
    revision: DocumentRevision,
}

impl TextComposition {
    /// 미리보기를 계산한 revision이 그대로일 때만 scene에 미리보기를 그립니다.
    pub(crate) fn preview_at(&self, revision: DocumentRevision) -> Option<&TextPreview> {
        if self.revision == revision {
            self.preview.as_ref()
        } else {
            None
        }
    }

    fn refresh(&mut self, window: &ProjectWindowSession) {
        let Some(doc) = window.document() else {
            self.preview = None;
            return;
        };
        self.preview = self.editor.preview(doc);
        self.revision = doc.revision();
    }
}

/// history grouping 시각. 프로세스 안 단조 시계입니다.
struct InputClock;

impl MonotonicClock for InputClock {
    fn now_nanos(&self) -> u64 {
        static ORIGIN: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        let elapsed = ORIGIN
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_nanos();
        u64::try_from(elapsed).unwrap_or(u64::MAX)
    }
}

/// 창 session을 편집기 기록 host로 씁니다. 기록마다 Undo 메뉴 이름표를 붙입니다.
struct SessionHost<'a>(&'a mut ProjectWindowSession);

fn session_error(error: WindowSessionEditError) -> TextEditError {
    match error {
        WindowSessionEditError::History(error) => TextEditError::History(error),
        _ => TextEditError::Stale,
    }
}

impl TextEditHost for SessionHost<'_> {
    fn document(&self) -> Option<&DocumentState> {
        self.0.document()
    }

    fn selected(&self) -> &[StableId] {
        self.0.view().selection().ids()
    }

    fn record_input<C: MonotonicClock>(
        &mut self,
        transaction: &CommandTransaction,
        key: GroupingKey,
        clock: &C,
    ) -> Result<(), TextEditError> {
        self.0
            .apply_grouped(transaction, key, clock, history_label::TYPING)
            .map(drop)
            .map_err(session_error)
    }

    fn record_creation<C: MonotonicClock>(
        &mut self,
        transaction: &CommandTransaction,
        key: GroupingKey,
        node: StableId,
        clock: &C,
    ) -> Result<(), TextEditError> {
        self.0
            .insert_grouped_and_select(transaction, key, node, clock, history_label::TYPING)
            .map(drop)
            .map_err(session_error)
    }

    fn record_style(&mut self, transaction: &CommandTransaction) -> Result<(), TextEditError> {
        self.0
            .apply(transaction, history_label::CHANGE_PROPERTY)
            .map(drop)
            .map_err(session_error)
    }

    fn record_removal(&mut self, transaction: &CommandTransaction) -> Result<(), TextEditError> {
        self.0
            .apply(transaction, history_label::DELETE)
            .map(drop)
            .map_err(session_error)
    }

    fn select_only(&mut self, node: StableId) -> Result<Option<HistorySequence>, TextEditError> {
        self.0
            .select(Selection::new([node], Some(node)))
            .map_err(session_error)
    }
}

fn stale() -> Failure {
    Failure::new(
        RequestFailure::UnknownView,
        "Text 편집의 문서·View·편집 상태가 바뀌었습니다",
    )
}

fn edit_failure(error: TextEditError) -> Failure {
    let failure = match error {
        TextEditError::History(_) => RequestFailure::HistoryUnavailable,
        _ => RequestFailure::UnknownView,
    };
    Failure::new(failure, error.to_string())
}

fn entry_mut(
    ctx: &mut DocumentContext,
    session: WindowSessionHandle,
) -> Result<&mut WindowEntry, Failure> {
    ctx.app_sessions_mut()
        .windows
        .get_mut(&session)
        .ok_or_else(|| unknown_window(session))
}

/// 활성 View가 `view`이고 `node`가 그 View tree 안에 있는지.
fn in_active_view(window: &ProjectWindowSession, view: StableId, node: StableId) -> Option<()> {
    if window.tabs().active_view()? != view {
        return None;
    }
    let model = window.document()?.model();
    in_view(model, node, view_root(model, view)?)
}

/// 편집할 수 있는 Text의 저장된 본문(활성 View 안, 잠금 없음, 숨김은 막지 않음).
fn editable_text(window: &ProjectWindowSession, view: StableId, node: StableId) -> Option<String> {
    in_active_view(window, view, node)?;
    let model = window.document()?.model();
    let NodePayload::Text(payload) = model.payload(node)? else {
        return None;
    };
    edit_permission(model, node, EditIntent::Edit).ok()?;
    Some(payload.content().to_owned())
}

fn selection_of(range: TextRange) -> EditSelection {
    EditSelection {
        anchor: range.start.0,
        focus: range.end.0,
    }
}

/// 진행 중 편집을 끝냅니다(남은 조합 확정, 빈 새 Text 정리, Text 선택). 편집기는 버립니다.
fn finish(entry: &mut WindowEntry, request: RequestId) -> Result<Option<EditEnd>, Failure> {
    let Some(composition) = entry.text.take() else {
        return Ok(None);
    };
    let id = command_id(entry.session.window(), request);
    composition
        .editor
        .finish(&mut SessionHost(&mut entry.session), id, &InputClock)
        .map(Some)
        .map_err(edit_failure)
}

/// 편집을 시작합니다. `ids`는 새 Text가 쓸 node·Transform modifier ID입니다.
fn begin(
    entry: &mut WindowEntry,
    request: RequestId,
    view: StableId,
    revision: DocumentRevision,
    target: TextTarget,
    replacement: TextRange,
    ids: Option<[StableId; 2]>,
) -> Result<StableId, Failure> {
    if entry.session.revision() != revision || entry.session.tabs().active_view() != Some(view) {
        return Err(stale());
    }
    let token = request.0;
    if let TextTarget::Node(node) = target {
        // 같은 Text의 편집이 이어지면 편집기(이 편집에서 만들었는지 포함)를 그대로 씁니다.
        if let Some(live) = entry
            .text
            .as_mut()
            .filter(|c| c.view == view && c.editor.node() == node && c.editor.exists())
        {
            if replacement.end.0 > scalar_len(live.editor.buffer().text()) {
                return Err(stale());
            }
            live.editor.escape();
            let doc = entry.session.document().ok_or_else(stale)?;
            live.editor
                .set_selection(doc, selection_of(replacement))
                .map_err(edit_failure)?;
            live.token = token;
            live.refresh(&entry.session);
            return Ok(node);
        }
    }
    // 다른 대상의 편집이 남아 있으면 먼저 끝냅니다.
    finish(entry, request)?;
    if entry.session.revision() != revision {
        return Err(stale());
    }
    let window = &entry.session;
    let doc = window.document().ok_or_else(stale)?;
    let placement = match target {
        TextTarget::Node(node) => {
            let content = editable_text(window, view, node).ok_or_else(stale)?;
            if replacement.end.0 > scalar_len(&content) {
                return Err(stale());
            }
            None
        }
        TextTarget::New { parent, origin } => Some(TextPlacement {
            parent,
            origin,
            width: TextWidth::Auto,
        }),
        TextTarget::Placement(placement) => Some(placement),
    };
    let editor = match (target, placement) {
        (TextTarget::Node(node), _) => {
            CanvasTextEditor::begin_existing(doc, node, Some(selection_of(replacement)), token)
                .map_err(edit_failure)?
        }
        (_, Some(placement)) => {
            if replacement != TextRange::new(0, 0).expect("빈 범위") {
                return Err(stale());
            }
            in_active_view(window, view, placement.parent).ok_or_else(stale)?;
            let [node, modifier] = ids.ok_or_else(stale)?;
            CanvasTextEditor::begin_new(doc, placement, node, modifier, token)
                .map_err(edit_failure)?
        }
        (_, None) => return Err(stale()),
    };
    let node = editor.node();
    let mut composition = TextComposition {
        token,
        view,
        editor,
        preview: None,
        revision,
    };
    composition.refresh(&entry.session);
    entry.text = Some(composition);
    Ok(node)
}

/// 진행 중 편집기 하나에 일을 시킵니다. 대상이 사라졌거나 잠겼으면 편집기를 버립니다.
fn with_editor<T>(
    entry: &mut WindowEntry,
    token: u64,
    run: impl FnOnce(&mut CanvasTextEditor, &mut SessionHost<'_>) -> Result<T, TextEditError>,
) -> Result<T, Failure> {
    let composition = entry
        .text
        .as_mut()
        .filter(|c| c.token == token)
        .ok_or_else(stale)?;
    let result = run(
        &mut composition.editor,
        &mut SessionHost(&mut entry.session),
    );
    match result {
        Ok(value) => {
            composition.refresh(&entry.session);
            Ok(value)
        }
        Err(error @ (TextEditError::Stale | TextEditError::NotEditable)) => {
            entry.text = None;
            Err(edit_failure(error))
        }
        Err(error) => {
            composition.refresh(&entry.session);
            Err(edit_failure(error))
        }
    }
}

impl AppRuntime {
    /// Text 편집을 시작합니다. 반환한 요청 ID가 이후 편집 요청의 token이고, StableId는 편집 대상 node입니다(새
    /// Text는 첫 확정 때 이 ID로 만듭니다). `replacement`는 문서 전체 Unicode scalar 범위이며 편집 선택이 됩니다
    /// (이어 오는 조합이 바꿀 범위, 새 Text는 0..0). 같은 Text의 편집이 이어지면 그 편집기를 그대로 쓰고, 다른 대상의
    /// 편집이 남아 있으면 먼저 끝냅니다.
    pub fn begin_canvas_text(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
        target: TextTarget,
        replacement: TextRange,
    ) -> Result<(RequestId, StableId), RequestError> {
        let ids = match target {
            TextTarget::Node(_) => None,
            _ => Some(stellar_project::new_ids::new_ids().map_err(|_| RequestError::Stopped)?),
        };
        let node = match (target, ids) {
            (TextTarget::Node(node), _) => node,
            (_, Some([node, _])) => node,
            (_, None) => StableId::NIL,
        };
        self.submit(move |ctx, request| {
            let entry = entry_mut(ctx, session)?;
            begin(entry, request, view, revision, target, replacement, ids)?;
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
        .map(|request| (request, node))
    }

    /// 조합 중 본문을 바꿉니다(`setMarkedText`, 빈 문자열은 조합 취소). 문서와 history는 바뀌지 않고 scene
    /// 미리보기만 바뀝니다.
    pub fn mark_canvas_text(
        &self,
        session: WindowSessionHandle,
        token: u64,
        text: String,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            let entry = entry_mut(ctx, session)?;
            let caret = ScalarRange::caret(scalar_len(&text));
            with_editor(entry, token, |editor, host| {
                let doc = host.document().ok_or(TextEditError::Stale)?;
                editor.set_marked(doc, &text, caret)
            })?;
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
    }

    /// 확정 입력(`insertText`)을 기록합니다. 조합 중이면 조합을 이 문자열로 확정하고, 아니면 편집 선택을 바꿉니다.
    /// 본문이 같으면 history를 늘리지 않으며, 그때도 같은 완료 event를 보냅니다(셸은 revision이 그대로인지로
    /// 구분합니다). 편집은 이어집니다.
    pub fn commit_canvas_text(
        &self,
        session: WindowSessionHandle,
        token: u64,
        text: String,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            let entry = entry_mut(ctx, session)?;
            let id = command_id(entry.session.window(), request);
            with_editor(entry, token, |editor, host| {
                editor.insert(host, &text, id, &InputClock)
            })?;
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
    }

    /// 조합만 버립니다(편집은 이어짐). 이미 끝났거나 더 새 편집이 있으면 아무것도 바꾸지 않습니다.
    pub fn cancel_canvas_text(
        &self,
        session: WindowSessionHandle,
        token: u64,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            let entry = entry_mut(ctx, session)?;
            if let Some(composition) = entry.text.as_mut().filter(|c| c.token == token) {
                composition.editor.escape();
                composition.refresh(&entry.session);
            }
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
    }

    /// Backspace(`forward == false`)·Delete 한 번. 단위는 확장 grapheme cluster입니다.
    pub fn delete_canvas_text(
        &self,
        session: WindowSessionHandle,
        token: u64,
        forward: bool,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            let entry = entry_mut(ctx, session)?;
            let id = command_id(entry.session.window(), request);
            with_editor(entry, token, |editor, host| {
                if forward {
                    editor.delete_forward(host, id, &InputClock)
                } else {
                    editor.delete_backward(host, id, &InputClock)
                }
            })?;
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
    }

    /// caret·선택 이동(기록 없음).
    pub fn move_canvas_caret(
        &self,
        session: WindowSessionHandle,
        token: u64,
        motion: CaretMove,
        extend: bool,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            let entry = entry_mut(ctx, session)?;
            with_editor(entry, token, |editor, host| {
                let doc = host.document().ok_or(TextEditError::Stale)?;
                editor.move_caret(doc, motion, extend)
            })?;
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
    }

    /// 편집 중 문자 style(`DocumentTextStyles::runs` 번호, `None`은 기본 style) 변경(ADR-0118 결정 3).
    pub fn set_canvas_text_style(
        &self,
        session: WindowSessionHandle,
        token: u64,
        style: Option<u32>,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            let entry = entry_mut(ctx, session)?;
            let id = command_id(entry.session.window(), request);
            with_editor(entry, token, |editor, host| {
                editor.apply_style(host, style, id)
            })?;
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
    }

    /// 편집을 끝냅니다(Escape 뒤, 바깥 클릭, 도구 전환). 남은 조합은 확정하고, 이 편집에서 만든 Text가 비었으면
    /// 지우며, 그 밖에는 Text 하나를 선택합니다. 다른 token이면 아무것도 바꾸지 않습니다.
    pub fn end_canvas_text(
        &self,
        session: WindowSessionHandle,
        token: u64,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            let entry = entry_mut(ctx, session)?;
            if entry.text.as_ref().is_some_and(|c| c.token == token) {
                finish(entry, request)?;
            }
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
    }

    /// 조합 없는 입력(타이핑·삭제·붙여넣기) 한 번: Text의 `range`(그 revision의 본문 기준)를 `text`로 바꾸는 확정
    /// 입력으로 기록합니다. 그 Text의 편집이 없으면 편집을 시작해 이어 둡니다(token은 이 요청 ID).
    pub fn replace_canvas_text(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
        node: StableId,
        range: TextRange,
        text: String,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            let entry = entry_mut(ctx, session)?;
            begin(
                entry,
                request,
                view,
                revision,
                TextTarget::Node(node),
                range,
                None,
            )?;
            let id = command_id(entry.session.window(), request);
            with_editor(entry, request.0, |editor, host| {
                editor.insert(host, &text, id, &InputClock)
            })?;
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
    }

    /// 편집 가능한 Text의 저장된 본문. 편집 상태를 만들거나 바꾸지 않습니다.
    pub fn canvas_text_content(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
        node: StableId,
    ) -> Result<JobTicket<Option<String>>, SubmitError> {
        self.sender.submit(move |ctx| {
            let window = &ctx.app_sessions().windows.get(&session)?.session;
            if window.revision() != revision {
                return None;
            }
            editable_text(window, view, node)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_window_tests::{next_solicited, open_project, open_window};
    use crate::window_session::test_fixtures::{sample_document, saved_project_with, sid, TempDir};
    use stellar_core_types::text::UserText;
    use stellar_document::text::{Paragraph, StyleRun, TextPayload};
    use stellar_document::text_styles::DocumentTextStyles;

    fn range(start: u32, end: u32) -> TextRange {
        TextRange::new(start, end).unwrap()
    }

    const VIEW: u64 = 3;

    /// View root 10 아래 Frame 11을 Text "🎨old\n끝"로 바꾼 프로젝트. `locked`면 root 10을 잠급니다.
    fn project(tag: &str, locked: bool) -> TempDir {
        let mut model = sample_document();
        model.replace_text_styles(DocumentTextStyles::initial(sid(900)));
        let mut record = model.remove_leaf(sid(11)).unwrap();
        record.payload = NodePayload::Text(
            TextPayload::new(
                UserText::new("🎨old\n끝"),
                vec![Paragraph { style: 0 }, Paragraph { style: 0 }],
                vec![StyleRun {
                    range: range(1, 4),
                    style: 0,
                }],
            )
            .unwrap(),
        );
        model.insert_node(record).unwrap();
        model.replace_flags(sid(10), true, locked).unwrap();
        saved_project_with(tag, &[&model])
    }

    struct Rig {
        _dir: TempDir,
        app: AppRuntime,
        window: WindowSessionHandle,
    }

    impl Rig {
        fn new(tag: &str, locked: bool) -> Rig {
            let dir = project(tag, locked);
            let app = AppRuntime::start().unwrap();
            let project = open_project(&app, dir.path());
            let (window, _, _) = open_window(&app, project);
            Rig {
                _dir: dir,
                app,
                window,
            }
        }

        fn summary(&self) -> crate::app::WindowSessionSummary {
            self.app.state().window(self.window).unwrap().clone()
        }

        fn revision(&self) -> DocumentRevision {
            self.summary().revision
        }

        /// 요청 완료를 기다리고 성공이면 `true`.
        fn done(&self, request: Result<RequestId, RequestError>) -> bool {
            let request = request.unwrap();
            let event = next_solicited(&self.app);
            assert_eq!(event.request(), request, "{event:?}");
            !matches!(event, RuntimeEvent::RequestFailed { .. })
        }

        fn begin(&self, target: TextTarget, replacement: TextRange) -> Option<(u64, StableId)> {
            let (request, node) = self
                .app
                .begin_canvas_text(self.window, sid(VIEW), self.revision(), target, replacement)
                .unwrap();
            self.done(Ok(request)).then_some((request.0, node))
        }

        fn content(&self, node: StableId) -> Option<String> {
            self.app
                .canvas_text_content(self.window, sid(VIEW), self.revision(), node)
                .unwrap()
                .wait()
                .unwrap()
        }

        fn contains(&self, node: StableId) -> bool {
            let window = self.window;
            self.app
                .sender
                .submit(move |ctx| {
                    ctx.app_sessions()
                        .windows
                        .get(&window)
                        .and_then(|e| e.session.document())
                        .is_some_and(|d| d.model().contains(node))
                })
                .unwrap()
                .wait()
                .unwrap()
        }
    }

    #[test]
    fn hangul_composition_previews_without_history_and_consecutive_commits_undo_once() {
        let rig = Rig::new("text-input-hangul", false);
        let depth = rig.summary().undo_depth;
        let (token, node) = rig.begin(TextTarget::Node(sid(11)), range(1, 4)).unwrap();
        assert_eq!(node, sid(11));
        for step in ["ㅎ", "하", "한"] {
            assert!(rig.done(rig.app.mark_canvas_text(rig.window, token, step.into())));
            assert!(!rig.summary().has_unsaved_changes);
            assert_eq!(rig.summary().undo_depth, depth);
        }
        assert!(rig.done(rig.app.commit_canvas_text(rig.window, token, "한".into())));
        // 선택("old")을 바꾼 확정은 교체 입력이라 따로 묶입니다. 이어지는 조합 확정(caret 2 → 3)과 직접 입력(caret
        // 3)은 같은 삽입 입력이라 한 Undo 항목에 합쳐집니다(입력 종류별 grouping key, XII.10.32).
        assert!(rig.done(rig.app.mark_canvas_text(rig.window, token, "그".into())));
        assert!(rig.done(rig.app.commit_canvas_text(rig.window, token, "글".into())));
        let revision = rig.revision();
        assert!(rig.done(rig.app.replace_canvas_text(
            rig.window,
            sid(VIEW),
            revision,
            sid(11),
            range(3, 3),
            "!".into(),
        )));
        assert_eq!(rig.content(sid(11)).unwrap(), "🎨한글!\n끝");
        let summary = rig.summary();
        assert_eq!(summary.undo_depth, depth + 2);
        assert_eq!(summary.undo_label, history_label::TYPING);
        assert!(rig.done(rig.app.undo(rig.window)));
        assert_eq!(rig.content(sid(11)).unwrap(), "🎨한\n끝");
        assert!(rig.done(rig.app.undo(rig.window)));
        assert_eq!(rig.content(sid(11)).unwrap(), "🎨old\n끝");
    }

    #[test]
    fn unchanged_stale_and_locked_edits_leave_history_alone() {
        let rig = Rig::new("text-input-stale", false);
        let depth = rig.summary().undo_depth;
        let (token, _) = rig.begin(TextTarget::Node(sid(11)), range(1, 4)).unwrap();
        // 같은 값 확정은 성공이지만 history를 늘리지 않습니다.
        assert!(rig.done(rig.app.commit_canvas_text(rig.window, token, "old".into())));
        assert_eq!(rig.summary().undo_depth, depth);
        // 범위 밖, 다른 View, 낡은 revision, Text가 아닌 node는 거부합니다.
        assert!(rig
            .begin(TextTarget::Node(sid(11)), range(100, 100))
            .is_none());
        assert!(rig.begin(TextTarget::Node(sid(12)), range(0, 0)).is_none());
        let (request, _) = rig
            .app
            .begin_canvas_text(
                rig.window,
                sid(4),
                rig.revision(),
                TextTarget::Node(sid(11)),
                range(0, 0),
            )
            .unwrap();
        assert!(!rig.done(Ok(request)));
        let stale = DocumentRevision(rig.revision().0 + 7);
        let (request, _) = rig
            .app
            .begin_canvas_text(
                rig.window,
                sid(VIEW),
                stale,
                TextTarget::Node(sid(11)),
                range(0, 0),
            )
            .unwrap();
        assert!(!rig.done(Ok(request)));
        // 잠긴 View root 아래 Text는 편집하지 못하고 본문 조회도 없음입니다.
        let locked = Rig::new("text-input-locked", true);
        assert!(locked
            .begin(TextTarget::Node(sid(11)), range(0, 0))
            .is_none());
        assert_eq!(locked.content(sid(11)), None);
    }

    #[test]
    fn new_text_is_created_on_first_commit_and_undone_with_following_input() {
        let rig = Rig::new("text-input-new", false);
        let depth = rig.summary().undo_depth;
        let origin = Vec2::new(-3.5, 42.).unwrap();
        let new = TextTarget::New {
            parent: sid(10),
            origin,
        };
        // 입력 없이 끝나면 node와 history가 생기지 않습니다.
        let (empty, node) = rig.begin(new, range(0, 0)).unwrap();
        assert!(rig.done(rig.app.end_canvas_text(rig.window, empty)));
        assert!(!rig.contains(node));
        assert_eq!(rig.summary().undo_depth, depth);
        let (token, node) = rig.begin(new, range(0, 0)).unwrap();
        assert!(rig.done(rig.app.commit_canvas_text(rig.window, token, "안".into())));
        assert_eq!(rig.summary().primary_selection, Some(node));
        assert!(rig.done(rig.app.commit_canvas_text(rig.window, token, "녕".into())));
        assert_eq!(rig.content(node).unwrap(), "안녕");
        assert!(rig.done(rig.app.end_canvas_text(rig.window, token)));
        let summary = rig.summary();
        assert_eq!(summary.undo_depth, depth + 1);
        assert_eq!(summary.primary_selection, Some(node));
        assert!(rig.done(rig.app.undo(rig.window)));
        assert!(!rig.contains(node));
    }

    /// 이 편집에서 만든 Text를 입력한 뒤 모두 지우고 끝내면 빈 Text를 지우는 항목 하나가 남습니다(ADR-0129 결정 1).
    #[test]
    fn emptied_new_text_is_removed_when_editing_ends() {
        let rig = Rig::new("text-input-emptied", false);
        let depth = rig.summary().undo_depth;
        let placement = TextPlacement {
            parent: sid(10),
            origin: Vec2::new(5., 6.).unwrap(),
            width: TextWidth::Fixed(80.),
        };
        let (token, node) = rig
            .begin(TextTarget::Placement(placement), range(0, 0))
            .unwrap();
        assert!(rig.done(rig.app.commit_canvas_text(rig.window, token, "ab".into())));
        for _ in 0..2 {
            assert!(rig.done(rig.app.delete_canvas_text(rig.window, token, false)));
        }
        assert_eq!(rig.content(node).as_deref(), Some(""));
        assert!(rig.done(rig.app.end_canvas_text(rig.window, token)));
        assert!(!rig.contains(node));
        let summary = rig.summary();
        // 생성·입력·지우기 항목 + 빈 Text 삭제 항목.
        assert_eq!(summary.undo_label, history_label::DELETE);
        assert!(summary.undo_depth > depth);
        assert_eq!(summary.selection_len, 0);
        // Undo 한 번은 빈 Text를 되살리고 선택합니다.
        assert!(rig.done(rig.app.undo(rig.window)));
        assert!(rig.contains(node));
        assert_eq!(rig.summary().primary_selection, Some(node));
        // 기존 Text는 비워도 남습니다.
        let (token, _) = rig.begin(TextTarget::Node(sid(11)), range(0, 6)).unwrap();
        assert!(rig.done(rig.app.commit_canvas_text(rig.window, token, String::new())));
        assert!(rig.done(rig.app.end_canvas_text(rig.window, token)));
        assert!(rig.contains(sid(11)));
        assert_eq!(rig.content(sid(11)).as_deref(), Some(""));
    }

    #[test]
    fn caret_moves_style_and_cancel_go_through_the_core_editor() {
        let rig = Rig::new("text-input-keys", false);
        let (token, _) = rig.begin(TextTarget::Node(sid(11)), range(6, 6)).unwrap();
        assert!(rig.done(
            rig.app
                .move_canvas_caret(rig.window, token, CaretMove::Backward, true)
        ));
        // 선택한 "끝"을 지웁니다(grapheme 단위).
        assert!(rig.done(rig.app.delete_canvas_text(rig.window, token, false)));
        assert_eq!(rig.content(sid(11)).as_deref(), Some("🎨old\n"));
        // 조합 취소는 문서를 바꾸지 않고 편집을 이어 둡니다.
        let revision = rig.revision();
        assert!(rig.done(rig.app.mark_canvas_text(rig.window, token, "ㄱ".into())));
        assert!(rig.done(rig.app.cancel_canvas_text(rig.window, token)));
        assert_eq!(rig.revision(), revision);
        assert!(rig.done(rig.app.commit_canvas_text(rig.window, token, "x".into())));
        assert_eq!(rig.content(sid(11)).as_deref(), Some("🎨old\nx"));
        // 표에 없는 style 번호는 거부합니다.
        assert!(rig.done(rig.app.move_canvas_caret(
            rig.window,
            token,
            CaretMove::SelectAll,
            false
        )));
        assert!(!rig.done(rig.app.set_canvas_text_style(rig.window, token, Some(9))));
        assert!(rig.done(rig.app.set_canvas_text_style(rig.window, token, None)));
        assert_eq!(rig.summary().undo_label, history_label::CHANGE_PROPERTY);
        // 끝난 token은 더 쓸 수 없습니다.
        assert!(rig.done(rig.app.end_canvas_text(rig.window, token)));
        assert!(!rig.done(rig.app.commit_canvas_text(rig.window, token, "y".into())));
    }

    #[test]
    fn new_text_requires_a_resolvable_default_style() {
        let mut model = sample_document();
        model.replace_text_styles(DocumentTextStyles::default());
        let dir = saved_project_with("text-input-no-style", &[&model]);
        let app = AppRuntime::start().unwrap();
        let project = open_project(&app, dir.path());
        let (window, _, _) = open_window(&app, project);
        let revision = app.state().window(window).unwrap().revision;
        let (request, _) = app
            .begin_canvas_text(
                window,
                sid(VIEW),
                revision,
                TextTarget::New {
                    parent: sid(10),
                    origin: Vec2::ZERO,
                },
                range(0, 0),
            )
            .unwrap();
        let event = next_solicited(&app);
        assert_eq!(event.request(), request);
        assert!(matches!(event, RuntimeEvent::RequestFailed { .. }));
    }
}
