//! 캔버스 Text 편집(ADR-0067 §6).
//!
//! - IME 조합은 창 session의 조합 하나([`TextComposition`])가 소유합니다. 조합 중 본문은 문서와 history에 넣지
//!   않고, 같은 창의 canvas scene이 미리보기로 그립니다(글꼴 cache는 scene의 것 하나).
//! - 확정·직접 입력만 Text 범위 편집으로 기록합니다. 같은 node에서 caret이 이어지는 입력은
//!   `GroupingKey::TextInput`으로 묶여 Undo 한 번이 됩니다. 새 Text는 첫 확정 때 만들고 이어지는 입력과 같은
//!   group으로 기록합니다.
//! - 편집 가능 여부는 문서 값(표시·잠금·View 소속)으로만 정합니다. 프로젝트 신뢰는 보지 않습니다(D1).
use super::frame_move::{command_id, editable_frame};
use super::*;
use crate::canvas::text::TextPreview;
use crate::document_thread::JobTicket;
use stellar_commands::{
    command::Command, id::CommandOrigin, node_ops::InsertNode, text_edit::ReplaceText,
    transaction::CommandTransaction,
};
use stellar_core_types::{
    clock::MonotonicClock,
    order_key::OrderKey,
    value::{Transform2D, Vec2},
};
use stellar_document::{
    modifier::{ModifierEntry, ModifierId, ModifierKind, ModifierPayload, ModifierStack},
    records::{NodePayload, NodeRecord},
    text::{edit_text, TextPayload},
};
use stellar_history::entry::{GroupingKey, TextInputKind};

pub use stellar_document::text::TextRange;

/// 조합을 시작할 대상.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TextTarget {
    /// 선택한 기존 Text.
    Node(StableId),
    /// 부모 Frame local 좌표의 원점에 만들 새 Text.
    New {
        /// 자유 배치 부모 Frame.
        parent: StableId,
        /// 부모 local 원점.
        origin: Vec2,
    },
}

/// 첫 확정 때 만들 새 Text의 위치와 미리 정한 영속 ID(UUIDv7).
#[derive(Debug, Clone)]
struct NewText {
    parent: StableId,
    stack: ModifierStack,
}

/// 창 하나의 진행 중 IME 조합. document thread만 소유합니다.
#[derive(Debug)]
pub(crate) struct TextComposition {
    token: u64,
    view: StableId,
    revision: DocumentRevision,
    replacement: TextRange,
    original: TextPayload,
    new_text: Option<NewText>,
    preview: TextPreview,
}

