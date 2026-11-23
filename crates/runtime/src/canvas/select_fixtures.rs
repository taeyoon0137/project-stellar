//! 선택 엔진 테스트 fixture(테스트 전용).
//!
//! `canvas/select-scope-nested`: View root Frame 10(800×600) 아래 중첩 구조입니다.
//!
//! ```text
//! 10 Root Frame (0,0) 800×600
//! ├─ 11 Frame (100,100) 200×200
//! │  ├─ 13 Rectangle (120,120) 60×60
//! │  ├─ 14 Frame (200,200) 80×80
//! │  │  └─ 17 Rectangle (210,210) 30×30
//! │  ├─ 15 Rectangle (120,120) 40×40, 잠김
//! │  └─ 16 Rectangle (120,120) 60×60, 숨김
//! ├─ 12 Fragment
//! │  ├─ 18 Rectangle (300,20) 60×60
//! │  └─ 19 Rectangle (360,80) 60×60
//! └─ 21 ComponentInstance (500,300)
//! ```
//!
//! 형제 순서(OrderKey)는 위 나열 순서이며 뒤 형제가 위에 그려집니다.
#![cfg(test)]

use std::sync::Arc;

use stellar_commands::state::DocumentState;
use stellar_core_types::clock::ManualClock;
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::ComponentRef;
use stellar_core_types::revision::DocumentRevision;
use stellar_core_types::value::{Size2, Transform2D, Vec2};
use stellar_document::component::instance::ComponentInstancePayload;
use stellar_document::fragment::FragmentPayload;
use stellar_document::frame::FramePayload;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{
    ModifierEntry, ModifierId, ModifierKind, ModifierPayload, ModifierStack,
};
use stellar_document::record_codec::encode_node_record;
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_document::shape::{RectangleParams, ShapeGeometry};
use stellar_scene::incremental::IncrementalScene;
use stellar_scene::snapshot::SceneSnapshot;
use stellar_scene::spatial::SceneSpatialIndex;
use stellar_scene::visibility::Viewport;

use crate::window_session::test_fixtures::sample_document;
pub(crate) use crate::window_session::test_fixtures::sid;

use super::scene_source::{document_scene_source, SceneSourceOptions};

/// fixture 문서와 View root.
pub(crate) struct Fixture {
    pub model: DocumentModel,
    pub root: StableId,
}

/// 문서에서 만든 scene, 공간 인덱스, 항등 camera viewport.
pub(crate) struct Scene {
    pub snapshot: Arc<SceneSnapshot>,
    pub index: SceneSpatialIndex,
    pub viewport: Viewport,
}

pub(crate) fn frame_payload(x: f32, y: f32, w: f32, h: f32) -> NodePayload {
    NodePayload::Frame(FramePayload {
        size: Size2::new(w, h).unwrap(),
        transform: Transform2D::new(1., 0., 0., 1., x, y).unwrap(),
        ..FramePayload::default()
    })
}

pub(crate) fn record(n: u64, parent: u64, key: u8, payload: NodePayload) -> NodeRecord {
    NodeRecord {
        id: sid(n),
        parent: Some(sid(parent)),
        order_key: OrderKey::from_bytes(vec![key]).unwrap(),
        visible: true,
        locked: false,
        display_name: format!("n{n}"),
        payload,
    }
}

/// `parent` 아래 Frame. 위치는 부모 local 좌표입니다.
pub(crate) fn add_frame(
    model: &mut DocumentModel,
    n: u64,
    parent: u64,
    key: u8,
    rect: (f32, f32, f32, f32),
) {
    let (x, y, w, h) = rect;
    model
        .insert_node(record(n, parent, key, frame_payload(x, y, w, h)))
        .unwrap();
}

