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
//! **구조 정합**: 한쪽이 삭제한 parent 아래에 다른 쪽이 객체를 추가·이동해 candidate에 parent
//! 없는 객체가 생기면, 그 parent 삭제를 받아들이지 않고 delete-vs-modify conflict로 바꿉니다
//! (입력 자체에 이미 있던 고아는 건드리지 않습니다).

use std::collections::{BTreeMap, BTreeSet};

use stellar_semantic_diff::apply::apply;
use stellar_semantic_diff::change::{Position, SemanticChange, SemanticChangeKind};
use stellar_semantic_diff::core_types::ids::StableId;
use stellar_semantic_diff::core_types::property::PropertyPath;
use stellar_semantic_diff::diff::{diff, object_provenance, SemanticDiff};
use stellar_semantic_diff::snapshot::SemanticSnapshot;
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

/// merge 진행 상태.
struct Merger<'a> {
    input: &'a MergeInput,
    accepted: Vec<SemanticChange>,
    conflicts: Vec<SemanticConflict>,
}

impl<'a> Merger<'a> {
    fn accept(&mut self, edits: &Edits<'_>) {
        self.accepted.extend(edits.all().cloned());
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
        match (ours.position, theirs.position) {
            (Some(change), None) | (None, Some(change)) => self.accepted.push(change.clone()),
            (Some(change), Some(other)) if same_position(change, other) => {
                self.accepted.push(change.clone())
            }
            (Some(change), Some(_)) => self.conflicts.push(SemanticConflict {
                kind: ConflictKind::ConflictingReorder,
                provenance: *change.provenance(),
                object: id,
                target: ConflictTarget::Position,
                base: position_state(&input.base, id),
                ours: position_state(&input.ours, id),
                theirs: position_state(&input.theirs, id),
            }),
            (None, None) => {}
        }
        let paths: BTreeSet<&PropertyPath> = ours
            .properties
            .keys()
            .chain(theirs.properties.keys())
            .copied()
            .collect();
        for path in paths {
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

/// 3-way merge를 실행합니다.
pub fn merge(input: &MergeInput) -> Result<MergeResult, MergeError> {
    let ours_diff = diff(&input.base, &input.ours);
    let theirs_diff = diff(&input.base, &input.theirs);
    let ours = group(&ours_diff);
    let theirs = group(&theirs_diff);
    let mut merger = Merger {
        input,
        accepted: Vec::new(),
        conflicts: Vec::new(),
    };
    let ids: BTreeSet<StableId> = ours.keys().chain(theirs.keys()).copied().collect();
    for id in ids {
        match (ours.get(&id), theirs.get(&id)) {
            (Some(edits), None) | (None, Some(edits)) => merger.accept(edits),
            (Some(a), Some(b)) => merger.merge_object(id, a, b),
            (None, None) => {}
        }
    }

    // 고아를 만든 parent 삭제를 conflict로 되돌리고 다시 적용합니다. 되돌릴 때마다 받아들인
    // 변경이 줄어들므로 유한 번에 끝납니다.
    let mut merged = apply(&input.base, &merger.accepted).map_err(MergeError::Apply)?;
    while let Some((parent, orphan)) = merger.merge_orphan(&merged) {
        let before = merger.accepted.len();
        merger.accepted.retain(|change| {
            !(change.object_id() == parent
                && matches!(
                    change.kind(),
                    SemanticChangeKind::RemoveObject | SemanticChangeKind::RemoveToken
                ))
        });
        if merger.accepted.len() == before {
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
        merger.object_conflict(kind, parent);
        merged = apply(&input.base, &merger.accepted).map_err(MergeError::Apply)?;
    }

    let mut conflicts = merger.conflicts;
    conflicts.sort_by(|a, b| {
        (a.document(), a.object, &a.target, a.kind).cmp(&(
            b.document(),
            b.object,
            &b.target,
            b.kind,
        ))
    });
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
        // matrix가 모든 conflict 종류를 서로 다른 kind로 냅니다.
        assert_eq!(
            produced.into_iter().collect::<Vec<_>>(),
            ConflictKind::ALL.to_vec()
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
}
