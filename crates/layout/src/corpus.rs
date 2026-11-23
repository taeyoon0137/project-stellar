//! 레이아웃 회귀 fixture corpus: schema, 결정적 생성기, runner (X.11.1, ADR-0076).
//!
//! # schema (version [`FIXTURE_SCHEMA_VERSION`])
//!
//! fixture 하나는 [`LayoutFixture`] `{ name, mode, seed, input }`입니다.
//!
//! - `name`은 `"<mode>-<3자리 순번>"`(예: `flex-007`)이며 corpus 안에서 유일합니다.
//! - `mode`는 root의 `display`와 자식 배치 방식에 맞아야 합니다([`CorpusMode::matches`]).
//! - `input`은 [`LayoutInput::validate`]를 통과하고, canonical bytes 왕복이 같은 bytes를 내며, node 수는
//!   [`MAX_FIXTURE_NODES`] 이하입니다.
//! - 배치 방식마다 [`MIN_FIXTURES_PER_MODE`]개 이상 있어야 합니다.
//!
//! # 결정적 생성과 CSS 입력 이전
//!
//! [`generate_corpus`]는 고정 seed의 SplitMix64로 입력을 만듭니다. 생성기는 엔진 입력을 CSS 값으로 바꾸기 전
//! corpus(X.11.1)와 같은 seed·같은 난수 호출 순서를 그대로 쓰고, 각 선택을 속성 모델 4.3절 변환 표의 CSS
//! longhand로 냅니다. 그래서 node id·tree 모양·길이 값이 이전 corpus와 같고, 결과 차이는 CSS 의미 차이에서만
//! 옵니다(보고서 IX.11.4·IX.11.5의 변경 목록). 형제·baseline 기준 배치처럼 CSS 값이 없는 선택은 `auto` inset으로
//! 냅니다. 길이는 1/4 px 배수(정확한 f32)이고, 자식 삽입 순서와 node 목록 순서를 섞어 visual order 규칙을 함께
//! 검사합니다.
//!
//! # runner
//!
//! [`run_fixture`]는 레이아웃을 계산해 결과 canonical bytes의 hash([`FixtureOutcome::digest`])를,
//! [`run_corpus`]는 배치 방식별 fixture 수·box 수·진단 수·digest 묶음을 냅니다. 저장소의 작은 golden
//! (`crates/layout/tests/fixtures/layout_corpus_v2.golden`)이 이 요약을 고정합니다.

use std::collections::BTreeSet;
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::value::{FiniteF32, Insets, Length, Percentage, Transform2D};

use crate::cache::{LayoutInputHash, LayoutInputHasher};
use crate::engine::compute_layout;
use crate::error::LayoutError;
use crate::geometry::Axis;
use crate::input::{IntrinsicContent, LayoutContent, LayoutInput, LayoutNode};
use crate::style::{
    AspectRatio, ContentAlignment, Display, FlexBasis, FlexDirection, FlexWrap, ItemAlignment,
    LayoutStyle, LengthPercentageAuto, MaxSize, Position, SelfAlignment, Size,
};

/// fixture schema version.
pub const FIXTURE_SCHEMA_VERSION: u16 = 2;
/// 배치 방식별 최소 fixture 수.
pub const MIN_FIXTURES_PER_MODE: usize = 50;
/// fixture 하나의 최대 node 수.
pub const MAX_FIXTURE_NODES: usize = 64;

/// corpus 배치 방식 (X.11.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CorpusMode {
    /// block root 안 `position: absolute` 자식(inset·백분율 위치).
    Absolute,
    /// `display: flex` root.
    Flex,
    /// block root 안 부모 변 기준 inset만 가진 absolute 자식.
    Inset,
}

impl CorpusMode {
    /// 모든 배치 방식(golden 순서).
    pub const ALL: [CorpusMode; 3] = [CorpusMode::Absolute, CorpusMode::Flex, CorpusMode::Inset];

