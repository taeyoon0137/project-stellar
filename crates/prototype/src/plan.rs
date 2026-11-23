//! 그래프 편집 → command plan (XXX.2.3, XXX.1.2, ADR-0025 결정 2.2·2.4·2.5, ADR-0094 결정 3·4).
//!
//! 사용자 동작 하나([`GraphEdit`])를 기존 binding·transition·MotionSpec command로 낮춘 [`PrototypeTransaction`] 하나로
//! 만듭니다. plan을 만드는 동안 문서는 바뀌지 않으며, 적용은 [`PrototypeTransaction::apply`]가 원자적으로 합니다.
//!
//! 연결 규칙:
//!
//! - State 출력(상태 값·식·motion drive)은 State 입력(속성 값·motion 시작·속성 motion)에만, Pulse 출력(이벤트)은 Pulse
//!   입력(transition trigger)에만 바로 연결합니다. 종류가 다르면 [`PlanError::Connection`]으로 거부합니다.
//! - 이벤트(Pulse)로 상태 변수(State)를 바꾸려면 등록된 변환 node([`GraphEdit::AddConversion`])를 씁니다. 토글은
//!   `상태 = not 상태`, 카운터는 `상태 = 상태 + 1` transition으로 낮춥니다. 상태 변화→펄스 변환은 지금 상태 머신(IX.5)에
//!   상태 변화 trigger가 없어 낮출 곳이 없으므로 [`PlanError::Unsupported`]입니다.
//! - 속성 입력의 State 포트 타입은 CSS 값 문법의 `ExpressionType` 집합이고, 연결된 식의 정적 타입이 binding의 기대
//!   타입(저장 값 variant)이 됩니다.
//! - 다른 컴포넌트 정의 안(nested instance 내부)의 포트에 직접 잇는 edge는 [`PlanError::NestedInternalPort`]입니다.

use std::fmt;

use stellar_bindings::binding::{BindingId, ScopeOwner, ValueBinding};
use stellar_bindings::binding_commands::BindingCommand;
use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_core_types::reference::ComponentRef;
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::FiniteF32;
use stellar_document::component::definition::ComponentEntry;
use stellar_document::component::event::EventKind;
use stellar_document::component::state_machine::{
    state_value_path, ActionSpec, StateMachine, StateMachineError, StateVariable, StateVariableId,
    TransitionId, TransitionSpec, Trigger,
};
use stellar_document::motion::{
    MotionCommand, MotionOwner, MotionSpec, MotionSpecError, MotionSpecId, MotionSpecInput,
    StartCondition,
};
use stellar_document::prototype::layout_hint::HintPoint;
use stellar_document::prototype::node_kind::{ConversionKind, PrototypeNodeKind};
use stellar_document::prototype::socket::{
    check_socket_kinds, check_state_connection, ConnectionError, SocketKind,
};
use stellar_expressions::ast::{ExprNode, ExprNodeId, ExpressionAst, ReferenceTarget};
use stellar_expressions::ops::{BinaryOp, UnaryOp};
use stellar_expressions::typecheck::type_check;
use stellar_expressions::types::{ExpressionType, TypeSpec};

use crate::edit::{OwnerTypes, PrototypeDocument, PrototypeStep, PrototypeTransaction};
use crate::graph::GraphNodeId;

/// 연결 끝점(의미 단위 socket).
#[derive(Debug, Clone, PartialEq)]
pub enum Endpoint {
    /// 상태 변수 값(State 출력).
    StateValue(StateVariableId),
    /// 아직 연결되지 않은 식(State 출력).
    Expression(ExpressionAst),
    /// 이벤트 원천의 이벤트(Pulse 출력).
    EventPulse {
        /// 원천 node.
        source: StableId,
        /// 종류.
        kind: EventKind,
    },
    /// MotionSpec 출력(State 출력).
    MotionDrive(MotionSpecId),
    /// 속성 값 입력(State 입력).
    PropertyInput(PropertyPath),
    /// 속성 motion 입력(State 입력).
    PropertyMotion(PropertyPath),
    /// MotionSpec 시작 조건 입력(State 입력).
    MotionStart(MotionSpecId),
    /// transition trigger 입력(Pulse 입력).
    TransitionTrigger(TransitionId),
    /// 상태 변수 쓰기 입력(Pulse 입력).
    StateWrite(StateVariableId),
}

