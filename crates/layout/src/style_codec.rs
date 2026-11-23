//! [`LayoutStyle`] canonical bytes (crate 내부, ADR-0076).
//!
//! 필드 순서는 [`LayoutStyle`] 선언 순서입니다. enum은 고정 tag 한 byte, 길이는 f32 LE, 목록은 `u32` 길이 뒤
//! 항목입니다. 런타임·fixture 비교용이며 저장 형식이 아닙니다.

use stellar_core_types::value::{FiniteF32, Length, Percentage};

use crate::codec::{Reader, Writer};
use crate::error::LayoutError;
use crate::style::{
    AspectRatio, BoxSizing, ContentAlignment, Display, FlexBasis, FlexDirection, FlexWrap,
    GridAutoFlow, GridLine, ItemAlignment, LayoutStyle, LengthPercentage, LengthPercentageAuto,
    MaxSize, MaxTrackSize, MinTrackSize, Position, SelfAlignment, Size, TrackSize,
};

fn malformed(reason: &'static str) -> LayoutError {
    LayoutError::Malformed { reason }
}

fn percentage(r: &mut Reader<'_>, field: &'static str) -> Result<Percentage, LayoutError> {
    Ok(Percentage::from_raw(r.finite(field)?))
}

fn write_lp(w: &mut Writer, value: LengthPercentage) {
    match value {
        LengthPercentage::Length(l) => {
            w.u8(0);
            w.length(l);
        }
        LengthPercentage::Percentage(p) => {
            w.u8(1);
            w.finite(p.raw());
        }
    }
}

fn read_lp(r: &mut Reader<'_>) -> Result<LengthPercentage, LayoutError> {
    Ok(match r.u8()? {
        0 => LengthPercentage::Length(r.length("length")?),
        1 => LengthPercentage::Percentage(percentage(r, "percentage")?),
        _ => return Err(malformed("알 수 없는 length-percentage tag")),
    })
}

fn write_inset(w: &mut Writer, value: LengthPercentageAuto) {
    match value {
        LengthPercentageAuto::Auto => w.u8(0),
        LengthPercentageAuto::Length(l) => {
            w.u8(1);
            w.length(l);
        }
        LengthPercentageAuto::Percentage(p) => {
            w.u8(2);
            w.finite(p.raw());
        }
    }
}

fn read_inset(r: &mut Reader<'_>) -> Result<LengthPercentageAuto, LayoutError> {
    Ok(match r.u8()? {
        0 => LengthPercentageAuto::Auto,
        1 => LengthPercentageAuto::Length(r.length("inset")?),
        2 => LengthPercentageAuto::Percentage(percentage(r, "inset")?),
        _ => return Err(malformed("알 수 없는 inset tag")),
    })
}

fn write_size(w: &mut Writer, value: Size) {
    match value {
        Size::Auto => w.u8(0),
        Size::Length(l) => {
            w.u8(1);
            w.length(l);
        }
        Size::Percentage(p) => {
            w.u8(2);
            w.finite(p.raw());
        }
        Size::MinContent => w.u8(3),
        Size::MaxContent => w.u8(4),
        Size::FitContent => w.u8(5),
    }
}

fn read_size(r: &mut Reader<'_>, field: &'static str) -> Result<Size, LayoutError> {
    Ok(match r.u8()? {
        0 => Size::Auto,
        1 => Size::Length(r.length(field)?),
        2 => Size::Percentage(percentage(r, field)?),
        3 => Size::MinContent,
        4 => Size::MaxContent,
        5 => Size::FitContent,
        _ => return Err(malformed("알 수 없는 size tag")),
    })
}

fn write_max(w: &mut Writer, value: MaxSize) {
    match value {
        MaxSize::None => w.u8(0),
        MaxSize::Length(l) => {
            w.u8(1);
            w.length(l);
        }
        MaxSize::Percentage(p) => {
            w.u8(2);
            w.finite(p.raw());
        }
        MaxSize::MinContent => w.u8(3),
        MaxSize::MaxContent => w.u8(4),
        MaxSize::FitContent => w.u8(5),
    }
}

fn read_max(r: &mut Reader<'_>, field: &'static str) -> Result<MaxSize, LayoutError> {
    Ok(match r.u8()? {
        0 => MaxSize::None,
        1 => MaxSize::Length(r.length(field)?),
        2 => MaxSize::Percentage(percentage(r, field)?),
        3 => MaxSize::MinContent,
        4 => MaxSize::MaxContent,
        5 => MaxSize::FitContent,
        _ => return Err(malformed("알 수 없는 max size tag")),
    })
}

