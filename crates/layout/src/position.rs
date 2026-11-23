//! CSS positioning: `position: absolute`·`relative` (CSS Positioned Layout 3, ADR-0076).
//!
//! # absolute
//!
//! - containing block은 부모 box의 **padding box**입니다. border가 없으므로 padding box는 부모 border box
//!   `(0, 0, 너비, 높이)`와 같습니다. 백분율 inset·크기의 기준도 padding box 크기입니다. Web export는 모든 요소를
//!   positioned로 내보내므로(속성 모델 3장 stacking 규칙) absolute 요소의 containing block은 항상 그 부모입니다.
//! - absolute 자식은 부모 flow와 부모의 콘텐츠 기반 크기(`auto`·`fit-content` 등)에 참여하지 않습니다.
//! - 축 크기: 크기 속성이 `auto`이고 시작·끝 inset이 모두 `auto`가 아니면 `containing block - 시작 - 끝`
//!   (stretch)입니다. 그 밖의 `auto`는 `fit-content`이며 가용 공간은 `containing block - 시작 - 끝`
//!   (`auto` inset은 0, 둘 다 `auto`면 시작 쪽은 static position)입니다.
//! - 축 위치: 시작 inset이 있으면 그 값, 없고 끝 inset이 있으면 `containing block - 끝 - 크기`, 둘 다 없으면
//!   static position입니다. 시작·끝·크기가 모두 정해진 over-constrained 축은 끝 inset을 무시합니다(ltr·
//!   horizontal-tb).
//! - static position은 부모 배치 방식이 정합니다: block은 그 자리의 흐름 위치(content box 시작 + 앞 in-flow
//!   형제 높이), flex는 유일한 flex item으로 놓았을 때의 위치(`justify-content`·`align-self`).
//!
//! # relative
//!
//! 배치가 끝난 뒤 inset만큼 시각 위치를 옮깁니다. 가로는 `left`, 없으면 `-right`이고 세로는 `top`, 없으면
//! `-bottom`입니다. 백분율 기준은 부모 content box이며 indefinite 축의 백분율은 0입니다. 형제 배치·부모 크기·
//! baseline 계산에는 영향을 주지 않고 overflow bounds에는 옮긴 위치가 들어갑니다.

use crate::engine::{at, clamp, sub_floor_zero, Avail, Engine, InsetP, Placement, Request, SizeP};
use crate::error::LayoutError;
use crate::geometry::Axis;
use crate::rounding::LayoutUnit;
use crate::style::Position;

/// inset 하나를 기준 길이로 해석합니다(`auto`·indefinite 백분율은 `None`).
fn inset(
    engine: &Engine<'_>,
    index: usize,
    value: InsetP,
    base: Option<LayoutUnit>,
) -> Result<Option<LayoutUnit>, LayoutError> {
    match value {
        InsetP::Auto => Ok(None),
        InsetP::Length(length) => length.resolve(base, engine.nodes[index].id),
    }
}

/// relative 자식의 시각 이동량 `[dx, dy]`(`position: relative`가 아니면 0).
pub(crate) fn relative_offset(
    engine: &Engine<'_>,
    index: usize,
    cb: [Option<LayoutUnit>; 2],
) -> Result<[LayoutUnit; 2], LayoutError> {
    let prepared = &engine.nodes[index];
    if prepared.position != Position::Relative {
        return Ok([LayoutUnit::ZERO; 2]);
    }
    let mut offset = [LayoutUnit::ZERO; 2];
    for axis in Axis::BOTH {
        let i = axis.index();
        let [start, end] = prepared.inset[i];
        offset[i] = match inset(engine, index, start, cb[i])? {
            Some(value) => value,
            None => match inset(engine, index, end, cb[i])? {
                Some(value) => LayoutUnit::ZERO
                    .checked_sub(value)
                    .map_err(at(prepared.id))?,
                None => LayoutUnit::ZERO,
            },
        };
    }
    Ok(offset)
}

