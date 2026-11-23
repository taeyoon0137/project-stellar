//! 토큰 → Web CSS custom property·생성 상수 (XXII.2.3, ADR-0106 결정 5, 속성 모델 10장).
//!
//! 어디에 선언할지는 codegen 계획([`stellar_codegen::web::tokens::plan_tokens`])이 정하고, 이 module은 문서 함수로
//! 텍스트를 조립합니다.
//!
//! - CSS 이름: 문서 `derive_names`(IX.11.16). 토큰과 ContextKey를 한 이름 공간으로 파생합니다([`web_names`]).
//! - 값 텍스트: 문서 `literal_text`. `:root` 블록: 문서 `root_block`(alias는 `var()`).
//! - `@property`: ops는 expressions의 `ExpressionType`을 이름으로 쓸 수 없어(crate 그래프) 문서 `property_rule`을 부를 수
//!   없습니다. 등록 문법은 literal variant로 정하고([`registered_syntax_of`]), 문법 표와 규칙 template이 문서 함수와
//!   같은지 test가 문서 source와 대조합니다.
//! - 조건부 값: `:root`를 감싼 `@media` 블록(안쪽은 문서 `root_block`을 두 칸 들여 씀).

use std::fmt::Write as _;

use stellar_codegen::ir::IrTree;
use stellar_codegen::web::js::{js_string, js_value};
use stellar_codegen::web::tokens::{plan_tokens, TokenPlan};
use stellar_document::css::custom_property::{
    derive_names, literal_text, root_block, CustomPropertyKind, DerivedNames, NameInput, RootValue,
};

use crate::core_types::ids::StableId;
use crate::core_types::typed_value::TypedValue;

/// 토큰 출력 진단.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct WebTokenDiagnostic {
    /// 대상 토큰·ContextKey.
    pub subject: StableId,
    /// 안정 code.
    pub code: &'static str,
}

/// 토큰 출력.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebTokens {
    /// `tokens.css`(없으면 빈 문자열).
    pub css: String,
    /// `tokens.js`(상수 토큰이 없으면 빈 문자열).
    pub js: String,
    /// 이름 파생 결과(토큰·ContextKey 한 이름 공간).
    pub names: DerivedNames,
    /// 출력 계획.
    pub plan: TokenPlan,
    /// 진단(대상·code 순).
    pub diagnostics: Vec<WebTokenDiagnostic>,
}

/// 같은 export scope의 토큰과 ContextKey 이름을 한 이름 공간으로 파생합니다.
pub fn web_names(ir: &IrTree) -> DerivedNames {
    let mut inputs: Vec<NameInput<'_>> = ir
        .tokens
        .iter()
        .map(|t| NameInput {
            name: &t.name,
            kind: CustomPropertyKind::Token,
            id: t.id,
        })
        .collect();
    inputs.extend(ir.contexts.keys.iter().map(|k| NameInput {
        name: &k.name,
        kind: CustomPropertyKind::ContextKey,
        id: k.id,
    }));
    derive_names(&inputs)
}

/// 등록 custom property의 `syntax`(literal variant 기준, 문서 `registered_syntax`와 같은 표).
pub fn registered_syntax_of(value: &TypedValue) -> Option<&'static str> {
    Some(match value {
        TypedValue::Length(_) => "<length>",
        TypedValue::Percentage(_) => "<percentage>",
        TypedValue::Angle(_) => "<angle>",
        TypedValue::Duration(_) => "<time>",
        TypedValue::Number(_) => "<number>",
        TypedValue::Color(_) => "<color>",
        _ => return None,
    })
}

/// `@property` 규칙(문서 `property_rule`과 같은 template, `inherits: true`, literal `initial-value`).
pub fn property_rule_of(css_name: &str, initial: &TypedValue) -> Option<String> {
    let syntax = registered_syntax_of(initial)?;
    let initial = literal_text(initial)?;
    Some(format!(
        "@property {css_name} {{\n  syntax: \"{syntax}\";\n  inherits: true;\n  initial-value: {initial};\n}}\n"
    ))
}

/// 블록 텍스트의 줄마다 두 칸을 앞에 붙입니다.
pub fn indent_block(block: &str) -> String {
    block.lines().fold(String::new(), |mut out, line| {
        let _ = writeln!(out, "  {line}");
        out
    })
}

/// IR 토큰을 Web CSS와 생성 상수로 내보냅니다.
pub fn export_tokens(ir: &IrTree) -> WebTokens {
    let names = web_names(ir);
    let plan = plan_tokens(ir, &|value| literal_text(value).is_some());
    let name = |id: StableId| names.name_of(id).unwrap_or_default().to_string();
    let mut diagnostics: Vec<WebTokenDiagnostic> = plan
        .diagnostics
        .iter()
        .map(|d| WebTokenDiagnostic {
            subject: d.token,
            code: d.kind.code(),
        })
        .collect();
    for collision in &names.collisions {
        for id in &collision.ids {
            diagnostics.push(WebTokenDiagnostic {
                subject: *id,
                code: collision.code(),
            });
        }
    }
    diagnostics.sort();
    diagnostics.dedup();

    let mut css = String::new();
    for token in &plan.css {
        if let Some(rule) = property_rule_of(&name(token.id), &token.default) {
            css.push_str(&rule);
        }
    }
    let root: Vec<(String, RootValue)> = plan
        .css
        .iter()
        .map(|token| {
            let value = match token.alias {
                Some(alias) => RootValue::Alias(name(alias)),
                None => RootValue::Literal(token.default.clone()),
            };
            (name(token.id), value)
        })
        .collect();
    if !root.is_empty() {
        css.push_str(&root_block(&root));
    }
    for block in &plan.media {
        let values: Vec<(String, RootValue)> = block
            .values
            .iter()
            .map(|(id, value)| (name(*id), RootValue::Literal(value.clone())))
            .collect();
        let _ = write!(
            css,
            "@media {} {{\n{}}}\n",
            block.query,
            indent_block(&root_block(&values))
        );
    }

    let mut js = String::new();
    if !plan.constants.is_empty() {
        js.push_str("export const tokens = Object.freeze({\n");
        for id in &plan.constants {
            let Some(token) = ir.token(*id) else {
                continue;
            };
            let _ = writeln!(
                js,
                "  {}: {},",
                js_string(&name(*id)),
                js_value(&token.value)
            );
        }
        js.push_str("});\n");
    }
    WebTokens {
        css,
        js,
        names,
        plan,
        diagnostics,
    }
}

#[cfg(test)]
pub(crate) mod tests;