fn content_tag(value: ContentAlignment) -> u8 {
    match value {
        ContentAlignment::Normal => 0,
        ContentAlignment::Start => 1,
        ContentAlignment::End => 2,
        ContentAlignment::FlexStart => 3,
        ContentAlignment::FlexEnd => 4,
        ContentAlignment::Center => 5,
        ContentAlignment::SpaceBetween => 6,
        ContentAlignment::SpaceAround => 7,
        ContentAlignment::SpaceEvenly => 8,
        ContentAlignment::Stretch => 9,
    }
}

fn read_content(r: &mut Reader<'_>) -> Result<ContentAlignment, LayoutError> {
    Ok(match r.u8()? {
        0 => ContentAlignment::Normal,
        1 => ContentAlignment::Start,
        2 => ContentAlignment::End,
        3 => ContentAlignment::FlexStart,
        4 => ContentAlignment::FlexEnd,
        5 => ContentAlignment::Center,
        6 => ContentAlignment::SpaceBetween,
        7 => ContentAlignment::SpaceAround,
        8 => ContentAlignment::SpaceEvenly,
        9 => ContentAlignment::Stretch,
        _ => return Err(malformed("알 수 없는 content alignment tag")),
    })
}

fn item_tag(value: ItemAlignment) -> u8 {
    match value {
        ItemAlignment::Normal => 0,
        ItemAlignment::Stretch => 1,
        ItemAlignment::Start => 2,
        ItemAlignment::End => 3,
        ItemAlignment::FlexStart => 4,
        ItemAlignment::FlexEnd => 5,
        ItemAlignment::Center => 6,
        ItemAlignment::Baseline => 7,
    }
}

fn read_item_tag(tag: u8) -> Result<ItemAlignment, LayoutError> {
    Ok(match tag {
        0 => ItemAlignment::Normal,
        1 => ItemAlignment::Stretch,
        2 => ItemAlignment::Start,
        3 => ItemAlignment::End,
        4 => ItemAlignment::FlexStart,
        5 => ItemAlignment::FlexEnd,
        6 => ItemAlignment::Center,
        7 => ItemAlignment::Baseline,
        _ => return Err(malformed("알 수 없는 item alignment tag")),
    })
}

fn write_self(w: &mut Writer, value: SelfAlignment) {
    match value {
        SelfAlignment::Auto => w.u8(255),
        SelfAlignment::Item(item) => w.u8(item_tag(item)),
    }
}

fn read_self(r: &mut Reader<'_>) -> Result<SelfAlignment, LayoutError> {
    Ok(match r.u8()? {
        255 => SelfAlignment::Auto,
        tag => SelfAlignment::Item(read_item_tag(tag)?),
    })
}

fn write_line(w: &mut Writer, value: GridLine) {
    match value {
        GridLine::Auto => w.u8(0),
        GridLine::Line(n) => {
            w.u8(1);
            w.bytes(&n.to_le_bytes());
        }
        GridLine::Span(n) => {
            w.u8(2);
            w.u16(n);
        }
    }
}

fn read_line(r: &mut Reader<'_>) -> Result<GridLine, LayoutError> {
    Ok(match r.u8()? {
        0 => GridLine::Auto,
        1 => GridLine::Line(i16::from_le_bytes([r.u8()?, r.u8()?])),
        2 => GridLine::Span(r.u16()?),
        _ => return Err(malformed("알 수 없는 grid line tag")),
    })
}

fn write_track(w: &mut Writer, track: &TrackSize) {
    match track.min {
        MinTrackSize::Auto => w.u8(0),
        MinTrackSize::Length(l) => {
            w.u8(1);
            w.length(l);
        }
        MinTrackSize::Percentage(p) => {
            w.u8(2);
            w.finite(p.raw());
        }
        MinTrackSize::MinContent => w.u8(3),
        MinTrackSize::MaxContent => w.u8(4),
    }
    match track.max {
        MaxTrackSize::Auto => w.u8(0),
        MaxTrackSize::Length(l) => {
            w.u8(1);
            w.length(l);
        }
        MaxTrackSize::Percentage(p) => {
            w.u8(2);
            w.finite(p.raw());
        }
        MaxTrackSize::MinContent => w.u8(3),
        MaxTrackSize::MaxContent => w.u8(4),
        MaxTrackSize::Fraction(fr) => {
            w.u8(5);
            w.finite(fr);
        }
        MaxTrackSize::FitContent(limit) => {
            w.u8(6);
            write_lp(w, limit);
        }
    }
}

