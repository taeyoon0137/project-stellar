//! CSS 레이아웃 적합성 fixture (IX.11.8, ADR-0076).
//!
//! `tests/css/expected.json`은 `tests/css/generate.py`가 고정 버전 Chromium 계열 브라우저로 생성한 기대 border box
//! rect(1/64 px LayoutUnit, 부모 border box 원점 기준)입니다. 테스트는 브라우저를 실행하지 않고 같은 CSS 선언을
//! 엔진 입력으로 바꿔 계산한 rect와 비교합니다.
//!
//! - `basic` 묶음: 1 LayoutUnit 안에서 같아야 합니다.
//! - `distribution` 묶음: 같은 줄(부모의 표시되는 자식) item 수 이하 LayoutUnit 차이를 허용합니다.

mod support;

use std::collections::BTreeMap;

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::value::Length;
use stellar_layout::engine::compute_layout;
use stellar_layout::input::{IntrinsicContent, LayoutContent, LayoutInput, LayoutNode};
use stellar_layout::style::Display;

use support::css::parse_style;
use support::json::{parse, Json};

const EXPECTED: &str = include_str!("css/expected.json");

/// fixture 하나의 비교 결과.
struct Outcome {
    name: String,
    mismatches: Vec<String>,
}

fn order_key(n: usize) -> OrderKey {
    let [_, hi, mid, lo] = (n as u32 + 1).to_be_bytes();
    OrderKey::from_bytes(vec![hi, mid, lo, 1]).unwrap()
}

/// fixture node 목록을 엔진 입력으로 바꿉니다. id는 목록 순번입니다.
fn build(fixture: &Json) -> (LayoutInput, BTreeMap<String, StableId>) {
    let nodes = fixture.get("nodes").as_array();
    let mut ids = BTreeMap::new();
    for (n, node) in nodes.iter().enumerate() {
        ids.insert(
            node.get("id").as_str().to_owned(),
            StableId::new_v7(n as u64 + 1, 0, n as u64 + 1),
        );
    }
    let mut out = Vec::new();
    for node in nodes {
        let id = ids[node.get("id").as_str()];
        let mut layout = LayoutNode::new(id).with_style(parse_style(node.get("style").as_str()));
        for (k, child) in node.get("children").as_array().iter().enumerate() {
            layout = layout.with_child(ids[child.as_str()], order_key(k));
        }
        let content = node.get("content");
        if !content.is_null() {
            let px = |v: &Json| Length::new(v.as_f64() as f32).unwrap();
            let baseline = content.get("baseline");
            layout = layout.with_content(LayoutContent::Intrinsic(IntrinsicContent {
                width: px(content.get("width")),
                height: px(content.get("height")),
                baseline: (!baseline.is_null()).then(|| px(baseline)),
            }));
        }
        out.push(layout);
    }
    let root = ids[nodes[0].get("id").as_str()];
    (LayoutInput::new(root, out), ids)
}

fn run(fixture: &Json) -> Outcome {
    let name = fixture.get("name").as_str().to_owned();
    let distribution = fixture.get("bundle").as_str() == "distribution";
    let (input, ids) = build(fixture);
    let result = match compute_layout(&input) {
        Ok(result) => result,
        Err(error) => {
            return Outcome {
                name,
                mismatches: vec![format!("계산 오류: {error}")],
            }
        }
    };
    let nodes = fixture.get("nodes").as_array();
    let shown_children = |parent: &str| -> usize {
        nodes
            .iter()
            .filter(|n| !n.get("parent").is_null() && n.get("parent").as_str() == parent)
            .filter(|n| parse_style(n.get("style").as_str()).display != Display::None)
            .count()
    };
    let mut mismatches = Vec::new();
    for (node_id, expected) in fixture.get("expected").as_object() {
        let expected: Vec<i64> = expected
            .as_array()
            .iter()
            .map(|v| v.as_f64() as i64)
            .collect();
        let node = nodes
            .iter()
            .find(|n| n.get("id").as_str() == node_id)
            .unwrap();
        let hidden_ancestor = {
            let mut current = Some(node);
            let mut hidden = false;
            while let Some(n) = current {
                if parse_style(n.get("style").as_str()).display == Display::None {
                    hidden = true;
                }
                current = if n.get("parent").is_null() {
                    None
                } else {
                    nodes
                        .iter()
                        .find(|p| p.get("id").as_str() == n.get("parent").as_str())
                };
            }
            hidden
        };
        let actual = result.get(ids[node_id]);
        if hidden_ancestor {
            if actual.is_some() {
                mismatches.push(format!("{node_id}: display:none인데 box가 있습니다"));
            }
            continue;
        }
        let Some(actual) = actual else {
            mismatches.push(format!("{node_id}: box가 없습니다"));
            continue;
        };
        let rect = actual.local_rect();
        let actual = [
            i64::from(rect.x().raw()),
            i64::from(rect.y().raw()),
            i64::from(rect.width().raw()),
            i64::from(rect.height().raw()),
        ];
        let tolerance = if distribution {
            let parent = node.get("parent");
            if parent.is_null() {
                1
            } else {
                shown_children(parent.as_str()).max(1) as i64
            }
        } else {
            1
        };
        let off = actual
            .iter()
            .zip(&expected)
            .any(|(a, e)| (a - e).abs() > tolerance);
        if off {
            let px = |v: &[i64]| {
                v.iter()
                    .map(|x| format!("{}", *x as f64 / 64.0))
                    .collect::<Vec<_>>()
                    .join(",")
            };
            mismatches.push(format!(
                "{node_id}: 기대 [{}] 실제 [{}]",
                px(&expected),
                px(&actual)
            ));
        }
    }
    Outcome { name, mismatches }
}

