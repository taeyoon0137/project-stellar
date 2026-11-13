//! diff 적용 (XIV.3.9).
//!
//! base snapshot에 변경 목록을 적용해 target snapshot을 재현합니다: `apply(diff(A, B), A) == B`.
//!
//! - **목록 순서와 무관**: 삭제 → 추가 → 위치 → 속성 단계로 적용하므로 같은 객체의 교체(삭제 +
//!   추가)도 정렬 순서와 관계없이 동작합니다.
//! - **base 검증**: 모든 변경은 이전 상태를 담고 있어(XIV.3.2) 적용 전 base가 그 상태인지
//!   확인합니다 — 삭제는 객체 전체 동일, 위치는 from 동일, 속성은 old 동일(canonical 비교).
//!   어긋나면 오래된(stale) diff로 보고 거부합니다.
//! - **목록 검증**: 같은 객체의 같은 단위(삭제·추가·위치·속성 경로)를 두 번 다루거나, 종류
//!   분류가 대상과 맞지 않거나(예: token이 아닌 객체에 ChangeToken), 효과 없는 변경은 거부합니다.
//! - **원자성**: 결과는 새 snapshot이고 실패하면 아무것도 반환하지 않습니다. base는 바뀌지
//!   않습니다.
//! - 출처(`Provenance`)는 표시 정보이며 적용 결과에 영향이 없습니다.

use std::collections::BTreeSet;
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;

use crate::change::{
    ObjectChange, Position, PositionChange, PropertyChange, SemanticChange, SemanticChangeKind,
};
use crate::snapshot::{SemanticSnapshot, SnapshotError};

/// 변경 적용 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    /// 같은 대상(객체 삭제·추가·위치, 또는 속성 경로)을 두 번 다룹니다.
    DuplicateTarget {
        /// 대상 객체.
        object: StableId,
        /// 대상 속성 경로(객체 단위면 `None`).
        path: Option<PropertyPath>,
    },
    /// 변경 종류가 대상·내용과 맞지 않습니다.
    InvalidChange {
        /// 대상 객체.
        object: StableId,
        /// 원인.
        reason: &'static str,
    },
    /// 대상 객체가 없습니다.
    MissingObject {
        /// 객체.
        id: StableId,
    },
    /// 추가할 객체가 이미 있습니다.
    ObjectExists {
        /// 객체.
        id: StableId,
    },
    /// 삭제할 객체가 변경에 기록된 상태와 다릅니다(stale).
    ObjectMismatch {
        /// 객체.
        id: StableId,
    },
    /// 객체 위치가 변경의 from과 다릅니다(stale).
    PositionMismatch {
        /// 객체.
        id: StableId,
    },
    /// 속성 값이 변경의 old와 다릅니다(stale).
    PropertyMismatch {
        /// 속성 경로.
        path: PropertyPath,
    },
    /// snapshot 규칙 위반(속성 root 불일치 등).
    Snapshot(SnapshotError),
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApplyError::DuplicateTarget { object, path } => {
                write!(f, "같은 대상을 두 번 변경합니다: {object} {path:?}")
            }
            ApplyError::InvalidChange { object, reason } => {
                write!(f, "잘못된 변경({object}): {reason}")
            }
            ApplyError::MissingObject { id } => write!(f, "대상 객체가 없습니다: {id}"),
            ApplyError::ObjectExists { id } => write!(f, "객체가 이미 있습니다: {id}"),
            ApplyError::ObjectMismatch { id } => {
                write!(f, "삭제할 객체가 기록된 상태와 다릅니다: {id}")
            }
            ApplyError::PositionMismatch { id } => {
                write!(f, "객체 위치가 기록된 이전 위치와 다릅니다: {id}")
            }
            ApplyError::PropertyMismatch { path } => {
                write!(f, "속성 값이 기록된 이전 값과 다릅니다: {path:?}")
            }
            ApplyError::Snapshot(error) => write!(f, "snapshot 규칙 위반: {error}"),
        }
    }
}

impl std::error::Error for ApplyError {}

impl From<SnapshotError> for ApplyError {
    fn from(error: SnapshotError) -> ApplyError {
        ApplyError::Snapshot(error)
    }
}

