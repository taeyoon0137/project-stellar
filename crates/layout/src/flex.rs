//! `display: flex` 배치 (CSS Flexbox 1 §9, IX.11.4, ADR-0076).
//!
//! # 알고리즘
//!
//! CSS Flexbox §9를 따릅니다.
//!
//! 1. **container 너비**: 부모가 정하지 않았으면 `fit-content`입니다. row container의 max-content 너비는 item
//!    max-content 기여 합 + gap, min-content 너비는 `nowrap`이면 min-content 기여 합 + gap, `wrap`이면 가장 큰
//!    min-content 기여입니다. column container는 item 기여의 최댓값입니다.
//! 2. **flex base size**(§9.2.3): `flex-basis`가 definite면 그 값, `auto`면 main 축 크기 속성, 둘 다 없으면 비율과
//!    definite cross 크기로 변환한 값, 그 밖에는 콘텐츠 크기(max-content, column은 cross 크기에서의 콘텐츠 높이)입니다.
//!    hypothetical main 크기는 base를 `min-*`·`max-*`로 clamp한 값입니다(`min-*: auto`의 자동 최소 크기는
//!    [`automatic_min_main`]).
//! 3. **line**(§9.3): `nowrap`은 한 줄, `wrap`은 가용 main 공간을 넘지 않게 앞에서부터 나눕니다.
//! 4. **flexible length**(§9.7): 남는 공간이 양수면 `flex-grow`, 음수면 `flex-shrink × flex base size` 비례로
//!    나누고 min·max 위반 item을 freeze하며 반복합니다. factor 합이 1보다 작으면 초기 남는 공간 × 합만 나눕니다.
//!    나눌 때 내림 몫 뒤 남은 subpixel은 visual order 앞 item부터 1씩 줍니다.
//! 5. **cross 크기**(§9.4): hypothetical cross 크기는 결정된 main 크기로 배치한 크기입니다. 한 줄 container의 line
//!    cross 크기는 container inner cross 크기, 여러 줄이면 item 바깥 cross 크기의 최댓값입니다. `align-self`가
//!    `stretch`(`normal` 포함)이고 cross 크기 속성이 `auto`인 item만 line cross 크기로 늘립니다. `fit-content`·
//!    `<length>`·`<percentage>` 등 `auto`가 아닌 cross 크기 item은 늘리지 않고 `flex-start`처럼 놓습니다.
//! 6. **정렬**: `justify-content`(남는 공간이 음수면 `space-between`은 `flex-start`, `space-around`·`space-evenly`는
//!    `center`), `align-content`(`normal` = `stretch`, 여러 줄에서 남는 cross 공간을 line에 고르게 더함),
//!    `align-self`(`auto`는 부모 `align-items`)입니다. `row-gap`·`column-gap`은 축 고정 이름이라 main gap은 row
//!    container에서 `column-gap`, column container에서 `row-gap`입니다.
//! 7. **absolute 자식**: 유일한 flex item으로 놓았을 때의 위치를 static position으로 씁니다(§4.1).
//!
//! 첫 baseline은 첫 줄 첫 item의 baseline이고, item에 baseline이 없으면 그 border box 아래 변으로 합성합니다.

use crate::distribute::{distribute_equal, distribute_weighted};
use crate::engine::{
    at, clamp, fit_content, sub_floor_zero, transfer_ratio, Avail, BasisP, Engine, LengthP,
    NodeLayout, Placement, Request, SizeP, Sizing,
};
use crate::error::LayoutError;
use crate::geometry::Axis;
use crate::output::LayoutDiagnostic;
use crate::position::{place_absolute, relative_offset, StaticAlign, StaticRect};
use crate::rounding::LayoutUnit;
use crate::style::{ContentAlignment, FlexWrap, ItemAlignment, Position};

/// flex-relative cross 정렬(`start`·`end`는 wrap-reverse를 반영해 flex-start·flex-end로 바꾼 값).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CrossAlign {
    Stretch,
    Start,
    End,
    Center,
    Baseline,
}

/// flex item 하나의 계산 상태.
#[derive(Debug, Clone)]
struct Item {
    index: usize,
    slot: usize,
    base: LayoutUnit,
    hypothetical: LayoutUnit,
    min_main: LayoutUnit,
    max_main: Option<LayoutUnit>,
    target: LayoutUnit,
    frozen: bool,
    grow: u64,
    shrink: u64,
    align: CrossAlign,
    stretch: bool,
    cross: LayoutUnit,
    min_cross: LayoutUnit,
    max_cross: Option<LayoutUnit>,
    baseline: Option<LayoutUnit>,
    main_position: LayoutUnit,
    cross_position: LayoutUnit,
    request: Request,
}

