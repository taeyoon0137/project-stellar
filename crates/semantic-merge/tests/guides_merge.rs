//! Frame 가이드·레이아웃 그리드 병합(ADR-0117 결정 2.4, ADR-0126).
//!
//! snapshot 투영은 가이드를 가이드 id별 속성(`[0xED01] + id`), 그리드 목록을 속성 하나(`[0xED02]`)로 둡니다. 그래서 서로
//! 다른 가이드의 동시 편집은 conflict 없이 합쳐지고, 같은 가이드를 양쪽에서 옮기거나 한쪽이 옮기고 다른 쪽이 지우면 그
//! 가이드 경로 하나가 conflict이며, 그리드 목록을 양쪽이 다르게 바꾸면 값 conflict입니다.

mod common;

use common::*;
use stellar_semantic_diff::core_types::property::PropertyPath;
use stellar_semantic_diff::guides::{NODE_GUIDE, NODE_LAYOUT_GRIDS};
use stellar_semantic_diff::snapshot::SemanticSnapshot;
use stellar_semantic_diff::value::SemanticValue;
use stellar_semantic_merge::model::ConflictTarget;

const GUIDE_A: u64 = 100;
const GUIDE_B: u64 = 101;

fn guide_path(guide: u64) -> PropertyPath {
    PropertyPath::with_keys(sid(ROOT), vec![NODE_GUIDE]).with_key_segment(&sid(guide).to_string())
}

fn grids_path() -> PropertyPath {
    PropertyPath::with_keys(sid(ROOT), vec![NODE_LAYOUT_GRIDS])
}

fn line(guide: u64, position: &str) -> SemanticValue {
    text(&format!("guide x {position} {}", sid(guide)))
}

fn base() -> SemanticSnapshot {
    let root = node(ROOT, None, 0x80, "root");
    let base = snapshot(vec![root]);
    let base = put(&base, &guide_path(GUIDE_A), line(GUIDE_A, "10px"));
    let base = put(&base, &guide_path(GUIDE_B), line(GUIDE_B, "20px"));
    put(
        &base,
        &grids_path(),
        text("grid columns 12 auto gap 20px margin 32px stretch"),
    )
}

fn without(snapshot: &SemanticSnapshot, path: &PropertyPath) -> SemanticSnapshot {
    let mut out = snapshot.clone();
    let mut object = out.remove(path.root()).unwrap();
    object.remove_property(path).unwrap();
    out.insert(object).unwrap();
    out
}

fn value(snapshot: &SemanticSnapshot, path: &PropertyPath) -> Option<SemanticValue> {
    snapshot.get(path.root()).unwrap().property(path).cloned()
}

#[test]
fn different_guides_merge_cleanly() {
    let base = base();
    let ours = put(&base, &guide_path(GUIDE_A), line(GUIDE_A, "12px"));
    let theirs = without(&base, &guide_path(GUIDE_B));
    for (a, b) in [(&ours, &theirs), (&theirs, &ours)] {
        let result = run(&base, a, b);
        assert!(result.is_clean(), "{:?}", result.conflicts);
        assert_eq!(
            value(&result.merged, &guide_path(GUIDE_A)),
            Some(line(GUIDE_A, "12px"))
        );
        assert_eq!(value(&result.merged, &guide_path(GUIDE_B)), None);
    }
    // 한쪽이 새 가이드를 더하고 다른 쪽이 그리드를 바꾸는 것도 다른 경로입니다.
    let added = put(&base, &guide_path(102), line(102, "30px"));
    let regrid = put(
        &base,
        &grids_path(),
        text("grid rows 4 auto gap 8px margin 0px stretch"),
    );
    assert!(run(&base, &added, &regrid).is_clean());
}

#[test]
fn same_guide_moved_twice_or_moved_and_removed_conflicts_on_that_guide() {
    let base = base();
    let ours = put(&base, &guide_path(GUIDE_A), line(GUIDE_A, "12px"));
    let theirs = put(&base, &guide_path(GUIDE_A), line(GUIDE_A, "14px"));
    let result = run(&base, &ours, &theirs);
    assert_eq!(result.conflicts.len(), 1, "{:?}", result.conflicts);
    assert_eq!(
        result.conflicts[0].target,
        ConflictTarget::Property(guide_path(GUIDE_A))
    );
    let removed = without(&base, &guide_path(GUIDE_A));
    let result = run(&base, &ours, &removed);
    assert_eq!(result.conflicts.len(), 1, "{:?}", result.conflicts);
    assert_eq!(
        result.conflicts[0].target,
        ConflictTarget::Property(guide_path(GUIDE_A))
    );
}

#[test]
fn grid_lists_changed_on_both_sides_conflict_as_one_value() {
    let base = base();
    let ours = put(
        &base,
        &grids_path(),
        text("grid columns 8 auto gap 16px margin 24px stretch"),
    );
    let theirs = put(
        &base,
        &grids_path(),
        text("grid columns 12 auto gap 24px margin 32px stretch"),
    );
    let result = run(&base, &ours, &theirs);
    assert_eq!(result.conflicts.len(), 1, "{:?}", result.conflicts);
    assert_eq!(
        result.conflicts[0].target,
        ConflictTarget::Property(grids_path())
    );
    // 같은 값으로 바꾸면 conflict가 아닙니다.
    assert!(run(&base, &ours, &ours).is_clean());
}
