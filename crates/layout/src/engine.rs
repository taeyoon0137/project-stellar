//! 레이아웃 엔진 진입점과 공통 box 크기 해석 (X.2.1, ADR-0076).
//!
//! # 계산 모델
//!
//! 엔진은 CSS 의미를 따릅니다. node 하나의 배치는 **요청**([`Request`])의 순수 함수입니다.
//!
//! - `known`: 부모가 정한 border-box 크기(flex 결과 main 크기, stretch된 cross 크기, grid 영역 크기, 양쪽 inset
//!   absolute 크기). 정해진 축은 definite입니다.
//! - `parent`: 이 box 자신의 백분율(`width`·`height`·`min-*`·`max-*`)을 해석할 containing block 크기.
//!   indefinite 축은 `None`이고 그 축의 백분율은 `auto`로 해석합니다.
//! - `available`: `auto` 크기를 정할 가용 공간. definite 값이면 `fit-content`(shrink-to-fit), `MinContent`·
//!   `MaxContent`면 그 intrinsic 크기입니다. 흐름 안 `auto` 너비를 늘리는 일(block stretch, flex stretch)은 부모가
//!   `known`으로 정합니다.
//! - `content_only`: 축의 `width`·`height`·`min-*`·`max-*`를 무시하고 콘텐츠 크기만 구합니다(자동 최소 크기의
//!   content size suggestion 등).
//!
//! 결과([`NodeLayout`])는 border-box 크기, 첫 baseline, 자식 배치(위치와 그 자식에게 준 요청), 진단입니다. 같은
//! node·같은 요청은 한 계산에서 한 번만 계산하고(in-run memo), [`compute_layout_cached`]는 subtree input hash로 계산
//! 사이에도 재사용합니다. 최종 결과는 root 요청에서 시작해 자식 배치의 요청을 따라 pre-order로 조립합니다.
//!
//! # root
//!
//! root는 containing block이 없습니다. root 요청은 가로 가용 공간 [`ROOT_AVAILABLE_PX`] px, 세로 indefinite입니다.
//! 그래서 `auto` 너비는 max-content(가용 공간보다 넓지 않을 때), `auto` 높이는 콘텐츠 높이이고, root 자신의
//! 백분율 크기는 `auto`입니다. root는 위치 (0, 0)에 놓이고 `position`·inset을 쓰지 않습니다.
//!
//! # 재귀 깊이와 stack
//!
//! 배치는 tree 깊이에 비례해 재귀합니다. 깊이가 [`INLINE_LAYOUT_DEPTH`] 이하인 tree만 호출 thread에서 계산하고,
//! 더 깊은 tree는 [`DEEP_LAYOUT_STACK_BYTES`] stack을 가진 전용 thread에서 계산합니다. 결과는 계산 thread와
//! 무관합니다.

use std::collections::HashMap;
use std::sync::Arc;

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{FiniteF32, Insets};

use crate::block;
use crate::cache::LayoutInputHash;
use crate::error::LayoutError;
use crate::fast_hash::FastMap;
use crate::flex;
use crate::geometry::{Axis, LayoutRect, LayoutSize};
use crate::input::{
    length_unit, weight, LayoutContent, LayoutInput, LayoutNode, TreeIndex, MAX_LAYOUT_DEPTH,
};
use crate::intrinsic::{
    ContentKind, ContentMeasurer, DefaultMeasurer, IntrinsicMeasure, IntrinsicMode, IntrinsicSize,
    MeasureRequest,
};
use crate::measure::{subtree_hashes, MeasureCache};
use crate::output::{LayoutBox, LayoutDiagnostic, LayoutResult};
use crate::property::LayoutProperty;
use crate::rounding::{LayoutUnit, LayoutUnitError};
use crate::style::{
    ContentAlignment, Display, FlexBasis, FlexDirection, FlexWrap, ItemAlignment,
    LengthPercentageAuto, MaxSize, Position, SelfAlignment, Size,
};

/// 호출 thread에서 바로 계산하는 최대 tree 깊이. 더 깊으면 전용 stack thread에서 계산합니다.
pub const INLINE_LAYOUT_DEPTH: usize = 32;

/// [`INLINE_LAYOUT_DEPTH`]보다 깊은 tree를 계산하는 전용 thread의 stack 크기(level당 64 KiB).
pub const DEEP_LAYOUT_STACK_BYTES: usize = MAX_LAYOUT_DEPTH * 64 * 1024;

/// root의 가로 가용 공간(px). root `auto` 너비는 이 폭 안의 `fit-content`입니다.
pub const ROOT_AVAILABLE_PX: i32 = 1_000_000;

/// 입력 전체를 기본 측정기([`DefaultMeasurer`])로 레이아웃합니다.
pub fn compute_layout(input: &LayoutInput) -> Result<LayoutResult, LayoutError> {
    compute_layout_with(input, &DefaultMeasurer)
}

/// 입력 전체를 주어진 콘텐츠 측정기로 레이아웃합니다 (X.6.4).
pub fn compute_layout_with(
    input: &LayoutInput,
    measurer: &dyn ContentMeasurer,
) -> Result<LayoutResult, LayoutError> {
    run(input, measurer, None).map(|run| run.result)
}

/// 계산 간 cache를 써서 레이아웃하고 집계를 함께 돌려줍니다 (X.7.2).
///
/// node 배치는 subtree input hash·요청·측정기 revision이 같으면 `cache`의 결과를 재사용합니다. 결과는 cache 유무와
/// 무관하게 같습니다. 계산이 성공하면 `cache`에서 현재 입력에 맞지 않는 entry를 지웁니다.
pub fn compute_layout_cached(
    input: &LayoutInput,
    measurer: &dyn ContentMeasurer,
    cache: &mut MeasureCache,
) -> Result<LayoutRun, LayoutError> {
    run(input, measurer, Some(cache))
}

/// 한 번의 계산 집계 (X.7.2). 값은 포화 증가합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayoutStats {
    /// 실제로 계산한 node 배치(요청 하나) 수. intrinsic 측정 요청을 포함합니다.
    pub measured: u64,
    /// 같은 계산 안에서 이미 계산한 배치를 다시 읽은 수.
    pub memo_hits: u64,
    /// 계산 간 [`MeasureCache`]에서 읽은 배치 수.
    pub cache_hits: u64,
    /// 측정기([`ContentMeasurer`]) 호출 수.
    pub measurer_calls: u64,
    /// 최종 결과 tree에서 이번 계산이 새로 계산한 node 수(cache에서 재사용한 node 제외).
    pub arranged: u64,
}