fn read_track(r: &mut Reader<'_>) -> Result<TrackSize, LayoutError> {
    let min = match r.u8()? {
        0 => MinTrackSize::Auto,
        1 => MinTrackSize::Length(r.length("track")?),
        2 => MinTrackSize::Percentage(percentage(r, "track")?),
        3 => MinTrackSize::MinContent,
        4 => MinTrackSize::MaxContent,
        _ => return Err(malformed("알 수 없는 track min tag")),
    };
    let max = match r.u8()? {
        0 => MaxTrackSize::Auto,
        1 => MaxTrackSize::Length(r.length("track")?),
        2 => MaxTrackSize::Percentage(percentage(r, "track")?),
        3 => MaxTrackSize::MinContent,
        4 => MaxTrackSize::MaxContent,
        5 => MaxTrackSize::Fraction(r.finite("track fr")?),
        6 => MaxTrackSize::FitContent(read_lp(r)?),
        _ => return Err(malformed("알 수 없는 track max tag")),
    };
    Ok(TrackSize { min, max })
}

fn write_tracks(w: &mut Writer, tracks: &[TrackSize]) {
    w.len(tracks.len());
    for track in tracks {
        write_track(w, track);
    }
}

fn read_tracks(r: &mut Reader<'_>) -> Result<Vec<TrackSize>, LayoutError> {
    let count = r.len(2)?;
    let mut tracks = Vec::with_capacity(count);
    for _ in 0..count {
        tracks.push(read_track(r)?);
    }
    Ok(tracks)
}

/// style canonical bytes를 씁니다.
pub(crate) fn encode_style(w: &mut Writer, s: &LayoutStyle) {
    w.u8(match s.display {
        Display::Block => 0,
        Display::Flex => 1,
        Display::Grid => 2,
        Display::None => 3,
    });
    w.u8(match s.position {
        Position::Static => 0,
        Position::Relative => 1,
        Position::Absolute => 2,
    });
    for inset in [s.top, s.right, s.bottom, s.left] {
        write_inset(w, inset);
    }
    for size in [s.width, s.height, s.min_width, s.min_height] {
        write_size(w, size);
    }
    write_max(w, s.max_width);
    write_max(w, s.max_height);
    match s.aspect_ratio {
        None => w.bool(false),
        Some(ratio) => {
            w.bool(true);
            w.finite(ratio.width);
            w.finite(ratio.height);
        }
    }
    w.insets(&s.padding);
    w.u8(match s.flex_direction {
        FlexDirection::Row => 0,
        FlexDirection::RowReverse => 1,
        FlexDirection::Column => 2,
        FlexDirection::ColumnReverse => 3,
    });
    w.u8(match s.flex_wrap {
        FlexWrap::NoWrap => 0,
        FlexWrap::Wrap => 1,
        FlexWrap::WrapReverse => 2,
    });
    w.u8(content_tag(s.justify_content));
    w.u8(content_tag(s.align_content));
    w.u8(item_tag(s.align_items));
    w.u8(item_tag(s.justify_items));
    w.length(s.row_gap);
    w.length(s.column_gap);
    w.finite(s.flex_grow);
    w.finite(s.flex_shrink);
    match s.flex_basis {
        FlexBasis::Auto => w.u8(0),
        FlexBasis::Content => w.u8(1),
        FlexBasis::Length(l) => {
            w.u8(2);
            w.length(l);
        }
        FlexBasis::Percentage(p) => {
            w.u8(3);
            w.finite(p.raw());
        }
    }
    write_self(w, s.align_self);
    write_self(w, s.justify_self);
    write_tracks(w, &s.grid_template_columns);
    write_tracks(w, &s.grid_template_rows);
    write_tracks(w, &s.grid_auto_columns);
    write_tracks(w, &s.grid_auto_rows);
    w.u8(match s.grid_auto_flow {
        GridAutoFlow::Row => 0,
        GridAutoFlow::Column => 1,
    });
    for line in [
        s.grid_column_start,
        s.grid_column_end,
        s.grid_row_start,
        s.grid_row_end,
    ] {
        write_line(w, line);
    }
    w.u8(match s.box_sizing {
        BoxSizing::ContentBox => 0,
        BoxSizing::BorderBox => 1,
    });
}

