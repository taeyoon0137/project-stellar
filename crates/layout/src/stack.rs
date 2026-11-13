//! Stack(Flex 계열) 배치 (X.3.1~X.3.4).
//!
//! [`StackLayout`]은 자식을 한 축(main axis)을 따라 차례로 놓는 container 설정입니다.
//!
//! - `axis`: main axis(가로 = 왼쪽→오른쪽, 세로 = 위→아래).
//! - `direction`: `Forward`는 visual order대로 main 시작부터, `Reverse`는 main 끝부터 놓습니다.
//! - `gap`: 이웃한 자식 사이 main 방향 간격(비음수).
//! - `main_alignment`·`cross_alignment`: 남는 공간 분배와 cross 방향 정렬.
//! - padding은 모든 배치 방식이 공유하는 node padding([`crate::input::LayoutNode::padding`])을
//!   사용합니다. Stack 자식은 content box 안에 놓입니다.
//!
//! # 배치 (X.3.2)
//!
//! 1. 자식(visual order) 크기를 해석합니다. cross 축 `Fill`은 content cross 크기를 채우고,
//!    `Stretch` 정렬은 cross가 `HugContent`인 자식도 cross 크기로 늘립니다.
//! 2. main 합계 = 자식 main 크기 합 + `gap × (자식 수 − 1)`. 남는 공간 = content main − 합계.
//! 3. main 정렬로 시작 offset과 자식 앞뒤 공간을 정하고 visual order대로 놓습니다. 균등 분배의
//!    잔여 subpixel은 visual order 앞 칸부터 1씩 줍니다. 남는 공간이 0 이하이면 `SpaceBetween`은
//!    `Start`, `SpaceAround`·`SpaceEvenly`는 `Center`로 동작합니다(넘침 허용).
//! 4. `Reverse`는 정방향 배치를 content main 축에서 거울상으로 뒤집습니다(main 시작 = 반대쪽 끝).
//! 5. cross 위치는 `Start`(·`Stretch`) 0, `Center` `(cross − 크기)/2`(ties away from zero), `End`
//!    `cross − 크기`입니다.
//!
//! Hug 측정: main extent = 위 합계, cross extent = 자식 cross 크기 최댓값.
//!
//! # main 크기 (X.3.3)
//!
//! 자식 main 크기는 [`crate::flex`]의 grow·shrink·basis 분배로 정합니다. content main이 definite면
//! 남는(모자란) 공간을 분배하고, indefinite(main Hug 측정)면 base(hypothetical) 크기를 씁니다. 분배
//! 잔여 subpixel은 child visual order 앞에서부터 줍니다. main 축 `Fill`은 `grow 1, shrink 1, basis 0`
//! 입니다. 분배 뒤 자식은 정해진 main 크기(forced)로 다시 해석되어 cross 크기가 확정됩니다.
//!
//! # wrap과 여러 줄 정렬 (X.3.4)
//!
//! `wrap = Wrap`이고 content main이 definite면, visual order로 자식 hypothetical main 크기(+ gap)를
//! 누적하다 content main을 넘기 직전에 줄을 바꿉니다(줄마다 최소 한 자식). 줄마다 grow·shrink를 따로
//! 분배하고, 줄 cross 크기는 그 줄 자식 cross 최댓값입니다. 줄 사이 간격은 `cross_gap`이며, 줄
//! 묶음은 `line_alignment`(align-content)로 content cross 안에 정렬합니다. `Stretch`는 남는 cross
//! 공간을 줄에 균등 분배(잔여는 줄 순서)합니다. 자식은 자기 줄 안에서 `cross_alignment`를 따릅니다.
//! `NoWrap`(기본)은 한 줄이며 그 줄의 cross 크기는 content cross 전체입니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{FiniteF32, Length};

use crate::codec::{Reader, Writer};
use crate::distribute::distribute_equal;
use crate::engine::{at, AxisSizing, ChildConstraint, Engine};
use crate::error::LayoutError;
use crate::flex::{resolve_flexible_lengths, FlexInput, PreparedBasis, FILL_FACTOR};
use crate::geometry::{Axis, LayoutRect, LayoutSize};
use crate::input::{length_unit, ContainerLayout};
use crate::property::LayoutProperty;
use crate::rounding::{LayoutUnit, LayoutUnitError};

/// 배치 방향 (X.3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StackDirection {
    /// visual order대로 main 시작부터.
    #[default]
    Forward,
    /// visual order대로 main 끝부터(main 시작이 반대쪽).
    Reverse,
}

/// main axis 정렬(남는 공간 분배) (X.3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MainAlignment {
    /// main 시작에 붙임.
    #[default]
    Start,
    /// 가운데.
    Center,
    /// main 끝에 붙임.
    End,
    /// 남는 공간을 자식 사이에 균등 분배(양 끝 0).
    SpaceBetween,
    /// 각 자식 양옆에 같은 공간(양 끝은 사이 공간의 절반).
    SpaceAround,
    /// 양 끝과 자식 사이에 같은 공간.
    SpaceEvenly,
}

/// cross axis 정렬 (X.3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CrossAlignment {
    /// cross 시작.
    #[default]
    Start,
    /// 가운데.
    Center,
    /// cross 끝.
    End,
    /// 콘텐츠 크기(HugContent) 자식을 cross 크기로 늘림.
    Stretch,
}

/// 줄 바꿈 (X.3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StackWrap {
    /// 한 줄.
    #[default]
    NoWrap,
    /// content main을 넘기면 다음 줄.
    Wrap,
}

/// 여러 줄의 cross 정렬(align-content) (X.3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LineAlignment {
    /// cross 시작.
    #[default]
    Start,
    /// 가운데.
    Center,
    /// cross 끝.
    End,
    /// 줄 사이 균등.
    SpaceBetween,
    /// 줄마다 앞뒤 반 칸.
    SpaceAround,
    /// 양 끝과 줄 사이 균등.
    SpaceEvenly,
    /// 남는 공간을 줄 cross 크기에 균등 분배.
    Stretch,
}

/// Stack container 설정 (X.3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StackLayout {
    /// main axis.
    pub axis: Axis,
    /// 배치 방향.
    pub direction: StackDirection,
    /// 자식 사이 main 간격(비음수).
    pub gap: Length,
    /// main 정렬.
    pub main_alignment: MainAlignment,
    /// cross 정렬.
    pub cross_alignment: CrossAlignment,
    /// 줄 바꿈(X.3.4).
    pub wrap: StackWrap,
    /// 줄 사이 cross 간격(비음수, X.3.4).
    pub cross_gap: Length,
    /// 여러 줄 cross 정렬(X.3.4).
    pub line_alignment: LineAlignment,
}

impl StackLayout {
    /// 기본값(Forward, gap 0, Start/Start) Stack.
    pub fn new(axis: Axis) -> StackLayout {
        StackLayout {
            axis,
            direction: StackDirection::Forward,
            gap: Length::from_raw(FiniteF32::ZERO),
            main_alignment: MainAlignment::Start,
            cross_alignment: CrossAlignment::Start,
            wrap: StackWrap::NoWrap,
            cross_gap: Length::from_raw(FiniteF32::ZERO),
            line_alignment: LineAlignment::Start,
        }
    }
}

// --- canonical tag (golden 고정) ---

pub(crate) const fn axis_tag(axis: Axis) -> u8 {
    match axis {
        Axis::Horizontal => 0,
        Axis::Vertical => 1,
    }
}

pub(crate) fn axis_from_tag(tag: u8) -> Result<Axis, LayoutError> {
    match tag {
        0 => Ok(Axis::Horizontal),
        1 => Ok(Axis::Vertical),
        _ => Err(LayoutError::Malformed {
            reason: "알 수 없는 axis tag",
        }),
    }
}

const fn direction_tag(direction: StackDirection) -> u8 {
    match direction {
        StackDirection::Forward => 0,
        StackDirection::Reverse => 1,
    }
}

fn direction_from_tag(tag: u8) -> Result<StackDirection, LayoutError> {
    match tag {
        0 => Ok(StackDirection::Forward),
        1 => Ok(StackDirection::Reverse),
        _ => Err(LayoutError::Malformed {
            reason: "알 수 없는 direction tag",
        }),
    }
}

