//! 정렬 6종·분배 2종·행열 정리 (XII.10.21, ADR-0105 결정 6).
//!
//! 순수 계산([`align_deltas`], [`distribute_deltas`], [`grid_deltas`])은 같은 좌표계의 상자 목록을 받아 상자마다
//! 이동량을 돌려줍니다. 호출자는 회전한 node도 world 경계 상자로 넘기고(완료 기준 "회전 node는 world bounds 기준"),
//! 이동량을 node마다 부모 공간으로 바꿔 [`super::place::translate`]로 씁니다.
//!
//! 흐름 컨테이너(CSS `display: flex | grid`)의 자식은 위치를 쓰지 않습니다([`flow_align`]).
//! - flex 자식: 주축 정렬은 비활성(컨테이너 `justify-content` 안내), 교차축 정렬은 자식 `align-self`.
//! - grid 자식: 가로 정렬은 `justify-self`, 세로 정렬은 `align-self`.
//! - 흐름 자식의 분배·행열 정리는 비활성입니다.
//! - `position: absolute` 자식은 흐름 밖이라 위치를 씁니다.
//!
//! ADR-0074가 Overlay 같은 비CSS 배치 방식을 없앴으므로 계획서의 "Overlay 자식"은 CSS 어휘로 대응하는 것이
//! 없습니다. 옛 `layout_mode` 참조만 있는 Frame의 자식은 엔진이 CSS 의미로 풀지 못해 정렬 전부를 비활성으로 둡니다.

use stellar_core_types::ids::StableId;
use stellar_document::css::style_defaults::{self, FlatSet};
use stellar_document::layout_record::{LayoutLonghand, LayoutValue};
use stellar_document::model::DocumentModel;
use stellar_document::records::NodePayload;

use super::place::Box4;
use crate::style_defaults::SetLayoutProperty;

/// 정렬 6종.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    /// 왼쪽 가장자리.
    Left,
    /// 가로 중심.
    CenterX,
    /// 오른쪽 가장자리.
    Right,
    /// 위 가장자리.
    Top,
    /// 세로 중심.
    CenterY,
    /// 아래 가장자리.
    Bottom,
}

impl Align {
    /// 6종 전체.
    pub const ALL: [Align; 6] = [
        Align::Left,
        Align::CenterX,
        Align::Right,
        Align::Top,
        Align::CenterY,
        Align::Bottom,
    ];

    /// 축(0 x, 1 y).
    pub fn axis(self) -> usize {
        match self {
            Align::Left | Align::CenterX | Align::Right => 0,
            Align::Top | Align::CenterY | Align::Bottom => 1,
        }
    }

    /// 축 위 기준점 비율(0 시작, 0.5 중심, 1 끝).
    fn fraction(self) -> f32 {
        match self {
            Align::Left | Align::Top => 0.0,
            Align::CenterX | Align::CenterY => 0.5,
            Align::Right | Align::Bottom => 1.0,
        }
    }

    /// CLI·MCP 이름.
    pub fn name(self) -> &'static str {
        match self {
            Align::Left => "left",
            Align::CenterX => "center-x",
            Align::Right => "right",
            Align::Top => "top",
            Align::CenterY => "center-y",
            Align::Bottom => "bottom",
        }
    }

    /// 이름에서 찾습니다.
    pub fn from_name(name: &str) -> Option<Align> {
        Align::ALL.into_iter().find(|a| a.name() == name)
    }
}

/// 정렬 기준.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reference {
    /// 부모 상자(단일 선택).
    Parent(Box4),
    /// 선택 전체의 경계(다중 선택 기본).
    Selection,
    /// 기준 객체(primary selection, 목록 번호). 기준 객체는 움직이지 않습니다.
    Key(usize),
}

/// 분배 2종. 가장 바깥 두 객체를 고정하고 사이 간격을 같게 합니다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Distribute {
    /// 가로 간격.
    Horizontal,
    /// 세로 간격.
    Vertical,
}

