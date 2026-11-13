//! Stack(Flex 계열) 배치 (X.3.1).
//!
//! [`StackLayout`]은 자식을 한 축(main axis)을 따라 차례로 놓는 container 설정입니다.
//!
//! - `axis`: main axis(가로 = 왼쪽→오른쪽, 세로 = 위→아래).
//! - `direction`: `Forward`는 visual order대로 main 시작부터, `Reverse`는 main 끝부터 놓습니다.
//! - `gap`: 이웃한 자식 사이 main 방향 간격(비음수).
//! - `main_alignment`·`cross_alignment`: 남는 공간 분배와 cross 방향 정렬.
//! - padding은 모든 배치 방식이 공유하는 node padding([`crate::input::LayoutNode::padding`])을
//!   사용합니다. Stack 자식은 content box 안에 놓입니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{FiniteF32, Length};

use crate::codec::{Reader, Writer};
use crate::error::LayoutError;
use crate::geometry::Axis;
use crate::input::length_unit;
use crate::property::LayoutProperty;

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
}
