use super::*;
use crate::merge::tests::{base, edit, set};
use crate::model::tests::{node, sid, text};
use stellar_semantic_diff::keys;
use stellar_semantic_diff::value::SemanticValue;

fn input(
    base: &SemanticSnapshot,
    ours: &SemanticSnapshot,
    theirs: &SemanticSnapshot,
) -> MergeInput {
    MergeInput {
        base: base.clone(),
        ours: ours.clone(),
        theirs: theirs.clone(),
    }
}

fn renamed(snapshot: &SemanticSnapshot, id: u64, name: &str) -> SemanticSnapshot {
    edit(snapshot, id, |o| set(o, keys::DISPLAY_NAME, text(name)))
}

fn property(
    snapshot: &SemanticSnapshot,
    id: u64,
    key: stellar_semantic_diff::core_types::property::PropertyKey,
) -> Option<SemanticValue> {
    let object = snapshot.get(sid(id))?;
    object.property(&object.path(&[key])).cloned()
}

#[test]
fn unchanged_theirs_keeps_ours() {
    let base = base();
    let ours = renamed(&base, 11, "ours");
    let result = reconcile(input(&base, &ours, &base)).unwrap();
    assert_eq!(result.name(), "keep-ours");
    assert_eq!(result.result(), Some(&ours));
    assert!(result.conflicts().is_empty());
}

#[test]
fn unchanged_or_equal_ours_takes_theirs() {
    let base = base();
    let theirs = renamed(&base, 11, "theirs");
    let result = reconcile(input(&base, &base, &theirs)).unwrap();
    assert_eq!(result.name(), "take-theirs");
    assert_eq!(result.result(), Some(&theirs));
    // 두 쪽이 같은 변경을 했으면 conflict 없이 theirs입니다.
    let same = reconcile(input(&base, &theirs, &theirs)).unwrap();
    assert_eq!(same.name(), "take-theirs");
    assert_eq!(same.result(), Some(&theirs));
}

#[test]
fn non_overlapping_changes_merge_and_validate() {
    let base = base();
    let ours = renamed(&base, 11, "ours");
    let theirs = edit(&base, 12, |o| {
        set(o, keys::VISIBLE, SemanticValue::Bool(false))
    });
    let result = reconcile(input(&base, &ours, &theirs)).unwrap();
    assert_eq!(result.name(), "merged");
    let merged = result.result().unwrap();
    assert_eq!(property(merged, 11, keys::DISPLAY_NAME), Some(text("ours")));
    assert_eq!(
        property(merged, 12, keys::VISIBLE),
        Some(SemanticValue::Bool(false))
    );
    // 같은 입력이면 같은 결과입니다.
    let again = reconcile(input(&base, &ours, &theirs)).unwrap();
    assert_eq!(again.result(), Some(merged));
}

#[test]
fn overlapping_changes_are_conflicted_without_result() {
    let base = base();
    let ours = renamed(&base, 11, "ours");
    let theirs = renamed(&base, 11, "theirs");
    let result = reconcile(input(&base, &ours, &theirs)).unwrap();
    assert_eq!(result.name(), "conflicted");
    assert_eq!(result.result(), None);
    assert_eq!(result.conflicts().len(), 1);
    assert_eq!(result.conflicts()[0].object, sid(11));
}

#[test]
fn every_writable_result_passes_the_candidate_gate() {
    // 두 쪽이 같은 parent·같은 order key로 서로 다른 새 node를 더해도, 엔진(XIV.10.8)이 결과 위반을 되돌리거나
    // conflict로 남기므로 쓸 수 있는 결과는 언제나 C 쓰기 조건(XXIII.2.4)을 통과합니다.
    let base = base();
    let mut ours = base.clone();
    ours.insert(node(20, Some(10), 0x60, &[(keys::DISPLAY_NAME, text("x"))]))
        .unwrap();
    let mut theirs = base.clone();
    theirs
        .insert(node(21, Some(10), 0x60, &[(keys::DISPLAY_NAME, text("y"))]))
        .unwrap();
    let result = reconcile(input(&base, &ours, &theirs)).unwrap();
    match &result {
        Reconciliation::Merged(candidate) => assert!(candidate.validate().is_ok()),
        Reconciliation::Conflicted(candidate) => assert!(candidate.unresolved_count() > 0),
        other => panic!("예상하지 않은 판정: {}", other.name()),
    }
}
