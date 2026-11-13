//! AST 기준 evaluator (VIII.4.1, VIII.4.3).
//!
//! bytecode를 거치지 않고 AST를 직접 평가하는 의미 기준 구현입니다. bytecode evaluator와 결과를 비교하는
//! fixture·fuzz 검증에 씁니다. 값 의미는 [`crate::eval`] 공용 함수를 그대로 사용하고, 공유 node는 한 번만
//! 평가합니다. 명시적 작업 stack으로 순회해 재귀하지 않습니다.
//!
//! `if`·`match`·`and`·`or`는 short-circuit입니다(VIII.4.3). 조건·대상·왼쪽 값을 먼저 평가한 뒤 선택된
//! child만 평가하므로, 선택되지 않은 branch의 오류는 발생하지 않습니다.
//!
//! 평가는 bytecode evaluator와 같은 [`EvalLimits`] 안에서만 진행합니다: 처리한 작업 수(instruction budget 자리),
//! 값 stack 깊이(push 전 검사), stack·memo가 보유한 값 bytes. 결과를 memo하는 node는 bytecode와 같이 부모가 둘
//! 이상인 공유 node뿐입니다. 작업 stack은 AST node 수에 비례하므로(입력 크기) 별도 한도를 두지 않으며,
//! `max_call_depth`(공유 block 중첩)는 bytecode 전용입니다. 작업 수와 명령 수는 단위가 달라 한도 근처에서는 두
//! evaluator의 한도 오류 시점이 다를 수 있습니다.

use std::collections::{BTreeMap, BTreeSet};

use stellar_core_types::typed_value::TypedValue;

use crate::ast::{
    ChildSlot, ExprNode, ExprNodeId, ExpressionAst, Literal, NodePayload, ReferenceTarget,
};
use crate::eval::{
    apply_binary, apply_unary, load_reference, match_arm, short_circuit, truth, value_heap_bytes,
    BoundedStack, EvalError, EvalErrorKind, ReferenceValues,
};
use crate::function_eval::apply_function;
use crate::ops::{BinaryOp, BinaryOpClass};
use crate::typecheck::{type_check, ReferenceTypes};
use crate::vm::EvalLimits;

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

impl Task {
    /// 작업 대상 node.
    const fn node(&self) -> ExprNodeId {
        match self {
            Task::Eval(id)
            | Task::Apply(id)
            | Task::Branch(id)
            | Task::Logic(id)
            | Task::Select(id)
            | Task::Finish(id) => *id,
        }
    }
}

/// 공유 node(부모 2개 이상) 결과 memo. 보관한 값의 bytes는 stack 계량에 예약합니다.
struct Memo {
    shared: BTreeSet<ExprNodeId>,
    values: BTreeMap<ExprNodeId, Option<TypedValue>>,
}

impl Memo {
    fn new(ast: &ExpressionAst) -> Memo {
        let shared = ast
            .parent_counts()
            .into_iter()
            .filter(|(_, parents)| *parents >= 2)
            .map(|(id, _)| id)
            .collect();
        Memo {
            shared,
            values: BTreeMap::new(),
        }
    }

    fn is_shared(&self, id: ExprNodeId) -> bool {
        self.shared.contains(&id)
    }

    fn get(&self, id: ExprNodeId) -> Option<&Option<TypedValue>> {
        self.values.get(&id)
    }

    /// 공유 node면 결과를 보관합니다(bytes 한도 검사 후 복제).
    fn record(
        &mut self,
        id: ExprNodeId,
        value: &Option<TypedValue>,
        stack: &mut BoundedStack,
    ) -> Result<(), EvalErrorKind> {
        if self.is_shared(id) && !self.values.contains_key(&id) {
            stack.hold(value_heap_bytes(value))?;
            self.values.insert(id, value.clone());
        }
        Ok(())
    }
}

/// 기본 한도([`EvalLimits::DEFAULT`])로 AST를 직접 평가합니다. type 오류가 있으면 [`EvalErrorKind::IllTyped`]입니다.
pub fn evaluate_ast(
    ast: &ExpressionAst,
    types: &dyn ReferenceTypes,
    values: &dyn ReferenceValues,
) -> Result<Option<TypedValue>, EvalError> {
    evaluate_ast_with_limits(ast, types, values, EvalLimits::DEFAULT)
}