    /// 이름(fixture name 접두사).
    pub const fn name(self) -> &'static str {
        match self {
            CorpusMode::Absolute => "absolute",
            CorpusMode::Flex => "flex",
            CorpusMode::Inset => "inset",
        }
    }

    /// root style이 이 배치 방식인지.
    pub fn matches(self, root: &LayoutStyle) -> bool {
        match self {
            CorpusMode::Absolute | CorpusMode::Inset => root.display == Display::Block,
            CorpusMode::Flex => root.display == Display::Flex,
        }
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
    /// root 배치 방식이 mode와 다릅니다.
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
    if !fixture.mode.matches(&root.style) {
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
    Length::new(quarters as f32 / 4.0).unwrap_or(Length::from_raw(FiniteF32::ZERO))
}

fn factor(quarters: i32) -> FiniteF32 {
    FiniteF32::new(quarters as f32 / 4.0).unwrap_or(FiniteF32::ZERO)
}

fn percent(value: i32) -> Percentage {
    Percentage::new(value as f32).unwrap_or(Percentage::from_raw(FiniteF32::ZERO))
}

fn px(length: Length) -> LengthPercentageAuto {
    LengthPercentageAuto::Length(length)
}

/// 이전 corpus 생성기의 축 크기 선택(속성 모델 4.3절 변환 표의 왼쪽 열).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Choice {
    /// `<length>`.
    Px(Length),
    /// 콘텐츠 크기(`fit-content`).
    Content,
    /// `<percentage>`.
    Percent(Percentage),
    /// 부모 영역 채움(문맥별 longhand 조합).
    Stretch,
}

/// 축 크기 선택을 쓰는 부모 문맥.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Context {
    /// block root 안 absolute 자식.
    Absolute,
    /// flex item main 축.
    FlexMain,
    /// flex item cross 축.
    FlexCross,
}

/// 축 크기 선택을 style에 씁니다(4.3절).
fn apply_choice(style: &mut LayoutStyle, axis: Axis, choice: Choice, context: Context) {
    let size = match choice {
        Choice::Px(length) => Size::Length(length),
        Choice::Content => Size::FitContent,
        Choice::Percent(p) => Size::Percentage(p),
        Choice::Stretch => Size::Auto,
    };
    match axis {
        Axis::Horizontal => style.width = size,
        Axis::Vertical => style.height = size,
    }
    match (choice, context) {
        (Choice::Stretch, Context::FlexMain) => {
            style.flex_grow = factor(4);
            style.flex_shrink = factor(4);
            style.flex_basis = FlexBasis::Length(quarter(0));
            match axis {
                Axis::Horizontal => style.min_width = Size::Length(quarter(0)),
                Axis::Vertical => style.min_height = Size::Length(quarter(0)),
            }
        }
        (_, Context::FlexMain) => {
            style.flex_grow = FiniteF32::ZERO;
            style.flex_shrink = FiniteF32::ZERO;
            style.flex_basis = FlexBasis::Auto;
        }
        (Choice::Stretch, Context::FlexCross) => {
            style.align_self = SelfAlignment::Item(ItemAlignment::Stretch);
        }
        (Choice::Stretch, Context::Absolute) => {
            let (start, end) = style.insets(axis);
            let fill = |inset: LengthPercentageAuto| match inset {
                LengthPercentageAuto::Auto => px(quarter(0)),
                other => other,
            };
            match axis {
                Axis::Horizontal => {
                    style.left = fill(start);
                    style.right = fill(end);
                }
                Axis::Vertical => {
                    style.top = fill(start);
                    style.bottom = fill(end);
                }
            }
        }
        _ => {}
    }
}

/// fixture 안 node 생성 상태.
struct Builder {
    rng: Rng,
    seed: u64,
    nodes: Vec<LayoutNode>,
}

/// leaf 하나와 그 축 크기 선택.
struct Leaf {
    node: LayoutNode,
    width: Choice,
    height: Choice,
}

impl Builder {
    fn new(seed: u64) -> Builder {
        Builder {
            rng: Rng(seed),
            seed,
            nodes: Vec::new(),
        }
    }

    fn id(&self, n: usize) -> StableId {
        let n = n as u64;
        StableId::new_v7(
            1_700_000_000_000 + n,
            (self.seed & 0x0fff) as u16,
            self.seed ^ (n << 32),
        )
    }

