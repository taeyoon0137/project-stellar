//! 프로토타입 문서 상태와 원자 transaction (XXX.2.3, ADR-0025 결정 2.4, ADR-0094 결정 4).
//!
//! 그래프 편집은 새 원본을 만들지 않고 기존 command로 낮춥니다. [`PrototypeStep`]은 binding command(VIII.6.6),
//! MotionSpec command(XXX.1.6), Component Property command(IX.1.3), 상태 머신 교체(IX.5), 배치 hint 고정, Context
//! requirement 추가·삭제(XXX.7.8의 `AddRequirement`·`RemoveRequirement`와 같은 `ContextSet` 연산)입니다.
//! [`PrototypeTransaction`]은 사용자 동작 하나에 해당하며 전부 적용되거나 전부 되돌아갑니다. 적용 결과의 inverse
//! transaction 하나를 적용하면(Undo 1회) 동작 전 상태로 돌아갑니다.
//!
//! Phase VII `CommandTransaction`(DocumentModel command journal)에 이 step 종류를 등록하는 일은 영속 command registry
//! 작업(ADR-0079)의 몫이고, 이 crate는 같은 원자성·inverse 규칙을 document 계층 command 위에서 지킵니다.

use std::collections::BTreeMap;
use std::fmt;

use stellar_bindings::binding::BindingSet;
use stellar_bindings::binding_commands::{
    BindingCommand, BindingCommandError, BindingContext, TargetCheck, TargetRejection,
};
use stellar_bindings::registry::TokenRegistry;
use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_core_types::reference::ComponentRef;
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::component::drilling::DrillingSet;
use stellar_document::component::property_commands::{PropertyCommand, PropertyCommandError};
use stellar_document::component::state_machine::{
    state_reference, PropertyScope, StateMachine, StateMachineError,
};
use stellar_document::context::set::ContextSet;
use stellar_document::motion::{
    LibraryWorld, MotionCommand, MotionCommandError, MotionOwner, MotionSet,
};
use stellar_document::prototype::layout_hint::{HintPoint, LayoutHints};
use stellar_document::prototype::socket::{property_port, PropertyPort, TargetKind};
use stellar_expressions::ast::ReferenceTarget;
use stellar_expressions::typecheck::ReferenceTypes;
use stellar_expressions::types::TypeSpec;

/// 프로토타입 편집이 다루는 문서 상태(컴포넌트 소유자).
#[derive(Debug, Clone, Default)]
pub struct PrototypeDocument {
    /// 컴포넌트 library(상태 머신 포함).
    pub library: ComponentLibrary,
    /// drilling binding.
    pub drilling: DrillingSet,
    /// 토큰.
    pub tokens: TokenRegistry,
    /// value binding.
    pub bindings: BindingSet,
    /// MotionSpec.
    pub motions: MotionSet,
    /// 소유자별 배치 hint.
    pub hints: BTreeMap<MotionOwner, LayoutHints>,
    /// 문서 Component Context(key·provision·requirement·Slot 공개 목록, ADR-0099).
    pub context: ContextSet,
}

impl PrototypeDocument {
    /// 상태 비교용 canonical bytes(library·binding·MotionSpec·hint).
    pub fn state_bytes(&self) -> Vec<u8> {
        let mut out = self.library.encode();
        let mut bindings: Vec<_> = self.bindings.iter().collect();
        bindings.sort_by_key(|b| b.id());
        for binding in bindings {
            out.extend(binding.to_bytes().unwrap_or_default());
        }
        out.extend(self.motions.encode());
        for (owner, hints) in &self.hints {
            out.extend(owner.id().to_bytes());
            out.extend(hints.encode());
        }
        out.extend(self.context.encode());
        out
    }

    /// `component` 정의 tree에서 `root` node의 대상 종류.
    pub fn target_kind(&self, component: ComponentRef, root: StableId) -> Option<TargetKind> {
        self.library
            .get(component)?
            .tree()
            .node(root)
            .map(|n| TargetKind::Node(n.kind))
    }

    /// `component` 소유 속성 `target`의 포트.
    pub fn port(&self, component: ComponentRef, target: &PropertyPath) -> Option<PropertyPort> {
        let kind = self.target_kind(component, target.root())?;
        let key = *target.keys().first()?;
        property_port(kind, key).ok()
    }
}

/// 컴포넌트 소유 binding의 참조 타입: Component Property, 상태 변수, 포트가 있는 정의 tree 속성.
pub struct OwnerTypes<'a> {
    /// 문서.
    pub doc: &'a PrototypeDocument,
    /// 컴포넌트.
    pub component: ComponentRef,
}

