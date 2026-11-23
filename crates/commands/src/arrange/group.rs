//! 묶음 명령: Group·Ungroup·Frame Selection (XII.10.23, ADR-0105 결정 7).
//!
//! - Group: 고른 node(같은 부모)의 가장 앞 node 자리에 Fragment를 넣고(`InsertNode`) 고른 node를 그 안으로 옮깁니다
//!   (`MoveNode`, 원래 OrderKey 유지). Fragment는 상자·변환이 없으므로 world 위치와 흐름 컨테이너 안 배치가 그대로입니다.
//! - Frame Selection: 같은 자리에 고른 node 경계 크기의 Frame을 넣고 자식으로 옮깁니다. 새 Frame은 배경·테두리가
//!   없어 외형이 바뀌지 않고, 자식 위치는 새 Frame 원점만큼 되돌려 world 위치를 지킵니다. 흐름 자식은 거부합니다.
//! - Ungroup: Fragment·Frame의 자식을 그 부모의 같은 자리로 옮기고 빈 Fragment·Frame을 지웁니다(`DeleteSubtree`).
//!   Frame을 풀 때는 Frame 변환을 자식 위치에 합쳐 world 위치를 보존합니다([`super::place::apply_affine`]).
//!
//! 모두 호출자가 CommandTransaction 하나로 적용합니다(Undo 한 번).

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::value::{Size2, Transform2D};
use stellar_document::css::style_defaults::FlatSet;
use stellar_document::fragment::FragmentPayload;
use stellar_document::frame::FramePayload;
use stellar_document::model::DocumentModel;
use stellar_document::records::{NodePayload, NodeRecord};

use super::align::flow_parent;
use super::place::{apply_affine, translation, Box4, PlaceError};
use crate::command::Command;
use crate::lock::{edit_permission, EditIntent};
use crate::node_ops::{DeleteSubtree, InsertNode, MoveNode};

/// 묶음 명령을 만들 수 없는 이유.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupError {
    /// 선택이 비었습니다.
    Empty,
    /// 고른 node가 한 부모를 공유하지 않거나 부모가 없습니다(View root).
    MixedParents,
    /// 잠금이 막습니다.
    Blocked(StableId),
    /// 풀 수 있는 Fragment·Frame이 아닙니다.
    NotAGroup(StableId),
    /// 흐름 컨테이너 자식이라 Frame으로 감쌀 수 없습니다.
    Flow(StableId),
    /// 위치 보정을 표현할 수 없습니다.
    Place(PlaceError),
}

impl From<PlaceError> for GroupError {
    fn from(error: PlaceError) -> GroupError {
        GroupError::Place(error)
    }
}

fn parent(model: &DocumentModel, node: StableId) -> Option<StableId> {
    crate::tree::parent_of(model, node).flatten()
}

fn key(model: &DocumentModel, node: StableId) -> Option<OrderKey> {
    crate::tree::order_key_of(model, node)
}

/// 고른 node의 공통 부모와, 그 부모 안에서 가장 앞 node 바로 위(다음 형제 앞) 자리.
fn common_slot(
    model: &DocumentModel,
    nodes: &[StableId],
) -> Result<(StableId, OrderKey), GroupError> {
    let first = *nodes.first().ok_or(GroupError::Empty)?;
    let parent_id = parent(model, first).ok_or(GroupError::MixedParents)?;
    for &node in nodes {
        if parent(model, node) != Some(parent_id) {
            return Err(GroupError::MixedParents);
        }
        edit_permission(model, node, EditIntent::Move).map_err(|_| GroupError::Blocked(node))?;
    }
    let siblings = model.children_in_order(parent_id);
    let front = siblings
        .iter()
        .rposition(|s| nodes.contains(s))
        .ok_or(GroupError::Empty)?;
    let own = key(model, siblings[front]);
    let next = siblings.get(front + 1).and_then(|&s| key(model, s));
    Ok((parent_id, OrderKey::between(own.as_ref(), next.as_ref())))
}

/// 고른 node를 형제 순서대로 `container` 안으로 옮기는 MoveNode(원래 OrderKey 유지).
fn move_into(
    model: &DocumentModel,
    parent_id: StableId,
    nodes: &[StableId],
    container: StableId,
) -> Vec<Command> {
    model
        .children_in_order(parent_id)
        .into_iter()
        .filter(|s| nodes.contains(s))
        .filter_map(|node| {
            Some(Command::MoveNode(MoveNode {
                node,
                new_parent: container,
                order_key: key(model, node)?,
            }))
        })
        .collect()
}

fn new_record(
    id: StableId,
    parent_id: StableId,
    order_key: OrderKey,
    name: &str,
    payload: NodePayload,
) -> NodeRecord {
    NodeRecord {
        id,
        parent: Some(parent_id),
        order_key,
        visible: true,
        locked: false,
        display_name: name.to_string(),
        payload,
    }
}

