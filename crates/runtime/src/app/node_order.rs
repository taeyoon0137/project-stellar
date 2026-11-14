//! 같은 부모 안에서 선택한 레이어 하나의 저장 OrderKey를 바꿉니다.
use super::*;
use stellar_commands::{
    command::Command, id::CommandOrigin, node_ops::MoveNode, transaction::CommandTransaction,
};
use stellar_document::{model::SiblingOrder, records::NodePayload};

pub(super) const CAN_FORWARD: u32 = 1;
pub(super) const CAN_BACKWARD: u32 = 2;
pub(super) const FLOW_PARENT: u32 = 4;

struct Target {
    node: StableId,
    parent: StableId,
    flags: u32,
}

fn target(window: &ProjectWindowSession) -> Option<Target> {
    let [node] = window.view().selection().ids() else {
        return None;
    };
    let view = window.tabs().active_view()?;
    let model = window.document()?.model();
    let root = model
        .definition()
        .pages
        .iter()
        .flat_map(|p| &p.views)
        .find(|v| v.id == view)?
        .root_frame;
    let mut current = model.runtime_id(*node)?;
    let parent = model.stable_id(model.columns().parent(current)?)?;
    loop {
        let item = model.columns().get(current)?;
        if item.locked {
            return None;
        }
        if item.stable_id == root {
            break;
        }
        current = item.parent?;
    }
    let (back, front) = model.child_order_ends(parent)?;
    let mut flags = 0;
    if *node != front {
        flags |= CAN_FORWARD;
    }
    if *node != back {
        flags |= CAN_BACKWARD;
    }
    if matches!(model.payload(parent), Some(NodePayload::Frame(frame)) if frame.layout_mode.is_some())
    {
        flags |= FLOW_PARENT;
    }
    Some(Target {
        node: *node,
        parent,
        flags,
    })
}

pub(super) fn capabilities(window: &ProjectWindowSession) -> u32 {
    target(window).map_or(0, |t| t.flags)
}

pub(super) fn reorder(
    window: &mut ProjectWindowSession,
    request: RequestId,
    direction: SiblingOrder,
) -> Result<(), Failure> {
    let invalid = || {
        Failure::new(
            RequestFailure::UnknownView,
            "같은 부모 안에서 순서를 바꿀 수 있는 레이어 하나를 선택해야 합니다",
        )
    };
    let target = target(window).ok_or_else(invalid)?;
    let key = window
        .document()
        .ok_or_else(invalid)?
        .model()
        .reordered_child_key(target.parent, target.node, direction)
        .ok_or_else(invalid)?;
    let tx = CommandTransaction::new(
        super::frame_move::command_id(window.window(), request),
        CommandOrigin::User,
    )
    .with_command(
        window.revision(),
        Command::MoveNode(MoveNode {
            node: target.node,
            new_parent: target.parent,
            order_key: key,
        }),
    );
    window.apply(&tx).map_err(view_failure)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window_session::test_fixtures::{
        identity, sample_document, saved_project_with, sid, window,
    };
    use stellar_commands::selection::Selection;

    #[test]
    fn boundaries_locked_ancestors_root_other_view_and_multiselection_do_not_edit() {
        let mut model = sample_document();
        model.replace_flags(sid(12), true, true).unwrap();
        let dir = saved_project_with("locked-layer-order", &[&model]);
        let mut w = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        for ids in [vec![], vec![sid(10)], vec![sid(12)], vec![sid(11), sid(12)]] {
            w.select(Selection::new(ids, None)).unwrap();
            assert_eq!(capabilities(&w), 0);
            let history = w.history_sequence();
            let rev = w.revision();
            assert!(reorder(&mut w, RequestId(1), SiblingOrder::Front).is_err());
            assert_eq!((w.history_sequence(), w.revision()), (history, rev));
        }
        w.select(Selection::new([sid(11)], None)).unwrap();
        assert_eq!(capabilities(&w), CAN_FORWARD);
        assert!(reorder(&mut w, RequestId(2), SiblingOrder::Back).is_err());
        assert!(reorder(&mut w, RequestId(3), SiblingOrder::Front).is_ok());
        assert_eq!(capabilities(&w), CAN_BACKWARD);
        assert_eq!(w.view().selection().ids(), &[sid(11)]);
        w.undo().unwrap();
        assert_eq!(capabilities(&w), CAN_FORWARD);
        w.open_view(sid(4)).unwrap();
        w.select(Selection::new([sid(11)], None)).unwrap();
        assert_eq!(capabilities(&w), 0);
        let mut locked = sample_document();
        locked.replace_flags(sid(10), true, true).unwrap();
        let dir = saved_project_with("locked-order-parent", &[&locked]);
        let mut w = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        w.select(Selection::new([sid(11)], None)).unwrap();
        assert_eq!(capabilities(&w), 0);
    }
}
