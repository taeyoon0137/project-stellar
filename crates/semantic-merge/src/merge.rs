//! 3-way Semantic Merge 엔진 (XIV.10.2~).
//!
//! base → ours, base → theirs의 semantic diff를 **대상** 단위로 나눠 비교합니다.
//!
//! | 대상 | 변경 |
//! |---|---|
//! | 객체 구조 | 추가·삭제·교체(AddObject/RemoveObject, token 포함) |
//! | 객체 위치 | MoveObject·ReorderObject |
//! | 속성 | `StableId + PropertyPath`의 속성 변경 4종 |
//!
//! **자동 병합(XIV.10.2)**: 한쪽만 바꾼 대상은 그대로 받아들입니다. 서로 다른 StableId의 변경,
//! 같은 객체의 서로 다른 PropertyPath 변경, 같은 객체의 위치 변경과 속성 변경은 겹치지 않으므로
//! conflict가 없습니다. 받아들인 변경은 semantic-diff `apply`로 base에 적용해 candidate를 만듭니다.
//!
//! **동일 변경(XIV.10.3)**: 양쪽이 같은 대상을 **같은 결과**로 바꾸면(같은 PropertyPath를
//! canonical 비교로 같은 값으로, 같은 위치로, 같은 삭제·추가로) conflict 없이 한 변경으로
//! 합칩니다.
//!
//! **겹침**: 양쪽이 같은 대상을 서로 다르게 바꾸면 conflict이며 candidate는 그 대상의 base 상태를
//! 유지합니다. 한쪽이 객체 구조를 바꾸면(삭제·교체) 다른 쪽의 그 객체 변경 전체가 겹칩니다.
//!
//! **conflict 분류(XIV.10.5)**: 한쪽 삭제 대 다른 쪽 편집은 다른 쪽이 위치만 바꿨으면
//! delete-vs-move, 속성·구조를 바꿨으면 delete-vs-modify입니다. 같은 속성의 서로 다른 변경은
//! 한쪽이 값 타입(`ValueType`)을 바꾸고 다른 쪽이 같은 타입 값만 바꿨으면
//! type-change-vs-value-change, 그 밖은 property-value입니다. 양쪽이 같은 StableId를 서로 다르게
//! 정의(추가·교체)하면 inconsistent-definition입니다.
//!
//! **전용 conflict(XIV.10.7)**: 서로 다른 대상이라 따로 받아들이면 결과 의미가 깨지는 세 경우는 전용
//! 종류입니다.
//!
//! - token-type-vs-value: 한쪽이 token 타입(`TOKEN_TYPE`)을, 다른 쪽이 같은 token의 값(`TOKEN_VALUE`)
//!   이나 expression(`TOKEN_EXPRESSION`)을 바꿨고 두 결과 token의 (타입, 값, expression)이 다르면 token
//!   객체 전체가 conflict입니다. candidate는 token의 base 상태를 유지합니다.
//! - component-property-removed-vs-override / drilling-chain-break: 한쪽이 컴포넌트 Property 목록
//!   (`COMPONENT_PROPERTIES`)에서 Property를 빼고, 그 Property를 아직 목록에 둔 다른 쪽이 그 Property를
//!   사용하는 변경을 더하면(값이 있는 새 상태) 목록 삭제를 받아들이지 않고 목록 속성 대상 conflict로
//!   남깁니다. 사용이 scene(문서) 인스턴스의 override 경로면 component-property-removed-vs-override,
//!   정의 tree 안 nested 인스턴스의 override(drilling link, IX.4.1)이거나 값 bytes가 그 Property
//!   StableId를 담으면(binding chain) drilling-chain-break입니다. 둘 다 있으면 앞의 것입니다. 사용 변경은
//!   목록이 base(Property 있음)로 남으므로 그대로 받아들입니다.
//!
//! **순서(XIV.10.6)**: 같은 parent에 양쪽이 독립적으로 삽입한 자식은 모두 받아들입니다. 같은
//! 간격에 삽입해 같은 OrderKey가 나오면 `(OrderKey, StableId)` 순서를 유지한 채 key를 유일하게
//! 다시 매깁니다. 같은 child를 양쪽이 다른 위치로 옮기면 conflicting-reorder이고, 서로 다른
//! 객체의 이동이 합쳐져 parent cycle이 생기면 그 이동들도 conflicting-reorder로 되돌립니다.
//!
//! **구조 정합**: 한쪽이 삭제한 parent 아래에 다른 쪽이 객체를 추가·이동해 candidate에 parent
//! 없는 객체가 생기면, 그 parent 삭제를 받아들이지 않고 delete-vs-modify conflict로 바꿉니다
//! (입력 자체에 이미 있던 고아는 건드리지 않습니다).
//!
//! **정의 목록(SM-1)**: 페이지·View·Slot 순서는 OrderKey가 아니라 부모 정의 객체의 Ids 속성 하나
//! (`DOCUMENT_PAGES`·`PAGE_VIEWS`·`COMPONENT_SLOTS`)입니다. 이 목록을 원자 값으로 비교하면 서로 다른
//! 페이지·View 독립 삽입(아키텍처 §27.1 "충돌하지 않는 자식 삽입")이 conflict가 되고, 소속 객체 추가·삭제는
//! 따로 받아들여져 목록과 객체가 어긋납니다. 그래서 양쪽이 같은 목록을 바꾸면 객체 단계에서 받아들이지
//! 않고, 모든 객체 결정이 끝난 뒤 candidate에 실제로 남은 소속 객체를 원소로 목록을 다시 만듭니다
//! ([`merge_member_order`]). 원소 집합은 항상 candidate 소속 객체와 같고, 순서는 base 순서 위에 양쪽의
//! 독립 삽입·삭제를 합칩니다. 양쪽이 목록을 서로 다른 순서로 재정렬했을 때만 목록 속성 대상
//! conflicting-reorder이며, 그때 candidate는 남은 객체의 base 순서를 유지합니다.

use std::collections::{BTreeMap, BTreeSet};

use stellar_semantic_diff::apply::apply;
use stellar_semantic_diff::change::{Position, Provenance, SemanticChange, SemanticChangeKind};
use stellar_semantic_diff::core_types::ids::StableId;
use stellar_semantic_diff::core_types::order_key::OrderKey;
use stellar_semantic_diff::core_types::property::{PropertyKey, PropertyPath};
use stellar_semantic_diff::diff::{diff, object_provenance, SemanticDiff};
use stellar_semantic_diff::keys;
use stellar_semantic_diff::snapshot::{ObjectKind, Origin, SemanticObject, SemanticSnapshot};
use stellar_semantic_diff::validate::{introduced_violations, override_member, SnapshotViolation};
use stellar_semantic_diff::value::SemanticValue;

use crate::model::{
    ConflictKind, ConflictTarget, MergeError, MergeInput, MergeResult, SemanticConflict, SideState,
};

/// 한쪽 diff에서 객체 하나에 대한 변경 묶음.
#[derive(Default)]
struct Edits<'a> {
    /// 추가·삭제(교체면 둘 다).
    structure: Vec<&'a SemanticChange>,
    /// 이동·재정렬.
    position: Option<&'a SemanticChange>,
    /// 속성 경로별 변경.
    properties: BTreeMap<&'a PropertyPath, &'a SemanticChange>,
}

impl Edits<'_> {
    /// 삭제만 있는지(교체가 아님).
    fn deletes(&self) -> bool {
        self.structure.iter().all(|c| {
            matches!(
                c.kind(),
                SemanticChangeKind::RemoveObject | SemanticChangeKind::RemoveToken
            )
        }) && !self.structure.is_empty()
    }

    fn all(&self) -> impl Iterator<Item = &SemanticChange> + '_ {
        self.structure
            .iter()
            .copied()
            .chain(self.position)
            .chain(self.properties.values().copied())
    }
}

fn group(diff: &SemanticDiff) -> BTreeMap<StableId, Edits<'_>> {
    let mut edits: BTreeMap<StableId, Edits<'_>> = BTreeMap::new();
    for change in diff.iter() {
        let entry = edits.entry(change.object_id()).or_default();
        match change.kind() {
            SemanticChangeKind::AddObject
            | SemanticChangeKind::RemoveObject
            | SemanticChangeKind::AddToken
            | SemanticChangeKind::RemoveToken => entry.structure.push(change),
            SemanticChangeKind::MoveObject | SemanticChangeKind::ReorderObject => {
                entry.position = Some(change)
            }
            _ => {
                if let Some(path) = change.property_path() {
                    entry.properties.insert(path, change);
                }
            }
        }
    }
    edits
}

/// 객체 상태(없으면 Missing).
fn object_state(snapshot: &SemanticSnapshot, id: StableId) -> SideState {
    snapshot.get(id).map_or(SideState::Missing, |o| {
        SideState::Object(Box::new(o.clone()))
    })
}

/// 위치 상태(객체가 없으면 Missing).
fn position_state(snapshot: &SemanticSnapshot, id: StableId) -> SideState {
    snapshot
        .get(id)
        .map_or(SideState::Missing, |o| SideState::Position(Position::of(o)))
}

/// 속성 상태(객체가 없으면 Missing).
fn value_state(snapshot: &SemanticSnapshot, path: &PropertyPath) -> SideState {
    snapshot.get(path.root()).map_or(SideState::Missing, |o| {
        SideState::Value(o.property(path).cloned())
    })
}

/// 한쪽 삭제 대 다른 쪽 편집의 conflict 종류(XIV.10.5): 다른 쪽이 위치만 바꿨으면
/// delete-vs-move, 속성·구조를 바꿨으면 delete-vs-modify.
fn delete_conflict_kind(other: &Edits<'_>) -> ConflictKind {
    if other.structure.is_empty() && other.properties.is_empty() && other.position.is_some() {
        ConflictKind::DeleteVsMove
    } else {
        ConflictKind::DeleteVsModify
    }
}

/// 속성 변경의 이전·이후 값.
fn property_values(
    change: &SemanticChange,
) -> Option<(Option<&SemanticValue>, Option<&SemanticValue>)> {
    match change {
        SemanticChange::SetProperty(c)
        | SemanticChange::ChangeToken(c)
        | SemanticChange::ComponentInterfaceChange(c)
        | SemanticChange::AssetRefChange(c) => Some((c.old.as_ref(), c.new.as_ref())),
        _ => None,
    }
}

/// 값이 있는 상태에서 타입이 바뀌는 변경인지(`Some(true)`), 같은 타입 값 변경인지(`Some(false)`).
/// 속성 추가·삭제는 `None`.
fn type_changes(change: &SemanticChange) -> Option<bool> {
    match property_values(change)? {
        (Some(old), Some(new)) => Some(old.value_type() != new.value_type()),
        _ => None,
    }
}

/// 같은 속성의 서로 다른 변경 conflict 종류(XIV.10.5): 한쪽은 값 타입을 바꾸고 다른 쪽은 같은
/// 타입 안에서 값만 바꿨으면 type-change-vs-value-change, 그 밖은 property-value.
fn property_conflict_kind(a: &SemanticChange, b: &SemanticChange) -> ConflictKind {
    match (type_changes(a), type_changes(b)) {
        (Some(true), Some(false)) | (Some(false), Some(true)) => {
            ConflictKind::TypeChangeVsValueChange
        }
        _ => ConflictKind::PropertyValue,
    }
}

/// 두 위치 변경의 결과 위치가 같은지.
fn same_position(a: &SemanticChange, b: &SemanticChange) -> bool {
    match (a, b) {
        (
            SemanticChange::MoveObject(x) | SemanticChange::ReorderObject(x),
            SemanticChange::MoveObject(y) | SemanticChange::ReorderObject(y),
        ) => x.to == y.to,
        _ => false,
    }
}

/// 두 속성 변경의 이후 값이 canonical 비교로 같은지(둘 다 삭제 포함).
fn same_new_value(a: &SemanticChange, b: &SemanticChange) -> bool {
    let new = |change: &SemanticChange| match change {
        SemanticChange::SetProperty(c)
        | SemanticChange::ChangeToken(c)
        | SemanticChange::ComponentInterfaceChange(c)
        | SemanticChange::AssetRefChange(c) => Some(c.new.clone()),
        _ => None,
    };
    match (new(a), new(b)) {
        (Some(None), Some(None)) => true,
        (Some(Some(x)), Some(Some(y))) => x.canonical_eq(&y),
        _ => false,
    }
}