/// flex line 하나.
#[derive(Debug, Clone)]
struct Line {
    items: Vec<usize>,
    cross: LayoutUnit,
    position: LayoutUnit,
}

/// cross 정렬 keyword를 flex-relative 값으로 바꿉니다.
fn cross_align(value: ItemAlignment, wrap_reverse: bool) -> CrossAlign {
    match value {
        ItemAlignment::Normal | ItemAlignment::Stretch => CrossAlign::Stretch,
        ItemAlignment::FlexStart => CrossAlign::Start,
        ItemAlignment::FlexEnd => CrossAlign::End,
        ItemAlignment::Start => {
            if wrap_reverse {
                CrossAlign::End
            } else {
                CrossAlign::Start
            }
        }
        ItemAlignment::End => {
            if wrap_reverse {
                CrossAlign::Start
            } else {
                CrossAlign::End
            }
        }
        ItemAlignment::Center => CrossAlign::Center,
        ItemAlignment::Baseline => CrossAlign::Baseline,
    }
}

/// content 정렬의 flex-relative 위치 해석.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Distribution {
    Start,
    End,
    Center,
    Between,
    Around,
    Evenly,
}

/// `justify-content`·`align-content` 값을 flex-relative 분배로 바꿉니다. `flipped`는 물리 시작이 flex 끝인지
/// 여부(`*-reverse`·`wrap-reverse`)이며 `start`·`end`(writing mode 기준)에만 영향을 줍니다. `normal`·`stretch`의
/// 위치는 `flex-start`입니다(line stretch는 호출자가 따로 처리).
fn distribution(value: ContentAlignment, flipped: bool) -> Distribution {
    let start = if flipped {
        Distribution::End
    } else {
        Distribution::Start
    };
    let end = if flipped {
        Distribution::Start
    } else {
        Distribution::End
    };
    match value {
        ContentAlignment::Normal | ContentAlignment::Stretch => Distribution::Start,
        ContentAlignment::FlexStart => Distribution::Start,
        ContentAlignment::FlexEnd => Distribution::End,
        ContentAlignment::Start => start,
        ContentAlignment::End => end,
        ContentAlignment::Center => Distribution::Center,
        ContentAlignment::SpaceBetween => Distribution::Between,
        ContentAlignment::SpaceAround => Distribution::Around,
        ContentAlignment::SpaceEvenly => Distribution::Evenly,
    }
}

/// 남는 공간 `free`를 `count`개 항목 사이에 나눈 (앞 offset, 항목 사이 추가 간격 목록).
fn distribute_free(
    kind: Distribution,
    free: LayoutUnit,
    count: usize,
) -> Result<(LayoutUnit, Vec<LayoutUnit>), crate::rounding::LayoutUnitError> {
    let gaps = count.saturating_sub(1);
    let zero = || vec![LayoutUnit::ZERO; gaps];
    // 분배 정렬의 fallback(CSS Box Alignment 3 §5.3): `space-between`은 `flex-start`, `space-around`·`space-evenly`는
    // `safe center`입니다. 남는 공간이 음수면 safe라 시작 쪽(Chromium 157 fixture로 확인), 항목이 하나면 가운데입니다.
    let kind = match kind {
        Distribution::Between if free < LayoutUnit::ZERO || count <= 1 => Distribution::Start,
        Distribution::Around | Distribution::Evenly if free < LayoutUnit::ZERO => {
            Distribution::Start
        }
        Distribution::Around | Distribution::Evenly if count <= 1 => Distribution::Center,
        other => other,
    };
    Ok(match kind {
        Distribution::Start => (LayoutUnit::ZERO, zero()),
        Distribution::End => (free, zero()),
        Distribution::Center => (free.checked_div_round(2)?, zero()),
        Distribution::Between => (LayoutUnit::ZERO, distribute_equal(free, gaps)),
        Distribution::Around => {
            // 항목마다 양옆 절반씩: 앞 offset = 몫/2, 사이 = 몫.
            let shares = distribute_equal(free, count);
            let first = shares[0].checked_div_round(2)?;
            let mut between = Vec::with_capacity(gaps);
            let mut previous_half = shares[0].checked_sub(first)?;
            for share in shares.iter().skip(1) {
                let half = share.checked_div_round(2)?;
                between.push(previous_half.checked_add(half)?);
                previous_half = share.checked_sub(half)?;
            }
            (first, between)
        }
        Distribution::Evenly => {
            let shares = distribute_equal(free, count + 1);
            (shares[0], shares[1..count].to_vec())
        }
    })
}

