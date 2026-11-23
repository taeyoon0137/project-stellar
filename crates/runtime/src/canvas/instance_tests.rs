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
