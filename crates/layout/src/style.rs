//! 레이아웃 입력 CSS 값 (ADR-0074 결정 6, ADR-0076).
//!
//! 엔진 입력은 CSS longhand와 CSS 값을 그대로 담습니다. 배치 방식은 `display`(`block`·`flex`·`grid`·`none`)와
//! `position`(`static`·`relative`·`absolute`)이고, 크기는 `width`·`height`의 `<length>`·`<percentage>`·`auto`·
//! `min-content`·`max-content`·`fit-content`입니다. 엔진 고유 배치 방식이나 크기 방식 이름은 두지 않습니다.
//!
//! # 생략값
//!
//! [`LayoutStyle::default`]의 모든 값은 CSS initial value입니다(`flex-shrink: 1`, `align-items: normal`,
//! `min-width: auto`, `grid-auto-rows: auto` 등). 예외는 하나입니다. `display`의 CSS initial value는 `inline`이지만
//! Stellar에는 inline formatting context가 없으므로 생략값은 `div`의 UA 기본값인 `block`입니다(ADR-0076 결정 3).
//!
//! # 받아들이는 부분집합
//!
//! 여기서 표현할 수 있는 값은 모두 CSS와 같은 결과를 냅니다. CSS가 받지만 엔진이 같은 결과를 보장하지 못하는
//! 조합은 [`crate::input::LayoutInput::validate`]가 [`crate::error::LayoutError::Unsupported`]로 거부합니다
//! (grid item의 `baseline` 정렬, grid 안 absolute 자식의 명시 line 배치). 고정 값은 `box-sizing: border-box`,
//! `margin: 0`, `border: 0`, `direction: ltr`, `writing-mode: horizontal-tb`, `overflow: visible`입니다.

use std::fmt;

use stellar_core_types::value::{FiniteF32, Insets, Length, Percentage};

use crate::geometry::Axis;

/// `display` (CSS Display 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Display {
    /// `block`: block formatting context. in-flow 자식을 세로로 차례로 놓습니다.
    #[default]
    Block,
    /// `flex`: flex container (CSS Flexbox).
    Flex,
    /// `grid`: grid container (CSS Grid).
    Grid,
    /// `none`: box를 만들지 않습니다. 부모 배치·측정·overflow에서 빠지고 subtree가 결과에 없습니다.
    None,
}

/// `position` (CSS Positioned Layout 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Position {
    /// `static`.
    #[default]
    Static,
    /// `relative`: 배치 뒤 inset만큼 시각 위치를 옮깁니다(형제 배치에 영향 없음).
    Relative,
    /// `absolute`: 부모 flow에서 빠지고 부모 padding box를 containing block으로 inset 배치합니다.
    Absolute,
}

/// `<length-percentage>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LengthPercentage {
    /// `<length>`(px).
    Length(Length),
    /// `<percentage>`.
    Percentage(Percentage),
}

/// `<length-percentage> | auto` (inset longhand).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LengthPercentageAuto {
    /// `auto`.
    #[default]
    Auto,
    /// `<length>`(px).
    Length(Length),
    /// `<percentage>`.
    Percentage(Percentage),
}

/// `width`·`height`·`min-width`·`min-height` 값.
///
/// `min-*`에서 `Auto`는 CSS 자동 최소 크기입니다(flex·grid item은 콘텐츠 기반 최소 크기, 그 밖은 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Size {
    /// `auto`.
    #[default]
    Auto,
    /// `<length>`(px, border box).
    Length(Length),
    /// `<percentage>`(containing block 크기 기준, border box).
    Percentage(Percentage),
    /// `min-content`.
    MinContent,
    /// `max-content`.
    MaxContent,
    /// `fit-content`.
    FitContent,
}

/// `max-width`·`max-height` 값.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MaxSize {
    /// `none`.
    #[default]
    None,
    /// `<length>`(px).
    Length(Length),
    /// `<percentage>`.
    Percentage(Percentage),
    /// `min-content`.
    MinContent,
    /// `max-content`.
    MaxContent,
    /// `fit-content`.
    FitContent,
}

/// `flex-basis` 값.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FlexBasis {
    /// `auto`: main 축 크기 속성(`width`·`height`)을 씁니다. 그 값도 `auto`면 `content`입니다.
    #[default]
    Auto,
    /// `content`: 콘텐츠 크기(max-content).
    Content,
    /// `<length>`(px).
    Length(Length),
    /// `<percentage>`(flex container inner main 크기 기준).
    Percentage(Percentage),
}