    fn key(n: usize) -> OrderKey {
        let [_, hi, mid, lo] = (n as u32 + 1).to_be_bytes();
        OrderKey::from_bytes(vec![hi, mid, lo, 1]).unwrap_or_else(|_| OrderKey::between(None, None))
    }

    fn push(&mut self, node: LayoutNode) -> usize {
        self.nodes.push(node);
        self.nodes.len() - 1
    }

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

    fn choice(&mut self, allow_stretch: bool) -> Choice {
        match self.rng.range(0, if allow_stretch { 3 } else { 2 }) {
            0 => Choice::Px(self.rng.length(0, 120)),
            1 => Choice::Content,
            2 => Choice::Percent(percent(self.rng.range(5, 100))),
            _ => Choice::Stretch,
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

    fn limits(&mut self, style: &mut LayoutStyle) {
        if self.rng.chance(25) {
            let min = self.rng.range(0, 40);
            let max = min + self.rng.range(0, 80);
            style.min_width = Size::Length(quarter(min * 4));
            style.max_width = MaxSize::Length(quarter(max * 4));
        }
        if self.rng.chance(25) {
            let min = self.rng.range(0, 30);
            style.min_height = Size::Length(quarter(min * 4));
        }
    }

    /// leaf 자식 하나(콘텐츠 크기 축이 있으면 intrinsic 콘텐츠).
    fn leaf(&mut self, n: usize, allow_stretch: bool) -> Leaf {
        let width = self.choice(allow_stretch);
        let height = self.choice(allow_stretch);
        let mut style = LayoutStyle::default();
        self.limits(&mut style);
        let mut node = LayoutNode::new(self.id(n));
        if (width == Choice::Content || height == Choice::Content) && self.rng.chance(80) {
            node = node.with_content(self.intrinsic());
        }
        if self.rng.chance(10) {
            style.display = Display::None;
        }
        if self.rng.chance(15) {
            let a = self.rng.range(-2, 2) as f32;
            node = node.with_transform(
                Transform2D::new(a, 1.0, -1.0, a, 3.0, -2.0)
                    .unwrap_or_else(|_| Transform2D::identity()),
            );
        }
        if self.rng.chance(10) && (width != Choice::Content || height != Choice::Content) {
            let w = self.rng.range(4, 16) as f32 / 4.0;
            let h = self.rng.range(4, 16) as f32 / 4.0;
            style.aspect_ratio = AspectRatio::new(w, h);
        }
        node.style = style;
        Leaf {
            node,
            width,
            height,
        }
    }

    fn root(&mut self, display: Display) -> (usize, LayoutStyle) {
        let width = if self.rng.chance(80) {
            Size::Length(self.rng.length(80, 400))
        } else {
            Size::FitContent
        };
        let height = if self.rng.chance(80) {
            Size::Length(self.rng.length(60, 300))
        } else {
            Size::FitContent
        };
        let style = LayoutStyle {
            display,
            width,
            height,
            padding: self.padding(),
            ..LayoutStyle::default()
        };
        let node = LayoutNode::new(self.id(0)).with_style(style.clone());
        (self.push(node), style)
    }

    fn finish(self, mode: CorpusMode, index: usize) -> LayoutFixture {
        let root = self.nodes[0].id;
        let mut nodes = self.nodes;
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

fn inset_offset(rng: &mut Rng) -> LengthPercentageAuto {
    if rng.chance(25) {
        LengthPercentageAuto::Percentage(percent(rng.range(-10, 50)))
    } else {
        px(rng.length(-20, 100))
    }
}

fn absolute_fixture(seed: u64, index: usize) -> LayoutFixture {
    let mut b = Builder::new(seed);
    let (root, _) = b.root(Display::Block);
    let count = b.rng.range(1, 6) as usize;
    let mut children = Vec::new();
    for n in 0..count {
        let leaf = b.leaf(1 + n, true);
        let mut node = leaf.node;
        node.style.position = Position::Absolute;
        for (axis, choice) in [
            (Axis::Horizontal, leaf.width),
            (Axis::Vertical, leaf.height),
        ] {
            let pick = b
                .rng
                .range(0, if choice == Choice::Stretch { 3 } else { 2 });
            let (start, end) = match pick {
                0 => (LengthPercentageAuto::Auto, LengthPercentageAuto::Auto),
                1 => (inset_offset(&mut b.rng), LengthPercentageAuto::Auto),
                2 => (LengthPercentageAuto::Auto, inset_offset(&mut b.rng)),
                _ => (inset_offset(&mut b.rng), inset_offset(&mut b.rng)),
            };
            match axis {
                Axis::Horizontal => {
                    node.style.left = start;
                    node.style.right = end;
                }
                Axis::Vertical => {
                    node.style.top = start;
                    node.style.bottom = end;
                }
            }
            apply_choice(&mut node.style, axis, choice, Context::Absolute);
        }
        children.push(b.push(node));
    }
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
                grandchildren.push(b.push(LayoutNode::new(id).with_style(LayoutStyle {
                    position: Position::Absolute,
                    left: px(x),
                    top: px(y),
                    width: Size::Length(w),
                    height: Size::Length(h),
                    ..LayoutStyle::default()
                })));
            }
            b.attach(parent, &grandchildren);
        }
    }
    b.attach(root, &children);
    b.finish(CorpusMode::Absolute, index)
}

fn flex_fixture(seed: u64, index: usize) -> LayoutFixture {
    let mut b = Builder::new(seed);
    let horizontal = b.rng.chance(50);
    let reverse = b.rng.pick(&[false, true]);
    let gap = b.rng.length(0, 16);
    let justify = b.rng.pick(&[
        ContentAlignment::FlexStart,
        ContentAlignment::Center,
        ContentAlignment::FlexEnd,
        ContentAlignment::SpaceBetween,
        ContentAlignment::SpaceAround,
        ContentAlignment::SpaceEvenly,
    ]);
    let align = b.rng.pick(&[
        ItemAlignment::FlexStart,
        ItemAlignment::Center,
        ItemAlignment::FlexEnd,
        ItemAlignment::Stretch,
    ]);
    let wrap = b.rng.pick(&[FlexWrap::NoWrap, FlexWrap::Wrap]);
    let cross_gap = b.rng.length(0, 12);
    let lines = b.rng.pick(&[
        ContentAlignment::FlexStart,
        ContentAlignment::Center,
        ContentAlignment::FlexEnd,
        ContentAlignment::SpaceBetween,
        ContentAlignment::SpaceAround,
        ContentAlignment::SpaceEvenly,
        ContentAlignment::Stretch,
    ]);
    let (root, _) = b.root(Display::Flex);
    {
        let style = &mut b.nodes[root].style;
        style.flex_direction = match (horizontal, reverse) {
            (true, false) => FlexDirection::Row,
            (true, true) => FlexDirection::RowReverse,
            (false, false) => FlexDirection::Column,
            (false, true) => FlexDirection::ColumnReverse,
        };
        let (main_gap, other_gap) = (gap, cross_gap);
        if horizontal {
            style.column_gap = main_gap;
            style.row_gap = other_gap;
        } else {
            style.row_gap = main_gap;
            style.column_gap = other_gap;
        }
        style.justify_content = justify;
        style.align_items = align;
        style.flex_wrap = wrap;
        style.align_content = lines;
    }
    let (main, cross) = if horizontal {
        (Axis::Horizontal, Axis::Vertical)
    } else {
        (Axis::Vertical, Axis::Horizontal)
    };
    let count = b.rng.range(1, 8) as usize;
    let mut children = Vec::new();
    for n in 0..count {
        let leaf = b.leaf(1 + n, true);
        let mut node = leaf.node;
        let (main_choice, cross_choice) = if horizontal {
            (leaf.width, leaf.height)
        } else {
            (leaf.height, leaf.width)
        };
        apply_choice(&mut node.style, main, main_choice, Context::FlexMain);
        apply_choice(&mut node.style, cross, cross_choice, Context::FlexCross);
        if main_choice != Choice::Stretch && b.rng.chance(60) {
            let basis = match b.rng.range(0, 2) {
                0 => FlexBasis::Auto,
                1 => FlexBasis::Length(b.rng.length(0, 80)),
                _ => FlexBasis::Percentage(percent(b.rng.range(0, 60))),
            };
            node.style.flex_grow = factor(b.rng.range(0, 12));
            node.style.flex_shrink = factor(b.rng.range(0, 8));
            node.style.flex_basis = basis;
        }
        children.push(b.push(node));
    }
    b.attach(root, &children);
    b.finish(CorpusMode::Flex, index)
}

fn inset_fixture(seed: u64, index: usize) -> LayoutFixture {
    let mut b = Builder::new(seed);
    let (root, _) = b.root(Display::Block);
    let count = b.rng.range(1, 6) as usize;
    let mut children = Vec::new();
    for n in 0..count {
        let stretch_width = b.rng.chance(30);
        let width = if stretch_width {
            Size::Auto
        } else {
            Size::Length(b.rng.length(4, 80))
        };
        let content_height = b.rng.chance(40);
        let height = if content_height {
            Size::FitContent
        } else {
            Size::Length(b.rng.length(4, 60))
        };
        let mut node = LayoutNode::new(b.id(1 + n)).with_style(LayoutStyle {
            position: Position::Absolute,
            width,
            height,
            ..LayoutStyle::default()
        });
        if content_height {
            node = node.with_content(b.intrinsic());
        }
        let has_previous = n > 0;
        let offset = |rng: &mut Rng| rng.length(-10, 60);
        // 가로 시작: 형제 기준(CSS 값 없음 → static position), 가운데(CSS 값 없음 → static position), 부모 왼쪽.
        match (has_previous, b.rng.range(0, 2)) {
            (true, 0) => {
                let _ = offset(&mut b.rng);
            }
            (_, 1) if !stretch_width => {
                let _ = offset(&mut b.rng);
            }
            _ => node.style.left = px(offset(&mut b.rng)),
        }
        if stretch_width {
            if node.style.left == LengthPercentageAuto::Auto {
                node.style.left = px(quarter(0));
            }
            node.style.right = px(quarter(b.rng.range(0, 40) * 4));
        }
        match (has_previous, b.rng.range(0, 5)) {
            (true, 0) | (true, 1) => {
                let _ = offset(&mut b.rng);
            }
            (_, 2) => {
                let value = offset(&mut b.rng);
                node.style.bottom = px(quarter(-(value.get() * 4.0) as i32));
            }
            (_, 3) if b.rng.chance(30) => {}
            (_, 4) if b.rng.chance(20) => {
                let _ = offset(&mut b.rng);
            }
            _ => node.style.top = px(offset(&mut b.rng)),
        }
        children.push(b.push(node));
    }
    b.attach(root, &children);
    b.finish(CorpusMode::Inset, index)
}

/// 배치 방식마다 `per_mode`개 fixture를 결정적으로 생성합니다(`CorpusMode::ALL` 순서).
///
/// seed의 배치 방식 번호는 이전 corpus와 같습니다(absolute = Free 0, flex = Stack 1, inset = Constraint 4).
pub fn generate_corpus(per_mode: usize) -> Vec<LayoutFixture> {
    let mut fixtures = Vec::with_capacity(per_mode * CorpusMode::ALL.len());
    for mode in CorpusMode::ALL {
        let m: u64 = match mode {
            CorpusMode::Absolute => 0,
            CorpusMode::Flex => 1,
            CorpusMode::Inset => 4,
        };
        for index in 0..per_mode {
            let seed = 0x5354_4c59_0000_0000 ^ (m << 24) ^ index as u64;
            let fixture = match mode {
                CorpusMode::Absolute => absolute_fixture(seed, index),
                CorpusMode::Flex => flex_fixture(seed, index),
                CorpusMode::Inset => inset_fixture(seed, index),
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
        validate_corpus(&corpus).unwrap();
        let mut bad = corpus[0].clone();
        bad.name = "absolute-1".to_owned();
        assert!(matches!(
            validate_fixture(&bad),
            Err(FixtureSchemaError::BadName { .. })
        ));
        let mut mismatch = corpus[0].clone();
        mismatch.mode = CorpusMode::Flex;
        mismatch.name = "flex-000".to_owned();
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
                mode: CorpusMode::Absolute,
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
