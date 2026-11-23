//! codegen Web 출력이 쓰는 문서 CSS 이름·값 표기 (XXII.2.5, XXII.2.6, ADR-0106 결정 5·6).
//!
//! codegen은 문서 crate를 읽지 못하므로 이 module이 문서 `derive_names` 결과와 `literal_text`를 codegen의
//! [`CssNames`]로 넘깁니다. Context key 등록(`@property`)은 토큰과 같은 함수([`property_rule_of`])로 냅니다.

use std::collections::BTreeSet;

use stellar_codegen::ir::{IrExpr, IrTree};
use stellar_codegen::web::context::{lower_contexts, WebContexts};
use stellar_codegen::web::expr::CssNames;
use stellar_document::css::custom_property::{literal_text, DerivedNames};

use crate::core_types::ids::StableId;
use crate::core_types::typed_value::TypedValue;
use crate::export_web_tokens::{property_rule_of, web_names};

/// 문서 이름 파생 결과와 값 formatter.
pub struct DocumentCss<'a> {
    /// 토큰·ContextKey 이름.
    pub names: &'a DerivedNames,
}

impl CssNames for DocumentCss<'_> {
    fn name(&self, id: StableId) -> Option<String> {
        self.names.name_of(id).map(str::to_string)
    }

    fn literal(&self, value: &TypedValue) -> Option<String> {
        literal_text(value)
    }
}

/// 이름 공간 전체(토큰·ContextKey CSS 이름).
pub fn taken_names(names: &DerivedNames) -> BTreeSet<String> {
    names.names.iter().map(|n| n.css_name.clone()).collect()
}

/// Context Web 출력과 CSS key 등록 규칙.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebContextCss {
    /// codegen 출력.
    pub contexts: WebContexts,
    /// CSS key의 `@property` 규칙(default가 literal인 key만, key 순).
    pub property_rules: String,
}

/// IR Context를 문서 이름·값 표기로 낮춥니다.
pub fn export_contexts(ir: &IrTree) -> WebContextCss {
    let names = web_names(ir);
    let css = DocumentCss { names: &names };
    let contexts = lower_contexts(ir, &css, &taken_names(&names));
    let mut property_rules = String::new();
    for key in &contexts.css_keys {
        let Some(decl) = ir.contexts.key(*key) else {
            continue;
        };
        if let (IrExpr::Literal(value), Some(name)) = (&decl.default, names.name_of(*key)) {
            if let Some(rule) = property_rule_of(name, value) {
                property_rules.push_str(&rule);
            }
        }
    }
    WebContextCss {
        contexts,
        property_rules,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_types::color::{Color, ColorSpace};
    use crate::export_web_tokens::tests::{id, node};
    use stellar_codegen::ir::{
        IrContextConsumption, IrContextKey, IrContextProvision, IrContextSource, IrContexts,
    };
    use stellar_target_profile::capability::ElementKind;

    fn red() -> TypedValue {
        TypedValue::Color(Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 1.0).expect("color"))
    }

    #[test]
    fn context_keys_use_document_names_values_and_registration() {
        let mut root = node(1, None, 0, ElementKind::FRAME);
        root.children = vec![1];
        let child = node(2, Some(0), 1, ElementKind::FRAME);
        let mut ir = IrTree::from_nodes(vec![root, child]).expect("tree");
        ir.contexts = IrContexts {
            keys: vec![IrContextKey {
                id: id(20),
                name: "theme accent".into(),
                default: IrExpr::Literal(red()),
            }],
            provisions: vec![IrContextProvision {
                node: id(1),
                key: id(20),
                value: IrExpr::Literal(red()),
                depths: vec![1],
            }],
            consumptions: vec![IrContextConsumption {
                node: id(2),
                path: crate::core_types::property::PropertyPath::new(id(2))
                    .push(crate::core_types::property::PropertyKey(4)),
                key: id(20),
                source: IrContextSource::Provision(id(1)),
                slots: Vec::new(),
            }],
            slots: Vec::new(),
        };
        let out = export_contexts(&ir);
        assert_eq!(
            out.property_rules,
            "@property --context-theme-accent {\n  syntax: \"<color>\";\n  inherits: true;\n  initial-value: #ff0000;\n}\n"
        );
        assert_eq!(
            out.contexts.rules[0].declarations,
            vec![
                ("--context-theme-accent".to_string(), "#ff0000".to_string()),
                ("--context-theme-accent".to_string(), "#ff0000".to_string()),
            ]
        );
        assert_eq!(
            out.contexts.consumers[0].value,
            "var(--context-theme-accent)"
        );
    }
}
