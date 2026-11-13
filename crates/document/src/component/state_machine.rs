//! 컴포넌트 선언형 상태 머신 (IX.5).
//!
//! 컴포넌트는 GUI로 작성하는 선언형 상태 머신을 가질 수 있습니다(기술 아키텍처 15.3). 임의 코드는
//! 실행하지 않습니다. 이 모듈은 저장되는 **정의**만 다룹니다 — preview 중 바뀌는 값은 세션 메모리의
//! runtime store(IX.5.4)에만 있고 프로젝트에 저장하지 않습니다.
//!
//! - **상태 변수**([`StateVariable`], IX.5.1): StableId, 이름, 타입, initial expression. initial은 이
//!   컴포넌트의 Property만 참조할 수 있고(인스턴스별 초기값), 추론 타입이 선언 타입에 대입되지 않으면
//!   거부합니다.
//! - **semantic action**([`SemanticAction`], IX.5.3): 상태 머신이 밖으로 알리는 공개 이벤트입니다. 상위
//!   컴포넌트는 nested instance가 emit하는 action에만 반응할 수 있습니다(Event chain, IX.5.6).
//! - **transition**([`Transition`], IX.5.3): source event([`Trigger`]), 선택 조건식(Boolean), 순서 있는
//!   action 목록. action은 닫힌 enum [`TransitionAction`] — 상태 변수 set과 semantic action emit 두 가지뿐이며
//!   파일·네트워크·임의 코드 action은 표현할 수 없습니다(저장 tag도 둘뿐이고 다른 tag는 거부). 조건식·값
//!   표현식 역시 닫힌 표현식 AST(VIII.1.5)라 I/O를 표현할 수 없습니다. 목록 순서가 우선순위입니다.
//!
//! 표현식에서 상태 변수 값은 `ReferenceTarget::Property(`[`state_value_path`]`(var))`로 참조합니다.
//! 컴포넌트 Property는 `ReferenceTarget::ComponentProperty(id)`입니다.

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_core_types::reference::ComponentRef;
use stellar_expressions::ast::{ExpressionAst, ReferenceTarget};
use stellar_expressions::codec::CodecError;
use stellar_expressions::typecheck::ReferenceTypes;
use stellar_expressions::types::TypeSpec;

use super::codec::{put_count, put_id, put_str, put_u8, Malformed, Reader};
use super::definition::{ComponentEntry, ComponentLibrary};
use super::event::EventKind;
use super::property::{validate_property_name, ComponentPropertyError};
use super::typed_expr::{put_expression, put_type_spec, CheckedExpression, ExpressionCheckError};

/// 상태 머신 속성 key(0x0Bxx). 숫자 값은 표현식 참조 경로의 일부입니다.
pub mod keys {
    use stellar_core_types::property::PropertyKey;

    /// 상태 변수의 현재 값(`PropertyPath` root = 변수 StableId).
    pub const STATE_VALUE: PropertyKey = PropertyKey(0x0B01);
    /// 이벤트 payload 값(`PropertyPath` root = transition StableId, IX.5.3).
    pub const EVENT_PAYLOAD: PropertyKey = PropertyKey(0x0B02);
}

/// 상태 변수 식별자.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StateVariableId(StableId);

impl StateVariableId {
    /// `StableId`로부터 만듭니다.
    pub const fn new(id: StableId) -> StateVariableId {
        StateVariableId(id)
    }

    /// 내부 `StableId`.
    pub const fn stable_id(&self) -> StableId {
        self.0
    }
}

/// 상태 변수 값을 가리키는 표현식 참조 경로.
pub fn state_value_path(variable: StateVariableId) -> PropertyPath {
    PropertyPath::with_keys(variable.stable_id(), vec![keys::STATE_VALUE])
}

/// 표현식 참조 대상이 상태 변수 값이면 그 변수.
pub fn state_reference(target: &ReferenceTarget) -> Option<StateVariableId> {
    match target {
        ReferenceTarget::Property(path)
            if path.keys() == [keys::STATE_VALUE] && path.tail().is_none() =>
        {
            Some(StateVariableId::new(path.root()))
        }
        _ => None,
    }
}

/// 상태 머신 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateMachineError {
    /// 이름이 잘못됐습니다.
    InvalidName {
        /// 원인.
        reason: &'static str,
    },
    /// 같은 이름의 상태 변수가 있습니다.
    DuplicateName {
        /// 이름.
        name: String,
    },
    /// 같은 id가 이미 있습니다.
    DuplicateId {
        /// id.
        id: StableId,
    },
    /// initial expression의 타입이 선언 타입과 맞지 않습니다.
    InitialTypeMismatch {
        /// 변수.
        variable: StateVariableId,
        /// 선언 타입.
        expected: TypeSpec,
        /// 추론 타입.
        found: TypeSpec,
    },
    /// initial expression이 잘못됐습니다(알 수 없는 참조 포함).
    InitialInvalid {
        /// 변수.
        variable: StateVariableId,
        /// 원인.
        error: ExpressionCheckError,
    },
    /// 컴포넌트가 library에 없습니다.
    ComponentNotFound {
        /// 컴포넌트.
        component: ComponentRef,
    },
    /// 이 상태 머신에 없는 상태 변수를 set합니다.
    UnknownVariable {
        /// transition.
        transition: TransitionId,
        /// 변수.
        variable: StateVariableId,
    },
    /// 이 상태 머신이 선언하지 않은 semantic action을 emit합니다.
    UnknownAction {
        /// transition.
        transition: TransitionId,
        /// action.
        action: SemanticActionId,
    },
    /// 한 transition이 같은 상태 변수를 두 번 set합니다(결과 모호).
    DuplicateStateWrite {
        /// transition.
        transition: TransitionId,
        /// 변수.
        variable: StateVariableId,
    },
    /// 조건식이 Boolean이 아니거나 잘못됐습니다.
    ConditionInvalid {
        /// transition.
        transition: TransitionId,
        /// 원인.
        error: ExpressionCheckError,
    },
    /// set 값 표현식이 변수 타입과 맞지 않거나 잘못됐습니다.
    ActionValueInvalid {
        /// transition.
        transition: TransitionId,
        /// 변수.
        variable: StateVariableId,
        /// 원인.
        error: ExpressionCheckError,
    },
    /// 표현식 bytes가 잘못됐습니다.
    Codec(CodecError),
    /// 입력이 부족하거나 형식이 아닙니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
}

