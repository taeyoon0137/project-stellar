//! Grid 배치 (X.4.1~X.4.3).
//!
//! # track 타입
//!
//! - `Fixed(len)`: 고정 크기(비음수).
//! - `Fraction(fr)`: 남는 공간을 fr 비례로 나눔(비음수, 1/64 단위로 반올림).
//! - `MinContent`·`MaxContent`: 그 track에 놓인 item의 최소·최대 콘텐츠 기여 크기.
//! - `MinMax(min, max)`: `min` 이상 `max` 이하. `min`은 `Fraction`일 수 없고, 둘 다 `Fixed`면
//!   `min ≤ max`여야 합니다.
//!
//! [`GridLayout`]은 열(column)·행(row) track 목록, 열·행 gap, 자동 배치가 명시 행을 넘을 때 쓰는
//! 암시 행 크기를 가집니다. 열은 하나 이상이어야 합니다.
//!
//! # item 배치와 gap (X.4.3)
//!
//! - item은 [`GridItem`]으로 열·행 위치를 정합니다. 축마다 `Span { start, span }`(시작 line +
//!   칸 수) 또는 `Lines { start, end }`(시작·끝 line)이며 line 번호는 1부터입니다. `Auto`는 자동
//!   배치(X.4.4)입니다.
//! - **invalid span은 오류**: span 0, line 0, 끝 ≤ 시작, 열 범위가 명시 열을 넘음 →
//!   [`LayoutError::InvalidGrid`]. 행 범위가 명시 행을 넘으면 `implicit_row` 크기의 암시 행을
//!   만듭니다.
//! - **겹침 허용**: 명시 위치 item끼리 영역이 겹쳐도 오류가 아니며 그대로 겹쳐 놓입니다. 그리는
//!   순서(z-order)는 visual order입니다.
//! - gap은 이웃 track 사이에만 들어갑니다. 여러 칸 영역은 걸친 track 크기 합 + 사이 gap입니다.
//! - track sizing은 열 먼저(item 기여 = containing block indefinite 측정 크기), 그 다음 행(열 영역
//!   너비가 정해진 측정 크기)입니다. item은 영역 시작에 놓이고 `Fill` 축은 영역을 채우며
//!   `Percentage`는 영역 크기 기준입니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{FiniteF32, Length, Scalar};

use crate::codec::{Reader, Writer};
use crate::engine::{at, ChildConstraint, Engine};
use crate::error::LayoutError;
use crate::geometry::{LayoutRect, LayoutSize};
use crate::grid_tracks::{size_tracks, track_offsets, Breadth, Track, TrackItem};
use crate::input::{length_unit, ContainerLayout};
use crate::property::LayoutProperty;
use crate::rounding::{LayoutUnit, LayoutUnitError};

/// `MinMax`의 한쪽 경계 (X.4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrackBreadth {
    /// 고정 크기(비음수).
    Fixed(Length),
    /// fr(최댓값 쪽에서만 허용).
    Fraction(Scalar),
    /// 최소 콘텐츠 기여.
    MinContent,
    /// 최대 콘텐츠 기여.
    MaxContent,
}

/// grid track 크기 (X.4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrackSize {
    /// 고정 크기(비음수).
    Fixed(Length),
    /// 남는 공간의 fr 비례 몫(비음수).
    Fraction(Scalar),
    /// 최소 콘텐츠 기여.
    MinContent,
    /// 최대 콘텐츠 기여.
    MaxContent,
    /// `min` 이상 `max` 이하.
    MinMax(TrackBreadth, TrackBreadth),
}

/// Grid container 설정 (X.4.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GridLayout {
    /// 열 track(하나 이상).
    pub columns: Vec<TrackSize>,
    /// 명시 행 track.
    pub rows: Vec<TrackSize>,
    /// 열 사이 간격(비음수).
    pub column_gap: Length,
    /// 행 사이 간격(비음수).
    pub row_gap: Length,
    /// 명시 행을 넘는 암시 행의 크기.
    pub implicit_row: TrackSize,
}

impl GridLayout {
    /// 열·행 track으로 Grid를 만듭니다(gap 0, 암시 행 `MaxContent`).
    pub fn new(columns: Vec<TrackSize>, rows: Vec<TrackSize>) -> GridLayout {
        GridLayout {
            columns,
            rows,
            column_gap: Length::from_raw(FiniteF32::ZERO),
            row_gap: Length::from_raw(FiniteF32::ZERO),
            implicit_row: TrackSize::MaxContent,
        }
    }
}

/// grid 설정 오류 사유(사람이 읽는 설명).
fn invalid(node: StableId, reason: &'static str) -> LayoutError {
    LayoutError::InvalidGrid { node, reason }
}