/// Group: 고른 node를 새 Fragment `group`으로 묶습니다.
pub fn group(
    model: &DocumentModel,
    nodes: &[StableId],
    group: StableId,
) -> Result<Vec<Command>, GroupError> {
    let (parent_id, slot) = common_slot(model, nodes)?;
    let mut commands = vec![Command::InsertNode(InsertNode::single(new_record(
        group,
        parent_id,
        slot,
        "Group",
        NodePayload::Fragment(FragmentPayload),
    )))];
    commands.extend(move_into(model, parent_id, nodes, group));
    Ok(commands)
}

/// Frame Selection: 고른 node를 부모 공간 상자 `bounds`(보통 고른 node의 경계) 크기의 새 Frame `frame`으로 감쌉니다.
/// `new_modifier`는 위치 Transform modifier가 없는 자식의 보정에 씁니다.
pub fn frame_selection(
    model: &DocumentModel,
    set: &FlatSet,
    nodes: &[StableId],
    bounds: Box4,
    frame: StableId,
    new_modifier: &mut dyn FnMut() -> StableId,
) -> Result<Vec<Command>, GroupError> {
    let (parent_id, slot) = common_slot(model, nodes)?;
    if let Some(&flow) = nodes
        .iter()
        .find(|&&n| flow_parent(model, set, n).is_flow())
    {
        return Err(GroupError::Flow(flow));
    }
    let [x, y, w, h] = bounds;
    let size = Size2::new(w, h).map_err(|_| GroupError::Place(PlaceError::NonFinite(frame)))?;
    let transform = translation(x, y).ok_or(GroupError::Place(PlaceError::NonFinite(frame)))?;
    let payload = NodePayload::Frame(FramePayload {
        size,
        transform,
        background: None,
        border: None,
        ..FramePayload::default()
    });
    let mut commands = vec![Command::InsertNode(InsertNode::single(new_record(
        frame, parent_id, slot, "Frame", payload,
    )))];
    commands.extend(move_into(model, parent_id, nodes, frame));
    let back = translation(-x, -y).ok_or(GroupError::Place(PlaceError::NonFinite(frame)))?;
    for &node in nodes {
        if let Some(command) = apply_affine(model, node, back, new_modifier)? {
            commands.push(command);
        }
    }
    Ok(commands)
}