/// 레이아웃 결과와 집계 (X.7.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutRun {
    /// 결과.
    pub result: LayoutResult,
    /// 집계.
    pub stats: LayoutStats,
}

fn run(
    input: &LayoutInput,
    measurer: &dyn ContentMeasurer,
    cache: Option<&mut MeasureCache>,
) -> Result<LayoutRun, LayoutError> {
    let tree = input.tree()?;
    let depth = tree.depth;
    with_layout_stack(depth, move || {
        compute_validated(input, tree, measurer, cache)
    })
}

/// 깊이 `depth`인 tree의 재귀 계산 `f`를 실행합니다 (crate 내부).
pub(crate) fn with_layout_stack<T: Send>(
    depth: usize,
    f: impl FnOnce() -> Result<T, LayoutError> + Send,
) -> Result<T, LayoutError> {
    if depth <= INLINE_LAYOUT_DEPTH {
        return f();
    }
    std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name("stellar-layout-deep".to_owned())
            .stack_size(DEEP_LAYOUT_STACK_BYTES)
            .spawn_scoped(scope, f)
            .map_err(|_| LayoutError::DeepStackUnavailable { depth })?;
        match worker.join() {
            Ok(result) => result,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    })
}

fn compute_validated(
    input: &LayoutInput,
    tree: TreeIndex,
    measurer: &dyn ContentMeasurer,
    cache: Option<&mut MeasureCache>,
) -> Result<LayoutRun, LayoutError> {
    let nodes = prepare_all(input)?;
    let subtree = if cache.is_some() {
        subtree_hashes(input, &tree)
    } else {
        Vec::new()
    };
    let mut engine = Engine::new(input, &tree, &nodes, &subtree, measurer, cache);
    let root = engine.tree.root;
    let request = Request::root();
    let layout = engine.layout(root, request)?;
    let mut boxes = Vec::with_capacity(input.nodes.len());
    let mut diagnostics = Vec::new();
    engine.assemble(
        root,
        request,
        &layout,
        LayoutUnit::ZERO,
        LayoutUnit::ZERO,
        &mut boxes,
        &mut diagnostics,
    )?;
    let result = LayoutResult::with_diagnostics(boxes, diagnostics)?;
    if let Some(cache) = engine.cache.as_deref_mut() {
        let current: HashMap<StableId, LayoutInputHash> = engine
            .nodes
            .iter()
            .map(|node| node.id)
            .zip(engine.subtree.iter().copied())
            .collect();
        cache.retain_current(&current, engine.measurer.revision());
    }
    Ok(LayoutRun {
        result,
        stats: engine.stats,
    })
}

/// 모든 node 입력을 변환합니다(입력 순서).
pub(crate) fn prepare_all(input: &LayoutInput) -> Result<Vec<Prepared>, LayoutError> {
    input.nodes.iter().map(prepare).collect()
}

// --- 요청과 결과 ---

/// 축의 가용 공간.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Avail {
    /// definite 가용 크기(border box 바깥 크기).
    Definite(LayoutUnit),
    /// min-content 제약.
    MinContent,
    /// max-content 제약.
    MaxContent,
}

impl Avail {
    /// definite 값.
    pub(crate) const fn definite(self) -> Option<LayoutUnit> {
        match self {
            Avail::Definite(value) => Some(value),
            Avail::MinContent | Avail::MaxContent => None,
        }
    }
}

/// node 배치 요청(모듈 문서 참조).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Request {
    /// 부모가 정한 border-box 크기.
    pub(crate) known: [Option<LayoutUnit>; 2],
    /// 자신의 백분율 기준(containing block content 크기). indefinite면 `None`.
    pub(crate) parent: [Option<LayoutUnit>; 2],
    /// `auto` 크기의 가용 공간.
    pub(crate) available: [Avail; 2],
    /// 축의 크기 속성을 무시하고 콘텐츠 크기만 구하는지 여부.
    pub(crate) content_only: [bool; 2],
}

impl Request {
    /// 너비만 묻는 intrinsic 요청인지 여부(부모가 너비를 정하지 않았고 가로 가용 공간이 min/max-content 제약).
    ///
    /// 이런 요청의 결과는 너비(min-content·max-content 기여)만 읽힙니다. 엔진의 모든 호출자가 그렇게 쓰므로 배치
    /// 알고리즘은 너비를 정한 뒤 높이·자식 배치를 계산하지 않고 돌려줍니다(높이 0, 자식 배치 없음).
    pub(crate) fn is_width_query(&self) -> bool {
        self.known[0].is_none()
            && matches!(self.available[0], Avail::MinContent | Avail::MaxContent)
    }

    /// root 요청.
    pub(crate) fn root() -> Request {
        Request {
            known: [None, None],
            parent: [None, None],
            available: [
                Avail::Definite(LayoutUnit::from_px_int(ROOT_AVAILABLE_PX).expect("범위 안")),
                Avail::MaxContent,
            ],
            content_only: [false, false],
        }
    }
}

/// 자식 배치 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Placement {
    /// 자식 node. index가 아니라 StableId라 증분 레이아웃(X.7.3)의 node index 이동과 무관합니다.
    pub(crate) child: StableId,
    /// 부모 local 좌표 border box 원점 x.
    pub(crate) x: LayoutUnit,
    /// 부모 local 좌표 border box 원점 y.
    pub(crate) y: LayoutUnit,
    /// 자식에게 준 요청.
    pub(crate) request: Request,
}

/// node 하나의 배치 결과(자식 배치 포함, 자손 결과는 요청으로 따라감).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NodeLayout {
    /// border-box 크기 `[너비, 높이]`.
    pub(crate) size: [LayoutUnit; 2],
    /// border-box top 기준 첫 baseline.
    pub(crate) baseline: Option<LayoutUnit>,
    /// 표시되는 자식 배치(visual order).
    pub(crate) placements: Vec<Placement>,
    /// 이 node 배치가 만든 진단(배치 순서).
    pub(crate) diagnostics: Vec<LayoutDiagnostic>,
}

