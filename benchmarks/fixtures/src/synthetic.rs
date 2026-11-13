//! 규모별 synthetic fixture generator (XVI.3.1).
//!
//! 아키텍처 §5.3 성능 보장 문서 규모의 node 수를 **정확히** 만족하는 문서를 seed에서 만듭니다.
//!
//! | tier | 문서 | node |
//! |---|---|---:|
//! | [`ScaleTier::Nodes10k`] | 1 | 10,000 |
//! | [`ScaleTier::Nodes200k`] | 1 (활성 문서 상한) | 200,000 |
//! | [`ScaleTier::Project1M`] | 9 (200,000 1개 + 100,000 8개, 동시 열림 상한 10 이내) | 1,000,000 |
//!
//! 파일을 저장소에 두지 않고 `(generator version, tier, seed)`로 재현합니다. 결과의 정체는
//! [`SyntheticSummary::content_hash`]이며, 같은 명세는 언제나 같은 hash를 냅니다.
//!
//! ## 문서 모양
//!
//! 문서 → 페이지(View [`TreeShape::views_per_page`]개) → View root Frame(화면) → 중첩
//! Frame·Fragment container → leaf(Shape·Text·Path·Image·빈 Frame·빈 Fragment). View 하나는
//! 약 [`TreeShape::nodes_per_view`]개 node라 View subtree chunk가 split 임계치(V.6.4,
//! 256 KiB)보다 작습니다([`crate::split`]로 확인). container의 자식 수는
//! `min_fanout..=max_fanout`, depth는 `max_depth` 이하입니다. 일부 node는 modifier stack
//! (Fill·Stroke·Shadow·Opacity·Blur·Corner·Transform·Blend, 드물게 앞서 만든 node를 가리키는
//! Mask)을 가집니다. 참조는 언제나 **먼저 만든** node만 가리켜 modifier 참조 graph가 DAG입니다.
//!
//! 생성은 반복(비재귀)이고, 프로젝트 tier는 문서를 하나씩 만들어 직렬화·hash한 뒤 버리므로
//! 최대 메모리는 가장 큰 문서 하나 수준입니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::{AssetRef, NodeRef};
use stellar_document::definitions::{
    DocumentDefinition, PageDefinition, ProjectDocumentIndex, ViewDefinition,
};
use stellar_document::fragment::FragmentPayload;
use stellar_document::modifier::{ModifierKind, ModifierPayload};
use stellar_document::node_kind::NodeKind;
use stellar_document::records::{DocumentRecords, NodePayload};
use stellar_document::validate::MAX_NODE_DEPTH;
use stellar_storage::hash::ContentHash;

use crate::build::{self, finish_document, BuiltDocument, Quarter};
use crate::digest::FixtureDigest;
use crate::error::FixtureError;
use crate::ids::IdAllocator;
use crate::rng::SplitMix64;

/// 한 명세가 만들 수 있는 node 총수 상한(오입력으로 인한 폭주 방지).
pub const MAX_TOTAL_NODES: usize = 5_000_000;

/// 한 명세의 문서 수 상한.
pub const MAX_DOCUMENTS: usize = 64;

/// 성능 보장 규모 tier (아키텍처 §5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScaleTier {
    /// 문서 하나, node 1만.
    Nodes10k,
    /// 활성 문서 상한: 문서 하나, node 20만.
    Nodes200k,
    /// 프로젝트 전체 node 100만(문서 9개).
    Project1M,
}

impl ScaleTier {
    /// 모든 tier.
    pub const ALL: [ScaleTier; 3] = [
        ScaleTier::Nodes10k,
        ScaleTier::Nodes200k,
        ScaleTier::Project1M,
    ];

    /// 안정 이름(content hash 입력·CLI 인자).
    pub fn name(self) -> &'static str {
        match self {
            ScaleTier::Nodes10k => "nodes-10k",
            ScaleTier::Nodes200k => "nodes-200k",
            ScaleTier::Project1M => "project-1m",
        }
    }

    /// 이름으로 찾습니다.
    pub fn from_name(name: &str) -> Option<ScaleTier> {
        ScaleTier::ALL.into_iter().find(|t| t.name() == name)
    }

    /// 문서별 node 수(생성 순).
    pub fn document_nodes(self) -> Vec<usize> {
        match self {
            ScaleTier::Nodes10k => vec![10_000],
            ScaleTier::Nodes200k => vec![200_000],
            ScaleTier::Project1M => {
                let mut nodes = vec![200_000];
                nodes.extend(std::iter::repeat_n(100_000, 8));
                nodes
            }
        }
    }
}

