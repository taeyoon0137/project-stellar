//! IR → Web 생성 파일 집합 (XXII.5.1, ADR-0106 결정 8).
//!
//! 각 부분은 이미 있는 출력 함수가 만들고(HTML 구조·접근성 XXII.2.1·XXII.2.4, 토큰 XXII.2.3, Context XXII.2.5,
//! motion·상태·상호작용 XXII.2.6), 이 module은 파일 순서·식별자·formatting을 고정해 [`GeneratedFiles`]로 묶습니다.
//!
//! | 파일 | 내용 |
//! |---|---|
//! | `context.js` | Context 구조(CSS로 낼 수 없는 key) |
//! | `index.html` | HTML 구조와 접근성 의미, CSS·script 연결 |
//! | `state.js` | 상태 머신 |
//! | `styles.css` | Context 등록·선언·소비, motion transition, reduced motion, 상호작용 상태 selector |
//! | `tokens.css` | 토큰 `@property`·`:root`·`@media` |
//! | `tokens.js` | 토큰 생성 상수 |
//!
//! 규칙이 가리키는 element와 export root에만 `id="s-<StableId hex>"`를 붙입니다. 비어 있는 파일은 만들지 않습니다.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use stellar_codegen::ir::{IrTree, IrTrigger};
use stellar_codegen::output::{GeneratedFiles, OutputError};
use stellar_codegen::web::aria::{element_id, map_accessibility};
use stellar_codegen::web::motion::{rules_text, CssRule, EasingSampler};

use crate::core_types::ids::StableId;
use crate::export_web_css::export_contexts;
use crate::export_web_motion::{css_property_name, export_behavior};
use crate::export_web_tokens::{export_tokens, indent_block};

/// Web 출력 진단(출처 module의 code 그대로).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct WebExportDiagnostic {
    /// 대상(node·토큰·key·motion·transition).
    pub subject: StableId,
    /// 안정 code.
    pub code: &'static str,
}

/// Web 출력.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebExport {
    /// 생성 파일(경로 순).
    pub files: GeneratedFiles,
    /// 진단(대상·code 순, 중복 없음).
    pub diagnostics: Vec<WebExportDiagnostic>,
}

/// IR을 Web 생성 파일 집합으로 내보냅니다. `sampler`는 spring 표본용 엔진 evaluator입니다.
pub fn export_web(
    ir: &IrTree,
    sampler: Option<&dyn EasingSampler>,
) -> Result<WebExport, OutputError> {
    let mut diagnostics: Vec<WebExportDiagnostic> = Vec::new();
    let accessibility = map_accessibility(ir);
    let tokens = export_tokens(ir);
    let contexts = export_contexts(ir);
    let behavior = export_behavior(ir, sampler);
    diagnostics.extend(tokens.diagnostics.iter().map(|d| WebExportDiagnostic {
        subject: d.subject,
        code: d.code,
    }));
    diagnostics.extend(
        contexts
            .contexts
            .diagnostics
            .iter()
            .map(|d| WebExportDiagnostic {
                subject: d.key,
                code: d.kind.code(),
            }),
    );
    for list in [
        &behavior.motion.diagnostics,
        &behavior.interaction.diagnostics,
        &behavior.state.diagnostics,
    ] {
        diagnostics.extend(list.iter().map(|d| WebExportDiagnostic {
            subject: d.subject,
            code: d.code,
        }));
    }

    // styles.css
    let mut targets: BTreeSet<StableId> = BTreeSet::new();
    targets.insert(ir.root().id);
    let mut rules: Vec<CssRule> = Vec::new();
    for rule in &contexts.contexts.rules {
        targets.insert(rule.node);
        rules.push(CssRule {
            selector: format!("#{}", element_id(rule.node)),
            declarations: rule.declarations.clone(),
        });
    }
    for consumer in &contexts.contexts.consumers {
        match css_property_name(&consumer.path) {
            Some(name) => {
                targets.insert(consumer.node);
                rules.push(CssRule {
                    selector: format!("#{}", element_id(consumer.node)),
                    declarations: vec![(name.to_string(), consumer.value.clone())],
                });
            }
            None => diagnostics.push(WebExportDiagnostic {
                subject: consumer.node,
                code: "context-property-unmapped",
            }),
        }
    }
    for motion in &ir.motions {
        targets.extend(motion.targets.iter().map(|t| t.root()));
    }
    for node in &ir.nodes {
        let selector = format!("#{}", element_id(node.id));
        if behavior
            .interaction
            .rules
            .iter()
            .any(|r| r.selector.starts_with(&selector))
        {
            targets.insert(node.id);
        }
    }
    for machine in &ir.machines {
        for transition in &machine.transitions {
            targets.insert(match transition.trigger {
                IrTrigger::Event { source, .. } => source,
                IrTrigger::NestedAction { instance, .. } => instance,
            });
        }
    }
    let mut styles = String::new();
    styles.push_str(&contexts.property_rules);
    styles.push_str(&rules_text(&rules));
    styles.push_str(&rules_text(&behavior.motion.rules));
    if !behavior.motion.reduced.is_empty() {
        let _ = write!(
            styles,
            "@media (prefers-reduced-motion: reduce) {{\n{}}}\n",
            indent_block(&rules_text(&behavior.motion.reduced))
        );
    }
    styles.push_str(&rules_text(&behavior.interaction.rules));

    // index.html
    let mut document = accessibility.document;
    let marked: Vec<u32> = document
        .elements()
        .iter()
        .enumerate()
        .filter(|(_, e)| e.source.is_some_and(|s| targets.contains(&s)))
        .map(|(index, _)| index as u32)
        .collect();
    for index in marked {
        if let Some(source) = document.element(index).source {
            document.set_attr(index, "id", element_id(source));
        }
    }
    let mut files = GeneratedFiles::new();
    files.insert("tokens.css", &tokens.css)?;
    files.insert("tokens.js", &tokens.js)?;
    files.insert("styles.css", &styles)?;
    files.insert("context.js", &contexts.contexts.structure)?;
    files.insert("state.js", &behavior.state.script)?;
    let mut head = String::from("    <meta charset=\"utf-8\">\n");
    for css in ["tokens.css", "styles.css"] {
        if files.get(css).is_some() {
            let _ = writeln!(head, "    <link rel=\"stylesheet\" href=\"{css}\">");
        }
    }
    for js in ["context.js", "state.js", "tokens.js"] {
        if files.get(js).is_some() {
            let _ = writeln!(head, "    <script type=\"module\" src=\"{js}\"></script>");
        }
    }
    let body = indent_block(&indent_block(&document.to_html()));
    let html = format!(
        "<!doctype html>\n<html>\n  <head>\n{head}  </head>\n  <body>\n{body}  </body>\n</html>\n"
    );
    files.insert("index.html", &html)?;
    diagnostics.sort();
    diagnostics.dedup();
    Ok(WebExport { files, diagnostics })
}

#[cfg(test)]
mod tests;
