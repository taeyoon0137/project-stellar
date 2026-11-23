//! ComponentInstance 캔버스 편집 fixture (XII.10.34).
#![cfg(test)]

use super::*;
use crate::canvas::select_fixtures::{nested, nested_library, record};
use crate::window_session::test_fixtures::window;
use stellar_commands::id::{CommandId, CommandOrigin};
use stellar_commands::state::DocumentState;
use stellar_commands::transaction::CommandTransaction;
use stellar_document::component::definition::ComponentSlotId;
use stellar_document::component::instance::SlotOverride;
use stellar_document::component::property::PropertyVisibility;
use stellar_document::component::property_commands::PropertyCommand;
use stellar_document::test_support::component_fixtures::{
    self as fx, button_label, button_ref, component_library_fixture, modal_ref, MODAL_CLOSE_BUTTON,
};
use stellar_document::test_support::semantic_snapshot::model_snapshot_json;
use stellar_history::window::WindowHistory;

fn v(x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y).unwrap()
}

#[test]
fn deep_pick_returns_a_read_only_outline_of_the_topmost_definition_node() {
    let fixture = nested();
    let library = nested_library();
    let scene = CanvasScene::with_library(&fixture.model, fixture.root, Some(&library), None, None);
    // 인스턴스 21은 (500,300), 정의 root 901 100×80 아래 902 (10,10) 40×40.
    let child = pick_inside(&fixture.model, &library, &scene, sid(21), v(520., 320.)).unwrap();
    assert_eq!(child.node, sid(902));
    assert_eq!(child.path, vec![sid(901), sid(902)]);
    assert_eq!(
        child.outline,
        [v(510., 310.), v(550., 310.), v(550., 350.), v(510., 350.)]
    );
    let root = pick_inside(&fixture.model, &library, &scene, sid(21), v(590., 370.)).unwrap();
    assert_eq!(root.node, sid(901));
    assert_eq!(root.outline[2], v(600., 380.));
    // 인스턴스 밖, 인스턴스가 아닌 node는 없음.
    assert_eq!(
        pick_inside(&fixture.model, &library, &scene, sid(21), v(10., 10.)),
        None
    );
    assert_eq!(
        pick_inside(&fixture.model, &library, &scene, sid(11), v(150., 150.)),
        None
    );
    // 깊은 선택은 문서·선택을 바꾸지 않는 값입니다.
    assert!(!fixture.model.contains(sid(902)));
}

fn sid(n: u64) -> StableId {
    crate::window_session::test_fixtures::sid(n)
}

/// root 10 아래 Button 인스턴스 30(label override 없음)과 Modal 인스턴스 31(소유 Text 32를 body slot에 투영).
fn rig() -> (ComponentLibrary, DocumentState, WindowHistory) {
    let library = component_library_fixture().unwrap().library;
    let mut model = crate::window_session::test_fixtures::sample_document();
    let instance = |n: u64, key: u8, payload: ComponentInstancePayload| {
        record(n, 10, key, NodePayload::ComponentInstance(payload))
    };
    model
        .insert_node(instance(
            30,
            0x30,
            ComponentInstancePayload::new(button_ref()),
        ))
        .unwrap();
    let mut modal = ComponentInstancePayload::new(modal_ref());
    modal.set_slot_override(body(), SlotOverride::new(vec![sid(32)]).unwrap());
    model.insert_node(instance(31, 0x40, modal)).unwrap();
    model
        .insert_node(record(
            32,
            31,
            0x10,
            NodePayload::Text(stellar_document::text::TextPayload::empty(0)),
        ))
        .unwrap();
    (
        library,
        DocumentState::new(model),
        WindowHistory::new(window(1)),
    )
}

fn body() -> ComponentSlotId {
    ComponentSlotId::new(fx::sid(1040))
}

/// Button label Text(1012)의 본문 속성이 label Property를 씁니다.
const CONTENT: PropertyKey = PropertyKey(0x0401);