/// tree 모양 매개변수.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeShape {
    /// View 하나의 목표 node 수(마지막 View 등은 나머지만큼 달라질 수 있음).
    pub nodes_per_view: usize,
    /// 페이지 하나의 View 수.
    pub views_per_page: usize,
    /// 최대 depth(root = 0). `1..=MAX_NODE_DEPTH`.
    pub max_depth: u32,
    /// container 자식 수 하한(1 이상).
    pub min_fanout: u32,
    /// container 자식 수 상한(하한 이상).
    pub max_fanout: u32,
}

impl TreeShape {
    /// 기본 모양: View당 500 node, 페이지당 20 View, depth ≤ 10, fan-out 4..=12.
    pub const DEFAULT: TreeShape = TreeShape {
        nodes_per_view: 500,
        views_per_page: 20,
        max_depth: 10,
        min_fanout: 4,
        max_fanout: 12,
    };

    fn validate(&self) -> Result<(), FixtureError> {
        let reason = if self.nodes_per_view == 0 {
            Some("nodes_per_view는 1 이상이어야 합니다")
        } else if self.views_per_page == 0 {
            Some("views_per_page는 1 이상이어야 합니다")
        } else if self.max_depth == 0 || self.max_depth > MAX_NODE_DEPTH {
            Some("max_depth는 1..=64여야 합니다")
        } else if self.min_fanout == 0 || self.max_fanout < self.min_fanout {
            Some("fan-out 범위가 잘못됐습니다")
        } else {
            None
        };
        match reason {
            Some(reason) => Err(FixtureError::InvalidSpec { reason }),
            None => Ok(()),
        }
    }
}

impl Default for TreeShape {
    fn default() -> TreeShape {
        TreeShape::DEFAULT
    }
}

/// synthetic fixture 명세.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntheticSpec {
    /// fixture 이름(tier 이름 또는 CI smoke 등 사용자 지정 이름).
    pub name: String,
    /// seed.
    pub seed: u64,
    /// 문서별 node 수.
    pub document_nodes: Vec<usize>,
    /// tree 모양.
    pub shape: TreeShape,
}

impl SyntheticSpec {
    /// tier 명세.
    pub fn tier(tier: ScaleTier, seed: u64) -> SyntheticSpec {
        SyntheticSpec {
            name: tier.name().to_string(),
            seed,
            document_nodes: tier.document_nodes(),
            shape: TreeShape::DEFAULT,
        }
    }

    /// 사용자 지정 규모(CI smoke 변형 등). 이름이 content hash에 들어갑니다.
    pub fn custom(name: &str, seed: u64, document_nodes: Vec<usize>) -> SyntheticSpec {
        SyntheticSpec {
            name: name.to_string(),
            seed,
            document_nodes,
            shape: TreeShape::DEFAULT,
        }
    }

    /// node 총수.
    pub fn total_nodes(&self) -> usize {
        self.document_nodes.iter().sum()
    }

    fn validate(&self) -> Result<(), FixtureError> {
        self.shape.validate()?;
        let reason = if self.document_nodes.is_empty() {
            Some("문서가 하나 이상이어야 합니다")
        } else if self.document_nodes.len() > MAX_DOCUMENTS {
            Some("문서 수가 상한(64)을 넘습니다")
        } else if self.document_nodes.contains(&0) {
            Some("문서 node 수는 1 이상이어야 합니다")
        } else if self
            .document_nodes
            .iter()
            .try_fold(0usize, |acc, n| acc.checked_add(*n))
            .is_none_or(|total| total > MAX_TOTAL_NODES)
        {
            Some("node 총수가 상한(500만)을 넘습니다")
        } else {
            None
        };
        match reason {
            Some(reason) => Err(FixtureError::InvalidSpec { reason }),
            None => Ok(()),
        }
    }
}

/// node 종류별 수.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KindCounts {
    /// Frame.
    pub frame: usize,
    /// Fragment.
    pub fragment: usize,
    /// Shape.
    pub shape: usize,
    /// Path.
    pub path: usize,
    /// Text.
    pub text: usize,
    /// Image.
    pub image: usize,
}