/// 주어진 한도로 AST를 직접 평가합니다. `instruction_budget`은 처리한 작업 수, `max_stack_depth`는 값 stack 깊이,
/// `max_value_bytes`는 stack·memo가 보유한 값 bytes 상한으로 씁니다.
pub fn evaluate_ast_with_limits(
    ast: &ExpressionAst,
    types: &dyn ReferenceTypes,
    values: &dyn ReferenceValues,
    limits: EvalLimits,
) -> Result<Option<TypedValue>, EvalError> {
    run(ast, types, values, limits, None)
}

/// 평가 중 실제로 읽은 참조 하나(VIII.6.5 provenance).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReferenceRead {
    /// 참조 node.
    pub node: ExprNodeId,
    /// 읽은 대상.
    pub target: ReferenceTarget,
}

/// 평가 중 선택한 branch 하나(VIII.6.5 provenance).
///
/// `if`는 `Then`/`Else`, `match`는 `Arm(i)`/`Default`, `and`·`or`는 왼쪽만으로 결과가 정해지면 `Lhs`, 오른쪽까지
/// 평가하면 `Rhs`입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BranchDecision {
    /// 선택 node.
    pub node: ExprNodeId,
    /// 선택된 slot.
    pub taken: ChildSlot,
}

/// 평가 경로: 실제로 읽은 참조와 선택한 branch(평가 순서). short-circuit으로 평가하지 않은 branch의 참조는 없습니다.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct EvaluationPath {
    /// 읽은 참조(공유 node는 한 번).
    pub reads: Vec<ReferenceRead>,
    /// 선택한 branch.
    pub branches: Vec<BranchDecision>,
}

/// AST를 평가하면서 평가 경로(읽은 참조·선택 branch)를 함께 기록합니다(VIII.6.5). 값 의미와 한도는
/// [`evaluate_ast_with_limits`]와 같습니다. 평가가 실패해도 실패 지점까지의 경로를 돌려줍니다.
pub fn evaluate_ast_traced(
    ast: &ExpressionAst,
    types: &dyn ReferenceTypes,
    values: &dyn ReferenceValues,
    limits: EvalLimits,
) -> (Result<Option<TypedValue>, EvalError>, EvaluationPath) {
    let mut path = EvaluationPath::default();
    let result = run(ast, types, values, limits, Some(&mut path));
    (result, path)
}