impl Distribute {
    fn axis(self) -> usize {
        match self {
            Distribute::Horizontal => 0,
            Distribute::Vertical => 1,
        }
    }
}

/// 행열 정리의 기본 간격(px). XII.10.2 단위 ADR(ADR-0104)이 다른 값을 정하면 그 값으로 바꿉니다(ADR-0105 결정 6).
pub const DEFAULT_GRID_GAP: f32 = 10.0;

fn valid(boxes: &[Box4]) -> bool {
    boxes
        .iter()
        .all(|b| b.iter().all(|v| v.is_finite()) && b[2] >= 0.0 && b[3] >= 0.0)
}

/// 상자들의 경계.
pub fn union(boxes: &[Box4]) -> Option<Box4> {
    let first = boxes.first()?;
    let mut min = [first[0], first[1]];
    let mut max = [first[0] + first[2], first[1] + first[3]];
    for b in &boxes[1..] {
        min = [min[0].min(b[0]), min[1].min(b[1])];
        max = [max[0].max(b[0] + b[2]), max[1].max(b[1] + b[3])];
    }
    Some([min[0], min[1], max[0] - min[0], max[1] - min[1]])
}

/// 정렬 이동량(상자 순서). 비유한·음수 크기 상자, 빈 목록, 범위 밖 기준 번호는 `None`입니다.
pub fn align_deltas(boxes: &[Box4], align: Align, reference: Reference) -> Option<Vec<[f32; 2]>> {
    if boxes.is_empty() || !valid(boxes) {
        return None;
    }
    let target = match reference {
        Reference::Parent(parent) => {
            if !valid(&[parent]) {
                return None;
            }
            parent
        }
        Reference::Selection => union(boxes)?,
        Reference::Key(index) => *boxes.get(index)?,
    };
    let axis = align.axis();
    let f = align.fraction();
    let goal = target[axis] + target[axis + 2] * f;
    Some(
        boxes
            .iter()
            .map(|b| {
                let mut delta = [0.0; 2];
                delta[axis] = goal - (b[axis] + b[axis + 2] * f);
                delta
            })
            .collect(),
    )
}

/// 분배 이동량(상자 순서). 상자 3개 미만은 `None`(비활성)입니다.
///
/// 축 위 중심 순서(같으면 목록 순서)로 늘어놓고 첫 상자와 마지막 상자를 고정합니다. 사이 상자는 첫 상자 끝과
/// 마지막 상자 시작 사이 공간을 같은 간격으로 나눕니다. 공간이 모자라면 간격이 음수(겹침)가 됩니다.
pub fn distribute_deltas(boxes: &[Box4], distribute: Distribute) -> Option<Vec<[f32; 2]>> {
    if boxes.len() < 3 || !valid(boxes) {
        return None;
    }
    let axis = distribute.axis();
    let mut order: Vec<usize> = (0..boxes.len()).collect();
    order.sort_by(|&a, &b| {
        let center = |i: usize| boxes[i][axis] + boxes[i][axis + 2] / 2.0;
        center(a).total_cmp(&center(b)).then(a.cmp(&b))
    });
    let first = boxes[order[0]];
    let last = boxes[*order.last()?];
    let inner: f32 = order[1..order.len() - 1]
        .iter()
        .map(|&i| boxes[i][axis + 2])
        .sum();
    let space = last[axis] - (first[axis] + first[axis + 2]);
    let gap = (space - inner) / (order.len() - 1) as f32;
    let mut deltas = vec![[0.0; 2]; boxes.len()];
    let mut cursor = first[axis] + first[axis + 2] + gap;
    for &i in &order[1..order.len() - 1] {
        deltas[i][axis] = cursor - boxes[i][axis];
        cursor += boxes[i][axis + 2] + gap;
    }
    Some(deltas)
}

