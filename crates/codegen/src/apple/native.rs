//! Apple·Android 공용 native 구조·layout 분석 (XXII.3.1·XXII.3.2·XXII.4.1·XXII.4.2, ADR-0107 결정 2·3).
//!
//! SwiftUI·AppKit·Compose adapter는 같은 IR을 같은 구조로 읽습니다. 이 module은 플랫폼 표현과 무관한 판정만 합니다.
//!
//! - **출력 view 종류**([`ViewKind`])와 **Control contract**([`Control`]): element 종류와 contract 번호, 그 node의
//!   capability 진단으로 정합니다. 한 node에 contract가 여럿이면 TextInput > Button > Modal > SingleSelectionGroup >
//!   List 순으로 하나를 고릅니다. 고른 contract가 이 profile에서 Unsupported이면 contract 없는 container로 냅니다.
//! - **자식 분할**: Fragment는 view가 없으므로 자식을 펼쳐 부모의 자식으로 셉니다. `display: none` node는 box가
//!   없어 출력에서 빠집니다. `position: absolute` 자식은 흐름에서 빠져 부모 padding box 기준 겹침 층이 됩니다.
//! - **채움**([`Fill`]): CSS가 box를 부모 공간까지 늘리는 경우입니다. block 부모의 `width: auto`, flex 부모의 cross
//!   축 stretch(`align-items: normal|stretch`, `align-self: auto|stretch`, cross 크기 `auto`), main 축 `flex-grow > 0`,
//!   grid 부모의 `justify-items`·`align-items` stretch, root의 `width: auto`(viewport 폭)입니다.
//! - **확정 크기**(definite): 명시 `<length>`이거나 채움이면서 부모의 같은 축이 확정인 경우, 그리고 root 폭(viewport)
//!   입니다. 확정이 아닌 축에서 자식을 채우는 container는 플랫폼 이상(ideal) 크기를 쓰도록 고정합니다(SwiftUI
//!   `fixedSize`, Compose `IntrinsicSize.Max`). CSS의 line cross 크기·shrink-to-fit과 같은 결과입니다.
//! - **grid 배치**([`GridPlan`]): 모든 item이 자동 배치이거나 모든 item이 양 축 시작 line을 명시한 경우만 칸 좌표를
//!   정합니다. 둘이 섞이거나 음수 line·row span이면 배치 규칙을 다시 구현하지 않고 Unsupported로 진단한 뒤 문서
//!   순서 자동 배치로 냅니다.

use stellar_layout::style::{
    ContentAlignment, Display, FlexBasis, FlexDirection, GridAutoFlow, GridLine, ItemAlignment,
    LayoutStyle, MaxTrackSize, MinTrackSize, Position, SelfAlignment, Size, TrackSize,
};
use stellar_target_profile::capability::{
    CapabilityStatus, CapabilitySubject, ContractCode, ElementKind,
};

use crate::ir::{CodegenIr, IrNode};

/// 축.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Axis {
    /// 가로(`width`).
    Horizontal,
    /// 세로(`height`).
    Vertical,
}

impl Axis {
    /// 배열 index.
    pub(crate) const fn index(self) -> usize {
        match self {
            Axis::Horizontal => 0,
            Axis::Vertical => 1,
        }
    }

    /// 반대 축.
    pub(crate) const fn cross(self) -> Axis {
        match self {
            Axis::Horizontal => Axis::Vertical,
            Axis::Vertical => Axis::Horizontal,
        }
    }
}

/// 출력 view 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ViewKind {
    /// Frame(자식 container).
    Container,
    /// Text.
    Text,
    /// Image.
    Image,
    /// Shape·Path(도형 기하는 IR에 없음).
    Shape,
    /// 펼치지 않은 ComponentInstance.
    Instance,
}

/// Control contract 대응.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Control {
    /// Button contract.
    Button,
    /// TextInput contract.
    TextInput,
    /// Modal contract.
    Modal,
    /// List contract.
    List,
    /// SingleSelectionGroup contract.
    SelectionGroup,
}