/// fr 값을 1/64 정수로 변환합니다(비음수).
pub(crate) fn fraction_units(node: StableId, value: Scalar) -> Result<u64, LayoutError> {
    if value.get() < 0.0 {
        return Err(LayoutError::NegativeValue {
            node,
            property: LayoutProperty::ContainerLayout,
        });
    }
    let unit = LayoutUnit::from_px(value.raw()).map_err(|_| LayoutError::OutOfRange {
        node,
        property: LayoutProperty::ContainerLayout,
    })?;
    u64::try_from(unit.raw()).map_err(|_| invalid(node, "fr 값이 음수입니다"))
}

fn validate_breadth(node: StableId, breadth: TrackBreadth) -> Result<(), LayoutError> {
    match breadth {
        TrackBreadth::Fixed(length) => {
            length_unit(node, LayoutProperty::ContainerLayout, length, false)?;
        }
        TrackBreadth::Fraction(value) => {
            fraction_units(node, value)?;
        }
        TrackBreadth::MinContent | TrackBreadth::MaxContent => {}
    }
    Ok(())
}

/// track 하나를 검증합니다.
pub(crate) fn validate_track(node: StableId, track: TrackSize) -> Result<(), LayoutError> {
    match track {
        TrackSize::Fixed(length) => validate_breadth(node, TrackBreadth::Fixed(length)),
        TrackSize::Fraction(value) => validate_breadth(node, TrackBreadth::Fraction(value)),
        TrackSize::MinContent | TrackSize::MaxContent => Ok(()),
        TrackSize::MinMax(min, max) => {
            validate_breadth(node, min)?;
            validate_breadth(node, max)?;
            if matches!(min, TrackBreadth::Fraction(_)) {
                return Err(invalid(node, "minmax 최솟값은 fr일 수 없습니다"));
            }
            if let (TrackBreadth::Fixed(lo), TrackBreadth::Fixed(hi)) = (min, max) {
                if lo.get() > hi.get() {
                    return Err(invalid(node, "minmax 최솟값이 최댓값보다 큽니다"));
                }
            }
            Ok(())
        }
    }
}

/// Grid 설정을 검증합니다.
pub(crate) fn validate_grid(node: StableId, grid: &GridLayout) -> Result<(), LayoutError> {
    if grid.columns.is_empty() {
        return Err(invalid(node, "열 track이 하나 이상 필요합니다"));
    }
    for track in grid.columns.iter().chain(&grid.rows) {
        validate_track(node, *track)?;
    }
    validate_track(node, grid.implicit_row)?;
    length_unit(
        node,
        LayoutProperty::ContainerLayout,
        grid.column_gap,
        false,
    )?;
    length_unit(node, LayoutProperty::ContainerLayout, grid.row_gap, false)?;
    Ok(())
}

// --- canonical bytes ---

const BREADTH_FIXED: u8 = 0;
const BREADTH_FRACTION: u8 = 1;
const BREADTH_MIN_CONTENT: u8 = 2;
const BREADTH_MAX_CONTENT: u8 = 3;
const TRACK_MINMAX: u8 = 4;

fn encode_breadth(w: &mut Writer, breadth: TrackBreadth) {
    match breadth {
        TrackBreadth::Fixed(length) => {
            w.u8(BREADTH_FIXED);
            w.length(length);
        }
        TrackBreadth::Fraction(value) => {
            w.u8(BREADTH_FRACTION);
            w.finite(value.raw());
        }
        TrackBreadth::MinContent => w.u8(BREADTH_MIN_CONTENT),
        TrackBreadth::MaxContent => w.u8(BREADTH_MAX_CONTENT),
    }
}

fn decode_breadth_tag(r: &mut Reader<'_>, tag: u8) -> Result<TrackBreadth, LayoutError> {
    match tag {
        BREADTH_FIXED => Ok(TrackBreadth::Fixed(r.length("track")?)),
        BREADTH_FRACTION => Ok(TrackBreadth::Fraction(Scalar::from_raw(r.finite("track")?))),
        BREADTH_MIN_CONTENT => Ok(TrackBreadth::MinContent),
        BREADTH_MAX_CONTENT => Ok(TrackBreadth::MaxContent),
        _ => Err(LayoutError::Malformed {
            reason: "알 수 없는 track tag",
        }),
    }
}

/// track canonical bytes. 단일 track은 breadth tag(0~3), `MinMax`는 tag 4 + 두 breadth입니다.
pub(crate) fn encode_track(w: &mut Writer, track: TrackSize) {
    match track {
        TrackSize::Fixed(length) => encode_breadth(w, TrackBreadth::Fixed(length)),
        TrackSize::Fraction(value) => encode_breadth(w, TrackBreadth::Fraction(value)),
        TrackSize::MinContent => encode_breadth(w, TrackBreadth::MinContent),
        TrackSize::MaxContent => encode_breadth(w, TrackBreadth::MaxContent),
        TrackSize::MinMax(min, max) => {
            w.u8(TRACK_MINMAX);
            encode_breadth(w, min);
            encode_breadth(w, max);
        }
    }
}