fn links() -> Vec<PropertyLink> {
    vec![PropertyLink {
        node: fx::sid(1012),
        key: CONTENT,
        property: button_label(),
    }]
}

fn record_one(history: &mut WindowHistory, doc: &mut DocumentState, commands: Vec<Command>) {
    let revision = doc.revision();
    let tx = commands.into_iter().fold(
        CommandTransaction::new(CommandId::from_stable_id(sid(8_000)), CommandOrigin::User),
        |tx, c| tx.with_command(revision, c),
    );
    history.record_document(doc, &tx, None).unwrap();
}

fn label(doc: &DocumentState, instance: u64) -> Option<TypedValue> {
    payload_of(doc.model(), sid(instance))
        .and_then(|p| p.property_override(&OverrideKey::direct(button_label())))
        .cloned()
}

#[test]
fn linked_public_value_becomes_an_override_and_resets_undo_once_each() {
    let (library, mut doc, mut history) = rig();
    let before = model_snapshot_json(doc.model()).unwrap();
    let value = TypedValue::String("저장".into());
    let set = plan_inner_edit(
        doc.model(),
        &library,
        &links(),
        sid(30),
        fx::sid(1012),
        CONTENT,
        value.clone(),
    )
    .unwrap();
    record_one(&mut history, &mut doc, vec![set]);
    assert_eq!(label(&doc, 30), Some(value));
    assert_eq!(history.history().entries().len(), 1);
    let actions = instance_actions(doc.model(), &library, sid(30)).unwrap();
    assert_eq!(actions.reset_properties, vec![button_label()]);
    assert!(actions.reset_all && actions.detach);
    assert_eq!(actions.definition.unwrap().0, button_ref());
    // 속성별 Reset.
    let reset = plan_reset(doc.model(), &library, sid(30), button_label()).unwrap();
    record_one(&mut history, &mut doc, vec![reset]);
    assert_eq!(label(&doc, 30), None);
    assert_eq!(model_snapshot_json(doc.model()).unwrap(), before);
    assert_eq!(
        plan_reset(doc.model(), &library, sid(30), button_label())
            .unwrap_err()
            .reason,
        InstanceEditRefusal::NotOverridden
    );
    // 전체 Reset은 slot content를 남깁니다.
    history.undo(&mut doc).unwrap();
    assert!(label(&doc, 30).is_some());
    let all = plan_reset_all(doc.model(), &library, sid(30)).unwrap();
    record_one(&mut history, &mut doc, vec![all]);
    assert_eq!(model_snapshot_json(doc.model()).unwrap(), before);
    history.undo(&mut doc).unwrap();
    history.undo(&mut doc).unwrap();
    assert_eq!(model_snapshot_json(doc.model()).unwrap(), before);
}

