//! 두 [`SemanticSnapshot`]의 의미 diff (XIV.3.3~).
//!
//! **객체 대응은 StableId로만 합니다(XIV.3.3).** 이름·geometry·내용 유사도로 rename이나
//! 이동을 추정하지 않습니다. 따라서 같은 이름·같은 모양이라도 StableId가 다르면 삭제 +
//! 추가입니다. 같은 StableId라도 종류나 범위(`Origin`)가 다르면 속성 단위로 비교할 수 없는
//! 다른 객체이므로 역시 삭제 + 추가로 냅니다.
//!
//! **위치는 parent와 OrderKey로 분류합니다(XIV.3.5).** parent가 바뀌면 MoveObject(새 OrderKey
//! 포함), 같은 parent에서 OrderKey가 바뀌면 ReorderObject입니다. 순서는 OrderKey의 의미 순서
//! (byte 사전순)이며 레코드 배열 순서·chunk·파일 배치는 snapshot에 없으므로 저장 chunk 이동만
//! 있는 경우 변경이 0입니다. 위치 변경은 같은 객체의 속성 변경보다 앞에 둡니다.
//!
//! **출처(XIV.3.6)**: 컴포넌트 정의(정의 resource·정의 tree·Slot) 변경은 ComponentDefinition,
//! scene 인스턴스의 Property·Slot override 변경은 InstanceOverride 출처를 가집니다. 같은 값
//! 변화라도 어디서 바뀌었는지 구분됩니다([`property_provenance`]).
//!
//! **속성 비교는 `StableId + PropertyPath` 단위입니다(XIV.3.4).** 같은 정체성 객체의 속성 map을
//! 경로 오름차순으로 merge-join해 값이 다른 경로마다 변경 하나를 냅니다. 값 비교는
//! canonical 비교([`SemanticValue::canonical_eq`])이며 float은 bit 단위입니다 — epsilon
//! diff를 쓰지 않으므로 bit가 다른 유한 float은 항상 변경입니다. 한쪽에만 있는 경로는
//! `None` ↔ 값 변경입니다.
//!
//! **결정론(XIV.3.8)**: 결과는 [`canonical_order`](문서, 객체, 속성 경로, 변경 종류)로 정렬된
//! [`SemanticDiff`]이며 canonical bytes를 가집니다. snapshot이 StableId map이라 입력 레코드·
//! builder 호출 순서와 무관하게 같은 bytes가 나옵니다.
//!
//! 두 snapshot을 StableId 오름차순으로 한 번 merge-join하므로 비용은 객체·속성 수에 선형입니다.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use stellar_core_types::property::PropertyPath;

use crate::change::{Position, PositionChange, PropertyChange, Provenance, SemanticChange};
use crate::codec::{self, DecodeError};
use crate::keys;
use crate::snapshot::{NodeKind, ObjectKind, Origin, SemanticObject, SemanticSnapshot};
use crate::value::SemanticValue;