/// 행열 정리 이동량(상자 순서). 상자 2개 미만은 `None`입니다. 흐름 컨테이너를 만들지 않고 위치만 바꿉니다.
///
/// 읽는 순서(위 가장자리, 왼쪽 가장자리, 목록 순서)로 놓고 열 수는 `ceil(√n)`입니다. 열 너비는 그 열 상자의 최대
/// 너비, 행 높이는 그 행 상자의 최대 높이이고 칸 사이는 `gap`입니다. 첫 칸은 선택 경계의 왼쪽 위입니다.
pub fn grid_deltas(boxes: &[Box4], gap: f32) -> Option<Vec<[f32; 2]>> {
    if boxes.len() < 2 || !valid(boxes) || !gap.is_finite() || gap < 0.0 {
        return None;
    }
    let bounds = union(boxes)?;
    let mut order: Vec<usize> = (0..boxes.len()).collect();
    order.sort_by(|&a, &b| {
        boxes[a][1]
            .total_cmp(&boxes[b][1])
            .then(boxes[a][0].total_cmp(&boxes[b][0]))
            .then(a.cmp(&b))
    });
    let n = order.len();
    let columns = (n as f64).sqrt().ceil() as usize;
    let rows = n.div_ceil(columns);
    let mut widths = vec![0.0f32; columns];
    let mut heights = vec![0.0f32; rows];
    for (slot, &i) in order.iter().enumerate() {
        let (row, column) = (slot / columns, slot % columns);
        widths[column] = widths[column].max(boxes[i][2]);
        heights[row] = heights[row].max(boxes[i][3]);
    }
    let offset = |sizes: &[f32], index: usize| -> f32 {
        sizes[..index].iter().map(|s| s + gap).sum::<f32>()
    };
    let mut deltas = vec![[0.0; 2]; n];
    for (slot, &i) in order.iter().enumerate() {
        let (row, column) = (slot / columns, slot % columns);
        deltas[i] = [
            bounds[0] + offset(&widths, column) - boxes[i][0],
            bounds[1] + offset(&heights, row) - boxes[i][1],
        ];
    }
    Some(deltas)
}

/// node가 놓인 배치 문맥.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowParent {
    /// 자유 배치(흐름 밖, `position: absolute` 포함). 위치를 씁니다.
    Free,
    /// flex 컨테이너의 흐름 자식.
    Flex {
        /// 주축이 가로(`row`·`row-reverse`)인지.
        row: bool,
        /// 교차축이 뒤집혔는지(`flex-wrap: wrap-reverse`).
        wrap_reverse: bool,
    },
    /// grid 컨테이너의 흐름 자식.
    Grid,
    /// 옛 `layout_mode` 참조 Frame의 자식. CSS 의미로 풀 수 없어 배치 편집을 하지 않습니다.
    Legacy,
}

impl FlowParent {
    /// 흐름 자식인지(위치를 쓰지 않음).
    pub fn is_flow(self) -> bool {
        self != FlowParent::Free
    }
}

/// 활성 기본값 세트(모델의 세트 이름이 내장 세트가 아니면 `css`).
///
/// 사용자 세트 원본은 프로젝트 파일에 있어 모델만으로 펼칠 수 없습니다. 호출자가 펼친 세트를 가지면(ops
/// snapshot) 그 값을 넘깁니다.
pub fn model_style_set(model: &DocumentModel) -> FlatSet {
    style_defaults::builtin(model.style_defaults())
        .or_else(|| style_defaults::builtin("css"))
        .expect("내장 css 세트")
}

fn resolved(
    model: &DocumentModel,
    set: &FlatSet,
    node: StableId,
    longhand: LayoutLonghand,
) -> Option<LayoutValue> {
    let payload = model.payload(node)?;
    let layout = style_defaults::resolve(set, payload, model.layout_record(node))?;
    Some(layout.get(longhand).value.clone())
}

