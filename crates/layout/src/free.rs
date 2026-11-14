//! 자유 배치(Free layout) (X.2.1, X.2.2, X.2.3).
//!
//! # 규칙
//!
//! - 자식 위치는 부모 **content box**(border box 원점 + padding left·top, 크기 = content 크기) 기준
//!   anchor로 정합니다. 축마다 시작(left/top) anchor `s`와 끝(right/bottom) anchor `e`를 선택적으로
//!   가집니다. [`LayoutNode::with_position`]의 `(x, y)`는 시작 anchor입니다.
//! - 자식 크기는 자식 sizing을 그대로 씁니다. `Fill`은 anchor 사이 영역을 채웁니다.
//! - 자식 z-order는 visual order(OrderKey)이며 배치는 서로 영향을 주지 않습니다.
//!
//! # anchor·크기 해석 순서 (X.2.2)
//!
//! 한 축의 content 크기를 `C`라 할 때 다음 순서로 해석합니다.
//!
//! 1. **크기**: `Fill`이면 `C - (s 또는 0) - (e 또는 0)`(0 미만은 0). 그 밖의 mode(`Fixed`·
//!    `HugContent` 등 definite 크기)는 anchor와 무관하게 자기 크기입니다.
//! 2. **위치**: `s`가 있으면 `s`. `e`만 있으면 `C - e - 크기`. 둘 다 없으면 0.
//!
//! | 시작 | 끝 | 크기 mode | 위치 | 크기 |
//! |---|---|---|---|---|
//! | - | - | definite | 0 | 자기 크기 |
//! | s | - | definite | s | 자기 크기 |
//! | - | e | definite | C − e − 크기 | 자기 크기 |
//! | s | e | definite | **오류(과다 제약)** | — |
//! | - | - | Fill | 0 | C |
//! | s | - | Fill | s | C − s |
//! | - | e | Fill | 0 | C − e |
//! | s | e | Fill | s | C − s − e |
//!
//! 양 끝 anchor와 definite 크기가 함께 있으면 어느 쪽을 우선할지 정하지 않고
//! [`LayoutError::OverConstrained`]로 거부합니다(입력 검증 단계).
//!
//! # Hug 측정
//!
//! 부모가 `HugContent`인 축의 content extent는 끝 anchor가 **없는** 자식의 `(s 또는 0) + 크기`
//! 최댓값(0 이상)입니다. 끝 anchor 자식은 부모 끝을 기준으로 놓이므로 부모 크기에 기여하지
//! 않습니다. 측정 중 자식 `Fill`은 콘텐츠 크기로 기여합니다.
//!
//! # 백분율 (X.2.3)
//!
//! 백분율 offset([`Offset::Percentage`])과 백분율 크기(`SizingMode::Percentage`)는 **containing
//! block = 부모 content rect** 크기의 해당 축 값 기준입니다(가로 offset·너비는 content 너비, 세로는
//! content 높이). 계산은 [`LayoutUnit::checked_mul_percent`]로 정확한 유리수에서 한 번만
//! 반올림합니다. 중첩된 경우 각 단계의 content rect가 다음 단계의 기준입니다.
//!
//! 부모가 `HugContent`인 축에서는 containing block이 아직 정해지지 않았으므로, 백분율 offset 자식은
//! Hug extent에 기여하지 않고(끝 anchor와 같음), 백분율 크기 자식은 콘텐츠 크기로 기여한 뒤 확정된
//! content rect 기준으로 다시 해석됩니다.
//!
//! anchor는 Free 부모의 자식에서만 해석하며, 다른 배치 방식의 자식에서는 무시합니다.

use stellar_core_types::value::{FiniteF32, Length, Percentage};

use crate::codec::{Reader, Writer};
use crate::engine::{at, sub_floor_zero, ChildConstraint, Engine};
use crate::error::LayoutError;
use crate::geometry::{Axis, LayoutRect, LayoutSize};
use crate::input::{length_unit, LayoutNode, SizingMode};
use crate::property::LayoutProperty;
use crate::rounding::{LayoutUnit, LayoutUnitError};

/// anchor offset 값 (X.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Offset {
    /// 길이(음수 허용).
    Length(Length),
    /// containing block content 크기의 백분율(음수 허용, `50` = 50%, X.2.3).
    Percentage(Percentage),
}

/// 한 축의 시작·끝 anchor (X.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct AxisAnchors {
    /// 시작(left/top) anchor: content box 시작에서 자식 시작까지.
    pub start: Option<Offset>,
    /// 끝(right/bottom) anchor: 자식 끝에서 content box 끝까지.
    pub end: Option<Offset>,
}

