//! 키보드 이동·크기 조절 (XII.10.14, ADR-0105 결정 2).
//!
//! - 화살표: 선택이 있으면 부모 공간 `step` px 이동입니다. 이동은 기존 캔버스 이동 경로(`CanvasEdit::Move`, 부모 공간
//!   단위)이고, 같은 경로가 0.5초 안 반복 입력을 한 history 항목으로 합칩니다(VII.4.4).
//! - 흐름 컨테이너 자식: 주축 화살표는 흐름 순서를 한 칸 바꿉니다(OrderKey MoveNode). 교차축 화살표는 비활성입니다.
//! - 선택이 없으면 화살표는 camera를 팬합니다. 문서 revision과 history는 바뀌지 않습니다.
//! - 크기 조절 키([`resize_key`]): 오른쪽·아래 가장자리를 옮깁니다(원점 고정, 최소 1px). 반복 입력은 같은 node·속성이면
//!   한 항목입니다.
//! - 단위(`step`, 화살표 1px·Shift 10px 등)와 키 배정은 XII.10.2 단위 ADR(ADR-0104)이 정하고 셸이 넘깁니다.

use stellar_commands::arrange::align::{flow_parent, model_style_set, FlowParent};
use stellar_commands::arrange::order::order_commands;
use stellar_commands::arrange::place;
use stellar_commands::inspector::{InspectorProperty, NodeClass};
use stellar_commands::kind::CommandKind;
use stellar_core_types::clock::MonotonicClock;
use stellar_core_types::ids::StableId;
use stellar_core_types::value::Vec2;
use stellar_document::model::SiblingOrder;
use stellar_history::entry::{CommandSignature, GroupingKey};

use super::{geometries, selection_roots, TransformError};
use crate::app::canvas_gesture::{apply_edit, CanvasEdit};
use crate::app::gesture_math::MIN_SIZE;
use crate::app::RequestId;
use crate::canvas::CanvasScene;
use crate::window_session::{history_label, ProjectWindowSession};

/// 화살표 방향.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrow {
    /// 왼쪽.
    Left,
    /// 오른쪽.
    Right,
    /// 위.
    Up,
    /// 아래.
    Down,
}

impl Arrow {
    fn vector(self) -> [f32; 2] {
        match self {
            Arrow::Left => [-1.0, 0.0],
            Arrow::Right => [1.0, 0.0],
            Arrow::Up => [0.0, -1.0],
            Arrow::Down => [0.0, 1.0],
        }
    }

    fn horizontal(self) -> bool {
        matches!(self, Arrow::Left | Arrow::Right)
    }
}

/// 화살표 한 번의 결과.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyOutcome {
    /// 선택이 없어 camera를 팬했습니다.
    Panned,
    /// 선택을 옮겼습니다.
    Moved,
    /// 흐름 순서를 바꿨습니다.
    Reordered,
    /// 바꿀 것이 없었습니다(맨 끝 흐름 자식, 교차축 화살표).
    Unchanged,
}

/// 흐름 자식의 주축 화살표가 흐름 순서에서 뒤(앞 그리기 순서)로 가는지. 교차축이면 `None`.
fn flow_step(parent: FlowParent, arrow: Arrow) -> Option<SiblingOrder> {
    let forward = match parent {
        FlowParent::Flex { row, reverse, .. } => {
            if arrow.horizontal() != row {
                return None;
            }
            matches!(arrow, Arrow::Right | Arrow::Down) != reverse
        }
        FlowParent::Grid => matches!(arrow, Arrow::Right | Arrow::Down),
        FlowParent::Free | FlowParent::Legacy => return None,
    };
    Some(if forward {
        SiblingOrder::Forward
    } else {
        SiblingOrder::Backward
    })
}