impl Control {
    /// 안정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            Control::Button => "button",
            Control::TextInput => "text_input",
            Control::Modal => "modal",
            Control::List => "list",
            Control::SelectionGroup => "single_selection_group",
        }
    }

    fn code(self) -> ContractCode {
        match self {
            Control::Button => ContractCode::BUTTON,
            Control::TextInput => ContractCode::TEXT_INPUT,
            Control::Modal => ContractCode::MODAL,
            Control::List => ContractCode::LIST,
            Control::SelectionGroup => ContractCode::SINGLE_SELECTION_GROUP,
        }
    }
}

/// contract 선택 우선순위.
const CONTROL_PRIORITY: [Control; 5] = [
    Control::TextInput,
    Control::Button,
    Control::Modal,
    Control::SelectionGroup,
    Control::List,
];

/// node의 container 배치 방식.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Container {
    /// `display: block`(세로 흐름, margin 0).
    Block,
    /// `display: flex`.
    Flex {
        /// main 축.
        main: Axis,
        /// `*-reverse`.
        reverse: bool,
    },
    /// `display: grid`.
    Grid(GridPlan),
}

/// grid 칸 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GridCell {
    /// 자식 IR node index.
    pub child: u32,
    /// 0부터 센 열.
    pub column: u16,
    /// 0부터 센 행.
    pub row: u16,
    /// 열 span(1 이상).
    pub column_span: u16,
}

/// grid 배치 계획.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GridPlan {
    /// 열 track(명시 track 뒤에 `grid-auto-columns`로 채운 암시 track).
    pub columns: Vec<TrackSize>,
    /// 행 track.
    pub rows: Vec<TrackSize>,
    /// 칸(행 → 열 순).
    pub cells: Vec<GridCell>,
    /// 배치를 정하지 못해 문서 순서 자동 배치로 대신한 이유.
    pub fallback: Option<&'static str>,
}

/// 축별 채움.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Fill {
    /// 축별(가로, 세로) 채움 여부.
    pub axes: [bool; 2],
}

/// node 하나의 분석 결과.
#[derive(Debug, Clone)]
pub(crate) struct NodeInfo {
    /// 출력 view 종류.
    pub kind: ViewKind,
    /// 고른 Control contract.
    pub control: Option<Control>,
    /// 이 profile에서 Unsupported라 버린 contract.
    pub dropped_control: Option<Control>,
    /// 출력되는지(`display: none`이거나 그 아래면 `false`, Fragment는 `false`).
    pub emitted: bool,
    /// 흐름 자식(Fragment를 펼친 IR node index, 문서 순서).
    pub flow: Vec<u32>,
    /// `position: absolute` 자식.
    pub absolute: Vec<u32>,
    /// 출력 부모(Fragment를 건너뛴 가장 가까운 출력 조상).
    pub parent: Option<u32>,
    /// container 배치(자식이 없는 leaf도 style의 display를 따릅니다).
    pub container: Container,
    /// 부모가 이 node를 늘리는 축.
    pub fill: Fill,
    /// 축별 확정 크기 여부.
    pub definite: [bool; 2],
}

/// IR 전체 분석.
#[derive(Debug, Clone)]
pub(crate) struct Analysis {
    /// node별 결과(IR node index 순).
    pub nodes: Vec<NodeInfo>,
}

fn kind_of(node: &IrNode) -> ViewKind {
    match node.element {
        ElementKind::TEXT => ViewKind::Text,
        ElementKind::IMAGE => ViewKind::Image,
        ElementKind::SHAPE | ElementKind::PATH => ViewKind::Shape,
        ElementKind::COMPONENT_INSTANCE => ViewKind::Instance,
        _ => ViewKind::Container,
    }
}

fn contract_status(node: &IrNode, code: ContractCode) -> Option<CapabilityStatus> {
    node.capabilities
        .iter()
        .find(|finding| finding.subject == CapabilitySubject::Contract(code))
        .and_then(|finding| finding.status)
}

