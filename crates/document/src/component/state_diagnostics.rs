//! 상태 머신 진단 (IX.5.7).
//!
//! 저장을 막지 않는 작성 단계 진단입니다. 결과 순서는 transition 목록 순서(우선순위) → 진단 종류 순서라
//! 결정적입니다.
//!
//! - **unreachable transition**: 같은 trigger의 앞선 transition이 항상 참(조건 없음, 또는 참조 없는 조건이
//!   `true`로 평가)이면 뒤 transition은 실행될 수 없습니다(dispatch는 처음 참인 transition 하나만 실행,
//!   IX.5.5). 참조 없는 조건이 `false`로 평가되는 transition도 실행될 수 없습니다.
//! - **duplicate condition**: 같은 trigger의 앞선 transition과 조건식이 같습니다(node id를 무시한 구조 bytes
//!   동일 — [`CheckedExpression::structural_bytes`]). 앞 transition이 거짓이면 뒤도 거짓이라 뒤 transition은
//!   실행되지 않습니다.
//! - **missing target**: trigger가 가리키는 대상이 없습니다 — source node가 정의 tree에 없음, nested instance
//!   node가 없음, nested instance의 컴포넌트가 library에 없음(미해결 참조), nested 컴포넌트가 그 semantic
//!   action을 선언하지 않음. 정의 tree·다른 컴포넌트는 상태 머신과 따로 바뀌므로 오류가 아니라 진단입니다.

use std::collections::BTreeMap;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::ComponentRef;
use stellar_core_types::typed_value::TypedValue;
use stellar_expressions::ast::ReferenceTarget;

use super::definition::ComponentLibrary;
use super::instance::ComponentInstancePayload;
use super::state_machine::{SemanticActionId, Transition, TransitionId, Trigger};
use super::typed_expr::CheckedExpression;
use crate::node_kind::NodeKind;

/// missing target 종류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissingTarget {
    /// 이벤트 source node가 정의 tree에 없습니다.
    SourceNode {
        /// node.
        node: StableId,
    },
    /// nested instance node가 정의 tree에 없습니다.
    NestedInstance {
        /// node.
        node: StableId,
    },
    /// nested instance가 참조하는 컴포넌트가 library에 없습니다.
    NestedComponent {
        /// nested instance.
        instance: StableId,
        /// 참조 컴포넌트.
        component: ComponentRef,
    },
    /// nested 컴포넌트가 semantic action을 선언하지 않습니다.
    NestedAction {
        /// nested instance.
        instance: StableId,
        /// 없는 action.
        action: SemanticActionId,
    },
}

/// 상태 머신 진단 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateDiagnostic {
    /// 실행될 수 없는 transition.
    Unreachable {
        /// 실행될 수 없는 transition.
        transition: TransitionId,
        /// 가리는 앞 transition(`None`이면 조건이 항상 거짓).
        shadowed_by: Option<TransitionId>,
    },
    /// 같은 trigger의 앞 transition과 조건이 같습니다.
    DuplicateCondition {
        /// 뒤 transition.
        transition: TransitionId,
        /// 같은 조건의 앞 transition.
        duplicate_of: TransitionId,
    },
    /// trigger 대상이 없습니다.
    MissingTarget {
        /// transition.
        transition: TransitionId,
        /// 없는 대상.
        target: MissingTarget,
    },
}

/// 참조 없는 조건의 상수 값(참조가 있거나 평가 실패면 `None`).
fn constant_truth(condition: &CheckedExpression) -> Option<bool> {
    if !condition.references().is_empty() {
        return None;
    }
    let none: BTreeMap<ReferenceTarget, Option<TypedValue>> = BTreeMap::new();
    match condition.evaluate(&none) {
        Ok(Some(TypedValue::Boolean(value))) => Some(value),
        _ => None,
    }
}

/// transition이 항상 참인지(조건 없음 또는 상수 참).
fn always_true(transition: &Transition) -> bool {
    match transition.condition() {
        None => true,
        Some(condition) => constant_truth(condition) == Some(true),
    }
}

