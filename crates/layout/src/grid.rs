//! `display: grid` 배치 (CSS Grid 1 §7·§8·§10·§11, IX.11.5, ADR-0076).
//!
//! # 배치
//!
//! - **track 정의**: `grid-template-columns`·`-rows`가 명시 grid이고, 그 밖의 line을 쓰는 item은 암시 track을 만듭니다.
//!   암시 track 크기는 `grid-auto-columns`·`-rows` pattern입니다(명시 grid 뒤는 앞에서부터, 앞은 끝에서부터 반복).
//! - **item 배치**(§8.5): 두 축이 정해진 item, 행(열 흐름이면 열)이 정해진 item, 나머지 순서로 sparse 자동 배치합니다
//!   (`grid-auto-flow: row | column`). 음수 line은 명시 grid 끝에서 셉니다.
//! - **track sizing**(§11): 열을 먼저, 그 열 크기로 행을 정합니다. intrinsic track은 span 1 item부터, 여러 track에 걸친
//!   item은 span 크기 순서로 extra space를 나눠 키우고, flexible(`fr`) track에 걸친 item은 마지막에 fr 비율로 나눕니다.
//!   track 최솟값 `auto`는 item 최소 기여(자동 최소 크기)이므로 bare `<n>fr`은 `minmax(auto, <n>fr)`입니다. fr 크기는
//!   §11.7(합이 1보다 작으면 1로 나눔, indefinite면 factor 1 이하 track은 base를 factor로 나누지 않음)입니다. 남는 공간이
//!   있으면 `auto` 최댓값 track을 늘립니다(`justify-content`·`align-content`가 `normal`·`stretch`일 때).
//! - **정렬**: track 묶음은 `justify-content`·`align-content`, item은 grid 영역 안 `justify-self`·`align-self`
//!   (`auto`는 `justify-items`·`align-items`)로 놓습니다. `normal`은 크기가 `auto`인 축에서 `stretch`이고(비율이 그 축을
//!   정하면 `start`), 그 밖에는 `start`입니다. `stretch`는 크기가 `auto`인 축만 영역 크기로 늘립니다.
//! - **absolute 자식**: containing block은 padding box이고, static position은 padding box 안 `justify-self`·`align-self`
//!   정렬입니다(명시 line 배치는 받지 않음).
//! - **첫 baseline**: 첫 행에서 열 순서가 가장 앞인 item의 baseline(없으면 border box 아래 변)입니다.
//!
//! 한 축의 track은 [`crate::input::MAX_GRID_TRACKS`]개, cell은 [`MAX_GRID_CELLS`]개까지입니다.

use std::sync::Arc;

use stellar_core_types::ids::StableId;
use stellar_core_types::value::Length;

use crate::distribute::distribute_equal;
use crate::engine::{
    at, clamp, fit_content, sub_floor_zero, Avail, Engine, LengthP, NodeLayout, Placement, Request,
    SizeP, Sizing,
};
use crate::error::{LayoutError, LayoutLimit};
use crate::geometry::Axis;
use crate::input::{length_unit, weight, MAX_GRID_TRACKS};
use crate::output::LayoutDiagnostic;
use crate::position::{place_absolute, relative_offset, StaticAlign, StaticRect};
use crate::property::LayoutProperty;
use crate::rounding::{LayoutUnit, LayoutUnitError};
use crate::style::{
    ContentAlignment, GridAutoFlow, GridLine, ItemAlignment, LengthPercentage, MaxTrackSize,
    MinTrackSize, Position, TrackSize,
};

/// grid 하나의 최대 cell 수(열 × 행).
pub const MAX_GRID_CELLS: usize = 1 << 20;

/// grid 자동 배치 작업량 상한(cell 검사 수).
pub const MAX_GRID_WORK: usize = MAX_GRID_CELLS * 4;

/// 변환된 track 최솟값.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MinTrackP {
    Auto,
    Length(LengthP),
    MinContent,
    MaxContent,
}

/// 변환된 track 최댓값.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MaxTrackP {
    Auto,
    Length(LengthP),
    MinContent,
    MaxContent,
    /// `fr` factor(1/64 단위).
    Fraction(u64),
    FitContent(LengthP),
}

/// 변환된 track 크기.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TrackP {
    pub(crate) min: MinTrackP,
    pub(crate) max: MaxTrackP,
}

/// 변환된 grid container·item 값.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GridPrepared {
    /// 명시 track(`[열, 행]`).
    pub(crate) template: [Arc<[TrackP]>; 2],
    /// 암시 track pattern(`[열, 행]`).
    pub(crate) auto: [Arc<[TrackP]>; 2],
    pub(crate) flow: GridAutoFlow,
    /// item line(`[[열 시작, 열 끝], [행 시작, 행 끝]]`).
    pub(crate) lines: [[GridLine; 2]; 2],
}

fn prepare_length_percentage(
    node: StableId,
    property: LayoutProperty,
    value: LengthPercentage,
) -> Result<LengthP, LayoutError> {
    Ok(match value {
        LengthPercentage::Length(l) => LengthP::Unit(length_unit(node, property, l, false)?),
        LengthPercentage::Percentage(p) => LengthP::Percent(p.raw()),
    })
}

fn prepare_tracks(
    node: StableId,
    property: LayoutProperty,
    tracks: &[TrackSize],
) -> Result<Arc<[TrackP]>, LayoutError> {
    let unit = |l: Length| length_unit(node, property, l, false).map(LengthP::Unit);
    tracks
        .iter()
        .map(|track| {
            let min = match track.min {
                MinTrackSize::Auto => MinTrackP::Auto,
                MinTrackSize::Length(l) => MinTrackP::Length(unit(l)?),
                MinTrackSize::Percentage(p) => MinTrackP::Length(LengthP::Percent(p.raw())),
                MinTrackSize::MinContent => MinTrackP::MinContent,
                MinTrackSize::MaxContent => MinTrackP::MaxContent,
            };
            let max = match track.max {
                MaxTrackSize::Auto => MaxTrackP::Auto,
                MaxTrackSize::Length(l) => MaxTrackP::Length(unit(l)?),
                MaxTrackSize::Percentage(p) => MaxTrackP::Length(LengthP::Percent(p.raw())),
                MaxTrackSize::MinContent => MaxTrackP::MinContent,
                MaxTrackSize::MaxContent => MaxTrackP::MaxContent,
                MaxTrackSize::Fraction(f) => MaxTrackP::Fraction(weight(node, property, f)?),
                MaxTrackSize::FitContent(limit) => {
                    MaxTrackP::FitContent(prepare_length_percentage(node, property, limit)?)
                }
            };
            Ok(TrackP { min, max })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Arc::from)
}

/// node의 grid 값을 변환합니다.
pub(crate) fn prepare_grid(
    node: StableId,
    style: &crate::style::LayoutStyle,
) -> Result<GridPrepared, LayoutError> {
    Ok(GridPrepared {
        template: [
            prepare_tracks(
                node,
                LayoutProperty::GridTemplateColumns,
                &style.grid_template_columns,
            )?,
            prepare_tracks(
                node,
                LayoutProperty::GridTemplateRows,
                &style.grid_template_rows,
            )?,
        ],
        auto: [
            prepare_tracks(
                node,
                LayoutProperty::GridAutoColumns,
                &style.grid_auto_columns,
            )?,
            prepare_tracks(node, LayoutProperty::GridAutoRows, &style.grid_auto_rows)?,
        ],
        flow: style.grid_auto_flow,
        lines: [
            [style.grid_column_start, style.grid_column_end],
            [style.grid_row_start, style.grid_row_end],
        ],
    })
}

// --- 배치(§8) ---

/// 한 축의 item 범위(line 번호, 시작 포함·끝 제외, 명시 grid line은 1..=N+1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Span {
    start: i32,
    end: i32,
}

