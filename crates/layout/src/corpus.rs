//! 레이아웃 적합성 fixture corpus: schema, 결정적 생성기, runner (X.11.1).
//!
//! # schema (version [`FIXTURE_SCHEMA_VERSION`])
//!
//! fixture 하나는 [`LayoutFixture`] `{ name, mode, seed, input }`입니다.
//!
//! - `name`은 `"<mode>-<3자리 순번>"`(예: `stack-007`)이며 corpus 안에서 유일합니다.
//! - `mode`는 root container 배치 방식과 같아야 합니다(Free·Stack·Grid·Overlay·Constraint).
//! - `input`은 [`LayoutInput::validate`]를 통과하고, canonical bytes 왕복이 같은 bytes를 내며, node 수는
//!   [`MAX_FIXTURE_NODES`] 이하입니다.
//! - 배치 방식마다 [`MIN_FIXTURES_PER_MODE`]개 이상 있어야 합니다.
//!
//! # 결정적 생성
//!
//! [`generate_corpus`]는 고정 seed의 SplitMix64로 입력을 만듭니다. 길이는 1/4 px 배수(정확한 f32),
//! StableId·OrderKey도 seed에서 결정적으로 만들고, 자식 삽입 순서는 섞어서 visual order 규칙을 함께
//! 검사합니다. 같은 `per_mode`는 항상 같은 corpus(같은 bytes)를 만듭니다. 생성 입력은 레이아웃 오류가
//! 나지 않도록(과다 제약·순환 없음) 만들지만, 진단(미해결 sibling·underdetermined)은 일부 포함합니다.
//!
//! # runner
//!
//! [`run_fixture`]는 레이아웃을 계산해 결과 canonical bytes의 hash([`FixtureOutcome::digest`])를,
//! [`run_corpus`]는 배치 방식별 fixture 수·box 수·진단 수·digest 묶음을 냅니다. 저장소의 작은 golden
//! (`crates/layout/tests/fixtures/layout_corpus_v1.golden`)이 이 요약을 고정합니다.

use std::collections::BTreeSet;
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::value::{Insets, Length, Percentage, Scalar, Transform2D};

use crate::cache::{LayoutInputHash, LayoutInputHasher};
use crate::constraint::{AnchorEdge, AnchorTarget, Constraint};
use crate::engine::compute_layout;
use crate::error::LayoutError;
use crate::flex::{FlexBasis, FlexItem};
use crate::free::{AxisAnchors, Offset};
use crate::geometry::Axis;
use crate::grid::{GridItem, GridLayout, GridPlacement, TrackBreadth, TrackSize};
use crate::input::{
    ContainerLayout, IntrinsicContent, LayoutContent, LayoutInput, LayoutNode, SizingMode,
};
use crate::overlay::{OverlayAlignment, OverlayLayout};
use crate::sizing::{AspectRatio, SizeLimits};
use crate::stack::{
    CrossAlignment, LineAlignment, MainAlignment, StackDirection, StackLayout, StackWrap,
};

/// fixture schema version.
pub const FIXTURE_SCHEMA_VERSION: u16 = 1;
/// 배치 방식별 최소 fixture 수.
pub const MIN_FIXTURES_PER_MODE: usize = 50;
/// fixture 하나의 최대 node 수.
pub const MAX_FIXTURE_NODES: usize = 64;

/// corpus 배치 방식 (X.11.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CorpusMode {
    /// 자유 배치.
    Free,
    /// Stack.
    Stack,
    /// Grid.
    Grid,
    /// Overlay.
    Overlay,
    /// Constraint.
    Constraint,
}

impl CorpusMode {
    /// 모든 배치 방식(golden 순서).
    pub const ALL: [CorpusMode; 5] = [
        CorpusMode::Free,
        CorpusMode::Stack,
        CorpusMode::Grid,
        CorpusMode::Overlay,
        CorpusMode::Constraint,
    ];

