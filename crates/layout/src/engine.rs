//! 레이아웃 엔진 진입점과 공통 크기 해석 (X.2.1).
//!
//! # 계산 순서
//!
//! 1. 입력을 검증하고([`LayoutInput::validate`]) 길이를 [`LayoutUnit`]으로 한 번 변환합니다.
//! 2. root 크기를 해석합니다(root는 `Fixed`·`HugContent`만 허용, X.6.1).
//! 3. top-down으로 각 node의 content box 안에 자식을 배치합니다. 자식 크기는 부모 배치 방식이
//!    정한 [`ChildConstraint`]로 해석하고, `HugContent` 축은 자식 배치 extent(측정)로 정합니다.
//! 4. 결과는 pre-order(부모 다음 자식, 자식은 visual order) [`LayoutBox`] 목록입니다.
//!
//! 같은 node·같은 제약의 크기 해석은 한 계산 안에서 memo하여 중첩 Hug의 반복 측정을 막습니다.
//!
//! # 재귀 깊이와 stack
//!
//! 크기 해석·배치는 tree 깊이에 비례해 재귀합니다(배치 → 크기 해석 → 측정 → 자식 크기 해석).
//! level당 stack 사용량은 빌드 설정에 따라 크게 다르므로(측정값: release 약 2 KiB, debug 약 14 KiB)
//! 깊이가 [`INLINE_LAYOUT_DEPTH`] 이하인 tree만 호출 thread에서 계산하고, 그보다 깊은 tree는
//! [`DEEP_LAYOUT_STACK_BYTES`] stack을 가진 전용 thread에서 계산한 뒤 결과를 돌려줍니다. 전용 stack은
//! [`MAX_LAYOUT_DEPTH`] level × 64 KiB라 검증을 통과한 모든 깊이에서 overflow가 나지 않습니다. 계산
//! 결과는 어느 thread에서 계산했는지와 무관합니다.

use std::collections::HashMap;

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{FiniteF32, Insets};

use crate::constraint;
use crate::error::LayoutError;
use crate::flex::{prepare_flex, PreparedFlex};
use crate::free::{self, PreparedAnchors};
use crate::geometry::{Axis, LayoutRect, LayoutSize};
use crate::grid::{self, AreaDependence};
use crate::input::{
    length_unit, ContainerLayout, LayoutContent, LayoutInput, LayoutNode, SizingMode, TreeIndex,
    MAX_LAYOUT_DEPTH,
};
use crate::intrinsic::{
    ContentKind, ContentMeasurer, DefaultMeasurer, IntrinsicMeasure, IntrinsicMode, IntrinsicSize,
    MeasureRequest,
};
use crate::output::{LayoutBox, LayoutDiagnostic, LayoutResult};
use crate::overlay;
use crate::property::LayoutProperty;
use crate::rounding::{LayoutUnit, LayoutUnitError};
use crate::sizing::{prepare_limits, prepare_ratio, AxisLimits, PreparedRatio};
use crate::stack;

/// 호출 thread에서 바로 계산하는 최대 tree 깊이. 더 깊으면 전용 stack thread에서 계산합니다.
///
/// debug 빌드에서도 이 깊이는 기본 thread stack(2 MiB)의 4분의 1 안팎만 씁니다.
pub const INLINE_LAYOUT_DEPTH: usize = 32;

/// [`INLINE_LAYOUT_DEPTH`]보다 깊은 tree를 계산하는 전용 thread의 stack 크기(level당 64 KiB).
pub const DEEP_LAYOUT_STACK_BYTES: usize = MAX_LAYOUT_DEPTH * 64 * 1024;

/// 입력 전체를 기본 측정기([`DefaultMeasurer`])로 레이아웃합니다.
///
/// tree 깊이가 [`INLINE_LAYOUT_DEPTH`]를 넘으면 [`DEEP_LAYOUT_STACK_BYTES`] stack의 전용 thread에서
/// 계산합니다(결과는 같음). 그 thread를 만들 수 없으면 [`LayoutError::DeepStackUnavailable`]입니다.
pub fn compute_layout(input: &LayoutInput) -> Result<LayoutResult, LayoutError> {
    compute_layout_with(input, &DefaultMeasurer)
}

/// 입력 전체를 주어진 콘텐츠 측정기로 레이아웃합니다 (X.6.4).
///
/// [`LayoutContent::Measured`] 콘텐츠는 `measurer`로 측정합니다. 깊은 tree 처리는
/// [`compute_layout`]과 같습니다.
pub fn compute_layout_with(
    input: &LayoutInput,
    measurer: &dyn ContentMeasurer,
) -> Result<LayoutResult, LayoutError> {
    let tree = input.tree()?;
    if tree.depth <= INLINE_LAYOUT_DEPTH {
        return compute_validated(input, tree, measurer);
    }
    let depth = tree.depth;
    std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name("stellar-layout-deep".to_owned())
            .stack_size(DEEP_LAYOUT_STACK_BYTES)
            .spawn_scoped(scope, move || compute_validated(input, tree, measurer))
            .map_err(|_| LayoutError::DeepStackUnavailable { depth })?;
        match worker.join() {
            Ok(result) => result,
            // 계산 중 panic은 호출 thread에서 그대로 다시 일으킵니다.
            Err(panic) => std::panic::resume_unwind(panic),
        }
    })
}

