//! [`IrExpr`] → CSS·JavaScript 식 (XXII.2.5, XXII.2.6, ADR-0106 결정 6·7).
//!
//! - CSS: literal(상위 formatter), 토큰 `var()`, Context `var()`, 사칙 연산 `calc()`, `min()`·`max()`만 냅니다. 그 밖의 식은
//!   CSS로 낼 수 없어 `None`입니다.
//! - JavaScript: 닫힌 연산 기호·함수 이름을 고정 표로 바꿉니다. 표에 없는 함수는 `None`입니다. 참조는 생성 코드의
//!   읽기 함수(`read.state`·`read.token`·`read.context`·`read.property`·`read.componentProperty`·`read.env`)로 냅니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::TypedValue;

use crate::ir::snapshot::{id_hex, path_text};
use crate::ir::IrExpr;
use crate::web::js::{js_string, js_value};

/// CSS 식을 만들 때 쓰는 이름·값 표기.
pub trait CssNames {
    /// 토큰·ContextKey의 CSS custom property 이름(IX.11.16 파생 결과).
    fn name(&self, id: StableId) -> Option<String>;
    /// literal 값의 CSS 텍스트(문서 `literal_text`). 없으면 `None`.
    fn literal(&self, value: &TypedValue) -> Option<String>;
}

/// Context 읽기를 CSS 이름으로 바꾸는 규칙.
pub trait ContextRead {
    /// key 읽기의 CSS 이름(같은 key 재읽기면 깊이 이름).
    fn context_name(&self, key: StableId) -> Option<String>;
}

/// CSS 식. 낼 수 없으면 `None`.
pub fn css_expr(expr: &IrExpr, names: &dyn CssNames, context: &dyn ContextRead) -> Option<String> {
    Some(match expr {
        IrExpr::Literal(value) => names.literal(value)?,
        IrExpr::Token(id) => format!("var({})", names.name(*id)?),
        IrExpr::Context(key) => format!("var({})", context.context_name(*key)?),
        IrExpr::Binary { op, lhs, rhs } if matches!(*op, "+" | "-" | "*" | "/") => format!(
            "calc({} {op} {})",
            css_expr(lhs, names, context)?,
            css_expr(rhs, names, context)?
        ),
        IrExpr::Call { function, args }
            if matches!(*function, "min" | "max") && !args.is_empty() =>
        {
            let args: Option<Vec<String>> =
                args.iter().map(|a| css_expr(a, names, context)).collect();
            format!("{function}({})", args?.join(", "))
        }
        _ => return None,
    })
}

/// JavaScript 식에서 같은 Context key 재읽기를 무엇으로 낼지.
pub trait JsContext {
    /// key 읽기의 JavaScript 식.
    fn read_context(&self, key: StableId) -> String;
}

/// 기본 Context 읽기: `read.context("<key hex>")`.
pub struct ReadContext;

impl JsContext for ReadContext {
    fn read_context(&self, key: StableId) -> String {
        format!("read.context({})", js_string(&id_hex(key)))
    }
}

fn js_unary(op: &str) -> Option<&'static str> {
    Some(match op {
        "neg" => "-",
        "not" => "!",
        _ => return None,
    })
}

fn js_binary(op: &str) -> Option<&'static str> {
    Some(match op {
        "+" => "+",
        "-" => "-",
        "*" => "*",
        "/" => "/",
        "%" => "%",
        "==" => "===",
        "!=" => "!==",
        "<" => "<",
        "<=" => "<=",
        ">" => ">",
        ">=" => ">=",
        "and" => "&&",
        "or" => "||",
        _ => return None,
    })
}

fn js_function(name: &str) -> Option<&'static str> {
    Some(match name {
        "min" => "Math.min",
        "max" => "Math.max",
        "abs" => "Math.abs",
        "round" => "Math.round",
        "floor" => "Math.floor",
        "ceil" => "Math.ceil",
        "clamp" => "read.clamp",
        "concat" => "read.concat",
        _ => return None,
    })
}

