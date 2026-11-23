//! Component Context의 semantic snapshot 투영 (XXX.7.10, ADR-0099 결정 10).
//!
//! - `ContextKey` → [`ObjectKind::ContextKey`] 객체: 이름 `0xEC01`(Text), scope `0xEC02`(Bytes), 값 타입 `0xEC03`(Bytes),
//!   default `0xEC04`(canonical AST Bytes).
//! - `ContextProvision` → [`ObjectKind::ContextProvision`] 객체: 값 `0xEC10`(canonical AST Bytes, 문서 crate의
//!   `PROVISION_VALUE`와 같은 key), owner `0xEC11`(Bytes), provider `0xEC12`(Id), key `0xEC13`(Id).
//! - `ContextRequirement` → Component 객체의 `0xE705`(Ids, 정렬), Slot 공개 목록 → Slot 객체의 `0xE716`(Ids, 정렬). 빈
//!   목록은 속성을 두지 않습니다(Context가 없는 문서의 snapshot은 그대로).
//! - 그 Component·Slot 객체가 snapshot에 없으면(컴포넌트 library를 투영하지 않는 문서 범위) 목록은 소유 문서의 Document
//!   객체에 붙습니다: `0xE705`·`0xE716` 경로에 map key segment(컴포넌트·Slot StableId 문자열)를 단 Ids 속성입니다
//!   (ADR-0123 결정 1). 그래서 문서 범위 snapshot도 "문서 → snapshot → 문서" 왕복이 무손실입니다. 대상마다 속성 하나라
//!   병합 단위는 Component·Slot 객체에 붙을 때와 같습니다(같은 대상 목록을 두 쪽이 다르게 바꾸면 conflict).
//!
//! bytes 형식은 문서 crate의 canonical 형식을 그대로 씁니다(scope·owner·TypeSpec·AST). [`context_from_snapshot`]이 같은
//! 객체에서 `ContextSet`을 다시 만듭니다(merge 결과 반영).

use std::collections::BTreeSet;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PathSegment, PropertyKey};
use stellar_core_types::reserved::context::{
    SNAPSHOT_COMPONENT_CONTEXT_REQUIREMENTS, SNAPSHOT_SLOT_CONTEXT_KEYS,
};
use stellar_document::context::key::{
    type_spec_bytes, type_spec_from_bytes, ContextKey, ContextScope,
};
use stellar_document::context::provision::{ContextOwner, ContextProvision, PROVISION_VALUE};
use stellar_document::context::set::ContextSet;
use stellar_document::expressions::ast::ExpressionAst;

use super::SnapshotBuilder;
use crate::snapshot::{ObjectKind, Origin, SemanticObject, SemanticSnapshot, SnapshotError};
use crate::value::SemanticValue;

/// key 이름.
pub const CONTEXT_KEY_NAME: PropertyKey = PropertyKey(0xEC01);
/// key scope(canonical bytes).
pub const CONTEXT_KEY_SCOPE: PropertyKey = PropertyKey(0xEC02);
/// key 값 타입(TypeSpec bytes).
pub const CONTEXT_KEY_TYPE: PropertyKey = PropertyKey(0xEC03);
/// key default(canonical AST bytes).
pub const CONTEXT_KEY_DEFAULT: PropertyKey = PropertyKey(0xEC04);
/// provision 값(canonical AST bytes). 문서 crate 경로 key와 같습니다.
pub const CONTEXT_PROVISION_VALUE: PropertyKey = PROVISION_VALUE;
/// provision owner(canonical bytes).
pub const CONTEXT_PROVISION_OWNER: PropertyKey = PropertyKey(0xEC11);
/// provision provider node.
pub const CONTEXT_PROVISION_PROVIDER: PropertyKey = PropertyKey(0xEC12);
/// provision key.
pub const CONTEXT_PROVISION_KEY: PropertyKey = PropertyKey(0xEC13);
/// 컴포넌트 requirement 목록(IX.12.1 예약).
pub const COMPONENT_CONTEXT_REQUIREMENTS: PropertyKey =
    PropertyKey(SNAPSHOT_COMPONENT_CONTEXT_REQUIREMENTS);
/// Slot 공개 Context key 목록(IX.12.1 예약).
pub const SLOT_CONTEXT_KEYS: PropertyKey = PropertyKey(SNAPSHOT_SLOT_CONTEXT_KEYS);

