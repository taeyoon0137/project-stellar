//! Stack(Flex 계열) 배치 (X.3.1, X.3.2).
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
//! main 축 `Fill`(늘림)과 grow·shrink는 X.3.3에서 구현하며, 그 전까지 main 축 `Fill`은 콘텐츠
//! 크기로 해석합니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{FiniteF32, Length};

use crate::codec::{Reader, Writer};
use crate::distribute::distribute_equal;
use crate::engine::{at, AxisSizing, ChildConstraint, Engine};
use crate::error::LayoutError;
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
        axis,
        direction,
        gap,
        main_alignment,
        cross_alignment,
    })
}

/// Stack 설정 값을 검증합니다(gap 비음수·범위).
pub(crate) fn validate_stack(node: StableId, stack: &StackLayout) -> Result<(), LayoutError> {
    length_unit(node, LayoutProperty::ContainerLayout, stack.gap, false)?;
    Ok(())
}

// --- 배치 알고리즘 (X.3.2) ---

/// Stack 계산에 쓰는 변환된 설정.
#[derive(Debug, Clone, Copy)]
struct StackUnits {
    main: Axis,
    cross: Axis,
    gap: LayoutUnit,
    config: StackLayout,
}

fn stack_units(id: StableId, stack: &StackLayout) -> Result<StackUnits, LayoutError> {
    Ok(StackUnits {
        main: stack.axis,
        cross: stack.axis.cross(),
        gap: length_unit(id, LayoutProperty::ContainerLayout, stack.gap, false)?,
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

/// 자식 하나의 제약. main 축 Fill은 아직 늘리지 않고(X.3.3) 콘텐츠 크기로 해석합니다. cross 축
/// Fill은 cross 크기를 채우고, `Stretch` 정렬은 cross가 HugContent인 자식을 cross 크기로 늘립니다.
fn child_constraint(
    engine: &Engine<'_>,
    child: usize,
    units: &StackUnits,
    inner: [Option<LayoutUnit>; 2],
) -> ChildConstraint {
    let main = units.main.index();
    let cross = units.cross.index();
    let mut fill = [None, None];
    fill[cross] = inner[cross];
    let mut forced = [None, None];
    if units.config.cross_alignment == CrossAlignment::Stretch
        && engine.nodes[child].sizing[cross] == AxisSizing::Hug
    {
        forced[cross] = inner[cross];
    }
    fill[main] = None;
    ChildConstraint {
        cb: inner,
        fill,
        forced,
    }
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

/// Stack container의 content extent(Hug 측정).
pub(crate) fn measure(
    engine: &mut Engine<'_>,
    index: usize,
    inner: [Option<LayoutUnit>; 2],
) -> Result<[LayoutUnit; 2], LayoutError> {
    let id = engine.nodes[index].id;
    let units = stack_of(engine, index)?;
    let mut sizes = Vec::new();
    for child in engine.children(index) {
        let constraint = child_constraint(engine, child, &units, inner);
        sizes.push(engine.resolve_size(child, constraint)?);
    }
    let mut extent = [LayoutUnit::ZERO; 2];
    extent[units.main.index()] = main_total(id, &sizes, &units)?;
    extent[units.cross.index()] = sizes
        .iter()
        .map(|size| size.get(units.cross))
        .max()
        .unwrap_or(LayoutUnit::ZERO);
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
            (
                slots[0],
                zeros,
                slots[1..count]
                    .iter()
                    .copied()
                    .chain([LayoutUnit::ZERO])
                    .collect(),
            )
        }
    })
}

/// cross 정렬에 따른 cross 위치(content box 기준).
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
    let mut sizes = Vec::with_capacity(children.len());
    for &child in &children {
        let constraint = child_constraint(engine, child, &units, inner_axes);
        sizes.push(engine.resolve_size(child, constraint)?);
    }
    let main_extent = inner.get(units.main);
    let cross_extent = inner.get(units.cross);
    let free = main_extent
        .checked_sub(main_total(id, &sizes, &units)?)
        .map_err(at(id))?;
    let (offset, before, after) =
        main_spacing(units.config.main_alignment, free, children.len()).map_err(at(id))?;

    let mut placements = Vec::with_capacity(children.len());
    let mut cursor = offset;
    for (i, (&child, size)) in children.iter().zip(&sizes).enumerate() {
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
            cross_extent,
            size.get(units.cross),
        )
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
}
