//! 자유 배치(Free layout) (X.2.1).
//!
//! # 규칙
//!
//! - 자식의 local position `(x, y)`는 부모 **content box 원점**(border box 원점 + padding left·top)
//!   기준입니다. 자식 rect의 origin은 `(padding.left + x, padding.top + y)`입니다.
//! - 자식 크기는 자식 sizing을 그대로 씁니다. `Fixed`는 그 값, `HugContent`는 자식 콘텐츠 크기,
//!   `Fill`은 position부터 부모 content box 끝까지(`content - position`, 0 미만은 0)입니다.
//! - 부모가 `HugContent`인 축의 content extent는 자식 `position + size`의 최댓값(0 이상)입니다.
//!   이 측정에서 자식 `Fill`은 콘텐츠 크기로 기여합니다.
//! - 자식 z-order는 visual order(OrderKey)이며 배치는 서로 영향을 주지 않습니다.

use stellar_core_types::value::{FiniteF32, Length};

use crate::engine::{at, sub_floor_zero, ChildConstraint, Engine};
use crate::error::LayoutError;
use crate::geometry::{Axis, LayoutRect, LayoutSize};
use crate::rounding::LayoutUnit;

/// Free 부모 안에서 자식의 local position (X.2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FreePosition {
    /// 부모 content box 왼쪽에서의 x.
    pub x: Length,
    /// 부모 content box 위쪽에서의 y.
    pub y: Length,
}

impl Default for FreePosition {
    fn default() -> FreePosition {
        FreePosition {
            x: Length::from_raw(FiniteF32::ZERO),
            y: Length::from_raw(FiniteF32::ZERO),
        }
    }
}

/// 자식 하나의 제약(부모 content box 크기 기준).
fn child_constraint(
    inner: [Option<LayoutUnit>; 2],
    position: [LayoutUnit; 2],
) -> Result<ChildConstraint, crate::rounding::LayoutUnitError> {
    let mut fill = [None, None];
    for axis in Axis::BOTH {
        let i = axis.index();
        fill[i] = match inner[i] {
            Some(extent) => Some(sub_floor_zero(extent, position[i])?),
            None => None,
        };
    }
    Ok(ChildConstraint { cb: inner, fill })
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
        let position = engine.nodes[child].position;
        let constraint = child_constraint(inner, position).map_err(at(id))?;
        let size = engine.resolve_size(child, constraint)?;
        for axis in Axis::BOTH {
            let i = axis.index();
            let end = position[i].checked_add(size.get(axis)).map_err(at(id))?;
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
    let id = engine.nodes[index].id;
    let padding = engine.nodes[index].padding;
    let inner_axes = [Some(inner.width()), Some(inner.height())];
    let mut placements = Vec::new();
    for child in engine.children(index) {
        let position = engine.nodes[child].position;
        let constraint = child_constraint(inner_axes, position).map_err(at(id))?;
        let size = engine.resolve_size(child, constraint)?;
        let x = padding.left.checked_add(position[0]).map_err(at(id))?;
        let y = padding.top.checked_add(position[1]).map_err(at(id))?;
        placements.push((child, LayoutRect::from_origin_size(x, y, size)));
    }
    Ok(placements)
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
}