impl TextComposition {
    /// 시작한 revision이 그대로일 때만 scene에 미리보기를 그립니다.
    pub(crate) fn preview_at(&self, revision: DocumentRevision) -> Option<&TextPreview> {
        (self.revision == revision).then_some(&self.preview)
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

fn stale() -> Failure {
    Failure::new(
        RequestFailure::UnknownView,
        "Text 편집의 문서·선택·조합 상태가 바뀌었습니다",
    )
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

impl AppRuntime {
    /// IME 조합을 시작합니다. 반환한 요청 ID가 이후 조합 요청의 token이고, StableId는 조합 대상 node입니다
    /// (새 Text는 확정 때 이 ID로 만듭니다). range는 문서 전체 Unicode scalar 범위입니다.
    pub fn begin_canvas_text(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
        target: TextTarget,
        replacement: TextRange,
    ) -> Result<(RequestId, StableId), RequestError> {
        let (node, new_text) = match target {
            TextTarget::Node(node) => (node, None),
            TextTarget::New { parent, origin } => {
                let [node, modifier] =
                    stellar_project::new_ids::new_ids().map_err(|_| RequestError::Stopped)?;
                (node, Some((parent, origin, modifier)))
            }
        };
        self.submit(move |ctx, request| {
            let entry = entry_mut(ctx, session)?;
            let composition = begin(
                &entry.session,
                request.0,
                view,
                revision,
                node,
                new_text,
                replacement,
            )?;
            entry.text = Some(composition);
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
        .map(|request| (request, node))
    }

    /// 조합 중 본문을 바꿉니다. 문서와 history는 바뀌지 않고 scene 미리보기만 바뀝니다.
    pub fn mark_canvas_text(
        &self,
        session: WindowSessionHandle,
        token: u64,
        text: String,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            let entry = entry_mut(ctx, session)?;
            let revision = entry.session.revision();
            let composition = entry
                .text
                .as_mut()
                .filter(|c| c.token == token && c.revision == revision)
                .ok_or_else(stale)?;
            let payload = if text.is_empty() {
                composition.original.clone()
            } else {
                edit_text(&composition.original, composition.replacement, &text, &[])
                    .map_err(|_| stale())?
            };
            composition.preview.payload = payload;
            composition.preview.key.1 += 1;
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
    }

    /// 입력기가 확정한 본문을 조합 범위에 기록하고 조합을 끝냅니다. 본문이 같으면 history를 늘리지 않으며,
    /// 그때도 같은 완료 event를 보냅니다(셸은 revision이 그대로인지로 구분합니다).
    pub fn commit_canvas_text(
        &self,
        session: WindowSessionHandle,
        token: u64,
        text: String,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            let entry = entry_mut(ctx, session)?;
            let composition = entry.text.take_if(|c| c.token == token).ok_or_else(stale)?;
            commit(&mut entry.session, composition, text, request)?;
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
    }

    /// 지정한 조합만 버립니다. 이미 끝났거나 더 새 조합이 있으면 아무것도 바꾸지 않습니다.
    pub fn cancel_canvas_text(
        &self,
        session: WindowSessionHandle,
        token: u64,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            entry_mut(ctx, session)?.text.take_if(|c| c.token == token);
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
    }

    /// 조합 없는 입력(타이핑·삭제·붙여넣기) 한 번을 선택한 Text의 범위 편집으로 기록합니다.
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
            let window = &mut entry_mut(ctx, session)?.session;
            replace(window, view, revision, node, range, text, request)?;
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
    }

    /// 편집 가능한 Text의 저장된 본문. 조합 상태를 만들거나 바꾸지 않습니다.
    pub fn canvas_text_content(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
        node: StableId,
    ) -> Result<JobTicket<Option<String>>, SubmitError> {
        self.sender.submit(move |ctx| {
            let window = &ctx.app_sessions().windows.get(&session)?.session;
            editable_text(window, view, revision, node, false).map(|t| t.content().to_owned())
        })
    }
}

/// 같은 View·revision에서 표시·잠금 해제된 조상만 가진 Text. `selected`면 그 Text 하나만 선택돼 있어야 합니다.
fn editable_text(
    window: &ProjectWindowSession,
    view: StableId,
    revision: DocumentRevision,
    node: StableId,
    selected: bool,
) -> Option<&TextPayload> {
    if window.revision() != revision
        || window.tabs().active_view()? != view
        || (selected
            && (window.view().selection().ids() != [node] || window.view().area_selection_active()))
    {
        return None;
    }
    let model = window.document()?.model();
    let NodePayload::Text(payload) = model.payload(node)? else {
        return None;
    };
    let root = model
        .definition()
        .pages
        .iter()
        .flat_map(|p| &p.views)
        .find(|v| v.id == view)?
        .root_frame;
    let mut current = model.runtime_id(node)?;
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

/// 연속 입력 group 판단 key. 삽입은 시작에서 입력 끝으로, 삭제는 범위 끝에서 시작으로 caret이 움직입니다.
fn input_key(node: StableId, range: TextRange, text: &str) -> GroupingKey {
    if text.is_empty() {
        return GroupingKey::TextInput {
            node,
            kind: TextInputKind::DeleteBackward,
            caret_before: range.end.0,
            caret_after: range.start.0,
        };
    }
    let len = u32::try_from(text.chars().count()).unwrap_or(u32::MAX);
    GroupingKey::TextInput {
        node,
        kind: TextInputKind::Insert,
        caret_before: range.start.0,
        caret_after: range.start.0.saturating_add(len),
    }
}

fn new_text_stack(node: StableId, modifier: StableId, origin: Vec2) -> Option<ModifierStack> {
    let transform = Transform2D::new(1., 0., 0., 1., origin.x(), origin.y()).ok()?;
    let mut stack = ModifierStack::new(node);
    stack
        .insert(ModifierEntry {
            id: ModifierId::new(modifier),
            order: OrderKey::between(None, None),
            kind: ModifierKind::Transform,
            enabled: true,
            payload: ModifierPayload::Transform(transform),
        })
        .ok()?;
    Some(stack)
}

fn begin(
    window: &ProjectWindowSession,
    token: u64,
    view: StableId,
    revision: DocumentRevision,
    node: StableId,
    new_text: Option<(StableId, Vec2, StableId)>,
    replacement: TextRange,
) -> Result<TextComposition, Failure> {
    let (original, new_text) = match new_text {
        None => {
            let payload = editable_text(window, view, revision, node, true).ok_or_else(stale)?;
            payload.slice(replacement).ok_or_else(stale)?;
            (payload.clone(), None)
        }
        Some((parent, origin, modifier)) => {
            if window.revision() != revision
                || window.tabs().active_view() != Some(view)
                || replacement != TextRange::new(0, 0).expect("empty range")
            {
                return Err(stale());
            }
            let model = window.document().ok_or_else(stale)?.model();
            editable_frame(model, view, parent).ok_or_else(stale)?;
            // 해석할 수 없는 기본 style로 Text를 만들면 보이지 않는 node가 생깁니다(U122).
            let styles = &model.definition().text_styles;
            if styles
                .resolve(0, None)
                .is_none_or(|s| s.typography.record().is_err())
            {
                return Err(Failure::new(
                    RequestFailure::UnknownView,
                    "문서에 새 Text가 쓸 기본 Text style이 없습니다",
                ));
            }
            let stack = new_text_stack(node, modifier, origin).ok_or_else(stale)?;
            (TextPayload::empty(0), Some(NewText { parent, stack }))
        }
    };
    Ok(TextComposition {
        token,
        view,
        revision,
        replacement,
        preview: TextPreview {
            key: (token, 0),
            node,
            payload: original.clone(),
            new_in: new_text.as_ref().map(|n| (n.parent, n.stack.clone())),
        },
        original,
        new_text,
    })
}

fn commit(
    window: &mut ProjectWindowSession,
    composition: TextComposition,
    text: String,
    request: RequestId,
) -> Result<(), Failure> {
    let node = composition.preview.node;
    let Some(new_text) = composition.new_text else {
        return replace(
            window,
            composition.view,
            composition.revision,
            node,
            composition.replacement,
            text,
            request,
        );
    };
    if window.revision() != composition.revision
        || window.tabs().active_view() != Some(composition.view)
    {
        return Err(stale());
    }
    if text.is_empty() {
        // 입력 없이 끝난 새 Text는 만들지 않습니다(history 없음).
        return Ok(());
    }
    let model = window.document().ok_or_else(stale)?.model();
    editable_frame(model, composition.view, new_text.parent).ok_or_else(stale)?;
    let payload = edit_text(&composition.original, composition.replacement, &text, &[])
        .map_err(|_| stale())?;
    let mut insert = InsertNode::single(NodeRecord {
        id: node,
        parent: Some(new_text.parent),
        order_key: model.next_child_order_key(new_text.parent),
        visible: true,
        locked: false,
        display_name: "Text".into(),
        payload: NodePayload::Text(payload),
    });
    insert.modifier_stacks.push(new_text.stack);
    let tx = CommandTransaction::new(command_id(window.window(), request), CommandOrigin::User)
        .with_command(composition.revision, Command::InsertNode(insert));
    let key = input_key(node, composition.replacement, &text);
    window
        .insert_grouped_and_select(
            &tx,
            key,
            node,
            &InputClock,
            crate::window_session::history_label::TYPING,
        )
        .map_err(view_failure)?;
    Ok(())
}

fn replace(
    window: &mut ProjectWindowSession,
    view: StableId,
    revision: DocumentRevision,
    node: StableId,
    range: TextRange,
    text: String,
    request: RequestId,
) -> Result<(), Failure> {
    let original = editable_text(window, view, revision, node, true)
        .and_then(|payload| payload.slice(range))
        .ok_or_else(stale)?;
    if original == text {
        return Ok(());
    }
    let key = input_key(node, range, &text);
    let tx = CommandTransaction::new(command_id(window.window(), request), CommandOrigin::User)
        .with_command(
            revision,
            Command::ReplaceText(ReplaceText {
                node,
                range,
                replacement: text,
            }),
        );
    window
        .apply_grouped(
            &tx,
            key,
            &InputClock,
            crate::window_session::history_label::TYPING,
        )
        .map_err(view_failure)?;
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
    use stellar_document::text_styles::DocumentTextStyles;

    fn range(start: u32, end: u32) -> TextRange {
        TextRange::new(start, end).unwrap()
    }

    fn ok<T>(result: Result<T, Failure>) -> T {
        result.map_err(|e| e.message).unwrap()
    }

    const VIEW: u64 = 3;

    fn fixture(locked: bool) -> (TempDir, ProjectWindowSession) {
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
        let dir = saved_project_with("text-input", &[&model]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        session.select(Selection::new([sid(11)], None)).unwrap();
        (dir, session)
    }

    fn content(session: &ProjectWindowSession, node: StableId) -> Option<String> {
        editable_text(session, sid(VIEW), session.revision(), node, false)
            .map(|t| t.content().to_owned())
    }

    #[test]
    fn hangul_composition_previews_without_history_and_consecutive_commits_undo_once() {
        let (dir, mut session) = fixture(false);
        let history_before = session.history().history().len();
        let revision = session.revision();
        let mut composition = ok(begin(
            &session,
            7,
            sid(VIEW),
            revision,
            sid(11),
            None,
            range(1, 4),
        ));
        for step in ["ㅎ", "하", "한"] {
            composition.preview.payload =
                edit_text(&composition.original, composition.replacement, step, &[]).unwrap();
            assert_eq!(
                composition
                    .preview_at(session.revision())
                    .unwrap()
                    .payload
                    .content(),
                format!("🎨{step}\n끝")
            );
            assert_eq!(session.history().history().len(), history_before);
            assert!(!session.has_unsaved_changes());
        }
        ok(commit(&mut session, composition, "한".into(), RequestId(1)));
        // 이어지는 확정(caret 2 → 3)과 직접 입력(caret 3)은 같은 Undo 항목에 합쳐집니다.
        let revision = session.revision();
        let composition = ok(begin(
            &session,
            8,
            sid(VIEW),
            revision,
            sid(11),
            None,
            range(2, 2),
        ));
        ok(commit(&mut session, composition, "글".into(), RequestId(2)));
        let revision = session.revision();
        ok(replace(
            &mut session,
            sid(VIEW),
            revision,
            sid(11),
            range(3, 3),
            "!".into(),
            RequestId(3),
        ));
        assert_eq!(content(&session, sid(11)).unwrap(), "🎨한글!\n끝");
        assert_eq!(session.history().history().len(), history_before + 1);
        assert!(session.undo().unwrap());
        assert_eq!(content(&session, sid(11)).unwrap(), "🎨old\n끝");
        assert!(session.redo().unwrap());
        session
            .save_with_lock_dir(|| Ok(dir.path().join("locks")))
            .unwrap();
        let reopened = ProjectWindowSession::load(identity(dir.path()), window(2)).unwrap();
        assert_eq!(content(&reopened, sid(11)).unwrap(), "🎨한글!\n끝");
    }

    #[test]
    fn unchanged_stale_and_locked_edits_leave_history_alone() {
        let (_dir, mut session) = fixture(false);
        let history = session.history().history().len();
        let revision = session.revision();
        // 같은 값 확정과 같은 값 대체는 성공이지만 history를 늘리지 않습니다.
        let same = ok(begin(
            &session,
            1,
            sid(VIEW),
            revision,
            sid(11),
            None,
            range(1, 4),
        ));
        ok(commit(&mut session, same, "old".into(), RequestId(1)));
        ok(replace(
            &mut session,
            sid(VIEW),
            revision,
            sid(11),
            range(1, 4),
            "old".into(),
            RequestId(2),
        ));
        assert_eq!(session.history().history().len(), history);
        assert!(begin(
            &session,
            2,
            sid(VIEW),
            revision,
            sid(11),
            None,
            range(100, 100)
        )
        .is_err());
        assert!(begin(&session, 3, sid(4), revision, sid(11), None, range(0, 0)).is_err());
        let composition = ok(begin(
            &session,
            4,
            sid(VIEW),
            revision,
            sid(11),
            None,
            range(1, 4),
        ));
        ok(replace(
            &mut session,
            sid(VIEW),
            revision,
            sid(11),
            range(1, 4),
            "新".into(),
            RequestId(3),
        ));
        // 시작 뒤 문서가 바뀌면 조합 미리보기와 확정이 모두 거부됩니다.
        assert!(composition.preview_at(session.revision()).is_none());
        assert!(commit(&mut session, composition, "古".into(), RequestId(4)).is_err());
        session.select(Selection::new([sid(12)], None)).unwrap();
        let revision = session.revision();
        assert!(replace(
            &mut session,
            sid(VIEW),
            revision,
            sid(11),
            range(0, 0),
            "x".into(),
            RequestId(5)
        )
        .is_err());
        let (_dir, session) = fixture(true);
        assert!(begin(
            &session,
            5,
            sid(VIEW),
            session.revision(),
            sid(11),
            None,
            range(0, 0)
        )
        .is_err());
    }

    #[test]
    fn new_text_is_created_on_first_commit_and_undone_with_following_input() {
        let (dir, mut session) = fixture(false);
        let history = session.history().history().len();
        let origin = Vec2::new(-3.5, 42.).unwrap();
        let (node, modifier) = (sid(950), sid(951));
        let start = |session: &ProjectWindowSession, token| {
            begin(
                session,
                token,
                sid(VIEW),
                session.revision(),
                node,
                Some((sid(10), origin, modifier)),
                range(0, 0),
            )
        };
        // 입력 없이 끝나면 node와 history가 생기지 않습니다.
        let empty = ok(start(&session, 1));
        assert!(empty.preview.new_in.is_some());
        ok(commit(&mut session, empty, String::new(), RequestId(1)));
        assert!(!session.document().unwrap().model().contains(node));
        assert_eq!(session.history().history().len(), history);
        let composition = ok(start(&session, 2));
        ok(commit(&mut session, composition, "안".into(), RequestId(2)));
        assert_eq!(session.view().selection().ids(), [node]);
        let revision = session.revision();
        ok(replace(
            &mut session,
            sid(VIEW),
            revision,
            node,
            range(1, 1),
            "녕".into(),
            RequestId(3),
        ));
        let model = session.document().unwrap().model();
        assert_eq!(content(&session, node).unwrap(), "안녕");
        let stack = model.modifier_stack(node).unwrap().entries();
        assert_eq!(stack.len(), 1);
        assert_eq!(stack[0].id, ModifierId::new(modifier));
        assert!(
            matches!(stack[0].payload, ModifierPayload::Transform(t) if t.components() == (1., 0., 0., 1., -3.5, 42.))
        );
        assert_eq!(session.history().history().len(), history + 1);
        assert!(session.undo().unwrap());
        assert!(!session.document().unwrap().model().contains(node));
        assert!(session.redo().unwrap());
        session
            .save_with_lock_dir(|| Ok(dir.path().join("locks")))
            .unwrap();
        let reopened = ProjectWindowSession::load(identity(dir.path()), window(2)).unwrap();
        let model = reopened.document().unwrap().model();
        let Some(NodePayload::Text(text)) = model.payload(node) else {
            panic!("Text node");
        };
        assert_eq!(text.content(), "안녕");
        assert_eq!(
            text.paragraphs()
                .iter()
                .map(|p| p.style)
                .collect::<Vec<_>>(),
            [0]
        );
        assert_eq!(
            model.modifier_stack(node).unwrap().entries()[0].id,
            ModifierId::new(modifier)
        );
    }

    #[test]
    fn new_text_requires_a_resolvable_default_style() {
        let mut model = sample_document();
        model.replace_text_styles(DocumentTextStyles::default());
        let dir = saved_project_with("text-input-no-style", &[&model]);
        let session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        assert!(begin(
            &session,
            1,
            sid(VIEW),
            session.revision(),
            sid(950),
            Some((sid(10), Vec2::ZERO, sid(951))),
            range(0, 0),
        )
        .is_err());
        drop(dir);
    }

    #[test]
    fn runtime_composition_does_not_depend_on_project_trust() {
        use crate::app_window_tests::{next_event, open_project, open_window};
        let (dir, _) = fixture(false);
        let app = AppRuntime::start().unwrap();
        let project = open_project(&app, dir.path());
        let (handle, _, _) = open_window(&app, project);
        app.set_selection(handle, vec![sid(11)]).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::WindowViewChanged { .. }
        ));
        let revision = app.state().window(handle).unwrap().revision;
        let (token, node) = app
            .begin_canvas_text(
                handle,
                sid(VIEW),
                revision,
                TextTarget::Node(sid(11)),
                range(1, 4),
            )
            .unwrap();
        assert_eq!(node, sid(11));
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::WindowViewChanged { .. }
        ));
        app.mark_canvas_text(handle, token.0, "ㅎ".into()).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::WindowViewChanged { .. }
        ));
        assert!(!app.state().window(handle).unwrap().has_unsaved_changes);
        app.commit_canvas_text(handle, token.0, "한".into())
            .unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::WindowViewChanged { .. }
        ));
        let state = app.state();
        let window = state.window(handle).unwrap();
        assert!(window.has_unsaved_changes);
        assert_eq!(window.revision.0, revision.0 + 1);
        // 끝난 token은 다시 확정할 수 없습니다.
        app.commit_canvas_text(handle, token.0, "두 번".into())
            .unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::RequestFailed { .. }
        ));
        let content = app
            .canvas_text_content(handle, sid(VIEW), window.revision, sid(11))
            .unwrap()
            .wait()
            .unwrap();
        assert_eq!(content.as_deref(), Some("🎨한\n끝"));
    }
}