/// 축 하나의 line 해석 결과: 정해진 범위 또는 자동 배치할 span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Resolved {
    Definite(Span),
    Auto(i32),
}

/// `grid-*-start`·`-end`를 해석합니다(§8.3). `explicit`는 명시 track 수입니다.
fn resolve_lines(start: GridLine, end: GridLine, explicit: i32) -> Resolved {
    let line = |n: i16| -> i32 {
        let n = i32::from(n);
        if n > 0 {
            n
        } else {
            explicit + 2 + n
        }
    };
    match (start, end) {
        (GridLine::Line(s), GridLine::Line(e)) => {
            let (s, e) = (line(s), line(e));
            if s == e {
                Resolved::Definite(Span {
                    start: s,
                    end: s + 1,
                })
            } else {
                Resolved::Definite(Span {
                    start: s.min(e),
                    end: s.max(e),
                })
            }
        }
        (GridLine::Line(s), GridLine::Span(n)) => {
            let s = line(s);
            Resolved::Definite(Span {
                start: s,
                end: s + i32::from(n),
            })
        }
        (GridLine::Span(n), GridLine::Line(e)) => {
            let e = line(e);
            Resolved::Definite(Span {
                start: e - i32::from(n),
                end: e,
            })
        }
        (GridLine::Line(s), GridLine::Auto) => {
            let s = line(s);
            Resolved::Definite(Span {
                start: s,
                end: s + 1,
            })
        }
        (GridLine::Auto, GridLine::Line(e)) => {
            let e = line(e);
            Resolved::Definite(Span {
                start: e - 1,
                end: e,
            })
        }
        (GridLine::Span(n), GridLine::Auto | GridLine::Span(_))
        | (GridLine::Auto, GridLine::Span(n)) => Resolved::Auto(i32::from(n)),
        (GridLine::Auto, GridLine::Auto) => Resolved::Auto(1),
    }
}

/// 배치된 item(축 순서 `[열, 행]`, track index 0부터).
#[derive(Debug, Clone, Copy)]
struct GridItem {
    index: usize,
    slot: usize,
    area: [Span; 2],
}

/// 점유 cell 표(행 수는 늘어남).
struct Occupancy {
    columns: usize,
    rows: Vec<Vec<bool>>,
}

impl Occupancy {
    fn fits(&self, column: i32, row: i32, span: [i32; 2], offset: [i32; 2]) -> bool {
        for r in row..row + span[1] {
            let ri = (r - offset[1]) as usize;
            let Some(cells) = self.rows.get(ri) else {
                continue;
            };
            for c in column..column + span[0] {
                let ci = (c - offset[0]) as usize;
                if ci < cells.len() && cells[ci] {
                    return false;
                }
            }
        }
        true
    }

    fn mark(&mut self, area: [Span; 2], offset: [i32; 2]) {
        for r in area[1].start..area[1].end {
            let ri = (r - offset[1]) as usize;
            while self.rows.len() <= ri {
                self.rows.push(vec![false; self.columns]);
            }
            for c in area[0].start..area[0].end {
                let ci = (c - offset[0]) as usize;
                if ci >= self.rows[ri].len() {
                    self.rows[ri].resize(ci + 1, false);
                }
                self.rows[ri][ci] = true;
            }
        }
    }
}

/// item 배치 결과와 축별 line 범위.
struct Placed {
    items: Vec<GridItem>,
    /// 축별 가장 앞 line(1 이하)과 track 수.
    first_line: [i32; 2],
    count: [usize; 2],
}

/// item을 배치합니다(§8.5, sparse).
fn place(
    engine: &Engine<'_>,
    node: StableId,
    grid: &GridPrepared,
    items: &[(usize, usize)],
) -> Result<Placed, LayoutError> {
    let explicit = [grid.template[0].len() as i32, grid.template[1].len() as i32];
    // 흐름 축(flow)과 그 반대 축. row 흐름이면 행이 늘어나는 축(1), 열은 고정 축(0).
    let (fixed, grows) = match grid.flow {
        GridAutoFlow::Row => (0usize, 1usize),
        GridAutoFlow::Column => (1usize, 0usize),
    };
    let mut resolved = Vec::with_capacity(items.len());
    let mut first = [1i32, 1];
    let mut last = [explicit[0] + 1, explicit[1] + 1];
    for &(child, slot) in items {
        let g = &engine.nodes[child].grid;
        let mut r = [Resolved::Auto(1); 2];
        for axis in 0..2 {
            r[axis] = resolve_lines(g.lines[axis][0], g.lines[axis][1], explicit[axis]);
            match r[axis] {
                Resolved::Definite(span) => {
                    first[axis] = first[axis].min(span.start);
                    last[axis] = last[axis].max(span.end);
                }
                Resolved::Auto(n) => {
                    if axis == fixed {
                        last[axis] = last[axis].max(first[axis] + n);
                    }
                }
            }
        }
        resolved.push((child, slot, r));
    }
    let limit = MAX_GRID_TRACKS as i32;
    if last[fixed] - first[fixed] > limit {
        return Err(LayoutError::LimitExceeded {
            node,
            limit: LayoutLimit::GridLines,
        });
    }
    let fixed_count = (last[fixed] - first[fixed]) as usize;
    // 흐름 축 기준 좌표계로 occupancy를 둡니다(columns = 고정 축 track 수).
    let swap = |area: [Span; 2]| -> [Span; 2] {
        if fixed == 0 {
            area
        } else {
            [area[1], area[0]]
        }
    };
    let offset_flow = [first[fixed], first[grows]];
    let mut occupancy = Occupancy {
        columns: fixed_count,
        rows: Vec::new(),
    };
    let mut placed: Vec<Option<GridItem>> = vec![None; resolved.len()];
    let mut work: usize = 0;
    let mut bump = |amount: usize| -> Result<(), LayoutError> {
        work = work.saturating_add(amount);
        if work > MAX_GRID_WORK {
            return Err(LayoutError::LimitExceeded {
                node,
                limit: LayoutLimit::GridWork,
            });
        }
        Ok(())
    };
    // 1. 두 축이 정해진 item.
    for (n, (child, slot, r)) in resolved.iter().enumerate() {
        if let [Resolved::Definite(c), Resolved::Definite(w)] = r {
            let area = [*c, *w];
            occupancy.mark(swap(area), offset_flow);
            bump(1)?;
            placed[n] = Some(GridItem {
                index: *child,
                slot: *slot,
                area,
            });
        }
    }
    // 2. 흐름 축(행)이 정해지고 고정 축(열)이 자동인 item.
    let mut row_cursor: std::collections::HashMap<i32, i32> = std::collections::HashMap::new();
    for (n, (child, slot, r)) in resolved.iter().enumerate() {
        if placed[n].is_some() {
            continue;
        }
        if let (Resolved::Definite(flow_span), Resolved::Auto(span)) = (r[grows], r[fixed]) {
            let mut column = *row_cursor.get(&flow_span.start).unwrap_or(&first[fixed]);
            loop {
                bump(1)?;
                if column + span > last[fixed] {
                    // 고정 축이 넘치면 암시 track을 만듭니다.
                    last[fixed] = column + span;
                    occupancy.columns = (last[fixed] - first[fixed]) as usize;
                }
                if occupancy.fits(
                    column,
                    flow_span.start,
                    [span, flow_span.end - flow_span.start],
                    offset_flow,
                ) {
                    break;
                }
                column += 1;
            }
            let fixed_span = Span {
                start: column,
                end: column + span,
            };
            row_cursor.insert(flow_span.start, fixed_span.end);
            let mut area = [fixed_span, flow_span];
            if fixed == 1 {
                area = [flow_span, fixed_span];
            }
            occupancy.mark(swap(area), offset_flow);
            placed[n] = Some(GridItem {
                index: *child,
                slot: *slot,
                area,
            });
        }
    }
    // 3. 나머지 item(커서 자동 배치).
    let mut cursor = [first[fixed], first[grows]];
    for (n, (child, slot, r)) in resolved.iter().enumerate() {
        if placed[n].is_some() {
            continue;
        }
        let flow_span = match r[grows] {
            Resolved::Auto(span) => span,
            Resolved::Definite(_) => unreachable!("2단계에서 배치"),
        };
        let fixed_area = match r[fixed] {
            Resolved::Definite(span) => {
                if span.start < cursor[0] {
                    cursor[1] += 1;
                }
                cursor[0] = span.start;
                loop {
                    bump(1)?;
                    if occupancy.fits(
                        span.start,
                        cursor[1],
                        [span.end - span.start, flow_span],
                        offset_flow,
                    ) {
                        break;
                    }
                    cursor[1] += 1;
                }
                span
            }
            Resolved::Auto(span) => loop {
                bump(1)?;
                if cursor[0] + span <= last[fixed]
                    && occupancy.fits(cursor[0], cursor[1], [span, flow_span], offset_flow)
                {
                    break Span {
                        start: cursor[0],
                        end: cursor[0] + span,
                    };
                }
                if cursor[0] + span > last[fixed] {
                    cursor[1] += 1;
                    cursor[0] = first[fixed];
                } else {
                    cursor[0] += 1;
                }
            },
        };
        let flow = Span {
            start: cursor[1],
            end: cursor[1] + flow_span,
        };
        let mut area = [fixed_area, flow];
        if fixed == 1 {
            area = [flow, fixed_area];
        }
        occupancy.mark(swap(area), offset_flow);
        placed[n] = Some(GridItem {
            index: *child,
            slot: *slot,
            area,
        });
    }
    let items: Vec<GridItem> = placed.into_iter().flatten().collect();
    for item in &items {
        for axis in 0..2 {
            first[axis] = first[axis].min(item.area[axis].start);
            last[axis] = last[axis].max(item.area[axis].end);
        }
    }
    let count = [(last[0] - first[0]) as usize, (last[1] - first[1]) as usize];
    if count[0] > MAX_GRID_TRACKS || count[1] > MAX_GRID_TRACKS {
        return Err(LayoutError::LimitExceeded {
            node,
            limit: LayoutLimit::GridLines,
        });
    }
    if count[0].saturating_mul(count[1]) > MAX_GRID_CELLS {
        return Err(LayoutError::LimitExceeded {
            node,
            limit: LayoutLimit::GridCells,
        });
    }
    Ok(Placed {
        items,
        first_line: first,
        count,
    })
}

