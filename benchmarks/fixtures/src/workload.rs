//! 병목 특성별 workload fixture: text·image·effect·constraint-heavy (XVI.3.3).
//!
//! 같은 node 수의 기준 synthetic 문서(XVI.3.1)와 비교해 한 가지 병목 특성을 몰아준 fixture를
//! 따로 만듭니다. 특성은 생성 후 **계측값**([`DocumentProfile`], [`ConstraintProfile`])으로
//! 확인합니다 — 생성 규칙을 믿지 않고 실제 모델·인코딩·레이아웃 엔진 결과에서 셉니다.
//!
//! | workload | 표현 | 병목 특성(계측값) |
//! |---|---|---|
//! | [`DocumentWorkload::TextHeavy`] | 문서 | node당 text scalar 수, text payload bytes 비중, style run 수 |
//! | [`DocumentWorkload::ImageHeavy`] | 문서 | Image node 비중, 서로 다른 asset 수, 배치 pixel 면적 |
//! | [`DocumentWorkload::EffectHeavy`] | 문서 | node당 effect modifier 수(Shadow·Blur·Blend·Opacity), geometry 무효화 modifier 수 |
//! | constraint-heavy([`constraint_heavy`]) | 레이아웃 입력(`stellar_layout`) | node당 constraint 등식 수, sibling 참조 연결 요소 크기, 엔진 배치 결과 |
//!
//! **token-heavy는 아직 만들 수 없습니다**: 토큰 모델·registry·dependency graph(VIII.5)가
//! 구현되지 않아 토큰을 담을 원본 표현이 없습니다([`UNAVAILABLE_WORKLOADS`]). 문서 레코드에도
//! constraint 필드가 없으므로 constraint-heavy는 레이아웃 엔진 입력으로 만듭니다.

use std::collections::{BTreeSet, HashMap};

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::AssetRef;
use stellar_core_types::text::UserText;
use stellar_core_types::value::Length;
use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::image::{CropRect, FitMode, ImagePayload, SamplingMode};
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{ModifierKind, ModifierPayload};
use stellar_document::modifier_metadata::{kind_invalidation, InvalidationClass};
use stellar_document::node_kind::NodeKind;
use stellar_document::record_codec::encode_payload;
use stellar_document::records::{DocumentRecords, NodePayload};
use stellar_document::text::{scalar_len, Paragraph, StyleRun, TextPayload, TextRange};
use stellar_layout::constraint::{AnchorEdge, AnchorTarget, Constraint};
use stellar_layout::engine::compute_layout;
use stellar_layout::input::{ContainerLayout, LayoutInput, LayoutNode};
use stellar_layout::sizing::SizingMode;
use stellar_storage::hash::ContentHash;

use crate::build::{self, finish_document, BuiltDocument, Quarter};
use crate::digest::FixtureDigest;
use crate::error::FixtureError;
use crate::ids::IdAllocator;
use crate::rng::SplitMix64;

/// 아직 만들 수 없는 workload와 이유.
pub const UNAVAILABLE_WORKLOADS: [(&str, &str); 1] = [(
    "token-heavy",
    "토큰 모델·registry·dependency graph(VIII.5)가 구현되지 않았습니다",
)];

/// workload fixture node 수 상한(오입력 방지).
pub const MAX_WORKLOAD_NODES: usize = 1_000_000;

/// 문서 표현 workload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DocumentWorkload {
    /// 긴 다문단·다중 style run 텍스트 위주.
    TextHeavy,
    /// 서로 다른 asset을 참조하는 큰 이미지 위주.
    ImageHeavy,
    /// node마다 Shadow·Blur·Blend·Opacity 등 effect modifier stack.
    EffectHeavy,
}

impl DocumentWorkload {
    /// 모든 문서 workload.
    pub const ALL: [DocumentWorkload; 3] = [
        DocumentWorkload::TextHeavy,
        DocumentWorkload::ImageHeavy,
        DocumentWorkload::EffectHeavy,
    ];

    /// 안정 이름.
    pub fn name(self) -> &'static str {
        match self {
            DocumentWorkload::TextHeavy => "text-heavy",
            DocumentWorkload::ImageHeavy => "image-heavy",
            DocumentWorkload::EffectHeavy => "effect-heavy",
        }
    }
}

