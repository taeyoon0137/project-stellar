//! 순서 명령 4종: Bring Forward·Send Backward·Bring to Front·Send to Back (XII.10.23, ADR-0105 결정 7).
//!
//! 결과는 OrderKey만 바꾸는 `MoveNode`(같은 부모)입니다. 여러 node를 고르면 부모마다 따로, 고른 node끼리의 상대
//! 순서를 지키며 옮깁니다. 고르지 않은 형제의 OrderKey는 그대로이고, 옮긴 node의 새 key는 새 자리의 고르지 않은 두
//! 이웃 사이에서 만듭니다. 흐름 컨테이너(flex·grid)의 자식이면 순서가 곧 흐름 순서이므로 결과에 알림 표시
//! ([`OrderPlan::flow_order_changed`])를 둡니다. 셸은 이 값으로 도움말과 접근성 알림을 냅니다.

use std::collections::BTreeMap;

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_document::css::style_defaults::FlatSet;
use stellar_document::model::{DocumentModel, SiblingOrder};

use super::align::flow_parent;
use crate::command::Command;
use crate::lock::{edit_permission, EditIntent};
use crate::node_ops::MoveNode;

/// 순서 명령 계획.
#[derive(Clone, Debug, PartialEq)]
pub struct OrderPlan {
    /// OrderKey를 바꾸는 MoveNode(바뀐 node만).
    pub commands: Vec<Command>,
    /// 흐름 컨테이너 자식의 순서가 바뀌어 흐름 순서도 바뀌었는지.
    pub flow_order_changed: bool,
}

/// 순서 명령을 만들 수 없는 이유.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderError {
    /// 선택이 비었습니다.
    Empty,
    /// node가 없거나 부모가 없습니다(View root).
    NoParent(StableId),
    /// 잠금이 막습니다.
    Blocked(StableId),
}

/// 형제 목록(뒤 → 앞)에서 고른 node를 `direction`으로 옮긴 새 목록.
fn reordered(children: &[StableId], picked: &[bool], direction: SiblingOrder) -> Vec<StableId> {
    let mut items: Vec<(StableId, bool)> = children
        .iter()
        .copied()
        .zip(picked.iter().copied())
        .collect();
    match direction {
        SiblingOrder::Front => {
            let (mut rest, chosen): (Vec<_>, Vec<_>) = items.into_iter().partition(|(_, p)| !p);
            rest.extend(chosen);
            items = rest;
        }
        SiblingOrder::Back => {
            let (rest, mut chosen): (Vec<_>, Vec<_>) = items.into_iter().partition(|(_, p)| !p);
            chosen.extend(rest);
            items = chosen;
        }
        SiblingOrder::Forward => {
            // 앞쪽부터: 고른 node 바로 앞이 고르지 않은 node면 자리를 바꿉니다.
            for i in (0..items.len().saturating_sub(1)).rev() {
                if items[i].1 && !items[i + 1].1 {
                    items.swap(i, i + 1);
                }
            }
        }
        SiblingOrder::Backward => {
            for i in 1..items.len() {
                if items[i].1 && !items[i - 1].1 {
                    items.swap(i, i - 1);
                }
            }
        }
    }
    items.into_iter().map(|(id, _)| id).collect()
}

