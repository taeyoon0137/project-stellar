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
    // 고정한 node를 지우면 hint도 같은 transaction에서 풀리고, Undo 1회로 둘 다 돌아옵니다.
    pin.apply(&mut doc).expect("고정");
    let delete = planner_edit(
        &doc,
        GraphEdit::DeleteNode(GraphNodeId::new(sid(80), PrototypeNodeKind::PropertyOutput)),
    );
    assert_eq!(delete.steps.len(), 2, "binding 삭제 + hint 해제");
    let before = doc.state_bytes();
    let applied = delete.apply(&mut doc).expect("삭제");
    assert!(doc.hints.is_empty(), "지운 node의 hint가 남지 않음");
    applied.inverse.apply(&mut doc).expect("Undo");
    assert_eq!(doc.state_bytes(), before);
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

#[test]
fn connection_check_does_not_depend_on_stored_value_variant() {
    // 검증 기준(XXX.1.2): 저장 값을 Length에서 Percentage로 바꿔도 연결 검사 결과 불변. 실제 binding을 Length로 저장한
    // 문서와 Percentage로 바꿔 저장한 문서에서 같은 연결 plan의 결과가 같아야 합니다.
    let mut doc = document();
    let percent = TypedValue::Percentage(Percentage::new(50.0).expect("%"));
    let connect = |doc: &PrototypeDocument, value: TypedValue, binding: Option<BindingId>| {
        Planner::new(doc, toggle()).plan(&GraphEdit::Connect {
            from: Endpoint::Expression(literal_ast(420, value)),
            to: Endpoint::PropertyInput(width(ROOT)),
            binding,
            reference_node: None,
        })
    };
    let outcomes = |doc: &PrototypeDocument| {
        [px(10.0), percent.clone(), green()].map(|value| {
            connect(doc, value, None)
                .map(|_| ())
                .map_err(|e| format!("{e:?}"))
        })
    };
    // 저장 값 Length.
    connect(&doc, px(100.0), Some(BindingId::new(sid(421))))
        .expect("Length plan")
        .apply(&mut doc)
        .expect("Length 적용");
    let with_length = outcomes(&doc);
    // 저장 값 Percentage로 교체.
    connect(&doc, percent.clone(), None)
        .expect("Percentage plan")
        .apply(&mut doc)
        .expect("Percentage 적용");
    let stored = doc.bindings.get(BindingId::new(sid(421))).expect("binding");
    assert_eq!(
        stored.expected_type(),
        stellar_expressions::types::TypeSpec::required(ExpressionType::Percentage),
        "저장 variant가 바뀌었습니다"
    );
    let with_percent = outcomes(&doc);
    assert_eq!(with_length, with_percent);
    assert!(with_length[0].is_ok() && with_length[1].is_ok() && with_length[2].is_err());
}