/// track canonical bytes를 읽습니다.
pub(crate) fn decode_track(r: &mut Reader<'_>) -> Result<TrackSize, LayoutError> {
    let tag = r.u8()?;
    if tag == TRACK_MINMAX {
        let min_tag = r.u8()?;
        let min = decode_breadth_tag(r, min_tag)?;
        let max_tag = r.u8()?;
        let max = decode_breadth_tag(r, max_tag)?;
        return Ok(TrackSize::MinMax(min, max));
    }
    Ok(match decode_breadth_tag(r, tag)? {
        TrackBreadth::Fixed(length) => TrackSize::Fixed(length),
        TrackBreadth::Fraction(value) => TrackSize::Fraction(value),
        TrackBreadth::MinContent => TrackSize::MinContent,
        TrackBreadth::MaxContent => TrackSize::MaxContent,
    })
}

/// Grid 설정 canonical bytes.
pub(crate) fn encode_grid(w: &mut Writer, grid: &GridLayout) {
    w.len(grid.columns.len());
    for track in &grid.columns {
        encode_track(w, *track);
    }
    w.len(grid.rows.len());
    for track in &grid.rows {
        encode_track(w, *track);
    }
    w.length(grid.column_gap);
    w.length(grid.row_gap);
    encode_track(w, grid.implicit_row);
}

/// Grid 설정 canonical bytes를 읽습니다.
pub(crate) fn decode_grid(r: &mut Reader<'_>) -> Result<GridLayout, LayoutError> {
    let column_count = r.len(1)?;
    let mut columns = Vec::with_capacity(column_count);
    for _ in 0..column_count {
        columns.push(decode_track(r)?);
    }
    let row_count = r.len(1)?;
    let mut rows = Vec::with_capacity(row_count);
    for _ in 0..row_count {
        rows.push(decode_track(r)?);
    }
    let column_gap = r.length("grid column gap")?;
    let row_gap = r.length("grid row gap")?;
    let implicit_row = decode_track(r)?;
    Ok(GridLayout {
        columns,
        rows,
        column_gap,
        row_gap,
        implicit_row,
    })
}

// --- grid item 배치 (X.4.3) ---

/// grid item의 한 축 위치 (X.4.3). line 번호는 1부터입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GridPlacement {
    /// 자동 배치(X.4.4), `span`칸.
    Auto {
        /// 걸칠 track 수(1 이상).
        span: u16,
    },
    /// 시작 line과 span.
    Span {
        /// 시작 line(1 이상).
        start: u16,
        /// 걸칠 track 수(1 이상).
        span: u16,
    },
    /// 시작·끝 line(끝 > 시작).
    Lines {
        /// 시작 line(1 이상).
        start: u16,
        /// 끝 line(시작보다 큼).
        end: u16,
    },
}

impl Default for GridPlacement {
    fn default() -> GridPlacement {
        GridPlacement::Auto { span: 1 }
    }
}

impl GridPlacement {
    /// 걸칠 track 수.
    pub const fn span(&self) -> u16 {
        match *self {
            GridPlacement::Auto { span } | GridPlacement::Span { span, .. } => span,
            GridPlacement::Lines { start, end } => end.saturating_sub(start),
        }
    }

    /// 명시 시작 track index(0부터). 자동이면 `None`.
    pub const fn start_index(&self) -> Option<usize> {
        match *self {
            GridPlacement::Auto { .. } => None,
            GridPlacement::Span { start, .. } | GridPlacement::Lines { start, .. } => {
                Some((start as usize).saturating_sub(1))
            }
        }
    }
}

/// Grid 부모 안에서 item의 열·행 위치 (X.4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct GridItem {
    /// 열 위치.
    pub column: GridPlacement,
    /// 행 위치.
    pub row: GridPlacement,
}

impl GridItem {
    /// 시작 line·span으로 만든 명시 위치.
    pub const fn at(column: u16, row: u16, column_span: u16, row_span: u16) -> GridItem {
        GridItem {
            column: GridPlacement::Span {
                start: column,
                span: column_span,
            },
            row: GridPlacement::Span {
                start: row,
                span: row_span,
            },
        }
    }
}

fn validate_placement(node: StableId, placement: GridPlacement) -> Result<(), LayoutError> {
    match placement {
        GridPlacement::Auto { span } | GridPlacement::Span { span, .. } if span == 0 => {
            Err(invalid(node, "span은 1 이상이어야 합니다"))
        }
        GridPlacement::Span { start: 0, .. } | GridPlacement::Lines { start: 0, .. } => {
            Err(invalid(node, "grid line 번호는 1부터입니다"))
        }
        GridPlacement::Lines { start, end } if end <= start => {
            Err(invalid(node, "끝 line은 시작 line보다 커야 합니다"))
        }
        _ => Ok(()),
    }
}

