//! 진단 → 그래프 위치 매핑 (XXX.2.5).

mod support;

use stellar_bindings::binding::{BindingId, ScopeOwner, ValueBinding};
use stellar_document::component::event::EventKind;
use stellar_document::component::state_machine::{
    StateVariableId, TransitionId, TransitionSpec, Trigger,
};
use stellar_document::motion::{MotionCommand, MotionSpec, MotionSpecInput};
use stellar_document::prototype::node_kind::PrototypeNodeKind;
use stellar_expressions::ast::{ChildSlot, ExprNode, ExpressionAst};
use stellar_expressions::ops::UnaryOp;
use stellar_expressions::types::ExpressionType;
use stellar_expressions::types::TypeSpec;
use stellar_prototype::diagnostics::graph_diagnostics;
use stellar_prototype::edit::{PrototypeStep, PrototypeTransaction};
use stellar_prototype::graph::{GraphNodeId, GraphSocket, SocketKey};
use support::*;

#[test]
fn diagnostics_map_to_graph_nodes_and_sockets() {
    // 검증 기준(XXX.2.5): 오류 위치 fixture 정확.
    let mut doc = document();
    // 1. 대상이 없는 MotionSpec(Restore는 대상 검사를 하지 않음) → drive socket 1.
    let orphan = MotionSpec::new(MotionSpecInput {
        id: stellar_document::motion::MotionSpecId::new(sid(600)),
        targets: vec![background(ROOT), background(999)],
        start: stellar_document::motion::StartCondition::Changes {
            variable: StateVariableId::new(sid(998)),
        },
        ..motion(100.0, stellar_document::motion::ReducedMotion::Instant).to_input()
    })
    .expect("spec");
    PrototypeTransaction::new(
        "orphan",
        vec![PrototypeStep::Motion(MotionCommand::Restore(orphan))],
    )
    .apply(&mut doc)
    .expect("적용");
    // 2. source가 없는 transition → trigger socket.
    let entry = doc.library.get(toggle()).expect("entry").clone();
    let machine = stellar_prototype::plan::rebuild_machine(&entry, |_, transitions| {
        transitions.push(TransitionSpec {
            id: TransitionId::new(sid(610)),
            trigger: Trigger::Event {
                kind: EventKind::Activate,
                source: sid(997),
            },
            condition: None,
            actions: Vec::new(),
        });
    })
    .expect("machine");
    doc.library
        .set_state_machine(toggle(), machine)
        .expect("set");
    // 3. 타입 오류 식(`not 5px`)을 가진 binding → 오류 block의 operand socket.
    let bad = ExpressionAst::new(
        x(620),
        vec![
            ExprNode::unary(x(620), UnaryOp::Not, x(621)),
            ExprNode::literal(x(621), px(5.0)),
        ],
    )
    .expect("AST");
    doc.bindings
        .insert(ValueBinding::new(
            BindingId::new(sid(622)),
            left(LABEL),
            bad,
            TypeSpec::required(ExpressionType::Length),
            ScopeOwner::Component(toggle()),
        ))
        .expect("binding");
    let found = graph_diagnostics(&doc, toggle());
    let motion_node = GraphNodeId::new(sid(600), PrototypeNodeKind::MotionSpec);
    let has = |code: &str, node: GraphNodeId, socket: Option<(&'static str, SocketKey)>| {
        found.iter().any(|d| {
            d.code == code
                && d.location.node == node
                && d.location.socket == socket.map(|(n, k)| GraphSocket::new(node, n, k))
        })
    };
    assert!(
        has(
            "motion-orphan-target",
            motion_node,
            Some(("drive", SocketKey::Target(1)))
        ),
        "{found:#?}"
    );
    assert!(has(
        "motion-missing-variable",
        motion_node,
        Some(("start", SocketKey::Single))
    ));
    let transition = GraphNodeId::new(sid(610), PrototypeNodeKind::Transition);
    assert!(has(
        "state-missing-target",
        transition,
        Some(("trigger", SocketKey::Single))
    ));
    let block = GraphNodeId::new(sid(620), PrototypeNodeKind::ExpressionBlock);
    assert!(
        has(
            "expression-type",
            block,
            Some(("operand", SocketKey::Slot(ChildSlot::Operand)))
        ),
        "{found:#?}"
    );
    // 원래 fixture는 진단이 없습니다(새로 넣은 원본만).
    let output = GraphNodeId::new(sid(622), PrototypeNodeKind::PropertyOutput);
    assert!(
        found
            .iter()
            .all(|d| [motion_node, transition, block, output].contains(&d.location.node)),
        "{found:#?}"
    );
}
