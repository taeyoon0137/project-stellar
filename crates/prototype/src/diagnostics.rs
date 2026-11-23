//! 진단 → 그래프 node·socket 매핑 (XXX.2.5, ADR-0094 결정 3).
//!
//! expression 타입 오류(VIII.3.4), binding 포트 타입 불일치(XXX.1.2), 상태 머신 진단(IX.5.7), MotionSpec 진단(XXX.1.1·1.6)을
//! 그래프 위치([`GraphLocation`])로 옮깁니다. Context 진단(XXX.7.5)은 문서 validator(`validate_context`)가 계산한 결과를
//! [`context_graph_diagnostics`]가 같은 위치 규칙으로 옮깁니다(validator는 문서 모델이 필요해 호출자가 넘김).
//!
//! | 진단 | 위치 |
//! |---|---|
//! | expression 타입 오류 | 오류 block, slot이 있으면 그 `operand` socket |
//! | binding 타입이 포트 집합 밖 | PropertyOutput의 `value` socket |
//! | 포트로 노출하지 않는 속성의 binding | PropertyOutput node |
//! | transition unreachable·duplicate | Transition node |
//! | transition source 없음 | Transition의 `trigger` socket |
//! | MotionSpec 대상 orphan·보간 불가 | MotionSpec의 `drive` socket(대상 순번) |
//! | MotionSpec 시작 변수 없음·조건 타입 | MotionSpec의 `start` socket |
//! | Context 선언 없는 소비·없는 key 소비·default 해석·shadowing(binding) | 그 key를 읽는 참조 block의 `reference` socket(block이 여럿이면 각각) |
//! | Context 타입 불일치(binding) | PropertyOutput의 `value` socket |
//! | 쓰지 않는·없는 key의 requirement | 그 key의 ContextRead node |
//! | provision·Slot 공개 목록 진단 | 그래프 위치 없음(provision은 그래프 node가 아님) |

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::ComponentRef;
use stellar_document::component::state_diagnostics::{diagnose_state_machine, StateDiagnostic};
use stellar_document::context::validate::{ContextDiagnostic, ContextDiagnosticKind};
use stellar_document::motion::{diagnose, LibraryWorld, MotionDiagnostic, MotionOwner};
use stellar_document::prototype::node_kind::PrototypeNodeKind;
use stellar_document::prototype::socket::{check_state_connection, property_port};
use stellar_expressions::ast::{NodePayload, ReferenceTarget};
use stellar_expressions::typecheck::type_check;

use crate::edit::{OwnerTypes, PrototypeDocument};
use crate::graph::{GraphNodeId, GraphSocket, ProjectionScope, SocketKey};

/// 그래프 위치.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GraphLocation {
    /// node.
    pub node: GraphNodeId,
    /// socket(node 전체 진단이면 `None`).
    pub socket: Option<GraphSocket>,
}

/// 그래프 진단.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GraphDiagnostic {
    /// 위치.
    pub location: GraphLocation,
    /// 안정 코드.
    pub code: &'static str,
    /// 사람이 읽는 설명.
    pub message: String,
}

