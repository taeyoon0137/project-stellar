//! seed 기반 random valid document 생성기 (테스트 전용, VI.10.5).
//!
//! 외부 crate 없이 자체 PRNG(SplitMix64)로 **항상 validator(VI.10.1)를 통과하는** 문서를
//! 만듭니다. 같은 seed는 언제나 같은 문서(같은 레코드·같은 runtime 삽입 이력)를 냅니다.
//!
//! 유효성을 구성으로 보장하는 규칙:
//!
//! - id는 문서마다 단조 증가 counter를 timestamp에 넣어 전부 유일하고 NIL이 아닙니다.
//! - node는 부모가 먼저 만들어지고, 부모는 Frame·Fragment 중 depth < 64인 것만 고릅니다.
//! - 형제 OrderKey는 정렬된 형제 key 사이에서 `OrderKey::between`으로 만들어 유일합니다.
//! - modifier 참조(Boolean·Mask·Clip)는 **먼저 만든 node**만 가리켜 참조 graph가 DAG입니다.
//!   일부는 NIL 또는 문서에 없는 id(미해결 진단)입니다. Fragment에는 stack을 두지 않습니다.
//! - Frame clip은 Path node 또는 없는 id만 가리킵니다.
//! - float은 정수/2의 거듭제곱 격자 값이라 정확히 표현되고 플랫폼 무관합니다.

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::{AssetRef, NodeRef};
use stellar_core_types::text::UserText;
use stellar_core_types::value::{Angle, FiniteF32, Rect, Size2, Transform2D, Vec2};

use crate::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use crate::fragment::FragmentPayload;
use crate::frame::{Border, Clipping, CornerRadius, FramePayload};
use crate::image::{CropRect, FitMode, ImagePayload, SamplingMode};
use crate::model::DocumentModel;
use crate::modifier::{
    BlendMode, BooleanOp, ModifierId, ModifierKind, ModifierPayload, ModifierStack,
};
use crate::path::{IdentifiedPath, PathCommand, PathData, WindingRule};
use crate::records::{NodePayload, NodeRecord};
use crate::shape::{
    EllipseParams, LineCap, LineParams, PolygonParams, RectangleParams, ShapeGeometry,
};
use crate::text::{Paragraph, StyleRun, TextPayload, TextRange};
use crate::validate::MAX_NODE_DEPTH;

/// 결정론적 PRNG(SplitMix64).
#[derive(Debug, Clone)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// seed로 만듭니다.
    pub fn new(seed: u64) -> SplitMix64 {
        SplitMix64 { state: seed }
    }

    /// 다음 64비트 값.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// `0..n` 균등 값(`n == 0`이면 0).
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next_u64() % n
        }
    }

    /// 확률 `1/n`으로 true.
    pub fn one_in(&mut self, n: u64) -> bool {
        self.below(n) == 0
    }

    /// 목록에서 하나.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

const NAME_POOL: [&str; 10] = [
    "",
    "버튼",
    "Card",
    "🎨 아이콘",
    "따옴표 \"q\"",
    "역슬래시 \\",
    "탭\t",
    "줄\n바꿈",
    "\u{1}제어",
    "가나다라마바사",
];

const TEXT_CHARS: [char; 14] = [
    'a', 'Z', '0', ' ', '\n', '가', '힣', 'ㄱ', '\u{1100}', '\u{1161}', '😀', '\u{200D}', '👩', '"',
];

struct Generator {
    rng: SplitMix64,
    base: u64,
    counter: u64,
    model: DocumentModel,
    /// 생성 순서의 node id.
    created: Vec<StableId>,
    paths: Vec<StableId>,
}

impl Generator {
    fn next_id(&mut self) -> StableId {
        self.counter += 1;
        let rand_a = (self.rng.next_u64() & 0x0FFF) as u16;
        let rand_b = self.rng.next_u64();
        StableId::new_v7(self.base + self.counter, rand_a, rand_b)
    }

    /// 문서에 없는 id(미해결 참조용). counter 대역 밖 timestamp를 씁니다.
    fn dangling_id(&mut self) -> StableId {
        let rand_b = self.rng.next_u64();
        StableId::new_v7(self.base + 900_000 + self.rng.below(1_000), 0x0dd, rand_b)
    }

    /// `[-range, range]` 1/8 격자 값.
    fn coord(&mut self, range: u64) -> f32 {
        (self.rng.below(range * 16 + 1) as f32 - (range * 8) as f32) / 8.0
    }

