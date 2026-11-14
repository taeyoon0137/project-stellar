//! Overlay 배치 (X.5.1).
//!
//! # 규칙
//!
//! - 모든 자식은 **같은 containing block** = 부모 content rect를 씁니다. 자식끼리 서로의 위치·크기에
//!   영향을 주지 않습니다(flow 없음).
//! - 자식 크기는 자식 sizing으로 정합니다: `Fill`은 content rect 전체, `Percentage`는 content rect
//!   기준, `Fixed`·`HugContent`는 자기 크기입니다.
//! - 위치는 container의 축별 정렬([`OverlayLayout`])로 content rect 안에 놓습니다: `Start` 0,
//!   `Center` `(content − 크기)/2`(ties away from zero), `End` `content − 크기`.
//! - **z-order는 OrderKey**입니다: 결과 box 순서(뒤 = 위)가 child visual order이며 삽입 순서·StableId와
//!   무관합니다.
//! - Hug 측정: 축별 자식 크기 최댓값.

use stellar_core_types::ids::StableId;

use crate::codec::{Reader, Writer};
use crate::engine::{at, ChildConstraint, Engine};
use crate::error::LayoutError;
use crate::geometry::{Axis, LayoutRect, LayoutSize};
use crate::input::ContainerLayout;
use crate::rounding::{LayoutUnit, LayoutUnitError};

/// Overlay 한 축 정렬 (X.5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OverlayAlignment {
    /// 시작.
    #[default]
    Start,
    /// 가운데.
    Center,
    /// 끝.
    End,
}

/// Overlay container 설정 (X.5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct OverlayLayout {
    /// 가로 정렬.
    pub horizontal: OverlayAlignment,
    /// 세로 정렬.
    pub vertical: OverlayAlignment,
}

impl OverlayLayout {
    /// 축 정렬.
    pub const fn alignment(&self, axis: Axis) -> OverlayAlignment {
        match axis {
            Axis::Horizontal => self.horizontal,
            Axis::Vertical => self.vertical,
        }
    }
}

const fn alignment_tag(alignment: OverlayAlignment) -> u8 {
    match alignment {
        OverlayAlignment::Start => 0,
        OverlayAlignment::Center => 1,
        OverlayAlignment::End => 2,
    }
}

fn alignment_from_tag(tag: u8) -> Result<OverlayAlignment, LayoutError> {
    match tag {
        0 => Ok(OverlayAlignment::Start),
        1 => Ok(OverlayAlignment::Center),
        2 => Ok(OverlayAlignment::End),
        _ => Err(LayoutError::Malformed {
            reason: "알 수 없는 overlay alignment tag",
        }),
    }
}

/// Overlay 설정 canonical bytes(가로, 세로 정렬 tag).
pub(crate) fn encode_overlay(w: &mut Writer, overlay: &OverlayLayout) {
    w.u8(alignment_tag(overlay.horizontal));
    w.u8(alignment_tag(overlay.vertical));
}

/// Overlay 설정 canonical bytes를 읽습니다.
pub(crate) fn decode_overlay(r: &mut Reader<'_>) -> Result<OverlayLayout, LayoutError> {
    Ok(OverlayLayout {
        horizontal: alignment_from_tag(r.u8()?)?,
        vertical: alignment_from_tag(r.u8()?)?,
    })
}

fn overlay_of(engine: &Engine<'_>, index: usize) -> Result<OverlayLayout, LayoutError> {
    match &engine.node(index).container {
        ContainerLayout::Overlay(overlay) => Ok(*overlay),
        _ => Err(LayoutError::InvalidBox {
            node: engine.nodes[index].id,
            reason: "Overlay container가 아닙니다",
        }),
    }
}

/// 모든 자식이 공유하는 제약(containing block = content rect).
fn shared_constraint(inner: [Option<LayoutUnit>; 2]) -> ChildConstraint {
    ChildConstraint {
        cb: inner,
        fill: inner,
        forced: [None, None],
    }
}