    /// 이름(fixture name 접두사).
    pub const fn name(self) -> &'static str {
        match self {
            CorpusMode::Free => "free",
            CorpusMode::Stack => "stack",
            CorpusMode::Grid => "grid",
            CorpusMode::Overlay => "overlay",
            CorpusMode::Constraint => "constraint",
        }
    }

    /// root container가 이 배치 방식인지.
    pub fn matches(self, container: &ContainerLayout) -> bool {
        matches!(
            (self, container),
            (CorpusMode::Free, ContainerLayout::Free)
                | (CorpusMode::Stack, ContainerLayout::Stack(_))
                | (CorpusMode::Grid, ContainerLayout::Grid(_))
                | (CorpusMode::Overlay, ContainerLayout::Overlay(_))
                | (CorpusMode::Constraint, ContainerLayout::Constraint)
        )
    }
}

impl fmt::Display for CorpusMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// fixture 하나 (X.11.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutFixture {
    /// `"<mode>-<순번>"`.
    pub name: String,
    /// 배치 방식.
    pub mode: CorpusMode,
    /// 생성 seed.
    pub seed: u64,
    /// 레이아웃 입력.
    pub input: LayoutInput,
}

/// fixture schema 위반.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixtureSchemaError {
    /// 이름 형식이 `"<mode>-<3자리 순번>"`이 아닙니다.
    BadName {
        /// fixture 이름.
        name: String,
    },
    /// 이름이 중복되었습니다.
    DuplicateName {
        /// fixture 이름.
        name: String,
    },
    /// root container가 mode와 다릅니다.
    ModeMismatch {
        /// fixture 이름.
        name: String,
    },
    /// node 수가 상한을 넘습니다.
    TooManyNodes {
        /// fixture 이름.
        name: String,
        /// node 수.
        count: usize,
    },
    /// 입력 검증 실패.
    InvalidInput {
        /// fixture 이름.
        name: String,
        /// 원인.
        error: LayoutError,
    },
    /// canonical bytes 왕복이 같은 bytes를 내지 않습니다.
    NotCanonical {
        /// fixture 이름.
        name: String,
    },
    /// 배치 방식의 fixture 수가 최소보다 적습니다.
    TooFewFixtures {
        /// 배치 방식.
        mode: CorpusMode,
        /// fixture 수.
        count: usize,
    },
}

impl fmt::Display for FixtureSchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FixtureSchemaError::BadName { name } => write!(f, "fixture 이름 형식 오류: {name}"),
            FixtureSchemaError::DuplicateName { name } => write!(f, "fixture 이름 중복: {name}"),
            FixtureSchemaError::ModeMismatch { name } => {
                write!(f, "fixture {name}의 root 배치 방식이 mode와 다릅니다")
            }
            FixtureSchemaError::TooManyNodes { name, count } => {
                write!(f, "fixture {name}의 node 수 {count}가 상한을 넘습니다")
            }
            FixtureSchemaError::InvalidInput { name, error } => {
                write!(f, "fixture {name} 입력 오류: {error}")
            }
            FixtureSchemaError::NotCanonical { name } => {
                write!(f, "fixture {name}의 canonical bytes 왕복이 다릅니다")
            }
            FixtureSchemaError::TooFewFixtures { mode, count } => {
                write!(f, "{mode} fixture가 {count}개로 최소 개수보다 적습니다")
            }
        }
    }
}

impl std::error::Error for FixtureSchemaError {}

/// fixture 하나의 schema를 검증합니다.
pub fn validate_fixture(fixture: &LayoutFixture) -> Result<(), FixtureSchemaError> {
    let name = fixture.name.clone();
    let prefix = format!("{}-", fixture.mode.name());
    let well_formed = fixture
        .name
        .strip_prefix(&prefix)
        .is_some_and(|digits| digits.len() == 3 && digits.bytes().all(|b| b.is_ascii_digit()));
    if !well_formed {
        return Err(FixtureSchemaError::BadName { name });
    }
    if fixture.input.nodes.len() > MAX_FIXTURE_NODES {
        return Err(FixtureSchemaError::TooManyNodes {
            name,
            count: fixture.input.nodes.len(),
        });
    }
    fixture
        .input
        .validate()
        .map_err(|error| FixtureSchemaError::InvalidInput {
            name: name.clone(),
            error,
        })?;
    let root = fixture
        .input
        .node(fixture.input.root)
        .ok_or_else(|| FixtureSchemaError::ModeMismatch { name: name.clone() })?;
    if !fixture.mode.matches(&root.container) {
        return Err(FixtureSchemaError::ModeMismatch { name });
    }
    let bytes = fixture.input.encode();
    match LayoutInput::decode(&bytes) {
        Ok(decoded) if decoded.encode() == bytes && decoded == fixture.input => Ok(()),
        _ => Err(FixtureSchemaError::NotCanonical { name }),
    }
}

