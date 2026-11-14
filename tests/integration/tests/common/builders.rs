//! node 레코드·modifier 항목 생성 helper(시나리오 문서 구성용).

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::AssetRef;
use stellar_core_types::text::UserText;
use stellar_core_types::value::{Rect, Size2, Transform2D, Vec2};
use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::fragment::FragmentPayload;
use stellar_document::frame::{Border, CornerRadius, FramePayload};
use stellar_document::image::ImagePayload;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{ModifierEntry, ModifierId, ModifierKind, ModifierPayload};
use stellar_document::path::{IdentifiedPath, PathCommand, PathData, WindingRule};
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_document::shape::{EllipseParams, LineCap, LineParams, RectangleParams, ShapeGeometry};
use stellar_document::text::{Paragraph, StyleRun, TextPayload, TextRange};

/// sRGB 불투명 색.
pub fn color(r: f32, g: f32, b: f32) -> Color {
    Color::new(ColorSpace::Srgb, r, g, b, 1.0).expect("유효 색")
}

fn record(
    id: StableId,
    parent: StableId,
    order: OrderKey,
    name: &str,
    payload: NodePayload,
) -> NodeRecord {
    NodeRecord {
        id,
        parent: Some(parent),
        order_key: order,
        visible: true,
        locked: false,
        display_name: name.to_string(),
        payload,
    }
}

/// View root Frame 레코드(부모 없음).
pub fn root_frame(id: StableId, name: &str, width: f32, height: f32) -> NodeRecord {
    NodeRecord {
        id,
        parent: None,
        order_key: OrderKey::between(None, None),
        visible: true,
        locked: false,
        display_name: name.to_string(),
        payload: NodePayload::Frame(FramePayload {
            size: Size2::new(width, height).expect("크기"),
            ..FramePayload::default()
        }),
    }
}

/// 위치·크기·배경이 있는 Frame.
pub fn frame(
    id: StableId,
    parent: StableId,
    order: OrderKey,
    name: &str,
    origin: (f32, f32),
    size: (f32, f32),
) -> NodeRecord {
    record(
        id,
        parent,
        order,
        name,
        NodePayload::Frame(FramePayload {
            size: Size2::new(size.0, size.1).expect("크기"),
            transform: Transform2D::new(1.0, 0.0, 0.0, 1.0, origin.0, origin.1).expect("변환"),
            background: Some(color(1.0, 1.0, 1.0)),
            border: Some(Border::new(color(0.0, 0.0, 0.0), 1.0).expect("테두리")),
            corner_radius: CornerRadius::new(8.0, 8.0, 8.0, 8.0).expect("radius"),
            ..FramePayload::default()
        }),
    )
}

/// 다중 paragraph·style run을 가진 Text.
pub fn text(
    id: StableId,
    parent: StableId,
    order: OrderKey,
    name: &str,
    content: &str,
) -> NodeRecord {
    let paragraphs = (0..=content.matches('\n').count())
        .map(|i| Paragraph { style: i as u32 })
        .collect();
    let len = content.chars().count() as u32;
    let runs = if len >= 2 {
        vec![StyleRun {
            range: TextRange::new(0, len / 2).expect("범위"),
            style: 7,
        }]
    } else {
        Vec::new()
    };
    let payload = TextPayload::new(UserText::new(content), paragraphs, runs).expect("텍스트");
    record(id, parent, order, name, NodePayload::Text(payload))
}

/// 모서리가 둥근 사각형 Shape.
pub fn rectangle(
    id: StableId,
    parent: StableId,
    order: OrderKey,
    name: &str,
    w: f32,
    h: f32,
) -> NodeRecord {
    record(
        id,
        parent,
        order,
        name,
        NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
            size: Size2::new(w, h).expect("크기"),
            corner_radius: CornerRadius::new(4.0, 4.0, 0.0, 0.0).expect("radius"),
        })),
    )
}

/// 타원 Shape.
pub fn ellipse(
    id: StableId,
    parent: StableId,
    order: OrderKey,
    name: &str,
    origin: (f32, f32),
    d: f32,
) -> NodeRecord {
    record(
        id,
        parent,
        order,
        name,
        NodePayload::Shape(ShapeGeometry::Ellipse(EllipseParams {
            bounds: Rect::new(
                Vec2::new(origin.0, origin.1).expect("위치"),
                Size2::new(d, d).expect("크기"),
            ),
        })),
    )
}