fn run(
    ast: &ExpressionAst,
    types: &dyn ReferenceTypes,
    values: &dyn ReferenceValues,
    limits: EvalLimits,
    mut path: Option<&mut EvaluationPath>,
) -> Result<Option<TypedValue>, EvalError> {
    let report = type_check(ast, types);
    if let Some(error) = report.errors().first() {
        return Err(EvalError::new(Some(error.node), EvalErrorKind::IllTyped));
    }
    let mut memo = Memo::new(ast);
    let mut stack = BoundedStack::new(limits.max_stack_depth, limits.max_value_bytes);
    let mut tasks = vec![Task::Eval(ast.root())];
    let mut processed: u64 = 0;
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
        if processed >= limits.instruction_budget {
            return Err(EvalError::new(
                Some(task.node()),
                EvalErrorKind::InstructionBudgetExceeded {
                    budget: limits.instruction_budget,
                },
            ));
        }
        processed += 1;
        match task {
            Task::Eval(id) => {
                let fail = |kind| EvalError::new(Some(id), kind);
                if let Some(value) = memo.get(id) {
                    stack.push_clone(value).map_err(fail)?;
                    continue;
                }
                let node = node_of(id)?;
                match node.payload() {
                    NodePayload::Literal(literal) => {
                        let value = match literal {
                            Literal::Value(value) => Some(value.clone()),
                            Literal::Null(_) => None,
                        };
                        memo.record(id, &value, &mut stack).map_err(fail)?;
                        stack.push(value).map_err(fail)?;
                    }
                    NodePayload::Reference(target) => {
                        let expected = report
                            .type_of(id)
                            .ok_or(fail(EvalErrorKind::MalformedProgram))?;
                        let value = load_reference(target, expected, values).map_err(fail)?;
                        if let Some(path) = path.as_deref_mut() {
                            path.reads.push(ReferenceRead {
                                node: id,
                                target: target.clone(),
                            });
                        }
                        memo.record(id, &value, &mut stack).map_err(fail)?;
                        stack.push(value).map_err(fail)?;
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
                        let args = stack
                            .pop_many(function.signature().arity())
                            .ok_or_else(malformed)?;
                        apply_function(*function, &args).map_err(fail)?
                    }
                    _ => return Err(malformed()),
                };
                memo.record(id, &value, &mut stack).map_err(fail)?;
                stack.push(value).map_err(fail)?;
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
                if let Some(path) = path.as_deref_mut() {
                    path.branches.push(BranchDecision { node: id, taken });
                }
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
                if let Some(path) = path.as_deref_mut() {
                    let taken = if short_circuit(op, lhs).is_some() {
                        ChildSlot::Lhs
                    } else {
                        ChildSlot::Rhs
                    };
                    path.branches.push(BranchDecision { node: id, taken });
                }
                match short_circuit(op, lhs) {
                    Some(result) => {
                        let value = Some(TypedValue::Boolean(result));
                        memo.record(id, &value, &mut stack).map_err(fail)?;
                        stack.push(value).map_err(fail)?;
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
                if let Some(path) = path.as_deref_mut() {
                    path.branches.push(BranchDecision { node: id, taken });
                }
                tasks.push(Task::Finish(id));
                tasks.push(Task::Eval(child(node, taken)?));
            }
            Task::Finish(id) => {
                let fail = |kind| EvalError::new(Some(id), kind);
                let value = stack.last().ok_or(fail(EvalErrorKind::MalformedProgram))?;
                if memo.is_shared(id) {
                    let value = value.clone();
                    memo.record(id, &value, &mut stack).map_err(fail)?;
                }
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use stellar_core_types::ids::StableId;
    use stellar_core_types::typed_value::TypedValue;
    use stellar_core_types::value::FiniteF32;

    use super::*;
    use crate::ast::{ExprNode, ReferenceTarget};
    use crate::bytecode::Program;
    use crate::ops::{BinaryOp, UnaryOp};
    use crate::types::TypeSpec;

    fn id(n: u64) -> ExprNodeId {
        ExprNodeId::new(StableId::new_v7(n, 0, n))
    }

    fn one() -> TypedValue {
        TypedValue::Number(FiniteF32::ONE)
    }

    fn num(v: f32) -> TypedValue {
        TypedValue::Number(FiniteF32::new(v).expect("유한"))
    }

    fn types() -> BTreeMap<ReferenceTarget, TypeSpec> {
        BTreeMap::new()
    }

    fn values() -> BTreeMap<ReferenceTarget, Option<TypedValue>> {
        BTreeMap::new()
    }

    /// `-(-(…(-1)))` 단항 chain(깊이 `depth`).
    fn negate_chain(depth: u64) -> ExpressionAst {
        let mut nodes: Vec<ExprNode> = (1..depth)
            .map(|n| ExprNode::unary(id(n), UnaryOp::Negate, id(n + 1)))
            .collect();
        nodes.push(ExprNode::literal(id(depth), one()));
        ExpressionAst::new(id(1), nodes).expect("유효")
    }

    #[test]
    fn work_budget_stops_large_ast() {
        // 회귀(VIII.4.5 리뷰): 이전에는 AST 기준 evaluator에 작업·깊이·메모리 한도가 없었습니다.
        // node 1,000개 chain은 작업 1,999개(Eval 1,000 + Apply 999).
        let ast = negate_chain(1_000);
        let limited = |budget: u64| {
            evaluate_ast_with_limits(
                &ast,
                &types(),
                &values(),
                EvalLimits {
                    instruction_budget: budget,
                    ..EvalLimits::DEFAULT
                },
            )
        };
        let error = limited(100).expect_err("작업 budget 초과");
        assert_eq!(
            error.kind,
            EvalErrorKind::InstructionBudgetExceeded { budget: 100 }
        );
        assert!(error.node.is_some());
        assert!(limited(1_998).is_err(), "1 부족");
        assert_eq!(limited(1_999), Ok(Some(num(-1.0))), "경계 통과");
        // 기본 한도 evaluate_ast는 bytecode 결과와 같습니다.
        let program = Program::compile(&ast, &types()).expect("컴파일");
        assert_eq!(
            evaluate_ast(&ast, &types(), &values()),
            program.evaluate(&values())
        );
    }

    #[test]
    fn value_stack_depth_limit_applies() {
        // 1 + (1 + (… + 1)) — 값 stack 깊이 = depth + 1.
        let depth = 300u64;
        let mut nodes: Vec<ExprNode> = (1..=depth)
            .map(|n| ExprNode::binary(id(n), BinaryOp::Add, id(10_000 + n), id(n + 1)))
            .collect();
        nodes.extend((1..=depth).map(|n| ExprNode::literal(id(10_000 + n), one())));
        nodes.push(ExprNode::literal(id(depth + 1), one()));
        let ast = ExpressionAst::new(id(1), nodes).expect("유효");
        let with_depth = |limit: usize| {
            evaluate_ast_with_limits(
                &ast,
                &types(),
                &values(),
                EvalLimits {
                    max_stack_depth: limit,
                    ..EvalLimits::DEFAULT
                },
            )
        };
        assert_eq!(
            with_depth(300),
            Err(EvalError::new(
                Some(id(depth + 1)),
                EvalErrorKind::StackLimitExceeded { limit: 300 }
            ))
        );
        assert_eq!(with_depth(301), Ok(Some(num(301.0))), "경계 통과");
    }

    #[test]
    fn only_shared_nodes_are_memoized() {
        // (1 + 1) 공유 node는 memo하고 단독 node는 memo하지 않습니다.
        let ast = ExpressionAst::new(
            id(1),
            vec![
                ExprNode::binary(id(1), BinaryOp::Add, id(2), id(2)),
                ExprNode::binary(id(2), BinaryOp::Add, id(3), id(4)),
                ExprNode::literal(id(3), one()),
                ExprNode::literal(id(4), one()),
            ],
        )
        .expect("유효");
        let memo = Memo::new(&ast);
        assert!(memo.is_shared(id(2)));
        assert!(!memo.is_shared(id(1)) && !memo.is_shared(id(3)));
        assert_eq!(evaluate_ast(&ast, &types(), &values()), Ok(Some(num(4.0))));
    }

    #[test]
    fn traced_evaluation_records_reads_and_branches() {
        use crate::ast::EnvironmentKey;
        let flag = ReferenceTarget::Environment(EnvironmentKey::new(1));
        let a = ReferenceTarget::Environment(EnvironmentKey::new(2));
        let b = ReferenceTarget::Environment(EnvironmentKey::new(3));
        let c = ReferenceTarget::Environment(EnvironmentKey::new(4));
        // ((if flag then a else b) == 1) and (flag or c)
        let ast = ExpressionAst::new(
            id(1),
            vec![
                ExprNode::binary(id(1), BinaryOp::And, id(2), id(7)),
                ExprNode::binary(id(2), BinaryOp::Equal, id(3), id(6)),
                ExprNode::if_else(id(3), id(4), id(5), id(8)),
                ExprNode::reference(id(4), flag.clone()),
                ExprNode::reference(id(5), a.clone()),
                ExprNode::literal(id(6), one()),
                ExprNode::binary(id(7), BinaryOp::Or, id(9), id(10)),
                ExprNode::reference(id(8), b.clone()),
                ExprNode::reference(id(9), flag.clone()),
                ExprNode::reference(id(10), c.clone()),
            ],
        )
        .expect("유효");
        let types = BTreeMap::from([
            (
                flag.clone(),
                TypeSpec::required(crate::types::ExpressionType::Boolean),
            ),
            (
                a.clone(),
                TypeSpec::required(crate::types::ExpressionType::Number),
            ),
            (
                b.clone(),
                TypeSpec::required(crate::types::ExpressionType::Number),
            ),
            (
                c.clone(),
                TypeSpec::required(crate::types::ExpressionType::Boolean),
            ),
        ]);
        let values = BTreeMap::from([
            (flag.clone(), Some(TypedValue::Boolean(true))),
            (a.clone(), Some(one())),
            (b.clone(), Some(num(5.0))),
            (c, Some(TypedValue::Boolean(false))),
        ]);
        let (value, path) = evaluate_ast_traced(&ast, &types, &values, EvalLimits::DEFAULT);
        let value = value.expect("평가");
        assert_eq!(value, Some(TypedValue::Boolean(true)));
        assert_eq!(value, evaluate_ast(&ast, &types, &values).expect("평가"));
        // b(Else branch)와 c(or의 오른쪽)는 읽지 않습니다.
        assert_eq!(
            path.reads,
            vec![
                ReferenceRead {
                    node: id(4),
                    target: flag.clone()
                },
                ReferenceRead {
                    node: id(5),
                    target: a
                },
                ReferenceRead {
                    node: id(9),
                    target: flag
                },
            ]
        );
        assert_eq!(
            path.branches,
            vec![
                BranchDecision {
                    node: id(3),
                    taken: ChildSlot::Then
                },
                BranchDecision {
                    node: id(1),
                    taken: ChildSlot::Rhs
                },
                BranchDecision {
                    node: id(7),
                    taken: ChildSlot::Lhs
                },
            ]
        );
    }
}