pub(crate) const fn main_alignment_tag(alignment: MainAlignment) -> u8 {
    match alignment {
        MainAlignment::Start => 0,
        MainAlignment::Center => 1,
        MainAlignment::End => 2,
        MainAlignment::SpaceBetween => 3,
        MainAlignment::SpaceAround => 4,
        MainAlignment::SpaceEvenly => 5,
    }
}

pub(crate) fn main_alignment_from_tag(tag: u8) -> Result<MainAlignment, LayoutError> {
    match tag {
        0 => Ok(MainAlignment::Start),
        1 => Ok(MainAlignment::Center),
        2 => Ok(MainAlignment::End),
        3 => Ok(MainAlignment::SpaceBetween),
        4 => Ok(MainAlignment::SpaceAround),
        5 => Ok(MainAlignment::SpaceEvenly),
        _ => Err(LayoutError::Malformed {
            reason: "알 수 없는 main alignment tag",
        }),
    }
}

const fn cross_alignment_tag(alignment: CrossAlignment) -> u8 {
    match alignment {
        CrossAlignment::Start => 0,
        CrossAlignment::Center => 1,
        CrossAlignment::End => 2,
        CrossAlignment::Stretch => 3,
    }
}

fn cross_alignment_from_tag(tag: u8) -> Result<CrossAlignment, LayoutError> {
    match tag {
        0 => Ok(CrossAlignment::Start),
        1 => Ok(CrossAlignment::Center),
        2 => Ok(CrossAlignment::End),
        3 => Ok(CrossAlignment::Stretch),
        _ => Err(LayoutError::Malformed {
            reason: "알 수 없는 cross alignment tag",
        }),
    }
}

/// Stack 설정 canonical bytes.
pub(crate) fn encode_stack(w: &mut Writer, stack: &StackLayout) {
    w.u8(axis_tag(stack.axis));
    w.u8(direction_tag(stack.direction));
    w.length(stack.gap);
    w.u8(main_alignment_tag(stack.main_alignment));
    w.u8(cross_alignment_tag(stack.cross_alignment));
}

/// Stack 설정 canonical bytes를 읽습니다.
pub(crate) fn decode_stack(r: &mut Reader<'_>) -> Result<StackLayout, LayoutError> {
    let axis = axis_from_tag(r.u8()?)?;
    let direction = direction_from_tag(r.u8()?)?;
    let gap = r.length("stack gap")?;
    let main_alignment = main_alignment_from_tag(r.u8()?)?;
    let cross_alignment = cross_alignment_from_tag(r.u8()?)?;
    Ok(StackLayout {
        main_alignment,
        cross_alignment,
        direction,
        gap,
        ..StackLayout::new(axis)
    })
}

const fn wrap_tag(wrap: StackWrap) -> u8 {
    match wrap {
        StackWrap::NoWrap => 0,
        StackWrap::Wrap => 1,
    }
}

const fn line_alignment_tag(alignment: LineAlignment) -> u8 {
    match alignment {
        LineAlignment::Start => 0,
        LineAlignment::Center => 1,
        LineAlignment::End => 2,
        LineAlignment::SpaceBetween => 3,
        LineAlignment::SpaceAround => 4,
        LineAlignment::SpaceEvenly => 5,
        LineAlignment::Stretch => 6,
    }
}

fn line_alignment_from_tag(tag: u8) -> Result<LineAlignment, LayoutError> {
    match tag {
        0 => Ok(LineAlignment::Start),
        1 => Ok(LineAlignment::Center),
        2 => Ok(LineAlignment::End),
        3 => Ok(LineAlignment::SpaceBetween),
        4 => Ok(LineAlignment::SpaceAround),
        5 => Ok(LineAlignment::SpaceEvenly),
        6 => Ok(LineAlignment::Stretch),
        _ => Err(LayoutError::Malformed {
            reason: "알 수 없는 line alignment tag",
        }),
    }
}

/// wrap 설정 canonical bytes(`wrap u8, cross_gap f32, line_alignment u8`, X.3.4). 기본 Stack
/// bytes([`encode_stack`]) 뒤에 이어 씁니다.
pub(crate) fn encode_stack_wrap(w: &mut Writer, stack: &StackLayout) {
    w.u8(wrap_tag(stack.wrap));
    w.length(stack.cross_gap);
    w.u8(line_alignment_tag(stack.line_alignment));
}

/// wrap 설정 canonical bytes를 읽어 `stack`에 채웁니다.
pub(crate) fn decode_stack_wrap(
    r: &mut Reader<'_>,
    stack: &mut StackLayout,
) -> Result<(), LayoutError> {
    stack.wrap = match r.u8()? {
        0 => StackWrap::NoWrap,
        1 => StackWrap::Wrap,
        _ => {
            return Err(LayoutError::Malformed {
                reason: "알 수 없는 wrap tag",
            })
        }
    };
    stack.cross_gap = r.length("stack cross gap")?;
    stack.line_alignment = line_alignment_from_tag(r.u8()?)?;
    Ok(())
}

/// Stack 설정 값을 검증합니다(gap·cross gap 비음수·범위).
pub(crate) fn validate_stack(node: StableId, stack: &StackLayout) -> Result<(), LayoutError> {
    length_unit(node, LayoutProperty::ContainerLayout, stack.gap, false)?;
    length_unit(
        node,
        LayoutProperty::ContainerLayout,
        stack.cross_gap,
        false,
    )?;
    Ok(())
}

// --- 배치 알고리즘 (X.3.2~X.3.4) ---

/// Stack 계산에 쓰는 변환된 설정.
#[derive(Debug, Clone, Copy)]
struct StackUnits {
    main: Axis,
    cross: Axis,
    gap: LayoutUnit,
    cross_gap: LayoutUnit,
    config: StackLayout,
}

fn stack_units(id: StableId, stack: &StackLayout) -> Result<StackUnits, LayoutError> {
    Ok(StackUnits {
        main: stack.axis,
        cross: stack.axis.cross(),
        gap: length_unit(id, LayoutProperty::ContainerLayout, stack.gap, false)?,
        cross_gap: length_unit(id, LayoutProperty::ContainerLayout, stack.cross_gap, false)?,
        config: *stack,
    })
}

/// node의 Stack 설정(Stack container에서만 호출).
fn stack_of(engine: &Engine<'_>, index: usize) -> Result<StackUnits, LayoutError> {
    let id = engine.nodes[index].id;
    match &engine.node(index).container {
        ContainerLayout::Stack(stack) => stack_units(id, stack),
        _ => Err(LayoutError::InvalidBox {
            node: id,
            reason: "Stack container가 아닙니다",
        }),
    }
}

/// 자식 하나의 base 제약(main 크기 분배 전). `cb`는 content box 크기(Percentage 기준),
/// `cross_target`은 cross 축 `Fill`이 채우고 `Stretch` 정렬이 HugContent 자식을 늘릴 line cross
/// 크기입니다(`None`이면 콘텐츠 크기).
fn child_constraint(
    engine: &Engine<'_>,
    child: usize,
    units: &StackUnits,
    cb: [Option<LayoutUnit>; 2],
    cross_target: Option<LayoutUnit>,
) -> ChildConstraint {
    let cross = units.cross.index();
    let mut fill = [None, None];
    fill[cross] = cross_target;
    let mut forced = [None, None];
    if units.config.cross_alignment == CrossAlignment::Stretch
        && engine.nodes[child].sizing[cross] == AxisSizing::Hug
    {
        forced[cross] = cross_target;
    }
    ChildConstraint { cb, fill, forced }
}

/// 자식들의 main 크기 합 + gap.
fn main_total(
    id: StableId,
    sizes: &[LayoutSize],
    units: &StackUnits,
) -> Result<LayoutUnit, LayoutError> {
    let mut total = LayoutUnit::ZERO;
    for (i, size) in sizes.iter().enumerate() {
        if i > 0 {
            total = total.checked_add(units.gap).map_err(at(id))?;
        }
        total = total.checked_add(size.get(units.main)).map_err(at(id))?;
    }
    Ok(total)
}