/// 축의 track 정의 목록(암시 track 포함, index 0 = 가장 앞 line).
fn track_definitions(
    grid: &GridPrepared,
    axis: usize,
    first_line: i32,
    count: usize,
) -> Vec<TrackP> {
    let template = &grid.template[axis];
    let pattern = &grid.auto[axis];
    let explicit = template.len() as i32;
    (0..count)
        .map(|k| {
            let line = first_line + k as i32;
            if line >= 1 && line <= explicit {
                template[(line - 1) as usize]
            } else if line > explicit {
                pattern[((line - explicit - 1) as usize) % pattern.len()]
            } else {
                // 명시 grid 앞: pattern을 끝에서부터 반복.
                let back = (1 - line) as usize; // 1, 2, ...
                let len = pattern.len();
                pattern[(len - (back % len)) % len]
            }
        })
        .collect()
}

// --- track sizing(§11) ---

/// track sizing 상태.
#[derive(Debug, Clone, Copy)]
struct Track {
    def: TrackP,
    base: LayoutUnit,
    /// `None`은 무한대.
    limit: Option<LayoutUnit>,
    infinitely_growable: bool,
}

impl Track {
    fn flex(&self) -> u64 {
        match self.def.max {
            MaxTrackP::Fraction(f) => f,
            _ => 0,
        }
    }

    fn is_flexible(&self) -> bool {
        matches!(self.def.max, MaxTrackP::Fraction(_))
    }

    fn limit_or_base(&self) -> LayoutUnit {
        self.limit.unwrap_or(self.base)
    }
}

/// item 하나의 축 기여 값.
#[derive(Debug, Clone, Copy)]
struct Contributions {
    min_content: LayoutUnit,
    max_content: LayoutUnit,
    minimum: LayoutUnit,
}

/// 축의 track sizing 문맥.
struct AxisContext<'a> {
    axis: Axis,
    /// track 정의.
    defs: &'a [TrackP],
    /// 축의 가용 공간(definite = 내부 크기, MinContent·MaxContent = 제약).
    available: Avail,
    /// 백분율 track 기준.
    percent_base: Option<LayoutUnit>,
    gap: LayoutUnit,
    /// 행 sizing일 때 열 track 크기(item 너비 계산용).
    columns: Option<&'a [LayoutUnit]>,
    column_gap: LayoutUnit,
    /// container의 `justify-items`(행 sizing에서 item 너비 stretch 판정).
    justify_items: ItemAlignment,
    node: StableId,
}

/// item이 걸친 영역의 열 너비(행 sizing용).
fn area_width(
    columns: &[LayoutUnit],
    span: Span,
    gap: LayoutUnit,
) -> Result<LayoutUnit, LayoutUnitError> {
    let mut total = LayoutUnit::ZERO;
    for k in span.start..span.end {
        if k > span.start {
            total = total.checked_add(gap)?;
        }
        total = total.checked_add(columns[k as usize])?;
    }
    Ok(total)
}

/// item 정렬의 stretch 여부(크기 속성이 `auto`이고 `normal`·`stretch`, `normal`은 비율 없는 item만).
fn stretches(engine: &Engine<'_>, index: usize, axis: Axis, parent: ItemAlignment) -> bool {
    let prepared = &engine.nodes[index];
    let own = match axis {
        Axis::Horizontal => prepared.justify_self,
        Axis::Vertical => prepared.align_self,
    };
    let value = own.resolve(parent);
    let auto = prepared.size[axis.index()] == SizeP::Auto;
    // 비율이 있는 item의 `normal`: 다른 축 크기로 이 축이 정해지면 늘리지 않습니다. 가로는 높이가 정해졌을 때,
    // 세로는 항상(너비에서 비율로 정함) 늘리지 않습니다(Chromium 157 fixture `grid/aspect-ratio-item-normal`).
    let ratio_sizes_axis = prepared.ratio.is_some()
        && match axis {
            Axis::Horizontal => matches!(prepared.size[1], SizeP::Length(_)),
            Axis::Vertical => true,
        };
    match value {
        ItemAlignment::Normal => auto && !ratio_sizes_axis,
        ItemAlignment::Stretch => auto,
        _ => false,
    }
}

