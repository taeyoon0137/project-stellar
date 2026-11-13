//! nested 컴포넌트 상태 캡슐화 validator (IX.5.6).
//!
//! 상태 머신은 **자기 컴포넌트의** 상태만 읽고 씁니다. nested instance의 상태는 직접 수정할 수 없고,
//! 공개 Property(상위 정의가 drilling, IX.4)와 공개 Event chain(nested instance가 emit하는 semantic action에
//! 반응하는 [`Trigger::NestedAction`])으로만 연결합니다(기술 아키텍처 15.3).
//!
//! 거부하는 직접 경로:
//!
//! - **직접 write**: 다른 컴포넌트 상태 머신의 변수를 `SetState`로 씁니다([`StateEncapsulationError::ForeignStateWrite`]).
//! - **직접 read**: 조건·값 표현식이 다른 컴포넌트 상태 변수를 읽습니다([`StateEncapsulationError::ForeignStateRead`]).
//! - **내부 이벤트 관찰**: trigger source가 다른 컴포넌트 정의 tree의 node(nested instance 내부)입니다
//!   ([`StateEncapsulationError::ForeignEventSource`]).
//! - **인스턴스가 아닌 nested action source**: `NestedAction`의 node가 자기 정의 tree의 ComponentInstance가
//!   아닙니다([`StateEncapsulationError::NotNestedInstance`]).
//!
//! 어떤 컴포넌트에도 없는 node·action(삭제됨, 외부 library 갱신)은 오류가 아니라 IX.5.7 진단(missing
//! target)입니다.

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::ComponentRef;
use stellar_expressions::ast::ExpressionAst;

use super::definition::ComponentLibrary;
use super::state_machine::{
    state_reference, ActionSpec, StateMachineError, StateVariableId, TransitionId, TransitionSpec,
    Trigger,
};
use crate::node_kind::NodeKind;

/// 상태 캡슐화 위반.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateEncapsulationError {
    /// 다른 컴포넌트의 상태 변수를 직접 씁니다.
    ForeignStateWrite {
        /// transition.
        transition: TransitionId,
        /// 변수.
        variable: StateVariableId,
        /// 변수를 소유한 컴포넌트.
        owner: ComponentRef,
    },
    /// 다른 컴포넌트의 상태 변수를 직접 읽습니다.
    ForeignStateRead {
        /// transition.
        transition: TransitionId,
        /// 변수.
        variable: StateVariableId,
        /// 변수를 소유한 컴포넌트.
        owner: ComponentRef,
    },
    /// trigger source가 다른 컴포넌트 정의 tree의 node입니다.
    ForeignEventSource {
        /// transition.
        transition: TransitionId,
        /// node.
        node: StableId,
        /// node를 소유한 컴포넌트.
        owner: ComponentRef,
    },
    /// nested action source가 자기 정의 tree의 ComponentInstance가 아닙니다.
    NotNestedInstance {
        /// transition.
        transition: TransitionId,
        /// node.
        node: StableId,
    },
}

impl fmt::Display for StateEncapsulationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateEncapsulationError::ForeignStateWrite {
                transition,
                variable,
                owner,
            } => write!(
                f,
                "transition {}이 컴포넌트 {}의 상태 변수 {}를 직접 씁니다",
                transition.stable_id(),
                owner.id(),
                variable.stable_id()
            ),
            StateEncapsulationError::ForeignStateRead {
                transition,
                variable,
                owner,
            } => write!(
                f,
                "transition {}이 컴포넌트 {}의 상태 변수 {}를 직접 읽습니다",
                transition.stable_id(),
                owner.id(),
                variable.stable_id()
            ),
            StateEncapsulationError::ForeignEventSource {
                transition,
                node,
                owner,
            } => write!(
                f,
                "transition {}이 컴포넌트 {} 내부 node {node}의 이벤트를 직접 관찰합니다",
                transition.stable_id(),
                owner.id()
            ),
            StateEncapsulationError::NotNestedInstance { transition, node } => write!(
                f,
                "transition {}의 nested action source {node}가 nested instance가 아닙니다",
                transition.stable_id()
            ),
        }
    }
}

impl std::error::Error for StateEncapsulationError {}

impl ComponentLibrary {
    /// 상태 변수를 소유한 컴포넌트.
    pub fn state_variable_owner(&self, variable: StateVariableId) -> Option<ComponentRef> {
        self.iter()
            .find(|entry| entry.state_machine().variable(variable).is_some())
            .map(|entry| entry.definition().reference())
    }