/// owner 종류의 정의 목록 key: 문서의 페이지 목록, 페이지의 View 목록, 컴포넌트의 Slot 목록.
fn member_list_key(kind: ObjectKind) -> Option<PropertyKey> {
    match kind {
        ObjectKind::Document => Some(keys::DOCUMENT_PAGES),
        ObjectKind::Page => Some(keys::PAGE_VIEWS),
        ObjectKind::Component => Some(keys::COMPONENT_SLOTS),
        _ => None,
    }
}

/// 객체가 들어 있어야 하는 정의 목록 `(owner, 목록 key)`. semantic validator(XIV.5.3)의 소속 규칙과
/// 같습니다: Page는 parent의 페이지 목록, View는 parent의 View 목록, Slot은 소유 컴포넌트의 Slot 목록.
fn membership(object: &SemanticObject) -> Option<(StableId, PropertyKey)> {
    match (object.kind(), object.origin()) {
        (ObjectKind::Page, _) => object.parent().map(|p| (p, keys::DOCUMENT_PAGES)),
        (ObjectKind::View, _) => object.parent().map(|p| (p, keys::PAGE_VIEWS)),
        (ObjectKind::ComponentSlot, origin) => {
            origin.component().map(|id| (id, keys::COMPONENT_SLOTS))
        }
        _ => None,
    }
}

/// 경로가 `kind` 객체의 정의 목록 속성 경로인지.
fn is_member_list_path(kind: ObjectKind, path: &PropertyPath) -> bool {
    member_list_key(kind)
        .is_some_and(|key| *path == PropertyPath::with_keys(path.root(), vec![key]))
}

/// `snapshot`에서 `owner`와 같은 정체성 객체의 목록 값. 객체·속성이 없으면 빈 목록, Ids가 아닌 값이면
/// `None`(정의 목록으로 병합하지 않고 원자 값으로 비교합니다).
fn listed<'s>(
    snapshot: &'s SemanticSnapshot,
    owner: &SemanticObject,
    path: &PropertyPath,
) -> Option<&'s [StableId]> {
    match snapshot
        .get(owner.id())
        .filter(|o| o.same_identity(owner))
        .and_then(|o| o.property(path))
    {
        None => Some(&[]),
        Some(SemanticValue::Ids(ids)) => Some(ids),
        Some(_) => None,
    }
}

/// snapshot의 정의 목록별 소속 객체 `(owner, 목록 key) → 객체 집합`(객체를 한 번 순회).
fn members_by_list(
    snapshot: &SemanticSnapshot,
) -> BTreeMap<(StableId, PropertyKey), BTreeSet<StableId>> {
    let mut out: BTreeMap<(StableId, PropertyKey), BTreeSet<StableId>> = BTreeMap::new();
    for object in snapshot.objects() {
        if let Some(list) = membership(object) {
            out.entry(list).or_default().insert(object.id());
        }
    }
    out
}

/// 목록에서 spine 밖 객체를, 그 목록에서 바로 앞에 있는 spine 객체(없으면 `None` = 맨 앞) 기준 묶음으로
/// 나눕니다. `skip`이 참인 객체는 넣지 않고 기준도 바꾸지 않습니다. spine 객체는 목록마다 한 번만 나오므로
/// 기준 하나에 묶음 하나(그 목록의 연속 순서)입니다.
fn anchored_runs(
    list: &[StableId],
    spine: &BTreeSet<StableId>,
    skip: impl Fn(&StableId) -> bool,
) -> BTreeMap<Option<StableId>, Vec<StableId>> {
    let mut runs: BTreeMap<Option<StableId>, Vec<StableId>> = BTreeMap::new();
    let mut anchor = None;
    for id in list {
        if spine.contains(id) {
            anchor = Some(*id);
        } else if !skip(id) {
            runs.entry(anchor).or_default().push(*id);
        }
    }
    runs
}

/// 정의 목록 3-way 순서 병합(아키텍처 §27.1 "충돌하지 않는 자식 삽입", SM-1).
///
/// 결과 원소 집합은 항상 `members`(candidate에 실제로 남은 소속 객체)와 같습니다. 각 목록은 먼저
/// `members`로 거르고 중복은 첫 등장만 남깁니다.
///
/// 1. **spine**(ours·theirs 목록에 모두 있는 객체)의 순서: 양쪽이 같으면 그 순서, 한쪽만 base 순서와
///    다르면 그쪽, `keep_base`(owner 자체가 객체 conflict)면 base 순서입니다. 양쪽이 서로 다르게
///    재정렬했으면 base 순서를 쓰고 conflict(`true`)를 돌려줍니다. base에 없던 spine 객체(양쪽이 같은
///    객체를 넣음)는 base 순서 뒤에 StableId 순서로 둡니다.
/// 2. **한쪽 목록에만 있는 객체**(그쪽 삽입, 또는 다른 쪽 삭제를 받아들이지 않은 base 객체)는 그 목록에서
///    바로 앞 spine 객체 뒤(없으면 맨 앞)에 그 목록의 연속 순서대로 둡니다. 같은 자리에 양쪽 묶음이 오면
///    첫 객체의 StableId가 작은 묶음이 먼저입니다(좌우 교환에도 같은 결과).
/// 3. **어느 쪽 목록에도 없는 객체**는 base 목록 위치에 먼저 두고, base에도 없으면 맨 뒤에 StableId
///    순서로 둡니다.
///
/// 정렬 집합·map만 쓰므로 비용은 목록 길이 합 n에 대해 O(n log n)입니다.
fn merge_member_order(
    base: &[StableId],
    ours: &[StableId],
    theirs: &[StableId],
    members: &BTreeSet<StableId>,
    keep_base: bool,
) -> (Vec<StableId>, bool) {
    let restrict = |list: &[StableId]| {
        let mut seen = BTreeSet::new();
        list.iter()
            .copied()
            .filter(|id| members.contains(id) && seen.insert(*id))
            .collect::<Vec<_>>()
    };
    let (base, ours, theirs) = (restrict(base), restrict(ours), restrict(theirs));
    let in_base: BTreeSet<StableId> = base.iter().copied().collect();
    let in_ours: BTreeSet<StableId> = ours.iter().copied().collect();
    let in_theirs: BTreeSet<StableId> = theirs.iter().copied().collect();
    let spine: BTreeSet<StableId> = in_ours.intersection(&in_theirs).copied().collect();
    let on_spine = |list: &[StableId]| {
        list.iter()
            .copied()
            .filter(|id| spine.contains(id))
            .collect::<Vec<_>>()
    };
    let (base_spine, ours_spine, theirs_spine) =
        (on_spine(&base), on_spine(&ours), on_spine(&theirs));
    let base_order = || {
        let mut order = base_spine.clone();
        order.extend(spine.iter().copied().filter(|id| !in_base.contains(id)));
        order
    };
    let spine_in_base = base_spine.len() == spine.len();
    let (order, conflict) = if keep_base {
        (base_order(), false)
    } else if ours_spine == theirs_spine {
        (ours_spine, false)
    } else if spine_in_base && ours_spine == base_spine {
        (theirs_spine, false)
    } else if spine_in_base && theirs_spine == base_spine {
        (ours_spine, false)
    } else {
        (base_order(), true)
    };

    let ours_runs = anchored_runs(&ours, &spine, |_| false);
    let theirs_runs = anchored_runs(&theirs, &spine, |_| false);
    let base_runs = anchored_runs(&base, &spine, |id| {
        in_ours.contains(id) || in_theirs.contains(id)
    });
    let place = |anchor: Option<StableId>, out: &mut Vec<StableId>| {
        if let Some(run) = base_runs.get(&anchor) {
            out.extend_from_slice(run);
        }
        let mut sides: Vec<&Vec<StableId>> = [ours_runs.get(&anchor), theirs_runs.get(&anchor)]
            .into_iter()
            .flatten()
            .collect();
        sides.sort_by_key(|run| run.first().copied());
        for run in sides {
            out.extend_from_slice(run);
        }
    };
    let mut out = Vec::with_capacity(members.len());
    place(None, &mut out);
    for id in order {
        out.push(id);
        place(Some(id), &mut out);
    }
    out.extend(
        members
            .iter()
            .copied()
            .filter(|id| !in_ours.contains(id) && !in_theirs.contains(id) && !in_base.contains(id)),
    );
    (out, conflict)
}

/// candidate의 정의 목록을 candidate 소속 객체와 맞춥니다(SM-1).
///
/// 대상은 어느 한쪽이라도 바꾼 목록(`changed`)과 소속 객체 집합이 base와 달라진 목록입니다. owner가
/// candidate에 있고 세 입력·candidate의 목록 값이 모두 Ids(또는 없음)일 때 [`merge_member_order`]로 다시
/// 만듭니다. owner 자체가 객체 conflict면 base 순서를 유지하고 목록 conflict를 따로 만들지 않습니다(객체
/// conflict가 세 상태를 이미 담음). 양쪽 재정렬이 겹치면 목록 속성 대상 conflicting-reorder를 남깁니다.
fn reconcile_member_lists(
    input: &MergeInput,
    changed: &BTreeSet<PropertyPath>,
    merged: &mut SemanticSnapshot,
    conflicts: &mut Vec<SemanticConflict>,
) {
    let base_members = members_by_list(&input.base);
    let merged_members = members_by_list(merged);
    let mut lists: BTreeSet<(StableId, PropertyKey)> = changed
        .iter()
        .filter_map(|path| Some((path.root(), *path.keys().first()?)))
        .collect();
    for list in base_members.keys().chain(merged_members.keys()) {
        if base_members.get(list) != merged_members.get(list) {
            lists.insert(*list);
        }
    }
    let held: BTreeSet<StableId> = conflicts
        .iter()
        .filter(|c| c.target == ConflictTarget::Object)
        .map(|c| c.object)
        .collect();
    let none = BTreeSet::new();
    for (owner, key) in lists {
        let Some(object) = merged.get(owner) else {
            continue;
        };
        if member_list_key(object.kind()) != Some(key) {
            continue;
        }
        let path = object.path(&[key]);
        let (Some(base), Some(ours), Some(theirs), Some(current)) = (
            listed(&input.base, object, &path),
            listed(&input.ours, object, &path),
            listed(&input.theirs, object, &path),
            listed(merged, object, &path),
        ) else {
            // Ids가 아닌 값은 merge_object가 원자 값으로 이미 처리했습니다.
            continue;
        };
        let members = merged_members.get(&(owner, key)).unwrap_or(&none);
        let (list, conflict) =
            merge_member_order(base, ours, theirs, members, held.contains(&owner));
        if conflict {
            let provenance = [&input.base, &input.ours, &input.theirs]
                .into_iter()
                .find_map(|side| side.get(owner))
                .map_or_else(|| object_provenance(object), object_provenance);
            conflicts.push(SemanticConflict {
                kind: ConflictKind::ConflictingReorder,
                provenance,
                object: owner,
                target: ConflictTarget::Property(path.clone()),
                base: value_state(&input.base, &path),
                ours: value_state(&input.ours, &path),
                theirs: value_state(&input.theirs, &path),
            });
        }
        // 이미 같으면(속성이 없고 결과도 빈 목록인 경우 포함) 그대로 둡니다.
        if current == list.as_slice() {
            continue;
        }
        if let Some(mut object) = merged.remove(owner) {
            // 경로 root가 이 객체이고 같은 id를 막 지웠으므로 둘 다 실패하지 않습니다.
            let _ = object.set_property(path, SemanticValue::Ids(list));
            let _ = merged.insert(object);
        }
    }
}

/// Component Context 전용 conflict(XXX.7.10, ADR-0099 결정 10). Merger 내부 변경 목록을 고쳐야 해서 하위 모듈입니다.
#[path = "context.rs"]
mod context;

/// merge 진행 상태.
struct Merger<'a> {
    input: &'a MergeInput,
    accepted: Vec<SemanticChange>,
    conflicts: Vec<SemanticConflict>,
    /// 마지막 [`Merger::settle`]이 계산한 정의 목록 conflict.
    list_conflicts: Vec<SemanticConflict>,
}

impl<'a> Merger<'a> {
    fn accept(&mut self, edits: &Edits<'_>) {
        self.accepted.extend(edits.all().cloned());
    }

    /// 양쪽이 모두 바꾼 정의 목록 속성은 여기서 받아들이거나 conflict로 만들지 않고
    /// [`reconcile_member_lists`]에 맡깁니다(SM-1). 세 입력의 값이 모두 Ids 목록(또는 없음)일 때만이며,
    /// 그 밖의 값은 원자 값으로 비교합니다. 한쪽만 바꾼 목록은 그대로 받아들입니다.
    fn defers_member_list(
        &self,
        id: StableId,
        path: &PropertyPath,
        ours: &Edits<'_>,
        theirs: &Edits<'_>,
    ) -> bool {
        let input = self.input;
        let Some(owner) = input.base.get(id) else {
            return false;
        };
        ours.properties.contains_key(path)
            && theirs.properties.contains_key(path)
            && is_member_list_path(owner.kind(), path)
            && [&input.base, &input.ours, &input.theirs]
                .into_iter()
                .all(|side| listed(side, owner, path).is_some())
    }

