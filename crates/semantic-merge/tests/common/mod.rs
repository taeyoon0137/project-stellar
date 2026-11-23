//! semantic-merge 통합 테스트 공용 fixture.
//!
//! 공개 API(`SemanticObject`·`SemanticSnapshot`)로만 snapshot을 만듭니다. 문서 1 scene, 컴포넌트 정의,
//! scene 인스턴스, 정의 tree 안 nested 인스턴스, token을 한 snapshot에 담는 `world`가 기본 base입니다.

#![allow(dead_code)]

use stellar_semantic_diff::core_types::ids::StableId;
use stellar_semantic_diff::core_types::order_key::OrderKey;
use stellar_semantic_diff::core_types::property::{PropertyKey, PropertyPath};
use stellar_semantic_diff::keys;
use stellar_semantic_diff::snapshot::{
    NodeKind, ObjectKind, Origin, SemanticObject, SemanticSnapshot,
};
use stellar_semantic_diff::value::SemanticValue;
use stellar_semantic_merge::merge::merge;
use stellar_semantic_merge::model::{MergeInput, MergeResult};

/// 결정론적 StableId.
pub fn sid(n: u64) -> StableId {
    StableId::new_v7(1_700_000_000_000 + n, 0x0abc, 0x0123_4567_89ab_cdef ^ n)
}

pub fn key(byte: u8) -> OrderKey {
    OrderKey::from_bytes(vec![byte]).unwrap()
}

pub fn text(value: &str) -> SemanticValue {
    SemanticValue::Text(value.into())
}

pub const DOCUMENT: u64 = 1;
pub const ROOT: u64 = 10;
pub const CHILD_A: u64 = 11;
pub const CHILD_B: u64 = 12;
/// 바깥 컴포넌트(Property `OUTER_PROP`, 정의 tree에 nested 인스턴스 `NESTED`).
pub const OUTER: u64 = 20;
pub const OUTER_ROOT: u64 = 21;
pub const NESTED: u64 = 22;
pub const OUTER_PROP: u64 = 23;
/// 안쪽 컴포넌트(Property `INNER_PROP`·`INNER_PROP_2`).
pub const INNER: u64 = 30;
pub const INNER_ROOT: u64 = 31;
pub const INNER_PROP: u64 = 32;
pub const INNER_PROP_2: u64 = 33;
/// scene 인스턴스(INNER 참조).
pub const INSTANCE: u64 = 40;
/// token.
pub const TOKEN: u64 = 50;

pub fn set(object: &mut SemanticObject, key: PropertyKey, value: SemanticValue) {
    object.set_property(object.path(&[key]), value).unwrap();
}

/// 문서 1 scene node.
pub fn node(id: u64, parent: Option<u64>, order: u8, name: &str) -> SemanticObject {
    let kind = if parent.is_none() {
        NodeKind::Frame
    } else {
        NodeKind::Fragment
    };
    let mut object = SemanticObject::new(
        sid(id),
        ObjectKind::Node(kind),
        Origin::Document(sid(DOCUMENT)),
    )
    .with_position(parent.map(sid), Some(key(order)));
    set(&mut object, keys::DISPLAY_NAME, text(name));
    object
}

/// 프로젝트 컴포넌트 정의 객체.
pub fn component(id: u64, root: u64, properties: &[u64]) -> SemanticObject {
    let mut object = SemanticObject::new(
        sid(id),
        ObjectKind::Component,
        Origin::ProjectComponent(sid(id)),
    );
    set(&mut object, keys::DISPLAY_NAME, text("component"));
    set(
        &mut object,
        keys::COMPONENT_ROOT,
        SemanticValue::Id(sid(root)),
    );
    set(
        &mut object,
        keys::COMPONENT_PROPERTIES,
        SemanticValue::Ids(properties.iter().map(|p| sid(*p)).collect()),
    );
    set(
        &mut object,
        keys::COMPONENT_SLOTS,
        SemanticValue::Ids(Vec::new()),
    );
    set(
        &mut object,
        keys::COMPONENT_LOCAL_TOKENS,
        SemanticValue::Ids(Vec::new()),
    );
    object
}

/// 컴포넌트 정의 tree node(root Frame 또는 nested 인스턴스).
pub fn definition_node(id: u64, owner: u64, parent: Option<u64>, kind: NodeKind) -> SemanticObject {
    let mut object = SemanticObject::new(
        sid(id),
        ObjectKind::Node(kind),
        Origin::ProjectComponent(sid(owner)),
    )
    .with_position(parent.map(sid), Some(key(0x80)));
    set(&mut object, keys::DISPLAY_NAME, text("definition"));
    object
}

