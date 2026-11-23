//! `MotionSpec`·상호작용 상태 토큰 → Web CSS (XXII.2.6, ADR-0106 결정 7).
//!
//! - motion은 대상 element 규칙의 `transition-*` longhand입니다. `Becomes` 시작 조건은 export root의
//!   `[data-s-<상태 hex>="<값>"]` 아래 규칙(그 상태로 들어갈 때만 재생), `Changes`는 element 규칙입니다.
//! - timing function과 진단은 [`crate::matrix::behavior`] 표만 따릅니다. spring은 호출자가 넘기는 [`EasingSampler`]
//!   (엔진 evaluator)로 `linear()` 표본을 냅니다. sampler가 없으면 미지원 진단을 내고 그 motion을 뺍니다.
//! - reduced motion `instant`는 `@media (prefers-reduced-motion: reduce)` 안에서 그 motion을 뺀 목록(없으면 `none`)을,
//!   `play`는 그대로 둡니다.
//! - `interaction_state` 조건부 토큰을 쓰는 Control element 속성은 element 규칙에 기본 값, 상태 selector에 상태 값을
//!   냅니다. 대응: hover `:hover`, focused `:focus-visible`, pressed `:active`, disabled `:disabled`. Button은 네 상태,
//!   TextInput은 hover·focused·disabled, 그 밖의 contract는 hover·focused를 지원합니다.

use std::collections::BTreeMap;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_core_types::typed_value::TypedValue;
use stellar_target_profile::capability::ContractCode;
use stellar_target_profile::profile::TargetProfileKind;

use crate::ir::snapshot::id_hex;
use crate::ir::{IrInteractionState, IrMotion, IrMotionStart, IrReducedMotion, IrTiming, IrTree};
use crate::matrix::behavior::{behavior_entry, BehaviorSubject, MotionFeature};
use crate::text::num;
use crate::web::aria::element_id;
use crate::web::expr::CssNames;
use crate::web::tokens::InteractionToken;

/// spring 같은 timing function의 진행률 표본기(엔진 evaluator, 호출자 구현).
pub trait EasingSampler {
    /// `elapsed_ms` 시점의 진행률(0 시작, 1 끝).
    fn progress(&self, timing: &IrTiming, duration_ms: f64, elapsed_ms: f64) -> f64;
}

/// `linear()` 표본 수(구간 수). 끝점 포함 `SPRING_SAMPLES + 1`개입니다.
pub const SPRING_SAMPLES: u32 = 32;

/// CSS 규칙 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssRule {
    /// selector.
    pub selector: String,
    /// (속성, 값).
    pub declarations: Vec<(String, String)>,
}

/// motion·상호작용 진단.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MotionDiagnostic {
    /// 대상(motion id 또는 element node).
    pub subject: StableId,
    /// 안정 code.
    pub code: &'static str,
}

/// Web 출력.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WebMotion {
    /// 규칙(출력 순).
    pub rules: Vec<CssRule>,
    /// `@media (prefers-reduced-motion: reduce)` 안 규칙.
    pub reduced: Vec<CssRule>,
    /// 진단(대상·code 순).
    pub diagnostics: Vec<MotionDiagnostic>,
}

/// 상태 값의 data 속성 텍스트(`state.js`와 selector가 같은 함수를 씁니다).
pub fn state_attr_value(value: &TypedValue) -> String {
    match value {
        TypedValue::Boolean(flag) => flag.to_string(),
        TypedValue::Number(n) => num(n.get()),
        TypedValue::String(text) => text.clone(),
        TypedValue::Enum(e) => format!("{}:{}", e.enum_type(), e.variant()),
        other => crate::ir::snapshot::value_text(other),
    }
}

/// 상태 변수의 data 속성 이름.
pub fn state_attr_name(state: StableId) -> String {
    format!("data-s-{}", id_hex(state))
}

