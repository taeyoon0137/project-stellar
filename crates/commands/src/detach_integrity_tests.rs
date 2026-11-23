//! 인스턴스 구조 무결성 fixture (IX.3.3, ADR-0133 결정 1·2).
//!
//! - 구조 command(`delete-subtree`·`move-node`)는 Slot content를 slot override 없이 지우거나 꺼내지 않습니다.
//! - Slot content 지우기는 `place-slot-content` 하나이고 undo가 정확합니다.
//! - 인스턴스 subtree를 지운 뒤의 undo, 복제, 붙여넣기는 인스턴스 자식(투영된 Slot content)을 그대로 옮깁니다.
#![cfg(test)]

use super::tests::{card_library, modal_state};
use super::*;
use crate::codec::{decode_command, encode_command};
use crate::command::test_fixtures::{frame, sid};
use crate::command::Command;
use crate::duplicate::DuplicateSubtree;
use crate::node_ops::{DeleteSubtree, InsertNode, MoveNode};
use crate::paste::ClipboardSubtree;
use crate::request::CommandRequest;
use crate::state::DocumentState;
use stellar_core_types::clock::ManualClock;
use stellar_core_types::ids::StableIdGenerator;
use stellar_document::component::definition::ComponentSlotId;
use stellar_document::component::instance::ComponentInstancePayload;
use stellar_document::test_support::semantic_snapshot::model_snapshot_json;
use stellar_document::text::TextPayload;

fn body() -> ComponentSlotId {
    ComponentSlotId::new(sid(940))
}

fn run(state: &mut DocumentState, command: Command) -> Result<Command, CommandError> {
    state
        .execute(&CommandRequest::at(state, command))
        .map(|outcome| outcome.inverse)
}

fn valid(state: &DocumentState) -> bool {
    validate_document(&state.model().to_records().unwrap()).is_ok()
}

fn projected(state: &DocumentState, instance: StableId) -> Vec<StableId> {
    let Some(NodePayload::ComponentInstance(payload)) = state.model().payload(instance) else {
        panic!("인스턴스가 아닙니다");
    };
    payload
        .slot_overrides()
        .flat_map(|(_, content)| content.content().to_vec())
        .collect()
}

#[test]
fn structure_commands_refuse_to_drop_slot_content_out_of_its_projection() {
    let mut state = modal_state();
    let before = model_snapshot_json(state.model()).unwrap();
    assert_eq!(slot_content_owner(state.model(), sid(31)), Some(sid(30)));
    assert_eq!(slot_content_owner(state.model(), sid(11)), None);
    // Slot content를 root로 지우거나 인스턴스 밖으로 옮기면 투영이 자식이 아닌 node를 가리키게 됩니다.
    assert!(run(
        &mut state,
        Command::DeleteSubtree(DeleteSubtree { root: sid(31) })
    )
    .is_err());
    assert!(run(
        &mut state,
        Command::MoveNode(MoveNode {
            node: sid(31),
            new_parent: sid(10),
            order_key: OrderKey::from_bytes(vec![0x90]).unwrap(),
        })
    )
    .is_err());
    // 인스턴스 아래로 투영 없이 옮기거나 넣는 것도 거부합니다(자유 자식 판정).
    assert!(run(
        &mut state,
        Command::MoveNode(MoveNode {
            node: sid(12),
            new_parent: sid(30),
            order_key: OrderKey::from_bytes(vec![0x90]).unwrap(),
        })
    )
    .is_err());
    assert!(run(
        &mut state,
        Command::InsertNode(InsertNode::single(frame(50, Some(30), 0x90)))
    )
    .is_err());
    assert_eq!(model_snapshot_json(state.model()).unwrap(), before);
    // 같은 인스턴스 안 순서 key 변경은 투영 순서와 무관해 받습니다.
    let undo = run(
        &mut state,
        Command::MoveNode(MoveNode {
            node: sid(31),
            new_parent: sid(30),
            order_key: OrderKey::from_bytes(vec![0x70]).unwrap(),
        }),
    )
    .unwrap();
    assert!(valid(&state));
    run(&mut state, undo).unwrap();
    assert_eq!(model_snapshot_json(state.model()).unwrap(), before);
}

#[test]
fn slot_content_delete_is_one_restructure_and_undoes_exactly() {
    let library = card_library();
    for restore_default in [false, true] {
        let mut state = modal_state();
        let mut grandchild = frame(32, Some(31), 0x10);
        grandchild.display_name = "안쪽".into();
        // Text 31 아래에는 자식을 둘 수 없어 Frame content로 바꿔 subtree 삭제를 봅니다.
        let mut model = state.into_model();
        let mut content = model.remove_leaf(sid(31)).unwrap();
        content.payload = NodePayload::Frame(Default::default());
        model.insert_node(content).unwrap();
        model.insert_node(grandchild).unwrap();
        state = DocumentState::new(model);
        let before = model_snapshot_json(state.model()).unwrap();

        let plan =
            plan_slot_content_delete(state.model(), &library, sid(31), restore_default).unwrap();
        assert_eq!(plan.remove, vec![sid(32), sid(31)], "자식부터 지웁니다");
        let command = Command::PlaceSlotContent(Box::new(plan));
        assert_eq!(
            decode_command(&encode_command(&command)).unwrap(),
            command,
            "codec 왕복"
        );
        let undo = run(&mut state, command).unwrap();
        assert!(!state.model().contains(sid(31)));
        assert!(!state.model().contains(sid(32)));
        assert!(valid(&state));
        let Some(NodePayload::ComponentInstance(payload)) = state.model().payload(sid(30)) else {
            panic!("인스턴스");
        };
        // 마지막 content: default 복귀면 override가 없고, 아니면 빈 override로 Slot을 비웁니다.
        assert_eq!(
            payload.slot_override(body()).map(|o| o.content().len()),
            (!restore_default).then_some(0)
        );
        // undo 한 번으로 content subtree와 투영이 돌아옵니다(역 command도 codec 왕복).
        assert_eq!(decode_command(&encode_command(&undo)).unwrap(), undo);
        run(&mut state, undo).unwrap();
        assert_eq!(model_snapshot_json(state.model()).unwrap(), before);
    }
}

