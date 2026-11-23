//! 캔버스 변형 엔진 test 문서와 창 session.

use std::sync::Arc;

use stellar_commands::arrange::place::local_transform;
use stellar_commands::selection::Selection;
use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::value::{Angle, FiniteF32, Rect, Size2, Transform2D, Vec2};
use stellar_document::frame::{CornerRadius, FramePayload};
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{ModifierEntry, ModifierId, ModifierPayload, ModifierStack};
use stellar_document::records::NodePayload;
use stellar_document::shape::{
    EllipseParams, LineCap, LineParams, PolygonParams, RectangleParams, ShapeGeometry,
};

use crate::canvas::{CanvasCache, CanvasScene};
use crate::window_session::test_fixtures::{
    identity, sample_document, saved_project_with, sid, window, TempDir,
};
use crate::window_session::ProjectWindowSession;

pub(crate) fn at(x: f32, y: f32) -> Transform2D {
    Transform2D::new(1., 0., 0., 1., x, y).unwrap()
}

pub(crate) fn frame(w: f32, h: f32, transform: Transform2D) -> NodePayload {
    NodePayload::Frame(FramePayload {
        size: Size2::new(w, h).unwrap(),
        transform,
        ..FramePayload::default()
    })
}

fn gray() -> Color {
    Color::new(ColorSpace::Srgb, 0.8, 0.8, 0.8, 1.).unwrap()
}

/// Transform 하나와 칠 하나(Line은 Stroke)를 가진 도형 node를 `parent` 아래 맨 앞에 넣습니다.
pub(crate) fn add_shape(
    model: &mut DocumentModel,
    id: u64,
    parent: u64,
    geometry: ShapeGeometry,
    position: Transform2D,
) {
    let mut record = model.record(sid(12)).unwrap();
    record.id = sid(id);
    record.parent = Some(sid(parent));
    record.order_key = model.next_child_order_key(sid(parent));
    record.display_name = format!("n{id}");
    let line = matches!(geometry, ShapeGeometry::Line(_));
    record.payload = NodePayload::Shape(geometry);
    model.insert_node(record).unwrap();
    let mut stack = ModifierStack::new(sid(id));
    let paint = if line {
        ModifierPayload::Stroke {
            color: gray(),
            width: FiniteF32::ONE,
        }
    } else {
        ModifierPayload::Fill { color: gray() }
    };
    for (slot, payload) in [ModifierPayload::Transform(position), paint]
        .into_iter()
        .enumerate()
    {
        let order = OrderKey::between(stack.entries().last().map(|e| &e.order), None);
        stack
            .insert(ModifierEntry {
                id: ModifierId::new(sid(id * 10 + slot as u64)),
                order,
                kind: payload.kind(),
                enabled: true,
                payload,
            })
            .unwrap();
    }
    model.set_modifier_stack(stack).unwrap();
}

/// View 3의 root 10(400×300) 아래:
/// - 11 Frame 100×50 at (10, 20), 12 Frame 60×40 at (200, 50)
/// - 13 Rectangle 40×20 at (30, 100), 14 Ellipse bounds (0,0,40,20) at (100, 100)
/// - 15 Line (0,0)-(30,10) at (150, 100), 16 Polygon 6변 40×20 at (200, 150)
pub(crate) fn canvas_document() -> DocumentModel {
    let mut model = sample_document();
    model
        .set_payload(sid(10), frame(400., 300., at(0., 0.)))
        .unwrap();
    model
        .set_payload(sid(11), frame(100., 50., at(10., 20.)))
        .unwrap();
    model
        .set_payload(sid(12), frame(60., 40., at(200., 50.)))
        .unwrap();
    let size = Size2::new(40., 20.).unwrap();
    add_shape(
        &mut model,
        13,
        10,
        ShapeGeometry::Rectangle(RectangleParams {
            size,
            corner_radius: CornerRadius::ZERO,
        }),
        at(30., 100.),
    );
    add_shape(
        &mut model,
        14,
        10,
        ShapeGeometry::Ellipse(EllipseParams::new(Rect::new(Vec2::ZERO, size)).unwrap()),
        at(100., 100.),
    );
    add_shape(
        &mut model,
        15,
        10,
        ShapeGeometry::Line(LineParams {
            start: Vec2::ZERO,
            end: Vec2::new(30., 10.).unwrap(),
            cap: LineCap::Butt,
        }),
        at(150., 100.),
    );
    add_shape(
        &mut model,
        16,
        10,
        ShapeGeometry::Polygon(
            PolygonParams::new(Rect::new(Vec2::ZERO, size), 6, Angle::new(0.).unwrap(), 3.)
                .unwrap(),
        ),
        at(200., 150.),
    );
    model
}

/// 저장한 프로젝트와 그 창 session.
pub(crate) fn open(tag: &str, model: &DocumentModel) -> (TempDir, ProjectWindowSession) {
    let dir = saved_project_with(tag, &[model]);
    let session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
    (dir, session)
}

/// 창의 지금 캔버스 scene.
pub(crate) fn scene(window: &ProjectWindowSession) -> Arc<CanvasScene> {
    CanvasCache::default()
        .current(window, None, &mut Default::default())
        .unwrap()
}

pub(crate) fn select(window: &mut ProjectWindowSession, nodes: &[u64]) {
    let ids: Vec<StableId> = nodes.iter().map(|&n| sid(n)).collect();
    let primary = ids.last().copied();
    window.select(Selection::new(ids, primary)).unwrap();
}

pub(crate) fn model(window: &ProjectWindowSession) -> &DocumentModel {
    window.document().unwrap().model()
}

/// node local → 부모 변환 성분.
pub(crate) fn matrix(window: &ProjectWindowSession, node: u64) -> (f32, f32, f32, f32, f32, f32) {
    local_transform(model(window), sid(node))
        .unwrap()
        .components()
}

pub(crate) fn history_len(window: &ProjectWindowSession) -> usize {
    window.history().history().len()
}

/// 두 행렬이 1e-3 안에서 같은지.
pub(crate) fn close(a: (f32, f32, f32, f32, f32, f32), b: (f32, f32, f32, f32, f32, f32)) -> bool {
    [
        a.0 - b.0,
        a.1 - b.1,
        a.2 - b.2,
        a.3 - b.3,
        a.4 - b.4,
        a.5 - b.5,
    ]
    .iter()
    .all(|d| d.abs() < 1e-3)
}
