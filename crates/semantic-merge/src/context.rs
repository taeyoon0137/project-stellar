//! Component Context 전용 merge conflict (XXX.7.10, 아키텍처 27.1, ADR-0099 결정 10).
//!
//! - **provision 삭제 대 소비 추가**([`ConflictKind::ContextProvisionRemovedVsConsume`]): 한쪽이 provision P(key K, provider
//!   X)를 지우고 다른 쪽이 X 하위 트리에서 K를 새로 읽으면(값이 K의 StableId를 새로 담는 객체·속성 추가) 그 소비는 P를
//!   전제로 작성됐을 수 있으므로 삭제를 받아들이지 않고 conflict로 남깁니다. 소비 객체가 scene tree에 놓이지 않아(binding
//!   소유 expression node 등) 위치를 알 수 없으면 같은 문서 안이면 보수적으로 conflict입니다.
//! - **key 타입 변경 대 provision 값 변경**([`ConflictKind::ContextKeyTypeVsProvisionValue`]): 한쪽이 key K의 값 타입을
//!   바꾸고 다른 쪽이 K provision의 값을 바꾸거나 K provision을 더하면, 따로 받아들이면 타입과 값이 어긋나므로 두 변경을
//!   모두 되돌리고 key·provision 각각에 conflict를 남깁니다. key는 타입과 default가 함께 바뀌므로 key 객체의 변경 전체를
//!   되돌리고 대상은 key 객체입니다.

use std::collections::BTreeSet;

use stellar_semantic_diff::build::context::{
    CONTEXT_KEY_TYPE, CONTEXT_PROVISION_KEY, CONTEXT_PROVISION_PROVIDER, CONTEXT_PROVISION_VALUE,
};

use super::*;

fn id_property(object: &SemanticObject, key: PropertyKey) -> Option<StableId> {
    match object.property(&object.path(&[key]))? {
        SemanticValue::Id(id) => Some(*id),
        _ => None,
    }
}

/// `object`(또는 그 parent chain)가 `provider` 하위 트리에 있는지. tree에 놓이지 않은 객체는 `None`.
fn under(snapshot: &SemanticSnapshot, object: StableId, provider: StableId) -> Option<bool> {
    let mut current = Some(object);
    let mut placed = false;
    let mut steps = 0usize;
    while let Some(id) = current {
        if id == provider {
            return Some(true);
        }
        let Some(found) = snapshot.get(id) else {
            break;
        };
        placed |= matches!(found.kind(), ObjectKind::Node(_));
        current = found.parent();
        steps += 1;
        if steps > snapshot.len() {
            break;
        }
    }
    placed.then_some(false)
}

/// `changes` 가운데 `key`를 새로 읽는 변경의 대상 객체(Context 객체 자신 제외).
fn new_reads(changes: &SemanticDiff, base: &SemanticSnapshot, key: StableId) -> Vec<StableId> {
    let ids = BTreeSet::from([key]);
    let is_context =
        |kind: ObjectKind| matches!(kind, ObjectKind::ContextKey | ObjectKind::ContextProvision);
    let mut out = Vec::new();
    for change in changes.iter() {
        match change {
            SemanticChange::AddObject(c) if !is_context(c.object.kind()) => {
                if c.object.properties().values().any(|v| mentions(v, &ids)) {
                    out.push(c.object.id());
                }
            }
            _ => {
                let (Some(path), Some((old, Some(new)))) =
                    (change.property_path(), property_values(change))
                else {
                    continue;
                };
                if base.get(path.root()).is_some_and(|o| is_context(o.kind())) {
                    continue;
                }
                if mentions(new, &ids) && !old.is_some_and(|old| mentions(old, &ids)) {
                    out.push(path.root());
                }
            }
        }
    }
    out
}

