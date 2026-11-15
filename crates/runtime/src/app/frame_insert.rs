//! 자유 배치 Frame에 새 Frame, Shape 또는 본문이 있는 Text를 삽입하고 선택합니다.

use super::frame_move::{command_id, editable_frame, trusted_canvas_window};
use super::*;
use stellar_commands::{
    command::Command, id::CommandOrigin, node_ops::InsertNode, transaction::CommandTransaction,
};
use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::value::{Angle, Rect, Size2, Transform2D, Vec2};
use stellar_document::frame::{CornerRadius, FramePayload};
use stellar_document::modifier::{
    ModifierEntry, ModifierId, ModifierKind, ModifierPayload, ModifierStack,
};
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_document::shape::{
    EllipseParams, LineCap, LineParams, PolygonParams, RectangleParams, ShapeGeometry,
};
use stellar_document::text::{edit_text, TextPayload, TextRange};

enum CanvasItem {
    Frame(Rect),
    Shape {
        origin: Vec2,
        geometry: ShapeGeometry,
    },
    Text {
        id: StableId,
        origin: Vec2,
        content: String,
    },
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
        self.insert_canvas_item(
            session,
            view,
            revision,
            parent,
            Some(CanvasItem::Frame(bounds)),
        )
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
        self.insert_canvas_item(session, view, revision, parent, rectangle(bounds))
    }

    /// 방향이 있는 두 부모 local 점으로 도형을 만들고, 선택까지 한 history 항목에 기록합니다.
    pub fn insert_canvas_shape(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
        parent: StableId,
        kind: ShapeKind,
        points: [Vec2; 2],
    ) -> Result<RequestId, RequestError> {
        self.insert_canvas_item(
            session,
            view,
            revision,
            parent,
            shape_from_points(kind, points),
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
    ) -> Result<(RequestId, StableId), RequestError> {
        let [id] = stellar_project::new_ids::new_ids().map_err(|_| RequestError::Stopped)?;
        self.insert_canvas_item(
            session,
            view,
            revision,
            parent,
            Some(CanvasItem::Text {
                id,
                origin,
                content,
            }),
        )
        .map(|request| (request, id))
    }

    fn insert_canvas_item(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
        parent: StableId,
        item: Option<CanvasItem>,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            let window = trusted_canvas_window(ctx, session)?;
            let item = item.ok_or_else(|| {
                Failure::new(
                    RequestFailure::UnknownView,
                    "도형의 원점 또는 크기가 유효하지 않습니다",
                )
            })?;
            insert_item(window, request, view, revision, parent, item)?;
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
    }
}

fn rectangle(bounds: Rect) -> Option<CanvasItem> {
    (!bounds.is_empty()).then_some(CanvasItem::Shape {
        origin: bounds.origin(),
        geometry: ShapeGeometry::Rectangle(RectangleParams {
            size: bounds.size(),
            corner_radius: CornerRadius::ZERO,
        }),
    })
}

