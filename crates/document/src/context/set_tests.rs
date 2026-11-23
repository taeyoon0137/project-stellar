//! `ContextSet`·provision 시험(XXX.7.2).

use stellar_core_types::ids::StableId;

use super::fixtures::*;
use super::key::{ContextKey, ContextScope};
use super::provision::{check_provision_scope, ContextOwner, ContextProvision, ProvisionScopeError};
use super::set::{ContextSet, ContextSetError};
use crate::component::definition::ComponentLibrary;
use crate::model::DocumentModel;

const DOC: u64 = 1;
const PAGE: u64 = 2;
const VIEW: u64 = 3;
const ROOT: u64 = 10;
const CARD: u64 = 100;
const CARD_ROOT: u64 = 101;
const CARD_HOST: u64 = 102;
const CARD_BUTTON: u64 = 103;
const BODY_SLOT: u64 = 105;
const BUTTON: u64 = 200;
const BUTTON_ROOT: u64 = 201;
const THEME: u64 = 900;

/// scene: root ← card instance(11, slot body ← 12), frame 13. Card: root ← host(slot body), nested Button instance.
fn world() -> (DocumentModel, ComponentLibrary) {
    let mut library = ComponentLibrary::new();
    ComponentBuilder::new(sid(BUTTON), "Button", sid(BUTTON_ROOT)).register(&mut library);
    ComponentBuilder::new(sid(CARD), "Card", sid(CARD_ROOT))
        .frame(sid(CARD_HOST), sid(CARD_ROOT))
        .instance(sid(CARD_BUTTON), sid(CARD_ROOT), instance_of(sid(BUTTON), &[]))
        .slot(sid(BODY_SLOT), sid(CARD_HOST))
        .register(&mut library);
    let model = SceneBuilder::new(sid(DOC), sid(PAGE), sid(VIEW), sid(ROOT))
        .instance(
            sid(11),
            sid(ROOT),
            instance_of(sid(CARD), &[(sid(BODY_SLOT), vec![sid(12)])]),
        )
        .frame(sid(12), sid(11))
        .frame(sid(13), sid(ROOT))
        .build();
    (model, library)
}

fn theme() -> ContextKey {
    ContextKey::new(
        sid(THEME),
        "theme",
        ContextScope::Document(sid(DOC)),
        color_type(),
        literal(1, rgb(1.0, 1.0, 1.0)),
    )
    .unwrap()
}

fn provision(id: u64, owner: ContextOwner, provider: u64) -> ContextProvision {
    ContextProvision::new(
        sid(id),
        owner,
        sid(provider),
        sid(THEME),
        literal(id, rgb(0.0, 0.0, 1.0)),
    )
    .unwrap()
}

fn sample_set() -> ContextSet {
    let mut set = ContextSet::new();
    set.insert_key(theme()).unwrap();
    set.insert_provision(provision(1000, ContextOwner::View(sid(VIEW)), ROOT))
        .unwrap();
    set.insert_provision(provision(1001, ContextOwner::Component(sid(CARD)), CARD_ROOT))
        .unwrap();
    set.add_requirement(sid(CARD), sid(THEME));
    set.add_slot_key(sid(BODY_SLOT), sid(THEME));
    set
}

#[test]
fn set_round_trips_with_requirements_and_slot_keys() {
    // 검증 기준(XXX.7.2): round-trip, Slot 공개 목록 round-trip.
    let set = sample_set();
    let bytes = set.encode();
    let decoded = ContextSet::decode(&bytes).unwrap();
    assert_eq!(decoded, set);
    assert_eq!(decoded.encode(), bytes);
    assert!(decoded.slot_keys(sid(BODY_SLOT)).contains(&sid(THEME)));
    assert!(decoded.requirements(sid(CARD)).contains(&sid(THEME)));
    assert_eq!(
        decoded.provision_at(sid(ROOT), sid(THEME)).map(|p| p.id()),
        Some(sid(1000))
    );
    // 빈 set도 왕복합니다.
    assert_eq!(ContextSet::decode(&ContextSet::new().encode()).unwrap(), ContextSet::new());
    // 꼬리 bytes·version 오류는 거부.
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(ContextSet::decode(&trailing).is_err());
    let mut version = bytes;
    version[0] = 9;
    assert!(ContextSet::decode(&version).is_err());
}

