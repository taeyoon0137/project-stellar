//! 문서 I/O 통합 테스트 공용 helper.
#![allow(dead_code)]

pub mod golden;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::{RuntimeEntityId, StableId};
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::{AssetRef, NodeRef};
use stellar_core_types::text::UserText;
use stellar_core_types::value::{Rect, Size2, Transform2D, Vec2};
use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::fragment::FragmentPayload;
use stellar_document::frame::{Border, Clipping, CornerRadius, FramePayload};
use stellar_document::image::ImagePayload;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{
    BooleanOp, ModifierId, ModifierKind, ModifierPayload, ModifierStack,
};
use stellar_document::path::{IdentifiedPath, PathCommand, PathData, WindingRule};
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_document::shape::{EllipseParams, LineCap, LineParams, RectangleParams, ShapeGeometry};
use stellar_document::text::{Paragraph, StyleRun, TextPayload, TextRange};

/// 임시 디렉터리(drop 시 삭제).
pub struct TempDir {
    pub path: PathBuf,
}

impl TempDir {
    pub fn new(prefix: &str) -> TempDir {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// 문서 `doc`의 n번째 id. 문서마다 timestamp 대역을 나눠 문서 간 id가 겹치지 않습니다.
pub fn id(doc: u64, n: u64) -> StableId {
    StableId::new_v7(doc * 1_000_000 + n, 0x0abc, 0x0123_4567_89ab ^ n)
}

pub fn key(byte: u8) -> OrderKey {
    OrderKey::from_bytes(vec![byte]).unwrap()
}

pub fn node(
    doc: u64,
    n: u64,
    parent: Option<u64>,
    order: u8,
    name: &str,
    payload: NodePayload,
) -> NodeRecord {
    NodeRecord {
        id: id(doc, n),
        parent: parent.map(|p| id(doc, p)),
        order_key: key(order),
        visible: !n.is_multiple_of(5),
        locked: n.is_multiple_of(7),
        display_name: name.to_string(),
        payload,
    }
}

fn color(r: f32, g: f32, b: f32) -> Color {
    Color::new(ColorSpace::Srgb, r, g, b, 1.0).unwrap()
}

/// `RuntimeEntityId`의 (slot, generation). 공개 접근자가 없어 Debug 출력에서 읽습니다
/// (테스트 전용).
pub fn runtime_parts(handle: RuntimeEntityId) -> (u32, u32) {
    let text = format!("{handle:?}");
    let field = |name: &str| -> u32 {
        let start = text.find(name).unwrap() + name.len();
        text[start..]
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .unwrap()
    };
    (field("slot"), field("generation"))
}

/// `record`를 넣기 전에 `churn`번 dummy insert/remove를 반복해 그 slot의 generation을
/// `churn`으로 올립니다. 반환값은 실제 node의 runtime 핸들입니다.
pub fn insert_churned(
    model: &mut DocumentModel,
    record: NodeRecord,
    churn: u32,
) -> RuntimeEntityId {
    let dummy = StableId::new_v7(999_999_999, 0x0fff, 0xdead);
    for _ in 0..churn {
        model
            .insert_node(NodeRecord {
                id: dummy,
                parent: None,
                order_key: key(0x01),
                visible: true,
                locked: false,
                display_name: String::new(),
                payload: NodePayload::Fragment(FragmentPayload),
            })
            .unwrap();
        model.remove_leaf(dummy).unwrap();
    }
    model.insert_node(record).unwrap()
}

/// 모든 node 종류·modifier 참조·clip 참조·다국어 텍스트를 담은 문서의 node 목록(부모 먼저).
pub fn rich_nodes(doc: u64) -> Vec<NodeRecord> {
    let mut next = 900u64;
    let path = IdentifiedPath::assign(
        PathData::new(
            WindingRule::EvenOdd,
            vec![
                PathCommand::MoveTo(Vec2::ZERO),
                PathCommand::CubicTo {
                    c1: Vec2::new(10.0, 0.0).unwrap(),
                    c2: Vec2::new(10.0, 10.0).unwrap(),
                    to: Vec2::new(0.0, 10.0).unwrap(),
                },
                PathCommand::Close,
            ],
        )
        .unwrap(),
        &mut || {
            next += 1;
            id(doc, next)
        },
    );
    let text = TextPayload::new(
        UserText::new("안녕하세요\n👩‍👩‍👧 emoji"),
        vec![Paragraph { style: 0 }, Paragraph { style: 1 }],
        vec![StyleRun {
            range: TextRange::new(0, 5).unwrap(),
            style: 2,
        }],
    )
    .unwrap();
    let card = FramePayload {
        size: Size2::new(320.0, 200.0).unwrap(),
        transform: Transform2D::new(1.0, 0.0, 0.0, 1.0, 24.0, 48.0).unwrap(),
        clipping: Clipping::Path(NodeRef::new(id(doc, 14))),
        background: Some(color(1.0, 1.0, 1.0)),
        border: Some(Border::new(color(0.2, 0.2, 0.2), 1.5).unwrap()),
        corner_radius: CornerRadius::new(8.0, 8.0, 8.0, 8.0).unwrap(),
        layout_mode: None,
    };
    vec![
        // View A.
        node(
            doc,
            10,
            None,
            0x10,
            "홈",
            NodePayload::Frame(FramePayload::default()),
        ),
        node(doc, 11, Some(10), 0x10, "카드", NodePayload::Frame(card)),
        node(
            doc,
            12,
            Some(11),
            0x10,
            "그룹",
            NodePayload::Fragment(FragmentPayload),
        ),
        node(
            doc,
            13,
            Some(12),
            0x10,
            "배경",
            NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
                size: Size2::new(100.0, 40.0).unwrap(),
                corner_radius: CornerRadius::new(4.0, 4.0, 0.0, 0.0).unwrap(),
            })),
        ),
        node(doc, 14, Some(11), 0x20, "곡선", NodePayload::Path(path)),
        node(doc, 15, Some(11), 0x30, "제목", NodePayload::Text(text)),
        node(
            doc,
            16,
            Some(10),
            0x20,
            "사진",
            NodePayload::Image(ImagePayload::new(
                AssetRef::new(id(doc, 5_000)),
                Size2::new(64.0, 64.0).unwrap(),
            )),
        ),
        node(
            doc,
            17,
            Some(12),
            0x20,
            "",
            NodePayload::Shape(ShapeGeometry::Ellipse(EllipseParams {
                bounds: Rect::new(Vec2::new(-2.0, 3.5).unwrap(), Size2::new(9.0, 9.0).unwrap()),
            })),
        ),
        // View B.
        node(
            doc,
            20,
            None,
            0x10,
            "설정",
            NodePayload::Frame(FramePayload::default()),
        ),
        node(
            doc,
            21,
            Some(20),
            0x10,
            "구분선",
            NodePayload::Shape(ShapeGeometry::Line(LineParams {
                start: Vec2::ZERO,
                end: Vec2::new(300.0, 0.0).unwrap(),
                cap: LineCap::Round,
            })),
        ),
        node(
            doc,
            22,
            Some(20),
            0x20,
            "빈 텍스트",
            NodePayload::Text(TextPayload::empty(3)),
        ),
        // View C(다른 페이지).
        node(
            doc,
            30,
            None,
            0x10,
            "빈 화면",
            NodePayload::Frame(FramePayload::default()),
        ),
    ]
}