/// `nodes`를 `direction`으로 옮기는 계획. 이미 끝에 있어 바뀌는 것이 없으면 명령이 비어 있습니다.
pub fn order_commands(
    model: &DocumentModel,
    set: &FlatSet,
    nodes: &[StableId],
    direction: SiblingOrder,
) -> Result<OrderPlan, OrderError> {
    if nodes.is_empty() {
        return Err(OrderError::Empty);
    }
    let mut by_parent: BTreeMap<StableId, Vec<StableId>> = BTreeMap::new();
    for &node in nodes {
        let parent = crate::tree::parent_of(model, node)
            .flatten()
            .ok_or(OrderError::NoParent(node))?;
        edit_permission(model, node, EditIntent::Edit).map_err(|_| OrderError::Blocked(node))?;
        by_parent.entry(parent).or_default().push(node);
    }
    let mut commands = Vec::new();
    let mut flow_order_changed = false;
    for (parent, chosen) in by_parent {
        let children = model.children_in_order(parent);
        let picked: Vec<bool> = children.iter().map(|c| chosen.contains(c)).collect();
        let next = reordered(&children, &picked, direction);
        if next == children {
            continue;
        }
        let key = |id: StableId| crate::tree::order_key_of(model, id);
        let taken: std::collections::BTreeSet<OrderKey> =
            children.iter().filter_map(|&c| key(c)).collect();
        // 새 목록에서 고른 node 묶음마다 고르지 않은 이웃 사이 key를 차례로 만듭니다.
        let mut index = 0;
        while index < next.len() {
            if !chosen.contains(&next[index]) {
                index += 1;
                continue;
            }
            let start = index;
            while index < next.len() && chosen.contains(&next[index]) {
                index += 1;
            }
            let left = start.checked_sub(1).and_then(|i| key(next[i]));
            let right = next.get(index).and_then(|&id| key(id));
            let mut previous = left;
            for &node in &next[start..index] {
                // 앞 MoveNode가 적용될 때 아직 옮기지 않은 형제 key와 겹치지 않게 합니다(DuplicateOrderKey 방지).
                let mut order_key = OrderKey::between(previous.as_ref(), right.as_ref());
                while taken.contains(&order_key) {
                    order_key = OrderKey::between(previous.as_ref(), Some(&order_key));
                }
                previous = Some(order_key.clone());
                if key(node).as_ref() == Some(&order_key) {
                    continue;
                }
                commands.push(Command::MoveNode(MoveNode {
                    node,
                    new_parent: parent,
                    order_key,
                }));
            }
        }
        flow_order_changed |= chosen.iter().any(|&n| flow_parent(model, set, n).is_flow());
    }
    Ok(OrderPlan {
        flow_order_changed: flow_order_changed && !commands.is_empty(),
        commands,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrange::align::model_style_set;
    use crate::arrange::test_fixtures::*;

    fn order(state: &crate::state::DocumentState) -> Vec<StableId> {
        state.model().children_in_order(sid(10))
    }

    #[test]
    fn four_order_commands_move_selected_blocks_and_keep_relative_order() {
        let model = shapes();
        let set = model_style_set(&model);
        // 뒤 → 앞: 11, 12, 13, 14, 15, 16.
        let cases = [
            (SiblingOrder::Front, vec![11, 12, 14, 16, 13, 15]),
            (SiblingOrder::Back, vec![13, 15, 11, 12, 14, 16]),
            (SiblingOrder::Forward, vec![11, 12, 14, 13, 16, 15]),
            (SiblingOrder::Backward, vec![11, 13, 12, 15, 14, 16]),
        ];
        for (direction, expected) in cases {
            let plan = order_commands(&model, &set, &[sid(15), sid(13)], direction).unwrap();
            assert!(!plan.flow_order_changed);
            let (mut state, inverse) = apply(copy(&model), plan.commands);
            let expected: Vec<StableId> = expected.into_iter().map(sid).collect();
            assert_eq!(order(&state), expected, "{direction:?}");
            undo(&mut state, &inverse);
            assert_eq!(order(&state), model.children_in_order(sid(10)));
        }
        // 이미 맨 앞이면 명령이 없습니다.
        let plan = order_commands(&model, &set, &[sid(16)], SiblingOrder::Front).unwrap();
        assert!(plan.commands.is_empty());
        assert_eq!(
            order_commands(&model, &set, &[sid(10)], SiblingOrder::Front),
            Err(OrderError::NoParent(sid(10)))
        );
    }

    #[test]
    fn flow_container_children_report_the_flow_order_notice() {
        use stellar_document::layout_record::{LayoutLonghand, LayoutRecord, LayoutValue};
        let mut model = shapes();
        model
            .insert_node(record(30, Some(11), 0x10, rectangle(5.0, 5.0)))
            .unwrap();
        model
            .insert_node(record(31, Some(11), 0x20, rectangle(5.0, 5.0)))
            .unwrap();
        let mut flex = LayoutRecord::new(sid(11));
        flex.set(LayoutLonghand::Display, LayoutValue::Keyword("flex"))
            .unwrap();
        model.set_layout_record(flex).unwrap();
        let set = model_style_set(&model);
        let plan = order_commands(&model, &set, &[sid(30)], SiblingOrder::Forward).unwrap();
        assert_eq!(plan.commands.len(), 1);
        assert!(plan.flow_order_changed);
    }
}
