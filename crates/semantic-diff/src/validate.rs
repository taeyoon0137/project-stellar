//! semantic snapshot 무결성 validator (XIV.5.3).
//!
//! 선택 staging(XIV.5)·unstage(XIV.7)는 Index snapshot에 변경 일부만 적용합니다. 그 결과가 참조·구조
//! 무결성을 깨는지 이 validator로 확인합니다(아키텍처 §26.3 "참조·스키마·컴포넌트·토큰 무결성 검증").
//!
//! | 검사 | 위반 |
//! |---|---|
//! | parent 존재 | [`SnapshotViolation::MissingParent`](expression node·Variant의 owner는 binding 등 객체가 아닐 수 있어 제외) |
//! | parent cycle | [`SnapshotViolation::ParentCycle`] |
//! | 형제 OrderKey 유일(modifier stack은 별도 sequence) | [`SnapshotViolation::DuplicateOrderKey`] |
//! | 참조 대상 존재(View root, 문서 페이지·페이지 View·Slot 목록, clip·mask·boolean 대상, image asset, 컴포넌트 참조, token expression, AST child 등) | [`SnapshotViolation::DanglingReference`] |
//! | Page·View·Slot이 소속 목록에 있음 | [`SnapshotViolation::UnlistedMember`] |
//! | 부모 없는 node는 View root·컴포넌트 root | [`SnapshotViolation::OrphanRoot`] |
//! | instance override의 Property·Slot이 컴포넌트 interface에 있음 | [`SnapshotViolation::UnknownInterfaceMember`] |
//!
//! 결과는 정렬된 집합이라 결정론적입니다. 입력 snapshot에 이미 있던 위반(예: snapshot에 담지 않은 asset을
//! 가리키는 image)은 호출자가 [`introduced_violations`]로 걸러 **변경 적용이 새로 만든 위반**만 봅니다. parent
//! 사슬은 반복문으로 따라가며 방문 집합으로 끝을 보장합니다(재귀 없음).

use std::collections::{BTreeMap, BTreeSet};

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::property::{PathSegment, PropertyKey, PropertyPath};

use crate::dependency::sibling_class;
use crate::keys;
#[cfg(test)]
use crate::snapshot::Origin;
use crate::snapshot::{NodeKind, ObjectKind, SemanticObject, SemanticSnapshot};
use crate::value::SemanticValue;

/// snapshot 무결성 위반.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SnapshotViolation {
    /// parent가 snapshot에 없음.
    MissingParent {
        /// 객체.
        object: StableId,
        /// 없는 parent.
        parent: StableId,
    },
    /// parent 사슬이 순환함(cycle 구성원 중 가장 작은 id).
    ParentCycle {
        /// cycle 대표 객체.
        object: StableId,
    },
    /// 같은 parent·sequence의 형제가 같은 OrderKey를 가짐.
    DuplicateOrderKey {
        /// parent.
        parent: StableId,
        /// 중복 key.
        key: OrderKey,
        /// 그 key를 가진 형제(오름차순).
        objects: Vec<StableId>,
    },
    /// 참조 대상이 snapshot에 없음.
    DanglingReference {
        /// 참조 속성 경로.
        path: PropertyPath,
        /// 없는 대상.
        target: StableId,
    },
    /// Page·View·Slot이 소속 목록에 없음.
    UnlistedMember {
        /// 객체.
        object: StableId,
        /// 목록 owner.
        owner: StableId,
        /// 목록 key.
        list: PropertyKey,
    },
    /// 부모 없는 node가 어떤 View root·컴포넌트 root도 아님.
    OrphanRoot {
        /// node.
        node: StableId,
    },
    /// instance override가 컴포넌트 interface에 없는 Property·Slot을 가리킴.
    UnknownInterfaceMember {
        /// override 경로.
        path: PropertyPath,
        /// interface에 없는 member id.
        member: StableId,
    },
}