impl NodeLayout {
    /// 너비만 정한 결과([`Request::is_width_query`]).
    pub(crate) fn width_only(width: LayoutUnit) -> NodeLayout {
        NodeLayout {
            size: [width, LayoutUnit::ZERO],
            baseline: None,
            placements: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
}

// --- 변환된 입력 ---

/// 변환된 `<length-percentage>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LengthP {
    Unit(LayoutUnit),
    Percent(FiniteF32),
}

impl LengthP {
    /// 기준 길이로 해석합니다(백분율 기준이 indefinite면 `None`).
    pub(crate) fn resolve(
        self,
        base: Option<LayoutUnit>,
        node: StableId,
    ) -> Result<Option<LayoutUnit>, LayoutError> {
        match self {
            LengthP::Unit(value) => Ok(Some(value)),
            LengthP::Percent(percent) => base
                .map(|b| b.checked_mul_percent(percent).map_err(at(node)))
                .transpose(),
        }
    }
}

/// 변환된 `width`·`height`·`min-*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SizeP {
    Auto,
    Length(LengthP),
    MinContent,
    MaxContent,
    FitContent,
}

/// 변환된 `max-*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MaxP {
    None,
    Length(LengthP),
    MinContent,
    MaxContent,
    FitContent,
}

/// 변환된 inset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InsetP {
    Auto,
    Length(LengthP),
}

/// 변환된 `flex-basis`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BasisP {
    Auto,
    Content,
    Length(LengthP),
}

/// 변환된 padding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Padding {
    pub(crate) top: LayoutUnit,
    pub(crate) right: LayoutUnit,
    pub(crate) bottom: LayoutUnit,
    pub(crate) left: LayoutUnit,
}

impl Padding {
    /// 축 방향 시작 쪽 padding.
    pub(crate) const fn start(&self, axis: Axis) -> LayoutUnit {
        match axis {
            Axis::Horizontal => self.left,
            Axis::Vertical => self.top,
        }
    }

    /// 축 방향 끝 쪽 padding.
    pub(crate) const fn end(&self, axis: Axis) -> LayoutUnit {
        match axis {
            Axis::Horizontal => self.right,
            Axis::Vertical => self.bottom,
        }
    }

    /// 축 방향 양쪽 padding 합.
    pub(crate) fn sum(&self, axis: Axis) -> Result<LayoutUnit, LayoutUnitError> {
        self.start(axis).checked_add(self.end(axis))
    }
}

/// 변환된 콘텐츠.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreparedContent {
    /// 콘텐츠 없음(container).
    None,
    /// 고정 intrinsic (너비·높이, baseline).
    Intrinsic([LayoutUnit; 2], Option<LayoutUnit>),
    /// 측정기로 측정(X.6.4).
    Measured(ContentKind),
}

impl PreparedContent {
    /// 자식을 갖지 않는 콘텐츠 leaf인지 여부.
    pub(crate) const fn is_leaf(self) -> bool {
        match self {
            PreparedContent::None => false,
            PreparedContent::Intrinsic(..) => true,
            PreparedContent::Measured(kind) => !kind.allows_children(),
        }
    }
}

/// 변환된 node 입력(길이는 [`LayoutUnit`]).
#[derive(Debug, Clone)]
pub(crate) struct Prepared {
    pub(crate) id: StableId,
    pub(crate) display: Display,
    pub(crate) position: Position,
    /// 축별 (시작, 끝) inset.
    pub(crate) inset: [[InsetP; 2]; 2],
    pub(crate) size: [SizeP; 2],
    pub(crate) min: [SizeP; 2],
    pub(crate) max: [MaxP; 2],
    /// `aspect-ratio`(너비, 높이) 1/64 정수 성분.
    pub(crate) ratio: Option<(i64, i64)>,
    pub(crate) padding: Padding,
    pub(crate) direction: FlexDirection,
    pub(crate) wrap: FlexWrap,
    pub(crate) justify_content: ContentAlignment,
    pub(crate) align_content: ContentAlignment,
    pub(crate) align_items: ItemAlignment,
    pub(crate) justify_items: ItemAlignment,
    /// 축별 gap(가로 = `column-gap`, 세로 = `row-gap`).
    pub(crate) gap: [LayoutUnit; 2],
    pub(crate) grow: u64,
    pub(crate) shrink: u64,
    pub(crate) basis: BasisP,
    pub(crate) align_self: SelfAlignment,
    pub(crate) justify_self: SelfAlignment,
    pub(crate) content: PreparedContent,
    /// grid container·item 값.
    pub(crate) grid: crate::grid::GridPrepared,
}

impl Prepared {
    /// 부모 flow에 참여하는지 여부.
    pub(crate) fn in_flow(&self) -> bool {
        self.display != Display::None && self.position != Position::Absolute
    }
}

fn prepare_length(
    node: StableId,
    property: LayoutProperty,
    value: stellar_core_types::value::Length,
    allow_negative: bool,
) -> Result<LengthP, LayoutError> {
    Ok(LengthP::Unit(length_unit(
        node,
        property,
        value,
        allow_negative,
    )?))
}

fn prepare_size(
    node: StableId,
    property: LayoutProperty,
    value: Size,
) -> Result<SizeP, LayoutError> {
    Ok(match value {
        Size::Auto => SizeP::Auto,
        Size::Length(l) => SizeP::Length(prepare_length(node, property, l, false)?),
        Size::Percentage(p) => SizeP::Length(LengthP::Percent(p.raw())),
        Size::MinContent => SizeP::MinContent,
        Size::MaxContent => SizeP::MaxContent,
        Size::FitContent => SizeP::FitContent,
    })
}

fn prepare_max(
    node: StableId,
    property: LayoutProperty,
    value: MaxSize,
) -> Result<MaxP, LayoutError> {
    Ok(match value {
        MaxSize::None => MaxP::None,
        MaxSize::Length(l) => MaxP::Length(prepare_length(node, property, l, false)?),
        MaxSize::Percentage(p) => MaxP::Length(LengthP::Percent(p.raw())),
        MaxSize::MinContent => MaxP::MinContent,
        MaxSize::MaxContent => MaxP::MaxContent,
        MaxSize::FitContent => MaxP::FitContent,
    })
}