/// item 기여를 계산합니다(축, item, 영역).
fn contributions(
    engine: &mut Engine<'_>,
    context: &AxisContext<'_>,
    item: &GridItem,
    spans_auto_min: bool,
    spans_flexible: bool,
) -> Result<Contributions, LayoutError> {
    let index = item.index;
    let i = context.axis.index();
    let mut request = Request {
        known: [None, None],
        parent: [None, None],
        available: [Avail::MaxContent, Avail::MaxContent],
        content_only: [false, false],
    };
    if context.axis == Axis::Vertical {
        if let Some(columns) = context.columns {
            let width =
                area_width(columns, item.area[0], context.column_gap).map_err(at(context.node))?;
            request.available[0] = Avail::Definite(width);
            request.parent[0] = Some(width);
            if stretches(engine, index, Axis::Horizontal, context.justify_items) {
                let min = engine.min_size(index, Axis::Horizontal, request)?;
                let max = engine.max_size(index, Axis::Horizontal, request)?;
                request.known[0] = Some(clamp(width, min, max));
            }
        }
    }
    let measure = |engine: &mut Engine<'_>,
                   mode: Avail,
                   content_only: bool|
     -> Result<LayoutUnit, LayoutError> {
        let mut probe = request;
        probe.available[i] = mode;
        probe.content_only[i] = content_only;
        Ok(engine.layout(index, probe)?.size[i])
    };
    let min_content = measure(engine, Avail::MinContent, false)?;
    let max_content = measure(engine, Avail::MaxContent, false)?.max(min_content);
    let prepared = engine.nodes[index].clone();
    let minimum = match prepared.min[i] {
        SizeP::Auto => {
            if spans_auto_min && !spans_flexible {
                // 콘텐츠 기반 최소 크기: 지정 크기 suggestion과 콘텐츠 크기 suggestion 중 작은 값(max로 제한).
                let content = measure(engine, Avail::MinContent, true)?;
                let specified = engine.preferred(index, context.axis, request)?;
                let mut value = match specified {
                    Some(size) => size.min(content),
                    None => content,
                };
                if let Some(max) = engine.max_size(index, context.axis, request)? {
                    value = value.min(max);
                }
                value.max(
                    prepared
                        .padding
                        .sum(context.axis)
                        .map_err(at(prepared.id))?,
                )
            } else {
                LayoutUnit::ZERO
            }
        }
        _ => engine.min_size(index, context.axis, request)?,
    };
    Ok(Contributions {
        min_content,
        max_content,
        minimum,
    })
}

/// extra space를 대상 track에 나눕니다(§11.5.1). `limits`는 track별 상한(`None` = 무한), `weights`가 있으면 비례.
fn distribute_extra(
    space: LayoutUnit,
    affected: &[usize],
    sizes: &[LayoutUnit],
    limits: &[Option<LayoutUnit>],
    beyond: &[usize],
    weights: Option<&[u64]>,
) -> Vec<LayoutUnit> {
    let mut increase = vec![LayoutUnit::ZERO; sizes.len()];
    let mut remaining = space.raw().max(0) as i64;
    if affected.is_empty() || remaining == 0 {
        return increase;
    }
    let mut frozen = vec![false; sizes.len()];
    for _ in 0..=affected.len() {
        let open: Vec<usize> = affected.iter().copied().filter(|&t| !frozen[t]).collect();
        if open.is_empty() || remaining == 0 {
            break;
        }
        let shares: Vec<i64> = match weights {
            Some(w) => {
                let total: u64 = open.iter().map(|&t| w[t]).sum();
                if total == 0 {
                    even(remaining, open.len())
                } else {
                    proportional(remaining, &open.iter().map(|&t| w[t]).collect::<Vec<_>>())
                }
            }
            None => even(remaining, open.len()),
        };
        let mut any_frozen = false;
        let mut used = 0i64;
        for (k, &t) in open.iter().enumerate() {
            let current = i64::from(sizes[t].raw()) + i64::from(increase[t].raw());
            let room = match limits[t] {
                Some(limit) => (i64::from(limit.raw()) - current).max(0),
                None => i64::MAX,
            };
            let give = shares[k].min(room);
            if give < shares[k] {
                frozen[t] = true;
                any_frozen = true;
            }
            increase[t] = LayoutUnit::from_raw((i64::from(increase[t].raw()) + give) as i32);
            used += give;
        }
        remaining -= used;
        if !any_frozen {
            break;
        }
    }
    if remaining > 0 && !beyond.is_empty() {
        let shares = even(remaining, beyond.len());
        for (k, &t) in beyond.iter().enumerate() {
            increase[t] = LayoutUnit::from_raw((i64::from(increase[t].raw()) + shares[k]) as i32);
        }
    }
    increase
}

/// `total`을 `count`칸에 고르게(내림 몫, 잔여는 앞 칸부터 1씩).
fn even(total: i64, count: usize) -> Vec<i64> {
    let count_i = count as i64;
    let base = total / count_i;
    let mut rest = total - base * count_i;
    (0..count)
        .map(|_| {
            if rest > 0 {
                rest -= 1;
                base + 1
            } else {
                base
            }
        })
        .collect()
}

/// `total`을 가중치 비례로(내림 몫, 잔여는 앞 칸부터 1씩).
fn proportional(total: i64, weights: &[u64]) -> Vec<i64> {
    let sum: i128 = weights.iter().map(|w| i128::from(*w)).sum();
    let mut shares: Vec<i64> = weights
        .iter()
        .map(|w| (i128::from(total) * i128::from(*w) / sum) as i64)
        .collect();
    let mut rest = total - shares.iter().sum::<i64>();
    for (share, w) in shares.iter_mut().zip(weights) {
        if rest == 0 {
            break;
        }
        if *w > 0 {
            *share += 1;
            rest -= 1;
        }
    }
    shares
}

