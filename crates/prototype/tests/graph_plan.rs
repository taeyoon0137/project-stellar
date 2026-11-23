//! 그래프 편집 plan과 socket·포트 연결 규칙 (XXX.2.3, XXX.1.2).

mod support;

use stellar_bindings::binding::BindingId;
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::Percentage;
use stellar_document::component::event::EventKind;
use stellar_document::component::state_machine::TransitionId;
use stellar_document::prototype::layout_hint::HintPoint;
use stellar_document::prototype::node_kind::{ConversionKind, PrototypeNodeKind};
use stellar_document::prototype::socket::ConnectionError;
use stellar_expressions::types::ExpressionType;
use stellar_prototype::edit::{PrototypeDocument, PrototypeStep, PrototypeTransaction};
use stellar_prototype::graph::GraphNodeId;
use stellar_prototype::plan::{Endpoint, GraphEdit, PlanError, Planner};
use support::*;

fn apply_and_undo(doc: &mut PrototypeDocument, tx: &PrototypeTransaction) {
    let before = doc.state_bytes();
    let applied = tx.apply(doc).expect("적용");
    assert_ne!(doc.state_bytes(), before, "변경이 있어야 합니다");
    applied.inverse.apply(doc).expect("Undo");
    assert_eq!(doc.state_bytes(), before, "Undo 1회로 동작 전 상태");
}

#[test]
fn graph_edits_lower_to_commands_and_undo_once() {
    // 검증 기준(XXX.2.3): 각 plan 적용 후 valid 문서, apply/inverse 통과, Undo 1회로 동작 전 상태 복원.
    let mut doc = document();
    let planner_edit = |doc: &PrototypeDocument, edit: GraphEdit| {
        Planner::new(doc, toggle()).plan(&edit).expect("plan")
    };
    // 상태 값 → 새 속성 입력(label opacity가 없으므로 label left).
    let connect = planner_edit(
        &doc,
        GraphEdit::Connect {
            from: Endpoint::Expression(literal_ast(300, px(8.0))),
            to: Endpoint::PropertyInput(left(LABEL)),
            binding: Some(BindingId::new(sid(301))),
            reference_node: None,
        },
    );
    apply_and_undo(&mut doc, &connect);
    // 기존 binding을 상태 값 참조로 바꾸기는 Replace입니다(타입 불일치면 거부).
    let bad = Planner::new(&doc, toggle()).plan(&GraphEdit::Connect {
        from: Endpoint::StateValue(on()),
        to: Endpoint::PropertyInput(left(KNOB)),
        binding: None,
        reference_node: Some(sid(302)),
    });
    assert!(matches!(
        bad,
        Err(PlanError::Connection(
            ConnectionError::TypeNotAccepted { .. }
        ))
    ));
    // 변환 node(토글)로 이벤트 → 상태.
    let toggle_edit = planner_edit(
        &doc,
        GraphEdit::AddConversion {
            kind: ConversionKind::PulseToggle,
            transition: TransitionId::new(sid(310)),
            source: sid(LABEL),
            event: EventKind::LongPress,
            variable: on(),
            expression_ids: [sid(311), sid(312), sid(313)],
        },
    );
    apply_and_undo(&mut doc, &toggle_edit);
    let counter = planner_edit(
        &doc,
        GraphEdit::AddConversion {
            kind: ConversionKind::PulseCounter,
            transition: TransitionId::new(sid(320)),
            source: sid(ROOT),
            event: EventKind::PointerDown,
            variable: count(),
            expression_ids: [sid(321), sid(322), sid(323)],
        },
    );
    apply_and_undo(&mut doc, &counter);
    // MotionSpec 대상 추가·해제, 시작 조건 변경.
    let drive = planner_edit(
        &doc,
        GraphEdit::Connect {
            from: Endpoint::MotionDrive(motion_id()),
            to: Endpoint::PropertyMotion(width(KNOB)),
            binding: None,
            reference_node: None,
        },
    );
    apply_and_undo(&mut doc, &drive);
    let undrive = planner_edit(
        &doc,
        GraphEdit::Disconnect {
            from: Endpoint::MotionDrive(motion_id()),
            to: Endpoint::PropertyMotion(left(KNOB)),
        },
    );
    apply_and_undo(&mut doc, &undrive);
    // node 삭제(binding·MotionSpec·transition)와 위치 고정.
    for node in [
        GraphNodeId::new(sid(80), PrototypeNodeKind::PropertyOutput),
        GraphNodeId::new(motion_id().stable_id(), PrototypeNodeKind::MotionSpec),
        GraphNodeId::new(sid(50), PrototypeNodeKind::Transition),
    ] {
        let tx = planner_edit(&doc, GraphEdit::DeleteNode(node));
        apply_and_undo(&mut doc, &tx);
    }
    let pin = planner_edit(
        &doc,
        GraphEdit::PinNode {
            node: sid(80),
            point: Some(HintPoint::new(10.0, 20.0).expect("좌표")),
        },
    );
    apply_and_undo(&mut doc, &pin);
    // 쓰는 곳이 있는 상태 변수는 지우지 않습니다.
    assert!(matches!(
        Planner::new(&doc, toggle()).plan(&GraphEdit::DeleteNode(GraphNodeId::new(
            on().stable_id(),
            PrototypeNodeKind::StateVariable
        ))),
        Err(PlanError::InUse(_))
    ));
}