fn at(node: GraphNodeId, socket: Option<(&'static str, SocketKey)>) -> GraphLocation {
    GraphLocation {
        node,
        socket: socket.map(|(name, key)| GraphSocket::new(node, name, key)),
    }
}

/// 컴포넌트 소유자의 진단을 그래프 위치로 매핑합니다(위치·코드 순).
pub fn graph_diagnostics(doc: &PrototypeDocument, component: ComponentRef) -> Vec<GraphDiagnostic> {
    let mut out = Vec::new();
    let scope = ProjectionScope::Component(component);
    let input = crate::graph::ProjectionInput {
        library: &doc.library,
        bindings: &doc.bindings,
        motions: &doc.motions,
    };
    let types = OwnerTypes { doc, component };
    for binding in crate::graph::scoped_bindings(&scope, input) {
        let output = GraphNodeId::new(binding.id().stable_id(), PrototypeNodeKind::PropertyOutput);
        let report = type_check(binding.source(), &types);
        for error in report.errors() {
            let block =
                GraphNodeId::new(error.node.stable_id(), PrototypeNodeKind::ExpressionBlock);
            out.push(GraphDiagnostic {
                location: at(block, error.slot.map(|s| ("operand", SocketKey::Slot(s)))),
                code: "expression-type",
                message: error.to_string(),
            });
        }
        let target = binding.target();
        let port = doc
            .target_kind(component, target.root())
            .and_then(|kind| target.keys().first().map(|key| property_port(kind, *key)));
        match port {
            Some(Ok(port)) => {
                if check_state_connection(&port, binding.expected_type()).is_err() {
                    out.push(GraphDiagnostic {
                        location: at(output, Some(("value", SocketKey::Single))),
                        code: "port-type-mismatch",
                        message: format!("{} 포트가 받지 않는 타입입니다", port.name),
                    });
                }
            }
            Some(Err(diag)) => out.push(GraphDiagnostic {
                location: at(output, None),
                code: diag.code,
                message: diag.message,
            }),
            None => out.push(GraphDiagnostic {
                location: at(output, None),
                code: "prototype-target-outside-owner",
                message: "대상 node가 정의 tree에 없습니다".into(),
            }),
        }
    }
    for diag in diagnose_state_machine(&doc.library, component) {
        let (transition, socket, code) = match &diag {
            StateDiagnostic::Unreachable { transition, .. } => {
                (*transition, None, "state-unreachable-transition")
            }
            StateDiagnostic::DuplicateCondition { transition, .. } => {
                (*transition, None, "state-duplicate-condition")
            }
            StateDiagnostic::MissingTarget { transition, .. } => (
                *transition,
                Some(("trigger", SocketKey::Single)),
                "state-missing-target",
            ),
        };
        let node = GraphNodeId::new(transition.stable_id(), PrototypeNodeKind::Transition);
        out.push(GraphDiagnostic {
            location: at(node, socket),
            code,
            message: format!("{diag:?}"),
        });
    }
    let world = LibraryWorld {
        library: &doc.library,
        model: None,
    };
    for spec in doc.motions.owned_by(MotionOwner::Component(component)) {
        let node = GraphNodeId::new(spec.id().stable_id(), PrototypeNodeKind::MotionSpec);
        for diag in diagnose(spec, &world) {
            let socket = match &diag {
                MotionDiagnostic::OrphanTarget { target, .. }
                | MotionDiagnostic::NotInterpolable { target, .. } => {
                    let index = spec.targets().iter().position(|t| t == target).unwrap_or(0);
                    ("drive", SocketKey::Target(index as u16))
                }
                MotionDiagnostic::MissingVariable { .. }
                | MotionDiagnostic::ConditionTypeMismatch { .. } => ("start", SocketKey::Single),
            };
            out.push(GraphDiagnostic {
                location: at(node, Some(socket)),
                code: diag.code(),
                message: diag.to_string(),
            });
        }
    }
    out.sort();
    out
}

/// Context validator 진단(XXX.7.5) 가운데 `component` 소유자 그래프에 위치가 있는 것을 옮깁니다(모듈 문서 표, 위치·코드 순).
pub fn context_graph_diagnostics(
    doc: &PrototypeDocument,
    component: ComponentRef,
    diagnostics: &[ContextDiagnostic],
) -> Vec<GraphDiagnostic> {
    let scope = ProjectionScope::Component(component);
    let input = crate::graph::ProjectionInput {
        library: &doc.library,
        bindings: &doc.bindings,
        motions: &doc.motions,
    };
    let scoped: Vec<_> = crate::graph::scoped_bindings(&scope, input).collect();
    let binding = |id: StableId| scoped.iter().find(|b| b.id().stable_id() == id);
    // binding source에서 key를 읽는 참조 block의 `reference` socket.
    let readers = |id: StableId, key: StableId| -> Vec<GraphLocation> {
        binding(id)
            .map(|b| {
                let source = b.source();
                source
                    .nodes()
                    .iter()
                    .filter(|node| {
                        matches!(node.payload(), NodePayload::Reference(ReferenceTarget::Context(k)) if *k == key)
                    })
                    .map(|node| {
                        let block = GraphNodeId::new(
                            node.id().stable_id(),
                            PrototypeNodeKind::ExpressionBlock,
                        );
                        at(block, Some(("reference", SocketKey::Single)))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let read_node = |key: StableId| at(GraphNodeId::new(key, PrototypeNodeKind::ContextRead), None);
    let mut out = Vec::new();
    for diagnostic in diagnostics {
        let kind = &diagnostic.kind;
        let locations: Vec<GraphLocation> = match kind {
            ContextDiagnosticKind::UndeclaredConsumption { reader, key, .. } => {
                readers(*reader, *key)
            }
            ContextDiagnosticKind::OrphanConsumption { binding, key }
            | ContextDiagnosticKind::DefaultResolved { binding, key }
            | ContextDiagnosticKind::Shadowing { binding, key, .. } => readers(*binding, *key),
            ContextDiagnosticKind::TypeMismatch { reader, .. } => binding(*reader)
                .map(|b| {
                    let output =
                        GraphNodeId::new(b.id().stable_id(), PrototypeNodeKind::PropertyOutput);
                    vec![at(output, Some(("value", SocketKey::Single)))]
                })
                .unwrap_or_default(),
            ContextDiagnosticKind::UnusedRequirement {
                component: owner,
                key,
            }
            | ContextDiagnosticKind::OrphanRequirement {
                component: owner,
                key,
            } if *owner == component.id() => vec![read_node(*key)],
            _ => Vec::new(),
        };
        for location in locations {
            out.push(GraphDiagnostic {
                location,
                code: kind.code(),
                message: kind.to_string(),
            });
        }
    }
    out.sort();
    out
}