/// 문서 workload fixture.
#[derive(Debug)]
pub struct WorkloadDocument {
    /// workload.
    pub workload: DocumentWorkload,
    /// 문서(validator 통과).
    pub built: BuiltDocument,
    /// content hash.
    pub content_hash: ContentHash,
}

/// View 하나의 node 수(root + section + leaf).
const NODES_PER_VIEW: usize = 250;
/// section 하나의 leaf 수.
const LEAVES_PER_SECTION: usize = 24;

/// `workload` 문서를 정확히 `nodes`개 node로 만듭니다.
///
/// 모양: 페이지 → View(약 250 node) → root Frame → section Frame(leaf 24개씩) → leaf. leaf와
/// modifier가 workload별로 다릅니다.
pub fn workload_document(
    workload: DocumentWorkload,
    seed: u64,
    nodes: usize,
) -> Result<WorkloadDocument, FixtureError> {
    if !(2..=MAX_WORKLOAD_NODES).contains(&nodes) {
        return Err(FixtureError::InvalidSpec {
            reason: "workload node 수는 2..=1,000,000이어야 합니다",
        });
    }
    let mut rng = SplitMix64::new(seed ^ 0x3017_10AD_0000_0006 ^ workload as u64);
    let mut ids = IdAllocator::new(seed ^ 0x3017_10AD ^ ((workload as u64) << 32));
    let views = nodes.div_ceil(NODES_PER_VIEW);
    let document = ids.next_id();
    let mut definition = DocumentDefinition::new(document, workload.name().to_string());
    let mut page = PageDefinition::new(ids.next_id(), "Workload".to_string());
    let mut roots = Vec::with_capacity(views);
    for v in 0..views {
        let root = ids.next_id();
        page.add_view(ViewDefinition::new(
            ids.next_id(),
            format!("View {}", v + 1),
            root,
        ))
        .map_err(FixtureError::Definition)?;
        roots.push(root);
    }
    definition
        .add_page(page)
        .map_err(FixtureError::Definition)?;
    let mut records = DocumentRecords::new(definition);
    records.nodes.reserve(nodes);

    let base = nodes / views;
    let remainder = nodes % views;
    for (v, root) in roots.into_iter().enumerate() {
        let budget = base + usize::from(v < remainder);
        build_workload_view(workload, &mut rng, &mut ids, &mut records, root, budget);
    }
    let built = finish_document(records)?;
    let mut digest = FixtureDigest::new(workload.name(), seed);
    digest.parameter("nodes", nodes as u64);
    digest.document(&built.serialized);
    Ok(WorkloadDocument {
        workload,
        built,
        content_hash: digest.finish(),
    })
}