fn shape_from_points(kind: ShapeKind, [start, end]: [Vec2; 2]) -> Option<CanvasItem> {
    let delta = Vec2::new(end.x() - start.x(), end.y() - start.y()).ok()?;
    let size = Size2::new(delta.x().abs(), delta.y().abs()).ok()?;
    let origin = Vec2::new(start.x().min(end.x()), start.y().min(end.y())).ok()?;
    let bounds = Rect::new(Vec2::ZERO, size);
    let geometry = match kind {
        ShapeKind::Line if delta != Vec2::ZERO => {
            return Some(CanvasItem::Shape {
                origin: start,
                geometry: ShapeGeometry::Line(LineParams {
                    start: Vec2::ZERO,
                    end: delta,
                    cap: LineCap::Butt,
                }),
            });
        }
        ShapeKind::Rectangle => return rectangle(Rect::new(origin, size)),
        ShapeKind::Ellipse if !size.is_empty() => {
            ShapeGeometry::Ellipse(EllipseParams::new(bounds).ok()?)
        }
        ShapeKind::Polygon if !size.is_empty() => {
            ShapeGeometry::Polygon(PolygonParams::new(bounds, 5, Angle::new(0.).ok()?, 0.).ok()?)
        }
        _ => return None,
    };
    Some(CanvasItem::Shape { origin, geometry })
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
    let node = if let CanvasItem::Text { id, .. } = &item {
        *id
    } else {
        // 새 node 식별자는 창 ID와 요청 순번으로부터 얻고 UUIDv7의 variant·version을 유지합니다.
        let bytes = id.to_bytes();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| invalid())?;
        let ms = u64::try_from(now.as_millis()).map_err(|_| invalid())?;
        StableId::new_v7(
            ms,
            u16::from_le_bytes([bytes[0], bytes[1]]),
            u64::from_le_bytes(bytes[2..10].try_into().expect("eight bytes")),
        )
    };
    let origin = match &item {
        CanvasItem::Frame(bounds) if !bounds.is_empty() => bounds.origin(),
        CanvasItem::Shape { origin, .. } => *origin,
        CanvasItem::Text {
            origin, content, ..
        } if !content.is_empty() => *origin,
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
        CanvasItem::Shape { geometry, .. } => {
            let (name, paint) = match geometry.kind() {
                ShapeKind::Rectangle => ("Rectangle", ModifierKind::Fill),
                ShapeKind::Ellipse => ("Ellipse", ModifierKind::Fill),
                ShapeKind::Line => ("Line", ModifierKind::Stroke),
                ShapeKind::Polygon => ("Polygon", ModifierKind::Fill),
            };
            (
                name,
                NodePayload::Shape(geometry),
                [
                    Some(ModifierPayload::Transform(transform)),
                    Some(ModifierPayload::default_for(paint)),
                ],
            )
        }
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
                    id: sid(331),
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
        use crate::window_session::test_fixtures::{other_document, saved_project_with};
        let document = other_document();
        let view = &document.definition().pages[0].views[0];
        let project = saved_project_with("canvas-insert-rectangle", &[&document]);
        let mut session = ProjectWindowSession::load(identity(project.path()), window(1)).unwrap();
        let bounds = Rect::new(
            Vec2::new(14.0, 22.0).unwrap(),
            Size2::new(70.0, 40.0).unwrap(),
        );
        let revision = session.revision();
        let node = insert_item(
            &mut session,
            RequestId(171),
            view.id,
            revision,
            view.root_frame,
            rectangle(bounds).unwrap(),
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
        assert_eq!(model.children_in_order(view.root_frame).last(), Some(&node));
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
        let view_source = project.path().join("rectangle.stellar.yaml");
        stellar_ops::text_source::export(project.path(), &view_source).unwrap();
        let source = stellar_project::document_io::text_source::open(&view_source).unwrap();
        assert_eq!(
            loaded.to_records().unwrap(),
            source.model.to_records().unwrap()
        );
    }

    #[test]
    fn drawn_shapes_preserve_local_geometry_paint_selection_and_history_on_reopen() {
        let project = saved_project("draw-shapes");
        let mut session = ProjectWindowSession::load(identity(project.path()), window(1)).unwrap();
        let parent = sid(11);
        session.select(Selection::new([parent], None)).unwrap();
        let point = |x, y| Vec2::new(x, y).unwrap();
        let start = point(25., 35.);
        let targets = [
            (ShapeKind::Ellipse, point(5., 11.)),
            (ShapeKind::Polygon, point(5., 11.)),
            (ShapeKind::Line, point(-12., 35.)),
            (ShapeKind::Line, point(25., -12.)),
            (ShapeKind::Line, point(-12., 50.)),
        ];
        for kind in ShapeKind::ALL {
            assert!(shape_from_points(kind, [start, start]).is_none());
        }
        assert!(shape_from_points(ShapeKind::Ellipse, [start, point(25., -12.)]).is_none());
        let mut saved = Vec::new();
        for (index, (kind, end)) in targets.into_iter().enumerate() {
            let revision = session.revision();
            let previous = session.view().selection().ids().to_vec();
            let draw = || shape_from_points(kind, [start, end]).unwrap();
            let request = RequestId(250 + index as u64);
            let id = insert_item(&mut session, request, sid(3), revision, parent, draw())
                .ok()
                .unwrap();
            assert!(insert_item(&mut session, request, sid(3), revision, parent, draw()).is_err());
            let model = session.document().unwrap().model();
            assert_eq!(model.children_in_order(parent).last(), Some(&id));
            let Some(NodePayload::Shape(shape)) = model.payload(id) else {
                panic!("shape");
            };
            let bounds = Rect::new(Vec2::ZERO, Size2::new(20., 24.).unwrap());
            let (origin, paint) = match shape {
                ShapeGeometry::Ellipse(ellipse) => {
                    assert_eq!(ellipse.bounds, bounds);
                    (end, ModifierKind::Fill)
                }
                ShapeGeometry::Polygon(polygon) => {
                    assert_eq!(polygon.bounds(), bounds);
                    assert_eq!(
                        (
                            polygon.sides(),
                            polygon.rotation_degrees(),
                            polygon.corner_radius()
                        ),
                        (5, 0., 0.)
                    );
                    (end, ModifierKind::Fill)
                }
                ShapeGeometry::Line(line) => {
                    assert_eq!(line.start, Vec2::ZERO);
                    assert_eq!(line.end, point(end.x() - start.x(), end.y() - start.y()));
                    assert_eq!(line.cap, LineCap::Butt);
                    (start, ModifierKind::Stroke)
                }
                _ => panic!("unexpected shape"),
            };
            assert_eq!(shape.kind(), kind);
            let stack = model.modifier_stack(id).unwrap();
            assert_eq!(stack.entries().len(), 2);
            assert!(
                matches!(stack.entries()[0].payload, ModifierPayload::Transform(t)
                if t.components() == (1., 0., 0., 1., origin.x(), origin.y()))
            );
            assert_eq!(
                stack.entries()[1].payload,
                ModifierPayload::default_for(paint)
            );
            saved.push((id, *shape, stack.clone()));
            assert_eq!(session.view().selection().ids(), [id]);
            assert!(session.undo().unwrap());
            assert_eq!(session.view().selection().ids(), previous);
            assert!(!session.document().unwrap().model().contains(id));
            assert!(session.redo().unwrap());
            assert_eq!(session.view().selection().ids(), [id]);
        }
        session.save_now().unwrap();
        let reopened = ProjectWindowSession::load(identity(project.path()), window(2)).unwrap();
        let model = reopened.document().unwrap().model();
        for (id, shape, stack) in saved {
            assert_eq!(model.payload(id), Some(&NodePayload::Shape(shape)));
            assert_eq!(model.modifier_stack(id), Some(&stack));
        }
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
