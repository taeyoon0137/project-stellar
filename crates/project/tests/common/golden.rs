//! VI.10.3 golden document 생성기(index → 고정 문서).
//!
//! 파일로 대량 저장하지 않고 index에서 결정론적으로 만듭니다. 정수 연산과 정확히 표현되는
//! f32(분모가 2의 거듭제곱)만 써서 플랫폼과 무관하게 같은 문서가 나옵니다.

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::{AssetRef, NodeRef};
use stellar_core_types::text::UserText;
use stellar_core_types::value::{Angle, Rect, Size2, Transform2D, Vec2};
use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::fragment::FragmentPayload;
use stellar_document::frame::{Border, Clipping, CornerRadius, FramePayload};
use stellar_document::image::{CropRect, FitMode, ImagePayload, SamplingMode};
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{
    BlendMode, BooleanOp, ModifierId, ModifierKind, ModifierPayload, ModifierStack,
};
use stellar_document::path::{IdentifiedPath, PathCommand, PathData, WindingRule};
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_document::shape::{
    EllipseParams, LineCap, LineParams, PolygonParams, RectangleParams, ShapeGeometry,
};
use stellar_document::text::{Paragraph, StyleRun, TextPayload, TextRange};
use stellar_document::validate::MAX_NODE_DEPTH;

use super::id;

const NAMES: [&str; 6] = [
    "",
    "버튼",
    "카드 🎴",
    "따옴표 \"x\"",
    "탭\t이름",
    "line\nbreak",
];
const CONTENTS: [&str; 5] = ["", "안녕", "첫 줄\n둘째 줄", "👩‍💻 코드", "a\nb\nc"];

/// 정확히 표현되는 f32(1/4 단위).
fn quarter(seed: u64, range: u64) -> f32 {
    (seed % range) as f32 / 4.0
}

fn signed_quarter(seed: u64, range: u64) -> f32 {
    quarter(seed, range) - (range / 8) as f32
}

fn color(seed: u64) -> Color {
    let c = |shift: u64| ((seed >> shift) % 5) as f32 / 4.0;
    let space = if seed.is_multiple_of(2) {
        ColorSpace::Srgb
    } else {
        ColorSpace::DisplayP3
    };
    Color::new(space, c(0), c(3), c(6), c(9)).unwrap()
}

struct Builder {
    doc: u64,
    seed: u64,
    model: DocumentModel,
    /// 생성 순서대로의 node id(modifier 참조는 더 먼저 만든 node만 가리켜 cycle이 없음).
    created: Vec<StableId>,
    paths: Vec<StableId>,
    element: u64,
    modifier: u64,
}

impl Builder {
    fn next_element(&mut self) -> StableId {
        self.element += 1;
        id(self.doc, 500_000 + self.element)
    }