/// Ungroup: Fragment·Frame `node`의 자식을 그 부모의 같은 자리로 옮기고 `node`를 지웁니다.
pub fn ungroup(
    model: &DocumentModel,
    node: StableId,
    new_modifier: &mut dyn FnMut() -> StableId,
) -> Result<Vec<Command>, GroupError> {
    let frame_transform: Option<Transform2D> = match model.payload(node) {
        Some(NodePayload::Fragment(_)) => None,
        Some(NodePayload::Frame(frame)) => Some(frame.transform),
        _ => return Err(GroupError::NotAGroup(node)),
    };
    let parent_id = parent(model, node).ok_or(GroupError::NotAGroup(node))?;
    edit_permission(model, node, EditIntent::Delete).map_err(|_| GroupError::Blocked(node))?;
    let siblings = model.children_in_order(parent_id);
    let index = siblings
        .iter()
        .position(|&s| s == node)
        .ok_or(GroupError::NotAGroup(node))?;
    let left = index.checked_sub(1).and_then(|i| key(model, siblings[i]));
    let right = siblings.get(index + 1).and_then(|&s| key(model, s));
    let own = key(model, node).ok_or(GroupError::NotAGroup(node))?;
    let children = model.children_in_order(node);
    let child_keys: Vec<OrderKey> = children.iter().filter_map(|&c| key(model, c)).collect();
    let taken: std::collections::BTreeSet<OrderKey> =
        siblings.iter().filter_map(|&s| key(model, s)).collect();
    // 자식의 지금 key가 모두 묶음 자리(아래 이웃과 위 이웃 사이)에 들어가고 형제 key와 겹치지 않으면 그대로 씁니다.
    // Group이 원래 key를 그대로 옮겨 두므로 Group 뒤 Ungroup은 순서 key까지 묶기 전과 같습니다.
    let fits = child_keys.len() == children.len()
        && child_keys.iter().all(|k| {
            left.as_ref().is_none_or(|l| k > l)
                && right.as_ref().is_none_or(|r| k < r)
                && !taken.contains(k)
        });
    let mut commands = Vec::new();
    let mut previous = left;
    for (i, &child) in children.iter().enumerate() {
        let order_key = if fits {
            child_keys[i].clone()
        } else {
            // 아직 남아 있는 묶음 key와 겹치지 않게 (아래 이웃, 묶음) 사이에 차례로 둡니다. 묶음을 지우면 그 자리입니다.
            let next = OrderKey::between(previous.as_ref(), Some(&own));
            previous = Some(next.clone());
            next
        };
        commands.push(Command::MoveNode(MoveNode {
            node: child,
            new_parent: parent_id,
            order_key,
        }));
    }
    if let Some(transform) = frame_transform {
        for &child in &children {
            if let Some(command) = apply_affine(model, child, transform, new_modifier)? {
                commands.push(command);
            }
        }
    }
    commands.push(Command::DeleteSubtree(DeleteSubtree { root: node }));
    Ok(commands)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrange::align::model_style_set;
    use crate::arrange::place::local_transform;
    use crate::arrange::test_fixtures::*;

    #[test]
    fn group_then_ungroup_restores_the_same_tree_and_values() {
        let model = shapes();
        let mut fresh = ids();
        let commands = group(&model, &[sid(15), sid(13)], fresh()).unwrap();
        let (state, _) = apply(copy(&model), commands);
        let grouped = copy(state.model());
        let fragment = sid(1001);
        assert_eq!(grouped.children_in_order(fragment), vec![sid(13), sid(15)]);
        // 그룹은 가장 앞 node(15)의 자리입니다: 11, 12, 14, (13, 15), 16.
        assert_eq!(
            grouped.children_in_order(sid(10)),
            vec![sid(11), sid(12), sid(14), fragment, sid(16)]
        );
        let commands = ungroup(&grouped, fragment, &mut fresh).unwrap();
        let (state, _) = apply(grouped, commands);
        let back = state.model();
        assert_eq!(
            back.children_in_order(sid(10)),
            vec![sid(11), sid(12), sid(14), sid(13), sid(15), sid(16)]
        );
        // 떨어져 있던 13·15는 묶음 자리에 이웃해 남고, 위치·modifier·payload는 묶기 전과 같습니다.
        for id in [11, 13, 14, 15, 16] {
            let (a, b) = (
                model.record(sid(id)).unwrap(),
                back.record(sid(id)).unwrap(),
            );
            assert_eq!((a.payload, a.parent), (b.payload, b.parent));
            assert_eq!(model.modifier_stack(sid(id)), back.modifier_stack(sid(id)));
        }
        // 이웃한 node를 묶었다 풀면 순서 key까지 레코드 전체가 묶기 전과 같습니다(semantic snapshot 동일).
        let commands = group(&model, &[sid(13), sid(14)], fresh()).unwrap();
        let (state, _) = apply(copy(&model), commands);
        let grouped = copy(state.model());
        let fragment = grouped.children_in_order(sid(10))[2];
        let commands = ungroup(&grouped, fragment, &mut fresh).unwrap();
        let (state, _) = apply(grouped, commands);
        assert_eq!(
            state.model().to_records().unwrap(),
            model.to_records().unwrap()
        );
        assert!(back.payload(fragment).is_none());
        assert!(group(&model, &[sid(13), sid(10)], sid(1)).is_err());
        assert_eq!(group(&model, &[], sid(1)), Err(GroupError::Empty));
    }

    #[test]
    fn frame_selection_and_frame_ungroup_keep_world_positions() {
        let model = shapes();
        let set = model_style_set(&model);
        let mut fresh = ids();
        // 13(위치 30, 40) 과 11(위치 10, 20)을 (10, 20) 원점 Frame으로 감쌉니다.
        let frame = fresh();
        let commands = frame_selection(
            &model,
            &set,
            &[sid(11), sid(13)],
            [10.0, 20.0, 100.0, 50.0],
            frame,
            &mut fresh,
        )
        .unwrap();
        let (mut state, inverse) = apply(copy(&model), commands);
        let wrapped = copy(state.model());
        assert_eq!(wrapped.children_in_order(frame), vec![sid(11), sid(13)]);
        let world = |m: &DocumentModel, node: u64| -> (f32, f32) {
            let mut t = local_transform(m, sid(node)).unwrap();
            let mut p = crate::tree::parent_of(m, sid(node)).flatten();
            while let Some(id) = p {
                if let Some(up) = local_transform(m, id) {
                    t = t.then(&up).unwrap();
                }
                p = crate::tree::parent_of(m, id).flatten();
            }
            let (_, _, _, _, x, y) = t.components();
            (x, y)
        };
        assert_eq!(world(&wrapped, 13), (30.0, 40.0));
        assert_eq!(world(&wrapped, 11), (10.0, 20.0));
        // undo 한 번이면 감싸기 전과 같습니다.
        undo(&mut state, &inverse);
        assert_eq!(
            state.model().to_records().unwrap(),
            model.to_records().unwrap()
        );
        // Frame을 풀면 자식 위치에 Frame 변환을 합칩니다.
        let commands = ungroup(&wrapped, frame, &mut fresh).unwrap();
        let (state, _) = apply(wrapped, commands);
        assert_eq!(world(state.model(), 13), (30.0, 40.0));
        assert_eq!(world(state.model(), 11), (10.0, 20.0));
        assert!(state.model().payload(frame).is_none());
        assert!(matches!(
            ungroup(&model, sid(13), &mut fresh),
            Err(GroupError::NotAGroup(_))
        ));
    }
}
