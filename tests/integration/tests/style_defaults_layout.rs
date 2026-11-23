//! 기본값 세트 → 레이아웃 엔진 종단 적합성 (SD.2·SD.8, ADR-0082, ADR-0086).
//!
//! 문서 쪽 해석(`stellar_document::css::style_defaults`)이 푼 선언 목록을 엔진(`stellar_layout::declared`)에 넣어
//! 계산한 rect가 기대값과 같은지 봅니다.
//!
//! - `css` 세트: IX.11.8 fixture를 브라우저 UA `box-sizing`(content-box)으로 잰 기대값
//!   (`crates/layout/tests/css/expected_ua_box_sizing.json`, 생성기 `--ua-box-sizing`)과 비교합니다. fixture 선언은
//!   문서 parser(`LayoutLonghand::parse`, shorthand 확장)로 읽어 명시 값으로 둡니다. 생략한 longhand는 세트가 풉니다.
//! - `react-native` 세트: Yoga를 실행한 기대값은 없습니다. 같은 문서를 `react-native` 세트로 푼 결과와, `css` 세트
//!   위에 세트 차이(`differences`)를 명시한 결과가 node마다 같은 엔진 입력·rect인지 봅니다.

#[path = "../../../crates/layout/tests/support/json.rs"]
#[allow(dead_code)]
mod json;

use std::collections::BTreeMap;

use json::{parse, Json};
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::value::Length;
use stellar_document::css::shorthand;
use stellar_document::css::style_defaults::{self, FlatSet, NodeClass};
use stellar_document::layout_record::{LayoutLonghand, LayoutRecord};
use stellar_document::node_kind::NodeKind;
use stellar_layout::declared::style_from_declarations;
use stellar_layout::engine::compute_layout;
use stellar_layout::error::LayoutError;
use stellar_layout::input::{IntrinsicContent, LayoutContent, LayoutInput, LayoutNode};
use stellar_layout::style::LayoutStyle;

const UA_EXPECTED: &str =
    include_str!("../../../crates/layout/tests/css/expected_ua_box_sizing.json");

fn order_key(n: usize) -> OrderKey {
    let [_, hi, mid, lo] = (n as u32 + 1).to_be_bytes();
    OrderKey::from_bytes(vec![hi, mid, lo, 1]).unwrap()
}

/// fixture style 문자열을 문서 명시 record로 읽습니다. 문서가 받지 않는 선언이 있으면 `Err`(건너뜀 사유)입니다.
fn record_of(node: StableId, style: &str) -> Result<LayoutRecord, String> {
    let mut record = LayoutRecord::new(node);
    for part in style.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let (name, value) = part.split_once(':').ok_or_else(|| part.to_string())?;
        let (name, value) = (name.trim(), value.trim());
        let longhands = match shorthand::expand(name, value) {
            Some(expanded) => expanded
                .map_err(|e| format!("{name}: {e}"))?
                .into_iter()
                .map(|l| (l.name.to_string(), l.value))
                .collect(),
            None => vec![(name.to_string(), value.to_string())],
        };
        for (name, value) in longhands {
            let longhand =
                LayoutLonghand::from_name(&name).ok_or_else(|| format!("longhand {name}"))?;
            let parsed = longhand
                .parse(&value)
                .map_err(|e| format!("{name}: {value}: {e}"))?;
            record.set(longhand, parsed).map_err(|e| e.to_string())?;
        }
    }
    Ok(record)
}

fn style_of(set: &FlatSet, record: &LayoutRecord) -> LayoutStyle {
    // fixture node는 모두 `div`(Frame 구획)이고 크기가 희소한 Frame처럼 풉니다.
    let resolved = style_defaults::resolve_kind(set, NodeKind::Frame, Some(record), None).unwrap();
    let declarations = resolved.declarations();
    let list: Vec<(&str, &str)> = declarations.iter().map(|(n, v)| (*n, v.as_str())).collect();
    style_from_declarations(&list).expect("문서 정규형은 엔진이 받습니다")
}