    fn payload(&mut self, kind: u64, k: u64) -> NodePayload {
        let s = self.seed * 31 + k * 17;
        match kind {
            0 => {
                let clipping = match s % 6 {
                    0 if !self.paths.is_empty() => {
                        Clipping::Path(NodeRef::new(self.paths[(s as usize) % self.paths.len()]))
                    }
                    1 => Clipping::Path(NodeRef::new(id(self.doc, 900_000 + k))), // 미해결
                    2 | 3 => Clipping::Disabled,
                    _ => Clipping::FrameBounds,
                };
                NodePayload::Frame(FramePayload {
                    size: Size2::new(quarter(s, 2_000), quarter(s >> 3, 2_000)).unwrap(),
                    transform: Transform2D::new(
                        if s.is_multiple_of(4) { -1.0 } else { 1.0 },
                        0.0,
                        0.0,
                        1.0,
                        signed_quarter(s, 800),
                        signed_quarter(s >> 2, 800),
                    )
                    .unwrap(),
                    clipping,
                    background: (!s.is_multiple_of(3)).then(|| color(s)),
                    border: (s % 4 == 1)
                        .then(|| Border::new(color(s >> 1), quarter(s, 16)).unwrap()),
                    corner_radius: CornerRadius::new(
                        quarter(s, 40),
                        quarter(s >> 1, 40),
                        quarter(s >> 2, 40),
                        quarter(s >> 3, 40),
                    )
                    .unwrap(),
                    layout_mode: (s % 5 == 2).then(|| id(self.doc, 800_000 + k)),
                })
            }
            1 => NodePayload::Fragment(FragmentPayload),
            2 => NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
                size: Size2::new(quarter(s, 400), quarter(s >> 2, 400)).unwrap(),
                corner_radius: CornerRadius::new(quarter(s, 400), 0.0, quarter(s >> 1, 8), 1.0)
                    .unwrap(),
            })),
            3 => NodePayload::Shape(ShapeGeometry::Ellipse(EllipseParams {
                bounds: Rect::new(
                    Vec2::new(signed_quarter(s, 100), signed_quarter(s >> 1, 100)).unwrap(),
                    Size2::new(quarter(s, 300), quarter(s >> 3, 300)).unwrap(),
                ),
            })),
            4 => NodePayload::Shape(ShapeGeometry::Line(LineParams {
                start: Vec2::new(signed_quarter(s, 64), 0.0).unwrap(),
                end: Vec2::new(signed_quarter(s >> 2, 64), signed_quarter(s >> 4, 64)).unwrap(),
                cap: match s % 3 {
                    0 => LineCap::Butt,
                    1 => LineCap::Round,
                    _ => LineCap::Square,
                },
            })),
            5 => NodePayload::Shape(ShapeGeometry::Polygon(
                PolygonParams::new(
                    Rect::new(
                        Vec2::ZERO,
                        Size2::new(quarter(s, 200), quarter(s, 200)).unwrap(),
                    ),
                    3 + (s % 10) as u32,
                    Angle::new(signed_quarter(s, 1_440)).unwrap(),
                    quarter(s >> 1, 20),
                )
                .unwrap(),
            )),
            6 => {
                let mut commands = vec![PathCommand::MoveTo(Vec2::ZERO)];
                for j in 0..(1 + s % 4) {
                    let p = Vec2::new(quarter(s + j, 80), quarter(s * 3 + j, 80)).unwrap();
                    if (s + j).is_multiple_of(2) {
                        commands.push(PathCommand::LineTo(p));
                    } else {
                        commands.push(PathCommand::CubicTo {
                            c1: p,
                            c2: Vec2::ZERO,
                            to: p,
                        });
                    }
                }
                if s.is_multiple_of(3) {
                    commands.push(PathCommand::Close);
                }
                let winding = if s.is_multiple_of(2) {
                    WindingRule::NonZero
                } else {
                    WindingRule::EvenOdd
                };
                let data = PathData::new(winding, commands).unwrap();
                let path = IdentifiedPath::assign(data, &mut || self.next_element());
                NodePayload::Path(path)
            }
            7 => {
                let content = CONTENTS[(s % CONTENTS.len() as u64) as usize];
                let paragraphs = (0..=content.matches('\n').count())
                    .map(|p| Paragraph { style: p as u32 })
                    .collect();
                let len = content.chars().count() as u32;
                let runs = if len >= 2 {
                    vec![StyleRun {
                        range: TextRange::new(0, len / 2).unwrap(),
                        style: (s % 4) as u32,
                    }]
                } else {
                    Vec::new()
                };
                NodePayload::Text(
                    TextPayload::new(UserText::new(content), paragraphs, runs).unwrap(),
                )
            }
            _ => {
                let mut image = ImagePayload::new(
                    AssetRef::new(id(self.doc, 600_000 + s % 7)),
                    Size2::new(quarter(s, 800), quarter(s >> 1, 800)).unwrap(),
                );
                image.fit = match s % 4 {
                    0 => FitMode::Fill,
                    1 => FitMode::Contain,
                    2 => FitMode::Cover,
                    _ => FitMode::None,
                };
                if s.is_multiple_of(3) {
                    image.crop = CropRect::from_components(0.25, 0.25, 0.5, 0.5).unwrap();
                }
                image.sampling = if s.is_multiple_of(2) {
                    SamplingMode::Linear
                } else {
                    SamplingMode::Nearest
                };
                image.color_space_override = match s % 3 {
                    0 => None,
                    1 => Some(ColorSpace::Srgb),
                    _ => Some(ColorSpace::DisplayP3),
                };
                NodePayload::Image(image)
            }
        }
    }

    fn insert(&mut self, record: NodeRecord) {
        let is_path = matches!(record.payload, NodePayload::Path(_));
        let node = record.id;
        self.model.insert_node(record).unwrap();
        self.created.push(node);
        if is_path {
            self.paths.push(node);
        }
    }

    fn modifiers_for(&mut self, owner: StableId, k: u64) {
        let s = self.seed * 7 + k * 13;
        let count = 1 + s % 3;
        let owner_index = self.created.iter().position(|n| *n == owner).unwrap();
        let mut stack = ModifierStack::new(owner);
        for j in 0..count {
            self.modifier += 1;
            let mid = ModifierId::new(id(self.doc, 700_000 + self.modifier));
            let kind = ModifierKind::ALL[((s + j * 5) % 12) as usize];
            stack.push_back(mid, kind).unwrap();
            // 더 먼저 만든 node만 참조(owner_index 미만) → modifier 참조 graph는 DAG.
            let earlier = |offset: u64| -> NodeRef {
                if owner_index == 0 {
                    NodeRef::new(StableId::NIL)
                } else {
                    NodeRef::new(self.created[(offset as usize) % owner_index])
                }
            };
            let payload = match kind {
                ModifierKind::Transform => ModifierPayload::Transform(
                    Transform2D::new(1.0, 0.25, -0.25, 1.0, quarter(s, 40), 0.0).unwrap(),
                ),
                ModifierKind::Boolean => ModifierPayload::Boolean {
                    op: [
                        BooleanOp::Union,
                        BooleanOp::Subtract,
                        BooleanOp::Intersect,
                        BooleanOp::Exclude,
                    ][(s % 4) as usize],
                    operands: (0..(s % 3)).map(|o| earlier(s + o)).collect(),
                },
                ModifierKind::StrokeExpand => ModifierPayload::StrokeExpand {
                    width: stellar_core_types::value::FiniteF32::new(quarter(s, 20)).unwrap(),
                },
                ModifierKind::Corner => ModifierPayload::Corner {
                    radius: stellar_core_types::value::FiniteF32::new(quarter(s, 40)).unwrap(),
                },
                ModifierKind::Mask => ModifierPayload::Mask { source: earlier(s) },
                ModifierKind::Clip => ModifierPayload::Clip {
                    source: if s.is_multiple_of(5) {
                        NodeRef::new(id(self.doc, 950_000 + k)) // 미해결
                    } else {
                        earlier(s + 1)
                    },
                },
                ModifierKind::Fill => ModifierPayload::Fill { color: color(s) },
                ModifierKind::Stroke => ModifierPayload::Stroke {
                    color: color(s >> 2),
                    width: stellar_core_types::value::FiniteF32::new(quarter(s, 12)).unwrap(),
                },
                ModifierKind::Shadow => ModifierPayload::Shadow {
                    color: color(s >> 1),
                    offset: Vec2::new(signed_quarter(s, 32), signed_quarter(s >> 1, 32)).unwrap(),
                    blur: stellar_core_types::value::FiniteF32::new(quarter(s, 64)).unwrap(),
                },
                ModifierKind::Blur => ModifierPayload::Blur {
                    radius: stellar_core_types::value::FiniteF32::new(quarter(s, 64)).unwrap(),
                },
                ModifierKind::Blend => ModifierPayload::Blend {
                    mode: [
                        BlendMode::Normal,
                        BlendMode::Multiply,
                        BlendMode::Screen,
                        BlendMode::Overlay,
                        BlendMode::Darken,
                        BlendMode::Lighten,
                    ][(s % 6) as usize],
                },
                ModifierKind::Opacity => ModifierPayload::Opacity {
                    value: stellar_core_types::value::FiniteF32::new((s % 5) as f32 / 4.0).unwrap(),
                },
            };
            stack.set_payload(mid, payload).unwrap();
            if (s + j).is_multiple_of(4) {
                stack.set_enabled(mid, false).unwrap();
            }
        }
        self.model.set_modifier_stack(stack).unwrap();
    }
}