fn ast_bytes(ast: &ExpressionAst) -> Vec<u8> {
    ast.to_canonical_bytes().unwrap_or_default()
}

/// key 객체.
pub fn key_object(document: StableId, key: &ContextKey) -> Result<SemanticObject, SnapshotError> {
    let mut object =
        SemanticObject::new(key.id(), ObjectKind::ContextKey, Origin::Document(document));
    let props = [
        (CONTEXT_KEY_NAME, SemanticValue::Text(key.name().to_owned())),
        (
            CONTEXT_KEY_SCOPE,
            SemanticValue::Bytes(key.scope().to_bytes()),
        ),
        (
            CONTEXT_KEY_TYPE,
            SemanticValue::Bytes(type_spec_bytes(key.value_type())),
        ),
        (
            CONTEXT_KEY_DEFAULT,
            SemanticValue::Bytes(key.default_expression().canonical_bytes().to_vec()),
        ),
    ];
    for (k, value) in props {
        object.insert_property(object.path(&[k]), value)?;
    }
    Ok(object)
}

/// provision 객체.
pub fn provision_object(
    document: StableId,
    provision: &ContextProvision,
) -> Result<SemanticObject, SnapshotError> {
    let mut object = SemanticObject::new(
        provision.id(),
        ObjectKind::ContextProvision,
        Origin::Document(document),
    );
    let props = [
        (
            CONTEXT_PROVISION_VALUE,
            SemanticValue::Bytes(ast_bytes(provision.value())),
        ),
        (
            CONTEXT_PROVISION_OWNER,
            SemanticValue::Bytes(provision.owner().to_bytes()),
        ),
        (
            CONTEXT_PROVISION_PROVIDER,
            SemanticValue::Id(provision.provider()),
        ),
        (CONTEXT_PROVISION_KEY, SemanticValue::Id(provision.key())),
    ];
    for (k, value) in props {
        object.insert_property(object.path(&[k]), value)?;
    }
    Ok(object)
}

impl SnapshotBuilder {
    /// 문서 Context를 추가합니다(XXX.7.10). requirement·Slot 공개 목록은 이미 추가된 Component·Slot 객체에 속성으로 붙고,
    /// 그 객체가 없으면 문서 `document`의 Document 객체에 대상 id map key로 붙습니다(모듈 문서). Document 객체도 없으면
    /// [`SnapshotError::ContextOwnerMissing`]입니다. 묶음 전체가 원자적입니다.
    pub fn add_context(
        &mut self,
        document: StableId,
        set: &ContextSet,
    ) -> Result<(), SnapshotError> {
        let mut objects = Vec::new();
        for key in set.keys() {
            objects.push(key_object(document, key)?);
        }
        for provision in set.provisions() {
            objects.push(provision_object(document, provision)?);
        }
        let mut members: Vec<(StableId, PropertyKey, Vec<StableId>)> = Vec::new();
        for (component, keys) in set.all_requirements() {
            members.push((
                component,
                COMPONENT_CONTEXT_REQUIREMENTS,
                keys.iter().copied().collect(),
            ));
        }
        for (slot, keys) in set.all_slot_keys() {
            members.push((slot, SLOT_CONTEXT_KEYS, keys.iter().copied().collect()));
        }
        // 같은 객체에 여러 속성이 붙을 수 있어(Document 객체) 객체별로 모읍니다.
        let mut owners: Vec<SemanticObject> = Vec::new();
        for (owner, key, ids) in members {
            let (target, path) = match self.snapshot.get(owner) {
                Some(object) => (owner, object.path(&[key])),
                None => {
                    let anchor = self
                        .snapshot
                        .get(document)
                        .filter(|o| o.kind() == ObjectKind::Document)
                        .ok_or(SnapshotError::ContextOwnerMissing { owner })?;
                    (
                        document,
                        anchor.path(&[key]).with_key_segment(&owner.to_string()),
                    )
                }
            };
            let index = match owners.iter().position(|o| o.id() == target) {
                Some(index) => index,
                None => {
                    let object = self
                        .snapshot
                        .get(target)
                        .cloned()
                        .expect("위에서 찾은 객체");
                    owners.push(object);
                    owners.len() - 1
                }
            };
            owners[index].insert_property(path, SemanticValue::Ids(ids))?;
        }
        self.commit(objects)?;
        for object in owners {
            self.snapshot.remove(object.id());
            self.snapshot
                .insert(object)
                .expect("같은 id 객체를 바꿔 넣습니다");
        }
        Ok(())
    }
}