impl fmt::Display for StateMachineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateMachineError::InvalidName { reason } => {
                write!(f, "상태 머신 이름이 잘못됐습니다: {reason}")
            }
            StateMachineError::DuplicateName { name } => {
                write!(f, "같은 이름의 상태 변수가 이미 있습니다: {name}")
            }
            StateMachineError::DuplicateId { id } => write!(f, "중복 StableId: {id}"),
            StateMachineError::InitialTypeMismatch {
                variable,
                expected,
                found,
            } => write!(
                f,
                "상태 변수 {} initial 타입 {found}가 선언 타입 {expected}와 맞지 않습니다",
                variable.stable_id()
            ),
            StateMachineError::InitialInvalid { variable, error } => {
                write!(f, "상태 변수 {} initial: {error}", variable.stable_id())
            }
            StateMachineError::ComponentNotFound { component } => {
                write!(f, "컴포넌트 {}가 없습니다", component.id())
            }
            StateMachineError::UnknownVariable {
                transition,
                variable,
            } => write!(
                f,
                "transition {}이 없는 상태 변수 {}를 set합니다",
                transition.stable_id(),
                variable.stable_id()
            ),
            StateMachineError::UnknownAction { transition, action } => write!(
                f,
                "transition {}이 선언되지 않은 action {}를 emit합니다",
                transition.stable_id(),
                action.stable_id()
            ),
            StateMachineError::DuplicateStateWrite {
                transition,
                variable,
            } => write!(
                f,
                "transition {}이 상태 변수 {}를 두 번 set합니다",
                transition.stable_id(),
                variable.stable_id()
            ),
            StateMachineError::ConditionInvalid { transition, error } => {
                write!(f, "transition {} 조건: {error}", transition.stable_id())
            }
            StateMachineError::ActionValueInvalid {
                transition,
                variable,
                error,
            } => write!(
                f,
                "transition {}의 상태 변수 {} 값: {error}",
                transition.stable_id(),
                variable.stable_id()
            ),
            StateMachineError::Codec(err) => write!(f, "상태 머신 표현식 bytes 오류: {err}"),
            StateMachineError::Malformed { reason } => {
                write!(f, "상태 머신 binary 형식 위반: {reason}")
            }
        }
    }
}

impl std::error::Error for StateMachineError {}

impl From<Malformed> for StateMachineError {
    fn from(err: Malformed) -> StateMachineError {
        StateMachineError::Malformed { reason: err.0 }
    }
}

fn check_name(name: &str) -> Result<(), StateMachineError> {
    validate_property_name(name).map_err(|err| match err {
        ComponentPropertyError::InvalidName { reason } => StateMachineError::InvalidName { reason },
        _ => StateMachineError::InvalidName {
            reason: "이름 규칙 위반",
        },
    })
}

/// 컴포넌트 Property만 보이는 참조 타입 환경(initial expression용).
pub struct PropertyScope<'a> {
    entry: &'a ComponentEntry,
}

impl<'a> PropertyScope<'a> {
    /// 컴포넌트 `entry`의 Property 환경.
    pub fn new(entry: &'a ComponentEntry) -> PropertyScope<'a> {
        PropertyScope { entry }
    }
}

impl ReferenceTypes for PropertyScope<'_> {
    fn reference_type(&self, target: &ReferenceTarget) -> Option<TypeSpec> {
        match target {
            ReferenceTarget::ComponentProperty(id) => self
                .entry
                .property(super::definition::ComponentPropertyId::new(*id))
                .map(|p| p.value_type()),
            _ => None,
        }
    }
}

/// 컴포넌트 local 상태 변수 (IX.5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateVariable {
    id: StateVariableId,
    name: String,
    initial: CheckedExpression,
}

impl StateVariable {
    /// 상태 변수를 만듭니다. initial은 `scope`(보통 [`PropertyScope`])로 type 검사하고 root 타입이
    /// `value_type`에 대입되지 않으면 거부합니다.
    pub fn new(
        id: StateVariableId,
        name: String,
        value_type: TypeSpec,
        initial: ExpressionAst,
        scope: &dyn ReferenceTypes,
    ) -> Result<StateVariable, StateMachineError> {
        check_name(&name)?;
        let initial =
            CheckedExpression::new(initial, value_type, scope).map_err(|error| match error {
                ExpressionCheckError::TypeMismatch { expected, found } => {
                    StateMachineError::InitialTypeMismatch {
                        variable: id,
                        expected,
                        found,
                    }
                }
                error => StateMachineError::InitialInvalid {
                    variable: id,
                    error,
                },
            })?;
        Ok(StateVariable { id, name, initial })
    }

    /// 식별자.
    pub fn id(&self) -> StateVariableId {
        self.id
    }

    /// 이름(식별에 사용 금지).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 선언 타입.
    pub fn value_type(&self) -> TypeSpec {
        self.initial.declared_type()
    }

    /// initial expression.
    pub fn initial(&self) -> &CheckedExpression {
        &self.initial
    }

    fn encode_into(&self, buf: &mut Vec<u8>) {
        put_id(buf, self.id.stable_id());
        put_str(buf, &self.name);
        put_type_spec(buf, self.value_type());
        put_expression(buf, &self.initial);
    }