impl ReferenceTypes for OwnerTypes<'_> {
    fn reference_type(&self, target: &ReferenceTarget) -> Option<TypeSpec> {
        let entry = self.doc.library.get(self.component)?;
        if let Some(variable) = state_reference(target) {
            return entry
                .state_machine()
                .variable(variable)
                .map(|v| v.value_type());
        }
        if let ReferenceTarget::Property(path) = target {
            let port = self.doc.port(self.component, path)?;
            if port.types.len() == 1 {
                return port.types.iter().next().map(|t| TypeSpec::required(*t));
            }
            return None;
        }
        PropertyScope::new(entry).reference_type(target)
    }
}

/// 컴포넌트 소유 binding target 검사: 정의 tree 안 node의 포트가 있는 속성만 받습니다.
pub struct OwnerTargets<'a> {
    /// 문서.
    pub doc: &'a PrototypeDocument,
    /// 컴포넌트.
    pub component: ComponentRef,
}

impl TargetCheck for OwnerTargets<'_> {
    fn reject_target(&self, target: &PropertyPath) -> Option<TargetRejection> {
        let Some(kind) = self.doc.target_kind(self.component, target.root()) else {
            return Some(TargetRejection {
                code: "prototype-target-outside-owner",
                message: "대상 node가 컴포넌트 정의 tree에 없습니다".into(),
            });
        };
        let ([key], None) = (target.keys(), target.tail()) else {
            return Some(TargetRejection {
                code: "prototype-target-path",
                message: "대상은 속성 하나여야 합니다".into(),
            });
        };
        property_port(kind, *key).err().map(|diag| TargetRejection {
            code: diag.code,
            message: diag.message,
        })
    }
}

/// transaction step.
#[derive(Debug, Clone, PartialEq)]
pub enum PrototypeStep {
    /// binding command(대상 컴포넌트 소유).
    Binding {
        /// 소유 컴포넌트.
        component: ComponentRef,
        /// command.
        command: BindingCommand,
    },
    /// MotionSpec command.
    Motion(MotionCommand),
    /// Component Property command.
    Property(PropertyCommand),
    /// 상태 머신 교체(inverse는 이전 상태 머신).
    StateMachine {
        /// 컴포넌트.
        component: ComponentRef,
        /// 새 상태 머신.
        machine: StateMachine,
    },
    /// 배치 hint 고정(`None`이면 해제).
    Hint {
        /// 소유자.
        owner: MotionOwner,
        /// node.
        node: StableId,
        /// 좌표.
        point: Option<HintPoint>,
    },
    /// Context requirement 추가(`add`) 또는 삭제(inverse는 반대 연산).
    ContextRequirement {
        /// 컴포넌트.
        component: ComponentRef,
        /// key.
        key: StableId,
        /// 추가인지.
        add: bool,
    },
}

/// step 실패.
#[derive(Debug, Clone, PartialEq)]
pub enum StepError {
    /// binding command.
    Binding(BindingCommandError),
    /// MotionSpec command.
    Motion(MotionCommandError),
    /// Property command.
    Property(PropertyCommandError),
    /// 상태 머신 교체.
    StateMachine(StateMachineError),
    /// Context requirement 연산(이미 있는 추가·없는 삭제).
    Context(&'static str),
    /// 시험용 주입 실패.
    InjectedFault,
}

impl fmt::Display for StepError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StepError::Binding(e) => write!(f, "binding: {e:?}"),
            StepError::Motion(e) => write!(f, "motion: {e}"),
            StepError::Property(e) => write!(f, "property: {e:?}"),
            StepError::StateMachine(e) => write!(f, "상태 머신: {e:?}"),
            StepError::Context(reason) => write!(f, "Context: {reason}"),
            StepError::InjectedFault => f.write_str("주입된 실패"),
        }
    }
}

impl std::error::Error for StepError {}

/// transaction 실패: `step` 번째 step이 실패해 그 앞 step을 모두 되돌렸습니다.
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionError {
    /// 실패한 step 순번.
    pub step: usize,
    /// 원인.
    pub error: StepError,
}

impl fmt::Display for TransactionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}번째 step 실패(전부 되돌림): {}",
            self.step, self.error
        )
    }
}

impl std::error::Error for TransactionError {}

/// 사용자 동작 하나.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PrototypeTransaction {
    /// 표시 이름.
    pub label: String,
    /// step(순서대로 적용).
    pub steps: Vec<PrototypeStep>,
}

