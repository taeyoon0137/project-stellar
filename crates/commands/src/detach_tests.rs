//! 인스턴스 구조 command fixture (XII.10.34, ADR-0129 결정 4).
#![cfg(test)]

use super::*;
use crate::command::test_fixtures::{frame, sid, small_model};
use crate::command::Command;
use crate::request::CommandRequest;
use crate::state::DocumentState;
use stellar_core_types::clock::ManualClock;
use stellar_core_types::ids::StableIdGenerator;
use stellar_core_types::reference::{ComponentRef, NodeRef};
use stellar_document::component::definition::ComponentSlotId;
use stellar_document::component::definition::{
    ComponentDefinition, DefinitionNode, DefinitionTree,
};
use stellar_document::component::instance::{ComponentInstancePayload, SlotOverride};
use stellar_document::component::slot::{ComponentSlot, SlotCardinality};
use stellar_document::frame::FramePayload;
use stellar_document::test_support::semantic_snapshot::model_snapshot_json;
use stellar_document::text::TextPayload;

fn body() -> ComponentSlotId {
    ComponentSlotId::new(sid(940))
}

fn card_ref() -> ComponentRef {
    ComponentRef::new(sid(900))
}

/// Card 컴포넌트: root 901(100×80, 정의 캔버스 (2000, 0)) 아래 body 902(slot host, Frame·Text 허용)와 title 903(Text).
fn card_library() -> ComponentLibrary {
    let frame = |w: f32, h: f32, x: f32, y: f32| {
        encode_payload(&NodePayload::Frame(FramePayload {
            size: Size2::new(w, h).unwrap(),
            transform: Transform2D::new(1., 0., 0., 1., x, y).unwrap(),
            ..FramePayload::default()
        }))
    };
    let mut tree = DefinitionTree::new(card_ref());
    for (id, kind, parent, key, name, payload) in [
        (
            901,
            NodeKind::Frame,
            None,
            0x40,
            "card",
            frame(100., 80., 2000., 0.),
        ),
        (
            902,
            NodeKind::Frame,
            Some(901),
            0x40,
            "body",
            frame(80., 40., 10., 10.),
        ),
        (
            903,
            NodeKind::Text,
            Some(901),
            0x80,
            "title",
            encode_payload(&NodePayload::Text(TextPayload::empty(0))),
        ),
    ] {
        tree.insert(DefinitionNode {
            id: sid(id),
            kind,
            parent: parent.map(sid),
            order_key: OrderKey::from_bytes(vec![key]).unwrap(),
            visible: true,
            locked: false,
            display_name: name.into(),
            payload,
        })
        .unwrap();
    }
    let mut definition = ComponentDefinition::new(sid(900), "Card".into(), NodeRef::new(sid(901)));
    definition.add_slot(body()).unwrap();
    let mut library = ComponentLibrary::new();
    library.insert(definition, tree).unwrap();
    library
        .add_slot(
            card_ref(),
            ComponentSlot::new(
                body(),
                NodeRef::new(sid(902)),
                [NodeKind::Frame, NodeKind::Text],
                SlotCardinality::new(0, None).unwrap(),
                Vec::new(),
            ),
        )
        .unwrap();
    library
}

/// root(10) 아래 Card 인스턴스 30(배치 (200, 100), width 320px)과 그 body slot에 투영한 소유 Text 31.
fn modal_state() -> DocumentState {
    let mut model = small_model();
    let mut payload = ComponentInstancePayload::new(card_ref());
    payload.set_slot_override(body(), SlotOverride::new(vec![sid(31)]).unwrap());
    let mut instance = frame(30, Some(10), 0x30);
    instance.display_name = "확인 창".into();
    instance.payload = NodePayload::ComponentInstance(payload);
    model.insert_node(instance).unwrap();
    let mut text = frame(31, Some(30), 0x10);
    text.payload = NodePayload::Text(TextPayload::empty(0));
    model.insert_node(text).unwrap();
    let mut stack = ModifierStack::new(sid(30));
    stack
        .insert(stellar_document::modifier::ModifierEntry {
            id: stellar_document::modifier::ModifierId::new(sid(130)),
            order: OrderKey::between(None, None),
            kind: stellar_document::modifier::ModifierKind::Transform,
            enabled: true,
            payload: stellar_document::modifier::ModifierPayload::Transform(
                Transform2D::new(1., 0., 0., 1., 200., 100.).unwrap(),
            ),
        })
        .unwrap();
    model.set_modifier_stack(stack).unwrap();
    let mut layout = LayoutRecord::new(sid(30));
    layout
        .set(LayoutLonghand::Width, LayoutValue::Length(320.))
        .unwrap();
    model.set_layout_record(layout).unwrap();
    DocumentState::new(model)
}

fn plan(state: &DocumentState) -> InstanceRestructure {
    let library = card_library();
    plan_detach(
        state.model(),
        &library,
        sid(30),
        &mut StableIdGenerator::new(7),
        &ManualClock::new(1_000),
    )
    .unwrap()
}

