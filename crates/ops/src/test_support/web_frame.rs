//! 저장·CLI·브라우저 검증용 고정 크기 View. UI의 카드·겹침·중첩과 affine을 포함합니다.

use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::fragment::FragmentPayload;
use stellar_document::frame::{Border, Clipping, CornerRadius, FramePayload};
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{ModifierId, ModifierKind, ModifierStack};
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_document::text::TextPayload;

use crate::core_types::{
    color::{Color, ColorSpace},
    ids::StableId,
    order_key::OrderKey,
    reference::NodeRef,
    value::{Size2, Transform2D},
};

/// 결정론 fixture id(문서 1, 페이지 2, View 3~6, root 10·20·30·40).
pub fn id(n: u64) -> StableId {
    StableId::new_v7(n, 0, n)
}

fn color(space: ColorSpace, rgba: [f32; 4]) -> Color {
    Color::new(space, rgba[0], rgba[1], rgba[2], rgba[3]).unwrap()
}

fn frame(w: f32, h: f32, x: f32, y: f32, rgba: [f32; 4]) -> FramePayload {
    FramePayload {
        size: Size2::new(w, h).unwrap(),
        transform: Transform2D::new(1.0, 0.0, 0.0, 1.0, x, y).unwrap(),
        background: Some(color(ColorSpace::Srgb, rgba)),
        ..FramePayload::default()
    }
}

/// View 3은 카드 UI, 4는 여러 affine, 5는 확대된 소수 이동입니다. 삽입·ID·OrderKey 순서를 다르게 둡니다.
pub fn document() -> DocumentModel {
    let mut definition = DocumentDefinition::new(id(1), "고정 Frame fixture".into());
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
    let mut root = frame(640.0, 400.0, 700.0, -900.0, [0.95, 0.96, 0.98, 1.0]);
    root.border = Some(Border::new(color(ColorSpace::Srgb, [0.1, 0.2, 0.3, 1.0]), 4.0).unwrap());
    root.corner_radius = CornerRadius::new(8.0, 16.0, 24.0, 32.0).unwrap();
    let mut card = frame(240.0, 160.0, 32.0, 48.0, [0.0; 4]);
    card.background = Some(color(ColorSpace::DisplayP3, [0.1, 0.2, 0.3, 0.8]));
    card.border =
        Some(Border::new(color(ColorSpace::DisplayP3, [0.8, 0.2, 0.1, 0.5]), 3.0).unwrap());
    card.corner_radius = CornerRadius::new(12.0, 20.0, 28.0, 36.0).unwrap();
    let mut bar = frame(80.0, 24.0, 10.0, 12.0, [1.0, 0.0, 0.0, 1.0]);
    bar.border = Some(Border::new(color(ColorSpace::Srgb, [0.0, 0.0, 0.0, 1.0]), 1.0).unwrap());
    bar.clipping = Clipping::Disabled;
    let mut second = frame(240.0, 160.0, 0.0, 0.0, [1.0, 1.0, 1.0, 1.0]);
    second.transform = Transform2D::new(1.25, 0.0, 0.0, 1.5, -500.0, 800.0).unwrap();
    second.clipping = Clipping::Disabled;
    second.border = Some(Border::new(color(ColorSpace::Srgb, [0.0, 0.0, 0.0, 1.0]), 2.0).unwrap());
    let mut rotated = frame(40.0, 20.0, 0.0, 0.0, [0.2, 0.4, 0.8, 1.0]);
    rotated.transform = Transform2D::new(0.0, 1.0, -1.0, 0.0, 100.0, 25.0).unwrap();
    let mut mirrored = frame(60.0, 30.0, 0.0, 0.0, [0.1, 0.8, 0.5, 1.0]);
    mirrored.transform = Transform2D::new(-1.0, 0.0, 0.0, 1.0, 210.0, 100.0).unwrap();
    let mut sheared = frame(40.0, 24.0, 0.0, 0.0, [0.4, 0.2, 0.9, 0.5]);
    sheared.background = Some(color(ColorSpace::DisplayP3, [1.2, -0.1, 0.3, 0.25]));
    sheared.transform = Transform2D::new(1.0, 0.25, 0.5, 1.0, 20.0, 100.0).unwrap();
    let mut zoomed = frame(4.0, 3.0, 0.0, 0.0, [1.0; 4]);
    zoomed.transform = Transform2D::new(128.0, 0.0, 0.0, 128.0, 700.0, -900.0).unwrap();
    zoomed.border = Some(Border::new(color(ColorSpace::Srgb, [0.0, 0.0, 0.0, 1.0]), 1.0).unwrap());
    let mut subpixel = frame(2.0, 1.0, 0.0, 0.0, [0.0, 1.0, 0.0, 1.0]);
    subpixel.transform = Transform2D::new(0.0, 1.0, -1.0, 0.0, 1.005, -0.005).unwrap();
    let f = NodePayload::Frame;
    for (n, parent, order, visible, payload) in [
        (10, None, 1, true, f(root)),
        (
            13,
            Some(10),
            2,
            true,
            f(frame(100.0, 40.0, 63.0, 72.0, [0.0, 1.0, 0.0, 1.0])),
        ),
        (
            11,
            Some(10),
            1,
            true,
            NodePayload::Fragment(FragmentPayload),
        ),
        (
            14,
            Some(11),
            1,
            true,
            NodePayload::Fragment(FragmentPayload),
        ),
        (12, Some(14), 1, true, f(card)),
        (15, Some(12), 1, true, f(bar)),
        (
            16,
            Some(10),
            3,
            false,
            NodePayload::Fragment(FragmentPayload),
        ),
        (
            17,
            Some(16),
            1,
            true,
            f(frame(100.0, 40.0, 300.0, 50.0, [0.0, 0.0, 0.0, 1.0])),
        ),
        (
            18,
            Some(10),
            4,
            true,
            f(frame(8.0, 8.0, 0.0, 100.0, [0.0, 1.0, 1.0, 1.0])),
        ),
        (
            19,
            Some(10),
            5,
            true,
            f(frame(30.0, 40.0, 630.0, 300.0, [0.0, 0.0, 1.0, 1.0])),
        ),
        (20, None, 2, true, f(second)),
        (21, Some(20), 1, true, f(rotated)),
        (22, Some(20), 2, true, f(mirrored)),
        (23, Some(20), 3, true, f(sheared)),
        (30, None, 3, true, f(zoomed)),
        (31, Some(30), 1, true, f(subpixel)),
    ] {
        model
            .insert_node(NodeRecord {
                id: id(n),
                parent: parent.map(id),
                order_key: OrderKey::from_bytes(vec![order]).unwrap(),
                visible,
                locked: false,
                display_name: "button <script>는 의미나 코드가 아님".into(),
                payload,
            })
            .unwrap();
    }
    model
}

