//! merge 결과·conflict 순서의 결정론 (XIV.10.9).
//!
//! 입력 snapshot의 객체 삽입 순서와 conflict 목록 순서를 바꿔도 `MergeResult` canonical bytes가 같아야
//! 합니다. conflict는 `(StableId, 대상)` 순서이고 같은 키는 하나뿐입니다.

mod common;

use common::*;
use stellar_semantic_diff::keys;
use stellar_semantic_diff::snapshot::{SemanticObject, SemanticSnapshot};
use stellar_semantic_diff::value::SemanticValue;
use stellar_semantic_merge::merge::{canonical_conflicts, merge};
use stellar_semantic_merge::model::MergeInput;

/// 결정론적 순열(xorshift).
fn shuffled(mut objects: Vec<SemanticObject>, seed: u64) -> Vec<SemanticObject> {
    let mut state = seed | 1;
    for i in (1..objects.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        objects.swap(i, (state % (i as u64 + 1)) as usize);
    }
    objects
}

fn rebuilt(snapshot: &SemanticSnapshot, seed: u64) -> SemanticSnapshot {
    let objects: Vec<SemanticObject> = snapshot.objects().cloned().collect();
    let mut out = SemanticSnapshot::new();
    for object in shuffled(objects, seed) {
        out.insert(object).unwrap();
    }
    out
}

/// conflict 여러 종류와 자동 병합이 함께 나오는 입력.
fn inputs() -> (SemanticSnapshot, SemanticSnapshot, SemanticSnapshot) {
    let base = world();
    let mut ours = edit(&base, CHILD_A, |o| set(o, keys::DISPLAY_NAME, text("ours")));
    ours = edit(&ours, TOKEN, |o| {
        set(o, keys::TOKEN_TYPE, SemanticValue::Uint(9))
    });
    ours = set_properties(&ours, INNER, &[INNER_PROP_2]);
    ours.remove(sid(CHILD_B)).unwrap();
    let mut theirs = edit(&base, CHILD_A, |o| {
        set(o, keys::DISPLAY_NAME, text("theirs"))
    });
    theirs = edit(&theirs, TOKEN, |o| {
        set(o, keys::TOKEN_VALUE, SemanticValue::Uint(8))
    });
    theirs = put(&theirs, &override_path(INSTANCE, INNER_PROP), text("v"));
    theirs = edit(&theirs, CHILD_B, |o| {
        set(o, keys::VISIBLE, SemanticValue::Bool(false))
    });
    theirs = edit(&theirs, ROOT, |o| set(o, keys::DISPLAY_NAME, text("root2")));
    (base, ours, theirs)
}

#[test]
fn insertion_order_does_not_change_result_bytes() {
    let (base, ours, theirs) = inputs();
    let expected = merge(&MergeInput {
        base: base.clone(),
        ours: ours.clone(),
        theirs: theirs.clone(),
    })
    .unwrap();
    assert!(expected.conflicts.len() >= 4, "{:?}", expected.conflicts);
    let bytes = expected.to_canonical_bytes();
    for seed in 1..40 {
        let input = MergeInput {
            base: rebuilt(&base, seed),
            ours: rebuilt(&ours, seed * 3),
            theirs: rebuilt(&theirs, seed * 7),
        };
        assert_eq!(
            merge(&input).unwrap().to_canonical_bytes(),
            bytes,
            "seed {seed}"
        );
    }
}

#[test]
fn conflicts_are_sorted_by_stable_id_and_target_without_duplicates() {
    let (base, ours, theirs) = inputs();
    let result = run(&base, &ours, &theirs);
    for pair in result.conflicts.windows(2) {
        assert!(
            (pair[0].object, &pair[0].target) < (pair[1].object, &pair[1].target),
            "{pair:?}"
        );
    }
}

#[test]
fn canonical_conflicts_ignores_list_order_and_duplicates() {
    let (base, ours, theirs) = inputs();
    let result = run(&base, &ours, &theirs);
    let mut noisy = result.conflicts.clone();
    noisy.extend(result.conflicts.iter().cloned());
    noisy.reverse();
    assert_eq!(canonical_conflicts(noisy), result.conflicts);
    let mut swapped = result.conflicts.clone();
    swapped.rotate_left(1);
    assert_eq!(canonical_conflicts(swapped), result.conflicts);
}