#[test]
fn unlinked_private_nested_and_locked_edits_are_refused_with_detach_path() {
    let (mut library, doc, _) = rig();
    let model = doc.model();
    let edit = |library: &ComponentLibrary, model: &DocumentModel, instance, node, key, value| {
        plan_inner_edit(model, library, &links(), sid(instance), node, key, value)
    };
    let text = TypedValue::String("x".into());
    // 연결 없는 속성.
    let refused = edit(
        &library,
        model,
        30,
        fx::sid(1012),
        PropertyKey(0x0402),
        text.clone(),
    )
    .unwrap_err();
    assert_eq!(refused.reason, InstanceEditRefusal::NotLinked);
    assert!(refused.detach);
    assert!(refused.to_string().contains("Detach"));
    // 값 타입 불일치(IX.2.3 규칙).
    let refused = edit(
        &library,
        model,
        30,
        fx::sid(1012),
        CONTENT,
        TypedValue::Boolean(true),
    )
    .unwrap_err();
    assert!(matches!(
        refused.reason,
        InstanceEditRefusal::Rule(PropertyOverrideError::ValueTypeMismatch { .. })
    ));
    // nested instance(Modal의 닫기 Button) 직접 편집.
    let refused = edit(
        &library,
        model,
        31,
        fx::sid(MODAL_CLOSE_BUTTON),
        CONTENT,
        text.clone(),
    )
    .unwrap_err();
    assert_eq!(refused.reason, InstanceEditRefusal::NestedInstance);
    // 비공개 Property.
    let entry = library.get(button_ref()).unwrap();
    let private = entry
        .property(button_label())
        .unwrap()
        .with_visibility(PropertyVisibility::Private);
    let mut drilling = component_library_fixture().unwrap().drilling;
    PropertyCommand::RestoreRecord {
        component: button_ref(),
        property: private,
        revive: Vec::new(),
    }
    .apply(&mut library, &mut drilling)
    .unwrap();
    let refused = edit(&library, model, 30, fx::sid(1012), CONTENT, text.clone()).unwrap_err();
    assert!(matches!(
        refused.reason,
        InstanceEditRefusal::Rule(PropertyOverrideError::PrivateProperty { .. })
    ));
    // 잠긴 인스턴스: 편집·Detach 모두 불가.
    let mut locked = rig().1.into_model();
    let mut node = locked.record(sid(30)).unwrap();
    node.locked = true;
    locked.remove_leaf(sid(30)).unwrap();
    locked.insert_node(node).unwrap();
    let refused = edit(&library, &locked, 30, fx::sid(1012), CONTENT, text).unwrap_err();
    assert_eq!(
        refused,
        Refused {
            reason: InstanceEditRefusal::Locked,
            detach: false
        }
    );
    let actions = instance_actions(&locked, &library, sid(30)).unwrap();
    assert!(!actions.detach && !actions.reset_all);
}

fn body_content(doc: &DocumentState) -> Option<Vec<StableId>> {
    payload_of(doc.model(), sid(31))
        .and_then(|p| p.slot_override(body()))
        .map(|c| c.content().to_vec())
}

fn valid(doc: &DocumentState) -> bool {
    stellar_document::validate::validate_document(&doc.model().to_records().unwrap()).is_ok()
}

/// 공개 Slot 끌어 놓기·꺼내기는 동작마다 `place-slot-content` 한 건이고 undo가 정확합니다(ADR-0129 결정 3·4).
#[test]
fn slot_drop_and_take_out_are_one_entry_each_and_undo_exactly() {
    let (library, mut doc, mut history) = rig();
    let before = model_snapshot_json(doc.model()).unwrap();
    // Frame 11을 Modal 31 body slot 맨 앞에 넣습니다.
    let drop = plan_slot_drop(
        doc.model(),
        &library,
        sid(31),
        body(),
        0,
        SlotContent::Existing(sid(11)),
    )
    .unwrap();
    assert_eq!(drop.kind().name(), "place-slot-content");
    record_one(&mut history, &mut doc, vec![drop]);
    assert_eq!(history.history().entries().len(), 1);
    assert_eq!(body_content(&doc), Some(vec![sid(11), sid(32)]));
    assert_eq!(doc.model().record(sid(11)).unwrap().parent, Some(sid(31)));
    assert!(valid(&doc));
    // 같은 인스턴스 안 순서 바꾸기: 11을 끝으로(부모는 그대로).
    let reorder = plan_slot_drop(
        doc.model(),
        &library,
        sid(31),
        body(),
        9,
        SlotContent::Existing(sid(11)),
    )
    .unwrap();
    record_one(&mut history, &mut doc, vec![reorder]);
    assert_eq!(body_content(&doc), Some(vec![sid(32), sid(11)]));
    // 꺼내기: 11을 root 10 아래로, Slot은 Text 32만 남습니다.
    let out = plan_slot_take_out(
        doc.model(),
        &library,
        sid(31),
        sid(11),
        sid(10),
        OrderKey::from_bytes(vec![0x05]).unwrap(),
        false,
    )
    .unwrap();
    record_one(&mut history, &mut doc, vec![out]);
    assert_eq!(body_content(&doc), Some(vec![sid(32)]));
    assert_eq!(doc.model().record(sid(11)).unwrap().parent, Some(sid(10)));
    assert!(valid(&doc));
    // 마지막 content를 restore_default로 꺼내면 override가 없어져 default content로 돌아갑니다.
    let last = plan_slot_take_out(
        doc.model(),
        &library,
        sid(31),
        sid(32),
        sid(10),
        OrderKey::from_bytes(vec![0x06]).unwrap(),
        true,
    )
    .unwrap();
    record_one(&mut history, &mut doc, vec![last]);
    assert_eq!(body_content(&doc), None);
    assert_eq!(history.history().entries().len(), 4);
    for _ in 0..4 {
        history.undo(&mut doc).unwrap();
    }
    assert_eq!(model_snapshot_json(doc.model()).unwrap(), before);
}

