//! longhand 경로 병합(IX.11.22·IX.11.3, ADR-0075 결정 1).
//!
//! snapshot 투영은 Frame·Rectangle 모서리를 `border-*-radius` longhand 네 경로로, Shadow offset을 `shadow-offset-x`·
//! `shadow-offset-y` 두 경로로 나눕니다. 그래서 같은 객체의 서로 다른 longhand를 양쪽에서 바꾸면 conflict 없이
//! 합쳐지고, 같은 longhand를 다르게 바꾼 경우에만 그 경로 하나가 conflict입니다.

mod common;

use common::*;
use stellar_semantic_diff::core_types::property::{PropertyKey, PropertyPath};
use stellar_semantic_diff::core_types::value::FiniteF32;
use stellar_semantic_diff::keys;
use stellar_semantic_diff::snapshot::{ModifierKind, ObjectKind, Origin, SemanticObject};
use stellar_semantic_diff::value::SemanticValue;
use stellar_semantic_merge::model::ConflictTarget;

const SHADOW: u64 = 60;

fn px(value: f32) -> SemanticValue {
    SemanticValue::Floats(vec![FiniteF32::new(value).unwrap()])
}

fn base() -> stellar_semantic_diff::snapshot::SemanticSnapshot {
    let mut root = node(ROOT, None, 0x80, "root");
    for corner in keys::frame::BORDER_RADIUS_LONGHANDS {
        set(&mut root, corner, px(0.0));
    }
    let mut shadow = SemanticObject::new(
        sid(SHADOW),
        ObjectKind::Modifier(ModifierKind::Shadow),
        Origin::Document(sid(DOCUMENT)),
    )
    .with_position(Some(sid(ROOT)), Some(key(0x80)));
    for offset in keys::modifier::SHADOW_OFFSET_LONGHANDS {
        set(&mut shadow, offset, px(0.0));
    }
    snapshot(vec![root, shadow])
}

fn path(id: u64, key: PropertyKey) -> PropertyPath {
    PropertyPath::with_keys(sid(id), vec![key])
}

fn value(
    snapshot: &stellar_semantic_diff::snapshot::SemanticSnapshot,
    id: u64,
    key: PropertyKey,
) -> Option<SemanticValue> {
    snapshot
        .get(sid(id))
        .unwrap()
        .property(&path(id, key))
        .cloned()
}

#[test]
fn different_corners_of_one_frame_merge_cleanly() {
    let [tl, tr, br, bl] = keys::frame::BORDER_RADIUS_LONGHANDS;
    let base = base();
    let ours = put(&base, &path(ROOT, tl), px(8.0));
    let theirs = put(&base, &path(ROOT, br), px(4.0));
    for (a, b) in [(&ours, &theirs), (&theirs, &ours)] {
        let result = run(&base, a, b);
        assert!(result.is_clean(), "{:?}", result.conflicts);
        assert_eq!(value(&result.merged, ROOT, tl), Some(px(8.0)));
        assert_eq!(value(&result.merged, ROOT, br), Some(px(4.0)));
        assert_eq!(value(&result.merged, ROOT, tr), Some(px(0.0)));
        assert_eq!(value(&result.merged, ROOT, bl), Some(px(0.0)));
    }
    // 같은 모서리를 다르게 바꾸면 그 longhand 경로 하나만 conflict입니다.
    let other = put(&base, &path(ROOT, tl), px(2.0));
    let result = run(&base, &ours, &other);
    assert_eq!(result.conflicts.len(), 1);
    assert_eq!(
        result.conflicts[0].target,
        ConflictTarget::Property(path(ROOT, tl))
    );
}

#[test]
fn shadow_offset_x_and_y_merge_cleanly() {
    let [x, y] = keys::modifier::SHADOW_OFFSET_LONGHANDS;
    let base = base();
    let ours = put(&base, &path(SHADOW, x), px(2.0));
    let theirs = put(&base, &path(SHADOW, y), px(-3.0));
    let result = run(&base, &ours, &theirs);
    assert!(result.is_clean(), "{:?}", result.conflicts);
    assert_eq!(value(&result.merged, SHADOW, x), Some(px(2.0)));
    assert_eq!(value(&result.merged, SHADOW, y), Some(px(-3.0)));
    assert_eq!(
        value(&result.merged, SHADOW, keys::modifier::SHADOW_OFFSET),
        None
    );
}