/// 인스턴스 node에 정의 참조를 둡니다.
pub fn refer(object: &mut SemanticObject, component: u64) {
    set(
        object,
        keys::instance::COMPONENT_REF,
        SemanticValue::Id(sid(component)),
    );
}

/// override 경로(직접 override: segment = Property id).
pub fn override_path(instance: u64, property: u64) -> PropertyPath {
    PropertyPath::with_keys(sid(instance), vec![keys::instance::PROPERTY_OVERRIDES])
        .with_key_segment(&sid(property).to_string())
}

pub fn token(id: u64, value_type: u64, value: SemanticValue) -> SemanticObject {
    let mut object = SemanticObject::new(sid(id), ObjectKind::Token, Origin::Project);
    set(&mut object, keys::TOKEN_NAME, text("token"));
    set(
        &mut object,
        keys::TOKEN_TYPE,
        SemanticValue::Uint(value_type),
    );
    set(&mut object, keys::TOKEN_VALUE, value);
    object
}

pub fn snapshot(objects: Vec<SemanticObject>) -> SemanticSnapshot {
    let mut snapshot = SemanticSnapshot::new();
    for object in objects {
        snapshot.insert(object).unwrap();
    }
    snapshot
}

/// 기본 base: scene root·자식 둘, 바깥·안쪽 컴포넌트, scene 인스턴스, token.
pub fn world() -> SemanticSnapshot {
    let mut nested = definition_node(NESTED, OUTER, Some(OUTER_ROOT), NodeKind::ComponentInstance);
    refer(&mut nested, INNER);
    let mut instance = SemanticObject::new(
        sid(INSTANCE),
        ObjectKind::Node(NodeKind::ComponentInstance),
        Origin::Document(sid(DOCUMENT)),
    )
    .with_position(Some(sid(ROOT)), Some(key(0xE0)));
    set(&mut instance, keys::DISPLAY_NAME, text("instance"));
    refer(&mut instance, INNER);
    snapshot(vec![
        node(ROOT, None, 0x80, "root"),
        node(CHILD_A, Some(ROOT), 0x40, "a"),
        node(CHILD_B, Some(ROOT), 0xC0, "b"),
        component(OUTER, OUTER_ROOT, &[OUTER_PROP]),
        definition_node(OUTER_ROOT, OUTER, None, NodeKind::Frame),
        nested,
        component(INNER, INNER_ROOT, &[INNER_PROP, INNER_PROP_2]),
        definition_node(INNER_ROOT, INNER, None, NodeKind::Frame),
        instance,
        token(TOKEN, 1, SemanticValue::Uint(5)),
    ])
}

/// snapshot에서 객체 하나를 바꿉니다.
pub fn edit(
    snapshot: &SemanticSnapshot,
    id: u64,
    f: impl FnOnce(&mut SemanticObject),
) -> SemanticSnapshot {
    let mut out = snapshot.clone();
    let mut object = out.remove(sid(id)).unwrap();
    f(&mut object);
    out.insert(object).unwrap();
    out
}

/// 경로 하나에 값을 둡니다.
pub fn put(
    snapshot: &SemanticSnapshot,
    path: &PropertyPath,
    value: SemanticValue,
) -> SemanticSnapshot {
    let mut out = snapshot.clone();
    let mut object = out.remove(path.root()).unwrap();
    object.set_property(path.clone(), value).unwrap();
    out.insert(object).unwrap();
    out
}

/// 컴포넌트의 Property 목록을 바꿉니다.
pub fn set_properties(
    snapshot: &SemanticSnapshot,
    component: u64,
    properties: &[u64],
) -> SemanticSnapshot {
    edit(snapshot, component, |o| {
        set(
            o,
            keys::COMPONENT_PROPERTIES,
            SemanticValue::Ids(properties.iter().map(|p| sid(*p)).collect()),
        )
    })
}

pub fn run(
    base: &SemanticSnapshot,
    ours: &SemanticSnapshot,
    theirs: &SemanticSnapshot,
) -> MergeResult {
    merge(&MergeInput {
        base: base.clone(),
        ours: ours.clone(),
        theirs: theirs.clone(),
    })
    .unwrap()
}