    fn decode_from(
        reader: &mut Reader<'_>,
        scope: &dyn ReferenceTypes,
    ) -> Result<StateVariable, StateMachineError> {
        let id = StateVariableId::new(reader.id()?);
        let name = reader.string()?;
        let value_type = reader.type_spec()?;
        let initial = reader.expression_ast()?.map_err(StateMachineError::Codec)?;
        StateVariable::new(id, name, value_type, initial, scope)
    }
}

/// semantic action 식별자.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SemanticActionId(StableId);

impl SemanticActionId {
    /// `StableId`로부터 만듭니다.
    pub const fn new(id: StableId) -> SemanticActionId {
        SemanticActionId(id)
    }

    /// 내부 `StableId`.
    pub const fn stable_id(&self) -> StableId {
        self.0
    }
}

/// 상태 머신이 밖으로 알리는 공개 semantic action 선언 (IX.5.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticAction {
    id: SemanticActionId,
    name: String,
}

impl SemanticAction {
    /// action을 선언합니다.
    pub fn new(id: SemanticActionId, name: String) -> Result<SemanticAction, StateMachineError> {
        check_name(&name)?;
        Ok(SemanticAction { id, name })
    }

    /// 식별자.
    pub fn id(&self) -> SemanticActionId {
        self.id
    }

    /// 이름(식별에 사용 금지).
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// transition 식별자.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TransitionId(StableId);

impl TransitionId {
    /// `StableId`로부터 만듭니다.
    pub const fn new(id: StableId) -> TransitionId {
        TransitionId(id)
    }

    /// 내부 `StableId`.
    pub const fn stable_id(&self) -> StableId {
        self.0
    }
}

/// transition의 이벤트 payload 값을 가리키는 표현식 참조 경로.
pub fn event_payload_path(transition: TransitionId) -> PropertyPath {
    PropertyPath::with_keys(transition.stable_id(), vec![keys::EVENT_PAYLOAD])
}

/// transition source event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Trigger {
    /// 이 컴포넌트 정의 tree node에서 난 이벤트.
    Event {
        /// 이벤트 종류.
        kind: EventKind,
        /// 이벤트를 내는 정의 tree node.
        source: StableId,
    },
    /// 정의 tree 안 nested instance가 emit한 공개 semantic action(Event chain).
    NestedAction {
        /// nested instance node(이 정의 tree의 ComponentInstance).
        instance: StableId,
        /// nested 컴포넌트가 선언한 action.
        action: SemanticActionId,
    },
}

const TRIGGER_EVENT: u8 = 0x01;
const TRIGGER_NESTED_ACTION: u8 = 0x02;
/// 상태 변수 set action tag.
pub const ACTION_SET_STATE: u8 = 0x01;
/// semantic action emit action tag.
pub const ACTION_EMIT: u8 = 0x02;

/// transition action (닫힌 enum: 상태 set과 semantic action emit만).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionAction {
    /// 상태 변수에 값을 set합니다.
    SetState {
        /// 이 상태 머신의 변수.
        variable: StateVariableId,
        /// 값 표현식(변수 타입).
        value: CheckedExpression,
    },
    /// 선언된 semantic action을 emit합니다.
    Emit {
        /// 이 상태 머신이 선언한 action.
        action: SemanticActionId,
    },
}

impl TransitionAction {
    /// 저장 tag.
    pub const fn tag(&self) -> u8 {
        match self {
            TransitionAction::SetState { .. } => ACTION_SET_STATE,
            TransitionAction::Emit { .. } => ACTION_EMIT,
        }
    }
}

/// 검사 전 action 입력.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionSpec {
    /// 상태 변수 set.
    SetState {
        /// 변수.
        variable: StateVariableId,
        /// 값 AST.
        value: ExpressionAst,
    },
    /// semantic action emit.
    Emit {
        /// action.
        action: SemanticActionId,
    },
}

/// 검사 전 transition 입력.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionSpec {
    /// 식별자.
    pub id: TransitionId,
    /// source event.
    pub trigger: Trigger,
    /// 조건식(없으면 항상 참).
    pub condition: Option<ExpressionAst>,
    /// 순서 있는 action.
    pub actions: Vec<ActionSpec>,
}

/// 검사된 transition (IX.5.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    id: TransitionId,
    trigger: Trigger,
    condition: Option<CheckedExpression>,
    actions: Vec<TransitionAction>,
}

impl Transition {
    /// 식별자.
    pub fn id(&self) -> TransitionId {
        self.id
    }

    /// source event.
    pub fn trigger(&self) -> Trigger {
        self.trigger
    }

    /// 조건식(없으면 항상 참).
    pub fn condition(&self) -> Option<&CheckedExpression> {
        self.condition.as_ref()
    }

    /// 순서 있는 action.
    pub fn actions(&self) -> &[TransitionAction] {
        &self.actions
    }

    /// 이 transition에서 쓰는 이벤트 payload 타입(Event trigger이고 payload가 있는 종류일 때).
    pub fn payload_type(&self) -> Option<TypeSpec> {
        match self.trigger {
            Trigger::Event { kind, .. } => kind.payload_type(),
            Trigger::NestedAction { .. } => None,
        }
    }

    /// 검사 전 입력으로 되돌립니다.
    pub fn to_spec(&self) -> TransitionSpec {
        TransitionSpec {
            id: self.id,
            trigger: self.trigger,
            condition: self.condition.as_ref().map(|c| c.ast().clone()),
            actions: self
                .actions
                .iter()
                .map(|action| match action {
                    TransitionAction::SetState { variable, value } => ActionSpec::SetState {
                        variable: *variable,
                        value: value.ast().clone(),
                    },
                    TransitionAction::Emit { action } => ActionSpec::Emit { action: *action },
                })
                .collect(),
        }
    }