/// 자식들의 flex 입력(base, factor)과 base 제약.
fn flex_inputs(
    engine: &mut Engine<'_>,
    id: StableId,
    children: &[usize],
    units: &StackUnits,
    cb: [Option<LayoutUnit>; 2],
    cross_target: Option<LayoutUnit>,
) -> Result<Vec<(FlexInput, ChildConstraint)>, LayoutError> {
    let main = units.main.index();
    let mut inputs = Vec::with_capacity(children.len());
    for &child in children {
        let constraint = child_constraint(engine, child, units, cb, cross_target);
        let flex = engine.nodes[child].flex;
        let main_fill = engine.nodes[child].sizing[main] == AxisSizing::Fill;
        let base = match (flex.basis, cb[main]) {
            (PreparedBasis::Length(value), _) => value,
            (PreparedBasis::Percentage(percent), Some(extent)) => {
                extent.checked_mul_percent(percent).map_err(at(id))?
            }
            _ if main_fill => LayoutUnit::ZERO,
            _ => engine.resolve_size(child, constraint)?.get(units.main),
        };
        let (grow, shrink) = if main_fill {
            (FILL_FACTOR, FILL_FACTOR)
        } else {
            (flex.grow, flex.shrink)
        };
        let input = FlexInput {
            base,
            grow,
            shrink,
            min: LayoutUnit::ZERO,
            max: None,
        };
        inputs.push((input, constraint));
    }
    Ok(inputs)
}

/// 한 줄 자식들의 최종 크기(visual order). main 크기는 grow·shrink·basis로 분배합니다(X.3.3).
fn line_sizes(
    engine: &mut Engine<'_>,
    id: StableId,
    children: &[usize],
    units: &StackUnits,
    cb: [Option<LayoutUnit>; 2],
    cross_target: Option<LayoutUnit>,
) -> Result<Vec<LayoutSize>, LayoutError> {
    let main = units.main.index();
    let inputs = flex_inputs(engine, id, children, units, cb, cross_target)?;
    let flex: Vec<FlexInput> = inputs.iter().map(|(input, _)| *input).collect();
    let main_sizes: Vec<LayoutUnit> = match cb[main] {
        Some(extent) => {
            let mut gaps = LayoutUnit::ZERO;
            for _ in 1..children.len() {
                gaps = gaps.checked_add(units.gap).map_err(at(id))?;
            }
            let available = extent.checked_sub(gaps).map_err(at(id))?;
            resolve_flexible_lengths(available, &flex).map_err(at(id))?
        }
        None => flex.iter().map(|input| input.base.max(input.min)).collect(),
    };
    let mut sizes = Vec::with_capacity(children.len());
    for (&child, ((_, mut constraint), main_size)) in
        children.iter().zip(inputs.into_iter().zip(main_sizes))
    {
        constraint.forced[main] = Some(main_size);
        sizes.push(engine.resolve_size(child, constraint)?);
    }
    Ok(sizes)
}

/// 배치된 한 줄.
#[derive(Debug, Clone)]
struct Line {
    children: Vec<usize>,
    sizes: Vec<LayoutSize>,
    /// line cross 크기.
    cross: LayoutUnit,
}

/// 자식을 줄로 나누고 각 줄의 크기를 정합니다(X.3.4).
///
/// - `NoWrap` 또는 content main이 indefinite면 한 줄입니다. 한 줄의 cross 크기는 content cross가
///   definite면 그 값, 아니면 자식 cross 최댓값입니다.
/// - `Wrap`은 hypothetical main 크기(+ gap)가 content main을 넘기 직전에 줄을 바꿉니다(줄마다 최소
///   한 자식). 줄 cross 크기는 그 줄 자식 cross 최댓값이고, `LineAlignment::Stretch`면 남는 cross
///   공간을 줄에 균등 분배합니다(잔여는 줄 순서). 줄 cross 확정 뒤 cross `Fill`·Stretch 자식을 그
///   크기로 다시 해석합니다.
fn build_lines(
    engine: &mut Engine<'_>,
    id: StableId,
    children: &[usize],
    units: &StackUnits,
    cb: [Option<LayoutUnit>; 2],
) -> Result<Vec<Line>, LayoutError> {
    let main = units.main.index();
    let cross = units.cross.index();
    let wrapping = units.config.wrap == StackWrap::Wrap && cb[main].is_some();
    if !wrapping {
        let sizes = line_sizes(engine, id, children, units, cb, cb[cross])?;
        let line_cross = match cb[cross] {
            Some(extent) => extent,
            None => max_cross(&sizes, units),
        };
        return Ok(vec![Line {
            children: children.to_vec(),
            sizes,
            cross: line_cross,
        }]);
    }
    let extent = cb[main].unwrap_or(LayoutUnit::ZERO);
    // 1) hypothetical main 크기로 줄 나누기.
    let inputs = flex_inputs(engine, id, children, units, cb, None)?;
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut used = LayoutUnit::ZERO;
    for (&child, (input, _)) in children.iter().zip(&inputs) {
        let hypothetical = input.base.max(input.min);
        let start_new = match groups.last() {
            None => true,
            Some(current) if current.is_empty() => false,
            Some(_) => {
                let next = used
                    .checked_add(units.gap)
                    .and_then(|u| u.checked_add(hypothetical))
                    .map_err(at(id))?;
                next > extent
            }
        };
        if start_new {
            groups.push(vec![child]);
            used = hypothetical;
        } else if let Some(current) = groups.last_mut() {
            current.push(child);
            used = used
                .checked_add(units.gap)
                .and_then(|u| u.checked_add(hypothetical))
                .map_err(at(id))?;
        }
    }
    // 2) 줄별 분배(stretch 전)로 줄 cross 크기.
    let mut crosses = Vec::with_capacity(groups.len());
    for group in &groups {
        let sizes = line_sizes(engine, id, group, units, cb, None)?;
        crosses.push(max_cross(&sizes, units));
    }
    // 3) LineAlignment::Stretch: 남는 cross 공간을 줄에 균등 분배.
    if units.config.line_alignment == LineAlignment::Stretch {
        if let Some(cross_extent) = cb[cross] {
            let used = lines_total(id, &crosses, units)?;
            let free = cross_extent.checked_sub(used).map_err(at(id))?;
            if free > LayoutUnit::ZERO {
                let extra = distribute_equal(free, crosses.len());
                for (line_cross, add) in crosses.iter_mut().zip(extra) {
                    *line_cross = line_cross.checked_add(add).map_err(at(id))?;
                }
            }
        }
    }
    // 4) 줄 cross 확정 뒤 최종 크기.
    let mut lines = Vec::with_capacity(groups.len());
    for (group, line_cross) in groups.into_iter().zip(crosses) {
        let sizes = line_sizes(engine, id, &group, units, cb, Some(line_cross))?;
        lines.push(Line {
            children: group,
            sizes,
            cross: line_cross,
        });
    }
    Ok(lines)
}

fn max_cross(sizes: &[LayoutSize], units: &StackUnits) -> LayoutUnit {
    sizes
        .iter()
        .map(|size| size.get(units.cross))
        .max()
        .unwrap_or(LayoutUnit::ZERO)
}

/// 줄 cross 크기 합 + cross gap.
fn lines_total(
    id: StableId,
    crosses: &[LayoutUnit],
    units: &StackUnits,
) -> Result<LayoutUnit, LayoutError> {
    let mut total = LayoutUnit::ZERO;
    for (i, line_cross) in crosses.iter().enumerate() {
        if i > 0 {
            total = total.checked_add(units.cross_gap).map_err(at(id))?;
        }
        total = total.checked_add(*line_cross).map_err(at(id))?;
    }
    Ok(total)
}