/// 검증된 tree를 현재 thread에서 레이아웃합니다.
fn compute_validated(
    input: &LayoutInput,
    tree: TreeIndex,
    measurer: &dyn ContentMeasurer,
) -> Result<LayoutResult, LayoutError> {
    let mut engine = Engine::new(input, tree, measurer)?;
    let root = engine.tree.root;
    let size = engine.resolve_size(root, ChildConstraint::UNCONSTRAINED)?;
    let mut pending = Vec::with_capacity(input.nodes.len());
    engine.arrange(
        root,
        LayoutRect::at_origin(size),
        [None, None],
        &mut pending,
    )?;
    let boxes = pending
        .into_iter()
        .map(|p| LayoutBox::new(p.id, p.rect, p.content, p.baseline, p.overflow))
        .collect::<Result<Vec<_>, _>>()?;
    LayoutResult::with_diagnostics(boxes, engine.diagnostics)
}

/// 부모 배치 방식이 자식 크기 해석에 주는 축별 제약.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ChildConstraint {
    /// containing block content 크기(Percentage 기준). `None`이면 indefinite.
    pub(crate) cb: [Option<LayoutUnit>; 2],
    /// `Fill`이 채울 크기. `None`이면 `Fill`은 콘텐츠 크기(Hug)로 해석합니다.
    pub(crate) fill: [Option<LayoutUnit>; 2],
    /// 부모가 sizing mode와 무관하게 정한 크기(Stretch 등).
    pub(crate) forced: [Option<LayoutUnit>; 2],
}

impl ChildConstraint {
    /// 제약 없음(root).
    pub(crate) const UNCONSTRAINED: ChildConstraint = ChildConstraint {
        cb: [None, None],
        fill: [None, None],
        forced: [None, None],
    };
}

/// node 한 축의 변환된 sizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AxisSizing {
    /// 고정 크기.
    Fixed(LayoutUnit),
    /// 부모 영역 채움.
    Fill,
    /// 콘텐츠 크기.
    Hug,
    /// containing block content 크기의 백분율(X.2.3).
    Percentage(FiniteF32),
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

    /// 축 방향 양쪽 padding 합.
    pub(crate) fn sum(&self, axis: Axis) -> Result<LayoutUnit, LayoutUnitError> {
        match axis {
            Axis::Horizontal => self.left.checked_add(self.right),
            Axis::Vertical => self.top.checked_add(self.bottom),
        }
    }
}

/// 변환된 콘텐츠.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreparedContent {
    /// 콘텐츠 없음(container).
    None,
    /// 고정 intrinsic (너비·높이, baseline).
    Fixed([LayoutUnit; 2], Option<LayoutUnit>),
    /// 측정기로 측정(X.6.4).
    Measured(ContentKind),
}

/// 변환된 node 입력.
#[derive(Debug, Clone)]
pub(crate) struct Prepared {
    pub(crate) id: StableId,
    pub(crate) padding: Padding,
    pub(crate) sizing: [AxisSizing; 2],
    /// 콘텐츠.
    pub(crate) content: PreparedContent,
    /// Free 배치 anchor (가로, 세로).
    pub(crate) anchors: [PreparedAnchors; 2],
    /// Stack 자식 grow·shrink·basis.
    pub(crate) flex: PreparedFlex,
    /// 축별 min/max(X.6.2).
    pub(crate) limits: [AxisLimits; 2],
    /// 너비 : 높이 비율(X.6.3).
    pub(crate) ratio: Option<PreparedRatio>,
}

/// HugContent 순환 검사(X.6.5)용 축별 콘텐츠 크기 조상.
type CycleLink = [Option<StableId>; 2];

/// 결과 box 조립 전 값.
#[derive(Debug, Clone, Copy)]
struct PendingBox {
    id: StableId,
    rect: LayoutRect,
    content: LayoutRect,
    baseline: Option<LayoutUnit>,
    overflow: LayoutRect,
}

/// 레이아웃 계산 상태(한 번의 계산 동안만 유지).
pub(crate) struct Engine<'a> {
    pub(crate) input: &'a LayoutInput,
    pub(crate) tree: TreeIndex,
    pub(crate) nodes: Vec<Prepared>,
    memo: HashMap<(usize, ChildConstraint), LayoutSize>,
    /// 측정 콘텐츠 결과 memo((node index, 가용 크기, mode) → 측정값).
    content_memo: HashMap<(usize, [Option<LayoutUnit>; 2], IntrinsicMode), IntrinsicSize>,
    /// 콘텐츠 측정기(X.6.4).
    measurer: &'a dyn ContentMeasurer,
    /// Grid item(node index)의 영역 축별 의존성(X.6.5, Grid 배치가 기록).
    pub(crate) grid_areas: HashMap<usize, [AreaDependence; 2]>,
    /// 레이아웃을 실패시키지 않는 진단(배치 순서대로).
    pub(crate) diagnostics: Vec<LayoutDiagnostic>,
}

