//! Grid 배치 (X.4.1~X.4.4).
//!
//! # track 타입
//!
//! - `Fixed(len)`: 고정 크기(비음수).
//! - `Fraction(fr)`: 남는 공간을 fr 비례로 나눔(비음수, 1/64 단위로 반올림). 0이 아닌데 0으로
//!   반올림되는 값(1/128 미만)은 [`LayoutError::BelowResolution`]입니다.
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
//!   배치입니다.
//!
//! # 자동 배치 (X.4.4)
//!
//! row-major sparse 알고리즘을 visual order(OrderKey)로 적용합니다: 명시 item → 행만 명시된 item →
//! cursor 자동 배치 순입니다. 자동 item은 점유된 칸을 피하고, 열이 모자라면 다음 행으로, 행은
//! `implicit_row` 크기 암시 행으로 늘어납니다. 삽입 순서와 StableId는 결과에 영향을 주지 않습니다.
//! - **invalid span은 오류**: span 0, line 0, 끝 ≤ 시작, 열 범위가 명시 열을 넘음 →
//!   [`LayoutError::InvalidGrid`]. 행 범위가 명시 행을 넘으면 `implicit_row` 크기의 암시 행을
//!   만듭니다.
//! - **겹침 허용**: 명시 위치 item끼리 영역이 겹쳐도 오류가 아니며 그대로 겹쳐 놓입니다. 그리는
//!   순서(z-order)는 visual order입니다.
//! - gap은 이웃 track 사이에만 들어갑니다. 여러 칸 영역은 걸친 track 크기 합 + 사이 gap입니다.
//! - track sizing은 열 먼저(item 기여 = containing block indefinite 측정 크기), 그 다음 행(열 영역
//!   너비가 정해진 측정 크기)입니다. item은 영역 시작에 놓이고 `Fill` 축은 영역을 채우며
//!   `Percentage`는 영역 크기 기준입니다.
//!
//! # 크기·작업량 상한
//!
//! 신뢰할 수 없는 입력이 계산 시간·메모리를 무한히 늘리지 못하도록 결정적 상한을 둡니다. 넘으면
//! [`LayoutError::LimitExceeded`]입니다.
//!
//! - **line 범위** ([`LayoutLimit::GridLines`]): line 번호는 `u16`이라 한 축의 track은 최대
//!   [`MAX_GRID_TRACKS`]개(line 1~65535)입니다. 명시 track 수, 명시 item의 끝 line, 자동 배치가 만드는
//!   암시 행의 끝 line이 모두 이 범위 안이어야 합니다. 자동 item이 범위 안에 들어갈 자리가 없으면
//!   "공간 없음"이 아니라 이 상한 오류입니다.
//! - **cell 수** ([`LayoutLimit::GridCells`]): 열 × 행(명시·암시 포함)은 [`MAX_GRID_CELLS`] 이하입니다.
//!   명시 track·명시 item은 검증([`LayoutInput::validate`](crate::input::LayoutInput::validate))에서,
//!   자동 배치가 늘린 행은 계산 중에 검사합니다.
//! - **작업량** ([`LayoutLimit::GridWork`]): 한 번의 grid 계산에서 cell 검사·점유·track sizing 반복은
//!   [`MAX_GRID_WORK`] 단위까지입니다. 자동 배치는 점유 cell을 만나면 그 cell을 포함하는 시작 위치를
//!   건너뛰므로(결과는 순차 탐색과 같음) 일반 입력은 상한에 한참 못 미칩니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{FiniteF32, Length, Scalar};

use crate::codec::{Reader, Writer};
use crate::engine::{at, ChildConstraint, Engine};
use crate::error::{LayoutError, LayoutLimit};
use crate::geometry::{LayoutRect, LayoutSize};
use crate::grid_tracks::{
    size_tracks, track_offsets, Breadth, Track, TrackError, TrackItem, WorkBudget, WorkExceeded,
};
use crate::input::{length_unit, ContainerLayout};
use crate::property::LayoutProperty;
use crate::rounding::{LayoutUnit, LayoutUnitError};
use crate::sizing::quantize_weight;

