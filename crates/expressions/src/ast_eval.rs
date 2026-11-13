//! AST 기준 evaluator (VIII.4.1, VIII.4.3).
//!
//! bytecode를 거치지 않고 AST를 직접 평가하는 의미 기준 구현입니다. bytecode evaluator와 결과를 비교하는
//! fixture·fuzz 검증에 씁니다. 값 의미는 [`crate::eval`] 공용 함수를 그대로 사용하고, 공유 node는 한 번만
//! 평가합니다. 명시적 작업 stack으로 순회해 재귀하지 않습니다.
//!
//! `if`·`match`·`and`·`or`는 short-circuit입니다(VIII.4.3). 조건·대상·왼쪽 값을 먼저 평가한 뒤 선택된
//! child만 평가하므로, 선택되지 않은 branch의 오류는 발생하지 않습니다.

use std::collections::BTreeMap;

use stellar_core_types::typed_value::TypedValue;

use crate::ast::{ChildSlot, ExprNode, ExprNodeId, ExpressionAst, Literal, NodePayload};
use crate::eval::{
    apply_binary, apply_unary, load_reference, match_arm, short_circuit, truth, EvalError,
    EvalErrorKind, ReferenceValues,
};
use crate::function_eval::apply_function;
use crate::ops::{BinaryOp, BinaryOpClass};
use crate::typecheck::{type_check, ReferenceTypes};