/// Grid 부모의 item 위치를 검증합니다. 열 범위는 명시 열 안이어야 합니다(행은 암시 행 허용).
pub(crate) fn validate_item(
    node: StableId,
    item: &GridItem,
    grid: &GridLayout,
) -> Result<(), LayoutError> {
    validate_placement(node, item.column)?;
    validate_placement(node, item.row)?;
    let columns = grid.columns.len();
    let span = usize::from(item.column.span());
    let end = item.column.start_index().map_or(span, |start| start + span);
    if end > columns {
        return Err(invalid(node, "열 범위가 명시 열을 벗어납니다"));
    }
    let row_end = item.row.start_index().unwrap_or(0) + usize::from(item.row.span());
    if row_end > usize::from(u16::MAX) {
        return Err(invalid(node, "행 범위가 너무 큽니다"));
    }
    Ok(())
}

const PLACEMENT_AUTO: u8 = 0;
const PLACEMENT_SPAN: u8 = 1;
const PLACEMENT_LINES: u8 = 2;

fn encode_placement(w: &mut Writer, placement: GridPlacement) {
    match placement {
        GridPlacement::Auto { span } => {
            w.u8(PLACEMENT_AUTO);
            w.u16(span);
        }
        GridPlacement::Span { start, span } => {
            w.u8(PLACEMENT_SPAN);
            w.u16(start);
            w.u16(span);
        }
        GridPlacement::Lines { start, end } => {
            w.u8(PLACEMENT_LINES);
            w.u16(start);
            w.u16(end);
        }
    }
}

fn decode_placement(r: &mut Reader<'_>) -> Result<GridPlacement, LayoutError> {
    match r.u8()? {
        PLACEMENT_AUTO => Ok(GridPlacement::Auto { span: r.u16()? }),
        PLACEMENT_SPAN => Ok(GridPlacement::Span {
            start: r.u16()?,
            span: r.u16()?,
        }),
        PLACEMENT_LINES => Ok(GridPlacement::Lines {
            start: r.u16()?,
            end: r.u16()?,
        }),
        _ => Err(LayoutError::Malformed {
            reason: "알 수 없는 grid placement tag",
        }),
    }
}

/// item 위치 canonical bytes(열, 행).
pub(crate) fn encode_item(w: &mut Writer, item: &GridItem) {
    encode_placement(w, item.column);
    encode_placement(w, item.row);
}

/// item 위치 canonical bytes를 읽습니다.
pub(crate) fn decode_item(r: &mut Reader<'_>) -> Result<GridItem, LayoutError> {
    Ok(GridItem {
        column: decode_placement(r)?,
        row: decode_placement(r)?,
    })
}

/// track을 계산용 값으로 변환합니다.
fn prepare_track(node: StableId, track: TrackSize) -> Result<Track, LayoutError> {
    let breadth = |b: TrackBreadth| -> Result<Breadth, LayoutError> {
        Ok(match b {
            TrackBreadth::Fixed(length) => Breadth::Fixed(length_unit(
                node,
                LayoutProperty::ContainerLayout,
                length,
                false,
            )?),
            TrackBreadth::Fraction(value) => Breadth::Fraction(fraction_units(node, value)?),
            TrackBreadth::MinContent => Breadth::MinContent,
            TrackBreadth::MaxContent => Breadth::MaxContent,
        })
    };
    Ok(match track {
        TrackSize::Fixed(length) => {
            let value = breadth(TrackBreadth::Fixed(length))?;
            Track {
                min: value,
                max: value,
            }
        }
        TrackSize::Fraction(value) => Track {
            min: Breadth::Fixed(LayoutUnit::ZERO),
            max: breadth(TrackBreadth::Fraction(value))?,
        },
        TrackSize::MinContent => Track {
            min: Breadth::MinContent,
            max: Breadth::MinContent,
        },
        TrackSize::MaxContent => Track {
            min: Breadth::MaxContent,
            max: Breadth::MaxContent,
        },
        TrackSize::MinMax(min, max) => Track {
            min: breadth(min)?,
            max: breadth(max)?,
        },
    })
}

/// 배치가 정해진 item 하나(track index는 0부터).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlacedItem {
    pub(crate) child: usize,
    pub(crate) column: usize,
    pub(crate) column_span: usize,
    pub(crate) row: usize,
    pub(crate) row_span: usize,
}

/// Grid 계산 결과(track 크기·offset과 item 위치).
struct GridSolution {
    column_offsets: Vec<LayoutUnit>,
    column_sizes: Vec<LayoutUnit>,
    row_offsets: Vec<LayoutUnit>,
    row_sizes: Vec<LayoutUnit>,
    extent: [LayoutUnit; 2],
    items: Vec<PlacedItem>,
}

