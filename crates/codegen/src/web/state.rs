//! 상태 머신 → 생성 JavaScript `state.js` (XXII.2.6, ADR-0106 결정 7).
//!
//! - 상태 값은 객체 하나에 두고 export root element의 `data-s-<상태 hex>` 속성에 쓰며, 값 텍스트는 motion selector의
//!   [`crate::web::motion::state_attr_value`]와 같은 규칙(불리언·수·문자열 그대로, enum `<type>:<variant>`)입니다.
//! - transition trigger 이벤트는 DOM 이벤트로 등록합니다. 같은 (source, 이벤트)의 transition은 listener 하나에서 목록
//!   순서대로 조건을 보고 처음 참인 transition 하나만 실행합니다. action 값은 모두 계산한 뒤 한 번에 반영합니다(IX.5.5
//!   원자 commit).
//! - nested action trigger는 instance element의 `stellar:<action hex>` CustomEvent, emit은 같은 이름의 bubbling
//!   CustomEvent입니다.
//! - DOM 이벤트가 없는 종류(drag·scroll·long press)와 JavaScript로 낼 수 없는 식은 진단하고 그 transition을 뺍니다.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use stellar_core_types::ids::StableId;

use crate::ir::snapshot::id_hex;
use crate::ir::{IrAction, IrExpr, IrTransition, IrTree, IrTrigger};
use crate::web::aria::element_id;
use crate::web::expr::{js_expr, literal_js, ReadContext};
use crate::web::js::js_string;
use crate::web::motion::MotionDiagnostic;

/// 문서 이벤트 이름 → DOM 이벤트 이름.
pub fn dom_event(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "activate" => "click",
        "input" => "input",
        "focus" => "focus",
        "blur" => "blur",
        "key" => "keydown",
        "selectionChange" => "change",
        "open" => "toggle",
        "close" => "close",
        "pointerEnter" => "pointerenter",
        "pointerLeave" => "pointerleave",
        "pointerDown" => "pointerdown",
        "pointerUp" => "pointerup",
        _ => return None,
    })
}

/// 생성 `state.js`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WebState {
    /// JavaScript(상태 머신이 없으면 빈 문자열).
    pub script: String,
    /// 진단.
    pub diagnostics: Vec<MotionDiagnostic>,
}

fn expr_js(
    expr: &IrExpr,
    transition: StableId,
    diagnostics: &mut Vec<MotionDiagnostic>,
) -> Option<String> {
    // `state.js`의 `read`는 상태 값만 제공합니다. 토큰·속성·환경·Context·Component Property 읽기는 낼 수 없습니다.
    let mut stack = vec![expr];
    while let Some(e) = stack.pop() {
        if matches!(
            e,
            IrExpr::Token(_)
                | IrExpr::Property(_)
                | IrExpr::Environment(_)
                | IrExpr::Context(_)
                | IrExpr::ComponentProperty(_)
        ) {
            diagnostics.push(MotionDiagnostic {
                subject: transition,
                code: "state-reference-unsupported",
            });
            return None;
        }
        stack.extend(e.children());
    }
    let text = js_expr(expr, &ReadContext);
    if text.is_none() {
        diagnostics.push(MotionDiagnostic {
            subject: transition,
            code: "state-expression-unsupported",
        });
    }
    text
}

/// 상태 머신을 `state.js`로 낮춥니다.
pub fn lower_state_machines(ir: &IrTree) -> WebState {
    let mut out = WebState::default();
    if ir.machines.is_empty() {
        return out;
    }
    let root = element_id(ir.root().id);
    let mut js = String::new();
    js.push_str("const root = document.getElementById(");
    js.push_str(&js_string(&root));
    js.push_str(");\nconst state = {\n");
    let mut variables: Vec<StableId> = ir
        .machines
        .iter()
        .flat_map(|m| m.variables.iter().copied())
        .collect();
    variables.sort();
    variables.dedup();
    for variable in &variables {
        if let Some(decl) = ir.state(*variable) {
            let _ = writeln!(
                js,
                "  {}: {},",
                js_string(&id_hex(*variable)),
                literal_js(&decl.initial)
            );
        }
    }
    js.push_str("};\n");
    js.push_str(
        "const read = {\n  state: (id) => state[id],\n  equals: (a, b) => JSON.stringify(a) === JSON.stringify(b),\n  clamp: (v, lo, hi) => Math.min(Math.max(v, lo), hi),\n  concat: (...parts) => parts.join(\"\"),\n};\n",
    );
    js.push_str(
        "function text(value) {\n  if (value !== null && typeof value === \"object\" && value.type === \"enum\") {\n    return value.enumType + \":\" + value.variant;\n  }\n  return String(value);\n}\n",
    );
    js.push_str("function commit(changes) {\n  for (const [id, value] of changes) {\n    state[id] = value;\n    root.setAttribute(\"data-s-\" + id, text(value));\n  }\n}\n");
    js.push_str("for (const id of Object.keys(state)) {\n  root.setAttribute(\"data-s-\" + id, text(state[id]));\n}\n");
    // (element, DOM 이벤트) → transition 목록(머신 순, 목록 순).
    let mut listeners: BTreeMap<(String, String), Vec<&IrTransition>> = BTreeMap::new();
    let mut order: Vec<(String, String)> = Vec::new();
    for machine in &ir.machines {
        for transition in &machine.transitions {
            let (element, event) = match transition.trigger {
                IrTrigger::Event { kind, source } => match dom_event(kind) {
                    Some(event) => (element_id(source), event.to_string()),
                    None => {
                        out.diagnostics.push(MotionDiagnostic {
                            subject: transition.id,
                            code: "state-event-unsupported",
                        });
                        continue;
                    }
                },
                IrTrigger::NestedAction { instance, action } => {
                    (element_id(instance), format!("stellar:{}", id_hex(action)))
                }
            };
            let key = (element, event);
            if !listeners.contains_key(&key) {
                order.push(key.clone());
            }
            listeners.entry(key).or_default().push(transition);
        }
    }
    for key in order {
        let (element, event) = &key;
        let mut body = String::new();
        for transition in &listeners[&key] {
            let condition = match &transition.condition {
                Some(expr) => match expr_js(expr, transition.id, &mut out.diagnostics) {
                    Some(text) => text,
                    None => continue,
                },
                None => "true".to_string(),
            };
            let mut sets = Vec::new();
            let mut emits = Vec::new();
            let mut ok = true;
            for action in &transition.actions {
                match action {
                    IrAction::SetState { variable, value } => {
                        match expr_js(value, transition.id, &mut out.diagnostics) {
                            Some(text) => {
                                sets.push(format!("[{}, {text}]", js_string(&id_hex(*variable))))
                            }
                            None => ok = false,
                        }
                    }
                    IrAction::Emit { action } => emits.push(format!(
                        "    root.dispatchEvent(new CustomEvent({}, {{ bubbles: true }}));\n",
                        js_string(&format!("stellar:{}", id_hex(*action)))
                    )),
                }
            }
            if !ok {
                continue;
            }
            let _ = write!(
                body,
                "  if ({condition}) {{\n    commit([{}]);\n{}    return;\n  }}\n",
                sets.join(", "),
                emits.concat()
            );
        }
        if body.is_empty() {
            continue;
        }
        let _ = write!(
            js,
            "document.getElementById({}).addEventListener({}, () => {{\n{body}}});\n",
            js_string(element),
            js_string(event)
        );
    }
    out.script = js;
    out.diagnostics.sort();
    out.diagnostics.dedup();
    out
}
