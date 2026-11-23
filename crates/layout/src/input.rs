//! 레이아웃 입력 (X.1.1, ADR-0076).
//!
//! [`LayoutInput`]은 레이아웃 엔진이 읽는 node tree 하나입니다. node 하나는 CSS longhand 묶음
//! [`LayoutStyle`]과 leaf 콘텐츠, local transform을 가집니다. 문서 모델과 분리된 레이아웃 전용 값만 담으며
//! (문서를 수정하지 않음), 모든 float은 finite 타입이라 NaN·±무한대를 담을 수 없습니다.
//! [`LayoutInput::validate`]는 구조(tree)와 값 범위, 받아들이는 CSS 부분집합을 검사합니다.
//!
//! # 자식 순서
//!
//! 자식은 [`LayoutChild::order_key`] 오름차순(같은 키는 `StableId` tie-break, III.5)으로 정렬한
//! **visual order**(= DOM 순서)로 배치합니다. 입력 목록의 삽입 순서는 결과에 영향을 주지 않습니다.
//!
//! # canonical bytes
//!
//! [`LayoutInput::encode`]/[`LayoutInput::decode`]는 runtime·fixture 비교용 little-endian 형식
//! (magic `SLYI`, version 2)입니다. 저장 포맷이 아니며, 디코딩은 비유한 float과 비정규 OrderKey를 거부합니다.

use std::collections::HashMap;

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::value::{FiniteF32, Length, Percentage, Transform2D};

use crate::codec::{Reader, Writer};
use crate::error::LayoutError;
use crate::intrinsic::{
    decode_measured, encode_measured, validate_measured, IntrinsicMeasure, MeasuredContent,
};
use crate::property::LayoutProperty;
use crate::rounding::LayoutUnit;
use crate::style::{
    Display, FlexBasis, GridLine, ItemAlignment, LayoutStyle, LengthPercentage,
    LengthPercentageAuto, MaxSize, MaxTrackSize, MinTrackSize, Position, SelfAlignment, Size,
    TrackSize,
};
use crate::style_codec::{decode_style, encode_style};

/// 레이아웃 재귀가 허용하는 최대 tree 깊이(root = 0).
///
/// 엔진은 깊이에 비례해 재귀합니다. [`crate::engine::INLINE_LAYOUT_DEPTH`]보다 깊은 tree는
/// [`crate::engine::DEEP_LAYOUT_STACK_BYTES`] stack을 가진 전용 thread에서 계산하므로, 이 깊이까지는
/// 호출 thread의 stack 크기·빌드 설정(debug/release)과 무관하게 stack overflow 없이 끝납니다.
pub const MAX_LAYOUT_DEPTH: usize = 256;

/// 한 축의 grid track 최대 수(명시 + 암시, line 1~65535).
pub const MAX_GRID_TRACKS: usize = 65_534;

/// 입력 canonical bytes magic.
const INPUT_MAGIC: &[u8; 4] = b"SLYI";
/// 입력 canonical bytes version.
const INPUT_VERSION: u16 = 2;

/// 부모가 가진 자식 참조 하나.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LayoutChild {
    /// 자식 node.
    pub id: StableId,
    /// 형제 사이 visual order 키.
    pub order_key: OrderKey,
}

/// leaf 콘텐츠의 고정 intrinsic 크기와 baseline (X.1.1).
///
/// min-content·max-content 너비가 모두 `width`이고 콘텐츠 높이가 `height`인 상자 안 콘텐츠입니다(줄바꿈 없는
/// 텍스트 한 줄, 고정 크기 그림 등). baseline은 콘텐츠 top에서의 거리입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IntrinsicContent {
    /// intrinsic 너비(비음수).
    pub width: Length,
    /// intrinsic 높이(비음수).
    pub height: Length,
    /// 첫 baseline(콘텐츠 top 기준). 없으면 `None`.
    pub baseline: Option<Length>,
}

/// node 콘텐츠 (X.1.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum LayoutContent {
    /// intrinsic 콘텐츠 없음(container 또는 내용 크기 없는 leaf).
    #[default]
    None,
    /// 미리 측정된 고정 intrinsic 콘텐츠 leaf. 자식을 가질 수 없습니다.
    Intrinsic(IntrinsicContent),
    /// 계산 중 측정하는 콘텐츠(X.6.4). `ComponentInstance`만 자식을 가질 수 있습니다.
    Measured(MeasuredContent),
}

impl LayoutContent {
    /// 자식을 가질 수 없는 leaf 콘텐츠인지 여부.
    pub fn is_leaf(&self) -> bool {
        match self {
            LayoutContent::None => false,
            LayoutContent::Intrinsic(_) => true,
            LayoutContent::Measured(content) => !content.kind().allows_children(),
        }
    }
}

