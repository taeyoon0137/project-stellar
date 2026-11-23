//! CandidateSnapshot C: 출처, 해결 action, command 적용·역, 쓰기 조건 (XXIII.2, XIV.12.1~XIV.12.3).

mod common;

use common::*;
use stellar_semantic_diff::keys;
use stellar_semantic_diff::value::SemanticValue;
use stellar_semantic_merge::candidate::*;
use stellar_semantic_merge::model::ConflictKind;
use stellar_semantic_merge::model::{ConflictTarget, MergeInput};

fn candidate(
    base: &stellar_semantic_diff::snapshot::SemanticSnapshot,
    ours: &stellar_semantic_diff::snapshot::SemanticSnapshot,
    theirs: &stellar_semantic_diff::snapshot::SemanticSnapshot,
) -> CandidateSnapshot {
    CandidateSnapshot::build(MergeInput {
        base: base.clone(),
        ours: ours.clone(),
        theirs: theirs.clone(),
    })
    .unwrap()
}

fn name_path(id: u64) -> ConflictTarget {
    ConflictTarget::Property(
        stellar_semantic_diff::core_types::property::PropertyPath::with_keys(
            sid(id),
            vec![keys::DISPLAY_NAME],
        ),
    )
}

/// A는 CHILD_A 이름·ROOT 이름, B는 CHILD_B 이름·ROOT 이름(같은 값), 양쪽이 TOKEN 값을 다르게.
fn mixed() -> CandidateSnapshot {
    let base = world();
    let mut ours = edit(&base, CHILD_A, |o| {
        set(o, keys::DISPLAY_NAME, text("a-ours"))
    });
    ours = edit(&ours, ROOT, |o| set(o, keys::DISPLAY_NAME, text("same")));
    ours = edit(&ours, TOKEN, |o| {
        set(o, keys::TOKEN_VALUE, SemanticValue::Uint(1))
    });
    let mut theirs = edit(&base, CHILD_B, |o| {
        set(o, keys::DISPLAY_NAME, text("b-theirs"))
    });
    theirs = edit(&theirs, ROOT, |o| set(o, keys::DISPLAY_NAME, text("same")));
    theirs = edit(&theirs, TOKEN, |o| {
        set(o, keys::TOKEN_VALUE, SemanticValue::Uint(2))
    });
    candidate(&base, &ours, &theirs)
}

fn token_value() -> ConflictTarget {
    ConflictTarget::Property(
        stellar_semantic_diff::core_types::property::PropertyPath::with_keys(
            sid(TOKEN),
            vec![keys::TOKEN_VALUE],
        ),
    )
}

#[test]
fn provenance_records_a_b_auto_and_base() {
    let c = mixed();
    assert_eq!(c.provenance(sid(CHILD_A), &name_path(CHILD_A)), Source::A);
    assert_eq!(c.provenance(sid(CHILD_B), &name_path(CHILD_B)), Source::B);
    assert_eq!(c.provenance(sid(ROOT), &name_path(ROOT)), Source::Auto);
    assert_eq!(
        c.provenance(sid(TOKEN), &token_value()),
        Source::Base,
        "conflict는 base 유지"
    );
    assert_eq!(
        c.provenance(sid(INSTANCE), &name_path(INSTANCE)),
        Source::Base,
        "변경 없음"
    );
    let recorded: Vec<_> = c
        .provenance_entries()
        .map(|(key, source)| (key.0, source))
        .collect();
    assert_eq!(
        recorded,
        vec![
            (sid(ROOT), Source::Auto),
            (sid(CHILD_A), Source::A),
            (sid(CHILD_B), Source::B),
            (sid(TOKEN), Source::Base),
        ]
    );
}

#[test]
fn allowed_actions_depend_on_target_and_states() {
    use ResolutionKind::*;
    let c = mixed();
    assert_eq!(
        allowed_resolutions(&c.conflicts()[0]),
        vec![ChooseBase, ChooseOurs, ChooseTheirs, CustomValue]
    );
    // 삭제 대 수정(객체): side 선택과 삭제, custom 없음.
    let base = world();
    let mut ours = base.clone();
    ours.remove(sid(CHILD_A)).unwrap();
    let theirs = edit(&base, CHILD_A, |o| set(o, keys::DISPLAY_NAME, text("x")));
    let deleted = candidate(&base, &ours, &theirs);
    assert_eq!(deleted.conflicts()[0].kind, ConflictKind::DeleteVsModify);
    assert_eq!(
        allowed_resolutions(&deleted.conflicts()[0]),
        vec![ChooseBase, ChooseOurs, ChooseTheirs, Delete]
    );
    // 위치 conflict: side 선택만.
    let ours = edit(&base, CHILD_A, |o| {
        o.set_position(Some(sid(ROOT)), Some(key(0x10)))
    });
    let theirs = edit(&base, CHILD_A, |o| {
        o.set_position(Some(sid(ROOT)), Some(key(0x20)))
    });
    let moved = candidate(&base, &ours, &theirs);
    assert_eq!(moved.conflicts()[0].target, ConflictTarget::Position);
    assert_eq!(
        allowed_resolutions(&moved.conflicts()[0]),
        vec![ChooseBase, ChooseOurs, ChooseTheirs]
    );
    // 한쪽이 속성을 지운 속성 conflict: delete 허용.
    let ours = edit(&base, CHILD_A, |o| {
        o.remove_property(&o.path(&[keys::DISPLAY_NAME]));
    });
    let theirs = edit(&base, CHILD_A, |o| set(o, keys::DISPLAY_NAME, text("y")));
    let removed = candidate(&base, &ours, &theirs);
    assert_eq!(
        allowed_resolutions(&removed.conflicts()[0]),
        vec![ChooseBase, ChooseOurs, ChooseTheirs, CustomValue, Delete]
    );
}