/// Stack container의 content extent(Hug 측정).
pub(crate) fn measure(
    engine: &mut Engine<'_>,
    index: usize,
    inner: [Option<LayoutUnit>; 2],
) -> Result<[LayoutUnit; 2], LayoutError> {
    let id = engine.nodes[index].id;
    let units = stack_of(engine, index)?;
    let children = engine.children(index);
    let lines = build_lines(engine, id, &children, &units, inner)?;
    let mut extent = [LayoutUnit::ZERO; 2];
    for line in &lines {
        extent[units.main.index()] =
            extent[units.main.index()].max(main_total(id, &line.sizes, &units)?);
    }
    let crosses: Vec<LayoutUnit> = if lines.len() == 1 {
        // 한 줄: cross extent는 자식 cross 최댓값(definite cross의 줄 크기가 아님).
        vec![max_cross(&lines[0].sizes, &units)]
    } else {
        lines.iter().map(|line| line.cross).collect()
    };
    extent[units.cross.index()] = lines_total(id, &crosses, &units)?;
    Ok(extent)
}

/// main 정렬에 따른 (시작 offset, 자식 앞 공간, 자식 뒤 공간). 공간 목록 길이는 자식 수입니다.
///
/// 남는 공간이 0 이하이면 `SpaceBetween`은 `Start`, `SpaceAround`·`SpaceEvenly`는 `Center`처럼
/// 동작합니다.
fn main_spacing(
    alignment: MainAlignment,
    free: LayoutUnit,
    count: usize,
) -> Result<(LayoutUnit, Vec<LayoutUnit>, Vec<LayoutUnit>), LayoutUnitError> {
    let zeros = vec![LayoutUnit::ZERO; count];
    let positive = free > LayoutUnit::ZERO && count > 0;
    let effective = match alignment {
        MainAlignment::SpaceBetween if !positive || count < 2 => MainAlignment::Start,
        MainAlignment::SpaceAround | MainAlignment::SpaceEvenly if !positive => {
            MainAlignment::Center
        }
        other => other,
    };
    Ok(match effective {
        MainAlignment::Start => (LayoutUnit::ZERO, zeros.clone(), zeros),
        MainAlignment::Center => (free.checked_div_round(2)?, zeros.clone(), zeros),
        MainAlignment::End => (free, zeros.clone(), zeros),
        MainAlignment::SpaceBetween => {
            // 자식 사이 count-1칸. 마지막 자식 뒤는 0.
            let mut after = distribute_equal(free, count - 1);
            after.push(LayoutUnit::ZERO);
            (LayoutUnit::ZERO, zeros, after)
        }
        MainAlignment::SpaceAround => {
            // 자식마다 앞·뒤 반 칸: 2·count칸을 순서대로 분배.
            let slots = distribute_equal(free, count * 2);
            let before = slots.iter().step_by(2).copied().collect();
            let after = slots.iter().skip(1).step_by(2).copied().collect();
            (LayoutUnit::ZERO, before, after)
        }
        MainAlignment::SpaceEvenly => {
            // 양 끝 포함 count+1칸: 첫 칸은 시작 offset, 나머지는 각 자식 뒤.
            let slots = distribute_equal(free, count + 1);
            let after = slots[1..count]
                .iter()
                .copied()
                .chain([LayoutUnit::ZERO])
                .collect();
            (slots[0], zeros, after)
        }
    })
}

/// cross 정렬에 따른 cross 위치(line 기준).
fn cross_position(
    alignment: CrossAlignment,
    extent: LayoutUnit,
    size: LayoutUnit,
) -> Result<LayoutUnit, LayoutUnitError> {
    match alignment {
        CrossAlignment::Start | CrossAlignment::Stretch => Ok(LayoutUnit::ZERO),
        CrossAlignment::Center => extent.checked_sub(size)?.checked_div_round(2),
        CrossAlignment::End => extent.checked_sub(size),
    }
}

/// line alignment(align-content)에 따른 줄 시작 offset 목록(content cross 기준).
///
/// `NoWrap`(한 줄이 content cross 전체)이면 offset은 0입니다. `Wrap`이면 줄이 하나여도 정렬합니다.
fn line_offsets(
    id: StableId,
    lines: &[Line],
    units: &StackUnits,
    cross_extent: LayoutUnit,
) -> Result<Vec<LayoutUnit>, LayoutError> {
    if units.config.wrap == StackWrap::NoWrap {
        return Ok(vec![LayoutUnit::ZERO; lines.len()]);
    }
    let crosses: Vec<LayoutUnit> = lines.iter().map(|line| line.cross).collect();
    let free = cross_extent
        .checked_sub(lines_total(id, &crosses, units)?)
        .map_err(at(id))?;
    let alignment = match units.config.line_alignment {
        LineAlignment::Start | LineAlignment::Stretch => MainAlignment::Start,
        LineAlignment::Center => MainAlignment::Center,
        LineAlignment::End => MainAlignment::End,
        LineAlignment::SpaceBetween => MainAlignment::SpaceBetween,
        LineAlignment::SpaceAround => MainAlignment::SpaceAround,
        LineAlignment::SpaceEvenly => MainAlignment::SpaceEvenly,
    };
    let (offset, before, after) = main_spacing(alignment, free, lines.len()).map_err(at(id))?;
    let mut offsets = Vec::with_capacity(lines.len());
    let mut cursor = offset;
    for (i, line) in lines.iter().enumerate() {
        cursor = cursor.checked_add(before[i]).map_err(at(id))?;
        offsets.push(cursor);
        cursor = cursor
            .checked_add(line.cross)
            .and_then(|c| c.checked_add(after[i]))
            .and_then(|c| c.checked_add(units.cross_gap))
            .map_err(at(id))?;
    }
    Ok(offsets)
}

