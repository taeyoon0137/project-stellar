//! projection 결정론과 원본 의미 보존 (XXX.2.1).

mod support;

use stellar_document::component::state_machine::TransitionSpec;
use stellar_document::prototype::node_kind::PrototypeNodeKind;
use stellar_prototype::edit::PrototypeDocument;
use stellar_prototype::graph::{
    lower_bindings, lower_transitions, GraphNodeId, GraphSocket, ProjectionInput, ProjectionScope,
    PrototypeGraphView, SocketKey,
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
fn projection_is_deterministic_and_lowers_to_same_meaning() {
    // 검증 기준(XXX.2.1): 같은 입력 projection bytes 동일, projection→편집 plan→원본 의미 동일.
    let doc = document();
    let a = project(&doc);
    let b = project(&document());
    assert_eq!(a.to_bytes(), b.to_bytes());
    // node·edge 순서는 StableId 기준.
    let ids: Vec<GraphNodeId> = a.nodes().iter().map(|n| n.id).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);
    // 원본 binding을 projection에서 다시 만들면 canonical AST가 같습니다.
    let lowered = lower_bindings(&a);
    assert_eq!(lowered.len(), 3);
    for (id, target, ast) in lowered {
        let original = doc.bindings.get(id).expect("binding");
        assert_eq!(original.target(), &target);
        assert_eq!(
            original.source().to_canonical_bytes().expect("bytes"),
            ast.to_canonical_bytes().expect("bytes")
        );
    }
    let machine = doc.library.get(toggle()).expect("entry").state_machine();
    let original: Vec<TransitionSpec> = machine.transitions().iter().map(|t| t.to_spec()).collect();
    assert_eq!(lower_transitions(&a), original);
    // 상태 변수 → 참조 block, MotionSpec → 대상 속성 출력 edge가 있습니다.
    let motion = GraphNodeId::new(motion_id().stable_id(), PrototypeNodeKind::MotionSpec);
    let drives: Vec<_> = a.edges().iter().filter(|e| e.from.node == motion).collect();
    assert_eq!(drives.len(), 2);
    let start = GraphSocket::new(motion, "start", SocketKey::Single);
    assert_eq!(
        a.source_of(start).map(|s| s.node),
        Some(GraphNodeId::new(
            on().stable_id(),
            PrototypeNodeKind::StateVariable
        ))
    );
    // 모든 edge의 socket 이름은 registry에 있고 출력→입력 종류가 같습니다.
    for edge in a.edges() {
        let from = edge
            .from
            .node
            .kind
            .socket(edge.from.name)
            .expect("출력 socket");
        let to = edge.to.node.kind.socket(edge.to.name).expect("입력 socket");
        assert!(!from.input && to.input, "{edge:?}");
        assert_eq!(from.kind, to.kind, "{edge:?}");
    }
}
