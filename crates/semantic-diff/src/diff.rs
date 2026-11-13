//! 두 [`SemanticSnapshot`]의 의미 diff (XIV.3.3~).
//!
//! **객체 대응은 StableId로만 합니다(XIV.3.3).** 이름·geometry·내용 유사도로 rename이나
//! 이동을 추정하지 않습니다. 따라서 같은 이름·같은 모양이라도 StableId가 다르면 삭제 +
//! 추가입니다. 같은 StableId라도 종류나 범위(`Origin`)가 다르면 속성 단위로 비교할 수 없는
//! 다른 객체이므로 역시 삭제 + 추가로 냅니다.
//!
//! **속성 비교는 `StableId + PropertyPath` 단위입니다(XIV.3.4).** 같은 정체성 객체의 속성 map을
//! 경로 오름차순으로 merge-join해 값이 다른 경로마다 변경 하나를 냅니다. 값 비교는
//! canonical 비교([`SemanticValue::canonical_eq`])이며 float은 bit 단위입니다 — epsilon
//! diff를 쓰지 않으므로 bit가 다른 유한 float은 항상 변경입니다. 한쪽에만 있는 경로는
//! `None` ↔ 값 변경입니다.
//!
//! 두 snapshot을 StableId 오름차순으로 한 번 merge-join하므로 비용은 객체·속성 수에 선형입니다.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use stellar_core_types::property::PropertyPath;

use crate::change::{PropertyChange, Provenance, SemanticChange};
use crate::snapshot::{Origin, SemanticObject, SemanticSnapshot};
use crate::value::SemanticValue;

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
/// 현재 범위: StableId 대응에 따른 객체 추가·삭제(XIV.3.3), 같은 정체성 객체의 속성
/// 변경(XIV.3.4). 출력은 StableId 오름차순(같은 객체 안에서는 경로 오름차순)이며 같은 id의
/// 삭제가 추가보다 앞섭니다.
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
        return;
    }
    let provenance = object_provenance(target);
    for (path, old, new) in changed_properties(base.properties(), target.properties()) {
        out.push(SemanticChange::property(
            target.kind(),
            PropertyChange {
                provenance,
                path: path.clone(),
                old: old.cloned(),
                new: new.cloned(),
            },
        ));
    }
}

