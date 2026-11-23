//! 포트 공개 plan (XXX.2.4, ADR-0025 결정 2.5, ADR-0094 결정 4).
//!
//! 그래프에서 컴포넌트 밖으로 포트를 공개하면, 다른 컴포넌트의 내부 포트에 직접 잇는 대신 이 컴포넌트의 공개 인터페이스를
//! 만듭니다. DrillingPlan(IX.4.3)처럼 계획([`ExposePlan`])을 먼저 계산해 미리 보여 주고, 그 transaction 하나로 원자
//! 적용합니다.
//!
//! - 속성 입력 공개([`ExposeRequest::PropertyInput`]): 공개 Component Property를 만들고, 대상 속성의 binding source를 그
//!   Property 참조로 바꿉니다(없으면 만듭니다). Property 타입은 포트 타입 집합에 있어야 합니다.
//! - 이벤트 공개([`ExposeRequest::Event`]): semantic action(공개 Event)을 선언하고, 그 이벤트가 나면 action을 emit하는
//!   transition을 더합니다.
//! - Context 요구 공개는 ContextRequirement(XXX.7.8)가 생기기 전에는 [`PlanError::Unsupported`]입니다.
//!
//! nested instance 내부 포트에 직접 잇는 edge는 [`crate::plan::Planner`]가 [`PlanError::NestedInternalPort`]로 거부합니다.

use stellar_bindings::binding::{BindingId, ScopeOwner, ValueBinding};
use stellar_bindings::binding_commands::BindingCommand;
use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_core_types::reference::ComponentRef;
use stellar_document::component::definition::ComponentPropertyId;
use stellar_document::component::event::EventKind;
use stellar_document::component::property::{ComponentProperty, PropertyVisibility};
use stellar_document::component::property_commands::PropertyCommand;
use stellar_document::component::state_machine::{
    ActionSpec, SemanticAction, SemanticActionId, StateMachine, TransitionId, TransitionSpec,
    Trigger,
};
use stellar_expressions::ast::{ExprNode, ExprNodeId, ExpressionAst, ReferenceTarget};
use stellar_expressions::types::{ExpressionType, TypeSpec};

use crate::edit::{PrototypeDocument, PrototypeStep, PrototypeTransaction};
use crate::plan::PlanError;

/// 공개 요청.
#[derive(Debug, Clone, PartialEq)]
pub enum ExposeRequest {
    /// 속성 입력 포트를 공개 Property로.
    PropertyInput {
        /// 컴포넌트.
        component: ComponentRef,
        /// 대상 속성.
        target: PropertyPath,
        /// 새 Property id.
        property: ComponentPropertyId,
        /// Property 이름.
        name: String,
        /// Property 타입(포트 타입 집합 안).
        value_type: ExpressionType,
        /// 닫힌 default 식.
        default: ExpressionAst,
        /// 새 binding id(대상에 binding이 없을 때).
        binding: BindingId,
        /// Property 참조 block id.
        reference_node: StableId,
    },
    /// 이벤트 출력 포트를 공개 Event(semantic action)로.
    Event {
        /// 컴포넌트.
        component: ComponentRef,
        /// 이벤트 원천 node.
        source: StableId,
        /// 이벤트 종류.
        kind: EventKind,
        /// 새 action id.
        action: SemanticActionId,
        /// action 이름.
        name: String,
        /// emit transition id.
        transition: TransitionId,
    },
    /// Context 요구 공개(XXX.7.8 전까지 지원하지 않음).
    ContextRequirement {
        /// 컴포넌트.
        component: ComponentRef,
    },
}

/// 미리보기 항목.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExposedItem {
    /// 공개 Property.
    Property(ComponentPropertyId),
    /// 공개 Event.
    Event(SemanticActionId),
    /// 그 공개 항목을 읽도록 바뀌는 binding.
    Binding(BindingId),
}

/// 공개 계획: 미리보기 목록과 transaction 하나.
#[derive(Debug, Clone, PartialEq)]
pub struct ExposePlan {
    /// 만들어지거나 바뀌는 항목.
    pub preview: Vec<ExposedItem>,
    /// 원자 적용할 transaction.
    pub transaction: PrototypeTransaction,
}