/// corpus 전체 schema를 검증합니다(fixture별 검증, 이름 유일성, 배치 방식별 최소 개수).
pub fn validate_corpus(fixtures: &[LayoutFixture]) -> Result<(), FixtureSchemaError> {
    let mut names = BTreeSet::new();
    for fixture in fixtures {
        validate_fixture(fixture)?;
        if !names.insert(fixture.name.clone()) {
            return Err(FixtureSchemaError::DuplicateName {
                name: fixture.name.clone(),
            });
        }
    }
    for mode in CorpusMode::ALL {
        let count = fixtures.iter().filter(|f| f.mode == mode).count();
        if count < MIN_FIXTURES_PER_MODE {
            return Err(FixtureSchemaError::TooFewFixtures { mode, count });
        }
    }
    Ok(())
}

/// fixture 실행 결과 요약.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureOutcome {
    /// fixture 이름.
    pub name: String,
    /// 결과 canonical bytes의 hash.
    pub digest: LayoutInputHash,
    /// 결과 box 수.
    pub boxes: usize,
    /// 진단 수.
    pub diagnostics: usize,
}

/// fixture 하나를 계산합니다.
pub fn run_fixture(fixture: &LayoutFixture) -> Result<FixtureOutcome, LayoutError> {
    let result = compute_layout(&fixture.input)?;
    let mut hasher = LayoutInputHasher::new();
    hasher.write_bytes(&result.encode());
    Ok(FixtureOutcome {
        name: fixture.name.clone(),
        digest: hasher.finish(),
        boxes: result.len(),
        diagnostics: result.diagnostics().len(),
    })
}

/// 배치 방식 하나의 corpus 실행 요약.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeSummary {
    /// 배치 방식.
    pub mode: CorpusMode,
    /// fixture 수.
    pub fixtures: usize,
    /// box 수 합.
    pub boxes: usize,
    /// 진단 수 합.
    pub diagnostics: usize,
    /// fixture digest를 순서대로 묶은 hash.
    pub digest: LayoutInputHash,
}

impl fmt::Display for ModeSummary {
    /// golden 한 줄: `<mode> fixtures=<n> boxes=<n> diagnostics=<n> digest=<primary>-<check>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} fixtures={} boxes={} diagnostics={} digest={:016x}-{:016x}",
            self.mode,
            self.fixtures,
            self.boxes,
            self.diagnostics,
            self.digest.raw(),
            self.digest.check()
        )
    }
}

/// corpus 전체를 실행해 배치 방식별 요약을 냅니다(`CorpusMode::ALL` 순서).
pub fn run_corpus(fixtures: &[LayoutFixture]) -> Result<Vec<ModeSummary>, LayoutError> {
    let mut summaries = Vec::new();
    for mode in CorpusMode::ALL {
        let mut hasher = LayoutInputHasher::new();
        let mut summary = ModeSummary {
            mode,
            fixtures: 0,
            boxes: 0,
            diagnostics: 0,
            digest: LayoutInputHash::from_raw(0),
        };
        for fixture in fixtures.iter().filter(|f| f.mode == mode) {
            let outcome = run_fixture(fixture)?;
            hasher.write_bytes(outcome.name.as_bytes());
            hasher.write_hash(outcome.digest);
            summary.fixtures += 1;
            summary.boxes += outcome.boxes;
            summary.diagnostics += outcome.diagnostics;
        }
        summary.digest = hasher.finish();
        summaries.push(summary);
    }
    Ok(summaries)
}

/// corpus 입력 전체(canonical bytes)의 hash. 생성기 변경을 감지합니다.
pub fn corpus_input_digest(fixtures: &[LayoutFixture]) -> LayoutInputHash {
    let mut hasher = LayoutInputHasher::new();
    for fixture in fixtures {
        hasher.write_bytes(fixture.name.as_bytes());
        hasher.write_bytes(&fixture.input.encode());
    }
    hasher.finish()
}