    /// transition 입력이 `component`의 상태 캡슐화 규칙을 지키는지 검사합니다.
    pub fn check_state_encapsulation(
        &self,
        component: ComponentRef,
        spec: &TransitionSpec,
    ) -> Result<(), StateEncapsulationError> {
        let transition = spec.id;
        let foreign = |variable: StateVariableId| {
            self.state_variable_owner(variable)
                .filter(|owner| *owner != component)
        };
        match spec.trigger {
            Trigger::Event { source, .. } => {
                if let Some(owner) = self.owner_of(source).filter(|owner| *owner != component) {
                    return Err(StateEncapsulationError::ForeignEventSource {
                        transition,
                        node: source,
                        owner,
                    });
                }
            }
            Trigger::NestedAction { instance, .. } => {
                let own_node = self
                    .get(component)
                    .and_then(|entry| entry.tree().node(instance));
                match (own_node, self.owner_of(instance)) {
                    (Some(node), _) if node.kind != NodeKind::ComponentInstance => {
                        return Err(StateEncapsulationError::NotNestedInstance {
                            transition,
                            node: instance,
                        })
                    }
                    (None, Some(owner)) if owner != component => {
                        return Err(StateEncapsulationError::ForeignEventSource {
                            transition,
                            node: instance,
                            owner,
                        })
                    }
                    _ => {}
                }
            }
        }
        let reads = |ast: &ExpressionAst| -> Result<(), StateEncapsulationError> {
            for target in ast.referenced_targets() {
                if let Some(variable) = state_reference(target) {
                    if let Some(owner) = foreign(variable) {
                        return Err(StateEncapsulationError::ForeignStateRead {
                            transition,
                            variable,
                            owner,
                        });
                    }
                }
            }
            Ok(())
        };
        if let Some(condition) = &spec.condition {
            reads(condition)?;
        }
        for action in &spec.actions {
            if let ActionSpec::SetState { variable, value } = action {
                if let Some(owner) = foreign(*variable) {
                    return Err(StateEncapsulationError::ForeignStateWrite {
                        transition,
                        variable: *variable,
                        owner,
                    });
                }
                reads(value)?;
            }
        }
        Ok(())
    }

    /// `component` 상태 머신 끝에 transition을 추가합니다. 상태 캡슐화(IX.5.6)를 먼저 검사한 뒤 상태 머신
    /// 규칙(IX.5.3)을 검사합니다. 실패하면 library는 변경되지 않습니다.
    pub fn add_transition(
        &mut self,
        component: ComponentRef,
        spec: TransitionSpec,
    ) -> Result<(), StateMachineError> {
        self.check_state_encapsulation(component, &spec)
            .map_err(StateMachineError::Encapsulation)?;
        let entry = self
            .get(component)
            .ok_or(StateMachineError::ComponentNotFound { component })?;
        let mut machine = entry.state_machine().clone();
        machine.add_transition(entry, spec)?;
        self.set_state_machine(component, machine)?;
        Ok(())
    }

