//! 해석기 시험(XXX.7.4).

use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::FiniteF32;

use super::fixtures::world::*;
use super::fixtures::*;
use super::resolve::{
    ContextEvaluator, ContextOrigin, ContextResolver, ContextSite, NoInputs,
};
use super::set::ContextKeys;
use crate::component::expansion::{ExpandedNodeId, InstanceExpansion, SceneIndex, ScenePayloads};
use crate::component::definition::ComponentLibrary;
use crate::model::DocumentModel;
use crate::record_codec::encode_payload;

fn n(v: f32) -> Option<TypedValue> {
    Some(TypedValue::Number(FiniteF32::new(v).unwrap()))
}

fn scene(id: u64) -> ContextSite {
    ContextSite::scene(sid(id))
}

fn inside(chain: &[u64], node: u64) -> ContextSite {
    ContextSite::virtual_node(chain.iter().map(|c| sid(*c)).collect(), sid(node))
}

fn value(model: &DocumentModel, library: &ComponentLibrary, key: u64, site: &ContextSite) -> Option<TypedValue> {
    let resolver = ContextResolver::new(model, library);
    let keys = ContextKeys::new(model.context());
    let evaluator = ContextEvaluator::new(&resolver, &keys, &NoInputs);
    evaluator.value(sid(key), site).unwrap()
}

fn origin(provision: u64, chain: &[u64], provider: u64) -> ContextOrigin {
    ContextOrigin::Provision {
        provision: sid(provision),
        provider: sid(provider),
        instances: chain.iter().map(|c| sid(*c)).collect(),
    }
}

#[test]
fn nearest_ancestor_provision_shadowing_default_and_same_key_reread() {
    // 검증 기준(XXX.7.4): 중첩·shadowing·provision 없음·같은 key 재읽기 값 정확.
    let (model, library) = sample_world();
    let resolver = ContextResolver::new(&model, &library);
    // scene Frame: View root provision.
    let r = resolver.resolve(sid(N), &scene(FRAME)).unwrap();
    assert_eq!(r.origin, origin(P_ROOT_N, &[], ROOT));
    assert_eq!(value(&model, &library, N, &scene(FRAME)), n(10.0));
    // provision 없음: default.
    let r = resolver.resolve(sid(D), &scene(FRAME)).unwrap();
    assert!(r.is_default());
    assert_eq!(value(&model, &library, D, &scene(FRAME)), n(0.0));
    // 중첩: Card 인스턴스 안 Button 안 Frame은 Card root provision(N = 바깥 N + 5)이 가장 가깝습니다(View root 가림).
    let deep = inside(&[CARD_INSTANCE, CARD_BUTTON], BUTTON_FRAME);
    let r = resolver.resolve(sid(N), &deep).unwrap();
    assert_eq!(r.origin, origin(P_CARD_N, &[CARD_INSTANCE], CARD_ROOT));
    assert_eq!(r.shadowed, vec![sid(P_ROOT_N)]);
    assert_eq!(
        r.path,
        vec![
            deep.clone(),
            inside(&[CARD_INSTANCE, CARD_BUTTON], BUTTON_ROOT),
            inside(&[CARD_INSTANCE], CARD_BUTTON),
            inside(&[CARD_INSTANCE], CARD_ROOT),
        ]
    );
    // 같은 key 재읽기는 바깥 조상 값: 10 + 5.
    assert_eq!(value(&model, &library, N, &deep), n(15.0));
    // provider 자신도 자기 provision을 봅니다(하위 트리에 자기 포함).
    assert_eq!(
        value(&model, &library, N, &inside(&[CARD_INSTANCE], CARD_ROOT)),
        n(15.0)
    );
}