impl AxisAnchors {
    /// anchor 없음.
    pub const NONE: AxisAnchors = AxisAnchors {
        start: None,
        end: None,
    };

    /// 시작 anchor만 있는 축.
    pub const fn start(offset: Length) -> AxisAnchors {
        AxisAnchors {
            start: Some(Offset::Length(offset)),
            end: None,
        }
    }

    /// 끝 anchor만 있는 축.
    pub const fn end(offset: Length) -> AxisAnchors {
        AxisAnchors {
            start: None,
            end: Some(Offset::Length(offset)),
        }
    }

    /// 양 끝 anchor가 있는 축.
    pub const fn both(start: Length, end: Length) -> AxisAnchors {
        AxisAnchors {
            start: Some(Offset::Length(start)),
            end: Some(Offset::Length(end)),
        }
    }
}

/// Free 부모 안에서 자식의 위치 anchor (X.2.1, X.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FreePosition {
    /// 가로(left/right).
    pub horizontal: AxisAnchors,
    /// 세로(top/bottom).
    pub vertical: AxisAnchors,
}

impl FreePosition {
    /// 시작 anchor `(x, y)` 위치.
    pub const fn at(x: Length, y: Length) -> FreePosition {
        FreePosition {
            horizontal: AxisAnchors::start(x),
            vertical: AxisAnchors::start(y),
        }
    }

    /// 축 anchor.
    pub const fn axis(&self, axis: Axis) -> &AxisAnchors {
        match axis {
            Axis::Horizontal => &self.horizontal,
            Axis::Vertical => &self.vertical,
        }
    }
}

/// 변환된 offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreparedOffset {
    /// 길이.
    Length(LayoutUnit),
    /// 백분율.
    Percentage(FiniteF32),
}

impl PreparedOffset {
    /// content 크기 `extent` 기준 offset 값.
    fn resolve(self, extent: LayoutUnit) -> Result<LayoutUnit, LayoutUnitError> {
        match self {
            PreparedOffset::Length(value) => Ok(value),
            PreparedOffset::Percentage(percent) => extent.checked_mul_percent(percent),
        }
    }

    /// containing block 크기에 의존하는지 여부.
    const fn is_relative(self) -> bool {
        matches!(self, PreparedOffset::Percentage(_))
    }
}

/// 변환된 한 축 anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct PreparedAnchors {
    pub(crate) start: Option<PreparedOffset>,
    pub(crate) end: Option<PreparedOffset>,
}

impl PreparedAnchors {
    /// 이 축에서 부모 Hug extent(측정)에 기여하는지 여부. 끝 anchor·백분율 시작 anchor 자식은 부모
    /// 크기 기준이라 기여하지 않습니다.
    pub(crate) fn contributes_to_extent(&self) -> bool {
        self.end.is_none() && !self.start.is_some_and(PreparedOffset::is_relative)
    }
}

/// 없는 anchor를 0으로 보고 offset 값을 해석합니다.
fn offset_or_zero(
    offset: Option<PreparedOffset>,
    extent: LayoutUnit,
) -> Result<LayoutUnit, LayoutUnitError> {
    match offset {
        Some(offset) => offset.resolve(extent),
        None => Ok(LayoutUnit::ZERO),
    }
}

/// position을 subpixel로 변환합니다.
pub(crate) fn prepare_position(node: &LayoutNode) -> Result<[PreparedAnchors; 2], LayoutError> {
    let convert = |offset: Option<Offset>| -> Result<Option<PreparedOffset>, LayoutError> {
        match offset {
            None => Ok(None),
            Some(Offset::Length(length)) => Ok(Some(PreparedOffset::Length(length_unit(
                node.id,
                LayoutProperty::Position,
                length,
                true,
            )?))),
            Some(Offset::Percentage(percent)) => {
                Ok(Some(PreparedOffset::Percentage(percent.raw())))
            }
        }
    };
    let mut out = [PreparedAnchors::default(); 2];
    for axis in Axis::BOTH {
        let anchors = node.position.axis(axis);
        out[axis.index()] = PreparedAnchors {
            start: convert(anchors.start)?,
            end: convert(anchors.end)?,
        };
    }
    Ok(out)
}

/// position 값 범위를 검증합니다(모든 node).
pub(crate) fn validate_position_values(node: &LayoutNode) -> Result<(), LayoutError> {
    prepare_position(node).map(|_| ())
}