#[test]
fn slot_drop_refuses_rule_violations_without_changing_the_document() {
    use stellar_document::shape::{RectangleParams, ShapeGeometry};
    let (library, doc, _) = rig();
    let model = doc.model();
    let drop = |instance: u64, slot: ComponentSlotId, content: SlotContent| {
        plan_slot_drop(model, &library, sid(instance), slot, 0, content)
            .unwrap_err()
            .reason
    };
    // 인스턴스의 조상(View root)은 넣을 수 없습니다.
    assert_eq!(
        drop(31, body(), SlotContent::Existing(sid(10))),
        InstanceEditRefusal::ContentNotMovable
    );
    // 컴포넌트에 없는 Slot.
    assert_eq!(
        drop(
            31,
            ComponentSlotId::new(sid(4_444)),
            SlotContent::Existing(sid(11))
        ),
        InstanceEditRefusal::UnknownSlot
    );
    // body는 Frame·Text만 받습니다(IX.3.1 종류 규칙).
    let rectangle = record(
        60,
        31,
        0x60,
        NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
            size: stellar_core_types::value::Size2::new(4., 4.).unwrap(),
            corner_radius: Default::default(),
        })),
    );
    assert!(matches!(
        drop(
            31,
            body(),
            SlotContent::New {
                nodes: vec![rectangle],
                modifier_stacks: Vec::new(),
                layout_records: Vec::new(),
            }
        ),
        InstanceEditRefusal::SlotRule(SlotError::DisallowedKind { .. })
    ));
    // 새 content의 root 부모는 인스턴스여야 합니다.
    assert_eq!(
        drop(
            31,
            body(),
            SlotContent::New {
                nodes: vec![record(
                    61,
                    10,
                    0x61,
                    NodePayload::Text(stellar_document::text::TextPayload::empty(0))
                )],
                modifier_stacks: Vec::new(),
                layout_records: Vec::new(),
            }
        ),
        InstanceEditRefusal::ContentNotMovable
    );
    // Slot content가 아닌 node는 꺼낼 수 없습니다.
    assert_eq!(
        plan_slot_take_out(
            model,
            &library,
            sid(31),
            sid(11),
            sid(10),
            OrderKey::from_bytes(vec![0x05]).unwrap(),
            false
        )
        .unwrap_err()
        .reason,
        InstanceEditRefusal::ContentNotMovable
    );
}