/// 두 속성 map에서 canonical 값이 다른 경로(경로 오름차순)와 이전·이후 값.
fn changed_properties<'a>(
    base: &'a BTreeMap<PropertyPath, SemanticValue>,
    target: &'a BTreeMap<PropertyPath, SemanticValue>,
) -> Vec<(
    &'a PropertyPath,
    Option<&'a SemanticValue>,
    Option<&'a SemanticValue>,
)> {
    let mut out = Vec::new();
    let mut left = base.iter().peekable();
    let mut right = target.iter().peekable();
    loop {
        let order = match (left.peek(), right.peek()) {
            (None, None) => break,
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (Some((a, _)), Some((b, _))) => a.cmp(b),
        };
        match order {
            Ordering::Less => {
                if let Some((path, value)) = left.next() {
                    out.push((path, Some(value), None));
                }
            }
            Ordering::Greater => {
                if let Some((path, value)) = right.next() {
                    out.push((path, None, Some(value)));
                }
            }
            Ordering::Equal => {
                if let (Some((path, a)), Some((_, b))) = (left.next(), right.next()) {
                    if !a.canonical_eq(b) {
                        out.push((path, Some(a), Some(b)));
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::tests::{build, head_records, node, rect, sid};
    use crate::build::{SnapshotBuilder, TokenEntry};
    use crate::change::SemanticChangeKind;
    use crate::keys;
    use crate::snapshot::{NodeKind, ObjectKind};
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::reference::AssetRef;
    use stellar_core_types::typed_value::TypedValue;
    use stellar_core_types::value::{FiniteF32, Size2, Transform2D};
    use stellar_document::frame::{Clipping, FramePayload};
    use stellar_document::records::{DocumentRecords, NodePayload};

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

    fn frame_with(mutate: impl FnOnce(&mut FramePayload)) -> DocumentRecords {
        let mut records = head_records();
        let root = records.nodes.iter_mut().find(|n| n.id == sid(10)).unwrap();
        let NodePayload::Frame(frame) = &mut root.payload else {
            panic!("root는 Frame");
        };
        mutate(frame);
        records
    }

    #[test]
    fn bit_different_finite_float_is_detected() {
        // 320.0과 그 다음 표현 가능 f32는 epsilon 비교라면 같다고 볼 값입니다.
        let next = f32::from_bits(320.0f32.to_bits() + 1);
        assert!((next - 320.0).abs() < 1e-4);
        let base = frame_with(|_| {});
        let target = frame_with(|f| f.size = Size2::new(next, 240.0).unwrap());
        let changes = diff(&build(&base), &build(&target));
        assert_eq!(changes.len(), 1);
        let SemanticChange::SetProperty(change) = &changes[0] else {
            panic!("SetProperty 기대: {changes:?}");
        };
        assert_eq!(
            change.path,
            PropertyPath::with_keys(sid(10), vec![keys::frame::SIZE])
        );
        let floats = |w: f32| {
            SemanticValue::Floats(vec![
                FiniteF32::new(w).unwrap(),
                FiniteF32::new(240.0).unwrap(),
            ])
        };
        assert_eq!(change.old, Some(floats(320.0)));
        assert_eq!(change.new, Some(floats(next)));
        assert_eq!(change.provenance, Provenance::Document { document: sid(1) });
    }

    #[test]
    fn property_paths_are_compared_independently() {
        let color = Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 1.0).unwrap();
        let base = frame_with(|_| {});
        let target = frame_with(|f| {
            f.background = Some(color);
            f.clipping = Clipping::Disabled;
        });
        let changes = diff(&build(&base), &build(&target));
        let paths: Vec<_> = changes
            .iter()
            .map(|c| c.property_path().unwrap().keys()[0])
            .collect();
        // 경로 오름차순: clipping(0x0103) < background(0x0104).
        assert_eq!(paths, vec![keys::frame::CLIPPING, keys::frame::BACKGROUND]);
        let SemanticChange::SetProperty(added) = &changes[1] else {
            panic!("SetProperty 기대");
        };
        assert_eq!(added.old, None, "없던 속성은 None → 값");
        assert_eq!(
            added.new,
            Some(SemanticValue::Typed(TypedValue::Color(color)))
        );
        // 되돌리면 값 → None.
        let back = diff(&build(&target), &build(&base));
        let SemanticChange::SetProperty(removed) = &back[1] else {
            panic!("SetProperty 기대");
        };
        assert_eq!((removed.old.is_some(), removed.new.is_none()), (true, true));
    }

    #[test]
    fn negative_zero_is_not_a_change() {
        let base = frame_with(|_| {});
        let target =
            frame_with(|f| f.transform = Transform2D::new(1.0, -0.0, 0.0, 1.0, -0.0, 0.0).unwrap());
        assert!(diff(&build(&base), &build(&target)).is_empty());
    }

    #[test]
    fn property_kinds_follow_classification() {
        let base = head_records();
        let mut target = base.clone();
        let image = target.nodes.iter_mut().find(|n| n.id == sid(12)).unwrap();
        let NodePayload::Image(payload) = &mut image.payload else {
            panic!("Image 기대");
        };
        payload.asset = AssetRef::new(sid(91));
        image.display_name = "사진".into();
        let changes = diff(&build(&base), &build(&target));
        let kinds: Vec<_> = changes
            .iter()
            .map(|c| (c.kind(), c.property_path().unwrap().keys()[0]))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (SemanticChangeKind::AssetRefChange, keys::IMAGE_ASSET),
                (SemanticChangeKind::SetProperty, keys::DISPLAY_NAME),
            ]
        );

        let token = |value: f32| {
            let mut builder = SnapshotBuilder::new();
            builder
                .add_token(&TokenEntry {
                    id: sid(200),
                    document: None,
                    name: "space".into(),
                    value_type: 2,
                    value: Some(TypedValue::Number(FiniteF32::new(value).unwrap())),
                    expression: None,
                })
                .unwrap();
            builder.finish()
        };
        let changes = diff(&token(4.0), &token(8.0));
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind(), SemanticChangeKind::ChangeToken);
        assert_eq!(*changes[0].provenance(), Provenance::Project);
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