/// 한 축의 track 크기를 정합니다(§11.3~§11.8).
fn size_tracks(
    engine: &mut Engine<'_>,
    context: &AxisContext<'_>,
    items: &[GridItem],
    content_distribution: ContentAlignment,
) -> Result<Vec<LayoutUnit>, LayoutError> {
    let node = context.node;
    let err = at(node);
    let i = context.axis.index();
    let resolve = |length: LengthP| -> Result<Option<LayoutUnit>, LayoutError> {
        length.resolve(context.percent_base, node)
    };
    // 1. 초기화(§11.4).
    let mut tracks: Vec<Track> = Vec::with_capacity(context.defs.len());
    for def in context.defs {
        let mut def = *def;
        // indefinite 기준 백분율 track은 auto로 봅니다.
        if let MinTrackP::Length(length) = def.min {
            if resolve(length)?.is_none() {
                def.min = MinTrackP::Auto;
            }
        }
        if let MaxTrackP::Length(length) = def.max {
            if resolve(length)?.is_none() {
                def.max = MaxTrackP::Auto;
            }
        }
        let base = match def.min {
            MinTrackP::Length(length) => resolve(length)?.unwrap_or_default(),
            _ => LayoutUnit::ZERO,
        };
        let limit = match def.max {
            MaxTrackP::Length(length) => resolve(length)?,
            _ => None,
        };
        let limit = limit.map(|l| l.max(base));
        tracks.push(Track {
            def,
            base,
            limit,
            infinitely_growable: false,
        });
    }
    let count = tracks.len();
    // item 기여.
    struct Entry {
        span: Span,
        contributions: Contributions,
        flexible: bool,
    }
    let mut entries: Vec<Entry> = Vec::with_capacity(items.len());
    for item in items {
        let span = item.area[i];
        let range = span.start as usize..span.end as usize;
        let spans_flexible = tracks[range.clone()].iter().any(Track::is_flexible);
        let spans_auto_min = tracks[range.clone()]
            .iter()
            .any(|t| t.def.min == MinTrackP::Auto);
        let multi_flexible = spans_flexible && range.len() > 1;
        let contributions = contributions(engine, context, item, spans_auto_min, multi_flexible)?;
        entries.push(Entry {
            span,
            contributions,
            flexible: spans_flexible,
        });
    }
    let constraint = context.available;
    let gaps_in = |span: Span| -> Result<LayoutUnit, LayoutUnitError> {
        context
            .gap
            .checked_mul_int((span.end - span.start - 1).max(0))
    };
    let limited = |track: &Track, value: LayoutUnit, minimum: LayoutUnit| -> LayoutUnit {
        // max track sizing 함수가 고정이면(또는 fit-content 인자) 그 값으로 제한하고 최소 기여로 floor합니다.
        let cap = match track.def.max {
            MaxTrackP::Length(_) => track.limit,
            MaxTrackP::FitContent(length) => {
                length.resolve(context.percent_base, node).ok().flatten()
            }
            _ => None,
        };
        match cap {
            Some(cap) => value.min(cap).max(minimum),
            None => value,
        }
    };
    // 2. span 1, flexible이 아닌 track(§11.5 2).
    for entry in entries
        .iter()
        .filter(|e| e.span.end - e.span.start == 1 && !e.flexible)
    {
        let t = entry.span.start as usize;
        let c = entry.contributions;
        let track = &mut tracks[t];
        let min_value = match track.def.min {
            MinTrackP::MinContent => Some(c.min_content),
            MinTrackP::MaxContent => Some(c.max_content),
            MinTrackP::Auto => Some(match constraint {
                Avail::MinContent => limited(track, c.min_content, c.minimum),
                Avail::MaxContent => limited(track, c.max_content, c.minimum),
                Avail::Definite(_) => c.minimum,
            }),
            MinTrackP::Length(_) => None,
        };
        if let Some(value) = min_value {
            track.base = track.base.max(value);
        }
        let max_value = match track.def.max {
            MaxTrackP::MinContent => Some(c.min_content),
            MaxTrackP::MaxContent | MaxTrackP::Auto => Some(c.max_content),
            MaxTrackP::FitContent(length) => {
                let cap = length
                    .resolve(context.percent_base, node)?
                    .unwrap_or(c.max_content);
                Some(c.max_content.min(cap))
            }
            _ => None,
        };
        if let Some(value) = max_value {
            track.limit = Some(track.limit.map_or(value, |limit| limit.max(value)));
        }
    }
    for track in &mut tracks {
        if let Some(limit) = track.limit {
            if limit < track.base {
                track.limit = Some(track.base);
            }
        }
    }
    // 3. 여러 track에 걸친 item(flexible 없음), span 크기 순서(§11.5 3).
    let mut spans: Vec<i32> = entries
        .iter()
        .filter(|e| e.span.end - e.span.start > 1 && !e.flexible)
        .map(|e| e.span.end - e.span.start)
        .collect();
    spans.sort_unstable();
    spans.dedup();
    for span_len in spans {
        let group: Vec<&Entry> = entries
            .iter()
            .filter(|e| e.span.end - e.span.start == span_len && !e.flexible)
            .collect();
        // a~c: base size.
        type Pick = fn(&Track, Avail) -> bool;
        type Value = fn(&Contributions, Avail) -> LayoutUnit;
        type LimitValue = fn(&Contributions) -> LayoutUnit;
        let phases: [(Pick, Value, Pick); 3] = [
            (
                |t, _| {
                    matches!(
                        t.def.min,
                        MinTrackP::MinContent | MinTrackP::MaxContent | MinTrackP::Auto
                    )
                },
                |c, constraint| match constraint {
                    Avail::MinContent => c.min_content.max(c.minimum),
                    Avail::MaxContent => c.max_content.max(c.minimum),
                    Avail::Definite(_) => c.minimum,
                },
                |t, _| {
                    matches!(
                        t.def.max,
                        MaxTrackP::MinContent
                            | MaxTrackP::MaxContent
                            | MaxTrackP::Auto
                            | MaxTrackP::FitContent(_)
                    )
                },
            ),
            (
                |t, _| matches!(t.def.min, MinTrackP::MinContent | MinTrackP::MaxContent),
                |c, _| c.min_content,
                |t, _| {
                    matches!(
                        t.def.max,
                        MaxTrackP::MinContent
                            | MaxTrackP::MaxContent
                            | MaxTrackP::Auto
                            | MaxTrackP::FitContent(_)
                    )
                },
            ),
            (
                |t, constraint| {
                    t.def.min == MinTrackP::MaxContent
                        || (t.def.min == MinTrackP::Auto && constraint == Avail::MaxContent)
                },
                |c, _| c.max_content,
                |t, _| matches!(t.def.max, MaxTrackP::MaxContent | MaxTrackP::Auto),
            ),
        ];
        for (pick, value, beyond_pick) in phases {
            let mut planned = vec![LayoutUnit::ZERO; count];
            for entry in &group {
                let range = entry.span.start as usize..entry.span.end as usize;
                let affected: Vec<usize> = range
                    .clone()
                    .filter(|&t| pick(&tracks[t], constraint))
                    .collect();
                if affected.is_empty() {
                    continue;
                }
                let mut used = gaps_in(entry.span).map_err(&err)?;
                for t in range.clone() {
                    used = used.checked_add(tracks[t].base).map_err(&err)?;
                }
                let space = value(&entry.contributions, constraint)
                    .checked_sub(used)
                    .map_err(&err)?;
                if space <= LayoutUnit::ZERO {
                    continue;
                }
                let sizes: Vec<LayoutUnit> = tracks.iter().map(|t| t.base).collect();
                let limits: Vec<Option<LayoutUnit>> = tracks.iter().map(|t| t.limit).collect();
                let mut beyond: Vec<usize> = affected
                    .iter()
                    .copied()
                    .filter(|&t| beyond_pick(&tracks[t], constraint))
                    .collect();
                if beyond.is_empty() {
                    beyond = affected.clone();
                }
                let increase = distribute_extra(space, &affected, &sizes, &limits, &beyond, None);
                for t in 0..count {
                    planned[t] = planned[t].max(increase[t]);
                }
            }
            for t in 0..count {
                tracks[t].base = tracks[t].base.checked_add(planned[t]).map_err(&err)?;
                if let Some(limit) = tracks[t].limit {
                    if limit < tracks[t].base {
                        tracks[t].limit = Some(tracks[t].base);
                    }
                }
            }
        }
        // e~f: growth limit.
        let limit_phases: [(Pick, LimitValue); 2] = [
            (
                |t, _| {
                    matches!(
                        t.def.max,
                        MaxTrackP::MinContent
                            | MaxTrackP::MaxContent
                            | MaxTrackP::Auto
                            | MaxTrackP::FitContent(_)
                    )
                },
                |c| c.min_content,
            ),
            (
                |t, _| {
                    matches!(
                        t.def.max,
                        MaxTrackP::MaxContent | MaxTrackP::Auto | MaxTrackP::FitContent(_)
                    )
                },
                |c| c.max_content,
            ),
        ];
        for (pick, value) in limit_phases {
            let mut planned = vec![LayoutUnit::ZERO; count];
            let mut grew_from_infinite = vec![false; count];
            for entry in &group {
                let range = entry.span.start as usize..entry.span.end as usize;
                let affected: Vec<usize> = range
                    .clone()
                    .filter(|&t| pick(&tracks[t], constraint))
                    .collect();
                if affected.is_empty() {
                    continue;
                }
                let mut used = gaps_in(entry.span).map_err(&err)?;
                for t in range.clone() {
                    used = used.checked_add(tracks[t].limit_or_base()).map_err(&err)?;
                }
                let space = value(&entry.contributions)
                    .checked_sub(used)
                    .map_err(&err)?;
                if space <= LayoutUnit::ZERO {
                    continue;
                }
                let sizes: Vec<LayoutUnit> = tracks.iter().map(Track::limit_or_base).collect();
                let limits: Vec<Option<LayoutUnit>> = tracks
                    .iter()
                    .map(|t| match t.def.max {
                        MaxTrackP::FitContent(length) => {
                            length.resolve(context.percent_base, node).ok().flatten()
                        }
                        _ if t.limit.is_none() || t.infinitely_growable => None,
                        _ => t.limit,
                    })
                    .collect();
                let increase = distribute_extra(space, &affected, &sizes, &limits, &affected, None);
                for t in 0..count {
                    if increase[t] > planned[t] {
                        planned[t] = increase[t];
                    }
                    if tracks[t].limit.is_none() && increase[t] > LayoutUnit::ZERO {
                        grew_from_infinite[t] = true;
                    }
                }
            }
            for t in 0..count {
                if planned[t] > LayoutUnit::ZERO || grew_from_infinite[t] {
                    let current = tracks[t].limit_or_base();
                    tracks[t].limit = Some(current.checked_add(planned[t]).map_err(&err)?);
                    if grew_from_infinite[t] {
                        tracks[t].infinitely_growable = true;
                    }
                }
            }
        }
    }
    // 4. flexible track에 걸친 item(§11.5 4): fr 비율로 base size를 늘립니다.
    {
        let mut planned = vec![LayoutUnit::ZERO; count];
        for entry in entries.iter().filter(|e| e.flexible) {
            let range = entry.span.start as usize..entry.span.end as usize;
            let affected: Vec<usize> = range
                .clone()
                .filter(|&t| {
                    tracks[t].is_flexible()
                        && matches!(
                            tracks[t].def.min,
                            MinTrackP::MinContent | MinTrackP::MaxContent | MinTrackP::Auto
                        )
                })
                .collect();
            if affected.is_empty() {
                continue;
            }
            let value = match constraint {
                Avail::MinContent => entry
                    .contributions
                    .min_content
                    .max(entry.contributions.minimum),
                Avail::MaxContent => entry
                    .contributions
                    .max_content
                    .max(entry.contributions.minimum),
                Avail::Definite(_) => entry.contributions.minimum,
            };
            let mut used = gaps_in(entry.span).map_err(&err)?;
            for t in range.clone() {
                used = used.checked_add(tracks[t].base).map_err(&err)?;
            }
            let space = value.checked_sub(used).map_err(&err)?;
            if space <= LayoutUnit::ZERO {
                continue;
            }
            let sizes: Vec<LayoutUnit> = tracks.iter().map(|t| t.base).collect();
            let limits: Vec<Option<LayoutUnit>> = vec![None; count];
            let weights: Vec<u64> = tracks.iter().map(Track::flex).collect();
            let increase = distribute_extra(space, &affected, &sizes, &limits, &[], Some(&weights));
            for t in 0..count {
                planned[t] = planned[t].max(increase[t]);
            }
        }
        for t in 0..count {
            tracks[t].base = tracks[t].base.checked_add(planned[t]).map_err(&err)?;
            if let Some(limit) = tracks[t].limit {
                if limit < tracks[t].base {
                    tracks[t].limit = Some(tracks[t].base);
                }
            }
        }
    }
    // 5. 남은 무한 growth limit은 base size로.
    for track in &mut tracks {
        if track.limit.is_none() {
            track.limit = Some(track.base);
        }
    }
    let total_gaps = context
        .gap
        .checked_mul_int((count as i32 - 1).max(0))
        .map_err(&err)?;
    let sum_base = |tracks: &[Track]| -> Result<LayoutUnit, LayoutError> {
        let mut total = total_gaps;
        for t in tracks {
            total = total.checked_add(t.base).map_err(at(node))?;
        }
        Ok(total)
    };
    // 6. maximize(§11.6).
    match constraint {
        Avail::Definite(space) => {
            let free = space.checked_sub(sum_base(&tracks)?).map_err(&err)?;
            if free > LayoutUnit::ZERO {
                let affected: Vec<usize> =
                    (0..count).filter(|&t| !tracks[t].is_flexible()).collect();
                let sizes: Vec<LayoutUnit> = tracks.iter().map(|t| t.base).collect();
                let limits: Vec<Option<LayoutUnit>> = tracks.iter().map(|t| t.limit).collect();
                let increase = distribute_extra(free, &affected, &sizes, &limits, &[], None);
                for t in 0..count {
                    tracks[t].base = tracks[t].base.checked_add(increase[t]).map_err(&err)?;
                }
            }
        }
        Avail::MaxContent => {
            for track in &mut tracks {
                if !track.is_flexible() {
                    track.base = track.limit_or_base();
                }
            }
        }
        Avail::MinContent => {}
    }
    // 7. flexible track 확장(§11.7).
    if tracks.iter().any(Track::is_flexible) {
        let fraction = match constraint {
            Avail::MinContent => None,
            Avail::Definite(space) => Some(size_of_fr(
                &tracks,
                &(0..count).collect::<Vec<_>>(),
                space,
                context.gap,
                node,
            )?),
            Avail::MaxContent => {
                // indefinite: track base와 item max-content 기여로 정합니다.
                let mut fr = 0i64; // raw LayoutUnit per 1fr(×64 정밀)
                for track in tracks.iter().filter(|t| t.is_flexible()) {
                    let factor = track.flex();
                    let value = if factor > 64 {
                        i64::from(track.base.raw()) * 64 / factor as i64
                    } else {
                        i64::from(track.base.raw())
                    };
                    fr = fr.max(value);
                }
                for entry in entries.iter().filter(|e| e.flexible) {
                    let range: Vec<usize> =
                        (entry.span.start as usize..entry.span.end as usize).collect();
                    let value = size_of_fr(
                        &tracks,
                        &range,
                        entry.contributions.max_content,
                        context.gap,
                        node,
                    )?;
                    fr = fr.max(i64::from(value.raw()));
                }
                Some(LayoutUnit::from_raw(i32::try_from(fr).unwrap_or(i32::MAX)))
            }
        };
        if let Some(fraction) = fraction {
            for track in &mut tracks {
                if track.is_flexible() {
                    let size = fraction
                        .checked_mul_ratio(track.flex() as i64, 64)
                        .map_err(&err)?;
                    track.base = track.base.max(size);
                }
            }
        }
    }
    // 8. auto track stretch(§11.8).
    if let Avail::Definite(space) = constraint {
        if matches!(
            content_distribution,
            ContentAlignment::Normal | ContentAlignment::Stretch
        ) {
            stretch_auto_tracks(&mut tracks, space, total_gaps, node)?;
        }
    }
    Ok(tracks.iter().map(|t| t.base).collect())
}