/// 레이아웃 node 하나의 입력 (X.1.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutNode {
    /// node 식별자.
    pub id: StableId,
    /// 자식(삽입 순서 무관, visual order는 OrderKey).
    pub children: Vec<LayoutChild>,
    /// CSS 레이아웃 longhand.
    pub style: LayoutStyle,
    /// 콘텐츠.
    pub content: LayoutContent,
    /// local transform(X.2.4). 레이아웃 계산에 참여하지 않고 layout rect 확정 뒤 적용합니다.
    pub transform: Transform2D,
}

impl LayoutNode {
    /// CSS 생략값 style, 자식 없음, 콘텐츠 없음 node를 만듭니다.
    pub fn new(id: StableId) -> LayoutNode {
        LayoutNode {
            id,
            children: Vec::new(),
            style: LayoutStyle::default(),
            content: LayoutContent::None,
            transform: Transform2D::identity(),
        }
    }

    /// style을 지정합니다.
    pub fn with_style(mut self, style: LayoutStyle) -> LayoutNode {
        self.style = style;
        self
    }

    /// 자식을 추가합니다.
    pub fn with_child(mut self, id: StableId, order_key: OrderKey) -> LayoutNode {
        self.children.push(LayoutChild { id, order_key });
        self
    }

    /// 콘텐츠를 지정합니다.
    pub fn with_content(mut self, content: LayoutContent) -> LayoutNode {
        self.content = content;
        self
    }

    /// local transform을 지정합니다.
    pub fn with_transform(mut self, transform: Transform2D) -> LayoutNode {
        self.transform = transform;
        self
    }

    /// 부모 flow에 참여하는지 여부([`LayoutStyle::is_in_flow`]).
    pub fn is_in_flow(&self) -> bool {
        self.style.is_in_flow()
    }
}

/// 레이아웃 입력 tree (X.1.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutInput {
    /// root node.
    pub root: StableId,
    /// 모든 node(순서 무관).
    pub nodes: Vec<LayoutNode>,
}

/// 검증된 tree index (crate 내부).
#[derive(Debug, Clone)]
pub(crate) struct TreeIndex {
    /// StableId → `nodes` index.
    pub(crate) index: HashMap<StableId, usize>,
    /// 부모 index.
    pub(crate) parent: Vec<Option<usize>>,
    /// visual order로 정렬한 자식 index.
    pub(crate) children: Vec<Vec<usize>>,
    /// root index.
    pub(crate) root: usize,
    /// tree 최대 깊이(root = 0, [`MAX_LAYOUT_DEPTH`] 이하). 계산 thread 선택에만 쓰입니다.
    pub(crate) depth: usize,
}

impl LayoutInput {
    /// 입력을 만듭니다(검증은 [`LayoutInput::validate`]).
    pub fn new(root: StableId, nodes: Vec<LayoutNode>) -> LayoutInput {
        LayoutInput { root, nodes }
    }

    /// StableId로 node를 찾습니다.
    pub fn node(&self, id: StableId) -> Option<&LayoutNode> {
        self.nodes.iter().find(|node| node.id == id)
    }

    /// 구조(tree)와 값 범위를 검증합니다.
    pub fn validate(&self) -> Result<(), LayoutError> {
        self.tree().map(|_| ())
    }

