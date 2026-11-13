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

use std::collections::HashMap;

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{FiniteF32, Insets};

use crate::error::LayoutError;
use crate::flex::{prepare_flex, PreparedFlex};
use crate::free::{self, PreparedAnchors};
use crate::geometry::{Axis, LayoutRect, LayoutSize};
use crate::grid;
use crate::input::{
    length_unit, ContainerLayout, LayoutContent, LayoutInput, LayoutNode, SizingMode, TreeIndex,
};
use crate::output::{LayoutBox, LayoutResult};
use crate::overlay;
use crate::property::LayoutProperty;
use crate::rounding::{LayoutUnit, LayoutUnitError};
use crate::stack;

/// 입력 전체를 레이아웃합니다.
pub fn compute_layout(input: &LayoutInput) -> Result<LayoutResult, LayoutError> {
    let tree = input.tree()?;
    let mut engine = Engine::new(input, tree)?;
    let root = engine.tree.root;
    let size = engine.resolve_size(root, ChildConstraint::UNCONSTRAINED)?;
    let mut pending = Vec::with_capacity(input.nodes.len());
    engine.arrange(root, LayoutRect::at_origin(size), &mut pending)?;
    let boxes = pending
        .into_iter()
        .map(|p| LayoutBox::new(p.id, p.rect, p.content, p.baseline, p.overflow))
        .collect::<Result<Vec<_>, _>>()?;
    LayoutResult::new(boxes)
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

/// 변환된 node 입력.
#[derive(Debug, Clone)]
pub(crate) struct Prepared {
    pub(crate) id: StableId,
    pub(crate) padding: Padding,
    pub(crate) sizing: [AxisSizing; 2],
    /// intrinsic 콘텐츠 (너비, 높이, baseline).
    pub(crate) intrinsic: Option<([LayoutUnit; 2], Option<LayoutUnit>)>,
    /// Free 배치 anchor (가로, 세로).
    pub(crate) anchors: [PreparedAnchors; 2],
    /// Stack 자식 grow·shrink·basis.
    pub(crate) flex: PreparedFlex,
}

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
    fn new(input: &'a LayoutInput, tree: TreeIndex) -> Result<Engine<'a>, LayoutError> {
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
        })
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
        let mut definite = [None, None];
        for axis in Axis::BOTH {
            let i = axis.index();
            if let Some(forced) = constraint.forced[i] {
                definite[i] = Some(forced);
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
                    None => content[i]
                        .checked_add(padding.sum(axis).map_err(at(id))?)
                        .map_err(at(id))?,
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
        if let Some((size, _)) = self.nodes[index].intrinsic {
            return Ok(size);
        }
        match &self.node(index).container {
            ContainerLayout::Free => free::measure(self, index, inner),
            ContainerLayout::Stack(_) => stack::measure(self, index, inner),
            ContainerLayout::Grid(_) => grid::measure(self, index, inner),
            ContainerLayout::Overlay(_) => overlay::measure(self, index, inner),
        }
    }

    /// node를 `rect`(부모 local 좌표)에 놓고 자손을 배치합니다. 자신의 overflow bounds를 반환합니다.
    fn arrange(
        &mut self,
        index: usize,
        rect: LayoutRect,
        out: &mut Vec<PendingBox>,
    ) -> Result<LayoutRect, LayoutError> {
        let prepared = &self.nodes[index];
        let id = prepared.id;
        let padding = prepared.padding;
        let size = rect.size();
        let inner_width =
            sub_floor_zero(size.width(), padding.sum(Axis::Horizontal).map_err(at(id))?)
                .map_err(at(id))?;
        let inner_height =
            sub_floor_zero(size.height(), padding.sum(Axis::Vertical).map_err(at(id))?)
                .map_err(at(id))?;
        let inner = size_of(id, inner_width, inner_height)?;
        let content = LayoutRect::from_origin_size(padding.left, padding.top, inner);
        let baseline = match prepared.intrinsic {
            Some((_, Some(baseline))) => Some(padding.top.checked_add(baseline).map_err(at(id))?),
            _ => None,
        };
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
        };
        let mut overflow = LayoutRect::at_origin(size);
        for (child, child_rect) in placements {
            let child_overflow = self.arrange(child, child_rect, out)?;
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
    let intrinsic = match node.content {
        LayoutContent::None => None,
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
            Some(([width, height], baseline))
        }
    };
    let anchors = free::prepare_position(node)?;
    let flex = prepare_flex(node)?;
    Ok(Prepared {
        id: node.id,
        padding,
        sizing,
        intrinsic,
        anchors,
        flex,
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
