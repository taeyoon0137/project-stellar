//! 프로토타입 엔진 통합 시험 fixture: motion이 있는 토글 컴포넌트.
//!
//! 컴포넌트 `Toggle`(sid 1):
//! - tree: root Frame(10) ─ knob Frame(11), label Text(12)
//! - Property: `label`(String, 20), `size`(Length 20px, 21)
//! - 상태: `on`(Boolean false, 30), `count`(Number 0, 31)
//! - action: `toggled`(40)
//! - transition 50: root Activate → `on = not on`, emit `toggled`
//! - binding 70: root `background-color` = if on then green else gray
//! - binding 80: knob `left` = if on then 24px else 0px
//! - binding 90: knob `width` = Property `size` (상태와 무관)
//! - MotionSpec 100: [root background-color, knob left], `on` 변화에 200ms `ease`
//!
//! 두 번째 컴포넌트 `Other`(sid 2): root Frame(200).

#![allow(dead_code)]

use stellar_bindings::binding::{BindingId, BindingSet, ScopeOwner, ValueBinding};
use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_core_types::reference::{ComponentRef, NodeRef};
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::{FiniteF32, Length};
use stellar_document::component::definition::{
    ComponentDefinition, ComponentLibrary, ComponentPropertyId, DefinitionNode, DefinitionTree,
};
use stellar_document::component::event::EventKind;
use stellar_document::component::property::{ComponentProperty, PropertyVisibility};
use stellar_document::component::state_machine::{
    state_value_path, ActionSpec, PropertyScope, SemanticAction, SemanticActionId, StateMachine,
    StateVariable, StateVariableId, TransitionId, TransitionSpec, Trigger,
};
use stellar_document::layout_record::LayoutLonghand;
use stellar_document::motion::{
    MotionOwner, MotionSet, MotionSpec, MotionSpecId, MotionSpecInput, ReducedMotion,
    StartCondition, TimingFunction,
};
use stellar_document::node_kind::NodeKind;
use stellar_expressions::ast::{ExprNode, ExprNodeId, ExpressionAst, ReferenceTarget};
use stellar_expressions::ops::UnaryOp;
use stellar_expressions::types::{ExpressionType, TypeSpec};
use stellar_prototype::edit::PrototypeDocument;

pub fn sid(n: u64) -> StableId {
    StableId::new_v7(n, 0x0abc, n)
}

pub const TOGGLE: u64 = 1;
pub const OTHER: u64 = 2;
pub const ROOT: u64 = 10;
pub const KNOB: u64 = 11;
pub const LABEL: u64 = 12;
pub const OTHER_ROOT: u64 = 200;

pub fn toggle() -> ComponentRef {
    ComponentRef::new(sid(TOGGLE))
}

pub fn other() -> ComponentRef {
    ComponentRef::new(sid(OTHER))
}

pub fn on() -> StateVariableId {
    StateVariableId::new(sid(30))
}

pub fn count() -> StateVariableId {
    StateVariableId::new(sid(31))
}

pub fn toggled() -> SemanticActionId {
    SemanticActionId::new(sid(40))
}

pub fn background(node: u64) -> PropertyPath {
    PropertyPath::new(sid(node)).push(PropertyKey(0x0104))
}

pub fn left(node: u64) -> PropertyPath {
    PropertyPath::new(sid(node)).push(LayoutLonghand::Left.record_key())
}

pub fn width(node: u64) -> PropertyPath {
    PropertyPath::new(sid(node)).push(PropertyKey(0x0108))
}

pub fn color(r: f32, g: f32, b: f32) -> TypedValue {
    TypedValue::Color(Color::new(ColorSpace::Srgb, r, g, b, 1.0).expect("색"))
}

pub fn px(v: f32) -> TypedValue {
    TypedValue::Length(Length::new(v).expect("길이"))
}

pub fn x(n: u64) -> ExprNodeId {
    ExprNodeId::new(sid(n))
}

pub fn state_ref(id: u64, variable: StateVariableId) -> ExprNode {
    ExprNode::reference(x(id), ReferenceTarget::Property(state_value_path(variable)))
}