/// `component` 상태 머신을 진단합니다. 컴포넌트가 없으면 빈 목록입니다.
pub fn diagnose_state_machine(
    library: &ComponentLibrary,
    component: ComponentRef,
) -> Vec<StateDiagnostic> {
    let Some(entry) = library.get(component) else {
        return Vec::new();
    };
    let transitions = entry.state_machine().transitions();
    let mut out = Vec::new();
    for (index, transition) in transitions.iter().enumerate() {
        let id = transition.id();
        let earlier = transitions[..index]
            .iter()
            .filter(|t| t.trigger() == transition.trigger());
        // unreachable / duplicate.
        let mut shadow = None;
        let mut duplicate = None;
        for previous in earlier {
            if shadow.is_none() && always_true(previous) {
                shadow = Some(previous.id());
            }
            if duplicate.is_none() {
                if let (Some(a), Some(b)) = (previous.condition(), transition.condition()) {
                    if a.structural_bytes() == b.structural_bytes() {
                        duplicate = Some(previous.id());
                    }
                }
            }
        }
        if let Some(shadowed_by) = shadow {
            out.push(StateDiagnostic::Unreachable {
                transition: id,
                shadowed_by: Some(shadowed_by),
            });
        } else if transition
            .condition()
            .is_some_and(|c| constant_truth(c) == Some(false))
        {
            out.push(StateDiagnostic::Unreachable {
                transition: id,
                shadowed_by: None,
            });
        }
        if let Some(duplicate_of) = duplicate {
            out.push(StateDiagnostic::DuplicateCondition {
                transition: id,
                duplicate_of,
            });
        }
        // missing target.
        let missing = match transition.trigger() {
            Trigger::Event { source, .. } => (!entry.tree().contains(source))
                .then_some(MissingTarget::SourceNode { node: source }),
            Trigger::NestedAction { instance, action } => match entry.tree().node(instance) {
                None => Some(MissingTarget::NestedInstance { node: instance }),
                Some(node) if node.kind != NodeKind::ComponentInstance => {
                    // 캡슐화 validator(IX.5.6)가 거부하는 상태라 저장된 머신에는 없습니다.
                    Some(MissingTarget::NestedInstance { node: instance })
                }
                Some(node) => match ComponentInstancePayload::decode(&node.payload) {
                    Err(_) => Some(MissingTarget::NestedInstance { node: instance }),
                    Ok(payload) => match library.get(payload.component()) {
                        None => Some(MissingTarget::NestedComponent {
                            instance,
                            component: payload.component(),
                        }),
                        Some(nested) => nested
                            .state_machine()
                            .action(action)
                            .is_none()
                            .then_some(MissingTarget::NestedAction { instance, action }),
                    },
                },
            },
        };
        if let Some(target) = missing {
            out.push(StateDiagnostic::MissingTarget {
                transition: id,
                target,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::definition::{ComponentDefinition, DefinitionNode, DefinitionTree};
    use crate::component::event::EventKind;
    use crate::component::state_encapsulation::fixtures::*;
    use crate::component::state_machine::fixtures::*;
    use crate::component::state_machine::{state_value_path, ActionSpec, TransitionSpec};
    use crate::component::typed_expr::test_util::{binary, literal, reference};
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::reference::NodeRef;
    use stellar_expressions::ops::BinaryOp;

    fn active_is(value: bool) -> stellar_expressions::ast::ExpressionAst {
        binary(
            BinaryOp::Equal,
            reference(ReferenceTarget::Property(state_value_path(var(ACTIVE)))),
            literal(TypedValue::Boolean(value)),
        )
    }

    fn label_activate() -> Trigger {
        Trigger::Event {
            kind: EventKind::Activate,
            source: stable(SWITCH_LABEL),
        }
    }

    fn spec(
        id: u64,
        trigger: Trigger,
        condition: Option<stellar_expressions::ast::ExpressionAst>,
    ) -> TransitionSpec {
        TransitionSpec {
            id: transition_id(id),
            trigger,
            condition,
            actions: vec![ActionSpec::SetState {
                variable: var(ACTIVE),
                value: literal(TypedValue::Boolean(true)),
            }],
        }
    }

    #[test]
    fn diagnostic_fixture_is_exact() {
        // 검증 기준(IX.5.7): diagnostic fixture 정확.
        let mut library = switch_library();
        for transition in [
            // 960: active == false (정상).
            spec(960, label_activate(), Some(active_is(false))),
            // 961: 960과 같은 조건 → duplicate condition.
            spec(961, label_activate(), Some(active_is(false))),
            // 962: 조건 없음(항상 참).
            spec(962, label_activate(), None),
            // 963: 962 뒤 → unreachable(가림).
            spec(963, label_activate(), Some(active_is(true))),
            // 964: 다른 trigger(Focus)이고 상수 거짓 조건 → unreachable(항상 거짓).
            spec(
                964,
                Trigger::Event {
                    kind: EventKind::Focus,
                    source: stable(SWITCH_LABEL),
                },
                Some(literal(TypedValue::Boolean(false))),
            ),
            // 965: 정의 tree에 없는 source node → missing target.
            spec(
                965,
                Trigger::Event {
                    kind: EventKind::Blur,
                    source: stable(4_999),
                },
                None,
            ),
            // 966: nested Toggle이 선언하지 않은 action → missing target.
            spec(
                966,
                Trigger::NestedAction {
                    instance: stable(NESTED_TOGGLE),
                    action: action_id(959),
                },
                None,
            ),
            // 967: 없는 nested instance node → missing target.
            spec(
                967,
                Trigger::NestedAction {
                    instance: stable(4_998),
                    action: action_id(TOGGLED),
                },
                None,
            ),
            // 968: 정상 Event chain.
            spec(
                968,
                Trigger::NestedAction {
                    instance: stable(NESTED_TOGGLE),
                    action: action_id(TOGGLED),
                },
                None,
            ),
        ] {
            library.add_transition(switch(), transition).unwrap();
        }
        assert_eq!(
            diagnose_state_machine(&library, switch()),
            vec![
                StateDiagnostic::DuplicateCondition {
                    transition: transition_id(961),
                    duplicate_of: transition_id(960),
                },
                StateDiagnostic::Unreachable {
                    transition: transition_id(963),
                    shadowed_by: Some(transition_id(962)),
                },
                StateDiagnostic::Unreachable {
                    transition: transition_id(964),
                    shadowed_by: None,
                },
                StateDiagnostic::MissingTarget {
                    transition: transition_id(965),
                    target: MissingTarget::SourceNode {
                        node: stable(4_999)
                    },
                },
                StateDiagnostic::MissingTarget {
                    transition: transition_id(966),
                    target: MissingTarget::NestedAction {
                        instance: stable(NESTED_TOGGLE),
                        action: action_id(959),
                    },
                },
                StateDiagnostic::MissingTarget {
                    transition: transition_id(967),
                    target: MissingTarget::NestedInstance {
                        node: stable(4_998)
                    },
                },
            ]
        );
        // 진단은 저장을 막지 않습니다.
        assert_eq!(
            ComponentLibrary::decode(&library.encode()).unwrap(),
            library
        );
        // 문제 없는 머신은 진단 0건.
        assert!(diagnose_state_machine(&switch_library(), switch()).is_empty());
        assert!(diagnose_state_machine(&library, ComponentRef::new(stable(77))).is_empty());
    }

    #[test]
    fn unresolved_nested_component_is_a_missing_target() {
        // 정의 tree 안 nested instance가 library에 없는 컴포넌트(77)를 참조합니다.
        let mut library = ComponentLibrary::new();
        let host = ComponentRef::new(stable(5));
        let mut tree = DefinitionTree::new(host);
        for (id, kind, parent, payload) in [
            (500, NodeKind::Frame, None, Vec::new()),
            (
                501,
                NodeKind::ComponentInstance,
                Some(500),
                ComponentInstancePayload::new(ComponentRef::new(stable(77))).encode(),
            ),
        ] {
            tree.insert(DefinitionNode {
                id: stable(id),
                kind,
                parent: parent.map(stable),
                order_key: OrderKey::from_bytes(vec![0x80]).unwrap(),
                visible: true,
                locked: false,
                display_name: String::new(),
                payload,
            })
            .unwrap();
        }
        library
            .insert(
                ComponentDefinition::new(stable(5), "Host".into(), NodeRef::new(stable(500))),
                tree,
            )
            .unwrap();
        library
            .add_transition(
                host,
                TransitionSpec {
                    id: transition_id(990),
                    trigger: Trigger::NestedAction {
                        instance: stable(501),
                        action: action_id(TOGGLED),
                    },
                    condition: None,
                    actions: vec![],
                },
            )
            .unwrap();
        assert_eq!(
            diagnose_state_machine(&library, host),
            vec![StateDiagnostic::MissingTarget {
                transition: transition_id(990),
                target: MissingTarget::NestedComponent {
                    instance: stable(501),
                    component: ComponentRef::new(stable(77)),
                },
            }]
        );
    }
}