    fn encode_into(&self, buf: &mut Vec<u8>) {
        put_id(buf, self.id.stable_id());
        match self.trigger {
            Trigger::Event { kind, source } => {
                put_u8(buf, TRIGGER_EVENT);
                put_u8(buf, kind.tag());
                put_id(buf, source);
            }
            Trigger::NestedAction { instance, action } => {
                put_u8(buf, TRIGGER_NESTED_ACTION);
                put_id(buf, instance);
                put_id(buf, action.stable_id());
            }
        }
        match &self.condition {
            Some(condition) => {
                put_u8(buf, 1);
                put_expression(buf, condition);
            }
            None => put_u8(buf, 0),
        }
        put_count(buf, self.actions.len());
        for action in &self.actions {
            put_u8(buf, action.tag());
            match action {
                TransitionAction::SetState { variable, value } => {
                    put_id(buf, variable.stable_id());
                    put_expression(buf, value);
                }
                TransitionAction::Emit { action } => put_id(buf, action.stable_id()),
            }
        }
    }

    fn decode_spec(reader: &mut Reader<'_>) -> Result<TransitionSpec, StateMachineError> {
        let id = TransitionId::new(reader.id()?);
        let trigger = match reader.u8()? {
            TRIGGER_EVENT => Trigger::Event {
                kind: EventKind::from_tag(reader.u8()?).map_err(|_| {
                    StateMachineError::Malformed {
                        reason: "알 수 없는 이벤트 종류",
                    }
                })?,
                source: reader.id()?,
            },
            TRIGGER_NESTED_ACTION => Trigger::NestedAction {
                instance: reader.id()?,
                action: SemanticActionId::new(reader.id()?),
            },
            _ => {
                return Err(StateMachineError::Malformed {
                    reason: "알 수 없는 trigger tag",
                })
            }
        };
        let condition = if reader.bool()? {
            Some(reader.expression_ast()?.map_err(StateMachineError::Codec)?)
        } else {
            None
        };
        let count = reader.count()?;
        let mut actions = Vec::with_capacity(super::codec::capacity_hint(count));
        for _ in 0..count {
            actions.push(match reader.u8()? {
                ACTION_SET_STATE => ActionSpec::SetState {
                    variable: StateVariableId::new(reader.id()?),
                    value: reader.expression_ast()?.map_err(StateMachineError::Codec)?,
                },
                ACTION_EMIT => ActionSpec::Emit {
                    action: SemanticActionId::new(reader.id()?),
                },
                // 파일·네트워크 등 다른 action은 표현할 수 없습니다.
                _ => {
                    return Err(StateMachineError::Malformed {
                        reason: "알 수 없는 transition action tag",
                    })
                }
            });
        }
        Ok(TransitionSpec {
            id,
            trigger,
            condition,
            actions,
        })
    }
}

/// transition 표현식의 참조 타입 환경: 컴포넌트 Property, 이 상태 머신의 상태 변수, 이 transition의
/// 이벤트 payload.
pub struct MachineScope<'a> {
    entry: &'a ComponentEntry,
    machine: &'a StateMachine,
    payload: Option<(TransitionId, TypeSpec)>,
}

impl<'a> MachineScope<'a> {
    /// transition `transition`(payload 타입 `payload`)의 환경.
    pub fn new(
        entry: &'a ComponentEntry,
        machine: &'a StateMachine,
        transition: TransitionId,
        payload: Option<TypeSpec>,
    ) -> MachineScope<'a> {
        MachineScope {
            entry,
            machine,
            payload: payload.map(|spec| (transition, spec)),
        }
    }
}

impl ReferenceTypes for MachineScope<'_> {
    fn reference_type(&self, target: &ReferenceTarget) -> Option<TypeSpec> {
        if let Some(variable) = state_reference(target) {
            return self
                .machine
                .variable(variable)
                .map(StateVariable::value_type);
        }
        if let (ReferenceTarget::Property(path), Some((transition, spec))) = (target, self.payload)
        {
            if *path == event_payload_path(transition) {
                return Some(spec);
            }
        }
        PropertyScope::new(self.entry).reference_type(target)
    }
}

/// transition 입력을 상태 머신 `machine`과 정의 `entry`에 대해 검사합니다.
fn check_transition(
    spec: TransitionSpec,
    entry: &ComponentEntry,
    machine: &StateMachine,
) -> Result<Transition, StateMachineError> {
    let transition = spec.id;
    let payload = match spec.trigger {
        Trigger::Event { kind, .. } => kind.payload_type(),
        Trigger::NestedAction { .. } => None,
    };
    let scope = MachineScope::new(entry, machine, transition, payload);
    let condition = spec
        .condition
        .map(|ast| {
            CheckedExpression::new(
                ast,
                TypeSpec::required(stellar_expressions::types::ExpressionType::Boolean),
                &scope,
            )
            .map_err(|error| StateMachineError::ConditionInvalid { transition, error })
        })
        .transpose()?;
    let mut written = std::collections::BTreeSet::new();
    let mut actions = Vec::with_capacity(spec.actions.len());
    for action in spec.actions {
        actions.push(match action {
            ActionSpec::SetState { variable, value } => {
                let declared = machine
                    .variable(variable)
                    .ok_or(StateMachineError::UnknownVariable {
                        transition,
                        variable,
                    })?
                    .value_type();
                if !written.insert(variable) {
                    return Err(StateMachineError::DuplicateStateWrite {
                        transition,
                        variable,
                    });
                }
                let value = CheckedExpression::new(value, declared, &scope).map_err(|error| {
                    StateMachineError::ActionValueInvalid {
                        transition,
                        variable,
                        error,
                    }
                })?;
                TransitionAction::SetState { variable, value }
            }
            ActionSpec::Emit { action } => {
                if machine.action(action).is_none() {
                    return Err(StateMachineError::UnknownAction { transition, action });
                }
                TransitionAction::Emit { action }
            }
        });
    }
    Ok(Transition {
        id: transition,
        trigger: spec.trigger,
        condition,
        actions,
    })
}

