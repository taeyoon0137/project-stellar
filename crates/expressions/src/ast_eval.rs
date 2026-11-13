//! AST 기준 evaluator (VIII.4.1).
//!
//! bytecode를 거치지 않고 AST를 직접 평가하는 의미 기준 구현입니다. bytecode evaluator와 결과를 비교하는
//! fixture·fuzz 검증에 씁니다. 값 의미는 [`crate::eval`] 공용 함수를 그대로 사용하고, 공유 node는 한 번만
//! 평가합니다. 명시적 stack으로 순회해 재귀하지 않습니다.

use std::collections::BTreeMap;

use stellar_core_types::typed_value::TypedValue;

use crate::ast::{ChildSlot, ExprNodeId, ExpressionAst, Literal, NodePayload};
use crate::eval::{
    apply_binary, apply_unary, load_reference, match_arm, select_if, EvalError, EvalErrorKind,
    ReferenceValues,
};
use crate::typecheck::{type_check, ReferenceTypes};

/// AST를 직접 평가합니다. type 오류가 있으면 [`EvalErrorKind::IllTyped`]입니다.
pub fn evaluate_ast(
    ast: &ExpressionAst,
    types: &dyn ReferenceTypes,
    values: &dyn ReferenceValues,
) -> Result<Option<TypedValue>, EvalError> {
    let report = type_check(ast, types);
    if let Some(error) = report.errors().first() {
        return Err(EvalError::new(Some(error.node), EvalErrorKind::IllTyped));
    }
    let mut memo: BTreeMap<ExprNodeId, Option<TypedValue>> = BTreeMap::new();
    // (node, child 확장 완료 여부)
    let mut stack = vec![(ast.root(), false)];
    while let Some((id, expanded)) = stack.pop() {
        if memo.contains_key(&id) {
            continue;
        }
        let node = ast
            .node(id)
            .ok_or(EvalError::new(Some(id), EvalErrorKind::MalformedProgram))?;
        if !expanded {
            stack.push((id, true));
            for child in node.children().iter().rev() {
                if !memo.contains_key(&child.target) {
                    stack.push((child.target, false));
                }
            }
            continue;
        }
        let fail = |kind| EvalError::new(Some(id), kind);
        let value_of = |slot: ChildSlot| -> Result<Option<TypedValue>, EvalError> {
            node.child(slot)
                .and_then(|child| memo.get(&child).cloned())
                .ok_or(EvalError::new(Some(id), EvalErrorKind::MalformedProgram))
        };
        let value = match node.payload() {
            NodePayload::Literal(Literal::Value(value)) => Some(value.clone()),
            NodePayload::Literal(Literal::Null(_)) => None,
            NodePayload::Reference(target) => {
                let expected = report
                    .type_of(id)
                    .ok_or(fail(EvalErrorKind::MalformedProgram))?;
                load_reference(target, expected, values).map_err(fail)?
            }
            NodePayload::If => select_if(
                &value_of(ChildSlot::Condition)?,
                value_of(ChildSlot::Then)?,
                value_of(ChildSlot::Else)?,
            )
            .map_err(fail)?,
            NodePayload::Match(patterns) => {
                let scrutinee = value_of(ChildSlot::Scrutinee)?;
                match match_arm(&scrutinee, patterns) {
                    Some(arm) => value_of(ChildSlot::Arm(
                        u16::try_from(arm).map_err(|_| fail(EvalErrorKind::MalformedProgram))?,
                    ))?,
                    None => value_of(ChildSlot::Default)?,
                }
            }
            NodePayload::Unary(op) => {
                apply_unary(*op, &value_of(ChildSlot::Operand)?).map_err(fail)?
            }
            NodePayload::Binary(op) => {
                apply_binary(*op, &value_of(ChildSlot::Lhs)?, &value_of(ChildSlot::Rhs)?)
                    .map_err(fail)?
            }
            NodePayload::Call(_) => return Err(fail(EvalErrorKind::UnsupportedNode)),
        };
        memo.insert(id, value);
    }
    memo.remove(&ast.root()).ok_or(EvalError::new(
        Some(ast.root()),
        EvalErrorKind::MalformedProgram,
    ))
}
