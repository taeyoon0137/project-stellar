//! node layout record: CSS 레이아웃 longhand의 명시 값 (IX.11.6, ADR-0086).
//!
//! 상자를 가진 node(Frame, Shape, Path, Text, Image, ComponentInstance)마다 레이아웃 longhand의 **명시 값만**
//! 담은 희소 record를 하나 둡니다. 명시하지 않은 longhand는 record에 없고, 활성 기본값 세트
//! ([`crate::css::style_defaults`])가 값을 정합니다(ADR-0082 결정 1). record는 modifier stack처럼 payload와
//! 분리된 목록([`crate::records::DocumentRecords::layout_records`])으로 저장합니다.
//!
//! - item longhand(`position`, inset, 크기, `flex-*`, `align-self`·`justify-self`, `grid-*-start`·`-end`,
//!   `box-sizing`)는 상자를 가진 모든 node 종류에 같은 key로 붙습니다.
//! - container longhand(`display`, `padding-*`, `flex-direction` …, `grid-template-*`)는 Frame에만 붙습니다.
//! - Frame·Rectangle·Ellipse·Polygon·Image의 `width`·`height`는 payload 크기(기존 key `0x0108`·`0x0301`·`0x0602`
//!   등)가 저장 위치입니다. 이 종류의 record에는 `width`·`height`를 두지 않습니다(같은 값을 두 곳에 두지 않음).
//! - Fragment는 상자가 없어 record를 가질 수 없습니다(아키텍처 §12.4).
//!
//! 값의 CSS 텍스트는 [`crate::css::value`]의 수 표기(`format_length` 등)를 그대로 씁니다. 해석도 수 부분은
//! [`crate::css::value::parse`]를 거칩니다.
//!
//! # binary 형식
//!
//! `node`(16) + 항목 수 `u16` LE + 항목*(longhand 번호 `u16` LE + 값). 항목은 longhand 번호 오름차순이고 같은
//! longhand는 한 번만 나옵니다. 값은 tag `u8` 뒤에 payload입니다([`LayoutValue`] 참고). f32는 LE이고 `-0.0`은
//! `0.0`으로 씁니다.

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyKey;

use crate::css::grammar::{CssGrammar, LENGTH_NON_NEGATIVE, LENGTH_SIGNED};
use crate::css::value::{self, format_length, format_number, format_percentage, CssValue};
use crate::node_kind::NodeKind;
use crate::records::NodePayload;
use crate::shape::ShapeGeometry;

/// longhand가 붙는 범위.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayoutRole {
    /// 상자를 가진 모든 node(부모 배치에 참여하는 쪽).
    Item,
    /// Frame만(자식을 배치하는 쪽).
    Container,
}

/// 값 문법.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutGrammar {
    /// keyword 하나.
    Keyword(&'static [&'static str]),
    /// `auto | <length> | <percentage>`(inset, 음수 허용).
    Inset,
    /// `auto | <length> | <percentage> | min-content | max-content | fit-content`(비음수).
    Size,
    /// `none | <length> | <percentage> | min-content | max-content | fit-content`(비음수).
    MaxSize,
    /// `auto | <number> / <number>`(양수).
    AspectRatio,
    /// `<number>`(비음수).
    NonNegativeNumber,
    /// `auto | content | <length> | <percentage>`(비음수).
    FlexBasis,
    /// `auto | <integer>(0 아님) | span <integer>(1 이상)`.
    GridLine,
    /// `<length>`(비음수).
    NonNegativeLength,
    /// `normal | <length>`(비음수).
    Gap,
    /// `none | <track-size>+`.
    TrackList,
    /// `<track-size>+`.
    AutoTracks,
}

/// 레이아웃 longhand. 번호([`LayoutLonghand::id`])는 저장 형식의 일부입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LayoutLonghand {
    /// `position`.
    Position,
    /// `box-sizing`.
    BoxSizing,
    /// `top`.
    Top,
    /// `right`.
    Right,
    /// `bottom`.
    Bottom,
    /// `left`.
    Left,
    /// `width`.
    Width,
    /// `height`.
    Height,
    /// `min-width`.
    MinWidth,
    /// `min-height`.
    MinHeight,
    /// `max-width`.
    MaxWidth,
    /// `max-height`.
    MaxHeight,
    /// `aspect-ratio`.
    AspectRatio,
    /// `flex-grow`.
    FlexGrow,
    /// `flex-shrink`.
    FlexShrink,
    /// `flex-basis`.
    FlexBasis,
    /// `align-self`.
    AlignSelf,
    /// `justify-self`.
    JustifySelf,
    /// `grid-column-start`.
    GridColumnStart,
    /// `grid-column-end`.
    GridColumnEnd,
    /// `grid-row-start`.
    GridRowStart,
    /// `grid-row-end`.
    GridRowEnd,
    /// `display`.
    Display,
    /// `padding-top`.
    PaddingTop,
    /// `padding-right`.
    PaddingRight,
    /// `padding-bottom`.
    PaddingBottom,
    /// `padding-left`.
    PaddingLeft,
    /// `flex-direction`.
    FlexDirection,
    /// `flex-wrap`.
    FlexWrap,
    /// `justify-content`.
    JustifyContent,
    /// `align-content`.
    AlignContent,
    /// `align-items`.
    AlignItems,
    /// `justify-items`.
    JustifyItems,
    /// `row-gap`.
    RowGap,
    /// `column-gap`.
    ColumnGap,
    /// `grid-template-columns`.
    GridTemplateColumns,
    /// `grid-template-rows`.
    GridTemplateRows,
    /// `grid-auto-columns`.
    GridAutoColumns,
    /// `grid-auto-rows`.
    GridAutoRows,
    /// `grid-auto-flow`.
    GridAutoFlow,
}

const CONTENT_ALIGNMENT: &[&str] = &[
    "normal",
    "start",
    "end",
    "flex-start",
    "flex-end",
    "center",
    "space-between",
    "space-around",
    "space-evenly",
    "stretch",
];
const ITEM_ALIGNMENT: &[&str] = &[
    "normal",
    "stretch",
    "start",
    "end",
    "flex-start",
    "flex-end",
    "center",
    "baseline",
];
const SELF_ALIGNMENT: &[&str] = &[
    "auto",
    "normal",
    "stretch",
    "start",
    "end",
    "flex-start",
    "flex-end",
    "center",
    "baseline",
];

/// node 공통(`0x00xx`) item longhand key.
pub mod keys {
    use stellar_core_types::property::PropertyKey;

    /// `position`.
    pub const POSITION: PropertyKey = PropertyKey(0x0010);
    /// `box-sizing`.
    pub const BOX_SIZING: PropertyKey = PropertyKey(0x0011);
    /// `top`.
    pub const TOP: PropertyKey = PropertyKey(0x0012);
    /// `right`.
    pub const RIGHT: PropertyKey = PropertyKey(0x0013);
    /// `bottom`.
    pub const BOTTOM: PropertyKey = PropertyKey(0x0014);
    /// `left`.
    pub const LEFT: PropertyKey = PropertyKey(0x0015);
    /// `min-width`.
    pub const MIN_WIDTH: PropertyKey = PropertyKey(0x0018);
    /// `min-height`.
    pub const MIN_HEIGHT: PropertyKey = PropertyKey(0x0019);
    /// `max-width`.
    pub const MAX_WIDTH: PropertyKey = PropertyKey(0x001a);
    /// `max-height`.
    pub const MAX_HEIGHT: PropertyKey = PropertyKey(0x001b);
    /// `aspect-ratio`.
    pub const ASPECT_RATIO: PropertyKey = PropertyKey(0x001c);
    /// `flex-grow`.
    pub const FLEX_GROW: PropertyKey = PropertyKey(0x001d);
    /// `flex-shrink`.
    pub const FLEX_SHRINK: PropertyKey = PropertyKey(0x001e);
    /// `flex-basis`.
    pub const FLEX_BASIS: PropertyKey = PropertyKey(0x001f);
    /// `align-self`.
    pub const ALIGN_SELF: PropertyKey = PropertyKey(0x0020);
    /// `justify-self`.
    pub const JUSTIFY_SELF: PropertyKey = PropertyKey(0x0021);
    /// `grid-column-start`.
    pub const GRID_COLUMN_START: PropertyKey = PropertyKey(0x0022);
    /// `grid-column-end`.
    pub const GRID_COLUMN_END: PropertyKey = PropertyKey(0x0023);
    /// `grid-row-start`.
    pub const GRID_ROW_START: PropertyKey = PropertyKey(0x0024);
    /// `grid-row-end`.
    pub const GRID_ROW_END: PropertyKey = PropertyKey(0x0025);
    /// payload 크기가 없는 종류(Text, Path, Line, ComponentInstance)의 `width`.
    pub const WIDTH: PropertyKey = PropertyKey(0x0016);
    /// payload 크기가 없는 종류의 `height`.
    pub const HEIGHT: PropertyKey = PropertyKey(0x0017);
}

/// Frame(`0x01xx`) container longhand key.
pub mod frame_keys {
    use stellar_core_types::property::PropertyKey;