/// `auto` 최댓값 track에 남는 공간을 고르게 더합니다.
fn stretch_auto_tracks(
    tracks: &mut [Track],
    space: LayoutUnit,
    gaps: LayoutUnit,
    node: StableId,
) -> Result<(), LayoutError> {
    let mut used = gaps;
    for t in tracks.iter() {
        used = used.checked_add(t.base).map_err(at(node))?;
    }
    let free = space.checked_sub(used).map_err(at(node))?;
    let auto: Vec<usize> = (0..tracks.len())
        .filter(|&t| tracks[t].def.max == MaxTrackP::Auto)
        .collect();
    if free > LayoutUnit::ZERO && !auto.is_empty() {
        let shares = distribute_equal(free, auto.len());
        for (&t, share) in auto.iter().zip(shares) {
            tracks[t].base = tracks[t].base.checked_add(share).map_err(at(node))?;
        }
    }
    Ok(())
}

/// fr 하나의 크기(§11.7.1): `range` track과 채울 공간 `space`.
fn size_of_fr(
    tracks: &[Track],
    range: &[usize],
    space: LayoutUnit,
    gap: LayoutUnit,
    node: StableId,
) -> Result<LayoutUnit, LayoutError> {
    let mut inflexible: Vec<bool> = range.iter().map(|&t| !tracks[t].is_flexible()).collect();
    let gaps = gap
        .checked_mul_int((range.len() as i32 - 1).max(0))
        .map_err(at(node))?;
    for _ in 0..=range.len() {
        let mut leftover = i64::from(space.raw()) - i64::from(gaps.raw());
        let mut factor_sum: u64 = 0;
        for (k, &t) in range.iter().enumerate() {
            if inflexible[k] {
                leftover -= i64::from(tracks[t].base.raw());
            } else {
                factor_sum += tracks[t].flex();
            }
        }
        let leftover = leftover.max(0);
        let divisor = factor_sum.max(64) as i64;
        // 1fr의 크기(raw subpixel): leftover / (factor 합, 1 미만이면 1).
        let hypothetical = leftover * 64 / divisor;
        let mut changed = false;
        for (k, &t) in range.iter().enumerate() {
            if inflexible[k] {
                continue;
            }
            let size = hypothetical * tracks[t].flex() as i64 / 64;
            if i64::from(tracks[t].base.raw()) > size {
                inflexible[k] = true;
                changed = true;
            }
        }
        if !changed {
            return Ok(LayoutUnit::from_raw(
                i32::try_from(hypothetical).unwrap_or(i32::MAX),
            ));
        }
    }
    Ok(LayoutUnit::ZERO)
}