fn bytes(object: &SemanticObject, key: PropertyKey) -> Option<&[u8]> {
    match object.property(&object.path(&[key]))? {
        SemanticValue::Bytes(bytes) => Some(bytes),
        _ => None,
    }
}

fn id(object: &SemanticObject, key: PropertyKey) -> Option<StableId> {
    match object.property(&object.path(&[key]))? {
        SemanticValue::Id(id) => Some(*id),
        _ => None,
    }
}

/// snapshot의 Context 객체에서 문서 `document`의 ContextSet을 다시 만듭니다(merge 결과 반영). 형식이 맞지 않는 객체는
/// 그 id와 함께 거부합니다.
pub fn context_from_snapshot(
    snapshot: &SemanticSnapshot,
    document: StableId,
) -> Result<ContextSet, StableId> {
    let mut set = ContextSet::new();
    let mine = |object: &&SemanticObject| object.origin().document() == Some(document);
    for object in snapshot.objects().filter(mine) {
        match object.kind() {
            ObjectKind::ContextKey => {
                let name = match object.property(&object.path(&[CONTEXT_KEY_NAME])) {
                    Some(SemanticValue::Text(name)) => name.clone(),
                    _ => return Err(object.id()),
                };
                let scope = bytes(object, CONTEXT_KEY_SCOPE)
                    .and_then(ContextScope::from_bytes)
                    .ok_or(object.id())?;
                let value_type = bytes(object, CONTEXT_KEY_TYPE)
                    .and_then(type_spec_from_bytes)
                    .ok_or(object.id())?;
                let default = bytes(object, CONTEXT_KEY_DEFAULT)
                    .and_then(|b| ExpressionAst::from_canonical_bytes(b).ok())
                    .ok_or(object.id())?;
                let key = ContextKey::new(object.id(), name, scope, value_type, default)
                    .map_err(|_| object.id())?;
                set.insert_key(key).map_err(|_| object.id())?;
            }
            ObjectKind::ContextProvision => {
                let owner = bytes(object, CONTEXT_PROVISION_OWNER)
                    .and_then(ContextOwner::from_bytes)
                    .ok_or(object.id())?;
                let value = bytes(object, CONTEXT_PROVISION_VALUE)
                    .and_then(|b| ExpressionAst::from_canonical_bytes(b).ok())
                    .ok_or(object.id())?;
                let provision = ContextProvision::new(
                    object.id(),
                    owner,
                    id(object, CONTEXT_PROVISION_PROVIDER).ok_or(object.id())?,
                    id(object, CONTEXT_PROVISION_KEY).ok_or(object.id())?,
                    value,
                )
                .map_err(|_| object.id())?;
                set.insert_provision(provision).map_err(|_| object.id())?;
            }
            _ => {}
        }
    }
    if let Some(anchor) = snapshot
        .get(document)
        .filter(|o| o.kind() == ObjectKind::Document)
    {
        for (path, value) in anchor.properties() {
            let requirement = match path.keys() {
                [key] if *key == COMPONENT_CONTEXT_REQUIREMENTS => true,
                [key] if *key == SLOT_CONTEXT_KEYS => false,
                _ => continue,
            };
            let (Some(PathSegment::Key(owner)), SemanticValue::Ids(ids)) = (path.tail(), value)
            else {
                return Err(anchor.id());
            };
            let owner: StableId = owner.parse().map_err(|_| anchor.id())?;
            if ids.is_empty() {
                return Err(anchor.id());
            }
            for member in ids.iter().copied().collect::<BTreeSet<_>>() {
                if requirement {
                    set.add_requirement(owner, member);
                } else {
                    set.add_slot_key(owner, member);
                }
            }
        }
    }
    for object in snapshot.objects() {
        for (kind, key) in [
            (ObjectKind::Component, COMPONENT_CONTEXT_REQUIREMENTS),
            (ObjectKind::ComponentSlot, SLOT_CONTEXT_KEYS),
        ] {
            if object.kind() != kind || object.origin().document() != Some(document) {
                continue;
            }
            let Some(value) = object.property(&object.path(&[key])) else {
                continue;
            };
            let SemanticValue::Ids(ids) = value else {
                return Err(object.id());
            };
            let unique: BTreeSet<StableId> = ids.iter().copied().collect();
            for member in unique {
                if kind == ObjectKind::Component {
                    set.add_requirement(object.id(), member);
                } else {
                    set.add_slot_key(object.id(), member);
                }
            }
        }
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use stellar_document::component::definition::ComponentLibrary;
    use stellar_document::context::fixtures::{literal, rgb, sid, ComponentBuilder, SceneBuilder};
    use stellar_document::expressions::types::{ExpressionType, TypeSpec};

    use super::*;
    use stellar_core_types::property::PropertyPath;

    fn context() -> ContextSet {
        let mut set = ContextSet::new();
        set.insert_key(
            ContextKey::new(
                sid(900),
                "theme",
                ContextScope::Document(sid(1)),
                TypeSpec::required(ExpressionType::Color),
                literal(901, rgb(1.0, 1.0, 1.0)),
            )
            .unwrap(),
        )
        .unwrap();
        set.add_requirement(sid(50), sid(900));
        set.add_requirement(sid(51), sid(900));
        set.add_slot_key(sid(60), sid(900));
        set
    }

    fn lists(object: &SemanticObject) -> Vec<&PropertyPath> {
        object
            .properties()
            .keys()
            .filter(|p| {
                p.keys() == [COMPONENT_CONTEXT_REQUIREMENTS] || p.keys() == [SLOT_CONTEXT_KEYS]
            })
            .collect()
    }

    #[test]
    fn lists_without_their_component_attach_to_the_document_object_and_round_trip() {
        // 검증 기준(ADR-0123 결정 1): 문서 범위 snapshot(library 없음)도 requirement·Slot 공개 목록을 잃지 않습니다.
        let model = SceneBuilder::new(sid(1), sid(2), sid(3), sid(10)).build();
        let set = context();
        let mut builder = SnapshotBuilder::new();
        builder.add_document(&model.to_records().unwrap()).unwrap();
        builder.add_context(sid(1), &set).unwrap();
        let snapshot = builder.finish();
        let document = snapshot.get(sid(1)).unwrap();
        for (key, owner) in [
            (COMPONENT_CONTEXT_REQUIREMENTS, 50),
            (COMPONENT_CONTEXT_REQUIREMENTS, 51),
            (SLOT_CONTEXT_KEYS, 60),
        ] {
            let path = document
                .path(&[key])
                .with_key_segment(&sid(owner).to_string());
            assert_eq!(
                document.property(&path),
                Some(&SemanticValue::Ids(vec![sid(900)]))
            );
        }
        assert_eq!(context_from_snapshot(&snapshot, sid(1)).unwrap(), set);
        // Document 객체도 없으면 거부합니다.
        let mut empty = SnapshotBuilder::new();
        assert_eq!(
            empty.add_context(sid(1), &set),
            Err(SnapshotError::ContextOwnerMissing { owner: sid(50) })
        );
    }

    #[test]
    fn lists_with_their_component_stay_on_the_component_object() {
        // 프로젝트 범위(library 투영 뒤)는 Component·Slot 객체 속성 그대로이고, library에 없는 대상만 Document 객체입니다.
        let model = SceneBuilder::new(sid(1), sid(2), sid(3), sid(10)).build();
        let mut library = ComponentLibrary::new();
        ComponentBuilder::new(sid(50), "card", sid(70))
            .slot(sid(60), sid(70))
            .register(&mut library);
        let mut builder = SnapshotBuilder::new();
        builder.add_document(&model.to_records().unwrap()).unwrap();
        builder.add_project_component_library(&library).unwrap();
        builder.add_context(sid(1), &context()).unwrap();
        let snapshot = builder.finish();
        for (owner, key) in [
            (50, COMPONENT_CONTEXT_REQUIREMENTS),
            (60, SLOT_CONTEXT_KEYS),
        ] {
            let object = snapshot.get(sid(owner)).unwrap();
            assert_eq!(
                object.property(&object.path(&[key])),
                Some(&SemanticValue::Ids(vec![sid(900)]))
            );
        }
        let document = snapshot.get(sid(1)).unwrap();
        let on_document = lists(document);
        assert_eq!(on_document.len(), 1);
        assert_eq!(
            on_document[0].tail(),
            Some(&PathSegment::Key(sid(51).to_string()))
        );
    }
}
