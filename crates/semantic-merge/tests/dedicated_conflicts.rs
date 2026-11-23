//! 전용 conflict fixture corpus (XIV.10.7).
//!
//! component property 삭제 대 instance override 추가, token 타입 변경 대 값 변경, drilling chain break를
//! 각각 전용 종류로 분류하는지, 그리고 전용 conflict가 아닌 이웃 경우(같은 쪽 편집, 다른 Property, 사용
//! 삭제)는 그대로 자동 병합하는지 봅니다. 모든 경우를 좌우 교환해도 같은 종류·대상이어야 합니다.

mod common;

use common::*;
use stellar_semantic_diff::keys;
use stellar_semantic_diff::snapshot::SemanticSnapshot;
use stellar_semantic_diff::value::SemanticValue;
use stellar_semantic_merge::model::{ConflictKind, ConflictTarget, MergeResult, SideState};

struct Case {
    name: &'static str,
    ours: SemanticSnapshot,
    theirs: SemanticSnapshot,
    /// 기대 conflict (종류, 객체, 대상). 비면 clean.
    expected: Vec<(ConflictKind, u64, Target)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Object,
    PropertyList,
}

fn summary(
    result: &MergeResult,
) -> Vec<(
    ConflictKind,
    stellar_semantic_diff::core_types::ids::StableId,
    Target,
)> {
    result
        .conflicts
        .iter()
        .map(|c| {
            let target = match &c.target {
                ConflictTarget::Object => Target::Object,
                ConflictTarget::Property(path) => {
                    assert_eq!(path.keys(), &[keys::COMPONENT_PROPERTIES], "{c:?}");
                    Target::PropertyList
                }
                ConflictTarget::Position => panic!("위치 conflict 없음: {c:?}"),
            };
            (c.kind, c.object, target)
        })
        .collect()
}

fn corpus() -> Vec<Case> {
    let base = world();
    let override_value = text("override");
    let removed_inner = set_properties(&base, INNER, &[INNER_PROP_2]);
    vec![
        Case {
            name: "token 타입과 값을 바꾼 쪽 대 값만 바꾼 쪽",
            ours: edit(&base, TOKEN, |o| {
                set(o, keys::TOKEN_TYPE, SemanticValue::Uint(2));
                set(o, keys::TOKEN_VALUE, text("dark"));
            }),
            theirs: edit(&base, TOKEN, |o| {
                set(o, keys::TOKEN_VALUE, SemanticValue::Uint(7))
            }),
            expected: vec![(ConflictKind::TokenTypeVsValue, TOKEN, Target::Object)],
        },
        Case {
            name: "token 타입만 대 값만",
            ours: edit(&base, TOKEN, |o| {
                set(o, keys::TOKEN_TYPE, SemanticValue::Uint(2))
            }),
            theirs: edit(&base, TOKEN, |o| {
                set(o, keys::TOKEN_VALUE, SemanticValue::Uint(7))
            }),
            expected: vec![(ConflictKind::TokenTypeVsValue, TOKEN, Target::Object)],
        },
        Case {
            name: "token 타입 대 expression",
            ours: edit(&base, TOKEN, |o| {
                set(o, keys::TOKEN_TYPE, SemanticValue::Uint(2))
            }),
            theirs: edit(&base, TOKEN, |o| {
                set(o, keys::TOKEN_EXPRESSION, SemanticValue::Id(sid(60)))
            }),
            expected: vec![(ConflictKind::TokenTypeVsValue, TOKEN, Target::Object)],
        },
        Case {
            name: "token 타입·값을 양쪽이 똑같이 바꿈",
            ours: edit(&base, TOKEN, |o| {
                set(o, keys::TOKEN_TYPE, SemanticValue::Uint(2));
                set(o, keys::TOKEN_VALUE, text("dark"));
            }),
            theirs: edit(&base, TOKEN, |o| {
                set(o, keys::TOKEN_TYPE, SemanticValue::Uint(2));
                set(o, keys::TOKEN_VALUE, text("dark"));
            }),
            expected: vec![],
        },
        Case {
            name: "token 이름 대 타입(값 변경 없음)",
            ours: edit(&base, TOKEN, |o| set(o, keys::TOKEN_NAME, text("renamed"))),
            theirs: edit(&base, TOKEN, |o| {
                set(o, keys::TOKEN_TYPE, SemanticValue::Uint(2))
            }),
            expected: vec![],
        },
        Case {
            name: "Property 삭제 대 scene 인스턴스 override 추가",
            ours: removed_inner.clone(),
            theirs: put(
                &base,
                &override_path(INSTANCE, INNER_PROP),
                override_value.clone(),
            ),
            expected: vec![(
                ConflictKind::ComponentPropertyRemovedVsOverride,
                INNER,
                Target::PropertyList,
            )],
        },
        Case {
            name: "Property 삭제 대 정의 tree nested 인스턴스 override(drilling link)",
            ours: removed_inner.clone(),
            theirs: put(
                &base,
                &override_path(NESTED, INNER_PROP),
                override_value.clone(),
            ),
            expected: vec![(
                ConflictKind::DrillingChainBreak,
                INNER,
                Target::PropertyList,
            )],
        },
        Case {
            name: "바깥 Property 삭제 대 그 Property를 참조하는 drilling binding",
            ours: set_properties(&base, OUTER, &[]),
            theirs: put(
                &base,
                &override_path(NESTED, INNER_PROP),
                SemanticValue::Id(sid(OUTER_PROP)),
            ),
            expected: vec![(
                ConflictKind::DrillingChainBreak,
                OUTER,
                Target::PropertyList,
            )],
        },
        Case {
            name: "같은 목록을 양쪽이 바꾸고 삭제한 Property를 사용(기존 목록 conflict 종류 교체)",
            ours: removed_inner.clone(),
            theirs: put(
                &set_properties(&base, INNER, &[INNER_PROP, INNER_PROP_2, 34]),
                &override_path(INSTANCE, INNER_PROP),
                override_value.clone(),
            ),
            expected: vec![(
                ConflictKind::ComponentPropertyRemovedVsOverride,
                INNER,
                Target::PropertyList,
            )],
        },
        Case {
            name: "다른 Property 삭제 대 override 추가",
            ours: set_properties(&base, INNER, &[INNER_PROP]),
            theirs: put(
                &base,
                &override_path(INSTANCE, INNER_PROP),
                override_value.clone(),
            ),
            expected: vec![],
        },
        Case {
            name: "같은 Property를 양쪽이 삭제하고 한쪽은 다른 node도 고침",
            ours: removed_inner.clone(),
            theirs: edit(&removed_inner, CHILD_A, |o| {
                set(o, keys::DISPLAY_NAME, text("a2"))
            }),
            expected: vec![],
        },
    ]
}