/// View B의 node 21만 미지원으로 바꿉니다. 다른 View는 계속 내보낼 수 있어야 합니다.
pub fn unsupported(case: &str) -> DocumentModel {
    let mut model = document();
    let NodePayload::Frame(mut frame) = model.payload(id(21)).unwrap().clone() else {
        unreachable!()
    };
    match case {
        "layout" => frame.layout_mode = Some(id(99)),
        "clip" => frame.clipping = Clipping::Path(NodeRef::new(id(99))),
        "fractional-border" => {
            frame.border = Some(Border::new(color(ColorSpace::Srgb, [1.0; 4]), 0.5).unwrap())
        }
        "oversize-border" => {
            frame.border = Some(Border::new(color(ColorSpace::Srgb, [1.0; 4]), 11.0).unwrap())
        }
        "range" => frame.size = Size2::new(2_000_000.0, 20.0).unwrap(),
        "amplified-size-rounding" => {
            frame.size = Size2::new(10.005, 2.0).unwrap();
            frame.transform = Transform2D::new(0.0, 128.0, -128.0, 0.0, 100.0, 25.0).unwrap();
        }
        "matrix-range" => {
            frame.transform = Transform2D::new(1025.0, 0.0, 0.0, 1.0, 0.0, 0.0).unwrap();
        }
        "world-range" => {
            frame.transform = Transform2D::new(1.0, 0.0, 0.0, 1.0, 900_000.0, 0.0).unwrap();
        }
        "modifier" => {
            let mut stack = ModifierStack::new(id(21));
            stack
                .push_back(ModifierId::new(id(90)), ModifierKind::Opacity)
                .unwrap();
            model.set_modifier_stack(stack).unwrap();
        }
        "text" | "hidden-text" => {
            let mut record = model.remove_leaf(id(21)).unwrap();
            record.payload = NodePayload::Text(TextPayload::empty(0));
            record.visible = case != "hidden-text";
            model.insert_node(record).unwrap();
            return model;
        }
        _ => panic!("unknown fixture {case}"),
    }
    model
        .set_payload(id(21), NodePayload::Frame(frame))
        .unwrap();
    model
}

