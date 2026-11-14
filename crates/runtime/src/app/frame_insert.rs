//! 자유 배치 Frame에 새 Frame, 사각형 Shape 또는 본문이 있는 Text를 삽입하고 선택합니다.

use super::frame_move::{command_id, editable_frame, trusted_canvas_window};
use super::*;
use stellar_commands::{
    command::Command, id::CommandOrigin, node_ops::InsertNode, transaction::CommandTransaction,
};
use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::value::{Rect, Transform2D, Vec2};
use stellar_document::frame::{CornerRadius, FramePayload};
use stellar_document::modifier::{
    ModifierEntry, ModifierId, ModifierKind, ModifierPayload, ModifierStack,
};
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_document::shape::{RectangleParams, ShapeGeometry};

use stellar_document::text::{edit_text, TextPayload, TextRange};

enum CanvasItem {
    Frame(Rect),
    Rectangle(Rect),
    Text { origin: Vec2, content: String },
}

impl AppRuntime {
    /// 부모의 local 공간에서 그린 영역에 Frame을 삽입하고 선택합니다.
    pub fn insert_canvas_frame(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
        parent: StableId,
        bounds: Rect,
    ) -> Result<RequestId, RequestError> {
        self.insert_canvas_item(session, view, revision, parent, CanvasItem::Frame(bounds))
    }

    /// 부모의 local 영역에 실제 Rectangle Shape와 Transform·Fill modifier를 생성합니다.
    pub fn insert_canvas_rectangle(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
        parent: StableId,
        bounds: Rect,
    ) -> Result<RequestId, RequestError> {
        self.insert_canvas_item(
            session,
            view,
            revision,
            parent,
            CanvasItem::Rectangle(bounds),
        )
    }

    /// 부모의 local 원점에 기본 문단 스타일을 상속한 Text와 위치를 한 번에 기록합니다.
    pub fn insert_canvas_text(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
        parent: StableId,
        origin: Vec2,
        content: String,
    ) -> Result<RequestId, RequestError> {
        self.insert_canvas_item(
            session,
            view,
            revision,
            parent,
            CanvasItem::Text { origin, content },
        )
    }

    fn insert_canvas_item(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
        parent: StableId,
        item: CanvasItem,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            let window = trusted_canvas_window(ctx, session)?;
            insert_item(window, request, view, revision, parent, item)?;
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
    }
}

