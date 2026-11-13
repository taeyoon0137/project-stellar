//! 두 [`SemanticSnapshot`]의 의미 diff (XIV.3.3~).
//!
//! **객체 대응은 StableId로만 합니다(XIV.3.3).** 이름·geometry·내용 유사도로 rename이나
//! 이동을 추정하지 않습니다. 따라서 같은 이름·같은 모양이라도 StableId가 다르면 삭제 +
//! 추가입니다. 같은 StableId라도 종류나 범위(`Origin`)가 다르면 속성 단위로 비교할 수 없는
//! 다른 객체이므로 역시 삭제 + 추가로 냅니다.
//!
//! 두 snapshot을 StableId 오름차순으로 한 번 merge-join하므로 비용은 객체 수에 선형입니다.

use std::cmp::Ordering;

use crate::change::{Provenance, SemanticChange};
use crate::snapshot::{Origin, SemanticObject, SemanticSnapshot};

/// 객체의 기본 출처(범위에서 유도).
pub fn object_provenance(object: &SemanticObject) -> Provenance {
    match object.origin() {
        Origin::Project => Provenance::Project,
        Origin::Document(document) => Provenance::Document { document },
        Origin::Component {
            document,
            component,
        } => Provenance::ComponentDefinition {
            document,
            component,
        },
    }
}

/// `base`에서 `target`으로 가는 의미 변경 목록.
///
/// 현재 범위(XIV.3.3): StableId 대응에 따른 객체 추가·삭제. 출력은 StableId 오름차순이며
/// 같은 id의 삭제가 추가보다 앞섭니다.
pub fn diff(base: &SemanticSnapshot, target: &SemanticSnapshot) -> Vec<SemanticChange> {
    let mut changes = Vec::new();
    let mut left = base.objects().peekable();
    let mut right = target.objects().peekable();
    loop {
        let order = match (left.peek(), right.peek()) {
            (None, None) => break,
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (Some(a), Some(b)) => a.id().cmp(&b.id()),
        };
        match order {
            Ordering::Less => {
                if let Some(a) = left.next() {
                    changes.push(SemanticChange::removed(object_provenance(a), a.clone()));
                }
            }
            Ordering::Greater => {
                if let Some(b) = right.next() {
                    changes.push(SemanticChange::added(object_provenance(b), b.clone()));
                }
            }
            Ordering::Equal => {
                if let (Some(a), Some(b)) = (left.next(), right.next()) {
                    diff_matched(a, b, &mut changes);
                }
            }
        }
    }
    changes
}

/// 같은 StableId 객체 한 쌍을 비교합니다.
fn diff_matched(base: &SemanticObject, target: &SemanticObject, out: &mut Vec<SemanticChange>) {
    if !base.same_identity(target) {
        out.push(SemanticChange::removed(
            object_provenance(base),
            base.clone(),
        ));
        out.push(SemanticChange::added(
            object_provenance(target),
            target.clone(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::tests::{build, head_records, node, rect, sid};
    use crate::change::SemanticChangeKind;
    use crate::snapshot::{NodeKind, ObjectKind};

    #[test]
    fn identical_snapshots_have_no_changes() {
        let snapshot = build(&head_records());
        assert!(diff(&snapshot, &snapshot).is_empty());
        assert!(diff(&SemanticSnapshot::new(), &SemanticSnapshot::new()).is_empty());
    }

    #[test]
    fn same_name_different_id_is_add_and_remove() {
        // rect 11("box")을 이름·크기·위치가 같은 새 id 21로 바꿉니다. rename으로 추정하지 않습니다.
        let base = head_records();
        let mut target = base.clone();
        target.nodes.retain(|n| n.id != sid(11));
        target.modifier_stacks.clear();
        target
            .nodes
            .push(node(21, Some(10), 0x40, "box", rect(10.0, 20.0)));

        let changes = diff(&build(&base), &build(&target));
        let summary: Vec<_> = changes.iter().map(|c| (c.kind(), c.object_id())).collect();
        assert_eq!(
            summary,
            vec![
                (SemanticChangeKind::RemoveObject, sid(11)),
                (SemanticChangeKind::AddObject, sid(21)),
                (SemanticChangeKind::RemoveObject, sid(50)),
            ]
        );
        let SemanticChange::AddObject(added) = &changes[1] else {
            panic!("AddObject 기대");
        };
        assert_eq!(added.object.kind(), ObjectKind::Node(NodeKind::Shape));
        assert_eq!(added.provenance, Provenance::Document { document: sid(1) });
        let SemanticChange::RemoveObject(removed) = &changes[0] else {
            panic!("RemoveObject 기대");
        };
        assert_eq!(removed.object, build(&base).get(sid(11)).unwrap().clone());
    }

    #[test]
    fn same_id_different_kind_is_replace() {
        let base = head_records();
        let mut target = base.clone();
        let fragment = target.nodes.iter_mut().find(|n| n.id == sid(13)).unwrap();
        fragment.payload = rect(1.0, 1.0);
        let changes = diff(&build(&base), &build(&target));
        let summary: Vec<_> = changes.iter().map(|c| (c.kind(), c.object_id())).collect();
        assert_eq!(
            summary,
            vec![
                (SemanticChangeKind::RemoveObject, sid(13)),
                (SemanticChangeKind::AddObject, sid(13)),
            ]
        );
    }

    #[test]
    fn whole_snapshot_add_and_remove() {
        let snapshot = build(&head_records());
        let added = diff(&SemanticSnapshot::new(), &snapshot);
        assert_eq!(added.len(), snapshot.len());
        assert!(added
            .iter()
            .all(|c| c.kind() == SemanticChangeKind::AddObject));
        let removed = diff(&snapshot, &SemanticSnapshot::new());
        assert!(removed
            .iter()
            .all(|c| c.kind() == SemanticChangeKind::RemoveObject));
        let ids: Vec<_> = removed.iter().map(|c| c.object_id()).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted, "StableId 오름차순");
    }
}
