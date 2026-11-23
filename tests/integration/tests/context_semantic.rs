//! Context Semantic Diff·Merge 종단 시험(XXX.7.10).

use stellar_core_types::ids::StableId;
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::context::fixtures::world::*;
use stellar_document::context::fixtures::*;
use stellar_document::context::key::{ContextKey, ContextScope};
use stellar_document::context::provision::{ContextOwner, ContextProvision};
use stellar_document::context::set::ContextSet;
use stellar_document::model::DocumentModel;
use stellar_expressions::ast::{ExprNode, ExprNodeId, ReferenceTarget};
use stellar_expressions::types::{ExpressionType, TypeSpec};
use stellar_semantic_diff::build::context::{context_from_snapshot, CONTEXT_KEY_TYPE};
use stellar_semantic_diff::build::{ExpressionNodeEntry, SnapshotBuilder};
use stellar_semantic_diff::diff::diff;
use stellar_semantic_diff::snapshot::{ObjectKind, SemanticSnapshot};
use stellar_semantic_merge::merge::merge;
use stellar_semantic_merge::model::{ConflictKind, ConflictTarget, MergeInput};

fn snapshot(
    model: &DocumentModel,
    library: &ComponentLibrary,
    set: &ContextSet,
    reads: &[(u64, u64, u64)],
) -> SemanticSnapshot {
    let mut builder = SnapshotBuilder::new();
    let records = model.to_records().unwrap();
    builder.add_document(&records).unwrap();
    builder.add_component_library(sid(DOC), &library).unwrap();
    builder.add_context(sid(DOC), set).unwrap();
    // 소비: (expression node seed, owner node, key) — owner node 아래 expression node가 key를 읽습니다.
    let entries: Vec<ExpressionNodeEntry> = reads
        .iter()
        .map(|(seed, owner, key)| {
            let id = ExprNodeId::new(sid(*seed));
            ExpressionNodeEntry {
                id: id.stable_id(),
                owner: sid(*owner),
                document: Some(sid(DOC)),
                payload: ExprNode::reference(id, ReferenceTarget::Context(sid(*key)))
                    .to_bytes()
                    .unwrap(),
                children: Vec::new(),
            }
        })
        .collect();
    builder.add_expression_nodes(&entries).unwrap();
    builder.finish()
}

#[test]
fn context_objects_project_round_trip_and_diff_bytes_are_deterministic() {
    // 검증 기준(XXX.7.10): StableId 단위 diff, diff bytes deterministic.
    let (model, library) = sample_world();
    let set = model.context().live(&model);
    let base = snapshot(&model, &library, &set, &[]);
    assert_eq!(context_from_snapshot(&base, sid(DOC)).unwrap(), set);
    let kinds: Vec<ObjectKind> = base
        .objects()
        .filter(|o| {
            matches!(
                o.kind(),
                ObjectKind::ContextKey | ObjectKind::ContextProvision
            )
        })
        .map(|o| o.kind())
        .collect();
    assert_eq!(kinds.len(), 3 + 5);
    let mut changed = set.clone();
    changed
        .replace_provision(
            changed
                .provision(sid(P_ROOT_N))
                .unwrap()
                .with_value(number(1, 12.0))
                .unwrap(),
        )
        .unwrap();
    changed.add_requirement(sid(CARD), sid(D));
    changed.remove_slot_key(sid(LABEL), sid(D));
    let target = snapshot(&model, &library, &changed, &[]);
    let first = diff(&base, &target);
    let second = diff(&base, &target);
    assert_eq!(first.to_canonical_bytes(), second.to_canonical_bytes());
    let objects: Vec<StableId> = first.iter().map(|c| c.object_id()).collect();
    assert_eq!(objects, vec![sid(CARD), sid(LABEL), sid(P_ROOT_N)]);
    assert_eq!(context_from_snapshot(&target, sid(DOC)).unwrap(), changed);
}

fn three_way(
    base: &ContextSet,
    ours: &ContextSet,
    theirs: &ContextSet,
    reads: [&[(u64, u64, u64)]; 3],
) -> stellar_semantic_merge::model::MergeResult {
    let (model, library) = sample_world();
    merge(&MergeInput {
        base: snapshot(&model, &library, base, reads[0]),
        ours: snapshot(&model, &library, ours, reads[1]),
        theirs: snapshot(&model, &library, theirs, reads[2]),
    })
    .unwrap()
}