struct Built {
    input: LayoutInput,
    ids: BTreeMap<String, StableId>,
    styles: BTreeMap<String, LayoutStyle>,
}

fn build(
    fixture: &Json,
    style: impl Fn(StableId, &str) -> Result<LayoutStyle, String>,
) -> Result<Built, String> {
    let nodes = fixture.get("nodes").as_array();
    let mut ids = BTreeMap::new();
    for (n, node) in nodes.iter().enumerate() {
        ids.insert(
            node.get("id").as_str().to_owned(),
            StableId::new_v7(n as u64 + 1, 0, n as u64 + 1),
        );
    }
    let mut out = Vec::new();
    let mut styles = BTreeMap::new();
    for node in nodes {
        let name = node.get("id").as_str();
        let id = ids[name];
        let computed = style(id, node.get("style").as_str())?;
        styles.insert(name.to_owned(), computed.clone());
        let mut layout = LayoutNode::new(id).with_style(computed);
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
    Ok(Built {
        input: LayoutInput::new(root, out),
        ids,
        styles,
    })
}

fn rects(built: &Built) -> Result<BTreeMap<String, [i64; 4]>, LayoutError> {
    let result = compute_layout(&built.input)?;
    Ok(built
        .ids
        .iter()
        .filter_map(|(name, id)| {
            result.get(*id).map(|b| {
                let r = b.local_rect();
                (
                    name.clone(),
                    [
                        i64::from(r.x().raw()),
                        i64::from(r.y().raw()),
                        i64::from(r.width().raw()),
                        i64::from(r.height().raw()),
                    ],
                )
            })
        })
        .collect())
}

fn fixtures() -> Vec<Json> {
    let document = parse(UA_EXPECTED);
    assert_eq!(document.get("layout_unit").as_f64(), 64.0);
    document.get("fixtures").as_array().to_vec()
}

#[test]
fn css_set_matches_browser_ua_box_sizing() {
    let css = style_defaults::builtin(style_defaults::CSS).unwrap();
    let mut compared = 0usize;
    let mut skipped = Vec::new();
    let mut failed = Vec::new();
    for fixture in fixtures() {
        let name = fixture.get("name").as_str().to_owned();
        if !fixture.get("exception").is_null() {
            continue;
        }
        let built = match build(&fixture, |id, text| {
            record_of(id, text).map(|r| style_of(&css, &r))
        }) {
            Ok(built) => built,
            Err(reason) => {
                skipped.push(format!("{name}: {reason}"));
                continue;
            }
        };
        let actual = match rects(&built) {
            Ok(actual) => actual,
            Err(LayoutError::Unsupported { reason, .. }) => {
                skipped.push(format!("{name}: {reason}"));
                continue;
            }
            Err(error) => {
                failed.push(format!("{name}: 계산 오류 {error}"));
                continue;
            }
        };
        let distribution = fixture.get("bundle").as_str() == "distribution";
        let nodes = fixture.get("nodes").as_array();
        let mut mismatches = Vec::new();
        for (node_id, expected) in fixture.get("expected").as_object() {
            let expected: Vec<i64> = expected
                .as_array()
                .iter()
                .map(|v| v.as_f64() as i64)
                .collect();
            let Some(actual) = actual.get(node_id) else {
                continue; // display: none 조상 아래(box 없음).
            };
            let siblings = nodes
                .iter()
                .find(|n| n.get("id").as_str() == node_id)
                .map(|n| n.get("parent"))
                .filter(|p| !p.is_null())
                .map(|p| {
                    nodes
                        .iter()
                        .filter(|n| {
                            !n.get("parent").is_null() && n.get("parent").as_str() == p.as_str()
                        })
                        .count()
                })
                .unwrap_or(1);
            let tolerance = if distribution {
                siblings.max(1) as i64
            } else {
                1
            };
            if actual
                .iter()
                .zip(&expected)
                .any(|(a, e)| (a - e).abs() > tolerance)
            {
                mismatches.push(format!("{node_id}: 기대 {expected:?} 실제 {actual:?}"));
            }
        }
        if mismatches.is_empty() {
            compared += 1;
        } else {
            failed.push(format!("{name}\n    {}", mismatches.join("\n    ")));
        }
    }
    assert!(
        failed.is_empty(),
        "{compared}개 일치, {}개 불일치:\n{}",
        failed.len(),
        failed.join("\n")
    );
    // 모든 fixture를 문서 parser로 읽고 엔진이 계산해야 합니다(건너뜀 0).
    assert!(
        skipped.is_empty() && compared >= 260,
        "비교한 fixture {compared}개, 건너뜀 {}개:\n{}",
        skipped.len(),
        skipped.join("\n")
    );
}

#[test]
fn react_native_set_equals_css_set_with_explicit_differences() {
    let css = style_defaults::builtin(style_defaults::CSS).unwrap();
    let rn = style_defaults::builtin(style_defaults::REACT_NATIVE).unwrap();
    let differences = style_defaults::differences(&rn, &css, NodeClass::Frame);
    assert!(!differences.is_empty());
    let mut compared = 0usize;
    for fixture in fixtures() {
        let as_rn = build(&fixture, |id, text| {
            record_of(id, text).map(|r| style_of(&rn, &r))
        });
        let as_explicit = build(&fixture, |id, text| {
            let mut record = record_of(id, text)?;
            for (longhand, value) in &differences {
                if record.get(*longhand).is_none() {
                    record.set(*longhand, value.clone()).unwrap();
                }
            }
            Ok(style_of(&css, &record))
        });
        let (Ok(as_rn), Ok(as_explicit)) = (as_rn, as_explicit) else {
            continue;
        };
        assert_eq!(
            as_rn.styles,
            as_explicit.styles,
            "{}",
            fixture.get("name").as_str()
        );
        match (rects(&as_rn), rects(&as_explicit)) {
            (Ok(a), Ok(b)) => assert_eq!(a, b),
            (Err(a), Err(b)) => assert_eq!(a, b),
            (a, b) => panic!("{}: {a:?} / {b:?}", fixture.get("name").as_str()),
        }
        compared += 1;
    }
    assert!(compared >= 240, "{compared}");
}

#[test]
fn react_native_defaults_lay_out_a_column_without_shrinking() {
    // RN View 기본 동작: 세로 main 축, 자식이 줄어들지 않고(flex-shrink 0) cross 축으로 늘어남(stretch).
    let rn = style_defaults::builtin(style_defaults::REACT_NATIVE).unwrap();
    let root = StableId::new_v7(1, 0, 1);
    let a = StableId::new_v7(2, 0, 2);
    let b = StableId::new_v7(3, 0, 3);
    let style = |id, text: &str| style_of(&rn, &record_of(id, text).unwrap());
    let input = LayoutInput::new(
        root,
        vec![
            LayoutNode::new(root)
                .with_style(style(root, "width:100px;height:50px"))
                .with_child(a, order_key(0))
                .with_child(b, order_key(1)),
            LayoutNode::new(a).with_style(style(a, "height:40px")),
            LayoutNode::new(b).with_style(style(b, "height:40px;padding-top:10px")),
        ],
    );
    let result = compute_layout(&input).unwrap();
    let rect = |id| {
        let r = result.get(id).unwrap().local_rect();
        [r.x().raw(), r.y().raw(), r.width().raw(), r.height().raw()].map(|v| v as f32 / 64.0)
    };
    assert_eq!(rect(a), [0.0, 0.0, 100.0, 40.0]);
    // border-box: height 40px가 padding을 포함하고, 넘쳐도 줄어들지 않습니다.
    assert_eq!(rect(b), [0.0, 40.0, 100.0, 40.0]);
}