/// track 묶음의 정렬 offset(앞 offset, track 사이 추가 간격).
fn content_offsets(
    alignment: ContentAlignment,
    free: LayoutUnit,
    count: usize,
) -> Result<(LayoutUnit, Vec<LayoutUnit>), LayoutUnitError> {
    let gaps = count.saturating_sub(1);
    let zero = vec![LayoutUnit::ZERO; gaps];
    let alignment = match alignment {
        ContentAlignment::SpaceBetween if free < LayoutUnit::ZERO || count <= 1 => {
            ContentAlignment::Start
        }
        ContentAlignment::SpaceAround | ContentAlignment::SpaceEvenly
            if free < LayoutUnit::ZERO =>
        {
            ContentAlignment::Start
        }
        ContentAlignment::SpaceAround | ContentAlignment::SpaceEvenly if count <= 1 => {
            ContentAlignment::Center
        }
        other => other,
    };
    Ok(match alignment {
        ContentAlignment::Normal
        | ContentAlignment::Stretch
        | ContentAlignment::Start
        | ContentAlignment::FlexStart => (LayoutUnit::ZERO, zero),
        ContentAlignment::End | ContentAlignment::FlexEnd => (free, zero),
        ContentAlignment::Center => (free.checked_div_round(2)?, zero),
        ContentAlignment::SpaceBetween => (LayoutUnit::ZERO, distribute_equal(free, gaps)),
        ContentAlignment::SpaceAround => {
            let shares = distribute_equal(free, count);
            let first = shares[0].checked_div_round(2)?;
            let mut between = Vec::with_capacity(gaps);
            let mut previous = shares[0].checked_sub(first)?;
            for share in shares.iter().skip(1) {
                let half = share.checked_div_round(2)?;
                between.push(previous.checked_add(half)?);
                previous = share.checked_sub(half)?;
            }
            (first, between)
        }
        ContentAlignment::SpaceEvenly => {
            let shares = distribute_equal(free, count + 1);
            (shares[0], shares[1..count].to_vec())
        }
    })
}

/// track 시작 좌표(content box 기준).
fn track_positions(
    sizes: &[LayoutUnit],
    gap: LayoutUnit,
    start: LayoutUnit,
    between: &[LayoutUnit],
) -> Result<Vec<LayoutUnit>, LayoutUnitError> {
    let mut positions = Vec::with_capacity(sizes.len() + 1);
    let mut cursor = start;
    for (k, size) in sizes.iter().enumerate() {
        if k > 0 {
            cursor = cursor.checked_add(gap)?;
            if let Some(extra) = between.get(k - 1) {
                cursor = cursor.checked_add(*extra)?;
            }
        }
        positions.push(cursor);
        cursor = cursor.checked_add(*size)?;
    }
    positions.push(cursor);
    Ok(positions)
}