#[test]
fn dedicated_conflict_corpus_passes() {
    let base = world();
    for case in corpus() {
        for (label, ours, theirs) in [
            ("ours→theirs", &case.ours, &case.theirs),
            ("좌우 교환", &case.theirs, &case.ours),
        ] {
            let result = run(&base, ours, theirs);
            let expected: Vec<_> = case
                .expected
                .iter()
                .map(|(kind, object, target)| (*kind, sid(*object), *target))
                .collect();
            assert_eq!(summary(&result), expected, "{} ({label})", case.name);
        }
    }
}

#[test]
fn token_conflict_keeps_base_token_and_records_three_tokens() {
    let base = world();
    let ours = edit(&base, TOKEN, |o| {
        set(o, keys::TOKEN_TYPE, SemanticValue::Uint(2))
    });
    let theirs = edit(&base, TOKEN, |o| {
        set(o, keys::TOKEN_VALUE, SemanticValue::Uint(7))
    });
    let result = run(&base, &ours, &theirs);
    let conflict = &result.conflicts[0];
    assert_eq!(result.merged.get(sid(TOKEN)), base.get(sid(TOKEN)));
    for (side, snapshot) in [
        (&conflict.base, &base),
        (&conflict.ours, &ours),
        (&conflict.theirs, &theirs),
    ] {
        assert_eq!(
            side,
            &SideState::Object(Box::new(snapshot.get(sid(TOKEN)).unwrap().clone()))
        );
    }
}

#[test]
fn property_removal_is_held_and_usage_is_accepted() {
    let base = world();
    let ours = set_properties(&base, INNER, &[INNER_PROP_2]);
    let path = override_path(INSTANCE, INNER_PROP);
    let theirs = put(&base, &path, text("override"));
    let result = run(&base, &ours, &theirs);
    // 목록은 base(Property 있음)로 남고 override는 받아들였으므로 candidate는 interface 규칙을 지킵니다.
    let list = base
        .get(sid(INNER))
        .unwrap()
        .path(&[keys::COMPONENT_PROPERTIES]);
    assert_eq!(
        result.merged.get(sid(INNER)).unwrap().property(&list),
        base.get(sid(INNER)).unwrap().property(&list)
    );
    assert_eq!(
        result.merged.get(sid(INSTANCE)).unwrap().property(&path),
        Some(&text("override"))
    );
    let values = result.conflicts[0].property_values().unwrap();
    assert_eq!(values.ours, ours.get(sid(INNER)).unwrap().property(&list));
    assert!(stellar_semantic_diff::validate::introduced_violations(
        &result.merged,
        &[&base, &ours, &theirs]
    )
    .is_empty());
}