impl Endpoint {
    /// socket 종류와 방향(`true` = 출력).
    pub fn socket(&self) -> (SocketKind, bool) {
        match self {
            Endpoint::StateValue(_) | Endpoint::Expression(_) | Endpoint::MotionDrive(_) => {
                (SocketKind::State, true)
            }
            Endpoint::EventPulse { .. } => (SocketKind::Pulse, true),
            Endpoint::PropertyInput(_) | Endpoint::PropertyMotion(_) | Endpoint::MotionStart(_) => {
                (SocketKind::State, false)
            }
            Endpoint::TransitionTrigger(_) | Endpoint::StateWrite(_) => (SocketKind::Pulse, false),
        }
    }
}

/// 그래프 편집 하나(사용자 동작 하나).
#[derive(Debug, Clone, PartialEq)]
pub enum GraphEdit {
    /// 출력을 입력에 연결합니다. 속성 입력에 새 binding이 필요하면 `binding` id를 씁니다.
    Connect {
        /// 출력 끝점.
        from: Endpoint,
        /// 입력 끝점.
        to: Endpoint,
        /// 새 binding id(대상 속성에 binding이 없을 때).
        binding: Option<BindingId>,
        /// 상태 값 연결 때 만드는 참조 block id.
        reference_node: Option<StableId>,
    },
    /// 연결을 끊습니다.
    Disconnect {
        /// 출력 끝점.
        from: Endpoint,
        /// 입력 끝점.
        to: Endpoint,
    },
    /// 변환 node를 더합니다(이벤트 → 상태 변수).
    AddConversion {
        /// 변환 종류.
        kind: ConversionKind,
        /// 새 transition id.
        transition: TransitionId,
        /// 입력 이벤트.
        source: StableId,
        /// 이벤트 종류.
        event: EventKind,
        /// 쓰는 상태 변수.
        variable: StateVariableId,
        /// 값 식 block id 두 개(변환 연산, 상태 참조) + 카운터의 상수 block.
        expression_ids: [StableId; 3],
    },
    /// 상태 변수를 더합니다.
    AddStateVariable(StateVariable),
    /// MotionSpec node를 더합니다.
    AddMotion(MotionSpec),
    /// node를 지웁니다.
    DeleteNode(GraphNodeId),
    /// node 위치를 고정하거나(`Some`) 풉니다(`None`).
    PinNode {
        /// node StableId.
        node: StableId,
        /// 좌표.
        point: Option<HintPoint>,
    },
}

