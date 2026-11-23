//! 레이아웃 회귀 corpus runner (X.11.1, ADR-0076).
//!
//! 코드로 결정적으로 생성한 corpus(배치 방식별 50개 이상)를 schema 검증하고 실행해, 배치 방식별 요약을 작은
//! checked-in golden과 비교합니다. 생성기는 CSS 값 입력으로 바꾸기 전 corpus와 같은 seed·난수 순서를 쓰고 선택을
//! CSS longhand로 냅니다(`corpus.rs` 문서).

use std::collections::{HashMap, HashSet};

use stellar_core_types::ids::StableId;
use stellar_layout::corpus::{
    corpus_input_digest, generate_corpus, run_corpus, validate_corpus, CorpusMode, LayoutFixture,
    FIXTURE_SCHEMA_VERSION, MIN_FIXTURES_PER_MODE,
};
use stellar_layout::engine::compute_layout;
use stellar_layout::style::Display;

const GOLDEN: &str = include_str!("fixtures/layout_corpus_v2.golden");

fn golden_lines() -> Vec<&'static str> {
    GOLDEN
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
}

/// root에서 표시되는(`display: none`이 아닌) 자식만 따라 도달하는 node 수(= 결과 box 수).
fn visible_count(fixture: &LayoutFixture) -> usize {
    let nodes: HashMap<StableId, _> = fixture.input.nodes.iter().map(|n| (n.id, n)).collect();
    let mut stack = vec![fixture.input.root];
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        for child in &nodes[&id].children {
            if nodes[&child.id].style.display != Display::None {
                stack.push(child.id);
            }
        }
    }
    seen.len()
}

#[test]
fn corpus_count_schema_and_golden() {
    let corpus = generate_corpus(MIN_FIXTURES_PER_MODE);
    for mode in CorpusMode::ALL {
        let count = corpus.iter().filter(|f| f.mode == mode).count();
        assert!(count >= MIN_FIXTURES_PER_MODE, "{mode}: {count}");
    }
    assert_eq!(corpus.len(), CorpusMode::ALL.len() * MIN_FIXTURES_PER_MODE);
    validate_corpus(&corpus).expect("corpus schema");
    assert_eq!(generate_corpus(MIN_FIXTURES_PER_MODE), corpus);

    let summaries = run_corpus(&corpus).expect("모든 fixture 계산 성공");
    let mut actual = vec![
        format!("schema={FIXTURE_SCHEMA_VERSION}"),
        format!(
            "inputs={:016x}-{:016x}",
            corpus_input_digest(&corpus).raw(),
            corpus_input_digest(&corpus).check()
        ),
    ];
    actual.extend(summaries.iter().map(ToString::to_string));
    let expected: Vec<String> = golden_lines().into_iter().map(str::to_owned).collect();
    assert_eq!(
        actual,
        expected,
        "golden 불일치. 실제 값:\n{}",
        actual.join("\n")
    );
}

#[test]
fn every_fixture_lays_out_all_visible_nodes() {
    let corpus = generate_corpus(MIN_FIXTURES_PER_MODE);
    for fixture in &corpus {
        let result = compute_layout(&fixture.input)
            .unwrap_or_else(|error| panic!("{}: {error}", fixture.name));
        assert_eq!(result.len(), visible_count(fixture), "{}", fixture.name);
        let bytes = result.encode();
        assert_eq!(
            stellar_layout::output::LayoutResult::decode(&bytes).unwrap(),
            result
        );
        assert_eq!(compute_layout(&fixture.input).unwrap().encode(), bytes);
    }
}