/// Free 부모의 자식 anchor 조합을 검증합니다(양 끝 anchor + definite 크기는 과다 제약).
pub(crate) fn validate_anchor_combination(node: &LayoutNode) -> Result<(), LayoutError> {
    for (axis, mode) in [
        (Axis::Horizontal, node.width),
        (Axis::Vertical, node.height),
    ] {
        let anchors = node.position.axis(axis);
        if anchors.start.is_some() && anchors.end.is_some() && mode != SizingMode::Fill {
            return Err(LayoutError::OverConstrained {
                node: node.id,
                axis,
            });
        }
    }
    Ok(())
}

const OFFSET_LENGTH: u8 = 0;
const OFFSET_PERCENTAGE: u8 = 1;

fn encode_offset(w: &mut Writer, offset: Option<Offset>) {
    match offset {
        None => w.bool(false),
        Some(Offset::Length(length)) => {
            w.bool(true);
            w.u8(OFFSET_LENGTH);
            w.length(length);
        }
        Some(Offset::Percentage(percent)) => {
            w.bool(true);
            w.u8(OFFSET_PERCENTAGE);
            w.finite(percent.raw());
        }
    }
}

fn decode_offset(r: &mut Reader<'_>) -> Result<Option<Offset>, LayoutError> {
    if !r.bool("anchor flag")? {
        return Ok(None);
    }
    match r.u8()? {
        OFFSET_LENGTH => Ok(Some(Offset::Length(r.length("anchor offset")?))),
        OFFSET_PERCENTAGE => Ok(Some(Offset::Percentage(Percentage::from_raw(
            r.finite("anchor offset")?,
        )))),
        _ => Err(LayoutError::Malformed {
            reason: "알 수 없는 offset tag",
        }),
    }
}

/// position canonical bytes(가로 시작·끝, 세로 시작·끝).
pub(crate) fn encode_position(w: &mut Writer, position: &FreePosition) {
    for axis in Axis::BOTH {
        let anchors = position.axis(axis);
        encode_offset(w, anchors.start);
        encode_offset(w, anchors.end);
    }
}

/// position canonical bytes를 읽습니다.
pub(crate) fn decode_position(r: &mut Reader<'_>) -> Result<FreePosition, LayoutError> {
    let horizontal = AxisAnchors {
        start: decode_offset(r)?,
        end: decode_offset(r)?,
    };
    let vertical = AxisAnchors {
        start: decode_offset(r)?,
        end: decode_offset(r)?,
    };
    Ok(FreePosition {
        horizontal,
        vertical,
    })
}

/// 자식 하나의 제약(부모 content 크기 기준). Fill 영역은 anchor 사이입니다.
fn child_constraint(
    inner: [Option<LayoutUnit>; 2],
    anchors: &[PreparedAnchors; 2],
) -> Result<ChildConstraint, LayoutUnitError> {
    let mut fill = [None, None];
    for axis in Axis::BOTH {
        let i = axis.index();
        fill[i] = match inner[i] {
            Some(extent) => {
                let start = offset_or_zero(anchors[i].start, extent)?;
                let end = offset_or_zero(anchors[i].end, extent)?;
                Some(sub_floor_zero(extent.checked_sub(start)?, end)?)
            }
            None => None,
        };
    }
    Ok(ChildConstraint {
        cb: inner,
        fill,
        forced: [None, None],
    })
}

/// 한 축의 위치(content box 기준). `extent`는 content 크기입니다.
fn axis_position(
    anchors: PreparedAnchors,
    extent: LayoutUnit,
    size: LayoutUnit,
) -> Result<LayoutUnit, LayoutUnitError> {
    match (anchors.start, anchors.end) {
        (Some(start), _) => start.resolve(extent),
        (None, Some(end)) => extent.checked_sub(end.resolve(extent)?)?.checked_sub(size),
        (None, None) => Ok(LayoutUnit::ZERO),
    }
}

/// Free container의 content extent(Hug 측정).
pub(crate) fn measure(
    engine: &mut Engine<'_>,
    index: usize,
    inner: [Option<LayoutUnit>; 2],
) -> Result<[LayoutUnit; 2], LayoutError> {
    let id = engine.nodes[index].id;
    let mut extent = [LayoutUnit::ZERO; 2];
    for child in engine.children(index) {
        let anchors = engine.nodes[child].anchors;
        let constraint = child_constraint(inner, &anchors).map_err(at(id))?;
        let size = engine.resolve_size(child, constraint)?;
        for axis in Axis::BOTH {
            let i = axis.index();
            if !anchors[i].contributes_to_extent() {
                // 끝 anchor·백분율 시작 anchor 자식은 부모 크기 기준이라 extent에 기여하지 않습니다.
                continue;
            }
            let start = offset_or_zero(anchors[i].start, LayoutUnit::ZERO).map_err(at(id))?;
            let end = start.checked_add(size.get(axis)).map_err(at(id))?;
            extent[i] = extent[i].max(end);
        }
    }
    Ok(extent)
}