#[test]
fn socket_kinds_and_port_types_are_enforced() {
    // 검증 기준(XXX.1.2): 종류가 다른 직접 연결 거부, 변환 node 경유 연결 허용, width 포트 Length·Percentage 허용·Color 거부.
    let doc = document();
    let planner = Planner::new(&doc, toggle());
    let pulse_to_state = planner.plan(&GraphEdit::Connect {
        from: Endpoint::EventPulse {
            source: sid(ROOT),
            kind: EventKind::Activate,
        },
        to: Endpoint::PropertyInput(background(ROOT)),
        binding: Some(BindingId::new(sid(400))),
        reference_node: None,
    });
    assert!(matches!(
        pulse_to_state,
        Err(PlanError::Connection(
            ConnectionError::SocketKindMismatch { .. }
        ))
    ));
    let state_to_pulse = planner.plan(&GraphEdit::Connect {
        from: Endpoint::StateValue(on()),
        to: Endpoint::TransitionTrigger(TransitionId::new(sid(50))),
        binding: None,
        reference_node: Some(sid(401)),
    });
    assert!(matches!(
        state_to_pulse,
        Err(PlanError::Connection(
            ConnectionError::SocketKindMismatch { .. }
        ))
    ));
    let pulse_to_write = planner.plan(&GraphEdit::Connect {
        from: Endpoint::EventPulse {
            source: sid(ROOT),
            kind: EventKind::Activate,
        },
        to: Endpoint::StateWrite(on()),
        binding: None,
        reference_node: None,
    });
    assert!(matches!(
        pulse_to_write,
        Err(PlanError::NoSuchConnection(_))
    ));
    let state_change = planner.plan(&GraphEdit::AddConversion {
        kind: ConversionKind::StateChangeToPulse,
        transition: TransitionId::new(sid(402)),
        source: sid(ROOT),
        event: EventKind::Activate,
        variable: on(),
        expression_ids: [sid(403), sid(404), sid(405)],
    });
    assert!(matches!(state_change, Err(PlanError::Unsupported(_))));
    // width 포트.
    let width_plan = |value: TypedValue| {
        planner.plan(&GraphEdit::Connect {
            from: Endpoint::Expression(literal_ast(410, value)),
            to: Endpoint::PropertyInput(width(ROOT)),
            binding: Some(BindingId::new(sid(411))),
            reference_node: None,
        })
    };
    assert!(width_plan(px(100.0)).is_ok());
    let percent = TypedValue::Percentage(Percentage::new(50.0).expect("%"));
    let tx = width_plan(percent).expect("Percentage 허용");
    let PrototypeStep::Binding { command, .. } = &tx.steps[0] else {
        panic!("binding step");
    };
    assert!(
        format!("{command:?}").contains("Percentage"),
        "저장 variant는 식의 정적 타입"
    );
    assert!(matches!(
        width_plan(green()),
        Err(PlanError::Connection(
            ConnectionError::TypeNotAccepted { .. }
        ))
    ));
    let _ = ExpressionType::Length;
}