/// 공개 계획을 만듭니다. 문서는 바뀌지 않습니다.
pub fn plan_expose(
    doc: &PrototypeDocument,
    request: &ExposeRequest,
) -> Result<ExposePlan, PlanError> {
    match request {
        ExposeRequest::PropertyInput {
            component,
            target,
            property,
            name,
            value_type,
            default,
            binding,
            reference_node,
        } => {
            // 대상 검사(nested instance 내부면 거부)와 포트 타입 검사.
            let port = doc.port(*component, target).ok_or_else(|| {
                match doc.library.owner_of(target.root()) {
                    Some(other) if other != *component => {
                        PlanError::NestedInternalPort(target.clone())
                    }
                    _ => PlanError::OutsideOwner(target.clone()),
                }
            })?;
            if !port.accepts(*value_type) {
                return Err(PlanError::Connection(
                    stellar_document::prototype::socket::ConnectionError::TypeNotAccepted {
                        found: TypeSpec::required(*value_type),
                        port: port.name,
                    },
                ));
            }
            let record = ComponentProperty::new(
                *property,
                name.clone(),
                TypeSpec::required(*value_type),
                default.clone(),
                PropertyVisibility::Public,
            )
            .map_err(|_| PlanError::ExpressionType)?;
            let reference = ExprNodeId::new(*reference_node);
            let source = ExpressionAst::new(
                reference,
                vec![ExprNode::reference(
                    reference,
                    ReferenceTarget::ComponentProperty(property.stable_id()),
                )],
            )
            .map_err(|_| PlanError::ExpressionType)?;
            let expected = TypeSpec::required(*value_type);
            let (binding_id, command) = match doc.bindings.by_target(target) {
                Some(existing) => (
                    existing.id(),
                    BindingCommand::Replace {
                        binding: existing.id(),
                        source,
                        expected_type: expected,
                    },
                ),
                None => (
                    *binding,
                    BindingCommand::Create(ValueBinding::new(
                        *binding,
                        target.clone(),
                        source,
                        expected,
                        ScopeOwner::Component(*component),
                    )),
                ),
            };
            Ok(ExposePlan {
                preview: vec![
                    ExposedItem::Property(*property),
                    ExposedItem::Binding(binding_id),
                ],
                transaction: PrototypeTransaction::new(
                    "포트 공개",
                    vec![
                        PrototypeStep::Property(PropertyCommand::Create {
                            component: *component,
                            property: record,
                            index: None,
                        }),
                        PrototypeStep::Binding {
                            component: *component,
                            command,
                        },
                    ],
                ),
            })
        }
        ExposeRequest::Event {
            component,
            source,
            kind,
            action,
            name,
            transition,
        } => {
            let entry = doc
                .library
                .get(*component)
                .ok_or(PlanError::Missing("컴포넌트"))?;
            if !entry.tree().contains(*source) {
                return Err(match doc.library.owner_of(*source) {
                    Some(other) if other != *component => {
                        PlanError::NestedInternalPort(PropertyPath::new(*source))
                    }
                    _ => PlanError::OutsideOwner(PropertyPath::new(*source)),
                });
            }
            let current = entry.state_machine();
            let mut machine = StateMachine::new();
            for existing in current.actions() {
                machine
                    .add_action(existing.clone())
                    .map_err(PlanError::StateMachine)?;
            }
            machine
                .add_action(
                    SemanticAction::new(*action, name.clone()).map_err(PlanError::StateMachine)?,
                )
                .map_err(PlanError::StateMachine)?;
            for variable in current.variables() {
                machine
                    .add_variable(variable.clone())
                    .map_err(PlanError::StateMachine)?;
            }
            for existing in current.transitions() {
                machine
                    .add_transition(entry, existing.to_spec())
                    .map_err(PlanError::StateMachine)?;
            }
            machine
                .add_transition(
                    entry,
                    TransitionSpec {
                        id: *transition,
                        trigger: Trigger::Event {
                            kind: *kind,
                            source: *source,
                        },
                        condition: None,
                        actions: vec![ActionSpec::Emit { action: *action }],
                    },
                )
                .map_err(PlanError::StateMachine)?;
            Ok(ExposePlan {
                preview: vec![ExposedItem::Event(*action)],
                transaction: PrototypeTransaction::new(
                    "이벤트 공개",
                    vec![PrototypeStep::StateMachine {
                        component: *component,
                        machine,
                    }],
                ),
            })
        }
        ExposeRequest::ContextRequirement { .. } => Err(PlanError::Unsupported(
            "ContextRequirement(XXX.7.8)가 아직 없습니다",
        )),
    }
}
