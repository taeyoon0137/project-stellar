//! 활성 창에 표시된 Free Frame의 위치·크기를 하나의 문서 transaction으로 편집합니다.

use super::*;
use stellar_commands::command::Command;
use stellar_commands::document::{
    frame::{keys, Clipping},
    records::NodePayload,
};
use stellar_commands::id::{CommandId, CommandOrigin};
use stellar_commands::modifier_ops::{InsertModifier, SetModifierProperty};
use stellar_commands::property::{PropertyValue, SetProperty};
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::color::Color;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::value::{Size2, Transform2D, Vec2};
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{ModifierEntry, ModifierId, ModifierKind, ModifierPayload};
use stellar_document::modifier_metadata::keys::TRANSFORM_MATRIX;
use stellar_document::shape::ShapeGeometry;

#[derive(Clone, Copy)]
enum FrameEdit {
    Move(Vec2),
    Resize(Size2),
    Background(Option<Color>),
}

impl AppRuntime {
    /// 표시한 View와 revision에서 선택한 Frame을 부모 좌표계의 delta만큼 이동합니다.
    /// 신뢰, 활성 View, 선택 및 조상 잠금은 document thread에서 다시 확인합니다.
    pub fn move_canvas_frame(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
        node: StableId,
        delta: Vec2,
    ) -> Result<RequestId, RequestError> {
        self.edit_canvas_frame(session, view, revision, node, FrameEdit::Move(delta))
    }

    /// 선택된 Free Frame의 고정 크기를 하나의 Undo 가능한 문서 편집으로 바꿉니다.
    pub fn resize_canvas_frame(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
        node: StableId,
        size: Size2,
    ) -> Result<RequestId, RequestError> {
        self.edit_canvas_frame(session, view, revision, node, FrameEdit::Resize(size))
    }

    /// 선택한 Free Frame의 단색 배경을 설정하거나 제거합니다.
    pub fn set_canvas_frame_background(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
        node: StableId,
        color: Option<Color>,
    ) -> Result<RequestId, RequestError> {
        self.edit_canvas_frame(session, view, revision, node, FrameEdit::Background(color))
    }

    fn edit_canvas_frame(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
        node: StableId,
        edit: FrameEdit,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            let window = trusted_canvas_window(ctx, session)?;
            edit_selected(window, request, view, revision, node, edit)?;
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
    }
}

pub(super) fn trusted_canvas_window(
    ctx: &mut DocumentContext,
    session: WindowSessionHandle,
) -> Result<&mut ProjectWindowSession, Failure> {
    let sessions = ctx.app_sessions();
    let entry = sessions
        .windows
        .get(&session)
        .ok_or_else(|| unknown_window(session))?;
    let trusted = sessions
        .projects
        .get(&entry.project)
        .is_some_and(|p| p.trust == TrustStatus::Trusted);
    if !trusted {
        return Err(Failure::new(
            RequestFailure::TrustUnavailable,
            "Frame 편집에 프로젝트 신뢰가 필요합니다",
        ));
    }
    window_mut(ctx, session)
}