// --- 결정적 생성기 ---

/// SplitMix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// `lo..=hi` 정수.
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        let span = u64::try_from(i64::from(hi) - i64::from(lo) + 1).unwrap_or(1);
        lo + i32::try_from(self.next() % span).unwrap_or(0)
    }

    /// `percent`% 확률.
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }

    /// `lo..=hi` px의 1/4 px 배수 길이.
    fn length(&mut self, lo: i32, hi: i32) -> Length {
        quarter(self.range(lo * 4, hi * 4))
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        let index = usize::try_from(self.next() % items.len() as u64).unwrap_or(0);
        items[index]
    }
}

/// `quarters / 4` px(정확한 f32).
fn quarter(quarters: i32) -> Length {
    Length::new(quarters as f32 / 4.0)
        .unwrap_or(Length::from_raw(stellar_core_types::value::FiniteF32::ZERO))
}

fn scalar(quarters: i32) -> Scalar {
    Scalar::new(quarters as f32 / 4.0)
        .unwrap_or(Scalar::from_raw(stellar_core_types::value::FiniteF32::ZERO))
}

fn percent(value: i32) -> Percentage {
    Percentage::new(value as f32).unwrap_or(Percentage::from_raw(
        stellar_core_types::value::FiniteF32::ZERO,
    ))
}

/// fixture 안 node 생성 상태.
struct Builder {
    rng: Rng,
    seed: u64,
    nodes: Vec<LayoutNode>,
}

impl Builder {
    fn new(seed: u64) -> Builder {
        Builder {
            rng: Rng(seed),
            seed,
            nodes: Vec::new(),
        }
    }

    /// 결정적 StableId(n번째 node).
    fn id(&self, n: usize) -> StableId {
        let n = n as u64;
        StableId::new_v7(
            1_700_000_000_000 + n,
            (self.seed & 0x0fff) as u16,
            self.seed ^ (n << 32),
        )
    }

    /// n번째 형제 OrderKey(같은 길이, 후행 1).
    fn key(n: usize) -> OrderKey {
        let [_, hi, mid, lo] = (n as u32 + 1).to_be_bytes();
        OrderKey::from_bytes(vec![hi, mid, lo, 1]).unwrap_or_else(|_| OrderKey::between(None, None))
    }