/// static position 정렬.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StaticAlign {
    /// 시작 쪽.
    Start,
    /// 가운데(ties away from zero).
    Center,
    /// 끝 쪽.
    End,
}

/// static position rectangle(CSS Positioned Layout 3 §4.1): 시작·끝 inset이 모두 `auto`인 축에서 absolute 자식이
/// 놓이는 영역과 그 안의 정렬(부모 local 좌표).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StaticRect {
    /// 축별 시작 좌표.
    pub(crate) start: [LayoutUnit; 2],
    /// 축별 크기.
    pub(crate) size: [LayoutUnit; 2],
    /// 축별 정렬.
    pub(crate) align: [StaticAlign; 2],
}

impl StaticRect {
    /// 크기 `size`인 자식의 축 위치.
    fn position(
        &self,
        axis: Axis,
        size: LayoutUnit,
    ) -> Result<LayoutUnit, crate::rounding::LayoutUnitError> {
        let i = axis.index();
        let free = self.size[i].checked_sub(size)?;
        let offset = match self.align[i] {
            StaticAlign::Start => LayoutUnit::ZERO,
            StaticAlign::Center => free.checked_div_round(2)?,
            StaticAlign::End => free,
        };
        self.start[i].checked_add(offset)
    }
}

/// absolute 자식 `index`를 부모 padding box `cb` 안에 배치합니다.
pub(crate) fn place_absolute(
    engine: &mut Engine<'_>,
    index: usize,
    cb: [LayoutUnit; 2],
    static_rect: StaticRect,
) -> Result<Placement, LayoutError> {
    let nodes = engine.nodes;
    let prepared = &nodes[index];
    let id = prepared.id;
    let mut starts = [None; 2];
    let mut ends = [None; 2];
    for axis in Axis::BOTH {
        let i = axis.index();
        starts[i] = inset(engine, index, prepared.inset[i][0], Some(cb[i]))?;
        ends[i] = inset(engine, index, prepared.inset[i][1], Some(cb[i]))?;
    }
    let parent = [Some(cb[0]), Some(cb[1])];
    let mut request = Request {
        known: [None, None],
        parent,
        available: [Avail::MaxContent; 2],
        content_only: [false, false],
    };
    for axis in Axis::BOTH {
        let i = axis.index();
        let space = match (starts[i], ends[i]) {
            (None, None) => static_rect.size[i].max(LayoutUnit::ZERO),
            (start, end) => sub_floor_zero(
                cb[i],
                start
                    .unwrap_or(LayoutUnit::ZERO)
                    .checked_add(end.unwrap_or(LayoutUnit::ZERO))
                    .map_err(at(id))?,
            )
            .map_err(at(id))?,
        };
        request.available[i] = Avail::Definite(space);
        let stretch = prepared.size[i] == SizeP::Auto
            && starts[i].is_some()
            && ends[i].is_some()
            && !(prepared.ratio.is_some() && other_axis_definite(&prepared.size, axis));
        if stretch {
            let min = engine.min_size(index, axis, request)?;
            let max = engine.max_size(index, axis, request)?;
            request.known[i] = Some(clamp(space, min, max));
        }
    }
    let layout = engine.layout(index, request)?;
    let mut origin = [LayoutUnit::ZERO; 2];
    for axis in Axis::BOTH {
        let i = axis.index();
        origin[i] = match (starts[i], ends[i]) {
            (Some(start), _) => start,
            (None, Some(end)) => cb[i]
                .checked_sub(end)
                .and_then(|v| v.checked_sub(layout.size[i]))
                .map_err(at(id))?,
            (None, None) => static_rect.position(axis, layout.size[i]).map_err(at(id))?,
        };
    }
    Ok(Placement {
        child: id,
        x: origin[0],
        y: origin[1],
        request,
    })
}

/// 다른 축 크기 속성이 `<length-percentage>`인지 여부(비율이 `auto` 축을 정하는 경우).
fn other_axis_definite(size: &[SizeP; 2], axis: Axis) -> bool {
    matches!(size[axis.cross().index()], SizeP::Length(_))
}