/// CSS 문자열(따옴표 포함, `"`·`\`·줄바꿈 escape).
pub fn css_string(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\a "),
            c if (c as u32) < 0x20 || c == '\u{7f}' => out.push_str(&format!("\\{:x} ", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn diag(subject: StableId, subject_kind: BehaviorSubject) -> Option<MotionDiagnostic> {
    behavior_entry(subject_kind, TargetProfileKind::Web)
        .and_then(|e| e.diagnostic)
        .map(|code| MotionDiagnostic { subject, code })
}

fn timing_css(motion: &IrMotion, sampler: Option<&dyn EasingSampler>) -> Option<String> {
    Some(match motion.timing {
        IrTiming::Linear => "linear".to_string(),
        IrTiming::CubicBezier { x1, y1, x2, y2 } => format!(
            "cubic-bezier({}, {}, {}, {})",
            num(x1),
            num(y1),
            num(x2),
            num(y2)
        ),
        IrTiming::Steps { count, position } => format!("steps({count}, {})", position.css_name()),
        IrTiming::Spring { .. } => {
            let sampler = sampler?;
            let duration = f64::from(motion.duration_ms);
            let points: Vec<String> = (0..=SPRING_SAMPLES)
                .map(|i| {
                    let t = duration * f64::from(i) / f64::from(SPRING_SAMPLES);
                    let p = sampler.progress(&motion.timing, duration, t);
                    // 표본은 f32 최단 표기(결정론).
                    num(p as f32)
                })
                .collect();
            format!("linear({})", points.join(", "))
        }
    })
}

/// 한 selector의 transition 항목.
#[derive(Debug, Clone)]
struct Entry {
    property: &'static str,
    duration: String,
    delay: String,
    timing: String,
    play_reduced: bool,
}

fn transition_declarations(entries: &[&Entry]) -> Vec<(String, String)> {
    let join =
        |f: &dyn Fn(&Entry) -> String| entries.iter().map(|e| f(e)).collect::<Vec<_>>().join(", ");
    vec![
        (
            "transition-property".into(),
            join(&|e| e.property.to_string()),
        ),
        ("transition-duration".into(), join(&|e| e.duration.clone())),
        ("transition-delay".into(), join(&|e| e.delay.clone())),
        (
            "transition-timing-function".into(),
            join(&|e| e.timing.clone()),
        ),
    ]
}

/// motion을 Web CSS로 낮춥니다. `property_name`은 대상 경로의 CSS longhand 이름(문서 registry)입니다.
pub fn lower_motions(
    ir: &IrTree,
    property_name: &dyn Fn(&PropertyPath) -> Option<&'static str>,
    sampler: Option<&dyn EasingSampler>,
) -> WebMotion {
    let mut out = WebMotion::default();
    let root = element_id(ir.root().id);
    // (selector) → (대상 element, Becomes 여부, 항목). 입력 순: motion id 순, 대상 순.
    let mut groups: BTreeMap<String, (StableId, bool, Vec<Entry>)> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for motion in &ir.motions {
        let timing_feature = MotionFeature::of_timing(&motion.timing);
        let Some(timing) = timing_css(motion, sampler) else {
            out.diagnostics.push(MotionDiagnostic {
                subject: motion.id,
                code: "motion-spring-unsupported",
            });
            continue;
        };
        out.diagnostics
            .extend(diag(motion.id, BehaviorSubject::Motion(timing_feature)));
        let start_feature = match motion.start {
            IrMotionStart::Becomes { .. } => MotionFeature::StartBecomes,
            IrMotionStart::Changes { .. } => MotionFeature::StartChanges,
        };
        out.diagnostics
            .extend(diag(motion.id, BehaviorSubject::Motion(start_feature)));
        for target in &motion.targets {
            let Some(property) = property_name(target) else {
                out.diagnostics.push(MotionDiagnostic {
                    subject: motion.id,
                    code: "motion-target-unmapped",
                });
                continue;
            };
            if matches!(
                ir.property(target).map(|p| &p.value),
                Some(TypedValue::Color(_))
            ) {
                out.diagnostics.extend(diag(
                    motion.id,
                    BehaviorSubject::Motion(MotionFeature::ColorInterpolation),
                ));
            }
            let element = element_id(target.root());
            let (selector, becomes) = match &motion.start {
                IrMotionStart::Becomes { state, value } => {
                    let condition = format!(
                        "#{root}[{}={}]",
                        state_attr_name(*state),
                        css_string(&state_attr_value(value))
                    );
                    // 상태 속성은 export root에 있으므로 root 자신이 대상이면 자손 결합자를 쓰지 않습니다.
                    if target.root() == ir.root().id {
                        (condition, true)
                    } else {
                        (format!("{condition} #{element}"), true)
                    }
                }
                IrMotionStart::Changes { .. } => (format!("#{element}"), false),
            };
            if !groups.contains_key(&selector) {
                order.push(selector.clone());
            }
            groups
                .entry(selector)
                .or_insert_with(|| (target.root(), becomes, Vec::new()))
                .2
                .push(Entry {
                    property,
                    duration: format!("{}ms", num(motion.duration_ms)),
                    delay: format!("{}ms", num(motion.delay_ms)),
                    timing: timing.clone(),
                    play_reduced: motion.reduced_motion == IrReducedMotion::Play,
                });
        }
    }
    // Becomes 규칙은 specificity가 높아 같은 element의 `Changes` 목록을 가립니다. 그래서 Becomes 규칙에 같은
    // element의 `Changes` 항목을 앞에 함께 씁니다. 같은 속성이 겹치면 CSS는 목록의 마지막 항목을 쓰므로 그 상태로
    // 들어갈 때는 Becomes 항목이 이깁니다.
    let changes: BTreeMap<StableId, Vec<Entry>> = groups
        .values()
        .filter(|(_, becomes, _)| !becomes)
        .map(|(element, _, entries)| (*element, entries.clone()))
        .collect();
    for selector in order {
        let (element, becomes, own) = &groups[&selector];
        let mut entries: Vec<Entry> = Vec::new();
        if *becomes {
            entries.extend(changes.get(element).into_iter().flatten().cloned());
        }
        entries.extend(own.iter().cloned());
        let all: Vec<&Entry> = entries.iter().collect();
        out.rules.push(CssRule {
            selector: selector.clone(),
            declarations: transition_declarations(&all),
        });
        if entries.iter().any(|e| !e.play_reduced) {
            let kept: Vec<&Entry> = entries.iter().filter(|e| e.play_reduced).collect();
            let declarations = if kept.is_empty() {
                vec![("transition-property".to_string(), "none".to_string())]
            } else {
                transition_declarations(&kept)
            };
            out.reduced.push(CssRule {
                selector,
                declarations,
            });
        }
    }
    out.diagnostics.sort();
    out.diagnostics.dedup();
    out
}

/// contract가 지원하는 상호작용 상태.
pub fn supported_states(contracts: &[ContractCode]) -> Vec<IrInteractionState> {
    use IrInteractionState as S;
    if contracts.contains(&ContractCode::BUTTON) {
        vec![S::Hover, S::Focused, S::Pressed, S::Disabled]
    } else if contracts.contains(&ContractCode::TEXT_INPUT) {
        vec![S::Hover, S::Focused, S::Disabled]
    } else if contracts.is_empty() {
        Vec::new()
    } else {
        vec![S::Hover, S::Focused]
    }
}

/// 상태 selector 접미사.
pub const fn state_selector(state: IrInteractionState) -> &'static str {
    match state {
        IrInteractionState::Default => "",
        IrInteractionState::Hover => ":hover",
        IrInteractionState::Focused => ":focus-visible",
        IrInteractionState::Pressed => ":active",
        IrInteractionState::Disabled => ":disabled",
    }
}

/// 상태 selector 출력 순서(나중 규칙이 이김): hover, focused, pressed, disabled.
const STATE_ORDER: [IrInteractionState; 4] = [
    IrInteractionState::Hover,
    IrInteractionState::Focused,
    IrInteractionState::Pressed,
    IrInteractionState::Disabled,
];

/// `interaction_state` 조건부 토큰을 쓰는 Control 속성을 상태 selector 규칙으로 냅니다.
pub fn lower_interaction_tokens(
    ir: &IrTree,
    tokens: &[InteractionToken],
    names: &dyn CssNames,
    property_name: &dyn Fn(&PropertyPath) -> Option<&'static str>,
) -> WebMotion {
    let mut out = WebMotion::default();
    let by_id: BTreeMap<StableId, &InteractionToken> = tokens.iter().map(|t| (t.id, t)).collect();
    // element → (상태 → 선언).
    for node in &ir.nodes {
        let element = element_id(node.id);
        let supported = supported_states(&node.contracts);
        let mut base: Vec<(String, String)> = Vec::new();
        let mut states: BTreeMap<IrInteractionState, Vec<(String, String)>> = BTreeMap::new();
        for property in &node.properties {
            let Some(token) = property.token.and_then(|t| by_id.get(&t)) else {
                continue;
            };
            let Some(name) = property_name(&property.path) else {
                out.diagnostics.push(MotionDiagnostic {
                    subject: node.id,
                    code: "interaction-property-unmapped",
                });
                continue;
            };
            let Some(default) = names.literal(&token.default) else {
                continue;
            };
            base.push((name.to_string(), default));
            if supported.is_empty() {
                out.diagnostics.push(MotionDiagnostic {
                    subject: node.id,
                    code: "interaction-state-no-control",
                });
                continue;
            }
            for (state, value) in &token.states {
                if !supported.contains(state) {
                    out.diagnostics.push(MotionDiagnostic {
                        subject: node.id,
                        code: "interaction-state-unsupported",
                    });
                    continue;
                }
                if let Some(text) = names.literal(value) {
                    states
                        .entry(*state)
                        .or_default()
                        .push((name.to_string(), text));
                }
            }
        }
        if base.is_empty() {
            continue;
        }
        out.rules.push(CssRule {
            selector: format!("#{element}"),
            declarations: base,
        });
        for state in STATE_ORDER {
            if let Some(declarations) = states.remove(&state) {
                out.rules.push(CssRule {
                    selector: format!("#{element}{}", state_selector(state)),
                    declarations,
                });
            }
        }
    }
    out.diagnostics.sort();
    out.diagnostics.dedup();
    out
}

/// 규칙 목록의 CSS 텍스트(규칙마다 `selector {` 한 줄, 선언은 두 칸 들여쓰기).
pub fn rules_text(rules: &[CssRule]) -> String {
    let mut out = String::new();
    for rule in rules {
        out.push_str(&rule.selector);
        out.push_str(" {\n");
        for (name, value) in &rule.declarations {
            out.push_str(&format!("  {name}: {value};\n"));
        }
        out.push_str("}\n");
    }
    out
}

#[cfg(test)]
mod tests;