impl KindCounts {
    /// 한 node를 셉니다.
    pub fn add(&mut self, kind: NodeKind) {
        match kind {
            NodeKind::Frame => self.frame += 1,
            NodeKind::Fragment => self.fragment += 1,
            NodeKind::Shape => self.shape += 1,
            NodeKind::Path => self.path += 1,
            NodeKind::Text => self.text += 1,
            NodeKind::Image => self.image += 1,
            NodeKind::ComponentInstance => {}
        }
    }

    /// 합계.
    pub fn total(&self) -> usize {
        self.frame + self.fragment + self.shape + self.path + self.text + self.image
    }
}

/// 생성된 문서 하나의 통계.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DocumentStats {
    /// node 수.
    pub nodes: usize,
    /// 페이지 수.
    pub pages: usize,
    /// View 수.
    pub views: usize,
    /// modifier stack을 가진 node 수.
    pub modifier_stacks: usize,
    /// modifier 항목 수.
    pub modifiers: usize,
    /// node→node 참조(Mask) 수.
    pub node_references: usize,
    /// 최대 depth(root = 0).
    pub max_depth: u32,
    /// 종류별 node 수.
    pub kinds: KindCounts,
}

/// 생성된 문서.
#[derive(Debug)]
pub struct GeneratedDocument {
    /// 명세 안 문서 순번(0부터).
    pub index: usize,
    /// 런타임 모델과 chunk model.
    pub built: BuiltDocument,
    /// 통계.
    pub stats: DocumentStats,
}

/// 명세 하나의 생성 결과 요약.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntheticSummary {
    /// fixture 이름.
    pub name: String,
    /// seed.
    pub seed: u64,
    /// generator version.
    pub generator_version: u32,
    /// 문서 수.
    pub documents: usize,
    /// node 총수.
    pub nodes: usize,
    /// hash에 넣은 chunk·resource 수.
    pub chunks: u64,
    /// chunk payload bytes 합.
    pub payload_bytes: u64,
    /// 문서별 통계(생성 순).
    pub document_stats: Vec<DocumentStats>,
    /// fixture content hash.
    pub content_hash: ContentHash,
}

/// 명세의 모든 문서를 만들고 요약을 반환합니다.
///
/// 문서는 하나씩 만들어 `visit`에 넘긴 뒤 버립니다(프로젝트 tier의 메모리 상한). `visit`이
/// 오류를 내면 즉시 중단합니다. content hash는 각 문서의 chunk model과 프로젝트 문서
/// index(모든 문서 정의)의 canonical bytes로 계산합니다.
pub fn generate_synthetic<F>(
    spec: &SyntheticSpec,
    mut visit: F,
) -> Result<SyntheticSummary, FixtureError>
where
    F: FnMut(&GeneratedDocument) -> Result<(), FixtureError>,
{
    spec.validate()?;
    let mut digest = FixtureDigest::new(&spec.name, spec.seed);
    digest.parameter("documents", spec.document_nodes.len() as u64);
    for nodes in &spec.document_nodes {
        digest.parameter("document_nodes", *nodes as u64);
    }
    digest.parameter("nodes_per_view", spec.shape.nodes_per_view as u64);
    digest.parameter("views_per_page", spec.shape.views_per_page as u64);
    digest.parameter("max_depth", u64::from(spec.shape.max_depth));
    digest.parameter("min_fanout", u64::from(spec.shape.min_fanout));
    digest.parameter("max_fanout", u64::from(spec.shape.max_fanout));

    let mut master = SplitMix64::new(spec.seed ^ 0x5717_E7A1_C0DE_0001);
    let mut ids = IdAllocator::new(spec.seed);
    let mut index = ProjectDocumentIndex::new();
    let mut document_stats = Vec::with_capacity(spec.document_nodes.len());
    for (i, nodes) in spec.document_nodes.iter().enumerate() {
        let mut rng = master.fork(i as u64);
        let (built, stats) = generate_document(&mut rng, &mut ids, *nodes, &spec.shape)?;
        digest.document(&built.serialized);
        index
            .add_document(built.model.definition().clone())
            .map_err(FixtureError::Definition)?;
        let generated = GeneratedDocument {
            index: i,
            built,
            stats,
        };
        visit(&generated)?;
        document_stats.push(stats);
    }
    index
        .validate_ownership()
        .map_err(FixtureError::Definition)?;
    digest.resource("project-document-index", StableId::NIL, &index.encode());
    Ok(SyntheticSummary {
        name: spec.name.clone(),
        seed: spec.seed,
        generator_version: crate::GENERATOR_VERSION,
        documents: spec.document_nodes.len(),
        nodes: document_stats.iter().map(|s| s.nodes).sum(),
        chunks: digest.chunk_count(),
        payload_bytes: digest.payload_bytes(),
        document_stats,
        content_hash: digest.finish(),
    })
}