fn control_of(node: &IrNode) -> (Option<Control>, Option<Control>) {
    let Some(control) = CONTROL_PRIORITY
        .into_iter()
        .find(|control| node.has_contract(control.code()))
    else {
        return (None, None);
    };
    match contract_status(node, control.code()) {
        Some(CapabilityStatus::Unsupported) => (None, Some(control)),
        _ => (Some(control), None),
    }
}

/// node style(Fragment·layout 없는 node는 `None`).
pub(crate) fn style(ir: &CodegenIr, index: u32) -> Option<&LayoutStyle> {
    ir.nodes[index as usize]
        .layout
        .as_ref()
        .map(|layout| &layout.style)
}

/// 축의 `width`·`height`.
pub(crate) fn size(style: &LayoutStyle, axis: Axis) -> Size {
    match axis {
        Axis::Horizontal => style.width,
        Axis::Vertical => style.height,
    }
}

fn container_of(style: Option<&LayoutStyle>) -> Container {
    let Some(style) = style else {
        return Container::Block;
    };
    match style.display {
        Display::Flex => Container::Flex {
            main: match style.flex_direction {
                FlexDirection::Row | FlexDirection::RowReverse => Axis::Horizontal,
                FlexDirection::Column | FlexDirection::ColumnReverse => Axis::Vertical,
            },
            reverse: style.flex_direction.is_reverse(),
        },
        Display::Grid => Container::Grid(GridPlan {
            columns: Vec::new(),
            rows: Vec::new(),
            cells: Vec::new(),
            fallback: None,
        }),
        Display::Block | Display::None => Container::Block,
    }
}

/// 자식의 cross 축 정렬(`align-self: auto`는 부모 `align-items`).
pub(crate) fn self_alignment(parent: &LayoutStyle, child: &LayoutStyle) -> ItemAlignment {
    child.align_self.resolve(parent.align_items)
}

/// grid 자식의 가로 정렬(`justify-self: auto`는 부모 `justify-items`).
pub(crate) fn justify_alignment(parent: &LayoutStyle, child: &LayoutStyle) -> ItemAlignment {
    child.justify_self.resolve(parent.justify_items)
}

/// 정렬이 stretch인지(`normal`은 non-replaced box에서 stretch).
pub(crate) fn is_stretch(alignment: ItemAlignment) -> bool {
    matches!(alignment, ItemAlignment::Normal | ItemAlignment::Stretch)
}

fn fill_of(parent: Option<(&Container, &LayoutStyle, [bool; 2])>, child: &LayoutStyle) -> Fill {
    let mut fill = Fill::default();
    if child.position == Position::Absolute {
        return fill;
    }
    let Some((container, parent_style, parent_definite)) = parent else {
        // root: viewport 폭을 채웁니다.
        fill.axes[0] = child.width == Size::Auto;
        return fill;
    };
    match container {
        Container::Block => fill.axes[0] = child.width == Size::Auto,
        Container::Flex { main, .. } => {
            let cross = main.cross();
            // 부모 main 크기가 확정이 아니면 남는 공간이 없어 CSS에서도 grow 효과가 없습니다.
            fill.axes[main.index()] = child.flex_grow.get() > 0.0 && parent_definite[main.index()];
            fill.axes[cross.index()] =
                is_stretch(self_alignment(parent_style, child)) && size(child, cross) == Size::Auto;
        }
        Container::Grid(_) => {
            fill.axes[0] =
                is_stretch(justify_alignment(parent_style, child)) && child.width == Size::Auto;
            fill.axes[1] =
                is_stretch(self_alignment(parent_style, child)) && child.height == Size::Auto;
        }
    }
    fill
}