/// 화살표 한 번. `step`은 부모 공간 px, `pan`은 선택이 없을 때 camera 팬 거리(화면 pt)입니다.
pub fn arrow(
    window: &mut ProjectWindowSession,
    scene: &CanvasScene,
    arrow: Arrow,
    step: f32,
    pan: f64,
    request: RequestId,
) -> Result<KeyOutcome, TransformError> {
    let [dx, dy] = arrow.vector();
    if window.view().selection().ids().is_empty() {
        window
            .pan(-f64::from(dx) * pan, -f64::from(dy) * pan)
            .map_err(|error| TransformError::Edit(format!("{error:?}")))?;
        return Ok(KeyOutcome::Panned);
    }
    let view = window.tabs().active_view().ok_or(TransformError::Stale)?;
    let (model, _root, roots) = selection_roots(window)?;
    let set = model_style_set(model);
    let contexts: Vec<FlowParent> = roots.iter().map(|&n| flow_parent(model, &set, n)).collect();
    if let Some(context) = contexts.iter().copied().find(|c| c.is_flow()) {
        // 주축 방향(`*-reverse`)은 Fragment를 건너뛴 흐름 컨테이너 기준으로 `flow_parent`가 이미 풀었습니다.
        let Some(direction) = flow_step(context, arrow) else {
            return Ok(KeyOutcome::Unchanged);
        };
        let plan = order_commands(model, &set, &roots, direction)?;
        let changed = super::commit(window, plan.commands, history_label::REORDER)?;
        return Ok(if changed {
            KeyOutcome::Reordered
        } else {
            KeyOutcome::Unchanged
        });
    }
    let delta =
        Vec2::new(dx * step, dy * step).map_err(|_| TransformError::Disabled("non-finite"))?;
    apply_edit(
        window,
        scene,
        request,
        view,
        CanvasEdit::Move {
            anchor: None,
            delta,
            parent_space: true,
            duplicate: false,
        },
    )
    .map_err(|_| TransformError::Stale)?;
    Ok(KeyOutcome::Moved)
}