/// Stack container의 자식 배치. 각 자식의 rect(부모 local 좌표)를 visual order로 반환합니다.
pub(crate) fn place(
    engine: &mut Engine<'_>,
    index: usize,
    inner: LayoutSize,
) -> Result<Vec<(usize, LayoutRect)>, LayoutError> {
    let id = engine.nodes[index].id;
    let padding = engine.nodes[index].padding;
    let units = stack_of(engine, index)?;
    let inner_axes = [Some(inner.width()), Some(inner.height())];
    let children = engine.children(index);
    let lines = build_lines(engine, id, &children, &units, inner_axes)?;
    let main_extent = inner.get(units.main);
    let offsets = line_offsets(id, &lines, &units, inner.get(units.cross))?;

    let mut placements = Vec::with_capacity(children.len());
    for (line, line_offset) in lines.iter().zip(offsets) {
        let free = main_extent
            .checked_sub(main_total(id, &line.sizes, &units)?)
            .map_err(at(id))?;
        let (offset, before, after) =
            main_spacing(units.config.main_alignment, free, line.children.len()).map_err(at(id))?;
        let mut cursor = offset;
        for (i, (&child, size)) in line.children.iter().zip(&line.sizes).enumerate() {
            cursor = cursor.checked_add(before[i]).map_err(at(id))?;
            let main_size = size.get(units.main);
            let forward = cursor;
            cursor = cursor
                .checked_add(main_size)
                .and_then(|c| c.checked_add(after[i]))
                .and_then(|c| c.checked_add(units.gap))
                .map_err(at(id))?;
            let main_pos = match units.config.direction {
                StackDirection::Forward => forward,
                // Reverse: main 끝을 시작으로 삼아 정방향 배치를 거울상으로 뒤집습니다.
                StackDirection::Reverse => main_extent
                    .checked_sub(forward)
                    .and_then(|p| p.checked_sub(main_size))
                    .map_err(at(id))?,
            };
            let cross_pos = cross_position(
                units.config.cross_alignment,
                line.cross,
                size.get(units.cross),
            )
            .and_then(|p| p.checked_add(line_offset))
            .map_err(at(id))?;
            let main_abs = padding
                .start(units.main)
                .checked_add(main_pos)
                .map_err(at(id))?;
            let cross_abs = padding
                .start(units.cross)
                .checked_add(cross_pos)
                .map_err(at(id))?;
            let (x, y) = match units.main {
                Axis::Horizontal => (main_abs, cross_abs),
                Axis::Vertical => (cross_abs, main_abs),
            };
            placements.push((child, LayoutRect::from_origin_size(x, y, *size)));
        }
    }
    Ok(placements)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{ContainerLayout, LayoutInput, LayoutNode};
    use crate::test_support::{id, len};

    #[test]
    fn enum_tags_are_golden() {
        assert_eq!(
            [axis_tag(Axis::Horizontal), axis_tag(Axis::Vertical)],
            [0, 1]
        );
        assert_eq!(
            [
                direction_tag(StackDirection::Forward),
                direction_tag(StackDirection::Reverse)
            ],
            [0, 1]
        );
        let main = [
            MainAlignment::Start,
            MainAlignment::Center,
            MainAlignment::End,
            MainAlignment::SpaceBetween,
            MainAlignment::SpaceAround,
            MainAlignment::SpaceEvenly,
        ];
        let main_tags: Vec<u8> = main.iter().map(|a| main_alignment_tag(*a)).collect();
        assert_eq!(main_tags, vec![0, 1, 2, 3, 4, 5]);
        let cross = [
            CrossAlignment::Start,
            CrossAlignment::Center,
            CrossAlignment::End,
            CrossAlignment::Stretch,
        ];
        let cross_tags: Vec<u8> = cross.iter().map(|a| cross_alignment_tag(*a)).collect();
        assert_eq!(cross_tags, vec![0, 1, 2, 3]);
        // tag → enum 역변환과 미지 tag 거부.
        for (tag, value) in main_tags.iter().zip(main) {
            assert_eq!(main_alignment_from_tag(*tag).unwrap(), value);
        }
        for (tag, value) in cross_tags.iter().zip(cross) {
            assert_eq!(cross_alignment_from_tag(*tag).unwrap(), value);
        }
        assert!(axis_from_tag(2).is_err());
        assert!(direction_from_tag(2).is_err());
        assert!(main_alignment_from_tag(6).is_err());
        assert!(cross_alignment_from_tag(4).is_err());
        assert_eq!(StackDirection::default(), StackDirection::Forward);
        assert_eq!(MainAlignment::default(), MainAlignment::Start);
        assert_eq!(CrossAlignment::default(), CrossAlignment::Start);
    }

    #[test]
    fn stack_bytes_are_golden_and_round_trip() {
        let stack = StackLayout {
            direction: StackDirection::Reverse,
            gap: len(8.5),
            main_alignment: MainAlignment::SpaceEvenly,
            cross_alignment: CrossAlignment::Stretch,
            ..StackLayout::new(Axis::Vertical)
        };
        let mut w = Writer::new();
        encode_stack(&mut w, &stack);
        let bytes = w.finish();
        // axis 1, direction 1, gap 8.5f32 LE(0x41080000), main 5, cross 3.
        assert_eq!(bytes, vec![1, 1, 0x00, 0x00, 0x08, 0x41, 5, 3]);
        let mut r = Reader::new(&bytes);
        assert_eq!(decode_stack(&mut r).unwrap(), stack);

        // node 입력 전체 round-trip.
        let input = LayoutInput::new(
            id(1),
            vec![LayoutNode::new(id(1)).with_container(ContainerLayout::Stack(stack))],
        );
        assert_eq!(LayoutInput::decode(&input.encode()).unwrap(), input);
    }

    #[test]
    fn negative_gap_is_rejected() {
        let stack = StackLayout {
            gap: len(-1.0),
            ..StackLayout::new(Axis::Horizontal)
        };
        let input = LayoutInput::new(
            id(1),
            vec![LayoutNode::new(id(1)).with_container(ContainerLayout::Stack(stack))],
        );
        assert_eq!(
            input.validate(),
            Err(LayoutError::NegativeValue {
                node: id(1),
                property: LayoutProperty::ContainerLayout
            })
        );
    }

    use crate::engine::compute_layout;
    use crate::input::{IntrinsicContent, LayoutContent, SizingMode};
    use crate::output::LayoutResult;
    use crate::test_support::{key, px};
    use stellar_core_types::value::Insets;

    /// main 크기 (30, 40, 66), cross 크기 (20, 10, 30) 고정 자식 3개. content 200×50
    /// (padding left/right 10, top/bottom 5) → main 합계 136 + gap 8×2 = 152, 남는 공간 48.
    fn fixed_children_stack(stack: StackLayout) -> LayoutResult {
        let (outer_w, outer_h, pad) = match stack.axis {
            Axis::Horizontal => (220.0, 60.0, Insets::new(5.0, 10.0, 5.0, 10.0).unwrap()),
            Axis::Vertical => (60.0, 220.0, Insets::new(10.0, 5.0, 10.0, 5.0).unwrap()),
        };
        let root = LayoutNode::new(id(1))
            .with_fixed_size(len(outer_w), len(outer_h))
            .with_padding(pad)
            .with_container(ContainerLayout::Stack(stack))
            // 삽입 순서와 visual order를 다르게.
            .with_child(id(4), key(3))
            .with_child(id(2), key(1))
            .with_child(id(3), key(2));
        let mut nodes = vec![root];
        for (n, (main, cross)) in [(2u64, (30.0, 20.0)), (3, (40.0, 10.0)), (4, (66.0, 30.0))] {
            let (w, h) = match stack.axis {
                Axis::Horizontal => (main, cross),
                Axis::Vertical => (cross, main),
            };
            nodes.push(LayoutNode::new(id(n)).with_fixed_size(len(w), len(h)));
        }
        compute_layout(&LayoutInput::new(id(1), nodes)).unwrap()
    }

    /// 자식 (main 위치, cross 위치) 목록(visual order).
    fn positions(result: &LayoutResult, axis: Axis) -> Vec<(i32, i32)> {
        [id(2), id(3), id(4)]
            .iter()
            .map(|n| {
                let r = result.get(*n).unwrap().local_rect();
                let (main, cross) = (r.start(axis), r.start(axis.cross()));
                (main.raw() / 64, cross.raw() / 64)
            })
            .collect()
    }

    #[test]
    fn horizontal_vertical_and_reverse_fixtures() {
        // (main 정렬, 방향, 기대 main 위치(padding 10 포함)).
        let main_cases = [
            (MainAlignment::Start, StackDirection::Forward, [10, 48, 96]),
            (
                MainAlignment::Center,
                StackDirection::Forward,
                [34, 72, 120],
            ),
            (MainAlignment::End, StackDirection::Forward, [58, 96, 144]),
            (
                MainAlignment::SpaceBetween,
                StackDirection::Forward,
                [10, 72, 144],
            ),
            (
                MainAlignment::SpaceAround,
                StackDirection::Forward,
                [18, 72, 136],
            ),
            (
                MainAlignment::SpaceEvenly,
                StackDirection::Forward,
                [22, 72, 132],
            ),
            // Reverse: 정방향 위치 p를 200 − p − 크기로 뒤집음.
            (
                MainAlignment::Start,
                StackDirection::Reverse,
                [180, 132, 58],
            ),
            (MainAlignment::End, StackDirection::Reverse, [132, 84, 10]),
            (
                MainAlignment::SpaceBetween,
                StackDirection::Reverse,
                [180, 108, 10],
            ),
        ];
        // (cross 정렬, 기대 cross 위치(padding 5 포함)): cross 50, 크기 20/10/30.
        let cross_cases = [
            (CrossAlignment::Start, [5, 5, 5]),
            (CrossAlignment::Center, [20, 25, 15]),
            (CrossAlignment::End, [35, 45, 25]),
        ];
        for axis in Axis::BOTH {
            for (main_alignment, direction, expected_main) in main_cases {
                for (cross_alignment, expected_cross) in cross_cases {
                    let stack = StackLayout {
                        direction,
                        gap: len(8.0),
                        main_alignment,
                        cross_alignment,
                        ..StackLayout::new(axis)
                    };
                    let result = fixed_children_stack(stack);
                    let expected: Vec<(i32, i32)> = expected_main
                        .iter()
                        .zip(expected_cross)
                        .map(|(m, c)| (*m, c))
                        .collect();
                    assert_eq!(
                        positions(&result, axis),
                        expected,
                        "{axis} {main_alignment:?} {direction:?} {cross_alignment:?}"
                    );
                    // 크기는 고정 그대로.
                    let size = result.get(id(4)).unwrap().local_rect().size();
                    assert_eq!(size.get(axis), px(66));
                }
            }
        }
    }

    #[test]
    fn residual_subpixels_follow_visual_order_and_overflow_falls_back() {
        // 남는 공간 1 px(64 subpixel)을 SpaceBetween 2칸에 → 32/32, 3칸(Evenly 4칸) → 16씩.
        // 남는 공간 65 subpixel / SpaceEvenly 4칸 → 17, 16, 16, 16(첫 칸이 잔여).
        let root = LayoutNode::new(id(1))
            .with_sizing(
                SizingMode::Fixed(Length::new(30.0 + 65.0 / 64.0).unwrap()),
                SizingMode::Fixed(len(10.0)),
            )
            .with_container(ContainerLayout::Stack(StackLayout {
                main_alignment: MainAlignment::SpaceEvenly,
                ..StackLayout::new(Axis::Horizontal)
            }))
            .with_child(id(2), key(1))
            .with_child(id(3), key(2))
            .with_child(id(4), key(3));
        let mut nodes = vec![root];
        for n in 2..=4u64 {
            nodes.push(LayoutNode::new(id(n)).with_fixed_size(len(10.0), len(10.0)));
        }
        let result = compute_layout(&LayoutInput::new(id(1), nodes.clone())).unwrap();
        let xs: Vec<i32> = [id(2), id(3), id(4)]
            .iter()
            .map(|n| result.get(*n).unwrap().local_rect().x().raw())
            .collect();
        assert_eq!(xs, vec![17, 17 + 640 + 16, 17 + 640 + 16 + 640 + 16]);

        // 넘침(남는 공간 음수): SpaceBetween → Start, SpaceEvenly → Center(양쪽 넘침).
        for (alignment, first_x) in [
            (MainAlignment::SpaceBetween, 0),
            (MainAlignment::SpaceEvenly, -5 * 64),
            (MainAlignment::SpaceAround, -5 * 64),
        ] {
            let mut overflow = nodes.clone();
            overflow[0] = overflow[0]
                .clone()
                .with_fixed_size(len(20.0), len(10.0))
                .with_container(ContainerLayout::Stack(StackLayout {
                    main_alignment: alignment,
                    ..StackLayout::new(Axis::Horizontal)
                }));
            let result = compute_layout(&LayoutInput::new(id(1), overflow)).unwrap();
            assert_eq!(
                result.get(id(2)).unwrap().local_rect().x().raw(),
                first_x,
                "{alignment:?}"
            );
            assert_eq!(
                result.get(id(1)).unwrap().overflow_bounds().width(),
                px(30),
                "{alignment:?}"
            );
        }
    }

    #[test]
    fn hug_stack_stretch_and_cross_fill() {
        // Hug 세로 Stack(gap 4, padding 2): 크기 = (max 너비 30, 20 + 4 + 5) + padding.
        let root = LayoutNode::new(id(1))
            .with_sizing(SizingMode::HugContent, SizingMode::HugContent)
            .with_padding(Insets::new(2.0, 2.0, 2.0, 2.0).unwrap())
            .with_container(ContainerLayout::Stack(StackLayout {
                gap: len(4.0),
                cross_alignment: CrossAlignment::Stretch,
                ..StackLayout::new(Axis::Vertical)
            }))
            .with_child(id(2), key(1))
            .with_child(id(3), key(2))
            .with_child(id(4), key(3))
            .with_child(id(5), key(4));
        let fixed_narrow = LayoutNode::new(id(2)).with_fixed_size(len(10.0), len(20.0));
        let fixed_wide = LayoutNode::new(id(3)).with_fixed_size(len(30.0), len(5.0));
        // Hug cross 자식(intrinsic 12×6)은 Stretch로 cross 30까지 늘어남.
        let hug = LayoutNode::new(id(4))
            .with_sizing(SizingMode::HugContent, SizingMode::HugContent)
            .with_content(LayoutContent::Intrinsic(IntrinsicContent {
                width: len(12.0),
                height: len(6.0),
                baseline: None,
            }));
        // cross Fill 자식은 정렬과 무관하게 cross를 채움, main은 고정 3.
        let fill =
            LayoutNode::new(id(5)).with_sizing(SizingMode::Fill, SizingMode::Fixed(len(3.0)));
        let result = compute_layout(&LayoutInput::new(
            id(1),
            vec![root, fixed_narrow, fixed_wide, hug, fill],
        ))
        .unwrap();
        let rect = |n: u64| result.get(id(n)).unwrap().local_rect();
        // main 합계: 20 + 5 + 6 + 3 + gap 4×3 = 46.
        assert_eq!(rect(1).size().width(), px(34));
        assert_eq!(rect(1).size().height(), px(50));
        assert_eq!(
            (rect(2).x(), rect(2).y(), rect(2).width()),
            (px(2), px(2), px(10))
        );
        assert_eq!((rect(3).y(), rect(3).width()), (px(26), px(30)));
        assert_eq!(
            (rect(4).y(), rect(4).width(), rect(4).height()),
            (px(35), px(30), px(6))
        );
        assert_eq!(
            (rect(5).y(), rect(5).width(), rect(5).height()),
            (px(45), px(30), px(3))
        );
    }

    use crate::flex::{FlexBasis, FlexItem};
    use stellar_core_types::value::{Percentage, Scalar};

    fn scalar(value: f32) -> Scalar {
        Scalar::new(value).unwrap()
    }

    fn flex(grow: f32, shrink: f32, basis: FlexBasis) -> FlexItem {
        FlexItem::new(scalar(grow), scalar(shrink), basis)
    }

    /// 가로 Stack(너비 `width`, 높이 10) 안의 (StableId, OrderKey 순번, 자식) 목록을 계산합니다.
    fn row(
        width: SizingMode,
        stack: StackLayout,
        children: Vec<(u64, u32, LayoutNode)>,
    ) -> LayoutResult {
        let mut root = LayoutNode::new(id(1))
            .with_sizing(width, SizingMode::Fixed(len(10.0)))
            .with_container(ContainerLayout::Stack(stack));
        let mut nodes = Vec::new();
        for (n, order, child) in children {
            root = root.with_child(id(n), key(order));
            nodes.push(child);
        }
        nodes.insert(0, root);
        compute_layout(&LayoutInput::new(id(1), nodes)).unwrap()
    }

    fn child(n: u64, width: SizingMode, item: FlexItem) -> LayoutNode {
        LayoutNode::new(id(n))
            .with_sizing(width, SizingMode::Fixed(len(10.0)))
            .with_flex(item)
    }

    /// (x px, width px) 목록.
    fn spans(result: &LayoutResult, ids: &[u64]) -> Vec<(f64, f64)> {
        ids.iter()
            .map(|n| {
                let r = result.get(id(*n)).unwrap().local_rect();
                (r.x().to_px_f64(), r.width().to_px_f64())
            })
            .collect()
    }

    #[test]
    fn reference_flex_grow_shrink_basis_fixtures() {
        let fixed = |w: f32| SizingMode::Fixed(len(w));
        let horizontal = StackLayout::new(Axis::Horizontal);
        // 1) basis 50/100/50, grow 1/2/1, 300 → 75/150/75.
        let basis = |v: f32| FlexBasis::Length(len(v));
        let grow = row(
            fixed(300.0),
            horizontal,
            vec![
                (2, 1, child(2, fixed(0.0), flex(1.0, 0.0, basis(50.0)))),
                (3, 2, child(3, fixed(0.0), flex(2.0, 0.0, basis(100.0)))),
                (4, 3, child(4, fixed(0.0), flex(1.0, 0.0, basis(50.0)))),
            ],
        );
        assert_eq!(
            spans(&grow, &[2, 3, 4]),
            vec![(0.0, 75.0), (75.0, 150.0), (225.0, 75.0)]
        );
        // 2) 같은 입력 + gap 10: 사용 가능 280, free 80 → 70/140/70.
        let gapped = row(
            fixed(300.0),
            StackLayout {
                gap: len(10.0),
                ..horizontal
            },
            vec![
                (2, 1, child(2, fixed(0.0), flex(1.0, 0.0, basis(50.0)))),
                (3, 2, child(3, fixed(0.0), flex(2.0, 0.0, basis(100.0)))),
                (4, 3, child(4, fixed(0.0), flex(1.0, 0.0, basis(50.0)))),
            ],
        );
        assert_eq!(
            spans(&gapped, &[2, 3, 4]),
            vec![(0.0, 70.0), (80.0, 140.0), (230.0, 70.0)]
        );
        // 3) shrink: Fixed 100×3(basis Auto), shrink 1/1/2, 200 → 75/75/50.
        let auto = FlexBasis::Auto;
        let shrink = row(
            fixed(200.0),
            horizontal,
            vec![
                (2, 1, child(2, fixed(100.0), flex(0.0, 1.0, auto))),
                (3, 2, child(3, fixed(100.0), flex(0.0, 1.0, auto))),
                (4, 3, child(4, fixed(100.0), flex(0.0, 2.0, auto))),
            ],
        );
        assert_eq!(
            spans(&shrink, &[2, 3, 4]),
            vec![(0.0, 75.0), (75.0, 75.0), (150.0, 50.0)]
        );
        // 4) Fill + Fixed 100 + Fill in 400 → 150/100/150.
        let fill = row(
            fixed(400.0),
            horizontal,
            vec![
                (2, 1, child(2, SizingMode::Fill, FlexItem::default())),
                (3, 2, child(3, fixed(100.0), FlexItem::default())),
                (4, 3, child(4, SizingMode::Fill, FlexItem::default())),
            ],
        );
        assert_eq!(
            spans(&fill, &[2, 3, 4]),
            vec![(0.0, 150.0), (150.0, 100.0), (250.0, 150.0)]
        );
        // 5) percentage basis 25% of 400 = 100, grow 0.5 vs 1.5(basis 0) in 나머지 300 → 75/225.
        let pct = FlexBasis::Percentage(Percentage::new(25.0).unwrap());
        let fractional = row(
            fixed(400.0),
            horizontal,
            vec![
                (2, 1, child(2, fixed(0.0), flex(0.0, 0.0, pct))),
                (3, 2, child(3, fixed(0.0), flex(0.5, 0.0, basis(0.0)))),
                (4, 3, child(4, fixed(0.0), flex(1.5, 0.0, basis(0.0)))),
            ],
        );
        assert_eq!(
            spans(&fractional, &[2, 3, 4]),
            vec![(0.0, 100.0), (100.0, 75.0), (175.0, 225.0)]
        );
        // 6) Hug container: 분배하지 않고 base 합(30 + 40) = 70.
        let hug = row(
            SizingMode::HugContent,
            horizontal,
            vec![
                (2, 1, child(2, fixed(0.0), flex(1.0, 1.0, basis(30.0)))),
                (3, 2, child(3, fixed(0.0), flex(3.0, 1.0, basis(40.0)))),
            ],
        );
        assert_eq!(spans(&hug, &[2, 3]), vec![(0.0, 30.0), (30.0, 40.0)]);
        assert_eq!(
            hug.get(id(1)).unwrap().local_rect().width().to_px_f64(),
            70.0
        );
    }

    #[test]
    fn residual_follows_visual_order_not_stable_id() {
        // visual order: id 9(key 1), id 5(key 2), id 7(key 3). StableId 순서(5, 7, 9)와 다릅니다.
        // 100 px / Fill 3개 = 2133.33 subpixel → 잔여 1은 visual order 첫 자식(id 9).
        for direction in [StackDirection::Forward, StackDirection::Reverse] {
            let stack = StackLayout {
                direction,
                ..StackLayout::new(Axis::Horizontal)
            };
            let result = row(
                SizingMode::Fixed(len(100.0)),
                stack,
                vec![
                    (5, 2, child(5, SizingMode::Fill, FlexItem::default())),
                    (7, 3, child(7, SizingMode::Fill, FlexItem::default())),
                    (9, 1, child(9, SizingMode::Fill, FlexItem::default())),
                ],
            );
            let raw = |n: u64| {
                let r = result.get(id(n)).unwrap().local_rect();
                (r.x().raw(), r.width().raw())
            };
            match direction {
                StackDirection::Forward => {
                    assert_eq!(
                        [raw(9), raw(5), raw(7)],
                        [(0, 2134), (2134, 2133), (4267, 2133)]
                    );
                }
                StackDirection::Reverse => {
                    assert_eq!(
                        [raw(9), raw(5), raw(7)],
                        [(4266, 2134), (2133, 2133), (0, 2133)]
                    );
                }
            }
        }
    }

    #[test]
    fn flex_validation_and_round_trip() {
        let stack = ContainerLayout::Stack(StackLayout::new(Axis::Horizontal));
        let parent = || {
            LayoutNode::new(id(1))
                .with_fixed_size(len(100.0), len(10.0))
                .with_container(stack.clone())
                .with_child(id(2), key(1))
        };
        // main 축 Fill + 기본값이 아닌 flex → InvalidSizing.
        let conflict = child(2, SizingMode::Fill, flex(1.0, 0.0, FlexBasis::Auto));
        assert!(matches!(
            LayoutInput::new(id(1), vec![parent(), conflict]).validate(),
            Err(LayoutError::InvalidSizing { node, axis: Axis::Horizontal, .. }) if node == id(2)
        ));
        // cross 축 Fill + flex는 허용.
        let cross_fill = LayoutNode::new(id(2))
            .with_sizing(SizingMode::Fixed(len(5.0)), SizingMode::Fill)
            .with_flex(flex(1.0, 1.0, FlexBasis::Auto));
        let input = LayoutInput::new(id(1), vec![parent(), cross_fill]);
        assert!(input.validate().is_ok());
        let result = compute_layout(&input).unwrap();
        assert_eq!(result.get(id(2)).unwrap().local_rect().width(), px(100));
        assert_eq!(result.get(id(2)).unwrap().local_rect().height(), px(10));
        // 음수 factor·basis 거부.
        let negative = child(
            2,
            SizingMode::Fixed(len(1.0)),
            flex(-1.0, 0.0, FlexBasis::Auto),
        );
        assert_eq!(
            LayoutInput::new(id(1), vec![parent(), negative]).validate(),
            Err(LayoutError::NegativeValue {
                node: id(2),
                property: LayoutProperty::Flex
            })
        );
        // round-trip(모든 basis 형태).
        for basis in [
            FlexBasis::Auto,
            FlexBasis::Length(len(12.5)),
            FlexBasis::Percentage(Percentage::new(30.0).unwrap()),
        ] {
            let node = child(2, SizingMode::Fixed(len(1.0)), flex(0.25, 3.0, basis));
            let input = LayoutInput::new(id(1), vec![parent(), node]);
            assert_eq!(LayoutInput::decode(&input.encode()).unwrap(), input);
        }
    }

    /// 가로 wrap Stack: (너비, 높이 sizing, 설정, 자식 (너비 sizing, 높이 sizing, flex)).
    fn wrap_row(
        width: SizingMode,
        height: SizingMode,
        stack: StackLayout,
        children: &[(SizingMode, SizingMode, FlexItem)],
    ) -> LayoutResult {
        let mut root = LayoutNode::new(id(1))
            .with_sizing(width, height)
            .with_container(ContainerLayout::Stack(stack));
        let mut nodes = Vec::new();
        for (i, (w, h, item)) in children.iter().enumerate() {
            let n = i as u64 + 2;
            root = root.with_child(id(n), key(i as u32 + 1));
            nodes.push(LayoutNode::new(id(n)).with_sizing(*w, *h).with_flex(*item));
        }
        nodes.insert(0, root);
        compute_layout(&LayoutInput::new(id(1), nodes)).unwrap()
    }

    fn fx(value: f32) -> SizingMode {
        SizingMode::Fixed(len(value))
    }

    /// 자식 (x, y, w, h) px 목록.
    fn boxes_px(result: &LayoutResult, count: usize) -> Vec<(f64, f64, f64, f64)> {
        (0..count)
            .map(|i| {
                let r = result.get(id(i as u64 + 2)).unwrap().local_rect();
                (
                    r.x().to_px_f64(),
                    r.y().to_px_f64(),
                    r.width().to_px_f64(),
                    r.height().to_px_f64(),
                )
            })
            .collect()
    }

    #[test]
    fn line_break_fixture() {
        // 너비 100, gap 10, cross gap 5. 줄: [40, 40] | [40, 30] | [100](정확히 맞음) | [120](단독 넘침).
        let stack = StackLayout {
            gap: len(10.0),
            cross_gap: len(5.0),
            wrap: StackWrap::Wrap,
            ..StackLayout::new(Axis::Horizontal)
        };
        let none = FlexItem::default();
        let result = wrap_row(
            fx(100.0),
            SizingMode::HugContent,
            stack,
            &[
                (fx(40.0), fx(10.0), none),
                (fx(40.0), fx(20.0), none),
                (fx(40.0), fx(15.0), none),
                (fx(30.0), fx(10.0), none),
                (fx(100.0), fx(5.0), none),
                (fx(120.0), fx(8.0), none),
            ],
        );
        assert_eq!(
            boxes_px(&result, 6),
            vec![
                (0.0, 0.0, 40.0, 10.0),
                (50.0, 0.0, 40.0, 20.0),
                (0.0, 25.0, 40.0, 15.0),
                (50.0, 25.0, 30.0, 10.0),
                (0.0, 45.0, 100.0, 5.0),
                (0.0, 55.0, 120.0, 8.0),
            ]
        );
        // Hug 높이 = 줄 cross 20 + 15 + 5 + 8 + cross gap 5×3.
        assert_eq!(
            result.get(id(1)).unwrap().local_rect().height().to_px_f64(),
            63.0
        );
        // main Hug(indefinite)면 wrap하지 않고 한 줄.
        let single = wrap_row(
            SizingMode::HugContent,
            SizingMode::HugContent,
            stack,
            &[(fx(40.0), fx(10.0), none), (fx(80.0), fx(20.0), none)],
        );
        assert_eq!(
            boxes_px(&single, 2),
            vec![(0.0, 0.0, 40.0, 10.0), (50.0, 0.0, 80.0, 20.0)]
        );
    }

    #[test]
    fn align_content_fixture() {
        // 높이 90 고정, 줄 [a 60×20, c 30×Fill] | [b 60×30], cross gap 10 → 사용 60, 남음 30.
        let none = FlexItem::default();
        let children = [
            (fx(60.0), fx(20.0), none),
            (fx(30.0), SizingMode::Fill, none),
            (fx(60.0), fx(30.0), none),
        ];
        // (정렬, 줄 1 y, 줄 2 y, 줄 1 cross(c 높이)).
        let cases = [
            (LineAlignment::Start, 0.0, 30.0, 20.0),
            (LineAlignment::Center, 15.0, 45.0, 20.0),
            (LineAlignment::End, 30.0, 60.0, 20.0),
            (LineAlignment::SpaceBetween, 0.0, 60.0, 20.0),
            (LineAlignment::SpaceAround, 7.5, 52.5, 20.0),
            (LineAlignment::SpaceEvenly, 10.0, 50.0, 20.0),
            // Stretch: 남은 30을 줄에 15씩 → 줄 cross 35, 45.
            (LineAlignment::Stretch, 0.0, 45.0, 35.0),
        ];
        for (line_alignment, first, second, fill_height) in cases {
            let stack = StackLayout {
                cross_gap: len(10.0),
                wrap: StackWrap::Wrap,
                line_alignment,
                ..StackLayout::new(Axis::Horizontal)
            };
            let result = wrap_row(fx(100.0), fx(90.0), stack, &children);
            let boxes = boxes_px(&result, 3);
            assert_eq!(boxes[0].1, first, "{line_alignment:?} 줄 1");
            assert_eq!(
                boxes[1],
                (60.0, first, 30.0, fill_height),
                "{line_alignment:?} Fill"
            );
            assert_eq!(boxes[2].1, second, "{line_alignment:?} 줄 2");
        }
        // 줄 안 cross 정렬: Center면 줄 1(cross 20) 안의 높이 10 자식은 +5.
        let stack = StackLayout {
            wrap: StackWrap::Wrap,
            cross_alignment: CrossAlignment::Center,
            ..StackLayout::new(Axis::Horizontal)
        };
        let result = wrap_row(
            fx(100.0),
            SizingMode::HugContent,
            stack,
            &[
                (fx(60.0), fx(20.0), none),
                (fx(30.0), fx(10.0), none),
                (fx(60.0), fx(4.0), none),
            ],
        );
        assert_eq!(
            boxes_px(&result, 3),
            vec![
                (0.0, 0.0, 60.0, 20.0),
                (60.0, 5.0, 30.0, 10.0),
                (0.0, 20.0, 60.0, 4.0)
            ]
        );
    }

    #[test]
    fn wrapped_lines_flex_independently_and_reverse() {
        // grow 1인 40×3을 너비 100에: 줄 [40, 40] free 20 → 50/50, 줄 [40] → 100.
        let grow = FlexItem::new(
            Scalar::new(1.0).unwrap(),
            Scalar::new(0.0).unwrap(),
            FlexBasis::Auto,
        );
        let children = [(fx(40.0), fx(10.0), grow); 3];
        let stack = StackLayout {
            wrap: StackWrap::Wrap,
            ..StackLayout::new(Axis::Horizontal)
        };
        let result = wrap_row(fx(100.0), SizingMode::HugContent, stack, &children);
        assert_eq!(
            boxes_px(&result, 3),
            vec![
                (0.0, 0.0, 50.0, 10.0),
                (50.0, 0.0, 50.0, 10.0),
                (0.0, 10.0, 100.0, 10.0)
            ]
        );
        // Reverse: 줄마다 main 축 거울상(줄 순서는 유지).
        let reverse = StackLayout {
            direction: StackDirection::Reverse,
            ..stack
        };
        let none = FlexItem::default();
        let result = wrap_row(
            fx(100.0),
            SizingMode::HugContent,
            reverse,
            &[
                (fx(40.0), fx(10.0), none),
                (fx(40.0), fx(10.0), none),
                (fx(40.0), fx(10.0), none),
            ],
        );
        assert_eq!(
            boxes_px(&result, 3),
            vec![
                (60.0, 0.0, 40.0, 10.0),
                (20.0, 0.0, 40.0, 10.0),
                (60.0, 10.0, 40.0, 10.0)
            ]
        );
    }

    #[test]
    fn wrap_fields_bytes_golden_and_validation() {
        let stack = StackLayout {
            wrap: StackWrap::Wrap,
            cross_gap: len(5.0),
            line_alignment: LineAlignment::Stretch,
            ..StackLayout::new(Axis::Horizontal)
        };
        let mut w = Writer::new();
        encode_stack_wrap(&mut w, &stack);
        assert_eq!(w.finish(), vec![1, 0x00, 0x00, 0xa0, 0x40, 6]);
        let tags: Vec<u8> = [
            LineAlignment::Start,
            LineAlignment::Center,
            LineAlignment::End,
            LineAlignment::SpaceBetween,
            LineAlignment::SpaceAround,
            LineAlignment::SpaceEvenly,
            LineAlignment::Stretch,
        ]
        .iter()
        .map(|a| line_alignment_tag(*a))
        .collect();
        assert_eq!(tags, vec![0, 1, 2, 3, 4, 5, 6]);
        assert!(line_alignment_from_tag(7).is_err());
        let input = LayoutInput::new(
            id(1),
            vec![LayoutNode::new(id(1)).with_container(ContainerLayout::Stack(stack))],
        );
        assert_eq!(LayoutInput::decode(&input.encode()).unwrap(), input);
        let negative = LayoutInput::new(
            id(1),
            vec![
                LayoutNode::new(id(1)).with_container(ContainerLayout::Stack(StackLayout {
                    cross_gap: len(-2.0),
                    ..stack
                })),
            ],
        );
        assert!(matches!(
            negative.validate(),
            Err(LayoutError::NegativeValue { .. })
        ));
    }
}