/// 평가 작업.
enum Task {
    /// node 값을 value stack에 push(memo가 있으면 재사용).
    Eval(ExprNodeId),
    /// child 값을 모두 계산한 node에 연산을 적용.
    Apply(ExprNodeId),
    /// `if` 조건 값을 보고 branch 하나만 평가.
    Branch(ExprNodeId),
    /// `and`/`or` 왼쪽 값을 보고 필요할 때만 오른쪽 평가.
    Logic(ExprNodeId),
    /// `match` 대상 값을 보고 arm 하나만 평가.
    Select(ExprNodeId),
    /// stack top을 node 결과로 memo.
    Finish(ExprNodeId),
}

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
    let mut stack: Vec<Option<TypedValue>> = Vec::new();
    let mut tasks = vec![Task::Eval(ast.root())];
    let node_of = |id: ExprNodeId| {
        ast.node(id)
            .ok_or(EvalError::new(Some(id), EvalErrorKind::MalformedProgram))
    };
    let child = |node: &ExprNode, slot: ChildSlot| {
        node.child(slot).ok_or(EvalError::new(
            Some(node.id()),
            EvalErrorKind::MalformedProgram,
        ))
    };
    while let Some(task) = tasks.pop() {
        match task {
            Task::Eval(id) => {
                if let Some(value) = memo.get(&id) {
                    stack.push(value.clone());
                    continue;
                }
                let node = node_of(id)?;
                let fail = |kind| EvalError::new(Some(id), kind);
                match node.payload() {
                    NodePayload::Literal(Literal::Value(value)) => {
                        memo.insert(id, Some(value.clone()));
                        stack.push(Some(value.clone()));
                    }
                    NodePayload::Literal(Literal::Null(_)) => {
                        memo.insert(id, None);
                        stack.push(None);
                    }
                    NodePayload::Reference(target) => {
                        let expected = report
                            .type_of(id)
                            .ok_or(fail(EvalErrorKind::MalformedProgram))?;
                        let value = load_reference(target, expected, values).map_err(fail)?;
                        memo.insert(id, value.clone());
                        stack.push(value);
                    }
                    NodePayload::If => {
                        tasks.push(Task::Branch(id));
                        tasks.push(Task::Eval(child(node, ChildSlot::Condition)?));
                    }
                    NodePayload::Match(_) => {
                        tasks.push(Task::Select(id));
                        tasks.push(Task::Eval(child(node, ChildSlot::Scrutinee)?));
                    }
                    NodePayload::Binary(BinaryOp::And | BinaryOp::Or) => {
                        tasks.push(Task::Logic(id));
                        tasks.push(Task::Eval(child(node, ChildSlot::Lhs)?));
                    }
                    NodePayload::Unary(_) => {
                        tasks.push(Task::Apply(id));
                        tasks.push(Task::Eval(child(node, ChildSlot::Operand)?));
                    }
                    NodePayload::Binary(_) => {
                        tasks.push(Task::Apply(id));
                        tasks.push(Task::Eval(child(node, ChildSlot::Rhs)?));
                        tasks.push(Task::Eval(child(node, ChildSlot::Lhs)?));
                    }
                    NodePayload::Call(function) => {
                        tasks.push(Task::Apply(id));
                        for index in (0..function.signature().arity()).rev() {
                            let slot = ChildSlot::Argument(
                                u16::try_from(index)
                                    .map_err(|_| fail(EvalErrorKind::MalformedProgram))?,
                            );
                            tasks.push(Task::Eval(child(node, slot)?));
                        }
                    }
                }
            }
            Task::Apply(id) => {
                let node = node_of(id)?;
                let fail = |kind| EvalError::new(Some(id), kind);
                let malformed = || fail(EvalErrorKind::MalformedProgram);
                let value = match node.payload() {
                    NodePayload::Unary(op) => {
                        let operand = stack.pop().ok_or_else(malformed)?;
                        apply_unary(*op, &operand).map_err(fail)?
                    }
                    NodePayload::Binary(op) => {
                        let rhs = stack.pop().ok_or_else(malformed)?;
                        let lhs = stack.pop().ok_or_else(malformed)?;
                        apply_binary(*op, &lhs, &rhs).map_err(fail)?
                    }
                    NodePayload::Call(function) => {
                        let arity = function.signature().arity();
                        let start = stack.len().checked_sub(arity).ok_or_else(malformed)?;
                        let args = stack.split_off(start);
                        apply_function(*function, &args).map_err(fail)?
                    }
                    _ => return Err(malformed()),
                };
                memo.insert(id, value.clone());
                stack.push(value);
            }
            Task::Branch(id) => {
                let node = node_of(id)?;
                let condition = stack
                    .pop()
                    .ok_or(EvalError::new(Some(id), EvalErrorKind::MalformedProgram))?;
                let taken = if truth(&condition).map_err(|kind| EvalError::new(Some(id), kind))? {
                    ChildSlot::Then
                } else {
                    ChildSlot::Else
                };
                tasks.push(Task::Finish(id));
                tasks.push(Task::Eval(child(node, taken)?));
            }
            Task::Logic(id) => {
                let node = node_of(id)?;
                let fail = |kind| EvalError::new(Some(id), kind);
                let lhs = stack.pop().ok_or(fail(EvalErrorKind::MalformedProgram))?;
                let lhs = truth(&lhs).map_err(fail)?;
                let op = match node.payload() {
                    NodePayload::Binary(op) => match op.class() {
                        BinaryOpClass::Logical(op) => op,
                        _ => return Err(fail(EvalErrorKind::MalformedProgram)),
                    },
                    _ => return Err(fail(EvalErrorKind::MalformedProgram)),
                };
                match short_circuit(op, lhs) {
                    Some(result) => {
                        let value = Some(TypedValue::Boolean(result));
                        memo.insert(id, value.clone());
                        stack.push(value);
                    }
                    None => {
                        tasks.push(Task::Finish(id));
                        tasks.push(Task::Eval(child(node, ChildSlot::Rhs)?));
                    }
                }
            }
            Task::Select(id) => {
                let node = node_of(id)?;
                let fail = |kind| EvalError::new(Some(id), kind);
                let scrutinee = stack.pop().ok_or(fail(EvalErrorKind::MalformedProgram))?;
                let NodePayload::Match(patterns) = node.payload() else {
                    return Err(fail(EvalErrorKind::MalformedProgram));
                };
                let taken = match match_arm(&scrutinee, patterns) {
                    Some(arm) => ChildSlot::Arm(
                        u16::try_from(arm).map_err(|_| fail(EvalErrorKind::MalformedProgram))?,
                    ),
                    None => ChildSlot::Default,
                };
                tasks.push(Task::Finish(id));
                tasks.push(Task::Eval(child(node, taken)?));
            }
            Task::Finish(id) => {
                let value = stack
                    .last()
                    .cloned()
                    .ok_or(EvalError::new(Some(id), EvalErrorKind::MalformedProgram))?;
                memo.insert(id, value);
            }
        }
    }
    match (stack.pop(), stack.is_empty()) {
        (Some(value), true) => Ok(value),
        _ => Err(EvalError::new(
            Some(ast.root()),
            EvalErrorKind::MalformedProgram,
        )),
    }
}