/// 크기 조절 키 한 번: `Right`·`Down`은 오른쪽·아래 가장자리를 `step`만큼 밖으로, `Left`·`Up`은 안으로 옮깁니다.
/// 같은 node·속성의 반복은 `clock` 기준 VII.4.4 규칙으로 한 history 항목입니다. 크기가 없는 node는 건너뜁니다.
pub fn resize_key<C: MonotonicClock>(
    window: &mut ProjectWindowSession,
    scene: &CanvasScene,
    arrow: Arrow,
    step: f32,
    clock: &C,
) -> Result<bool, TransformError> {
    let (model, _root, roots) = selection_roots(window)?;
    if roots.is_empty() {
        return Ok(false);
    }
    let targets = geometries(scene, &roots)?;
    let [dx, dy] = arrow.vector();
    let mut commands = Vec::new();
    for target in &targets {
        if !place::has_size(model, target.id) {
            continue;
        }
        let Some(local) = place::local_box(model, target.id) else {
            continue;
        };
        let width = (local[2] + dx * step).max(MIN_SIZE);
        let height = (local[3] + dy * step).max(MIN_SIZE);
        commands.extend(place::set_size(model, target.id, width, height)?);
    }
    if commands.is_empty() {
        return Ok(false);
    }
    let first: StableId = roots[0];
    let class = model.payload(first).map_or(NodeClass::Other, NodeClass::of);
    let property = if arrow.horizontal() {
        InspectorProperty::Width
    } else {
        InspectorProperty::Height
    };
    let key = GroupingKey::KeyboardRepeat(CommandSignature {
        kind: CommandKind::SetProperty,
        target: first,
        key: Some(property.key(class)),
    });
    let tx = super::transaction(window, commands)?;
    window
        .apply_grouped(&tx, key, clock, history_label::RESIZE)
        .map_err(|error| TransformError::Edit(format!("{error:?}")))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::*;
    use super::*;
    use crate::window_session::test_fixtures::sid;
    use stellar_commands::command::Command;
    use stellar_commands::style_defaults::SetLayoutProperty;
    use stellar_core_types::clock::ManualClock;
    use stellar_document::layout_record::{LayoutLonghand, LayoutValue};
    use stellar_document::records::NodePayload;
    use stellar_document::shape::ShapeGeometry;

    #[test]
    fn twenty_repeated_resize_keys_are_one_entry_and_move_only_the_far_edges() {
        let (_dir, mut w) = open("key-resize", &canvas_document());
        select(&mut w, &[13]);
        let s = scene(&w);
        let clock = ManualClock::new(0);
        let before = history_len(&w);
        for _ in 0..20 {
            assert!(resize_key(&mut w, &s, Arrow::Right, 1.0, &clock).unwrap());
        }
        assert_eq!(history_len(&w), before + 1);
        assert!(
            matches!(model(&w).payload(sid(13)), Some(NodePayload::Shape(ShapeGeometry::Rectangle(r))) if r.size.width() == 60.0)
        );
        // 원점(왼쪽 위)은 그대로입니다.
        assert!(close(matrix(&w, 13), (1., 0., 0., 1., 30., 100.)));
        // 줄이기는 최소 1px에서 멈춥니다.
        for _ in 0..100 {
            resize_key(&mut w, &s, Arrow::Up, 1.0, &clock).unwrap();
        }
        assert!(
            matches!(model(&w).payload(sid(13)), Some(NodePayload::Shape(ShapeGeometry::Rectangle(r))) if r.size.height() == 1.0)
        );
    }

    #[test]
    fn arrows_move_the_selection_in_parent_space_and_repeats_group() {
        let (_dir, mut w) = open("key-move", &canvas_document());
        select(&mut w, &[13]);
        let before = history_len(&w);
        for request in 0..20 {
            let s = scene(&w);
            assert_eq!(
                arrow(&mut w, &s, Arrow::Right, 1.0, 10.0, RequestId(request)).unwrap(),
                KeyOutcome::Moved
            );
        }
        assert!(close(matrix(&w, 13), (1., 0., 0., 1., 50., 100.)));
        // 0.5초 안 반복은 한 항목(부하가 크면 둘로 나뉠 수 있어 상한만 봅니다).
        assert!(
            history_len(&w) - before <= 2,
            "{}",
            history_len(&w) - before
        );
    }

    #[test]
    fn no_selection_pans_the_camera_without_touching_the_document() {
        let (_dir, mut w) = open("key-pan", &canvas_document());
        select(&mut w, &[]);
        let before = (w.revision(), history_len(&w));
        let s = scene(&w);
        assert_eq!(
            arrow(&mut w, &s, Arrow::Down, 1.0, 20.0, RequestId(1)).unwrap(),
            KeyOutcome::Panned
        );
        assert_eq!((w.revision(), history_len(&w)), before);
    }

    #[test]
    fn flow_children_reorder_on_the_main_axis_only() {
        let mut doc = canvas_document();
        for (id, x) in [(30u64, 0.0f32), (31, 20.0)] {
            add_shape(
                &mut doc,
                id,
                11,
                ShapeGeometry::Rectangle(stellar_document::shape::RectangleParams {
                    size: stellar_core_types::value::Size2::new(10., 10.).unwrap(),
                    corner_radius: Default::default(),
                }),
                at(x, 0.0),
            );
        }
        let (_dir, mut w) = open("key-flow", &doc);
        let flex = vec![Command::SetLayoutProperty(SetLayoutProperty {
            node: sid(11),
            longhand: LayoutLonghand::Display,
            value: LayoutValue::Keyword("flex"),
        })];
        super::super::commit(&mut w, flex, 0).unwrap();
        select(&mut w, &[30]);
        let s = scene(&w);
        let revision = w.revision();
        assert_eq!(
            arrow(&mut w, &s, Arrow::Down, 1.0, 10.0, RequestId(1)).unwrap(),
            KeyOutcome::Unchanged
        );
        assert_eq!(w.revision(), revision);
        assert_eq!(
            arrow(&mut w, &s, Arrow::Right, 1.0, 10.0, RequestId(2)).unwrap(),
            KeyOutcome::Reordered
        );
        assert_eq!(model(&w).children_in_order(sid(11)), vec![sid(31), sid(30)]);
        // 위치는 바뀌지 않습니다.
        assert!(close(matrix(&w, 30), (1., 0., 0., 1., 0., 0.)));
    }
}