    /// 객체 구조 conflict. 출처는 base 객체(없으면 ours, theirs 순) 기준입니다.
    fn object_conflict(&mut self, kind: ConflictKind, id: StableId) {
        let input = self.input;
        let Some(owner) = input
            .base
            .get(id)
            .or_else(|| input.ours.get(id))
            .or_else(|| input.theirs.get(id))
        else {
            return;
        };
        self.conflicts.push(SemanticConflict {
            kind,
            provenance: object_provenance(owner),
            object: id,
            target: ConflictTarget::Object,
            base: object_state(&input.base, id),
            ours: object_state(&input.ours, id),
            theirs: object_state(&input.theirs, id),
        });
    }

    /// 객체 위치 conflict(conflicting-reorder).
    fn position_conflict(&mut self, id: StableId) {
        let input = self.input;
        let Some(owner) = input.base.get(id).or_else(|| input.ours.get(id)) else {
            return;
        };
        self.conflicts.push(SemanticConflict {
            kind: ConflictKind::ConflictingReorder,
            provenance: object_provenance(owner),
            object: id,
            target: ConflictTarget::Position,
            base: position_state(&input.base, id),
            ours: position_state(&input.ours, id),
            theirs: position_state(&input.theirs, id),
        });
    }

    fn merge_object(&mut self, id: StableId, ours: &Edits<'_>, theirs: &Edits<'_>) {
        let input = self.input;
        if !ours.structure.is_empty() && ours.structure == theirs.structure {
            // 같은 삭제·추가·교체(XIV.10.3). 구조 변경 객체에는 다른 변경이 없습니다.
            self.accept(ours);
            return;
        }
        if !ours.structure.is_empty() || !theirs.structure.is_empty() {
            let kind = match (ours.deletes(), theirs.deletes()) {
                (true, false) => delete_conflict_kind(theirs),
                (false, true) => delete_conflict_kind(ours),
                _ => ConflictKind::InconsistentDefinition,
            };
            self.object_conflict(kind, id);
            return;
        }
        if self.token_type_conflicts(id, ours, theirs) {
            self.object_conflict(ConflictKind::TokenTypeVsValue, id);
            return;
        }
        match (ours.position, theirs.position) {
            (Some(change), None) | (None, Some(change)) => self.accepted.push(change.clone()),
            (Some(change), Some(other)) if same_position(change, other) => {
                self.accepted.push(change.clone())
            }
            // 같은 child를 양쪽이 다른 위치로(XIV.10.6).
            (Some(_), Some(_)) => self.position_conflict(id),
            (None, None) => {}
        }
        let paths: BTreeSet<&PropertyPath> = ours
            .properties
            .keys()
            .chain(theirs.properties.keys())
            .copied()
            .collect();
        for path in paths {
            if self.defers_member_list(id, path, ours, theirs) {
                continue;
            }
            match (ours.properties.get(path), theirs.properties.get(path)) {
                (Some(change), None) | (None, Some(change)) => {
                    self.accepted.push((*change).clone())
                }
                (Some(change), Some(other)) if same_new_value(change, other) => {
                    // 같은 경로를 같은 값으로(XIV.10.3): 한 변경으로 합칩니다.
                    self.accepted.push((*change).clone())
                }
                (Some(change), Some(other)) => self.conflicts.push(SemanticConflict {
                    kind: property_conflict_kind(change, other),
                    provenance: *change.provenance(),
                    object: id,
                    target: ConflictTarget::Property(path.clone()),
                    base: value_state(&input.base, path),
                    ours: value_state(&input.ours, path),
                    theirs: value_state(&input.theirs, path),
                }),
                (None, None) => {}
            }
        }
    }

    /// token 타입 변경 대 값 변경(XIV.10.7): 한쪽이 `TOKEN_TYPE`을, 다른 쪽이 `TOKEN_VALUE`·
    /// `TOKEN_EXPRESSION`을 바꿨고 두 쪽 결과의 (타입, 값, expression)이 다르면 참입니다.
    fn token_type_conflicts(&self, id: StableId, ours: &Edits<'_>, theirs: &Edits<'_>) -> bool {
        let input = self.input;
        let Some(base) = input.base.get(id).filter(|o| o.kind() == ObjectKind::Token) else {
            return false;
        };
        let type_path = base.path(&[keys::TOKEN_TYPE]);
        let value_paths = [
            base.path(&[keys::TOKEN_VALUE]),
            base.path(&[keys::TOKEN_EXPRESSION]),
        ];
        let changes_type = |edits: &Edits<'_>| edits.properties.contains_key(&type_path);
        let changes_value =
            |edits: &Edits<'_>| value_paths.iter().any(|p| edits.properties.contains_key(p));
        if !((changes_type(ours) && changes_value(theirs))
            || (changes_type(theirs) && changes_value(ours)))
        {
            return false;
        }
        let tuple = |side: &SemanticSnapshot| {
            let object = side.get(id)?;
            Some(
                [&type_path, &value_paths[0], &value_paths[1]]
                    .map(|path| object.property(path).map(SemanticValue::to_canonical_bytes)),
            )
        };
        tuple(&input.ours) != tuple(&input.theirs)
    }

    /// 컴포넌트 Property 삭제 대 사용 추가(XIV.10.7). 한쪽(삭제 쪽)이 Property 목록에서 뺀 Property를 다른
    /// 쪽이 아직 목록에 두고 사용하는 변경을 더하면, 삭제 쪽 목록 변경을 받아들이지 않고 목록 속성 대상
    /// conflict로 바꿉니다. 양쪽이 모두 목록을 바꿔 이미 목록 conflict가 있으면 그 종류만 바꿉니다.
    fn interface_use_conflicts(&mut self, ours: &SemanticDiff, theirs: &SemanticDiff) {
        let input = self.input;
        let mut found: BTreeMap<PropertyPath, ConflictKind> = BTreeMap::new();
        for (deleter, user, user_diff) in
            [(ours, &input.theirs, theirs), (theirs, &input.ours, ours)]
        {
            for change in deleter.iter() {
                let Some(path) = change.property_path() else {
                    continue;
                };
                let Some(component) = input
                    .base
                    .get(path.root())
                    .filter(|o| o.kind() == ObjectKind::Component)
                else {
                    continue;
                };
                if *path != component.path(&[keys::COMPONENT_PROPERTIES]) {
                    continue;
                }
                let Some((old, new)) = property_values(change) else {
                    continue;
                };
                let kept = id_set(new);
                let still_listed = id_set(user.get(component.id()).and_then(|o| o.property(path)));
                let removed: BTreeSet<StableId> = id_set(old)
                    .into_iter()
                    .filter(|id| !kept.contains(id) && still_listed.contains(id))
                    .collect();
                if removed.is_empty() {
                    continue;
                }
                if let Some(kind) = uses_member(user, user_diff, &removed) {
                    let entry = found.entry(path.clone()).or_insert(kind);
                    *entry = (*entry).min(kind);
                }
            }
        }
        for (path, kind) in found {
            let target = ConflictTarget::Property(path.clone());
            if let Some(existing) = self
                .conflicts
                .iter_mut()
                .find(|c| c.object == path.root() && c.target == target)
            {
                existing.kind = kind;
                continue;
            }
            let before = self.accepted.len();
            self.accepted
                .retain(|change| change.property_path() != Some(&path));
            if self.accepted.len() == before {
                continue;
            }
            let provenance = input
                .base
                .get(path.root())
                .map_or(Provenance::Project, object_provenance);
            self.conflicts.push(SemanticConflict {
                kind,
                provenance,
                object: path.root(),
                target,
                base: value_state(&input.base, &path),
                ours: value_state(&input.ours, &path),
                theirs: value_state(&input.theirs, &path),
            });
        }
    }

    /// candidate에서 merge가 만든 고아를 찾습니다: parent가 없지만, 그 객체를 그 parent 아래에
    /// 둔 쪽 snapshot에는 parent가 있던 경우.
    ///
    /// 반환: (삭제된 parent, 고아가 된 객체).
    fn merge_orphan(&self, merged: &SemanticSnapshot) -> Option<(StableId, StableId)> {
        let input = self.input;
        merged.objects().find_map(|object| {
            let parent = object.parent()?;
            if merged.contains(parent) {
                return None;
            }
            let placed_with_parent = [&input.ours, &input.theirs].into_iter().any(|side| {
                side.get(object.id())
                    .is_some_and(|o| o.parent() == Some(parent) && side.contains(parent))
            });
            placed_with_parent.then_some((parent, object.id()))
        })
    }
}

/// Ids 값의 원소 집합(없거나 Ids가 아니면 빈 집합).
fn id_set(value: Option<&SemanticValue>) -> BTreeSet<StableId> {
    match value {
        Some(SemanticValue::Ids(ids)) => ids.iter().copied().collect(),
        _ => BTreeSet::new(),
    }
}

/// 값의 canonical bytes가 `ids` 중 하나의 16바이트 StableId를 담는지(보수적 scan, dependency graph와
/// 같은 방식). StableId는 전역 유일 값이라 우연 일치는 사실상 없고, 일치하면 conflict가 늘어나는
/// 방향입니다.
pub(crate) fn mentions(value: &SemanticValue, ids: &BTreeSet<StableId>) -> bool {
    let windows: BTreeSet<[u8; 16]> = ids.iter().map(StableId::to_bytes).collect();
    value.to_canonical_bytes().windows(16).any(|window| {
        let mut key = [0u8; 16];
        key.copy_from_slice(window);
        windows.contains(&key)
    })
}

/// 속성 하나가 `removed` Property를 사용하는 종류(XIV.10.7). override 경로의 member가 삭제 Property면
/// 인스턴스 위치(scene 문서 = component-property-removed-vs-override, 정의 tree = drilling-chain-break),
/// 값이 삭제 Property를 참조하면 drilling-chain-break입니다.
fn property_use(
    object: &SemanticObject,
    path: &PropertyPath,
    value: &SemanticValue,
    removed: &BTreeSet<StableId>,
) -> Option<ConflictKind> {
    let is_override = path.keys().first() == Some(&keys::instance::PROPERTY_OVERRIDES);
    if is_override && override_member(path).is_some_and(|(_, member)| removed.contains(&member)) {
        return Some(match object.origin() {
            Origin::Document(_) => ConflictKind::ComponentPropertyRemovedVsOverride,
            _ => ConflictKind::DrillingChainBreak,
        });
    }
    mentions(value, removed).then_some(ConflictKind::DrillingChainBreak)
}

/// `side`의 변경 중 `removed` Property를 새로 사용하는 변경의 conflict 종류(여럿이면 code가 작은 것).
/// 값이 없어지거나 그대로인 변경(override 삭제 등)은 사용이 아닙니다.
fn uses_member(
    side: &SemanticSnapshot,
    changes: &SemanticDiff,
    removed: &BTreeSet<StableId>,
) -> Option<ConflictKind> {
    let mut best: Option<ConflictKind> = None;
    let mut note = |kind: Option<ConflictKind>| {
        if let Some(kind) = kind {
            best = Some(best.map_or(kind, |b| b.min(kind)));
        }
    };
    for change in changes.iter() {
        match change {
            SemanticChange::AddObject(c) | SemanticChange::AddToken(c) => {
                for (path, value) in c.object.properties() {
                    note(property_use(&c.object, path, value, removed));
                }
            }
            _ => {
                let (Some(path), Some((old, Some(new)))) =
                    (change.property_path(), property_values(change))
                else {
                    continue;
                };
                if old.is_some_and(|old| old.canonical_eq(new)) {
                    continue;
                }
                if let Some(object) = side.get(path.root()) {
                    note(property_use(object, path, new, removed));
                }
            }
        }
    }
    best
}