/// 수치 오류를 node 문맥과 함께 레이아웃 오류로 바꿉니다.
pub(crate) fn at(node: StableId) -> impl Fn(LayoutUnitError) -> LayoutError {
    move |error| LayoutError::Arithmetic { node, error }
}

/// `a - b`를 0 이상으로 자릅니다.
pub(crate) fn sub_floor_zero(a: LayoutUnit, b: LayoutUnit) -> Result<LayoutUnit, LayoutUnitError> {
    Ok(a.checked_sub(b)?.max(LayoutUnit::ZERO))
}

/// 비음수 두 값으로 크기를 만듭니다.
pub(crate) fn size_of(
    node: StableId,
    width: LayoutUnit,
    height: LayoutUnit,
) -> Result<LayoutSize, LayoutError> {
    LayoutSize::new(width, height).ok_or(LayoutError::InvalidBox {
        node,
        reason: "계산된 크기가 음수입니다",
    })
}

impl<'a> Engine<'a> {
    fn new(
        input: &'a LayoutInput,
        tree: TreeIndex,
        measurer: &'a dyn ContentMeasurer,
    ) -> Result<Engine<'a>, LayoutError> {
        let nodes = input
            .nodes
            .iter()
            .map(prepare)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Engine {
            input,
            tree,
            nodes,
            memo: HashMap::new(),
            content_memo: HashMap::new(),
            measurer,
            grid_areas: HashMap::new(),
            diagnostics: Vec::new(),
        })
    }

    /// 측정 콘텐츠를 측정합니다(같은 요청은 한 계산에서 한 번만 측정기에 보냄). 결과 크기는 비음수로
    /// 검증합니다. `subtree`는 ComponentInstance 자식 extent입니다.
    fn measure_content(
        &mut self,
        index: usize,
        available: [Option<LayoutUnit>; 2],
        mode: IntrinsicMode,
        subtree: Option<[LayoutUnit; 2]>,
    ) -> Result<IntrinsicSize, LayoutError> {
        if let Some(size) = self.content_memo.get(&(index, available, mode)) {
            return Ok(*size);
        }
        let node = self.node(index);
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
            subtree,
        };
        let failed = |reason| LayoutError::ContentMeasure {
            node: node.id,
            kind,
            reason,
        };
        let size = self
            .measurer
            .measure(&request, content)
            .map_err(|error| failed(error.reason))?;
        if size.width < LayoutUnit::ZERO || size.height < LayoutUnit::ZERO {
            return Err(failed("측정 결과 크기가 음수입니다"));
        }
        self.content_memo.insert((index, available, mode), size);
        Ok(size)
    }

    /// node top 기준 첫 baseline(border box 좌표). `size`는 node border-box 크기입니다.
    ///
    /// 고정 intrinsic은 그 baseline, 측정 콘텐츠는 content 너비가 정해진 측정의 baseline에 위쪽
    /// padding을 더한 값입니다. container는 `None`입니다(X.3.6·X.10.4 소관).
    pub(crate) fn border_baseline(
        &mut self,
        index: usize,
        size: LayoutSize,
    ) -> Result<Option<LayoutUnit>, LayoutError> {
        let prepared = &self.nodes[index];
        let id = prepared.id;
        let padding = prepared.padding;
        let baseline = match prepared.content {
            PreparedContent::None | PreparedContent::Measured(ContentKind::ComponentInstance) => {
                None
            }
            PreparedContent::Fixed(_, baseline) => baseline,
            PreparedContent::Measured(_) => {
                let width =
                    sub_floor_zero(size.width(), padding.sum(Axis::Horizontal).map_err(at(id))?)
                        .map_err(at(id))?;
                self.measure_content(index, [Some(width), None], IntrinsicMode::MaxContent, None)?
                    .baseline
            }
        };
        match baseline {
            Some(baseline) => Ok(Some(padding.top.checked_add(baseline).map_err(at(id))?)),
            None => Ok(None),
        }
    }

    /// node 원본 입력.
    pub(crate) fn node(&self, index: usize) -> &'a LayoutNode {
        &self.input.nodes[index]
    }

    /// 레이아웃에 참여하는(표시되는) 자식 index 목록(visual order, X.3.5).
    ///
    /// `visible = false` 자식은 모든 배치 방식에서 flow·Hug 측정·overflow에서 빠지고, 그 subtree는
    /// 결과에 포함되지 않습니다.
    pub(crate) fn children(&self, index: usize) -> Vec<usize> {
        self.tree.children[index]
            .iter()
            .copied()
            .filter(|&child| self.input.nodes[child].visible)
            .collect()
    }

    /// 제약 아래에서 node의 border-box 크기를 해석합니다.
    pub(crate) fn resolve_size(
        &mut self,
        index: usize,
        constraint: ChildConstraint,
    ) -> Result<LayoutSize, LayoutError> {
        if let Some(size) = self.memo.get(&(index, constraint)) {
            return Ok(*size);
        }
        let prepared = &self.nodes[index];
        let id = prepared.id;
        let padding = prepared.padding;
        let limits = prepared.limits;
        let mut definite = [None, None];
        for axis in Axis::BOTH {
            let i = axis.index();
            if let Some(forced) = constraint.forced[i] {
                definite[i] = Some(limits[i].clamp(forced));
                continue;
            }
            definite[i] = match prepared.sizing[i] {
                AxisSizing::Fixed(value) => Some(value),
                AxisSizing::Fill => constraint.fill[i],
                AxisSizing::Hug => None,
                AxisSizing::Percentage(percent) => match constraint.cb[i] {
                    Some(base) => Some(base.checked_mul_percent(percent).map_err(at(id))?),
                    // containing block이 indefinite면 콘텐츠 크기로 측정합니다(X.6.1).
                    None => None,
                },
            };
            // X.6.2: definite 값은 clamp한 뒤 content box 계산에 씁니다.
            definite[i] = definite[i].map(|value| limits[i].clamp(value));
        }
        if let Some(ratio) = prepared.ratio {
            // X.6.3: 한 축만 definite면 비율로 다른 축을 정하고, 둘 다 indefinite면 가로를
            // intrinsic(콘텐츠) 크기로 정한 뒤 비율을 적용합니다. 둘 다 definite면 적용하지 않습니다.
            if definite == [None, None] {
                let content = self.content_extent(index, [None, None])?;
                let width = content[0]
                    .checked_add(padding.sum(Axis::Horizontal).map_err(at(id))?)
                    .map_err(at(id))?;
                definite[0] = Some(limits[0].clamp(width));
            }
            match definite {
                [Some(width), None] => {
                    let height = ratio.height_for(width).map_err(at(id))?;
                    definite[1] = Some(limits[1].clamp(height.max(LayoutUnit::ZERO)));
                }
                [None, Some(height)] => {
                    let width = ratio.width_for(height).map_err(at(id))?;
                    definite[0] = Some(limits[0].clamp(width.max(LayoutUnit::ZERO)));
                }
                _ => {}
            }
        }
        let mut resolved = [LayoutUnit::ZERO; 2];
        if definite.iter().any(Option::is_none) {
            let mut inner = [None, None];
            for axis in Axis::BOTH {
                let i = axis.index();
                inner[i] = match definite[i] {
                    Some(value) => Some(
                        sub_floor_zero(value, padding.sum(axis).map_err(at(id))?)
                            .map_err(at(id))?,
                    ),
                    None => None,
                };
            }
            let content = self.content_extent(index, inner)?;
            for axis in Axis::BOTH {
                let i = axis.index();
                resolved[i] = match definite[i] {
                    Some(value) => value,
                    None => limits[i].clamp(
                        content[i]
                            .checked_add(padding.sum(axis).map_err(at(id))?)
                            .map_err(at(id))?,
                    ),
                };
            }
        } else {
            resolved = [
                definite[0].unwrap_or(LayoutUnit::ZERO),
                definite[1].unwrap_or(LayoutUnit::ZERO),
            ];
        }
        let size = size_of(id, resolved[0], resolved[1])?;
        self.memo.insert((index, constraint), size);
        Ok(size)
    }

    /// node 콘텐츠 extent(padding 제외). `inner`는 축별 content box 크기(indefinite면 `None`).
    fn content_extent(
        &mut self,
        index: usize,
        inner: [Option<LayoutUnit>; 2],
    ) -> Result<[LayoutUnit; 2], LayoutError> {
        let mode = IntrinsicMode::MaxContent;
        match self.nodes[index].content {
            PreparedContent::Fixed(size, _) => return Ok(size),
            PreparedContent::Measured(kind) if !kind.allows_children() => {
                let size = self.measure_content(index, inner, mode, None)?;
                return Ok([size.width, size.height]);
            }
            PreparedContent::None | PreparedContent::Measured(_) => {}
        }
        let extent = match &self.node(index).container {
            ContainerLayout::Free => free::measure(self, index, inner),
            ContainerLayout::Stack(_) => stack::measure(self, index, inner),
            ContainerLayout::Grid(_) => grid::measure(self, index, inner),
            ContainerLayout::Overlay(_) => overlay::measure(self, index, inner),
            ContainerLayout::Constraint => constraint::measure(self, index, inner),
        }?;
        if self.nodes[index].content == PreparedContent::Measured(ContentKind::ComponentInstance) {
            // ComponentInstance: 자식(virtual subtree) extent를 측정기에 넘깁니다.
            let size = self.measure_content(index, inner, mode, Some(extent))?;
            return Ok([size.width, size.height]);
        }
        Ok(extent)
    }

    /// node 자신의 크기가 콘텐츠로 정해지는 조상(축별, X.6.5). `link`는 node의 containing block이
    /// 측정 기여를 통해 의존하는 콘텐츠 크기 조상입니다.
    ///
    /// - `HugContent` 축: node 자신(비율로 다른 `Fixed` 축에서 정해지면 제외).
    /// - `Fill`·`Percentage` 축: containing block을 그대로 따르므로 `link`.
    /// - `Fixed` 축: 없음.
    fn content_source(&self, index: usize, link: CycleLink) -> CycleLink {
        let prepared = &self.nodes[index];
        let mut own = [None, None];
        for axis in Axis::BOTH {
            let i = axis.index();
            own[i] = match prepared.sizing[i] {
                AxisSizing::Hug => {
                    let other = prepared.sizing[1 - i];
                    if prepared.ratio.is_some() && matches!(other, AxisSizing::Fixed(_)) {
                        None
                    } else {
                        Some(prepared.id)
                    }
                }
                AxisSizing::Fill | AxisSizing::Percentage(_) => link[i],
                AxisSizing::Fixed(_) => None,
            };
        }
        own
    }

    /// 자식의 containing block이 측정 기여를 통해 의존하는 콘텐츠 크기 조상(축별, X.6.5).
    ///
    /// Free는 부모 extent에 기여하는 자식(끝 anchor·백분율 시작 anchor 없음)만, Stack·Overlay·
    /// Constraint는 모든 자식이 부모 콘텐츠 box를 따릅니다. Grid item은 영역 track이 콘텐츠 기반이면
    /// Grid 자신, fr track이면 Grid 축의 콘텐츠 크기 조상을 따릅니다.
    fn child_link(&self, parent: usize, child: usize, own: CycleLink) -> CycleLink {
        let mut link = [None, None];
        for axis in Axis::BOTH {
            let i = axis.index();
            link[i] = match &self.node(parent).container {
                ContainerLayout::Free => {
                    if self.nodes[child].anchors[i].contributes_to_extent() {
                        own[i]
                    } else {
                        None
                    }
                }
                ContainerLayout::Stack(_)
                | ContainerLayout::Overlay(_)
                | ContainerLayout::Constraint => own[i],
                ContainerLayout::Grid(_) => {
                    let area = self.grid_areas.get(&child).copied().unwrap_or_default()[i];
                    if area.content {
                        Some(self.nodes[parent].id)
                    } else if area.flexible {
                        own[i]
                    } else {
                        None
                    }
                }
            };
        }
        link
    }

    /// node를 `rect`(부모 local 좌표)에 놓고 자손을 배치합니다. 자신의 overflow bounds를 반환합니다.
    ///
    /// `link`는 HugContent 순환 검사(X.6.5)용 축별 콘텐츠 크기 조상입니다.
    fn arrange(
        &mut self,
        index: usize,
        rect: LayoutRect,
        link: CycleLink,
        out: &mut Vec<PendingBox>,
    ) -> Result<LayoutRect, LayoutError> {
        let prepared = &self.nodes[index];
        let id = prepared.id;
        let padding = prepared.padding;
        let sizing = prepared.sizing;
        let size = rect.size();
        // X.6.5: Percentage 축이 자신의 측정 기여로 크기가 정해지는 조상에 비례하면 순환입니다.
        for axis in Axis::BOTH {
            if let (AxisSizing::Percentage(_), Some(ancestor)) =
                (sizing[axis.index()], link[axis.index()])
            {
                self.diagnostics.push(LayoutDiagnostic::HugContentCycle {
                    ancestor,
                    node: id,
                    axis,
                });
            }
        }
        let own = self.content_source(index, link);
        let inner_width =
            sub_floor_zero(size.width(), padding.sum(Axis::Horizontal).map_err(at(id))?)
                .map_err(at(id))?;
        let inner_height =
            sub_floor_zero(size.height(), padding.sum(Axis::Vertical).map_err(at(id))?)
                .map_err(at(id))?;
        let inner = size_of(id, inner_width, inner_height)?;
        let content = LayoutRect::from_origin_size(padding.left, padding.top, inner);
        let baseline = self.border_baseline(index, size)?;
        let slot = out.len();
        out.push(PendingBox {
            id,
            rect,
            content,
            baseline,
            overflow: LayoutRect::at_origin(size),
        });

        let placements = match &self.node(index).container {
            ContainerLayout::Free => free::place(self, index, inner)?,
            ContainerLayout::Stack(_) => stack::place(self, index, inner)?,
            ContainerLayout::Grid(_) => grid::place(self, index, inner)?,
            ContainerLayout::Overlay(_) => overlay::place(self, index, inner)?,
            ContainerLayout::Constraint => constraint::place(self, index, inner)?,
        };
        let mut overflow = LayoutRect::at_origin(size);
        for (child, child_rect) in placements {
            let child_link = self.child_link(index, child, own);
            let child_overflow = self.arrange(child, child_rect, child_link, out)?;
            let moved = child_overflow
                .translated(child_rect.x(), child_rect.y())
                .map_err(at(id))?;
            overflow = overflow.union(&moved).map_err(at(id))?;
        }
        out[slot].overflow = overflow;
        Ok(overflow)
    }
}

