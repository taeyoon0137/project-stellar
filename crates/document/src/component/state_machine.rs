//! 컴포넌트 선언형 상태 머신 (IX.5).
//!
//! 컴포넌트는 GUI로 작성하는 선언형 상태 머신을 가질 수 있습니다(기술 아키텍처 15.3). 임의 코드는
//! 실행하지 않습니다. 이 모듈은 저장되는 **정의**만 다룹니다 — preview 중 바뀌는 값은 세션 메모리의
//! runtime store(IX.5.4)에만 있고 프로젝트에 저장하지 않습니다.
//!
//! - **상태 변수**([`StateVariable`], IX.5.1): StableId, 이름, 타입, initial expression. initial은 이
//!   컴포넌트의 Property만 참조할 수 있고(인스턴스별 초기값), 추론 타입이 선언 타입에 대입되지 않으면
//!   거부합니다.
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

use super::codec::{put_count, put_id, put_str, Malformed, Reader};
use super::definition::{ComponentEntry, ComponentLibrary};
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

/// 컴포넌트 하나의 상태 머신 정의.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StateMachine {
    variables: BTreeMap<StateVariableId, StateVariable>,
}

impl StateMachine {
    /// 빈 상태 머신.
    pub fn new() -> StateMachine {
        StateMachine::default()
    }

    /// 아무것도 정의하지 않았는지.
    pub fn is_empty(&self) -> bool {
        self.variables.is_empty()
    }

    /// 상태 변수를 추가합니다. id·이름 중복은 거부합니다.
    pub fn add_variable(&mut self, variable: StateVariable) -> Result<(), StateMachineError> {
        if self.variables.contains_key(&variable.id) {
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
        put_count(buf, self.variables.len());
        for variable in self.variables.values() {
            variable.encode_into(buf);
        }
    }

    pub(super) fn decode_from(
        reader: &mut Reader<'_>,
        entry: &ComponentEntry,
    ) -> Result<StateMachine, StateMachineError> {
        let scope = PropertyScope::new(entry);
        let mut machine = StateMachine::new();
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
        Ok(machine)
    }

    /// 정의 `entry`에 대해 검증합니다: initial expression을 그 컴포넌트 Property 환경으로 다시
    /// type 검사합니다.
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
}