    /// `[0, range]` 1/4 격자 값.
    fn length(&mut self, range: u64) -> f32 {
        self.rng.below(range * 4 + 1) as f32 / 4.0
    }

    fn finite_length(&mut self, range: u64) -> FiniteF32 {
        FiniteF32::new(self.length(range)).unwrap_or(FiniteF32::ZERO)
    }

    fn vec2(&mut self, range: u64) -> Vec2 {
        Vec2::new(self.coord(range), self.coord(range)).unwrap_or(Vec2::ZERO)
    }

    fn size(&mut self, range: u64) -> Size2 {
        Size2::new(self.length(range), self.length(range)).unwrap_or(Size2::ZERO)
    }

    fn color(&mut self) -> Color {
        let space = if self.rng.one_in(2) {
            ColorSpace::Srgb
        } else {
            ColorSpace::DisplayP3
        };
        // 채널은 sRGB 범위 밖(HDR)도 허용, 알파는 0..=1.
        let channel = |g: &mut Generator| (g.rng.below(41) as f32 - 10.0) / 16.0;
        let r = channel(self);
        let gc = channel(self);
        let b = channel(self);
        let a = self.rng.below(17) as f32 / 16.0;
        Color::new(space, r, gc, b, a)
            .unwrap_or_else(|_| Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0).expect("상수 색"))
    }

    fn transform(&mut self) -> Transform2D {
        let scale = |g: &mut Generator| (g.rng.below(33) as f32 - 16.0) / 8.0;
        let a = scale(self);
        let b = scale(self);
        let c = scale(self);
        let d = scale(self);
        let tx = self.coord(500);
        let ty = self.coord(500);
        Transform2D::new(a, b, c, d, tx, ty).unwrap_or_else(|_| Transform2D::identity())
    }

    fn name(&mut self) -> String {
        let mut name = (*self.rng.pick(&NAME_POOL)).to_string();
        if self.rng.one_in(3) {
            name.push(*self.rng.pick(&TEXT_CHARS));
        }
        name
    }

    fn corner_radius(&mut self) -> CornerRadius {
        let v = [
            self.length(50),
            self.length(50),
            self.length(50),
            self.length(50),
        ];
        CornerRadius::new(v[0], v[1], v[2], v[3]).unwrap_or(CornerRadius::ZERO)
    }

    fn frame(&mut self) -> FramePayload {
        let clipping = match self.rng.below(5) {
            0 => Clipping::Disabled,
            1 if !self.paths.is_empty() => {
                let target = *self.rng.pick(&self.paths.clone());
                Clipping::Path(NodeRef::new(target))
            }
            2 => Clipping::Path(NodeRef::new(self.dangling_id())),
            _ => Clipping::FrameBounds,
        };
        let background = self.rng.one_in(2).then(|| self.color());
        let border = if self.rng.one_in(3) {
            let color = self.color();
            let thickness = self.length(8);
            Border::new(color, thickness).ok()
        } else {
            None
        };
        let layout_mode = self.rng.one_in(4).then(|| self.dangling_id());
        FramePayload {
            size: self.size(2_000),
            transform: self.transform(),
            clipping,
            background,
            border,
            corner_radius: self.corner_radius(),
            layout_mode,
        }
    }

    fn shape(&mut self) -> ShapeGeometry {
        match self.rng.below(4) {
            0 => ShapeGeometry::Rectangle(RectangleParams {
                size: self.size(500),
                corner_radius: self.corner_radius(),
            }),
            1 => ShapeGeometry::Ellipse(EllipseParams {
                bounds: Rect::new(self.vec2(200), self.size(400)),
            }),
            2 => ShapeGeometry::Line(LineParams {
                start: self.vec2(300),
                end: self.vec2(300),
                cap: *self
                    .rng
                    .pick(&[LineCap::Butt, LineCap::Round, LineCap::Square]),
            }),
            _ => {
                let bounds = Rect::new(self.vec2(100), self.size(300));
                let sides = 3 + self.rng.below(62) as u32;
                let rotation = Angle::new(self.coord(360)).unwrap_or(Angle::new(0.0).expect("0"));
                let radius = self.length(20);
                match PolygonParams::new(bounds, sides, rotation, radius) {
                    Ok(params) => ShapeGeometry::Polygon(params),
                    Err(_) => ShapeGeometry::Ellipse(EllipseParams { bounds }),
                }
            }
        }
    }

    fn path(&mut self) -> IdentifiedPath {
        let mut commands = Vec::new();
        let subpaths = self.rng.below(4);
        for _ in 0..subpaths {
            commands.push(PathCommand::MoveTo(self.vec2(200)));
            for _ in 0..self.rng.below(6) {
                if self.rng.one_in(2) {
                    commands.push(PathCommand::LineTo(self.vec2(200)));
                } else {
                    commands.push(PathCommand::CubicTo {
                        c1: self.vec2(200),
                        c2: self.vec2(200),
                        to: self.vec2(200),
                    });
                }
            }
            if self.rng.one_in(2) {
                commands.push(PathCommand::Close);
            }
        }
        let winding = if self.rng.one_in(2) {
            WindingRule::NonZero
        } else {
            WindingRule::EvenOdd
        };
        let data = PathData::new(winding, commands).unwrap_or_default();
        IdentifiedPath::assign(data, &mut || self.next_id())
    }

    fn text(&mut self) -> TextPayload {
        let len = self.rng.below(24);
        let content: String = (0..len).map(|_| *self.rng.pick(&TEXT_CHARS)).collect();
        let paragraphs = (0..=content.matches('\n').count())
            .map(|_| Paragraph {
                style: self.rng.below(8) as u32,
            })
            .collect();
        let scalar_len = content.chars().count() as u64;
        let runs = (0..self.rng.below(4))
            .filter_map(|_| {
                let a = self.rng.below(scalar_len + 1) as u32;
                let b = self.rng.below(scalar_len + 1) as u32;
                TextRange::new(a.min(b), a.max(b)).map(|range| StyleRun {
                    range,
                    style: self.rng.below(8) as u32,
                })
            })
            .collect();
        TextPayload::new(UserText::new(&content), paragraphs, runs)
            .unwrap_or_else(|_| TextPayload::empty(0))
    }

    fn image(&mut self) -> ImagePayload {
        let mut image = ImagePayload::new(AssetRef::new(self.dangling_id()), self.size(1_000));
        image.fit = *self.rng.pick(&[
            FitMode::Fill,
            FitMode::Contain,
            FitMode::Cover,
            FitMode::None,
        ]);
        let x = self.rng.below(4) as f32 / 4.0;
        let y = self.rng.below(4) as f32 / 4.0;
        let w = (1 + self.rng.below(4 - (x * 4.0) as u64)) as f32 / 4.0;
        let h = (1 + self.rng.below(4 - (y * 4.0) as u64)) as f32 / 4.0;
        image.crop = CropRect::from_components(x, y, w, h).unwrap_or(CropRect::FULL);
        image.sampling = *self
            .rng
            .pick(&[SamplingMode::Linear, SamplingMode::Nearest]);
        image.color_space_override =
            *self
                .rng
                .pick(&[None, Some(ColorSpace::Srgb), Some(ColorSpace::DisplayP3)]);
        image
    }

    fn payload(&mut self) -> NodePayload {
        match self.rng.below(9) {
            0 | 1 => NodePayload::Frame(self.frame()),
            2 => NodePayload::Fragment(FragmentPayload),
            3 | 4 => NodePayload::Shape(self.shape()),
            5 => NodePayload::Path(self.path()),
            6 | 7 => NodePayload::Text(self.text()),
            _ => NodePayload::Image(self.image()),
        }
    }

    /// 먼저 만든 node(owner 이전) 중 하나, 또는 NIL/미해결.
    fn earlier_ref(&mut self, owner_index: usize) -> NodeRef {
        match self.rng.below(8) {
            0 => NodeRef::new(StableId::NIL),
            1 => NodeRef::new(self.dangling_id()),
            _ if owner_index > 0 => {
                NodeRef::new(self.created[self.rng.below(owner_index as u64) as usize])
            }
            _ => NodeRef::new(StableId::NIL),
        }
    }

    fn modifier_payload(&mut self, kind: ModifierKind, owner_index: usize) -> ModifierPayload {
        match kind {
            ModifierKind::Transform => ModifierPayload::Transform(self.transform()),
            ModifierKind::Boolean => ModifierPayload::Boolean {
                op: *self.rng.pick(&[
                    BooleanOp::Union,
                    BooleanOp::Subtract,
                    BooleanOp::Intersect,
                    BooleanOp::Exclude,
                ]),
                operands: (0..self.rng.below(4))
                    .map(|_| self.earlier_ref(owner_index))
                    .collect(),
            },
            ModifierKind::StrokeExpand => ModifierPayload::StrokeExpand {
                width: self.finite_length(16),
            },
            ModifierKind::Corner => ModifierPayload::Corner {
                radius: self.finite_length(64),
            },
            ModifierKind::Mask => ModifierPayload::Mask {
                source: self.earlier_ref(owner_index),
            },
            ModifierKind::Clip => ModifierPayload::Clip {
                source: self.earlier_ref(owner_index),
            },
            ModifierKind::Fill => ModifierPayload::Fill {
                color: self.color(),
            },
            ModifierKind::Stroke => ModifierPayload::Stroke {
                color: self.color(),
                width: self.finite_length(16),
            },
            ModifierKind::Shadow => ModifierPayload::Shadow {
                color: self.color(),
                offset: self.vec2(64),
                blur: self.finite_length(64),
            },
            ModifierKind::Blur => ModifierPayload::Blur {
                radius: self.finite_length(64),
            },
            ModifierKind::Blend => ModifierPayload::Blend {
                mode: *self.rng.pick(&[
                    BlendMode::Normal,
                    BlendMode::Multiply,
                    BlendMode::Screen,
                    BlendMode::Overlay,
                    BlendMode::Darken,
                    BlendMode::Lighten,
                ]),
            },
            ModifierKind::Opacity => ModifierPayload::Opacity {
                value: FiniteF32::new(self.rng.below(17) as f32 / 16.0).unwrap_or(FiniteF32::ONE),
            },
        }
    }

    fn modifier_stack(&mut self, owner: StableId, owner_index: usize) -> ModifierStack {
        let mut stack = ModifierStack::new(owner);
        for _ in 0..(1 + self.rng.below(4)) {
            let id = ModifierId::new(self.next_id());
            let kind = *self.rng.pick(&ModifierKind::ALL);
            let payload = self.modifier_payload(kind, owner_index);
            // 무작위 위치에 삽입(기존 항목 key 사이).
            let position = self.rng.below(stack.entries().len() as u64 + 1) as usize;
            let entries = stack.entries();
            let left = position.checked_sub(1).map(|i| entries[i].order.clone());
            let right = entries.get(position).map(|e| e.order.clone());
            let order = OrderKey::between(left.as_ref(), right.as_ref());
            let enabled = !self.rng.one_in(4);
            let entry = crate::modifier::ModifierEntry {
                id,
                order,
                kind,
                enabled,
                payload,
            };
            // 생성 규칙상 실패하지 않지만, 실패하면 그 항목만 건너뜁니다.
            let _ = stack.insert(entry);
        }
        stack
    }

    /// runtime 핸들 배치를 흔듭니다(dummy node insert/remove).
    fn churn(&mut self) {
        let dummy = StableId::new_v7(self.base + 999_999, 0x0fff, 0xdead_beef);
        for _ in 0..self.rng.below(3) {
            let record = NodeRecord {
                id: dummy,
                parent: None,
                order_key: OrderKey::between(None, None),
                visible: true,
                locked: false,
                display_name: String::new(),
                payload: NodePayload::Fragment(FragmentPayload),
            };
            if self.model.insert_node(record).is_ok() {
                let _ = self.model.remove_leaf(dummy);
            }
        }
    }
}