#[test]
fn provision_removal_versus_new_consumption_is_a_dedicated_conflict() {
    // 검증 기준(XXX.7.10): 3-way fixture conflict 분류 정확(provision 삭제 대 소비 추가).
    let (model, _) = sample_world();
    let mut base = model.context().clone();
    base.insert_provision(
        ContextProvision::new(
            sid(5000),
            ContextOwner::View(sid(VIEW)),
            sid(FRAME),
            sid(D),
            number(5000, 9.0),
        )
        .unwrap(),
    )
    .unwrap();
    let mut ours = base.clone();
    ours.remove_provision(sid(5000));
    // theirs가 FRAME 아래에서 D를 새로 읽음 → conflict.
    let result = three_way(&base, &ours, &base, [&[], &[], &[(6000, FRAME, D)]]);
    let conflict: Vec<_> = result
        .conflicts
        .iter()
        .filter(|c| c.kind == ConflictKind::ContextProvisionRemovedVsConsume)
        .collect();
    assert_eq!(conflict.len(), 1);
    assert_eq!(conflict[0].object, sid(5000));
    assert_eq!(conflict[0].target, ConflictTarget::Object);
    assert!(
        result.merged.contains(sid(5000)),
        "삭제는 받아들이지 않습니다"
    );
    // provider 하위 트리 밖의 소비는 conflict가 아닙니다.
    let result = three_way(&base, &ours, &base, [&[], &[], &[(6001, CONTENT, D)]]);
    assert!(result.conflicts.is_empty(), "{:?}", result.conflicts);
    assert!(!result.merged.contains(sid(5000)));
    // 다른 key 소비도 conflict가 아닙니다.
    let result = three_way(&base, &ours, &base, [&[], &[], &[(6002, FRAME, N)]]);
    assert!(result.conflicts.is_empty());
}

#[test]
fn key_type_change_versus_provision_value_change_is_a_dedicated_conflict() {
    // 검증 기준(XXX.7.10): key 타입 변경 대 provision 값 변경.
    let (model, _) = sample_world();
    let base = model.context().clone();
    let mut ours = base.clone();
    let key = ours.key(sid(E)).unwrap().clone();
    ours.replace_key(
        ContextKey::new(
            key.id(),
            key.name(),
            ContextScope::Document(sid(DOC)),
            TypeSpec::required(ExpressionType::Boolean),
            literal(
                1,
                stellar_core_types::typed_value::TypedValue::Boolean(true),
            ),
        )
        .unwrap(),
    )
    .unwrap();
    ours.remove_provision(sid(P_ROOT_E));
    let mut theirs = base.clone();
    theirs
        .replace_provision(
            theirs
                .provision(sid(P_ROOT_E))
                .unwrap()
                .with_value(number(2, 4.0))
                .unwrap(),
        )
        .unwrap();
    // ours는 provision도 지웠으므로 삭제 대 수정(delete-vs-modify)이 먼저입니다. provision을 남긴 경우를 봅니다.
    let mut ours_keep = ours.clone();
    ours_keep
        .insert_provision(base.provision(sid(P_ROOT_E)).unwrap().clone())
        .unwrap();
    let result = three_way(&base, &ours_keep, &theirs, [&[], &[], &[]]);
    let mut found: Vec<(StableId, ConflictKind)> = result
        .conflicts
        .iter()
        .map(|c| (c.object, c.kind))
        .collect();
    found.sort();
    assert_eq!(
        found,
        vec![
            (sid(E), ConflictKind::ContextKeyTypeVsProvisionValue),
            (sid(P_ROOT_E), ConflictKind::ContextKeyTypeVsProvisionValue),
        ]
    );
    // 두 변경 모두 받아들이지 않아 merged는 base 타입·값입니다.
    let merged = context_from_snapshot(&result.merged, sid(DOC)).unwrap();
    assert_eq!(merged.key(sid(E)).unwrap().value_type(), number_type());
    assert_eq!(
        merged.provision(sid(P_ROOT_E)),
        base.provision(sid(P_ROOT_E))
    );
    let type_conflict = result
        .conflicts
        .iter()
        .find(|c| c.object == sid(E))
        .unwrap();
    assert_eq!(type_conflict.target, ConflictTarget::Object);
    let value_conflict = result
        .conflicts
        .iter()
        .find(|c| c.object == sid(P_ROOT_E))
        .unwrap();
    assert!(
        matches!(&value_conflict.target, ConflictTarget::Property(p) if p.root() == sid(P_ROOT_E))
    );
    assert_ne!(
        CONTEXT_KEY_TYPE,
        stellar_semantic_diff::build::context::CONTEXT_PROVISION_VALUE
    );
    // 같은 입력은 같은 결과(bytes)입니다.
    let again = three_way(&base, &ours_keep, &theirs, [&[], &[], &[]]);
    assert_eq!(again.to_canonical_bytes(), result.to_canonical_bytes());
    let _ = ours;
}