    /// `display`.
    pub const DISPLAY: PropertyKey = PropertyKey(0x0110);
    /// `padding-top`.
    pub const PADDING_TOP: PropertyKey = PropertyKey(0x0111);
    /// `padding-right`.
    pub const PADDING_RIGHT: PropertyKey = PropertyKey(0x0112);
    /// `padding-bottom`.
    pub const PADDING_BOTTOM: PropertyKey = PropertyKey(0x0113);
    /// `padding-left`.
    pub const PADDING_LEFT: PropertyKey = PropertyKey(0x0114);
    /// `flex-direction`.
    pub const FLEX_DIRECTION: PropertyKey = PropertyKey(0x0115);
    /// `flex-wrap`.
    pub const FLEX_WRAP: PropertyKey = PropertyKey(0x0116);
    /// `justify-content`.
    pub const JUSTIFY_CONTENT: PropertyKey = PropertyKey(0x0117);
    /// `align-content`.
    pub const ALIGN_CONTENT: PropertyKey = PropertyKey(0x0118);
    /// `align-items`.
    pub const ALIGN_ITEMS: PropertyKey = PropertyKey(0x0119);
    /// `justify-items`.
    pub const JUSTIFY_ITEMS: PropertyKey = PropertyKey(0x011a);
    /// `row-gap`.
    pub const ROW_GAP: PropertyKey = PropertyKey(0x011b);
    /// `column-gap`.
    pub const COLUMN_GAP: PropertyKey = PropertyKey(0x011c);
    /// `grid-template-columns`.
    pub const GRID_TEMPLATE_COLUMNS: PropertyKey = PropertyKey(0x011d);
    /// `grid-template-rows`.
    pub const GRID_TEMPLATE_ROWS: PropertyKey = PropertyKey(0x011e);
    /// `grid-auto-columns`.
    pub const GRID_AUTO_COLUMNS: PropertyKey = PropertyKey(0x011f);
    /// `grid-auto-rows`.
    pub const GRID_AUTO_ROWS: PropertyKey = PropertyKey(0x0120);
    /// `grid-auto-flow`.
    pub const GRID_AUTO_FLOW: PropertyKey = PropertyKey(0x0121);
}

impl LayoutLonghand {
    /// 모든 longhand(번호 오름차순, item 먼저).
    pub const ALL: [LayoutLonghand; 40] = [
        LayoutLonghand::Position,
        LayoutLonghand::BoxSizing,
        LayoutLonghand::Top,
        LayoutLonghand::Right,
        LayoutLonghand::Bottom,
        LayoutLonghand::Left,
        LayoutLonghand::Width,
        LayoutLonghand::Height,
        LayoutLonghand::MinWidth,
        LayoutLonghand::MinHeight,
        LayoutLonghand::MaxWidth,
        LayoutLonghand::MaxHeight,
        LayoutLonghand::AspectRatio,
        LayoutLonghand::FlexGrow,
        LayoutLonghand::FlexShrink,
        LayoutLonghand::FlexBasis,
        LayoutLonghand::AlignSelf,
        LayoutLonghand::JustifySelf,
        LayoutLonghand::GridColumnStart,
        LayoutLonghand::GridColumnEnd,
        LayoutLonghand::GridRowStart,
        LayoutLonghand::GridRowEnd,
        LayoutLonghand::Display,
        LayoutLonghand::PaddingTop,
        LayoutLonghand::PaddingRight,
        LayoutLonghand::PaddingBottom,
        LayoutLonghand::PaddingLeft,
        LayoutLonghand::FlexDirection,
        LayoutLonghand::FlexWrap,
        LayoutLonghand::JustifyContent,
        LayoutLonghand::AlignContent,
        LayoutLonghand::AlignItems,
        LayoutLonghand::JustifyItems,
        LayoutLonghand::RowGap,
        LayoutLonghand::ColumnGap,
        LayoutLonghand::GridTemplateColumns,
        LayoutLonghand::GridTemplateRows,
        LayoutLonghand::GridAutoColumns,
        LayoutLonghand::GridAutoRows,
        LayoutLonghand::GridAutoFlow,
    ];

    /// 저장 번호. node 공통 key 또는 Frame key의 숫자 값과 같습니다(`width`·`height`는 node 공통 key).
    pub fn id(self) -> u16 {
        self.record_key().0 as u16
    }

    /// record에 쓰는 key.
    pub fn record_key(self) -> PropertyKey {
        use LayoutLonghand as L;
        match self {
            L::Position => keys::POSITION,
            L::BoxSizing => keys::BOX_SIZING,
            L::Top => keys::TOP,
            L::Right => keys::RIGHT,
            L::Bottom => keys::BOTTOM,
            L::Left => keys::LEFT,
            L::Width => keys::WIDTH,
            L::Height => keys::HEIGHT,
            L::MinWidth => keys::MIN_WIDTH,
            L::MinHeight => keys::MIN_HEIGHT,
            L::MaxWidth => keys::MAX_WIDTH,
            L::MaxHeight => keys::MAX_HEIGHT,
            L::AspectRatio => keys::ASPECT_RATIO,
            L::FlexGrow => keys::FLEX_GROW,
            L::FlexShrink => keys::FLEX_SHRINK,
            L::FlexBasis => keys::FLEX_BASIS,
            L::AlignSelf => keys::ALIGN_SELF,
            L::JustifySelf => keys::JUSTIFY_SELF,
            L::GridColumnStart => keys::GRID_COLUMN_START,
            L::GridColumnEnd => keys::GRID_COLUMN_END,
            L::GridRowStart => keys::GRID_ROW_START,
            L::GridRowEnd => keys::GRID_ROW_END,
            L::Display => frame_keys::DISPLAY,
            L::PaddingTop => frame_keys::PADDING_TOP,
            L::PaddingRight => frame_keys::PADDING_RIGHT,
            L::PaddingBottom => frame_keys::PADDING_BOTTOM,
            L::PaddingLeft => frame_keys::PADDING_LEFT,
            L::FlexDirection => frame_keys::FLEX_DIRECTION,
            L::FlexWrap => frame_keys::FLEX_WRAP,
            L::JustifyContent => frame_keys::JUSTIFY_CONTENT,
            L::AlignContent => frame_keys::ALIGN_CONTENT,
            L::AlignItems => frame_keys::ALIGN_ITEMS,
            L::JustifyItems => frame_keys::JUSTIFY_ITEMS,
            L::RowGap => frame_keys::ROW_GAP,
            L::ColumnGap => frame_keys::COLUMN_GAP,
            L::GridTemplateColumns => frame_keys::GRID_TEMPLATE_COLUMNS,
            L::GridTemplateRows => frame_keys::GRID_TEMPLATE_ROWS,
            L::GridAutoColumns => frame_keys::GRID_AUTO_COLUMNS,
            L::GridAutoRows => frame_keys::GRID_AUTO_ROWS,
            L::GridAutoFlow => frame_keys::GRID_AUTO_FLOW,
        }
    }

    /// `kind` node에서 이 longhand를 가리키는 속성 key. `width`·`height`는 payload 크기가 있는 종류면 그
    /// payload key입니다(IX.11.22·IX.11.3이 할당한 key).
    pub fn key_for(self, payload: &NodePayload) -> PropertyKey {
        match (self, payload_size_keys(payload)) {
            (LayoutLonghand::Width, Some((width, _))) => width,
            (LayoutLonghand::Height, Some((_, height))) => height,
            _ => self.record_key(),
        }
    }

    /// 번호에서 복원합니다.
    pub fn from_id(id: u16) -> Option<LayoutLonghand> {
        LayoutLonghand::ALL.into_iter().find(|l| l.id() == id)
    }

    /// CSS longhand 이름.
    pub fn name(self) -> &'static str {
        use LayoutLonghand as L;
        match self {
            L::Position => "position",
            L::BoxSizing => "box-sizing",
            L::Top => "top",
            L::Right => "right",
            L::Bottom => "bottom",
            L::Left => "left",
            L::Width => "width",
            L::Height => "height",
            L::MinWidth => "min-width",
            L::MinHeight => "min-height",
            L::MaxWidth => "max-width",
            L::MaxHeight => "max-height",
            L::AspectRatio => "aspect-ratio",
            L::FlexGrow => "flex-grow",
            L::FlexShrink => "flex-shrink",
            L::FlexBasis => "flex-basis",
            L::AlignSelf => "align-self",
            L::JustifySelf => "justify-self",
            L::GridColumnStart => "grid-column-start",
            L::GridColumnEnd => "grid-column-end",
            L::GridRowStart => "grid-row-start",
            L::GridRowEnd => "grid-row-end",
            L::Display => "display",
            L::PaddingTop => "padding-top",
            L::PaddingRight => "padding-right",
            L::PaddingBottom => "padding-bottom",
            L::PaddingLeft => "padding-left",
            L::FlexDirection => "flex-direction",
            L::FlexWrap => "flex-wrap",
            L::JustifyContent => "justify-content",
            L::AlignContent => "align-content",
            L::AlignItems => "align-items",
            L::JustifyItems => "justify-items",
            L::RowGap => "row-gap",
            L::ColumnGap => "column-gap",
            L::GridTemplateColumns => "grid-template-columns",
            L::GridTemplateRows => "grid-template-rows",
            L::GridAutoColumns => "grid-auto-columns",
            L::GridAutoRows => "grid-auto-rows",
            L::GridAutoFlow => "grid-auto-flow",
        }
    }

    /// CSS 이름에서 찾습니다.
    pub fn from_name(name: &str) -> Option<LayoutLonghand> {
        LayoutLonghand::ALL.into_iter().find(|l| l.name() == name)
    }

    /// 범위.
    pub fn role(self) -> LayoutRole {
        if self.id() >= 0x0100 {
            LayoutRole::Container
        } else {
            LayoutRole::Item
        }
    }

    /// 값 문법.
    pub fn grammar(self) -> LayoutGrammar {
        use LayoutGrammar as G;
        use LayoutLonghand as L;
        match self {
            L::Position => G::Keyword(&["static", "relative", "absolute"]),
            L::BoxSizing => G::Keyword(&["content-box", "border-box"]),
            L::Top | L::Right | L::Bottom | L::Left => G::Inset,
            L::Width | L::Height | L::MinWidth | L::MinHeight => G::Size,
            L::MaxWidth | L::MaxHeight => G::MaxSize,
            L::AspectRatio => G::AspectRatio,
            L::FlexGrow | L::FlexShrink => G::NonNegativeNumber,
            L::FlexBasis => G::FlexBasis,
            L::AlignSelf | L::JustifySelf => G::Keyword(SELF_ALIGNMENT),
            L::GridColumnStart | L::GridColumnEnd | L::GridRowStart | L::GridRowEnd => G::GridLine,
            L::Display => G::Keyword(&["block", "flex", "grid", "none"]),
            L::PaddingTop | L::PaddingRight | L::PaddingBottom | L::PaddingLeft => {
                G::NonNegativeLength
            }
            L::FlexDirection => G::Keyword(&["row", "row-reverse", "column", "column-reverse"]),
            L::FlexWrap => G::Keyword(&["nowrap", "wrap", "wrap-reverse"]),
            L::JustifyContent | L::AlignContent => G::Keyword(CONTENT_ALIGNMENT),
            L::AlignItems | L::JustifyItems => G::Keyword(ITEM_ALIGNMENT),
            L::RowGap | L::ColumnGap => G::Gap,
            L::GridTemplateColumns | L::GridTemplateRows => G::TrackList,
            L::GridAutoColumns | L::GridAutoRows => G::AutoTracks,
            L::GridAutoFlow => G::Keyword(&["row", "column"]),
        }
    }

    /// `payload` node가 이 longhand를 가질 수 있는지(record 또는 payload).
    pub fn applies_to(self, kind: NodeKind) -> bool {
        match kind {
            NodeKind::Fragment => false,
            NodeKind::Frame => true,
            _ => self.role() == LayoutRole::Item,
        }
    }
}

