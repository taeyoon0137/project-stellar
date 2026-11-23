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

#[test]
fn resolve_applies_to_candidate_and_inverse_restores_it() {
    let mut c = mixed();
    let original = c.clone();
    let inverse = c
        .apply(&CandidateCommand::Resolve {
            object: sid(TOKEN),
            target: token_value(),
            resolution: Resolution::ChooseTheirs,
        })
        .unwrap();
    assert_eq!(c.unresolved_count(), 0);
    assert_eq!(c.provenance(sid(TOKEN), &token_value()), Source::B);
    let token = c.snapshot().get(sid(TOKEN)).unwrap();
    assert_eq!(
        token.property(&token.path(&[keys::TOKEN_VALUE])),
        Some(&SemanticValue::Uint(2))
    );
    assert_eq!(c.input(), original.input(), "입력 A·B 미변경");
    let redo = c.apply(&inverse).unwrap();
    assert_eq!(c, original);
    c.apply(&redo).unwrap();
    assert_eq!(c.unresolved_count(), 0);
}

#[test]
fn invalid_actions_are_rejected_without_change() {
    let mut c = mixed();
    let original = c.clone();
    assert_eq!(
        c.apply(&CandidateCommand::Resolve {
            object: sid(TOKEN),
            target: token_value(),
            resolution: Resolution::Delete,
        }),
        Err(ResolutionError::NotAllowed(ResolutionKind::Delete))
    );
    assert_eq!(
        c.apply(&CandidateCommand::Resolve {
            object: sid(CHILD_A),
            target: name_path(CHILD_A),
            resolution: Resolution::ChooseBase,
        }),
        Err(ResolutionError::UnknownConflict)
    );
    assert_eq!(c, original);
}

#[test]
fn custom_value_and_select_on_auto_merged_unit() {
    let mut c = mixed();
    c.apply(&CandidateCommand::Resolve {
        object: sid(TOKEN),
        target: token_value(),
        resolution: Resolution::CustomValue(SemanticValue::Uint(9)),
    })
    .unwrap();
    assert_eq!(c.provenance(sid(TOKEN), &token_value()), Source::Custom);
    // 자동 병합된 A 변경을 base로 되돌립니다.
    let before = c.clone();
    let inverse = c
        .apply(&CandidateCommand::Select {
            object: sid(CHILD_A),
            target: name_path(CHILD_A),
            side: Side::Base,
        })
        .unwrap();
    let child = c.snapshot().get(sid(CHILD_A)).unwrap();
    assert_eq!(
        child.property(&child.path(&[keys::DISPLAY_NAME])),
        Some(&text("a"))
    );
    assert_eq!(
        c.provenance(sid(CHILD_A), &name_path(CHILD_A)),
        Source::Base
    );
    c.apply(&inverse).unwrap();
    assert_eq!(c, before);
}

#[test]
fn removing_side_resolution_releases_overrides() {
    let base = world();
    let ours = set_properties(&base, INNER, &[INNER_PROP_2]);
    let path = override_path(INSTANCE, INNER_PROP);
    let theirs = put(&base, &path, text("v"));
    let mut c = candidate(&base, &ours, &theirs);
    let conflict = c.conflicts()[0].clone();
    assert_eq!(
        conflict.kind,
        ConflictKind::ComponentPropertyRemovedVsOverride
    );
    let original = c.clone();
    let inverse = c
        .apply(&CandidateCommand::Resolve {
            object: conflict.object,
            target: conflict.target.clone(),
            resolution: Resolution::ChooseOurs,
        })
        .unwrap();
    assert_eq!(
        c.snapshot().get(sid(INSTANCE)).unwrap().property(&path),
        None
    );
    assert_eq!(
        c.provenance(sid(INSTANCE), &ConflictTarget::Property(path.clone())),
        Source::A
    );
    c.apply(&inverse).unwrap();
    assert_eq!(c, original);
}

#[test]
fn random_command_sequences_undo_to_the_original() {
    let c0 = mixed();
    let units = [
        (sid(TOKEN), token_value()),
        (sid(CHILD_A), name_path(CHILD_A)),
        (sid(CHILD_B), name_path(CHILD_B)),
        (sid(ROOT), name_path(ROOT)),
        (sid(CHILD_A), ConflictTarget::Position),
        (sid(CHILD_B), ConflictTarget::Object),
    ];
    for seed in 0..64u64 {
        let mut c = c0.clone();
        let mut inverses = Vec::new();
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        for _ in 0..6 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let (object, target) = units[(state % units.len() as u64) as usize].clone();
            let side = [Side::Base, Side::Ours, Side::Theirs][(state >> 8) as usize % 3];
            if let Ok(inverse) = c.apply(&CandidateCommand::Select {
                object,
                target,
                side,
            }) {
                inverses.push(inverse);
            }
        }
        while let Some(inverse) = inverses.pop() {
            c.apply(&inverse).unwrap();
        }
        assert_eq!(c, c0, "seed {seed}");
    }
}