    /// 검증하고 tree index를 만듭니다.
    pub(crate) fn tree(&self) -> Result<TreeIndex, LayoutError> {
        let mut index = HashMap::with_capacity(self.nodes.len());
        for (position, node) in self.nodes.iter().enumerate() {
            if index.insert(node.id, position).is_some() {
                return Err(LayoutError::DuplicateNode { node: node.id });
            }
        }
        let root = *index
            .get(&self.root)
            .ok_or(LayoutError::MissingRoot { root: self.root })?;

        let mut parent: Vec<Option<usize>> = vec![None; self.nodes.len()];
        let mut children: Vec<Vec<usize>> = Vec::with_capacity(self.nodes.len());
        for (position, node) in self.nodes.iter().enumerate() {
            if !node.children.is_empty() && node.content.is_leaf() {
                return Err(LayoutError::ChildrenOnContentLeaf { node: node.id });
            }
            let mut ordered: Vec<(&OrderKey, StableId, usize)> =
                Vec::with_capacity(node.children.len());
            for child in &node.children {
                let child_index = *index.get(&child.id).ok_or(LayoutError::MissingChild {
                    parent: node.id,
                    child: child.id,
                })?;
                if child_index == root {
                    return Err(LayoutError::Cycle { node: child.id });
                }
                if parent[child_index].is_some() {
                    return Err(LayoutError::MultipleParents { node: child.id });
                }
                parent[child_index] = Some(position);
                ordered.push((&child.order_key, child.id, child_index));
            }
            ordered.sort_by(|a, b| a.0.cmp(b.0).then(a.1.cmp(&b.1)));
            children.push(ordered.into_iter().map(|(_, _, i)| i).collect());
        }

        let mut visited = vec![false; self.nodes.len()];
        let mut stack = vec![(root, 0usize)];
        let mut max_depth = 0usize;
        while let Some((current, depth)) = stack.pop() {
            if depth > MAX_LAYOUT_DEPTH {
                return Err(LayoutError::DepthLimitExceeded {
                    node: self.nodes[current].id,
                    limit: MAX_LAYOUT_DEPTH,
                });
            }
            max_depth = max_depth.max(depth);
            visited[current] = true;
            for &child in &children[current] {
                stack.push((child, depth + 1));
            }
        }
        for (position, seen) in visited.iter().enumerate() {
            if !*seen {
                let node = self.nodes[position].id;
                return Err(if parent_chain_loops(&parent, position) {
                    LayoutError::Cycle { node }
                } else {
                    LayoutError::Unreachable { node }
                });
            }
        }

        for (position, node) in self.nodes.iter().enumerate() {
            validate_node_values(node, position == root)?;
            if let Some(parent_index) = parent[position] {
                validate_in_parent(node, &self.nodes[parent_index])?;
            }
        }
        Ok(TreeIndex {
            index,
            parent,
            children,
            root,
            depth: max_depth,
        })
    }

    /// canonical bytes로 인코딩합니다.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.bytes(INPUT_MAGIC);
        w.u16(INPUT_VERSION);
        w.id(self.root);
        w.len(self.nodes.len());
        for node in &self.nodes {
            encode_node(&mut w, node);
        }
        w.finish()
    }

    /// canonical bytes에서 디코딩합니다. 형식·finite 검사만 하며 tree 검증은
    /// [`LayoutInput::validate`]가 합니다.
    pub fn decode(bytes: &[u8]) -> Result<LayoutInput, LayoutError> {
        let mut r = Reader::new(bytes);
        r.expect_magic(INPUT_MAGIC, INPUT_VERSION)?;
        let root = r.id()?;
        let count = r.len(16)?;
        let mut nodes = Vec::with_capacity(count);
        for _ in 0..count {
            nodes.push(decode_node(&mut r)?);
        }
        r.expect_end()?;
        Ok(LayoutInput { root, nodes })
    }
}

/// 부모 사슬이 root(부모 없음)에 닿지 않고 순환하는지 검사합니다.
fn parent_chain_loops(parent: &[Option<usize>], start: usize) -> bool {
    let mut current = start;
    for _ in 0..=parent.len() {
        match parent[current] {
            Some(next) => current = next,
            None => return false,
        }
    }
    true
}

/// 길이 값을 subpixel로 변환합니다. 범위 밖은 `OutOfRange`, 음수 불허 시 음수는 `NegativeValue`.
pub(crate) fn length_unit(
    node: StableId,
    property: LayoutProperty,
    value: Length,
    allow_negative: bool,
) -> Result<LayoutUnit, LayoutError> {
    if !allow_negative && value.get() < 0.0 {
        return Err(LayoutError::NegativeValue { node, property });
    }
    LayoutUnit::from_length(value).map_err(|_| LayoutError::OutOfRange { node, property })
}

/// 백분율 값 범위(비음수 여부)를 검사합니다.
fn check_percentage(
    node: StableId,
    property: LayoutProperty,
    value: Percentage,
    allow_negative: bool,
) -> Result<(), LayoutError> {
    if !allow_negative && value.get() < 0.0 {
        return Err(LayoutError::NegativeValue { node, property });
    }
    // 1e6%를 넘는 값은 어떤 기준 길이에서도 LayoutUnit 범위를 넘을 수 있어 거부합니다.
    if value.get().abs() > 1_000_000.0 {
        return Err(LayoutError::OutOfRange { node, property });
    }
    Ok(())
}

/// flex·grid 가중치(`flex-grow`·`flex-shrink`·`fr`) 값을 1/64 단위 정수 가중치로 바꿉니다.
///
/// 음수는 `NegativeValue`, 0이 아닌데 1/64 격자에서 0이 되는 값(1/128 미만)은 `BelowResolution`입니다.
pub(crate) fn weight(
    node: StableId,
    property: LayoutProperty,
    value: FiniteF32,
) -> Result<u64, LayoutError> {
    if value.get() < 0.0 {
        return Err(LayoutError::NegativeValue { node, property });
    }
    let unit =
        LayoutUnit::from_px(value).map_err(|_| LayoutError::OutOfRange { node, property })?;
    if unit.raw() == 0 && value.get() != 0.0 {
        return Err(LayoutError::BelowResolution { node, property });
    }
    Ok(u64::try_from(unit.raw()).unwrap_or(0))
}

