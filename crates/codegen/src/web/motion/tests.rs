//! XXII.2.6 검증: motion CSS·근사·미지원 진단·reduced motion·상호작용 상태 selector·state.js.

use super::*;
use crate::fixture::{self as fx, check_golden, sid, Fixture};
use crate::web::state::lower_state_machines;
use crate::web::tokens::plan_tokens;

fn property_name(path: &PropertyPath) -> Option<&'static str> {
    match path.keys().first()?.0 {
        fx::KEY_FILL => Some("background-color"),
        fx::KEY_RADIUS => Some("border-top-left-radius"),
        _ => None,
    }
}

/// 선형 진행률 표본기(test 전용: 엔진 evaluator 자리).
struct Linear;

impl EasingSampler for Linear {
    fn progress(&self, _timing: &IrTiming, duration_ms: f64, elapsed_ms: f64) -> f64 {
        elapsed_ms / duration_ms
    }
}

struct Names;

impl CssNames for Names {
    fn name(&self, _id: StableId) -> Option<String> {
        None
    }
    fn literal(&self, value: &TypedValue) -> Option<String> {
        match value {
            TypedValue::Color(c) => {
                Some(format!("c({},{},{})", num(c.r()), num(c.g()), num(c.b())))
            }
            _ => None,
        }
    }
}

#[test]
fn motions_become_transitions_with_state_selectors_and_reduced_motion() {
    // 검증 기준(XXII.2.6): fixture output. Becomes는 상태 selector 아래, reduced motion instant는 media 블록에서 뺍니다.
    let ir = Fixture::new().ir(&fx::web_profile());
    let out = lower_motions(&ir, &property_name, Some(&Linear));
    let mut text = rules_text(&out.rules);
    text.push_str("@media (prefers-reduced-motion: reduce) {\n");
    text.push_str(&rules_text(&out.reduced));
    text.push_str("}\n");
    check_golden("web_motion.css.txt", &text);
    let save = &out.rules[0];
    assert!(save
        .selector
        .contains("[data-s-00000000000000000000000000000034=\"true\"]"));
    assert_eq!(
        save.declarations[0],
        (
            "transition-property".to_string(),
            "background-color, border-top-left-radius".to_string()
        )
    );
    // spring은 sampler 표본 linear()(근사)입니다.
    assert!(out.rules[1].declarations[3]
        .1
        .starts_with("linear(0, 0.03125, "));
    // reduced motion: save(instant)는 none, grid(play)는 그대로라 media 블록에 없습니다.
    assert_eq!(out.reduced.len(), 1);
    assert_eq!(
        out.reduced[0].declarations,
        vec![("transition-property".to_string(), "none".to_string())]
    );
}

#[test]
fn approximated_and_unsupported_motion_diagnostics_are_exact() {
    // 검증 기준(XXII.2.6): 근사·미지원 진단 fixture 정확.
    let ir = Fixture::new().ir(&fx::web_profile());
    let with_sampler = lower_motions(&ir, &property_name, Some(&Linear));
    let codes = |out: &WebMotion| -> Vec<(StableId, &'static str)> {
        out.diagnostics
            .iter()
            .map(|d| (d.subject, d.code))
            .collect()
    };
    assert_eq!(
        codes(&with_sampler),
        vec![
            (sid(fx::MOTION_SAVE), "motion-color-space-approximated"),
            (sid(fx::MOTION_SAVE), "motion-start-selector"),
            (sid(fx::MOTION_GRID), "motion-color-space-approximated"),
            (sid(fx::MOTION_GRID), "motion-spring-approximated"),
        ]
    );
    // sampler가 없으면 spring은 미지원이고 그 motion은 빠집니다.
    let without = lower_motions(&ir, &property_name, None);
    assert!(codes(&without).contains(&(sid(fx::MOTION_GRID), "motion-spring-unsupported")));
    assert_eq!(without.rules.len(), 1);
    // 대상 속성 이름이 없으면 진단합니다.
    let unmapped = lower_motions(&ir, &|_| None, Some(&Linear));
    assert!(codes(&unmapped).contains(&(sid(fx::MOTION_SAVE), "motion-target-unmapped")));
    assert!(unmapped.rules.is_empty());
}