#[test]
fn detach_replaces_the_instance_with_a_frame_copy_and_moves_slot_content() {
    let mut state = modal_state();
    let before = model_snapshot_json(state.model()).unwrap();
    let plan = plan(&state);
    // 인스턴스 id·부모·순서·이름은 그대로, 종류는 정의 root(Frame), 배치는 modifier가 계속 가집니다.
    assert_eq!(plan.record.id, sid(30));
    assert_eq!(plan.record.parent, Some(sid(10)));
    assert_eq!(plan.record.display_name, "확인 창");
    let NodePayload::Frame(root) = &plan.record.payload else {
        panic!("Frame");
    };
    let (_, _, _, _, e, f) = root.transform.components();
    assert_eq!((e, f), (0., 0.));
    // 명시 width 320px는 Frame payload 크기로 옮기고 layout record에서 지웁니다.
    assert_eq!(root.size.width(), 320.);
    assert!(plan.layout.get(LayoutLonghand::Width).is_none());
    // body·title 복제(새 id), body 복제 아래로 Text 31을 옮깁니다.
    assert_eq!(plan.insert.len(), 2);
    assert!(plan.insert.iter().all(|r| !state.model().contains(r.id)));
    let body_copy = plan
        .insert
        .iter()
        .find(|r| r.display_name == "body")
        .unwrap()
        .id;
    assert_eq!(
        plan.moves,
        vec![NodePlacement {
            node: sid(31),
            parent: body_copy,
            order_key: plan.moves[0].order_key.clone(),
        }]
    );
    assert!(plan.insert.iter().all(|r| r.parent == Some(sid(30))));
    // 같은 입력은 같은 계획입니다.
    assert_eq!(plan, super::tests::plan(&modal_state()));
    let outcome = state
        .execute(&CommandRequest::at(
            &state,
            Command::DetachInstance(Box::new(plan.clone())),
        ))
        .unwrap();
    let model = state.model();
    assert_eq!(model.record(sid(30)).unwrap().kind(), NodeKind::Frame);
    assert_eq!(model.record(sid(31)).unwrap().parent, Some(body_copy));
    assert!(model.modifier_stack(sid(30)).is_some());
    assert!(validate_document(&model.to_records().unwrap()).is_ok());
    // 역 command가 정확히 되돌립니다(같은 종류).
    assert!(matches!(outcome.inverse, Command::DetachInstance(_)));
    let redo = state
        .execute(&CommandRequest::at(&state, outcome.inverse))
        .unwrap();
    assert_eq!(model_snapshot_json(state.model()).unwrap(), before);
    state
        .execute(&CommandRequest::at(&state, redo.inverse))
        .unwrap();
    assert_eq!(
        state.model().record(sid(31)).unwrap().parent,
        Some(body_copy)
    );
    // codec 왕복.
    let command = Command::DetachInstance(Box::new(plan));
    assert_eq!(
        crate::codec::decode_command(&crate::codec::encode_command(&command)).unwrap(),
        command
    );
}

#[test]
fn plan_detach_refuses_non_instances_and_unresolved_definitions() {
    let state = modal_state();
    let library = card_library();
    let run = |library: &ComponentLibrary, node: u64| {
        plan_detach(
            state.model(),
            library,
            sid(node),
            &mut StableIdGenerator::new(1),
            &ManualClock::new(0),
        )
    };
    assert_eq!(run(&library, 11), Err(DetachPlanError::NotInstance));
    assert!(matches!(
        run(&ComponentLibrary::new(), 30),
        Err(DetachPlanError::Detach(_))
    ));
}

#[test]
fn restructure_refuses_unprojected_children_and_bad_shapes_without_changes() {
    let mut state = modal_state();
    let before = model_snapshot_json(state.model()).unwrap();
    let current = state.model().record(sid(30)).unwrap();
    let layout = state.model().layout_record(sid(30)).cloned().unwrap();
    let base = InstanceRestructure {
        node: sid(30),
        record: current.clone(),
        layout,
        insert: Vec::new(),
        insert_stacks: Vec::new(),
        insert_layouts: Vec::new(),
        moves: Vec::new(),
        remove: Vec::new(),
    };
    let run = |state: &mut DocumentState, command: InstanceRestructure| {
        state.execute(&CommandRequest::at(
            state,
            Command::PlaceSlotContent(Box::new(command)),
        ))
    };
    // 바꿀 것이 없음.
    assert!(run(&mut state, base.clone()).is_err());
    // 투영 없이 인스턴스 아래로 옮기면 결과 검사가 되돌립니다.
    let mut unprojected = base.clone();
    unprojected.moves.push(NodePlacement {
        node: sid(12),
        parent: sid(30),
        order_key: state.model().next_child_order_key(sid(30)),
    });
    assert!(matches!(
        run(&mut state, unprojected),
        Err(CommandError::InvalidResult(_))
    ));
    assert_eq!(model_snapshot_json(state.model()).unwrap(), before);
    // 부모·순서 변경, 자기 자신·조상으로 옮기기(cycle), 다른 부모의 node 지우기는 적용 전에 거부합니다.
    let mut moved_target = base.clone();
    moved_target.record.order_key = OrderKey::from_bytes(vec![0x99]).unwrap();
    assert!(run(&mut state, moved_target).is_err());
    let mut cycle = base.clone();
    cycle.moves.push(NodePlacement {
        node: sid(10),
        parent: sid(30),
        order_key: OrderKey::from_bytes(vec![0x50]).unwrap(),
    });
    assert!(run(&mut state, cycle).is_err());
    let mut foreign = base;
    foreign.remove.push(sid(13));
    assert!(run(&mut state, foreign).is_err());
    assert_eq!(model_snapshot_json(state.model()).unwrap(), before);
}