/// index `i`의 golden 문서.
pub fn golden_document(i: u64) -> DocumentModel {
    let doc = 100 + i;
    let page_count = 1 + i % 2;
    let views_per_page = 1 + (i / 2) % 3;
    let mut definition = DocumentDefinition::new(id(doc, 1), NAMES[(i % 6) as usize].to_string());
    let mut view_index = 0u64;
    let mut views = Vec::new();
    for p in 0..page_count {
        let mut page = PageDefinition::new(id(doc, 2 + p), NAMES[((i + p) % 6) as usize].into());
        for _ in 0..views_per_page {
            let view = id(doc, 100 + view_index);
            let root = id(doc, 10_000 + view_index * 1_000);
            page.add_view(ViewDefinition::new(
                view,
                NAMES[((i + view_index) % 6) as usize].into(),
                root,
            ))
            .unwrap();
            views.push((view_index, root));
            view_index += 1;
        }
        definition.add_page(page).unwrap();
    }

    let mut b = Builder {
        doc,
        seed: i,
        model: DocumentModel::new(definition),
        created: Vec::new(),
        paths: Vec::new(),
        element: 0,
        modifier: 0,
    };
    for (v, root) in views {
        let mut root_payload = b.payload(0, v * 101);
        if let NodePayload::Frame(frame) = &mut root_payload {
            frame.clipping = Clipping::FrameBounds;
        }
        b.insert(NodeRecord {
            id: root,
            parent: None,
            order_key: OrderKey::between(None, None),
            visible: true,
            locked: false,
            display_name: NAMES[(v % 6) as usize].into(),
            payload: root_payload,
        });
        // (부모 id, 마지막 자식 key, 첫 자식 key).
        let mut containers: Vec<(StableId, Option<OrderKey>, Option<OrderKey>)> =
            vec![(root, None, None)];
        let count = 1 + (i * 7 + v * 3) % 12;
        for k in 1..count {
            let node_id = id(doc, 10_000 + v * 1_000 + k);
            let slot = ((i + k * 3) as usize) % containers.len();
            let (parent, last, first) = containers[slot].clone();
            let order_key = if (i + k).is_multiple_of(4) && first.is_some() {
                OrderKey::between(None, first.as_ref())
            } else {
                OrderKey::between(last.as_ref(), None)
            };
            let new_first = match &first {
                Some(f) if order_key > *f => Some(f.clone()),
                _ => Some(order_key.clone()),
            };
            let new_last = match &last {
                Some(l) if order_key < *l => Some(l.clone()),
                _ => Some(order_key.clone()),
            };
            containers[slot] = (parent, new_last, new_first);
            let kind = (i + k + v) % 9;
            let payload = b.payload(kind, v * 1_000 + k);
            let is_container = matches!(payload, NodePayload::Frame(_) | NodePayload::Fragment(_));
            let is_fragment = matches!(payload, NodePayload::Fragment(_));
            b.insert(NodeRecord {
                id: node_id,
                parent: Some(parent),
                order_key,
                visible: !(i + k).is_multiple_of(5),
                locked: (i + k).is_multiple_of(6),
                display_name: NAMES[((i + k) % 6) as usize].into(),
                payload,
            });
            if is_container {
                containers.push((node_id, None, None));
            }
            if !is_fragment && (i + k).is_multiple_of(3) {
                b.modifiers_for(node_id, v * 1_000 + k);
            }
        }
        // 일부 문서는 최대 depth(64) chain을 가집니다.
        if v == 0 && i % 25 == 24 {
            let mut parent = root;
            for level in 1..=u64::from(MAX_NODE_DEPTH) {
                let node_id = id(doc, 400_000 + level);
                // 첫 chain node는 root의 마지막 자식 뒤, 이후는 외동 자식.
                let order_key = if level == 1 {
                    OrderKey::between(containers[0].1.as_ref(), None)
                } else {
                    OrderKey::between(None, None)
                };
                b.insert(NodeRecord {
                    id: node_id,
                    parent: Some(parent),
                    order_key,
                    visible: true,
                    locked: false,
                    display_name: String::new(),
                    payload: NodePayload::Frame(FramePayload::default()),
                });
                parent = node_id;
            }
        }
    }
    b.model
}