impl Merger<'_> {
    /// Context 전용 conflict를 찾아 관련 변경을 되돌립니다.
    pub(super) fn context_conflicts(&mut self, ours: &SemanticDiff, theirs: &SemanticDiff) {
        let input = self.input;
        // 1. provision 삭제 대 소비 추가.
        let mut removed: BTreeSet<StableId> = BTreeSet::new();
        for (deleter, user_side, user_diff) in
            [(ours, &input.theirs, theirs), (theirs, &input.ours, ours)]
        {
            for change in deleter.iter() {
                let SemanticChange::RemoveObject(c) = change else {
                    continue;
                };
                if c.object.kind() != ObjectKind::ContextProvision
                    || !user_side.contains(c.object.id())
                {
                    continue;
                }
                let (Some(key), Some(provider)) = (
                    id_property(&c.object, CONTEXT_PROVISION_KEY),
                    id_property(&c.object, CONTEXT_PROVISION_PROVIDER),
                ) else {
                    continue;
                };
                let document = c.object.origin().document();
                let consumed = new_reads(user_diff, &input.base, key)
                    .into_iter()
                    .any(|reader| match under(user_side, reader, provider) {
                        Some(inside) => inside,
                        None => user_side
                            .get(reader)
                            .is_some_and(|o| o.origin().document() == document),
                    });
                if consumed {
                    removed.insert(c.object.id());
                }
            }
        }
        for provision in removed {
            self.accepted.retain(|change| {
                !(change.object_id() == provision
                    && matches!(change, SemanticChange::RemoveObject(_)))
            });
            self.object_conflict(ConflictKind::ContextProvisionRemovedVsConsume, provision);
        }

        // 2. key 타입 변경 대 provision 값 변경.
        let mut paths: BTreeSet<PropertyPath> = BTreeSet::new();
        let mut keys: BTreeSet<StableId> = BTreeSet::new();
        let mut added: BTreeSet<StableId> = BTreeSet::new();
        for (typer, valuer, valuer_side) in
            [(ours, theirs, &input.theirs), (theirs, ours, &input.ours)]
        {
            for change in typer.iter() {
                let Some(path) = change.property_path() else {
                    continue;
                };
                let Some(key_object) = input
                    .base
                    .get(path.root())
                    .filter(|o| o.kind() == ObjectKind::ContextKey)
                else {
                    continue;
                };
                if *path != key_object.path(&[CONTEXT_KEY_TYPE]) {
                    continue;
                }
                let key = key_object.id();
                let mut hit = false;
                for other in valuer.iter() {
                    match other {
                        SemanticChange::AddObject(c)
                            if c.object.kind() == ObjectKind::ContextProvision
                                && id_property(&c.object, CONTEXT_PROVISION_KEY) == Some(key) =>
                        {
                            added.insert(c.object.id());
                            hit = true;
                        }
                        _ => {
                            let Some(value_path) = other.property_path() else {
                                continue;
                            };
                            let Some(provision) = valuer_side
                                .get(value_path.root())
                                .filter(|o| o.kind() == ObjectKind::ContextProvision)
                            else {
                                continue;
                            };
                            if *value_path == provision.path(&[CONTEXT_PROVISION_VALUE])
                                && id_property(provision, CONTEXT_PROVISION_KEY) == Some(key)
                            {
                                paths.insert(value_path.clone());
                                hit = true;
                            }
                        }
                    }
                }
                if hit {
                    keys.insert(key);
                }
            }
        }
        for path in &paths {
            self.accepted
                .retain(|change| change.property_path() != Some(path));
            let provenance = input
                .base
                .get(path.root())
                .map_or(Provenance::Project, object_provenance);
            self.conflicts.push(SemanticConflict {
                kind: ConflictKind::ContextKeyTypeVsProvisionValue,
                provenance,
                object: path.root(),
                target: ConflictTarget::Property(path.clone()),
                base: value_state(&input.base, path),
                ours: value_state(&input.ours, path),
                theirs: value_state(&input.theirs, path),
            });
        }
        for key in keys {
            self.accepted.retain(|change| change.object_id() != key);
            self.object_conflict(ConflictKind::ContextKeyTypeVsProvisionValue, key);
        }
        for provision in added {
            self.accepted.retain(|change| {
                !(change.object_id() == provision && matches!(change, SemanticChange::AddObject(_)))
            });
            self.object_conflict(ConflictKind::ContextKeyTypeVsProvisionValue, provision);
        }
    }
}