/// 값이 단일 객체 참조인 key.
const ID_REFERENCES: [PropertyKey; 9] = [
    keys::frame::CLIPPING,
    keys::IMAGE_ASSET,
    keys::VIEW_ROOT_FRAME,
    keys::COMPONENT_ROOT,
    keys::SLOT_HOST,
    keys::instance::COMPONENT_REF,
    keys::TOKEN_EXPRESSION,
    keys::modifier::MASK_SOURCE,
    keys::modifier::CLIP_SOURCE,
];

/// 값이 객체 참조 목록인 key.
const IDS_REFERENCES: [PropertyKey; 7] = [
    keys::DOCUMENT_PAGES,
    keys::PAGE_VIEWS,
    keys::COMPONENT_SLOTS,
    keys::COMPONENT_LOCAL_TOKENS,
    keys::SLOT_DEFAULT_CONTENT,
    keys::modifier::BOOLEAN_OPERANDS,
    keys::instance::SLOT_OVERRIDES,
];

/// 속성이 가리키는 객체 id(알려진 참조 key만, NIL 제외).
fn referenced_objects(path: &PropertyPath, value: &SemanticValue) -> Vec<StableId> {
    let Some(first) = path.keys().first().copied() else {
        return Vec::new();
    };
    let ids = match value {
        SemanticValue::Id(id) if ID_REFERENCES.contains(&first) => vec![*id],
        SemanticValue::Ids(ids) if IDS_REFERENCES.contains(&first) => ids.clone(),
        SemanticValue::Bytes(bytes) if first == keys::EXPR_CHILDREN => expression_children(bytes),
        _ => Vec::new(),
    };
    ids.into_iter().filter(|id| *id != StableId::NIL).collect()
}