fn insert_item(
    window: &mut ProjectWindowSession,
    request: RequestId,
    view: StableId,
    revision: DocumentRevision,
    parent: StableId,
    item: CanvasItem,
) -> Result<StableId, Failure> {
    let invalid = || {
        Failure::new(
            RequestFailure::UnknownView,
            "레이어를 추가할 뷰·부모·위치 또는 내용이 유효하지 않습니다",
        )
    };
    if window.revision() != revision || window.tabs().active_view() != Some(view) {
        return Err(invalid());
    }
    let model = window.document().ok_or_else(invalid)?.model();
    editable_frame(model, view, parent).ok_or_else(invalid)?;
    let id = command_id(window.window(), request);
    // 새 node 식별자는 창 ID와 요청 순번으로부터 얻고 UUIDv7의 variant·version을 유지합니다.
    let bytes = id.to_bytes();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| invalid())?;
    let ms = u64::try_from(now.as_millis()).map_err(|_| invalid())?;
    let node = StableId::new_v7(
        ms,
        u16::from_le_bytes([bytes[0], bytes[1]]),
        u64::from_le_bytes(bytes[2..10].try_into().expect("eight bytes")),
    );
    let origin = match &item {
        CanvasItem::Frame(bounds) | CanvasItem::Rectangle(bounds) if !bounds.is_empty() => {
            bounds.origin()
        }
        CanvasItem::Text { origin, content } if !content.is_empty() => *origin,
        _ => return Err(invalid()),
    };
    let transform =
        Transform2D::new(1.0, 0.0, 0.0, 1.0, origin.x(), origin.y()).map_err(|_| invalid())?;
    let (name, payload, modifiers) = match item {
        CanvasItem::Frame(bounds) => (
            "Frame",
            NodePayload::Frame(FramePayload {
                size: bounds.size(),
                transform,
                background: Some(
                    Color::new(ColorSpace::Srgb, 1.0, 1.0, 1.0, 1.0).expect("opaque white"),
                ),
                ..FramePayload::default()
            }),
            [None, None],
        ),
        CanvasItem::Rectangle(bounds) => (
            "Rectangle",
            NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
                size: bounds.size(),
                corner_radius: CornerRadius::ZERO,
            })),
            [
                Some(ModifierPayload::Transform(transform)),
                Some(ModifierPayload::default_for(ModifierKind::Fill)),
            ],
        ),
        CanvasItem::Text { content, .. } => {
            let text = edit_text(
                &TextPayload::empty(0),
                TextRange::new(0, 0).expect("empty range"),
                &content,
                &[],
            )
            .map_err(|_| invalid())?;
            (
                "Text",
                NodePayload::Text(text),
                [Some(ModifierPayload::Transform(transform)), None],
            )
        }
    };
    let mut insert = InsertNode::single(NodeRecord {
        id: node,
        parent: Some(parent),
        order_key: model.next_child_order_key(parent),
        visible: true,
        locked: false,
        display_name: name.into(),
        payload,
    });
    let mut stack = ModifierStack::new(node);
    for (slot, payload) in modifiers.into_iter().flatten().enumerate() {
        let mut raw = node.to_bytes();
        raw[15] ^= slot as u8 + 1;
        let order = OrderKey::between(stack.entries().last().map(|entry| &entry.order), None);
        stack
            .insert(ModifierEntry {
                id: ModifierId::new(StableId::from_bytes(raw)),
                order,
                kind: payload.kind(),
                enabled: true,
                payload,
            })
            .map_err(|_| invalid())?;
    }
    if !stack.entries().is_empty() {
        insert.modifier_stacks.push(stack);
    }
    let transaction = CommandTransaction::new(id, CommandOrigin::User)
        .with_command(revision, Command::InsertNode(insert));
    window
        .insert_and_select(&transaction, node)
        .map_err(view_failure)?;
    Ok(node)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window_session::test_fixtures::{identity, saved_project, sid, window};
    use stellar_commands::selection::Selection;
    use stellar_core_types::value::{Size2, Vec2};

    fn insert_frame(
        window: &mut ProjectWindowSession,
        request: RequestId,
        view: StableId,
        revision: DocumentRevision,
        parent: StableId,
        bounds: Rect,
    ) -> Result<StableId, Failure> {
        insert_item(
            window,
            request,
            view,
            revision,
            parent,
            CanvasItem::Frame(bounds),
        )
    }

    #[test]
    fn inserted_text_keeps_paragraphs_and_local_anchor_on_undo_redo_and_reopen() {
        let project = saved_project("canvas-insert-text");
        let mut session = ProjectWindowSession::load(identity(project.path()), window(1)).unwrap();
        let origin = Vec2::new(-3.5, 42.).unwrap();
        let revision = session.revision();
        for content in ["", "한글🎨\n둘째 줄"] {
            let result = insert_item(
                &mut session,
                RequestId(200),
                sid(3),
                revision,
                sid(10),
                CanvasItem::Text {
                    origin,
                    content: content.into(),
                },
            );
            if content.is_empty() {
                assert!(result.is_err());
                assert_eq!(session.revision(), revision);
                continue;
            }
            let node = result.ok().expect("text insertion");
            assert_eq!(session.view().selection().ids(), [node]);
            session.undo().unwrap();
            assert!(!session.document().unwrap().model().contains(node));
            session.redo().unwrap();
            session.save_now().unwrap();
            let reopened = ProjectWindowSession::load(identity(project.path()), window(2)).unwrap();
            let model = reopened.document().unwrap().model();
            let Some(NodePayload::Text(text)) = model.payload(node) else {
                panic!("Text node");
            };
            assert_eq!(text.content(), content);
            assert_eq!(
                text.paragraphs()
                    .iter()
                    .map(|p| p.style)
                    .collect::<Vec<_>>(),
                [0, 0]
            );
            let stack = model.modifier_stack(node).unwrap().entries();
            assert_eq!(stack.len(), 1);
            assert!(
                matches!(stack[0].payload, ModifierPayload::Transform(t) if t.components() == (1., 0., 0., 1., -3.5, 42.))
            );
        }
    }

    #[test]
    fn inserted_rectangle_keeps_shape_geometry_and_modifier_identity_after_reopen() {
        let project = saved_project("canvas-insert-rectangle");
        let mut session = ProjectWindowSession::load(identity(project.path()), window(1)).unwrap();
        let bounds = Rect::new(
            Vec2::new(14.0, 22.0).unwrap(),
            Size2::new(70.0, 40.0).unwrap(),
        );
        let revision = session.revision();
        let node = insert_item(
            &mut session,
            RequestId(171),
            sid(3),
            revision,
            sid(10),
            CanvasItem::Rectangle(bounds),
        )
        .ok()
        .expect("Rectangle 삽입");
        let model = session.document().unwrap().model();
        let stack = model.modifier_stack(node).unwrap().clone();
        let entries = stack.entries();
        assert!(
            matches!(entries[0].payload, ModifierPayload::Transform(t) if t.components() == (1.,0.,0.,1.,14.,22.))
        );
        assert_eq!(
            entries[1].payload,
            ModifierPayload::default_for(ModifierKind::Fill)
        );
        assert_eq!(model.children_in_order(sid(10)).last(), Some(&node));
        assert_eq!(session.view().selection().ids(), [node]);
        session.undo().unwrap();
        assert!(!session.document().unwrap().model().contains(node));
        session.redo().unwrap();
        session.save_now().unwrap();
        let reopened = ProjectWindowSession::load(identity(project.path()), window(2)).unwrap();
        let loaded = reopened.document().unwrap().model();
        assert!(
            matches!(loaded.payload(node), Some(NodePayload::Shape(ShapeGeometry::Rectangle(r))) if r.size == bounds.size())
        );
        assert_eq!(loaded.modifier_stack(node), Some(&stack));
    }

    #[test]
    fn insert_and_selection_share_an_entry_and_save_real_geometry() {
        let dir = saved_project("canvas-insert");
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        session.select(Selection::new([sid(12)], None)).unwrap();
        let revision = session.revision();
        let bounds = Rect::new(
            Vec2::new(12.5, 45.25).unwrap(),
            Size2::new(101.0, 68.0).unwrap(),
        );
        assert!(insert_frame(
            &mut session,
            RequestId(1),
            sid(4),
            revision,
            sid(10),
            bounds
        )
        .is_err());
        assert!(insert_frame(
            &mut session,
            RequestId(1),
            sid(3),
            revision,
            sid(20),
            bounds
        )
        .is_err());
        let created = insert_frame(
            &mut session,
            RequestId(2),
            sid(3),
            revision,
            sid(10),
            bounds,
        )
        .ok()
        .unwrap();
        assert_eq!(session.view().selection().ids(), [created]);
        assert_eq!(
            session
                .document()
                .unwrap()
                .model()
                .children_in_order(sid(10))
                .last(),
            Some(&created)
        );
        assert!(insert_frame(
            &mut session,
            RequestId(3),
            sid(3),
            revision,
            sid(10),
            bounds
        )
        .is_err());
        assert!(session.undo().unwrap());
        assert_eq!(session.view().selection().ids(), [sid(12)]);
        assert!(!session.document().unwrap().model().contains(created));
        assert!(session.redo().unwrap());
        assert_eq!(session.view().selection().ids(), [created]);
        session.save_now().unwrap();
        let reopened = ProjectWindowSession::load(identity(dir.path()), window(2)).unwrap();
        let model = reopened.document().unwrap().model();
        let Some(NodePayload::Frame(frame)) = model.payload(created) else {
            panic!("저장된 Frame이 없음");
        };
        assert_eq!(frame.size, bounds.size());
        assert_eq!(
            frame.transform.components(),
            (1.0, 0.0, 0.0, 1.0, 12.5, 45.25)
        );
    }
}
