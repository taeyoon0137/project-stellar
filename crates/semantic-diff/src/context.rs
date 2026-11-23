//! Component Context의 semantic snapshot 투영 (XXX.7.10, ADR-0099 결정 10).
//!
//! - `ContextKey` → [`ObjectKind::ContextKey`] 객체: 이름 `0xEC01`(Text), scope `0xEC02`(Bytes), 값 타입 `0xEC03`(Bytes),
//!   default `0xEC04`(canonical AST Bytes).
//! - `ContextProvision` → [`ObjectKind::ContextProvision`] 객체: 값 `0xEC10`(canonical AST Bytes, 문서 crate의
//!   `PROVISION_VALUE`와 같은 key), owner `0xEC11`(Bytes), provider `0xEC12`(Id), key `0xEC13`(Id).
//! - `ContextRequirement` → Component 객체의 `0xE705`(Ids, 정렬), Slot 공개 목록 → Slot 객체의 `0xE716`(Ids, 정렬). 빈
//!   목록은 속성을 두지 않습니다(Context가 없는 문서의 snapshot은 그대로).
//!
//! bytes 형식은 문서 crate의 canonical 형식을 그대로 씁니다(scope·owner·TypeSpec·AST). [`context_from_snapshot`]이 같은
//! 객체에서 `ContextSet`을 다시 만듭니다(merge 결과 반영).

use std::collections::BTreeSet;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyKey;
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
    /// 문서 Context를 추가합니다(XXX.7.10). requirement·Slot 공개 목록은 이미 추가된 Component·Slot 객체에 속성으로 붙습니다.
    /// 묶음 전체가 원자적입니다.
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
        let mut owners = Vec::new();
        for (owner, key, ids) in members {
            let mut object = self
                .snapshot
                .get(owner)
                .cloned()
                .ok_or(SnapshotError::ContextOwnerMissing { owner })?;
            object.insert_property(object.path(&[key]), SemanticValue::Ids(ids))?;
            owners.push(object);
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

fn bytes<'a>(object: &'a SemanticObject, key: PropertyKey) -> Option<&'a [u8]> {
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