#[test]
fn slot_content_resolves_in_authoring_scope_and_exposed_keys_see_internal_provisions() {
    // 검증 기준(XXX.7.4): slot fixture(공개하지 않은 key는 instance 바깥 값, 공개한 key는 내부 provision 값, nested slot).
    let (model, library) = sample_world();
    let resolver = ContextResolver::new(&model, &library);
    for content in [CONTENT, CONTENT_CHILD] {
        // N은 body가 공개하지 않습니다: Card 내부(N = 15)를 건너뛰고 instance 바깥 View root(10).
        let r = resolver.resolve(sid(N), &scene(content)).unwrap();
        assert_eq!(r.origin, origin(P_ROOT_N, &[], ROOT));
        assert_eq!(r.skipped_slots, vec![sid(BODY)]);
        assert_eq!(value(&model, &library, N, &scene(content)), n(10.0));
        // D는 공개: slot host부터 내부 provision(2).
        let r = resolver.resolve(sid(D), &scene(content)).unwrap();
        assert_eq!(r.origin, origin(P_HOST_D, &[CARD_INSTANCE], CARD_HOST));
        assert_eq!(r.via_slots, vec![sid(BODY)]);
        assert_eq!(value(&model, &library, D, &scene(content)), n(2.0));
        // E는 공개했지만 내부 provision이 없어 instance node로 이어 올라가 View root(3).
        let r = resolver.resolve(sid(E), &scene(content)).unwrap();
        assert_eq!(r.origin, origin(P_ROOT_E, &[], ROOT));
        assert_eq!(r.via_slots, vec![sid(BODY)]);
        assert_eq!(value(&model, &library, E, &scene(content)), n(3.0));
    }
    // nested slot: Card가 Button label slot에 작성한 콘텐츠.
    let label = inside(&[CARD_INSTANCE], CARD_LABEL);
    // D는 label이 공개: Button 내부 root provision(7).
    let r = resolver.resolve(sid(D), &label).unwrap();
    assert_eq!(r.origin, origin(P_BUTTON_D, &[CARD_INSTANCE, CARD_BUTTON], BUTTON_ROOT));
    assert_eq!(value(&model, &library, D, &label), n(7.0));
    // N은 공개하지 않음: Button 내부를 건너뛰고 Card root(15).
    let r = resolver.resolve(sid(N), &label).unwrap();
    assert_eq!(r.origin, origin(P_CARD_N, &[CARD_INSTANCE], CARD_ROOT));
    assert_eq!(r.skipped_slots, vec![sid(LABEL)]);
    assert_eq!(value(&model, &library, N, &label), n(15.0));
}

#[test]
fn resolution_is_cached_and_deterministic_and_matches_expansion_ids() {
    // 검증 기준(XXX.7.4): 같은 입력 같은 결과, 조상 탐색 runtime index.
    let (model, library) = sample_world();
    let resolver = ContextResolver::new(&model, &library);
    let deep = inside(&[CARD_INSTANCE, CARD_BUTTON], BUTTON_FRAME);
    let first = resolver.resolve(sid(N), &deep).unwrap();
    let cached = resolver.cached();
    assert!(cached >= 1);
    assert_eq!(resolver.resolve(sid(N), &deep).unwrap(), first);
    assert_eq!(resolver.cached(), cached, "두 번째 해석은 캐시");
    let fresh = ContextResolver::new(&model, &library);
    assert_eq!(fresh.resolve(sid(N), &deep).unwrap(), first);

    // instance expansion view의 node id와 같은 site 식별입니다.
    let mut payloads = ScenePayloads::new();
    for id in model.columns().iter_live() {
        let stable = model.stable_id(id).unwrap();
        payloads.insert(stable, encode_payload(model.payload(stable).unwrap()));
    }
    let index = SceneIndex::build(model.columns(), &payloads);
    let expansion = InstanceExpansion::new(&library, &index, sid(CARD_INSTANCE)).unwrap();
    let mut visited = 0;
    for id in expansion.walk().unwrap() {
        let site = ContextSite::from(&id);
        let expected = match &id {
            ExpandedNodeId::Owned(_) => n(10.0),
            ExpandedNodeId::Virtual { .. } => n(15.0),
        };
        assert_eq!(value(&model, &library, N, &site), expected, "{id:?}");
        visited += 1;
    }
    assert!(visited >= 7);
    assert_ne!(sid(N), StableId::NIL);
}
