//! Semantic Diff/Merge conformance corpus와 runner (XXVIII.2.3).
//!
//! corpus 파일은 `conformance/`에 있습니다. 사례 입력은 아래 생성기(합성 fixture)에서 나오며, 공식 구현이 낸 기대 bytes를
//! 함께 기록합니다. `STELLAR_CONFORMANCE_BLESS=1`이면 corpus를 다시 씁니다(형식 변경 때만, 변경은 리뷰 대상).

mod common;

use std::path::PathBuf;

use common::*;
use stellar_semantic_diff::keys;
use stellar_semantic_diff::snapshot::SemanticSnapshot;
use stellar_semantic_diff::value::SemanticValue;
use stellar_semantic_merge::conformance::*;
use stellar_semantic_merge::model::{MergeInput, MergeResult};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("conformance")
}

fn input(ours: SemanticSnapshot, theirs: SemanticSnapshot) -> MergeInput {
    MergeInput {
        base: world(),
        ours,
        theirs,
    }
}

fn without(snapshot: &SemanticSnapshot, ids: &[u64]) -> SemanticSnapshot {
    let mut out = snapshot.clone();
    for id in ids {
        out.remove(sid(*id));
    }
    out
}

/// corpus 사례 생성기(이름 순서와 무관, runner가 이름순으로 읽음).
fn generated() -> Vec<(&'static str, MergeInput)> {
    let base = world();
    let rename = |id, name: &str| edit(&base, id, |o| set(o, keys::DISPLAY_NAME, text(name)));
    let place = |id, order| {
        edit(&base, id, |o| {
            o.set_position(Some(sid(ROOT)), Some(key(order)))
        })
    };
    let clipping = base.get(sid(ROOT)).unwrap().path(&[keys::frame::CLIPPING]);
    let with_child = |id: u64| {
        let mut out = base.clone();
        out.insert(node(id, Some(ROOT), 0xF0, "new")).unwrap();
        out
    };
    vec![
        (
            "clean-different-objects",
            input(rename(CHILD_A, "a2"), rename(CHILD_B, "b2")),
        ),
        (
            "clean-identical-edit",
            input(rename(CHILD_A, "same"), rename(CHILD_A, "same")),
        ),
        (
            "property-value",
            input(rename(CHILD_A, "ours"), rename(CHILD_A, "theirs")),
        ),
        (
            "delete-vs-modify",
            input(without(&base, &[CHILD_A]), rename(CHILD_A, "x")),
        ),
        (
            "conflicting-reorder",
            input(place(CHILD_A, 0x10), place(CHILD_A, 0x20)),
        ),
        (
            "independent-insertions",
            input(with_child(60), with_child(61)),
        ),
        (
            "token-type-vs-value",
            input(
                edit(&base, TOKEN, |o| {
                    set(o, keys::TOKEN_TYPE, SemanticValue::Uint(2))
                }),
                edit(&base, TOKEN, |o| {
                    set(o, keys::TOKEN_VALUE, SemanticValue::Uint(7))
                }),
            ),
        ),
        (
            "component-property-removed-vs-override",
            input(
                set_properties(&base, INNER, &[INNER_PROP_2]),
                put(&base, &override_path(INSTANCE, INNER_PROP), text("v")),
            ),
        ),
        (
            "drilling-chain-break",
            input(
                set_properties(&base, OUTER, &[]),
                put(
                    &base,
                    &override_path(NESTED, INNER_PROP),
                    SemanticValue::Id(sid(OUTER_PROP)),
                ),
            ),
        ),
        (
            "invalid-result-dangling-reference",
            input(
                without(&base, &[CHILD_B]),
                put(&base, &clipping, SemanticValue::Id(sid(CHILD_B))),
            ),
        ),
    ]
}

fn expected() -> Vec<Case> {
    let mut cases: Vec<Case> = generated()
        .into_iter()
        .map(|(name, input)| expected_case(name, input).unwrap())
        .collect();
    cases.sort_by(|a, b| a.name.cmp(&b.name));
    cases
}

#[test]
fn corpus_on_disk_matches_the_generator() {
    let dir = corpus_dir();
    if std::env::var("STELLAR_CONFORMANCE_BLESS").as_deref() == Ok("1") {
        for case in expected() {
            write_case(&dir, &case).unwrap();
        }
    }
    let loaded = load_corpus(&dir).unwrap();
    let names: Vec<_> = loaded.iter().map(|c| c.name.as_str()).collect();
    let expected = expected();
    assert_eq!(
        names,
        expected.iter().map(|c| c.name.as_str()).collect::<Vec<_>>()
    );
    for (on_disk, generated) in loaded.iter().zip(&expected) {
        assert_eq!(on_disk.input, generated.input, "{} 입력", on_disk.name);
    }
}

#[test]
fn official_implementation_passes_the_corpus() {
    let report = run_corpus(&corpus_dir(), &official()).unwrap();
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(report.verdicts.len(), generated().len());
    // corpus는 clean과 conflict 사례를 모두 담습니다.
    let cases = load_corpus(&corpus_dir()).unwrap();
    let results: Vec<MergeResult> = cases
        .iter()
        .map(|c| MergeResult::from_canonical_bytes(&c.result).unwrap())
        .collect();
    assert!(results.iter().any(MergeResult::is_clean));
    assert!(results.iter().any(|r| !r.is_clean()));
}

/// 결과에서 conflict를 지우는 잘못된 구현.
struct DropsConflicts;

impl Implementation for DropsConflicts {
    fn diff(&self, base: &SemanticSnapshot, target: &SemanticSnapshot) -> Vec<u8> {
        official().diff(base, target)
    }
    fn merge(&self, input: &MergeInput) -> Result<Vec<u8>, String> {
        let mut result = stellar_semantic_merge::merge::merge(input).map_err(|e| e.to_string())?;
        result.conflicts.clear();
        Ok(result.to_canonical_bytes())
    }
}

/// diff를 비우는 잘못된 구현.
struct EmptyDiff;

impl Implementation for EmptyDiff {
    fn diff(&self, base: &SemanticSnapshot, _: &SemanticSnapshot) -> Vec<u8> {
        official().diff(base, base)
    }
    fn merge(&self, input: &MergeInput) -> Result<Vec<u8>, String> {
        official().merge(input)
    }
}

#[test]
fn runner_reports_divergent_implementations() {
    let dir = corpus_dir();
    let dropped = run_corpus(&dir, &DropsConflicts).unwrap();
    assert!(!dropped.passed());
    assert!(dropped
        .failures()
        .iter()
        .all(|(_, verdict)| *verdict == Verdict::ResultMismatch));
    let empty = run_corpus(&dir, &EmptyDiff).unwrap();
    assert!(empty
        .failures()
        .iter()
        .all(|(_, verdict)| *verdict == Verdict::DiffMismatch));
    assert!(!empty.failures().is_empty());
}

#[test]
fn corpus_format_rejects_broken_files() {
    assert_eq!(
        from_hex(&to_hex(&[0, 1, 0xfe, 0xff])),
        Some(vec![0, 1, 0xfe, 0xff])
    );
    assert_eq!(from_hex("abc"), None);
    assert_eq!(from_hex("zz"), None);
    let dir = std::env::temp_dir().join(format!("stellar-conformance-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("lonely.input.hex"), "00").unwrap();
    assert!(matches!(load_corpus(&dir), Err(CorpusError::Incomplete(_))));
    std::fs::remove_dir_all(&dir).unwrap();
}