/// item `min-*: auto`의 자동 최소 크기(main 축). IX.11.23이 콘텐츠 기반 최소 크기로 바꿉니다.
fn automatic_min_main(
    _engine: &mut Engine<'_>,
    _item: usize,
    _main: Axis,
    _context: &ItemContext,
) -> Result<LayoutUnit, LayoutError> {
    Ok(LayoutUnit::ZERO)
}

/// item 계산에 쓰는 container 문맥.
#[derive(Debug, Clone, Copy)]
struct ItemContext {
    main: Axis,
    /// item 백분율 기준(container content box, indefinite 축은 `None`).
    parent: [Option<LayoutUnit>; 2],
    /// container inner cross 크기(정해졌으면).
    inner_cross: Option<LayoutUnit>,
    /// cross 축 가용 공간.
    cross_available: Avail,
}

/// item 기본 요청(크기 미정).
fn item_request(context: &ItemContext) -> Request {
    let cross = context.main.cross();
    let mut available = [Avail::MaxContent; 2];
    available[cross.index()] = context.cross_available;
    Request {
        known: [None, None],
        parent: context.parent,
        available,
        content_only: [false, false],
    }
}

/// min-content·max-content 기여(padding 제외 container 기준)로 container 너비 intrinsic을 구합니다.
fn intrinsic_widths(
    engine: &mut Engine<'_>,
    items: &[usize],
    main: Axis,
    wrap: bool,
    gap: LayoutUnit,
    parent_height: Option<LayoutUnit>,
    id: stellar_core_types::ids::StableId,
) -> Result<(LayoutUnit, LayoutUnit), LayoutError> {
    let mut min_sum = LayoutUnit::ZERO;
    let mut max_sum = LayoutUnit::ZERO;
    let mut min_max = LayoutUnit::ZERO;
    let mut max_max = LayoutUnit::ZERO;
    for &item in items {
        let mut contributions = [LayoutUnit::ZERO; 2];
        for (slot, mode) in [Avail::MinContent, Avail::MaxContent]
            .into_iter()
            .enumerate()
        {
            let request = Request {
                known: [None, None],
                parent: [None, parent_height],
                available: [
                    mode,
                    parent_height.map_or(Avail::MaxContent, Avail::Definite),
                ],
                content_only: [false, false],
            };
            contributions[slot] = engine.layout(item, request)?.size[0];
        }
        min_sum = min_sum.checked_add(contributions[0]).map_err(at(id))?;
        max_sum = max_sum.checked_add(contributions[1]).map_err(at(id))?;
        min_max = min_max.max(contributions[0]);
        max_max = max_max.max(contributions[1]);
    }
    let gaps = gap
        .checked_mul_int(i32::try_from(items.len().saturating_sub(1)).unwrap_or(i32::MAX))
        .map_err(at(id))?;
    Ok(if main == Axis::Horizontal {
        let max = max_sum.checked_add(gaps).map_err(at(id))?;
        let min = if wrap {
            min_max
        } else {
            min_sum.checked_add(gaps).map_err(at(id))?
        };
        (min, max.max(min))
    } else {
        (min_max, max_max.max(min_max))
    })
}