/// `EXPR_CHILDREN` bytes(`u32` 개수 + (`u32` slot + 16바이트 id)*)의 대상 id. 형식이 어긋나면 읽은 데까지.
fn expression_children(bytes: &[u8]) -> Vec<StableId> {
    let Some(count) = bytes
        .get(..4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    else {
        return Vec::new();
    };
    bytes[4..]
        .chunks_exact(20)
        .take(count as usize)
        .map(|entry| {
            let mut id = [0u8; 16];
            id.copy_from_slice(&entry[4..20]);
            StableId::from_bytes(id)
        })
        .collect()
}

fn ids(object: &SemanticObject, key: PropertyKey) -> BTreeSet<StableId> {
    match object.property(&object.path(&[key])) {
        Some(SemanticValue::Ids(ids)) => ids.iter().copied().collect(),
        _ => BTreeSet::new(),
    }
}

fn id(object: &SemanticObject, key: PropertyKey) -> Option<StableId> {
    match object.property(&object.path(&[key])) {
        Some(SemanticValue::Id(id)) => Some(*id),
        _ => None,
    }
}

/// override 경로 segment의 마지막 부분(Property·Slot id)과 nested 여부(경로에 `/`가 있음).
///
/// semantic-merge의 interface member 삭제 대 사용 분류(XIV.10.7)도 같은 해석을 씁니다.
pub fn override_member(path: &PropertyPath) -> Option<(bool, StableId)> {
    let Some(PathSegment::Key(text)) = path.tail() else {
        return None;
    };
    let nested = text.contains('/');
    let last = text.rsplit('/').next()?;
    last.parse::<StableId>().ok().map(|id| (nested, id))
}

/// snapshot을 검증합니다(위반 집합, 정렬됨).
pub fn validate_snapshot(snapshot: &SemanticSnapshot) -> BTreeSet<SnapshotViolation> {
    let mut out = BTreeSet::new();
    let mut siblings: BTreeMap<(StableId, u8, &OrderKey), Vec<StableId>> = BTreeMap::new();
    let mut roots: BTreeSet<StableId> = BTreeSet::new();
    let mut all_properties: BTreeSet<StableId> = BTreeSet::new();
    for object in snapshot.objects() {
        if object.kind() == ObjectKind::View {
            roots.extend(id(object, keys::VIEW_ROOT_FRAME));
        }
        if object.kind() == ObjectKind::Component {
            roots.extend(id(object, keys::COMPONENT_ROOT));
            all_properties.extend(ids(object, keys::COMPONENT_PROPERTIES));
        }
    }

    for object in snapshot.objects() {
        // parent 존재.
        if let Some(parent) = object.parent() {
            let owner_may_be_record = matches!(
                object.kind(),
                ObjectKind::ExpressionNode | ObjectKind::Variant
            );
            if !owner_may_be_record && !snapshot.contains(parent) {
                out.insert(SnapshotViolation::MissingParent {
                    object: object.id(),
                    parent,
                });
            }
            if let Some(key) = object.order_key() {
                siblings
                    .entry((parent, sibling_class(object), key))
                    .or_default()
                    .push(object.id());
            }
        }
        // 참조 대상 존재.
        for (path, value) in object.properties() {
            for target in referenced_objects(path, value) {
                if !snapshot.contains(target) {
                    out.insert(SnapshotViolation::DanglingReference {
                        path: path.clone(),
                        target,
                    });
                }
            }
        }
        // 소속 목록.
        let membership = match (object.kind(), object.origin()) {
            (ObjectKind::Page, _) => object.parent().map(|p| (p, keys::DOCUMENT_PAGES)),
            (ObjectKind::View, _) => object.parent().map(|p| (p, keys::PAGE_VIEWS)),
            (ObjectKind::ComponentSlot, origin) => {
                origin.component().map(|id| (id, keys::COMPONENT_SLOTS))
            }
            _ => None,
        };
        if let Some((owner, list)) = membership {
            if let Some(container) = snapshot.get(owner) {
                if !ids(container, list).contains(&object.id()) {
                    out.insert(SnapshotViolation::UnlistedMember {
                        object: object.id(),
                        owner,
                        list,
                    });
                }
            }
        }
        // 부모 없는 node.
        if matches!(object.kind(), ObjectKind::Node(_))
            && object.parent().is_none()
            && !roots.contains(&object.id())
        {
            out.insert(SnapshotViolation::OrphanRoot { node: object.id() });
        }
        // instance override member.
        if object.kind() == ObjectKind::Node(NodeKind::ComponentInstance) {
            let component = id(object, keys::instance::COMPONENT_REF).and_then(|c| snapshot.get(c));
            for path in object.properties().keys() {
                let first = path.keys().first().copied();
                let Some((nested, member)) = override_member(path) else {
                    continue;
                };
                let known = if first == Some(keys::instance::PROPERTY_OVERRIDES) {
                    match (nested, component) {
                        (false, Some(component)) => {
                            Some(ids(component, keys::COMPONENT_PROPERTIES).contains(&member))
                        }
                        // nested 경로의 Property는 안쪽 컴포넌트 것이라 전체 interface에서 찾습니다.
                        (true, _) => Some(all_properties.contains(&member)),
                        (false, None) => None,
                    }
                } else if first == Some(keys::instance::SLOT_OVERRIDES) {
                    component.map(|c| ids(c, keys::COMPONENT_SLOTS).contains(&member))
                } else {
                    None
                };
                if known == Some(false) {
                    out.insert(SnapshotViolation::UnknownInterfaceMember {
                        path: path.clone(),
                        member,
                    });
                }
            }
        }
    }

    for ((parent, _, key), mut objects) in siblings {
        if objects.len() > 1 {
            objects.sort();
            out.insert(SnapshotViolation::DuplicateOrderKey {
                parent,
                key: key.clone(),
                objects,
            });
        }
    }

    // parent cycle(반복, 방문 집합).
    let mut settled: BTreeSet<StableId> = BTreeSet::new();
    for start in snapshot.objects().map(SemanticObject::id) {
        if settled.contains(&start) {
            continue;
        }
        let mut chain: Vec<StableId> = Vec::new();
        let mut on_chain: BTreeSet<StableId> = BTreeSet::new();
        let mut current = Some(start);
        while let Some(id) = current {
            if settled.contains(&id) {
                break;
            }
            if !on_chain.insert(id) {
                let first = chain.iter().position(|c| *c == id).unwrap_or(0);
                if let Some(min) = chain[first..].iter().min() {
                    out.insert(SnapshotViolation::ParentCycle { object: *min });
                }
                break;
            }
            chain.push(id);
            current = snapshot.get(id).and_then(SemanticObject::parent);
        }
        settled.extend(chain);
    }
    out
}

/// `candidate`의 위반 중 기준 snapshot들(`baselines`) 어디에도 없던 위반 — 변경 적용이 새로 만든 위반.
pub fn introduced_violations(
    candidate: &SemanticSnapshot,
    baselines: &[&SemanticSnapshot],
) -> BTreeSet<SnapshotViolation> {
    let mut violations = validate_snapshot(candidate);
    for baseline in baselines {
        if violations.is_empty() {
            break;
        }
        for existing in validate_snapshot(baseline) {
            violations.remove(&existing);
        }
    }
    violations
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::tests::{build, head_records, key, sid};
    use crate::dependency::tests::{component_base, frame, object, replace, snapshot};
    use crate::stage::tests::path;
    use stellar_document::test_support::random_document::random_valid_document;

    #[test]
    fn valid_documents_have_no_violations() {
        // HEAD fixture의 image 12는 snapshot에 담지 않은 asset 90을 가리킵니다(그 외 위반 없음).
        assert_eq!(
            validate_snapshot(&build(&head_records())),
            [SnapshotViolation::DanglingReference {
                path: path(12, keys::IMAGE_ASSET),
                target: sid(90),
            }]
            .into()
        );
        for seed in 0..30 {
            let snapshot = build(&random_valid_document(seed).to_records().unwrap());
            let violations = validate_snapshot(&snapshot);
            // 생성기 문서는 snapshot 밖 asset·clip 대상 외에는 위반이 없습니다.
            assert!(
                violations
                    .iter()
                    .all(|v| matches!(v, SnapshotViolation::DanglingReference { .. })),
                "seed {seed}: {violations:?}"
            );
        }
    }

    #[test]
    fn each_invariant_reports_its_location() {
        use SnapshotViolation::*;
        let doc = Origin::Document(sid(1));
        let base = snapshot(vec![
            object(
                1,
                ObjectKind::Document,
                doc,
                None,
                None,
                vec![(
                    path(1, keys::DOCUMENT_PAGES),
                    SemanticValue::Ids(vec![sid(2)]),
                )],
            ),
            object(
                2,
                ObjectKind::Page,
                doc,
                Some(1),
                None,
                vec![(path(2, keys::PAGE_VIEWS), SemanticValue::Ids(vec![sid(3)]))],
            ),
            object(
                3,
                ObjectKind::View,
                doc,
                Some(2),
                None,
                vec![(path(3, keys::VIEW_ROOT_FRAME), SemanticValue::Id(sid(10)))],
            ),
            frame(10, None, 0x80),
            frame(11, Some(10), 0x40),
        ]);
        assert!(validate_snapshot(&base).is_empty());

        let mut broken = base.clone();
        broken.insert(frame(12, Some(99), 0x40)).unwrap(); // 없는 parent
        broken.insert(frame(13, Some(10), 0x40)).unwrap(); // 11과 같은 key
        broken.insert(frame(14, None, 0x80)).unwrap(); // root가 아닌 부모 없는 node
        broken
            .insert(object(
                4,
                ObjectKind::Page,
                doc,
                Some(1),
                None,
                vec![(path(4, keys::PAGE_VIEWS), SemanticValue::Ids(vec![sid(98)]))],
            ))
            .unwrap(); // 목록에 없는 페이지, 없는 View 참조
                       // cycle 20 → 21 → 20.
        broken.insert(frame(20, Some(21), 0x10)).unwrap();
        broken.insert(frame(21, Some(20), 0x10)).unwrap();
        let violations = validate_snapshot(&broken);
        let expected: BTreeSet<SnapshotViolation> = [
            MissingParent {
                object: sid(12),
                parent: sid(99),
            },
            DuplicateOrderKey {
                parent: sid(10),
                key: key(0x40),
                objects: vec![sid(11), sid(13)],
            },
            OrphanRoot { node: sid(14) },
            UnlistedMember {
                object: sid(4),
                owner: sid(1),
                list: keys::DOCUMENT_PAGES,
            },
            DanglingReference {
                path: path(4, keys::PAGE_VIEWS),
                target: sid(98),
            },
            ParentCycle { object: sid(20) },
        ]
        .into();
        assert_eq!(violations, expected);
        // 이미 있던 위반은 걸러집니다.
        assert!(introduced_violations(&broken, &[&broken]).is_empty());
        assert_eq!(introduced_violations(&broken, &[&base]), expected);
    }

    #[test]
    fn instance_overrides_must_name_interface_members() {
        let base = component_base();
        // component_base의 frame 10은 View root가 아니므로 그 위반만 있습니다.
        assert_eq!(
            validate_snapshot(&base),
            [SnapshotViolation::OrphanRoot { node: sid(10) }].into()
        );
        let mut broken = base.clone();
        let mut instance = broken.get(sid(400)).unwrap().clone();
        let unknown =
            path(400, keys::instance::PROPERTY_OVERRIDES).with_key_segment(&sid(311).to_string());
        instance
            .set_property(unknown.clone(), SemanticValue::Bool(true))
            .unwrap();
        let slot =
            path(400, keys::instance::SLOT_OVERRIDES).with_key_segment(&sid(330).to_string());
        instance
            .set_property(slot.clone(), SemanticValue::Ids(vec![sid(10)]))
            .unwrap();
        // nested 경로: 다른 컴포넌트 Property 310(전체 interface에 있음)은 허용.
        let nested = path(400, keys::instance::PROPERTY_OVERRIDES).with_key_segment(&format!(
            "{}/{}",
            sid(401),
            sid(310)
        ));
        instance
            .set_property(nested, SemanticValue::Bool(true))
            .unwrap();
        replace(&mut broken, instance);
        assert_eq!(
            introduced_violations(&broken, &[&base]),
            [
                SnapshotViolation::UnknownInterfaceMember {
                    path: unknown,
                    member: sid(311)
                },
                SnapshotViolation::UnknownInterfaceMember {
                    path: slot,
                    member: sid(330)
                },
            ]
            .into()
        );
    }

    #[test]
    fn expression_children_and_nil_references() {
        let mut children = 2u32.to_le_bytes().to_vec();
        for (slot, target) in [(1u32, sid(601)), (2, sid(602))] {
            children.extend_from_slice(&slot.to_le_bytes());
            children.extend_from_slice(&target.to_bytes());
        }
        let node = object(
            600,
            ObjectKind::ExpressionNode,
            Origin::Project,
            Some(800),
            None,
            vec![
                (
                    path(600, keys::EXPR_CHILDREN),
                    SemanticValue::Bytes(children),
                ),
                (
                    path(600, keys::TOKEN_EXPRESSION),
                    SemanticValue::Id(StableId::NIL),
                ),
            ],
        );
        let alone = snapshot(vec![node.clone()]);
        // owner 800(binding record)은 객체가 아니어도 위반이 아니고, NIL 참조는 미해결 진단 대상이라 제외.
        assert_eq!(
            validate_snapshot(&alone),
            [601, 602]
                .into_iter()
                .map(|n| SnapshotViolation::DanglingReference {
                    path: path(600, keys::EXPR_CHILDREN),
                    target: sid(n),
                })
                .collect()
        );
        assert_eq!(expression_children(&[1, 0]), Vec::<StableId>::new());
    }
}