#[test]
fn set_rejects_duplicates() {
    let mut set = sample_set();
    assert_eq!(
        set.insert_provision(provision(1002, ContextOwner::View(sid(VIEW)), ROOT)),
        Err(ContextSetError::DuplicateProvision {
            provider: sid(ROOT),
            key: sid(THEME)
        })
    );
    let clash = ContextKey::new(
        sid(901),
        "theme",
        ContextScope::Document(sid(DOC)),
        color_type(),
        literal(2, rgb(0.0, 0.0, 0.0)),
    )
    .unwrap();
    assert!(matches!(
        set.insert_key(clash),
        Err(ContextSetError::DuplicateName { .. })
    ));
    // 다른 scope는 같은 이름 가능.
    let project = ContextKey::new(
        sid(902),
        "theme",
        ContextScope::Project,
        color_type(),
        literal(3, rgb(0.0, 0.0, 0.0)),
    )
    .unwrap();
    set.insert_key(project).unwrap();
    assert_eq!(
        set.insert_provision(provision(THEME, ContextOwner::View(sid(VIEW)), 13)),
        Err(ContextSetError::DuplicateId { id: sid(THEME) })
    );
}

#[test]
fn provision_scope_rejects_nodes_outside_owner_and_allows_own_instance_nodes() {
    // 검증 기준(XXX.7.2): 소유 범위 밖 node를 가리키는 provision 거부, instance node provision 허용.
    let (model, library) = world();
    let ok = |p: &ContextProvision| check_provision_scope(p, &model, &library);
    // View 안 scene node, scene instance node.
    assert_eq!(ok(&provision(1, ContextOwner::View(sid(VIEW)), ROOT)), Ok(()));
    assert_eq!(ok(&provision(2, ContextOwner::View(sid(VIEW)), 11)), Ok(()));
    // 정의 tree 안 node, 정의 tree 안 nested instance node.
    assert_eq!(
        ok(&provision(3, ContextOwner::Component(sid(CARD)), CARD_HOST)),
        Ok(())
    );
    assert_eq!(
        ok(&provision(4, ContextOwner::Component(sid(CARD)), CARD_BUTTON)),
        Ok(())
    );
    // 다른 컴포넌트 내부 node(nested Button의 root)는 Card 소유 provision이 가리킬 수 없습니다.
    assert_eq!(
        ok(&provision(5, ContextOwner::Component(sid(CARD)), BUTTON_ROOT)),
        Err(ProvisionScopeError::OutsideOwner {
            provider: sid(BUTTON_ROOT),
            owner: ContextOwner::Component(sid(CARD)),
        })
    );
    // View owner가 정의 node를, 정의 owner가 scene node를 가리킬 수 없습니다.
    assert!(matches!(
        ok(&provision(6, ContextOwner::View(sid(VIEW)), CARD_ROOT)),
        Err(ProvisionScopeError::OutsideOwner { .. })
    ));
    assert!(matches!(
        ok(&provision(7, ContextOwner::Component(sid(CARD)), ROOT)),
        Err(ProvisionScopeError::OutsideOwner { .. })
    ));
    assert_eq!(
        ok(&provision(8, ContextOwner::View(sid(77)), ROOT)),
        Err(ProvisionScopeError::UnknownView { view: sid(77) })
    );
    assert_eq!(
        ok(&provision(9, ContextOwner::Component(sid(78)), ROOT)),
        Err(ProvisionScopeError::UnknownComponent { component: sid(78) })
    );
}

#[test]
fn live_set_hides_provisions_of_deleted_scene_nodes() {
    let (mut model, _) = world();
    let mut set = sample_set();
    set.insert_provision(provision(1003, ContextOwner::View(sid(VIEW)), 13))
        .unwrap();
    model.replace_context(set.clone());
    assert_eq!(model.context().live(&model), set);
    model.remove_leaf(sid(13)).unwrap();
    let live = model.context().live(&model);
    assert!(live.provision(sid(1003)).is_none());
    // 모델의 set에는 남아 있어 삭제 undo 뒤 다시 보입니다.
    assert!(model.context().provision(sid(1003)).is_some());
    // 정의 node provision은 모델 node와 무관하게 남습니다.
    assert!(live.provision(sid(1001)).is_some());
    assert_ne!(sid(1003), StableId::NIL);
}
