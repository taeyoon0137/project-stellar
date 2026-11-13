//! XVI.3.2 검증: 가시 node 2만, 컴포넌트 인스턴스 5만, 중첩 64 fixture가 각각 validator를
//! 통과하고 목표 수를 정확히 만족합니다. 64 초과는 해당 validator가 거부합니다(음성 대조).

use stellar_bench_fixtures::error::FixtureError;
use stellar_bench_fixtures::limits::{
    component_instances, count_visible_in_viewport, deep_document, nesting_chain,
    validate_component_fixture, visible_nodes, COMPONENT_INSTANCES_TARGET, NESTING_TARGET,
    VISIBLE_NODES_TARGET,
};
use stellar_document::component::expansion::{ExpansionError, MAX_INSTANCE_NESTING};
use stellar_document::model::LoadModelError;
use stellar_document::validate::{validate_document, DocumentViolation, MAX_NODE_DEPTH};

#[test]
fn visible_20k_fixture_has_exactly_20k_visible_nodes() {
    let fixture = visible_nodes(1, VISIBLE_NODES_TARGET).expect("fixture 생성");
    let model = &fixture.built.model;
    // 문서 validator 통과, 미해결 참조 없음.
    let diagnostics = validate_document(&model.to_records().unwrap()).expect("validator 통과");
    assert!(diagnostics.unresolved.is_empty());
    // 독립 검산: 유효 가시 + viewport 겹침 node가 정확히 2만.
    assert_eq!(
        count_visible_in_viewport(model, fixture.root, &fixture.viewport).unwrap(),
        VISIBLE_NODES_TARGET
    );
    assert_eq!(fixture.visible_nodes, VISIBLE_NODES_TARGET);
    // 숨김·viewport 밖 subtree가 culling 대상으로 함께 있습니다.
    assert!(fixture.hidden_nodes > 0 && fixture.offscreen_nodes > 0);
    assert_eq!(
        model.node_count(),
        VISIBLE_NODES_TARGET + fixture.hidden_nodes + fixture.offscreen_nodes
    );

    // 재현: 같은 seed 같은 hash, 다른 seed는 내용이 달라도 가시 수는 같습니다.
    assert_eq!(
        visible_nodes(1, VISIBLE_NODES_TARGET).unwrap().content_hash,
        fixture.content_hash
    );
    let other = visible_nodes(2, VISIBLE_NODES_TARGET).unwrap();
    assert_ne!(other.content_hash, fixture.content_hash);
    assert_eq!(
        count_visible_in_viewport(&other.built.model, other.root, &other.viewport).unwrap(),
        VISIBLE_NODES_TARGET
    );

    // 경계: card 단위로 나누어떨어지지 않는 목표도 정확합니다.
    let odd = visible_nodes(3, 1_234).unwrap();
    assert_eq!(
        count_visible_in_viewport(&odd.built.model, odd.root, &odd.viewport).unwrap(),
        1_234
    );
    assert!(matches!(
        visible_nodes(1, 0),
        Err(FixtureError::InvalidSpec { .. })
    ));
    assert!(matches!(
        visible_nodes(1, 30_000),
        Err(FixtureError::InvalidSpec { .. })
    ));
}

#[test]
fn component_50k_fixture_expands_every_instance_without_copies() {
    let fixture = component_instances(1, COMPONENT_INSTANCES_TARGET).expect("fixture 생성");
    assert_eq!(fixture.instances.len(), COMPONENT_INSTANCES_TARGET);
    let validation = validate_component_fixture(&fixture).expect("validator 통과");
    assert_eq!(validation.instances, COMPONENT_INSTANCES_TARGET);
    // 정의 node는 한 벌: 가상 node가 참조한 저장소 주소 수 = 정의 node 수.
    assert_eq!(
        validation.distinct_definition_nodes,
        validation.definition_nodes
    );
    assert!(validation.virtual_nodes > COMPONENT_INSTANCES_TARGET * 3);
    // Card → Button → Icon: scene 인스턴스 포함 chain 3.
    assert_eq!(validation.max_instance_chain, 3);
    assert!(validation.property_overrides > 0);
    // scene node: root + section(500 인스턴스씩) + 인스턴스.
    assert_eq!(
        fixture.columns.len(),
        1 + COMPONENT_INSTANCES_TARGET.div_ceil(500) + COMPONENT_INSTANCES_TARGET
    );
    assert_eq!(
        component_instances(1, COMPONENT_INSTANCES_TARGET)
            .unwrap()
            .content_hash,
        fixture.content_hash
    );
    assert_ne!(
        component_instances(2, COMPONENT_INSTANCES_TARGET)
            .unwrap()
            .content_hash,
        fixture.content_hash
    );
    assert!(matches!(
        component_instances(1, 0),
        Err(FixtureError::InvalidSpec { .. })
    ));
}

#[test]
fn nesting_64_fixtures_sit_exactly_at_the_limit() {
    assert_eq!(NESTING_TARGET, MAX_INSTANCE_NESTING);
    assert_eq!(NESTING_TARGET as u32, MAX_NODE_DEPTH);

    // 컴포넌트 인스턴스 chain 64: expansion 통과, chain 길이 정확히 64.
    let chain = nesting_chain(1, NESTING_TARGET).expect("fixture 생성");
    assert_eq!(chain.library.len(), NESTING_TARGET);
    let validation = validate_component_fixture(&chain).expect("validator 통과");
    assert_eq!(validation.max_instance_chain, NESTING_TARGET);
    assert_eq!(
        nesting_chain(1, NESTING_TARGET).unwrap().content_hash,
        chain.content_hash
    );
    // 음성 대조: 65는 중첩 상한 초과로 거부됩니다.
    let over = nesting_chain(1, NESTING_TARGET + 1).expect("fixture 생성");
    assert_eq!(
        validate_component_fixture(&over),
        Err(FixtureError::Expansion(ExpansionError::NestingTooDeep {
            depth: NESTING_TARGET + 1
        }))
    );

    // 문서 node depth 64: validator 통과, 가장 깊은 node의 조상 수 = 64.
    let deep = deep_document(1, MAX_NODE_DEPTH).expect("fixture 생성");
    let model = &deep.built.model;
    validate_document(&model.to_records().unwrap()).expect("validator 통과");
    let mut depth = 0u32;
    let mut current = model.record(deep.deepest).and_then(|r| r.parent);
    while let Some(parent) = current {
        depth += 1;
        current = model.record(parent).and_then(|r| r.parent);
    }
    assert_eq!(depth, MAX_NODE_DEPTH);
    assert_eq!(
        deep_document(1, MAX_NODE_DEPTH).unwrap().content_hash,
        deep.content_hash
    );
    // 음성 대조: depth 65는 문서 validator가 거부합니다.
    match deep_document(1, MAX_NODE_DEPTH + 1) {
        Err(FixtureError::Load(LoadModelError::Invalid(error))) => {
            assert!(error
                .violations
                .iter()
                .any(|v| matches!(v, DocumentViolation::DepthExceeded { depth: 65, .. })));
        }
        other => panic!("depth 65가 거부되지 않았습니다: {other:?}"),
    }
}