/// JavaScript 식. 표에 없는 연산·함수가 있으면 `None`.
pub fn js_expr(expr: &IrExpr, context: &dyn JsContext) -> Option<String> {
    Some(match expr {
        IrExpr::Literal(value) => literal_js(value),
        IrExpr::Null => "null".to_string(),
        IrExpr::State(id) => format!("read.state({})", js_string(&id_hex(*id))),
        IrExpr::Token(id) => format!("read.token({})", js_string(&id_hex(*id))),
        IrExpr::Property(path) => format!("read.property({})", js_string(&path_text(path))),
        IrExpr::ComponentProperty(id) => {
            format!("read.componentProperty({})", js_string(&id_hex(*id)))
        }
        IrExpr::Environment(key) => format!("read.env({key})"),
        IrExpr::Context(key) => context.read_context(*key),
        IrExpr::Unary { op, operand } => {
            format!("({}{})", js_unary(op)?, js_expr(operand, context)?)
        }
        IrExpr::Binary { op, lhs, rhs } => format!(
            "({} {} {})",
            js_expr(lhs, context)?,
            js_binary(op)?,
            js_expr(rhs, context)?
        ),
        IrExpr::Call { function, args } => {
            let args: Option<Vec<String>> = args.iter().map(|a| js_expr(a, context)).collect();
            format!("{}({})", js_function(function)?, args?.join(", "))
        }
        IrExpr::If {
            condition,
            then,
            otherwise,
        } => format!(
            "({} ? {} : {})",
            js_expr(condition, context)?,
            js_expr(then, context)?,
            js_expr(otherwise, context)?
        ),
        IrExpr::Match {
            scrutinee,
            arms,
            default,
        } => {
            // 순서대로 비교하는 중첩 삼항식(값 비교는 read.equals).
            let subject = js_expr(scrutinee, context)?;
            let mut out = js_expr(default, context)?;
            for (pattern, result) in arms.iter().rev() {
                out = format!(
                    "(read.equals({subject}, {}) ? {} : {out})",
                    literal_js(pattern),
                    js_expr(result, context)?
                );
            }
            out
        }
    })
}

/// literal의 JavaScript 값(수·문자열·불리언은 그대로, 나머지는 [`js_value`] 객체).
pub fn literal_js(value: &TypedValue) -> String {
    js_value(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{number, sid};

    struct Names;
    impl CssNames for Names {
        fn name(&self, id: StableId) -> Option<String> {
            Some(format!("--t{}", id.to_bytes()[15]))
        }
        fn literal(&self, value: &TypedValue) -> Option<String> {
            match value {
                TypedValue::Number(n) => Some(crate::text::num(n.get())),
                _ => None,
            }
        }
    }
    struct Outer;
    impl ContextRead for Outer {
        fn context_name(&self, _key: StableId) -> Option<String> {
            Some("--context-k-0".to_string())
        }
    }

    #[test]
    fn css_and_js_lowering_cover_the_closed_operator_table() {
        let expr = IrExpr::Binary {
            op: "*",
            lhs: Box::new(IrExpr::Context(sid(1))),
            rhs: Box::new(IrExpr::Literal(number(2.0))),
        };
        assert_eq!(
            css_expr(&expr, &Names, &Outer).as_deref(),
            Some("calc(var(--context-k-0) * 2)")
        );
        assert_eq!(
            js_expr(&expr, &ReadContext).as_deref(),
            Some("(read.context(\"00000000000000000000000000000001\") * 2)")
        );
        let not_css = IrExpr::If {
            condition: Box::new(IrExpr::State(sid(2))),
            then: Box::new(IrExpr::Literal(number(1.0))),
            otherwise: Box::new(IrExpr::Literal(number(0.0))),
        };
        assert_eq!(css_expr(&not_css, &Names, &Outer), None);
        assert_eq!(
            js_expr(&not_css, &ReadContext).as_deref(),
            Some("(read.state(\"00000000000000000000000000000002\") ? 1 : 0)")
        );
        let unknown = IrExpr::Call {
            function: "remap",
            args: vec![],
        };
        assert_eq!(js_expr(&unknown, &ReadContext), None);
    }
}