/// IR을 분석합니다.
pub(crate) fn analyze(ir: &CodegenIr) -> Analysis {
    let mut nodes: Vec<NodeInfo> = ir
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let (control, dropped) = control_of(node);
            NodeInfo {
                kind: kind_of(node),
                control,
                dropped_control: dropped,
                emitted: false,
                flow: Vec::new(),
                absolute: Vec::new(),
                parent: None,
                container: container_of(style(ir, index as u32)),
                fill: Fill::default(),
                definite: [false; 2],
            }
        })
        .collect();
    // preorder라 부모가 먼저 정해집니다. 출력 부모와 출력 여부를 정합니다.
    let parents = ir.element_parents();
    for index in 0..ir.nodes.len() {
        let node = &ir.nodes[index];
        let parent = parents[index];
        let parent_emitted = parent.is_none_or(|p| nodes[p as usize].emitted);
        let displayed = style(ir, index as u32).is_none_or(|s| s.display != Display::None);
        nodes[index].parent = parent;
        nodes[index].emitted = node.element != ElementKind::FRAGMENT && displayed && parent_emitted;
        if nodes[index].emitted {
            if let Some(parent) = parent {
                let absolute =
                    style(ir, index as u32).is_some_and(|s| s.position == Position::Absolute);
                let list = if absolute {
                    &mut nodes[parent as usize].absolute
                } else {
                    &mut nodes[parent as usize].flow
                };
                list.push(index as u32);
            }
        }
    }
    // 채움과 확정 크기(preorder).
    for index in 0..ir.nodes.len() {
        if !nodes[index].emitted {
            continue;
        }
        let Some(child_style) = style(ir, index as u32) else {
            continue;
        };
        let parent = nodes[index].parent;
        let fill = match parent {
            Some(p) => match style(ir, p) {
                Some(parent_style) => fill_of(
                    Some((
                        &nodes[p as usize].container,
                        parent_style,
                        nodes[p as usize].definite,
                    )),
                    child_style,
                ),
                None => Fill::default(),
            },
            None => fill_of(None, child_style),
        };
        let mut definite = [false; 2];
        for axis in [Axis::Horizontal, Axis::Vertical] {
            definite[axis.index()] = match size(child_style, axis) {
                Size::Length(_) => true,
                _ if fill.axes[axis.index()] => match parent {
                    Some(p) => nodes[p as usize].definite[axis.index()],
                    None => axis == Axis::Horizontal,
                },
                _ => false,
            };
        }
        nodes[index].fill = fill;
        nodes[index].definite = definite;
    }
    // grid 배치.
    for index in 0..ir.nodes.len() {
        if !nodes[index].emitted {
            continue;
        }
        if let (Container::Grid(_), Some(grid_style)) =
            (&nodes[index].container, style(ir, index as u32))
        {
            let flow = nodes[index].flow.clone();
            let plan = grid_plan(ir, grid_style, &flow);
            nodes[index].container = Container::Grid(plan);
        }
    }
    Analysis { nodes }
}

fn track_list(template: &[TrackSize], auto: &[TrackSize], count: usize) -> Vec<TrackSize> {
    let mut tracks = template.to_vec();
    let mut repeat = auto.iter().cycle();
    while tracks.len() < count {
        tracks.push(*repeat.next().unwrap_or(&TrackSize::AUTO));
    }
    tracks
}

fn span_of(start: GridLine, end: GridLine) -> Option<u16> {
    match (start, end) {
        (GridLine::Span(n), GridLine::Auto) => Some(n.max(1)),
        (_, GridLine::Auto) => Some(1),
        (_, GridLine::Span(n)) => Some(n.max(1)),
        (GridLine::Line(a), GridLine::Line(b)) if b > a && a > 0 => Some((b - a) as u16),
        _ => None,
    }
}

const GRID_MIXED: &str = "명시 line 배치와 자동 배치가 섞인 grid는 배치 규칙을 다시 구현하지 않고 문서 순서 자동 배치로 냅니다";
const GRID_AUTO_SPAN: &str = "자동 배치 item의 열 span은 줄 넘김 배치 규칙을 다시 구현하지 않고 span 1로 냅니다";
const GRID_LINES: &str =
    "음수 line, row span, 끝 line만 명시한 배치는 대상 grid API로 낼 수 없어 문서 순서 자동 배치로 냅니다";