/// `display: flex` node를 배치합니다.
pub(crate) fn layout(
    engine: &mut Engine<'_>,
    index: usize,
    request: Request,
    mut sizing: Sizing,
) -> Result<NodeLayout, LayoutError> {
    let prepared = engine.nodes[index].clone();
    let id = prepared.id;
    let err = at(id);
    let main = prepared.direction.main_axis();
    let cross = main.cross();
    let (mi, ci) = (main.index(), cross.index());
    let reverse = prepared.direction.is_reverse();
    let wrap = prepared.wrap != FlexWrap::NoWrap;
    let wrap_reverse = prepared.wrap == FlexWrap::WrapReverse;
    let pad = prepared.padding;
    let pad_sum = [
        pad.sum(Axis::Horizontal).map_err(&err)?,
        pad.sum(Axis::Vertical).map_err(&err)?,
    ];
    let gap_main = prepared.gap[mi];
    let gap_cross = prepared.gap[ci];
    let children = engine.children(index);
    let items_index: Vec<(usize, usize)> = children
        .iter()
        .copied()
        .enumerate()
        .filter(|&(_, child)| engine.nodes[child].in_flow())
        .collect();
    let item_nodes: Vec<usize> = items_index.iter().map(|&(_, child)| child).collect();

    // 1. container 너비.
    let inner_height_known = sizing.size[1]
        .map(|h| sub_floor_zero(h, pad_sum[1]))
        .transpose()
        .map_err(&err)?;
    let width_from_content = sizing.size[0].is_none();
    let width = match sizing.size[0] {
        Some(width) => width,
        None => {
            let (min, max) = intrinsic_widths(
                engine,
                &item_nodes,
                main,
                wrap,
                gap_main,
                inner_height_known,
                id,
            )?;
            let min = min.checked_add(pad_sum[0]).map_err(&err)?;
            let max = max.checked_add(pad_sum[0]).map_err(&err)?;
            let content = match sizing.ratio_floor {
                Some((Axis::Horizontal, value)) => value.max(min),
                _ => fit_content(request.available[0], min, max),
            };
            sizing.finish(Axis::Horizontal, content)
        }
    };
    sizing.after_width(width).map_err(&err)?;
    if request.is_width_query() {
        return Ok(NodeLayout::width_only(width));
    }
    let inner_width = sub_floor_zero(width, pad_sum[0]).map_err(&err)?;
    let width_definite = !width_from_content || request.available[0].definite().is_some();
    let inner_height = sizing.size[1]
        .map(|h| sub_floor_zero(h, pad_sum[1]))
        .transpose()
        .map_err(&err)?;
    let inner = [Some(inner_width), inner_height];
    let parent = [width_definite.then_some(inner_width), inner_height];
    let inner_main = inner[mi];
    let inner_cross = inner[ci];
    let cross_available = match inner_cross {
        Some(value) => Avail::Definite(value),
        None => request.available[ci]
            .definite()
            .map(|space| sub_floor_zero(space, pad_sum[ci]))
            .transpose()
            .map_err(&err)?
            .map_or(Avail::MaxContent, Avail::Definite),
    };
    let context = ItemContext {
        main,
        parent,
        inner_cross,
        cross_available,
    };

    // 2. flex base size와 hypothetical main 크기.
    let mut items = Vec::with_capacity(items_index.len());
    for &(slot, child) in &items_index {
        items.push(base_item(
            engine,
            child,
            slot,
            &prepared,
            &context,
            wrap_reverse,
        )?);
    }

    // 3. line 나누기.
    let available_main = match inner_main {
        Some(value) => Some(value),
        None => request.available[mi]
            .definite()
            .map(|space| sub_floor_zero(space, pad_sum[mi]))
            .transpose()
            .map_err(&err)?,
    };
    let mut lines: Vec<Line> = Vec::new();
    {
        let mut current: Vec<usize> = Vec::new();
        let mut used = LayoutUnit::ZERO;
        for (n, item) in items.iter().enumerate() {
            let outer = item.hypothetical;
            let next = if current.is_empty() {
                outer
            } else {
                used.checked_add(gap_main)
                    .and_then(|v| v.checked_add(outer))
                    .map_err(&err)?
            };
            if wrap && !current.is_empty() && available_main.is_some_and(|limit| next > limit) {
                lines.push(Line {
                    items: std::mem::take(&mut current),
                    cross: LayoutUnit::ZERO,
                    position: LayoutUnit::ZERO,
                });
                current.push(n);
                used = outer;
            } else {
                current.push(n);
                used = next;
            }
        }
        if !current.is_empty() || lines.is_empty() {
            lines.push(Line {
                items: current,
                cross: LayoutUnit::ZERO,
                position: LayoutUnit::ZERO,
            });
        }
    }

    // container main 크기(column의 indefinite 높이).
    let inner_main = match inner_main {
        Some(value) => value,
        None => {
            let mut longest = LayoutUnit::ZERO;
            for line in &lines {
                longest = longest.max(line_sum(&items, line, gap_main, |i| i.hypothetical, id)?);
            }
            let total = sizing.finish(main, longest.checked_add(pad_sum[mi]).map_err(&err)?);
            sub_floor_zero(total, pad_sum[mi]).map_err(&err)?
        }
    };

    // 4. flexible length.
    for line in &lines {
        resolve_flexible_lengths(&mut items, line, inner_main, gap_main, id)?;
    }

    // 5. hypothetical cross 크기.
    let single_line = !wrap;
    for item in &mut items {
        let mut item_req = item_request(&context);
        item_req.known[mi] = Some(item.target);
        item_req.available[mi] = Avail::Definite(item.target);
        // cross 크기 속성이 definite이면 그대로 씁니다(sizing이 해석).
        let layout = engine.layout(item.index, item_req)?;
        item.cross = layout.size[ci];
        item.baseline = layout.baseline;
        item.request = item_req;
    }
    for line in &mut lines {
        let mut cross_size = LayoutUnit::ZERO;
        for &n in &line.items {
            cross_size = cross_size.max(items[n].cross);
        }
        line.cross = cross_size;
    }
    // container cross 크기.
    let inner_cross = match inner_cross {
        Some(value) => value,
        None => {
            let mut total = LayoutUnit::ZERO;
            for (n, line) in lines.iter().enumerate() {
                if n > 0 {
                    total = total.checked_add(gap_cross).map_err(&err)?;
                }
                total = total.checked_add(line.cross).map_err(&err)?;
            }
            let outer = sizing.finish(cross, total.checked_add(pad_sum[ci]).map_err(&err)?);
            sub_floor_zero(outer, pad_sum[ci]).map_err(&err)?
        }
    };
    let cross_definite = context.inner_cross.is_some();
    if single_line {
        if let Some(line) = lines.first_mut() {
            line.cross = inner_cross;
        }
    }
    // align-content: 여러 줄의 남는 cross 공간.
    let line_count = lines.len();
    let mut lines_total = LayoutUnit::ZERO;
    for (n, line) in lines.iter().enumerate() {
        if n > 0 {
            lines_total = lines_total.checked_add(gap_cross).map_err(&err)?;
        }
        lines_total = lines_total.checked_add(line.cross).map_err(&err)?;
    }
    let cross_free = inner_cross.checked_sub(lines_total).map_err(&err)?;
    if !single_line
        && cross_free > LayoutUnit::ZERO
        && matches!(
            prepared.align_content,
            ContentAlignment::Normal | ContentAlignment::Stretch
        )
    {
        let extra = distribute_equal(cross_free, line_count);
        for (line, add) in lines.iter_mut().zip(extra) {
            line.cross = line.cross.checked_add(add).map_err(&err)?;
        }
    }
    let stretched_lines_total = {
        let mut total = LayoutUnit::ZERO;
        for (n, line) in lines.iter().enumerate() {
            if n > 0 {
                total = total.checked_add(gap_cross).map_err(&err)?;
            }
            total = total.checked_add(line.cross).map_err(&err)?;
        }
        total
    };
    let cross_free = inner_cross
        .checked_sub(stretched_lines_total)
        .map_err(&err)?;
    let (mut cursor, between) = if single_line {
        (
            LayoutUnit::ZERO,
            vec![LayoutUnit::ZERO; line_count.saturating_sub(1)],
        )
    } else {
        distribute_free(
            distribution(prepared.align_content, wrap_reverse),
            cross_free,
            line_count,
        )
        .map_err(&err)?
    };
    for (n, line) in lines.iter_mut().enumerate() {
        if n > 0 {
            cursor = cursor
                .checked_add(gap_cross)
                .and_then(|v| v.checked_add(between[n - 1]))
                .map_err(&err)?;
        }
        line.position = cursor;
        cursor = cursor.checked_add(line.cross).map_err(&err)?;
    }

    // stretch와 cross 정렬.
    let _ = cross_definite;
    for line in &lines {
        for &n in &line.items {
            let item = &mut items[n];
            if item.align == CrossAlign::Stretch && item.stretch {
                let size = clamp(line.cross, item.min_cross, item.max_cross);
                if size != item.cross || item.request.known[ci].is_none() {
                    let mut stretched = item.request;
                    stretched.known[ci] = Some(size);
                    let layout = engine.layout(item.index, stretched)?;
                    item.cross = layout.size[ci];
                    item.baseline = layout.baseline;
                    item.request = stretched;
                }
            }
            let free = line.cross.checked_sub(item.cross).map_err(&err)?;
            let offset = match item.align {
                CrossAlign::Stretch | CrossAlign::Start | CrossAlign::Baseline => LayoutUnit::ZERO,
                CrossAlign::End => free,
                CrossAlign::Center => free.checked_div_round(2).map_err(&err)?,
            };
            item.cross_position = line.position.checked_add(offset).map_err(&err)?;
        }
    }

    // 6. main 축 정렬.
    for line in &lines {
        let used = line_sum(&items, line, gap_main, |i| i.target, id)?;
        let free = inner_main.checked_sub(used).map_err(&err)?;
        let (start, between) = distribute_free(
            distribution(prepared.justify_content, reverse),
            free,
            line.items.len(),
        )
        .map_err(&err)?;
        let mut cursor = start;
        for (k, &n) in line.items.iter().enumerate() {
            if k > 0 {
                cursor = cursor
                    .checked_add(gap_main)
                    .and_then(|v| v.checked_add(between[k - 1]))
                    .map_err(&err)?;
            }
            items[n].main_position = cursor;
            cursor = cursor.checked_add(items[n].target).map_err(&err)?;
        }
    }

    // 7. 물리 좌표와 배치.
    let mut slots: Vec<Option<Placement>> = vec![None; children.len()];
    let mut diagnostics = Vec::new();
    let mut first_baseline = None;
    let mut inner_size = [LayoutUnit::ZERO; 2];
    inner_size[mi] = inner_main;
    inner_size[ci] = inner_cross;
    for (k, item) in items.iter().enumerate() {
        let size = {
            let mut s = [LayoutUnit::ZERO; 2];
            s[mi] = item.target;
            s[ci] = item.cross;
            s
        };
        let mut flow = [LayoutUnit::ZERO; 2];
        flow[mi] = if reverse {
            inner_main
                .checked_sub(item.main_position)
                .and_then(|v| v.checked_sub(item.target))
                .map_err(&err)?
        } else {
            item.main_position
        };
        flow[ci] = if wrap_reverse {
            inner_cross
                .checked_sub(item.cross_position)
                .and_then(|v| v.checked_sub(item.cross))
                .map_err(&err)?
        } else {
            item.cross_position
        };
        let x = pad.left.checked_add(flow[0]).map_err(&err)?;
        let y = pad.top.checked_add(flow[1]).map_err(&err)?;
        if k == 0 {
            let local = item.baseline.unwrap_or(size[1]);
            first_baseline = Some(y.checked_add(local).map_err(&err)?);
        }
        if sizing.content_based[0]
            && matches!(
                engine.nodes[item.index].size[0],
                SizeP::Length(LengthP::Percent(_))
            )
        {
            diagnostics.push(LayoutDiagnostic::CyclicPercentage {
                ancestor: id,
                node: engine.nodes[item.index].id,
                axis: Axis::Horizontal,
            });
        }
        let [dx, dy] = relative_offset(engine, item.index, parent)?;
        slots[item.slot] = Some(Placement {
            child: engine.nodes[item.index].id,
            x: x.checked_add(dx).map_err(&err)?,
            y: y.checked_add(dy).map_err(&err)?,
            request: item.request,
        });
    }
    let mut outer = [LayoutUnit::ZERO; 2];
    for axis in Axis::BOTH {
        let i = axis.index();
        outer[i] = inner_size[i].checked_add(pad_sum[i]).map_err(&err)?;
    }

    // absolute 자식: 유일한 flex item으로 놓은 위치(static position).
    for (slot, &child) in children.iter().enumerate() {
        if engine.nodes[child].position != Position::Absolute {
            continue;
        }
        let main_align = match distribution(prepared.justify_content, reverse) {
            Distribution::Start | Distribution::Between => StaticAlign::Start,
            Distribution::End => StaticAlign::End,
            Distribution::Center | Distribution::Around | Distribution::Evenly => {
                StaticAlign::Center
            }
        };
        let main_align = flip(main_align, reverse);
        let cross_value = engine.nodes[child].align_self.resolve(prepared.align_items);
        let cross_align = match cross_align(cross_value, wrap_reverse) {
            CrossAlign::Stretch | CrossAlign::Start | CrossAlign::Baseline => StaticAlign::Start,
            CrossAlign::End => StaticAlign::End,
            CrossAlign::Center => StaticAlign::Center,
        };
        let cross_align = flip(cross_align, wrap_reverse);
        let mut align = [StaticAlign::Start; 2];
        align[mi] = main_align;
        align[ci] = cross_align;
        let rect = StaticRect {
            start: [pad.left, pad.top],
            size: inner_size,
            align,
        };
        slots[slot] = Some(place_absolute(engine, child, outer, rect)?);
    }

    Ok(NodeLayout {
        size: outer,
        baseline: first_baseline,
        placements: slots.into_iter().flatten().collect(),
        diagnostics,
    })
}