/// `flex-direction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FlexDirection {
    /// `row`.
    #[default]
    Row,
    /// `row-reverse`.
    RowReverse,
    /// `column`.
    Column,
    /// `column-reverse`.
    ColumnReverse,
}

impl FlexDirection {
    /// main 축.
    pub const fn main_axis(self) -> Axis {
        match self {
            FlexDirection::Row | FlexDirection::RowReverse => Axis::Horizontal,
            FlexDirection::Column | FlexDirection::ColumnReverse => Axis::Vertical,
        }
    }

    /// main 축 방향이 뒤집혔는지 여부.
    pub const fn is_reverse(self) -> bool {
        matches!(
            self,
            FlexDirection::RowReverse | FlexDirection::ColumnReverse
        )
    }
}

/// `flex-wrap`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FlexWrap {
    /// `nowrap`.
    #[default]
    NoWrap,
    /// `wrap`.
    Wrap,
    /// `wrap-reverse`.
    WrapReverse,
}

/// `justify-content`·`align-content` 값(content distribution·position).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ContentAlignment {
    /// `normal`. flex `justify-content`에서는 `flex-start`, flex `align-content`·grid에서는 `stretch`입니다.
    #[default]
    Normal,
    /// `start`.
    Start,
    /// `end`.
    End,
    /// `flex-start`.
    FlexStart,
    /// `flex-end`.
    FlexEnd,
    /// `center`.
    Center,
    /// `space-between`.
    SpaceBetween,
    /// `space-around`.
    SpaceAround,
    /// `space-evenly`.
    SpaceEvenly,
    /// `stretch`.
    Stretch,
}

/// `align-items`·`justify-items` 값.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ItemAlignment {
    /// `normal`. flex·grid item에서는 `stretch`처럼 동작합니다(크기 `auto` 축만 늘어남).
    #[default]
    Normal,
    /// `stretch`.
    Stretch,
    /// `start`.
    Start,
    /// `end`.
    End,
    /// `flex-start`.
    FlexStart,
    /// `flex-end`.
    FlexEnd,
    /// `center`.
    Center,
    /// `baseline`(first baseline). flex container의 cross 축(가로 main 축)에서만 받습니다.
    Baseline,
}

/// `align-self`·`justify-self` 값.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SelfAlignment {
    /// `auto`: 부모의 `align-items`·`justify-items`를 씁니다.
    #[default]
    Auto,
    /// 부모 값 대신 쓸 정렬.
    Item(ItemAlignment),
}

impl SelfAlignment {
    /// `auto`를 부모 값으로 해석합니다.
    pub const fn resolve(self, parent: ItemAlignment) -> ItemAlignment {
        match self {
            SelfAlignment::Auto => parent,
            SelfAlignment::Item(item) => item,
        }
    }
}

/// `grid-auto-flow`(`dense`는 받지 않음).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GridAutoFlow {
    /// `row`.
    #[default]
    Row,
    /// `column`.
    Column,
}

/// `grid-*-start`·`grid-*-end` 값(named line은 받지 않음).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GridLine {
    /// `auto`.
    #[default]
    Auto,
    /// `<integer>` line 번호(0이 아님, 음수는 명시 grid 끝에서 셈).
    Line(i16),
    /// `span <integer>`(1 이상).
    Span(u16),
}

/// track 최솟값 sizing 함수(`minmax()`의 첫 인자).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MinTrackSize {
    /// `auto`: item 최소 기여(자동 최소 크기).
    Auto,
    /// `<length>`.
    Length(Length),
    /// `<percentage>`(grid container content box 기준, indefinite면 `auto`).
    Percentage(Percentage),
    /// `min-content`.
    MinContent,
    /// `max-content`.
    MaxContent,
}

/// track 최댓값 sizing 함수(`minmax()`의 두 번째 인자).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MaxTrackSize {
    /// `auto`(= `max-content`, stretch 대상).
    Auto,
    /// `<length>`.
    Length(Length),
    /// `<percentage>`.
    Percentage(Percentage),
    /// `min-content`.
    MinContent,
    /// `max-content`.
    MaxContent,
    /// `<flex>`(`fr`, 비음수).
    Fraction(FiniteF32),
    /// `fit-content(<length-percentage>)`.
    FitContent(LengthPercentage),
}

