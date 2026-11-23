//! 10단계 nested component property drilling fixture (IX.9.2).
//!
//! 컴포넌트 C0 ⊃ C1 ⊃ … ⊃ C10(각 정의 tree에 다음 컴포넌트 instance 하나). 가장 안쪽 C10의 공개 property `label`을
//! 제품 drilling 계획(`DrillingPlan::compute`, IX.4.3)·preview(IX.4.4)·한 transaction 적용(`apply_drilling_plan`, IX.4.5)으로
//! C0까지 노출합니다. scene의 C0 instance가 노출된 property를 override합니다.
//!
//! 최종 값의 출처 chain은 제품 drilling 모음 조회(`DrillingSet::link_into`)만으로 따라갑니다: scene override → C0 노출
//! property → 경계마다 link(owner property → nested instance property) → C10 `label`. chain은 경계 10개와 property 11개를
//! 빠짐없이 지나야 합니다. drilled 값을 계산해 provenance 기록을 내는 제품 evaluator는 아직 없습니다(IX.9.2 보고서).

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::ComponentRef;
use stellar_core_types::typed_value::TypedValue;
use stellar_document::component::definition::{ComponentLibrary, ComponentPropertyId};
use stellar_document::component::drilling::DrillingSet;
use stellar_document::component::drilling_apply::apply_drilling_plan;
use stellar_document::component::drilling_plan::{DrillingPlan, DrillingRequest};
use stellar_document::component::instance::{ComponentInstancePayload, OverrideKey};
use stellar_document::component::property::{ComponentProperty, PropertyVisibility};
use stellar_document::component::property_commands::PropertyCommand;
use stellar_document::context::fixtures::{instance_of, literal, ComponentBuilder};
use stellar_document::expressions::types::{ExpressionType, TypeSpec};

const DEPTH: usize = 10;

fn sid(n: u64) -> StableId {
    StableId::new_v7(6_000_000 + n, 0x0092, n)
}

fn component(k: usize) -> ComponentRef {
    ComponentRef::new(sid(100 + k as u64))
}

fn root(k: usize) -> StableId {
    sid(200 + k as u64)
}

/// C_k 정의 tree 안 C_{k+1} instance.
fn nested(k: usize) -> StableId {
    sid(300 + k as u64)
}

fn label() -> ComponentPropertyId {
    ComponentPropertyId::new(sid(400))
}

fn fixture() -> ComponentLibrary {
    let mut library = ComponentLibrary::new();
    for k in (0..=DEPTH).rev() {
        let mut builder = ComponentBuilder::new(component(k).id(), &format!("C{k}"), root(k));
        if k < DEPTH {
            builder = builder.instance(nested(k), root(k), instance_of(component(k + 1).id(), &[]));
        }
        builder.register(&mut library);
    }
    let property = ComponentProperty::new(
        label(),
        "label".into(),
        TypeSpec::required(ExpressionType::String),
        literal(500, TypedValue::String("기본".into())),
        PropertyVisibility::Public,
    )
    .unwrap();
    PropertyCommand::Create {
        component: component(DEPTH),
        property,
        index: None,
    }
    .apply(&mut library, &mut DrillingSet::new())
    .unwrap();
    library
}

#[test]
fn ten_level_drilling_chain_reaches_the_innermost_property() {
    let mut library = fixture();
    let mut drilling = DrillingSet::new();
    let request = DrillingRequest {
        top: component(0),
        path: (0..DEPTH).map(nested).collect(),
        property: label(),
    };
    let plan = DrillingPlan::compute(&library, &drilling, request.clone()).expect("계획");
    // 같은 입력은 같은 계획(IX.4.4).
    assert_eq!(
        DrillingPlan::compute(&library, &drilling, request).unwrap(),
        plan
    );
    assert_eq!(plan.components().len(), DEPTH + 1);
    assert_eq!(plan.new_properties().len(), DEPTH);
    assert_eq!(plan.links().len(), DEPTH);
    let preview = plan.preview(&library);
    let deepest = preview.properties.last().unwrap();
    assert_eq!(deepest.component_path.len(), DEPTH, "top → C9 경로");
    assert_eq!(deepest.instance_path.len(), DEPTH - 1);
    assert!(preview
        .properties
        .iter()
        .all(|p| p.name == "label" && p.default == Some(TypedValue::String("기본".into()))));

    let mut next = 10_000u64;
    let applied = apply_drilling_plan(&mut library, &mut drilling, &plan, &mut || {
        next += 1;
        sid(next)
    })
    .expect("한 transaction 적용");
    assert_eq!(applied.created_properties.len(), DEPTH);
    assert_eq!(applied.created_links.len(), DEPTH);

    // scene: C0 instance가 노출 property를 override합니다.
    let mut scene = ComponentInstancePayload::new(component(0));
    let value = TypedValue::String("저장".into());
    scene.set_property_override(OverrideKey::direct(applied.exposed), value.clone());

    // 출처 chain: scene override(C0 노출 property)에서 경계마다 link를 따라 C10 label까지.
    let top_entry = library.get(component(0)).unwrap();
    assert!(top_entry.property(applied.exposed).unwrap().is_public());
    assert_eq!(
        scene.property_override(&OverrideKey::direct(applied.exposed)),
        Some(&value)
    );
    let mut chain = vec![(component(0), applied.exposed)];
    let mut source = applied.exposed;
    for k in 0..DEPTH {
        // C_k가 소유한 link 가운데 source가 지금 property이고 nested instance로 들어가는 것.
        let target = library
            .get(component(k + 1))
            .unwrap()
            .property_records()
            .map(|p| p.id())
            .find(|p| {
                drilling
                    .link_into(component(k), nested(k), *p)
                    .is_some_and(|link| link.source() == source)
            })
            .unwrap_or_else(|| panic!("경계 {k} link"));
        let link = drilling.link_into(component(k), nested(k), target).unwrap();
        assert_eq!(link.owner(), component(k));
        assert_eq!(link.path(), [nested(k)]);
        assert_eq!(link.chain(), [target]);
        assert!(!drilling.is_orphan(link.id()));
        chain.push((component(k + 1), target));
        source = target;
    }
    assert_eq!(chain.len(), DEPTH + 1, "property 11개");
    assert_eq!(chain.last(), Some(&(component(DEPTH), label())));
    // chain의 컴포넌트 순서 = 계획 컴포넌트 경로.
    assert_eq!(
        chain.iter().map(|(c, _)| *c).collect::<Vec<_>>(),
        plan.components()
    );
    // 모든 binding이 검증을 통과합니다(IX.4.1 경계 chain 필수).
    drilling.validate(&library).expect("drilling 모음 검증");
}
