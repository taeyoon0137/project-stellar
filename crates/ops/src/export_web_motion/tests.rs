//! XXII.2.6 검증: `interaction_state` 조건 토큰을 쓰는 Control의 상태별 브라우저 계산값과 `:root` 미선언.
//!
//! 기대값(`fixture/expected.txt`)은 `fixture/generate.py`가 고정 버전 Chrome(DevTools `CSS.forcePseudoState`)으로
//! `fixture/control.css`와 생성 element 조각 `fixture/control.html`(구조·접근성 결과 그대로)을 읽어 만들어 커밋합니다.
//! test는 브라우저를 실행하지 않습니다.

use stellar_codegen::ir::{IrInteractionState, IrProperty, IrToken, IrTokenVariant, IrTree};
use stellar_codegen::web::aria::{element_id, map_accessibility};
use stellar_codegen::web::motion::{element_matches_state, rules_text, supported_states};
use stellar_target_profile::capability::{ContractCode, ElementKind};
use stellar_target_profile::overrides::ValueOrigin;

use super::*;
use crate::core_types::color::{Color, ColorSpace};
use crate::core_types::ids::StableId;
use crate::core_types::property::PropertyKey;
use crate::core_types::typed_value::TypedValue;
use crate::export_web_tokens::export_tokens;
use crate::export_web_tokens::tests::{id, node};

const TOKEN: u8 = 30;

fn rgb(r: u8, g: u8, b: u8) -> TypedValue {
    let c = |v: u8| f32::from(v) / 255.0;
    TypedValue::Color(Color::new(ColorSpace::Srgb, c(r), c(g), c(b), 1.0).expect("color"))
}

fn background_key() -> PropertyKey {
    registry::entries()
        .into_iter()
        .find(|e| e.scope.name() == "frame" && e.name == "background-color")
        .expect("background-color")
        .key
}

fn states() -> Vec<(IrInteractionState, TypedValue)> {
    vec![
        (IrInteractionState::Hover, rgb(238, 238, 238)),
        (IrInteractionState::Focused, rgb(221, 238, 255)),
        (IrInteractionState::Pressed, rgb(204, 204, 204)),
        (IrInteractionState::Disabled, rgb(153, 153, 153)),
    ]
}

/// root(1) 아래 Button(2), TextInput(3), contract 없는 Frame(4). 셋 다 배경이 상호작용 상태 토큰입니다.
fn fixture() -> IrTree {
    let mut root = node(1, None, 0, ElementKind::FRAME);
    root.children = vec![1, 2, 3];
    let mut nodes = vec![root];
    for (n, contract) in [
        (2, Some(ContractCode::BUTTON)),
        (3, Some(ContractCode::TEXT_INPUT)),
        (4, None),
    ] {
        let mut child = node(n, Some(0), 1, ElementKind::FRAME);
        child.contracts = contract.into_iter().collect();
        child.properties.push(IrProperty {
            path: PropertyPath::new(id(n)).push(background_key()),
            value: rgb(255, 255, 255),
            origin: ValueOrigin::Common,
            token: Some(id(TOKEN)),
            capability: None,
        });
        nodes.push(child);
    }
    let mut ir = IrTree::from_nodes(nodes).expect("tree");
    ir.tokens = vec![IrToken {
        id: id(TOKEN),
        name: "control-fill".into(),
        value: rgb(255, 255, 255),
        alias: None,
        variants: states()
            .into_iter()
            .map(|(state, value)| IrTokenVariant {
                when: vec![stellar_codegen::ir::IrEnvironmentValue::InteractionState(
                    state,
                )],
                value,
            })
            .collect(),
    }];
    ir
}

fn fixture_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/export_web_motion/fixture")
        .join(name)
}

/// 브라우저가 읽는 CSS: 토큰 CSS(이 토큰은 없음) + 상호작용 규칙.
fn page_css(ir: &IrTree) -> String {
    let tokens = export_tokens(ir);
    let behavior = export_behavior(ir, None);
    format!("{}{}", tokens.css, rules_text(&behavior.interaction.rules))
}

/// 브라우저가 읽는 element 조각: 생성 출력과 같은 구조·접근성 결과에 모든 element id를 붙인 HTML.
fn page_html(ir: &IrTree) -> String {
    let mut document = map_accessibility(ir).document;
    for index in 0..document.elements().len() as u32 {
        if let Some(source) = document.element(index).source {
            document.set_attr(index, "id", element_id(source));
        }
    }
    document.to_html()
}