/// 영역 크기(걸친 track 합 + 사이 gap).
fn area_extent(
    sizes: &[LayoutUnit],
    start: usize,
    span: usize,
    gap: LayoutUnit,
) -> Result<LayoutUnit, LayoutUnitError> {
    let mut total = LayoutUnit::ZERO;
    for (i, size) in sizes[start..start + span].iter().enumerate() {
        if i > 0 {
            total = total.checked_add(gap)?;
        }
        total = total.checked_add(*size)?;
    }
    Ok(total)
}

/// item을 track에 배치합니다. 명시 위치 item은 그대로 놓고(겹침 허용), 자동 위치 item은 X.4.4
/// 자동 배치가 처리합니다.
fn place_items(
    engine: &Engine<'_>,
    id: StableId,
    children: &[usize],
) -> Result<Vec<PlacedItem>, LayoutError> {
    let mut placed = Vec::with_capacity(children.len());
    for &child in children {
        let item = engine.node(child).grid;
        match (item.column.start_index(), item.row.start_index()) {
            (Some(column), Some(row)) => placed.push(PlacedItem {
                child,
                column,
                column_span: usize::from(item.column.span()),
                row,
                row_span: usize::from(item.row.span()),
            }),
            _ => {
                return Err(LayoutError::UnsupportedFeature {
                    node: id,
                    feature: "grid 자동 배치(X.4.4)",
                })
            }
        }
    }
    Ok(placed)
}

/// track sizing과 item 배치를 계산합니다. `inner`는 content 크기(indefinite면 `None`).
fn solve(
    engine: &mut Engine<'_>,
    index: usize,
    inner: [Option<LayoutUnit>; 2],
) -> Result<GridSolution, LayoutError> {
    let id = engine.nodes[index].id;
    let grid = match &engine.node(index).container {
        ContainerLayout::Grid(grid) => grid,
        _ => {
            return Err(LayoutError::InvalidBox {
                node: id,
                reason: "Grid container가 아닙니다",
            })
        }
    };
    let column_gap = length_unit(id, LayoutProperty::ContainerLayout, grid.column_gap, false)?;
    let row_gap = length_unit(id, LayoutProperty::ContainerLayout, grid.row_gap, false)?;
    let children = engine.children(index);
    let items = place_items(engine, id, &children)?;

    let columns = grid
        .columns
        .iter()
        .map(|t| prepare_track(id, *t))
        .collect::<Result<Vec<_>, _>>()?;
    let row_count = items
        .iter()
        .map(|item| item.row + item.row_span)
        .max()
        .unwrap_or(0)
        .max(grid.rows.len());
    let implicit = prepare_track(id, grid.implicit_row)?;
    let mut rows = grid
        .rows
        .iter()
        .map(|t| prepare_track(id, *t))
        .collect::<Result<Vec<_>, _>>()?;
    rows.resize(row_count, implicit);

    // 열: item 콘텐츠 기여(containing block indefinite).
    let mut column_items = Vec::with_capacity(items.len());
    for item in &items {
        let size = engine.resolve_size(item.child, ChildConstraint::UNCONSTRAINED)?;
        column_items.push(TrackItem {
            start: item.column,
            span: item.column_span,
            contribution: size.width(),
        });
    }
    let column_sizes =
        size_tracks(&columns, column_gap, inner[0], &column_items).map_err(at(id))?;

    // 행: 열 영역 너비가 정해진 뒤의 높이 기여.
    let mut row_items = Vec::with_capacity(items.len());
    for item in &items {
        let width = area_extent(&column_sizes, item.column, item.column_span, column_gap)
            .map_err(at(id))?;
        let constraint = ChildConstraint {
            cb: [Some(width), None],
            fill: [Some(width), None],
            forced: [None, None],
        };
        let size = engine.resolve_size(item.child, constraint)?;
        row_items.push(TrackItem {
            start: item.row,
            span: item.row_span,
            contribution: size.height(),
        });
    }
    let row_sizes = size_tracks(&rows, row_gap, inner[1], &row_items).map_err(at(id))?;

    let (column_offsets, column_extent) =
        track_offsets(&column_sizes, column_gap).map_err(at(id))?;
    let (row_offsets, row_extent) = track_offsets(&row_sizes, row_gap).map_err(at(id))?;
    Ok(GridSolution {
        column_offsets,
        column_sizes,
        row_offsets,
        row_sizes,
        extent: [column_extent, row_extent],
        items,
    })
}

/// Grid container의 content extent(Hug 측정).
pub(crate) fn measure(
    engine: &mut Engine<'_>,
    index: usize,
    inner: [Option<LayoutUnit>; 2],
) -> Result<[LayoutUnit; 2], LayoutError> {
    Ok(solve(engine, index, inner)?.extent)
}