fn fixtures() -> Vec<Json> {
    let document = parse(EXPECTED);
    assert_eq!(document.get("schema").as_f64(), 1.0);
    assert_eq!(document.get("layout_unit").as_f64(), 64.0);
    assert_eq!(
        document.get("root_available_px").as_f64() as i32,
        stellar_layout::engine::ROOT_AVAILABLE_PX
    );
    document.get("fixtures").as_array().to_vec()
}

#[test]
fn css_fixtures_match_browser_border_boxes() {
    let mut failed = Vec::new();
    let mut passed = 0;
    for fixture in fixtures() {
        let outcome = run(&fixture);
        // 속성 모델 14장 확장(예외 표시 fixture)은 CSS와 다르게 정한 항목이라 일치를 요구하지 않습니다. 예외 표시가
        // 있는데 일치하면 예외가 더는 필요 없다는 뜻이므로 실패로 알립니다.
        let exception = fixture.get("exception");
        if !exception.is_null() {
            if outcome.mismatches.is_empty() {
                failed.push(format!(
                    "{}: 예외({})인데 브라우저와 일치합니다",
                    outcome.name,
                    exception.as_str()
                ));
            }
            continue;
        }
        if outcome.mismatches.is_empty() {
            passed += 1;
        } else {
            failed.push(format!(
                "{}\n    {}",
                outcome.name,
                outcome.mismatches.join("\n    ")
            ));
        }
    }
    assert!(
        failed.is_empty(),
        "{}개 일치, {}개 불일치:\n{}",
        passed,
        failed.len(),
        failed.join("\n")
    );
}

#[test]
fn required_fixture_categories_exist() {
    // 계획 IX.11.8이 명시한 범주.
    let names: Vec<String> = fixtures()
        .iter()
        .map(|f| f.get("name").as_str().to_owned())
        .collect();
    for required in [
        "block/aspect-ratio-both-indefinite-absolute",
        "flex/aspect-ratio-both-indefinite-item",
        "block/aspect-ratio-content-overflow-min-auto",
        "flex/negative-free-space-space-around",
        "flex/negative-free-space-space-evenly",
        "grid/bare-fr-grows-to-content-min",
        "grid/fr-sum-below-one-single",
        "grid/fr-sum-below-one-pair",
        "grid/fr-sum-below-one-indefinite",
        "flex/default-align-fit-content-child-not-stretched",
        "flex/auto-min-content-prevents-shrink",
        "flex/baseline-mixed-text-image",
    ] {
        assert!(names.iter().any(|n| n == required), "{required} 없음");
    }
    let distribution = fixtures()
        .iter()
        .filter(|f| f.get("bundle").as_str() == "distribution")
        .count();
    assert!(distribution >= 5, "{distribution}");
}

#[test]
fn css_fixture_counts_meet_minimums() {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for fixture in fixtures() {
        *counts
            .entry(fixture.get("suite").as_str().to_owned())
            .or_default() += 1;
    }
    let count = |suite: &str| counts.get(suite).copied().unwrap_or(0);
    assert!(count("flex") >= 60, "{counts:?}");
    assert!(count("positioning") >= 30, "{counts:?}");
    assert!(count("grid") >= 40, "{counts:?}");
}

#[test]
fn css_fixture_inputs_are_deterministic() {
    for fixture in fixtures() {
        let (a, _) = build(&fixture);
        let (b, _) = build(&fixture);
        assert_eq!(a.encode(), b.encode());
        let first = compute_layout(&a).map(|r| r.encode());
        let second = compute_layout(&b).map(|r| r.encode());
        assert_eq!(first, second, "{}", fixture.get("name").as_str());
    }
}