impl fmt::Display for LayoutLonghand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// payload 크기가 `width`·`height` 저장 위치인 종류의 (width key, height key).
pub fn payload_size_keys(payload: &NodePayload) -> Option<(PropertyKey, PropertyKey)> {
    use crate::payload_metadata::{image_keys, shape_keys};
    match payload {
        NodePayload::Frame(_) => Some((crate::frame::keys::WIDTH, crate::frame::keys::HEIGHT)),
        NodePayload::Shape(
            ShapeGeometry::Rectangle(_) | ShapeGeometry::Ellipse(_) | ShapeGeometry::Polygon(_),
        ) => Some((shape_keys::WIDTH, shape_keys::HEIGHT)),
        NodePayload::Image(_) => Some((image_keys::WIDTH, image_keys::HEIGHT)),
        _ => None,
    }
}

/// payload 크기(px). [`payload_size_keys`]가 `Some`인 종류만 값이 있습니다.
pub fn payload_size(payload: &NodePayload) -> Option<(f32, f32)> {
    match payload {
        NodePayload::Frame(frame) => Some((frame.size.width(), frame.size.height())),
        NodePayload::Shape(ShapeGeometry::Rectangle(rect)) => {
            Some((rect.size.width(), rect.size.height()))
        }
        NodePayload::Shape(ShapeGeometry::Ellipse(ellipse)) => Some((
            ellipse.bounds.size().width(),
            ellipse.bounds.size().height(),
        )),
        NodePayload::Shape(ShapeGeometry::Polygon(polygon)) => Some((
            polygon.bounds().size().width(),
            polygon.bounds().size().height(),
        )),
        NodePayload::Image(image) => Some((image.size.width(), image.size.height())),
        _ => None,
    }
}

/// grid track breadth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrackBreadth {
    /// `auto`.
    Auto,
    /// `<length>`(px, 비음수).
    Length(f32),
    /// `<percentage>`(비음수).
    Percentage(f32),
    /// `min-content`.
    MinContent,
    /// `max-content`.
    MaxContent,
    /// `<flex>`(`fr`, 비음수). `minmax()`의 첫 인자에는 올 수 없습니다.
    Fraction(f32),
}

/// `<track-size>` 하나.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Track {
    /// breadth 하나.
    Breadth(TrackBreadth),
    /// `minmax(min, max)`.
    MinMax(TrackBreadth, TrackBreadth),
    /// `fit-content(<length> | <percentage>)`.
    FitContent(TrackBreadth),
}

/// longhand 값 하나(정규형).
#[derive(Debug, Clone, PartialEq)]
pub enum LayoutValue {
    /// keyword(longhand keyword 목록의 정규 이름).
    Keyword(&'static str),
    /// `<length>`(px).
    Length(f32),
    /// `<percentage>`.
    Percentage(f32),
    /// `<number>`.
    Number(f32),
    /// `<integer>` grid line.
    Line(i16),
    /// `span <integer>`.
    Span(u16),
    /// `<number> / <number>`.
    Ratio(f32, f32),
    /// track 목록(비어 있지 않음). `none`은 keyword입니다.
    Tracks(Vec<Track>),
}

/// 값 해석 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutValueError {
    /// 안정 오류 code.
    pub code: &'static str,
    /// 설명.
    pub message: String,
}

impl LayoutValueError {
    fn invalid(message: impl Into<String>) -> LayoutValueError {
        LayoutValueError {
            code: "invalid-value",
            message: message.into(),
        }
    }
}

impl fmt::Display for LayoutValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for LayoutValueError {}

impl From<value::CssValueError> for LayoutValueError {
    fn from(error: value::CssValueError) -> LayoutValueError {
        LayoutValueError {
            code: error.code,
            message: error.message,
        }
    }
}

fn number_with(grammar: CssGrammar, text: &str) -> Result<CssValue, LayoutValueError> {
    Ok(value::parse(grammar, text)?)
}

fn length(text: &str, non_negative: bool) -> Result<f32, LayoutValueError> {
    let grammar = if non_negative {
        LENGTH_NON_NEGATIVE
    } else {
        LENGTH_SIGNED
    };
    match number_with(grammar, text)? {
        CssValue::Length(v) => Ok(v + 0.0),
        _ => Err(LayoutValueError::invalid("길이가 아닙니다")),
    }
}

fn percentage(text: &str, non_negative: bool) -> Result<f32, LayoutValueError> {
    match number_with(CssGrammar::Percentage, text)? {
        CssValue::Percentage(v) if non_negative && v < 0.0 => {
            Err(LayoutValueError::invalid("음수는 받지 않습니다"))
        }
        CssValue::Percentage(v) => Ok(v + 0.0),
        _ => Err(LayoutValueError::invalid("백분율이 아닙니다")),
    }
}

fn number(text: &str) -> Result<f32, LayoutValueError> {
    match number_with(CssGrammar::Number, text)? {
        CssValue::Number(v) => Ok(v + 0.0),
        _ => Err(LayoutValueError::invalid("수가 아닙니다")),
    }
}

fn length_or_percentage(text: &str, non_negative: bool) -> Result<LayoutValue, LayoutValueError> {
    if text.ends_with('%') {
        percentage(text, non_negative).map(LayoutValue::Percentage)
    } else {
        length(text, non_negative).map(LayoutValue::Length)
    }
}

fn keyword_of(choices: &'static [&'static str], text: &str) -> Option<&'static str> {
    choices.iter().copied().find(|choice| *choice == text)
}

fn breadth(text: &str, allow_fraction: bool) -> Result<TrackBreadth, LayoutValueError> {
    Ok(match text {
        "auto" => TrackBreadth::Auto,
        "min-content" => TrackBreadth::MinContent,
        "max-content" => TrackBreadth::MaxContent,
        _ if text.ends_with("fr") => {
            if !allow_fraction {
                return Err(LayoutValueError::invalid(
                    "minmax() 첫 인자에는 fr을 쓸 수 없습니다",
                ));
            }
            let value = number(&text[..text.len() - 2])?;
            if value < 0.0 {
                return Err(LayoutValueError::invalid("fr은 음수일 수 없습니다"));
            }
            TrackBreadth::Fraction(value)
        }
        _ => match length_or_percentage(text, true)? {
            LayoutValue::Length(v) => TrackBreadth::Length(v),
            LayoutValue::Percentage(v) => TrackBreadth::Percentage(v),
            _ => unreachable!("길이 또는 백분율"),
        },
    })
}

fn function_body<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    text.strip_prefix(name)?
        .strip_prefix('(')?
        .strip_suffix(')')
        .map(str::trim)
}

fn track(text: &str) -> Result<Track, LayoutValueError> {
    if let Some(body) = function_body(text, "minmax") {
        let (min, max) = body
            .split_once(',')
            .ok_or_else(|| LayoutValueError::invalid("minmax()에는 인자가 두 개 필요합니다"))?;
        return Ok(Track::MinMax(
            breadth(min.trim(), false)?,
            breadth(max.trim(), true)?,
        ));
    }
    if let Some(body) = function_body(text, "fit-content") {
        return match length_or_percentage(body, true)? {
            LayoutValue::Length(v) => Ok(Track::FitContent(TrackBreadth::Length(v))),
            LayoutValue::Percentage(v) => Ok(Track::FitContent(TrackBreadth::Percentage(v))),
            _ => unreachable!("길이 또는 백분율"),
        };
    }
    breadth(text, true).map(Track::Breadth)
}

/// track 목록을 공백으로 나눕니다(함수 괄호 안의 공백은 나누지 않음).
fn split_tracks(text: &str) -> Result<Vec<&str>, LayoutValueError> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = None;
    for (index, c) in text.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| LayoutValueError::invalid("괄호가 맞지 않습니다"))?;
            }
            c if c.is_whitespace() && depth == 0 => {
                if let Some(s) = start.take() {
                    parts.push(&text[s..index]);
                }
                continue;
            }
            _ => {}
        }
        if start.is_none() {
            start = Some(index);
        }
    }
    if depth != 0 {
        return Err(LayoutValueError::invalid("괄호가 맞지 않습니다"));
    }
    if let Some(s) = start {
        parts.push(&text[s..]);
    }
    Ok(parts)
}

