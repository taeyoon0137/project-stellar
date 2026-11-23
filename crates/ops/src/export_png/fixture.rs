//! PNG 내보내기 테스트 fixture(ADR-0116 결정 5). 프로덕션 경로에서 사용하지 않습니다.
//!
//! 문서 1, 페이지 2, View 3(root 10). root Frame 10은 40×20 빨간 배경(world (100, 50)), 자식 Frame 11은 10×10 파란
//! 배경((5, 5))이고 Opacity 0.5 modifier를 가집니다. 변형은 Text 12(headless layout 없음)나 Shadow modifier를
//! 더합니다.
#![cfg(any(test, feature = "test-support"))]

use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::frame::FramePayload;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{ModifierEntry, ModifierId, ModifierPayload, ModifierStack};
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_document::text::{Paragraph, TextPayload};

use crate::core_types::color::{Color, ColorSpace};
use crate::core_types::ids::StableId;
use crate::core_types::order_key::OrderKey;
use crate::core_types::text::UserText;
use crate::core_types::value::{FiniteF32, Size2, Transform2D, Vec2};

/// 결정론 fixture id.
pub fn id(n: u64) -> StableId {
    StableId::new_v7(n, 0, n)
}

/// 문서 id.
pub fn document_id() -> StableId {
    id(1)
}

/// View id.
pub fn view_id() -> StableId {
    id(3)
}

/// fixture 변형.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// Frame 둘(불투명 빨강, 반투명 파랑).
    Plain,
    /// Text 하나를 더함(headless 변환은 Text를 넣지 못함).
    WithText,
    /// 파란 Frame에 Shadow modifier를 더함(CPU target 미지원 효과).
    WithShadow,
}

fn frame(w: f32, h: f32, x: f32, y: f32, rgb: [f32; 3]) -> FramePayload {
    FramePayload {
        size: Size2::new(w, h).unwrap(),
        transform: Transform2D::new(1.0, 0.0, 0.0, 1.0, x, y).unwrap(),
        background: Some(Color::new(ColorSpace::Srgb, rgb[0], rgb[1], rgb[2], 1.0).unwrap()),
        ..FramePayload::default()
    }
}

fn record(n: u64, parent: Option<u64>, key: u8, payload: NodePayload) -> NodeRecord {
    NodeRecord {
        id: id(n),
        parent: parent.map(id),
        order_key: OrderKey::from_bytes(vec![key]).unwrap(),
        visible: true,
        locked: false,
        display_name: format!("n{n}"),
        payload,
    }
}

fn entry(n: u64, key: u8, payload: ModifierPayload) -> ModifierEntry {
    ModifierEntry {
        id: ModifierId::new(id(n)),
        order: OrderKey::from_bytes(vec![key]).unwrap(),
        kind: payload.kind(),
        enabled: true,
        payload,
    }
}

/// fixture 문서.
pub fn document(variant: Variant) -> DocumentModel {
    let mut definition = DocumentDefinition::new(document_id(), "PNG fixture".into());
    let mut page = PageDefinition::new(id(2), "페이지".into());
    page.add_view(ViewDefinition::new(view_id(), "화면".into(), id(10)))
        .unwrap();
    definition.add_page(page).unwrap();
    let mut model = DocumentModel::new(definition);
    model
        .insert_node(record(
            10,
            None,
            0x10,
            NodePayload::Frame(frame(40.0, 20.0, 100.0, 50.0, [1.0, 0.0, 0.0])),
        ))
        .unwrap();
    model
        .insert_node(record(
            11,
            Some(10),
            0x10,
            NodePayload::Frame(frame(10.0, 10.0, 5.0, 5.0, [0.0, 0.0, 1.0])),
        ))
        .unwrap();
    let mut stack = ModifierStack::new(id(11));
    stack
        .insert(entry(
            111,
            0x10,
            ModifierPayload::Opacity {
                value: FiniteF32::new(0.5).unwrap(),
            },
        ))
        .unwrap();
    if variant == Variant::WithShadow {
        stack
            .insert(entry(
                112,
                0x20,
                ModifierPayload::Shadow {
                    color: Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 0.5).unwrap(),
                    offset: Vec2::new(1.0, 1.0).unwrap(),
                    blur: FiniteF32::new(2.0).unwrap(),
                },
            ))
            .unwrap();
    }
    model.set_modifier_stack(stack).unwrap();
    if variant == Variant::WithText {
        model
            .insert_node(record(
                12,
                Some(10),
                0x20,
                NodePayload::Text(
                    TextPayload::new(
                        UserText::new("PNG"),
                        vec![Paragraph { style: 0 }],
                        Vec::new(),
                    )
                    .unwrap(),
                ),
            ))
            .unwrap();
    }
    model
}