    /// 새 node를 등록하고 index를 반환합니다.
    fn push(&mut self, node: LayoutNode) -> usize {
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    /// 부모에 자식들을 OrderKey 순번대로 붙이되 삽입 순서는 섞습니다.
    fn attach(&mut self, parent: usize, children: &[usize]) {
        let mut order: Vec<usize> = (0..children.len()).collect();
        for i in (1..order.len()).rev() {
            let j = usize::try_from(self.rng.next() % (i as u64 + 1)).unwrap_or(0);
            order.swap(i, j);
        }
        for position in order {
            let child_id = self.nodes[children[position]].id;
            self.nodes[parent].children.push(crate::input::LayoutChild {
                id: child_id,
                order_key: Builder::key(position),
            });
        }
    }

    fn intrinsic(&mut self) -> LayoutContent {
        let width = self.rng.length(2, 80);
        let height = self.rng.length(2, 40);
        let baseline = if self.rng.chance(50) {
            Some(self.rng.length(1, 30))
        } else {
            None
        };
        LayoutContent::Intrinsic(IntrinsicContent {
            width,
            height,
            baseline,
        })
    }

    /// 무작위 sizing(Fill 허용 여부). HugContent면 intrinsic 콘텐츠를 붙일 수 있게 표시합니다.
    fn sizing(&mut self, allow_fill: bool) -> SizingMode {
        match self.rng.range(0, if allow_fill { 3 } else { 2 }) {
            0 => SizingMode::Fixed(self.rng.length(0, 120)),
            1 => SizingMode::HugContent,
            2 => SizingMode::Percentage(percent(self.rng.range(5, 100))),
            _ => SizingMode::Fill,
        }
    }

    fn padding(&mut self) -> Insets {
        let mut v = [0.0f32; 4];
        for value in &mut v {
            *value = self.rng.range(0, 48) as f32 / 4.0;
        }
        Insets::new(v[0], v[1], v[2], v[3])
            .unwrap_or_else(|_| Insets::new(0.0, 0.0, 0.0, 0.0).expect("0은 유한"))
    }

    fn limits(&mut self) -> SizeLimits {
        let mut limits = SizeLimits::default();
        if self.rng.chance(25) {
            let min = self.rng.range(0, 40);
            let max = min + self.rng.range(0, 80);
            limits.min_width = Some(quarter(min * 4));
            limits.max_width = Some(quarter(max * 4));
        }
        if self.rng.chance(25) {
            let min = self.rng.range(0, 30);
            limits.min_height = Some(quarter(min * 4));
        }
        limits
    }

    /// leaf 자식 하나(Hug 축이 있으면 intrinsic 콘텐츠).
    fn leaf(&mut self, n: usize, allow_fill: bool) -> LayoutNode {
        let width = self.sizing(allow_fill);
        let height = self.sizing(allow_fill);
        let mut node = LayoutNode::new(self.id(n))
            .with_sizing(width, height)
            .with_limits(self.limits());
        if (width == SizingMode::HugContent || height == SizingMode::HugContent)
            && self.rng.chance(80)
        {
            node = node.with_content(self.intrinsic());
        }
        if self.rng.chance(10) {
            node = node.with_visible(false);
        }
        if self.rng.chance(15) {
            let a = self.rng.range(-2, 2) as f32;
            node = node.with_transform(
                Transform2D::new(a, 1.0, -1.0, a, 3.0, -2.0)
                    .unwrap_or_else(|_| Transform2D::identity()),
            );
        }
        if self.rng.chance(10)
            && (width != SizingMode::HugContent || height != SizingMode::HugContent)
        {
            let ratio =
                AspectRatio::new(scalar(self.rng.range(4, 16)), scalar(self.rng.range(4, 16)));
            node = node.with_aspect_ratio(ratio);
        }
        node
    }

    fn root(&mut self, container: ContainerLayout) -> usize {
        let width = if self.rng.chance(80) {
            SizingMode::Fixed(self.rng.length(80, 400))
        } else {
            SizingMode::HugContent
        };
        let height = if self.rng.chance(80) {
            SizingMode::Fixed(self.rng.length(60, 300))
        } else {
            SizingMode::HugContent
        };
        let node = LayoutNode::new(self.id(0))
            .with_sizing(width, height)
            .with_padding(self.padding())
            .with_container(container);
        self.push(node)
    }

    fn finish(self, mode: CorpusMode, index: usize) -> LayoutFixture {
        let root = self.nodes[0].id;
        let mut nodes = self.nodes;
        // node 목록 순서도 결정적으로 뒤집어 입력 순서 무관성을 함께 검사합니다.
        if self.seed % 2 == 1 {
            nodes.reverse();
        }
        LayoutFixture {
            name: format!("{}-{index:03}", mode.name()),
            mode,
            seed: self.seed,
            input: LayoutInput::new(root, nodes),
        }
    }
}

fn anchor_offset(rng: &mut Rng) -> Offset {
    if rng.chance(25) {
        Offset::Percentage(percent(rng.range(-10, 50)))
    } else {
        Offset::Length(rng.length(-20, 100))
    }
}

fn free_fixture(seed: u64, index: usize) -> LayoutFixture {
    let mut b = Builder::new(seed);
    let root = b.root(ContainerLayout::Free);
    let count = b.rng.range(1, 6) as usize;
    let mut children = Vec::new();
    for n in 0..count {
        let mut node = b.leaf(1 + n, true);
        let mut axes = [AxisAnchors::NONE; 2];
        for (axis, mode) in [(0usize, node.width), (1, node.height)] {
            let choice = b.rng.range(0, if mode == SizingMode::Fill { 3 } else { 2 });
            axes[axis] = match choice {
                0 => AxisAnchors::NONE,
                1 => AxisAnchors {
                    start: Some(anchor_offset(&mut b.rng)),
                    end: None,
                },
                2 => AxisAnchors {
                    start: None,
                    end: Some(anchor_offset(&mut b.rng)),
                },
                _ => AxisAnchors {
                    start: Some(anchor_offset(&mut b.rng)),
                    end: Some(anchor_offset(&mut b.rng)),
                },
            };
        }
        node = node.with_anchors(axes[0], axes[1]);
        children.push(b.push(node));
    }
    // 일부 자식은 Free container(손자 고정 크기 1~3개).
    if b.rng.chance(40) {
        let parent = children[0];
        if matches!(b.nodes[parent].content, LayoutContent::None) {
            let mut grandchildren = Vec::new();
            for g in 0..b.rng.range(1, 3) as usize {
                let id = b.id(20 + g);
                let w = b.rng.length(1, 40);
                let h = b.rng.length(1, 40);
                let x = b.rng.length(0, 30);
                let y = b.rng.length(0, 30);
                grandchildren.push(
                    b.push(
                        LayoutNode::new(id)
                            .with_fixed_size(w, h)
                            .with_position(x, y),
                    ),
                );
            }
            b.attach(parent, &grandchildren);
        }
    }
    b.attach(root, &children);
    b.finish(CorpusMode::Free, index)
}

fn stack_fixture(seed: u64, index: usize) -> LayoutFixture {
    let mut b = Builder::new(seed);
    let axis = if b.rng.chance(50) {
        Axis::Horizontal
    } else {
        Axis::Vertical
    };
    let stack = StackLayout {
        axis,
        direction: b
            .rng
            .pick(&[StackDirection::Forward, StackDirection::Reverse]),
        gap: b.rng.length(0, 16),
        main_alignment: b.rng.pick(&[
            MainAlignment::Start,
            MainAlignment::Center,
            MainAlignment::End,
            MainAlignment::SpaceBetween,
            MainAlignment::SpaceAround,
            MainAlignment::SpaceEvenly,
        ]),
        cross_alignment: b.rng.pick(&[
            CrossAlignment::Start,
            CrossAlignment::Center,
            CrossAlignment::End,
            CrossAlignment::Stretch,
        ]),
        wrap: b.rng.pick(&[StackWrap::NoWrap, StackWrap::Wrap]),
        cross_gap: b.rng.length(0, 12),
        line_alignment: b.rng.pick(&[
            LineAlignment::Start,
            LineAlignment::Center,
            LineAlignment::End,
            LineAlignment::SpaceBetween,
            LineAlignment::SpaceAround,
            LineAlignment::SpaceEvenly,
            LineAlignment::Stretch,
        ]),
    };
    let root = b.root(ContainerLayout::Stack(stack));
    let count = b.rng.range(1, 8) as usize;
    let mut children = Vec::new();
    for n in 0..count {
        let mut node = b.leaf(1 + n, true);
        let main = match axis {
            Axis::Horizontal => node.width,
            Axis::Vertical => node.height,
        };
        if main != SizingMode::Fill && b.rng.chance(60) {
            let basis = match b.rng.range(0, 2) {
                0 => FlexBasis::Auto,
                1 => FlexBasis::Length(b.rng.length(0, 80)),
                _ => FlexBasis::Percentage(percent(b.rng.range(0, 60))),
            };
            node = node.with_flex(FlexItem::new(
                scalar(b.rng.range(0, 12)),
                scalar(b.rng.range(0, 8)),
                basis,
            ));
        }
        children.push(b.push(node));
    }
    b.attach(root, &children);
    b.finish(CorpusMode::Stack, index)
}

fn track(rng: &mut Rng) -> TrackSize {
    match rng.range(0, 5) {
        0 => TrackSize::Fixed(rng.length(0, 80)),
        1 => TrackSize::Fraction(scalar(rng.range(1, 12))),
        2 => TrackSize::MinContent,
        3 => TrackSize::MaxContent,
        4 => TrackSize::MinMax(
            TrackBreadth::Fixed(rng.length(0, 30)),
            TrackBreadth::Fraction(scalar(rng.range(1, 8))),
        ),
        _ => TrackSize::MinMax(
            TrackBreadth::MinContent,
            TrackBreadth::Fixed(rng.length(30, 90)),
        ),
    }
}

fn grid_fixture(seed: u64, index: usize) -> LayoutFixture {
    let mut b = Builder::new(seed);
    let columns: Vec<TrackSize> = (0..b.rng.range(1, 4)).map(|_| track(&mut b.rng)).collect();
    let rows: Vec<TrackSize> = (0..b.rng.range(0, 3)).map(|_| track(&mut b.rng)).collect();
    let column_count = columns.len() as u16;
    let grid = GridLayout {
        column_gap: b.rng.length(0, 12),
        row_gap: b.rng.length(0, 12),
        implicit_row: if b.rng.chance(50) {
            TrackSize::Fixed(b.rng.length(4, 40))
        } else {
            TrackSize::MaxContent
        },
        ..GridLayout::new(columns, rows)
    };
    let root = b.root(ContainerLayout::Grid(grid));
    let count = b.rng.range(1, 8) as usize;
    let mut children = Vec::new();
    for n in 0..count {
        let node = b.leaf(1 + n, true);
        let column_span = b.rng.range(1, i32::from(column_count)) as u16;
        let item = match b.rng.range(0, 2) {
            // 명시 위치(열 범위 안, 행은 암시 행 허용).
            0 => {
                let start = b.rng.range(1, i32::from(column_count - column_span + 1)) as u16;
                GridItem::at(
                    start,
                    b.rng.range(1, 4) as u16,
                    column_span,
                    b.rng.range(1, 2) as u16,
                )
            }
            // 완전 자동.
            1 => GridItem {
                column: GridPlacement::Auto { span: column_span },
                row: GridPlacement::Auto {
                    span: b.rng.range(1, 2) as u16,
                },
            },
            // 열만 명시.
            _ => {
                let start = b.rng.range(1, i32::from(column_count - column_span + 1)) as u16;
                GridItem {
                    column: GridPlacement::Span {
                        start,
                        span: column_span,
                    },
                    row: GridPlacement::Auto { span: 1 },
                }
            }
        };
        children.push(b.push(node.with_grid(item)));
    }
    b.attach(root, &children);
    b.finish(CorpusMode::Grid, index)
}

fn overlay_fixture(seed: u64, index: usize) -> LayoutFixture {
    let mut b = Builder::new(seed);
    let alignments = [
        OverlayAlignment::Start,
        OverlayAlignment::Center,
        OverlayAlignment::End,
    ];
    let overlay = OverlayLayout {
        horizontal: b.rng.pick(&alignments),
        vertical: b.rng.pick(&alignments),
    };
    let root = b.root(ContainerLayout::Overlay(overlay));
    let count = b.rng.range(1, 6) as usize;
    let mut children = Vec::new();
    for n in 0..count {
        let node = b.leaf(1 + n, true);
        children.push(b.push(node));
    }
    b.attach(root, &children);
    b.finish(CorpusMode::Overlay, index)
}

fn constraint_fixture(seed: u64, index: usize) -> LayoutFixture {
    let mut b = Builder::new(seed);
    let root = b.root(ContainerLayout::Constraint);
    let count = b.rng.range(1, 6) as usize;
    let mut children = Vec::new();
    for n in 0..count {
        // 가로: 고정 또는 Fill(양끝 등식 2개). 세로: 고정 또는 Hug(intrinsic).
        let fill_width = b.rng.chance(30);
        let width = if fill_width {
            SizingMode::Fill
        } else {
            SizingMode::Fixed(b.rng.length(4, 80))
        };
        let hug_height = b.rng.chance(40);
        let height = if hug_height {
            SizingMode::HugContent
        } else {
            SizingMode::Fixed(b.rng.length(4, 60))
        };
        let mut node = LayoutNode::new(b.id(1 + n)).with_sizing(width, height);
        if hug_height {
            node = node.with_content(b.intrinsic());
        }
        let previous = if n > 0 { Some(b.id(n)) } else { None };
        let offset = |rng: &mut Rng| rng.length(-10, 60);
        // 가로 시작 등식.
        let start = match (previous, b.rng.range(0, 2)) {
            (Some(prev), 0) => Constraint::new(
                AnchorEdge::Left,
                AnchorTarget::Sibling {
                    node: prev,
                    edge: AnchorEdge::Right,
                },
                offset(&mut b.rng),
            ),
            (_, 1) if !fill_width => Constraint::new(
                AnchorEdge::CenterX,
                AnchorTarget::Parent(AnchorEdge::CenterX),
                offset(&mut b.rng),
            ),
            _ => Constraint::new(
                AnchorEdge::Left,
                AnchorTarget::Parent(AnchorEdge::Left),
                offset(&mut b.rng),
            ),
        };
        node = node.with_constraint(start);
        if fill_width {
            node = node.with_constraint(Constraint::new(
                AnchorEdge::Right,
                AnchorTarget::Parent(AnchorEdge::Right),
                quarter(-b.rng.range(0, 40) * 4),
            ));
        }
        // 세로: 대부분 등식 하나, 일부는 비워 underdetermined 진단, 일부는 없는 sibling 참조.
        let vertical = match (previous, b.rng.range(0, 5)) {
            (Some(prev), 0) => Some(Constraint::new(
                AnchorEdge::CenterY,
                AnchorTarget::Sibling {
                    node: prev,
                    edge: AnchorEdge::CenterY,
                },
                offset(&mut b.rng),
            )),
            (Some(prev), 1) => Some(Constraint::new(
                AnchorEdge::Baseline,
                AnchorTarget::Sibling {
                    node: prev,
                    edge: AnchorEdge::Baseline,
                },
                offset(&mut b.rng),
            )),
            (_, 2) => Some(Constraint::new(
                AnchorEdge::Bottom,
                AnchorTarget::Parent(AnchorEdge::Bottom),
                offset(&mut b.rng),
            )),
            (_, 3) if b.rng.chance(30) => None,
            (_, 4) if b.rng.chance(20) => Some(Constraint::new(
                AnchorEdge::Top,
                AnchorTarget::Sibling {
                    node: b.id(99),
                    edge: AnchorEdge::Bottom,
                },
                offset(&mut b.rng),
            )),
            _ => Some(Constraint::new(
                AnchorEdge::Top,
                AnchorTarget::Parent(AnchorEdge::Top),
                offset(&mut b.rng),
            )),
        };
        if let Some(vertical) = vertical {
            node = node.with_constraint(vertical);
        }
        children.push(b.push(node));
    }
    b.attach(root, &children);
    b.finish(CorpusMode::Constraint, index)
}

/// 배치 방식마다 `per_mode`개 fixture를 결정적으로 생성합니다(`CorpusMode::ALL` 순서).
pub fn generate_corpus(per_mode: usize) -> Vec<LayoutFixture> {
    let mut fixtures = Vec::with_capacity(per_mode * CorpusMode::ALL.len());
    for (m, mode) in CorpusMode::ALL.iter().enumerate() {
        for index in 0..per_mode {
            let seed = 0x5354_4c59_0000_0000 ^ ((m as u64) << 24) ^ index as u64;
            let fixture = match mode {
                CorpusMode::Free => free_fixture(seed, index),
                CorpusMode::Stack => stack_fixture(seed, index),
                CorpusMode::Grid => grid_fixture(seed, index),
                CorpusMode::Overlay => overlay_fixture(seed, index),
                CorpusMode::Constraint => constraint_fixture(seed, index),
            };
            fixtures.push(fixture);
        }
    }
    fixtures
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_rejects_malformed_fixtures() {
        let corpus = generate_corpus(MIN_FIXTURES_PER_MODE);
        let mut bad = corpus[0].clone();
        bad.name = "free-1".to_owned();
        assert!(matches!(
            validate_fixture(&bad),
            Err(FixtureSchemaError::BadName { .. })
        ));
        let mut mismatch = corpus[0].clone();
        mismatch.mode = CorpusMode::Grid;
        mismatch.name = "grid-000".to_owned();
        assert!(matches!(
            validate_fixture(&mismatch),
            Err(FixtureSchemaError::ModeMismatch { .. })
        ));
        let mut duplicate = corpus.clone();
        duplicate.push(corpus[0].clone());
        assert!(matches!(
            validate_corpus(&duplicate),
            Err(FixtureSchemaError::DuplicateName { .. })
        ));
        let short = generate_corpus(MIN_FIXTURES_PER_MODE - 1);
        assert!(matches!(
            validate_corpus(&short),
            Err(FixtureSchemaError::TooFewFixtures {
                mode: CorpusMode::Free,
                count: 49
            })
        ));
        let mut invalid = corpus[0].clone();
        invalid.input.root = StableId::new_v7(1, 2, 3);
        assert!(matches!(
            validate_fixture(&invalid),
            Err(FixtureSchemaError::InvalidInput { .. })
        ));
    }
}
