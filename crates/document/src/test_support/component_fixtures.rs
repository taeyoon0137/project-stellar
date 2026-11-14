//! Button·TextInput·Modal·List·Radio 합성 예제 component library fixture (IX.9.1, 테스트 전용).
//!
//! 다섯 컴포넌트를 control contract(IX.6)·상태 머신(IX.5)·Property(IX.1)·Slot(IX.3)·drilling(IX.4)으로 합성합니다:
//!
//! - **Button**: root Frame(Button 계약: activate = `activated`, disabled = Property) ← label Text. Property
//!   `label`(String)·`disabled`(Boolean). Activate → emit `activated`.
//! - **TextInput**: root Frame(TextInput 계약: value = 상태 `text`, placeholder = Property) ← value Text. Input →
//!   `text = payload`.
//! - **Modal**: root Frame(Modal 계약: focus scope = content, overlay, dismiss = `dismissed`, open = 상태 `open`) ←
//!   overlay Frame, content Frame ← body slot host Frame, 닫기 Button **인스턴스**. Property `closeLabel`은 nested
//!   Button의 `label`로 drilling(link)됩니다. nested Button의 `activated` → `open = false`, emit `dismissed`.
//! - **List**: root Frame(List 계약: collection = items, Single, selection = 상태 `selection`) ← items Frame ← item
//!   Frame 3개(각 Text). SelectionChange → `selection = payload`.
//! - **Radio**: 별도 RadioButton node 없이 List + Button 합성(IX.6.6). root Frame(SingleSelectionGroup 계약: list,
//!   selected = 상태 `selected`) ← list Frame(List 계약, Single) ← options Frame ← option Frame 3개(각 Button 계약:
//!   activate = `chosen`, Text 자식).
//!
//! scene: root Frame 아래 각 컴포넌트 인스턴스 하나씩. Button 인스턴스는 `label`을 override하고 Modal 인스턴스는
//! 소유 Text를 body slot에 투영합니다.
//!
//! [`library_snapshot`]은 library 구성 요약과 canonical bytes checksum(FNV-1a 64)을 결정론적 JSON으로 출력합니다.
//! 저장 fixture `fixtures/component_library.json`과 byte 단위로 비교합니다(IX.9.4 플랫폼 간 비교의 입력).

use std::collections::BTreeMap;
use std::fmt::Debug;

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::{ComponentRef, NodeRef};
use stellar_core_types::typed_value::TypedValue;
use stellar_expressions::ast::{ExprNode, ExprNodeId, ExpressionAst, ReferenceTarget};
use stellar_expressions::types::{ExpressionType, TypeSpec};

use super::json::Json;
use crate::component::definition::{
    ComponentDefinition, ComponentLibrary, ComponentPropertyId, ComponentSlotId, DefinitionNode,
    DefinitionTree,
};
use crate::component::drilling::{DrillingBinding, DrillingBindingId, DrillingSet};
use crate::component::event::EventKind;
use crate::component::expansion::ScenePayloads;
use crate::component::instance::{ComponentInstancePayload, OverrideKey, SlotOverride};
use crate::component::property::{ComponentProperty, PropertyVisibility};
use crate::component::slot::{ComponentSlot, SlotCardinality};
use crate::component::state_machine::{
    event_payload_path, ActionSpec, PropertyScope, SemanticAction, SemanticActionId, StateMachine,
    StateVariable, StateVariableId, TransitionId, TransitionSpec, Trigger,
};
use crate::control::list::SelectionMode;
use crate::control::registry::descriptor;
use crate::control::{
    button, list, modal, selection_group, text_input, ContractValue, ControlContract,
    ControlContractTable, InteractionContract, SemanticContract,
};
use crate::node_columns::{NodeColumns, NodeFields};
use crate::node_kind::NodeKind;

/// fixture StableId.
pub fn sid(n: u64) -> StableId {
    StableId::new_v7(1_700_000_910_000 + n, 0x0f91, 0x0091_0000_0000 ^ n)
}

/// Button 컴포넌트.
pub fn button_ref() -> ComponentRef {
    ComponentRef::new(sid(101))
}