pub fn literal_ast(id: u64, value: TypedValue) -> ExpressionAst {
    ExpressionAst::new(x(id), vec![ExprNode::literal(x(id), value)]).expect("AST")
}

/// `if on then a else b`.
pub fn if_on(base: u64, a: TypedValue, b: TypedValue) -> ExpressionAst {
    ExpressionAst::new(
        x(base),
        vec![
            ExprNode::if_else(x(base), x(base + 1), x(base + 2), x(base + 3)),
            state_ref(base + 1, on()),
            ExprNode::literal(x(base + 2), a),
            ExprNode::literal(x(base + 3), b),
        ],
    )
    .expect("AST")
}

fn tree_node(
    id: u64,
    kind: NodeKind,
    parent: Option<u64>,
    index: u8,
    name: &str,
) -> DefinitionNode {
    DefinitionNode {
        id: sid(id),
        kind,
        parent: parent.map(sid),
        order_key: OrderKey::from_bytes(vec![0x40 + index * 0x10]).expect("order key"),
        visible: true,
        locked: false,
        display_name: name.to_string(),
        payload: Vec::new(),
    }
}

fn library() -> ComponentLibrary {
    let mut library = ComponentLibrary::new();
    let mut tree = DefinitionTree::new(toggle());
    tree.insert(tree_node(ROOT, NodeKind::Frame, None, 0, "root"))
        .expect("root");
    tree.insert(tree_node(KNOB, NodeKind::Frame, Some(ROOT), 0, "knob"))
        .expect("knob");
    tree.insert(tree_node(LABEL, NodeKind::Text, Some(ROOT), 1, "label"))
        .expect("label");
    let mut definition =
        ComponentDefinition::new(sid(TOGGLE), "Toggle".into(), NodeRef::new(sid(ROOT)));
    definition
        .add_property(ComponentPropertyId::new(sid(20)))
        .expect("선언");
    definition
        .add_property(ComponentPropertyId::new(sid(21)))
        .expect("선언");
    library.insert(definition, tree).expect("Toggle");
    library
        .add_property(
            toggle(),
            ComponentProperty::new(
                ComponentPropertyId::new(sid(20)),
                "label".into(),
                TypeSpec::required(ExpressionType::String),
                literal_ast(22, TypedValue::String("Toggle".into())),
                PropertyVisibility::Public,
            )
            .expect("label"),
        )
        .expect("label");
    library
        .add_property(
            toggle(),
            ComponentProperty::new(
                ComponentPropertyId::new(sid(21)),
                "size".into(),
                TypeSpec::required(ExpressionType::Length),
                literal_ast(23, px(20.0)),
                PropertyVisibility::Public,
            )
            .expect("size"),
        )
        .expect("size");

    let mut other_tree = DefinitionTree::new(other());
    other_tree
        .insert(tree_node(
            OTHER_ROOT,
            NodeKind::Frame,
            None,
            0,
            "other root",
        ))
        .expect("other root");
    library
        .insert(
            ComponentDefinition::new(sid(OTHER), "Other".into(), NodeRef::new(sid(OTHER_ROOT))),
            other_tree,
        )
        .expect("Other");

    let entry = library.get(toggle()).expect("entry").clone();
    let scope = PropertyScope::new(&entry);
    let mut machine = StateMachine::new();
    machine
        .add_action(SemanticAction::new(toggled(), "toggled".into()).expect("action"))
        .expect("action");
    machine
        .add_variable(
            StateVariable::new(
                on(),
                "on".into(),
                TypeSpec::required(ExpressionType::Boolean),
                literal_ast(32, TypedValue::Boolean(false)),
                &scope,
            )
            .expect("on"),
        )
        .expect("on");
    machine
        .add_variable(
            StateVariable::new(
                count(),
                "count".into(),
                TypeSpec::required(ExpressionType::Number),
                literal_ast(33, TypedValue::Number(FiniteF32::new(0.0).expect("0"))),
                &scope,
            )
            .expect("count"),
        )
        .expect("count");
    let toggle_value = ExpressionAst::new(
        x(60),
        vec![
            ExprNode::unary(x(60), UnaryOp::Not, x(61)),
            state_ref(61, on()),
        ],
    )
    .expect("AST");
    machine
        .add_transition(
            &entry,
            TransitionSpec {
                id: TransitionId::new(sid(50)),
                trigger: Trigger::Event {
                    kind: EventKind::Activate,
                    source: sid(ROOT),
                },
                condition: None,
                actions: vec![
                    ActionSpec::SetState {
                        variable: on(),
                        value: toggle_value,
                    },
                    ActionSpec::Emit { action: toggled() },
                ],
            },
        )
        .expect("transition");
    library
        .set_state_machine(toggle(), machine)
        .expect("상태 머신");
    library
}

