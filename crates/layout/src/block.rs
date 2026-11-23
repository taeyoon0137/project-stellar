//! `display: block` 배치 (CSS 2 §9.4.1·§10.3.3, ADR-0076).
//!
//! - in-flow 자식(`display: none`·`position: absolute` 제외)을 content box 시작에서 세로로 차례로 놓습니다. margin이
//!   없으므로 margin collapsing도 없습니다.
//! - 자식 `width: auto`는 content box 너비를 채웁니다(`min-*`·`max-*`로 clamp). 비율(`aspect-ratio`)이 있고
//!   `height`가 definite면 너비는 비율로 정합니다.
//! - 자신의 `auto` 너비는 부모가 정하지 않았을 때(absolute·root·intrinsic 측정) `fit-content`입니다. min-content·
//!   max-content 너비는 in-flow 자식 기여(자식의 `width`·`min-*`·`max-*`를 반영한 바깥 크기)의 최댓값 + padding입니다.
//! - `auto` 높이는 in-flow 자식 높이 합 + padding입니다.
//! - 첫 baseline은 baseline을 가진 첫 in-flow 자식의 baseline입니다(없으면 없음).

use crate::engine::{
    at, clamp, fit_content, sub_floor_zero, Avail, Engine, NodeLayout, Placement, Request, SizeP,
    Sizing,
};
use crate::error::LayoutError;
use crate::geometry::Axis;
use crate::output::LayoutDiagnostic;
use crate::position::{place_absolute, relative_offset, StaticAlign, StaticRect};
use crate::rounding::LayoutUnit;
use crate::style::Position;

/// in-flow block 자식의 요청: `auto` 너비는 content box 너비로 늘립니다.
pub(crate) fn child_request(
    engine: &mut Engine<'_>,
    child: usize,
    content_width: LayoutUnit,
    parent: [Option<LayoutUnit>; 2],
) -> Result<Request, LayoutError> {
    let mut request = Request {
        known: [None, None],
        parent,
        available: [
            Avail::Definite(content_width),
            parent[1].map_or(Avail::MaxContent, Avail::Definite),
        ],
        content_only: [false, false],
    };
    let prepared = &engine.nodes[child];
    let ratio_width = prepared.ratio.is_some()
        && match prepared.size[1] {
            SizeP::Length(length) => length.resolve(parent[1], prepared.id)?.is_some(),
            _ => false,
        };
    if prepared.size[0] == SizeP::Auto && !ratio_width {
        let min = engine.min_size(child, Axis::Horizontal, request)?;
        let max = engine.max_size(child, Axis::Horizontal, request)?;
        request.known[0] = Some(clamp(content_width, min, max));
    }
    Ok(request)
}

/// in-flow 자식 기여로 min-content·max-content 너비(padding 포함)를 구합니다.
fn intrinsic_widths(
    engine: &mut Engine<'_>,
    children: &[usize],
    content_height: Option<LayoutUnit>,
    padding: LayoutUnit,
    id: stellar_core_types::ids::StableId,
) -> Result<(LayoutUnit, LayoutUnit), LayoutError> {
    let mut min = LayoutUnit::ZERO;
    let mut max = LayoutUnit::ZERO;
    for &child in children {
        if !engine.nodes[child].in_flow() {
            continue;
        }
        for (mode, out) in [(Avail::MinContent, &mut min), (Avail::MaxContent, &mut max)] {
            let request = Request {
                known: [None, None],
                parent: [None, content_height],
                available: [
                    mode,
                    content_height.map_or(Avail::MaxContent, Avail::Definite),
                ],
                content_only: [false, false],
            };
            *out = (*out).max(engine.layout(child, request)?.size[0]);
        }
    }
    let min = min.checked_add(padding).map_err(at(id))?;
    let max = max.checked_add(padding).map_err(at(id))?.max(min);
    Ok((min, max))
}

/// `display: block` node를 배치합니다.
pub(crate) fn layout(
    engine: &mut Engine<'_>,
    index: usize,
    request: Request,
    mut sizing: Sizing,
) -> Result<NodeLayout, LayoutError> {
    let prepared = engine.nodes[index].clone();
    let id = prepared.id;
    let pad = prepared.padding;
    let ph = pad.sum(Axis::Horizontal).map_err(at(id))?;
    let pv = pad.sum(Axis::Vertical).map_err(at(id))?;
    let children = engine.children(index);
    let content_height = sizing.size[1]
        .map(|h| sub_floor_zero(h, pv))
        .transpose()
        .map_err(at(id))?;
    let width_from_content = sizing.size[0].is_none();
    let width = match sizing.size[0] {
        Some(width) => width,
        None => {
            let (min, max) = intrinsic_widths(engine, &children, content_height, ph, id)?;
            let content = match sizing.ratio_floor {
                Some((Axis::Horizontal, value)) => value.max(min),
                _ => fit_content(request.available[0], min, max),
            };
            sizing.finish(Axis::Horizontal, content)
        }
    };
    sizing.after_width(width).map_err(at(id))?;
    if request.is_width_query() {
        return Ok(NodeLayout::width_only(width));
    }
    let content_width = sub_floor_zero(width, ph).map_err(at(id))?;
    let width_definite = !width_from_content || request.available[0].definite().is_some();
    let content_height = sizing.size[1]
        .map(|h| sub_floor_zero(h, pv))
        .transpose()
        .map_err(at(id))?;
    let parent = [width_definite.then_some(content_width), content_height];

    let mut slots: Vec<Option<Placement>> = vec![None; children.len()];
    let mut absolute = Vec::new();
    let mut diagnostics = Vec::new();
    let mut baseline = None;
    let mut y = pad.top;
    for (slot, &child) in children.iter().enumerate() {
        if engine.nodes[child].position == Position::Absolute {
            absolute.push((slot, child, y));
            continue;
        }
        let child_request = child_request(engine, child, content_width, parent)?;
        let layout = engine.layout(child, child_request)?;
        if baseline.is_none() {
            if let Some(b) = layout.baseline {
                baseline = Some(y.checked_add(b).map_err(at(id))?);
            }
        }
        if sizing.content_based[0]
            && matches!(
                engine.nodes[child].size[0],
                SizeP::Length(crate::engine::LengthP::Percent(_))
            )
        {
            diagnostics.push(LayoutDiagnostic::CyclicPercentage {
                ancestor: id,
                node: engine.nodes[child].id,
                axis: Axis::Horizontal,
            });
        }
        let [dx, dy] = relative_offset(engine, child, parent)?;
        slots[slot] = Some(Placement {
            child: engine.nodes[child].id,
            x: pad.left.checked_add(dx).map_err(at(id))?,
            y: y.checked_add(dy).map_err(at(id))?,
            request: child_request,
        });
        y = y.checked_add(layout.size[1]).map_err(at(id))?;
    }
    let flow_height = sub_floor_zero(y, pad.top).map_err(at(id))?;
    let height = sizing.finish(Axis::Vertical, flow_height.checked_add(pv).map_err(at(id))?);
    for (slot, child, static_y) in absolute {
        let rect = StaticRect {
            start: [pad.left, static_y],
            size: [
                content_width,
                sub_floor_zero(height, static_y.checked_add(pad.bottom).map_err(at(id))?)
                    .map_err(at(id))?,
            ],
            align: [StaticAlign::Start, StaticAlign::Start],
        };
        slots[slot] = Some(place_absolute(engine, child, [width, height], rect)?);
    }
    Ok(NodeLayout {
        size: [width, height],
        baseline,
        placements: slots.into_iter().flatten().collect(),
        diagnostics,
    })
}
