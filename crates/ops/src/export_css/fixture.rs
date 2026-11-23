//! CSS layout 내보내기 검증용 고정 문서(테스트 전용). 제품 경로에서 쓰지 않습니다.
//!
//! - View 3(root 10): `display: flex` root 안에 record 없는 Frame(11), `flex-grow` Frame(12, 그 안에 block 흐름
//!   자식 16), Rectangle(13), 명시 inset absolute Frame(14), 숨긴 Frame(15).
//! - View 4(root 20): record가 하나도 없는 Free Frame 둘. 활성 세트가 `css`면 고정 Free Frame 출력입니다.
//! - View 5(root 30): border가 있는 flex root와 content-box·padding 자식(border 보정 규칙).

use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::frame::{Border, FramePayload};
use stellar_document::layout_record::{LayoutLonghand as L, LayoutRecord};
use stellar_document::model::DocumentModel;
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_document::shape::{RectangleParams, ShapeGeometry};

use crate::core_types::{
    color::{Color, ColorSpace},
    ids::StableId,
    order_key::OrderKey,
    value::{Size2, Transform2D},
};

/// 결정론 fixture id(문서 1, 페이지 2, View 3·4·5, root 10·20·30).
pub fn id(n: u64) -> StableId {
    StableId::new_v7(n, 0, n)
}

fn frame(w: f32, h: f32, x: f32, y: f32) -> FramePayload {
    FramePayload {
        size: Size2::new(w, h).unwrap(),
        transform: Transform2D::new(1.0, 0.0, 0.0, 1.0, x, y).unwrap(),
        background: Some(Color::new(ColorSpace::Srgb, 0.2, 0.4, 0.6, 1.0).unwrap()),
        ..FramePayload::default()
    }
}

fn record(node: u64, values: &[(L, &str)]) -> LayoutRecord {
    let mut record = LayoutRecord::new(id(node));
    for (longhand, text) in values {
        record
            .set(*longhand, longhand.parse(text).unwrap())
            .unwrap();
    }
    record
}

/// 세 View 문서.
pub fn document() -> DocumentModel {
    let mut definition = DocumentDefinition::new(id(1), "CSS layout fixture".into());
    let mut page = PageDefinition::new(id(2), "페이지".into());
    for (view, root) in [(3, 10), (4, 20), (5, 30)] {
        page.add_view(ViewDefinition::new(
            id(view),
            format!("View {view}"),
            id(root),
        ))
        .unwrap();
    }
    definition.add_page(page).unwrap();
    let mut model = DocumentModel::new(definition);
    let f = NodePayload::Frame;
    let rectangle = NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
        size: Size2::new(20.0, 20.0).unwrap(),
        corner_radius: Default::default(),
    }));
    let mut bordered = frame(300.0, 200.0, 0.0, 0.0);
    bordered.border = Some(
        Border::new(
            Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0).unwrap(),
            2.0,
        )
        .unwrap(),
    );
    let mut thin = frame(100.0, 60.0, 0.0, 0.0);
    thin.border = Some(
        Border::new(
            Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0).unwrap(),
            4.0,
        )
        .unwrap(),
    );
    let nodes: Vec<(u64, Option<u64>, u8, bool, NodePayload)> = vec![
        (10, None, 1, true, f(frame(500.0, 300.0, 900.0, -40.0))),
        (11, Some(10), 1, true, f(frame(100.0, 50.0, 0.0, 0.0))),
        (12, Some(10), 2, true, f(frame(80.0, 40.0, 3.0, 4.0))),
        (16, Some(12), 1, true, f(frame(30.0, 10.0, 0.0, 0.0))),
        (13, Some(10), 3, true, rectangle),
        (14, Some(10), 4, true, f(frame(30.0, 30.0, 0.0, 0.0))),
        (15, Some(10), 5, false, f(frame(10.0, 10.0, 0.0, 0.0))),
        (20, None, 2, true, f(frame(200.0, 100.0, 0.0, 0.0))),
        (21, Some(20), 1, true, f(frame(40.0, 30.0, 12.0, 8.0))),
        (22, Some(20), 2, true, f(frame(20.0, 20.0, 60.0, 8.5))),
        (30, None, 3, true, f(bordered)),
        (31, Some(30), 1, true, f(thin)),
        (32, Some(30), 2, true, f(frame(40.0, 40.0, 0.0, 0.0))),
    ];
    for (n, parent, order, visible, payload) in nodes {
        model
            .insert_node(NodeRecord {
                id: id(n),
                parent: parent.map(id),
                order_key: OrderKey::from_bytes(vec![order]).unwrap(),
                visible,
                locked: false,
                display_name: String::new(),
                payload,
            })
            .unwrap();
    }
    for record in [
        record(
            10,
            &[
                (L::Display, "flex"),
                (L::ColumnGap, "8px"),
                (L::PaddingLeft, "10px"),
                (L::AlignItems, "flex-start"),
            ],
        ),
        record(
            12,
            &[
                (L::FlexGrow, "1"),
                (L::Display, "block"),
                (L::FlexDirection, "row"),
            ],
        ),
        record(
            14,
            &[(L::Position, "absolute"), (L::Top, "5px"), (L::Left, "7px")],
        ),
        record(30, &[(L::Display, "flex"), (L::PaddingTop, "6px")]),
        record(
            31,
            &[
                (L::BoxSizing, "content-box"),
                (L::PaddingLeft, "5px"),
                (L::PaddingRight, "3px"),
                (L::MinWidth, "50%"),
            ],
        ),
    ] {
        model.set_layout_record(record).unwrap();
    }
    model
}