/// View 하나(root 포함 `budget`개, `budget ≥ 1`).
fn build_workload_view(
    workload: DocumentWorkload,
    rng: &mut SplitMix64,
    ids: &mut IdAllocator,
    records: &mut DocumentRecords,
    root: StableId,
    budget: usize,
) {
    let screen_w: Quarter = 1440 * 4;
    let screen_h: Quarter = 1024 * 4;
    records.nodes.push(build::record(
        root,
        None,
        OrderKey::between(None, None),
        true,
        "Screen",
        NodePayload::Frame(build::frame(rng, screen_w, screen_h, 0, 0)),
    ));
    if workload == DocumentWorkload::EffectHeavy {
        push_effect_stack(rng, ids, records, root);
    }
    // 나머지를 section(1 + leaf 최대 24)으로 나눕니다.
    let mut remaining = budget.saturating_sub(1);
    let sections = remaining.div_ceil(LEAVES_PER_SECTION + 1);
    let section_keys = OrderKey::evenly_spaced(sections);
    for (s, key) in section_keys.into_iter().enumerate() {
        if remaining == 0 {
            break;
        }
        let section = ids.next_id();
        let s = s as Quarter;
        let (w, h) = (360 * 4, 480 * 4);
        records.nodes.push(build::record(
            section,
            Some(root),
            key,
            true,
            "Section",
            NodePayload::Frame(build::frame(rng, w, h, (s % 4) * w, (s / 4 % 2) * h)),
        ));
        remaining -= 1;
        if workload == DocumentWorkload::EffectHeavy {
            push_effect_stack(rng, ids, records, section);
        }
        let leaves = remaining.min(LEAVES_PER_SECTION);
        remaining -= leaves;
        for key in OrderKey::evenly_spaced(leaves) {
            let id = ids.next_id();
            let payload = match workload {
                DocumentWorkload::TextHeavy => {
                    // 대부분 긴 본문 텍스트, 일부는 장식 도형.
                    if rng.one_in(8) {
                        NodePayload::Shape(build::shape(rng, w / 2, h / 8))
                    } else {
                        let words = rng.range(40, 200) as usize;
                        NodePayload::Text(rich_text(rng, words))
                    }
                }
                DocumentWorkload::ImageHeavy => {
                    if rng.one_in(8) {
                        NodePayload::Text(build::text(rng, 3, 0))
                    } else {
                        // 이미지마다 서로 다른 asset(디코딩·업로드 압박).
                        NodePayload::Image(large_image(rng, AssetRef::new(ids.next_id())))
                    }
                }
                DocumentWorkload::EffectHeavy => {
                    NodePayload::Shape(build::shape(rng, w / 3, h / 6))
                }
            };
            let is_fragment = payload.kind() == NodeKind::Fragment;
            records
                .nodes
                .push(build::record(id, Some(section), key, true, "Item", payload));
            if workload == DocumentWorkload::EffectHeavy && !is_fragment {
                push_effect_stack(rng, ids, records, id);
            }
        }
    }
}

/// effect modifier 3..=6개 stack(큰 blur·shadow 위주).
fn push_effect_stack(
    rng: &mut SplitMix64,
    ids: &mut IdAllocator,
    records: &mut DocumentRecords,
    owner: StableId,
) {
    const EFFECTS: [(ModifierKind, u32); 6] = [
        (ModifierKind::Shadow, 4),
        (ModifierKind::Blur, 3),
        (ModifierKind::Blend, 2),
        (ModifierKind::Opacity, 2),
        (ModifierKind::StrokeExpand, 1),
        (ModifierKind::Stroke, 1),
    ];
    let count = rng.range(3, 6) as usize;
    let payloads: Vec<ModifierPayload> = (0..count)
        .map(|_| {
            let kind = rng.weighted(&EFFECTS);
            build::plain_modifier(rng, kind)
        })
        .collect();
    records
        .modifier_stacks
        .push(build::modifier_stack(ids, owner, payloads));
}

/// 다문단·다중 style run 텍스트(문단당 12 단어, run 4..=16개).
fn rich_text(rng: &mut SplitMix64, words: usize) -> TextPayload {
    let mut content = String::new();
    for i in 0..words {
        if i > 0 {
            content.push(if i % 12 == 0 { '\n' } else { ' ' });
        }
        content.push_str(rng.pick(&build::WORD_POOL));
    }
    let paragraphs = (0..=content.matches('\n').count())
        .map(|_| Paragraph {
            style: rng.below(6) as u32,
        })
        .collect();
    let len = scalar_len(&content);
    let run_count = rng.range(4, 16) as u32;
    let step = (len / run_count).max(1);
    let mut runs = Vec::with_capacity(run_count as usize);
    let mut start = 0u32;
    while start < len && runs.len() < run_count as usize {
        let end = (start + step).min(len);
        if let Some(range) = TextRange::new(start, end) {
            runs.push(StyleRun {
                range,
                style: rng.below(12) as u32,
            });
        }
        start = end;
    }
    TextPayload::new(UserText::new(&content), paragraphs, runs)
        .unwrap_or_else(|_| TextPayload::empty(0))
}

/// 큰 배치 상자(512..=4096 px)의 이미지.
fn large_image(rng: &mut SplitMix64, asset: AssetRef) -> ImagePayload {
    let w = rng.range(512, 4096) as Quarter * 4;
    let h = rng.range(512, 4096) as Quarter * 4;
    let mut image = ImagePayload::new(asset, build::size(w, h));
    image.fit = rng.pick(&[
        FitMode::Fill,
        FitMode::Contain,
        FitMode::Cover,
        FitMode::None,
    ]);
    image.sampling = rng.pick(&[SamplingMode::Linear, SamplingMode::Nearest]);
    if rng.one_in(3) {
        image.crop = CropRect::from_components(0.0, 0.0, 0.5, 0.5).unwrap_or(CropRect::FULL);
    }
    image
}