/// Grid container의 item 배치. 각 item은 자기 영역(area) 시작에 놓이고, `Fill` 축은 영역을 채우며
/// `Percentage`는 영역 크기 기준입니다. 결과는 visual order입니다.
pub(crate) fn place(
    engine: &mut Engine<'_>,
    index: usize,
    inner: LayoutSize,
) -> Result<Vec<(usize, LayoutRect)>, LayoutError> {
    let id = engine.nodes[index].id;
    let padding = engine.nodes[index].padding;
    let solution = solve(engine, index, [Some(inner.width()), Some(inner.height())])?;
    let (column_gap, row_gap) = match &engine.node(index).container {
        ContainerLayout::Grid(grid) => (
            length_unit(id, LayoutProperty::ContainerLayout, grid.column_gap, false)?,
            length_unit(id, LayoutProperty::ContainerLayout, grid.row_gap, false)?,
        ),
        _ => (LayoutUnit::ZERO, LayoutUnit::ZERO),
    };
    let mut placements = Vec::with_capacity(solution.items.len());
    for item in &solution.items {
        let width = area_extent(
            &solution.column_sizes,
            item.column,
            item.column_span,
            column_gap,
        )
        .map_err(at(id))?;
        let height =
            area_extent(&solution.row_sizes, item.row, item.row_span, row_gap).map_err(at(id))?;
        let constraint = ChildConstraint {
            cb: [Some(width), Some(height)],
            fill: [Some(width), Some(height)],
            forced: [None, None],
        };
        let size = engine.resolve_size(item.child, constraint)?;
        let x = padding
            .left
            .checked_add(solution.column_offsets[item.column])
            .map_err(at(id))?;
        let y = padding
            .top
            .checked_add(solution.row_offsets[item.row])
            .map_err(at(id))?;
        placements.push((item.child, LayoutRect::from_origin_size(x, y, size)));
    }
    Ok(placements)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{ContainerLayout, LayoutInput, LayoutNode};
    use crate::test_support::{id, len};

    fn fr(value: f32) -> Scalar {
        Scalar::new(value).unwrap()
    }

    fn grid_input(grid: GridLayout) -> LayoutInput {
        LayoutInput::new(
            id(1),
            vec![LayoutNode::new(id(1)).with_container(ContainerLayout::Grid(grid))],
        )
    }

    fn all_track_kinds() -> Vec<TrackSize> {
        vec![
            TrackSize::Fixed(len(40.0)),
            TrackSize::Fraction(fr(1.5)),
            TrackSize::MinContent,
            TrackSize::MaxContent,
            TrackSize::MinMax(
                TrackBreadth::Fixed(len(10.0)),
                TrackBreadth::Fraction(fr(2.0)),
            ),
            TrackSize::MinMax(TrackBreadth::MinContent, TrackBreadth::MaxContent),
            TrackSize::MinMax(TrackBreadth::Fixed(len(5.0)), TrackBreadth::Fixed(len(5.0))),
        ]
    }

    #[test]
    fn valid_tracks_pass_and_round_trip() {
        let grid = GridLayout {
            column_gap: len(4.0),
            row_gap: len(2.5),
            implicit_row: TrackSize::Fixed(len(12.0)),
            ..GridLayout::new(all_track_kinds(), all_track_kinds())
        };
        let input = grid_input(grid);
        assert_eq!(input.validate(), Ok(()));
        assert_eq!(LayoutInput::decode(&input.encode()).unwrap(), input);
        // track bytes golden: MinMax(Fixed 10, Fraction 2) = [4, 0, 10f32, 1, 2f32].
        let mut w = Writer::new();
        encode_track(
            &mut w,
            TrackSize::MinMax(
                TrackBreadth::Fixed(len(10.0)),
                TrackBreadth::Fraction(fr(2.0)),
            ),
        );
        assert_eq!(
            w.finish(),
            vec![4, 0, 0x00, 0x00, 0x20, 0x41, 1, 0x00, 0x00, 0x00, 0x40]
        );
        assert_eq!(
            GridLayout::new(vec![], vec![]).implicit_row,
            TrackSize::MaxContent
        );
    }

    #[test]
    fn invalid_tracks_are_rejected() {
        let one = |track: TrackSize| grid_input(GridLayout::new(vec![track], vec![]));
        // 음수 고정·fr.
        assert!(matches!(
            one(TrackSize::Fixed(len(-1.0))).validate(),
            Err(LayoutError::NegativeValue { .. })
        ));
        assert!(matches!(
            one(TrackSize::Fraction(fr(-0.5))).validate(),
            Err(LayoutError::NegativeValue { .. })
        ));
        // minmax(fr, …) 거부, minmax(20, 10) 거부.
        assert!(matches!(
            one(TrackSize::MinMax(
                TrackBreadth::Fraction(fr(1.0)),
                TrackBreadth::MaxContent
            ))
            .validate(),
            Err(LayoutError::InvalidGrid { .. })
        ));
        assert!(matches!(
            one(TrackSize::MinMax(
                TrackBreadth::Fixed(len(20.0)),
                TrackBreadth::Fixed(len(10.0))
            ))
            .validate(),
            Err(LayoutError::InvalidGrid { .. })
        ));
        // 범위 초과, 열 없음, 음수 gap.
        assert!(matches!(
            one(TrackSize::Fixed(len(4.0e7))).validate(),
            Err(LayoutError::OutOfRange { .. })
        ));
        assert!(matches!(
            grid_input(GridLayout::new(vec![], vec![TrackSize::MinContent])).validate(),
            Err(LayoutError::InvalidGrid { .. })
        ));
        let negative_gap = GridLayout {
            row_gap: len(-3.0),
            ..GridLayout::new(vec![TrackSize::MinContent], vec![])
        };
        assert!(matches!(
            grid_input(negative_gap).validate(),
            Err(LayoutError::NegativeValue { .. })
        ));
        // 잘못된 암시 행도 검증.
        let bad_implicit = GridLayout {
            implicit_row: TrackSize::Fraction(fr(-1.0)),
            ..GridLayout::new(vec![TrackSize::MinContent], vec![])
        };
        assert!(grid_input(bad_implicit).validate().is_err());
        // 알 수 없는 tag 디코딩 거부.
        let mut r = Reader::new(&[9]);
        assert!(decode_track(&mut r).is_err());
    }

    use crate::engine::compute_layout;
    use crate::input::{IntrinsicContent, LayoutContent, SizingMode};
    use crate::output::LayoutResult;
    use crate::test_support::{key, px};
    use stellar_core_types::value::{Insets, Percentage};

    fn fixed_track(value: f32) -> TrackSize {
        TrackSize::Fixed(len(value))
    }

    /// Grid root(sizing, 설정)와 자식 (StableId, 위치, 가로·세로 sizing).
    fn grid_result(
        width: SizingMode,
        height: SizingMode,
        grid: GridLayout,
        items: Vec<(u64, GridItem, SizingMode, SizingMode)>,
    ) -> Result<LayoutResult, LayoutError> {
        let mut root = LayoutNode::new(id(1))
            .with_sizing(width, height)
            .with_padding(Insets::new(4.0, 4.0, 4.0, 4.0).unwrap())
            .with_container(ContainerLayout::Grid(grid));
        let mut nodes = Vec::new();
        for (i, (n, item, w, h)) in items.into_iter().enumerate() {
            root = root.with_child(id(n), key(i as u32 + 1));
            nodes.push(LayoutNode::new(id(n)).with_grid(item).with_sizing(w, h));
        }
        nodes.insert(0, root);
        compute_layout(&LayoutInput::new(id(1), nodes))
    }

    fn rect_px(result: &LayoutResult, n: u64) -> (f64, f64, f64, f64) {
        let r = result.get(id(n)).unwrap().local_rect();
        (
            r.x().to_px_f64(),
            r.y().to_px_f64(),
            r.width().to_px_f64(),
            r.height().to_px_f64(),
        )
    }

    fn three_by_two() -> GridLayout {
        GridLayout {
            column_gap: len(10.0),
            row_gap: len(5.0),
            ..GridLayout::new(
                vec![fixed_track(50.0), fixed_track(60.0), fixed_track(70.0)],
                vec![fixed_track(20.0), fixed_track(30.0)],
            )
        }
    }

    #[test]
    fn start_end_span_gap_and_overlap_fixture() {
        let fill = SizingMode::Fill;
        let lines = GridItem {
            column: GridPlacement::Lines { start: 3, end: 4 },
            row: GridPlacement::Lines { start: 2, end: 3 },
        };
        let result = grid_result(
            SizingMode::HugContent,
            SizingMode::HugContent,
            three_by_two(),
            vec![
                (2, GridItem::at(1, 1, 1, 1), fill, fill),
                // 열 2~3, 행 1~2 span: 영역 60 + 10 + 70, 20 + 5 + 30.
                (3, GridItem::at(2, 1, 2, 2), fill, fill),
                // Lines 열 3..4, 행 2..3, 고정 10×10 — 위 item과 겹침(허용).
                (
                    4,
                    lines,
                    SizingMode::Fixed(len(10.0)),
                    SizingMode::Fixed(len(10.0)),
                ),
            ],
        )
        .unwrap();
        assert_eq!(rect_px(&result, 2), (4.0, 4.0, 50.0, 20.0));
        assert_eq!(rect_px(&result, 3), (64.0, 4.0, 140.0, 55.0));
        assert_eq!(rect_px(&result, 4), (134.0, 29.0, 10.0, 10.0));
        // Hug: track 합 + gap + padding = (180 + 20 + 8, 50 + 5 + 8).
        assert_eq!(rect_px(&result, 1), (0.0, 0.0, 208.0, 63.0));
        // 겹친 item도 visual order로 모두 결과에 있음.
        let order: Vec<_> = result.boxes().iter().map(|b| b.id()).collect();
        assert_eq!(order, vec![id(1), id(2), id(3), id(4)]);
    }

    #[test]
    fn invalid_spans_are_errors() {
        let fixed = SizingMode::Fixed(len(1.0));
        let cases = [
            GridItem {
                column: GridPlacement::Span { start: 1, span: 0 },
                row: GridPlacement::Span { start: 1, span: 1 },
            },
            GridItem {
                column: GridPlacement::Span { start: 0, span: 1 },
                row: GridPlacement::Span { start: 1, span: 1 },
            },
            GridItem {
                column: GridPlacement::Lines { start: 2, end: 2 },
                row: GridPlacement::Span { start: 1, span: 1 },
            },
            GridItem {
                column: GridPlacement::Span { start: 1, span: 1 },
                row: GridPlacement::Lines { start: 3, end: 1 },
            },
            // 열 3에서 span 2 → 명시 열 3개를 넘음.
            GridItem::at(3, 1, 2, 1),
            // 자동 배치 span 4 > 열 3.
            GridItem {
                column: GridPlacement::Auto { span: 4 },
                row: GridPlacement::Auto { span: 1 },
            },
            GridItem {
                column: GridPlacement::Auto { span: 1 },
                row: GridPlacement::Auto { span: 0 },
            },
        ];
        for item in cases {
            let result = grid_result(fixed, fixed, three_by_two(), vec![(2, item, fixed, fixed)]);
            assert!(
                matches!(result, Err(LayoutError::InvalidGrid { node, .. }) if node == id(2)),
                "{item:?} → {result:?}"
            );
        }
    }

    #[test]
    fn implicit_rows_content_tracks_and_percentage_items() {
        // 명시 행 2개, item 행 4 → 암시 행 3·4는 implicit_row 15.
        let grid = GridLayout {
            implicit_row: fixed_track(15.0),
            ..three_by_two()
        };
        let fill = SizingMode::Fill;
        let result = grid_result(
            SizingMode::HugContent,
            SizingMode::HugContent,
            grid,
            vec![(2, GridItem::at(1, 4, 1, 1), fill, fill)],
        )
        .unwrap();
        // y = 4 + 20 + 5 + 30 + 5 + 15 + 5.
        assert_eq!(rect_px(&result, 2), (4.0, 84.0, 50.0, 15.0));
        assert_eq!(
            rect_px(&result, 1).3,
            4.0 + 20.0 + 5.0 + 30.0 + 5.0 + 15.0 + 5.0 + 15.0 + 4.0
        );

        // [MaxContent, 1fr] 너비 208(content 200): intrinsic 70 item이 열 1을 정하고 fr이 나머지.
        let content = GridLayout::new(
            vec![
                TrackSize::MaxContent,
                TrackSize::Fraction(Scalar::new(1.0).unwrap()),
            ],
            vec![TrackSize::MaxContent],
        );
        let mut root = LayoutNode::new(id(1))
            .with_sizing(SizingMode::Fixed(len(208.0)), SizingMode::HugContent)
            .with_padding(Insets::new(4.0, 4.0, 4.0, 4.0).unwrap())
            .with_container(ContainerLayout::Grid(content))
            .with_child(id(2), key(1))
            .with_child(id(3), key(2));
        root.children.reverse();
        let text = LayoutNode::new(id(2))
            .with_grid(GridItem::at(1, 1, 1, 1))
            .with_sizing(SizingMode::HugContent, SizingMode::HugContent)
            .with_content(LayoutContent::Intrinsic(IntrinsicContent {
                width: len(70.0),
                height: len(12.0),
                baseline: None,
            }));
        // 열 2 영역의 50% 너비, 높이 Fill.
        let half = LayoutNode::new(id(3))
            .with_grid(GridItem::at(2, 1, 1, 1))
            .with_sizing(
                SizingMode::Percentage(Percentage::new(50.0).unwrap()),
                SizingMode::Fill,
            );
        let result = compute_layout(&LayoutInput::new(id(1), vec![root, text, half])).unwrap();
        assert_eq!(rect_px(&result, 2), (4.0, 4.0, 70.0, 12.0));
        assert_eq!(rect_px(&result, 3), (74.0, 4.0, 65.0, 12.0));
        assert_eq!(result.get(id(1)).unwrap().local_rect().height(), px(20));
    }

    #[test]
    fn grid_item_round_trips() {
        let item = GridItem {
            column: GridPlacement::Lines { start: 2, end: 4 },
            row: GridPlacement::Auto { span: 3 },
        };
        let mut w = Writer::new();
        encode_item(&mut w, &item);
        let bytes = w.finish();
        assert_eq!(bytes, vec![2, 2, 0, 4, 0, 0, 3, 0]);
        assert_eq!(decode_item(&mut Reader::new(&bytes)).unwrap(), item);
        let node = LayoutNode::new(id(1)).with_grid(item);
        let input = LayoutInput::new(id(1), vec![node]);
        assert_eq!(LayoutInput::decode(&input.encode()).unwrap(), input);
        assert_eq!(GridItem::default().column, GridPlacement::Auto { span: 1 });
    }
}