/// TextInput 컴포넌트.
pub fn text_input_ref() -> ComponentRef {
    ComponentRef::new(sid(102))
}

/// Modal 컴포넌트.
pub fn modal_ref() -> ComponentRef {
    ComponentRef::new(sid(103))
}

/// List 컴포넌트.
pub fn list_ref() -> ComponentRef {
    ComponentRef::new(sid(104))
}

/// Radio 컴포넌트.
pub fn radio_ref() -> ComponentRef {
    ComponentRef::new(sid(105))
}

/// 다섯 컴포넌트(이름 순서 고정).
pub fn components() -> [(&'static str, ComponentRef); 5] {
    [
        ("Button", button_ref()),
        ("TextInput", text_input_ref()),
        ("Modal", modal_ref()),
        ("List", list_ref()),
        ("Radio", radio_ref()),
    ]
}

/// Button.label property.
pub fn button_label() -> ComponentPropertyId {
    ComponentPropertyId::new(sid(1013))
}

/// Button.activated action.
pub fn button_activated() -> SemanticActionId {
    SemanticActionId::new(sid(1015))
}

/// Modal의 nested 닫기 Button 인스턴스.
pub const MODAL_CLOSE_BUTTON: u64 = 1035;

/// scene 인스턴스(컴포넌트 순서): 9001~9005, 그리고 Modal slot content Text 9006.
pub const SCENE_ROOT: u64 = 9000;

/// 합성 fixture.
#[derive(Debug, Clone)]
pub struct ComponentLibraryFixture {
    /// 컴포넌트 library.
    pub library: ComponentLibrary,
    /// 컴포넌트별 control contract 표.
    pub contracts: BTreeMap<ComponentRef, ControlContractTable>,
    /// drilling binding.
    pub drilling: DrillingSet,
}

fn check<T, E: Debug>(result: Result<T, E>, what: &str) -> Result<T, String> {
    result.map_err(|err| format!("{what}: {err:?}"))
}

fn key(i: u8) -> Result<OrderKey, String> {
    check(OrderKey::from_bytes(vec![0x40 + i * 0x10]), "order key")
}

struct TreeBuilder {
    tree: DefinitionTree,
    children: BTreeMap<Option<u64>, u8>,
}

impl TreeBuilder {
    fn new(component: ComponentRef) -> TreeBuilder {
        TreeBuilder {
            tree: DefinitionTree::new(component),
            children: BTreeMap::new(),
        }
    }

    fn add(
        &mut self,
        id: u64,
        kind: NodeKind,
        parent: Option<u64>,
        name: &str,
        payload: Vec<u8>,
    ) -> Result<(), String> {
        let index = self.children.entry(parent).or_insert(0);
        let order_key = key(*index)?;
        *index += 1;
        check(
            self.tree.insert(DefinitionNode {
                id: sid(id),
                kind,
                parent: parent.map(sid),
                order_key,
                visible: true,
                locked: false,
                display_name: name.to_string(),
                payload,
            }),
            name,
        )
    }

    fn frame(&mut self, id: u64, parent: Option<u64>, name: &str) -> Result<(), String> {
        self.add(id, NodeKind::Frame, parent, name, Vec::new())
    }

    fn text(&mut self, id: u64, parent: u64, name: &str) -> Result<(), String> {
        self.add(id, NodeKind::Text, Some(parent), name, Vec::new())
    }
}

fn literal(id: u64, value: TypedValue) -> Result<ExpressionAst, String> {
    let node = ExprNodeId::new(sid(id));
    check(
        ExpressionAst::new(node, vec![ExprNode::literal(node, value)]),
        "literal AST",
    )
}

fn reference(id: u64, target: ReferenceTarget) -> Result<ExpressionAst, String> {
    let node = ExprNodeId::new(sid(id));
    check(
        ExpressionAst::new(node, vec![ExprNode::reference(node, target)]),
        "reference AST",
    )
}

fn string(value: &str) -> TypedValue {
    TypedValue::String(value.to_string())
}

fn required(value_type: ExpressionType) -> TypeSpec {
    TypeSpec::required(value_type)
}

struct ComponentSpec {
    component: ComponentRef,
    name: &'static str,
    root: u64,
    tree: TreeBuilder,
    properties: Vec<(u64, &'static str, TypedValue)>,
    slots: Vec<ComponentSlot>,
}

fn register(library: &mut ComponentLibrary, spec: ComponentSpec) -> Result<(), String> {
    let mut definition = ComponentDefinition::new(
        spec.component.id(),
        spec.name.to_string(),
        NodeRef::new(sid(spec.root)),
    );
    for (id, _, _) in &spec.properties {
        check(
            definition.add_property(ComponentPropertyId::new(sid(*id))),
            "property 선언",
        )?;
    }
    for slot in &spec.slots {
        check(definition.add_slot(slot.id), "slot 선언")?;
    }
    check(library.insert(definition, spec.tree.tree), spec.name)?;
    for (n, (id, name, value)) in spec.properties.into_iter().enumerate() {
        let property = check(
            ComponentProperty::new(
                ComponentPropertyId::new(sid(id)),
                name.to_string(),
                required(ExpressionType::of(&value)),
                literal(id * 10 + n as u64, value)?,
                PropertyVisibility::Public,
            ),
            name,
        )?;
        check(library.add_property(spec.component, property), name)?;
    }
    for slot in spec.slots {
        check(library.add_slot(spec.component, slot), "slot")?;
    }
    Ok(())
}

fn variable(
    library: &ComponentLibrary,
    component: ComponentRef,
    id: u64,
    name: &str,
    value_type: ExpressionType,
    initial: TypedValue,
) -> Result<StateVariable, String> {
    let entry = library.get(component).ok_or("컴포넌트 없음")?;
    check(
        StateVariable::new(
            StateVariableId::new(sid(id)),
            name.to_string(),
            required(value_type),
            literal(id * 10, initial)?,
            &PropertyScope::new(entry),
        ),
        name,
    )
}

fn set_machine(
    library: &mut ComponentLibrary,
    component: ComponentRef,
    variables: Vec<StateVariable>,
    actions: &[(u64, &str)],
    transitions: Vec<TransitionSpec>,
) -> Result<(), String> {
    let mut machine = StateMachine::new();
    for variable in variables {
        check(machine.add_variable(variable), "상태 변수")?;
    }
    for (id, name) in actions {
        check(
            machine.add_action(check(
                SemanticAction::new(SemanticActionId::new(sid(*id)), name.to_string()),
                name,
            )?),
            name,
        )?;
    }
    let entry = library.get(component).ok_or("컴포넌트 없음")?.clone();
    for spec in transitions {
        check(machine.add_transition(&entry, spec), "transition")?;
    }
    check(library.set_state_machine(component, machine), "상태 머신")?;
    Ok(())
}

fn set_from_payload(
    transition: u64,
    variable: u64,
    kind: EventKind,
    source: u64,
) -> Result<TransitionSpec, String> {
    let id = TransitionId::new(sid(transition));
    Ok(TransitionSpec {
        id,
        trigger: Trigger::Event {
            kind,
            source: sid(source),
        },
        condition: None,
        actions: vec![ActionSpec::SetState {
            variable: StateVariableId::new(sid(variable)),
            value: reference(
                transition * 10,
                ReferenceTarget::Property(event_payload_path(id)),
            )?,
        }],
    })
}

fn button_spec() -> Result<ComponentSpec, String> {
    let mut tree = TreeBuilder::new(button_ref());
    tree.frame(1011, None, "button")?;
    tree.text(1012, 1011, "label")?;
    Ok(ComponentSpec {
        component: button_ref(),
        name: "Button",
        root: 1011,
        tree,
        properties: vec![
            (1013, "label", string("확인")),
            (1014, "disabled", TypedValue::Boolean(false)),
        ],
        slots: Vec::new(),
    })
}

fn text_input_spec() -> Result<ComponentSpec, String> {
    let mut tree = TreeBuilder::new(text_input_ref());
    tree.frame(1021, None, "field")?;
    tree.text(1022, 1021, "value")?;
    Ok(ComponentSpec {
        component: text_input_ref(),
        name: "TextInput",
        root: 1021,
        tree,
        properties: vec![(1023, "placeholder", string("입력"))],
        slots: Vec::new(),
    })
}

fn modal_spec() -> Result<ComponentSpec, String> {
    let mut tree = TreeBuilder::new(modal_ref());
    tree.frame(1031, None, "modal")?;
    tree.frame(1032, Some(1031), "overlay")?;
    tree.frame(1033, Some(1031), "content")?;
    tree.frame(1034, Some(1033), "body")?;
    tree.add(
        MODAL_CLOSE_BUTTON,
        NodeKind::ComponentInstance,
        Some(1033),
        "close",
        ComponentInstancePayload::new(button_ref()).encode(),
    )?;
    let slot = ComponentSlot::new(
        ComponentSlotId::new(sid(1040)),
        NodeRef::new(sid(1034)),
        [NodeKind::Frame, NodeKind::Text],
        check(SlotCardinality::new(0, None), "cardinality")?,
        Vec::new(),
    );
    Ok(ComponentSpec {
        component: modal_ref(),
        name: "Modal",
        root: 1031,
        tree,
        properties: vec![(1036, "closeLabel", string("닫기"))],
        slots: vec![slot],
    })
}

fn list_spec() -> Result<ComponentSpec, String> {
    let mut tree = TreeBuilder::new(list_ref());
    tree.frame(1051, None, "list")?;
    tree.frame(1052, Some(1051), "items")?;
    for (i, item) in [1053u64, 1054, 1055].into_iter().enumerate() {
        tree.frame(item, Some(1052), &format!("item {i}"))?;
        tree.text(item + 3, item, &format!("item {i} label"))?;
    }
    Ok(ComponentSpec {
        component: list_ref(),
        name: "List",
        root: 1051,
        tree,
        properties: Vec::new(),
        slots: Vec::new(),
    })
}

fn radio_spec() -> Result<ComponentSpec, String> {
    let mut tree = TreeBuilder::new(radio_ref());
    tree.frame(1061, None, "radio group")?;
    tree.frame(1062, Some(1061), "options list")?;
    tree.frame(1063, Some(1062), "options")?;
    for (i, option) in [1064u64, 1065, 1066].into_iter().enumerate() {
        tree.frame(option, Some(1063), &format!("option {i}"))?;
        tree.text(option + 10, option, &format!("option {i} label"))?;
    }
    Ok(ComponentSpec {
        component: radio_ref(),
        name: "Radio",
        root: 1061,
        tree,
        properties: Vec::new(),
        slots: Vec::new(),
    })
}

fn attach(
    library: &ComponentLibrary,
    table: &mut ControlContractTable,
    component: ComponentRef,
    frame: u64,
    contract: ControlContract,
) -> Result<(), String> {
    let entry = library.get(component).ok_or("컴포넌트 없음")?;
    check(
        table.attach(
            sid(frame),
            |id| entry.tree().node(id).map(|n| n.kind),
            contract,
        ),
        "contract 부착",
    )
}

fn button_like(activate: SemanticActionId) -> SemanticContract {
    SemanticContract::new(button::BUTTON)
        .with(button::fields::ACTIVATE, ContractValue::Action(activate))
}

fn button_interaction() -> InteractionContract {
    InteractionContract::new(
        true,
        [EventKind::Activate, EventKind::Focus, EventKind::Blur],
    )
}

/// 합성 fixture를 만듭니다. 구성 오류는 설명 문자열입니다.
pub fn component_library_fixture() -> Result<ComponentLibraryFixture, String> {
    let mut library = ComponentLibrary::new();
    for spec in [
        button_spec()?,
        text_input_spec()?,
        modal_spec()?,
        list_spec()?,
        radio_spec()?,
    ] {
        register(&mut library, spec)?;
    }

    // 상태 머신.
    set_machine(
        &mut library,
        button_ref(),
        Vec::new(),
        &[(1015, "activated")],
        vec![TransitionSpec {
            id: TransitionId::new(sid(1016)),
            trigger: Trigger::Event {
                kind: EventKind::Activate,
                source: sid(1011),
            },
            condition: None,
            actions: vec![ActionSpec::Emit {
                action: button_activated(),
            }],
        }],
    )?;
    let text = variable(
        &library,
        text_input_ref(),
        1024,
        "text",
        ExpressionType::String,
        string(""),
    )?;
    set_machine(
        &mut library,
        text_input_ref(),
        vec![text],
        &[],
        vec![set_from_payload(1025, 1024, EventKind::Input, 1021)?],
    )?;
    let open = variable(
        &library,
        modal_ref(),
        1037,
        "open",
        ExpressionType::Boolean,
        TypedValue::Boolean(true),
    )?;
    set_machine(
        &mut library,
        modal_ref(),
        vec![open],
        &[(1038, "dismissed")],
        vec![TransitionSpec {
            id: TransitionId::new(sid(1039)),
            trigger: Trigger::NestedAction {
                instance: sid(MODAL_CLOSE_BUTTON),
                action: button_activated(),
            },
            condition: None,
            actions: vec![
                ActionSpec::SetState {
                    variable: StateVariableId::new(sid(1037)),
                    value: literal(10_390, TypedValue::Boolean(false))?,
                },
                ActionSpec::Emit {
                    action: SemanticActionId::new(sid(1038)),
                },
            ],
        }],
    )?;
    let selection = variable(
        &library,
        list_ref(),
        1059,
        "selection",
        ExpressionType::String,
        string(""),
    )?;
    set_machine(
        &mut library,
        list_ref(),
        vec![selection],
        &[],
        vec![set_from_payload(
            1060,
            1059,
            EventKind::SelectionChange,
            1052,
        )?],
    )?;
    let selected = variable(
        &library,
        radio_ref(),
        1067,
        "selected",
        ExpressionType::String,
        string(""),
    )?;
    set_machine(
        &mut library,
        radio_ref(),
        vec![selected],
        &[(1068, "chosen")],
        vec![set_from_payload(
            1070,
            1067,
            EventKind::SelectionChange,
            1062,
        )?],
    )?;

    // control contract.
    let mut contracts = BTreeMap::new();
    let mut table = ControlContractTable::new();
    attach(
        &library,
        &mut table,
        button_ref(),
        1011,
        ControlContract::new(
            button_like(button_activated()).with(
                button::fields::DISABLED,
                ContractValue::Property(ComponentPropertyId::new(sid(1014))),
            ),
            button_interaction(),
        ),
    )?;
    contracts.insert(button_ref(), table);

    let mut table = ControlContractTable::new();
    let mut input = text_input::text_input_contract(StateVariableId::new(sid(1024)));
    input.semantic.set(
        text_input::fields::PLACEHOLDER,
        ContractValue::Property(ComponentPropertyId::new(sid(1023))),
    );
    attach(&library, &mut table, text_input_ref(), 1021, input)?;
    contracts.insert(text_input_ref(), table);

    let mut table = ControlContractTable::new();
    let mut dialog = modal::modal_contract(sid(1033));
    dialog
        .semantic
        .set(modal::fields::OVERLAY, ContractValue::Node(sid(1032)));
    dialog.semantic.set(
        modal::fields::DISMISS,
        ContractValue::Action(SemanticActionId::new(sid(1038))),
    );
    dialog.semantic.set(
        modal::fields::OPEN,
        ContractValue::State(StateVariableId::new(sid(1037))),
    );
    attach(&library, &mut table, modal_ref(), 1031, dialog)?;
    contracts.insert(modal_ref(), table);

    let mut table = ControlContractTable::new();
    let mut items = list::list_contract(sid(1052), SelectionMode::Single);
    items.semantic.set(
        list::fields::SELECTION,
        ContractValue::State(StateVariableId::new(sid(1059))),
    );
    attach(&library, &mut table, list_ref(), 1051, items)?;
    contracts.insert(list_ref(), table);

    let mut table = ControlContractTable::new();
    let mut group = selection_group::single_selection_group_contract(sid(1062));
    group.semantic.set(
        selection_group::fields::SELECTED,
        ContractValue::State(StateVariableId::new(sid(1067))),
    );
    attach(&library, &mut table, radio_ref(), 1061, group)?;
    attach(
        &library,
        &mut table,
        radio_ref(),
        1062,
        list::list_contract(sid(1063), SelectionMode::Single),
    )?;
    for option in [1064u64, 1065, 1066] {
        attach(
            &library,
            &mut table,
            radio_ref(),
            option,
            ControlContract::new(
                button_like(SemanticActionId::new(sid(1068))),
                button_interaction(),
            ),
        )?;
    }
    contracts.insert(radio_ref(), table);

    // drilling: Modal.closeLabel → 닫기 Button.label.
    let mut drilling = DrillingSet::new();
    check(
        drilling.insert(DrillingBinding::link(
            DrillingBindingId::new(sid(1041)),
            modal_ref(),
            ComponentPropertyId::new(sid(1036)),
            sid(MODAL_CLOSE_BUTTON),
            button_label(),
        )),
        "drilling link",
    )?;
    Ok(ComponentLibraryFixture {
        library,
        contracts,
        drilling,
    })
}

/// fixture scene: root Frame(9000) ← 컴포넌트별 인스턴스(9001~9005). Button 인스턴스는 `label` override, Modal
/// 인스턴스는 소유 Text(9006)를 body slot에 투영합니다.
pub fn fixture_scene() -> Result<(NodeColumns, ScenePayloads), String> {
    let mut columns = NodeColumns::new();
    let field = |id: u64, kind: NodeKind, parent, k: u8| -> Result<NodeFields, String> {
        Ok(NodeFields {
            stable_id: sid(id),
            kind,
            parent,
            order_key: key(k)?,
            visible: true,
            locked: false,
            display_name: String::new(),
        })
    };
    let root = columns.insert(field(SCENE_ROOT, NodeKind::Frame, None, 0)?);
    let mut payloads = ScenePayloads::new();
    let mut handles = BTreeMap::new();
    for (i, (_, component)) in components().into_iter().enumerate() {
        let id = SCENE_ROOT + 1 + i as u64;
        let handle = columns.insert(field(id, NodeKind::ComponentInstance, Some(root), i as u8)?);
        handles.insert(id, handle);
        let mut payload = ComponentInstancePayload::new(component);
        if component == button_ref() {
            payload.set_property_override(OverrideKey::direct(button_label()), string("저장"));
        }
        if component == modal_ref() {
            payload.set_slot_override(
                ComponentSlotId::new(sid(1040)),
                check(SlotOverride::new(vec![sid(9006)]), "slot override")?,
            );
        }
        payloads.insert(sid(id), payload.encode());
    }
    let modal = handles
        .get(&(SCENE_ROOT + 3))
        .copied()
        .ok_or("Modal 인스턴스 없음")?;
    columns.insert(field(9006, NodeKind::Text, Some(modal), 0)?);
    payloads.insert(sid(9006), Vec::new());
    Ok((columns, payloads))
}

/// FNV-1a 64(플랫폼 무관 정수 연산).
fn fnv1a(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// library 구성 요약과 canonical bytes checksum JSON.
pub fn library_snapshot(fixture: &ComponentLibraryFixture) -> String {
    let library_bytes = fixture.library.encode();
    let mut components_json = Vec::new();
    for (name, component) in components() {
        let Some(entry) = fixture.library.get(component) else {
            continue;
        };
        let properties = entry
            .definition()
            .properties()
            .iter()
            .filter_map(|id| entry.property(*id))
            .map(|p| {
                Json::object([
                    ("name", Json::str(p.name())),
                    ("public", Json::Bool(p.is_public())),
                    ("type", Json::String(p.value_type().to_string())),
                ])
            })
            .collect();
        let machine = entry.state_machine();
        let contracts = fixture
            .contracts
            .get(&component)
            .map(|table| {
                table
                    .iter()
                    .map(|(frame, contract)| {
                        let node = entry
                            .tree()
                            .node(frame)
                            .map(|n| n.display_name.clone())
                            .unwrap_or_default();
                        let kind = descriptor(contract.kind()).map(|d| d.name).unwrap_or("?");
                        Json::object([("frame", Json::String(node)), ("kind", Json::str(kind))])
                    })
                    .collect()
            })
            .unwrap_or_default();
        components_json.push(Json::object([
            ("contracts", Json::Array(contracts)),
            ("name", Json::str(name)),
            ("nodes", Json::uint(entry.tree().len() as u64)),
            ("properties", Json::Array(properties)),
            ("slots", Json::uint(entry.slots().count() as u64)),
            (
                "stateActions",
                Json::Array(machine.actions().map(|a| Json::str(a.name())).collect()),
            ),
            (
                "stateVariables",
                Json::Array(machine.variables().map(|v| Json::str(v.name())).collect()),
            ),
            (
                "transitions",
                Json::uint(machine.transitions().len() as u64),
            ),
        ]));
    }
    let contract_bytes: Vec<u8> = fixture
        .contracts
        .values()
        .flat_map(ControlContractTable::encode)
        .collect();
    Json::object([
        ("components", Json::Array(components_json)),
        (
            "checksums",
            Json::object([
                ("contracts", Json::String(fnv1a(&contract_bytes))),
                ("drilling", Json::String(fnv1a(&fixture.drilling.encode()))),
                ("library", Json::String(fnv1a(&library_bytes))),
            ]),
        ),
        ("drillingLinks", Json::uint(fixture.drilling.len() as u64)),
        ("libraryBytes", Json::uint(library_bytes.len() as u64)),
    ])
    .to_pretty_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::encapsulation::validate_scene_encapsulation;
    use crate::component::expansion::{InstanceExpansion, SceneIndex};
    use crate::component::override_rules::{
        validate_definition_property_overrides, validate_scene_property_overrides,
    };
    use crate::component::slot_override::{
        validate_definition_slot_overrides, validate_scene_slot_overrides,
    };
    use crate::component::state_diagnostics::diagnose_state_machine;
    use crate::control::registry::validate_contracts;

    const FIXTURE: &str = "src/test_support/fixtures/component_library.json";

    #[test]
    fn composite_library_passes_every_document_validator_with_zero_errors() {
        // 검증 기준(IX.9.1): document validator 0 오류.
        let fixture = component_library_fixture().unwrap();
        let library = &fixture.library;
        assert_eq!(library.len(), 5);
        // library 저장 검증(정의·tree·Slot·Property·nested override·상태 머신) 통과와 round-trip.
        let bytes = library.encode();
        let decoded = ComponentLibrary::decode(&bytes).unwrap();
        assert_eq!(&decoded, library);
        assert_eq!(decoded.encode(), bytes);
        for (name, component) in components() {
            library.validate_properties(component).unwrap();
            library.validate_slots(component).unwrap();
            // 정의 tree 안 nested 인스턴스(Modal의 닫기 Button)의 property·slot override를 미해결 참조 건너뛰기 없이
            // 검사합니다(library 디코딩 경로는 미해결 참조를 건너뜀).
            validate_definition_property_overrides(library, component).unwrap();
            validate_definition_slot_overrides(library, component).unwrap();
            // control contract: 오류·진단 모두 0건.
            let entry = library.get(component).unwrap();
            let issues = validate_contracts(&fixture.contracts[&component], entry);
            assert!(issues.is_empty(), "{name}: {issues:?}");
            // 상태 머신 진단 0건.
            let diagnostics = diagnose_state_machine(library, component);
            assert!(diagnostics.is_empty(), "{name}: {diagnostics:?}");
            // contract 표 round-trip.
            let table = &fixture.contracts[&component];
            assert_eq!(
                &ControlContractTable::decode(&table.encode()).unwrap(),
                table
            );
        }
        // drilling link 검증 통과.
        fixture.drilling.validate(library).unwrap();
        assert_eq!(
            DrillingSet::decode(&fixture.drilling.encode()).unwrap(),
            fixture.drilling
        );
        // Radio는 RadioButton node 없이 List + Button 계약으로 합성.
        let radio = library.get(radio_ref()).unwrap();
        assert!(radio
            .tree()
            .iter()
            .all(|n| matches!(n.kind, NodeKind::Frame | NodeKind::Text)));
        assert_eq!(fixture.contracts[&radio_ref()].len(), 5);
    }

    #[test]
    fn validators_are_active_on_the_fixture() {
        // 0 오류가 검사 생략이 아님을 확인: 한 option의 Button 계약을 떼면 그룹 검사가 오류를 냅니다.
        let mut fixture = component_library_fixture().unwrap();
        let table = fixture.contracts.get_mut(&radio_ref()).unwrap();
        table.detach(sid(1065)).unwrap();
        let entry = fixture.library.get(radio_ref()).unwrap();
        let issues = validate_contracts(&fixture.contracts[&radio_ref()], entry);
        assert!(issues
            .iter()
            .any(|issue| issue.problem == crate::control::ContractProblem::InvalidLinkedContract));
        // drilling link가 가리키는 Button.label을 지우면 link가 orphan이 됩니다(IX.4.6).
        let mut fixture = component_library_fixture().unwrap();
        let mut drilling = fixture.drilling.clone();
        crate::component::property_commands::PropertyCommand::Delete {
            component: button_ref(),
            property: button_label(),
        }
        .apply(&mut fixture.library, &mut drilling)
        .unwrap();
        assert!(drilling.is_orphan(DrillingBindingId::new(sid(1041))));
    }

    #[test]
    fn scene_instances_validate_and_expand() {
        let fixture = component_library_fixture().unwrap();
        let (columns, payloads) = fixture_scene().unwrap();
        let scene = SceneIndex::build(&columns, &payloads);
        // scene 전체 node의 override 캡슐화 규칙(IX.4.2: nested 직접 경로 금지).
        let scene_nodes = (0..=5u64).map(|i| sid(SCENE_ROOT + i)).chain([sid(9006)]);
        validate_scene_encapsulation(&fixture.library, &scene, scene_nodes).unwrap();
        for i in 0..5u64 {
            let instance = sid(SCENE_ROOT + 1 + i);
            validate_scene_property_overrides(&fixture.library, &scene, instance).unwrap();
            validate_scene_slot_overrides(&fixture.library, &scene, instance).unwrap();
            let expansion = InstanceExpansion::new(&fixture.library, &scene, instance).unwrap();
            let walked = expansion.walk().unwrap();
            assert!(!walked.is_empty());
        }
        // Modal 인스턴스 expansion에는 nested 닫기 Button과 투영된 slot content가 포함됩니다.
        let modal = InstanceExpansion::new(&fixture.library, &scene, sid(SCENE_ROOT + 3)).unwrap();
        let walked = modal.walk().unwrap();
        assert!(
            walked.contains(&crate::component::expansion::ExpandedNodeId::Owned(sid(
                9006
            )))
        );
        assert!(walked.iter().any(|id| matches!(
            id,
            crate::component::expansion::ExpandedNodeId::Virtual { source, .. } if *source == sid(1011)
        )));
    }

    #[test]
    fn library_snapshot_matches_fixture_bytes() {
        let fixture = component_library_fixture().unwrap();
        let snapshot = library_snapshot(&fixture);
        assert!(snapshot.is_ascii());
        assert_eq!(
            snapshot,
            library_snapshot(&component_library_fixture().unwrap()),
            "결정론"
        );
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE);
        if std::env::var_os("STELLAR_UPDATE_SNAPSHOT_FIXTURES").is_some() {
            std::fs::write(&fixture_path, &snapshot).unwrap();
        }
        let expected = std::fs::read(&fixture_path).expect("fixture 파일");
        assert!(
            snapshot.as_bytes() == expected.as_slice(),
            "component library snapshot이 fixture와 다릅니다. 의도한 변경이면 \
             STELLAR_UPDATE_SNAPSHOT_FIXTURES=1로 갱신하세요.\n{snapshot}"
        );
    }
}