fn prepare_inset(
    node: StableId,
    property: LayoutProperty,
    value: LengthPercentageAuto,
) -> Result<InsetP, LayoutError> {
    Ok(match value {
        LengthPercentageAuto::Auto => InsetP::Auto,
        LengthPercentageAuto::Length(l) => InsetP::Length(prepare_length(node, property, l, true)?),
        LengthPercentageAuto::Percentage(p) => InsetP::Length(LengthP::Percent(p.raw())),
    })
}

fn prepare_padding(node: StableId, insets: &Insets) -> Result<Padding, LayoutError> {
    let (top, right, bottom, left) = insets.components();
    let unit = |property, value: f32| {
        let length = stellar_core_types::value::Length::new(value)
            .map_err(|_| LayoutError::OutOfRange { node, property })?;
        length_unit(node, property, length, false)
    };
    Ok(Padding {
        top: unit(LayoutProperty::PaddingTop, top)?,
        right: unit(LayoutProperty::PaddingRight, right)?,
        bottom: unit(LayoutProperty::PaddingBottom, bottom)?,
        left: unit(LayoutProperty::PaddingLeft, left)?,
    })
}

/// node 하나의 입력을 변환합니다.
pub(crate) fn prepare(node: &LayoutNode) -> Result<Prepared, LayoutError> {
    let id = node.id;
    let s = &node.style;
    let ratio = match s.aspect_ratio {
        Some(ratio) => Some((
            weight(id, LayoutProperty::AspectRatio, ratio.width)? as i64,
            weight(id, LayoutProperty::AspectRatio, ratio.height)? as i64,
        )),
        None => None,
    };
    let content = match &node.content {
        LayoutContent::None => PreparedContent::None,
        LayoutContent::Intrinsic(c) => PreparedContent::Intrinsic(
            [
                length_unit(id, LayoutProperty::Content, c.width, false)?,
                length_unit(id, LayoutProperty::Content, c.height, false)?,
            ],
            c.baseline
                .map(|b| length_unit(id, LayoutProperty::Content, b, true))
                .transpose()?,
        ),
        LayoutContent::Measured(m) => PreparedContent::Measured(m.kind()),
    };
    let basis = match s.flex_basis {
        FlexBasis::Auto => BasisP::Auto,
        FlexBasis::Content => BasisP::Content,
        FlexBasis::Length(l) => {
            BasisP::Length(prepare_length(id, LayoutProperty::FlexBasis, l, false)?)
        }
        FlexBasis::Percentage(p) => BasisP::Length(LengthP::Percent(p.raw())),
    };
    Ok(Prepared {
        id,
        display: s.display,
        position: s.position,
        inset: [
            [
                prepare_inset(id, LayoutProperty::Left, s.left)?,
                prepare_inset(id, LayoutProperty::Right, s.right)?,
            ],
            [
                prepare_inset(id, LayoutProperty::Top, s.top)?,
                prepare_inset(id, LayoutProperty::Bottom, s.bottom)?,
            ],
        ],
        size: [
            prepare_size(id, LayoutProperty::Width, s.width)?,
            prepare_size(id, LayoutProperty::Height, s.height)?,
        ],
        min: [
            prepare_size(id, LayoutProperty::MinWidth, s.min_width)?,
            prepare_size(id, LayoutProperty::MinHeight, s.min_height)?,
        ],
        max: [
            prepare_max(id, LayoutProperty::MaxWidth, s.max_width)?,
            prepare_max(id, LayoutProperty::MaxHeight, s.max_height)?,
        ],
        ratio,
        padding: prepare_padding(id, &s.padding)?,
        direction: s.flex_direction,
        wrap: s.flex_wrap,
        justify_content: s.justify_content,
        align_content: s.align_content,
        align_items: s.align_items,
        justify_items: s.justify_items,
        gap: [
            length_unit(id, LayoutProperty::ColumnGap, s.column_gap, false)?,
            length_unit(id, LayoutProperty::RowGap, s.row_gap, false)?,
        ],
        grow: weight(id, LayoutProperty::FlexGrow, s.flex_grow)?,
        shrink: weight(id, LayoutProperty::FlexShrink, s.flex_shrink)?,
        basis,
        align_self: s.align_self,
        justify_self: s.justify_self,
        content,
        grid: crate::grid::prepare_grid(id, s)?,
    })
}

// --- 수치 도우미 ---

/// 수치 오류를 node 문맥과 함께 레이아웃 오류로 바꿉니다.
pub(crate) fn at(node: StableId) -> impl Fn(LayoutUnitError) -> LayoutError {
    move |error| LayoutError::Arithmetic { node, error }
}

/// `a - b`를 0 이상으로 자릅니다.
pub(crate) fn sub_floor_zero(a: LayoutUnit, b: LayoutUnit) -> Result<LayoutUnit, LayoutUnitError> {
    Ok(a.checked_sub(b)?.max(LayoutUnit::ZERO))
}

/// CSS clamp: `max(min, min(value, max))`(최소가 최대를 이깁니다).
pub(crate) fn clamp(value: LayoutUnit, min: LayoutUnit, max: Option<LayoutUnit>) -> LayoutUnit {
    let capped = match max {
        Some(max) => value.min(max),
        None => value,
    };
    capped.max(min)
}

/// `fit-content` 크기: `min(max-content, max(min-content, available))`.
pub(crate) fn fit_content(
    available: Avail,
    min_content: LayoutUnit,
    max_content: LayoutUnit,
) -> LayoutUnit {
    match available {
        Avail::Definite(space) => max_content.min(min_content.max(space)),
        Avail::MinContent => min_content,
        Avail::MaxContent => max_content,
    }
}

/// 비율 `(너비, 높이)`로 `from` 축 크기에서 다른 축 크기를 구합니다(ties away from zero).
pub(crate) fn transfer_ratio(
    ratio: (i64, i64),
    from: Axis,
    value: LayoutUnit,
) -> Result<LayoutUnit, LayoutUnitError> {
    match from {
        Axis::Horizontal => value.checked_mul_ratio(ratio.1, ratio.0),
        Axis::Vertical => value.checked_mul_ratio(ratio.0, ratio.1),
    }
}

/// 비음수 두 값으로 크기를 만듭니다.
pub(crate) fn size_of(node: StableId, size: [LayoutUnit; 2]) -> Result<LayoutSize, LayoutError> {
    LayoutSize::new(size[0], size[1]).ok_or(LayoutError::InvalidBox {
        node,
        reason: "계산된 크기가 음수입니다",
    })
}