/// 문서 계측값.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DocumentProfile {
    /// node 수.
    pub nodes: usize,
    /// Text node 수.
    pub text_nodes: usize,
    /// text scalar(문자) 합.
    pub text_scalars: u64,
    /// text 문단 합.
    pub text_paragraphs: u64,
    /// style run 합.
    pub style_runs: u64,
    /// Image node 수.
    pub image_nodes: usize,
    /// Image가 참조한 서로 다른 asset 수.
    pub distinct_assets: usize,
    /// Image 배치 상자 면적 합(px²).
    pub image_pixels: u64,
    /// modifier 항목 합.
    pub modifiers: usize,
    /// effect modifier(Shadow·Blur·Blend·Opacity) 합.
    pub effect_modifiers: usize,
    /// geometry(visual bounds)를 무효화하는 modifier 합(VI.9.7 registry 기준).
    pub geometry_modifiers: usize,
    /// 전체 node payload canonical bytes.
    pub payload_bytes: u64,
    /// Text payload bytes.
    pub text_payload_bytes: u64,
    /// Image payload bytes.
    pub image_payload_bytes: u64,
    /// modifier stack canonical bytes.
    pub modifier_bytes: u64,
}

impl DocumentProfile {
    /// node당 text scalar.
    pub fn text_scalars_per_node(&self) -> f64 {
        ratio(self.text_scalars as f64, self.nodes)
    }

    /// Image node 비중.
    pub fn image_share(&self) -> f64 {
        ratio(self.image_nodes as f64, self.nodes)
    }

    /// node당 effect modifier.
    pub fn effects_per_node(&self) -> f64 {
        ratio(self.effect_modifiers as f64, self.nodes)
    }

    /// node payload bytes 중 Text 비중.
    pub fn text_byte_share(&self) -> f64 {
        if self.payload_bytes == 0 {
            0.0
        } else {
            self.text_payload_bytes as f64 / self.payload_bytes as f64
        }
    }
}

fn ratio(value: f64, nodes: usize) -> f64 {
    if nodes == 0 {
        0.0
    } else {
        value / nodes as f64
    }
}

/// 문서를 계측합니다(영속 레코드와 canonical 인코딩 기준).
pub fn profile_document(model: &DocumentModel) -> Result<DocumentProfile, FixtureError> {
    let records = model.to_records().map_err(FixtureError::Model)?;
    let mut profile = DocumentProfile {
        nodes: records.nodes.len(),
        ..DocumentProfile::default()
    };
    let mut assets: BTreeSet<StableId> = BTreeSet::new();
    for node in &records.nodes {
        let bytes = encode_payload(&node.payload).len() as u64;
        profile.payload_bytes += bytes;
        match &node.payload {
            NodePayload::Text(text) => {
                profile.text_nodes += 1;
                profile.text_scalars += u64::from(scalar_len(text.content()));
                profile.text_paragraphs += text.paragraphs().len() as u64;
                profile.style_runs += text.style_runs().len() as u64;
                profile.text_payload_bytes += bytes;
            }
            NodePayload::Image(image) => {
                profile.image_nodes += 1;
                assets.insert(image.asset.id());
                profile.image_pixels +=
                    (image.size.width() as f64 * image.size.height() as f64) as u64;
                profile.image_payload_bytes += bytes;
            }
            _ => {}
        }
    }
    profile.distinct_assets = assets.len();
    for stack in &records.modifier_stacks {
        profile.modifier_bytes += stack.encode().len() as u64;
        for entry in stack.entries() {
            profile.modifiers += 1;
            if matches!(
                entry.kind,
                ModifierKind::Shadow
                    | ModifierKind::Blur
                    | ModifierKind::Blend
                    | ModifierKind::Opacity
            ) {
                profile.effect_modifiers += 1;
            }
            if kind_invalidation(entry.kind).contains(InvalidationClass::Geometry) {
                profile.geometry_modifiers += 1;
            }
        }
    }
    Ok(profile)
}

