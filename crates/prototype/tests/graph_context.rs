//! 그래프의 Component Context: ContextRead projection과 Context 진단의 그래프 위치(XXX.2.5 남은 범위, ADR-0114 결정 7).

mod support;

use stellar_document::context::validate::{ContextDiagnostic, ContextDiagnosticKind};
use stellar_document::prototype::node_kind::PrototypeNodeKind;
use stellar_expressions::types::{ExpressionType, TypeSpec};
use stellar_prototype::diagnostics::context_graph_diagnostics;
use stellar_prototype::edit::PrototypeDocument;
use stellar_prototype::graph::{
    GraphNodeId, GraphSocket, NodeDetail, ProjectionInput, ProjectionScope, PrototypeGraphView,
    SocketKey,
};
use support::*;

fn project(doc: &PrototypeDocument) -> PrototypeGraphView {
    PrototypeGraphView::project(
        &ProjectionScope::Component(toggle()),
        ProjectionInput {
            library: &doc.library,
            bindings: &doc.bindings,
            motions: &doc.motions,
        },
    )
}

#[test]
fn context_reads_project_as_one_node_per_key() {
    let doc = context_document();
    let view = project(&doc);
    let read = GraphNodeId::new(context_key(), PrototypeNodeKind::ContextRead);
    let node = view.node(read).expect("ContextRead node");
    assert_eq!(node.detail, NodeDetail::ContextRead { key: context_key() });
    let block = GraphNodeId::new(sid(CONTEXT_READ), PrototypeNodeKind::ExpressionBlock);
    assert_eq!(
        view.source_of(GraphSocket::new(block, "reference", SocketKey::Single)),
        Some(GraphSocket::new(read, "value", SocketKey::Single))
    );
    // projection은 원본 의미를 잃지 않고(binding 되살림), 같은 입력은 같은 bytes입니다.
    let lowered = stellar_prototype::graph::lower_bindings(&view);
    assert!(lowered
        .iter()
        .any(|(id, _, ast)| id.stable_id() == sid(CONTEXT_BINDING)
            && ast
                == doc
                    .bindings
                    .iter()
                    .find(|b| b.id().stable_id() == sid(CONTEXT_BINDING))
                    .unwrap()
                    .source()));
    assert_eq!(view.to_bytes(), project(&doc).to_bytes());
    // Context 읽기가 없는 문서의 projection에는 ContextRead node가 없습니다(기존 bytes 그대로).
    assert!(project(&document())
        .nodes()
        .iter()
        .all(|n| n.id.kind != PrototypeNodeKind::ContextRead));
}

#[test]
fn context_diagnostics_map_to_graph_positions() {
    // 검증 기준(XXX.2.5 남은 범위): Context 진단의 오류 위치 fixture 정확.
    let doc = context_document();
    let block = GraphNodeId::new(sid(CONTEXT_READ), PrototypeNodeKind::ExpressionBlock);
    let output = GraphNodeId::new(sid(CONTEXT_BINDING), PrototypeNodeKind::PropertyOutput);
    let read = GraphNodeId::new(context_key(), PrototypeNodeKind::ContextRead);
    let diagnostics = [
        ContextDiagnosticKind::UndeclaredConsumption {
            component: toggle().id(),
            key: context_key(),
            reader: sid(CONTEXT_BINDING),
        },
        ContextDiagnosticKind::TypeMismatch {
            reader: sid(CONTEXT_BINDING),
            expected: TypeSpec::required(ExpressionType::Color),
            found: None,
        },
        ContextDiagnosticKind::UnusedRequirement {
            component: toggle().id(),
            key: context_key(),
        },
        // 다른 컴포넌트의 requirement와 provision 진단은 이 그래프에 위치가 없습니다.
        ContextDiagnosticKind::UnusedRequirement {
            component: other().id(),
            key: context_key(),
        },
        ContextDiagnosticKind::OrphanProvision {
            provision: sid(720),
            key: context_key(),
        },
    ]
    .into_iter()
    .map(|kind| ContextDiagnostic { kind })
    .collect::<Vec<_>>();
    let mapped = context_graph_diagnostics(&doc, toggle(), &diagnostics);
    let found: Vec<(GraphNodeId, Option<&str>, &str)> = mapped
        .iter()
        .map(|d| (d.location.node, d.location.socket.map(|s| s.name), d.code))
        .collect();
    assert_eq!(found.len(), 3, "{found:?}");
    assert!(found.contains(&(output, Some("value"), "context.type-mismatch")));
    assert!(found.contains(&(block, Some("reference"), "context.undeclared-consumption")));
    assert!(found.contains(&(read, None, "context.unused-requirement")));
}