/// 아직 자식을 다 만들지 않은 container.
struct Pending {
    id: StableId,
    depth: u32,
    /// 이 container 아래에 만들 후손 node 수.
    budget: usize,
    w: Quarter,
    h: Quarter,
}

/// 화면(View root) 크기: 1440 × 1024 px.
const SCREEN_W: Quarter = 1440 * 4;
const SCREEN_H: Quarter = 1024 * 4;

/// asset pool 크기(문서당). Image node가 이 중 하나를 참조합니다.
const ASSET_POOL: usize = 32;

/// leaf 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Leaf {
    Shape,
    Text,
    Path,
    Image,
    Frame,
    Fragment,
}

const LEAF_WEIGHTS: [(Leaf, u32); 6] = [
    (Leaf::Shape, 30),
    (Leaf::Text, 25),
    (Leaf::Path, 15),
    (Leaf::Image, 10),
    (Leaf::Frame, 15),
    (Leaf::Fragment, 5),
];

const MODIFIER_WEIGHTS: [(ModifierKind, u32); 9] = [
    (ModifierKind::Fill, 4),
    (ModifierKind::Stroke, 3),
    (ModifierKind::Shadow, 2),
    (ModifierKind::Opacity, 2),
    (ModifierKind::Blur, 1),
    (ModifierKind::Corner, 1),
    (ModifierKind::Transform, 1),
    (ModifierKind::Blend, 1),
    (ModifierKind::Mask, 1),
];

/// `extra`를 `fan`개 몫으로 나눕니다(합 = extra). 약 절반은 0(leaf)이 되지만, extra > 0이면
/// 적어도 하나는 0보다 큽니다.
fn distribute(rng: &mut SplitMix64, extra: usize, fan: usize) -> Vec<usize> {
    let mut shares = vec![0usize; fan];
    if extra == 0 || fan == 0 {
        return shares;
    }
    let mut weights: Vec<u64> = (0..fan)
        .map(|_| if rng.one_in(2) { 0 } else { rng.range(1, 8) })
        .collect();
    if weights.iter().all(|w| *w == 0) {
        let at = rng.below(fan as u64) as usize;
        weights[at] = 1;
    }
    let total: u64 = weights.iter().sum();
    let mut assigned = 0usize;
    for (share, weight) in shares.iter_mut().zip(&weights) {
        *share = ((extra as u128 * u128::from(*weight)) / u128::from(total)) as usize;
        assigned += *share;
    }
    // 나머지는 가중치가 있는 몫에 차례로 1씩.
    let mut remainder = extra - assigned;
    let mut i = 0usize;
    while remainder > 0 {
        if weights[i % fan] > 0 {
            shares[i % fan] += 1;
            remainder -= 1;
        }
        i += 1;
    }
    shares
}