#[test]
fn write_requires_zero_unresolved_and_no_new_violation() {
    let base = world();
    let ours = set_properties(&base, INNER, &[INNER_PROP_2]);
    let path = override_path(INSTANCE, INNER_PROP);
    let theirs = put(&base, &path, text("v"));
    let mut c = candidate(&base, &ours, &theirs);
    let conflict = c.conflicts()[0].clone();
    let blocked = c.validate().unwrap_err();
    assert_eq!(
        blocked.unresolved,
        vec![(conflict.object, conflict.target.clone())]
    );
    assert!(blocked.violations.is_empty());
    c.apply(&CandidateCommand::Resolve {
        object: conflict.object,
        target: conflict.target.clone(),
        resolution: Resolution::ChooseOurs,
    })
    .unwrap();
    assert!(c.validate().is_ok());
    // 지운 Property의 override를 B에서 다시 고르면 invalid candidate이므로 쓰기를 막습니다.
    c.apply(&CandidateCommand::Select {
        object: sid(INSTANCE),
        target: ConflictTarget::Property(path.clone()),
        side: Side::Theirs,
    })
    .unwrap();
    let blocked = c.validate().unwrap_err();
    assert!(blocked.unresolved.is_empty());
    assert_eq!(blocked.violations.len(), 1, "{:?}", blocked.violations);
    // base(Property 유지)로 해결을 바꾸면 다시 쓸 수 있습니다.
    c.apply(&CandidateCommand::Resolve {
        object: conflict.object,
        target: conflict.target,
        resolution: Resolution::ChooseBase,
    })
    .unwrap();
    let validated = c.validate().unwrap();
    assert_eq!(
        validated
            .snapshot()
            .get(sid(INSTANCE))
            .unwrap()
            .property(&path),
        Some(&text("v"))
    );
}

#[test]
fn removing_side_resolution_releases_drilling_binding() {
    // 바깥 Property 삭제 대 그 Property를 참조하는 drilling binding(drilling-chain-break). 삭제 쪽으로 해결하면
    // 지운 Property를 가리키는 binding도 C에 남지 않아야 합니다(삭제 쪽 상태와 같음).
    let base = world();
    let ours = set_properties(&base, OUTER, &[]);
    let path = override_path(NESTED, INNER_PROP);
    let theirs = put(&base, &path, SemanticValue::Id(sid(OUTER_PROP)));
    let mut c = candidate(&base, &ours, &theirs);
    let conflict = c.conflicts()[0].clone();
    assert_eq!(conflict.kind, ConflictKind::DrillingChainBreak);
    let original = c.clone();
    let inverse = c
        .apply(&CandidateCommand::Resolve {
            object: conflict.object,
            target: conflict.target.clone(),
            resolution: Resolution::ChooseOurs,
        })
        .unwrap();
    let nested = c.snapshot().get(sid(NESTED)).unwrap();
    assert_eq!(
        nested.property(&path),
        ours.get(sid(NESTED)).unwrap().property(&path),
        "binding은 삭제 쪽 상태"
    );
    assert_eq!(
        c.provenance(sid(NESTED), &ConflictTarget::Property(path.clone())),
        Source::A
    );
    assert!(c.validate().is_ok());
    c.apply(&inverse).unwrap();
    assert_eq!(c, original);
    // Property를 유지하는 쪽(theirs)으로 해결하면 binding을 그대로 둡니다.
    c.apply(&CandidateCommand::Resolve {
        object: conflict.object,
        target: conflict.target,
        resolution: Resolution::ChooseTheirs,
    })
    .unwrap();
    assert_eq!(
        c.snapshot().get(sid(NESTED)).unwrap().property(&path),
        Some(&SemanticValue::Id(sid(OUTER_PROP)))
    );
}

#[test]
fn restore_is_opaque_and_bound_to_the_candidate_that_made_it() {
    // 역 command는 apply 반환값으로만 생기고(필드·생성자 비공개), 만든 C와 그 사본에만 적용됩니다(ADR-0081 개정 4).
    let mut c = mixed();
    let original = c.clone();
    let inverse = c
        .apply(&CandidateCommand::Resolve {
            object: sid(TOKEN),
            target: token_value(),
            resolution: Resolution::ChooseOurs,
        })
        .unwrap();
    let CandidateCommand::Restore(restore) = &inverse else {
        panic!("역 command는 Restore");
    };
    assert!(!restore.is_empty());
    // 같은 입력으로 따로 만든 C는 내용이 같아도 다른 candidate입니다. 해결 기록을 옮겨 심을 수 없습니다.
    let mut other = mixed();
    assert_eq!(other, original);
    assert_eq!(
        other.apply(&inverse),
        Err(ResolutionError::NotApplicable(
            "다른 candidate가 만든 역 command입니다"
        ))
    );
    assert_eq!(other, original, "거부 시 C 불변");
    assert_eq!(other.unresolved_count(), original.unresolved_count());
    // 만든 C의 사본에는 적용됩니다.
    let mut copy = c.clone();
    copy.apply(&inverse).unwrap();
    assert_eq!(copy, original);
}
