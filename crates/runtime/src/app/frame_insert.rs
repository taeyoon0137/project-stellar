//! 자유 배치 Frame에 새 Frame 또는 Shape를 삽입하고 선택합니다. Text 생성은 `text_input`의 첫 확정이 합니다.

use super::frame_move::{command_id, editable_frame, fresh_ids};
use super::*;
use stellar_commands::{
    command::Command, id::CommandOrigin, node_ops::InsertNode, transaction::CommandTransaction,
};
use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::value::{Angle, FiniteF32, Rect, Size2, Transform2D, Vec2};
use stellar_document::frame::{CornerRadius, FramePayload};
use stellar_document::modifier::{ModifierEntry, ModifierId, ModifierPayload, ModifierStack};
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_document::shape::{
    EllipseParams, LineCap, LineParams, PolygonParams, RectangleParams, ShapeGeometry,
};

/// 새 도형의 단색 칠(D7). 셸이 앱 설정 값을 넘기고 코어는 기본 색을 두지 않습니다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShapePaint {
    /// Fill modifier 색. `None`이면 Fill을 만들지 않습니다.
    pub fill: Option<Color>,
    /// Stroke modifier 색과 두께. Line은 반드시 있어야 합니다.
    pub stroke: Option<(Color, FiniteF32)>,
}