/// Overlay container의 content extent(Hug 측정).
pub(crate) fn measure(
    engine: &mut Engine<'_>,
    index: usize,
    inner: [Option<LayoutUnit>; 2],
) -> Result<[LayoutUnit; 2], LayoutError> {
    let mut extent = [LayoutUnit::ZERO; 2];
    for child in engine.children(index) {
        let size = engine.resolve_size(child, shared_constraint(inner))?;
        for axis in Axis::BOTH {
            extent[axis.index()] = extent[axis.index()].max(size.get(axis));
        }
    }
    Ok(extent)
}

fn aligned(
    alignment: OverlayAlignment,
    extent: LayoutUnit,
    size: LayoutUnit,
) -> Result<LayoutUnit, LayoutUnitError> {
    match alignment {
        OverlayAlignment::Start => Ok(LayoutUnit::ZERO),
        OverlayAlignment::Center => extent.checked_sub(size)?.checked_div_round(2),
        OverlayAlignment::End => extent.checked_sub(size),
    }
}

/// Overlay container의 자식 배치(visual order = z-order).
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

/// Overlay container `index`(content 크기 `inner`) 안 자식 하나의 rect(부모 local 좌표).
///
/// 모든 자식이 같은 containing block을 쓰므로 위치는 자신의 크기와 정렬에만 의존합니다(형제와 무관).
/// 증분 레이아웃(X.7.3)은 바뀐 자식 하나만 이 함수로 다시 놓습니다.
pub(crate) fn child_rect(
    engine: &mut Engine<'_>,
    index: usize,
    inner: LayoutSize,
    child: usize,
) -> Result<LayoutRect, LayoutError> {
    let id: StableId = engine.nodes[index].id;
    let padding = engine.nodes[index].padding;
    let overlay = overlay_of(engine, index)?;
    let constraint = shared_constraint([Some(inner.width()), Some(inner.height())]);
    let size = engine.resolve_size(child, constraint)?;
    let x = aligned(overlay.horizontal, inner.width(), size.width())
        .and_then(|x| padding.left.checked_add(x))
        .map_err(at(id))?;
    let y = aligned(overlay.vertical, inner.height(), size.height())
        .and_then(|y| padding.top.checked_add(y))
        .map_err(at(id))?;
    Ok(LayoutRect::from_origin_size(x, y, size))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::compute_layout;
    use crate::input::{LayoutInput, LayoutNode, SizingMode};
    use crate::output::LayoutResult;
    use crate::test_support::{id, key, len, px};
    use stellar_core_types::value::{Insets, Percentage};

    fn overlay_root(overlay: OverlayLayout, children: &[(u64, u32)]) -> LayoutNode {
        let mut root = LayoutNode::new(id(1))
            .with_fixed_size(len(120.0), len(80.0))
            .with_padding(Insets::new(10.0, 10.0, 10.0, 10.0).unwrap())
            .with_container(ContainerLayout::Overlay(overlay));
        for (n, order) in children {
            root = root.with_child(id(*n), key(*order));
        }
        root
    }

    fn rect(result: &LayoutResult, n: u64) -> LayoutRect {
        result.get(id(n)).unwrap().local_rect()
    }

    #[test]
    fn overlay_fixture_shares_containing_block() {
        // content 100×60(원점 10, 10). 자식: Fill, 고정 40×20, 50% × 25%.
        let children = [(2, 1), (3, 2), (4, 3)];
        let cases = [
            (
                OverlayAlignment::Start,
                OverlayAlignment::Start,
                (10, 10),
                (10.0, 10.0),
            ),
            // 50×15를 100×60 가운데: (25, 22.5) + padding.
            (
                OverlayAlignment::Center,
                OverlayAlignment::Center,
                (40, 30),
                (35.0, 32.5),
            ),
            (
                OverlayAlignment::End,
                OverlayAlignment::End,
                (70, 50),
                (60.0, 55.0),
            ),
        ];
        for (horizontal, vertical, fixed_origin, pct_origin) in cases {
            let overlay = OverlayLayout {
                horizontal,
                vertical,
            };
            let root = overlay_root(overlay, &children);
            let fill = LayoutNode::new(id(2)).with_sizing(SizingMode::Fill, SizingMode::Fill);
            let fixed = LayoutNode::new(id(3)).with_fixed_size(len(40.0), len(20.0));
            let pct = LayoutNode::new(id(4)).with_sizing(
                SizingMode::Percentage(Percentage::new(50.0).unwrap()),
                SizingMode::Percentage(Percentage::new(25.0).unwrap()),
            );
            let result =
                compute_layout(&LayoutInput::new(id(1), vec![root, fill, fixed, pct])).unwrap();
            // Fill은 정렬과 무관하게 content rect 전체.
            assert_eq!(
                rect(&result, 2),
                LayoutRect::new(px(10), px(10), px(100), px(60)).unwrap()
            );
            let f = rect(&result, 3);
            assert_eq!(
                (f.x(), f.y()),
                (px(fixed_origin.0), px(fixed_origin.1)),
                "{overlay:?}"
            );
            let p = rect(&result, 4);
            assert_eq!((p.width(), p.height()), (px(50), px(15)));
            assert_eq!(
                (p.x().to_px_f64(), p.y().to_px_f64()),
                pct_origin,
                "{overlay:?}"
            );
        }
    }

    #[test]
    fn z_order_follows_order_key() {
        // 삽입 순서 2, 3, 4 / OrderKey 3, 1, 2 → z-order(아래→위) 3, 4, 2.
        let root = overlay_root(OverlayLayout::default(), &[(2, 3), (3, 1), (4, 2)]);
        let nodes = vec![
            root,
            LayoutNode::new(id(2)).with_fixed_size(len(30.0), len(30.0)),
            LayoutNode::new(id(3)).with_fixed_size(len(30.0), len(30.0)),
            LayoutNode::new(id(4)).with_fixed_size(len(30.0), len(30.0)),
        ];
        let result = compute_layout(&LayoutInput::new(id(1), nodes.clone())).unwrap();
        let order: Vec<_> = result.boxes().iter().skip(1).map(|b| b.id()).collect();
        assert_eq!(order, vec![id(3), id(4), id(2)]);
        // 모두 같은 containing block 원점에 겹침.
        for n in 2..=4 {
            assert_eq!(
                (rect(&result, n).x(), rect(&result, n).y()),
                (px(10), px(10))
            );
        }
        // node 목록 순서를 바꿔도 같은 결과.
        let mut reversed = nodes;
        reversed.reverse();
        assert_eq!(
            compute_layout(&LayoutInput::new(id(1), reversed)).unwrap(),
            result
        );
    }

    #[test]
    fn hug_overlay_and_round_trip() {
        let root = LayoutNode::new(id(1))
            .with_sizing(SizingMode::HugContent, SizingMode::HugContent)
            .with_container(ContainerLayout::Overlay(OverlayLayout {
                horizontal: OverlayAlignment::End,
                vertical: OverlayAlignment::Center,
            }))
            .with_child(id(2), key(1))
            .with_child(id(3), key(2));
        let wide = LayoutNode::new(id(2)).with_fixed_size(len(60.0), len(10.0));
        let tall = LayoutNode::new(id(3)).with_fixed_size(len(20.0), len(40.0));
        let input = LayoutInput::new(id(1), vec![root, wide, tall]);
        let result = compute_layout(&input).unwrap();
        assert_eq!(
            rect(&result, 1),
            LayoutRect::new(px(0), px(0), px(60), px(40)).unwrap()
        );
        assert_eq!(
            (rect(&result, 2).x(), rect(&result, 2).y()),
            (px(0), px(15))
        );
        assert_eq!(
            (rect(&result, 3).x(), rect(&result, 3).y()),
            (px(40), px(0))
        );
        assert_eq!(LayoutInput::decode(&input.encode()).unwrap(), input);
        let mut w = Writer::new();
        encode_overlay(
            &mut w,
            &OverlayLayout {
                horizontal: OverlayAlignment::End,
                vertical: OverlayAlignment::Center,
            },
        );
        assert_eq!(w.finish(), vec![2, 1]);
        assert!(alignment_from_tag(3).is_err());
    }
}