fn check_size(node: StableId, property: LayoutProperty, value: Size) -> Result<(), LayoutError> {
    match value {
        Size::Length(length) => length_unit(node, property, length, false).map(|_| ()),
        Size::Percentage(percent) => check_percentage(node, property, percent, false),
        Size::Auto | Size::MinContent | Size::MaxContent | Size::FitContent => Ok(()),
    }
}

fn check_max(node: StableId, property: LayoutProperty, value: MaxSize) -> Result<(), LayoutError> {
    match value {
        MaxSize::Length(length) => length_unit(node, property, length, false).map(|_| ()),
        MaxSize::Percentage(percent) => check_percentage(node, property, percent, false),
        MaxSize::None | MaxSize::MinContent | MaxSize::MaxContent | MaxSize::FitContent => Ok(()),
    }
}

fn check_inset(
    node: StableId,
    property: LayoutProperty,
    value: LengthPercentageAuto,
) -> Result<(), LayoutError> {
    match value {
        LengthPercentageAuto::Auto => Ok(()),
        LengthPercentageAuto::Length(length) => {
            length_unit(node, property, length, true).map(|_| ())
        }
        LengthPercentageAuto::Percentage(percent) => {
            check_percentage(node, property, percent, true)
        }
    }
}

fn check_tracks(
    node: StableId,
    property: LayoutProperty,
    tracks: &[TrackSize],
) -> Result<(), LayoutError> {
    if tracks.len() > MAX_GRID_TRACKS {
        return Err(LayoutError::InvalidValue {
            node,
            property,
            reason: "track 수가 상한을 넘습니다",
        });
    }
    for track in tracks {
        match track.min {
            MinTrackSize::Length(length) => {
                length_unit(node, property, length, false)?;
            }
            MinTrackSize::Percentage(percent) => check_percentage(node, property, percent, false)?,
            MinTrackSize::Auto | MinTrackSize::MinContent | MinTrackSize::MaxContent => {}
        }
        match track.max {
            MaxTrackSize::Length(length) => {
                length_unit(node, property, length, false)?;
            }
            MaxTrackSize::Percentage(percent) => check_percentage(node, property, percent, false)?,
            MaxTrackSize::Fraction(factor) => {
                weight(node, property, factor)?;
            }
            MaxTrackSize::FitContent(LengthPercentage::Length(length)) => {
                length_unit(node, property, length, false)?;
            }
            MaxTrackSize::FitContent(LengthPercentage::Percentage(percent)) => {
                check_percentage(node, property, percent, false)?
            }
            MaxTrackSize::Auto | MaxTrackSize::MinContent | MaxTrackSize::MaxContent => {}
        }
    }
    Ok(())
}

fn check_line(node: StableId, property: LayoutProperty, line: GridLine) -> Result<(), LayoutError> {
    match line {
        GridLine::Auto => Ok(()),
        GridLine::Line(0) => Err(LayoutError::InvalidValue {
            node,
            property,
            reason: "grid line 번호는 0일 수 없습니다",
        }),
        GridLine::Line(n) if usize::from(n.unsigned_abs()) > MAX_GRID_TRACKS + 1 => {
            Err(LayoutError::InvalidValue {
                node,
                property,
                reason: "grid line 번호가 track 상한을 넘습니다",
            })
        }
        GridLine::Span(0) => Err(LayoutError::InvalidValue {
            node,
            property,
            reason: "span은 1 이상이어야 합니다",
        }),
        GridLine::Span(n) if usize::from(n) > MAX_GRID_TRACKS => Err(LayoutError::InvalidValue {
            node,
            property,
            reason: "span이 track 상한을 넘습니다",
        }),
        GridLine::Line(_) | GridLine::Span(_) => Ok(()),
    }
}