enum CanvasItem {
    Frame(Rect),
    Shape {
        origin: Vec2,
        geometry: ShapeGeometry,
        paint: ShapePaint,
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

    /// 방향이 있는 두 부모 local 점으로 도형을 만들고, 선택까지 한 history 항목에 기록합니다.
    /// 칠은 셸이 넘긴 앱 설정 값(D7)을 그대로 Fill·Stroke modifier로 기록합니다.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_canvas_shape(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
        parent: StableId,
        kind: ShapeKind,
        points: [Vec2; 2],
        paint: ShapePaint,
    ) -> Result<RequestId, RequestError> {
        self.insert_canvas_item(
            session,
            view,
            revision,
            parent,
            shape_from_points(kind, points, paint),
        )
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
            let window = window_mut(ctx, session)?;
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

fn shape_from_points(
    kind: ShapeKind,
    [start, end]: [Vec2; 2],
    paint: ShapePaint,
) -> Option<CanvasItem> {
    // 선은 Stroke 없이는 보이지 않고, 닫힌 도형도 칠이 하나는 있어야 캔버스에서 고를 수 있습니다.
    let painted = match kind {
        ShapeKind::Line => paint.stroke.is_some(),
        _ => paint.fill.is_some() || paint.stroke.is_some(),
    };
    if !painted || paint.stroke.is_some_and(|(_, width)| width.get() <= 0.0) {
        return None;
    }
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
                paint,
            });
        }
        ShapeKind::Rectangle if !size.is_empty() => ShapeGeometry::Rectangle(RectangleParams {
            size,
            corner_radius: CornerRadius::ZERO,
        }),
        ShapeKind::Ellipse if !size.is_empty() => {
            ShapeGeometry::Ellipse(EllipseParams::new(bounds).ok()?)
        }
        ShapeKind::Polygon if !size.is_empty() => {
            ShapeGeometry::Polygon(PolygonParams::new(bounds, 5, Angle::new(0.).ok()?, 0.).ok()?)
        }
        _ => return None,
    };
    Some(CanvasItem::Shape {
        origin,
        geometry,
        paint,
    })
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
    // 새 node와 modifier는 독립 UUIDv7을 씁니다.
    let [node, transform_id, fill_id, stroke_id] = fresh_ids()?;
    let origin = match &item {
        CanvasItem::Frame(bounds) if !bounds.is_empty() => bounds.origin(),
        CanvasItem::Shape { origin, .. } => *origin,
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
            [None, None, None],
        ),
        CanvasItem::Shape {
            geometry, paint, ..
        } => {
            let name = match geometry.kind() {
                ShapeKind::Rectangle => "Rectangle",
                ShapeKind::Ellipse => "Ellipse",
                ShapeKind::Line => "Line",
                ShapeKind::Polygon => "Polygon",
            };
            // 캔버스 평가 순서(Transform → Fill → Stroke)대로 쌓습니다.
            (
                name,
                NodePayload::Shape(geometry),
                [
                    Some(ModifierPayload::Transform(transform)),
                    paint.fill.map(|color| ModifierPayload::Fill { color }),
                    paint
                        .stroke
                        .map(|(color, width)| ModifierPayload::Stroke { color, width }),
                ],
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
    for (modifier, payload) in [transform_id, fill_id, stroke_id]
        .into_iter()
        .zip(modifiers)
        .filter_map(|(id, payload)| Some((id, payload?)))
    {
        let order = OrderKey::between(stack.entries().last().map(|entry| &entry.order), None);
        stack
            .insert(ModifierEntry {
                id: ModifierId::new(modifier),
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
        .insert_and_select(
            &transaction,
            node,
            crate::window_session::history_label::INSERT,
        )
        .map_err(view_failure)?;
    Ok(node)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window_session::test_fixtures::{identity, saved_project, sid, window};
    use stellar_commands::selection::Selection;
    use stellar_core_types::value::{Size2, Vec2};

    /// Settings ▸ 캔버스 초기값과 같은 칠(#D9D9D9 채우기, #000000 1px 선).
    fn settings_paint(kind: ShapeKind) -> ShapePaint {
        let fill = Color::new(ColorSpace::Srgb, 217. / 255., 217. / 255., 217. / 255., 1.).unwrap();
        let line = Color::new(ColorSpace::Srgb, 0., 0., 0., 1.).unwrap();
        if kind == ShapeKind::Line {
            ShapePaint {
                fill: None,
                stroke: Some((line, FiniteF32::ONE)),
            }
        } else {
            ShapePaint {
                fill: Some(fill),
                stroke: None,
            }
        }
    }

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
        let paint = settings_paint(ShapeKind::Rectangle);
        // 끝점 순서가 반대여도 같은 상자를 만듭니다.
        let corners = [Vec2::new(84.0, 62.0).unwrap(), bounds.origin()];
        let node = insert_item(
            &mut session,
            RequestId(171),
            view.id,
            revision,
            view.root_frame,
            shape_from_points(ShapeKind::Rectangle, corners, paint).unwrap(),
        )
        .ok()
        .expect("Rectangle 삽입");
        let model = session.document().unwrap().model();
        let stack = model.modifier_stack(node).unwrap().clone();
        let entries = stack.entries();
        assert_eq!(entries.len(), 2);
        // node와 modifier는 서로 독립인 UUIDv7입니다(command ID·node ID에서 파생하지 않음).
        let ids = [node, entries[0].id.stable_id(), entries[1].id.stable_id()];
        assert!(ids.iter().all(|id| (id.version(), id.variant()) == (7, 2)));
        assert!(ids[0] != ids[1] && ids[1] != ids[2] && ids[0] != ids[2]);
        assert!(
            matches!(entries[0].payload, ModifierPayload::Transform(t) if t.components() == (1.,0.,0.,1.,14.,22.))
        );
        assert_eq!(
            entries[1].payload,
            ModifierPayload::Fill {
                color: paint.fill.unwrap()
            }
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
        // 프로젝트 안 export는 두 번째 원본이 되므로 별도 폴더에 씁니다(ADR-0061 결정 5).
        let exports = crate::window_session::test_fixtures::TempDir::new("draw-rectangle-export");
        let view_source = exports.path().join("rectangle.stellar.yaml");
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
            assert!(shape_from_points(kind, [start, start], settings_paint(kind)).is_none());
            // 칠이 없거나 두께 0인 선은 보이지 않는 도형이라 만들지 않습니다.
            let unpainted = ShapePaint {
                fill: None,
                stroke: None,
            };
            assert!(shape_from_points(kind, [start, point(5., 11.)], unpainted).is_none());
        }
        let hairline = ShapePaint {
            fill: None,
            stroke: Some((
                Color::new(ColorSpace::Srgb, 0., 0., 0., 1.).unwrap(),
                FiniteF32::ZERO,
            )),
        };
        assert!(shape_from_points(ShapeKind::Line, [start, point(5., 11.)], hairline).is_none());
        let fill_only = settings_paint(ShapeKind::Rectangle);
        assert!(shape_from_points(ShapeKind::Line, [start, point(5., 11.)], fill_only).is_none());
        assert!(shape_from_points(
            ShapeKind::Ellipse,
            [start, point(25., -12.)],
            settings_paint(ShapeKind::Ellipse)
        )
        .is_none());
        let mut saved = Vec::new();
        for (index, (kind, end)) in targets.into_iter().enumerate() {
            let revision = session.revision();
            let previous = session.view().selection().ids().to_vec();
            let draw = || shape_from_points(kind, [start, end], settings_paint(kind)).unwrap();
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
            let paint = settings_paint(kind);
            let fill = paint.fill.map(|color| ModifierPayload::Fill { color });
            let stroke = paint
                .stroke
                .map(|(color, width)| ModifierPayload::Stroke { color, width });
            let (origin, expected) = match shape {
                ShapeGeometry::Ellipse(ellipse) => {
                    assert_eq!(ellipse.bounds, bounds);
                    (end, fill.unwrap())
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
                    (end, fill.unwrap())
                }
                ShapeGeometry::Line(line) => {
                    assert_eq!(line.start, Vec2::ZERO);
                    assert_eq!(line.end, point(end.x() - start.x(), end.y() - start.y()));
                    assert_eq!(line.cap, LineCap::Butt);
                    (start, stroke.unwrap())
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
            assert_eq!(stack.entries()[1].payload, expected);
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