/// `display: grid` node를 배치합니다.
pub(crate) fn layout(
    engine: &mut Engine<'_>,
    index: usize,
    request: Request,
    mut sizing: Sizing,
) -> Result<NodeLayout, LayoutError> {
    let prepared = engine.nodes[index].clone();
    let id = prepared.id;
    let err = at(id);
    let pad = prepared.padding;
    let pad_sum = [
        pad.sum(Axis::Horizontal).map_err(&err)?,
        pad.sum(Axis::Vertical).map_err(&err)?,
    ];
    let children = engine.children(index);
    let in_flow: Vec<(usize, usize)> = children
        .iter()
        .copied()
        .enumerate()
        .filter(|&(_, child)| engine.nodes[child].in_flow())
        .map(|(slot, child)| (child, slot))
        .collect();
    let grid = prepared.grid.clone();
    let placed = place(engine, id, &grid, &in_flow)?;
    let mut items: Vec<GridItem> = placed.items.clone();
    for item in &mut items {
        for axis in 0..2 {
            let shift = placed.first_line[axis];
            item.area[axis] = Span {
                start: item.area[axis].start - shift,
                end: item.area[axis].end - shift,
            };
        }
    }
    let column_defs = track_definitions(&grid, 0, placed.first_line[0], placed.count[0]);
    let row_defs = track_definitions(&grid, 1, placed.first_line[1], placed.count[1]);
    let column_context = |available: Avail, percent_base: Option<LayoutUnit>| AxisContext {
        axis: Axis::Horizontal,
        defs: &column_defs,
        available,
        percent_base,
        gap: prepared.gap[0],
        columns: None,
        column_gap: prepared.gap[0],
        justify_items: prepared.justify_items,
        node: id,
    };
    // 1. 너비.
    let width = match sizing.size[0] {
        Some(width) => width,
        None => {
            let mut sum = |mode: Avail| -> Result<LayoutUnit, LayoutError> {
                let sizes = size_tracks(
                    engine,
                    &column_context(mode, None),
                    &items,
                    ContentAlignment::Start,
                )?;
                let mut total = prepared.gap[0]
                    .checked_mul_int((sizes.len() as i32 - 1).max(0))
                    .map_err(at(id))?;
                for size in sizes {
                    total = total.checked_add(size).map_err(at(id))?;
                }
                total.checked_add(pad_sum[0]).map_err(at(id))
            };
            let min = sum(Avail::MinContent)?;
            let max = sum(Avail::MaxContent)?.max(min);
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
    let columns = size_tracks(
        engine,
        &column_context(Avail::Definite(inner_width), Some(inner_width)),
        &items,
        prepared.justify_content,
    )?;
    // 2. 높이.
    let inner_height = sizing.size[1]
        .map(|h| sub_floor_zero(h, pad_sum[1]))
        .transpose()
        .map_err(&err)?;
    let row_context = |available: Avail, percent_base: Option<LayoutUnit>| AxisContext {
        axis: Axis::Vertical,
        defs: &row_defs,
        available,
        percent_base,
        gap: prepared.gap[1],
        columns: Some(&columns),
        column_gap: prepared.gap[0],
        justify_items: prepared.justify_items,
        node: id,
    };
    let mut rows = size_tracks(
        engine,
        &row_context(
            inner_height.map_or(Avail::MaxContent, Avail::Definite),
            inner_height,
        ),
        &items,
        prepared.align_content,
    )?;
    let row_gaps = prepared.gap[1]
        .checked_mul_int((rows.len() as i32 - 1).max(0))
        .map_err(&err)?;
    let mut rows_total = row_gaps;
    for row in &rows {
        rows_total = rows_total.checked_add(*row).map_err(&err)?;
    }
    let height = sizing.finish(
        Axis::Vertical,
        rows_total.checked_add(pad_sum[1]).map_err(&err)?,
    );
    let inner_height_final = sub_floor_zero(height, pad_sum[1]).map_err(&err)?;
    if inner_height.is_none() {
        // 콘텐츠로 정한 높이가 min-height 등으로 늘어나면 정렬 단계에서 auto 행을 늘립니다.
        if matches!(
            prepared.align_content,
            ContentAlignment::Normal | ContentAlignment::Stretch
        ) && inner_height_final > rows_total
        {
            let mut tracks: Vec<Track> = row_defs
                .iter()
                .zip(&rows)
                .map(|(def, size)| Track {
                    def: *def,
                    base: *size,
                    limit: Some(*size),
                    infinitely_growable: false,
                })
                .collect();
            stretch_auto_tracks(&mut tracks, inner_height_final, row_gaps, id)?;
            rows = tracks.iter().map(|t| t.base).collect();
        }
    }
    // 3. track 정렬.
    let total = |sizes: &[LayoutUnit], gap: LayoutUnit| -> Result<LayoutUnit, LayoutError> {
        let mut t = gap
            .checked_mul_int((sizes.len() as i32 - 1).max(0))
            .map_err(at(id))?;
        for s in sizes {
            t = t.checked_add(*s).map_err(at(id))?;
        }
        Ok(t)
    };
    let free_x = inner_width
        .checked_sub(total(&columns, prepared.gap[0])?)
        .map_err(&err)?;
    let free_y = inner_height_final
        .checked_sub(total(&rows, prepared.gap[1])?)
        .map_err(&err)?;
    let (start_x, between_x) =
        content_offsets(prepared.justify_content, free_x, columns.len()).map_err(&err)?;
    let (start_y, between_y) =
        content_offsets(prepared.align_content, free_y, rows.len()).map_err(&err)?;
    let x_positions =
        track_positions(&columns, prepared.gap[0], start_x, &between_x).map_err(&err)?;
    let y_positions = track_positions(&rows, prepared.gap[1], start_y, &between_y).map_err(&err)?;
    let area_size = |positions: &[LayoutUnit], span: Span, sizes: &[LayoutUnit]| -> LayoutUnit {
        let start = positions[span.start as usize];
        let last = (span.end - 1) as usize;
        positions[last]
            .checked_add(sizes[last])
            .and_then(|end| end.checked_sub(start))
            .unwrap_or_default()
    };
    // 4. item 배치.
    let mut slots: Vec<Option<Placement>> = vec![None; children.len()];
    let mut diagnostics = Vec::new();
    let mut first_baseline: Option<(i32, i32, LayoutUnit)> = None;
    let content_based_width = sizing.content_based[0];
    for item in &items {
        let area_x = x_positions[item.area[0].start as usize];
        let area_y = y_positions[item.area[1].start as usize];
        let area_w = area_size(&x_positions, item.area[0], &columns);
        let area_h = area_size(&y_positions, item.area[1], &rows);
        let mut request = Request {
            known: [None, None],
            parent: [Some(area_w), Some(area_h)],
            available: [Avail::Definite(area_w), Avail::Definite(area_h)],
            content_only: [false, false],
        };
        for (axis, area, parent_alignment) in [
            (Axis::Horizontal, area_w, prepared.justify_items),
            (Axis::Vertical, area_h, prepared.align_items),
        ] {
            if stretches(engine, item.index, axis, parent_alignment) {
                let min = engine.min_size(item.index, axis, request)?;
                let max = engine.max_size(item.index, axis, request)?;
                request.known[axis.index()] = Some(clamp(area, min, max));
            }
        }
        let layout = engine.layout(item.index, request)?;
        let mut origin = [area_x, area_y];
        for (axis, area, parent_alignment) in [
            (Axis::Horizontal, area_w, prepared.justify_items),
            (Axis::Vertical, area_h, prepared.align_items),
        ] {
            let i = axis.index();
            let own = match axis {
                Axis::Horizontal => engine.nodes[item.index].justify_self,
                Axis::Vertical => engine.nodes[item.index].align_self,
            };
            let free = area.checked_sub(layout.size[i]).map_err(&err)?;
            let offset = match own.resolve(parent_alignment) {
                ItemAlignment::End | ItemAlignment::FlexEnd => free,
                ItemAlignment::Center => free.checked_div_round(2).map_err(&err)?,
                _ => LayoutUnit::ZERO,
            };
            origin[i] = origin[i].checked_add(offset).map_err(&err)?;
        }
        let x = pad.left.checked_add(origin[0]).map_err(&err)?;
        let y = pad.top.checked_add(origin[1]).map_err(&err)?;
        let baseline = layout.baseline.unwrap_or(layout.size[1]);
        let key = (item.area[1].start, item.area[0].start);
        if first_baseline.is_none_or(|(row, column, _)| (key.0, key.1) < (row, column)) {
            first_baseline = Some((key.0, key.1, y.checked_add(baseline).map_err(&err)?));
        }
        if content_based_width
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
        let [dx, dy] = relative_offset(engine, item.index, [Some(area_w), Some(area_h)])?;
        slots[item.slot] = Some(Placement {
            child: engine.nodes[item.index].id,
            x: x.checked_add(dx).map_err(&err)?,
            y: y.checked_add(dy).map_err(&err)?,
            request,
        });
    }
    // 5. absolute 자식.
    for (slot, &child) in children.iter().enumerate() {
        if engine.nodes[child].position != Position::Absolute {
            continue;
        }
        let align =
            |own: crate::style::SelfAlignment, parent: ItemAlignment| match own.resolve(parent) {
                ItemAlignment::End | ItemAlignment::FlexEnd => StaticAlign::End,
                ItemAlignment::Center => StaticAlign::Center,
                _ => StaticAlign::Start,
            };
        let rect = StaticRect {
            start: [LayoutUnit::ZERO, LayoutUnit::ZERO],
            size: [width, height],
            align: [
                align(engine.nodes[child].justify_self, prepared.justify_items),
                align(engine.nodes[child].align_self, prepared.align_items),
            ],
        };
        slots[slot] = Some(place_absolute(engine, child, [width, height], rect)?);
    }
    Ok(NodeLayout {
        size: [width, height],
        baseline: first_baseline.map(|(_, _, b)| b),
        placements: slots.into_iter().flatten().collect(),
        diagnostics,
    })
}
