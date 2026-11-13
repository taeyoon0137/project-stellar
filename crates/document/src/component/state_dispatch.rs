//! 이벤트 dispatch·조건 평가·action commit (IX.5.5).
//!
//! 한 이벤트의 처리는 **원자적**입니다:
//!
//! 1. 이벤트 payload가 이벤트 종류(IX.5.2)와 맞는지 확인합니다.
//! 2. 인스턴스의 현재 상태를 snapshot으로 고정합니다.
//! 3. 그 이벤트를 trigger로 가진 transition을 **목록 순서(우선순위)** 로 보며 조건을 평가합니다. 조건이 없으면
//!    참입니다. 처음 참인 transition 하나만 실행합니다(결정론).
//! 4. 그 transition의 모든 `SetState` 값을 **snapshot 기준으로** 평가합니다. 앞 action의 결과가 뒤 action의
//!    입력이 되지 않으므로 action 순서가 값 계산을 바꾸지 않습니다. emit 목록은 action 순서를 따릅니다.
//! 5. 모든 값이 준비된 뒤에만 store에 한꺼번에 commit합니다.
//!
//! 조건·값 평가 중 하나라도 오류(0으로 나누기, 한도 초과 등)면 상태를 전혀 바꾸지 않고 오류를 돌려줍니다.

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::reference::ComponentRef;
use stellar_core_types::typed_value::TypedValue;
use stellar_expressions::ast::ReferenceTarget;
use stellar_expressions::eval::EvalError;

use super::definition::ComponentLibrary;
use super::state_machine::{
    event_payload_path, state_value_path, SemanticActionId, StateVariableId, TransitionAction,
    TransitionId, Trigger,
};
use super::state_store::{InstanceKey, PreviewSessionId, StateStore, StateStoreError};

/// dispatch 결과.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DispatchOutcome {
    /// 실행한 transition(없으면 반응한 transition 없음).
    pub fired: Option<TransitionId>,
    /// emit한 semantic action(action 순서).
    pub emitted: Vec<SemanticActionId>,
    /// commit한 상태 값(action 순서).
    pub committed: Vec<(StateVariableId, Option<TypedValue>)>,
}

/// dispatch 오류. 어떤 오류든 상태는 바뀌지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchError {
    /// store·인스턴스 오류.
    Store(StateStoreError),
    /// 이벤트 payload가 이벤트 종류와 맞지 않습니다.
    PayloadMismatch {
        /// 이벤트.
        event: Trigger,
    },
    /// 조건식 평가 오류.
    Condition {
        /// transition.
        transition: TransitionId,
        /// 원인.
        error: Box<EvalError>,
    },
    /// set 값 평가 오류.
    Action {
        /// transition.
        transition: TransitionId,
        /// 변수.
        variable: StateVariableId,
        /// 원인.
        error: Box<EvalError>,
    },
}

impl fmt::Display for DispatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DispatchError::Store(err) => write!(f, "{err}"),
            DispatchError::PayloadMismatch { event } => {
                write!(f, "이벤트 payload가 이벤트 종류와 맞지 않습니다: {event:?}")
            }
            DispatchError::Condition { transition, error } => write!(
                f,
                "transition {} 조건 평가 실패: {error}",
                transition.stable_id()
            ),
            DispatchError::Action {
                transition,
                variable,
                error,
            } => write!(
                f,
                "transition {}의 상태 변수 {} 값 평가 실패: {error}",
                transition.stable_id(),
                variable.stable_id()
            ),
        }
    }
}

impl std::error::Error for DispatchError {}

impl From<StateStoreError> for DispatchError {
    fn from(err: StateStoreError) -> DispatchError {
        DispatchError::Store(err)
    }
}

