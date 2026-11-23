//! 포트 공개 plan (XXX.2.4).

mod support;

use stellar_bindings::binding::BindingId;
use stellar_document::component::definition::ComponentPropertyId;
use stellar_document::component::event::EventKind;
use stellar_document::component::state_machine::{SemanticActionId, TransitionId};
use stellar_expressions::types::ExpressionType;
use stellar_prototype::edit::{PrototypeDocument, PrototypeTransaction};
use stellar_prototype::expose::{plan_expose, ExposeRequest, ExposedItem};
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
fn expose_port_is_atomic_and_rejects_nested_internal_edges() {
    // 검증 기준(XXX.2.4): nested 직접 edge fixture 거부, fault injection 시 생성된 Property 0.
    let mut doc = document();
    let request = ExposeRequest::PropertyInput {
        component: toggle(),
        target: background(ROOT),
        property: ComponentPropertyId::new(sid(500)),
        name: "tint".into(),
        value_type: ExpressionType::Color,
        default: literal_ast(501, green()),
        binding: BindingId::new(sid(502)),
        reference_node: sid(503),
    };
    let plan = plan_expose(&doc, &request).expect("plan");
    assert_eq!(plan.preview.len(), 2);
    let properties_before = doc
        .library
        .get(toggle())
        .expect("entry")
        .definition()
        .properties()
        .len();
    let before = doc.state_bytes();
    let failed = plan.transaction.apply_with_fault(&mut doc, Some(1));
    assert!(failed.is_err());
    assert_eq!(doc.state_bytes(), before);
    let entry = doc.library.get(toggle()).expect("entry");
    assert_eq!(
        entry.definition().properties().len(),
        properties_before,
        "생성된 Property 0"
    );
    assert!(entry.property(ComponentPropertyId::new(sid(500))).is_none());
    // 정상 적용과 Undo.
    let applied = plan.transaction.apply(&mut doc).expect("적용");
    assert!(doc
        .library
        .get(toggle())
        .expect("entry")
        .property(ComponentPropertyId::new(sid(500)))
        .is_some());
    applied.inverse.apply(&mut doc).expect("Undo");
    assert_eq!(doc.state_bytes(), before);
    // 이벤트 공개.
    let event = plan_expose(
        &doc,
        &ExposeRequest::Event {
            component: toggle(),
            source: sid(KNOB),
            kind: EventKind::PointerDown,
            action: SemanticActionId::new(sid(510)),
            name: "knobPressed".into(),
            transition: TransitionId::new(sid(511)),
        },
    )
    .expect("이벤트 plan");
    apply_and_undo(&mut doc, &event.transaction);
    // 다른 컴포넌트 정의 안의 포트: 직접 edge 거부.
    let nested = Planner::new(&doc, other()).plan(&GraphEdit::Connect {
        from: Endpoint::Expression(literal_ast(520, green())),
        to: Endpoint::PropertyInput(background(ROOT)),
        binding: Some(BindingId::new(sid(521))),
        reference_node: None,
    });
    assert!(matches!(nested, Err(PlanError::NestedInternalPort(_))));
    let nested_expose = plan_expose(
        &doc,
        &ExposeRequest::PropertyInput {
            component: other(),
            target: background(ROOT),
            property: ComponentPropertyId::new(sid(530)),
            name: "tint".into(),
            value_type: ExpressionType::Color,
            default: literal_ast(531, green()),
            binding: BindingId::new(sid(532)),
            reference_node: sid(533),
        },
    );
    assert!(matches!(
        nested_expose,
        Err(PlanError::NestedInternalPort(_))
    ));
    assert!(matches!(
        plan_expose(
            &doc,
            &ExposeRequest::ContextRequirement {
                component: toggle(),
                key: sid(540),
            }
        ),
        Err(PlanError::Missing(_))
    ));
}

#[test]
fn exposing_a_context_port_declares_the_requirement_atomically() {
    // 검증 기준(XXX.2.4 남은 범위): ContextRequirement 생성 plan, 미리보기, 한 transaction, fault injection 시 생성 0.
    let mut doc = context_document();
    let request = ExposeRequest::ContextRequirement {
        component: toggle(),
        key: context_key(),
    };
    let plan = plan_expose(&doc, &request).expect("plan");
    assert_eq!(
        plan.preview,
        vec![ExposedItem::ContextRequirement(context_key())]
    );
    let before = doc.state_bytes();
    assert!(plan
        .transaction
        .apply_with_fault(&mut doc, Some(0))
        .is_err());
    assert_eq!(doc.state_bytes(), before);
    assert!(doc.context.requirements(toggle().id()).is_empty(), "생성 0");
    let applied = plan.transaction.apply(&mut doc).expect("적용");
    assert!(doc
        .context
        .requirements(toggle().id())
        .contains(&context_key()));
    assert!(matches!(
        plan_expose(&doc, &request),
        Err(PlanError::InUse(_))
    ));
    applied.inverse.apply(&mut doc).expect("Undo");
    assert_eq!(doc.state_bytes(), before, "Undo 1회로 동작 전 상태");
    assert!(matches!(
        plan_expose(
            &doc,
            &ExposeRequest::ContextRequirement {
                component: toggle(),
                key: sid(999),
            }
        ),
        Err(PlanError::Missing(_))
    ));
}