/// 한 축의 최대 track 수(line 1~`u16::MAX`).
pub const MAX_GRID_TRACKS: usize = u16::MAX as usize - 1;

/// grid 하나의 최대 cell 수(열 × 명시·암시 행).
pub const MAX_GRID_CELLS: usize = 1 << 20;

/// grid 계산 한 번의 최대 작업량(cell 검사·점유·track sizing 반복 단위).
pub const MAX_GRID_WORK: u64 = 1 << 24;

/// 상한 오류.
fn limit(node: StableId, limit: LayoutLimit) -> LayoutError {
    LayoutError::LimitExceeded { node, limit }
}

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

/// fr 값을 1/64 정수로 변환합니다(비음수, 0이 아닌데 0으로 반올림되면 오류 — flex factor와 같은 규칙).
pub(crate) fn fraction_units(node: StableId, value: Scalar) -> Result<u64, LayoutError> {
    quantize_weight(node, LayoutProperty::ContainerLayout, value)
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
    if grid.columns.len() > MAX_GRID_TRACKS || grid.rows.len() > MAX_GRID_TRACKS {
        return Err(limit(node, LayoutLimit::GridLines));
    }
    if grid.columns.len().saturating_mul(grid.rows.len().max(1)) > MAX_GRID_CELLS {
        return Err(limit(node, LayoutLimit::GridCells));
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
///
/// 행 끝 line은 `u16` 범위 안([`MAX_GRID_TRACKS`] 이하 track)이어야 하고, 명시 행 위치가 만드는
/// cell 수(열 × 끝 행)는 [`MAX_GRID_CELLS`] 이하여야 합니다. 자동 행은 끝 행을 span으로 봅니다.
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
    if row_end > MAX_GRID_TRACKS {
        return Err(limit(node, LayoutLimit::GridLines));
    }
    if columns.saturating_mul(row_end) > MAX_GRID_CELLS {
        return Err(limit(node, LayoutLimit::GridCells));
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
    /// 열·행 track(암시 행 포함).
    tracks: [Vec<Track>; 2],
    extent: [LayoutUnit; 2],
    items: Vec<PlacedItem>,
}

/// item 영역 한 축의 크기가 무엇으로 정해지는지 (X.6.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct AreaDependence {
    /// 걸친 track 중 콘텐츠 기반 경계(MinContent·MaxContent)를 가진 track이 있습니다.
    pub(crate) content: bool,
    /// 걸친 track 중 fr track이 있습니다(grid 축이 indefinite면 item 기여로 정해짐).
    pub(crate) flexible: bool,
}

/// track 목록의 (콘텐츠 기반, fr) 누적 개수. 영역 의존성을 O(1)로 계산합니다.
fn dependence_prefix(tracks: &[Track]) -> Vec<(usize, usize)> {
    let mut prefix = Vec::with_capacity(tracks.len() + 1);
    let mut acc = (0usize, 0usize);
    prefix.push(acc);
    for track in tracks {
        if track.min.is_content() || track.max.is_content() {
            acc.0 += 1;
        }
        if track.is_flexible() {
            acc.1 += 1;
        }
        prefix.push(acc);
    }
    prefix
}

fn area_dependence(prefix: &[(usize, usize)], start: usize, span: usize) -> AreaDependence {
    let (lo, hi) = (prefix[start], prefix[start + span]);
    AreaDependence {
        content: hi.0 > lo.0,
        flexible: hi.1 > lo.1,
    }
}

/// 영역 크기(걸친 track 합 + 사이 gap). `offsets`는 [`track_offsets`] 결과라 O(1)입니다.
fn span_extent(
    offsets: &[LayoutUnit],
    sizes: &[LayoutUnit],
    start: usize,
    span: usize,
) -> Result<LayoutUnit, LayoutUnitError> {
    let last = start + span - 1;
    offsets[last]
        .checked_add(sizes[last])?
        .checked_sub(offsets[start])
}

/// 점유된 grid cell. 행 수는 필요에 따라 늘어나며 열 × 행은 [`MAX_GRID_CELLS`] 이하입니다.
struct Occupancy {
    columns: usize,
    cells: Vec<bool>,
}

impl Occupancy {
    fn new(columns: usize) -> Occupancy {
        Occupancy {
            columns,
            cells: Vec::new(),
        }
    }

    /// 영역 안의 점유 cell 하나(가장 아래 행, 그 행의 가장 오른쪽 열)를 찾습니다. 없으면 `None`.
    /// 저장된 행 밖은 비어 있습니다. 검사한 cell 수만큼 예산을 소모합니다.
    fn conflict(
        &self,
        area: (usize, usize, usize, usize),
        budget: &mut WorkBudget,
    ) -> Result<Option<(usize, usize)>, WorkExceeded> {
        let (row, column, row_span, column_span) = area;
        let stored_rows = self.cells.len() / self.columns;
        let last = (row + row_span).min(stored_rows);
        for r in (row..last).rev() {
            budget.charge(column_span)?;
            for c in (column..column + column_span).rev() {
                if self.cells[r * self.columns + c] {
                    return Ok(Some((r, c)));
                }
            }
        }
        Ok(None)
    }

    /// item 영역을 점유합니다. 늘어난 행으로 cell 수 상한을 넘으면 오류입니다.
    fn occupy(
        &mut self,
        id: StableId,
        item: &PlacedItem,
        budget: &mut WorkBudget,
    ) -> Result<(), LayoutError> {
        let needed = (item.row + item.row_span).saturating_mul(self.columns);
        if needed > MAX_GRID_CELLS {
            return Err(limit(id, LayoutLimit::GridCells));
        }
        budget
            .charge(item.row_span.saturating_mul(item.column_span))
            .map_err(|_| limit(id, LayoutLimit::GridWork))?;
        if self.cells.len() < needed {
            self.cells.resize(needed, false);
        }
        for r in item.row..item.row + item.row_span {
            for c in item.column..item.column + item.column_span {
                self.cells[r * self.columns + c] = true;
            }
        }
        Ok(())
    }
}

/// item을 track에 배치합니다 (X.4.3, X.4.4).
///
/// 1. 열·행이 모두 명시된 item을 그대로 놓습니다(겹침 허용).
/// 2. 행만 명시된 item을 visual order로 그 행의 가장 앞 빈 열에 놓습니다.
/// 3. 나머지(열만 명시 또는 완전 자동)를 visual order로 row-major cursor를 따라 놓습니다
///    (sparse): 완전 자동은 cursor부터 열을 늘리며 맞는 빈 칸을 찾고 줄 끝이면 다음 행 첫 열로,
///    열만 명시된 item은 그 열이 cursor 열보다 앞이면 다음 행부터 맞는 행을 찾습니다. 놓은 뒤
///    cursor는 그 item의 끝 열입니다.
///
/// 자동 배치 item은 서로·명시 item과 겹치지 않습니다. 순서는 StableId·삽입 순서가 아니라 visual
/// order(OrderKey)입니다. 결과는 visual order입니다.
///
/// 탐색 중 점유 cell `(r, c)`를 만나면 그 cell을 포함하는 시작 위치(열 탐색은 열 `c`까지, 행 탐색은
/// 행 `r`까지)를 건너뜁니다. 건너뛴 위치는 모두 점유 cell과 겹치므로 결과는 한 칸씩 탐색한 것과
/// 같습니다. 자동 item의 끝 line이 `u16` 범위를 넘어야만 놓을 수 있으면
/// [`LayoutLimit::GridLines`]입니다.
fn place_items(
    engine: &Engine<'_>,
    id: StableId,
    children: &[usize],
    columns: usize,
    budget: &mut WorkBudget,
) -> Result<Vec<PlacedItem>, LayoutError> {
    let no_space = || invalid(id, "grid 자동 배치 공간이 없습니다");
    let work = |_: WorkExceeded| limit(id, LayoutLimit::GridWork);
    let mut placed: Vec<Option<PlacedItem>> = vec![None; children.len()];
    let mut occupancy = Occupancy::new(columns);
    let spans = |child: usize| {
        let item = engine.node(child).grid;
        (
            item,
            usize::from(item.column.span()),
            usize::from(item.row.span()),
        )
    };
    // 1) 명시 item.
    for (slot, &child) in children.iter().enumerate() {
        let (item, column_span, row_span) = spans(child);
        if let (Some(column), Some(row)) = (item.column.start_index(), item.row.start_index()) {
            let p = PlacedItem {
                child,
                column,
                column_span,
                row,
                row_span,
            };
            occupancy.occupy(id, &p, budget)?;
            placed[slot] = Some(p);
        }
    }
    // 2) 행 고정 item.
    for (slot, &child) in children.iter().enumerate() {
        let (item, column_span, row_span) = spans(child);
        if let (None, Some(row)) = (item.column.start_index(), item.row.start_index()) {
            let mut column = 0;
            let found = loop {
                if column + column_span > columns {
                    break None;
                }
                budget.charge(1).map_err(work)?;
                match occupancy
                    .conflict((row, column, row_span, column_span), budget)
                    .map_err(work)?
                {
                    None => break Some(column),
                    Some((_, occupied)) => column = occupied + 1,
                }
            };
            let p = PlacedItem {
                child,
                column: found.ok_or_else(no_space)?,
                column_span,
                row,
                row_span,
            };
            occupancy.occupy(id, &p, budget)?;
            placed[slot] = Some(p);
        }
    }
    // 3) cursor 자동 배치.
    let (mut cursor_row, mut cursor_column) = (0usize, 0usize);
    for (slot, &child) in children.iter().enumerate() {
        let (item, column_span, row_span) = spans(child);
        if item.row.start_index().is_some() {
            continue;
        }
        let (row, column) = match item.column.start_index() {
            Some(column) => {
                if column < cursor_column {
                    cursor_row += 1;
                }
                let mut row = cursor_row;
                loop {
                    if row + row_span > MAX_GRID_TRACKS {
                        return Err(limit(id, LayoutLimit::GridLines));
                    }
                    budget.charge(1).map_err(work)?;
                    match occupancy
                        .conflict((row, column, row_span, column_span), budget)
                        .map_err(work)?
                    {
                        None => break,
                        Some((occupied, _)) => row = occupied + 1,
                    }
                }
                (row, column)
            }
            None => {
                let (mut row, mut column) = (cursor_row, cursor_column);
                loop {
                    if row + row_span > MAX_GRID_TRACKS {
                        return Err(limit(id, LayoutLimit::GridLines));
                    }
                    if column + column_span > columns {
                        row += 1;
                        column = 0;
                        continue;
                    }
                    budget.charge(1).map_err(work)?;
                    match occupancy
                        .conflict((row, column, row_span, column_span), budget)
                        .map_err(work)?
                    {
                        None => break,
                        Some((_, occupied)) => column = occupied + 1,
                    }
                }
                (row, column)
            }
        };
        let p = PlacedItem {
            child,
            column,
            column_span,
            row,
            row_span,
        };
        occupancy.occupy(id, &p, budget)?;
        placed[slot] = Some(p);
        cursor_row = row;
        cursor_column = column + column_span;
    }
    placed.into_iter().map(|p| p.ok_or_else(no_space)).collect()
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
    let mut budget = WorkBudget::new(MAX_GRID_WORK);
    let items = place_items(engine, id, &children, grid.columns.len(), &mut budget)?;
    let track_error = |error: TrackError| match error {
        TrackError::Arithmetic(error) => LayoutError::Arithmetic { node: id, error },
        TrackError::WorkLimit => limit(id, LayoutLimit::GridWork),
    };

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
    let column_sizes = size_tracks(&columns, column_gap, inner[0], &column_items, &mut budget)
        .map_err(track_error)?;
    let (column_offsets, column_extent) =
        track_offsets(&column_sizes, column_gap).map_err(at(id))?;

    // 행: 열 영역 너비가 정해진 뒤의 높이 기여.
    let mut row_items = Vec::with_capacity(items.len());
    for item in &items {
        let width = span_extent(
            &column_offsets,
            &column_sizes,
            item.column,
            item.column_span,
        )
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
    let row_sizes =
        size_tracks(&rows, row_gap, inner[1], &row_items, &mut budget).map_err(track_error)?;
    let (row_offsets, row_extent) = track_offsets(&row_sizes, row_gap).map_err(at(id))?;
    Ok(GridSolution {
        column_offsets,
        column_sizes,
        row_offsets,
        row_sizes,
        tracks: [columns, rows],
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
    // X.6.5: 자식 HugContent 순환 검사에 쓸 item 영역 의존성.
    let prefix = [
        dependence_prefix(&solution.tracks[0]),
        dependence_prefix(&solution.tracks[1]),
    ];
    for item in &solution.items {
        engine.grid_areas.insert(
            item.child,
            [
                area_dependence(&prefix[0], item.column, item.column_span),
                area_dependence(&prefix[1], item.row, item.row_span),
            ],
        );
    }
    let mut placements = Vec::with_capacity(solution.items.len());
    for item in &solution.items {
        let width = span_extent(
            &solution.column_offsets,
            &solution.column_sizes,
            item.column,
            item.column_span,
        )
        .map_err(at(id))?;
        let height = span_extent(
            &solution.row_offsets,
            &solution.row_sizes,
            item.row,
            item.row_span,
        )
        .map_err(at(id))?;
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

    fn auto(span: u16) -> GridPlacement {
        GridPlacement::Auto { span }
    }

    /// 3열(각 10) × 암시 행(10) grid, gap 0, padding 0. 자식은 (StableId, OrderKey 순번, 위치).
    fn auto_grid(children: &[(u64, u32, GridItem)]) -> LayoutResult {
        let grid = GridLayout {
            implicit_row: fixed_track(10.0),
            ..GridLayout::new(vec![fixed_track(10.0); 3], vec![])
        };
        let mut root = LayoutNode::new(id(1))
            .with_sizing(SizingMode::HugContent, SizingMode::HugContent)
            .with_container(ContainerLayout::Grid(grid));
        let mut nodes = Vec::new();
        for (n, order, item) in children {
            root = root.with_child(id(*n), key(*order));
            nodes.push(
                LayoutNode::new(id(*n))
                    .with_grid(*item)
                    .with_sizing(SizingMode::Fill, SizingMode::Fill),
            );
        }
        nodes.insert(0, root);
        compute_layout(&LayoutInput::new(id(1), nodes)).unwrap()
    }

    /// node의 (열, 행) cell 위치와 너비(칸 수).
    fn cell(result: &LayoutResult, n: u64) -> (i32, i32, i32) {
        let r = result.get(id(n)).unwrap().local_rect();
        (r.x().raw() / 640, r.y().raw() / 640, r.width().raw() / 640)
    }

    #[test]
    fn auto_placement_follows_visual_order_key_not_insertion() {
        let one = GridItem {
            column: auto(1),
            row: auto(1),
        };
        // 삽입 순서 id 2..6, OrderKey 5,3,1,4,2 → visual order: 4, 6, 3, 5, 2.
        let children = [
            (2, 5, one),
            (3, 3, one),
            (4, 1, one),
            (5, 4, one),
            (6, 2, one),
        ];
        let result = auto_grid(&children);
        assert_eq!(cell(&result, 4), (0, 0, 1));
        assert_eq!(cell(&result, 6), (1, 0, 1));
        assert_eq!(cell(&result, 3), (2, 0, 1));
        assert_eq!(cell(&result, 5), (0, 1, 1));
        assert_eq!(cell(&result, 2), (1, 1, 1));
        // 삽입 순서를 바꿔도(같은 OrderKey) 결과 bytes 동일.
        let mut shuffled = children;
        shuffled.reverse();
        assert_eq!(auto_grid(&shuffled).encode(), result.encode());
        // 결과 box 순서도 visual order.
        let order: Vec<_> = result.boxes().iter().skip(1).map(|b| b.id()).collect();
        assert_eq!(order, vec![id(4), id(6), id(3), id(5), id(2)]);
    }

    #[test]
    fn auto_placement_skips_occupied_cells_row_major() {
        // 명시 item (열 2, 행 1) → cell (1, 0) 점유.
        let explicit = GridItem::at(2, 1, 1, 1);
        let span2 = GridItem {
            column: auto(2),
            row: auto(1),
        };
        let one = GridItem {
            column: auto(1),
            row: auto(1),
        };
        // 행만 명시(행 4): 그 행 첫 빈 열.
        let row_locked = GridItem {
            column: auto(1),
            row: GridPlacement::Span { start: 4, span: 1 },
        };
        // 열만 명시(열 1): cursor 이후 그 열이 비는 첫 행.
        let column_locked = GridItem {
            column: GridPlacement::Span { start: 1, span: 1 },
            row: auto(1),
        };
        let full = GridItem {
            column: auto(3),
            row: auto(1),
        };
        let result = auto_grid(&[
            (2, 1, explicit),
            (3, 2, span2),
            (4, 3, one),
            (5, 4, row_locked),
            (6, 5, column_locked),
            (7, 6, full),
        ]);
        assert_eq!(cell(&result, 2), (1, 0, 1));
        // span 2: 행 0은 열 1이 점유 → 행 1 열 0.
        assert_eq!(cell(&result, 3), (0, 1, 2));
        // 다음 자동 item은 cursor(행 1, 열 2)부터 → (2, 1). 행 0의 빈 칸은 sparse로 남음.
        assert_eq!(cell(&result, 4), (2, 1, 1));
        // 행 고정은 cursor와 무관하게 행 3(0부터)의 첫 열.
        assert_eq!(cell(&result, 5), (0, 3, 1));
        // 열 고정(열 0 < cursor 열 3) → 다음 행(2)부터, 열 0이 빈 첫 행 = 2.
        assert_eq!(cell(&result, 6), (0, 2, 1));
        // 3칸 item: cursor (2, 1)부터 → 행 2는 열 0 점유·열 1부터 3칸 불가 → 행 3은 열 0 점유 → 행 4.
        assert_eq!(cell(&result, 7), (0, 4, 3));
        // 암시 행 5개(0~4) → Hug 높이 50.
        assert_eq!(result.get(id(1)).unwrap().local_rect().height(), px(50));
    }

    /// 열 `columns`개(각 1px)·암시 행 1px grid와 (StableId, 위치) 자식.
    fn limit_input(columns: usize, children: &[(u64, GridItem)]) -> LayoutInput {
        let grid = GridLayout {
            implicit_row: fixed_track(1.0),
            ..GridLayout::new(vec![fixed_track(1.0); columns], vec![])
        };
        let mut root = LayoutNode::new(id(1))
            .with_sizing(SizingMode::HugContent, SizingMode::HugContent)
            .with_container(ContainerLayout::Grid(grid));
        let mut nodes = Vec::new();
        for (i, (n, item)) in children.iter().enumerate() {
            root = root.with_child(id(*n), key(i as u32 + 1));
            nodes.push(
                LayoutNode::new(id(*n))
                    .with_grid(*item)
                    .with_sizing(SizingMode::Fill, SizingMode::Fill),
            );
        }
        nodes.insert(0, root);
        LayoutInput::new(id(1), nodes)
    }

    /// 1px cell grid에서 node의 (열, 행) 위치와 너비(칸 수).
    fn unit_cell(result: &LayoutResult, n: u64) -> (i32, i32, i32) {
        let r = result.get(id(n)).unwrap().local_rect();
        (r.x().raw() / 64, r.y().raw() / 64, r.width().raw() / 64)
    }

    fn exceeded(node: u64, limit: LayoutLimit) -> Result<(), LayoutError> {
        Err(LayoutError::LimitExceeded {
            node: id(node),
            limit,
        })
    }

    #[test]
    fn auto_placement_past_u16_line_range_is_limit_error() {
        // 리뷰 재현: 1열, 명시 item이 마지막 행(line 65534..65535)을 점유, 자동 item은 행 65534칸.
        // 이전에는 수 초 탐색 뒤 "공간 없음"이었습니다. 지금은 점유 cell을 건너뛰어 곧바로 line 범위
        // 상한 오류입니다(자동 item을 놓으려면 끝 line이 65535를 넘어야 함).
        let last_row = GridItem::at(1, 65534, 1, 1);
        let tall = GridItem {
            column: auto(1),
            row: auto(65534),
        };
        let input = limit_input(1, &[(2, last_row), (3, tall)]);
        assert_eq!(input.validate(), Ok(()));
        let started = std::time::Instant::now();
        assert_eq!(
            compute_layout(&input).map(|_| ()),
            exceeded(1, LayoutLimit::GridLines)
        );
        // 결정적 상한이라 시간 단언은 하지 않지만 debug에서도 즉시 끝나야 합니다(참고용).
        assert!(started.elapsed() < std::time::Duration::from_secs(30));
        // 끝 line이 u16을 넘는 명시·자동 item은 검증 단계에서 거부합니다.
        for item in [
            GridItem::at(1, 65535, 1, 1),
            GridItem {
                column: auto(1),
                row: auto(65535),
            },
        ] {
            assert_eq!(
                limit_input(1, &[(2, item)]).validate(),
                exceeded(2, LayoutLimit::GridLines)
            );
        }
        // 끝 line 65535(행 65534개)까지는 허용하고 암시 행은 그 범위 안에서 만들어집니다.
        let fits = limit_input(1, &[(2, GridItem::at(1, 65534, 1, 1))]);
        let result = compute_layout(&fits).unwrap();
        assert_eq!(result.get(id(1)).unwrap().local_rect().height(), px(65534));
        // 자동 item이 line 범위 끝에 꼭 맞으면 놓입니다.
        let edge = limit_input(
            1,
            &[
                (2, GridItem::at(1, 1, 1, 1)),
                (
                    3,
                    GridItem {
                        column: auto(1),
                        row: auto(65533),
                    },
                ),
            ],
        );
        let result = compute_layout(&edge).unwrap();
        assert_eq!(unit_cell(&result, 3), (0, 1, 1));
    }

    #[test]
    fn grid_cell_and_track_counts_are_bounded() {
        // 열 20000개 + 행 65534의 명시 item → 13억 cell 할당 대신 검증 단계 cell 상한 오류.
        let wide = limit_input(20_000, &[(2, GridItem::at(1, 65534, 1, 1))]);
        assert_eq!(wide.validate(), exceeded(2, LayoutLimit::GridCells));
        // 명시 track 수·명시 cell 수 상한.
        let too_many = GridLayout::new(vec![fixed_track(1.0); MAX_GRID_TRACKS + 1], vec![]);
        assert_eq!(
            grid_input(too_many).validate(),
            exceeded(1, LayoutLimit::GridLines)
        );
        let too_big = GridLayout::new(vec![fixed_track(1.0); 2048], vec![fixed_track(1.0); 1024]);
        assert_eq!(
            grid_input(too_big).validate(),
            exceeded(1, LayoutLimit::GridCells)
        );
        // 자동 배치가 늘린 행이 cell 상한을 넘으면 계산 중 오류(각 item은 검증을 통과).
        let block = GridItem {
            column: auto(100),
            row: auto(6000),
        };
        let input = limit_input(100, &[(2, block), (3, block)]);
        assert_eq!(input.validate(), Ok(()));
        assert_eq!(
            compute_layout(&input).map(|_| ()),
            exceeded(1, LayoutLimit::GridCells)
        );
    }

    #[test]
    fn grid_work_is_bounded() {
        // 겹침이 허용된 명시 item 17개가 각각 1024×1024 cell 전체를 점유 → 점유 작업량이
        // MAX_GRID_WORK(2^24)를 넘어 작업량 상한 오류.
        let full = GridItem::at(1, 1, 1024, 1024);
        let children: Vec<(u64, GridItem)> = (2..19).map(|n| (n, full)).collect();
        let input = limit_input(1024, &children);
        assert_eq!(input.validate(), Ok(()));
        assert_eq!(
            compute_layout(&input).map(|_| ()),
            exceeded(1, LayoutLimit::GridWork)
        );
        // 일반 크기 grid(12열, 자동 item 2000개)는 상한과 무관하게 계산됩니다.
        let one = GridItem {
            column: auto(1),
            row: auto(1),
        };
        let many: Vec<(u64, GridItem)> = (2..2002).map(|n| (n, one)).collect();
        let result = compute_layout(&limit_input(12, &many)).unwrap();
        // 2000개 / 12열 = 행 167개(마지막 행 8개).
        assert_eq!(result.get(id(1)).unwrap().local_rect().height(), px(167));
        assert_eq!(unit_cell(&result, 2001), (7, 166, 1));
    }
}