/// rich 문서 정의: 페이지 2개, View 3개.
pub fn rich_definition(doc: u64) -> DocumentDefinition {
    let mut definition = DocumentDefinition::new(id(doc, 1), format!("문서 {doc}"));
    let mut first = PageDefinition::new(id(doc, 2), "페이지 1".to_string());
    first
        .add_view(ViewDefinition::new(id(doc, 3), "홈".into(), id(doc, 10)))
        .unwrap();
    first
        .add_view(ViewDefinition::new(id(doc, 4), String::new(), id(doc, 20)))
        .unwrap();
    definition.add_page(first).unwrap();
    let mut second = PageDefinition::new(id(doc, 5), String::new());
    second
        .add_view(ViewDefinition::new(id(doc, 6), "빈".into(), id(doc, 30)))
        .unwrap();
    definition.add_page(second).unwrap();
    definition
}

/// rich 문서의 modifier stack들.
pub fn rich_stacks(doc: u64) -> Vec<ModifierStack> {
    let mid = |n: u64| ModifierId::new(id(doc, n));
    let mut bg = ModifierStack::new(id(doc, 13));
    bg.push_back(mid(100), ModifierKind::Fill).unwrap();
    bg.push_back(mid(101), ModifierKind::Mask).unwrap();
    bg.set_payload(
        mid(101),
        ModifierPayload::Mask {
            source: NodeRef::new(id(doc, 17)),
        },
    )
    .unwrap();
    bg.push_back(mid(102), ModifierKind::Boolean).unwrap();
    bg.set_payload(
        mid(102),
        ModifierPayload::Boolean {
            op: BooleanOp::Exclude,
            operands: vec![NodeRef::new(id(doc, 14)), NodeRef::new(id(doc, 9_999))],
        },
    )
    .unwrap();
    bg.set_enabled(mid(100), false).unwrap();
    let mut card = ModifierStack::new(id(doc, 11));
    card.push_back(mid(103), ModifierKind::Shadow).unwrap();
    card.push_back(mid(104), ModifierKind::Opacity).unwrap();
    vec![bg, card]
}

/// rich 문서 모델. `churn`이 0보다 크면 node마다 runtime generation을 그만큼 올립니다.
pub fn rich_model(doc: u64, churn: u32) -> DocumentModel {
    let mut model = DocumentModel::new(rich_definition(doc));
    for record in rich_nodes(doc) {
        insert_churned(&mut model, record, churn);
    }
    for stack in rich_stacks(doc) {
        model.set_modifier_stack(stack).unwrap();
    }
    model
}

/// `haystack`에 `needle`이 연속 bytes로 있는지.
pub fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}