/// 객체의 기본 출처(범위에서 유도).
pub fn object_provenance(object: &SemanticObject) -> Provenance {
    match object.origin() {
        Origin::Project => Provenance::Project,
        Origin::ProjectComponent(component) => Provenance::ProjectComponentDefinition(component),
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
/// StableId 대응에 따른 객체 추가·삭제(XIV.3.3), 같은 정체성 객체의 속성 변경(XIV.3.4),
/// 이동·재정렬(XIV.3.5)을 계산하고 [`canonical_order`]로 정렬해 반환합니다(XIV.3.8).
pub fn diff(base: &SemanticSnapshot, target: &SemanticSnapshot) -> SemanticDiff {
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
    SemanticDiff::new(changes)
}

/// 변경의 결정론적 정렬 순서(XIV.3.8).
///
/// key: (문서 — 프로젝트 범위(`None`)가 먼저, 객체 StableId, 속성 경로 — 객체 단위 변경
/// (`None`)이 먼저, 변경 종류 code). 남은 동률(예: 같은 객체를 두 번 다루는 비정상 목록)은
/// 변경 canonical bytes로 깨므로 전순서입니다. 객체 경로는 diff 양쪽에서 바뀔 수 있는 tree
/// 경로 대신 불변 StableId를 씁니다.
pub fn canonical_order(a: &SemanticChange, b: &SemanticChange) -> Ordering {
    a.document()
        .cmp(&b.document())
        .then_with(|| a.object_id().cmp(&b.object_id()))
        .then_with(|| a.property_path().cmp(&b.property_path()))
        .then_with(|| a.kind().code().cmp(&b.kind().code()))
        .then_with(|| codec::change_bytes(a).cmp(&codec::change_bytes(b)))
}

/// canonical 순서로 정렬된 변경 목록(XIV.3.8).
///
/// 만드는 방법과 무관하게 같은 변경 집합은 같은 순서·같은 bytes가 됩니다. 슬라이스로
/// 역참조됩니다.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SemanticDiff {
    changes: Vec<SemanticChange>,
}

impl SemanticDiff {
    /// 변경 목록을 canonical 순서로 정렬해 만듭니다.
    pub fn new(mut changes: Vec<SemanticChange>) -> SemanticDiff {
        changes.sort_by(canonical_order);
        SemanticDiff { changes }
    }

    /// 변경 목록(canonical 순서).
    pub fn changes(&self) -> &[SemanticChange] {
        &self.changes
    }

    /// 변경 목록을 꺼냅니다.
    pub fn into_changes(self) -> Vec<SemanticChange> {
        self.changes
    }

    /// canonical bytes([`codec`] 형식, 헤더 `SDIF`).
    pub fn to_canonical_bytes(&self) -> Vec<u8> {
        codec::diff_to_bytes(&self.changes)
    }

    /// canonical bytes를 읽습니다. canonical 순서가 아니면 거부합니다.
    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<SemanticDiff, DecodeError> {
        let changes = codec::diff_from_bytes(bytes)?;
        if changes
            .windows(2)
            .any(|pair| canonical_order(&pair[0], &pair[1]) != Ordering::Less)
        {
            return Err(DecodeError::NonCanonicalOrder {
                field: "diff.changes",
            });
        }
        Ok(SemanticDiff { changes })
    }
}

impl std::ops::Deref for SemanticDiff {
    type Target = [SemanticChange];

    fn deref(&self) -> &[SemanticChange] {
        &self.changes
    }
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
    let (from, to) = (Position::of(base), Position::of(target));
    if from.parent != to.parent {
        out.push(SemanticChange::MoveObject(PositionChange {
            provenance,
            object: target.id(),
            from,
            to,
        }));
    } else if from.order_key != to.order_key {
        out.push(SemanticChange::ReorderObject(PositionChange {
            provenance,
            object: target.id(),
            from,
            to,
        }));
    }
    for (path, old, new) in changed_properties(base.properties(), target.properties()) {
        out.push(SemanticChange::property(
            target.kind(),
            PropertyChange {
                provenance: property_provenance(base, target, path),
                path: path.clone(),
                old: old.cloned(),
                new: new.cloned(),
            },
        ));
    }
}

/// 속성 변경의 출처(XIV.3.6).
///
/// 문서 scene의 ComponentInstance node에서 `PROPERTY_OVERRIDES`·`SLOT_OVERRIDES` 아래 경로는
/// 인스턴스 override이고, 출처에 인스턴스와 참조 컴포넌트(이후 값 우선, 없으면 이전 값)를
/// 담습니다. 정의 참조 교체(`COMPONENT_REF`)와 공통 필드는 scene 변경입니다. 컴포넌트 정의
/// tree 안 nested instance의 override는 정의의 일부이므로 ComponentDefinition입니다.
pub fn property_provenance(
    base: &SemanticObject,
    target: &SemanticObject,
    path: &PropertyPath,
) -> Provenance {
    let default = object_provenance(target);
    let (Origin::Document(document), ObjectKind::Node(NodeKind::ComponentInstance)) =
        (target.origin(), target.kind())
    else {
        return default;
    };
    let first = path.keys().first().copied();
    if first != Some(keys::instance::PROPERTY_OVERRIDES)
        && first != Some(keys::instance::SLOT_OVERRIDES)
    {
        return default;
    }
    let reference = PropertyPath::with_keys(target.id(), vec![keys::instance::COMPONENT_REF]);
    let component =
        [target, base]
            .into_iter()
            .find_map(|object| match object.property(&reference) {
                Some(SemanticValue::Id(component)) => Some(*component),
                _ => None,
            });
    match component {
        Some(component) => Provenance::InstanceOverride {
            document,
            instance: target.id(),
            component,
        },
        None => default,
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
    use crate::build::tests::{build, head_records, key, node, rect, sid};
    use crate::build::{InstanceNode, SnapshotBuilder, TokenEntry};
    use crate::change::SemanticChangeKind;
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::ids::StableId;
    use stellar_core_types::property::PathSegment;
    use stellar_core_types::reference::AssetRef;
    use stellar_core_types::reference::{ComponentRef, NodeRef};
    use stellar_core_types::typed_value::TypedValue;
    use stellar_core_types::value::{FiniteF32, Size2, Transform2D};
    use stellar_document::component::definition::{
        ComponentDefinition, ComponentLibrary, ComponentPropertyId, DefinitionNode, DefinitionTree,
    };
    use stellar_document::component::instance::{ComponentInstancePayload, OverrideKey};
    use stellar_document::frame::{Clipping, FramePayload};
    use stellar_document::modifier::{ModifierEntry, ModifierStack};
    use stellar_document::record_codec::encode_payload;
    use stellar_document::record_codec::{decode_node_record, encode_node_record};
    use stellar_document::records::{DocumentRecords, NodePayload};
    use stellar_document::test_support::random_document::{random_valid_document, SplitMix64};

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
        assert_eq!(added.object.id(), sid(21));
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
                (SemanticChangeKind::AddObject, sid(13)),
                (SemanticChangeKind::RemoveObject, sid(13)),
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

    fn summary(changes: &[SemanticChange]) -> Vec<(SemanticChangeKind, StableId)> {
        changes.iter().map(|c| (c.kind(), c.object_id())).collect()
    }

    #[test]
    fn reorder_within_parent_is_not_move() {
        // rect 11을 image 12 뒤(0x40 → 0xA0)로 재정렬합니다.
        let base = head_records();
        let mut target = base.clone();
        target
            .nodes
            .iter_mut()
            .find(|n| n.id == sid(11))
            .unwrap()
            .order_key = key(0xA0);
        let changes = diff(&build(&base), &build(&target));
        assert_eq!(
            summary(&changes),
            vec![(SemanticChangeKind::ReorderObject, sid(11))]
        );
        let SemanticChange::ReorderObject(change) = &changes[0] else {
            panic!("ReorderObject 기대");
        };
        assert_eq!(change.from.parent, Some(sid(10)));
        assert_eq!(change.to.parent, Some(sid(10)));
        assert!(change.from.order_key < change.to.order_key);
        assert_eq!(
            build(&target).children(sid(10)),
            vec![sid(12), sid(11), sid(13)]
        );
    }

    #[test]
    fn parent_change_is_move_with_new_key() {
        // rect 11을 fragment 13 아래로 옮깁니다(OrderKey는 유지). 속성 변경 없음.
        let base = head_records();
        let mut target = base.clone();
        target
            .nodes
            .iter_mut()
            .find(|n| n.id == sid(11))
            .unwrap()
            .parent = Some(sid(13));
        let changes = diff(&build(&base), &build(&target));
        assert_eq!(
            summary(&changes),
            vec![(SemanticChangeKind::MoveObject, sid(11))]
        );
        let SemanticChange::MoveObject(change) = &changes[0] else {
            panic!("MoveObject 기대");
        };
        assert_eq!(change.from.parent, Some(sid(10)));
        assert_eq!(change.to.parent, Some(sid(13)));
        assert_eq!(change.to.order_key, Some(key(0x40)));

        // 이동 + 속성 변경은 위치 변경이 먼저입니다.
        target
            .nodes
            .iter_mut()
            .find(|n| n.id == sid(11))
            .unwrap()
            .visible = false;
        let changes = diff(&build(&base), &build(&target));
        assert_eq!(
            summary(&changes),
            vec![
                (SemanticChangeKind::MoveObject, sid(11)),
                (SemanticChangeKind::SetProperty, sid(11)),
            ]
        );
    }

    #[test]
    fn modifier_reorder_is_reorder() {
        let base = head_records();
        let mut target = base.clone();
        let stack = &mut target.modifier_stacks[0];
        let entry = stack.entries()[0].clone();
        let mut moved = ModifierStack::new(stack.node);
        moved
            .insert(ModifierEntry {
                order: key(0x20),
                ..entry
            })
            .unwrap();
        *stack = moved;
        let changes = diff(&build(&base), &build(&target));
        assert_eq!(
            summary(&changes),
            vec![(SemanticChangeKind::ReorderObject, sid(50))]
        );
    }

    /// seed로 node·stack을 임의의 chunk 묶음에 나눠 각 레코드를 canonical bytes로 왕복합니다.
    fn rechunk(records: &DocumentRecords, seed: u64) -> DocumentRecords {
        let mut rng = SplitMix64::new(seed);
        let buckets = 1 + rng.below(5) as usize;
        let mut chunks: Vec<Vec<Vec<u8>>> = vec![Vec::new(); buckets];
        for node in &records.nodes {
            chunks[rng.below(buckets as u64) as usize].push(encode_node_record(node));
        }
        chunks.reverse();
        let mut rebuilt = DocumentRecords::new(records.definition.clone());
        for chunk in chunks {
            for bytes in chunk.iter().rev() {
                rebuilt.nodes.push(decode_node_record(bytes).unwrap());
            }
        }
        if rebuilt.nodes.len() > 1 && rebuilt.nodes == records.nodes {
            // 우연히 같은 배치가 나오면 한 칸 밀어 실제로 다른 배치를 만듭니다.
            rebuilt.nodes.rotate_left(1);
        }
        let mut stacks = records.modifier_stacks.clone();
        let shift = rng.below(stacks.len() as u64) as usize;
        stacks.rotate_left(shift);
        rebuilt.modifier_stacks = stacks;
        rebuilt
    }

    #[test]
    fn rechunk_only_change_has_zero_semantic_diff() {
        for seed in 0..50 {
            let records = random_valid_document(seed).to_records().unwrap();
            let moved = rechunk(&records, seed ^ 0x5eed);
            if records.nodes.len() > 1 {
                assert_ne!(
                    records.nodes, moved.nodes,
                    "seed {seed}: 저장 배치가 실제로 달라야 함"
                );
            }
            assert!(
                diff(&build(&records), &build(&moved)).is_empty(),
                "seed {seed}: chunk 이동만으로 semantic change가 생겼습니다"
            );
        }
    }

    /// 컴포넌트 300(root frame 301, 공개 Property 310)과 그 인스턴스 400을 담은 snapshot.
    fn component_project(def_width: f32, override_value: Option<f32>) -> SemanticSnapshot {
        let mut library = ComponentLibrary::new();
        let mut definition =
            ComponentDefinition::new(sid(300), "Button".into(), NodeRef::new(sid(301)));
        definition
            .add_property(ComponentPropertyId::new(sid(310)))
            .unwrap();
        let mut tree = DefinitionTree::new(ComponentRef::new(sid(300)));
        let frame = FramePayload {
            size: Size2::new(def_width, 20.0).unwrap(),
            ..Default::default()
        };
        tree.insert(DefinitionNode {
            id: sid(301),
            kind: NodeKind::Frame,
            parent: None,
            order_key: key(0x80),
            visible: true,
            locked: false,
            display_name: String::new(),
            payload: encode_payload(&NodePayload::Frame(frame)),
        })
        .unwrap();
        library.insert(definition, tree).unwrap();

        let mut payload = ComponentInstancePayload::new(ComponentRef::new(sid(300)));
        if let Some(value) = override_value {
            payload.set_property_override(
                OverrideKey::direct(ComponentPropertyId::new(sid(310))),
                TypedValue::Number(FiniteF32::new(value).unwrap()),
            );
        }
        let records = head_records();
        let mut builder = SnapshotBuilder::new();
        builder.add_document(&records).unwrap();
        builder.add_component_library(sid(1), &library).unwrap();
        builder
            .add_instance(
                sid(1),
                &InstanceNode {
                    id: sid(400),
                    parent: Some(sid(10)),
                    order_key: key(0xE0),
                    visible: true,
                    locked: false,
                    display_name: "button".into(),
                    payload,
                },
            )
            .unwrap();
        builder.finish()
    }

    #[test]
    fn definition_change_and_instance_override_have_distinct_provenance() {
        let base = component_project(100.0, None);
        // 정의 root 크기 변경 + 인스턴스 override 추가를 한 번에.
        let target = component_project(120.0, Some(2.0));
        let changes = diff(&base, &target);
        assert_eq!(changes.len(), 2, "{changes:?}");
        let by_object = |id| changes.iter().find(|c| c.object_id() == id).unwrap();

        let definition = by_object(sid(301));
        assert_eq!(definition.kind(), SemanticChangeKind::SetProperty);
        assert_eq!(
            *definition.provenance(),
            Provenance::ComponentDefinition {
                document: sid(1),
                component: sid(300)
            }
        );

        let instance = by_object(sid(400));
        assert_eq!(instance.kind(), SemanticChangeKind::SetProperty);
        assert_eq!(
            *instance.provenance(),
            Provenance::InstanceOverride {
                document: sid(1),
                instance: sid(400),
                component: sid(300)
            }
        );
        let path = instance.property_path().unwrap();
        assert_eq!(path.keys(), &[keys::instance::PROPERTY_OVERRIDES]);
        assert_eq!(
            path.tail(),
            Some(&PathSegment::Key(sid(310).to_string())),
            "override 항목 단위 경로"
        );

        // override 값만 바꾸면 변경 1개, 출처는 인스턴스 override.
        let changes = diff(&target, &component_project(120.0, Some(3.0)));
        assert_eq!(changes.len(), 1);
        assert!(matches!(
            changes[0].provenance(),
            Provenance::InstanceOverride { .. }
        ));
    }

    #[test]
    fn instance_scene_fields_are_document_changes() {
        let base = component_project(100.0, Some(1.0));
        let mut target = base.clone();
        let mut instance = target.remove(sid(400)).unwrap();
        instance
            .set_property(
                instance.path(&[keys::DISPLAY_NAME]),
                SemanticValue::Text("primary".into()),
            )
            .unwrap();
        target.insert(instance).unwrap();
        let changes = diff(&base, &target);
        assert_eq!(changes.len(), 1);
        assert_eq!(
            *changes[0].provenance(),
            Provenance::Document { document: sid(1) }
        );
    }

    /// 같은 내용을 다른 입력 순서로 만든 snapshot.
    fn build_shuffled(records: &DocumentRecords, seed: u64) -> SemanticSnapshot {
        let mut rng = SplitMix64::new(seed);
        let mut shuffled = records.clone();
        for i in (1..shuffled.nodes.len()).rev() {
            let j = rng.below(i as u64 + 1) as usize;
            shuffled.nodes.swap(i, j);
        }
        shuffled.modifier_stacks.reverse();
        let mut builder = SnapshotBuilder::new();
        builder
            .add_asset(&crate::build::AssetEntry {
                id: sid(990),
                content_hash: vec![1; 32],
                media_type: "image/png".into(),
            })
            .unwrap();
        builder.add_document(&shuffled).unwrap();
        builder.finish()
    }

    fn build_plain(records: &DocumentRecords) -> SemanticSnapshot {
        let mut builder = SnapshotBuilder::new();
        builder.add_document(records).unwrap();
        builder
            .add_asset(&crate::build::AssetEntry {
                id: sid(990),
                content_hash: vec![1; 32],
                media_type: "image/png".into(),
            })
            .unwrap();
        builder.finish()
    }

    #[test]
    fn diff_bytes_ignore_input_order() {
        for seed in 0..30 {
            let a = random_valid_document(seed).to_records().unwrap();
            let b = random_valid_document(seed + 1000).to_records().unwrap();
            let expected = diff(&build_plain(&a), &build_plain(&b)).to_canonical_bytes();
            let shuffled = diff(&build_shuffled(&a, seed), &build_shuffled(&b, seed + 7));
            assert_eq!(shuffled.to_canonical_bytes(), expected, "seed {seed}");

            // 변경 목록 순서를 섞어 다시 만들어도 같은 bytes.
            let mut changes = shuffled.into_changes();
            changes.reverse();
            let mid = changes.len() / 2;
            changes.rotate_left(mid);
            assert_eq!(SemanticDiff::new(changes).to_canonical_bytes(), expected);
        }
    }

    #[test]
    fn canonical_order_key_is_document_object_path_kind() {
        let base = head_records();
        let mut target = base.clone();
        let rect = target.nodes.iter_mut().find(|n| n.id == sid(11)).unwrap();
        rect.parent = Some(sid(13));
        rect.visible = false;
        rect.locked = true;
        target
            .nodes
            .push(node(14, Some(10), 0xE0, "dot", rect_payload()));
        let mut builder = SnapshotBuilder::new();
        builder.add_document(&target).unwrap();
        builder
            .add_token(&TokenEntry {
                id: sid(5),
                document: None,
                name: "t".into(),
                value_type: 1,
                value: None,
                expression: None,
            })
            .unwrap();
        let changes = diff(&build(&base), &builder.finish());
        let keys: Vec<_> = changes
            .iter()
            .map(|c| {
                (
                    c.document(),
                    c.object_id(),
                    c.property_path().map(|p| p.keys()[0]),
                    c.kind(),
                )
            })
            .collect();
        assert_eq!(
            keys,
            vec![
                // 프로젝트 범위 token이 먼저.
                (None, sid(5), None, SemanticChangeKind::AddToken),
                // 같은 객체: 위치 변경 → 속성 경로 오름차순(VISIBLE 0xE001 < LOCKED 0xE002).
                (Some(sid(1)), sid(11), None, SemanticChangeKind::MoveObject),
                (
                    Some(sid(1)),
                    sid(11),
                    Some(keys::VISIBLE),
                    SemanticChangeKind::SetProperty
                ),
                (
                    Some(sid(1)),
                    sid(11),
                    Some(keys::LOCKED),
                    SemanticChangeKind::SetProperty
                ),
                (Some(sid(1)), sid(14), None, SemanticChangeKind::AddObject),
            ]
        );
    }

    #[test]
    fn diff_bytes_round_trip_and_reject_non_canonical_order() {
        let base = build(&head_records());
        let mut records = head_records();
        records.nodes.retain(|n| n.id != sid(13));
        records.nodes[1].display_name = "renamed".into();
        let changes = diff(&base, &build(&records));
        assert!(changes.len() >= 2);
        let bytes = changes.to_canonical_bytes();
        assert_eq!(SemanticDiff::from_canonical_bytes(&bytes).unwrap(), changes);

        let mut reversed: Vec<u8> = Vec::new();
        reversed.extend_from_slice(&bytes[..6]);
        let mut list = changes.changes().to_vec();
        list.reverse();
        codec::put_changes(&mut reversed, &list);
        assert_eq!(
            SemanticDiff::from_canonical_bytes(&reversed),
            Err(DecodeError::NonCanonicalOrder {
                field: "diff.changes"
            })
        );
        for cut in 0..bytes.len() {
            assert!(SemanticDiff::from_canonical_bytes(&bytes[..cut]).is_err());
        }
    }

    fn rect_payload() -> NodePayload {
        rect(2.0, 2.0)
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