/// flex-relative 정렬을 물리 방향으로 뒤집습니다.
fn flip(align: StaticAlign, flipped: bool) -> StaticAlign {
    match (align, flipped) {
        (StaticAlign::Start, true) => StaticAlign::End,
        (StaticAlign::End, true) => StaticAlign::Start,
        (other, _) => other,
    }
}

/// line의 main 크기 합 + gap.
fn line_sum(
    items: &[Item],
    line: &Line,
    gap: LayoutUnit,
    size: impl Fn(&Item) -> LayoutUnit,
    id: stellar_core_types::ids::StableId,
) -> Result<LayoutUnit, LayoutError> {
    let mut total = LayoutUnit::ZERO;
    for (k, &n) in line.items.iter().enumerate() {
        if k > 0 {
            total = total.checked_add(gap).map_err(at(id))?;
        }
        total = total.checked_add(size(&items[n])).map_err(at(id))?;
    }
    Ok(total)
}

/// item의 flex base size와 hypothetical main 크기를 구합니다.
fn base_item(
    engine: &mut Engine<'_>,
    child: usize,
    slot: usize,
    container: &crate::engine::Prepared,
    context: &ItemContext,
    wrap_reverse: bool,
) -> Result<Item, LayoutError> {
    let main = context.main;
    let cross = main.cross();
    let (mi, ci) = (main.index(), cross.index());
    let prepared = engine.nodes[child].clone();
    let id = prepared.id;
    let err = at(id);
    let request = item_request(context);
    let align = cross_align(
        prepared.align_self.resolve(container.align_items),
        wrap_reverse,
    );
    let stretch = align == CrossAlign::Stretch && prepared.size[ci] == SizeP::Auto;
    let min_cross = engine.min_size(child, cross, request)?;
    let max_cross = engine.max_size(child, cross, request)?;
    // definite cross 크기(비율·column 콘텐츠 높이 계산용).
    let definite_cross = match engine.preferred(child, cross, request)? {
        Some(value) => Some(clamp(value, min_cross, max_cross)),
        None if stretch => context
            .inner_cross
            .map(|value| clamp(value, min_cross, max_cross)),
        None => None,
    };
    let specified = match prepared.basis {
        BasisP::Length(length) => length.resolve(context.parent[mi], id)?,
        BasisP::Content => None,
        BasisP::Auto => match prepared.size[mi] {
            SizeP::Auto => None,
            _ => engine.preferred(child, main, request)?,
        },
    };
    let pad_main = prepared.padding.sum(main).map_err(&err)?;
    let base = match specified {
        Some(value) => value,
        None => match (prepared.ratio, definite_cross) {
            (Some(ratio), Some(cross_size)) => {
                transfer_ratio(ratio, cross, cross_size).map_err(&err)?
            }
            _ => {
                let mut probe = request;
                probe.content_only[mi] = true;
                probe.available[mi] = Avail::MaxContent;
                probe.known[ci] = definite_cross;
                engine.layout(child, probe)?.size[mi]
            }
        },
    }
    .max(pad_main);
    let min_main = match prepared.min[mi] {
        SizeP::Auto => automatic_min_main(engine, child, main, context)?,
        _ => engine.min_size(child, main, request)?,
    };
    let max_main = engine.max_size(child, main, request)?;
    let hypothetical = clamp(base, min_main, max_main).max(pad_main);
    Ok(Item {
        index: child,
        slot,
        base,
        hypothetical,
        min_main,
        max_main,
        target: hypothetical,
        frozen: false,
        grow: prepared.grow,
        shrink: prepared.shrink,
        align,
        stretch,
        cross: LayoutUnit::ZERO,
        min_cross,
        max_cross,
        baseline: None,
        main_position: LayoutUnit::ZERO,
        cross_position: LayoutUnit::ZERO,
        request,
    })
}