/// candidate에서 merge가 만든 parent cycle을 찾습니다(cycle 구성 객체, 오름차순).
///
/// 받아들인 MoveObject가 하나도 없는 cycle은 입력에 이미 있던 것이므로 무시합니다. parent
/// 사슬은 반복문으로 따라가며 방문 집합으로 끝을 보장합니다(재귀 없음).
fn merge_cycle(
    merged: &SemanticSnapshot,
    accepted: &[SemanticChange],
) -> Option<BTreeSet<StableId>> {
    let moved: BTreeSet<StableId> = accepted
        .iter()
        .filter(|c| c.kind() == SemanticChangeKind::MoveObject)
        .map(SemanticChange::object_id)
        .collect();
    let mut settled = BTreeSet::new();
    for start in moved.iter().copied() {
        if settled.contains(&start) {
            continue;
        }
        let mut chain = Vec::new();
        let mut on_chain = BTreeSet::new();
        let mut current = Some(start);
        while let Some(id) = current {
            if settled.contains(&id) {
                break;
            }
            if !on_chain.insert(id) {
                let first = chain.iter().position(|c| *c == id).unwrap_or(0);
                let cycle: BTreeSet<StableId> = chain[first..].iter().copied().collect();
                if cycle.iter().any(|c| moved.contains(c)) {
                    return Some(cycle);
                }
                break;
            }
            chain.push(id);
            current = merged.get(id).and_then(|o| o.parent());
        }
        settled.extend(chain);
    }
    None
}

/// 독립 삽입이 같은 parent에서 같은 OrderKey를 받은 경우를 결정론적으로 풉니다(XIV.10.6).
///
/// `OrderKey::between`은 결정론적이라 두 branch가 같은 이웃 사이에 삽입하면 같은 key가 나옵니다
/// (III.5.4 fixture). 순서는 `(OrderKey, StableId)`로 이미 정해지므로, 같은 key 묶음에서 StableId가
/// 가장 작은 객체는 key를 유지하고, merge가 새로 배치했으며(base와 parent·key가 다름) 앞선 묶음
/// 구성원과의 충돌이 어느 한 입력에도 이미 있던 것이 아닌(서로 다른 쪽에서 온) 객체만 그 다음
/// 다른 key와의 사이 key로 옮깁니다. 시각 순서는 그대로이고 형제 key가 유일해집니다(문서
/// validator VI.10.1의 OrderKey 유일성). 입력에 이미 있던 충돌은 바꾸지 않습니다.
fn rekey_independent_insertions(input: &MergeInput, merged: &mut SemanticSnapshot) {
    let base = &input.base;
    let at = |side: &SemanticSnapshot, id: StableId, parent: StableId, key: &OrderKey| {
        side.get(id)
            .is_some_and(|o| o.parent() == Some(parent) && o.order_key() == Some(key))
    };
    let mut siblings: BTreeMap<StableId, Vec<(OrderKey, StableId)>> = BTreeMap::new();
    for object in merged.objects() {
        if let (Some(parent), Some(key)) = (object.parent(), object.order_key()) {
            siblings
                .entry(parent)
                .or_default()
                .push((key.clone(), object.id()));
        }
    }
    let placed_by_merge = |id: StableId, parent: StableId, key: &OrderKey| {
        base.get(id)
            .is_none_or(|o| o.parent() != Some(parent) || o.order_key() != Some(key))
    };
    let mut updates: Vec<(StableId, StableId, OrderKey)> = Vec::new();
    for (parent, mut children) in siblings {
        children.sort();
        let mut start = 0;
        while start < children.len() {
            let mut end = start + 1;
            while end < children.len() && children[end].0 == children[start].0 {
                end += 1;
            }
            let next = children.get(end).map(|(key, _)| key.clone());
            let mut previous = children[start].0.clone();
            for (offset, (key, id)) in children[start + 1..end].iter().enumerate() {
                let earlier = &children[start..start + 1 + offset];
                let preexisting = [&input.ours, &input.theirs].into_iter().any(|side| {
                    at(side, *id, parent, key)
                        && earlier.iter().all(|(_, e)| at(side, *e, parent, key))
                });
                if placed_by_merge(*id, parent, key) && !preexisting {
                    let fresh = OrderKey::between(Some(&previous), next.as_ref());
                    updates.push((*id, parent, fresh.clone()));
                    previous = fresh;
                }
            }
            start = end;
        }
    }
    for (id, parent, key) in updates {
        if let Some(mut object) = merged.remove(id) {
            object.set_position(Some(parent), Some(key));
            // 같은 id를 막 지웠으므로 다시 넣기는 실패하지 않습니다.
            let _ = merged.insert(object);
        }
    }
}

/// 위반에 관련된 받아들인 변경의 단위(XIV.10.8).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Unit {
    /// 객체 삭제.
    Remove(StableId),
    /// 객체 추가.
    Add(StableId),
    /// 객체 위치.
    Position(StableId),
    /// 속성 경로.
    Property(PropertyPath),
    /// 이 member를 interface 목록(Property·Slot)에서 빼는 변경.
    InterfaceMember(StableId),
}

impl Unit {
    fn matches(&self, change: &SemanticChange) -> bool {
        let id = change.object_id();
        match self {
            Unit::Remove(object) => {
                id == *object
                    && matches!(
                        change.kind(),
                        SemanticChangeKind::RemoveObject | SemanticChangeKind::RemoveToken
                    )
            }
            Unit::Add(object) => {
                id == *object
                    && matches!(
                        change.kind(),
                        SemanticChangeKind::AddObject | SemanticChangeKind::AddToken
                    )
            }
            Unit::Position(object) => {
                id == *object
                    && matches!(
                        change.kind(),
                        SemanticChangeKind::MoveObject | SemanticChangeKind::ReorderObject
                    )
            }
            Unit::Property(path) => change.property_path() == Some(path),
            Unit::InterfaceMember(member) => {
                let is_list = change.property_path().is_some_and(|path| {
                    matches!(
                        path.keys(),
                        [keys::COMPONENT_PROPERTIES] | [keys::COMPONENT_SLOTS]
                    )
                });
                is_list
                    && property_values(change).is_some_and(|(old, new)| {
                        id_set(old).contains(member) && !id_set(new).contains(member)
                    })
            }
        }
    }

    /// 되돌린 변경의 conflict 대상.
    fn target(change: &SemanticChange) -> ConflictTarget {
        match change.kind() {
            SemanticChangeKind::AddObject
            | SemanticChangeKind::RemoveObject
            | SemanticChangeKind::AddToken
            | SemanticChangeKind::RemoveToken => ConflictTarget::Object,
            SemanticChangeKind::MoveObject | SemanticChangeKind::ReorderObject => {
                ConflictTarget::Position
            }
            _ => change
                .property_path()
                .map_or(ConflictTarget::Object, |path| {
                    ConflictTarget::Property(path.clone())
                }),
        }
    }
}

/// 위반 하나에 관련된 단위(되돌릴 후보)와, 되돌릴 변경이 없을 때 conflict로 남길 대표 `(객체, 대상)`.
fn violation_units(
    base: &SemanticSnapshot,
    violation: &SnapshotViolation,
) -> (Vec<Unit>, (StableId, ConflictTarget)) {
    match violation {
        SnapshotViolation::MissingParent { object, parent } => (
            vec![
                Unit::Add(*object),
                Unit::Position(*object),
                Unit::Remove(*parent),
            ],
            (*object, ConflictTarget::Position),
        ),
        SnapshotViolation::ParentCycle { object } => (
            vec![Unit::Position(*object)],
            (*object, ConflictTarget::Position),
        ),
        SnapshotViolation::DuplicateOrderKey { objects, .. } => (
            objects
                .iter()
                .flat_map(|id| [Unit::Add(*id), Unit::Position(*id)])
                .collect(),
            (
                objects.first().copied().unwrap_or(StableId::NIL),
                ConflictTarget::Position,
            ),
        ),
        SnapshotViolation::DanglingReference { path, target } => (
            vec![
                Unit::Property(path.clone()),
                Unit::Add(path.root()),
                Unit::Remove(*target),
            ],
            (path.root(), ConflictTarget::Property(path.clone())),
        ),
        SnapshotViolation::UnlistedMember {
            object,
            owner,
            list,
        } => (
            vec![
                Unit::Property(PropertyPath::with_keys(*owner, vec![*list])),
                Unit::Add(*object),
                Unit::Position(*object),
            ],
            (*object, ConflictTarget::Object),
        ),
        SnapshotViolation::OrphanRoot { node } => {
            // node를 root로 가리키던 base 객체(View·컴포넌트)의 삭제·root 변경도 관련 단위입니다.
            let mut units = vec![Unit::Add(*node), Unit::Position(*node)];
            for owner in base.objects() {
                for key in [keys::VIEW_ROOT_FRAME, keys::COMPONENT_ROOT] {
                    let path = owner.path(&[key]);
                    if owner.property(&path) == Some(&SemanticValue::Id(*node)) {
                        units.push(Unit::Remove(owner.id()));
                        units.push(Unit::Property(path));
                    }
                }
            }
            (units, (*node, ConflictTarget::Position))
        }
        SnapshotViolation::UnknownInterfaceMember { path, member } => (
            vec![
                Unit::Property(path.clone()),
                Unit::Add(path.root()),
                Unit::InterfaceMember(*member),
                Unit::Remove(*member),
            ],
            (path.root(), ConflictTarget::Property(path.clone())),
        ),
    }
}

/// 대상 하나의 세 상태(base, ours, theirs).
fn target_states(input: &MergeInput, object: StableId, target: &ConflictTarget) -> [SideState; 3] {
    [&input.base, &input.ours, &input.theirs].map(|side| match target {
        ConflictTarget::Object => object_state(side, object),
        ConflictTarget::Position => position_state(side, object),
        ConflictTarget::Property(path) => value_state(side, path),
    })
}

/// 대상 conflict의 출처(base 객체, 없으면 ours·theirs 순).
fn target_provenance(input: &MergeInput, object: StableId) -> Provenance {
    [&input.base, &input.ours, &input.theirs]
        .into_iter()
        .find_map(|side| side.get(object))
        .map_or(Provenance::Project, object_provenance)
}

impl Merger<'_> {
    /// 받아들인 변경으로 candidate를 만듭니다: 고아·cycle 되돌리기, 독립 삽입 key 재배정, 정의 목록 정합.
    /// 정의 목록 conflict는 매번 새로 계산해 `list_conflicts`에 둡니다.
    fn settle(
        &mut self,
        changed_lists: &BTreeSet<PropertyPath>,
    ) -> Result<SemanticSnapshot, MergeError> {
        let input = self.input;
        // 고아를 만든 parent 삭제를 conflict로 되돌리고 다시 적용합니다. 되돌릴 때마다 받아들인
        // 변경이 줄어들므로 유한 번에 끝납니다.
        let mut merged = apply(&input.base, &self.accepted).map_err(MergeError::Apply)?;
        while let Some((parent, orphan)) = self.merge_orphan(&merged) {
            let before = self.accepted.len();
            self.accepted.retain(|change| {
                !(change.object_id() == parent
                    && matches!(
                        change.kind(),
                        SemanticChangeKind::RemoveObject | SemanticChangeKind::RemoveToken
                    ))
            });
            if self.accepted.len() == before {
                // 삭제가 받아들여진 변경이 아니면(입력 불변식 위반) 더 되돌릴 것이 없습니다.
                break;
            }
            // 다른 쪽이 parent 아래로 옮겨 온 객체면 delete-vs-move, 새로 추가한 객체면
            // delete-vs-modify(subtree 수정)입니다(XIV.10.5).
            let kind = if input.base.contains(orphan) {
                ConflictKind::DeleteVsMove
            } else {
                ConflictKind::DeleteVsModify
            };
            self.object_conflict(kind, parent);
            merged = apply(&input.base, &self.accepted).map_err(MergeError::Apply)?;
        }

        // 서로 다른 객체의 독립 이동이 parent cycle을 만들면 그 이동들을 conflicting-reorder로
        // 되돌립니다(XIV.10.6). 되돌릴 때마다 받아들인 변경이 줄어 유한 번에 끝납니다.
        while let Some(cycle) = merge_cycle(&merged, &self.accepted) {
            let before = self.accepted.len();
            self.accepted.retain(|change| {
                !(cycle.contains(&change.object_id())
                    && change.kind() == SemanticChangeKind::MoveObject)
            });
            if self.accepted.len() == before {
                break;
            }
            for id in cycle {
                self.position_conflict(id);
            }
            merged = apply(&input.base, &self.accepted).map_err(MergeError::Apply)?;
        }
        rekey_independent_insertions(input, &mut merged);

        // 정의 목록을 candidate 소속 객체와 맞춥니다(SM-1). 객체 존재·위치 결정이 모두 끝난 뒤여야 합니다.
        let mut conflicts = self.conflicts.clone();
        let start = conflicts.len();
        reconcile_member_lists(input, changed_lists, &mut merged, &mut conflicts);
        self.list_conflicts = conflicts.split_off(start);
        Ok(merged)
    }

    /// 새 위반에 관련된 받아들인 변경을 되돌리고 그 단위를 invalid-result conflict로 남깁니다(XIV.10.8).
    /// 하나라도 되돌렸으면 참입니다. `record_unreverted`가 참이면 되돌릴 변경이 없는 위반을 대표 대상의
    /// conflict로 남깁니다.
    fn revert_violations(
        &mut self,
        violations: &BTreeSet<SnapshotViolation>,
        record_unreverted: bool,
    ) -> bool {
        let input = self.input;
        let mut reverted = false;
        for violation in violations {
            let (units, primary) = violation_units(&input.base, violation);
            let (gone, kept): (Vec<SemanticChange>, Vec<SemanticChange>) = self
                .accepted
                .drain(..)
                .partition(|change| units.iter().any(|unit| unit.matches(change)));
            self.accepted = kept;
            if gone.is_empty() {
                if record_unreverted {
                    self.invalid_conflict(primary.0, primary.1);
                }
                continue;
            }
            reverted = true;
            for change in &gone {
                self.invalid_conflict(change.object_id(), Unit::target(change));
            }
        }
        reverted
    }

    fn invalid_conflict(&mut self, object: StableId, target: ConflictTarget) {
        let input = self.input;
        let [base, ours, theirs] = target_states(input, object, &target);
        self.conflicts.push(SemanticConflict {
            kind: ConflictKind::InvalidResult,
            provenance: target_provenance(input, object),
            object,
            target,
            base,
            ours,
            theirs,
        });
    }
}