/// 직선 Shape.
pub fn line(
    id: StableId,
    parent: StableId,
    order: OrderKey,
    name: &str,
    length: f32,
) -> NodeRecord {
    record(
        id,
        parent,
        order,
        name,
        NodePayload::Shape(ShapeGeometry::Line(LineParams {
            start: Vec2::new(2.0, 2.0).expect("시작"),
            end: Vec2::new(2.0 + length, 2.0).expect("끝"),
            cap: LineCap::Round,
        })),
    )
}

/// 이미지(asset 참조).
pub fn image(
    id: StableId,
    parent: StableId,
    order: OrderKey,
    name: &str,
    asset: StableId,
) -> NodeRecord {
    record(
        id,
        parent,
        order,
        name,
        NodePayload::Image(ImagePayload::new(
            AssetRef::new(asset),
            Size2::new(64.0, 48.0).expect("크기"),
        )),
    )
}

/// 닫힌 곡선 Path. point·handle id는 `next_id`가 공급합니다.
pub fn path(
    id: StableId,
    parent: StableId,
    order: OrderKey,
    name: &str,
    next_id: &mut dyn FnMut() -> StableId,
) -> NodeRecord {
    let data = PathData::new(
        WindingRule::NonZero,
        vec![
            PathCommand::MoveTo(Vec2::ZERO),
            PathCommand::LineTo(Vec2::new(40.0, 0.0).expect("점")),
            PathCommand::CubicTo {
                c1: Vec2::new(48.0, 8.0).expect("점"),
                c2: Vec2::new(48.0, 24.0).expect("점"),
                to: Vec2::new(40.0, 32.0).expect("점"),
            },
            PathCommand::Close,
        ],
    )
    .expect("path");
    record(
        id,
        parent,
        order,
        name,
        NodePayload::Path(IdentifiedPath::assign(data, next_id)),
    )
}

/// 자식 순서만 정하는 Fragment.
pub fn fragment(id: StableId, parent: StableId, order: OrderKey, name: &str) -> NodeRecord {
    record(
        id,
        parent,
        order,
        name,
        NodePayload::Fragment(FragmentPayload),
    )
}

/// modifier 항목.
pub fn modifier(id: StableId, order: OrderKey, payload: ModifierPayload) -> ModifierEntry {
    ModifierEntry {
        id: ModifierId::new(id),
        order,
        kind: payload.kind(),
        enabled: true,
        payload,
    }
}

/// 종류 기본값 modifier 항목.
pub fn default_modifier(id: StableId, order: OrderKey, kind: ModifierKind) -> ModifierEntry {
    modifier(id, order, ModifierPayload::default_for(kind))
}

/// View 하나: (View id, 이름, root Frame id).
pub type ViewSpec<'a> = (StableId, &'a str, StableId);

/// 페이지 하나: (페이지 id, 이름, View 목록).
pub type PageSpec<'a> = (StableId, &'a str, Vec<ViewSpec<'a>>);

/// `pages`로 정의를 만들고 각 View root Frame을 넣은 새 문서.
///
/// View root Frame은 문서 정의와 함께 만들어지는 템플릿 단계입니다(InsertNode command는 부모 없는
/// node를 받지 않음). 그 아래 내용은 모두 command로 만듭니다.
pub fn new_document(document: StableId, name: &str, pages: &[PageSpec<'_>]) -> DocumentModel {
    let mut definition = DocumentDefinition::new(document, name.to_string());
    for (page_id, page_name, views) in pages {
        let mut page = PageDefinition::new(*page_id, page_name.to_string());
        for (view_id, view_name, root) in views {
            page.add_view(ViewDefinition::new(*view_id, view_name.to_string(), *root))
                .expect("View 추가");
        }
        definition.add_page(page).expect("페이지 추가");
    }
    let mut model = DocumentModel::new(definition);
    for (_, _, views) in pages {
        for (_, view_name, root) in views {
            model
                .insert_node(root_frame(*root, view_name, 1440.0, 900.0))
                .expect("View root 삽입");
        }
    }
    model
}
