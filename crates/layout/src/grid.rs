//! Grid 배치 (X.4.1).
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

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{FiniteF32, Length, Scalar};

use crate::codec::{Reader, Writer};
use crate::error::LayoutError;
use crate::input::length_unit;
use crate::property::LayoutProperty;
use crate::rounding::LayoutUnit;

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
}