/// conflict 목록의 결정론적 순서(XIV.10.9, ADR-0081 결정 3).
///
/// `(StableId, 대상)` 오름차순(대상은 객체 < 위치 < 속성 경로)이고 같은 키는 kind code 순입니다. 같은
/// `(StableId, 대상)`은 하나만 남깁니다(code가 작은 종류, 예: 전용 conflict가 있으면 invalid-result는
/// 버림). 해결 모델(XIV.12)은 `(StableId, 대상)`으로 conflict를 찾으므로 이 유일성이 필요합니다. 입력
/// 목록의 순서·중복과 무관하게 같은 결과를 냅니다.
pub fn canonical_conflicts(mut conflicts: Vec<SemanticConflict>) -> Vec<SemanticConflict> {
    conflicts.sort_by(|a, b| (a.object, &a.target, a.kind).cmp(&(b.object, &b.target, b.kind)));
    conflicts.dedup_by(|later, first| later.object == first.object && later.target == first.target);
    conflicts
}

/// 3-way merge를 실행합니다.
///
/// 결과 candidate는 snapshot validator(XIV.5.3)가 세 입력 어디에도 없던 위반을 찾지 못할 때만 clean일 수
/// 있습니다(XIV.10.8). 새 위반이 있으면 관련 변경을 되돌리고 invalid-result conflict를 남깁니다.
pub fn merge(input: &MergeInput) -> Result<MergeResult, MergeError> {
    let ours_diff = diff(&input.base, &input.ours);
    let theirs_diff = diff(&input.base, &input.theirs);
    let ours = group(&ours_diff);
    let theirs = group(&theirs_diff);
    let mut merger = Merger {
        input,
        accepted: Vec::new(),
        conflicts: Vec::new(),
        list_conflicts: Vec::new(),
    };
    let ids: BTreeSet<StableId> = ours.keys().chain(theirs.keys()).copied().collect();
    for id in ids {
        match (ours.get(&id), theirs.get(&id)) {
            (Some(edits), None) | (None, Some(edits)) => merger.accept(edits),
            (Some(a), Some(b)) => merger.merge_object(id, a, b),
            (None, None) => {}
        }
    }
    merger.interface_use_conflicts(&ours_diff, &theirs_diff);
    merger.context_conflicts(&ours_diff, &theirs_diff);

    // 속성 변경은 base에 있는 같은 정체성 객체에서만 나옵니다.
    let changed_lists: BTreeSet<PropertyPath> = ours_diff
        .iter()
        .chain(theirs_diff.iter())
        .filter_map(SemanticChange::property_path)
        .filter(|path| {
            input
                .base
                .get(path.root())
                .is_some_and(|owner| is_member_list_path(owner.kind(), path))
        })
        .cloned()
        .collect();
    let mut merged = merger.settle(&changed_lists)?;

    // 결과 검증(XIV.10.8): 새 위반에 관련된 변경을 되돌립니다. 되돌릴 때마다 받아들인 변경이 줄어 유한
    // 번에 끝나고, 더 되돌릴 것이 없으면 남은 위반을 conflict로 기록합니다.
    let baselines = [&input.base, &input.ours, &input.theirs];
    loop {
        let violations = introduced_violations(&merged, &baselines);
        if violations.is_empty() {
            break;
        }
        if !merger.revert_violations(&violations, false) {
            merger.revert_violations(&violations, true);
            break;
        }
        merged = merger.settle(&changed_lists)?;
    }

    let mut conflicts = merger.conflicts;
    conflicts.extend(merger.list_conflicts);
    let conflicts = canonical_conflicts(conflicts);
    let changes = diff(&input.base, &merged);
    Ok(MergeResult {
        merged,
        changes,
        conflicts,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::tests::{key, node, sid, snapshot, text};
    use stellar_semantic_diff::keys;
    use stellar_semantic_diff::snapshot::SemanticObject;

    /// base: root 10 ─ 11(a) ─ 13(c), 12(b).
    pub(crate) fn base() -> SemanticSnapshot {
        snapshot(vec![
            node(10, None, 0x80, &[(keys::DISPLAY_NAME, text("root"))]),
            node(
                11,
                Some(10),
                0x40,
                &[
                    (keys::DISPLAY_NAME, text("a")),
                    (keys::VISIBLE, SemanticValue::Bool(true)),
                ],
            ),
            node(12, Some(10), 0xC0, &[(keys::DISPLAY_NAME, text("b"))]),
            node(13, Some(11), 0x80, &[(keys::DISPLAY_NAME, text("c"))]),
        ])
    }

    /// snapshot에서 객체 하나를 바꿉니다.
    pub(crate) fn edit(
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

    pub(crate) fn set(
        object: &mut SemanticObject,
        key: stellar_semantic_diff::core_types::property::PropertyKey,
        value: SemanticValue,
    ) {
        object.set_property(object.path(&[key]), value).unwrap();
    }

    pub(crate) fn run(
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

    #[test]
    fn different_objects_merge_without_conflict() {
        let base = base();
        let ours = edit(&base, 11, |o| set(o, keys::DISPLAY_NAME, text("a2")));
        let theirs = edit(&base, 12, |o| set(o, keys::DISPLAY_NAME, text("b2")));
        let result = run(&base, &ours, &theirs);
        assert!(result.is_clean(), "{:?}", result.conflicts);
        let expected = edit(&ours, 12, |o| set(o, keys::DISPLAY_NAME, text("b2")));
        assert_eq!(result.merged, expected);
        assert_eq!(result.changes.len(), 2);
    }

    #[test]
    fn same_object_different_paths_merge_without_conflict() {
        let base = base();
        let ours = edit(&base, 11, |o| {
            set(o, keys::VISIBLE, SemanticValue::Bool(false))
        });
        let theirs = edit(&base, 11, |o| set(o, keys::DISPLAY_NAME, text("renamed")));
        let result = run(&base, &ours, &theirs);
        assert!(result.is_clean(), "{:?}", result.conflicts);
        let merged = result.merged.get(sid(11)).unwrap();
        assert_eq!(
            merged.property(&merged.path(&[keys::VISIBLE])),
            Some(&SemanticValue::Bool(false))
        );
        assert_eq!(
            merged.property(&merged.path(&[keys::DISPLAY_NAME])),
            Some(&text("renamed"))
        );

        // 위치 변경과 속성 변경도 겹치지 않습니다.
        let moved = edit(&base, 11, |o| {
            o.set_position(Some(sid(12)), Some(key(0x80)))
        });
        let result = run(&base, &moved, &theirs);
        assert!(result.is_clean());
        let merged = result.merged.get(sid(11)).unwrap();
        assert_eq!(merged.parent(), Some(sid(12)));
        assert_eq!(
            merged.property(&merged.path(&[keys::DISPLAY_NAME])),
            Some(&text("renamed"))
        );
    }

    #[test]
    fn independent_additions_and_deletions_merge() {
        let base = base();
        let mut ours = base.clone();
        ours.insert(node(20, Some(10), 0x60, &[])).unwrap();
        let mut theirs = base.clone();
        theirs.remove(sid(12));
        theirs.insert(node(21, Some(11), 0xC0, &[])).unwrap();
        let result = run(&base, &ours, &theirs);
        assert!(result.is_clean(), "{:?}", result.conflicts);
        assert!(result.merged.contains(sid(20)));
        assert!(result.merged.contains(sid(21)));
        assert!(!result.merged.contains(sid(12)));
        assert_eq!(result.merged.children(sid(10)), vec![sid(11), sid(20)]);
    }

    #[test]
    fn one_sided_merge_returns_that_side() {
        let base = base();
        let mut changed = edit(&base, 13, |o| {
            o.set_position(Some(sid(12)), Some(key(0x10)))
        });
        changed.remove(sid(11));
        changed
            .insert(node(
                30,
                Some(10),
                0x20,
                &[(keys::LOCKED, SemanticValue::Bool(true))],
            ))
            .unwrap();
        for (ours, theirs) in [(&changed, &base), (&base, &changed)] {
            let result = run(&base, ours, theirs);
            assert!(result.is_clean(), "{:?}", result.conflicts);
            assert_eq!(result.merged, changed);
            assert_eq!(result.changes, diff(&base, &changed));
        }
        // 양쪽 모두 변경 없음.
        let result = run(&base, &base, &base);
        assert!(result.is_clean());
        assert_eq!(result.merged, base);
        assert!(result.changes.is_empty());
    }

    #[test]
    fn overlapping_edits_are_conflicts_not_silent() {
        let base = base();
        let ours = edit(&base, 11, |o| set(o, keys::DISPLAY_NAME, text("x")));
        let theirs = edit(&base, 11, |o| set(o, keys::DISPLAY_NAME, text("y")));
        let result = run(&base, &ours, &theirs);
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.conflicts[0].kind, ConflictKind::PropertyValue);
        // candidate는 base 값을 유지합니다.
        assert_eq!(result.merged, base);
    }

    /// 결정론적 PRNG(SplitMix64).
    pub(crate) struct Rng(u64);

    impl Rng {
        pub(crate) fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }

        pub(crate) fn below(&mut self, n: u64) -> u64 {
            if n == 0 {
                0
            } else {
                self.next() % n
            }
        }
    }

    /// seed로 node tree(parent가 항상 먼저 있는 유효 tree)를 만듭니다.
    pub(crate) fn random_tree(seed: u64, size: u64) -> SemanticSnapshot {
        let mut rng = Rng(seed);
        let mut objects = vec![node(100, None, 0x80, &[(keys::DISPLAY_NAME, text("root"))])];
        for n in 1..size {
            let parent = 100 + rng.below(n);
            let order = 1 + (rng.below(254) as u8);
            objects.push(node(
                100 + n,
                Some(parent),
                order,
                &[(keys::DISPLAY_NAME, text(&format!("n{n}")))],
            ));
        }
        let mut snapshot = SemanticSnapshot::new();
        for object in objects {
            // 같은 parent·같은 key 충돌은 id로 순서가 정해지므로 그대로 둡니다.
            snapshot.insert(object).unwrap();
        }
        snapshot
    }

    #[test]
    fn disjoint_random_edits_merge_without_conflict() {
        for seed in 0..100 {
            let size = 3 + seed % 20;
            let base = random_tree(seed, size);
            let mut rng = Rng(seed ^ 0xFEED);
            let (mut ours, mut theirs) = (base.clone(), base.clone());
            let mut expected = base.clone();
            for n in 1..size {
                // 짝수 node는 ours, 홀수 node는 theirs가 이름 또는 visible을 바꿉니다.
                let id = 100 + n;
                let (key, value) = if rng.below(2) == 0 {
                    (keys::DISPLAY_NAME, text(&format!("s{seed}-{n}")))
                } else {
                    (keys::VISIBLE, SemanticValue::Bool(rng.below(2) == 0))
                };
                let side = if n % 2 == 0 { &mut ours } else { &mut theirs };
                *side = edit(side, id, |o| set(o, key, value.clone()));
                expected = edit(&expected, id, |o| set(o, key, value));
            }
            let result = run(&base, &ours, &theirs);
            assert!(result.is_clean(), "seed {seed}: {:?}", result.conflicts);
            assert_eq!(result.merged, expected, "seed {seed}");
        }
    }

    #[test]
    fn identical_edit_merges_as_one_change() {
        let base = base();
        let same = |s: &SemanticSnapshot| edit(s, 11, |o| set(o, keys::DISPLAY_NAME, text("same")));
        let ours = edit(&same(&base), 12, |o| {
            set(o, keys::LOCKED, SemanticValue::Bool(true))
        });
        let theirs = same(&base);
        let result = run(&base, &ours, &theirs);
        assert!(result.is_clean(), "{:?}", result.conflicts);
        assert_eq!(result.merged, ours);
        let renames: Vec<_> = result
            .changes
            .iter()
            .filter(|c| c.object_id() == sid(11))
            .collect();
        assert_eq!(renames.len(), 1, "같은 변경은 한 번만");

        // 같은 속성 삭제, 같은 위치 이동, 같은 객체 삭제·추가도 합칩니다.
        let both = |s: &SemanticSnapshot| {
            let mut out = edit(s, 11, |o| {
                o.remove_property(&o.path(&[keys::VISIBLE]));
                o.set_position(Some(sid(12)), Some(key(0x22)));
            });
            out.remove(sid(13));
            out.insert(node(
                40,
                Some(10),
                0x10,
                &[(keys::DISPLAY_NAME, text("new"))],
            ))
            .unwrap();
            out
        };
        let result = run(&base, &both(&base), &both(&base));
        assert!(result.is_clean(), "{:?}", result.conflicts);
        assert_eq!(result.merged, both(&base));
    }

    #[test]
    fn same_edits_on_both_sides_property() {
        // merge(base, X, X) == X: 양쪽 같은 무작위 편집은 항상 conflict 0.
        for seed in 0..100 {
            let size = 4 + seed % 16;
            let base = random_tree(seed, size);
            let mut rng = Rng(seed ^ 0xBEEF);
            let mut changed = base.clone();
            for n in 1..size {
                if rng.below(3) == 0 {
                    changed = edit(&changed, 100 + n, |o| set(o, keys::DISPLAY_NAME, text("e")));
                }
            }
            if rng.below(2) == 0 {
                changed.insert(node(900, Some(100), 0x7F, &[])).unwrap();
            }
            let result = run(&base, &changed, &changed);
            assert!(result.is_clean(), "seed {seed}: {:?}", result.conflicts);
            assert_eq!(result.merged, changed, "seed {seed}");
        }
    }

    #[test]
    fn different_values_conflict_preserves_all_three_values() {
        // 11.name: base "a", ours "x", theirs "y". 11.visible: ours만 바꿈(자동 병합).
        // 12.name: ours 삭제, theirs "z". 10.name: theirs만 바꿈.
        let base = base();
        let mut ours = edit(&base, 11, |o| {
            set(o, keys::DISPLAY_NAME, text("x"));
            set(o, keys::VISIBLE, SemanticValue::Bool(false));
        });
        ours = edit(&ours, 12, |o| {
            o.remove_property(&o.path(&[keys::DISPLAY_NAME]));
        });
        let mut theirs = edit(&base, 11, |o| set(o, keys::DISPLAY_NAME, text("y")));
        theirs = edit(&theirs, 12, |o| set(o, keys::DISPLAY_NAME, text("z")));
        theirs = edit(&theirs, 10, |o| set(o, keys::DISPLAY_NAME, text("top")));
        let result = run(&base, &ours, &theirs);

        assert_eq!(result.conflicts.len(), 2, "{:?}", result.conflicts);
        let name = |id| PropertyPath::with_keys(sid(id), vec![keys::DISPLAY_NAME]);
        let first = &result.conflicts[0];
        assert_eq!(first.kind, ConflictKind::PropertyValue);
        assert_eq!(
            (first.object, &first.target),
            (sid(11), &ConflictTarget::Property(name(11)))
        );
        let values = first.property_values().unwrap();
        assert_eq!(
            (values.base, values.ours, values.theirs),
            (Some(&text("a")), Some(&text("x")), Some(&text("y")))
        );
        let second = result.conflicts[1].property_values().unwrap();
        assert_eq!(
            (second.base, second.ours, second.theirs),
            (Some(&text("b")), None, Some(&text("z")))
        );
        assert_eq!(
            result.conflicts[1].provenance,
            object_provenance(base.get(sid(12)).unwrap())
        );

        // candidate: conflict 경로는 base 값, 나머지는 병합.
        let merged = |id, key| {
            let object = result.merged.get(sid(id)).unwrap();
            object.property(&object.path(&[key])).cloned()
        };
        assert_eq!(merged(11, keys::DISPLAY_NAME), Some(text("a")));
        assert_eq!(merged(12, keys::DISPLAY_NAME), Some(text("b")));
        assert_eq!(merged(11, keys::VISIBLE), Some(SemanticValue::Bool(false)));
        assert_eq!(merged(10, keys::DISPLAY_NAME), Some(text("top")));

        // 결과 bytes round-trip 후에도 conflict 기록이 같습니다.
        let decoded = MergeResult::from_canonical_bytes(&result.to_canonical_bytes()).unwrap();
        assert_eq!(decoded, result);
    }

    /// conflict matrix 한 줄: (이름, ours 편집, theirs 편집, 기대 conflict 종류 목록).
    type Action = fn(&SemanticSnapshot) -> SemanticSnapshot;

    fn delete_11(s: &SemanticSnapshot) -> SemanticSnapshot {
        let mut out = s.clone();
        out.remove(sid(11));
        out.remove(sid(13));
        out
    }

    fn rename_11(s: &SemanticSnapshot) -> SemanticSnapshot {
        edit(s, 11, |o| set(o, keys::DISPLAY_NAME, text("renamed")))
    }

    fn move_11(s: &SemanticSnapshot) -> SemanticSnapshot {
        edit(s, 11, |o| o.set_position(Some(sid(12)), Some(key(0x80))))
    }

    fn reorder_11(s: &SemanticSnapshot) -> SemanticSnapshot {
        edit(s, 11, |o| o.set_position(Some(sid(10)), Some(key(0xE0))))
    }

    fn move_and_rename_11(s: &SemanticSnapshot) -> SemanticSnapshot {
        rename_11(&move_11(s))
    }

    fn name_as_number_11(s: &SemanticSnapshot) -> SemanticSnapshot {
        edit(s, 11, |o| {
            set(o, keys::DISPLAY_NAME, SemanticValue::Uint(7))
        })
    }

    fn name_as_bool_11(s: &SemanticSnapshot) -> SemanticSnapshot {
        edit(s, 11, |o| {
            set(o, keys::DISPLAY_NAME, SemanticValue::Bool(true))
        })
    }

    fn add_child_under_11(s: &SemanticSnapshot) -> SemanticSnapshot {
        let mut out = s.clone();
        out.insert(node(50, Some(11), 0xF0, &[])).unwrap();
        out
    }

    fn move_12_under_11(s: &SemanticSnapshot) -> SemanticSnapshot {
        edit(s, 12, |o| o.set_position(Some(sid(11)), Some(key(0xF0))))
    }

    fn replace_11_as_token(s: &SemanticSnapshot) -> SemanticSnapshot {
        let mut out = delete_11(s);
        out.insert(SemanticObject::new(
            sid(11),
            stellar_semantic_diff::snapshot::ObjectKind::Token,
            stellar_semantic_diff::snapshot::Origin::Project,
        ))
        .unwrap();
        out
    }

    #[test]
    fn conflict_matrix() {
        use ConflictKind::*;
        let matrix: [(&str, Action, Action, &[ConflictKind]); 14] = [
            ("delete vs modify", delete_11, rename_11, &[DeleteVsModify]),
            ("modify vs delete", rename_11, delete_11, &[DeleteVsModify]),
            ("delete vs move", delete_11, move_11, &[DeleteVsMove]),
            ("reorder vs delete", reorder_11, delete_11, &[DeleteVsMove]),
            (
                "delete vs move+modify",
                delete_11,
                move_and_rename_11,
                &[DeleteVsModify],
            ),
            (
                "type vs value",
                name_as_number_11,
                rename_11,
                &[TypeChangeVsValueChange],
            ),
            (
                "value vs type",
                rename_11,
                name_as_number_11,
                &[TypeChangeVsValueChange],
            ),
            (
                "type vs other type",
                name_as_number_11,
                name_as_bool_11,
                &[PropertyValue],
            ),
            (
                "value vs value",
                rename_11,
                |s| edit(s, 11, |o| set(o, keys::DISPLAY_NAME, text("other"))),
                &[PropertyValue],
            ),
            (
                "delete parent vs add child",
                delete_11,
                add_child_under_11,
                &[DeleteVsModify],
            ),
            (
                "delete parent vs move child in",
                delete_11,
                move_12_under_11,
                &[DeleteVsMove],
            ),
            (
                "replace vs modify",
                replace_11_as_token,
                rename_11,
                &[InconsistentDefinition],
            ),
            ("delete vs delete", delete_11, delete_11, &[]),
            (
                "move vs move elsewhere",
                move_11,
                reorder_11,
                &[ConflictingReorder],
            ),
        ];
        let base = base();
        let mut produced = BTreeSet::new();
        for (name, ours, theirs, expected) in matrix {
            let result = run(&base, &ours(&base), &theirs(&base));
            let kinds: Vec<_> = result.conflicts.iter().map(|c| c.kind).collect();
            assert_eq!(kinds, expected, "{name}: {:?}", result.conflicts);
            for conflict in &result.conflicts {
                assert_eq!(conflict.object, sid(11), "{name}");
            }
            produced.extend(kinds);
        }
        // matrix가 XIV.10.4~XIV.10.6 conflict 종류를 서로 다른 kind로 냅니다. 전용 종류(XIV.10.7~)는
        // `tests/dedicated_conflicts.rs` corpus가 다룹니다.
        assert_eq!(
            produced.into_iter().collect::<Vec<_>>(),
            ConflictKind::ALL[..6].to_vec()
        );
    }

    #[test]
    fn delete_conflict_keeps_base_object_and_records_sides() {
        let base = base();
        let result = run(&base, &delete_11(&base), &move_11(&base));
        let conflict = &result.conflicts[0];
        assert_eq!(conflict.target, ConflictTarget::Object);
        assert_eq!(
            conflict.base,
            SideState::Object(Box::new(base.get(sid(11)).unwrap().clone()))
        );
        assert_eq!(conflict.ours, SideState::Missing);
        assert_eq!(
            conflict.theirs,
            SideState::Object(Box::new(move_11(&base).get(sid(11)).unwrap().clone()))
        );
        assert_eq!(result.merged.get(sid(11)), base.get(sid(11)));
    }

    #[test]
    fn order_key_merge_fixtures_pass() {
        use stellar_semantic_diff::core_types::order_key_merge_fixtures::fixtures;
        for fixture in fixtures() {
            fixture.validate().unwrap();
            let child =
                |c: &stellar_semantic_diff::core_types::order_key_merge_fixtures::OrderedChild| {
                    SemanticObject::new(
                        c.id,
                        stellar_semantic_diff::snapshot::ObjectKind::Node(
                            stellar_semantic_diff::snapshot::NodeKind::Fragment,
                        ),
                        stellar_semantic_diff::snapshot::Origin::Document(sid(1)),
                    )
                    .with_position(Some(sid(10)), Some(c.key.clone()))
                };
            let mut base = snapshot(vec![node(10, None, 0x80, &[])]);
            for c in &fixture.base {
                base.insert(child(c)).unwrap();
            }
            let (mut ours, mut theirs) = (base.clone(), base.clone());
            for c in &fixture.branch_a_inserts {
                ours.insert(child(c)).unwrap();
            }
            for c in &fixture.branch_b_inserts {
                theirs.insert(child(c)).unwrap();
            }
            let result = run(&base, &ours, &theirs);
            assert!(
                result.is_clean(),
                "{}: {:?}",
                fixture.name,
                result.conflicts
            );
            assert_eq!(
                result.merged.children(sid(10)),
                fixture.expected_merged,
                "{}",
                fixture.name
            );
            // 형제 key 유일(독립 삽입이 같은 key를 받아도).
            let keys: BTreeSet<_> = result
                .merged
                .children(sid(10))
                .into_iter()
                .map(|id| result.merged.get(id).unwrap().order_key().cloned())
                .collect();
            assert_eq!(
                keys.len(),
                fixture.expected_merged.len(),
                "{}",
                fixture.name
            );
            // base 자식은 key를 바꾸지 않습니다.
            for c in &fixture.base {
                assert_eq!(result.merged.get(c.id).unwrap().order_key(), Some(&c.key));
            }
        }
    }

    #[test]
    fn conflicting_reorder_of_same_child() {
        let base = base();
        // 같은 parent 안에서 서로 다른 key.
        let ours = edit(&base, 12, |o| {
            o.set_position(Some(sid(10)), Some(key(0x10)))
        });
        let theirs = edit(&base, 12, |o| {
            o.set_position(Some(sid(10)), Some(key(0xF0)))
        });
        let result = run(&base, &ours, &theirs);
        assert_eq!(result.conflicts.len(), 1);
        let conflict = &result.conflicts[0];
        assert_eq!(
            (conflict.kind, &conflict.target),
            (ConflictKind::ConflictingReorder, &ConflictTarget::Position)
        );
        assert_eq!(
            conflict.ours,
            SideState::Position(Position {
                parent: Some(sid(10)),
                order_key: Some(key(0x10))
            })
        );
        assert_eq!(
            result.merged.get(sid(12)),
            base.get(sid(12)),
            "candidate는 base 위치"
        );
        // 같은 재정렬은 conflict가 아닙니다.
        assert!(run(&base, &ours, &ours).is_clean());
        // 다른 child의 재정렬은 독립입니다.
        let other = edit(&base, 11, |o| {
            o.set_position(Some(sid(10)), Some(key(0xF8)))
        });
        assert!(run(&base, &ours, &other).is_clean());
    }

    #[test]
    fn independent_moves_forming_a_cycle_are_conflicts() {
        // ours: 12를 11 아래로. theirs: 11을 12 아래로. 각자는 유효하지만 합치면 cycle.
        let base = base();
        let ours = edit(&base, 12, |o| {
            o.set_position(Some(sid(11)), Some(key(0x20)))
        });
        let theirs = edit(&base, 11, |o| {
            o.set_position(Some(sid(12)), Some(key(0x20)))
        });
        let result = run(&base, &ours, &theirs);
        let kinds: Vec<_> = result
            .conflicts
            .iter()
            .map(|c| (c.kind, c.object))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (ConflictKind::ConflictingReorder, sid(11)),
                (ConflictKind::ConflictingReorder, sid(12)),
            ]
        );
        assert_eq!(result.merged, base);
        // cycle이 없으면 독립 이동은 그대로 병합됩니다.
        let theirs = edit(&base, 13, |o| {
            o.set_position(Some(sid(12)), Some(key(0x20)))
        });
        assert!(run(&base, &ours, &theirs).is_clean());
    }

    #[test]
    fn child_added_under_deleted_parent_is_conflict() {
        // ours: frame 11과 자식 13 삭제. theirs: 11 아래 새 자식 22 추가.
        let base = base();
        let mut ours = base.clone();
        ours.remove(sid(11));
        ours.remove(sid(13));
        let mut theirs = base.clone();
        theirs.insert(node(22, Some(11), 0xE0, &[])).unwrap();
        let result = run(&base, &ours, &theirs);
        assert_eq!(result.conflicts.len(), 1, "{:?}", result.conflicts);
        let conflict = &result.conflicts[0];
        assert_eq!(
            (conflict.kind, conflict.object),
            (ConflictKind::DeleteVsModify, sid(11))
        );
        assert_eq!(conflict.ours, SideState::Missing);
        assert!(matches!(conflict.theirs, SideState::Object(_)));
        // candidate는 parent를 유지해 고아가 없습니다.
        for object in result.merged.objects() {
            if let Some(parent) = object.parent() {
                assert!(result.merged.contains(parent), "{:?} 고아", object.id());
            }
        }
    }

    // --- 정의 목록(SM-1) ---

    use stellar_semantic_diff::validate::{validate_snapshot, SnapshotViolation};

    /// 문서 1의 정의 snapshot: `(페이지, [View])` 순서대로 페이지 목록·View 목록을 만들고, View `v`마다
    /// root Frame node `v + 100`을 둡니다.
    fn pages(pages: &[(u64, &[u64])]) -> SemanticSnapshot {
        let document = sid(1);
        let origin = Origin::Document(document);
        let mut doc = SemanticObject::new(document, ObjectKind::Document, origin);
        let page_ids = pages.iter().map(|(p, _)| sid(*p)).collect();
        set(&mut doc, keys::DOCUMENT_PAGES, SemanticValue::Ids(page_ids));
        let mut objects = vec![doc];
        for (page, views) in pages {
            let mut object = SemanticObject::new(sid(*page), ObjectKind::Page, origin)
                .with_position(Some(document), None);
            set(
                &mut object,
                keys::DISPLAY_NAME,
                text(&format!("page {page}")),
            );
            set(
                &mut object,
                keys::PAGE_VIEWS,
                SemanticValue::Ids(sids(views)),
            );
            objects.push(object);
            for view in *views {
                let mut object = SemanticObject::new(sid(*view), ObjectKind::View, origin)
                    .with_position(Some(sid(*page)), None);
                set(
                    &mut object,
                    keys::VIEW_ROOT_FRAME,
                    SemanticValue::Id(sid(view + 100)),
                );
                objects.push(object);
                objects.push(node(view + 100, None, 0x80, &[]));
            }
        }
        snapshot(objects)
    }

    /// 컴포넌트 300(문서 1)과 Slot 목록.
    fn component(slots: &[u64]) -> SemanticSnapshot {
        let origin = Origin::Component {
            document: sid(1),
            component: sid(300),
        };
        let mut component = SemanticObject::new(sid(300), ObjectKind::Component, origin);
        set(
            &mut component,
            keys::COMPONENT_SLOTS,
            SemanticValue::Ids(sids(slots)),
        );
        let mut objects = vec![component];
        for slot in slots {
            objects.push(SemanticObject::new(
                sid(*slot),
                ObjectKind::ComponentSlot,
                origin,
            ));
        }
        snapshot(objects)
    }

    fn sids(ns: &[u64]) -> Vec<StableId> {
        ns.iter().copied().map(sid).collect()
    }

    /// owner의 정의 목록 값.
    fn list_of(snapshot: &SemanticSnapshot, owner: u64, key: PropertyKey) -> Vec<StableId> {
        let object = snapshot.get(sid(owner)).unwrap();
        match object.property(&object.path(&[key])) {
            Some(SemanticValue::Ids(ids)) => ids.clone(),
            other => panic!("{owner}의 목록 없음: {other:?}"),
        }
    }

    /// 모든 정의 목록이 중복 없이 정확히 candidate 소속 객체를 담는지(validator XIV.5.3 소속 규칙 +
    /// 목록 원소가 그 owner의 소속 객체인지).
    fn assert_lists_coherent(snapshot: &SemanticSnapshot, label: &str) {
        for object in snapshot.objects() {
            let key = match object.kind() {
                ObjectKind::Document => keys::DOCUMENT_PAGES,
                ObjectKind::Page => keys::PAGE_VIEWS,
                ObjectKind::Component => keys::COMPONENT_SLOTS,
                _ => continue,
            };
            let listed = match object.property(&object.path(&[key])) {
                Some(SemanticValue::Ids(ids)) => ids.clone(),
                None => Vec::new(),
                other => panic!("{label}: {:?} 목록 값 {other:?}", object.id()),
            };
            let unique: BTreeSet<StableId> = listed.iter().copied().collect();
            assert_eq!(
                unique.len(),
                listed.len(),
                "{label}: {:?} 목록 중복",
                object.id()
            );
            for id in listed {
                let member = snapshot
                    .get(id)
                    .unwrap_or_else(|| panic!("{label}: 목록 원소 {id:?} 없음"));
                let belongs = match (member.kind(), member.origin()) {
                    (ObjectKind::Page, _) if key == keys::DOCUMENT_PAGES => {
                        member.parent() == Some(object.id())
                    }
                    (ObjectKind::View, _) if key == keys::PAGE_VIEWS => {
                        member.parent() == Some(object.id())
                    }
                    (ObjectKind::ComponentSlot, origin) => {
                        key == keys::COMPONENT_SLOTS && origin.component() == Some(object.id())
                    }
                    _ => false,
                };
                assert!(belongs, "{label}: {id:?}는 {:?} 소속이 아님", object.id());
            }
        }
        let unlisted: Vec<_> = validate_snapshot(snapshot)
            .into_iter()
            .filter(|v| matches!(v, SnapshotViolation::UnlistedMember { .. }))
            .collect();
        assert!(
            unlisted.is_empty(),
            "{label}: 목록에 없는 소속 객체 {unlisted:?}"
        );
    }

    #[test]
    fn independent_page_and_view_insertions_merge_without_conflict() {
        // ours: 새 페이지 40 + 페이지 2에 View 43. theirs: 새 페이지 50 + 페이지 2에 View 53.
        let base = pages(&[(2, &[3])]);
        let ours = pages(&[(2, &[3, 43]), (40, &[41])]);
        let theirs = pages(&[(2, &[3, 53]), (50, &[51])]);
        let result = run(&base, &ours, &theirs);
        assert!(result.is_clean(), "{:?}", result.conflicts);
        assert_lists_coherent(&result.merged, "candidate");
        assert!(validate_snapshot(&result.merged).is_empty());
        // 같은 끝 자리 삽입은 첫 StableId 순서.
        assert_eq!(
            result.merged,
            pages(&[(2, &[3, 43, 53]), (40, &[41]), (50, &[51])])
        );
        assert_eq!(
            run(&base, &theirs, &ours).merged,
            result.merged,
            "좌우 교환"
        );
        assert_eq!(apply(&base, &result.changes).as_ref(), Ok(&result.merged));
    }

    #[test]
    fn member_list_insertions_follow_their_anchor_and_deletions_apply() {
        // ours: 페이지 2 뒤에 40·42 연속 삽입. theirs: 페이지 2 뒤에 30 삽입 + 페이지 6 삭제.
        let base = pages(&[(2, &[3]), (4, &[5]), (6, &[7])]);
        let ours = pages(&[(2, &[3]), (40, &[41]), (42, &[43]), (4, &[5]), (6, &[7])]);
        let theirs = pages(&[(2, &[3]), (30, &[31]), (4, &[5])]);
        let result = run(&base, &ours, &theirs);
        assert!(result.is_clean(), "{:?}", result.conflicts);
        // 같은 자리 묶음은 첫 StableId가 작은 쪽(30)이 먼저이고, 묶음 안 순서(40, 42)는 유지.
        let expected = pages(&[(2, &[3]), (30, &[31]), (40, &[41]), (42, &[43]), (4, &[5])]);
        assert_eq!(result.merged, expected);
        assert_eq!(run(&base, &theirs, &ours).merged, expected, "좌우 교환");
        assert!(validate_snapshot(&result.merged).is_empty());
    }

    #[test]
    fn one_side_reorder_keeps_other_side_insertion() {
        let base = pages(&[(2, &[3]), (4, &[5]), (6, &[7])]);
        // ours: 6을 맨 앞으로. theirs: 4 뒤에 50 삽입.
        let ours = pages(&[(6, &[7]), (2, &[3]), (4, &[5])]);
        let theirs = pages(&[(2, &[3]), (4, &[5]), (50, &[51]), (6, &[7])]);
        let result = run(&base, &ours, &theirs);
        assert!(result.is_clean(), "{:?}", result.conflicts);
        let expected = pages(&[(6, &[7]), (2, &[3]), (4, &[5]), (50, &[51])]);
        assert_eq!(result.merged, expected, "삽입은 앞 객체(4)를 따라감");
        assert_eq!(run(&base, &theirs, &ours).merged, expected, "좌우 교환");
        // 양쪽 같은 재정렬은 한 변경.
        let same = run(&base, &ours, &ours);
        assert!(same.is_clean());
        assert_eq!(same.merged, ours);
    }

    #[test]
    fn conflicting_list_reorders_are_conflicts_and_candidate_stays_coherent() {
        let base = pages(&[(2, &[3]), (4, &[5]), (6, &[7])]);
        // ours: 4를 맨 앞으로. theirs: 4를 맨 뒤로 + 맨 앞에 50 삽입.
        let ours = pages(&[(4, &[5]), (2, &[3]), (6, &[7])]);
        let theirs = pages(&[(50, &[51]), (2, &[3]), (6, &[7]), (4, &[5])]);
        let input = MergeInput {
            base: base.clone(),
            ours: ours.clone(),
            theirs: theirs.clone(),
        };
        let result = merge(&input).unwrap();
        assert_eq!(result.conflicts.len(), 1, "{:?}", result.conflicts);
        let conflict = &result.conflicts[0];
        let path = PropertyPath::with_keys(sid(1), vec![keys::DOCUMENT_PAGES]);
        assert_eq!(
            (conflict.kind, conflict.object, &conflict.target),
            (
                ConflictKind::ConflictingReorder,
                sid(1),
                &ConflictTarget::Property(path.clone())
            )
        );
        let value = |s: &[u64]| SideState::Value(Some(SemanticValue::Ids(sids(s))));
        assert_eq!(conflict.base, value(&[2, 4, 6]));
        assert_eq!(conflict.ours, value(&[4, 2, 6]));
        assert_eq!(conflict.theirs, value(&[50, 2, 6, 4]));
        // candidate: 남은 객체는 base 순서, theirs 삽입(50)은 받아들여 목록과 객체가 맞음.
        assert_eq!(
            result.merged,
            pages(&[(50, &[51]), (2, &[3]), (4, &[5]), (6, &[7])])
        );
        assert_lists_coherent(&result.merged, "candidate");
        assert!(validate_snapshot(&result.merged).is_empty());
        // 좌우 교환: 같은 candidate, 좌우가 바뀐 conflict.
        let swapped = run(&base, &theirs, &ours);
        assert_eq!(swapped.merged, result.merged);
        assert_eq!(swapped.conflicts.len(), 1);
        assert_eq!(
            (&swapped.conflicts[0].ours, &swapped.conflicts[0].theirs),
            (&conflict.theirs, &conflict.ours)
        );
        let bytes = result.to_canonical_bytes();
        assert_eq!(MergeResult::from_canonical_bytes(&bytes).unwrap(), result);
    }

    #[test]
    fn deleted_page_versus_renamed_page_keeps_lists_coherent() {
        // ours: 페이지 4(와 View 5·root 105) 삭제. theirs: 페이지 4 이름 변경.
        let base = pages(&[(2, &[3]), (4, &[5])]);
        let ours = pages(&[(2, &[3])]);
        let theirs = edit(&base, 4, |o| set(o, keys::DISPLAY_NAME, text("renamed")));
        let result = run(&base, &ours, &theirs);
        let kinds: Vec<_> = result
            .conflicts
            .iter()
            .map(|c| (c.kind, c.object))
            .collect();
        assert_eq!(kinds, vec![(ConflictKind::DeleteVsModify, sid(4))]);
        // 페이지 4는 base 상태로 남고 문서 목록에도 남습니다. ours가 지운 View 5는 지워졌으므로 View 목록에서도
        // 빠집니다.
        assert_eq!(
            list_of(&result.merged, 1, keys::DOCUMENT_PAGES),
            sids(&[2, 4])
        );
        assert_eq!(list_of(&result.merged, 4, keys::PAGE_VIEWS), sids(&[]));
        assert!(!result.merged.contains(sid(5)));
        assert_lists_coherent(&result.merged, "candidate");
        assert!(validate_snapshot(&result.merged).is_empty());
        assert_eq!(
            run(&base, &theirs, &ours).merged,
            result.merged,
            "좌우 교환"
        );
    }

    #[test]
    fn independent_slot_insertions_merge_without_conflict() {
        let base = component(&[310]);
        let ours = component(&[310, 311]);
        let theirs = component(&[310, 312]);
        let result = run(&base, &ours, &theirs);
        assert!(result.is_clean(), "{:?}", result.conflicts);
        assert_eq!(result.merged, component(&[310, 311, 312]));
        assert_lists_coherent(&result.merged, "candidate");
        let list = PropertyPath::with_keys(sid(300), vec![keys::COMPONENT_SLOTS]);
        let kinds: Vec<_> = result
            .changes
            .iter()
            .filter(|c| c.property_path() == Some(&list))
            .map(SemanticChange::kind)
            .collect();
        assert_eq!(kinds, vec![SemanticChangeKind::ComponentInterfaceChange]);
    }

    /// 무작위 목록 편집 모델: `(페이지, [View])`.
    type PageModel = Vec<(u64, Vec<u64>)>;

    /// 페이지 삭제·삽입, View 삽입·삭제, 페이지·View 재정렬을 1~3번 적용합니다. 재정렬한 목록 owner를
    /// `reordered`에 기록합니다(페이지 목록 = 문서 1).
    fn mutate_pages(
        model: &mut PageModel,
        rng: &mut Rng,
        (mut next_page, mut next_view): (u64, u64),
        reordered: &mut BTreeSet<u64>,
    ) {
        for _ in 0..1 + rng.below(3) {
            let len = model.len() as u64;
            match rng.below(6) {
                0 if len > 1 => {
                    model.remove(rng.below(len) as usize);
                }
                1 => {
                    model.insert(
                        rng.below(len + 1) as usize,
                        (next_page, vec![next_page + 1]),
                    );
                    next_page += 10;
                }
                2 if len > 0 => {
                    let views = &mut model[rng.below(len) as usize].1;
                    views.insert(rng.below(views.len() as u64 + 1) as usize, next_view);
                    next_view += 1;
                }
                3 if len > 0 => {
                    let views = &mut model[rng.below(len) as usize].1;
                    if views.len() > 1 {
                        views.remove(rng.below(views.len() as u64) as usize);
                    }
                }
                4 if len > 1 => {
                    model.rotate_left(1 + rng.below(len - 1) as usize);
                    reordered.insert(1);
                }
                5 if len > 0 => {
                    let (page, views) = &mut model[rng.below(len) as usize];
                    if views.len() > 1 {
                        views.rotate_left(1);
                        reordered.insert(*page);
                    }
                }
                _ => {}
            }
        }
    }

    fn build_pages(model: &PageModel) -> SemanticSnapshot {
        let pairs: Vec<(u64, &[u64])> = model.iter().map(|(p, v)| (*p, v.as_slice())).collect();
        pages(&pairs)
    }

    #[test]
    fn random_member_list_edits_stay_coherent() {
        // 범위 확인: 목록 conflict가 난 seed 수, 양쪽이 같은 목록을 바꿨지만 그 목록 conflict가 없는 seed 수.
        let (mut list_conflicts, mut merged_lists) = (0, 0);
        for seed in 0..300 {
            let mut rng = Rng(seed ^ 0x5151);
            let base_model: PageModel = (1..=2 + rng.below(4))
                .map(|k| (10 * k, (1..=1 + rng.below(3)).map(|v| 10 * k + v).collect()))
                .collect();
            let (mut ours_model, mut theirs_model) = (base_model.clone(), base_model.clone());
            let (mut ours_reordered, mut theirs_reordered) = (BTreeSet::new(), BTreeSet::new());
            mutate_pages(&mut ours_model, &mut rng, (1000, 3000), &mut ours_reordered);
            mutate_pages(
                &mut theirs_model,
                &mut rng,
                (2000, 4000),
                &mut theirs_reordered,
            );
            let (base, ours, theirs) = (
                build_pages(&base_model),
                build_pages(&ours_model),
                build_pages(&theirs_model),
            );
            let label = format!("seed {seed}");
            let result = run(&base, &ours, &theirs);
            assert_lists_coherent(&result.merged, &label);
            assert_eq!(
                run(&base, &theirs, &ours).merged,
                result.merged,
                "{label}: 좌우 교환"
            );
            let lists_changed_by = |side: &SemanticSnapshot| -> BTreeSet<PropertyPath> {
                diff(&base, side)
                    .iter()
                    .filter_map(SemanticChange::property_path)
                    .filter(|p| {
                        p.keys()
                            .first()
                            .is_some_and(|k| [keys::DOCUMENT_PAGES, keys::PAGE_VIEWS].contains(k))
                    })
                    .cloned()
                    .collect()
            };
            let both: BTreeSet<PropertyPath> = lists_changed_by(&ours)
                .intersection(&lists_changed_by(&theirs))
                .cloned()
                .collect();
            let conflicted: BTreeSet<PropertyPath> = result
                .conflicts
                .iter()
                .filter_map(|c| match &c.target {
                    ConflictTarget::Property(path) => Some(path.clone()),
                    _ => None,
                })
                .collect();
            list_conflicts += usize::from(!conflicted.is_empty());
            merged_lists += usize::from(both.difference(&conflicted).next().is_some());
            // 목록 conflict는 양쪽이 같은 목록을 재정렬했을 때만.
            for conflict in &result.conflicts {
                if let ConflictTarget::Property(path) = &conflict.target {
                    assert_eq!(conflict.kind, ConflictKind::ConflictingReorder, "{label}");
                    let owner = base_model
                        .iter()
                        .map(|(p, _)| *p)
                        .chain([1])
                        .find(|n| sid(*n) == path.root())
                        .unwrap_or_else(|| panic!("{label}: base에 없는 목록 owner"));
                    assert!(
                        ours_reordered.contains(&owner) && theirs_reordered.contains(&owner),
                        "{label}: 재정렬 없는 목록 conflict {conflict:?}"
                    );
                }
            }
            // 한쪽만 바꾸면 그쪽 그대로, 양쪽 같으면 그대로(conflict 없음).
            for side in [&ours, &theirs] {
                assert_eq!(run(&base, side, &base).merged, *side, "{label}");
                let same = run(&base, side, side);
                assert!(same.is_clean(), "{label}: {:?}", same.conflicts);
                assert_eq!(same.merged, *side, "{label}");
            }
        }
        assert!(list_conflicts > 0, "목록 conflict 사례 없음");
        assert!(merged_lists > 0, "양쪽 목록 변경 병합 사례 없음");
    }

    #[test]
    fn member_order_merge_places_every_member_once() {
        // 목록이 소속 객체와 어긋난 입력(목록에만 있는 id, 중복, 어느 목록에도 없는 소속 객체)도 결과는
        // 정확히 소속 객체 집합입니다.
        let members: BTreeSet<StableId> = sids(&[2, 4, 6, 8, 9]).into_iter().collect();
        let (list, conflict) = merge_member_order(
            &sids(&[2, 4, 6, 7]),
            &sids(&[2, 2, 4, 99]),
            &sids(&[4, 2, 8]),
            &members,
            false,
        );
        assert!(!conflict, "ours는 base 순서, theirs만 재정렬");
        // spine {2, 4}는 theirs 순서, theirs 8은 2 뒤, base에만 남은 6은 base 위치(4 뒤), 9는 맨 뒤.
        assert_eq!(list, sids(&[4, 6, 2, 8, 9]));
        let (kept, conflict) = merge_member_order(
            &sids(&[2, 4]),
            &sids(&[4, 2]),
            &sids(&[4, 2]),
            &members,
            true,
        );
        assert!(!conflict);
        assert_eq!(
            kept,
            sids(&[2, 4, 6, 8, 9]),
            "owner 객체 conflict면 base 순서"
        );
    }

    /// 명시 layout longhand(ADR-0086)는 longhand마다 속성 하나라 서로 다른 longhand 편집은 합쳐지고, 같은 longhand를
    /// 다르게 바꾸면 그 속성 하나만 conflict입니다. 한쪽의 지우기(미설정)도 속성 제거로 합쳐집니다.
    #[test]
    fn layout_longhands_merge_per_longhand() {
        use keys::{layout, layout_container};
        let base = edit(&base(), 11, |o| {
            set(o, layout_container::DISPLAY, text("flex"));
            set(o, layout_container::COLUMN_GAP, text("4px"));
            set(o, layout::FLEX_GROW, text("1"));
        });
        let ours = edit(&base, 11, |o| {
            set(o, layout_container::COLUMN_GAP, text("8px"));
        });
        let theirs = edit(&base, 11, |o| {
            set(o, layout_container::JUSTIFY_CONTENT, text("center"));
            o.remove_property(&o.path(&[layout::FLEX_GROW])).unwrap();
        });
        let result = run(&base, &ours, &theirs);
        assert!(result.is_clean(), "{:?}", result.conflicts);
        let merged = result.merged.get(sid(11)).unwrap();
        let get = |key| merged.property(&merged.path(&[key]));
        assert_eq!(get(layout_container::DISPLAY), Some(&text("flex")));
        assert_eq!(get(layout_container::COLUMN_GAP), Some(&text("8px")));
        assert_eq!(
            get(layout_container::JUSTIFY_CONTENT),
            Some(&text("center"))
        );
        assert_eq!(get(layout::FLEX_GROW), None);

        let ours = edit(&base, 11, |o| {
            set(o, layout_container::DISPLAY, text("grid"))
        });
        let theirs = edit(&base, 11, |o| {
            set(o, layout_container::DISPLAY, text("block"))
        });
        let result = run(&base, &ours, &theirs);
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.conflicts[0].kind, ConflictKind::PropertyValue);
        assert_eq!(
            result.conflicts[0].target,
            ConflictTarget::Property(PropertyPath::with_keys(
                sid(11),
                vec![layout_container::DISPLAY]
            ))
        );
        let merged = result.merged.get(sid(11)).unwrap();
        assert_eq!(
            merged.property(&merged.path(&[layout_container::DISPLAY])),
            Some(&text("flex"))
        );
    }
}