fn grid_plan(ir: &CodegenIr, grid: &LayoutStyle, flow: &[u32]) -> GridPlan {
    let explicit_columns = grid.grid_template_columns.len().max(1);
    let explicit_rows = grid.grid_template_rows.len();
    let explicit = |s: &LayoutStyle| {
        matches!(s.grid_column_start, GridLine::Line(_))
            || matches!(s.grid_row_start, GridLine::Line(_))
            || matches!(s.grid_column_end, GridLine::Line(_))
            || matches!(s.grid_row_end, GridLine::Line(_))
    };
    let styles: Vec<&LayoutStyle> = flow
        .iter()
        .map(|child| style(ir, *child).expect("출력 node는 layout이 있음"))
        .collect();
    let all_explicit = !styles.is_empty() && styles.iter().all(|s| explicit(s));
    let all_auto = styles.iter().all(|s| {
        !explicit(s)
            && matches!(s.grid_row_start, GridLine::Auto)
            && matches!(s.grid_row_end, GridLine::Auto | GridLine::Span(1))
    });
    let mut fallback = None;
    let mut cells = Vec::new();
    if all_explicit {
        for (child, s) in flow.iter().zip(&styles) {
            let (GridLine::Line(column), GridLine::Line(row)) =
                (s.grid_column_start, s.grid_row_start)
            else {
                fallback = Some(GRID_LINES);
                break;
            };
            let column_span = span_of(s.grid_column_start, s.grid_column_end);
            let row_span = span_of(s.grid_row_start, s.grid_row_end);
            match (column > 0 && row > 0, column_span, row_span) {
                (true, Some(span), Some(1)) => cells.push(GridCell {
                    child: *child,
                    column: (column - 1) as u16,
                    row: (row - 1) as u16,
                    column_span: span,
                }),
                _ => {
                    fallback = Some(GRID_LINES);
                    break;
                }
            }
        }
    } else if !all_auto {
        // 명시 line이 하나도 없으면 섞인 것이 아니라 자동 배치의 row span입니다.
        fallback = Some(if styles.iter().any(|s| explicit(s)) {
            GRID_MIXED
        } else {
            GRID_LINES
        });
    }
    if fallback.is_some() || !all_explicit {
        // 문서 순서 자동 배치(span 1).
        cells.clear();
        let column_flow = grid.grid_auto_flow == GridAutoFlow::Column;
        let lanes = if column_flow {
            explicit_rows.max(1)
        } else {
            explicit_columns
        };
        for (position, (child, s)) in flow.iter().zip(&styles).enumerate() {
            let (major, minor) = (position / lanes, position % lanes);
            let span = if fallback.is_none() && !column_flow {
                span_of(s.grid_column_start, s.grid_column_end).unwrap_or(1)
            } else {
                1
            };
            let (column, row) = if column_flow {
                (major, minor)
            } else {
                (minor, major)
            };
            cells.push(GridCell {
                child: *child,
                column: column as u16,
                row: row as u16,
                column_span: span,
            });
        }
        if fallback.is_none() && !column_flow && cells.iter().any(|c| c.column_span > 1) {
            // span이 있는 자동 배치는 칸 이동이 생기므로 span 1로 냅니다.
            fallback = Some(GRID_AUTO_SPAN);
            for cell in &mut cells {
                cell.column_span = 1;
            }
        }
    }
    cells.sort_by_key(|cell| (cell.row, cell.column));
    let column_count = cells
        .iter()
        .map(|c| (c.column + c.column_span) as usize)
        .max()
        .unwrap_or(0)
        .max(explicit_columns);
    let row_count = cells
        .iter()
        .map(|c| c.row as usize + 1)
        .max()
        .unwrap_or(0)
        .max(explicit_rows);
    GridPlan {
        columns: track_list(
            &grid.grid_template_columns,
            &grid.grid_auto_columns,
            column_count,
        ),
        rows: track_list(&grid.grid_template_rows, &grid.grid_auto_rows, row_count),
        cells,
        fallback,
    }
}