/// `<track-size>` 하나(`minmax(min, max)`로 정규화한 값).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TrackSize {
    /// 최솟값 sizing 함수.
    pub min: MinTrackSize,
    /// 최댓값 sizing 함수.
    pub max: MaxTrackSize,
}

impl TrackSize {
    /// `auto` track.
    pub const AUTO: TrackSize = TrackSize {
        min: MinTrackSize::Auto,
        max: MaxTrackSize::Auto,
    };

    /// `<length>` track.
    pub fn length(px: Length) -> TrackSize {
        TrackSize {
            min: MinTrackSize::Length(px),
            max: MaxTrackSize::Length(px),
        }
    }

    /// bare `<flex>` track(`minmax(auto, <n>fr)`, CSS 의미).
    pub fn fr(factor: FiniteF32) -> TrackSize {
        TrackSize {
            min: MinTrackSize::Auto,
            max: MaxTrackSize::Fraction(factor),
        }
    }

    /// `minmax(0px, <n>fr)` track.
    pub fn fr_from_zero(factor: FiniteF32) -> TrackSize {
        TrackSize {
            min: MinTrackSize::Length(Length::from_raw(FiniteF32::ZERO)),
            max: MaxTrackSize::Fraction(factor),
        }
    }

    /// `min-content` track.
    pub const MIN_CONTENT: TrackSize = TrackSize {
        min: MinTrackSize::MinContent,
        max: MaxTrackSize::MinContent,
    };

    /// `max-content` track.
    pub const MAX_CONTENT: TrackSize = TrackSize {
        min: MinTrackSize::MaxContent,
        max: MaxTrackSize::MaxContent,
    };

    /// `fit-content(<limit>)` track.
    pub fn fit_content(limit: LengthPercentage) -> TrackSize {
        TrackSize {
            min: MinTrackSize::Auto,
            max: MaxTrackSize::FitContent(limit),
        }
    }

    /// `minmax(min, max)`.
    pub const fn minmax(min: MinTrackSize, max: MaxTrackSize) -> TrackSize {
        TrackSize { min, max }
    }

    /// flexible track(최댓값이 `fr`)인지 여부.
    pub const fn is_flexible(&self) -> bool {
        matches!(self.max, MaxTrackSize::Fraction(_))
    }
}

/// `aspect-ratio: <width> / <height>`(두 값 모두 양수).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AspectRatio {
    /// 너비 성분.
    pub width: FiniteF32,
    /// 높이 성분.
    pub height: FiniteF32,
}

impl AspectRatio {
    /// 비율을 만듭니다(검증은 입력 검증이 합니다).
    pub fn new(width: f32, height: f32) -> Option<AspectRatio> {
        Some(AspectRatio {
            width: FiniteF32::new(width).ok()?,
            height: FiniteF32::new(height).ok()?,
        })
    }
}

/// node 하나의 CSS 레이아웃 longhand (ADR-0076).
///
/// [`LayoutStyle::default`]는 CSS initial value입니다(모듈 문서의 `display` 예외 제외).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LayoutStyle {
    /// `display`.
    pub display: Display,
    /// `position`.
    pub position: Position,
    /// `top`.
    pub top: LengthPercentageAuto,
    /// `right`.
    pub right: LengthPercentageAuto,
    /// `bottom`.
    pub bottom: LengthPercentageAuto,
    /// `left`.
    pub left: LengthPercentageAuto,
    /// `width`.
    pub width: Size,
    /// `height`.
    pub height: Size,
    /// `min-width`.
    pub min_width: Size,
    /// `min-height`.
    pub min_height: Size,
    /// `max-width`.
    pub max_width: MaxSize,
    /// `max-height`.
    pub max_height: MaxSize,
    /// `aspect-ratio`(`None` = `auto`).
    pub aspect_ratio: Option<AspectRatio>,
    /// `padding-top`·`-right`·`-bottom`·`-left`(px, 비음수).
    pub padding: Insets,
    /// `flex-direction`.
    pub flex_direction: FlexDirection,
    /// `flex-wrap`.
    pub flex_wrap: FlexWrap,
    /// `justify-content`.
    pub justify_content: ContentAlignment,
    /// `align-content`.
    pub align_content: ContentAlignment,
    /// `align-items`.
    pub align_items: ItemAlignment,
    /// `justify-items`.
    pub justify_items: ItemAlignment,
    /// `row-gap`(px, 비음수, `normal` = 0).
    pub row_gap: Length,
    /// `column-gap`(px, 비음수, `normal` = 0).
    pub column_gap: Length,
    /// `flex-grow`(비음수).
    pub flex_grow: FiniteF32,
    /// `flex-shrink`(비음수).
    pub flex_shrink: FiniteF32,
    /// `flex-basis`.
    pub flex_basis: FlexBasis,
    /// `align-self`.
    pub align_self: SelfAlignment,
    /// `justify-self`.
    pub justify_self: SelfAlignment,
    /// `grid-template-columns`(빈 목록 = `none`).
    pub grid_template_columns: Vec<TrackSize>,
    /// `grid-template-rows`(빈 목록 = `none`).
    pub grid_template_rows: Vec<TrackSize>,
    /// `grid-auto-columns`(비어 있지 않은 반복 pattern).
    pub grid_auto_columns: Vec<TrackSize>,
    /// `grid-auto-rows`(비어 있지 않은 반복 pattern).
    pub grid_auto_rows: Vec<TrackSize>,
    /// `grid-auto-flow`.
    pub grid_auto_flow: GridAutoFlow,
    /// `grid-column-start`.
    pub grid_column_start: GridLine,
    /// `grid-column-end`.
    pub grid_column_end: GridLine,
    /// `grid-row-start`.
    pub grid_row_start: GridLine,
    /// `grid-row-end`.
    pub grid_row_end: GridLine,
}