/// node 입력을 subpixel 값으로 변환합니다(값 범위는 이미 검증됨).
fn prepare(node: &LayoutNode) -> Result<Prepared, LayoutError> {
    let padding = prepare_padding(node.id, &node.padding)?;
    let mut sizing = [AxisSizing::Hug; 2];
    for (axis, mode) in [
        (Axis::Horizontal, node.width),
        (Axis::Vertical, node.height),
    ] {
        let property = match axis {
            Axis::Horizontal => LayoutProperty::Width,
            Axis::Vertical => LayoutProperty::Height,
        };
        sizing[axis.index()] = match mode {
            SizingMode::Fixed(length) => {
                AxisSizing::Fixed(length_unit(node.id, property, length, false)?)
            }
            SizingMode::Fill => AxisSizing::Fill,
            SizingMode::HugContent => AxisSizing::Hug,
            SizingMode::Percentage(percent) => AxisSizing::Percentage(percent.raw()),
        };
    }
    let content = match &node.content {
        LayoutContent::None => PreparedContent::None,
        LayoutContent::Intrinsic(content) => {
            let width = length_unit(node.id, LayoutProperty::Content, content.width, false)?;
            let height = length_unit(node.id, LayoutProperty::Content, content.height, false)?;
            let baseline = match content.baseline {
                Some(baseline) => Some(length_unit(
                    node.id,
                    LayoutProperty::Content,
                    baseline,
                    true,
                )?),
                None => None,
            };
            PreparedContent::Fixed([width, height], baseline)
        }
        LayoutContent::Measured(content) => PreparedContent::Measured(content.kind()),
    };
    let anchors = free::prepare_position(node)?;
    let flex = prepare_flex(node)?;
    let limits = prepare_limits(node.id, &node.limits)?;
    let ratio = prepare_ratio(node.id, node.aspect_ratio)?;
    Ok(Prepared {
        id: node.id,
        padding,
        sizing,
        content,
        anchors,
        flex,
        limits,
        ratio,
    })
}