/// `parent` 아래 Rectangle. 위치는 부모 local 좌표의 Transform modifier입니다.
pub(crate) fn add_rectangle(
    model: &mut DocumentModel,
    n: u64,
    parent: u64,
    key: u8,
    rect: (f32, f32, f32, f32),
) {
    let (x, y, w, h) = rect;
    let payload = NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
        size: Size2::new(w, h).unwrap(),
        corner_radius: Default::default(),
    }));
    model.insert_node(record(n, parent, key, payload)).unwrap();
    let mut stack = ModifierStack::new(sid(n));
    stack
        .insert(ModifierEntry {
            id: ModifierId::new(sid(n + 10_000)),
            order: OrderKey::from_bytes(vec![0x40]).unwrap(),
            kind: ModifierKind::Transform,
            enabled: true,
            payload: ModifierPayload::Transform(Transform2D::new(1., 0., 0., 1., x, y).unwrap()),
        })
        .unwrap();
    model.set_modifier_stack(stack).unwrap();
}

pub(crate) fn set_flags(model: &mut DocumentModel, n: u64, visible: bool, locked: bool) {
    let mut record = model.record(sid(n)).unwrap();
    record.visible = visible;
    record.locked = locked;
    model.remove_leaf(sid(n)).unwrap();
    let stack = model.modifier_stack(sid(n)).cloned();
    model.insert_node(record).unwrap();
    if let Some(stack) = stack {
        model.set_modifier_stack(stack).unwrap();
    }
}

/// `canvas/select-scope-nested`.
pub(crate) fn nested() -> Fixture {
    let mut model = sample_document();
    model.remove_leaf(sid(11)).unwrap();
    model.remove_leaf(sid(12)).unwrap();
    model
        .set_payload(sid(10), frame_payload(0., 0., 800., 600.))
        .unwrap();
    add_frame(&mut model, 11, 10, 0x10, (100., 100., 200., 200.));
    add_rectangle(&mut model, 13, 11, 0x10, (20., 20., 60., 60.));
    add_frame(&mut model, 14, 11, 0x20, (100., 100., 80., 80.));
    add_rectangle(&mut model, 17, 14, 0x10, (10., 10., 30., 30.));
    add_rectangle(&mut model, 15, 11, 0x30, (20., 20., 40., 40.));
    add_rectangle(&mut model, 16, 11, 0x40, (20., 20., 60., 60.));
    model
        .insert_node(record(12, 10, 0x20, NodePayload::Fragment(FragmentPayload)))
        .unwrap();
    add_rectangle(&mut model, 18, 12, 0x10, (300., 20., 60., 60.));
    add_rectangle(&mut model, 19, 12, 0x20, (360., 80., 60., 60.));
    model
        .insert_node(record(
            21,
            10,
            0x30,
            NodePayload::ComponentInstance(ComponentInstancePayload::new(ComponentRef::new(sid(
                900,
            )))),
        ))
        .unwrap();
    set_flags(&mut model, 15, true, true);
    set_flags(&mut model, 16, false, false);
    Fixture {
        model,
        root: sid(10),
    }
}

/// 문서 View root subtree의 scene과 항등 camera.
pub(crate) fn scene_of(model: &DocumentModel, root: StableId) -> Scene {
    let built = document_scene_source(model, root, SceneSourceOptions::default()).unwrap();
    let (scene, _) =
        IncrementalScene::build(built.source, DocumentRevision(1), &ManualClock::new(0)).unwrap();
    let snapshot = Arc::new(scene.snapshot().clone());
    let index = SceneSpatialIndex::build(&snapshot);
    let viewport = Viewport::new(Size2::new(800., 600.).unwrap(), Transform2D::identity()).unwrap();
    Scene {
        snapshot,
        index,
        viewport,
    }
}

/// view 점(항등 camera라 world와 같음).
pub(crate) fn view_point(x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y).unwrap()
}

/// 문서 저장 bytes(정의 + node 레코드 + modifier stack canonical bytes).
pub(crate) fn save_bytes(state: &DocumentState) -> Vec<u8> {
    let records = state.model().to_records().unwrap();
    let mut bytes = records.definition.encode();
    for node in &records.nodes {
        bytes.extend(encode_node_record(node));
    }
    for stack in &records.modifier_stacks {
        bytes.extend(stack.encode());
    }
    bytes
}