fn zero_length() -> Length {
    Length::from_raw(FiniteF32::ZERO)
}

fn one() -> FiniteF32 {
    FiniteF32::new(1.0).expect("1은 유한")
}

impl Default for LayoutStyle {
    fn default() -> LayoutStyle {
        LayoutStyle {
            display: Display::Block,
            position: Position::Static,
            top: LengthPercentageAuto::Auto,
            right: LengthPercentageAuto::Auto,
            bottom: LengthPercentageAuto::Auto,
            left: LengthPercentageAuto::Auto,
            width: Size::Auto,
            height: Size::Auto,
            min_width: Size::Auto,
            min_height: Size::Auto,
            max_width: MaxSize::None,
            max_height: MaxSize::None,
            aspect_ratio: None,
            padding: Insets::new(0.0, 0.0, 0.0, 0.0).expect("0은 유한"),
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::NoWrap,
            justify_content: ContentAlignment::Normal,
            align_content: ContentAlignment::Normal,
            align_items: ItemAlignment::Normal,
            justify_items: ItemAlignment::Normal,
            row_gap: zero_length(),
            column_gap: zero_length(),
            flex_grow: FiniteF32::ZERO,
            flex_shrink: one(),
            flex_basis: FlexBasis::Auto,
            align_self: SelfAlignment::Auto,
            justify_self: SelfAlignment::Auto,
            grid_template_columns: Vec::new(),
            grid_template_rows: Vec::new(),
            grid_auto_columns: vec![TrackSize::AUTO],
            grid_auto_rows: vec![TrackSize::AUTO],
            grid_auto_flow: GridAutoFlow::Row,
            grid_column_start: GridLine::Auto,
            grid_column_end: GridLine::Auto,
            grid_row_start: GridLine::Auto,
            grid_row_end: GridLine::Auto,
        }
    }
}

impl LayoutStyle {
    /// 축의 `width`·`height`.
    pub const fn size(&self, axis: Axis) -> Size {
        match axis {
            Axis::Horizontal => self.width,
            Axis::Vertical => self.height,
        }
    }

    /// 축의 `min-width`·`min-height`.
    pub const fn min_size(&self, axis: Axis) -> Size {
        match axis {
            Axis::Horizontal => self.min_width,
            Axis::Vertical => self.min_height,
        }
    }

    /// 축의 `max-width`·`max-height`.
    pub const fn max_size(&self, axis: Axis) -> MaxSize {
        match axis {
            Axis::Horizontal => self.max_width,
            Axis::Vertical => self.max_height,
        }
    }

    /// 축의 시작·끝 inset(`left`·`right` 또는 `top`·`bottom`).
    pub const fn insets(&self, axis: Axis) -> (LengthPercentageAuto, LengthPercentageAuto) {
        match axis {
            Axis::Horizontal => (self.left, self.right),
            Axis::Vertical => (self.top, self.bottom),
        }
    }