/// 한 container(자식을 가질 수 있는 node)의 상태.
struct Container {
    id: StableId,
    depth: u32,
    /// 정렬된 자식 key.
    child_keys: Vec<OrderKey>,
}

/// `seed`의 random valid 문서를 만듭니다.
pub fn random_valid_document(seed: u64) -> DocumentModel {
    let mut rng = SplitMix64::new(seed ^ 0x5EED_0000_D0C0_0000);
    let base = 1_800_000_000_000 + (seed % 1_000_000) * 1_000_000;
    let mut g = Generator {
        rng: SplitMix64::new(rng.next_u64()),
        base,
        counter: 0,
        model: DocumentModel::new(DocumentDefinition::new(StableId::NIL, String::new())),
        created: Vec::new(),
        paths: Vec::new(),
    };

    let document_id = g.next_id();
    let document_name = g.name();
    let mut definition = DocumentDefinition::new(document_id, document_name);
    let mut views: Vec<StableId> = Vec::new();
    for _ in 0..(1 + g.rng.below(3)) {
        let page_id = g.next_id();
        let page_name = g.name();
        let mut page = PageDefinition::new(page_id, page_name);
        for _ in 0..g.rng.below(4) {
            let view_id = g.next_id();
            let root = g.next_id();
            let view_name = g.name();
            if page
                .add_view(ViewDefinition::new(view_id, view_name, root))
                .is_ok()
            {
                views.push(root);
            }
        }
        let _ = definition.add_page(page);
    }
    g.model = DocumentModel::new(definition);

    for root in views {
        let mut root_frame = g.frame();
        if matches!(root_frame.clipping, Clipping::Path(_)) {
            root_frame.clipping = Clipping::FrameBounds;
        }
        let root_record = NodeRecord {
            id: root,
            parent: None,
            order_key: OrderKey::between(None, None),
            visible: !g.rng.one_in(8),
            locked: g.rng.one_in(8),
            display_name: g.name(),
            payload: NodePayload::Frame(root_frame),
        };
        if g.model.insert_node(root_record).is_err() {
            continue;
        }
        g.created.push(root);
        let mut containers = vec![Container {
            id: root,
            depth: 0,
            child_keys: Vec::new(),
        }];
        // 가끔 depth 한도까지 깊은 chain.
        let deep = g.rng.one_in(16);
        let count = if deep {
            u64::from(MAX_NODE_DEPTH) + g.rng.below(8)
        } else {
            g.rng.below(25)
        };
        for _ in 0..count {
            g.churn();
            let eligible: Vec<usize> = containers
                .iter()
                .enumerate()
                .filter(|(_, c)| c.depth < MAX_NODE_DEPTH)
                .map(|(i, _)| i)
                .collect();
            let Some(&slot) = eligible.get(if deep {
                eligible.len() - 1
            } else {
                g.rng.below(eligible.len() as u64) as usize
            }) else {
                break;
            };
            let parent = containers[slot].id;
            let depth = containers[slot].depth + 1;
            let keys = &containers[slot].child_keys;
            let position = g.rng.below(keys.len() as u64 + 1) as usize;
            let left = position.checked_sub(1).map(|i| keys[i].clone());
            let right = keys.get(position).cloned();
            let order_key = OrderKey::between(left.as_ref(), right.as_ref());
            containers[slot]
                .child_keys
                .insert(position, order_key.clone());

            let payload = if deep {
                NodePayload::Frame(g.frame())
            } else {
                g.payload()
            };
            let id = g.next_id();
            let is_container = matches!(payload, NodePayload::Frame(_) | NodePayload::Fragment(_));
            let is_fragment = matches!(payload, NodePayload::Fragment(_));
            let is_path = matches!(payload, NodePayload::Path(_));
            let record = NodeRecord {
                id,
                parent: Some(parent),
                order_key,
                visible: !g.rng.one_in(6),
                locked: g.rng.one_in(6),
                display_name: g.name(),
                payload,
            };
            if g.model.insert_node(record).is_err() {
                continue;
            }
            let owner_index = g.created.len();
            g.created.push(id);
            if is_path {
                g.paths.push(id);
            }
            if is_container {
                containers.push(Container {
                    id,
                    depth,
                    child_keys: Vec::new(),
                });
            }
            if !is_fragment && g.rng.one_in(3) {
                let stack = g.modifier_stack(id, owner_index);
                let _ = g.model.set_modifier_stack(stack);
            }
        }
    }
    g.model
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node_kind::NodeKind;
    use crate::record_codec::SubtreeRecords;
    use crate::records::DocumentRecords;
    use crate::shape::ShapeKind;
    use crate::test_support::semantic_snapshot::{model_snapshot_json, semantic_snapshot_json};
    use crate::validate::validate_document;
    use std::collections::BTreeSet;

    const SEEDS: u64 = 1_000;

    /// storage chunk에 들어가는 bytes: 문서 정의 + View별 subtree payload.
    fn stored_bytes(records: &DocumentRecords) -> Vec<Vec<u8>> {
        let mut bytes = vec![records.definition.encode()];
        for (_, subtree) in records.view_subtrees().unwrap() {
            bytes.push(subtree.encode());
        }
        bytes
    }

    /// storage chunk와 같은 경로(View subtree 분할 → subtree bytes → 재결합 → 모델 재구축)로
    /// 문서를 왕복합니다.
    fn round_trip(bytes: &[Vec<u8>]) -> DocumentModel {
        let definition = DocumentDefinition::decode(&bytes[0]).unwrap();
        let subtrees = bytes[1..]
            .iter()
            .map(|b| SubtreeRecords::decode(b).unwrap())
            .collect();
        let merged = DocumentRecords::from_subtrees(definition, subtrees);
        let (model, _) = DocumentModel::from_records(merged).unwrap();
        model
    }

    #[test]
    fn smoke_thousand_seed_round_trip_property() {
        // 검증 기준(VI.10.5): 1,000 seed에서 panic·데이터 손실·비결정성 0건.
        let mut node_kinds = BTreeSet::new();
        let mut shape_kinds = BTreeSet::new();
        let mut modifier_kinds = BTreeSet::new();
        let mut max_depth_docs = 0;
        let mut unresolved = 0usize;
        let mut total_nodes = 0usize;
        for seed in 0..SEEDS {
            let model = random_valid_document(seed);
            let records = model.to_records().unwrap();
            // 생성기는 항상 유효한 문서를 만듭니다.
            let diagnostics = validate_document(&records)
                .unwrap_or_else(|e| panic!("seed {seed}: 유효하지 않은 문서: {e}"));
            unresolved += diagnostics.unresolved.len();
            total_nodes += records.nodes.len();

            // 비결정성 0: 같은 seed → 같은 레코드(snapshot은 레코드의 순수 함수).
            let again = random_valid_document(seed);
            assert_eq!(
                again.to_records().unwrap(),
                records,
                "seed {seed}: 레코드 비결정"
            );

            // 데이터 손실 0: 저장 bytes로 왕복한 모델의 레코드·semantic snapshot 동일.
            let bytes = stored_bytes(&records);
            let loaded = round_trip(&bytes);
            let loaded_records = loaded.to_records().unwrap();
            assert_eq!(loaded_records, records, "seed {seed}: 레코드 손실");
            assert_eq!(
                model_snapshot_json(&loaded).unwrap(),
                semantic_snapshot_json(&records),
                "seed {seed}: snapshot"
            );
            // 재로드 문서의 저장 bytes도 byte-identical입니다.
            assert_eq!(
                stored_bytes(&loaded_records),
                bytes,
                "seed {seed}: 저장 bytes"
            );

            for node in &records.nodes {
                node_kinds.insert(node.kind());
                if let NodePayload::Shape(shape) = &node.payload {
                    shape_kinds.insert(shape.kind());
                }
            }
            for stack in &records.modifier_stacks {
                for entry in stack.entries() {
                    modifier_kinds.insert(entry.kind);
                }
            }
            let deep = records
                .views()
                .any(|view| depth_of_view(&records, view.root_frame) == MAX_NODE_DEPTH);
            if deep {
                max_depth_docs += 1;
            }
        }
        // 생성기가 실제로 넓은 범위를 덮는지(분포 회귀 방지).
        let expected_kinds: BTreeSet<NodeKind> = NodeKind::ALL
            .into_iter()
            .filter(|k| *k != NodeKind::ComponentInstance)
            .collect();
        assert_eq!(node_kinds, expected_kinds);
        assert_eq!(shape_kinds, ShapeKind::ALL.into_iter().collect());
        assert_eq!(modifier_kinds, ModifierKind::ALL.into_iter().collect());
        assert!(max_depth_docs > 10, "depth 64 문서 {max_depth_docs}");
        assert!(unresolved > 100, "미해결 참조 {unresolved}");
        assert!(total_nodes > 10_000, "node {total_nodes}");
    }

    fn depth_of_view(records: &DocumentRecords, root: StableId) -> u32 {
        let parent_of: std::collections::BTreeMap<StableId, StableId> = records
            .nodes
            .iter()
            .filter_map(|n| n.parent.map(|p| (n.id, p)))
            .collect();
        records
            .nodes
            .iter()
            .map(|n| {
                let mut depth = 0;
                let mut current = n.id;
                while let Some(parent) = parent_of.get(&current) {
                    depth += 1;
                    current = *parent;
                }
                if current == root {
                    depth
                } else {
                    0
                }
            })
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn different_seeds_produce_different_documents() {
        let a = semantic_snapshot_json(&random_valid_document(1).to_records().unwrap());
        let b = semantic_snapshot_json(&random_valid_document(2).to_records().unwrap());
        assert_ne!(a, b);
    }

    #[test]
    fn prng_sequence_is_fixed() {
        // 자체 PRNG golden(플랫폼·버전 무관 고정).
        let mut rng = SplitMix64::new(0);
        assert_eq!(rng.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(rng.next_u64(), 0x6E78_9E6A_A1B9_65F4);
        assert_eq!(rng.below(0), 0);
    }
}