    /// library 전체 상태 머신의 캡슐화를 검사합니다(library 디코딩 경로).
    pub(super) fn validate_loaded_state_machines(&self) -> Result<(), StateMachineError> {
        for entry in self.iter() {
            let component = entry.definition().reference();
            for transition in entry.state_machine().transitions() {
                self.check_state_encapsulation(component, &transition.to_spec())
                    .map_err(StateMachineError::Encapsulation)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! Switch(4) 정의 tree 안에 nested Toggle(3) 인스턴스 하나를 둔 fixture.

    use super::*;
    use crate::component::definition::{ComponentDefinition, DefinitionNode, DefinitionTree};
    use crate::component::instance::ComponentInstancePayload;
    use crate::component::state_machine::fixtures::*;
    use crate::component::state_machine::{
        PropertyScope, SemanticAction, SemanticActionId, StateVariable,
    };
    use crate::component::typed_expr::test_util::literal;
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::reference::NodeRef;
    use stellar_core_types::typed_value::TypedValue;
    use stellar_expressions::types::{ExpressionType, TypeSpec};

    pub(crate) fn switch() -> ComponentRef {
        ComponentRef::new(stable(4))
    }

    /// Switch 정의 tree: 400(Frame) ← 401(Toggle 인스턴스), 402(Text).
    pub(crate) const NESTED_TOGGLE: u64 = 401;
    pub(crate) const SWITCH_LABEL: u64 = 402;
    /// Switch 상태 변수 910 active(Boolean).
    pub(crate) const ACTIVE: u64 = 910;
    /// Switch가 선언한 action 951 changed.
    pub(crate) const CHANGED: u64 = 951;

    /// Toggle(상태 머신 포함) + Switch library.
    pub(crate) fn switch_library() -> ComponentLibrary {
        let mut library = toggle_library();
        let machine = toggle_machine(&library);
        library.set_state_machine(toggle(), machine).unwrap();
        let mut tree = DefinitionTree::new(switch());
        for (id, kind, parent, key, payload) in [
            (400, NodeKind::Frame, None, 0x80u8, Vec::new()),
            (
                NESTED_TOGGLE,
                NodeKind::ComponentInstance,
                Some(400),
                0x40,
                ComponentInstancePayload::new(toggle()).encode(),
            ),
            (SWITCH_LABEL, NodeKind::Text, Some(400), 0x60, Vec::new()),
        ] {
            tree.insert(DefinitionNode {
                id: stable(id),
                kind,
                parent: parent.map(stable),
                order_key: OrderKey::from_bytes(vec![key]).unwrap(),
                visible: true,
                locked: false,
                display_name: String::new(),
                payload,
            })
            .unwrap();
        }
        library
            .insert(
                ComponentDefinition::new(stable(4), "Switch".into(), NodeRef::new(stable(400))),
                tree,
            )
            .unwrap();
        let entry = library.get(switch()).unwrap();
        let mut machine = crate::component::state_machine::StateMachine::new();
        machine
            .add_variable(
                StateVariable::new(
                    var(ACTIVE),
                    "active".into(),
                    TypeSpec::required(ExpressionType::Boolean),
                    literal(TypedValue::Boolean(false)),
                    &PropertyScope::new(entry),
                )
                .unwrap(),
            )
            .unwrap();
        machine
            .add_action(
                SemanticAction::new(SemanticActionId::new(stable(CHANGED)), "changed".into())
                    .unwrap(),
            )
            .unwrap();
        library.set_state_machine(switch(), machine).unwrap();
        library
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::component::event::EventKind;
    use crate::component::state_machine::fixtures::*;
    use crate::component::state_machine::{state_value_path, SemanticActionId};
    use crate::component::typed_expr::test_util::{literal, reference};
    use stellar_core_types::typed_value::TypedValue;
    use stellar_expressions::ast::ReferenceTarget;

    fn spec(trigger: Trigger, actions: Vec<ActionSpec>) -> TransitionSpec {
        TransitionSpec {
            id: transition_id(980),
            trigger,
            condition: None,
            actions,
        }
    }

    fn nested_toggled() -> Trigger {
        Trigger::NestedAction {
            instance: stable(NESTED_TOGGLE),
            action: action_id(TOGGLED),
        }
    }

    #[test]
    fn direct_nested_state_write_fixture_fails() {
        // 검증 기준(IX.5.6): direct nested state write fixture 실패. Switch가 nested Toggle의 isOn을 직접
        // 씁니다.
        let mut library = switch_library();
        let before = library.encode();
        let write = spec(
            nested_toggled(),
            vec![ActionSpec::SetState {
                variable: var(IS_ON),
                value: literal(TypedValue::Boolean(true)),
            }],
        );
        assert_eq!(
            library.add_transition(switch(), write).unwrap_err(),
            StateMachineError::Encapsulation(StateEncapsulationError::ForeignStateWrite {
                transition: transition_id(980),
                variable: var(IS_ON),
                owner: toggle(),
            })
        );
        assert_eq!(library.encode(), before, "거부 시 library 불변");
    }

    #[test]
    fn foreign_reads_and_internal_event_sources_fail() {
        let mut library = switch_library();
        // nested Toggle의 isOn을 읽어 자기 active에 씀.
        let read = spec(
            nested_toggled(),
            vec![ActionSpec::SetState {
                variable: var(ACTIVE),
                value: reference(ReferenceTarget::Property(state_value_path(var(IS_ON)))),
            }],
        );
        assert!(matches!(
            library.add_transition(switch(), read).unwrap_err(),
            StateMachineError::Encapsulation(StateEncapsulationError::ForeignStateRead { .. })
        ));
        // nested Toggle 내부 knob(301)의 Activate를 직접 관찰.
        let internal = spec(
            Trigger::Event {
                kind: EventKind::Activate,
                source: stable(KNOB),
            },
            vec![],
        );
        assert_eq!(
            library.add_transition(switch(), internal).unwrap_err(),
            StateMachineError::Encapsulation(StateEncapsulationError::ForeignEventSource {
                transition: transition_id(980),
                node: stable(KNOB),
                owner: toggle(),
            })
        );
        // nested action source가 인스턴스가 아닌 자기 node(Text).
        let not_instance = spec(
            Trigger::NestedAction {
                instance: stable(SWITCH_LABEL),
                action: action_id(TOGGLED),
            },
            vec![],
        );
        assert!(matches!(
            library.add_transition(switch(), not_instance).unwrap_err(),
            StateMachineError::Encapsulation(StateEncapsulationError::NotNestedInstance { .. })
        ));
    }

    #[test]
    fn public_event_chain_is_allowed_and_round_trips() {
        // nested Toggle이 emit한 toggled에 반응해 자기 상태를 바꾸고 자기 action을 emit합니다.
        let mut library = switch_library();
        library
            .add_transition(
                switch(),
                spec(
                    nested_toggled(),
                    vec![
                        ActionSpec::SetState {
                            variable: var(ACTIVE),
                            value: crate::component::typed_expr::test_util::unary(
                                stellar_expressions::ops::UnaryOp::Not,
                                reference(ReferenceTarget::Property(state_value_path(var(ACTIVE)))),
                            ),
                        },
                        ActionSpec::Emit {
                            action: SemanticActionId::new(stable(CHANGED)),
                        },
                    ],
                ),
            )
            .unwrap();
        let bytes = library.encode();
        assert_eq!(ComponentLibrary::decode(&bytes).unwrap(), library);
        assert_eq!(library.state_variable_owner(var(IS_ON)), Some(toggle()));
        assert_eq!(library.state_variable_owner(var(ACTIVE)), Some(switch()));
    }
}