/// 적용 결과.
#[derive(Debug, Clone, PartialEq)]
pub struct AppliedTransaction {
    /// Undo 1회로 적용할 inverse transaction.
    pub inverse: PrototypeTransaction,
}

fn apply_step(
    doc: &mut PrototypeDocument,
    step: &PrototypeStep,
) -> Result<PrototypeStep, StepError> {
    match step {
        PrototypeStep::Binding { component, command } => {
            let mut bindings = doc.bindings.clone();
            let inverse = {
                let types = OwnerTypes {
                    doc,
                    component: *component,
                };
                let targets = OwnerTargets {
                    doc,
                    component: *component,
                };
                let context = BindingContext {
                    tokens: &doc.tokens,
                    others: &types,
                    targets: &targets,
                };
                command
                    .clone()
                    .apply(&mut bindings, &context)
                    .map_err(StepError::Binding)?
            };
            doc.bindings = bindings;
            Ok(PrototypeStep::Binding {
                component: *component,
                command: inverse,
            })
        }
        PrototypeStep::Motion(command) => {
            let mut motions = doc.motions.clone();
            let world = LibraryWorld {
                library: &doc.library,
                model: None,
            };
            let outcome = command
                .apply(&mut motions, &world)
                .map_err(StepError::Motion)?;
            doc.motions = motions;
            Ok(PrototypeStep::Motion(outcome.inverse))
        }
        PrototypeStep::Property(command) => {
            let outcome = command
                .apply(&mut doc.library, &mut doc.drilling)
                .map_err(StepError::Property)?;
            Ok(PrototypeStep::Property(outcome.inverse))
        }
        PrototypeStep::StateMachine { component, machine } => {
            let previous = doc
                .library
                .set_state_machine(*component, machine.clone())
                .map_err(StepError::StateMachine)?;
            Ok(PrototypeStep::StateMachine {
                component: *component,
                machine: previous,
            })
        }
        PrototypeStep::Hint { owner, node, point } => {
            let hints = doc.hints.entry(*owner).or_default();
            let previous = match point {
                Some(p) => hints.pin(*node, *p),
                None => hints.unpin(*node),
            };
            if hints.is_empty() {
                doc.hints.remove(owner);
            }
            Ok(PrototypeStep::Hint {
                owner: *owner,
                node: *node,
                point: previous,
            })
        }
        PrototypeStep::ContextRequirement {
            component,
            key,
            add,
        } => {
            let changed = if *add {
                doc.context.add_requirement(component.id(), *key)
            } else {
                doc.context.remove_requirement(component.id(), *key)
            };
            if !changed {
                return Err(StepError::Context(if *add {
                    "requirement가 이미 있습니다"
                } else {
                    "requirement가 없습니다"
                }));
            }
            Ok(PrototypeStep::ContextRequirement {
                component: *component,
                key: *key,
                add: !*add,
            })
        }
    }
}

impl PrototypeTransaction {
    /// transaction을 만듭니다.
    pub fn new(label: impl Into<String>, steps: Vec<PrototypeStep>) -> PrototypeTransaction {
        PrototypeTransaction {
            label: label.into(),
            steps,
        }
    }

    /// 전부 적용하거나 전부 되돌립니다.
    pub fn apply(
        &self,
        doc: &mut PrototypeDocument,
    ) -> Result<AppliedTransaction, TransactionError> {
        self.apply_with_fault(doc, None)
    }

    /// `fault` 번째 step 직전에 실패를 주입해 적용합니다(fault injection 시험용). 실패하면 문서는 적용 전과 같습니다.
    pub fn apply_with_fault(
        &self,
        doc: &mut PrototypeDocument,
        fault: Option<usize>,
    ) -> Result<AppliedTransaction, TransactionError> {
        // 실패하면 이 snapshot으로 되돌립니다. inverse를 거꾸로 적용하는 것과 결과가 같지만, inverse 적용이 실패할 수
        // 있는 경로를 두지 않습니다.
        let snapshot = doc.clone();
        let mut inverses = Vec::with_capacity(self.steps.len());
        for (index, step) in self.steps.iter().enumerate() {
            let result = if fault == Some(index) {
                Err(StepError::InjectedFault)
            } else {
                apply_step(doc, step)
            };
            match result {
                Ok(inverse) => inverses.push(inverse),
                Err(error) => {
                    *doc = snapshot;
                    return Err(TransactionError { step: index, error });
                }
            }
        }
        inverses.reverse();
        Ok(AppliedTransaction {
            inverse: PrototypeTransaction {
                label: format!("{} 되돌리기", self.label),
                steps: inverses,
            },
        })
    }
}