#[test]
fn interaction_tokens_go_to_control_state_selectors_not_root() {
    // 검증 기준(XXII.2.6): interaction_state 조건 토큰을 쓰는 Control의 상태별 규칙과 그 토큰이 :root 선언에 없음.
    let ir = Fixture::new().ir(&fx::web_profile());
    let plan = plan_tokens(&ir, &|v| Names.literal(v).is_some());
    assert!(plan.css.iter().all(|t| t.id != sid(fx::TOKEN_ACCENT)));
    let out = lower_interaction_tokens(&ir, &plan.interaction, &Names, &property_name);
    let selectors: Vec<&str> = out.rules.iter().map(|r| r.selector.as_str()).collect();
    assert_eq!(
        selectors,
        [
            "#s-0000000000000000000000000000000c",
            "#s-0000000000000000000000000000000c:hover"
        ]
    );
    assert_eq!(
        out.rules[1].declarations,
        vec![("background-color".to_string(), "c(0,0.4,0.9)".to_string())]
    );
    assert!(out.diagnostics.is_empty());
    // contract가 지원하지 않는 상태는 진단합니다(TextInput에는 pressed가 없음).
    assert_eq!(
        supported_states(&[ContractCode::TEXT_INPUT]),
        vec![
            IrInteractionState::Hover,
            IrInteractionState::Focused,
            IrInteractionState::Disabled
        ]
    );
    assert_eq!(state_selector(IrInteractionState::Pressed), ":active");
    assert_eq!(
        state_selector(IrInteractionState::Focused),
        ":focus-visible"
    );
}

#[test]
fn state_machines_become_event_listeners_with_atomic_commit() {
    // 검증 기준(XXII.2.6): 상태 전환은 생성 코드로 냅니다.
    let ir = Fixture::new().ir(&fx::web_profile());
    let out = lower_state_machines(&ir);
    check_golden("web_state.js", &out.script);
    assert!(out.script.contains("addEventListener(\"click\""));
    assert!(out
        .script
        .contains("commit([[\"00000000000000000000000000000034\", true]]);"));
    assert!(out
        .script
        .contains("new CustomEvent(\"stellar:000000000000000000000000000000c9\""));
    assert!(out.diagnostics.is_empty());
    // 같은 입력은 같은 출력입니다.
    assert_eq!(lower_state_machines(&ir), out);
}

#[test]
fn state_transitions_with_unavailable_references_or_events_are_diagnosed_and_skipped() {
    use crate::ir::{IrExpr, IrTrigger};
    let mut fixture = Fixture::new();
    fixture.machines[0].transitions[1].condition = Some(IrExpr::Token(sid(fx::TOKEN_SPACING)));
    fixture.machines[0].transitions[2].trigger = IrTrigger::Event {
        kind: "longPress",
        source: sid(15),
    };
    let ir = fixture.ir(&fx::web_profile());
    let out = lower_state_machines(&ir);
    let codes: Vec<(StableId, &str)> = out
        .diagnostics
        .iter()
        .map(|d| (d.subject, d.code))
        .collect();
    assert_eq!(
        codes,
        vec![
            (sid(221), "state-reference-unsupported"),
            (sid(222), "state-event-unsupported"),
        ]
    );
    assert!(!out.script.contains("read.token"));
    assert!(!out.script.contains("\"input\""));
}

#[test]
fn becomes_rule_keeps_changes_entries_and_root_target_has_no_descendant_combinator() {
    // 검증 기준(XXII.2.6): 그 상태로 들어갈 때 같은 element의 `Changes` motion이 Becomes 규칙에 가려지지 않고,
    // export root가 대상이면 상태 selector가 root 자신을 가리킵니다.
    let mut fixture = Fixture::new();
    // grid(Changes, play)를 저장 버튼 배경으로 옮겨 같은 element에 Changes와 Becomes를 둡니다.
    fixture.motions[0].targets = vec![fx::path(12, fx::KEY_RADIUS)];
    let ir = fixture.ir(&fx::web_profile());
    let out = lower_motions(&ir, &property_name, Some(&Linear));
    let becomes = out
        .rules
        .iter()
        .find(|r| r.selector.contains("[data-s-"))
        .expect("becomes rule");
    // Changes 항목이 앞, Becomes 항목이 뒤(겹치는 속성은 마지막 항목이 이김).
    assert_eq!(
        becomes.declarations[0].1,
        "border-top-left-radius, background-color, border-top-left-radius"
    );
    assert_eq!(becomes.declarations[1].1, "480ms, 200ms, 200ms");
    // reduced motion: Becomes(instant)는 빠지고 Changes(play)는 남습니다.
    let reduced = out
        .reduced
        .iter()
        .find(|r| r.selector == becomes.selector)
        .expect("reduced rule");
    assert_eq!(
        reduced.declarations[0],
        (
            "transition-property".to_string(),
            "border-top-left-radius".to_string()
        )
    );

    let mut fixture = Fixture::new();
    let root = fixture.ir(&fx::web_profile()).root().id;
    let root_byte = root.to_bytes()[15];
    fixture.motions[1].targets = vec![fx::path(root_byte, fx::KEY_FILL)];
    let ir = fixture.ir(&fx::web_profile());
    let out = lower_motions(&ir, &property_name, Some(&Linear));
    let selector = &out
        .rules
        .iter()
        .find(|r| r.selector.contains("[data-s-"))
        .expect("becomes rule")
        .selector;
    assert_eq!(
        selector,
        &format!(
            "#{}[data-s-00000000000000000000000000000034=\"true\"]",
            element_id(root)
        )
    );
}