fn prepare_padding(node: StableId, insets: &Insets) -> Result<Padding, LayoutError> {
    let (top, right, bottom, left) = insets.components();
    let convert = |value: f32| -> Result<LayoutUnit, LayoutError> {
        let length =
            stellar_core_types::value::Length::new(value).map_err(|_| LayoutError::OutOfRange {
                node,
                property: LayoutProperty::Padding,
            })?;
        length_unit(node, LayoutProperty::Padding, length, false)
    };
    Ok(Padding {
        top: convert(top)?,
        right: convert(right)?,
        bottom: convert(bottom)?,
        left: convert(left)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flex::{FlexBasis, FlexItem};
    use crate::grid::{GridLayout, TrackSize};
    use crate::stack::{StackLayout, StackWrap};
    use crate::test_support::{id, key, len, px};
    use stellar_core_types::value::Scalar;

    /// 깊이 `depth` 사슬(root = 0). container는 Hug이고 마지막 leaf는 10×10 고정입니다.
    /// `kind`: 0 = wrap Stack(자식 flex grow·shrink 1), 1 = MaxContent Grid, 2 = Free.
    fn chain(depth: usize, kind: u8) -> LayoutInput {
        let one = Scalar::new(1.0).unwrap();
        let mut nodes = Vec::new();
        for n in 0..=depth as u64 {
            let container = match kind {
                0 => ContainerLayout::Stack(StackLayout {
                    wrap: StackWrap::Wrap,
                    ..StackLayout::new(Axis::Horizontal)
                }),
                1 => ContainerLayout::Grid(GridLayout::new(vec![TrackSize::MaxContent], vec![])),
                _ => ContainerLayout::Free,
            };
            let mut node = LayoutNode::new(id(n + 1))
                .with_sizing(SizingMode::HugContent, SizingMode::HugContent)
                .with_container(container);
            if n > 0 && kind == 0 {
                node = node.with_flex(FlexItem::new(one, one, FlexBasis::Auto));
            }
            if n < depth as u64 {
                node = node.with_child(id(n + 2), key(1));
            } else {
                node = node.with_fixed_size(len(10.0), len(10.0));
            }
            nodes.push(node);
        }
        LayoutInput::new(id(1), nodes)
    }

    /// `stack` byte stack을 가진 thread에서 `f`를 실행합니다.
    fn on_thread<T: Send>(stack: usize, f: impl FnOnce() -> T + Send) -> T {
        std::thread::scope(|scope| {
            std::thread::Builder::new()
                .stack_size(stack)
                .spawn_scoped(scope, f)
                .unwrap()
                .join()
                .unwrap()
        })
    }

    #[test]
    fn max_depth_chain_does_not_overflow_default_thread_stack() {
        // 리뷰 재현: 깊이 256 Hug Stack(wrap+flex)·Grid 사슬은 debug 빌드 2 MiB thread에서 stack
        // overflow로 abort했습니다. 지금은 전용 stack thread에서 계산해 모든 빌드에서 끝납니다.
        const DEFAULT_THREAD_STACK: usize = 2 << 20;
        for kind in [0u8, 1, 2] {
            let input = chain(MAX_LAYOUT_DEPTH, kind);
            assert_eq!(input.tree().unwrap().depth, MAX_LAYOUT_DEPTH);
            let result = on_thread(DEFAULT_THREAD_STACK, || compute_layout(&input)).unwrap();
            assert_eq!(result.boxes().len(), MAX_LAYOUT_DEPTH + 1);
            // Hug 사슬은 모두 leaf 크기 10×10.
            for node in [1u64, 2, MAX_LAYOUT_DEPTH as u64 + 1] {
                let rect = result.get(id(node)).unwrap().local_rect();
                assert_eq!((rect.width(), rect.height()), (px(10), px(10)), "{kind}");
            }
            // 결과는 계산 thread와 무관합니다(같은 tree를 큰 stack에서 직접 계산한 bytes와 동일).
            let direct = on_thread(DEEP_LAYOUT_STACK_BYTES, || {
                compute_validated(&input, input.tree().unwrap(), &DefaultMeasurer)
            })
            .unwrap();
            assert_eq!(direct.encode(), result.encode());
        }
    }

    #[test]
    fn inline_depth_chain_runs_on_caller_thread_stack() {
        // INLINE_LAYOUT_DEPTH 이하는 호출 thread에서 계산합니다. 1 MiB thread에서도 끝나야 합니다.
        for kind in [0u8, 1, 2] {
            let input = chain(INLINE_LAYOUT_DEPTH, kind);
            let result = on_thread(1 << 20, || compute_layout(&input)).unwrap();
            assert_eq!(result.boxes().len(), INLINE_LAYOUT_DEPTH + 1);
        }
        // 깊이 초과는 계산 전에 검증 오류(재귀하지 않음).
        assert!(matches!(
            compute_layout(&chain(MAX_LAYOUT_DEPTH + 1, 2)),
            Err(LayoutError::DepthLimitExceeded { .. })
        ));
        assert_eq!(DEEP_LAYOUT_STACK_BYTES, 16 << 20);
    }

    // --- HugContent 순환 (X.6.5) ---

    use crate::free::{AxisAnchors, Offset};
    use crate::grid::GridItem;
    use crate::intrinsic::{MeasuredContent, ShapeIntrinsic};
    use stellar_core_types::value::Percentage;

    fn pct(value: f32) -> SizingMode {
        SizingMode::Percentage(Percentage::new(value).unwrap())
    }

    fn shape(width: f32, height: f32) -> LayoutContent {
        LayoutContent::Measured(MeasuredContent::Shape(ShapeIntrinsic {
            width: len(width),
            height: len(height),
        }))
    }

    fn cycle(ancestor: u64, node: u64, axis: Axis) -> LayoutDiagnostic {
        LayoutDiagnostic::HugContentCycle {
            ancestor: id(ancestor),
            node: id(node),
            axis,
        }
    }

    /// root(id 1, sizing·container) 아래 자식들.
    fn tree(
        root_sizing: (SizingMode, SizingMode),
        container: ContainerLayout,
        children: Vec<LayoutNode>,
    ) -> LayoutResult {
        let mut root = LayoutNode::new(id(1))
            .with_sizing(root_sizing.0, root_sizing.1)
            .with_container(container);
        for (i, child) in children.iter().enumerate() {
            root = root.with_child(child.id, key(i as u32 + 1));
        }
        let mut nodes = vec![root];
        nodes.extend(children);
        compute_layout(&LayoutInput::new(id(1), nodes)).unwrap()
    }

    #[test]
    fn parent_child_hug_cycle_fixture_is_reported() {
        let hug = SizingMode::HugContent;
        // parent Hug 너비 ↔ child 50% 너비: child는 콘텐츠 40으로 기여 → parent 40 → child 50% = 20.
        let child = LayoutNode::new(id(2))
            .with_sizing(pct(50.0), SizingMode::Fixed(len(10.0)))
            .with_content(shape(40.0, 10.0));
        let result = tree((hug, hug), ContainerLayout::Free, vec![child]);
        assert_eq!(result.diagnostics(), &[cycle(1, 2, Axis::Horizontal)][..]);
        let parent = result.get(id(1)).unwrap().local_rect();
        let child = result.get(id(2)).unwrap().local_rect();
        assert_eq!((parent.width(), child.width()), (px(40), px(20)));
        // 진단 bytes 왕복.
        let bytes = result.encode();
        assert_eq!(LayoutResult::decode(&bytes).unwrap(), result);
        assert!(cycle(1, 2, Axis::Vertical).to_string().contains("순환"));
    }

    #[test]
    fn non_cycles_are_not_reported() {
        let hug = SizingMode::HugContent;
        let fixed = SizingMode::Fixed(len(100.0));
        let leaf = |n: u64, width: SizingMode| {
            LayoutNode::new(id(n))
                .with_sizing(width, SizingMode::Fixed(len(10.0)))
                .with_content(shape(40.0, 10.0))
        };
        // definite parent의 Percentage.
        assert!(tree(
            (fixed, fixed),
            ContainerLayout::Free,
            vec![leaf(2, pct(50.0))]
        )
        .diagnostics()
        .is_empty());
        // Hug parent의 Fill은 stretch(고정점)라 순환이 아님.
        assert!(tree(
            (hug, hug),
            ContainerLayout::Free,
            vec![leaf(2, SizingMode::Fill)]
        )
        .diagnostics()
        .is_empty());
        // Free에서 끝 anchor 자식은 parent extent에 기여하지 않아 순환이 아님.
        let anchored = leaf(2, pct(50.0)).with_anchors(
            AxisAnchors {
                start: None,
                end: Some(Offset::Length(len(0.0))),
            },
            AxisAnchors::NONE,
        );
        assert!(tree((hug, hug), ContainerLayout::Free, vec![anchored])
            .diagnostics()
            .is_empty());
        // Fixed 중간 node가 사슬을 끊습니다.
        let middle = LayoutNode::new(id(2))
            .with_fixed_size(len(80.0), len(20.0))
            .with_child(id(3), key(1));
        let mut nodes = vec![
            LayoutNode::new(id(1))
                .with_sizing(hug, hug)
                .with_child(id(2), key(1)),
            middle,
            leaf(3, pct(50.0)),
        ];
        assert!(compute_layout(&LayoutInput::new(id(1), nodes.clone()))
            .unwrap()
            .diagnostics()
            .is_empty());
        // 중간 node를 Fill로 바꾸면 Hug root까지 사슬이 이어집니다(조상 = root).
        nodes[1] = nodes[1]
            .clone()
            .with_sizing(SizingMode::Fill, SizingMode::Fixed(len(20.0)));
        assert_eq!(
            compute_layout(&LayoutInput::new(id(1), nodes.clone()))
                .unwrap()
                .diagnostics(),
            &[cycle(1, 3, Axis::Horizontal)][..]
        );
        // 중간 node가 Hug면 가장 가까운 콘텐츠 크기 조상(중간 node)이 보고됩니다.
        nodes[1] = nodes[1]
            .clone()
            .with_sizing(hug, SizingMode::Fixed(len(20.0)));
        assert_eq!(
            compute_layout(&LayoutInput::new(id(1), nodes))
                .unwrap()
                .diagnostics(),
            &[cycle(2, 3, Axis::Horizontal)][..]
        );
    }

    #[test]
    fn stack_and_grid_hug_cycles() {
        let hug = SizingMode::HugContent;
        // 가로 Hug Stack: cross(세로) 50% 자식 → 세로 순환.
        let child = LayoutNode::new(id(2))
            .with_sizing(SizingMode::Fixed(len(10.0)), pct(50.0))
            .with_content(shape(10.0, 30.0));
        let result = tree(
            (hug, hug),
            ContainerLayout::Stack(StackLayout::new(Axis::Horizontal)),
            vec![child],
        );
        assert_eq!(result.diagnostics(), &[cycle(1, 2, Axis::Vertical)][..]);

        // Grid: 고정 크기 grid라도 MaxContent 열의 50% item은 track ↔ item 순환.
        let item = |track_column: u16| {
            LayoutNode::new(id(2))
                .with_grid(GridItem::at(track_column, 1, 1, 1))
                .with_sizing(pct(50.0), SizingMode::Fixed(len(5.0)))
                .with_content(shape(30.0, 5.0))
        };
        let grid = || {
            ContainerLayout::Grid(GridLayout::new(
                vec![
                    TrackSize::MaxContent,
                    TrackSize::Fixed(len(40.0)),
                    TrackSize::Fraction(Scalar::new(1.0).unwrap()),
                ],
                vec![TrackSize::Fixed(len(5.0))],
            ))
        };
        let fixed = SizingMode::Fixed(len(200.0));
        assert_eq!(
            tree((fixed, fixed), grid(), vec![item(1)]).diagnostics(),
            &[cycle(1, 2, Axis::Horizontal)][..]
        );
        // 고정 track은 순환이 아님.
        assert!(tree((fixed, fixed), grid(), vec![item(2)])
            .diagnostics()
            .is_empty());
        // fr track: grid 너비가 definite면 순환이 아니고, Hug면 grid가 조상인 순환.
        assert!(tree((fixed, fixed), grid(), vec![item(3)])
            .diagnostics()
            .is_empty());
        assert_eq!(
            tree((hug, fixed), grid(), vec![item(3)]).diagnostics(),
            &[cycle(1, 2, Axis::Horizontal)][..]
        );
    }
}