/// grid track의 native 분류.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum TrackKind {
    /// `<length>` 고정.
    Fixed(f32),
    /// `minmax(0, <flex>)`: 남은 공간을 비율로 나눔.
    Flexible(f32),
    /// `<flex>`(최솟값 `auto`): 콘텐츠 최소 크기를 지키며 나눔.
    FlexibleAuto(f32),
    /// `auto`·`min-content`·`max-content`·`fit-content()`: 콘텐츠 크기.
    Content,
    /// 그 밖(`%`, 섞인 `minmax()`).
    Other,
}

/// track을 분류합니다.
pub(crate) fn track_kind(track: &TrackSize) -> TrackKind {
    match (track.min, track.max) {
        (MinTrackSize::Length(a), MaxTrackSize::Length(b)) if a == b => TrackKind::Fixed(a.get()),
        (MinTrackSize::Length(zero), MaxTrackSize::Fraction(fr)) if zero.get() == 0.0 => {
            TrackKind::Flexible(fr.get())
        }
        (MinTrackSize::Auto, MaxTrackSize::Fraction(fr)) => TrackKind::FlexibleAuto(fr.get()),
        (MinTrackSize::Auto, MaxTrackSize::Auto)
        | (MinTrackSize::MinContent, MaxTrackSize::MinContent)
        | (MinTrackSize::MaxContent, MaxTrackSize::MaxContent)
        | (MinTrackSize::Auto, MaxTrackSize::FitContent(_)) => TrackKind::Content,
        _ => TrackKind::Other,
    }
}

/// flex item의 grow 분배가 플랫폼 균등·비율 분배와 같은지: grow가 있는 형제 모두 `flex-basis: 0`(또는 `0%`)이고
/// main 크기가 `auto`인 경우입니다. 플랫폼 분배는 콘텐츠 크기를 빼지 않고 남은 공간을 나누기 때문입니다.
pub(crate) fn grow_is_basis_zero(child: &LayoutStyle, main: Axis) -> bool {
    let basis_zero = match child.flex_basis {
        FlexBasis::Length(length) => length.get() == 0.0,
        FlexBasis::Percentage(percentage) => percentage.get() == 0.0,
        FlexBasis::Auto | FlexBasis::Content => false,
    };
    basis_zero && size(child, main) == Size::Auto
}

/// 정렬 값이 시작·끝·가운데 중 무엇인지(`baseline`은 시작으로 봄).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Edge {
    /// 시작(위·왼쪽).
    Start,
    /// 가운데.
    Center,
    /// 끝(아래·오른쪽).
    End,
}

/// item 정렬 → 위치. stretch·normal은 시작입니다(채움은 [`Fill`]이 따로 다룹니다).
pub(crate) fn item_edge(alignment: ItemAlignment) -> Edge {
    match alignment {
        ItemAlignment::Center => Edge::Center,
        ItemAlignment::End | ItemAlignment::FlexEnd => Edge::End,
        _ => Edge::Start,
    }
}

/// `align-self`가 명시한 정렬(없으면 `None`).
pub(crate) fn explicit_self(alignment: SelfAlignment) -> Option<ItemAlignment> {
    match alignment {
        SelfAlignment::Auto => None,
        SelfAlignment::Item(item) => Some(item),
    }
}

/// flex container의 main 축 정렬(`reverse`면 flex-start·flex-end가 반대쪽).
pub(crate) fn justify_edge(value: ContentAlignment, reverse: bool) -> Edge {
    let (start, end) = if reverse {
        (Edge::End, Edge::Start)
    } else {
        (Edge::Start, Edge::End)
    };
    match value {
        ContentAlignment::Center => Edge::Center,
        ContentAlignment::Start => Edge::Start,
        ContentAlignment::End => Edge::End,
        ContentAlignment::FlexEnd => end,
        _ => start,
    }
}
