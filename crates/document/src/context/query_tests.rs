//! 조회 API 시험(XXX.7.11).

use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::FiniteF32;

use super::fixtures::world::*;
use super::fixtures::*;
use super::key::{ContextKey, ContextScope};
use super::query::{binding_candidates, inspect_binding};
use super::resolve::{ContextOrigin, ContextResolver, ContextSite, NoInputs};
use super::set::{ContextKeys, ContextSet};
use super::validate::ContextConsumer;

fn n(v: f32) -> Option<TypedValue> {
    Some(TypedValue::Number(FiniteF32::new(v).unwrap()))
}

#[test]
fn inspecting_a_context_binding_returns_key_value_and_provider_path() {
    // 검증 기준(XXX.7.11): 조회 결과 fixture 정확.
    let (model, library) = sample_world();
    let resolver = ContextResolver::new(&model, &library);
    let keys = ContextKeys::new(model.context());
    let site = ContextSite::virtual_node(vec![sid(CARD_INSTANCE)], sid(CARD_LABEL));
    let consumer = ContextConsumer {
        binding: sid(1),
        site: site.clone(),
        expected: number_type(),
        source: context_read(1, sid(D)),
    };
    let info = inspect_binding(&resolver, &keys, &consumer, &NoInputs).unwrap();
    assert_eq!(info.len(), 1);
    assert_eq!(info[0].name.as_deref(), Some("d"));
    assert_eq!(info[0].value, Some(n(7.0)));
    assert_eq!(
        info[0].origin,
        ContextOrigin::Provision {
            provision: sid(P_BUTTON_D),
            provider: sid(BUTTON_ROOT),
            instances: vec![sid(CARD_INSTANCE), sid(CARD_BUTTON)],
        }
    );
    // 경로: 소비 지점 → (label 공개) Button root provider.
    assert_eq!(
        info[0].path,
        vec![
            site,
            ContextSite::virtual_node(vec![sid(CARD_INSTANCE), sid(CARD_BUTTON)], sid(BUTTON_ROOT)),
        ]
    );
    assert_eq!(info[0].via_slots, vec![sid(LABEL)]);
    // Context를 읽지 않는 binding은 빈 결과.
    let plain = ContextConsumer {
        source: number(2, 1.0),
        ..consumer
    };
    assert!(inspect_binding(&resolver, &keys, &plain, &NoInputs)
        .unwrap()
        .is_empty());
}

#[test]
fn candidates_list_consumable_keys_with_declaration_needs_in_stable_order() {
    // 검증 기준(XXX.7.11): 같은 입력 같은 후보 순서.
    let (model, library) = sample_world();
    let mut library_keys = ContextSet::new();
    library_keys
        .insert_key(
            ContextKey::new(
                sid(990),
                "brand",
                ContextScope::Library(sid(77)),
                color_type(),
                literal(990, rgb(1.0, 0.0, 0.0)),
            )
            .unwrap(),
        )
        .unwrap();
    library_keys
        .insert_key(
            ContextKey::new(
                sid(991),
                "other-doc",
                ContextScope::Document(sid(78)),
                number_type(),
                number(991, 0.0),
            )
            .unwrap(),
        )
        .unwrap();
    let resolver = ContextResolver::new(&model, &library);
    let keys = ContextKeys::new(model.context()).with(&library_keys);
    // Card 안 node: N은 선언됨, D·E·brand는 선언 필요. 다른 문서 scope key는 후보가 아닙니다.
    let inside = ContextSite::virtual_node(vec![sid(CARD_INSTANCE)], sid(CARD_FRAME));
    let candidates = binding_candidates(&resolver, &keys, &inside, None).unwrap();
    let summary: Vec<(&str, bool)> = candidates
        .iter()
        .map(|c| (c.name.as_str(), c.needs_declaration))
        .collect();
    assert_eq!(
        summary,
        vec![("brand", true), ("d", true), ("e", true), ("n", false)]
    );
    assert_eq!(
        candidates[0].origin,
        ContextOrigin::Default,
        "라이브러리 key는 provision이 없어 default"
    );
    // View 최상위: 선언 필요 없음, 타입 필터.
    let top = ContextSite::scene(sid(FRAME));
    let numbers = binding_candidates(&resolver, &keys, &top, Some(number_type())).unwrap();
    assert_eq!(
        numbers.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        vec!["d", "e", "n"]
    );
    assert!(numbers.iter().all(|c| !c.needs_declaration));
    // 같은 입력 같은 순서.
    assert_eq!(
        binding_candidates(&resolver, &keys, &inside, None).unwrap(),
        candidates
    );
}