/// box 자신의 크기 해석 결과(배치 알고리즘 입력).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Sizing {
    /// 이미 정해진 border-box 크기(부모 `known`, definite 크기 속성, 비율 변환).
    pub(crate) size: [Option<LayoutUnit>; 2],
    /// 해석한 `min-*`(`auto`는 0, 자동 최소 크기는 부모 배치가 `known`으로 반영).
    pub(crate) min: [LayoutUnit; 2],
    /// 해석한 `max-*`.
    pub(crate) max: [Option<LayoutUnit>; 2],
    /// 비율 축 크기: 콘텐츠 크기를 하한으로 갖는 비율 값(`min-*: auto`인 비율 축, CSS Sizing 4 §5).
    pub(crate) ratio_floor: Option<(Axis, LayoutUnit)>,
    /// `aspect-ratio`.
    pub(crate) ratio: Option<(i64, i64)>,
    /// 비율 축 `min-*`이 `auto`인지 여부(축별).
    pub(crate) min_auto: [bool; 2],
    /// 축 크기가 콘텐츠로 정해지는지 여부(부모가 정하지 않았고 크기 속성이 `auto`·콘텐츠 keyword·indefinite
    /// 백분율). 자식 백분율의 순환 진단에 씁니다.
    pub(crate) content_based: [bool; 2],
}

impl Sizing {
    /// 축의 최종 크기: 정해진 크기, 아니면 콘텐츠 기반 크기 `content`를 clamp한 값.
    pub(crate) fn finish(&self, axis: Axis, content: LayoutUnit) -> LayoutUnit {
        let i = axis.index();
        if let Some(size) = self.size[i] {
            return size;
        }
        match self.ratio_floor {
            Some((ratio_axis, value)) if ratio_axis == axis => {
                clamp(value.max(content), self.min[i], self.max[i])
            }
            _ => clamp(content, self.min[i], self.max[i]),
        }
    }

    /// 너비가 정해진 뒤 비율로 높이를 정합니다(두 축 모두 정해지지 않았던 경우).
    pub(crate) fn after_width(&mut self, width: LayoutUnit) -> Result<(), LayoutUnitError> {
        if self.size[1].is_none() && self.ratio_floor.is_none() {
            if let Some(ratio) = self.ratio {
                let height = transfer_ratio(ratio, Axis::Horizontal, width)?;
                if self.min_auto[1] {
                    self.ratio_floor = Some((Axis::Vertical, height));
                } else {
                    self.size[1] = Some(clamp(height, self.min[1], self.max[1]));
                }
            }
        }
        Ok(())
    }
}

// --- 엔진 ---

/// memo 한 항목: (정규화한 요청, 결과, 이번 계산에서 새로 계산했는지).
type MemoEntry = (Request, Arc<NodeLayout>, bool);

/// in-run memo. node 하나의 요청 종류는 몇 개뿐이라 요청 전체를 hash하는 map보다 node별 목록의 선형 탐색이 빠릅니다
/// (ADR-0084 결정 1). 전체 계산은 node 수만큼 목록을 미리 두고(`Dense`), 일부 subtree만 방문하는 증분 계산은 방문한
/// node만 담습니다(`Sparse`).
enum Memo {
    Dense(Vec<Vec<MemoEntry>>),
    Sparse(FastMap<usize, Vec<MemoEntry>>),
}

impl Memo {
    fn entries(&self, index: usize) -> &[MemoEntry] {
        match self {
            Memo::Dense(slots) => &slots[index],
            Memo::Sparse(map) => map.get(&index).map_or(&[], Vec::as_slice),
        }
    }

    fn slot(&mut self, index: usize) -> &mut Vec<MemoEntry> {
        match self {
            Memo::Dense(slots) => &mut slots[index],
            Memo::Sparse(map) => map.entry(index).or_default(),
        }
    }
}

/// 레이아웃 계산 상태(한 번의 계산 동안만 유지).
pub(crate) struct Engine<'a> {
    pub(crate) input: &'a LayoutInput,
    pub(crate) tree: &'a TreeIndex,
    pub(crate) nodes: &'a [Prepared],
    /// in-run memo([`Memo`]).
    memo: Memo,
    /// 측정기 결과 in-run memo.
    measure_memo: FastMap<(usize, [Option<LayoutUnit>; 2], IntrinsicMode), IntrinsicSize>,
    /// node별 subtree input hash(입력 순서). cache가 없으면 비어 있을 수 있습니다.
    pub(crate) subtree: &'a [LayoutInputHash],
    /// 계산 간 cache(X.7.2).
    pub(crate) cache: Option<&'a mut MeasureCache>,
    /// 콘텐츠 측정기(X.6.4).
    pub(crate) measurer: &'a dyn ContentMeasurer,
    /// 집계(X.7.2).
    pub(crate) stats: LayoutStats,
}