/// 변경을 단계별로 나눈 목록.
#[derive(Default)]
struct Phases<'a> {
    removes: Vec<&'a ObjectChange>,
    adds: Vec<&'a ObjectChange>,
    positions: Vec<(SemanticChangeKind, &'a PositionChange)>,
    properties: Vec<(SemanticChangeKind, &'a PropertyChange)>,
}

fn invalid(object: StableId, reason: &'static str) -> ApplyError {
    ApplyError::InvalidChange { object, reason }
}

/// 목록 자체를 검증하고 단계별로 나눕니다(base와 무관한 검사).
fn plan(changes: &[SemanticChange]) -> Result<Phases<'_>, ApplyError> {
    let mut phases = Phases::default();
    let mut removed = BTreeSet::new();
    let mut added = BTreeSet::new();
    let mut moved = BTreeSet::new();
    let mut touched: BTreeSet<&PropertyPath> = BTreeSet::new();
    let duplicate = |object, path: Option<&PropertyPath>| ApplyError::DuplicateTarget {
        object,
        path: path.cloned(),
    };
    for change in changes {
        let kind = change.kind();
        let id = change.object_id();
        match change {
            SemanticChange::RemoveObject(c) | SemanticChange::RemoveToken(c) => {
                if SemanticChangeKind::remove_for(c.object.kind()) != kind {
                    return Err(invalid(id, "삭제 종류가 객체 종류와 맞지 않습니다"));
                }
                if !removed.insert(id) {
                    return Err(duplicate(id, None));
                }
                phases.removes.push(c);
            }
            SemanticChange::AddObject(c) | SemanticChange::AddToken(c) => {
                if SemanticChangeKind::add_for(c.object.kind()) != kind {
                    return Err(invalid(id, "추가 종류가 객체 종류와 맞지 않습니다"));
                }
                if !added.insert(id) {
                    return Err(duplicate(id, None));
                }
                phases.adds.push(c);
            }
            SemanticChange::MoveObject(c) | SemanticChange::ReorderObject(c) => {
                let same_parent = c.from.parent == c.to.parent;
                let valid = match kind {
                    SemanticChangeKind::MoveObject => !same_parent,
                    _ => same_parent && c.from.order_key != c.to.order_key,
                };
                if !valid {
                    return Err(invalid(id, "이동·재정렬 분류가 위치 변화와 맞지 않습니다"));
                }
                if !moved.insert(id) {
                    return Err(duplicate(id, None));
                }
                phases.positions.push((kind, c));
            }
            SemanticChange::SetProperty(c)
            | SemanticChange::ChangeToken(c)
            | SemanticChange::ComponentInterfaceChange(c)
            | SemanticChange::AssetRefChange(c) => {
                let no_op = match (&c.old, &c.new) {
                    (None, None) => true,
                    (Some(a), Some(b)) => a.canonical_eq(b),
                    _ => false,
                };
                if no_op {
                    return Err(invalid(id, "효과 없는 속성 변경입니다"));
                }
                if !touched.insert(&c.path) {
                    return Err(duplicate(id, Some(&c.path)));
                }
                phases.properties.push((kind, c));
            }
        }
    }
    Ok(phases)
}