/// Detach 실행은 `detach-instance` 한 건이고, Undo 한 번이 인스턴스와 Slot content 자리를 되돌립니다.
#[test]
fn detach_instance_is_one_entry_and_undoes_once() {
    use stellar_core_types::clock::ManualClock;
    use stellar_core_types::reference::NodeRef;
    use stellar_document::component::definition::{
        ComponentDefinition, DefinitionNode, DefinitionTree,
    };
    use stellar_document::component::slot::{ComponentSlot, SlotCardinality};
    use stellar_document::frame::FramePayload;
    use stellar_document::record_codec::encode_payload;
    let card = ComponentRef::new(sid(900));
    let slot = ComponentSlotId::new(sid(940));
    let mut tree = DefinitionTree::new(card);
    for (id, parent, key, name) in [(901, None, 0x40, "card"), (902, Some(901), 0x40, "body")] {
        tree.insert(DefinitionNode {
            id: sid(id),
            kind: NodeKind::Frame,
            parent: parent.map(sid),
            order_key: OrderKey::from_bytes(vec![key]).unwrap(),
            visible: true,
            locked: false,
            display_name: name.into(),
            payload: encode_payload(&NodePayload::Frame(FramePayload::default())),
        })
        .unwrap();
    }
    let mut definition = ComponentDefinition::new(sid(900), "Card".into(), NodeRef::new(sid(901)));
    definition.add_slot(slot).unwrap();
    let mut library = ComponentLibrary::new();
    library.insert(definition, tree).unwrap();
    library
        .add_slot(
            card,
            ComponentSlot::new(
                slot,
                NodeRef::new(sid(902)),
                [NodeKind::Frame],
                SlotCardinality::new(0, None).unwrap(),
                Vec::new(),
            ),
        )
        .unwrap();
    let mut model = crate::window_session::test_fixtures::sample_document();
    let mut payload = ComponentInstancePayload::new(card);
    payload.set_slot_override(slot, SlotOverride::new(vec![sid(51)]).unwrap());
    model
        .insert_node(record(
            50,
            10,
            0x50,
            NodePayload::ComponentInstance(payload),
        ))
        .unwrap();
    model
        .insert_node(record(
            51,
            50,
            0x10,
            NodePayload::Frame(FramePayload::default()),
        ))
        .unwrap();
    let mut doc = DocumentState::new(model);
    let mut history = WindowHistory::new(window(1));
    let before = model_snapshot_json(doc.model()).unwrap();
    let actions = instance_actions(doc.model(), &library, sid(50)).unwrap();
    assert!(actions.enabled(InstanceAction::DetachInstance));
    let detach = plan_detach_instance(
        doc.model(),
        &library,
        sid(50),
        &mut StableIdGenerator::new(3),
        &ManualClock::new(10),
    )
    .unwrap();
    assert_eq!(detach.kind().name(), "detach-instance");
    record_one(&mut history, &mut doc, vec![detach]);
    assert_eq!(history.history().entries().len(), 1);
    assert_eq!(doc.model().record(sid(50)).unwrap().kind(), NodeKind::Frame);
    let host = doc.model().record(sid(51)).unwrap().parent.unwrap();
    assert_eq!(doc.model().record(host).unwrap().display_name, "body");
    assert!(valid(&doc));
    history.undo(&mut doc).unwrap();
    assert_eq!(model_snapshot_json(doc.model()).unwrap(), before);
    // 미해결 정의는 Detach를 거부합니다.
    assert!(plan_detach_instance(
        doc.model(),
        &ComponentLibrary::new(),
        sid(50),
        &mut StableIdGenerator::new(3),
        &ManualClock::new(10),
    )
    .is_err());
}

/// 인스턴스 편집 Object 동작의 코어 command ID는 겹치지 않고 이름으로 되찾을 수 있습니다.
#[test]
fn instance_action_command_ids_are_unique_and_follow_availability() {
    let names: std::collections::BTreeSet<&str> =
        InstanceAction::ALL.iter().map(|a| a.name()).collect();
    assert_eq!(names.len(), InstanceAction::ALL.len());
    for action in InstanceAction::ALL {
        assert!(action.name().starts_with("object."));
        assert_eq!(InstanceAction::from_name(action.name()), Some(action));
    }
    assert_eq!(InstanceAction::from_name("object.unknown"), None);
    let (library, doc, _) = rig();
    // override 없는 Button: Reset 불가, Detach·정의로 이동 가능.
    let actions = instance_actions(doc.model(), &library, sid(30)).unwrap();
    assert!(!actions.enabled(InstanceAction::ResetOverrides));
    assert!(!actions.enabled(InstanceAction::ResetOverride));
    assert!(actions.enabled(InstanceAction::DetachInstance));
    assert!(actions.enabled(InstanceAction::GoToDefinition));
}