/// Free container의 자식 배치. 각 자식의 rect(부모 local 좌표)를 visual order로 반환합니다.
pub(crate) fn place(
    engine: &mut Engine<'_>,
    index: usize,
    inner: LayoutSize,
) -> Result<Vec<(usize, LayoutRect)>, LayoutError> {
    let mut placements = Vec::new();
    for child in engine.children(index) {
        placements.push((child, child_rect(engine, index, inner, child)?));
    }
    Ok(placements)
}

/// Free container `index`(content 크기 `inner`) 안 자식 하나의 rect(부모 local 좌표).
///
/// 자식 위치는 자신의 anchor·크기와 부모 content box에만 의존합니다(형제와 무관). 증분 레이아웃(X.7.3)은
/// 바뀐 자식 하나만 이 함수로 다시 놓습니다.
pub(crate) fn child_rect(
    engine: &mut Engine<'_>,
    index: usize,
    inner: LayoutSize,
    child: usize,
) -> Result<LayoutRect, LayoutError> {
    let id = engine.nodes[index].id;
    let padding = engine.nodes[index].padding;
    let inner_axes = [Some(inner.width()), Some(inner.height())];
    let anchors = engine.nodes[child].anchors;
    let constraint = child_constraint(inner_axes, &anchors).map_err(at(id))?;
    let size = engine.resolve_size(child, constraint)?;
    let x = axis_position(anchors[0], inner.width(), size.width()).map_err(at(id))?;
    let y = axis_position(anchors[1], inner.height(), size.height()).map_err(at(id))?;
    let x = padding.left.checked_add(x).map_err(at(id))?;
    let y = padding.top.checked_add(y).map_err(at(id))?;
    Ok(LayoutRect::from_origin_size(x, y, size))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::compute_layout;
    use crate::input::{IntrinsicContent, LayoutContent, LayoutInput, LayoutNode, SizingMode};
    use crate::test_support::{id, key, len, px};
    use stellar_core_types::value::Insets;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> LayoutRect {
        LayoutRect::new(px(x), px(y), px(w), px(h)).unwrap()
    }

    fn fixed(w: f32, h: f32) -> (SizingMode, SizingMode) {
        (SizingMode::Fixed(len(w)), SizingMode::Fixed(len(h)))
    }

    #[test]
    fn basic_position_fixture_applies_padding_and_content_origin() {
        // root 200×100, padding (top 10, right 4, bottom 6, left 20).
        // a: (5, 7) 30×20, b: (-10, 50) 40×80(넘침), c: (0, 0) 0×0.
        let root = LayoutNode::new(id(1))
            .with_fixed_size(len(200.0), len(100.0))
            .with_padding(Insets::new(10.0, 4.0, 6.0, 20.0).unwrap())
            .with_child(id(3), key(2))
            .with_child(id(2), key(1))
            .with_child(id(4), key(3));
        let a = LayoutNode::new(id(2))
            .with_fixed_size(len(30.0), len(20.0))
            .with_position(len(5.0), len(7.0));
        let b = LayoutNode::new(id(3))
            .with_fixed_size(len(40.0), len(80.0))
            .with_position(len(-10.0), len(50.0));
        let c = LayoutNode::new(id(4));
        let result = compute_layout(&LayoutInput::new(id(1), vec![root, a, b, c])).unwrap();

        let ids: Vec<_> = result.boxes().iter().map(|b| b.id()).collect();
        assert_eq!(
            ids,
            vec![id(1), id(2), id(3), id(4)],
            "pre-order, visual order"
        );
        let root_box = result.get(id(1)).unwrap();
        assert_eq!(root_box.local_rect(), rect(0, 0, 200, 100));
        assert_eq!(root_box.content_rect(), rect(20, 10, 176, 84));
        assert_eq!(
            result.get(id(2)).unwrap().local_rect(),
            rect(25, 17, 30, 20)
        );
        assert_eq!(
            result.get(id(3)).unwrap().local_rect(),
            rect(10, 60, 40, 80)
        );
        assert_eq!(result.get(id(4)).unwrap().local_rect(), rect(20, 10, 0, 0));
        // overflow: b가 아래로 넘침(60 + 80 = 140).
        assert_eq!(root_box.overflow_bounds(), rect(0, 0, 200, 140));
        assert_eq!(
            result.get(id(2)).unwrap().overflow_bounds(),
            rect(0, 0, 30, 20)
        );
    }

    #[test]
    fn hug_fill_and_intrinsic_in_free_layout() {
        // Hug root(padding 8): 자식 extent = max(position + size).
        let root = LayoutNode::new(id(1))
            .with_sizing(SizingMode::HugContent, SizingMode::HugContent)
            .with_padding(Insets::new(8.0, 8.0, 8.0, 8.0).unwrap())
            .with_child(id(2), key(1))
            .with_child(id(3), key(2));
        // 고정 자식 (10, 4) 50×30.
        let fixed_child = LayoutNode::new(id(2))
            .with_sizing(fixed(50.0, 30.0).0, fixed(50.0, 30.0).1)
            .with_position(len(10.0), len(4.0));
        // intrinsic(text 70×12, baseline 9) Hug 자식 (0, 40), padding 2.
        let text = LayoutNode::new(id(3))
            .with_sizing(SizingMode::HugContent, SizingMode::HugContent)
            .with_padding(Insets::new(2.0, 2.0, 2.0, 2.0).unwrap())
            .with_position(len(0.0), len(40.0))
            .with_content(LayoutContent::Intrinsic(IntrinsicContent {
                width: len(70.0),
                height: len(12.0),
                baseline: Some(len(9.0)),
            }));
        let result =
            compute_layout(&LayoutInput::new(id(1), vec![root, fixed_child, text])).unwrap();
        // content extent: 가로 max(60, 74) = 74, 세로 max(34, 56) = 56 → +16 padding.
        assert_eq!(result.get(id(1)).unwrap().local_rect(), rect(0, 0, 90, 72));
        assert_eq!(result.get(id(3)).unwrap().local_rect(), rect(8, 48, 74, 16));
        assert_eq!(result.get(id(3)).unwrap().baseline(), Some(px(11)));
        assert_eq!(result.get(id(1)).unwrap().baseline(), None);

        // Fill: position부터 content box 끝까지.
        let parent = LayoutNode::new(id(1))
            .with_fixed_size(len(100.0), len(60.0))
            .with_padding(Insets::new(5.0, 5.0, 5.0, 5.0).unwrap())
            .with_child(id(2), key(1))
            .with_child(id(3), key(2));
        let fill = LayoutNode::new(id(2))
            .with_sizing(SizingMode::Fill, SizingMode::Fill)
            .with_position(len(10.0), len(20.0));
        let past_end = LayoutNode::new(id(3))
            .with_sizing(SizingMode::Fill, SizingMode::Fixed(len(5.0)))
            .with_position(len(120.0), len(0.0));
        let result =
            compute_layout(&LayoutInput::new(id(1), vec![parent, fill, past_end])).unwrap();
        assert_eq!(
            result.get(id(2)).unwrap().local_rect(),
            rect(15, 25, 80, 30)
        );
        // position이 content box를 넘으면 Fill 크기는 0.
        assert_eq!(result.get(id(3)).unwrap().local_rect(), rect(125, 5, 0, 5));
    }

    #[test]
    fn hug_parent_measures_fill_child_by_content() {
        // Hug 부모 안의 Fill 자식은 측정 시 콘텐츠(손자 extent)로 기여하고, 확정 크기를 채웁니다.
        let root = LayoutNode::new(id(1))
            .with_sizing(SizingMode::HugContent, SizingMode::HugContent)
            .with_child(id(2), key(1))
            .with_child(id(3), key(2));
        let fill = LayoutNode::new(id(2))
            .with_sizing(SizingMode::Fill, SizingMode::Fill)
            .with_child(id(4), key(1));
        let grandchild = LayoutNode::new(id(4)).with_fixed_size(len(20.0), len(10.0));
        let wide = LayoutNode::new(id(3))
            .with_fixed_size(len(60.0), len(5.0))
            .with_position(len(0.0), len(30.0));
        let result =
            compute_layout(&LayoutInput::new(id(1), vec![root, fill, grandchild, wide])).unwrap();
        assert_eq!(result.get(id(1)).unwrap().local_rect(), rect(0, 0, 60, 35));
        assert_eq!(result.get(id(2)).unwrap().local_rect(), rect(0, 0, 60, 35));
        assert_eq!(result.get(id(4)).unwrap().local_rect(), rect(0, 0, 20, 10));
    }

    #[test]
    fn layout_is_deterministic_and_independent_of_insertion_order() {
        let build = |reverse: bool| {
            let mut root = LayoutNode::new(id(1)).with_fixed_size(len(50.0), len(50.0));
            let mut order = vec![(id(2), key(1)), (id(3), key(2)), (id(4), key(3))];
            if reverse {
                order.reverse();
            }
            for (child, k) in order {
                root = root.with_child(child, k);
            }
            let mut nodes = vec![root];
            for n in 2..=4u64 {
                nodes.push(
                    LayoutNode::new(id(n))
                        .with_fixed_size(len(n as f32), len(1.0 / 3.0))
                        .with_position(len(n as f32 * 1.5), len(0.1)),
                );
            }
            if reverse {
                nodes.reverse();
            }
            LayoutInput::new(id(1), nodes)
        };
        let forward = compute_layout(&build(false)).unwrap();
        let backward = compute_layout(&build(true)).unwrap();
        assert_eq!(forward.encode(), backward.encode());
        // 1/3 px 높이는 subpixel 21, 0.1 px y는 6.
        let b = forward.get(id(2)).unwrap().local_rect();
        assert_eq!((b.height().raw(), b.y().raw()), (21, 6));
    }

    /// 한 축 anchor 조합 하나: 자식 sizing, anchor, content 크기 `C` → 기대 (위치, 크기) 또는 오류.
    struct AnchorCase {
        name: &'static str,
        mode: SizingMode,
        anchors: AxisAnchors,
        expected: Option<fn(i32) -> (i32, i32)>,
    }

    fn intrinsic_30() -> LayoutContent {
        LayoutContent::Intrinsic(IntrinsicContent {
            width: len(30.0),
            height: len(30.0),
            baseline: None,
        })
    }

    #[test]
    fn anchor_combination_matrix_and_errors() {
        // 부모 100×80, padding 10 → content 80×60(원점 10, 10). 시작 s = 5, 끝 e = 7.
        // Fixed 크기 20, Hug 크기 30(intrinsic).
        let (s, e) = (len(5.0), len(7.0));
        let fixed20 = SizingMode::Fixed(len(20.0));
        let hug = SizingMode::HugContent;
        let fill = SizingMode::Fill;
        fn case(
            name: &'static str,
            mode: SizingMode,
            anchors: AxisAnchors,
            expected: Option<fn(i32) -> (i32, i32)>,
        ) -> AnchorCase {
            AnchorCase {
                name,
                mode,
                anchors,
                expected,
            }
        }
        let cases = [
            case("none/fixed", fixed20, AxisAnchors::NONE, Some(|_| (0, 20))),
            case(
                "start/fixed",
                fixed20,
                AxisAnchors::start(s),
                Some(|_| (5, 20)),
            ),
            case(
                "end/fixed",
                fixed20,
                AxisAnchors::end(e),
                Some(|c| (c - 7 - 20, 20)),
            ),
            case("both/fixed", fixed20, AxisAnchors::both(s, e), None),
            case("none/hug", hug, AxisAnchors::NONE, Some(|_| (0, 30))),
            case("start/hug", hug, AxisAnchors::start(s), Some(|_| (5, 30))),
            case(
                "end/hug",
                hug,
                AxisAnchors::end(e),
                Some(|c| (c - 7 - 30, 30)),
            ),
            case("both/hug", hug, AxisAnchors::both(s, e), None),
            case("none/fill", fill, AxisAnchors::NONE, Some(|c| (0, c))),
            case(
                "start/fill",
                fill,
                AxisAnchors::start(s),
                Some(|c| (5, c - 5)),
            ),
            case("end/fill", fill, AxisAnchors::end(e), Some(|c| (0, c - 7))),
            case(
                "both/fill",
                fill,
                AxisAnchors::both(s, e),
                Some(|c| (5, c - 12)),
            ),
        ];
        for axis in Axis::BOTH {
            let extent = match axis {
                Axis::Horizontal => 80,
                Axis::Vertical => 60,
            };
            for case in &cases {
                let other = SizingMode::Fixed(len(10.0));
                let (mode_w, mode_h, anchors_h, anchors_v) = match axis {
                    Axis::Horizontal => (case.mode, other, case.anchors, AxisAnchors::NONE),
                    Axis::Vertical => (other, case.mode, AxisAnchors::NONE, case.anchors),
                };
                let parent = LayoutNode::new(id(1))
                    .with_fixed_size(len(100.0), len(80.0))
                    .with_padding(Insets::new(10.0, 10.0, 10.0, 10.0).unwrap())
                    .with_child(id(2), key(1));
                let child = LayoutNode::new(id(2))
                    .with_sizing(mode_w, mode_h)
                    .with_anchors(anchors_h, anchors_v)
                    .with_content(intrinsic_30());
                let input = LayoutInput::new(id(1), vec![parent, child]);
                let result = compute_layout(&input);
                match case.expected {
                    None => {
                        let expected = Err(LayoutError::OverConstrained { node: id(2), axis });
                        assert_eq!(result, expected, "{} {axis}", case.name);
                        assert_eq!(
                            input.validate(),
                            Err(LayoutError::OverConstrained { node: id(2), axis })
                        );
                    }
                    Some(expected) => {
                        let (position, size) = expected(extent);
                        let rect = result.unwrap().get(id(2)).unwrap().local_rect();
                        assert_eq!(
                            (rect.start(axis), rect.size().get(axis)),
                            (px(10 + position), px(size)),
                            "{} {axis}",
                            case.name
                        );
                    }
                }
            }
        }
        assert_eq!(cases.len(), 12);
    }

    #[test]
    fn fill_between_crossing_anchors_is_zero_and_end_anchors_skip_hug() {
        // 시작 60 + 끝 30 > content 80 → Fill 크기 0(오류 아님).
        let parent = LayoutNode::new(id(1))
            .with_fixed_size(len(80.0), len(10.0))
            .with_child(id(2), key(1));
        let child = LayoutNode::new(id(2))
            .with_sizing(SizingMode::Fill, SizingMode::Fixed(len(4.0)))
            .with_anchors(AxisAnchors::both(len(60.0), len(30.0)), AxisAnchors::NONE);
        let result = compute_layout(&LayoutInput::new(id(1), vec![parent, child])).unwrap();
        assert_eq!(result.get(id(2)).unwrap().local_rect(), rect(60, 0, 0, 4));

        // Hug 부모: 시작 anchor 자식만 extent에 기여, 끝 anchor 자식은 부모 끝 기준으로 넘침.
        let hug = LayoutNode::new(id(1))
            .with_sizing(SizingMode::HugContent, SizingMode::HugContent)
            .with_child(id(2), key(1))
            .with_child(id(3), key(2));
        let start_child = LayoutNode::new(id(2))
            .with_fixed_size(len(20.0), len(10.0))
            .with_anchors(AxisAnchors::start(len(10.0)), AxisAnchors::NONE);
        let end_child = LayoutNode::new(id(3))
            .with_fixed_size(len(50.0), len(40.0))
            .with_anchors(AxisAnchors::end(len(5.0)), AxisAnchors::end(len(0.0)));
        let result =
            compute_layout(&LayoutInput::new(id(1), vec![hug, start_child, end_child])).unwrap();
        let root = result.get(id(1)).unwrap();
        assert_eq!(root.local_rect(), rect(0, 0, 30, 10));
        assert_eq!(
            result.get(id(3)).unwrap().local_rect(),
            rect(-25, -30, 50, 40)
        );
        assert_eq!(root.overflow_bounds(), rect(-25, -30, 55, 40));
    }

    #[test]
    fn anchors_round_trip_and_validate_ranges() {
        let parent = LayoutNode::new(id(1))
            .with_fixed_size(len(10.0), len(10.0))
            .with_child(id(2), key(1));
        let child = LayoutNode::new(id(2))
            .with_sizing(SizingMode::Fill, SizingMode::Fixed(len(1.0)))
            .with_anchors(
                AxisAnchors::both(len(-1.5), len(2.25)),
                AxisAnchors::end(len(3.0)),
            );
        let input = LayoutInput::new(id(1), vec![parent, child]);
        let decoded = LayoutInput::decode(&input.encode()).unwrap();
        assert_eq!(decoded, input);
        assert_eq!(
            decoded.nodes[1].position.horizontal.start,
            Some(Offset::Length(len(-1.5)))
        );

        let out_of_range = LayoutInput::new(
            id(1),
            vec![LayoutNode::new(id(1)).with_position(len(4.0e7), len(0.0))],
        );
        assert_eq!(
            out_of_range.validate(),
            Err(LayoutError::OutOfRange {
                node: id(1),
                property: LayoutProperty::Position
            })
        );
    }

    fn pct(value: f32) -> Percentage {
        Percentage::new(value).unwrap()
    }

    fn pct_anchor(start: Option<f32>, end: Option<f32>) -> AxisAnchors {
        AxisAnchors {
            start: start.map(|v| Offset::Percentage(pct(v))),
            end: end.map(|v| Offset::Percentage(pct(v))),
        }
    }

    #[test]
    fn nested_percentage_fixture_uses_containing_block_content_rect() {
        // root 400×200, padding 20 → content 360×160.
        let root = LayoutNode::new(id(1))
            .with_fixed_size(len(400.0), len(200.0))
            .with_padding(Insets::new(20.0, 20.0, 20.0, 20.0).unwrap())
            .with_child(id(2), key(1))
            .with_child(id(5), key(2));
        // A: 50%×50%, 시작 (10%, 25%) → 180×80 at (36, 40); padding 10 → content 160×60.
        let a = LayoutNode::new(id(2))
            .with_sizing(
                SizingMode::Percentage(pct(50.0)),
                SizingMode::Percentage(pct(50.0)),
            )
            .with_anchors(pct_anchor(Some(10.0), None), pct_anchor(Some(25.0), None))
            .with_padding(Insets::new(10.0, 10.0, 10.0, 10.0).unwrap())
            .with_child(id(3), key(1));
        // B(A 안): 25%×100%, 시작 (50%, 0) → 40×60 at (80, 0).
        let b = LayoutNode::new(id(3))
            .with_sizing(
                SizingMode::Percentage(pct(25.0)),
                SizingMode::Percentage(pct(100.0)),
            )
            .with_anchors(pct_anchor(Some(50.0), None), AxisAnchors::start(len(0.0)))
            .with_child(id(4), key(1));
        // C(B 안): 너비 33.333332%(f32 100/3) of 40 px, 끝 anchor 10%, 높이 Fill.
        let c = LayoutNode::new(id(4))
            .with_sizing(SizingMode::Percentage(pct(100.0 / 3.0)), SizingMode::Fill)
            .with_anchors(pct_anchor(None, Some(10.0)), AxisAnchors::NONE);
        // D(root 안): 시작 (-5%, 100%), 너비 100%, 높이 10.
        let d = LayoutNode::new(id(5))
            .with_sizing(
                SizingMode::Percentage(pct(100.0)),
                SizingMode::Fixed(len(10.0)),
            )
            .with_anchors(pct_anchor(Some(-5.0), None), pct_anchor(Some(100.0), None));
        let input = LayoutInput::new(id(1), vec![root, a, b, c, d]);
        let result = compute_layout(&input).unwrap();
        assert_eq!(
            result.get(id(2)).unwrap().local_rect(),
            rect(56, 60, 180, 80)
        );
        assert_eq!(
            result.get(id(2)).unwrap().content_rect(),
            rect(10, 10, 160, 60)
        );
        assert_eq!(
            result.get(id(3)).unwrap().local_rect(),
            rect(90, 10, 40, 60)
        );
        // C: 너비 2560 × 33.333332% = 853.33 → 853, x = 2560 − 256 − 853 = 1451 subpixel.
        let c_rect = result.get(id(4)).unwrap().local_rect();
        assert_eq!(
            (
                c_rect.x().raw(),
                c_rect.y().raw(),
                c_rect.width().raw(),
                c_rect.height().raw()
            ),
            (1451, 0, 853, 60 * 64)
        );
        // D: x = 20 + (−18), y = 20 + 160.
        assert_eq!(
            result.get(id(5)).unwrap().local_rect(),
            rect(2, 180, 360, 10)
        );
        // 입력 round-trip(백분율 offset·크기 포함).
        assert_eq!(LayoutInput::decode(&input.encode()).unwrap(), input);
    }

    #[test]
    fn percentage_children_of_hug_parent_resolve_after_parent_size() {
        // Hug root: 고정 자식 100×20이 크기를 정하고, 백분율 자식은 측정에서 콘텐츠(30×10)로
        // 기여(가로)하거나 백분율 시작 anchor라 기여하지 않습니다(세로).
        let root = LayoutNode::new(id(1))
            .with_sizing(SizingMode::HugContent, SizingMode::HugContent)
            .with_child(id(2), key(1))
            .with_child(id(3), key(2));
        let fixed = LayoutNode::new(id(2)).with_fixed_size(len(100.0), len(20.0));
        let relative = LayoutNode::new(id(3))
            .with_sizing(
                SizingMode::Percentage(pct(50.0)),
                SizingMode::Percentage(pct(50.0)),
            )
            .with_anchors(AxisAnchors::start(len(0.0)), pct_anchor(Some(50.0), None))
            .with_child(id(4), key(1));
        let grandchild = LayoutNode::new(id(4)).with_fixed_size(len(30.0), len(10.0));
        let result = compute_layout(&LayoutInput::new(
            id(1),
            vec![root, fixed, relative, grandchild],
        ))
        .unwrap();
        assert_eq!(result.get(id(1)).unwrap().local_rect(), rect(0, 0, 100, 20));
        assert_eq!(result.get(id(3)).unwrap().local_rect(), rect(0, 10, 50, 10));
        assert_eq!(result.get(id(4)).unwrap().local_rect(), rect(0, 0, 30, 10));
    }
}