    /// 축의 grid line(`grid-column-*` 또는 `grid-row-*`).
    pub const fn grid_lines(&self, axis: Axis) -> (GridLine, GridLine) {
        match axis {
            Axis::Horizontal => (self.grid_column_start, self.grid_column_end),
            Axis::Vertical => (self.grid_row_start, self.grid_row_end),
        }
    }

    /// 축의 gap(가로 축 = `column-gap`, 세로 축 = `row-gap`).
    pub const fn gap(&self, axis: Axis) -> Length {
        match axis {
            Axis::Horizontal => self.column_gap,
            Axis::Vertical => self.row_gap,
        }
    }

    /// 부모 flow에 참여하는지 여부(`display: none`과 `position: absolute`는 참여하지 않음).
    pub const fn is_in_flow(&self) -> bool {
        !matches!(self.display, Display::None) && !matches!(self.position, Position::Absolute)
    }
}

// --- CSS 텍스트 표기(진단·snapshot용, 파싱 API 아님) ---

/// px 값을 CSS 텍스트로 씁니다(정수는 소수점 없이).
fn px(f: &mut fmt::Formatter<'_>, value: f32) -> fmt::Result {
    write!(f, "{value}px")
}

impl fmt::Display for Display {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Display::Block => "block",
            Display::Flex => "flex",
            Display::Grid => "grid",
            Display::None => "none",
        })
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Position::Static => "static",
            Position::Relative => "relative",
            Position::Absolute => "absolute",
        })
    }
}

impl fmt::Display for LengthPercentage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LengthPercentage::Length(l) => px(f, l.get()),
            LengthPercentage::Percentage(p) => write!(f, "{}%", p.get()),
        }
    }
}

impl fmt::Display for LengthPercentageAuto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LengthPercentageAuto::Auto => f.write_str("auto"),
            LengthPercentageAuto::Length(l) => px(f, l.get()),
            LengthPercentageAuto::Percentage(p) => write!(f, "{}%", p.get()),
        }
    }
}

impl fmt::Display for Size {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Size::Auto => f.write_str("auto"),
            Size::Length(l) => px(f, l.get()),
            Size::Percentage(p) => write!(f, "{}%", p.get()),
            Size::MinContent => f.write_str("min-content"),
            Size::MaxContent => f.write_str("max-content"),
            Size::FitContent => f.write_str("fit-content"),
        }
    }
}

impl fmt::Display for MaxSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MaxSize::None => f.write_str("none"),
            MaxSize::Length(l) => px(f, l.get()),
            MaxSize::Percentage(p) => write!(f, "{}%", p.get()),
            MaxSize::MinContent => f.write_str("min-content"),
            MaxSize::MaxContent => f.write_str("max-content"),
            MaxSize::FitContent => f.write_str("fit-content"),
        }
    }
}

impl fmt::Display for FlexBasis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FlexBasis::Auto => f.write_str("auto"),
            FlexBasis::Content => f.write_str("content"),
            FlexBasis::Length(l) => px(f, l.get()),
            FlexBasis::Percentage(p) => write!(f, "{}%", p.get()),
        }
    }
}

impl fmt::Display for FlexDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            FlexDirection::Row => "row",
            FlexDirection::RowReverse => "row-reverse",
            FlexDirection::Column => "column",
            FlexDirection::ColumnReverse => "column-reverse",
        })
    }
}

impl fmt::Display for FlexWrap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            FlexWrap::NoWrap => "nowrap",
            FlexWrap::Wrap => "wrap",
            FlexWrap::WrapReverse => "wrap-reverse",
        })
    }
}

impl fmt::Display for ContentAlignment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ContentAlignment::Normal => "normal",
            ContentAlignment::Start => "start",
            ContentAlignment::End => "end",
            ContentAlignment::FlexStart => "flex-start",
            ContentAlignment::FlexEnd => "flex-end",
            ContentAlignment::Center => "center",
            ContentAlignment::SpaceBetween => "space-between",
            ContentAlignment::SpaceAround => "space-around",
            ContentAlignment::SpaceEvenly => "space-evenly",
            ContentAlignment::Stretch => "stretch",
        })
    }
}