/// line 하나의 flexible length를 해석합니다(CSS Flexbox §9.7).
fn resolve_flexible_lengths(
    items: &mut [Item],
    line: &Line,
    inner_main: LayoutUnit,
    gap: LayoutUnit,
    id: stellar_core_types::ids::StableId,
) -> Result<(), LayoutError> {
    let err = at(id);
    let count = line.items.len();
    if count == 0 {
        return Ok(());
    }
    let gaps = gap
        .checked_mul_int(i32::try_from(count - 1).unwrap_or(i32::MAX))
        .map_err(&err)?;
    let mut hypothetical_sum = gaps;
    for &n in &line.items {
        hypothetical_sum = hypothetical_sum
            .checked_add(items[n].hypothetical)
            .map_err(&err)?;
    }
    let growing = hypothetical_sum < inner_main;
    for &n in &line.items {
        let item = &mut items[n];
        item.target = item.hypothetical;
        let factor = if growing { item.grow } else { item.shrink };
        item.frozen = factor == 0
            || (growing && item.base > item.hypothetical)
            || (!growing && item.base < item.hypothetical);
    }
    let free_of = |items: &[Item]| -> Result<LayoutUnit, LayoutError> {
        let mut used = gaps;
        for &n in &line.items {
            let item = &items[n];
            let size = if item.frozen { item.target } else { item.base };
            used = used.checked_add(size).map_err(at(id))?;
        }
        inner_main.checked_sub(used).map_err(at(id))
    };
    let initial_free = free_of(items)?;
    for _ in 0..=count {
        let unfrozen: Vec<usize> = line
            .items
            .iter()
            .copied()
            .filter(|&n| !items[n].frozen)
            .collect();
        if unfrozen.is_empty() {
            break;
        }
        let mut free = free_of(items)?;
        let factor_sum: u64 = unfrozen
            .iter()
            .map(|&n| {
                if growing {
                    items[n].grow
                } else {
                    items[n].shrink
                }
            })
            .sum();
        // factor 합이 1(= 64/64)보다 작으면 초기 남는 공간 × 합만 씁니다.
        if factor_sum < 64 {
            let scaled = i64::from(initial_free.raw()) * factor_sum as i64 / 64;
            let scaled = LayoutUnit::from_raw(i32::try_from(scaled).unwrap_or(i32::MAX));
            if scaled.raw().abs() < free.raw().abs() {
                free = scaled;
            }
        }
        if growing {
            let weights: Vec<u64> = unfrozen.iter().map(|&n| items[n].grow).collect();
            let shares = distribute_weighted(free.max(LayoutUnit::ZERO), &weights).map_err(&err)?;
            for (&n, share) in unfrozen.iter().zip(shares) {
                items[n].target = items[n].base.checked_add(share).map_err(&err)?;
            }
        } else {
            let weights: Vec<u64> = unfrozen
                .iter()
                .map(|&n| {
                    items[n]
                        .shrink
                        .saturating_mul(items[n].base.raw().max(0) as u64)
                })
                .collect();
            let magnitude = LayoutUnit::ZERO.checked_sub(free).map_err(&err)?;
            let shares =
                distribute_weighted(magnitude.max(LayoutUnit::ZERO), &weights).map_err(&err)?;
            for (&n, share) in unfrozen.iter().zip(shares) {
                items[n].target = items[n].base.checked_sub(share).map_err(&err)?;
            }
        }
        // min·max 위반 처리.
        let mut total_violation: i64 = 0;
        let mut violation = vec![0i64; items.len()];
        for &n in &unfrozen {
            let item = &mut items[n];
            let clamped = clamp(item.target, item.min_main, item.max_main).max(LayoutUnit::ZERO);
            let delta = i64::from(clamped.raw()) - i64::from(item.target.raw());
            violation[n] = delta;
            total_violation += delta;
            item.target = clamped;
        }
        match total_violation.cmp(&0) {
            std::cmp::Ordering::Equal => {
                for &n in &unfrozen {
                    items[n].frozen = true;
                }
            }
            std::cmp::Ordering::Greater => {
                for &n in &unfrozen {
                    if violation[n] > 0 {
                        items[n].frozen = true;
                    }
                }
            }
            std::cmp::Ordering::Less => {
                for &n in &unfrozen {
                    if violation[n] < 0 {
                        items[n].frozen = true;
                    }
                }
            }
        }
    }
    Ok(())
}