/// 컴포넌트 하나의 상태 머신 정의.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StateMachine {
    actions: BTreeMap<SemanticActionId, SemanticAction>,
    variables: BTreeMap<StateVariableId, StateVariable>,
    transitions: Vec<Transition>,
}

impl StateMachine {
    /// 빈 상태 머신.
    pub fn new() -> StateMachine {
        StateMachine::default()
    }

    /// 아무것도 정의하지 않았는지.
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty() && self.variables.is_empty() && self.transitions.is_empty()
    }

    fn id_taken(&self, id: StableId) -> bool {
        self.actions.contains_key(&SemanticActionId::new(id))
            || self.variables.contains_key(&StateVariableId::new(id))
            || self.transitions.iter().any(|t| t.id.stable_id() == id)
    }

    /// semantic action을 선언합니다. id·이름 중복은 거부합니다.
    pub fn add_action(&mut self, action: SemanticAction) -> Result<(), StateMachineError> {
        if self.id_taken(action.id.stable_id()) {
            return Err(StateMachineError::DuplicateId {
                id: action.id.stable_id(),
            });
        }
        if self.actions.values().any(|a| a.name == action.name) {
            return Err(StateMachineError::DuplicateName {
                name: action.name.clone(),
            });
        }
        self.actions.insert(action.id, action);
        Ok(())
    }

    /// 선언된 semantic action.
    pub fn action(&self, id: SemanticActionId) -> Option<&SemanticAction> {
        self.actions.get(&id)
    }

    /// 선언된 semantic action(id 오름차순).
    pub fn actions(&self) -> impl Iterator<Item = &SemanticAction> + '_ {
        self.actions.values()
    }

    /// transition을 검사해 목록 끝(가장 낮은 우선순위)에 추가합니다.
    pub fn add_transition(
        &mut self,
        entry: &ComponentEntry,
        spec: TransitionSpec,
    ) -> Result<(), StateMachineError> {
        if self.id_taken(spec.id.stable_id()) {
            return Err(StateMachineError::DuplicateId {
                id: spec.id.stable_id(),
            });
        }
        let transition = check_transition(spec, entry, self)?;
        self.transitions.push(transition);
        Ok(())
    }

    /// transition(우선순위 순).
    pub fn transitions(&self) -> &[Transition] {
        &self.transitions
    }

    /// transition.
    pub fn transition(&self, id: TransitionId) -> Option<&Transition> {
        self.transitions.iter().find(|t| t.id == id)
    }

    /// 상태 변수를 추가합니다. id·이름 중복은 거부합니다.
    pub fn add_variable(&mut self, variable: StateVariable) -> Result<(), StateMachineError> {
        if self.id_taken(variable.id.stable_id()) {
            return Err(StateMachineError::DuplicateId {
                id: variable.id.stable_id(),
            });
        }
        if self.variables.values().any(|v| v.name == variable.name) {
            return Err(StateMachineError::DuplicateName {
                name: variable.name.clone(),
            });
        }
        self.variables.insert(variable.id, variable);
        Ok(())
    }

    /// 상태 변수.
    pub fn variable(&self, id: StateVariableId) -> Option<&StateVariable> {
        self.variables.get(&id)
    }

    /// 상태 변수(id 오름차순).
    pub fn variables(&self) -> impl Iterator<Item = &StateVariable> + '_ {
        self.variables.values()
    }

    pub(super) fn encode_into(&self, buf: &mut Vec<u8>) {
        put_count(buf, self.actions.len());
        for action in self.actions.values() {
            put_id(buf, action.id.stable_id());
            put_str(buf, &action.name);
        }
        put_count(buf, self.variables.len());
        for variable in self.variables.values() {
            variable.encode_into(buf);
        }
        put_count(buf, self.transitions.len());
        for transition in &self.transitions {
            transition.encode_into(buf);
        }
    }

    pub(super) fn decode_from(
        reader: &mut Reader<'_>,
        entry: &ComponentEntry,
    ) -> Result<StateMachine, StateMachineError> {
        let scope = PropertyScope::new(entry);
        let mut machine = StateMachine::new();
        let action_count = reader.count()?;
        let mut previous_action: Option<SemanticActionId> = None;
        for _ in 0..action_count {
            let action =
                SemanticAction::new(SemanticActionId::new(reader.id()?), reader.string()?)?;
            if previous_action.is_some_and(|p| p >= action.id) {
                return Err(StateMachineError::Malformed {
                    reason: "semantic action이 id 오름차순이 아님",
                });
            }
            previous_action = Some(action.id);
            machine.add_action(action)?;
        }
        let count = reader.count()?;
        let mut previous: Option<StateVariableId> = None;
        for _ in 0..count {
            let variable = StateVariable::decode_from(reader, &scope)?;
            if previous.is_some_and(|p| p >= variable.id) {
                return Err(StateMachineError::Malformed {
                    reason: "상태 변수가 id 오름차순이 아님",
                });
            }
            previous = Some(variable.id);
            machine.add_variable(variable)?;
        }
        let transition_count = reader.count()?;
        for _ in 0..transition_count {
            let spec = Transition::decode_spec(reader)?;
            machine.add_transition(entry, spec)?;
        }
        Ok(machine)
    }

    /// 정의 `entry`에 대해 검증합니다: initial expression을 그 컴포넌트 Property 환경으로, transition을
    /// 이 상태 머신·Property 환경으로 다시 검사합니다.
    pub fn validate(&self, entry: &ComponentEntry) -> Result<(), StateMachineError> {
        let scope = PropertyScope::new(entry);
        for variable in self.variables.values() {
            StateVariable::new(
                variable.id,
                variable.name.clone(),
                variable.value_type(),
                variable.initial.ast().clone(),
                &scope,
            )?;
        }
        for transition in &self.transitions {
            check_transition(transition.to_spec(), entry, self)?;
        }
        Ok(())
    }
}