/// node 하나의 값 범위와 속성 조합을 검증합니다.
pub(crate) fn validate_node_values(node: &LayoutNode, is_root: bool) -> Result<(), LayoutError> {
    let id = node.id;
    let s = &node.style;
    if is_root && s.display == Display::None {
        return Err(LayoutError::InvalidValue {
            node: id,
            property: LayoutProperty::Display,
            reason: "root는 display: none일 수 없습니다",
        });
    }
    let (top, right, bottom, left) = s.padding.components();
    for (property, value) in [
        (LayoutProperty::PaddingTop, top),
        (LayoutProperty::PaddingRight, right),
        (LayoutProperty::PaddingBottom, bottom),
        (LayoutProperty::PaddingLeft, left),
    ] {
        let length =
            Length::new(value).map_err(|_| LayoutError::OutOfRange { node: id, property })?;
        length_unit(id, property, length, false)?;
    }
    for (property, value) in [
        (LayoutProperty::Top, s.top),
        (LayoutProperty::Right, s.right),
        (LayoutProperty::Bottom, s.bottom),
        (LayoutProperty::Left, s.left),
    ] {
        check_inset(id, property, value)?;
    }
    check_size(id, LayoutProperty::Width, s.width)?;
    check_size(id, LayoutProperty::Height, s.height)?;
    check_size(id, LayoutProperty::MinWidth, s.min_width)?;
    check_size(id, LayoutProperty::MinHeight, s.min_height)?;
    check_max(id, LayoutProperty::MaxWidth, s.max_width)?;
    check_max(id, LayoutProperty::MaxHeight, s.max_height)?;
    if s.aspect_ratio.is_some()
        && s.box_sizing == crate::style::BoxSizing::ContentBox
        && (top, right, bottom, left) != (0.0, 0.0, 0.0, 0.0)
    {
        // CSS는 비율을 box-sizing 상자에 적용합니다. 엔진은 비율 변환을 border box에서 하므로 padding이 있는
        // content-box 상자는 같은 결과를 보장하지 못합니다.
        return Err(LayoutError::Unsupported {
            node: id,
            property: LayoutProperty::BoxSizing,
            reason: "padding이 있는 content-box 상자의 aspect-ratio는 아직 지원하지 않습니다",
        });
    }
    if let Some(ratio) = s.aspect_ratio {
        for component in [ratio.width, ratio.height] {
            if component.get() <= 0.0 {
                return Err(LayoutError::InvalidValue {
                    node: id,
                    property: LayoutProperty::AspectRatio,
                    reason: "aspect-ratio 성분은 양수여야 합니다",
                });
            }
            weight(id, LayoutProperty::AspectRatio, component)?;
        }
    }
    length_unit(id, LayoutProperty::RowGap, s.row_gap, false)?;
    length_unit(id, LayoutProperty::ColumnGap, s.column_gap, false)?;
    weight(id, LayoutProperty::FlexGrow, s.flex_grow)?;
    weight(id, LayoutProperty::FlexShrink, s.flex_shrink)?;
    match s.flex_basis {
        FlexBasis::Length(length) => {
            length_unit(id, LayoutProperty::FlexBasis, length, false)?;
        }
        FlexBasis::Percentage(percent) => {
            check_percentage(id, LayoutProperty::FlexBasis, percent, false)?
        }
        FlexBasis::Auto | FlexBasis::Content => {}
    }
    if s.justify_items == ItemAlignment::Baseline
        || s.justify_self == SelfAlignment::Item(ItemAlignment::Baseline)
    {
        return Err(LayoutError::Unsupported {
            node: id,
            property: if s.justify_items == ItemAlignment::Baseline {
                LayoutProperty::JustifyItems
            } else {
                LayoutProperty::JustifySelf
            },
            reason: "가로 축 baseline 정렬은 받지 않습니다",
        });
    }
    check_tracks(
        id,
        LayoutProperty::GridTemplateColumns,
        &s.grid_template_columns,
    )?;
    check_tracks(id, LayoutProperty::GridTemplateRows, &s.grid_template_rows)?;
    check_tracks(id, LayoutProperty::GridAutoColumns, &s.grid_auto_columns)?;
    check_tracks(id, LayoutProperty::GridAutoRows, &s.grid_auto_rows)?;
    for (property, tracks) in [
        (LayoutProperty::GridAutoColumns, &s.grid_auto_columns),
        (LayoutProperty::GridAutoRows, &s.grid_auto_rows),
    ] {
        if tracks.is_empty() {
            return Err(LayoutError::InvalidValue {
                node: id,
                property,
                reason: "암시 track pattern은 비어 있을 수 없습니다",
            });
        }
    }
    check_line(id, LayoutProperty::GridColumnStart, s.grid_column_start)?;
    check_line(id, LayoutProperty::GridColumnEnd, s.grid_column_end)?;
    check_line(id, LayoutProperty::GridRowStart, s.grid_row_start)?;
    check_line(id, LayoutProperty::GridRowEnd, s.grid_row_end)?;
    match &node.content {
        LayoutContent::None => {}
        LayoutContent::Intrinsic(content) => {
            length_unit(id, LayoutProperty::Content, content.width, false)?;
            length_unit(id, LayoutProperty::Content, content.height, false)?;
            if let Some(baseline) = content.baseline {
                length_unit(id, LayoutProperty::Content, baseline, true)?;
            }
        }
        LayoutContent::Measured(content) => validate_measured(id, content)?,
    }
    Ok(())
}