fn edit_selected(
    window: &mut ProjectWindowSession,
    request: RequestId,
    view: StableId,
    revision: DocumentRevision,
    node: StableId,
    edit: FrameEdit,
) -> Result<(), Failure> {
    let stale = || {
        Failure::new(
            RequestFailure::UnknownView,
            "Frame의 선택·뷰·편집 상태가 변경되었습니다",
        )
    };
    if window.revision() != revision
        || window.tabs().active_view() != Some(view)
        || window.view().selection().ids() != [node]
    {
        return Err(stale());
    }
    let model = window.document().ok_or_else(stale)?.model();
    let command_id = command_id(window.window(), request);
    let command =
        if let (FrameEdit::Move(delta), Some(NodePayload::Shape(ShapeGeometry::Rectangle(_)))) =
            (edit, model.payload(node))
        {
            if delta == Vec2::ZERO {
                return Ok(());
            }
            editable_ancestry(model, view, node, true).ok_or_else(stale)?;
            let stack = model.modifier_stack(node);
            let style = crate::canvas::shape::rectangle_style(stack).ok_or_else(stale)?;
            if let Some((modifier, last)) = style.last_transform {
                let next = translated(last, delta).ok_or_else(stale)?;
                if next == last {
                    return Ok(());
                }
                Command::SetModifierProperty(SetModifierProperty {
                    node,
                    modifier,
                    key: TRANSFORM_MATRIX,
                    value: PropertyValue::Transform(next),
                })
            } else {
                let transform = translated(Transform2D::identity(), delta).ok_or_else(stale)?;
                let order = OrderKey::between(
                    None,
                    stack.and_then(|s| s.entries().first()).map(|e| &e.order),
                );
                Command::InsertModifier(InsertModifier {
                    node,
                    entry: ModifierEntry {
                        id: ModifierId::new(StableId::from_bytes(command_id.to_bytes())),
                        order,
                        kind: ModifierKind::Transform,
                        enabled: true,
                        payload: ModifierPayload::Transform(transform),
                    },
                })
            }
        } else {
            let target = editable_frame(model, view, node).ok_or_else(stale)?;
            let (key, value) = match edit {
                FrameEdit::Move(delta) => {
                    let next = translated(target.transform, delta).ok_or_else(stale)?;
                    if next == target.transform {
                        return Ok(());
                    }
                    (keys::TRANSFORM, PropertyValue::Transform(next))
                }
                FrameEdit::Resize(size) => {
                    if size == target.size {
                        return Ok(());
                    }
                    (keys::SIZE, PropertyValue::Size(size))
                }
                FrameEdit::Background(color) => {
                    if target.background == color {
                        return Ok(());
                    }
                    (keys::BACKGROUND, PropertyValue::OptionalColor(color))
                }
            };
            Command::SetProperty(SetProperty { node, key, value })
        };
    let transaction =
        CommandTransaction::new(command_id, CommandOrigin::User).with_command(revision, command);
    window.apply(&transaction).map_err(view_failure)?;
    Ok(())
}

pub(super) fn editable_frame(
    model: &stellar_document::model::DocumentModel,
    view: StableId,
    node: StableId,
) -> Option<&stellar_document::frame::FramePayload> {
    let Some(NodePayload::Frame(target)) = model.payload(node) else {
        return None;
    };
    editable_ancestry(model, view, node, false)?;
    Some(target)
}

fn translated(current: Transform2D, delta: Vec2) -> Option<Transform2D> {
    let (a, b, c, d, x, y) = current.components();
    Transform2D::new(a, b, c, d, x + delta.x(), y + delta.y()).ok()
}

fn editable_ancestry(
    model: &DocumentModel,
    view: StableId,
    node: StableId,
    rectangle: bool,
) -> Option<()> {
    let root = model
        .definition()
        .pages
        .iter()
        .flat_map(|page| &page.views)
        .find(|v| v.id == view)?
        .root_frame;
    let mut current = node;
    loop {
        let fields = model
            .runtime_id(current)
            .and_then(|id| model.columns().get(id))?;
        if !fields.visible
            || fields.locked
            || (!(rectangle && current == node) && model.modifier_stack(current).is_some())
        {
            return None;
        }
        match model.payload(current) {
            Some(NodePayload::Frame(frame))
                if frame.layout_mode.is_none() && !matches!(frame.clipping, Clipping::Path(_)) => {}
            Some(NodePayload::Fragment(_)) => {}
            Some(NodePayload::Shape(ShapeGeometry::Rectangle(_)))
                if rectangle && current == node => {}
            _ => return None,
        }
        if current == root {
            break;
        }
        current = fields.parent.and_then(|id| model.stable_id(id))?;
    }
    Some(())
}