/// 생성 element의 (tag, tabindex 여부).
fn final_element(ir: &IrTree, n: u8) -> (&'static str, bool) {
    let document = map_accessibility(ir).document;
    let element = document
        .elements()
        .iter()
        .find(|e| e.source == Some(id(n)))
        .expect("element");
    (element.tag, element.attr("tabindex").is_some())
}

#[test]
fn interaction_rules_match_the_committed_browser_input_and_diagnose_unsupported_states() {
    let ir = fixture();
    let css = page_css(&ir);
    let html = page_html(&ir);
    let path = fixture_path("control.css");
    let html_path = fixture_path("control.html");
    if std::env::var_os("STELLAR_UPDATE_SNAPSHOT_FIXTURES").is_some() {
        std::fs::write(&path, &css).expect("갱신");
        std::fs::write(&html_path, &html).expect("갱신");
    }
    assert_eq!(css, std::fs::read_to_string(&path).expect("control.css"));
    assert_eq!(
        html,
        std::fs::read_to_string(&html_path).expect("control.html")
    );
    // 토큰은 :root 선언에 없습니다.
    assert!(!css.contains("--control-fill"));
    let behavior = export_behavior(&ir, None);
    let codes: Vec<(StableId, &str)> = behavior
        .interaction
        .diagnostics
        .iter()
        .map(|d| (d.subject, d.code))
        .collect();
    assert_eq!(
        codes,
        vec![
            // 접근성 entry가 없는 tree라 control은 숨김 `div`입니다(XXII.2.4). `:focus-visible`·`:disabled`가 맞지
            // 않아 그 상태 값은 규칙 없이 진단합니다.
            (id(2), "interaction-state-element-unsupported"),
            (id(3), "interaction-state-element-unsupported"),
            (id(3), "interaction-state-unsupported"),
            (id(4), "interaction-state-no-control"),
        ]
    );
}

fn computed_text(value: &TypedValue) -> String {
    match value {
        TypedValue::Color(c) => format!(
            "rgb({}, {}, {})",
            (c.r() * 255.0).round(),
            (c.g() * 255.0).round(),
            (c.b() * 255.0).round()
        ),
        other => panic!("fixture에 없는 타입 {other:?}"),
    }
}

#[test]
fn control_state_computed_values_in_the_browser_follow_the_contract_states() {
    // 검증 기준(XXII.2.6): interaction_state 조건 토큰을 쓰는 Control의 상태별 계산값 fixture.
    let ir = fixture();
    let expected = std::fs::read_to_string(fixture_path("expected.txt")).expect("expected.txt");
    let token = &ir.tokens[0];
    let mut checked = 0;
    for line in expected
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let mut parts = line.splitn(3, ' ');
        let n: u8 = parts.next().and_then(|v| v.parse().ok()).expect("node");
        let state = parts.next().expect("state");
        let actual = parts.next().unwrap_or("");
        if n == 1 {
            assert_eq!(state, "--control-fill");
            assert_eq!(actual, "", ":root에 토큰이 선언되었습니다");
            checked += 1;
            continue;
        }
        let node = ir.node(id(n)).expect("node");
        let state = match state {
            "default" => IrInteractionState::Default,
            "hover" => IrInteractionState::Hover,
            "focus-visible" => IrInteractionState::Focused,
            "active" => IrInteractionState::Pressed,
            "disabled" => IrInteractionState::Disabled,
            other => panic!("모르는 상태 {other}"),
        };
        let (tag, tabindex) = final_element(&ir, n);
        let value = if supported_states(&node.contracts).contains(&state)
            && element_matches_state(tag, tabindex, state)
        {
            token
                .variants
                .iter()
                .find(|v| {
                    v.when
                        == vec![stellar_codegen::ir::IrEnvironmentValue::InteractionState(
                            state,
                        )]
                })
                .map_or(&token.value, |v| &v.value)
        } else {
            &token.value
        };
        assert_eq!(actual, computed_text(value), "node {n} {state:?}");
        checked += 1;
    }
    assert_eq!(checked, 3 * 5 + 1);
}

#[test]
fn css_property_names_come_from_exact_registry_entries() {
    let path = PropertyPath::new(id(2)).push(background_key());
    assert_eq!(css_property_name(&path), Some("background-color"));
    let clipping = registry::entries()
        .into_iter()
        .find(|e| e.name == "clipping")
        .expect("clipping");
    assert_eq!(
        css_property_name(&PropertyPath::new(id(2)).push(clipping.key)),
        None
    );
}