#[test]
fn slot_content_delete_refuses_non_content_locks_and_missing_definitions() {
    let library = card_library();
    let state = modal_state();
    assert_eq!(
        plan_slot_content_delete(state.model(), &library, sid(11), false),
        Err(SlotContentDeleteError::NotSlotContent)
    );
    assert_eq!(
        plan_slot_content_delete(state.model(), &ComponentLibrary::new(), sid(31), false),
        Err(SlotContentDeleteError::UnresolvedComponent)
    );
    let mut model = modal_state().into_model();
    model.replace_flags(sid(30), true, true).unwrap();
    assert_eq!(
        plan_slot_content_delete(&model, &library, sid(31), false),
        Err(SlotContentDeleteError::Locked)
    );
}

#[test]
fn instance_subtree_delete_duplicate_and_paste_keep_projected_children() {
    let mut state = modal_state();
    let before = model_snapshot_json(state.model()).unwrap();
    // 인스턴스를 지운 뒤 undo: 역 InsertNode가 인스턴스와 그 Slot content를 함께 되살립니다.
    let undo = run(
        &mut state,
        Command::DeleteSubtree(DeleteSubtree { root: sid(30) }),
    )
    .unwrap();
    let undo = decode_command(&encode_command(&undo)).unwrap();
    run(&mut state, undo).unwrap();
    assert_eq!(model_snapshot_json(state.model()).unwrap(), before);

    // 복제: 복제 인스턴스의 slot override가 복제 content를 가리킵니다.
    let duplicate = DuplicateSubtree::plan(
        state.model(),
        sid(30),
        sid(10),
        OrderKey::from_bytes(vec![0x90]).unwrap(),
        &mut StableIdGenerator::new(5),
        &ManualClock::new(10),
    )
    .unwrap();
    let copy = duplicate.copy_root().unwrap();
    let undo = run(&mut state, Command::DuplicateSubtree(duplicate)).unwrap();
    let children = state.model().children_in_order(copy);
    assert_eq!(children.len(), 1);
    assert_ne!(children[0], sid(31));
    assert_eq!(projected(&state, copy), children);
    assert!(valid(&state));
    run(&mut state, undo).unwrap();
    assert_eq!(model_snapshot_json(state.model()).unwrap(), before);

    // 붙여넣기: 클립보드 bytes 왕복 뒤 새 id로 같은 구조를 넣습니다.
    let clipboard = ClipboardSubtree::decode(
        &ClipboardSubtree::capture(state.model(), sid(30))
            .unwrap()
            .encode(),
    )
    .unwrap();
    let insert = clipboard
        .plan(
            state.model(),
            sid(12),
            OrderKey::from_bytes(vec![0x10]).unwrap(),
            &mut StableIdGenerator::new(9),
            &ManualClock::new(20),
        )
        .unwrap();
    let pasted = insert.nodes[0].id;
    run(&mut state, Command::InsertNode(insert)).unwrap();
    assert_eq!(
        projected(&state, pasted),
        state.model().children_in_order(pasted)
    );
    assert!(valid(&state));
}

#[test]
fn insert_rejects_instance_children_that_are_not_projected_exactly_once() {
    let mut state = modal_state();
    let mut payload = ComponentInstancePayload::new(card_ref_of(&state));
    payload.set_slot_override(body(), SlotOverride::new(vec![sid(61)]).unwrap());
    let mut instance = frame(60, Some(10), 0x90);
    instance.payload = NodePayload::ComponentInstance(payload);
    let mut child = frame(61, Some(60), 0x10);
    child.payload = NodePayload::Text(TextPayload::empty(0));
    let stray = frame(62, Some(60), 0x20);
    let insert = |nodes: Vec<NodeRecord>| {
        Command::InsertNode(InsertNode {
            nodes,
            modifier_stacks: Vec::new(),
            layout_records: Vec::new(),
        })
    };
    // 투영되지 않은 자식(62)이 있으면 거부합니다.
    assert!(run(
        &mut state,
        insert(vec![instance.clone(), child.clone(), stray])
    )
    .is_err());
    // 투영된 자식만이면 받습니다.
    run(&mut state, insert(vec![instance, child])).unwrap();
    assert!(valid(&state));
}

fn card_ref_of(state: &DocumentState) -> stellar_core_types::reference::ComponentRef {
    let Some(NodePayload::ComponentInstance(payload)) = state.model().payload(sid(30)) else {
        panic!("인스턴스");
    };
    payload.component()
}
