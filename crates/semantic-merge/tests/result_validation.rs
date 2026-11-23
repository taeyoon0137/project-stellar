//! 병합 결과 full validator 검증 (XIV.10.8).
//!
//! 겹치지 않는 변경끼리 합쳐 참조가 끊기는 fixture(invalid cross-ref)에서 merge가 clean을 돌려주지 않고,
//! 관련 단위를 base 상태로 되돌린 candidate가 새 위반 없이 남는지 봅니다.

mod common;

use common::*;
use stellar_semantic_diff::keys;
use stellar_semantic_diff::snapshot::{
    NodeKind, ObjectKind, Origin, SemanticObject, SemanticSnapshot,
};
use stellar_semantic_diff::validate::introduced_violations;
use stellar_semantic_diff::value::SemanticValue;
use stellar_semantic_merge::model::{ConflictKind, ConflictTarget, MergeResult};

fn assert_valid_candidate(
    base: &SemanticSnapshot,
    ours: &SemanticSnapshot,
    theirs: &SemanticSnapshot,
    result: &MergeResult,
) {
    assert!(
        introduced_violations(&result.merged, &[base, ours, theirs]).is_empty(),
        "candidate 위반: {:?}",
        introduced_violations(&result.merged, &[base, ours, theirs])
    );
}

fn invalid_targets(result: &MergeResult) -> Vec<(u64, &'static str)> {
    let ids: Vec<u64> = (0..200).collect();
    result
        .conflicts
        .iter()
        .filter(|c| c.kind == ConflictKind::InvalidResult)
        .map(|c| {
            let n = *ids
                .iter()
                .find(|n| sid(**n) == c.object)
                .expect("fixture id");
            let target = match c.target {
                ConflictTarget::Object => "object",
                ConflictTarget::Position => "position",
                ConflictTarget::Property(_) => "property",
            };
            (n, target)
        })
        .collect()
}

#[test]
fn deleted_target_vs_new_reference_is_not_clean() {
    let base = world();
    let ours = {
        let mut out = base.clone();
        out.remove(sid(CHILD_B)).unwrap();
        out
    };
    let clipping = base.get(sid(ROOT)).unwrap().path(&[keys::frame::CLIPPING]);
    let theirs = put(&base, &clipping, SemanticValue::Id(sid(CHILD_B)));
    for (a, b) in [(&ours, &theirs), (&theirs, &ours)] {
        let result = run(&base, a, b);
        assert!(!result.is_clean());
        assert_eq!(
            invalid_targets(&result),
            vec![(ROOT, "property"), (CHILD_B, "object")]
        );
        // 두 단위 모두 base 상태입니다.
        assert_eq!(result.merged.get(sid(CHILD_B)), base.get(sid(CHILD_B)));
        assert_eq!(result.merged.get(sid(ROOT)), base.get(sid(ROOT)));
        assert_valid_candidate(&base, a, b, &result);
    }
}

#[test]
fn deleted_component_vs_new_instance_restores_definition_and_holds_instance() {
    let base = world();
    let ours = {
        let mut out = base.clone();
        out.remove(sid(INNER)).unwrap();
        out.remove(sid(INNER_ROOT)).unwrap();
        out
    };
    let mut instance = SemanticObject::new(
        sid(41),
        ObjectKind::Node(NodeKind::ComponentInstance),
        Origin::Document(sid(DOCUMENT)),
    )
    .with_position(Some(sid(ROOT)), Some(key(0xF0)));
    set(&mut instance, keys::DISPLAY_NAME, text("second"));
    refer(&mut instance, INNER);
    let mut theirs = base.clone();
    theirs.insert(instance).unwrap();
    let result = run(&base, &ours, &theirs);
    assert!(!result.is_clean());
    assert!(result.merged.get(sid(41)).is_none(), "새 인스턴스는 보류");
    assert!(result.merged.contains(sid(INNER)));
    assert!(result.merged.contains(sid(INNER_ROOT)));
    let targets = invalid_targets(&result);
    for expected in [(INNER, "object"), (41, "object")] {
        assert!(targets.contains(&expected), "{targets:?}");
    }
    assert_valid_candidate(&base, &ours, &theirs, &result);
}

/// 문서 1 → 페이지 2 → View 3(root = ROOT) 정의를 더한 base.
fn with_view(base: &SemanticSnapshot) -> SemanticSnapshot {
    let mut out = base.clone();
    let mut document = SemanticObject::new(
        sid(DOCUMENT),
        ObjectKind::Document,
        Origin::Document(sid(DOCUMENT)),
    );
    set(&mut document, keys::DISPLAY_NAME, text("doc"));
    set(
        &mut document,
        keys::DOCUMENT_PAGES,
        SemanticValue::Ids(vec![sid(2)]),
    );
    let mut page = SemanticObject::new(sid(2), ObjectKind::Page, Origin::Document(sid(DOCUMENT)))
        .with_position(Some(sid(DOCUMENT)), None);
    set(&mut page, keys::DISPLAY_NAME, text("page"));
    set(
        &mut page,
        keys::PAGE_VIEWS,
        SemanticValue::Ids(vec![sid(3)]),
    );
    let mut view = SemanticObject::new(sid(3), ObjectKind::View, Origin::Document(sid(DOCUMENT)))
        .with_position(Some(sid(2)), None);
    set(&mut view, keys::DISPLAY_NAME, text("view"));
    set(
        &mut view,
        keys::VIEW_ROOT_FRAME,
        SemanticValue::Id(sid(ROOT)),
    );
    for object in [document, page, view] {
        out.insert(object).unwrap();
    }
    out
}

#[test]
fn view_and_root_deletion_vs_view_edit_keeps_root_frame() {
    // SM-1 남은 한계: View는 delete-vs-modify로 남지만 root 삭제가 받아들여져 VIEW_ROOT_FRAME이 끊기던 경우.
    let base = with_view(&world());
    let ours = {
        let mut out = base.clone();
        for id in [3, ROOT, CHILD_A, CHILD_B, INSTANCE] {
            out.remove(sid(id)).unwrap();
        }
        edit(&out, 2, |o| {
            set(o, keys::PAGE_VIEWS, SemanticValue::Ids(Vec::new()))
        })
    };
    let theirs = edit(&base, 3, |o| set(o, keys::DISPLAY_NAME, text("renamed")));
    for (a, b) in [(&ours, &theirs), (&theirs, &ours)] {
        let result = run(&base, a, b);
        assert!(!result.is_clean());
        assert!(result
            .conflicts
            .iter()
            .any(|c| c.kind == ConflictKind::DeleteVsModify && c.object == sid(3)));
        assert!(result.merged.contains(sid(ROOT)), "root Frame 복원");
        assert_valid_candidate(&base, a, b, &result);
    }
}

#[test]
fn clean_merges_have_no_new_violations() {
    let base = world();
    let ours = edit(&base, CHILD_A, |o| set(o, keys::DISPLAY_NAME, text("a2")));
    let theirs = put(&base, &override_path(INSTANCE, INNER_PROP), text("v"));
    let result = run(&base, &ours, &theirs);
    assert!(result.is_clean(), "{:?}", result.conflicts);
    assert_valid_candidate(&base, &ours, &theirs, &result);
}