/// track 목록 상한.
pub const MAX_TRACKS: usize = 256;

impl LayoutLonghand {
    /// CSS 값 텍스트를 정규 값으로 해석합니다. 수 부분은 [`crate::css::value::parse`]를 거치므로 `var()`,
    /// 계산식, 전역 keyword, 지원하지 않는 단위는 같은 code로 거부합니다.
    pub fn parse(self, text: &str) -> Result<LayoutValue, LayoutValueError> {
        if text.len() > value::MAX_INPUT_LEN {
            return Err(LayoutValueError::invalid("값이 너무 깁니다"));
        }
        let lower = text.trim().to_ascii_lowercase();
        // var()·계산식·전역 keyword 거부는 value parser 규칙을 그대로 씁니다.
        if lower.contains("var(")
            || ["calc(", "min(", "max(", "clamp("]
                .iter()
                .any(|f| lower.starts_with(f))
            || ["inherit", "initial", "unset", "revert", "revert-layer"].contains(&lower.as_str())
        {
            return Err(value::parse(CssGrammar::Number, &lower)
                .err()
                .map(LayoutValueError::from)
                .unwrap_or_else(|| LayoutValueError::invalid("받지 않는 값입니다")));
        }
        let text = lower.as_str();
        use LayoutGrammar as G;
        match self.grammar() {
            G::Keyword(choices) => keyword_of(choices, text)
                .map(LayoutValue::Keyword)
                .ok_or_else(|| LayoutValueError {
                    code: "unsupported-value",
                    message: format!("{}: {} 중 하나를 넣으세요", self.name(), choices.join(", ")),
                }),
            G::Inset => match text {
                "auto" => Ok(LayoutValue::Keyword("auto")),
                _ => length_or_percentage(text, false),
            },
            G::Size => {
                match keyword_of(&["auto", "min-content", "max-content", "fit-content"], text) {
                    Some(k) => Ok(LayoutValue::Keyword(k)),
                    None => length_or_percentage(text, true),
                }
            }
            G::MaxSize => {
                match keyword_of(&["none", "min-content", "max-content", "fit-content"], text) {
                    Some(k) => Ok(LayoutValue::Keyword(k)),
                    None => length_or_percentage(text, true),
                }
            }
            G::AspectRatio => {
                if text == "auto" {
                    return Ok(LayoutValue::Keyword("auto"));
                }
                let (w, h) = match text.split_once('/') {
                    Some((w, h)) => (number(w.trim())?, number(h.trim())?),
                    None => (number(text)?, 1.0),
                };
                if w <= 0.0 || h <= 0.0 {
                    return Err(LayoutValueError::invalid(
                        "aspect-ratio 성분은 양수여야 합니다",
                    ));
                }
                Ok(LayoutValue::Ratio(w, h))
            }
            G::NonNegativeNumber => {
                let v = number(text)?;
                if v < 0.0 {
                    return Err(LayoutValueError::invalid("음수는 받지 않습니다"));
                }
                Ok(LayoutValue::Number(v))
            }
            G::FlexBasis => match keyword_of(&["auto", "content"], text) {
                Some(k) => Ok(LayoutValue::Keyword(k)),
                None => length_or_percentage(text, true),
            },
            G::GridLine => {
                if text == "auto" {
                    return Ok(LayoutValue::Keyword("auto"));
                }
                if let Some(rest) = text.strip_prefix("span") {
                    let n: u16 = rest.trim().parse().map_err(|_| {
                        LayoutValueError::invalid("span 뒤에는 1 이상 정수가 옵니다")
                    })?;
                    if n == 0 {
                        return Err(LayoutValueError::invalid("span은 1 이상입니다"));
                    }
                    return Ok(LayoutValue::Span(n));
                }
                let n: i16 = text.parse().map_err(|_| {
                    LayoutValueError::invalid("grid line은 auto, 정수, span <정수>입니다")
                })?;
                if n == 0 {
                    return Err(LayoutValueError::invalid("grid line 0은 없습니다"));
                }
                Ok(LayoutValue::Line(n))
            }
            G::NonNegativeLength => length(text, true).map(LayoutValue::Length),
            G::Gap => match text {
                "normal" => Ok(LayoutValue::Keyword("normal")),
                _ => length(text, true).map(LayoutValue::Length),
            },
            G::TrackList | G::AutoTracks => {
                if text == "none" && self.grammar() == G::TrackList {
                    return Ok(LayoutValue::Keyword("none"));
                }
                let mut tracks = Vec::new();
                for part in split_tracks(text)? {
                    // `repeat(<n>, <track-size>+)`는 펼친 목록이 정규형입니다(정수 반복만 받음).
                    if let Some(body) = function_body(part, "repeat") {
                        let (count, inner) = body.split_once(',').ok_or_else(|| {
                            LayoutValueError::invalid("repeat()에는 반복 수와 track이 필요합니다")
                        })?;
                        let count: usize = count.trim().parse().map_err(|_| LayoutValueError {
                            code: "unsupported-value",
                            message: "repeat()는 1 이상 정수 반복만 받습니다".into(),
                        })?;
                        let inner = split_tracks(inner.trim())?;
                        if count == 0 || inner.is_empty() || count * inner.len() > MAX_TRACKS {
                            return Err(LayoutValueError::invalid(
                                "repeat() 반복 수가 범위 밖입니다",
                            ));
                        }
                        for _ in 0..count {
                            for item in &inner {
                                tracks.push(track(item)?);
                            }
                        }
                    } else {
                        tracks.push(track(part)?);
                    }
                    if tracks.len() > MAX_TRACKS {
                        return Err(LayoutValueError::invalid("track이 너무 많습니다"));
                    }
                }
                if tracks.is_empty() {
                    return Err(LayoutValueError::invalid("track이 비어 있습니다"));
                }
                Ok(LayoutValue::Tracks(tracks))
            }
        }
    }

    /// 정규 값이 이 longhand 문법에 맞는지(binary decode·세트 표 검사).
    pub fn accepts(self, value: &LayoutValue) -> bool {
        self.parse(&format_value(value)).as_ref() == Ok(value)
    }
}

fn format_breadth(breadth: &TrackBreadth) -> String {
    match breadth {
        TrackBreadth::Auto => "auto".into(),
        TrackBreadth::Length(v) => format_length(*v),
        TrackBreadth::Percentage(v) => format_percentage(*v),
        TrackBreadth::MinContent => "min-content".into(),
        TrackBreadth::MaxContent => "max-content".into(),
        TrackBreadth::Fraction(v) => format!("{}fr", format_number(*v)),
    }
}

fn format_track(track: &Track) -> String {
    match track {
        Track::Breadth(b) => format_breadth(b),
        Track::MinMax(min, max) => {
            format!("minmax({}, {})", format_breadth(min), format_breadth(max))
        }
        Track::FitContent(limit) => format!("fit-content({})", format_breadth(limit)),
    }
}

/// 정규 CSS 값 텍스트. 수 표기는 [`crate::css::value`] formatter입니다.
pub fn format_value(value: &LayoutValue) -> String {
    match value {
        LayoutValue::Keyword(k) => (*k).to_string(),
        LayoutValue::Length(v) => format_length(*v),
        LayoutValue::Percentage(v) => format_percentage(*v),
        LayoutValue::Number(v) => format_number(*v),
        LayoutValue::Line(n) => n.to_string(),
        LayoutValue::Span(n) => format!("span {n}"),
        LayoutValue::Ratio(w, h) => format!("{} / {}", format_number(*w), format_number(*h)),
        LayoutValue::Tracks(tracks) => tracks
            .iter()
            .map(format_track)
            .collect::<Vec<_>>()
            .join(" "),
    }
}

impl fmt::Display for LayoutValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&format_value(self))
    }
}

/// layout record 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutRecordError {
    /// binary 형식 위반.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
    /// 모르는 longhand 번호.
    UnknownLonghand {
        /// 번호.
        id: u16,
    },
    /// 값이 longhand 문법에 맞지 않습니다.
    Value {
        /// longhand.
        longhand: LayoutLonghand,
        /// 원인.
        error: LayoutValueError,
    },
}

impl fmt::Display for LayoutRecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LayoutRecordError::Malformed { reason } => {
                write!(f, "layout record 형식 위반: {reason}")
            }
            LayoutRecordError::UnknownLonghand { id } => {
                write!(f, "모르는 layout longhand 번호 {id:#06x}")
            }
            LayoutRecordError::Value { longhand, error } => write!(f, "{longhand}: {error}"),
        }
    }
}

impl std::error::Error for LayoutRecordError {}

/// node 하나의 명시 layout longhand.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutRecord {
    /// 소유 node.
    pub node: StableId,
    values: BTreeMap<LayoutLonghand, LayoutValue>,
}

impl LayoutRecord {
    /// 명시 값이 없는 record.
    pub fn new(node: StableId) -> LayoutRecord {
        LayoutRecord {
            node,
            values: BTreeMap::new(),
        }
    }

    /// 명시 값 하나를 둡니다. 이전 명시 값을 돌려줍니다. 문법에 맞지 않으면 거부합니다.
    pub fn set(
        &mut self,
        longhand: LayoutLonghand,
        value: LayoutValue,
    ) -> Result<Option<LayoutValue>, LayoutRecordError> {
        if !longhand.accepts(&value) {
            return Err(LayoutRecordError::Value {
                longhand,
                error: LayoutValueError::invalid(format!("{value}은 정규 값이 아닙니다")),
            });
        }
        Ok(self.values.insert(longhand, value))
    }