/// 자식 node의 속성이 부모 배치 방식과 맞는지 검증합니다(받아들이는 부분집합).
pub(crate) fn validate_in_parent(
    node: &LayoutNode,
    parent: &LayoutNode,
) -> Result<(), LayoutError> {
    let s = &node.style;
    if s.display == Display::None {
        return Ok(());
    }
    let grid = parent.style.display == Display::Grid;
    if s.position == Position::Absolute {
        let lines = [
            s.grid_column_start,
            s.grid_column_end,
            s.grid_row_start,
            s.grid_row_end,
        ];
        if grid && lines.iter().any(|line| *line != GridLine::Auto) {
            return Err(LayoutError::Unsupported {
                node: node.id,
                property: LayoutProperty::GridColumnStart,
                reason: "grid 안 absolute 자식의 명시 line 배치는 받지 않습니다",
            });
        }
        return Ok(());
    }
    let baseline = s.align_self == SelfAlignment::Item(ItemAlignment::Baseline)
        || (s.align_self == SelfAlignment::Auto
            && parent.style.align_items == ItemAlignment::Baseline);
    if baseline && grid {
        return Err(LayoutError::Unsupported {
            node: node.id,
            property: LayoutProperty::AlignSelf,
            reason: "grid item의 baseline 정렬은 받지 않습니다",
        });
    }
    Ok(())
}

// --- canonical bytes ---

const CONTENT_NONE: u8 = 0;
const CONTENT_INTRINSIC: u8 = 1;
const CONTENT_MEASURED: u8 = 2;

/// node 하나의 **레이아웃 입력** bytes(transform 제외, X.7.2 측정 key용).
///
/// transform은 레이아웃 결과에 영향을 주지 않으므로(X.2.4) cache key에서 뺍니다.
pub(crate) fn node_layout_bytes(node: &LayoutNode) -> Vec<u8> {
    let mut w = Writer::new();
    encode_layout_part(&mut w, node);
    w.finish()
}

fn encode_layout_part(w: &mut Writer, node: &LayoutNode) {
    w.id(node.id);
    w.len(node.children.len());
    for child in &node.children {
        w.id(child.id);
        w.order_key(&child.order_key);
    }
    encode_style(w, &node.style);
    match &node.content {
        LayoutContent::None => w.u8(CONTENT_NONE),
        LayoutContent::Intrinsic(content) => {
            w.u8(CONTENT_INTRINSIC);
            w.length(content.width);
            w.length(content.height);
            match content.baseline {
                Some(baseline) => {
                    w.bool(true);
                    w.length(baseline);
                }
                None => w.bool(false),
            }
        }
        LayoutContent::Measured(content) => {
            w.u8(CONTENT_MEASURED);
            encode_measured(w, content);
        }
    }
}

fn encode_node(w: &mut Writer, node: &LayoutNode) {
    encode_layout_part(w, node);
    let (a, b, c, d, tx, ty) = node.transform.components();
    for component in [a, b, c, d, tx, ty] {
        w.bytes(&component.to_le_bytes());
    }
}