/// 요청 순번은 runtime 안에서만 유일하므로 창의 영속 ID와 함께 식별합니다.
pub(super) fn command_id(window: WindowSessionId, request: RequestId) -> CommandId {
    let mut key = [0; 24];
    key[..16].copy_from_slice(&window.stable_id().to_bytes());
    key[16..].copy_from_slice(&request.0.to_le_bytes());
    let digest = stellar_storage::hash::ContentHash::compute(&key);
    let mut raw = [0; 16];
    raw.copy_from_slice(&digest.digest()[..16]);
    CommandId::from_bytes(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_window_tests::{next_event, open_project, open_window};
    use crate::window_session::test_fixtures::{
        identity, sample_document, saved_project_with, sid, window,
    };
    use stellar_commands::selection::Selection;

    fn transform_of(window: &ProjectWindowSession, node: StableId) -> Transform2D {
        match window.document().unwrap().model().payload(node).unwrap() {
            NodePayload::Frame(frame) => frame.transform,
            _ => unreachable!(),
        }
    }

    #[test]
    fn move_uses_visible_revision_selection_and_persists_one_transform() {
        let mut model = sample_document();
        let node = sid(11);
        let Some(NodePayload::Frame(mut frame)) = model.payload(node).cloned() else {
            unreachable!()
        };
        let original = Transform2D::new(0.5, 0.25, -0.25, 0.5, 12.5, -6.0).unwrap();
        frame.transform = original;
        model.set_payload(node, NodePayload::Frame(frame)).unwrap();
        let project = saved_project_with("canvas-move", &[&model]);
        let mut session = ProjectWindowSession::load(identity(project.path()), window(1)).unwrap();
        let delta = Vec2::new(16.0, -3.5).unwrap();
        let revision = session.revision();
        assert!(edit_selected(
            &mut session,
            RequestId(1),
            sid(3),
            revision,
            node,
            FrameEdit::Move(delta)
        )
        .is_err());
        session.select(Selection::new([node], None)).unwrap();
        assert!(edit_selected(
            &mut session,
            RequestId(2),
            sid(4),
            revision,
            node,
            FrameEdit::Move(delta)
        )
        .is_err());
        assert!(edit_selected(
            &mut session,
            RequestId(3),
            sid(3),
            revision,
            node,
            FrameEdit::Move(delta)
        )
        .is_ok());
        let translated = Transform2D::new(0.5, 0.25, -0.25, 0.5, 28.5, -9.5).unwrap();
        assert_eq!(transform_of(&session, node), translated);
        assert!(edit_selected(
            &mut session,
            RequestId(4),
            sid(3),
            revision,
            node,
            FrameEdit::Move(delta)
        )
        .is_err());
        assert!(session.undo().unwrap());
        assert_eq!(transform_of(&session, node), original);
        assert_eq!(session.view().selection().ids(), [node]);
        assert!(session.redo().unwrap());
        session.save_now().unwrap();
        let reopened = ProjectWindowSession::load(identity(project.path()), window(2)).unwrap();
        assert_eq!(transform_of(&reopened, node), translated);
        assert_eq!(reopened.generation_number(), 2);
    }

    #[test]
    fn rectangle_translation_preserves_geometry_and_other_modifier_entries() {
        use stellar_document::frame::CornerRadius;
        use stellar_document::modifier::ModifierStack;
        use stellar_document::records::NodeRecord;
        use stellar_document::shape::RectangleParams;
        let delta = Vec2::new(7.0, -3.0).unwrap();
        let before = Transform2D::new(0.0, 2.0, -0.5, 0.0, -11.0, 8.0).unwrap();
        for has_transform in [false, true] {
            let mut model = sample_document();
            let parent = sid(11);
            let id = sid(30);
            let order_key = model.next_child_order_key(parent);
            let payload = NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
                size: Size2::new(40., 12.).unwrap(),
                corner_radius: CornerRadius::ZERO,
            }));
            model
                .insert_node(NodeRecord {
                    id,
                    parent: Some(parent),
                    order_key,
                    visible: true,
                    locked: false,
                    display_name: "Rectangle".into(),
                    payload: payload.clone(),
                })
                .unwrap();
            let mut stack = ModifierStack::new(id);
            let first = ModifierId::new(sid(31));
            let last = ModifierId::new(sid(32));
            if has_transform {
                stack.push_back(first, ModifierKind::Transform).unwrap();
                stack
                    .set_payload(
                        first,
                        ModifierPayload::Transform(
                            Transform2D::new(2., 0., 0., 1., 3., 4.).unwrap(),
                        ),
                    )
                    .unwrap();
                stack.push_back(last, ModifierKind::Transform).unwrap();
                stack
                    .set_payload(last, ModifierPayload::Transform(before))
                    .unwrap();
            }
            stack
                .push_back(ModifierId::new(sid(33)), ModifierKind::Fill)
                .unwrap();
            let disabled = ModifierId::new(sid(34));
            stack.push_back(disabled, ModifierKind::Transform).unwrap();
            stack.set_enabled(disabled, false).unwrap();
            model.set_modifier_stack(stack.clone()).unwrap();
            let dir = saved_project_with("rectangle-move-stack", &[&model]);
            let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
            session.select(Selection::new([id], None)).unwrap();
            let revision = session.revision();
            assert!(edit_selected(
                &mut session,
                RequestId(11),
                sid(3),
                revision,
                id,
                FrameEdit::Move(delta)
            )
            .is_ok());
            let changed = session
                .document()
                .unwrap()
                .model()
                .modifier_stack(id)
                .unwrap()
                .clone();
            let mut expected = stack.clone();
            if has_transform {
                expected
                    .set_payload(
                        last,
                        ModifierPayload::Transform(translated(before, delta).unwrap()),
                    )
                    .unwrap();
            } else {
                let inserted = &changed.entries()[0];
                assert_eq!(
                    inserted.payload,
                    ModifierPayload::Transform(translated(Transform2D::identity(), delta).unwrap())
                );
                expected.insert(inserted.clone()).unwrap();
            }
            assert_eq!(changed, expected);
            assert_eq!(session.view().selection().ids(), [id]);
            assert!(session.undo().unwrap());
            assert_eq!(
                session.document().unwrap().model().modifier_stack(id),
                Some(&stack)
            );
            assert!(session.redo().unwrap());
            session.save_now().unwrap();
            let reopened = ProjectWindowSession::load(identity(dir.path()), window(2)).unwrap();
            assert_eq!(
                reopened.document().unwrap().model().payload(id),
                Some(&payload)
            );
            assert_eq!(
                reopened.document().unwrap().model().modifier_stack(id),
                Some(&expected)
            );
            model.replace_flags(parent, true, true).unwrap();
            let locked = saved_project_with("rectangle-move-locked", &[&model]);
            let mut session =
                ProjectWindowSession::load(identity(locked.path()), window(1)).unwrap();
            session.select(Selection::new([id], None)).unwrap();
            let revision = session.revision();
            assert!(edit_selected(
                &mut session,
                RequestId(12),
                sid(3),
                revision,
                id,
                FrameEdit::Move(delta)
            )
            .is_err());
            assert!(!session.has_unsaved_changes());
        }
    }

    #[test]
    fn locked_ancestor_prevents_moving_a_selected_child() {
        let mut model = sample_document();
        model.replace_flags(sid(10), true, true).unwrap();
        let project = saved_project_with("canvas-move-locked", &[&model]);
        let mut session = ProjectWindowSession::load(identity(project.path()), window(1)).unwrap();
        let revision = session.revision();
        session.select(Selection::new([sid(11)], None)).unwrap();
        let delta = Vec2::new(12.0, -8.5).unwrap();
        assert!(edit_selected(
            &mut session,
            RequestId(1),
            sid(3),
            revision,
            sid(11),
            FrameEdit::Move(delta)
        )
        .is_err());
        assert!(edit_selected(
            &mut session,
            RequestId(2),
            sid(3),
            revision,
            sid(11),
            FrameEdit::Resize(Size2::new(45.0, 70.0).unwrap())
        )
        .is_err());
        assert!(edit_selected(
            &mut session,
            RequestId(3),
            sid(3),
            revision,
            sid(11),
            FrameEdit::Background(None)
        )
        .is_err());
        assert_eq!(session.revision(), revision);
        assert!(!session.has_unsaved_changes());
    }

    #[test]
    fn untrusted_window_can_select_but_cannot_move_a_frame() {
        let project = saved_project_with("canvas-move-restricted", &[&sample_document()]);
        let app = AppRuntime::start().unwrap();
        let owner = open_project(&app, project.path());
        let (session, _, _) = open_window(&app, owner);
        app.set_selection(session, vec![sid(11)]).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::WindowViewChanged { .. }
        ));
        let rev = app.state().window(session).unwrap().revision;
        let attempted = app
            .move_canvas_frame(
                session,
                sid(3),
                rev,
                sid(11),
                Vec2::new(20.0, 10.0).unwrap(),
            )
            .unwrap();
        assert!(
            matches!(next_event(&app),RuntimeEvent::RequestFailed{request,failure: RequestFailure::TrustUnavailable,..} if request == attempted)
        );
        let resized = app
            .resize_canvas_frame(
                session,
                sid(3),
                rev,
                sid(11),
                Size2::new(10.0, 15.0).unwrap(),
            )
            .unwrap();
        assert!(
            matches!(next_event(&app),RuntimeEvent::RequestFailed{request,failure: RequestFailure::TrustUnavailable,..} if request == resized)
        );
        let state = app.state();
        let summary = state.window(session).unwrap();
        assert_eq!(summary.revision, rev);
        assert!(!summary.has_unsaved_changes);
    }
}