/// 실제 저장·CLI Rectangle 검증. 기본 Frame fixture의 leaf만 교체합니다.
pub fn rectangle(case: &str) -> DocumentModel {
    use stellar_document::modifier::ModifierPayload;
    use stellar_document::shape::{RectangleParams, ShapeGeometry};
    let mut model = document();
    let mut record = model.remove_leaf(id(21)).unwrap();
    record.payload = NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
        size: Size2::new(40.0, 20.0).unwrap(),
        corner_radius: if case == "square" {
            CornerRadius::ZERO
        } else {
            CornerRadius::new(40.0, 20.0, 10.0, 0.0).unwrap()
        },
    }));
    record.visible = case != "hidden-unsupported";
    model.insert_node(record).unwrap();
    let mut stack = ModifierStack::new(id(21));
    let mut payloads = vec![
        ModifierPayload::Transform(Transform2D::new(1., 0., 0., 1., 10., 12.).unwrap()),
        ModifierPayload::Transform(Transform2D::new(0., 1., -1., 0., 100., 25.).unwrap()),
        ModifierPayload::Fill {
            color: color(ColorSpace::DisplayP3, [1.2, -0.1, 0.3, 0.25]),
        },
    ];
    match case {
        "supported" | "disabled" | "no-stack" | "square" => {}
        "after-fill" => payloads.swap(1, 2),
        "double-fill" => payloads.push(payloads[2].clone()),
        "unsupported" | "hidden-unsupported" => {
            payloads.push(ModifierPayload::default_for(ModifierKind::Opacity))
        }
        "singular" => {
            payloads[0] =
                ModifierPayload::Transform(Transform2D::new(0., 0., 0., 1., 0., 0.).unwrap())
        }
        "range" => {
            payloads[0] = ModifierPayload::Transform(
                Transform2D::new(1., 0., 0., 1., 0., 2_000_000.).unwrap(),
            )
        }
        _ => panic!("unknown Rectangle fixture {case}"),
    }
    if case == "disabled" {
        payloads.push(ModifierPayload::default_for(ModifierKind::Blur));
    }
    for (index, payload) in payloads.into_iter().enumerate() {
        let modifier = ModifierId::new(id(90 + index as u64));
        stack.push_back(modifier, payload.kind()).unwrap();
        stack.set_payload(modifier, payload).unwrap();
        if case == "disabled" && index == 3 {
            stack.set_enabled(modifier, false).unwrap();
        }
    }
    if case != "no-stack" {
        model.set_modifier_stack(stack).unwrap();
    }
    model
}

/// 하나의 정의를 두 좌표에 배치한 View. 둘째 인스턴스는 편집 잠금 상태입니다.
pub fn placed_components(case: &str) -> DocumentModel {
    use crate::core_types::reference::ComponentRef;
    use stellar_document::component::instance::ComponentInstancePayload;
    use stellar_document::modifier::ModifierPayload;
    let mut model = document();
    for (n, x, y) in [(21, 30.0, 20.0), (22, 230.5, -10.0)] {
        let mut node = model.remove_leaf(id(n)).unwrap();
        let mut payload = ComponentInstancePayload::new(ComponentRef::new(id(70)));
        if case == "override" {
            use stellar_document::component::{
                definition::ComponentPropertyId, instance::OverrideKey,
            };
            payload.set_property_override(
                OverrideKey::direct(ComponentPropertyId::new(id(89))),
                crate::core_types::typed_value::TypedValue::Boolean(true),
            );
        }
        node.payload = NodePayload::ComponentInstance(payload);
        node.locked = n == 22;
        node.visible = n != 21 || case != "hidden";
        model.insert_node(node).unwrap();
        let mut stack = ModifierStack::new(id(n));
        let modifier = ModifierId::new(id(n + 90));
        let kind = if case == "modifier" {
            ModifierKind::Opacity
        } else {
            ModifierKind::Transform
        };
        stack.push_back(modifier, kind).unwrap();
        if kind == ModifierKind::Transform {
            stack
                .set_payload(
                    modifier,
                    ModifierPayload::Transform(
                        Transform2D::new(
                            if case == "singular" { 0. } else { 1. },
                            0.,
                            0.,
                            1.,
                            x,
                            y,
                        )
                        .unwrap(),
                    ),
                )
                .unwrap();
        }
        model.set_modifier_stack(stack).unwrap();
    }
    model
}