// ---------------------------------------------------------------------------
// constraint-heavy
// ---------------------------------------------------------------------------

/// constraint-heavy fixture(레이아웃 엔진 입력).
#[derive(Debug, Clone)]
pub struct ConstraintFixture {
    /// 레이아웃 입력.
    pub input: LayoutInput,
    /// content hash(`LayoutInput::encode` canonical bytes).
    pub content_hash: ContentHash,
}

/// panel 하나의 item chain 길이.
const CHAIN_ITEMS: usize = 32;
/// panel 격자 열 수.
const PANEL_COLS: usize = 20;
const PANEL_W: f32 = 360.0;
const PANEL_H: f32 = 1_040.0;
const ITEM_H: f32 = 24.0;
const ITEM_GAP: f32 = 4.0;

fn length(value: f32) -> Length {
    Length::new(value).unwrap_or(Length::from_raw(stellar_core_types::value::FiniteF32::ZERO))
}

fn parent(edge: AnchorEdge, target: AnchorEdge, offset: f32) -> Constraint {
    Constraint::new(edge, AnchorTarget::Parent(target), length(offset))
}

fn sibling(edge: AnchorEdge, node: StableId, target: AnchorEdge, offset: f32) -> Constraint {
    Constraint::new(
        edge,
        AnchorTarget::Sibling { node, edge: target },
        length(offset),
    )
}

/// node가 정확히 `nodes`개인 constraint-heavy 레이아웃 입력을 만듭니다.
///
/// root(Constraint container) ← panel(Constraint container, 부모 기준 위치) ← item chain. 첫 item은
/// 부모 top에, 다음 item은 바로 앞 item의 bottom에 sibling constraint로 매달려 panel마다 길이
/// 32의 의존 chain(연결 요소)이 생깁니다. item 너비는 `Fill`이고 Left·Right 두 등식으로 정합니다.
pub fn constraint_heavy(seed: u64, nodes: usize) -> Result<ConstraintFixture, FixtureError> {
    if !(2..=MAX_WORKLOAD_NODES).contains(&nodes) {
        return Err(FixtureError::InvalidSpec {
            reason: "workload node 수는 2..=1,000,000이어야 합니다",
        });
    }
    let mut rng = SplitMix64::new(seed ^ 0xC057_0000_0000_0007);
    let mut ids = IdAllocator::new(seed ^ 0xC057);
    let root = ids.next_id();
    let panels = (nodes - 1).div_ceil(CHAIN_ITEMS + 1);
    let rows = panels.div_ceil(PANEL_COLS);
    let mut layout_nodes: Vec<LayoutNode> = Vec::with_capacity(nodes);
    let mut root_node = LayoutNode::new(root)
        .with_container(ContainerLayout::Constraint)
        .with_fixed_size(
            length(PANEL_COLS as f32 * (PANEL_W + 20.0)),
            length(rows as f32 * (PANEL_H + 20.0)),
        );
    let panel_keys = OrderKey::evenly_spaced(panels);
    let mut remaining = nodes - 1;
    for (p, key) in panel_keys.into_iter().enumerate() {
        let panel = ids.next_id();
        root_node = root_node.with_child(panel, key);
        let col = (p % PANEL_COLS) as f32;
        let row = (p / PANEL_COLS) as f32;
        let mut panel_node = LayoutNode::new(panel)
            .with_container(ContainerLayout::Constraint)
            .with_fixed_size(length(PANEL_W), length(PANEL_H))
            .with_constraint(parent(
                AnchorEdge::Left,
                AnchorEdge::Left,
                col * (PANEL_W + 20.0),
            ))
            .with_constraint(parent(
                AnchorEdge::Top,
                AnchorEdge::Top,
                row * (PANEL_H + 20.0),
            ));
        remaining -= 1;
        let items = remaining.min(CHAIN_ITEMS);
        remaining -= items;
        let mut previous: Option<StableId> = None;
        for key in OrderKey::evenly_spaced(items) {
            let item = ids.next_id();
            panel_node = panel_node.with_child(item, key);
            let inset = (rng.below(4) * 4) as f32;
            let mut node = LayoutNode::new(item)
                .with_sizing(SizingMode::Fill, SizingMode::Fixed(length(ITEM_H)))
                .with_constraint(parent(AnchorEdge::Right, AnchorEdge::Right, -8.0));
            node = match previous {
                None => node
                    .with_constraint(parent(AnchorEdge::Left, AnchorEdge::Left, 8.0 + inset))
                    .with_constraint(parent(AnchorEdge::Top, AnchorEdge::Top, 8.0)),
                Some(prev) => node
                    .with_constraint(sibling(AnchorEdge::Left, prev, AnchorEdge::Left, inset))
                    .with_constraint(sibling(AnchorEdge::Top, prev, AnchorEdge::Bottom, ITEM_GAP)),
            };
            layout_nodes.push(node);
            previous = Some(item);
        }
        layout_nodes.push(panel_node);
    }
    layout_nodes.push(root_node);
    let input = LayoutInput::new(root, layout_nodes);
    input.validate().map_err(FixtureError::Layout)?;
    let mut digest = FixtureDigest::new("constraint-heavy", seed);
    digest.parameter("nodes", nodes as u64);
    digest.resource("layout-input", root, &input.encode());
    Ok(ConstraintFixture {
        input,
        content_hash: digest.finish(),
    })
}