fn parent(model: &DocumentModel, node: StableId) -> Option<StableId> {
    crate::tree::parent_of(model, node).flatten()
}

/// `node`의 배치 문맥. Fragment는 상자가 없으므로 그 위 첫 상자 조상이 컨테이너입니다(아키텍처 §12.4).
pub fn flow_parent(model: &DocumentModel, set: &FlatSet, node: StableId) -> FlowParent {
    if matches!(
        resolved(model, set, node, LayoutLonghand::Position),
        Some(LayoutValue::Keyword("absolute"))
    ) {
        return FlowParent::Free;
    }
    let mut container = parent(model, node);
    while let Some(id) = container {
        if !matches!(model.payload(id), Some(NodePayload::Fragment(_))) {
            break;
        }
        container = parent(model, id);
    }
    let Some(container) = container else {
        return FlowParent::Free;
    };
    let Some(NodePayload::Frame(frame)) = model.payload(container) else {
        return FlowParent::Free;
    };
    if frame.layout_mode.is_some() {
        return FlowParent::Legacy;
    }
    let keyword = |longhand| match resolved(model, set, container, longhand) {
        Some(LayoutValue::Keyword(k)) => k,
        _ => "",
    };
    match keyword(LayoutLonghand::Display) {
        "flex" => FlowParent::Flex {
            row: matches!(
                keyword(LayoutLonghand::FlexDirection),
                "row" | "row-reverse"
            ),
            wrap_reverse: keyword(LayoutLonghand::FlexWrap) == "wrap-reverse",
        },
        "grid" => FlowParent::Grid,
        _ => FlowParent::Free,
    }
}

/// 흐름 자식 정렬 판정.
#[derive(Clone, Debug, PartialEq)]
pub enum FlowAlign {
    /// 흐름 밖: 위치를 씁니다.
    Free,
    /// 흐름 자식: 이 longhand 하나만 바꿉니다.
    Child(SetLayoutProperty),
    /// 비활성. `hint`는 대신 바꿀 컨테이너 longhand 이름입니다(도움말).
    Disabled {
        /// 컨테이너 longhand(`justify-content`) 또는 없음.
        hint: Option<&'static str>,
    },
}