impl<'a> Engine<'a> {
    /// 전체 계산 상태를 만듭니다. `cache`가 있으면 `subtree`는 모든 node의 현재 subtree hash여야 합니다.
    pub(crate) fn new(
        input: &'a LayoutInput,
        tree: &'a TreeIndex,
        nodes: &'a [Prepared],
        subtree: &'a [LayoutInputHash],
        measurer: &'a dyn ContentMeasurer,
        cache: Option<&'a mut MeasureCache>,
    ) -> Engine<'a> {
        let memo = Memo::Dense(vec![Vec::new(); nodes.len()]);
        Engine::with_memo(input, tree, nodes, subtree, measurer, cache, memo)
    }

    /// 일부 subtree만 다시 배치하는 증분 계산 상태를 만듭니다. memo를 node 수만큼 미리 할당하지 않습니다.
    pub(crate) fn new_partial(
        input: &'a LayoutInput,
        tree: &'a TreeIndex,
        nodes: &'a [Prepared],
        subtree: &'a [LayoutInputHash],
        measurer: &'a dyn ContentMeasurer,
        cache: Option<&'a mut MeasureCache>,
    ) -> Engine<'a> {
        let memo = Memo::Sparse(FastMap::default());
        Engine::with_memo(input, tree, nodes, subtree, measurer, cache, memo)
    }

    fn with_memo(
        input: &'a LayoutInput,
        tree: &'a TreeIndex,
        nodes: &'a [Prepared],
        subtree: &'a [LayoutInputHash],
        measurer: &'a dyn ContentMeasurer,
        cache: Option<&'a mut MeasureCache>,
        memo: Memo,
    ) -> Engine<'a> {
        Engine {
            input,
            tree,
            nodes,
            memo,
            measure_memo: FastMap::default(),
            subtree,
            cache,
            measurer,
            stats: LayoutStats::default(),
        }
    }

    /// StableId의 node index.
    pub(crate) fn index_of(&self, id: StableId) -> Result<usize, LayoutError> {
        self.tree
            .index
            .get(&id)
            .copied()
            .ok_or(LayoutError::UnknownNode { node: id })
    }

    /// 표시되는(`display: none`이 아닌) 자식 index(visual order).
    pub(crate) fn children(&self, index: usize) -> Vec<usize> {
        self.tree.children[index]
            .iter()
            .copied()
            .filter(|&child| self.nodes[child].display != Display::None)
            .collect()
    }

    /// node를 요청으로 배치합니다(memo·cache 사용).
    pub(crate) fn layout(
        &mut self,
        index: usize,
        request: Request,
    ) -> Result<Arc<NodeLayout>, LayoutError> {
        let request = self.normalize(index, request);
        if let Some(layout) = self.memo_get(index, &request) {
            self.stats.memo_hits = self.stats.memo_hits.saturating_add(1);
            return Ok(layout);
        }
        let id = self.nodes[index].id;
        let revision = self.measurer.revision();
        if let Some(cache) = self.cache.as_deref_mut() {
            if let Some(layout) = cache.get(id, self.subtree[index], revision, &request) {
                self.stats.cache_hits = self.stats.cache_hits.saturating_add(1);
                self.memo.slot(index).push((request, layout.clone(), false));
                return Ok(layout);
            }
        }
        let layout = Arc::new(self.compute(index, request)?);
        self.stats.measured = self.stats.measured.saturating_add(1);
        self.memo.slot(index).push((request, layout.clone(), true));
        if let Some(cache) = self.cache.as_deref_mut() {
            cache.insert(id, self.subtree[index], revision, request, layout.clone());
        }
        Ok(layout)
    }

    /// 정규화한 요청의 in-run memo 결과.
    fn memo_get(&self, index: usize, request: &Request) -> Option<Arc<NodeLayout>> {
        self.memo
            .entries(index)
            .iter()
            .find(|(memo, _, _)| memo == request)
            .map(|(_, layout, _)| layout.clone())
    }

    /// 결과에 영향을 주지 않는 요청 값을 정규형으로 바꿉니다(memo·cache 적중률).
    ///
    /// - `parent`(백분율 기준)는 그 축의 `width`·`height`·`min-*`·`max-*`가 백분율일 때만 결과에 쓰입니다. 자식에게
    ///   주는 백분율 기준은 이 node 자신의 크기에서 나옵니다.
    /// - `available`은 `known` 축에서 쓰이지 않습니다(크기가 이미 정해짐).
    /// - 세로 `available`의 `MinContent`·`MaxContent`는 구분되지 않습니다. 배치 알고리즘은 세로 가용 공간을
    ///   `.definite()`로만 읽고(가로 쓰기 방향에서 min-content 높이 = max-content 높이), 같은 node의 probe에 그대로
    ///   넘길 뿐입니다. 그래서 세로 `MinContent`를 `MaxContent`로 바꿉니다(ADR-0084 결정 1).
    /// - 자식이 없는 node는 세로 가용 공간을 읽지 않습니다(자식에게 넘길 일도 없음).
    /// - 축 크기가 `<length>`(px)이고 그 축의 `min-*`·`max-*`가 keyword가 아니며 `content_only`가 아니면, 그 축의 크기는
    ///   가용 공간과 무관합니다. 가로 축은 너비만 묻는 요청([`Request::is_width_query`]) 여부만 남기고, 세로 축은
    ///   `MaxContent`로 바꿉니다. 자식 요청은 이 node의 정해진 content 크기에서 나오므로 결과가 같습니다.
    pub(crate) fn normalize(&self, index: usize, mut request: Request) -> Request {
        let prepared = &self.nodes[index];
        for i in 0..2 {
            let percent = |size: SizeP| matches!(size, SizeP::Length(LengthP::Percent(_)));
            let uses_percent = percent(prepared.size[i])
                || percent(prepared.min[i])
                || matches!(prepared.max[i], MaxP::Length(LengthP::Percent(_)));
            if !uses_percent {
                request.parent[i] = None;
            }
            if let Some(known) = request.known[i] {
                request.available[i] = Avail::Definite(known);
                continue;
            }
            let fixed = !request.content_only[i]
                && matches!(prepared.size[i], SizeP::Length(LengthP::Unit(_)))
                && matches!(prepared.min[i], SizeP::Auto | SizeP::Length(_))
                && matches!(prepared.max[i], MaxP::None | MaxP::Length(_));
            if fixed {
                request.available[i] = match (i, request.available[i]) {
                    (0, Avail::Definite(_)) => Avail::Definite(LayoutUnit::ZERO),
                    _ => Avail::MaxContent,
                };
            }
        }
        if request.known[1].is_none()
            && (request.available[1] == Avail::MinContent || self.tree.children[index].is_empty())
        {
            request.available[1] = Avail::MaxContent;
        }
        request
    }

    /// 요청이 이번 계산에서 새로 계산되었는지 여부.
    pub(crate) fn is_fresh(&self, index: usize, request: &Request) -> bool {
        let request = self.normalize(index, *request);
        self.memo
            .entries(index)
            .iter()
            .any(|(memo, _, fresh)| *memo == request && *fresh)
    }

    /// 콘텐츠 측정기를 호출합니다(in-run memo).
    pub(crate) fn measure(
        &mut self,
        index: usize,
        available: [Option<LayoutUnit>; 2],
        mode: IntrinsicMode,
    ) -> Result<IntrinsicSize, LayoutError> {
        if let Some(size) = self.measure_memo.get(&(index, available, mode)) {
            return Ok(*size);
        }
        let node = &self.input.nodes[index];
        let LayoutContent::Measured(content) = &node.content else {
            return Err(LayoutError::InvalidBox {
                node: node.id,
                reason: "측정 콘텐츠가 아닙니다",
            });
        };
        let kind = content.kind();
        let request = MeasureRequest {
            node: node.id,
            kind,
            available,
            mode,
            subtree: None,
        };
        let failed = |reason| LayoutError::ContentMeasure {
            node: node.id,
            kind,
            reason,
        };
        self.stats.measurer_calls = self.stats.measurer_calls.saturating_add(1);
        let size = self
            .measurer
            .measure(&request, content)
            .map_err(|error| failed(error.reason))?;
        if size.width < LayoutUnit::ZERO || size.height < LayoutUnit::ZERO {
            return Err(failed("측정 결과 크기가 음수입니다"));
        }
        self.measure_memo.insert((index, available, mode), size);
        Ok(size)
    }

    /// box 자신의 content 기반 min-content·max-content 너비(border box, `width`·`min`·`max` 무시).
    pub(crate) fn content_widths(
        &mut self,
        index: usize,
        request: Request,
    ) -> Result<(LayoutUnit, LayoutUnit), LayoutError> {
        let mut probe = Request {
            known: [None, request.known[1]],
            parent: request.parent,
            available: [Avail::MinContent, request.available[1]],
            content_only: [true, request.content_only[1]],
        };
        let min = self.layout(index, probe)?.size[0];
        probe.available[0] = Avail::MaxContent;
        let max = self.layout(index, probe)?.size[0];
        Ok((min, max.max(min)))
    }

    /// 너비 keyword(`min-content`·`max-content`·`fit-content`) 값.
    fn keyword_width(
        &mut self,
        index: usize,
        request: Request,
        keyword: SizeP,
    ) -> Result<LayoutUnit, LayoutError> {
        let (min, max) = self.content_widths(index, request)?;
        Ok(match keyword {
            SizeP::MinContent => min,
            SizeP::MaxContent => max,
            _ => fit_content(request.available[0], min, max),
        })
    }

    /// 축의 선호 크기(`width`·`height`)를 해석합니다. `auto`와 indefinite 백분율·세로 keyword는 `None`.
    pub(crate) fn preferred(
        &mut self,
        index: usize,
        axis: Axis,
        request: Request,
    ) -> Result<Option<LayoutUnit>, LayoutError> {
        let id = self.nodes[index].id;
        let i = axis.index();
        match self.nodes[index].size[i] {
            SizeP::Auto => Ok(None),
            SizeP::Length(length) => length.resolve(request.parent[i], id),
            keyword @ (SizeP::MinContent | SizeP::MaxContent | SizeP::FitContent) => {
                if axis == Axis::Horizontal {
                    Ok(Some(self.keyword_width(index, request, keyword)?))
                } else {
                    Ok(None)
                }
            }
        }
    }

    /// 축의 `min-*`(`auto`는 0)를 해석합니다.
    pub(crate) fn min_size(
        &mut self,
        index: usize,
        axis: Axis,
        request: Request,
    ) -> Result<LayoutUnit, LayoutError> {
        let id = self.nodes[index].id;
        let i = axis.index();
        Ok(match self.nodes[index].min[i] {
            SizeP::Auto => LayoutUnit::ZERO,
            SizeP::Length(length) => length.resolve(request.parent[i], id)?.unwrap_or_default(),
            keyword @ (SizeP::MinContent | SizeP::MaxContent | SizeP::FitContent) => {
                if axis == Axis::Horizontal {
                    self.keyword_width(index, request, keyword)?
                } else {
                    LayoutUnit::ZERO
                }
            }
        })
    }

    /// 축의 `max-*`를 해석합니다(`none`·indefinite 백분율은 `None`).
    pub(crate) fn max_size(
        &mut self,
        index: usize,
        axis: Axis,
        request: Request,
    ) -> Result<Option<LayoutUnit>, LayoutError> {
        let id = self.nodes[index].id;
        let i = axis.index();
        Ok(match self.nodes[index].max[i] {
            MaxP::None => None,
            MaxP::Length(length) => length.resolve(request.parent[i], id)?,
            MaxP::MinContent => {
                (axis == Axis::Horizontal).then_some(self.content_widths(index, request)?.0)
            }
            MaxP::MaxContent => {
                (axis == Axis::Horizontal).then_some(self.content_widths(index, request)?.1)
            }
            MaxP::FitContent => {
                if axis == Axis::Horizontal {
                    Some(self.keyword_width(index, request, SizeP::FitContent)?)
                } else {
                    None
                }
            }
        })
    }

    /// 요청 아래 box 자신의 크기 해석(배치 알고리즘 전 단계).
    pub(crate) fn sizing(&mut self, index: usize, request: Request) -> Result<Sizing, LayoutError> {
        let id = self.nodes[index].id;
        let ratio = self.nodes[index].ratio;
        let mut sizing = Sizing {
            size: [None, None],
            min: [LayoutUnit::ZERO; 2],
            max: [None, None],
            ratio_floor: None,
            ratio,
            min_auto: [false, false],
            content_based: [false, false],
        };
        for axis in Axis::BOTH {
            let i = axis.index();
            sizing.content_based[i] = request.known[i].is_none()
                && match self.nodes[index].size[i] {
                    SizeP::Length(length) => length.resolve(request.parent[i], id)?.is_none(),
                    _ => true,
                };
            if request.content_only[i] {
                sizing.size[i] = request.known[i];
                continue;
            }
            sizing.min[i] = self.min_size(index, axis, request)?;
            sizing.max[i] = self.max_size(index, axis, request)?;
            sizing.min_auto[i] = self.nodes[index].min[i] == SizeP::Auto;
            sizing.size[i] = match request.known[i] {
                Some(known) => Some(known),
                None => self
                    .preferred(index, axis, request)?
                    .map(|value| clamp(value, sizing.min[i], sizing.max[i])),
            };
        }
        if let Some(ratio) = ratio {
            for from in Axis::BOTH {
                let to = from.cross();
                let (fi, ti) = (from.index(), to.index());
                if let (Some(value), None) = (sizing.size[fi], sizing.size[ti]) {
                    if request.content_only[ti] {
                        continue;
                    }
                    let transferred = transfer_ratio(ratio, from, value).map_err(at(id))?;
                    if sizing.min_auto[ti] {
                        sizing.ratio_floor = Some((to, transferred));
                    } else {
                        sizing.size[ti] = Some(clamp(transferred, sizing.min[ti], sizing.max[ti]));
                    }
                    break;
                }
            }
        }
        Ok(sizing)
    }

    /// 요청 아래 node를 계산합니다(memo 없이).
    fn compute(&mut self, index: usize, request: Request) -> Result<NodeLayout, LayoutError> {
        let sizing = self.sizing(index, request)?;
        let prepared = &self.nodes[index];
        if prepared.content.is_leaf() {
            return self.leaf(index, request, sizing);
        }
        match prepared.display {
            Display::Flex => flex::layout(self, index, request, sizing),
            Display::Grid => crate::grid::layout(self, index, request, sizing),
            Display::Block | Display::None => block::layout(self, index, request, sizing),
        }
    }

    /// 콘텐츠 leaf 배치.
    fn leaf(
        &mut self,
        index: usize,
        request: Request,
        mut sizing: Sizing,
    ) -> Result<NodeLayout, LayoutError> {
        let nodes = self.nodes;
        let prepared = &nodes[index];
        let id = prepared.id;
        let pad = prepared.padding;
        let ph = pad.sum(Axis::Horizontal).map_err(at(id))?;
        let pv = pad.sum(Axis::Vertical).map_err(at(id))?;
        let add = |a: LayoutUnit, b: LayoutUnit| a.checked_add(b).map_err(at(id));
        let width = match sizing.size[0] {
            Some(width) => width,
            None => {
                let image_height = match (prepared.content, sizing.size[1]) {
                    (PreparedContent::Measured(ContentKind::Image), Some(height)) => Some(height),
                    _ => None,
                };
                let content = if let Some(height) = image_height {
                    let ch = sub_floor_zero(height, pv).map_err(at(id))?;
                    let w = self
                        .measure(index, [None, Some(ch)], IntrinsicMode::MaxContent)?
                        .width;
                    add(w, ph)?
                } else {
                    let (min, max) = match prepared.content {
                        PreparedContent::Intrinsic([w, _], _) => (w, w),
                        PreparedContent::Measured(_) => (
                            self.measure(index, [None, None], IntrinsicMode::MinContent)?
                                .width,
                            self.measure(index, [None, None], IntrinsicMode::MaxContent)?
                                .width,
                        ),
                        PreparedContent::None => (LayoutUnit::ZERO, LayoutUnit::ZERO),
                    };
                    let (min, max) = (add(min, ph)?, add(max, ph)?);
                    let max = max.max(min);
                    match sizing.ratio_floor {
                        Some((Axis::Horizontal, value)) => value.max(min),
                        _ => fit_content(request.available[0], min, max),
                    }
                };
                sizing.finish(Axis::Horizontal, content)
            }
        };
        sizing.after_width(width).map_err(at(id))?;
        if request.is_width_query() {
            return Ok(NodeLayout::width_only(width));
        }
        let cw = sub_floor_zero(width, ph).map_err(at(id))?;
        let (content_height, baseline) = match prepared.content {
            PreparedContent::Intrinsic([_, h], baseline) => (h, baseline),
            PreparedContent::Measured(_) => {
                let ch = sizing.size[1]
                    .map(|h| sub_floor_zero(h, pv))
                    .transpose()
                    .map_err(at(id))?;
                let size = self.measure(index, [Some(cw), ch], IntrinsicMode::MaxContent)?;
                (size.height, size.baseline)
            }
            PreparedContent::None => (LayoutUnit::ZERO, None),
        };
        let height = sizing.finish(Axis::Vertical, add(content_height, pv)?);
        let baseline = baseline.map(|b| add(b, pad.top)).transpose()?;
        Ok(NodeLayout {
            size: [width, height],
            baseline,
            placements: Vec::new(),
            diagnostics: Vec::new(),
        })
    }

    /// 결과 box를 pre-order로 조립합니다. 반환값은 자신의 overflow bounds입니다.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn assemble(
        &mut self,
        index: usize,
        request: Request,
        layout: &NodeLayout,
        x: LayoutUnit,
        y: LayoutUnit,
        boxes: &mut Vec<LayoutBox>,
        diagnostics: &mut Vec<LayoutDiagnostic>,
    ) -> Result<LayoutRect, LayoutError> {
        let id = self.nodes[index].id;
        if self.is_fresh(index, &request) {
            self.stats.arranged = self.stats.arranged.saturating_add(1);
        }
        let size = size_of(id, layout.size)?;
        let slot = boxes.len();
        let rect = LayoutRect::from_origin_size(x, y, size);
        boxes.push(LayoutBox::new(
            id,
            rect,
            rect,
            None,
            LayoutRect::at_origin(size),
        )?);
        diagnostics.extend(layout.diagnostics.iter().copied());
        let mut overflow = LayoutRect::at_origin(size);
        for placement in &layout.placements {
            let child_index = self.index_of(placement.child)?;
            let child = self.layout(child_index, placement.request)?;
            let child_overflow = self.assemble(
                child_index,
                placement.request,
                &child,
                placement.x,
                placement.y,
                boxes,
                diagnostics,
            )?;
            let moved = child_overflow
                .translated(placement.x, placement.y)
                .map_err(at(id))?;
            overflow = overflow.union(&moved).map_err(at(id))?;
        }
        let content = content_rect(id, &self.nodes[index].padding, size)?;
        boxes[slot] = LayoutBox::new(id, rect, content, layout.baseline, overflow)?;
        Ok(overflow)
    }
}

/// border box 크기와 padding으로 content box(자신의 local 좌표)를 구합니다.
pub(crate) fn content_rect(
    id: StableId,
    padding: &Padding,
    size: LayoutSize,
) -> Result<LayoutRect, LayoutError> {
    let width = sub_floor_zero(size.width(), padding.sum(Axis::Horizontal).map_err(at(id))?)
        .map_err(at(id))?;
    let height = sub_floor_zero(size.height(), padding.sum(Axis::Vertical).map_err(at(id))?)
        .map_err(at(id))?;
    Ok(LayoutRect::from_origin_size(
        padding.left,
        padding.top,
        LayoutSize::new(width, height).expect("비음수"),
    ))
}

#[cfg(test)]
mod tests;