impl ComponentEntry {
    /// 상태 머신 정의.
    pub fn state_machine(&self) -> &StateMachine {
        self.state_machine_ref()
    }
}

impl ComponentLibrary {
    /// 컴포넌트의 상태 머신을 교체합니다. 정의에 대해 검증하고, 실패하면 library는 변경되지 않습니다.
    pub fn set_state_machine(
        &mut self,
        component: ComponentRef,
        machine: StateMachine,
    ) -> Result<StateMachine, StateMachineError> {
        let entry = self
            .get(component)
            .ok_or(StateMachineError::ComponentNotFound { component })?;
        machine.validate(entry)?;
        let entry = self
            .entry_mut(component)
            .ok_or(StateMachineError::ComponentNotFound { component })?;
        Ok(std::mem::replace(entry.state_machine_mut(), machine))
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! 상태 머신 테스트 공용 fixture.

    use super::*;
    use crate::component::definition::{
        ComponentDefinition, ComponentPropertyId, DefinitionNode, DefinitionTree,
    };
    use crate::component::property::{ComponentProperty, PropertyVisibility};
    use crate::component::typed_expr::test_util::literal;
    use crate::node_kind::NodeKind;
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::reference::NodeRef;
    use stellar_core_types::typed_value::TypedValue;
    use stellar_expressions::types::ExpressionType;

    pub(crate) fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    pub(crate) fn var(ts: u64) -> StateVariableId {
        StateVariableId::new(stable(ts))
    }

    pub(crate) fn toggle() -> ComponentRef {
        ComponentRef::new(stable(3))
    }

    /// Toggle(3) property: 820 initiallyOn(Boolean, public), 821 caption(String, private).
    pub(crate) const INITIALLY_ON: u64 = 820;
    pub(crate) const CAPTION: u64 = 821;
    /// 정의 tree: 300(Frame root) ← 301(Frame knob), 302(Text label).
    pub(crate) const ROOT: u64 = 300;
    pub(crate) const KNOB: u64 = 301;

    pub(crate) fn prop(id: u64) -> ComponentPropertyId {
        ComponentPropertyId::new(stable(id))
    }

    /// Toggle 컴포넌트 하나를 담은 library(상태 머신 없음).
    pub(crate) fn toggle_library() -> ComponentLibrary {
        let mut tree = DefinitionTree::new(toggle());
        for (id, kind, parent, key) in [
            (ROOT, NodeKind::Frame, None, 0x80u8),
            (KNOB, NodeKind::Frame, Some(ROOT), 0x40),
            (302, NodeKind::Text, Some(ROOT), 0x60),
        ] {
            tree.insert(DefinitionNode {
                id: stable(id),
                kind,
                parent: parent.map(stable),
                order_key: OrderKey::from_bytes(vec![key]).unwrap(),
                visible: true,
                locked: false,
                display_name: String::new(),
                payload: Vec::new(),
            })
            .unwrap();
        }
        let mut definition =
            ComponentDefinition::new(stable(3), "Toggle".into(), NodeRef::new(stable(ROOT)));
        definition.add_property(prop(INITIALLY_ON)).unwrap();
        definition.add_property(prop(CAPTION)).unwrap();
        let mut library = ComponentLibrary::new();
        library.insert(definition, tree).unwrap();
        for (id, name, value, visibility) in [
            (
                INITIALLY_ON,
                "initiallyOn",
                TypedValue::Boolean(false),
                PropertyVisibility::Public,
            ),
            (
                CAPTION,
                "caption",
                TypedValue::String("끔".into()),
                PropertyVisibility::Private,
            ),
        ] {
            library
                .add_property(
                    toggle(),
                    ComponentProperty::new(
                        prop(id),
                        name.into(),
                        TypeSpec::required(ExpressionType::of(&value)),
                        literal(value),
                        visibility,
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        library
    }

    /// 상태 변수: 900 isOn(Boolean, initial = initiallyOn), 903 text(String, initial "").
    pub(crate) const IS_ON: u64 = 900;
    pub(crate) const TEXT: u64 = 903;
    /// semantic action: 950 toggled.
    pub(crate) const TOGGLED: u64 = 950;

    pub(crate) fn action_id(ts: u64) -> SemanticActionId {
        SemanticActionId::new(stable(ts))
    }

    pub(crate) fn transition_id(ts: u64) -> TransitionId {
        TransitionId::new(stable(ts))
    }

    /// `var` 값 참조 AST.
    pub(crate) fn state(var_ts: u64) -> ExpressionAst {
        crate::component::typed_expr::test_util::reference(ReferenceTarget::Property(
            state_value_path(var(var_ts)),
        ))
    }

    /// transition 없는 Toggle 상태 머신(변수 2개, action 1개).
    pub(crate) fn toggle_machine(library: &ComponentLibrary) -> StateMachine {
        let entry = library.get(toggle()).unwrap();
        let scope = PropertyScope::new(entry);
        let mut machine = StateMachine::new();
        machine
            .add_variable(
                StateVariable::new(
                    var(IS_ON),
                    "isOn".into(),
                    TypeSpec::required(ExpressionType::Boolean),
                    crate::component::typed_expr::test_util::reference(
                        ReferenceTarget::ComponentProperty(stable(INITIALLY_ON)),
                    ),
                    &scope,
                )
                .unwrap(),
            )
            .unwrap();
        machine
            .add_variable(
                StateVariable::new(
                    var(TEXT),
                    "text".into(),
                    TypeSpec::required(ExpressionType::String),
                    literal(TypedValue::String(String::new())),
                    &scope,
                )
                .unwrap(),
            )
            .unwrap();
        machine
            .add_action(SemanticAction::new(action_id(TOGGLED), "toggled".into()).unwrap())
            .unwrap();
        machine
    }

    /// knob Activate → isOn = not isOn, emit toggled.
    pub(crate) fn flip_spec(id: u64, condition: Option<ExpressionAst>) -> TransitionSpec {
        TransitionSpec {
            id: transition_id(id),
            trigger: Trigger::Event {
                kind: EventKind::Activate,
                source: stable(KNOB),
            },
            condition,
            actions: vec![
                ActionSpec::SetState {
                    variable: var(IS_ON),
                    value: crate::component::typed_expr::test_util::unary(
                        stellar_expressions::ops::UnaryOp::Not,
                        state(IS_ON),
                    ),
                },
                ActionSpec::Emit {
                    action: action_id(TOGGLED),
                },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::component::typed_expr::test_util::{literal, reference};
    use stellar_core_types::typed_value::TypedValue;
    use stellar_expressions::types::ExpressionType;

    fn boolean() -> TypeSpec {
        TypeSpec::required(ExpressionType::Boolean)
    }

    #[test]
    fn initial_type_mismatch_is_rejected() {
        // 검증 기준(IX.5.1): initial type mismatch 거부.
        let library = toggle_library();
        let entry = library.get(toggle()).unwrap();
        let scope = PropertyScope::new(entry);
        assert_eq!(
            StateVariable::new(
                var(900),
                "isOn".into(),
                boolean(),
                literal(TypedValue::String("on".into())),
                &scope,
            )
            .unwrap_err(),
            StateMachineError::InitialTypeMismatch {
                variable: var(900),
                expected: boolean(),
                found: TypeSpec::required(ExpressionType::String),
            }
        );
        // Property를 참조하는 initial도 타입이 다르면 거부(caption은 String).
        assert!(matches!(
            StateVariable::new(
                var(900),
                "isOn".into(),
                boolean(),
                reference(ReferenceTarget::ComponentProperty(stable(CAPTION))),
                &scope,
            ),
            Err(StateMachineError::InitialTypeMismatch { .. })
        ));
        // 알 수 없는 참조(다른 상태 변수·없는 property)는 initial로 쓸 수 없습니다.
        for target in [
            ReferenceTarget::Property(state_value_path(var(901))),
            ReferenceTarget::ComponentProperty(stable(9_999)),
        ] {
            assert!(matches!(
                StateVariable::new(
                    var(900),
                    "isOn".into(),
                    boolean(),
                    reference(target),
                    &scope
                ),
                Err(StateMachineError::InitialInvalid { .. })
            ));
        }
        // 일치하는 initial(property 참조)은 통과.
        let ok = StateVariable::new(
            var(900),
            "isOn".into(),
            boolean(),
            reference(ReferenceTarget::ComponentProperty(stable(INITIALLY_ON))),
            &scope,
        )
        .unwrap();
        assert_eq!(ok.value_type(), boolean());
        // nullable 변수는 필수 타입 initial을 받습니다.
        StateVariable::new(
            var(902),
            "hover".into(),
            TypeSpec::optional(ExpressionType::Boolean),
            literal(TypedValue::Boolean(false)),
            &scope,
        )
        .unwrap();
    }

    #[test]
    fn machine_round_trips_in_library_and_rejects_duplicates() {
        let mut library = toggle_library();
        let entry = library.get(toggle()).unwrap();
        let scope = PropertyScope::new(entry);
        let is_on = StateVariable::new(
            var(900),
            "isOn".into(),
            boolean(),
            reference(ReferenceTarget::ComponentProperty(stable(INITIALLY_ON))),
            &scope,
        )
        .unwrap();
        let pressed = StateVariable::new(
            var(901),
            "pressed".into(),
            boolean(),
            literal(TypedValue::Boolean(false)),
            &scope,
        )
        .unwrap();
        let mut machine = StateMachine::new();
        machine.add_variable(pressed.clone()).unwrap();
        machine.add_variable(is_on.clone()).unwrap();
        assert!(matches!(
            machine.add_variable(is_on.clone()),
            Err(StateMachineError::DuplicateId { .. })
        ));
        let same_name = StateVariable::new(
            var(905),
            "pressed".into(),
            boolean(),
            literal(TypedValue::Boolean(true)),
            &scope,
        )
        .unwrap();
        assert!(matches!(
            machine.add_variable(same_name),
            Err(StateMachineError::DuplicateName { .. })
        ));
        library
            .set_state_machine(toggle(), machine.clone())
            .unwrap();
        let bytes = library.encode();
        let decoded = ComponentLibrary::decode(&bytes).unwrap();
        assert_eq!(decoded, library);
        assert_eq!(decoded.encode(), bytes);
        assert_eq!(
            decoded
                .get(toggle())
                .unwrap()
                .state_machine()
                .variables()
                .count(),
            2
        );
        assert!(library
            .set_state_machine(ComponentRef::new(stable(77)), StateMachine::new())
            .is_err());
    }

    // --- IX.5.3 transition ---

    use crate::component::typed_expr::test_util::binary;
    use stellar_expressions::ops::BinaryOp;

    fn machine_with(spec: TransitionSpec) -> Result<StateMachine, StateMachineError> {
        let library = toggle_library();
        let mut machine = toggle_machine(&library);
        machine.add_transition(library.get(toggle()).unwrap(), spec)?;
        Ok(machine)
    }

    #[test]
    fn transition_has_event_condition_and_ordered_actions() {
        let mut library = toggle_library();
        let mut machine = toggle_machine(&library);
        let entry = library.get(toggle()).unwrap();
        // 조건: isOn == false 일 때만.
        let condition = binary(
            BinaryOp::Equal,
            state(IS_ON),
            literal(TypedValue::Boolean(false)),
        );
        machine
            .add_transition(entry, flip_spec(960, Some(condition)))
            .unwrap();
        // Input 이벤트 payload(String)를 text 변수에 set.
        machine
            .add_transition(
                entry,
                TransitionSpec {
                    id: transition_id(961),
                    trigger: Trigger::Event {
                        kind: EventKind::Input,
                        source: stable(302),
                    },
                    condition: None,
                    actions: vec![ActionSpec::SetState {
                        variable: var(TEXT),
                        value: reference(ReferenceTarget::Property(event_payload_path(
                            transition_id(961),
                        ))),
                    }],
                },
            )
            .unwrap();
        let first = &machine.transitions()[0];
        assert_eq!(first.id(), transition_id(960));
        assert!(first.condition().is_some());
        assert_eq!(
            first
                .actions()
                .iter()
                .map(TransitionAction::tag)
                .collect::<Vec<_>>(),
            vec![ACTION_SET_STATE, ACTION_EMIT],
            "action 순서 보존"
        );
        assert_eq!(
            machine.transitions()[1].payload_type(),
            Some(TypeSpec::required(ExpressionType::String))
        );
        // library round-trip: transition 순서(우선순위)까지 byte-identical.
        library
            .set_state_machine(toggle(), machine.clone())
            .unwrap();
        let bytes = library.encode();
        let decoded = ComponentLibrary::decode(&bytes).unwrap();
        assert_eq!(decoded.get(toggle()).unwrap().state_machine(), &machine);
        assert_eq!(decoded.encode(), bytes);
    }

    #[test]
    fn invalid_transitions_are_rejected() {
        // 조건식이 Boolean이 아님.
        assert!(matches!(
            machine_with(flip_spec(
                960,
                Some(literal(TypedValue::String("x".into())))
            )),
            Err(StateMachineError::ConditionInvalid { .. })
        ));
        // 값 타입 불일치·없는 변수·선언 안 된 action·같은 변수 두 번 set.
        let mut spec = flip_spec(960, None);
        spec.actions = vec![ActionSpec::SetState {
            variable: var(IS_ON),
            value: literal(TypedValue::String("on".into())),
        }];
        assert!(matches!(
            machine_with(spec),
            Err(StateMachineError::ActionValueInvalid { .. })
        ));
        let mut spec = flip_spec(960, None);
        spec.actions = vec![ActionSpec::SetState {
            variable: var(999),
            value: literal(TypedValue::Boolean(true)),
        }];
        assert_eq!(
            machine_with(spec).unwrap_err(),
            StateMachineError::UnknownVariable {
                transition: transition_id(960),
                variable: var(999),
            }
        );
        let mut spec = flip_spec(960, None);
        spec.actions = vec![ActionSpec::Emit {
            action: action_id(998),
        }];
        assert!(matches!(
            machine_with(spec),
            Err(StateMachineError::UnknownAction { .. })
        ));
        let mut spec = flip_spec(960, None);
        spec.actions.push(ActionSpec::SetState {
            variable: var(IS_ON),
            value: literal(TypedValue::Boolean(true)),
        });
        assert!(matches!(
            machine_with(spec),
            Err(StateMachineError::DuplicateStateWrite { .. })
        ));
        // payload 없는 이벤트(Activate)에서 payload 참조는 알 수 없는 참조입니다.
        let mut spec = flip_spec(960, None);
        spec.actions = vec![ActionSpec::SetState {
            variable: var(TEXT),
            value: reference(ReferenceTarget::Property(event_payload_path(
                transition_id(960),
            ))),
        }];
        assert!(matches!(
            machine_with(spec),
            Err(StateMachineError::ActionValueInvalid { .. })
        ));
        // 같은 id 재사용(변수 id와 충돌).
        assert!(matches!(
            machine_with(flip_spec(IS_ON, None)),
            Err(StateMachineError::DuplicateId { .. })
        ));
    }

    #[test]
    fn file_or_network_actions_cannot_be_expressed() {
        // 검증 기준(IX.5.3): file/network action 표현 불가.
        // 1) action은 닫힌 enum 두 갈래뿐입니다(wildcard 없는 match가 컴파일되는 것으로 고정).
        let tags = |action: &TransitionAction| match action {
            TransitionAction::SetState { .. } => ACTION_SET_STATE,
            TransitionAction::Emit { .. } => ACTION_EMIT,
        };
        let machine = machine_with(flip_spec(960, None)).unwrap();
        assert!(machine.transitions()[0]
            .actions()
            .iter()
            .all(|a| tags(a) == a.tag()));
        // 2) 저장 bytes의 action tag를 다른 값으로 바꾸면 디코딩이 거부합니다.
        let library = toggle_library();
        let mut with_machine = library.clone();
        with_machine.set_state_machine(toggle(), machine).unwrap();
        let bytes = with_machine.encode();
        let action_tag_at = {
            // 마지막 action(Emit)의 tag 위치: 끝에서 action id(16) + tag(1).
            bytes.len() - 17
        };
        assert_eq!(bytes[action_tag_at], ACTION_EMIT);
        for tag in [0x00u8, 0x03, 0x10, 0xFF] {
            let mut tampered = bytes.clone();
            tampered[action_tag_at] = tag;
            assert!(
                matches!(
                    ComponentLibrary::decode(&tampered),
                    Err(
                        crate::component::definition::ComponentDefinitionError::StateMachine(
                            StateMachineError::Malformed { .. }
                        )
                    )
                ),
                "tag 0x{tag:02X}"
            );
        }
    }
}