/// 문서 하나를 만듭니다(정확히 `nodes`개 node).
fn generate_document(
    rng: &mut SplitMix64,
    ids: &mut IdAllocator,
    nodes: usize,
    shape: &TreeShape,
) -> Result<(BuiltDocument, DocumentStats), FixtureError> {
    let views = nodes.div_ceil(shape.nodes_per_view);
    let pages = views.div_ceil(shape.views_per_page);
    let document_id = ids.next_id();
    let mut definition = DocumentDefinition::new(document_id, "Synthetic".to_string());
    let mut roots: Vec<StableId> = Vec::with_capacity(views);
    for p in 0..pages {
        let mut page = PageDefinition::new(ids.next_id(), format!("Page {}", p + 1));
        let first = p * shape.views_per_page;
        let last = (first + shape.views_per_page).min(views);
        for v in first..last {
            let root = ids.next_id();
            page.add_view(ViewDefinition::new(
                ids.next_id(),
                format!("Screen {}", v + 1),
                root,
            ))
            .map_err(FixtureError::Definition)?;
            roots.push(root);
        }
        definition
            .add_page(page)
            .map_err(FixtureError::Definition)?;
    }
    let assets: Vec<AssetRef> = (0..ASSET_POOL)
        .map(|_| AssetRef::new(ids.next_id()))
        .collect();

    let mut records = DocumentRecords::new(definition);
    records.nodes.reserve(nodes);
    let mut stats = DocumentStats {
        pages,
        views,
        ..DocumentStats::default()
    };
    let base = nodes / views;
    let remainder = nodes % views;
    for (v, root) in roots.into_iter().enumerate() {
        let budget = base + usize::from(v < remainder);
        build_view(
            rng,
            ids,
            &assets,
            root,
            v,
            budget,
            shape,
            &mut records,
            &mut stats,
        );
    }
    let built = finish_document(records)?;
    Ok((built, stats))
}