fn decode_node(r: &mut Reader<'_>) -> Result<LayoutNode, LayoutError> {
    let id = r.id()?;
    let child_count = r.len(20)?;
    let mut children = Vec::with_capacity(child_count);
    for _ in 0..child_count {
        let child = r.id()?;
        let order_key = r.order_key()?;
        children.push(LayoutChild {
            id: child,
            order_key,
        });
    }
    let style = decode_style(r)?;
    let content = match r.u8()? {
        CONTENT_NONE => LayoutContent::None,
        CONTENT_INTRINSIC => {
            let width = r.length("content width")?;
            let height = r.length("content height")?;
            let baseline = if r.bool("baseline flag")? {
                Some(r.length("content baseline")?)
            } else {
                None
            };
            LayoutContent::Intrinsic(IntrinsicContent {
                width,
                height,
                baseline,
            })
        }
        CONTENT_MEASURED => LayoutContent::Measured(decode_measured(r)?),
        _ => {
            return Err(LayoutError::Malformed {
                reason: "알 수 없는 content tag",
            })
        }
    };
    let mut components = [0.0f32; 6];
    for component in &mut components {
        *component = r.finite("transform")?.get();
    }
    let [a, b, c, d, tx, ty] = components;
    let transform = Transform2D::new(a, b, c, d, tx, ty)
        .map_err(|_| LayoutError::NonFinite { field: "transform" })?;
    Ok(LayoutNode {
        id,
        children,
        style,
        content,
        transform,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{Size, TrackSize};
    use crate::test_support::{id, key, len};
    use stellar_core_types::value::Insets;

    fn sample_input() -> LayoutInput {
        let root = LayoutNode::new(id(1))
            .with_style(LayoutStyle {
                width: Size::Length(len(200.0)),
                height: Size::Length(len(100.0)),
                padding: Insets::new(4.0, 8.0, 4.0, 8.0).unwrap(),
                grid_template_columns: vec![TrackSize::length(len(3.0))],
                ..LayoutStyle::default()
            })
            .with_child(id(3), key(2))
            .with_child(id(2), key(1));
        let text =
            LayoutNode::new(id(2)).with_content(LayoutContent::Intrinsic(IntrinsicContent {
                width: len(48.5),
                height: len(18.25),
                baseline: Some(len(14.0)),
            }));
        let image = LayoutNode::new(id(3))
            .with_style(LayoutStyle {
                width: Size::Length(len(1.0 / 3.0)),
                position: Position::Absolute,
                ..LayoutStyle::default()
            })
            .with_content(LayoutContent::Intrinsic(IntrinsicContent {
                width: len(640.0),
                height: len(480.0),
                baseline: None,
            }));
        LayoutInput::new(id(1), vec![root, text, image])
    }

    #[test]
    fn input_round_trips_through_canonical_bytes() {
        let input = sample_input();
        input.validate().unwrap();
        let bytes = input.encode();
        assert_eq!(&bytes[..4], b"SLYI");
        let decoded = LayoutInput::decode(&bytes).unwrap();
        assert_eq!(decoded, input);
        assert_eq!(decoded.encode(), bytes);
        decoded.validate().unwrap();
    }

    #[test]
    fn decode_rejects_non_finite_and_malformed_bytes() {
        let input = sample_input();
        let bytes = input.encode();
        // root width 값(f32) 위치: magic4+ver2+root16+count4 + node id16 + child count4
        // + 자식 2개(id16+key len4+key4)*2 + display1 + position1 + inset 4개(auto 1byte) + width tag1.
        let width_offset = 4 + 2 + 16 + 4 + 16 + 4 + (16 + 4 + 4) * 2 + 1 + 1 + 4 + 1;
        assert_eq!(
            f32::from_le_bytes(bytes[width_offset..width_offset + 4].try_into().unwrap()),
            200.0
        );
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut corrupt = bytes.clone();
            corrupt[width_offset..width_offset + 4].copy_from_slice(&bad.to_le_bytes());
            assert_eq!(
                LayoutInput::decode(&corrupt),
                Err(LayoutError::NonFinite { field: "width" })
            );
        }
        let mut truncated = bytes.clone();
        truncated.pop();
        assert!(matches!(
            LayoutInput::decode(&truncated),
            Err(LayoutError::Malformed { .. })
        ));
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(
            LayoutInput::decode(&trailing),
            Err(LayoutError::Malformed {
                reason: "trailing bytes"
            })
        );
        let mut magic = bytes;
        magic[0] = b'X';
        assert!(LayoutInput::decode(&magic).is_err());
    }

    #[test]
    fn value_validation_rejects_negative_out_of_range_and_invalid_css() {
        let single = |style: LayoutStyle| {
            LayoutInput::new(id(1), vec![LayoutNode::new(id(1)).with_style(style)])
        };
        assert_eq!(
            single(LayoutStyle {
                width: Size::Length(len(-1.0)),
                ..LayoutStyle::default()
            })
            .validate(),
            Err(LayoutError::NegativeValue {
                node: id(1),
                property: LayoutProperty::Width
            })
        );
        assert_eq!(
            single(LayoutStyle {
                height: Size::Length(len(33_554_432.0)),
                ..LayoutStyle::default()
            })
            .validate(),
            Err(LayoutError::OutOfRange {
                node: id(1),
                property: LayoutProperty::Height
            })
        );
        assert_eq!(
            single(LayoutStyle {
                padding: Insets::new(0.0, -2.0, 0.0, 0.0).unwrap(),
                ..LayoutStyle::default()
            })
            .validate(),
            Err(LayoutError::NegativeValue {
                node: id(1),
                property: LayoutProperty::PaddingRight
            })
        );
        // inset은 음수 허용.
        assert!(single(LayoutStyle {
            left: LengthPercentageAuto::Length(len(-5.0)),
            ..LayoutStyle::default()
        })
        .validate()
        .is_ok());
        assert_eq!(
            single(LayoutStyle {
                flex_grow: FiniteF32::new(0.001).unwrap(),
                ..LayoutStyle::default()
            })
            .validate(),
            Err(LayoutError::BelowResolution {
                node: id(1),
                property: LayoutProperty::FlexGrow
            })
        );
        assert!(matches!(
            single(LayoutStyle {
                grid_row_start: GridLine::Line(0),
                ..LayoutStyle::default()
            })
            .validate(),
            Err(LayoutError::InvalidValue { .. })
        ));
        assert!(matches!(
            single(LayoutStyle {
                display: Display::None,
                ..LayoutStyle::default()
            })
            .validate(),
            Err(LayoutError::InvalidValue {
                property: LayoutProperty::Display,
                ..
            })
        ));
        assert!(Length::new(f32::NAN).is_err());
        assert!(Insets::new(f32::INFINITY, 0.0, 0.0, 0.0).is_err());
    }

    #[test]
    fn structure_validation_errors() {
        let leaf = |n: u64| LayoutNode::new(id(n));
        let dup = LayoutInput::new(id(1), vec![leaf(1), leaf(1)]);
        assert_eq!(
            dup.validate(),
            Err(LayoutError::DuplicateNode { node: id(1) })
        );
        assert_eq!(
            LayoutInput::new(id(9), vec![leaf(1)]).validate(),
            Err(LayoutError::MissingRoot { root: id(9) })
        );
        let missing = LayoutInput::new(id(1), vec![leaf(1).with_child(id(2), key(1))]);
        assert_eq!(
            missing.validate(),
            Err(LayoutError::MissingChild {
                parent: id(1),
                child: id(2)
            })
        );
        let two = LayoutInput::new(
            id(1),
            vec![
                leaf(1).with_child(id(2), key(1)).with_child(id(3), key(2)),
                leaf(2).with_child(id(4), key(1)),
                leaf(3).with_child(id(4), key(1)),
                leaf(4),
            ],
        );
        assert_eq!(
            two.validate(),
            Err(LayoutError::MultipleParents { node: id(4) })
        );
        let root_child = LayoutInput::new(
            id(1),
            vec![
                leaf(1).with_child(id(2), key(1)),
                leaf(2).with_child(id(1), key(1)),
            ],
        );
        assert_eq!(
            root_child.validate(),
            Err(LayoutError::Cycle { node: id(1) })
        );
        let detached = LayoutInput::new(
            id(1),
            vec![
                leaf(1),
                leaf(2).with_child(id(3), key(1)),
                leaf(3).with_child(id(2), key(1)),
            ],
        );
        assert_eq!(detached.validate(), Err(LayoutError::Cycle { node: id(2) }));
        let orphan = LayoutInput::new(id(1), vec![leaf(1), leaf(2)]);
        assert_eq!(
            orphan.validate(),
            Err(LayoutError::Unreachable { node: id(2) })
        );
        let content_leaf = LayoutInput::new(
            id(1),
            vec![
                leaf(1)
                    .with_child(id(2), key(1))
                    .with_content(LayoutContent::Intrinsic(IntrinsicContent {
                        width: len(1.0),
                        height: len(1.0),
                        baseline: None,
                    })),
                leaf(2),
            ],
        );
        assert_eq!(
            content_leaf.validate(),
            Err(LayoutError::ChildrenOnContentLeaf { node: id(1) })
        );
    }

    #[test]
    fn depth_limit_is_enforced_without_recursion() {
        let depth = MAX_LAYOUT_DEPTH + 1;
        let mut nodes = Vec::new();
        for n in 0..=depth as u64 {
            let mut node = LayoutNode::new(id(n + 1));
            if n < depth as u64 {
                node = node.with_child(id(n + 2), key(1));
            }
            nodes.push(node);
        }
        let input = LayoutInput::new(id(1), nodes.clone());
        assert_eq!(
            input.validate(),
            Err(LayoutError::DepthLimitExceeded {
                node: id(depth as u64 + 1),
                limit: MAX_LAYOUT_DEPTH
            })
        );
        nodes.pop();
        nodes.last_mut().unwrap().children.clear();
        assert!(LayoutInput::new(id(1), nodes).validate().is_ok());
    }

    #[test]
    fn visual_order_follows_order_key_not_insertion() {
        let input = sample_input();
        let tree = input.tree().unwrap();
        let visual: Vec<StableId> = tree.children[tree.root]
            .iter()
            .map(|&i| input.nodes[i].id)
            .collect();
        assert_eq!(visual, vec![id(2), id(3)]);
        assert_eq!(tree.parent[tree.index[&id(2)]], Some(tree.root));
        assert!(input.node(id(3)).is_some());
        assert!(input.node(id(4)).is_none());
    }
}