    /// 명시 값을 지웁니다(미설정). 이전 값을 돌려줍니다.
    pub fn unset(&mut self, longhand: LayoutLonghand) -> Option<LayoutValue> {
        self.values.remove(&longhand)
    }

    /// 명시 값.
    pub fn get(&self, longhand: LayoutLonghand) -> Option<&LayoutValue> {
        self.values.get(&longhand)
    }

    /// 명시 값 목록(longhand 번호 순).
    pub fn iter(&self) -> impl Iterator<Item = (LayoutLonghand, &LayoutValue)> {
        self.values.iter().map(|(k, v)| (*k, v))
    }

    /// 명시 값이 없는지.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// 명시 값 수.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// canonical bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.node.to_bytes());
        out.extend_from_slice(&(self.values.len() as u16).to_le_bytes());
        for (longhand, value) in &self.values {
            out.extend_from_slice(&longhand.id().to_le_bytes());
            encode_value(&mut out, *longhand, value);
        }
        out
    }

    /// [`LayoutRecord::encode`]를 읽습니다. 순서·중복·문법을 검사합니다.
    pub fn decode(bytes: &[u8]) -> Result<LayoutRecord, LayoutRecordError> {
        let mut reader = Reader { bytes, pos: 0 };
        let mut id = [0u8; 16];
        id.copy_from_slice(reader.take(16)?);
        let mut record = LayoutRecord::new(StableId::from_bytes(id));
        let count = reader.u16()?;
        let mut last: Option<u16> = None;
        for _ in 0..count {
            let number = reader.u16()?;
            if last.is_some_and(|l| l >= number) {
                return Err(LayoutRecordError::Malformed {
                    reason: "longhand 번호가 오름차순이 아닙니다",
                });
            }
            last = Some(number);
            let longhand = LayoutLonghand::from_id(number)
                .ok_or(LayoutRecordError::UnknownLonghand { id: number })?;
            let value = decode_value(&mut reader, longhand)?;
            record.set(longhand, value)?;
        }
        if reader.pos != bytes.len() {
            return Err(LayoutRecordError::Malformed {
                reason: "trailing bytes",
            });
        }
        Ok(record)
    }
}

const TAG_KEYWORD: u8 = 0;
const TAG_LENGTH: u8 = 1;
const TAG_PERCENTAGE: u8 = 2;
const TAG_NUMBER: u8 = 3;
const TAG_LINE: u8 = 4;
const TAG_SPAN: u8 = 5;
const TAG_RATIO: u8 = 6;
const TAG_TRACKS: u8 = 7;

fn put_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&(value + 0.0).to_le_bytes());
}

fn keyword_index(longhand: LayoutLonghand, keyword: &str) -> u8 {
    let list: &[&str] = match longhand.grammar() {
        LayoutGrammar::Keyword(choices) => choices,
        _ => GENERAL_KEYWORDS,
    };
    list.iter()
        .position(|k| *k == keyword)
        .unwrap_or(u8::MAX as usize) as u8
}

/// keyword 목록이 따로 없는 문법의 keyword(`auto`, `none`, `normal` …) 저장 번호표.
const GENERAL_KEYWORDS: &[&str] = &[
    "auto",
    "none",
    "normal",
    "content",
    "min-content",
    "max-content",
    "fit-content",
];

fn keyword_at(longhand: LayoutLonghand, index: u8) -> Option<&'static str> {
    let list: &'static [&'static str] = match longhand.grammar() {
        LayoutGrammar::Keyword(choices) => choices,
        _ => GENERAL_KEYWORDS,
    };
    list.get(index as usize).copied()
}

fn encode_breadth(out: &mut Vec<u8>, breadth: &TrackBreadth) {
    match breadth {
        TrackBreadth::Auto => out.push(0),
        TrackBreadth::Length(v) => {
            out.push(1);
            put_f32(out, *v);
        }
        TrackBreadth::Percentage(v) => {
            out.push(2);
            put_f32(out, *v);
        }
        TrackBreadth::MinContent => out.push(3),
        TrackBreadth::MaxContent => out.push(4),
        TrackBreadth::Fraction(v) => {
            out.push(5);
            put_f32(out, *v);
        }
    }
}

fn encode_value(out: &mut Vec<u8>, longhand: LayoutLonghand, value: &LayoutValue) {
    match value {
        LayoutValue::Keyword(k) => {
            out.push(TAG_KEYWORD);
            out.push(keyword_index(longhand, k));
        }
        LayoutValue::Length(v) => {
            out.push(TAG_LENGTH);
            put_f32(out, *v);
        }
        LayoutValue::Percentage(v) => {
            out.push(TAG_PERCENTAGE);
            put_f32(out, *v);
        }
        LayoutValue::Number(v) => {
            out.push(TAG_NUMBER);
            put_f32(out, *v);
        }
        LayoutValue::Line(n) => {
            out.push(TAG_LINE);
            out.extend_from_slice(&n.to_le_bytes());
        }
        LayoutValue::Span(n) => {
            out.push(TAG_SPAN);
            out.extend_from_slice(&n.to_le_bytes());
        }
        LayoutValue::Ratio(w, h) => {
            out.push(TAG_RATIO);
            put_f32(out, *w);
            put_f32(out, *h);
        }
        LayoutValue::Tracks(tracks) => {
            out.push(TAG_TRACKS);
            out.extend_from_slice(&(tracks.len() as u16).to_le_bytes());
            for track in tracks {
                match track {
                    Track::Breadth(b) => {
                        out.push(0);
                        encode_breadth(out, b);
                    }
                    Track::MinMax(min, max) => {
                        out.push(1);
                        encode_breadth(out, min);
                        encode_breadth(out, max);
                    }
                    Track::FitContent(limit) => {
                        out.push(2);
                        encode_breadth(out, limit);
                    }
                }
            }
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], LayoutRecordError> {
        if self.bytes.len() - self.pos < n {
            return Err(LayoutRecordError::Malformed {
                reason: "입력 부족",
            });
        }
        let slice = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, LayoutRecordError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, LayoutRecordError> {
        Ok(u16::from_le_bytes(
            self.take(2)?.try_into().expect("2바이트"),
        ))
    }

    fn f32(&mut self) -> Result<f32, LayoutRecordError> {
        let value = f32::from_le_bytes(self.take(4)?.try_into().expect("4바이트"));
        if !value.is_finite() {
            return Err(LayoutRecordError::Malformed {
                reason: "유한하지 않은 수",
            });
        }
        if value.to_bits() == (-0.0f32).to_bits() {
            return Err(LayoutRecordError::Malformed {
                reason: "-0은 0으로 씁니다",
            });
        }
        Ok(value)
    }

    fn breadth(&mut self) -> Result<TrackBreadth, LayoutRecordError> {
        Ok(match self.u8()? {
            0 => TrackBreadth::Auto,
            1 => TrackBreadth::Length(self.f32()?),
            2 => TrackBreadth::Percentage(self.f32()?),
            3 => TrackBreadth::MinContent,
            4 => TrackBreadth::MaxContent,
            5 => TrackBreadth::Fraction(self.f32()?),
            _ => {
                return Err(LayoutRecordError::Malformed {
                    reason: "모르는 track breadth tag",
                })
            }
        })
    }
}

fn decode_value(
    reader: &mut Reader<'_>,
    longhand: LayoutLonghand,
) -> Result<LayoutValue, LayoutRecordError> {
    Ok(match reader.u8()? {
        TAG_KEYWORD => {
            let index = reader.u8()?;
            LayoutValue::Keyword(keyword_at(longhand, index).ok_or(
                LayoutRecordError::Malformed {
                    reason: "모르는 keyword 번호",
                },
            )?)
        }
        TAG_LENGTH => LayoutValue::Length(reader.f32()?),
        TAG_PERCENTAGE => LayoutValue::Percentage(reader.f32()?),
        TAG_NUMBER => LayoutValue::Number(reader.f32()?),
        TAG_LINE => LayoutValue::Line(i16::from_le_bytes(
            reader.take(2)?.try_into().expect("2바이트"),
        )),
        TAG_SPAN => LayoutValue::Span(reader.u16()?),
        TAG_RATIO => LayoutValue::Ratio(reader.f32()?, reader.f32()?),
        TAG_TRACKS => {
            let count = reader.u16()? as usize;
            if count == 0 || count > MAX_TRACKS {
                return Err(LayoutRecordError::Malformed {
                    reason: "track 수"
                });
            }
            let mut tracks = Vec::with_capacity(count);
            for _ in 0..count {
                tracks.push(match reader.u8()? {
                    0 => Track::Breadth(reader.breadth()?),
                    1 => Track::MinMax(reader.breadth()?, reader.breadth()?),
                    2 => Track::FitContent(reader.breadth()?),
                    _ => {
                        return Err(LayoutRecordError::Malformed {
                            reason: "모르는 track tag",
                        })
                    }
                });
            }
            LayoutValue::Tracks(tracks)
        }
        _ => {
            return Err(LayoutRecordError::Malformed {
                reason: "모르는 값 tag",
            })
        }
    })
}

/// record 목록 검증 위반.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutRecordViolation {
    /// 소유 node가 문서에 없습니다.
    OwnerMissing {
        /// 소유 node.
        node: StableId,
    },
    /// 같은 node의 record가 두 번 있습니다.
    Duplicate {
        /// 소유 node.
        node: StableId,
    },
    /// 명시 값이 없는 record를 저장했습니다(빈 record는 저장하지 않음).
    Empty {
        /// 소유 node.
        node: StableId,
    },
    /// longhand가 이 node 종류에 붙지 않습니다(Fragment, Frame이 아닌 node의 container longhand).
    NotApplicable {
        /// 소유 node.
        node: StableId,
        /// longhand.
        longhand: LayoutLonghand,
    },
    /// payload 크기가 저장 위치인 종류에 `width`·`height`를 두었습니다.
    SizeInPayload {
        /// 소유 node.
        node: StableId,
        /// longhand.
        longhand: LayoutLonghand,
    },
}

impl fmt::Display for LayoutRecordViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LayoutRecordViolation::OwnerMissing { node } => {
                write!(f, "layout record의 node {node}가 없습니다")
            }
            LayoutRecordViolation::Duplicate { node } => {
                write!(f, "node {node}의 layout record가 두 번 있습니다")
            }
            LayoutRecordViolation::Empty { node } => {
                write!(f, "node {node}의 layout record에 명시 값이 없습니다")
            }
            LayoutRecordViolation::NotApplicable { node, longhand } => {
                write!(f, "node {node}에는 {longhand}를 둘 수 없습니다")
            }
            LayoutRecordViolation::SizeInPayload { node, longhand } => {
                write!(f, "node {node}의 {longhand}는 payload 크기에 저장합니다")
            }
        }
    }
}

