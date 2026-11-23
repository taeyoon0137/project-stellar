//! Context → Web 출력 (XXII.2.5, ADR-0106 결정 6, ADR-0025 결정 3.11·4.3).
//!
//! IR이 보존한 해석 결과만 씁니다(다시 해석하지 않음).
//!
//! - **CSS key**: default 식과 모든 provision 값 식이 CSS로 낼 수 있고 깊이 집합이 원소 하나인 key입니다. default는
//!   export root element에, provision 값은 provider element에 custom property 선언으로 내고 상속이 하위 트리에
//!   전달합니다. 등록(`@property`, `inherits: true`)은 상위(ops)가 토큰과 같은 함수로 냅니다.
//! - **같은 key 재읽기**: key의 provision 중 하나라도 같은 key를 읽으면 그 key는 깊이 이름을 씁니다. provision 깊이 d의
//!   값은 `<key>-<d>`에 쓰고 `<key>: var(<key>-<d>)`로 다시 공급하며, 값 식 안의 같은 key 읽기는 `<key>-<d-1>`입니다.
//!   default는 root element의 `<key>-0`입니다. 자기 참조 `var()`는 내지 않습니다.
//! - **context 구조**: 깊이 집합 원소가 둘 이상이면(`context-depth-varies` 정보 진단) 또는 CSS로 낼 수 없는 식이면
//!   그 key 전체를 `context.js` 정적 표로 냅니다.
//! - **slot 재공급**: Slot이 공개하지 않은 CSS key 중 instance와 slot host 사이(instance 제외, host 포함)에
//!   provision이 있는 key는 instance element에서 `<이름>-outer: var(<이름>)`으로 보존하고 slot host element에서
//!   `<이름>: var(<이름>-outer)`로 다시 공급합니다. 깊이 이름을 쓰는 key는 그 구간 provision의 깊이 이름도 같이
//!   보존·재공급합니다.
//! - **이름 충돌**: 깊이 이름·보존 이름은 토큰·key 이름과 한 이름 공간에서 검사하고 `name-collision`을 냅니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;

use crate::ir::snapshot::{id_hex, path_text};
use crate::ir::{IrContextSource, IrExpr, IrTree};
use crate::web::aria::element_id;
use crate::web::expr::{css_expr, js_expr, ContextRead, CssNames, JsContext};
use crate::web::js::js_string;

/// element 하나의 custom property 선언(선언 순서 = 출력 순서).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextRule {
    /// element의 IR node.
    pub node: StableId,
    /// (이름, 값 텍스트).
    pub declarations: Vec<(String, String)>,
}

/// Context 진단 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContextDiagnosticKind {
    /// 깊이가 instance 위치마다 달라 context 구조로 냄(정보).
    DepthVaries,
    /// CSS로 낼 수 없는 식이라 context 구조로 냄(정보).
    NonCssValue,
    /// 깊이·보존 이름이 다른 이름과 겹침.
    NameCollision,
    /// context 구조의 식을 JavaScript로 낼 수 없음.
    UnsupportedExpression,
}

impl ContextDiagnosticKind {
    /// 안정 code.
    pub const fn code(self) -> &'static str {
        match self {
            ContextDiagnosticKind::DepthVaries => "context-depth-varies",
            ContextDiagnosticKind::NonCssValue => "context-structure",
            ContextDiagnosticKind::NameCollision => "name-collision",
            ContextDiagnosticKind::UnsupportedExpression => "context-expression-unsupported",
        }
    }
}

/// Context 진단.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContextDiagnostic {
    /// key.
    pub key: StableId,
    /// 종류.
    pub kind: ContextDiagnosticKind,
    /// 관련 이름(충돌이면 겹친 이름).
    pub name: Option<String>,
}

/// CSS로 소비 지점을 낼 때 쓰는 선언.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextConsumerRule {
    /// 소비 node.
    pub node: StableId,
    /// 소비 속성.
    pub path: PropertyPath,
    /// 값 텍스트(`var(<key>)`).
    pub value: String,
}