/// 이벤트 하나를 인스턴스 상태 머신에 dispatch합니다.
///
/// `event`는 trigger와 같은 형식의 이벤트 식별(`Event { kind, source }` 또는 nested action)이고
/// `payload`는 이벤트 값입니다. `properties`는 인스턴스의 유효 Property 값입니다
/// ([`effective_property_values`](super::state_store::effective_property_values)).
#[allow(clippy::too_many_arguments)]
pub fn dispatch(
    store: &mut StateStore,
    session: PreviewSessionId,
    library: &ComponentLibrary,
    component: ComponentRef,
    key: &InstanceKey,
    properties: &BTreeMap<ReferenceTarget, Option<TypedValue>>,
    event: Trigger,
    payload: Option<TypedValue>,
) -> Result<DispatchOutcome, DispatchError> {
    let accepted = match event {
        Trigger::Event { kind, .. } => kind.accepts_payload(payload.as_ref()),
        Trigger::NestedAction { .. } => payload.is_none(),
    };
    if !accepted {
        return Err(DispatchError::PayloadMismatch { event });
    }
    let entry = library
        .get(component)
        .ok_or(StateStoreError::ComponentNotFound { component })?;
    let snapshot = store
        .state(session, key)
        .ok_or_else(|| StateStoreError::UnknownInstance {
            instance: key.clone(),
        })?
        .clone();

    // 평가 입력: 유효 Property 값 + snapshot 상태 값 + 이벤트 payload(transition별 경로).
    let mut values = properties.clone();
    for (variable, value) in &snapshot {
        values.insert(
            ReferenceTarget::Property(state_value_path(*variable)),
            value.clone(),
        );
    }

    let machine = entry.state_machine();
    let mut fired = None;
    for transition in machine.transitions() {
        if transition.trigger() != event {
            continue;
        }
        let payload_target = ReferenceTarget::Property(event_payload_path(transition.id()));
        if transition.payload_type().is_some() {
            values.insert(payload_target.clone(), payload.clone());
        }
        let truth = match transition.condition() {
            None => true,
            Some(condition) => {
                let value =
                    condition
                        .evaluate(&values)
                        .map_err(|error| DispatchError::Condition {
                            transition: transition.id(),
                            error: Box::new(error),
                        })?;
                value == Some(TypedValue::Boolean(true))
            }
        };
        if truth {
            fired = Some(transition);
            break;
        }
        values.remove(&payload_target);
    }
    let Some(transition) = fired else {
        return Ok(DispatchOutcome::default());
    };

    let mut committed = Vec::new();
    let mut emitted = Vec::new();
    for action in transition.actions() {
        match action {
            TransitionAction::SetState { variable, value } => {
                let result = value
                    .evaluate(&values)
                    .map_err(|error| DispatchError::Action {
                        transition: transition.id(),
                        variable: *variable,
                        error: Box::new(error),
                    })?;
                committed.push((*variable, result));
            }
            TransitionAction::Emit { action } => emitted.push(*action),
        }
    }
    store.commit(session, library, component, key, &committed)?;
    Ok(DispatchOutcome {
        fired: Some(transition.id()),
        emitted,
        committed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::event::EventKind;
    use crate::component::instance::ComponentInstancePayload;
    use crate::component::state_machine::fixtures::*;
    use crate::component::state_machine::{
        ActionSpec, PropertyScope, StateMachine, StateVariable, TransitionSpec,
    };
    use crate::component::state_store::effective_property_values;
    use crate::component::typed_expr::test_util::{binary, literal};
    use stellar_core_types::value::FiniteF32;
    use stellar_expressions::ops::BinaryOp;
    use stellar_expressions::types::{ExpressionType, TypeSpec};

    const COUNT: u64 = 904;
    const WAS_ON: u64 = 905;

    fn number(v: f32) -> TypedValue {
        TypedValue::Number(FiniteF32::new(v).unwrap())
    }

    fn activate() -> Trigger {
        Trigger::Event {
            kind: EventKind::Activate,
            source: stable(KNOB),
        }
    }

    /// Toggle 상태 머신 + count(Number)·wasOn(Boolean) 변수, 주어진 transition들.
    fn setup(
        specs: Vec<TransitionSpec>,
    ) -> (ComponentLibrary, StateStore, PreviewSessionId, InstanceKey) {
        let mut library = toggle_library();
        let mut machine: StateMachine = toggle_machine(&library);
        {
            let entry = library.get(toggle()).unwrap();
            machine
                .add_variable(
                    StateVariable::new(
                        var(COUNT),
                        "count".into(),
                        TypeSpec::required(ExpressionType::Number),
                        literal(number(1.0)),
                        &PropertyScope::new(entry),
                    )
                    .unwrap(),
                )
                .unwrap();
            machine
                .add_variable(
                    StateVariable::new(
                        var(WAS_ON),
                        "wasOn".into(),
                        TypeSpec::required(ExpressionType::Boolean),
                        literal(TypedValue::Boolean(false)),
                        &PropertyScope::new(entry),
                    )
                    .unwrap(),
                )
                .unwrap();
            for spec in specs {
                machine.add_transition(entry, spec).unwrap();
            }
        }
        library.set_state_machine(toggle(), machine).unwrap();
        let session = crate::component::state_store::PreviewSessionId::new(stable(1));
        let key = InstanceKey::scene(stable(1001));
        let mut store = StateStore::new();
        store.open_session(session).unwrap();
        let properties = properties(&library);
        store
            .initialize(session, &library, toggle(), key.clone(), &properties)
            .unwrap();
        (library, store, session, key)
    }

    fn properties(library: &ComponentLibrary) -> BTreeMap<ReferenceTarget, Option<TypedValue>> {
        effective_property_values(
            library.get(toggle()).unwrap(),
            &ComponentInstancePayload::new(toggle()),
        )
    }

    #[test]
    fn first_matching_transition_fires_and_commits() {
        // 우선순위: 조건 isOn == true 인 transition(960)이 먼저, 조건 없는 flip(961)이 다음.
        let mut guarded = flip_spec(
            960,
            Some(binary(
                BinaryOp::Equal,
                state(IS_ON),
                literal(TypedValue::Boolean(true)),
            )),
        );
        guarded.actions = vec![ActionSpec::SetState {
            variable: var(TEXT),
            value: literal(TypedValue::String("이미 켜짐".into())),
        }];
        let (library, mut store, session, key) = setup(vec![guarded, flip_spec(961, None)]);
        let props = properties(&library);
        // isOn = false → 960 조건 거짓, 961 실행.
        let outcome = dispatch(
            &mut store,
            session,
            &library,
            toggle(),
            &key,
            &props,
            activate(),
            None,
        )
        .unwrap();
        assert_eq!(outcome.fired, Some(transition_id(961)));
        assert_eq!(outcome.emitted, vec![action_id(TOGGLED)]);
        assert_eq!(
            store.value(session, &key, var(IS_ON)),
            Some(&Some(TypedValue::Boolean(true)))
        );
        // isOn = true → 960이 먼저 실행(961은 실행 안 됨).
        let outcome = dispatch(
            &mut store,
            session,
            &library,
            toggle(),
            &key,
            &props,
            activate(),
            None,
        )
        .unwrap();
        assert_eq!(outcome.fired, Some(transition_id(960)));
        assert!(outcome.emitted.is_empty());
        assert_eq!(
            store.value(session, &key, var(IS_ON)),
            Some(&Some(TypedValue::Boolean(true)))
        );
        // 반응하는 transition이 없는 이벤트.
        let other = Trigger::Event {
            kind: EventKind::Focus,
            source: stable(KNOB),
        };
        assert_eq!(
            dispatch(
                &mut store,
                session,
                &library,
                toggle(),
                &key,
                &props,
                other,
                None
            )
            .unwrap(),
            DispatchOutcome::default()
        );
    }

    #[test]
    fn error_during_actions_leaves_state_unchanged() {
        // 검증 기준(IX.5.5): action 중 오류 시 state 변화 없음.
        // isOn 반전(성공) 뒤 count = count / 0(실패).
        let mut spec = flip_spec(960, None);
        spec.actions.push(ActionSpec::SetState {
            variable: var(COUNT),
            value: binary(
                BinaryOp::Divide,
                crate::component::state_machine::fixtures::state(COUNT),
                literal(number(0.0)),
            ),
        });
        let (library, mut store, session, key) = setup(vec![spec]);
        let before = store.state(session, &key).cloned();
        let props = properties(&library);
        let err = dispatch(
            &mut store,
            session,
            &library,
            toggle(),
            &key,
            &props,
            activate(),
            None,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            DispatchError::Action { variable, .. } if variable == var(COUNT)
        ));
        assert_eq!(
            store.state(session, &key).cloned(),
            before,
            "앞 action 결과도 반영되지 않음"
        );
    }

    #[test]
    fn condition_error_and_payload_mismatch_leave_state_unchanged() {
        // 조건 평가 오류(1 / 0 > 0).
        let condition = binary(
            BinaryOp::Greater,
            binary(BinaryOp::Divide, literal(number(1.0)), literal(number(0.0))),
            literal(number(0.0)),
        );
        let (library, mut store, session, key) = setup(vec![flip_spec(960, Some(condition))]);
        let before = store.state(session, &key).cloned();
        let props = properties(&library);
        assert!(matches!(
            dispatch(
                &mut store,
                session,
                &library,
                toggle(),
                &key,
                &props,
                activate(),
                None
            ),
            Err(DispatchError::Condition { .. })
        ));
        // Activate에 payload를 실으면 거부.
        assert!(matches!(
            dispatch(
                &mut store,
                session,
                &library,
                toggle(),
                &key,
                &props,
                activate(),
                Some(TypedValue::String("x".into()))
            ),
            Err(DispatchError::PayloadMismatch { .. })
        ));
        assert_eq!(store.state(session, &key).cloned(), before);
    }

    #[test]
    fn values_are_computed_from_the_pre_event_snapshot() {
        // isOn = not isOn 다음 wasOn = isOn: snapshot 기준이면 wasOn은 이전 값(false)입니다
        // (순차 적용이었다면 새 값 true가 됐을 것).
        let mut spec = flip_spec(960, None);
        spec.actions.push(ActionSpec::SetState {
            variable: var(WAS_ON),
            value: crate::component::state_machine::fixtures::state(IS_ON),
        });
        let (library, mut store, session, key) = setup(vec![spec]);
        let props = properties(&library);
        let outcome = dispatch(
            &mut store,
            session,
            &library,
            toggle(),
            &key,
            &props,
            activate(),
            None,
        )
        .unwrap();
        assert_eq!(
            outcome.committed,
            vec![
                (var(IS_ON), Some(TypedValue::Boolean(true))),
                (var(WAS_ON), Some(TypedValue::Boolean(false))),
            ]
        );
        // Input payload는 그 transition의 payload 경로로 읽힙니다.
        let input = TransitionSpec {
            id: transition_id(970),
            trigger: Trigger::Event {
                kind: EventKind::Input,
                source: stable(302),
            },
            condition: None,
            actions: vec![ActionSpec::SetState {
                variable: var(TEXT),
                value: crate::component::typed_expr::test_util::reference(
                    ReferenceTarget::Property(event_payload_path(transition_id(970))),
                ),
            }],
        };
        let (library, mut store, session, key) = setup(vec![input]);
        let props = properties(&library);
        let event = Trigger::Event {
            kind: EventKind::Input,
            source: stable(302),
        };
        dispatch(
            &mut store,
            session,
            &library,
            toggle(),
            &key,
            &props,
            event,
            Some(TypedValue::String("안녕".into())),
        )
        .unwrap();
        assert_eq!(
            store.value(session, &key, var(TEXT)),
            Some(&Some(TypedValue::String("안녕".into())))
        );
    }
}