/// style canonical bytes를 읽습니다(형식·finite 검사만).
pub(crate) fn decode_style(r: &mut Reader<'_>) -> Result<LayoutStyle, LayoutError> {
    let display = match r.u8()? {
        0 => Display::Block,
        1 => Display::Flex,
        2 => Display::Grid,
        3 => Display::None,
        _ => return Err(malformed("알 수 없는 display tag")),
    };
    let position = match r.u8()? {
        0 => Position::Static,
        1 => Position::Relative,
        2 => Position::Absolute,
        _ => return Err(malformed("알 수 없는 position tag")),
    };
    let top = read_inset(r)?;
    let right = read_inset(r)?;
    let bottom = read_inset(r)?;
    let left = read_inset(r)?;
    let width = read_size(r, "width")?;
    let height = read_size(r, "height")?;
    let min_width = read_size(r, "min-width")?;
    let min_height = read_size(r, "min-height")?;
    let max_width = read_max(r, "max-width")?;
    let max_height = read_max(r, "max-height")?;
    let aspect_ratio = if r.bool("aspect-ratio flag")? {
        Some(AspectRatio {
            width: r.finite("aspect-ratio")?,
            height: r.finite("aspect-ratio")?,
        })
    } else {
        None
    };
    let padding = r.insets("padding")?;
    let flex_direction = match r.u8()? {
        0 => FlexDirection::Row,
        1 => FlexDirection::RowReverse,
        2 => FlexDirection::Column,
        3 => FlexDirection::ColumnReverse,
        _ => return Err(malformed("알 수 없는 flex-direction tag")),
    };
    let flex_wrap = match r.u8()? {
        0 => FlexWrap::NoWrap,
        1 => FlexWrap::Wrap,
        2 => FlexWrap::WrapReverse,
        _ => return Err(malformed("알 수 없는 flex-wrap tag")),
    };
    let justify_content = read_content(r)?;
    let align_content = read_content(r)?;
    let align_items = read_item_tag(r.u8()?)?;
    let justify_items = read_item_tag(r.u8()?)?;
    let row_gap: Length = r.length("row-gap")?;
    let column_gap = r.length("column-gap")?;
    let flex_grow: FiniteF32 = r.finite("flex-grow")?;
    let flex_shrink = r.finite("flex-shrink")?;
    let flex_basis = match r.u8()? {
        0 => FlexBasis::Auto,
        1 => FlexBasis::Content,
        2 => FlexBasis::Length(r.length("flex-basis")?),
        3 => FlexBasis::Percentage(percentage(r, "flex-basis")?),
        _ => return Err(malformed("알 수 없는 flex-basis tag")),
    };
    let align_self = read_self(r)?;
    let justify_self = read_self(r)?;
    let grid_template_columns = read_tracks(r)?;
    let grid_template_rows = read_tracks(r)?;
    let grid_auto_columns = read_tracks(r)?;
    let grid_auto_rows = read_tracks(r)?;
    let grid_auto_flow = match r.u8()? {
        0 => GridAutoFlow::Row,
        1 => GridAutoFlow::Column,
        _ => return Err(malformed("알 수 없는 grid-auto-flow tag")),
    };
    let grid_column_start = read_line(r)?;
    let grid_column_end = read_line(r)?;
    let grid_row_start = read_line(r)?;
    let grid_row_end = read_line(r)?;
    let box_sizing = match r.u8()? {
        0 => BoxSizing::ContentBox,
        1 => BoxSizing::BorderBox,
        _ => return Err(malformed("알 수 없는 box-sizing tag")),
    };
    Ok(LayoutStyle {
        display,
        position,
        box_sizing,
        top,
        right,
        bottom,
        left,
        width,
        height,
        min_width,
        min_height,
        max_width,
        max_height,
        aspect_ratio,
        padding,
        flex_direction,
        flex_wrap,
        justify_content,
        align_content,
        align_items,
        justify_items,
        row_gap,
        column_gap,
        flex_grow,
        flex_shrink,
        flex_basis,
        align_self,
        justify_self,
        grid_template_columns,
        grid_template_rows,
        grid_auto_columns,
        grid_auto_rows,
        grid_auto_flow,
        grid_column_start,
        grid_column_end,
        grid_row_start,
        grid_row_end,
    })
}