/// record 목록을 검사합니다. `payload_of`는 node의 payload를 찾습니다.
pub fn check_records<'a>(
    records: &[LayoutRecord],
    payload_of: impl Fn(StableId) -> Option<&'a NodePayload>,
) -> Vec<LayoutRecordViolation> {
    let mut violations = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for record in records {
        if !seen.insert(record.node) {
            violations.push(LayoutRecordViolation::Duplicate { node: record.node });
            continue;
        }
        if record.is_empty() {
            violations.push(LayoutRecordViolation::Empty { node: record.node });
        }
        let Some(payload) = payload_of(record.node) else {
            violations.push(LayoutRecordViolation::OwnerMissing { node: record.node });
            continue;
        };
        violations.extend(record_violations(record, payload));
    }
    violations
}

/// record 하나를 node payload에 대해 검사합니다.
pub fn record_violations(
    record: &LayoutRecord,
    payload: &NodePayload,
) -> Vec<LayoutRecordViolation> {
    let mut violations = Vec::new();
    let has_payload_size = payload_size_keys(payload).is_some();
    for (longhand, _) in record.iter() {
        if !longhand.applies_to(payload.kind()) {
            violations.push(LayoutRecordViolation::NotApplicable {
                node: record.node,
                longhand,
            });
        } else if has_payload_size
            && matches!(longhand, LayoutLonghand::Width | LayoutLonghand::Height)
        {
            violations.push(LayoutRecordViolation::SizeInPayload {
                node: record.node,
                longhand,
            });
        }
    }
    violations
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fragment::FragmentPayload;
    use crate::frame::FramePayload;
    use crate::text::TextPayload;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    #[test]
    fn longhand_numbers_and_names_are_unique_and_golden() {
        let mut ids = std::collections::BTreeSet::new();
        let mut names = std::collections::BTreeSet::new();
        for longhand in LayoutLonghand::ALL {
            assert!(ids.insert(longhand.id()), "번호 중복 {longhand}");
            assert!(names.insert(longhand.name()), "이름 중복 {longhand}");
            assert_eq!(LayoutLonghand::from_id(longhand.id()), Some(longhand));
            assert_eq!(LayoutLonghand::from_name(longhand.name()), Some(longhand));
            assert!(
                crate::css::mapping::is_known_longhand(longhand.name()),
                "{longhand}는 CSS longhand inventory에 있어야 합니다"
            );
        }
        let numbers: Vec<u16> = LayoutLonghand::ALL.iter().map(|l| l.id()).collect();
        let mut sorted = numbers.clone();
        sorted.sort_unstable();
        assert_eq!(numbers, sorted, "ALL은 번호 순");
        let golden: Vec<String> = LayoutLonghand::ALL
            .iter()
            .map(|l| format!("{:#06x} {}", l.id(), l.name()))
            .collect();
        assert_eq!(golden.first().unwrap(), "0x0010 position");
        assert_eq!(golden.last().unwrap(), "0x0121 grid-auto-flow");
        assert_eq!(LayoutLonghand::Width.id(), 0x0016);
        assert_eq!(LayoutLonghand::Display.role(), LayoutRole::Container);
        assert_eq!(LayoutLonghand::Left.role(), LayoutRole::Item);
    }

    #[test]
    fn width_height_key_is_payload_key_for_sized_kinds() {
        use crate::payload_metadata::image_keys;
        let frame = NodePayload::Frame(FramePayload::default());
        assert_eq!(
            LayoutLonghand::Width.key_for(&frame),
            crate::frame::keys::WIDTH
        );
        assert_eq!(
            LayoutLonghand::Height.key_for(&frame),
            crate::frame::keys::HEIGHT
        );
        let image = NodePayload::Image(crate::image::ImagePayload::new(
            stellar_core_types::reference::AssetRef::new(sid(9)),
            stellar_core_types::value::Size2::new(1.0, 1.0).unwrap(),
        ));
        assert_eq!(LayoutLonghand::Width.key_for(&image), image_keys::WIDTH);
        let text = NodePayload::Text(TextPayload::empty(0));
        assert_eq!(LayoutLonghand::Width.key_for(&text), keys::WIDTH);
        assert_eq!(LayoutLonghand::Left.key_for(&frame), keys::LEFT);
    }

    #[test]
    fn values_parse_to_canonical_text() {
        let cases: &[(LayoutLonghand, &str, &str)] = &[
            (LayoutLonghand::Display, "FLEX", "flex"),
            (LayoutLonghand::Left, "-12.5px", "-12.5px"),
            (LayoutLonghand::Left, "0px", "0px"),
            (LayoutLonghand::Top, "50%", "50%"),
            (LayoutLonghand::Width, "auto", "auto"),
            (LayoutLonghand::Width, "fit-content", "fit-content"),
            (LayoutLonghand::MaxWidth, "none", "none"),
            (LayoutLonghand::AspectRatio, "16/9", "16 / 9"),
            (LayoutLonghand::AspectRatio, "2", "2 / 1"),
            (LayoutLonghand::FlexGrow, "1", "1"),
            (LayoutLonghand::FlexBasis, "content", "content"),
            (LayoutLonghand::GridColumnStart, "span 2", "span 2"),
            (LayoutLonghand::GridRowEnd, "-1", "-1"),
            (LayoutLonghand::RowGap, "normal", "normal"),
            (
                LayoutLonghand::GridTemplateColumns,
                "1fr minmax(0px,  2fr) fit-content(40%) 100px",
                "1fr minmax(0px, 2fr) fit-content(40%) 100px",
            ),
            (LayoutLonghand::GridTemplateRows, "none", "none"),
            (
                LayoutLonghand::GridTemplateRows,
                "repeat(2, 10px 1fr) auto",
                "10px 1fr 10px 1fr auto",
            ),
            (LayoutLonghand::BoxSizing, "border-box", "border-box"),
        ];
        for (longhand, input, canonical) in cases {
            let value = longhand
                .parse(input)
                .unwrap_or_else(|e| panic!("{longhand} {input}: {e}"));
            assert_eq!(format_value(&value), *canonical, "{longhand} {input}");
            assert_eq!(longhand.parse(canonical).unwrap(), value, "정규형 재해석");
        }
    }

    #[test]
    fn invalid_values_are_rejected_with_value_codes() {
        let cases: &[(LayoutLonghand, &str)] = &[
            (LayoutLonghand::Display, "inline"),
            (LayoutLonghand::Width, "-1px"),
            (LayoutLonghand::Width, "1em"),
            (LayoutLonghand::Left, "var(--x)"),
            (LayoutLonghand::Left, "calc(1px + 2px)"),
            (LayoutLonghand::Left, "inherit"),
            (LayoutLonghand::GridColumnStart, "0"),
            (LayoutLonghand::GridColumnStart, "span 0"),
            (LayoutLonghand::GridAutoRows, "none"),
            (LayoutLonghand::GridTemplateColumns, "minmax(1fr, 2fr)"),
            (LayoutLonghand::AspectRatio, "0 / 1"),
            (LayoutLonghand::FlexShrink, "-1"),
            (LayoutLonghand::PaddingTop, "10%"),
            (LayoutLonghand::GridTemplateRows, "repeat(auto-fill, 10px)"),
            (LayoutLonghand::GridTemplateRows, "repeat(0, 10px)"),
            (LayoutLonghand::GridTemplateRows, "repeat(300, 10px)"),
        ];
        for (longhand, input) in cases {
            assert!(longhand.parse(input).is_err(), "{longhand} {input}은 거부");
        }
        assert_eq!(
            LayoutLonghand::Left.parse("var(--x)").unwrap_err().code,
            "unknown-token"
        );
        assert_eq!(
            LayoutLonghand::Left.parse("inherit").unwrap_err().code,
            "unsupported-value"
        );
    }

    /// 엔진(`stellar_layout::declared`)과 공유하는 정규 값 fixture. 모든 longhand의 대표 값을 이 formatter로 쓴
    /// 결과가 파일과 같아야 하고, 엔진 test는 같은 파일의 모든 줄을 받아야 합니다(ADR-0086 결정 8).
    #[test]
    fn declared_values_fixture_matches_formatter() {
        let samples: &[(LayoutLonghand, &[&str])] = &[
            (
                LayoutLonghand::Position,
                &["static", "relative", "absolute"],
            ),
            (LayoutLonghand::BoxSizing, &["content-box", "border-box"]),
            (LayoutLonghand::Top, &["auto", "-12.5px", "50%", "0px"]),
            (LayoutLonghand::Right, &["auto", "3px"]),
            (LayoutLonghand::Bottom, &["auto", "-10%"]),
            (LayoutLonghand::Left, &["auto", "0.25px"]),
            (
                LayoutLonghand::Width,
                &[
                    "auto",
                    "120px",
                    "50%",
                    "min-content",
                    "max-content",
                    "fit-content",
                ],
            ),
            (LayoutLonghand::Height, &["auto", "0.5px", "100%"]),
            (
                LayoutLonghand::MinWidth,
                &["auto", "0px", "10%", "min-content"],
            ),
            (LayoutLonghand::MinHeight, &["auto", "0px"]),
            (
                LayoutLonghand::MaxWidth,
                &["none", "300px", "75%", "max-content", "fit-content"],
            ),
            (LayoutLonghand::MaxHeight, &["none", "1px"]),
            (
                LayoutLonghand::AspectRatio,
                &["auto", "16/9", "2", "0.5 / 1"],
            ),
            (LayoutLonghand::FlexGrow, &["0", "1", "2.5"]),
            (LayoutLonghand::FlexShrink, &["0", "1"]),
            (
                LayoutLonghand::FlexBasis,
                &["auto", "content", "0px", "40%"],
            ),
            (
                LayoutLonghand::AlignSelf,
                &["auto", "center", "baseline", "stretch", "flex-end"],
            ),
            (
                LayoutLonghand::JustifySelf,
                &["auto", "start", "end", "normal"],
            ),
            (
                LayoutLonghand::GridColumnStart,
                &["auto", "1", "-1", "span 2"],
            ),
            (LayoutLonghand::GridColumnEnd, &["auto", "3"]),
            (LayoutLonghand::GridRowStart, &["auto", "span 1"]),
            (LayoutLonghand::GridRowEnd, &["auto", "-2"]),
            (LayoutLonghand::Display, &["block", "flex", "grid", "none"]),
            (LayoutLonghand::PaddingTop, &["0px", "8px"]),
            (LayoutLonghand::PaddingRight, &["1.5px"]),
            (LayoutLonghand::PaddingBottom, &["0px"]),
            (LayoutLonghand::PaddingLeft, &["16px"]),
            (
                LayoutLonghand::FlexDirection,
                &["row", "row-reverse", "column", "column-reverse"],
            ),
            (
                LayoutLonghand::FlexWrap,
                &["nowrap", "wrap", "wrap-reverse"],
            ),
            (
                LayoutLonghand::JustifyContent,
                &[
                    "normal",
                    "flex-start",
                    "center",
                    "space-between",
                    "space-evenly",
                ],
            ),
            (
                LayoutLonghand::AlignContent,
                &["normal", "flex-start", "stretch", "space-around"],
            ),
            (
                LayoutLonghand::AlignItems,
                &["normal", "stretch", "center", "baseline"],
            ),
            (LayoutLonghand::JustifyItems, &["normal", "start", "end"]),
            (LayoutLonghand::RowGap, &["normal", "8px"]),
            (LayoutLonghand::ColumnGap, &["normal", "0px", "12.5px"]),
            (
                LayoutLonghand::GridTemplateColumns,
                &[
                    "none",
                    "1fr 2fr",
                    "100px auto min-content max-content",
                    "minmax(0px, 1fr) minmax(auto, 200px) minmax(10%, max-content)",
                    "fit-content(40%) fit-content(120px)",
                    "25%",
                ],
            ),
            (LayoutLonghand::GridTemplateRows, &["none", "auto 1fr"]),
            (LayoutLonghand::GridAutoColumns, &["auto", "1fr"]),
            (
                LayoutLonghand::GridAutoRows,
                &["auto", "minmax(min-content, 1fr) 20px"],
            ),
            (LayoutLonghand::GridAutoFlow, &["row", "column"]),
        ];
        let covered: std::collections::BTreeSet<_> = samples.iter().map(|(l, _)| *l).collect();
        assert_eq!(covered.len(), LayoutLonghand::ALL.len(), "모든 longhand");
        let mut text = String::from(
            "# 문서 formatter가 쓴 정규 값(layout_record.rs declared_values_fixture_matches_formatter가 생성·검사).\n",
        );
        for (longhand, inputs) in samples {
            for input in *inputs {
                let value = longhand
                    .parse(input)
                    .unwrap_or_else(|e| panic!("{longhand} {input}: {e}"));
                text.push_str(&format!("{}: {}\n", longhand.name(), format_value(&value)));
            }
        }
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../layout/tests/fixtures/declared_values.txt"
        );
        if std::env::var_os("STELLAR_UPDATE_FIXTURES").is_some() {
            std::fs::write(path, &text).unwrap();
        }
        let fixture = std::fs::read_to_string(path).unwrap_or_default();
        assert_eq!(fixture, text, "STELLAR_UPDATE_FIXTURES=1로 다시 생성");
    }

    fn sample() -> LayoutRecord {
        let mut record = LayoutRecord::new(sid(1));
        for (longhand, text) in [
            (LayoutLonghand::Display, "grid"),
            (LayoutLonghand::Position, "absolute"),
            (LayoutLonghand::Left, "-3px"),
            (LayoutLonghand::Top, "25%"),
            (LayoutLonghand::FlexGrow, "2"),
            (LayoutLonghand::AspectRatio, "4 / 3"),
            (LayoutLonghand::GridColumnStart, "span 3"),
            (LayoutLonghand::GridRowStart, "-2"),
            (LayoutLonghand::MaxWidth, "max-content"),
            (LayoutLonghand::RowGap, "normal"),
            (
                LayoutLonghand::GridTemplateColumns,
                "auto minmax(min-content, 1fr) fit-content(20px) 10%",
            ),
        ] {
            record.set(longhand, longhand.parse(text).unwrap()).unwrap();
        }
        record
    }

    #[test]
    fn record_round_trips_with_canonical_bytes_golden() {
        let record = sample();
        let bytes = record.encode();
        let decoded = LayoutRecord::decode(&bytes).unwrap();
        assert_eq!(decoded, record);
        assert_eq!(decoded.encode(), bytes);
        // golden: 머리(16 + 2)와 첫 항목(position=absolute: 0x0010, keyword tag 0, 번호 2).
        assert_eq!(&bytes[16..18], &(11u16).to_le_bytes());
        assert_eq!(&bytes[18..22], &[0x10, 0x00, 0x00, 0x02]);
        let small = {
            let mut r = LayoutRecord::new(StableId::from_bytes([0; 16]));
            r.set(LayoutLonghand::Left, LayoutValue::Length(8.0))
                .unwrap();
            r.encode()
        };
        let mut expected = vec![0u8; 16];
        expected.extend_from_slice(&[1, 0, 0x15, 0x00, 1]);
        expected.extend_from_slice(&8.0f32.to_le_bytes());
        assert_eq!(small, expected);
    }

    #[test]
    fn malformed_records_are_rejected() {
        let bytes = sample().encode();
        for cut in [0, 10, 17, 19, bytes.len() - 1] {
            assert!(LayoutRecord::decode(&bytes[..cut]).is_err(), "절단 {cut}");
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(LayoutRecord::decode(&trailing).is_err());
        // 모르는 longhand 번호.
        let mut unknown = LayoutRecord::new(sid(1)).encode();
        unknown[16] = 1;
        unknown.extend_from_slice(&[0xff, 0x7f, 0, 0]);
        assert_eq!(
            LayoutRecord::decode(&unknown).unwrap_err(),
            LayoutRecordError::UnknownLonghand { id: 0x7fff }
        );
        // 순서 위반(같은 번호 두 번).
        let mut duplicate = LayoutRecord::new(sid(1)).encode();
        duplicate[16] = 2;
        duplicate.extend_from_slice(&[0x10, 0, 0, 0, 0x10, 0, 0, 1]);
        assert!(LayoutRecord::decode(&duplicate).is_err());
        // -0.
        let mut negative_zero = LayoutRecord::new(sid(1)).encode();
        negative_zero[16] = 1;
        negative_zero.extend_from_slice(&[0x15, 0, 1]);
        negative_zero.extend_from_slice(&(-0.0f32).to_le_bytes());
        assert!(LayoutRecord::decode(&negative_zero).is_err());
        // 문법 위반 값(width에 음수 길이).
        let mut bad = LayoutRecord::new(sid(1)).encode();
        bad[16] = 1;
        bad.extend_from_slice(&[0x16, 0, 1]);
        bad.extend_from_slice(&(-4.0f32).to_le_bytes());
        assert!(matches!(
            LayoutRecord::decode(&bad).unwrap_err(),
            LayoutRecordError::Value { .. }
        ));
    }

    #[test]
    fn set_rejects_non_canonical_values_and_unset_returns_previous() {
        let mut record = LayoutRecord::new(sid(1));
        assert!(record
            .set(LayoutLonghand::Width, LayoutValue::Length(-1.0))
            .is_err());
        assert!(record
            .set(LayoutLonghand::Display, LayoutValue::Keyword("inline"))
            .is_err());
        record
            .set(LayoutLonghand::Width, LayoutValue::Keyword("auto"))
            .unwrap();
        assert_eq!(
            record.unset(LayoutLonghand::Width),
            Some(LayoutValue::Keyword("auto"))
        );
        assert!(record.is_empty());
    }

    #[test]
    fn records_are_checked_against_node_kind() {
        let frame = NodePayload::Frame(FramePayload::default());
        let text = NodePayload::Text(TextPayload::empty(0));
        let fragment = NodePayload::Fragment(FragmentPayload);
        let mut on_text = LayoutRecord::new(sid(2));
        on_text
            .set(LayoutLonghand::Display, LayoutValue::Keyword("flex"))
            .unwrap();
        on_text
            .set(LayoutLonghand::Width, LayoutValue::Keyword("auto"))
            .unwrap();
        let mut on_frame = LayoutRecord::new(sid(1));
        on_frame
            .set(LayoutLonghand::Width, LayoutValue::Length(3.0))
            .unwrap();
        let mut on_fragment = LayoutRecord::new(sid(3));
        on_fragment
            .set(LayoutLonghand::Left, LayoutValue::Length(3.0))
            .unwrap();
        let lookup = |id: StableId| match id {
            id if id == sid(1) => Some(&frame),
            id if id == sid(2) => Some(&text),
            id if id == sid(3) => Some(&fragment),
            _ => None,
        };
        let violations = check_records(
            &[
                on_text.clone(),
                on_frame,
                on_fragment,
                LayoutRecord::new(sid(4)),
                on_text,
            ],
            lookup,
        );
        assert_eq!(
            violations,
            vec![
                LayoutRecordViolation::NotApplicable {
                    node: sid(2),
                    longhand: LayoutLonghand::Display
                },
                LayoutRecordViolation::SizeInPayload {
                    node: sid(1),
                    longhand: LayoutLonghand::Width
                },
                LayoutRecordViolation::NotApplicable {
                    node: sid(3),
                    longhand: LayoutLonghand::Left
                },
                LayoutRecordViolation::Empty { node: sid(4) },
                LayoutRecordViolation::OwnerMissing { node: sid(4) },
                LayoutRecordViolation::Duplicate { node: sid(2) },
            ]
        );
    }

    mod document {
        use super::*;
        use crate::component::instance::ComponentInstancePayload;
        use crate::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
        use crate::model::DocumentModel;
        use crate::record_codec::SubtreeRecords;
        use crate::records::{DocumentRecords, NodeRecord};
        use crate::shape::{EllipseParams, RectangleParams};
        use crate::validate::{validate_document, DocumentViolation};
        use stellar_core_types::order_key::OrderKey;
        use stellar_core_types::reference::{AssetRef, ComponentRef};
        use stellar_core_types::value::{Rect, Size2, Vec2};

        fn node(id: u64, parent: Option<u64>, key: u8, payload: NodePayload) -> NodeRecord {
            NodeRecord {
                id: sid(id),
                parent: parent.map(sid),
                order_key: OrderKey::from_bytes(vec![key]).unwrap(),
                visible: true,
                locked: false,
                display_name: String::new(),
                payload,
            }
        }

        fn item(id: u64, entries: &[(LayoutLonghand, &str)]) -> LayoutRecord {
            let mut record = LayoutRecord::new(sid(id));
            for (longhand, text) in entries {
                record
                    .set(*longhand, longhand.parse(text).unwrap())
                    .unwrap();
            }
            record
        }

        /// root Frame(flex container) 아래 Frame·Rectangle·Ellipse(원점이 0이 아닌 bounds)·Text·Image·
        /// ComponentInstance 자식이 각자 item record를 가진 문서.
        pub(crate) fn records(with_layout: bool) -> DocumentRecords {
            let mut definition = DocumentDefinition::new(sid(100), String::new());
            let mut page = PageDefinition::new(sid(101), String::new());
            page.add_view(ViewDefinition::new(sid(102), String::new(), sid(1)))
                .unwrap();
            definition.add_page(page).unwrap();
            let mut records = DocumentRecords::new(definition);
            let size = Size2::new(10.0, 20.0).unwrap();
            let rect = RectangleParams {
                size,
                corner_radius: Default::default(),
            };
            let ellipse =
                EllipseParams::new(Rect::new(Vec2::new(5.0, -3.0).unwrap(), size)).unwrap();
            records.nodes = vec![
                node(1, None, 1, NodePayload::Frame(FramePayload::default())),
                node(2, Some(1), 1, NodePayload::Frame(FramePayload::default())),
                node(
                    3,
                    Some(1),
                    2,
                    NodePayload::Shape(ShapeGeometry::Rectangle(rect)),
                ),
                node(
                    4,
                    Some(1),
                    3,
                    NodePayload::Shape(ShapeGeometry::Ellipse(ellipse)),
                ),
                node(5, Some(1), 4, NodePayload::Text(TextPayload::empty(0))),
                node(
                    6,
                    Some(1),
                    5,
                    NodePayload::Image(crate::image::ImagePayload::new(
                        AssetRef::new(sid(90)),
                        size,
                    )),
                ),
                node(
                    7,
                    Some(1),
                    6,
                    NodePayload::ComponentInstance(ComponentInstancePayload::new(
                        ComponentRef::new(sid(91)),
                    )),
                ),
            ];
            if with_layout {
                records.layout_records = vec![
                    item(
                        1,
                        &[
                            (LayoutLonghand::Display, "flex"),
                            (LayoutLonghand::ColumnGap, "4px"),
                            (LayoutLonghand::PaddingLeft, "8px"),
                        ],
                    ),
                    item(
                        2,
                        &[
                            (LayoutLonghand::FlexGrow, "1"),
                            (LayoutLonghand::Display, "grid"),
                        ],
                    ),
                    item(3, &[(LayoutLonghand::AlignSelf, "center")]),
                    item(
                        4,
                        &[
                            (LayoutLonghand::Position, "relative"),
                            (LayoutLonghand::Left, "2px"),
                        ],
                    ),
                    item(
                        5,
                        &[
                            (LayoutLonghand::Width, "fit-content"),
                            (LayoutLonghand::FlexShrink, "0"),
                        ],
                    ),
                    item(6, &[(LayoutLonghand::FlexBasis, "50%")]),
                    item(
                        7,
                        &[
                            (LayoutLonghand::Width, "120px"),
                            (LayoutLonghand::Height, "auto"),
                            (LayoutLonghand::GridColumnStart, "span 2"),
                        ],
                    ),
                ];
            }
            records.sort_canonical();
            records
        }

        #[test]
        fn every_boxed_kind_round_trips_item_records_through_model_and_subtree_bytes() {
            let records = records(true);
            validate_document(&records).unwrap();
            let (model, _) = DocumentModel::from_records(records.clone()).unwrap();
            assert_eq!(model.to_records().unwrap(), records, "모델 왕복");
            assert_eq!(
                model
                    .layout_record(sid(5))
                    .unwrap()
                    .get(LayoutLonghand::Width),
                Some(&LayoutValue::Keyword("fit-content"))
            );
            let subtrees = records.view_subtrees().unwrap();
            assert_eq!(subtrees.len(), 1);
            let bytes = subtrees[0].1.encode();
            let decoded = SubtreeRecords::decode(&bytes).unwrap();
            assert_eq!(decoded, subtrees[0].1);
            assert_eq!(decoded.encode(), bytes, "canonical bytes");
            let rebuilt = DocumentRecords::from_subtrees(records.definition.clone(), vec![decoded]);
            assert_eq!(rebuilt, records);
        }

        #[test]
        fn subtree_without_layout_records_keeps_previous_bytes() {
            // 명시 layout longhand가 없으면 layout 절을 쓰지 않아 IX.11.6 이전 형식과 bytes가 같습니다.
            let plain = records(false);
            let subtree = plain.view_subtrees().unwrap().remove(0).1;
            let bytes = subtree.encode();
            let mut expected = Vec::new();
            expected.extend_from_slice(&subtree.root.to_bytes());
            expected.extend_from_slice(&(subtree.nodes.len() as u32).to_le_bytes());
            for n in &subtree.nodes {
                let encoded = crate::record_codec::encode_node_record(n);
                expected.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
                expected.extend_from_slice(&encoded);
            }
            expected.extend_from_slice(&0u32.to_le_bytes());
            assert_eq!(bytes, expected);
            // 빈 layout 절(수 0)은 정규형이 아닙니다.
            let mut empty_section = bytes.clone();
            empty_section.extend_from_slice(&0u32.to_le_bytes());
            assert!(SubtreeRecords::decode(&empty_section).is_err());
            // subtree 밖 node의 record는 거부합니다.
            let mut foreign = subtree.clone();
            foreign.layout_records = vec![item(999, &[(LayoutLonghand::Left, "1px")])];
            assert!(SubtreeRecords::decode(&foreign.encode()).is_err());
        }

        #[test]
        fn item_records_do_not_move_existing_geometry() {
            // item record를 더해도 payload(Ellipse bounds 원점 포함)와 node 공통 필드는 그대로입니다.
            // 화면 위치는 payload·transform이 정하고, record는 배치 입력에만 쓰입니다(ADR-0086 결정 3).
            let before = records(false);
            let after = records(true);
            assert_eq!(before.nodes, after.nodes);
        }

        #[test]
        fn validator_reports_misplaced_records() {
            let mut records = records(true);
            records
                .layout_records
                .push(item(5, &[(LayoutLonghand::Display, "flex")]));
            records
                .layout_records
                .push(item(3, &[(LayoutLonghand::Width, "1px")]));
            records.sort_canonical();
            let error = validate_document(&records).unwrap_err();
            let layout: Vec<_> = error
                .violations
                .iter()
                .filter(|v| matches!(v, DocumentViolation::LayoutRecord { .. }))
                .collect();
            assert_eq!(layout.len(), 2, "{error:?}");
            let mut model = DocumentModel::from_records(super::document::records(true))
                .unwrap()
                .0;
            assert!(model
                .set_layout_record(item(6, &[(LayoutLonghand::Width, "1px")]))
                .is_err());
            let previous = model
                .set_layout_record(LayoutRecord::new(sid(6)))
                .unwrap()
                .unwrap();
            assert_eq!(
                previous.get(LayoutLonghand::FlexBasis),
                Some(&LayoutValue::Percentage(50.0))
            );
            assert!(model.layout_record(sid(6)).is_none(), "빈 record는 지움");
        }
    }
}