/// `base`에 `changes`를 적용한 새 snapshot을 반환합니다.
pub fn apply(
    base: &SemanticSnapshot,
    changes: &[SemanticChange],
) -> Result<SemanticSnapshot, ApplyError> {
    let phases = plan(changes)?;
    let mut snapshot = base.clone();

    for change in phases.removes {
        let id = change.object.id();
        match snapshot.get(id) {
            None => return Err(ApplyError::MissingObject { id }),
            Some(current) if *current != change.object => {
                return Err(ApplyError::ObjectMismatch { id })
            }
            Some(_) => {}
        }
        snapshot.remove(id);
    }

    for change in phases.adds {
        let id = change.object.id();
        if snapshot.contains(id) {
            return Err(ApplyError::ObjectExists { id });
        }
        if change
            .object
            .properties()
            .keys()
            .any(|path| path.root() != id)
        {
            return Err(invalid(id, "추가 객체의 속성 root가 객체가 아닙니다"));
        }
        snapshot.insert(change.object.clone())?;
    }

    for (_, change) in phases.positions {
        let id = change.object;
        let object = snapshot
            .get_mut(id)
            .ok_or(ApplyError::MissingObject { id })?;
        if Position::of(object) != change.from {
            return Err(ApplyError::PositionMismatch { id });
        }
        object.set_position(change.to.parent, change.to.order_key.clone());
    }

    for (kind, change) in phases.properties {
        let id = change.path.root();
        let object = snapshot
            .get_mut(id)
            .ok_or(ApplyError::MissingObject { id })?;
        if SemanticChangeKind::property_for(object.kind(), &change.path) != kind {
            return Err(invalid(
                id,
                "속성 변경 종류가 대상 객체·경로와 맞지 않습니다",
            ));
        }
        let matches = match (object.property(&change.path), &change.old) {
            (None, None) => true,
            (Some(current), Some(old)) => current.canonical_eq(old),
            _ => false,
        };
        if !matches {
            return Err(ApplyError::PropertyMismatch {
                path: change.path.clone(),
            });
        }
        match &change.new {
            Some(value) => {
                object.set_property(change.path.clone(), value.clone())?;
            }
            None => {
                object.remove_property(&change.path);
            }
        }
    }
    Ok(snapshot)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::build::tests::{build, head_records, sid};
    use crate::change::Provenance;
    use crate::diff::diff;
    use crate::keys;
    use crate::snapshot::{NodeKind, ObjectKind, Origin, SemanticObject};
    use crate::value::SemanticValue;
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::value::FiniteF32;
    use stellar_document::test_support::random_document::{random_valid_document, SplitMix64};

    /// seed로 snapshot을 무작위 변형합니다: 속성 값 변경·추가·삭제, 객체 삭제·추가·교체,
    /// 이동·재정렬. 결과는 문서로서 유효할 필요가 없습니다(diff/apply는 범용).
    pub(crate) fn mutate(snapshot: &SemanticSnapshot, seed: u64) -> SemanticSnapshot {
        let mut rng = SplitMix64::new(seed);
        let mut out = snapshot.clone();
        let ids: Vec<StableId> = out.objects().map(|o| o.id()).collect();
        let steps = 1 + rng.below(8);
        for step in 0..steps {
            let pick = |rng: &mut SplitMix64| ids[rng.below(ids.len() as u64) as usize];
            let fresh = StableId::new_v7(1_900_000_000_000 + seed * 100 + step, 7, seed ^ step);
            match rng.below(7) {
                0 if !ids.is_empty() => {
                    out.remove(pick(&mut rng));
                }
                1 => {
                    let mut object = SemanticObject::new(
                        fresh,
                        ObjectKind::Node(NodeKind::Fragment),
                        Origin::Document(sid(1)),
                    )
                    .with_position(ids.first().copied(), Some(OrderKey::between(None, None)));
                    object
                        .set_property(object.path(&[keys::VISIBLE]), SemanticValue::Bool(true))
                        .unwrap();
                    let _ = out.insert(object);
                }
                2 | 3 if !ids.is_empty() => {
                    let id = pick(&mut rng);
                    if let Some(mut object) = out.remove(id) {
                        let path = object.path(&[keys::DISPLAY_NAME]);
                        let value = match rng.below(3) {
                            0 => None,
                            1 => Some(SemanticValue::Text(format!("n{seed}-{step}"))),
                            _ => Some(SemanticValue::Floats(vec![FiniteF32::new(
                                rng.below(1000) as f32 / 8.0,
                            )
                            .unwrap()])),
                        };
                        match value {
                            Some(value) => {
                                object.set_property(path, value).unwrap();
                            }
                            None => {
                                object.remove_property(&path);
                            }
                        }
                        out.insert(object).unwrap();
                    }
                }
                4 if !ids.is_empty() => {
                    let id = pick(&mut rng);
                    let parent = pick(&mut rng);
                    if let Some(mut object) = out.remove(id) {
                        let key = OrderKey::between(object.order_key(), None);
                        object.set_position(Some(parent), Some(key));
                        out.insert(object).unwrap();
                    }
                }
                5 if !ids.is_empty() => {
                    let id = pick(&mut rng);
                    if let Some(mut object) = out.remove(id) {
                        let key = OrderKey::between(None, object.order_key());
                        let parent = object.parent();
                        object.set_position(parent, Some(key));
                        out.insert(object).unwrap();
                    }
                }
                6 if !ids.is_empty() => {
                    // 같은 id, 다른 종류(교체).
                    let id = pick(&mut rng);
                    if let Some(object) = out.remove(id) {
                        let kind = if object.kind() == ObjectKind::Token {
                            ObjectKind::Asset
                        } else {
                            ObjectKind::Token
                        };
                        out.insert(SemanticObject::new(id, kind, Origin::Project))
                            .unwrap();
                    }
                }
                _ => {}
            }
        }
        out
    }

    #[test]
    fn apply_diff_reproduces_target_property() {
        // property test: 무관한 두 문서, 그리고 문서와 그 변형.
        let mut mutated_kinds = BTreeSet::new();
        for seed in 0..200 {
            let a = build(&random_valid_document(seed).to_records().unwrap());
            let b = build(&random_valid_document(seed + 10_000).to_records().unwrap());
            let mutated = mutate(&a, seed);
            mutated_kinds.extend(diff(&a, &mutated).iter().map(|c| c.kind()));
            for (from, to) in [(&a, &b), (&a, &mutated), (&mutated, &a), (&b, &a)] {
                let changes = diff(from, to);
                assert_eq!(apply(from, &changes).unwrap(), *to, "seed {seed}");
                // 정렬 순서를 뒤집어도 같은 결과(단계별 적용).
                let mut reversed = changes.changes().to_vec();
                reversed.reverse();
                assert_eq!(apply(from, &reversed).unwrap(), *to, "seed {seed} reversed");
            }
        }
        // 변형이 실제로 여러 종류의 변경을 만들었는지.
        for kind in [
            SemanticChangeKind::AddObject,
            SemanticChangeKind::RemoveObject,
            SemanticChangeKind::MoveObject,
            SemanticChangeKind::ReorderObject,
            SemanticChangeKind::SetProperty,
            SemanticChangeKind::AddToken,
        ] {
            assert!(mutated_kinds.contains(&kind), "{kind:?} 미생성");
        }
    }

    #[test]
    fn stale_base_is_rejected() {
        let base = build(&head_records());
        let mut records = head_records();
        records.nodes[1].display_name = "renamed".into();
        records.nodes.retain(|n| n.id != sid(13));
        let target = build(&records);
        let changes = diff(&base, &target);

        // 같은 속성을 다른 값으로 이미 바꾼 base.
        let mut stale = base.clone();
        let mut rect = stale.remove(sid(11)).unwrap();
        rect.set_property(
            rect.path(&[keys::DISPLAY_NAME]),
            SemanticValue::Text("other".into()),
        )
        .unwrap();
        stale.insert(rect).unwrap();
        assert_eq!(
            apply(&stale, &changes),
            Err(ApplyError::PropertyMismatch {
                path: PropertyPath::with_keys(sid(11), vec![keys::DISPLAY_NAME])
            })
        );

        // 삭제 대상이 바뀐 base.
        let mut stale = base.clone();
        let mut fragment = stale.remove(sid(13)).unwrap();
        fragment
            .set_property(fragment.path(&[keys::LOCKED]), SemanticValue::Bool(true))
            .unwrap();
        stale.insert(fragment).unwrap();
        assert_eq!(
            apply(&stale, &changes),
            Err(ApplyError::ObjectMismatch { id: sid(13) })
        );

        // 대상이 이미 없는 base, 이미 적용된 target.
        assert_eq!(
            apply(&target, &changes),
            Err(ApplyError::MissingObject { id: sid(13) })
        );
        let added = diff(&SemanticSnapshot::new(), &base);
        assert!(matches!(
            apply(&base, &added),
            Err(ApplyError::ObjectExists { .. })
        ));
    }

    #[test]
    fn invalid_change_lists_are_rejected() {
        let base = build(&head_records());
        let path = PropertyPath::with_keys(sid(11), vec![keys::DISPLAY_NAME]);
        let set = |new: &str| {
            SemanticChange::SetProperty(PropertyChange {
                provenance: Provenance::Document { document: sid(1) },
                path: path.clone(),
                old: Some(SemanticValue::Text("box".into())),
                new: Some(SemanticValue::Text(new.into())),
            })
        };
        assert_eq!(
            apply(&base, &[set("a"), set("b")]),
            Err(ApplyError::DuplicateTarget {
                object: sid(11),
                path: Some(path.clone())
            })
        );
        assert!(matches!(
            apply(&base, &[set("box")]),
            Err(ApplyError::InvalidChange { .. })
        ));
        // token이 아닌 객체에 ChangeToken.
        let wrong_kind = SemanticChange::ChangeToken(PropertyChange {
            provenance: Provenance::Project,
            path: path.clone(),
            old: Some(SemanticValue::Text("box".into())),
            new: Some(SemanticValue::Text("x".into())),
        });
        assert!(matches!(
            apply(&base, &[wrong_kind]),
            Err(ApplyError::InvalidChange { .. })
        ));
        // 같은 parent인데 MoveObject.
        let rect = base.get(sid(11)).unwrap();
        let mut to = Position::of(rect);
        to.order_key = Some(OrderKey::from_bytes(vec![0x41]).unwrap());
        let wrong_move = SemanticChange::MoveObject(PositionChange {
            provenance: Provenance::Document { document: sid(1) },
            object: sid(11),
            from: Position::of(rect),
            to,
        });
        assert!(matches!(
            apply(&base, &[wrong_move]),
            Err(ApplyError::InvalidChange { .. })
        ));
        // 실패해도 base는 그대로.
        assert_eq!(base, build(&head_records()));
    }
}