fn binding(
    id: u64,
    target: PropertyPath,
    source: ExpressionAst,
    t: ExpressionType,
) -> ValueBinding {
    ValueBinding::new(
        BindingId::new(sid(id)),
        target,
        source,
        TypeSpec::required(t),
        ScopeOwner::Component(toggle()),
    )
}

pub fn green() -> TypedValue {
    color(0.0, 0.8, 0.2)
}

pub fn gray() -> TypedValue {
    color(0.6, 0.6, 0.6)
}

pub fn motion_id() -> MotionSpecId {
    MotionSpecId::new(sid(100))
}

pub fn motion(duration_ms: f32, reduced_motion: ReducedMotion) -> MotionSpec {
    MotionSpec::new(MotionSpecInput {
        id: motion_id(),
        owner: MotionOwner::Component(toggle()),
        targets: vec![background(ROOT), left(KNOB)],
        start: StartCondition::Changes { variable: on() },
        duration_ms,
        delay_ms: 0.0,
        timing: TimingFunction::EASE,
        reduced_motion,
    })
    .expect("motion")
}

/// fixture 문서.
pub fn document() -> PrototypeDocument {
    let mut bindings = BindingSet::new();
    bindings
        .insert(binding(
            70,
            background(ROOT),
            if_on(71, green(), gray()),
            ExpressionType::Color,
        ))
        .expect("70");
    bindings
        .insert(binding(
            80,
            left(KNOB),
            if_on(81, px(24.0), px(0.0)),
            ExpressionType::Length,
        ))
        .expect("80");
    bindings
        .insert(binding(
            90,
            width(KNOB),
            ExpressionAst::new(
                x(91),
                vec![ExprNode::reference(
                    x(91),
                    ReferenceTarget::ComponentProperty(sid(21)),
                )],
            )
            .expect("AST"),
            ExpressionType::Length,
        ))
        .expect("90");
    let mut motions = MotionSet::new();
    motions
        .insert(motion(200.0, ReducedMotion::Instant))
        .expect("motion");
    PrototypeDocument {
        library: library(),
        bindings,
        motions,
        ..PrototypeDocument::default()
    }
}

pub const CONTEXT_KEY: u64 = 700;
pub const CONTEXT_READ: u64 = 711;
pub const CONTEXT_BINDING: u64 = 710;

pub fn context_key() -> StableId {
    sid(CONTEXT_KEY)
}

/// fixture에 Color key 하나와, knob 배경이 그 key를 읽는 binding을 더한 문서.
pub fn context_document() -> PrototypeDocument {
    let mut doc = document();
    doc.context
        .insert_key(
            stellar_document::context::key::ContextKey::new(
                context_key(),
                "accent",
                stellar_document::context::key::ContextScope::Project,
                TypeSpec::required(ExpressionType::Color),
                literal_ast(701, green()),
            )
            .expect("key"),
        )
        .expect("insert");
    doc.bindings
        .insert(ValueBinding::new(
            BindingId::new(sid(CONTEXT_BINDING)),
            background(KNOB),
            ExpressionAst::new(
                x(CONTEXT_READ),
                vec![ExprNode::reference(
                    x(CONTEXT_READ),
                    ReferenceTarget::Context(context_key()),
                )],
            )
            .expect("AST"),
            TypeSpec::required(ExpressionType::Color),
            ScopeOwner::Component(toggle()),
        ))
        .expect("binding");
    doc
}