/// Context Web 출력.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WebContexts {
    /// element별 선언(IR preorder).
    pub rules: Vec<ContextRule>,
    /// CSS key(id 순): 등록 대상.
    pub css_keys: Vec<StableId>,
    /// 소비 지점의 CSS 값(소비 순).
    pub consumers: Vec<ContextConsumerRule>,
    /// `context.js`(context 구조 key가 없으면 빈 문자열).
    pub structure: String,
    /// 진단.
    pub diagnostics: Vec<ContextDiagnostic>,
}

/// key 하나의 출력 방식.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyMode {
    /// CSS custom property.
    Css,
    /// CSS custom property + 깊이 이름.
    CssDepth,
    /// context 구조.
    Structure,
}

/// 깊이 이름.
pub fn depth_name(key: &str, depth: u32) -> String {
    format!("{key}-{depth}")
}

/// slot 보존 이름.
pub fn outer_name(name: &str) -> String {
    format!("{name}-outer")
}

struct DepthRead<'a> {
    key: StableId,
    /// 같은 key 읽기를 바꿀 깊이 이름(`None`이면 같은 key 읽기를 허용하지 않음).
    outer: Option<String>,
    names: &'a dyn CssNames,
    modes: &'a BTreeMap<StableId, KeyMode>,
}

impl ContextRead for DepthRead<'_> {
    fn context_name(&self, key: StableId) -> Option<String> {
        if key == self.key {
            return self.outer.clone();
        }
        // 다른 key 읽기는 그 key가 CSS key일 때만(상속된 그 key 값).
        match self.modes.get(&key) {
            Some(KeyMode::Css) | Some(KeyMode::CssDepth) => self.names.name(key),
            _ => None,
        }
    }
}