/// `node` 하나의 정렬 판정.
pub fn flow_align(model: &DocumentModel, set: &FlatSet, node: StableId, align: Align) -> FlowAlign {
    let vertical = align.axis() == 1;
    let place = |start: &'static str, end: &'static str, reverse: bool| -> &'static str {
        let f = align.fraction();
        if f == 0.5 {
            "center"
        } else if (f == 0.0) != reverse {
            start
        } else {
            end
        }
    };
    let child = |longhand: LayoutLonghand, value: &'static str| {
        FlowAlign::Child(SetLayoutProperty {
            node,
            longhand,
            value: LayoutValue::Keyword(value),
        })
    };
    match flow_parent(model, set, node) {
        FlowParent::Free => FlowAlign::Free,
        FlowParent::Legacy => FlowAlign::Disabled { hint: None },
        FlowParent::Flex { row, wrap_reverse } => {
            if vertical != row {
                // 주축 정렬은 컨테이너가 정합니다.
                FlowAlign::Disabled {
                    hint: Some("justify-content"),
                }
            } else {
                child(
                    LayoutLonghand::AlignSelf,
                    place("flex-start", "flex-end", wrap_reverse),
                )
            }
        }
        FlowParent::Grid => {
            let longhand = if vertical {
                LayoutLonghand::AlignSelf
            } else {
                LayoutLonghand::JustifySelf
            };
            child(longhand, place("start", "end", false))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrange::test_fixtures::*;
    use crate::command::Command;
    use stellar_document::layout_record::LayoutRecord;

    #[test]
    fn align_matrix_moves_each_box_to_the_reference_edge_or_center() {
        let boxes = [[0.0, 0.0, 10.0, 10.0], [20.0, 5.0, 30.0, 20.0]];
        let cases = [
            (Align::Left, [[0.0, 0.0], [-20.0, 0.0]]),
            (Align::CenterX, [[20.0, 0.0], [-10.0, 0.0]]),
            (Align::Right, [[40.0, 0.0], [0.0, 0.0]]),
            (Align::Top, [[0.0, 0.0], [0.0, -5.0]]),
            (Align::CenterY, [[0.0, 7.5], [0.0, -2.5]]),
            (Align::Bottom, [[0.0, 15.0], [0.0, 0.0]]),
        ];
        for (align, expected) in cases {
            assert_eq!(
                align_deltas(&boxes, align, Reference::Selection).unwrap(),
                expected,
                "{align:?}"
            );
        }
        // 기준 객체는 움직이지 않습니다.
        let key = align_deltas(&boxes, Align::Left, Reference::Key(1)).unwrap();
        assert_eq!(key, [[20.0, 0.0], [0.0, 0.0]]);
        // 단일 선택은 부모 기준입니다.
        let parent = align_deltas(
            &boxes[..1],
            Align::Bottom,
            Reference::Parent([0.0, 0.0, 100.0, 50.0]),
        )
        .unwrap();
        assert_eq!(parent, [[0.0, 40.0]]);
        assert!(align_deltas(&[], Align::Left, Reference::Selection).is_none());
        assert!(align_deltas(&boxes, Align::Left, Reference::Key(5)).is_none());
        assert!(align_deltas(
            &[[f32::NAN, 0.0, 1.0, 1.0]],
            Align::Left,
            Reference::Selection
        )
        .is_none());
        for align in Align::ALL {
            assert_eq!(Align::from_name(align.name()), Some(align));
        }
    }

    #[test]
    fn distribute_fixes_the_outermost_two_and_equalizes_gaps() {
        // 중심 순서: 0(0..10), 2(14..20), 1(40..50). 공간 30 - 안쪽 6 = 24 / 2 = 간격 12.
        let boxes = [
            [0.0, 0.0, 10.0, 10.0],
            [40.0, 0.0, 10.0, 10.0],
            [14.0, 3.0, 6.0, 10.0],
        ];
        let deltas = distribute_deltas(&boxes, Distribute::Horizontal).unwrap();
        assert_eq!(deltas, [[0.0, 0.0], [0.0, 0.0], [8.0, 0.0]]);
        let vertical = distribute_deltas(&boxes, Distribute::Vertical).unwrap();
        assert_eq!(vertical[0], [0.0, 0.0]);
        assert!(distribute_deltas(&boxes[..2], Distribute::Horizontal).is_none());
    }

    #[test]
    fn grid_arranges_in_reading_order_with_equal_gaps_without_a_flow_container() {
        let boxes = [
            [100.0, 100.0, 20.0, 10.0],
            [0.0, 0.0, 10.0, 30.0],
            [50.0, 0.0, 40.0, 10.0],
            [0.0, 60.0, 10.0, 10.0],
        ];
        // 읽는 순서: 1, 2, 3, 0. 2열 2행. 열 너비 [10, 40], 행 높이 [30, 10].
        let deltas = grid_deltas(&boxes, 5.0).unwrap();
        let placed: Vec<[f32; 2]> = boxes
            .iter()
            .zip(&deltas)
            .map(|(b, d)| [b[0] + d[0], b[1] + d[1]])
            .collect();
        assert_eq!(placed[1], [0.0, 0.0]);
        assert_eq!(placed[2], [15.0, 0.0]);
        assert_eq!(placed[3], [0.0, 35.0]);
        assert_eq!(placed[0], [15.0, 35.0]);
        assert!(grid_deltas(&boxes[..1], 5.0).is_none());
        assert!(grid_deltas(&boxes, -1.0).is_none());
    }

    fn container(
        model: &mut stellar_document::model::DocumentModel,
        values: &[(LayoutLonghand, &'static str)],
    ) {
        let mut record = LayoutRecord::new(sid(11));
        for (longhand, value) in values {
            record.set(*longhand, LayoutValue::Keyword(value)).unwrap();
        }
        model.set_layout_record(record).unwrap();
    }

    fn child_of_11() -> stellar_document::model::DocumentModel {
        let mut model = shapes();
        model
            .insert_node(record(30, Some(11), 0x10, rectangle(5.0, 5.0)))
            .unwrap();
        model
    }

    #[test]
    fn flow_children_change_only_self_alignment_or_are_disabled() {
        let set = style_defaults::builtin("css").unwrap();
        // 자유 배치 자식은 위치를 씁니다.
        let model = child_of_11();
        assert_eq!(
            flow_align(&model, &set, sid(30), Align::Left),
            FlowAlign::Free
        );
        // flex row: 가로는 주축(비활성, justify-content 안내), 세로는 align-self.
        let mut model = child_of_11();
        container(&mut model, &[(LayoutLonghand::Display, "flex")]);
        assert_eq!(
            flow_align(&model, &set, sid(30), Align::Right),
            FlowAlign::Disabled {
                hint: Some("justify-content")
            }
        );
        let FlowAlign::Child(set_self) = flow_align(&model, &set, sid(30), Align::Bottom) else {
            panic!()
        };
        assert_eq!(set_self.longhand, LayoutLonghand::AlignSelf);
        assert_eq!(set_self.value, LayoutValue::Keyword("flex-end"));
        // wrap-reverse는 교차축 시작·끝이 바뀝니다.
        container(
            &mut model,
            &[
                (LayoutLonghand::Display, "flex"),
                (LayoutLonghand::FlexWrap, "wrap-reverse"),
            ],
        );
        let FlowAlign::Child(set_self) = flow_align(&model, &set, sid(30), Align::Top) else {
            panic!()
        };
        assert_eq!(set_self.value, LayoutValue::Keyword("flex-end"));
        // column: 가로가 교차축.
        container(
            &mut model,
            &[
                (LayoutLonghand::Display, "flex"),
                (LayoutLonghand::FlexDirection, "column"),
            ],
        );
        let FlowAlign::Child(set_self) = flow_align(&model, &set, sid(30), Align::CenterX) else {
            panic!()
        };
        assert_eq!(
            (set_self.longhand, set_self.value),
            (LayoutLonghand::AlignSelf, LayoutValue::Keyword("center"))
        );
        // grid: 가로 justify-self, 세로 align-self.
        container(&mut model, &[(LayoutLonghand::Display, "grid")]);
        let FlowAlign::Child(set_self) = flow_align(&model, &set, sid(30), Align::Right) else {
            panic!()
        };
        assert_eq!(
            (set_self.longhand, set_self.value.clone()),
            (LayoutLonghand::JustifySelf, LayoutValue::Keyword("end"))
        );
        // 적용하면 바뀐 longhand는 그 하나뿐입니다.
        let (state, _) = apply(model, vec![Command::SetLayoutProperty(set_self)]);
        let record = state.model().layout_record(sid(30)).unwrap();
        assert_eq!(record.iter().count(), 1);
        // react-native 세트는 Frame 기본이 flex(column)입니다.
        let rn = style_defaults::builtin("react-native").unwrap();
        let model = child_of_11();
        assert!(flow_parent(&model, &rn, sid(30)).is_flow());
        // absolute 자식은 흐름 밖입니다.
        let mut model = child_of_11();
        container(&mut model, &[(LayoutLonghand::Display, "flex")]);
        let mut own = LayoutRecord::new(sid(30));
        own.set(LayoutLonghand::Position, LayoutValue::Keyword("absolute"))
            .unwrap();
        model.set_layout_record(own).unwrap();
        assert_eq!(flow_parent(&model, &set, sid(30)), FlowParent::Free);
    }
}