impl fmt::Display for ItemAlignment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ItemAlignment::Normal => "normal",
            ItemAlignment::Stretch => "stretch",
            ItemAlignment::Start => "start",
            ItemAlignment::End => "end",
            ItemAlignment::FlexStart => "flex-start",
            ItemAlignment::FlexEnd => "flex-end",
            ItemAlignment::Center => "center",
            ItemAlignment::Baseline => "baseline",
        })
    }
}

impl fmt::Display for SelfAlignment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SelfAlignment::Auto => f.write_str("auto"),
            SelfAlignment::Item(item) => item.fmt(f),
        }
    }
}

impl fmt::Display for GridAutoFlow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            GridAutoFlow::Row => "row",
            GridAutoFlow::Column => "column",
        })
    }
}

impl fmt::Display for GridLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GridLine::Auto => f.write_str("auto"),
            GridLine::Line(n) => write!(f, "{n}"),
            GridLine::Span(n) => write!(f, "span {n}"),
        }
    }
}

impl fmt::Display for MinTrackSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MinTrackSize::Auto => f.write_str("auto"),
            MinTrackSize::Length(l) => px(f, l.get()),
            MinTrackSize::Percentage(p) => write!(f, "{}%", p.get()),
            MinTrackSize::MinContent => f.write_str("min-content"),
            MinTrackSize::MaxContent => f.write_str("max-content"),
        }
    }
}

impl fmt::Display for MaxTrackSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MaxTrackSize::Auto => f.write_str("auto"),
            MaxTrackSize::Length(l) => px(f, l.get()),
            MaxTrackSize::Percentage(p) => write!(f, "{}%", p.get()),
            MaxTrackSize::MinContent => f.write_str("min-content"),
            MaxTrackSize::MaxContent => f.write_str("max-content"),
            MaxTrackSize::Fraction(fr) => write!(f, "{}fr", fr.get()),
            MaxTrackSize::FitContent(limit) => write!(f, "fit-content({limit})"),
        }
    }
}

impl fmt::Display for TrackSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.min, self.max) {
            (MinTrackSize::Auto, MaxTrackSize::Auto) => f.write_str("auto"),
            (MinTrackSize::Length(a), MaxTrackSize::Length(b)) if a == b => px(f, a.get()),
            (MinTrackSize::Percentage(a), MaxTrackSize::Percentage(b)) if a == b => {
                write!(f, "{}%", a.get())
            }
            (MinTrackSize::MinContent, MaxTrackSize::MinContent) => f.write_str("min-content"),
            (MinTrackSize::MaxContent, MaxTrackSize::MaxContent) => f.write_str("max-content"),
            (MinTrackSize::Auto, MaxTrackSize::Fraction(fr)) => write!(f, "{}fr", fr.get()),
            (MinTrackSize::Auto, MaxTrackSize::FitContent(limit)) => {
                write!(f, "fit-content({limit})")
            }
            (min, max) => write!(f, "minmax({min}, {max})"),
        }
    }
}

impl fmt::Display for AspectRatio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} / {}", self.width.get(), self.height.get())
    }
}

/// track 목록을 CSS 텍스트로 씁니다(빈 목록은 `none`).
pub fn track_list_css(tracks: &[TrackSize]) -> String {
    if tracks.is_empty() {
        return "none".to_owned();
    }
    tracks
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}