/// View 하나(root 포함 `budget`개 node)를 만듭니다.
#[allow(clippy::too_many_arguments)]
fn build_view(
    rng: &mut SplitMix64,
    ids: &mut IdAllocator,
    assets: &[AssetRef],
    root: StableId,
    view_index: usize,
    budget: usize,
    shape: &TreeShape,
    records: &mut DocumentRecords,
    stats: &mut DocumentStats,
) {
    // 화면은 캔버스에 10열 격자로 배치합니다.
    let col = (view_index % 10) as Quarter;
    let row = (view_index / 10) as Quarter;
    let root_payload = build::frame(
        rng,
        SCREEN_W,
        SCREEN_H,
        col * (SCREEN_W + 160 * 4),
        row * (SCREEN_H + 160 * 4),
    );
    records.nodes.push(build::record(
        root,
        None,
        OrderKey::between(None, None),
        true,
        "Screen",
        NodePayload::Frame(root_payload),
    ));
    stats.nodes += 1;
    stats.kinds.add(NodeKind::Frame);

    // Mask 대상 후보: 이 View에서 최근에 만든 node.
    let mut recent: Vec<StableId> = Vec::with_capacity(16);
    let mut pending = vec![Pending {
        id: root,
        depth: 0,
        budget: budget.saturating_sub(1),
        w: SCREEN_W,
        h: SCREEN_H,
    }];
    while let Some(parent) = pending.pop() {
        if parent.budget == 0 {
            continue;
        }
        let fan = if parent.depth + 1 >= shape.max_depth {
            parent.budget
        } else {
            (rng.range(u64::from(shape.min_fanout), u64::from(shape.max_fanout)) as usize)
                .min(parent.budget)
        };
        let shares = distribute(rng, parent.budget - fan, fan);
        let keys = OrderKey::evenly_spaced(fan);
        let depth = parent.depth + 1;
        stats.max_depth = stats.max_depth.max(depth);
        for (share, order_key) in shares.into_iter().zip(keys) {
            let id = ids.next_id();
            // 자식 상자: 부모의 1/4..=1 크기, 부모 안쪽 위치.
            let w = (parent.w / 4).max(4) + build::quarter_below(rng, parent.w - parent.w / 4);
            let h = (parent.h / 4).max(4) + build::quarter_below(rng, parent.h - parent.h / 4);
            let w = w.min(parent.w.max(4));
            let h = h.min(parent.h.max(4));
            let x = build::quarter_below(rng, parent.w.saturating_sub(w));
            let y = build::quarter_below(rng, parent.h.saturating_sub(h));
            let payload = if share > 0 {
                pending.push(Pending {
                    id,
                    depth,
                    budget: share,
                    w,
                    h,
                });
                if rng.one_in(10) {
                    NodePayload::Fragment(FragmentPayload)
                } else {
                    NodePayload::Frame(build::frame(rng, w, h, x, y))
                }
            } else {
                match rng.weighted(&LEAF_WEIGHTS) {
                    Leaf::Shape => NodePayload::Shape(build::shape(rng, w, h)),
                    Leaf::Text => {
                        let words = rng.range(1, 12) as usize;
                        NodePayload::Text(build::text(rng, words, 6))
                    }
                    Leaf::Path => NodePayload::Path(build::path(rng, ids, w, h)),
                    Leaf::Image => {
                        let asset = assets[rng.below(assets.len() as u64) as usize];
                        NodePayload::Image(build::image(rng, asset, w, h))
                    }
                    Leaf::Frame => NodePayload::Frame(build::frame(rng, w, h, x, y)),
                    Leaf::Fragment => NodePayload::Fragment(FragmentPayload),
                }
            };
            let kind = payload.kind();
            let name = rng.pick(&build::NAME_POOL);
            let mut node = build::record(
                id,
                Some(parent.id),
                order_key,
                !rng.one_in(20),
                name,
                payload,
            );
            node.locked = rng.one_in(40);
            records.nodes.push(node);
            stats.nodes += 1;
            stats.kinds.add(kind);

            if kind != NodeKind::Fragment && rng.one_in(5) {
                let count = rng.range(1, 3) as usize;
                let mut payloads = Vec::with_capacity(count);
                for _ in 0..count {
                    let modifier = rng.weighted(&MODIFIER_WEIGHTS);
                    if modifier == ModifierKind::Mask {
                        if recent.is_empty() {
                            payloads.push(build::plain_modifier(rng, ModifierKind::Fill));
                        } else {
                            let target = recent[rng.below(recent.len() as u64) as usize];
                            payloads.push(ModifierPayload::Mask {
                                source: NodeRef::new(target),
                            });
                            stats.node_references += 1;
                        }
                    } else {
                        payloads.push(build::plain_modifier(rng, modifier));
                    }
                }
                stats.modifier_stacks += 1;
                stats.modifiers += payloads.len();
                records
                    .modifier_stacks
                    .push(build::modifier_stack(ids, id, payloads));
            }
            if recent.len() == 16 {
                recent.remove(0);
            }
            recent.push(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distribute_sums_to_extra() {
        let mut rng = SplitMix64::new(3);
        for extra in [0usize, 1, 7, 100, 4_999] {
            for fan in [1usize, 4, 12] {
                let shares = distribute(&mut rng, extra, fan);
                assert_eq!(shares.len(), fan);
                assert_eq!(shares.iter().sum::<usize>(), extra);
                if extra > 0 {
                    assert!(shares.iter().any(|s| *s > 0));
                }
            }
        }
    }

    #[test]
    fn tier_node_counts_match_architecture_targets() {
        assert_eq!(ScaleTier::Nodes10k.document_nodes(), vec![10_000]);
        assert_eq!(ScaleTier::Nodes200k.document_nodes(), vec![200_000]);
        let project = ScaleTier::Project1M.document_nodes();
        assert_eq!(project.iter().sum::<usize>(), 1_000_000);
        assert!(project.len() <= 10, "동시에 열린 문서 상한 10");
        assert!(project.iter().all(|n| *n <= 200_000), "활성 문서 상한 20만");
        for tier in ScaleTier::ALL {
            assert_eq!(ScaleTier::from_name(tier.name()), Some(tier));
        }
        assert_eq!(ScaleTier::from_name("nodes-1"), None);
    }

    #[test]
    fn invalid_specs_are_rejected_before_generation() {
        let reject = |spec: SyntheticSpec| {
            matches!(
                generate_synthetic(&spec, |_| Ok(())),
                Err(FixtureError::InvalidSpec { .. })
            )
        };
        assert!(reject(SyntheticSpec::custom("x", 1, vec![])));
        assert!(reject(SyntheticSpec::custom("x", 1, vec![0])));
        assert!(reject(SyntheticSpec::custom(
            "x",
            1,
            vec![MAX_TOTAL_NODES + 1]
        )));
        assert!(reject(SyntheticSpec::custom("x", 1, vec![usize::MAX, 2])));
        assert!(reject(SyntheticSpec::custom(
            "x",
            1,
            vec![1; MAX_DOCUMENTS + 1]
        )));
        let mut spec = SyntheticSpec::custom("x", 1, vec![10]);
        spec.shape.max_depth = MAX_NODE_DEPTH + 1;
        assert!(reject(spec.clone()));
        spec.shape = TreeShape {
            min_fanout: 5,
            max_fanout: 4,
            ..TreeShape::DEFAULT
        };
        assert!(reject(spec.clone()));
        spec.shape = TreeShape {
            nodes_per_view: 0,
            ..TreeShape::DEFAULT
        };
        assert!(reject(spec));
    }
}