/// constraint 계측값.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConstraintProfile {
    /// 입력 node 수.
    pub nodes: usize,
    /// constraint를 가진 node 수.
    pub constrained_nodes: usize,
    /// constraint 등식 합.
    pub constraints: usize,
    /// sibling target 등식 수.
    pub sibling_constraints: usize,
    /// sibling 참조로 이어진 가장 큰 형제 연결 요소 크기.
    pub max_component: usize,
    /// 레이아웃 엔진이 배치한 box 수.
    pub laid_out: usize,
    /// 레이아웃 진단 수(미해결 target·underdetermined).
    pub diagnostics: usize,
}

impl ConstraintProfile {
    /// node당 constraint 등식.
    pub fn constraints_per_node(&self) -> f64 {
        ratio(self.constraints as f64, self.nodes)
    }
}

/// constraint fixture를 계측합니다: 입력의 등식·sibling 연결 요소를 세고, 레이아웃 엔진
/// (`compute_layout`)으로 실제 배치합니다.
pub fn profile_constraints(fixture: &ConstraintFixture) -> Result<ConstraintProfile, FixtureError> {
    let input = &fixture.input;
    let mut profile = ConstraintProfile {
        nodes: input.nodes.len(),
        ..ConstraintProfile::default()
    };
    // sibling 참조 graph의 연결 요소(union-find, 반복).
    let index: HashMap<StableId, usize> = input
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id, i))
        .collect();
    let mut parent_of: Vec<usize> = (0..input.nodes.len()).collect();
    fn find(parents: &mut [usize], mut i: usize) -> usize {
        while parents[i] != i {
            parents[i] = parents[parents[i]];
            i = parents[i];
        }
        i
    }
    for (i, node) in input.nodes.iter().enumerate() {
        if !node.constraints.is_empty() {
            profile.constrained_nodes += 1;
        }
        for constraint in &node.constraints {
            profile.constraints += 1;
            if let AnchorTarget::Sibling { node: target, .. } = constraint.target {
                profile.sibling_constraints += 1;
                if let Some(&j) = index.get(&target) {
                    let (a, b) = (find(&mut parent_of, i), find(&mut parent_of, j));
                    if a != b {
                        parent_of[a] = b;
                    }
                }
            }
        }
    }
    let mut sizes: HashMap<usize, usize> = HashMap::new();
    for i in 0..input.nodes.len() {
        let r = find(&mut parent_of, i);
        *sizes.entry(r).or_default() += 1;
    }
    profile.max_component = sizes.values().copied().max().unwrap_or(0);
    let result = compute_layout(input).map_err(FixtureError::Layout)?;
    profile.laid_out = result.len();
    profile.diagnostics = result.diagnostics().len();
    Ok(profile)
}