/// plan 오류.
#[derive(Debug, Clone, PartialEq)]
pub enum PlanError {
    /// socket 종류·타입 연결 규칙 위반.
    Connection(ConnectionError),
    /// 출력·입력 방향이 맞지 않습니다.
    Direction,
    /// 이 조합의 연결은 없습니다(예: 변환 node 없이 이벤트를 상태 변수에 연결).
    NoSuchConnection(&'static str),
    /// 대상이 소유자 정의 tree 밖입니다.
    OutsideOwner(PropertyPath),
    /// nested instance 내부 포트에 직접 연결하려 했습니다.
    NestedInternalPort(PropertyPath),
    /// 포트로 노출하지 않는 속성입니다.
    Unexposed(&'static str),
    /// 지금 낮출 원본이 없는 편집입니다.
    Unsupported(&'static str),
    /// 없는 원본을 가리킵니다.
    Missing(&'static str),
    /// 지우려는 원본을 다른 곳이 씁니다.
    InUse(&'static str),
    /// 식 타입 오류.
    ExpressionType,
    /// 상태 머신 재구성 실패.
    StateMachine(StateMachineError),
    /// MotionSpec 재구성 실패.
    Motion(MotionSpecError),
    /// 새 id가 필요합니다.
    MissingId(&'static str),
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for PlanError {}

/// 컴포넌트 소유자 하나의 plan 작성기.
pub struct Planner<'a> {
    doc: &'a PrototypeDocument,
    component: ComponentRef,
}

fn reference_ast(id: StableId, target: ReferenceTarget) -> Result<ExpressionAst, PlanError> {
    let node = ExprNodeId::new(id);
    ExpressionAst::new(node, vec![ExprNode::reference(node, target)])
        .map_err(|_| PlanError::ExpressionType)
}

/// 상태 머신을 `edit` 결과로 다시 만듭니다. action·변수·transition 순서는 유지합니다.
pub fn rebuild_machine(
    entry: &ComponentEntry,
    edit: impl FnOnce(&mut Vec<StateVariable>, &mut Vec<TransitionSpec>),
) -> Result<StateMachine, StateMachineError> {
    let current = entry.state_machine();
    let mut variables: Vec<StateVariable> = current.variables().cloned().collect();
    let mut transitions: Vec<TransitionSpec> =
        current.transitions().iter().map(|t| t.to_spec()).collect();
    edit(&mut variables, &mut transitions);
    let mut machine = StateMachine::new();
    for action in current.actions() {
        machine.add_action(action.clone())?;
    }
    for variable in variables {
        machine.add_variable(variable)?;
    }
    for spec in transitions {
        machine.add_transition(entry, spec)?;
    }
    Ok(machine)
}

impl<'a> Planner<'a> {
    /// 컴포넌트 `component` 소유자의 plan 작성기.
    pub fn new(doc: &'a PrototypeDocument, component: ComponentRef) -> Planner<'a> {
        Planner { doc, component }
    }

    fn entry(&self) -> Result<&'a ComponentEntry, PlanError> {
        self.doc
            .library
            .get(self.component)
            .ok_or(PlanError::Missing("컴포넌트"))
    }

    fn owner(&self) -> MotionOwner {
        MotionOwner::Component(self.component)
    }

    /// 속성 대상이 소유자 안의 포트인지 확인합니다.
    fn check_target(&self, target: &PropertyPath) -> Result<(), PlanError> {
        if self
            .doc
            .target_kind(self.component, target.root())
            .is_none()
        {
            return Err(match self.doc.library.owner_of(target.root()) {
                Some(other) if other != self.component => {
                    PlanError::NestedInternalPort(target.clone())
                }
                _ => PlanError::OutsideOwner(target.clone()),
            });
        }
        Ok(())
    }

    fn state_type(&self, variable: StateVariableId) -> Result<TypeSpec, PlanError> {
        self.entry()?
            .state_machine()
            .variable(variable)
            .map(|v| v.value_type())
            .ok_or(PlanError::Missing("상태 변수"))
    }

    fn binding_step(
        &self,
        target: &PropertyPath,
        source: ExpressionAst,
        binding: Option<BindingId>,
    ) -> Result<PrototypeStep, PlanError> {
        self.check_target(target)?;
        let kind = self
            .doc
            .target_kind(self.component, target.root())
            .ok_or_else(|| PlanError::OutsideOwner(target.clone()))?;
        let key = *target
            .keys()
            .first()
            .ok_or(PlanError::Missing("속성 key"))?;
        let port = stellar_document::prototype::socket::property_port(kind, key)
            .map_err(|d| PlanError::Unexposed(d.code))?;
        let types = OwnerTypes {
            doc: self.doc,
            component: self.component,
        };
        let report = type_check(&source, &types);
        if !report.errors().is_empty() {
            return Err(PlanError::ExpressionType);
        }
        let found = report.root_type().ok_or(PlanError::ExpressionType)?;
        let stored = check_state_connection(&port, found).map_err(PlanError::Connection)?;
        let expected = TypeSpec::required(stored);
        let command = match self.doc.bindings.by_target(target) {
            Some(existing) => BindingCommand::Replace {
                binding: existing.id(),
                source,
                expected_type: expected,
            },
            None => BindingCommand::Create(ValueBinding::new(
                binding.ok_or(PlanError::MissingId("binding"))?,
                target.clone(),
                source,
                expected,
                ScopeOwner::Component(self.component),
            )),
        };
        Ok(PrototypeStep::Binding {
            component: self.component,
            command,
        })
    }

    fn motion(&self, id: MotionSpecId) -> Result<&'a MotionSpec, PlanError> {
        self.doc
            .motions
            .get(id)
            .ok_or(PlanError::Missing("MotionSpec"))
    }

    fn replace_motion(
        &self,
        id: MotionSpecId,
        edit: impl FnOnce(&mut MotionSpecInput),
    ) -> Result<PrototypeStep, PlanError> {
        let mut input = self.motion(id)?.to_input();
        edit(&mut input);
        let spec = MotionSpec::new(input).map_err(PlanError::Motion)?;
        Ok(PrototypeStep::Motion(MotionCommand::Replace(spec)))
    }

    fn machine_step(
        &self,
        edit: impl FnOnce(&mut Vec<StateVariable>, &mut Vec<TransitionSpec>),
    ) -> Result<PrototypeStep, PlanError> {
        let machine = rebuild_machine(self.entry()?, edit).map_err(PlanError::StateMachine)?;
        Ok(PrototypeStep::StateMachine {
            component: self.component,
            machine,
        })
    }

    fn connect(
        &self,
        from: &Endpoint,
        to: &Endpoint,
        binding: Option<BindingId>,
        reference_node: Option<StableId>,
    ) -> Result<Vec<PrototypeStep>, PlanError> {
        let ((from_kind, from_out), (to_kind, to_out)) = (from.socket(), to.socket());
        if !from_out || to_out {
            return Err(PlanError::Direction);
        }
        check_socket_kinds(from_kind, to_kind).map_err(PlanError::Connection)?;
        let step = match (from, to) {
            (Endpoint::StateValue(variable), Endpoint::PropertyInput(target)) => {
                self.state_type(*variable)?;
                let id = reference_node.ok_or(PlanError::MissingId("참조 block"))?;
                let ast =
                    reference_ast(id, ReferenceTarget::Property(state_value_path(*variable)))?;
                self.binding_step(target, ast, binding)?
            }
            (Endpoint::Expression(ast), Endpoint::PropertyInput(target)) => {
                self.binding_step(target, ast.clone(), binding)?
            }
            (Endpoint::StateValue(variable), Endpoint::MotionStart(motion)) => {
                self.state_type(*variable)?;
                let variable = *variable;
                self.replace_motion(*motion, |input| {
                    input.start = StartCondition::Changes { variable };
                })?
            }
            (Endpoint::MotionDrive(motion), Endpoint::PropertyMotion(target)) => {
                self.check_target(target)?;
                let target = target.clone();
                self.replace_motion(*motion, |input| {
                    if !input.targets.contains(&target) {
                        input.targets.push(target);
                    }
                })?
            }
            (Endpoint::EventPulse { source, kind }, Endpoint::TransitionTrigger(transition)) => {
                let (source, kind, transition) = (*source, *kind, *transition);
                if self
                    .entry()?
                    .state_machine()
                    .transition(transition)
                    .is_none()
                {
                    return Err(PlanError::Missing("transition"));
                }
                self.machine_step(|_, transitions| {
                    for spec in transitions.iter_mut().filter(|t| t.id == transition) {
                        spec.trigger = Trigger::Event { kind, source };
                    }
                })?
            }
            (Endpoint::EventPulse { .. }, Endpoint::StateWrite(_)) => {
                return Err(PlanError::NoSuchConnection(
                    "이벤트로 상태를 바꾸려면 변환 node(토글·카운터)를 씁니다",
                ))
            }
            _ => return Err(PlanError::NoSuchConnection("지원하지 않는 끝점 조합")),
        };
        Ok(vec![step])
    }

    fn disconnect(&self, from: &Endpoint, to: &Endpoint) -> Result<Vec<PrototypeStep>, PlanError> {
        let step = match (from, to) {
            (
                Endpoint::StateValue(_) | Endpoint::Expression(_),
                Endpoint::PropertyInput(target),
            ) => {
                let binding = self
                    .doc
                    .bindings
                    .by_target(target)
                    .ok_or(PlanError::Missing("binding"))?;
                PrototypeStep::Binding {
                    component: self.component,
                    command: BindingCommand::Delete(binding.id()),
                }
            }
            (Endpoint::MotionDrive(motion), Endpoint::PropertyMotion(target)) => {
                let spec = self.motion(*motion)?;
                if !spec.targets().contains(target) {
                    return Err(PlanError::Missing("motion 대상"));
                }
                if spec.targets().len() == 1 {
                    return Err(PlanError::InUse(
                        "MotionSpec의 마지막 대상입니다. MotionSpec node를 지우세요",
                    ));
                }
                let target = target.clone();
                self.replace_motion(*motion, |input| input.targets.retain(|t| *t != target))?
            }
            _ => return Err(PlanError::NoSuchConnection("끊을 수 없는 연결")),
        };
        Ok(vec![step])
    }

    fn add_conversion(&self, edit: &GraphEdit) -> Result<Vec<PrototypeStep>, PlanError> {
        let GraphEdit::AddConversion {
            kind,
            transition,
            source,
            event,
            variable,
            expression_ids,
        } = edit
        else {
            return Err(PlanError::Unsupported("변환 node가 아님"));
        };
        let value_type = self.state_type(*variable)?;
        let reference = ExprNodeId::new(expression_ids[1]);
        let op = ExprNodeId::new(expression_ids[0]);
        let state = ExprNode::reference(
            reference,
            ReferenceTarget::Property(state_value_path(*variable)),
        );
        let ast = match kind {
            ConversionKind::PulseToggle => {
                if value_type != TypeSpec::required(ExpressionType::Boolean) {
                    return Err(PlanError::Connection(ConnectionError::TypeNotAccepted {
                        found: value_type,
                        port: "pulse-toggle",
                    }));
                }
                ExpressionAst::new(op, vec![ExprNode::unary(op, UnaryOp::Not, reference), state])
            }
            ConversionKind::PulseCounter => {
                if value_type != TypeSpec::required(ExpressionType::Number) {
                    return Err(PlanError::Connection(ConnectionError::TypeNotAccepted {
                        found: value_type,
                        port: "pulse-counter",
                    }));
                }
                let one = ExprNodeId::new(expression_ids[2]);
                let literal = ExprNode::literal(
                    one,
                    TypedValue::Number(FiniteF32::new(1.0).map_err(|_| PlanError::ExpressionType)?),
                );
                ExpressionAst::new(
                    op,
                    vec![ExprNode::binary(op, BinaryOp::Add, reference, one), state, literal],
                )
            }
            ConversionKind::StateChangeToPulse => {
                return Err(PlanError::Unsupported(
                    "상태 머신(IX.5)에 상태 변화 trigger가 없어 상태 변화→펄스 변환을 낮출 곳이 없습니다",
                ))
            }
        }
        .map_err(|_| PlanError::ExpressionType)?;
        let spec = TransitionSpec {
            id: *transition,
            trigger: Trigger::Event {
                kind: *event,
                source: *source,
            },
            condition: None,
            actions: vec![ActionSpec::SetState {
                variable: *variable,
                value: ast,
            }],
        };
        Ok(vec![
            self.machine_step(|_, transitions| transitions.push(spec))?
        ])
    }

    fn delete_node(&self, node: GraphNodeId) -> Result<Vec<PrototypeStep>, PlanError> {
        let id = node.id;
        let step = match node.kind {
            PrototypeNodeKind::PropertyOutput => {
                let binding = BindingId::new(id);
                self.doc
                    .bindings
                    .get(binding)
                    .ok_or(PlanError::Missing("binding"))?;
                PrototypeStep::Binding {
                    component: self.component,
                    command: BindingCommand::Delete(binding),
                }
            }
            PrototypeNodeKind::MotionSpec => {
                let motion = MotionSpecId::new(id);
                self.motion(motion)?;
                PrototypeStep::Motion(MotionCommand::Delete(motion))
            }
            PrototypeNodeKind::Transition | PrototypeNodeKind::Conversion(_) => {
                let transition = TransitionId::new(id);
                if self
                    .entry()?
                    .state_machine()
                    .transition(transition)
                    .is_none()
                {
                    return Err(PlanError::Missing("transition"));
                }
                self.machine_step(|_, transitions| transitions.retain(|t| t.id != transition))?
            }
            PrototypeNodeKind::StateVariable => {
                let variable = StateVariableId::new(id);
                self.state_type(variable)?;
                let target = ReferenceTarget::Property(state_value_path(variable));
                if self.doc.bindings.iter().any(|b| b.refers_to(&target))
                    || self
                        .doc
                        .motions
                        .owned_by(self.owner())
                        .any(|m| m.start().variable() == variable)
                {
                    return Err(PlanError::InUse(
                        "상태 변수를 binding이나 MotionSpec이 씁니다",
                    ));
                }
                self.machine_step(|variables, _| variables.retain(|v| v.id() != variable))?
            }
            PrototypeNodeKind::ExpressionBlock => {
                return Err(PlanError::Unsupported(
                    "expression block은 expression 편집 plan(VIII.3.3)으로 지웁니다",
                ))
            }
            PrototypeNodeKind::EventSource
            | PrototypeNodeKind::SemanticEmit
            | PrototypeNodeKind::ContextRead => {
                return Err(PlanError::Unsupported(
                    "원본에서 파생된 node는 지울 수 없습니다",
                ))
            }
        };
        let mut steps = vec![step];
        // 지운 node의 배치 hint도 함께 풉니다(없는 node의 좌표를 남기지 않고, Undo 1회로 함께 돌아옵니다).
        if self
            .doc
            .hints
            .get(&self.owner())
            .is_some_and(|hints| hints.get(id).is_some())
        {
            steps.push(PrototypeStep::Hint {
                owner: self.owner(),
                node: id,
                point: None,
            });
        }
        Ok(steps)
    }

    /// 편집 하나를 transaction 하나로 낮춥니다. 문서는 바뀌지 않습니다.
    pub fn plan(&self, edit: &GraphEdit) -> Result<PrototypeTransaction, PlanError> {
        let (label, steps) = match edit {
            GraphEdit::Connect {
                from,
                to,
                binding,
                reference_node,
            } => ("연결", self.connect(from, to, *binding, *reference_node)?),
            GraphEdit::Disconnect { from, to } => ("연결 해제", self.disconnect(from, to)?),
            GraphEdit::AddConversion { .. } => ("변환 node 추가", self.add_conversion(edit)?),
            GraphEdit::AddStateVariable(variable) => {
                let variable = variable.clone();
                (
                    "상태 변수 추가",
                    vec![self.machine_step(|variables, _| variables.push(variable))?],
                )
            }
            GraphEdit::AddMotion(spec) => {
                if spec.owner() != self.owner() {
                    return Err(PlanError::Missing("소유자가 다른 MotionSpec"));
                }
                (
                    "MotionSpec 추가",
                    vec![PrototypeStep::Motion(MotionCommand::Create(spec.clone()))],
                )
            }
            GraphEdit::DeleteNode(node) => ("node 삭제", self.delete_node(*node)?),
            GraphEdit::PinNode { node, point } => (
                "node 위치 고정",
                vec![PrototypeStep::Hint {
                    owner: self.owner(),
                    node: *node,
                    point: *point,
                }],
            ),
        };
        Ok(PrototypeTransaction::new(label, steps))
    }
}