impl LayoutStyle {
    /// 생략값(CSS initial value, `display`는 `block`)과 다른 longhand를 `(이름, 값)`으로 돌려줍니다.
    ///
    /// 순서는 고정입니다(이 구조체의 필드 순서). 진단·snapshot 텍스트용이며 저장 형식이 아닙니다.
    pub fn declarations(&self) -> Vec<(&'static str, String)> {
        let initial = LayoutStyle::default();
        let mut out = Vec::new();
        macro_rules! push {
            ($name:literal, $field:ident) => {
                if self.$field != initial.$field {
                    out.push(($name, self.$field.to_string()));
                }
            };
        }
        push!("display", display);
        push!("position", position);
        push!("top", top);
        push!("right", right);
        push!("bottom", bottom);
        push!("left", left);
        push!("width", width);
        push!("height", height);
        push!("min-width", min_width);
        push!("min-height", min_height);
        push!("max-width", max_width);
        push!("max-height", max_height);
        if let Some(ratio) = self.aspect_ratio {
            out.push(("aspect-ratio", ratio.to_string()));
        }
        let (top, right, bottom, left) = self.padding.components();
        for (name, value) in [
            ("padding-top", top),
            ("padding-right", right),
            ("padding-bottom", bottom),
            ("padding-left", left),
        ] {
            if value != 0.0 {
                out.push((name, format!("{value}px")));
            }
        }
        push!("flex-direction", flex_direction);
        push!("flex-wrap", flex_wrap);
        push!("justify-content", justify_content);
        push!("align-content", align_content);
        push!("align-items", align_items);
        push!("justify-items", justify_items);
        if self.row_gap != initial.row_gap {
            out.push(("row-gap", format!("{}px", self.row_gap.get())));
        }
        if self.column_gap != initial.column_gap {
            out.push(("column-gap", format!("{}px", self.column_gap.get())));
        }
        if self.flex_grow != initial.flex_grow {
            out.push(("flex-grow", self.flex_grow.get().to_string()));
        }
        if self.flex_shrink != initial.flex_shrink {
            out.push(("flex-shrink", self.flex_shrink.get().to_string()));
        }
        push!("flex-basis", flex_basis);
        push!("align-self", align_self);
        push!("justify-self", justify_self);
        if self.grid_template_columns != initial.grid_template_columns {
            out.push((
                "grid-template-columns",
                track_list_css(&self.grid_template_columns),
            ));
        }
        if self.grid_template_rows != initial.grid_template_rows {
            out.push((
                "grid-template-rows",
                track_list_css(&self.grid_template_rows),
            ));
        }
        if self.grid_auto_columns != initial.grid_auto_columns {
            out.push(("grid-auto-columns", track_list_css(&self.grid_auto_columns)));
        }
        if self.grid_auto_rows != initial.grid_auto_rows {
            out.push(("grid-auto-rows", track_list_css(&self.grid_auto_rows)));
        }
        push!("grid-auto-flow", grid_auto_flow);
        push!("grid-column-start", grid_column_start);
        push!("grid-column-end", grid_column_end);
        push!("grid-row-start", grid_row_start);
        push!("grid-row-end", grid_row_end);
        out
    }

    /// [`LayoutStyle::declarations`]를 `name: value; ...` 한 줄로 씁니다.
    ///
    /// 수 표기는 문서 CSS 값 정규형(`stellar_document::css::value::format_number`, ADR-0075 결정 5)과 같은
    /// 규칙입니다: f32를 왕복하는 최단 10진, 지수 없음, `-0`은 `0`(값 타입이 `-0`을 `0`으로 정규화). layout은
    /// crate graph상 core-types에만 의존하므로(ADR-0076) 그 함수를 직접 부르지 않고, 테스트 parser와의 왕복
    /// test(`tests/support/css.rs`)로 한 쌍임을 고정합니다(ADR-0075 개정). 저장 형식이 아니라 진단·snapshot
    /// 텍스트입니다.
    pub fn declaration_text(&self) -> String {
        self.declarations()
            .into_iter()
            .map(|(name, value)| format!("{name}: {value}"))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_css_initial_value() {
        let style = LayoutStyle::default();
        assert_eq!(style.flex_shrink.get(), 1.0);
        assert_eq!(style.flex_grow.get(), 0.0);
        assert_eq!(style.align_items, ItemAlignment::Normal);
        assert_eq!(style.align_content, ContentAlignment::Normal);
        assert_eq!(style.min_width, Size::Auto);
        assert_eq!(style.max_height, MaxSize::None);
        assert_eq!(style.grid_auto_rows, vec![TrackSize::AUTO]);
        assert!(style.declarations().is_empty());
    }

    #[test]
    fn declarations_use_css_text() {
        let style = LayoutStyle {
            display: Display::Flex,
            width: Size::Length(Length::new(320.0).unwrap()),
            height: Size::FitContent,
            flex_shrink: FiniteF32::ZERO,
            grid_template_columns: vec![
                TrackSize::fr(FiniteF32::new(1.0).unwrap()),
                TrackSize::fr_from_zero(FiniteF32::new(2.0).unwrap()),
                TrackSize::length(Length::new(10.5).unwrap()),
            ],
            align_self: SelfAlignment::Item(ItemAlignment::Center),
            grid_row_end: GridLine::Span(2),
            ..LayoutStyle::default()
        };
        assert_eq!(
            style.declaration_text(),
            "display: flex; width: 320px; height: fit-content; flex-shrink: 0; align-self: center; \
             grid-template-columns: 1fr minmax(0px, 2fr) 10.5px; grid-row-end: span 2"
        );
    }
}