/// Context를 Web 출력으로 낮춥니다. `taken`은 같은 export scope의 토큰·key CSS 이름 집합입니다(충돌 검사).
pub fn lower_contexts(ir: &IrTree, names: &dyn CssNames, taken: &BTreeSet<String>) -> WebContexts {
    let contexts = &ir.contexts;
    let mut out = WebContexts::default();
    // 1. key별 방식. 다른 key 읽기가 있으면 그 key가 CSS여야 하므로, 먼저 다른 key를 읽지 않는다고 보고 정한 뒤
    //    다른 key를 읽는 식은 고정점까지 다시 확인합니다.
    let mut modes: BTreeMap<StableId, KeyMode> = BTreeMap::new();
    for key in &contexts.keys {
        let provisions: Vec<_> = contexts.provisions_of(key.id).collect();
        let self_read = provisions
            .iter()
            .any(|p| p.value.contexts().contains(&key.id));
        let varies = provisions.iter().any(|p| p.depths.len() > 1);
        let mode = if varies {
            out.diagnostics.push(ContextDiagnostic {
                key: key.id,
                kind: ContextDiagnosticKind::DepthVaries,
                name: None,
            });
            KeyMode::Structure
        } else if self_read {
            KeyMode::CssDepth
        } else {
            KeyMode::Css
        };
        modes.insert(key.id, mode);
    }
    loop {
        let mut changed = false;
        for key in &contexts.keys {
            if modes[&key.id] == KeyMode::Structure {
                continue;
            }
            let Some(key_name) = names.name(key.id) else {
                modes.insert(key.id, KeyMode::Structure);
                changed = true;
                continue;
            };
            let read = DepthRead {
                key: key.id,
                outer: Some(depth_name(&key_name, 0)),
                names,
                modes: &modes,
            };
            let ok = css_expr(&key.default, names, &read).is_some()
                && !key.default.contexts().contains(&key.id)
                && contexts
                    .provisions_of(key.id)
                    .all(|p| css_expr(&p.value, names, &read).is_some());
            if !ok {
                modes.insert(key.id, KeyMode::Structure);
                out.diagnostics.push(ContextDiagnostic {
                    key: key.id,
                    kind: ContextDiagnosticKind::NonCssValue,
                    name: None,
                });
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // 2. element별 선언.
    let mut rules: BTreeMap<u32, Vec<(String, String)>> = BTreeMap::new();
    let mut new_names: BTreeSet<(String, StableId)> = BTreeSet::new();
    for key in &contexts.keys {
        let mode = modes[&key.id];
        if mode == KeyMode::Structure {
            continue;
        }
        out.css_keys.push(key.id);
        let key_name = names.name(key.id).unwrap_or_default();
        let root = &mut rules.entry(0).or_default();
        let read0 = DepthRead {
            key: key.id,
            outer: None,
            names,
            modes: &modes,
        };
        let default = css_expr(&key.default, names, &read0).unwrap_or_default();
        if mode == KeyMode::CssDepth {
            let zero = depth_name(&key_name, 0);
            new_names.insert((zero.clone(), key.id));
            root.push((zero.clone(), default));
            root.push((key_name.clone(), format!("var({zero})")));
        } else {
            root.push((key_name.clone(), default));
        }
        for provision in contexts.provisions_of(key.id) {
            let Some(index) = ir.node_index(provision.node) else {
                continue;
            };
            let depth = provision.depths[0];
            let declarations = rules.entry(index).or_default();
            if mode == KeyMode::CssDepth {
                let read = DepthRead {
                    key: key.id,
                    outer: Some(depth_name(&key_name, depth - 1)),
                    names,
                    modes: &modes,
                };
                let value = css_expr(&provision.value, names, &read).unwrap_or_default();
                let named = depth_name(&key_name, depth);
                new_names.insert((named.clone(), key.id));
                declarations.push((named.clone(), value));
                declarations.push((key_name.clone(), format!("var({named})")));
            } else {
                let read = DepthRead {
                    key: key.id,
                    outer: None,
                    names,
                    modes: &modes,
                };
                let value = css_expr(&provision.value, names, &read).unwrap_or_default();
                declarations.push((key_name.clone(), value));
            }
        }
    }

    // 3. slot 재공급.
    let parents: Vec<Option<u32>> = ir.nodes.iter().map(|n| n.parent).collect();
    for slot in &contexts.slots {
        let (Some(host), Some(instance)) = (ir.node_index(slot.host), ir.node_index(slot.instance))
        else {
            continue;
        };
        // host에서 instance까지(instance 제외) 경로.
        let mut path = Vec::new();
        let mut current = Some(host);
        while let Some(index) = current {
            if index == instance {
                break;
            }
            path.push(index);
            current = parents[index as usize];
        }
        if current != Some(instance) {
            continue;
        }
        for key in &out.css_keys {
            if slot.public_keys.contains(key) {
                continue;
            }
            let inside: Vec<u32> = contexts
                .provisions_of(*key)
                .filter_map(|p| {
                    let index = ir.node_index(p.node)?;
                    path.contains(&index).then_some(p.depths[0])
                })
                .collect();
            if inside.is_empty() {
                continue;
            }
            let key_name = names.name(*key).unwrap_or_default();
            let mut carried = vec![key_name.clone()];
            if modes[key] == KeyMode::CssDepth {
                let mut depths = inside.clone();
                depths.sort_unstable();
                depths.dedup();
                carried.extend(depths.into_iter().map(|d| depth_name(&key_name, d)));
            }
            for name in carried {
                let outer = outer_name(&name);
                new_names.insert((outer.clone(), *key));
                rules
                    .entry(instance)
                    .or_default()
                    .push((outer.clone(), format!("var({name})")));
                rules
                    .entry(host)
                    .or_default()
                    .push((name, format!("var({outer})")));
            }
        }
    }

    // 4. 이름 충돌(깊이·보존 이름 vs 토큰·key 이름).
    for (name, key) in &new_names {
        if taken.contains(name) {
            out.diagnostics.push(ContextDiagnostic {
                key: *key,
                kind: ContextDiagnosticKind::NameCollision,
                name: Some(name.clone()),
            });
        }
    }

    out.rules = rules
        .into_iter()
        .map(|(index, declarations)| ContextRule {
            node: ir.nodes[index as usize].id,
            declarations,
        })
        .collect();

    // 5. 소비 지점.
    for consumption in &contexts.consumptions {
        if matches!(
            modes.get(&consumption.key),
            Some(KeyMode::Css | KeyMode::CssDepth)
        ) {
            if let Some(name) = names.name(consumption.key) {
                out.consumers.push(ContextConsumerRule {
                    node: consumption.node,
                    path: consumption.path.clone(),
                    value: format!("var({name})"),
                });
            }
        }
    }

    // 6. context 구조.
    out.structure = structure(ir, &modes, &mut out.diagnostics);
    out.diagnostics.sort();
    out.diagnostics.dedup();
    out
}

/// 같은 key 재읽기는 바깥 값 인자 `outer`입니다.
struct OuterArg(StableId);

impl JsContext for OuterArg {
    fn read_context(&self, key: StableId) -> String {
        if key == self.0 {
            "outer".to_string()
        } else {
            format!("read.context({})", js_string(&id_hex(key)))
        }
    }
}

fn structure(
    ir: &IrTree,
    modes: &BTreeMap<StableId, KeyMode>,
    diagnostics: &mut Vec<ContextDiagnostic>,
) -> String {
    let contexts = &ir.contexts;
    let keys: Vec<_> = contexts
        .keys
        .iter()
        .filter(|k| modes.get(&k.id) == Some(&KeyMode::Structure))
        .collect();
    if keys.is_empty() {
        return String::new();
    }
    let mut js = String::new();
    let mut expr = |key: StableId, value: &IrExpr| -> String {
        match js_expr(value, &OuterArg(key)) {
            Some(text) => text,
            None => {
                diagnostics.push(ContextDiagnostic {
                    key,
                    kind: ContextDiagnosticKind::UnsupportedExpression,
                    name: None,
                });
                "undefined".to_string()
            }
        }
    };
    js.push_str("export const contexts = Object.freeze({\n  keys: Object.freeze({\n");
    for key in &keys {
        let _ = writeln!(
            js,
            "    {}: Object.freeze({{ name: {}, defaultValue: (read) => {} }}),",
            js_string(&id_hex(key.id)),
            js_string(&key.name),
            expr(key.id, &key.default)
        );
    }
    js.push_str("  }),\n  provisions: Object.freeze([\n");
    for key in &keys {
        for provision in contexts.provisions_of(key.id) {
            let depths: Vec<String> = provision.depths.iter().map(|d| d.to_string()).collect();
            let _ = writeln!(
                js,
                "    Object.freeze({{ element: {}, key: {}, depths: [{}], value: (read, outer) => {} }}),",
                js_string(&element_id(provision.node)),
                js_string(&id_hex(key.id)),
                depths.join(", "),
                expr(key.id, &provision.value)
            );
        }
    }
    js.push_str("  ]),\n  consumers: Object.freeze([\n");
    for consumption in &contexts.consumptions {
        if modes.get(&consumption.key) != Some(&KeyMode::Structure) {
            continue;
        }
        let from = match consumption.source {
            IrContextSource::Provision(node) => js_string(&element_id(node)),
            IrContextSource::Default => "null".to_string(),
        };
        let _ = writeln!(
            js,
            "    Object.freeze({{ element: {}, property: {}, key: {}, from: {from} }}),",
            js_string(&element_id(consumption.node)),
            js_string(&path_text(&consumption.path)),
            js_string(&id_hex(consumption.key))
        );
    }
    js.push_str("  ]),\n});\n");
    js
}

#[cfg(test)]
mod tests;
