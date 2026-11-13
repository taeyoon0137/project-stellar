//! bytecode evaluator (VIII.4.1~VIII.4.5).
//!
//! 평가는 [`EvalLimits`]의 instruction budget, frame(공유 block 중첩) 깊이, operand stack 깊이, 보유 값 bytes
//! 안에서만 진행합니다(VIII.4.5). 한도를 넘는 비정상적으로 큰 표현식은 typed 오류로 종료하며 Rust stack을 쓰지
//! 않습니다.
//!
//! `if`·`match`·`and`·`or`는 compiler가 만든 jump로 short-circuit 실행합니다(VIII.4.3). 선택되지 않은 branch의
//! 명령·공유 block은 실행하지 않으므로 그 안의 오류도 발생하지 않습니다.
//!
//! [`Program`]을 operand stack과 명시적 frame stack으로 실행합니다. Rust 재귀를 쓰지 않으며, 공유 block은
//! 처음 [`Instr::Eval`]에서 한 번 평가한 뒤 결과를 캐시합니다. 값 의미는 [`crate::eval`]의 공용 함수를 써서
//! AST 기준 evaluator([`crate::ast_eval`])와 같은 결과를 냅니다.

use stellar_core_types::typed_value::TypedValue;

use crate::bytecode::{Instr, Program};
use crate::eval::{
    apply_binary, apply_unary, load_reference, truth, value_heap_bytes, BoundedStack, EvalError,
    EvalErrorKind, ReferenceValues,
};
use crate::function_eval::apply_function;

/// 평가 실행 기록.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EvalTrace {
    /// 실행한 명령 수.
    pub instructions: u64,
    /// 실행한 block 수(공유 block 캐시 적중은 제외).
    pub block_runs: u64,
    /// operand stack의 최대 깊이. 항상 [`EvalLimits::max_stack_depth`] 이하입니다.
    pub peak_stack_depth: usize,
    /// stack과 공유 결과 캐시가 동시에 보유한 최대 값 bytes. 항상 [`EvalLimits::max_value_bytes`] 이하입니다.
    pub peak_value_bytes: usize,
}

/// 평가 자원 제한 (VIII.4.5).
///
/// evaluator는 Rust 재귀 없이 명시적 frame stack으로 실행하며, 아래 한도를 넘으면 typed 오류로 즉시 종료합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EvalLimits {
    /// 한 번의 평가에서 실행할 수 있는 최대 명령 수.
    pub instruction_budget: u64,
    /// 동시에 열 수 있는 최대 frame 수(root block 포함). 공유 block 중첩 깊이 제한입니다.
    pub max_call_depth: usize,
    /// operand stack 최대 깊이.
    pub max_stack_depth: usize,
    /// operand stack과 공유 결과 캐시가 함께 보유할 수 있는 값의 최대 heap bytes
    /// ([`value_heap_bytes`](crate::eval::value_heap_bytes) 합). 공유 문자열의 복제가 stack에 쌓이는 증폭을
    /// 막습니다. 진행 중인 함수 결과 하나([`MAX_STRING_BYTES`](crate::eval::MAX_STRING_BYTES) 이하)는 검사 직전에
    /// 잠시 더 존재할 수 있습니다.
    pub max_value_bytes: usize,
}

impl EvalLimits {
    /// 기본 한도: 명령 1,000,000개, frame 256개, operand 65,536개, 값 16 MiB(문자열 결과 상한의 16배).
    pub const DEFAULT: EvalLimits = EvalLimits {
        instruction_budget: 1_000_000,
        max_call_depth: 256,
        max_stack_depth: 65_536,
        max_value_bytes: 16 * crate::eval::MAX_STRING_BYTES,
    };
}

impl Default for EvalLimits {
    fn default() -> EvalLimits {
        EvalLimits::DEFAULT
    }
}

struct Frame {
    block: usize,
    pc: usize,
}

impl Program {
    /// 기본 한도([`EvalLimits::DEFAULT`])로 program을 평가합니다.
    pub fn evaluate(&self, values: &dyn ReferenceValues) -> Result<Option<TypedValue>, EvalError> {
        self.evaluate_traced(values).map(|(value, _)| value)
    }

    /// 기본 한도로 평가하고 실행 기록을 함께 돌려줍니다.
    pub fn evaluate_traced(
        &self,
        values: &dyn ReferenceValues,
    ) -> Result<(Option<TypedValue>, EvalTrace), EvalError> {
        self.evaluate_with_limits(values, EvalLimits::DEFAULT)
    }

    /// 주어진 한도로 평가하고 실행 기록을 함께 돌려줍니다(VIII.4.5).
    ///
    /// operand stack 깊이와 값 bytes는 push 전에 검사하므로 stack은 어떤 순간에도 `max_stack_depth`개를 넘지 않고,
    /// stack·공유 결과 캐시가 보유한 값은 `max_value_bytes`를 넘지 않습니다.
    pub fn evaluate_with_limits(
        &self,
        values: &dyn ReferenceValues,
        limits: EvalLimits,
    ) -> Result<(Option<TypedValue>, EvalTrace), EvalError> {
        let mut trace = EvalTrace::default();
        let mut stack = BoundedStack::new(limits.max_stack_depth, limits.max_value_bytes);
        let mut cache: Vec<Option<Option<TypedValue>>> = vec![None; self.blocks().len()];
        let mut frames = vec![Frame { block: 0, pc: 0 }];
        trace.block_runs = 1;
        let malformed = |node| EvalError::new(node, EvalErrorKind::MalformedProgram);
        loop {
            let Some(frame) = frames.last_mut() else {
                return Err(malformed(None));
            };
            let block = self
                .blocks()
                .get(frame.block)
                .ok_or_else(|| malformed(None))?;
            let instr = *block.code().get(frame.pc).ok_or_else(|| malformed(None))?;
            let node = block.sources().get(frame.pc).copied();
            if trace.instructions >= limits.instruction_budget {
                return Err(EvalError::new(
                    node,
                    EvalErrorKind::InstructionBudgetExceeded {
                        budget: limits.instruction_budget,
                    },
                ));
            }
            frame.pc += 1;
            trace.instructions += 1;
            let fail = |kind| EvalError::new(node, kind);
            match instr {
                Instr::Const(index) => {
                    let value = self
                        .constants()
                        .get(index as usize)
                        .ok_or_else(|| malformed(node))?;
                    stack.push_clone(value).map_err(fail)?;
                }
                Instr::LoadRef(index) => {
                    let (target, expected) = self
                        .references()
                        .get(index as usize)
                        .ok_or_else(|| malformed(node))?;
                    let value = load_reference(target, *expected, values).map_err(fail)?;
                    stack.push(value).map_err(fail)?;
                }
                Instr::Eval(index) => {
                    let slot = cache.get(index as usize).ok_or_else(|| malformed(node))?;
                    match slot {
                        Some(value) => stack.push_clone(value).map_err(fail)?,
                        None => {
                            if frames.len() >= limits.max_call_depth {
                                return Err(fail(EvalErrorKind::CallDepthExceeded {
                                    limit: limits.max_call_depth,
                                }));
                            }
                            trace.block_runs += 1;
                            frames.push(Frame {
                                block: index as usize,
                                pc: 0,
                            });
                        }
                    }
                }
                Instr::JumpIfFalse(target) => {
                    let condition = stack.pop().ok_or_else(|| malformed(node))?;
                    if !truth(&condition).map_err(fail)? {
                        frame.pc = target as usize;
                    }
                }
                Instr::Jump(target) => frame.pc = target as usize,
                Instr::MatchCase { pattern, miss } => {
                    let pattern = self
                        .constants()
                        .get(pattern as usize)
                        .ok_or_else(|| malformed(node))?;
                    let scrutinee = stack.last().ok_or_else(|| malformed(node))?;
                    if scrutinee.is_some() && scrutinee == pattern {
                        stack.pop();
                    } else {
                        frame.pc = miss as usize;
                    }
                }
                Instr::Pop => {
                    stack.pop().ok_or_else(|| malformed(node))?;
                }
                Instr::Unary(op) => {
                    let operand = stack.pop().ok_or_else(|| malformed(node))?;
                    stack
                        .push(apply_unary(op, &operand).map_err(fail)?)
                        .map_err(fail)?;
                }
                Instr::Binary(op) => {
                    let rhs = stack.pop().ok_or_else(|| malformed(node))?;
                    let lhs = stack.pop().ok_or_else(|| malformed(node))?;
                    stack
                        .push(apply_binary(op, &lhs, &rhs).map_err(fail)?)
                        .map_err(fail)?;
                }
                Instr::Call { function, argc } => {
                    let args = stack
                        .pop_many(usize::from(argc))
                        .ok_or_else(|| malformed(node))?;
                    stack
                        .push(apply_function(function, &args).map_err(fail)?)
                        .map_err(fail)?;
                }
                Instr::Return => {
                    let value = stack.pop().ok_or_else(|| malformed(node))?;
                    let finished = frames.pop().ok_or_else(|| malformed(node))?;
                    if frames.is_empty() {
                        trace.peak_stack_depth = stack.peak_depth();
                        trace.peak_value_bytes = stack.peak_bytes();
                        return if stack.is_empty() {
                            Ok((value, trace))
                        } else {
                            Err(malformed(node))
                        };
                    }
                    if let Some(slot) = cache.get_mut(finished.block) {
                        // 캐시 복제도 같은 값 bytes 한도에 포함합니다(평가가 끝날 때까지 보유).
                        stack.hold(value_heap_bytes(&value)).map_err(fail)?;
                        *slot = Some(value.clone());
                    }
                    stack.push(value).map_err(fail)?;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use stellar_core_types::ids::StableId;
    use stellar_core_types::reference::TokenRef;
    use stellar_core_types::typed_value::{EnumValue, TypedValue};
    use stellar_core_types::value::Length;

    use super::*;
    use crate::ast::{EnvironmentKey, ExprNode, ExprNodeId, ExpressionAst, ReferenceTarget};
    use crate::ast_eval::evaluate_ast;
    use crate::bytecode::CompileError;
    use crate::ops::BinaryOp;
    use crate::types::{EnumTypeId, ExpressionType, TypeSpec};

    fn id(n: u8) -> ExprNodeId {
        ExprNodeId::new(StableId::from_bytes([n; 16]))
    }

    fn len(v: f32) -> TypedValue {
        TypedValue::Length(Length::new(v).expect("유한"))
    }

    fn token(n: u8) -> ReferenceTarget {
        ReferenceTarget::Token(TokenRef::new(StableId::from_bytes([0xA0 + n; 16])))
    }

    fn env(n: u32) -> ReferenceTarget {
        ReferenceTarget::Environment(EnvironmentKey::new(n))
    }

    fn types() -> BTreeMap<ReferenceTarget, TypeSpec> {
        BTreeMap::from([
            (env(1), TypeSpec::required(ExpressionType::Boolean)),
            (env(2), TypeSpec::required(ExpressionType::Boolean)),
            (token(1), TypeSpec::required(ExpressionType::Length)),
            (
                token(2),
                TypeSpec::required(ExpressionType::Enum(EnumTypeId::new(7))),
            ),
            (token(3), TypeSpec::optional(ExpressionType::Length)),
        ])
    }

    /// `match token2 { E0 => S, E1 => S, E2 => if env2 then null else token3 } else 4px`,
    /// `S = if env1 then token1 else 2px` (공유 if).
    fn fixture() -> ExpressionAst {
        ExpressionAst::new(
            id(1),
            vec![
                ExprNode::match_on(
                    id(1),
                    id(2),
                    vec![
                        (TypedValue::Enum(EnumValue::new(7, 0)), id(3)),
                        (TypedValue::Enum(EnumValue::new(7, 1)), id(3)),
                        (TypedValue::Enum(EnumValue::new(7, 2)), id(4)),
                    ],
                    id(5),
                ),
                ExprNode::reference(id(2), token(2)),
                ExprNode::if_else(id(3), id(6), id(7), id(8)),
                ExprNode::if_else(id(4), id(9), id(10), id(11)),
                ExprNode::literal(id(5), len(4.0)),
                ExprNode::reference(id(6), env(1)),
                ExprNode::reference(id(7), token(1)),
                ExprNode::literal(id(8), len(2.0)),
                ExprNode::reference(id(9), env(2)),
                ExprNode::null(id(10), ExpressionType::Length),
                ExprNode::reference(id(11), token(3)),
            ],
        )
        .expect("유효")
    }

    /// 입력 조합 전체: enum variant 0~3 × env1 × env2 × token3(null/값) × token1 값.
    fn environments() -> Vec<BTreeMap<ReferenceTarget, Option<TypedValue>>> {
        let mut out = Vec::new();
        for variant in 0..4u32 {
            for e1 in [false, true] {
                for e2 in [false, true] {
                    for t3 in [None, Some(len(9.0))] {
                        out.push(BTreeMap::from([
                            (token(2), Some(TypedValue::Enum(EnumValue::new(7, variant)))),
                            (env(1), Some(TypedValue::Boolean(e1))),
                            (env(2), Some(TypedValue::Boolean(e2))),
                            (token(3), t3.clone()),
                            (token(1), Some(len(10.0 + variant as f32))),
                        ]));
                    }
                }
            }
        }
        out
    }

    #[test]
    fn ast_and_bytecode_results_match_on_fixture_corpus() {
        let ast = fixture();
        let program = Program::compile(&ast, &types()).expect("컴파일");
        assert_eq!(
            program.result_type(),
            TypeSpec::optional(ExpressionType::Length)
        );
        let mut seen = std::collections::BTreeSet::new();
        for values in environments() {
            let by_ast = evaluate_ast(&ast, &types(), &values);
            let by_bytecode = program.evaluate(&values);
            assert_eq!(by_bytecode, by_ast, "입력 {values:?}");
            seen.insert(format!("{by_ast:?}"));
        }
        // 서로 다른 결과(4px, 2px, token1 값들, null, token3)가 실제로 나왔는지.
        assert!(seen.len() >= 6, "{seen:?}");
    }

    #[test]
    fn expected_values_for_selected_inputs() {
        let ast = fixture();
        let program = Program::compile(&ast, &types()).expect("컴파일");
        let mut values = environments().remove(0);
        // variant 0, env1 false → S = 2px.
        assert_eq!(program.evaluate(&values), Ok(Some(len(2.0))));
        values.insert(env(1), Some(TypedValue::Boolean(true)));
        assert_eq!(program.evaluate(&values), Ok(Some(len(10.0))));
        values.insert(token(2), Some(TypedValue::Enum(EnumValue::new(7, 2))));
        values.insert(env(2), Some(TypedValue::Boolean(true)));
        assert_eq!(program.evaluate(&values), Ok(None), "null branch");
        values.insert(token(2), Some(TypedValue::Enum(EnumValue::new(7, 3))));
        assert_eq!(program.evaluate(&values), Ok(Some(len(4.0))), "기본 branch");
    }

    #[test]
    fn shared_node_is_one_block_evaluated_once() {
        let ast = fixture();
        let program = Program::compile(&ast, &types()).expect("컴파일");
        assert_eq!(program.blocks().len(), 2, "root + 공유 if");
        assert_eq!(program.blocks()[1].node(), id(3));
        let evals = program.blocks()[0]
            .code()
            .iter()
            .filter(|instr| matches!(instr, Instr::Eval(1)))
            .count();
        assert_eq!(evals, 2, "공유 block을 두 arm이 호출");
        let (_, trace) = program
            .evaluate_traced(&environments().remove(0))
            .expect("평가");
        assert_eq!(
            trace.block_runs, 2,
            "공유 block은 한 번만 실행(두 번째는 캐시)"
        );
        // short-circuit(VIII.4.3): 선택되지 않은 branch 명령은 실행하지 않습니다.
        assert!((trace.instructions as usize) < program.instruction_count());
        // 명령마다 원인 node가 있음.
        for block in program.blocks() {
            assert_eq!(block.code().len(), block.sources().len());
            assert_eq!(block.code().last(), Some(&Instr::Return));
        }
        assert!(program.disassemble().starts_with("block 0:\n"));
    }

    #[test]
    fn constants_and_references_are_deduplicated() {
        // if env1 then 2px else (if env1 then 2px else 2px) — 같은 값 literal 3개, 같은 참조 2개.
        let ast = ExpressionAst::new(
            id(1),
            vec![
                ExprNode::if_else(id(1), id(2), id(3), id(4)),
                ExprNode::reference(id(2), env(1)),
                ExprNode::literal(id(3), len(2.0)),
                ExprNode::if_else(id(4), id(5), id(6), id(7)),
                ExprNode::reference(id(5), env(1)),
                ExprNode::literal(id(6), len(2.0)),
                ExprNode::literal(id(7), len(2.0)),
            ],
        )
        .expect("유효");
        let program = Program::compile(&ast, &types()).expect("컴파일");
        assert_eq!(program.constants(), &[Some(len(2.0))]);
        assert_eq!(program.references().len(), 1);
        let values = BTreeMap::from([(env(1), Some(TypedValue::Boolean(false)))]);
        assert_eq!(
            program.evaluate(&values),
            evaluate_ast(&ast, &types(), &values)
        );
    }

    #[test]
    fn errors_match_and_point_to_nodes() {
        let ast = fixture();
        let program = Program::compile(&ast, &types()).expect("컴파일");
        let mut values = environments().remove(0);
        values.remove(&token(1));
        // token1을 읽는 branch가 선택되도록 env1 = true(VIII.4.3 이후 미선택 branch는 평가하지 않음).
        values.insert(env(1), Some(TypedValue::Boolean(true)));
        let expected = EvalError::new(Some(id(7)), EvalErrorKind::MissingReference(token(1)));
        assert_eq!(program.evaluate(&values), Err(expected.clone()));
        assert_eq!(evaluate_ast(&ast, &types(), &values), Err(expected));
        // 타입이 다른 참조 값.
        let mut values = environments().remove(0);
        values.insert(env(1), Some(len(1.0)));
        let by_bytecode = program.evaluate(&values);
        assert_eq!(by_bytecode, evaluate_ast(&ast, &types(), &values));
        assert!(matches!(
            by_bytecode,
            Err(EvalError {
                node: Some(node),
                kind: EvalErrorKind::ReferenceTypeMismatch { .. }
            }) if node == id(6)
        ));
    }

    #[test]
    fn compile_rejects_ill_typed_and_unsupported_nodes() {
        let ill = ExpressionAst::new(
            id(1),
            vec![
                ExprNode::if_else(id(1), id(2), id(2), id(2)),
                ExprNode::literal(id(2), len(1.0)),
            ],
        )
        .expect("구조 유효");
        assert!(matches!(
            Program::compile(&ill, &types()),
            Err(CompileError::TypeErrors(errors)) if errors.len() == 1
        ));
        // VIII.4.4부터 함수 호출도 컴파일·평가합니다(미지원 node 없음).
        let call = ExpressionAst::new(
            id(1),
            vec![
                ExprNode::call(id(1), FunctionId::Abs, &[id(2)]),
                ExprNode::literal(id(2), len(-1.0)),
            ],
        )
        .expect("유효");
        let program = Program::compile(&call, &types()).expect("컴파일");
        assert_eq!(program.evaluate(&BTreeMap::new()), Ok(Some(len(1.0))));
        assert!(program.blocks()[0].code().iter().any(|instr| matches!(
            instr,
            Instr::Call {
                function: FunctionId::Abs,
                argc: 1
            }
        )));
    }

    // --- VIII.4.2: literal·산술·비교·논리 ---

    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::value::{Angle, Duration, FiniteF32, Percentage};

    use crate::functions::FunctionId;
    use crate::ops::UnaryOp;

    fn num(v: f32) -> TypedValue {
        TypedValue::Number(FiniteF32::new(v).expect("유한"))
    }

    fn pct(v: f32) -> TypedValue {
        TypedValue::Percentage(Percentage::new(v).expect("유한"))
    }

    fn deg(v: f32) -> TypedValue {
        TypedValue::Angle(Angle::new(v).expect("유한"))
    }

    fn secs(v: f32) -> TypedValue {
        TypedValue::Duration(Duration::new(v).expect("유한"))
    }

    fn literal_node(n: u8, value: &Option<TypedValue>, null_type: ExpressionType) -> ExprNode {
        match value {
            Some(value) => ExprNode::literal(id(n), value.clone()),
            None => ExprNode::null(id(n), null_type),
        }
    }

    /// `lhs op rhs`를 literal AST로 만들어 VM·AST evaluator 양쪽으로 평가합니다.
    fn eval_binary(
        op: BinaryOp,
        lhs: Option<TypedValue>,
        rhs: Option<TypedValue>,
        null_type: ExpressionType,
    ) -> Result<Option<TypedValue>, EvalError> {
        let ast = ExpressionAst::new(
            id(1),
            vec![
                ExprNode::binary(id(1), op, id(2), id(3)),
                literal_node(2, &lhs, null_type),
                literal_node(3, &rhs, null_type),
            ],
        )
        .expect("유효");
        let program = Program::compile(&ast, &types()).expect("컴파일");
        let values = BTreeMap::new();
        let by_bytecode = program.evaluate(&values);
        assert_eq!(
            by_bytecode,
            evaluate_ast(&ast, &types(), &values),
            "{op} AST/bytecode"
        );
        by_bytecode
    }

    fn ok(value: TypedValue) -> Result<Option<TypedValue>, EvalErrorKind> {
        Ok(Some(value))
    }

    #[test]
    fn arithmetic_table() {
        use BinaryOp::{Add, Divide, Multiply, Remainder, Subtract};
        type Case = (
            BinaryOp,
            TypedValue,
            TypedValue,
            Result<Option<TypedValue>, EvalErrorKind>,
        );
        let cases: Vec<Case> = vec![
            (Add, num(1.5), num(2.25), ok(num(3.75))),
            (Subtract, num(1.0), num(3.0), ok(num(-2.0))),
            (Multiply, num(-2.0), num(0.5), ok(num(-1.0))),
            (Divide, num(1.0), num(4.0), ok(num(0.25))),
            (Remainder, num(7.0), num(3.0), ok(num(1.0))),
            (Remainder, num(-7.0), num(3.0), ok(num(-1.0))),
            (Remainder, num(7.5), num(2.0), ok(num(1.5))),
            (Add, len(8.0), len(4.0), ok(len(12.0))),
            (Subtract, len(8.0), len(10.0), ok(len(-2.0))),
            (Multiply, len(8.0), num(1.5), ok(len(12.0))),
            (Multiply, num(1.5), len(8.0), ok(len(12.0))),
            (Divide, len(9.0), num(2.0), ok(len(4.5))),
            (Divide, len(9.0), len(2.0), ok(num(4.5))),
            (Remainder, len(9.0), len(4.0), ok(len(1.0))),
            (Add, pct(50.0), pct(25.0), ok(pct(75.0))),
            (Divide, pct(50.0), pct(25.0), ok(num(2.0))),
            (Subtract, deg(90.0), deg(180.0), ok(deg(-90.0))),
            (Multiply, deg(90.0), num(4.0), ok(deg(360.0))),
            (Remainder, deg(450.0), deg(360.0), ok(deg(90.0))),
            (Add, secs(0.25), secs(0.5), ok(secs(0.75))),
            (Divide, secs(1.0), num(4.0), ok(secs(0.25))),
            // -0.0 결과는 +0.0으로 정규화.
            (Multiply, num(0.0), num(-1.0), ok(num(0.0))),
            (Subtract, len(1.0), len(1.0), ok(len(0.0))),
            // 0으로 나누기·0에 대한 나머지는 typed error.
            (
                Divide,
                num(1.0),
                num(0.0),
                Err(EvalErrorKind::DivisionByZero),
            ),
            (
                Divide,
                num(0.0),
                num(0.0),
                Err(EvalErrorKind::DivisionByZero),
            ),
            (
                Divide,
                len(1.0),
                num(0.0),
                Err(EvalErrorKind::DivisionByZero),
            ),
            (
                Divide,
                len(1.0),
                len(0.0),
                Err(EvalErrorKind::DivisionByZero),
            ),
            (
                Remainder,
                num(5.0),
                num(0.0),
                Err(EvalErrorKind::DivisionByZero),
            ),
            (
                Remainder,
                deg(5.0),
                deg(0.0),
                Err(EvalErrorKind::DivisionByZero),
            ),
            // overflow는 typed error(무한대·saturate 없음).
            (
                Multiply,
                num(f32::MAX),
                num(2.0),
                Err(EvalErrorKind::NonFiniteResult),
            ),
            (
                Add,
                len(f32::MAX),
                len(f32::MAX),
                Err(EvalErrorKind::NonFiniteResult),
            ),
            (
                Divide,
                num(f32::MAX),
                num(0.5),
                Err(EvalErrorKind::NonFiniteResult),
            ),
            (
                Subtract,
                num(-f32::MAX),
                num(f32::MAX),
                Err(EvalErrorKind::NonFiniteResult),
            ),
        ];
        for (op, lhs, rhs, expected) in cases {
            assert_eq!(
                apply_binary(op, &Some(lhs.clone()), &Some(rhs.clone())),
                expected,
                "의미 {lhs:?} {op} {rhs:?}"
            );
            let evaluated = eval_binary(
                op,
                Some(lhs.clone()),
                Some(rhs.clone()),
                ExpressionType::Number,
            );
            let expected_eval = expected.map_err(|kind| EvalError::new(Some(id(1)), kind));
            assert_eq!(evaluated, expected_eval, "평가 {lhs:?} {op} {rhs:?}");
        }
    }

    #[test]
    fn comparison_table() {
        use BinaryOp::{Equal, Greater, GreaterEqual, Less, LessEqual, NotEqual};
        let t = Some(TypedValue::Boolean(true));
        let f = Some(TypedValue::Boolean(false));
        let red = TypedValue::Color(Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 1.0).expect("색"));
        let red_p3 =
            TypedValue::Color(Color::new(ColorSpace::DisplayP3, 1.0, 0.0, 0.0, 1.0).expect("색"));
        type Case = (
            BinaryOp,
            Option<TypedValue>,
            Option<TypedValue>,
            ExpressionType,
            Option<TypedValue>,
        );
        let cases: Vec<Case> = vec![
            (
                Less,
                Some(num(1.0)),
                Some(num(2.0)),
                ExpressionType::Number,
                t.clone(),
            ),
            (
                Less,
                Some(num(2.0)),
                Some(num(2.0)),
                ExpressionType::Number,
                f.clone(),
            ),
            (
                LessEqual,
                Some(num(2.0)),
                Some(num(2.0)),
                ExpressionType::Number,
                t.clone(),
            ),
            (
                Greater,
                Some(len(3.0)),
                Some(len(-3.0)),
                ExpressionType::Length,
                t.clone(),
            ),
            (
                GreaterEqual,
                Some(deg(1.0)),
                Some(deg(2.0)),
                ExpressionType::Angle,
                f.clone(),
            ),
            (
                Equal,
                Some(len(1.0)),
                Some(len(1.0)),
                ExpressionType::Length,
                t.clone(),
            ),
            (
                NotEqual,
                Some(len(1.0)),
                Some(len(2.0)),
                ExpressionType::Length,
                t.clone(),
            ),
            (
                Equal,
                Some(num(0.0)),
                Some(num(-0.0)),
                ExpressionType::Number,
                t.clone(),
            ),
            (
                Equal,
                Some(red.clone()),
                Some(red.clone()),
                ExpressionType::Color,
                t.clone(),
            ),
            (
                Equal,
                Some(red.clone()),
                Some(red_p3),
                ExpressionType::Color,
                f.clone(),
            ),
            (
                Equal,
                Some(TypedValue::String("가".to_string())),
                Some(TypedValue::String("가".to_string())),
                ExpressionType::String,
                t.clone(),
            ),
            // nullable 동등 비교: null == null, null != 값.
            (Equal, None, None, ExpressionType::Length, t.clone()),
            (
                Equal,
                None,
                Some(len(1.0)),
                ExpressionType::Length,
                f.clone(),
            ),
            (
                NotEqual,
                Some(len(1.0)),
                None,
                ExpressionType::Length,
                t.clone(),
            ),
        ];
        for (op, lhs, rhs, null_type, expected) in cases {
            assert_eq!(
                eval_binary(op, lhs.clone(), rhs.clone(), null_type),
                Ok(expected),
                "{lhs:?} {op} {rhs:?}"
            );
        }
    }

    #[test]
    fn logic_and_unary_tables() {
        for a in [false, true] {
            for b in [false, true] {
                let lhs = Some(TypedValue::Boolean(a));
                let rhs = Some(TypedValue::Boolean(b));
                assert_eq!(
                    eval_binary(
                        BinaryOp::And,
                        lhs.clone(),
                        rhs.clone(),
                        ExpressionType::Boolean
                    ),
                    Ok(Some(TypedValue::Boolean(a && b)))
                );
                assert_eq!(
                    eval_binary(BinaryOp::Or, lhs, rhs, ExpressionType::Boolean),
                    Ok(Some(TypedValue::Boolean(a || b)))
                );
            }
        }
        let unary_cases: Vec<(UnaryOp, TypedValue, TypedValue)> = vec![
            (
                UnaryOp::Not,
                TypedValue::Boolean(true),
                TypedValue::Boolean(false),
            ),
            (
                UnaryOp::Not,
                TypedValue::Boolean(false),
                TypedValue::Boolean(true),
            ),
            (UnaryOp::Negate, num(2.5), num(-2.5)),
            (UnaryOp::Negate, num(0.0), num(0.0)),
            (UnaryOp::Negate, len(-4.0), len(4.0)),
            (UnaryOp::Negate, pct(10.0), pct(-10.0)),
            (UnaryOp::Negate, deg(-0.0), deg(0.0)),
            (UnaryOp::Negate, secs(f32::MAX), secs(-f32::MAX)),
        ];
        for (op, operand, expected) in unary_cases {
            let ast = ExpressionAst::new(
                id(1),
                vec![
                    ExprNode::unary(id(1), op, id(2)),
                    ExprNode::literal(id(2), operand.clone()),
                ],
            )
            .expect("유효");
            let program = Program::compile(&ast, &types()).expect("컴파일");
            let values = BTreeMap::new();
            assert_eq!(
                program.evaluate(&values),
                Ok(Some(expected.clone())),
                "{op} {operand:?}"
            );
            assert_eq!(evaluate_ast(&ast, &types(), &values), Ok(Some(expected)));
        }
    }

    #[test]
    fn nested_arithmetic_with_references_matches_ast() {
        // (token1 * 2 + 4px) / (token1 - 1px) — token1 공유, 0 나누기 입력 포함.
        let ast = ExpressionAst::new(
            id(1),
            vec![
                ExprNode::binary(id(1), BinaryOp::Divide, id(2), id(3)),
                ExprNode::binary(id(2), BinaryOp::Add, id(4), id(5)),
                ExprNode::binary(id(3), BinaryOp::Subtract, id(6), id(7)),
                ExprNode::binary(id(4), BinaryOp::Multiply, id(6), id(8)),
                ExprNode::literal(id(5), len(4.0)),
                ExprNode::reference(id(6), token(1)),
                ExprNode::literal(id(7), len(1.0)),
                ExprNode::literal(id(8), num(2.0)),
            ],
        )
        .expect("유효");
        let program = Program::compile(&ast, &types()).expect("컴파일");
        assert_eq!(
            program.result_type(),
            TypeSpec::required(ExpressionType::Number)
        );
        for input in [-3.0f32, 0.0, 1.0, 2.0, 5.5, 1.0e30] {
            let values = BTreeMap::from([(token(1), Some(len(input)))]);
            let by_bytecode = program.evaluate(&values);
            assert_eq!(
                by_bytecode,
                evaluate_ast(&ast, &types(), &values),
                "token1 = {input}"
            );
            if input == 1.0 {
                assert_eq!(
                    by_bytecode,
                    Err(EvalError::new(Some(id(1)), EvalErrorKind::DivisionByZero))
                );
            }
            if input == 2.0 {
                assert_eq!(by_bytecode, Ok(Some(num(8.0))));
            }
        }
    }
}

/// VIII.4.3 short-circuit fixture.
#[cfg(test)]
mod short_circuit_tests {
    use std::collections::BTreeMap;

    use stellar_core_types::ids::StableId;
    use stellar_core_types::reference::TokenRef;
    use stellar_core_types::typed_value::{EnumValue, TypedValue};
    use stellar_core_types::value::FiniteF32;

    use super::*;
    use crate::ast::{ExprNode, ExprNodeId, ExpressionAst, ReferenceTarget};
    use crate::ast_eval::evaluate_ast;
    use crate::ops::BinaryOp;
    use crate::types::{EnumTypeId, ExpressionType, TypeSpec};

    fn id(n: u8) -> ExprNodeId {
        ExprNodeId::new(StableId::from_bytes([n; 16]))
    }

    fn num(v: f32) -> TypedValue {
        TypedValue::Number(FiniteF32::new(v).expect("유한"))
    }

    fn boolean(v: bool) -> TypedValue {
        TypedValue::Boolean(v)
    }

    /// 값을 넣지 않는(평가하면 MissingReference) 참조.
    fn missing() -> ReferenceTarget {
        ReferenceTarget::Token(TokenRef::new(StableId::from_bytes([0xDD; 16])))
    }

    fn selector() -> ReferenceTarget {
        ReferenceTarget::Token(TokenRef::new(StableId::from_bytes([0xEE; 16])))
    }

    fn flag() -> ReferenceTarget {
        ReferenceTarget::Token(TokenRef::new(StableId::from_bytes([0xFF; 16])))
    }

    fn types() -> BTreeMap<ReferenceTarget, TypeSpec> {
        BTreeMap::from([
            (missing(), TypeSpec::required(ExpressionType::Number)),
            (
                selector(),
                TypeSpec::required(ExpressionType::Enum(EnumTypeId::new(3))),
            ),
            (flag(), TypeSpec::required(ExpressionType::Boolean)),
        ])
    }

    /// node 90 = `1 / 0`(평가하면 DivisionByZero), node 91 = 누락 참조, 92·93 = 리터럴 1·0.
    fn poison() -> Vec<ExprNode> {
        vec![
            ExprNode::binary(id(90), BinaryOp::Divide, id(92), id(93)),
            ExprNode::reference(id(91), missing()),
            ExprNode::literal(id(92), num(1.0)),
            ExprNode::literal(id(93), num(0.0)),
        ]
    }

    fn run(
        root: ExprNodeId,
        mut nodes: Vec<ExprNode>,
        values: &BTreeMap<ReferenceTarget, Option<TypedValue>>,
    ) -> (Result<Option<TypedValue>, EvalError>, EvalTrace, usize) {
        // 참조된 poison node만 고정점까지 추가합니다(도달 불가 node 금지).
        loop {
            let mut added = false;
            for extra in poison() {
                let present = nodes.iter().any(|n| n.id() == extra.id());
                let referenced = nodes
                    .iter()
                    .any(|n| n.children().iter().any(|c| c.target == extra.id()));
                if !present && referenced {
                    nodes.push(extra);
                    added = true;
                }
            }
            if !added {
                break;
            }
        }
        let ast = ExpressionAst::new(root, nodes).expect("유효");
        let program = Program::compile(&ast, &types()).expect("컴파일");
        let traced = program.evaluate_traced(values);
        let by_ast = evaluate_ast(&ast, &types(), values);
        let result = traced.clone().map(|(value, _)| value);
        assert_eq!(result, by_ast, "VM과 AST evaluator 결과 동일");
        let trace = traced.map(|(_, trace)| trace).unwrap_or_default();
        (result, trace, program.instruction_count())
    }

    fn flag_values(value: bool) -> BTreeMap<ReferenceTarget, Option<TypedValue>> {
        BTreeMap::from([
            (flag(), Some(boolean(value))),
            (selector(), Some(TypedValue::Enum(EnumValue::new(3, 1)))),
        ])
    }

    #[test]
    fn if_skips_unselected_branch_errors() {
        // if flag then 7 else (1/0)
        let nodes = || {
            vec![
                ExprNode::if_else(id(1), id(2), id(3), id(90)),
                ExprNode::reference(id(2), flag()),
                ExprNode::literal(id(3), num(7.0)),
            ]
        };
        let (result, trace, total) = run(id(1), nodes(), &flag_values(true));
        assert_eq!(result, Ok(Some(num(7.0))));
        assert!(
            (trace.instructions as usize) < total,
            "else branch 명령 미실행"
        );
        // 선택된 branch의 오류는 그대로 발생(node = 1/0 node).
        let (result, _, _) = run(id(1), nodes(), &flag_values(false));
        assert_eq!(
            result,
            Err(EvalError::new(Some(id(90)), EvalErrorKind::DivisionByZero))
        );
        // if flag then 누락 참조 else 8 → false면 오류 없음.
        let nodes = vec![
            ExprNode::if_else(id(1), id(2), id(91), id(3)),
            ExprNode::reference(id(2), flag()),
            ExprNode::literal(id(3), num(8.0)),
        ];
        let (result, _, _) = run(id(1), nodes, &flag_values(false));
        assert_eq!(result, Ok(Some(num(8.0))));
    }

    #[test]
    fn match_skips_unselected_arm_and_default_errors() {
        // match selector { E0 => 1/0, E1 => 5, E2 => 누락 참조 } else 1/0
        let nodes = || {
            vec![
                ExprNode::match_on(
                    id(1),
                    id(2),
                    vec![
                        (TypedValue::Enum(EnumValue::new(3, 0)), id(90)),
                        (TypedValue::Enum(EnumValue::new(3, 1)), id(4)),
                        (TypedValue::Enum(EnumValue::new(3, 2)), id(91)),
                    ],
                    id(90),
                ),
                ExprNode::reference(id(2), selector()),
                ExprNode::literal(id(4), num(5.0)),
            ]
        };
        let with_variant = |variant: u32| {
            BTreeMap::from([(
                selector(),
                Some(TypedValue::Enum(EnumValue::new(3, variant))),
            )])
        };
        let (result, _, _) = run(id(1), nodes(), &with_variant(1));
        assert_eq!(result, Ok(Some(num(5.0))), "arm 1만 평가");
        let (result, _, _) = run(id(1), nodes(), &with_variant(2));
        assert_eq!(
            result,
            Err(EvalError::new(
                Some(id(91)),
                EvalErrorKind::MissingReference(missing())
            ))
        );
        let (result, _, _) = run(id(1), nodes(), &with_variant(9));
        assert_eq!(
            result,
            Err(EvalError::new(Some(id(90)), EvalErrorKind::DivisionByZero)),
            "일치 arm 없음 → 기본 branch 평가"
        );
        // 기본 branch만 안전하고 모든 arm이 오류여도 기본 branch 선택 시 오류 없음.
        let nodes = vec![
            ExprNode::match_on(
                id(1),
                id(2),
                vec![
                    (TypedValue::Enum(EnumValue::new(3, 0)), id(90)),
                    (TypedValue::Enum(EnumValue::new(3, 1)), id(91)),
                ],
                id(4),
            ),
            ExprNode::reference(id(2), selector()),
            ExprNode::literal(id(4), num(6.0)),
        ];
        let (result, _, _) = run(id(1), nodes, &with_variant(7));
        assert_eq!(result, Ok(Some(num(6.0))));
    }

    #[test]
    fn and_or_short_circuit() {
        // flag and (1/0 == 1), flag or (누락 참조 == 1)
        let and_nodes = || {
            vec![
                ExprNode::binary(id(1), BinaryOp::And, id(2), id(3)),
                ExprNode::reference(id(2), flag()),
                ExprNode::binary(id(3), BinaryOp::Equal, id(90), id(92)),
            ]
        };
        let (result, _, _) = run(id(1), and_nodes(), &flag_values(false));
        assert_eq!(
            result,
            Ok(Some(boolean(false))),
            "false and … → 오른쪽 미평가"
        );
        let (result, _, _) = run(id(1), and_nodes(), &flag_values(true));
        assert_eq!(
            result,
            Err(EvalError::new(Some(id(90)), EvalErrorKind::DivisionByZero))
        );
        let or_nodes = || {
            vec![
                ExprNode::binary(id(1), BinaryOp::Or, id(2), id(3)),
                ExprNode::reference(id(2), flag()),
                ExprNode::binary(id(3), BinaryOp::Equal, id(91), id(4)),
                ExprNode::literal(id(4), num(1.0)),
            ]
        };
        let (result, _, _) = run(id(1), or_nodes(), &flag_values(true));
        assert_eq!(result, Ok(Some(boolean(true))), "true or … → 오른쪽 미평가");
        let (result, _, _) = run(id(1), or_nodes(), &flag_values(false));
        assert_eq!(
            result,
            Err(EvalError::new(
                Some(id(91)),
                EvalErrorKind::MissingReference(missing())
            ))
        );
        // 진리표(오른쪽이 평가되는 경우 포함): VM = AST.
        for a in [false, true] {
            for b in [false, true] {
                for op in [BinaryOp::And, BinaryOp::Or] {
                    let nodes = vec![
                        ExprNode::binary(id(1), op, id(2), id(3)),
                        ExprNode::reference(id(2), flag()),
                        ExprNode::literal(id(3), boolean(b)),
                    ];
                    let (result, _, _) = run(id(1), nodes, &flag_values(a));
                    let expected = if op == BinaryOp::And { a && b } else { a || b };
                    assert_eq!(result, Ok(Some(boolean(expected))));
                }
            }
        }
    }

    #[test]
    fn shared_block_in_unselected_branch_is_not_run() {
        // S = 1/0 를 두 곳(else, 다른 if의 then)에서 공유 → 별도 block. flag=true면 실행 안 됨.
        let nodes = vec![
            ExprNode::if_else(id(1), id(2), id(3), id(90)),
            ExprNode::reference(id(2), flag()),
            ExprNode::if_else(id(3), id(2), id(4), id(90)),
            ExprNode::literal(id(4), num(3.0)),
        ];
        let mut full = nodes.clone();
        full.extend(poison().into_iter().filter(|n| n.id() != id(91)));
        let ast = ExpressionAst::new(id(1), full).expect("유효");
        let program = Program::compile(&ast, &types()).expect("컴파일");
        assert_eq!(program.blocks().len(), 2, "공유 연산 node는 별도 block");
        let (result, trace, _) = run(id(1), nodes, &flag_values(true));
        assert_eq!(result, Ok(Some(num(3.0))));
        assert_eq!(trace.block_runs, 1, "공유 1/0 block 미실행");
        let (result, trace, _) = run(id(1), ast.nodes().to_vec(), &flag_values(false));
        assert_eq!(
            result,
            Err(EvalError::new(Some(id(90)), EvalErrorKind::DivisionByZero))
        );
        assert_eq!(trace, EvalTrace::default(), "오류 시 trace 없음");
    }
}

/// VIII.4.5 instruction budget·call depth·stack 한도 fixture.
#[cfg(test)]
mod limit_tests {
    use std::collections::BTreeMap;

    use stellar_core_types::ids::StableId;
    use stellar_core_types::typed_value::TypedValue;
    use stellar_core_types::value::FiniteF32;

    use super::*;
    use crate::ast::{ExprNode, ExprNodeId, ExpressionAst, ReferenceTarget};
    use crate::functions::FunctionId;
    use crate::ops::{BinaryOp, UnaryOp};
    use crate::types::TypeSpec;

    fn id(n: u64) -> ExprNodeId {
        ExprNodeId::new(StableId::new_v7(n, 0, n))
    }

    fn one() -> TypedValue {
        TypedValue::Number(FiniteF32::ONE)
    }

    fn minus_one() -> TypedValue {
        TypedValue::Number(FiniteF32::new(-1.0).expect("유한"))
    }

    fn compile(ast: &ExpressionAst) -> Program {
        let types: BTreeMap<ReferenceTarget, TypeSpec> = BTreeMap::new();
        Program::compile(ast, &types).expect("컴파일")
    }

    /// `-(-(…(-1)))` 단항 chain(공유 없음, 깊이 `depth`).
    fn negate_chain(depth: u64) -> ExpressionAst {
        let mut nodes: Vec<ExprNode> = (1..depth)
            .map(|n| ExprNode::unary(id(n), UnaryOp::Negate, id(n + 1)))
            .collect();
        nodes.push(ExprNode::literal(id(depth), one()));
        ExpressionAst::new(id(1), nodes).expect("유효")
    }

    #[test]
    fn huge_ast_stops_at_instruction_budget() {
        // 30만 node AST는 명령 30만+1개 → budget 10만에서 typed 오류로 종료.
        let ast = negate_chain(300_000);
        let program = compile(&ast);
        assert_eq!(program.instruction_count(), 300_001);
        let limits = EvalLimits {
            instruction_budget: 100_000,
            ..EvalLimits::DEFAULT
        };
        let error = program
            .evaluate_with_limits(&BTreeMap::new(), limits)
            .expect_err("budget 초과");
        assert_eq!(
            error.kind,
            EvalErrorKind::InstructionBudgetExceeded { budget: 100_000 }
        );
        assert!(error.node.is_some(), "중단 지점 node 기록");
        // 같은 program도 budget이 충분하면 정상 종료(재귀 없이 깊이 30만 처리).
        let (value, trace) = program
            .evaluate_with_limits(
                &BTreeMap::new(),
                EvalLimits {
                    instruction_budget: 300_001,
                    ..EvalLimits::DEFAULT
                },
            )
            .expect("budget 정확히 충분");
        assert_eq!(value, Some(minus_one()), "홀수(299,999) 번 부호 반전");
        assert_eq!(trace.instructions, 300_001);
        // budget이 1 모자라면 실패(경계 정확).
        assert!(matches!(
            program.evaluate_with_limits(
                &BTreeMap::new(),
                EvalLimits {
                    instruction_budget: 300_000,
                    ..EvalLimits::DEFAULT
                }
            ),
            Err(EvalError {
                kind: EvalErrorKind::InstructionBudgetExceeded { budget: 300_000 },
                ..
            })
        ));
    }

    #[test]
    fn default_budget_stops_abnormally_large_program() {
        // 기본 budget(100만)보다 긴 program은 기본 평가에서도 제한 오류.
        let ast = negate_chain(1_000_001);
        let program = compile(&ast);
        assert!(matches!(
            program.evaluate(&BTreeMap::new()),
            Err(EvalError {
                kind: EvalErrorKind::InstructionBudgetExceeded { budget: 1_000_000 },
                ..
            })
        ));
    }

    #[test]
    fn deeply_nested_shared_blocks_stop_at_call_depth() {
        // n_i = n_(i+1) + n_(i+1) (모든 연산 node가 공유 → 별도 block, 중첩 깊이 = 단계 수).
        let shared_chain = |levels: u64| {
            let mut nodes: Vec<ExprNode> = (1..=levels)
                .map(|n| ExprNode::binary(id(n), BinaryOp::Add, id(n + 1), id(n + 1)))
                .collect();
            nodes.push(ExprNode::literal(id(levels + 1), one()));
            ExpressionAst::new(id(1), nodes).expect("유효")
        };
        let ast = shared_chain(300);
        let program = compile(&ast);
        assert_eq!(program.blocks().len(), 300, "root + 공유 연산 299개");
        let error = program.evaluate(&BTreeMap::new()).expect_err("깊이 초과");
        assert_eq!(error.kind, EvalErrorKind::CallDepthExceeded { limit: 256 });
        // 한도 안의 중첩은 정상(2^99 규모 경로를 캐시로 선형 실행).
        let ast = shared_chain(100);
        let (value, trace) = compile(&ast)
            .evaluate_traced(&BTreeMap::new())
            .expect("깊이 100 허용");
        assert_eq!(
            value,
            Some(TypedValue::Number(
                FiniteF32::new(2.0f32.powi(100)).expect("유한")
            ))
        );
        assert_eq!(trace.block_runs, 100, "공유 block은 각 1회");
        // 한도를 낮추면 같은 program도 거부.
        assert!(matches!(
            compile(&ast).evaluate_with_limits(
                &BTreeMap::new(),
                EvalLimits {
                    max_call_depth: 50,
                    ..EvalLimits::DEFAULT
                }
            ),
            Err(EvalError {
                kind: EvalErrorKind::CallDepthExceeded { limit: 50 },
                ..
            })
        ));
    }

    #[test]
    fn right_nested_expression_stops_at_stack_limit() {
        // 1 + (1 + (1 + …)) — 왼쪽 값이 쌓여 operand stack 깊이 = node 수.
        let depth = 70_000u64;
        let mut nodes: Vec<ExprNode> = (1..=depth)
            .map(|n| ExprNode::binary(id(n), BinaryOp::Add, id(1_000_000), id(n + 1)))
            .collect();
        nodes.push(ExprNode::literal(id(depth + 1), one()));
        nodes.push(ExprNode::literal(id(1_000_000), one()));
        let ast = ExpressionAst::new(id(1), nodes).expect("유효");
        let program = compile(&ast);
        assert!(matches!(
            program.evaluate(&BTreeMap::new()),
            Err(EvalError {
                kind: EvalErrorKind::StackLimitExceeded { limit: 65_536 },
                ..
            })
        ));
        let (value, _) = program
            .evaluate_with_limits(
                &BTreeMap::new(),
                EvalLimits {
                    max_stack_depth: 80_000,
                    ..EvalLimits::DEFAULT
                },
            )
            .expect("한도 상향 시 정상");
        assert_eq!(
            value,
            Some(TypedValue::Number(FiniteF32::new(70_001.0).expect("유한")))
        );
    }

    #[test]
    fn ordinary_expressions_fit_default_limits() {
        assert_eq!(EvalLimits::default(), EvalLimits::DEFAULT);
        let ast = negate_chain(10_000);
        let (value, trace) = compile(&ast)
            .evaluate_traced(&BTreeMap::new())
            .expect("정상");
        assert_eq!(value, Some(minus_one()), "홀수(9,999) 번 부호 반전");
        assert!(trace.instructions < EvalLimits::DEFAULT.instruction_budget);
    }

    /// `lhs(1) + (lhs(2) + (… + (lhs(depth) + 1)))` — 왼쪽 literal이 level마다 다른 node라서
    /// 한도를 넘는 push의 원인 node를 구분할 수 있습니다. 최대 stack 깊이는 `depth + 1`입니다.
    fn distinct_right_nested(depth: u64) -> ExpressionAst {
        let lhs = |n: u64| id(2_000_000 + n);
        let mut nodes: Vec<ExprNode> = (1..=depth)
            .map(|n| ExprNode::binary(id(n), BinaryOp::Add, lhs(n), id(n + 1)))
            .collect();
        nodes.extend((1..=depth).map(|n| ExprNode::literal(lhs(n), one())));
        nodes.push(ExprNode::literal(id(depth + 1), one()));
        ExpressionAst::new(id(1), nodes).expect("유효")
    }

    #[test]
    fn stack_never_exceeds_limit_and_stops_at_overflowing_push() {
        // 회귀(VIII.4.5 리뷰): 이전에는 실행 전 `len > limit`만 검사해 stack이 limit + 1개까지 찼고,
        // 오류도 넘친 push 다음 명령에서 났습니다.
        let depth = 1_000u64;
        let program = compile(&distinct_right_nested(depth));
        let with_limit = |limit: usize| {
            program.evaluate_with_limits(
                &BTreeMap::new(),
                EvalLimits {
                    max_stack_depth: limit,
                    ..EvalLimits::DEFAULT
                },
            )
        };
        // 501번째 push(= lhs(501))가 한도 500을 넘으므로 그 node에서 멈춥니다.
        let error = with_limit(500).expect_err("한도 초과");
        assert_eq!(error.kind, EvalErrorKind::StackLimitExceeded { limit: 500 });
        assert_eq!(error.node, Some(id(2_000_501)));
        // 경계: 최대 깊이(depth + 1)와 같은 한도는 통과, 1 작으면 innermost literal push에서 실패.
        let (value, trace) = with_limit(1_001).expect("경계 한도 통과");
        assert_eq!(
            value,
            Some(TypedValue::Number(FiniteF32::new(1_001.0).expect("유한")))
        );
        assert_eq!(trace.peak_stack_depth, 1_001);
        let error = with_limit(1_000).expect_err("1 부족");
        assert_eq!(error.node, Some(id(depth + 1)));
        // 한도 0이면 첫 push부터 거부.
        assert_eq!(
            with_limit(0).expect_err("빈 한도").node,
            Some(id(2_000_001))
        );
    }

    /// `e(1) and (e(2) and … e(k))`, `e(i) = c(i) == c(i)`, `c(i) = concat(H, "i")` — `c(i)`는 공유 node라서
    /// 결과가 캐시에 남습니다. `H`는 `half` bytes 공유 literal입니다.
    fn cached_concat_chain(k: u64, half: usize) -> ExpressionAst {
        let shared = id(9_000_000);
        let e = |n: u64| id(1_000 + n);
        let c = |n: u64| id(2_000 + n);
        let suffix = |n: u64| id(3_000 + n);
        let mut nodes = vec![ExprNode::literal(
            shared,
            TypedValue::String("h".repeat(half)),
        )];
        for n in 1..=k {
            if n + 1 < k {
                nodes.push(ExprNode::binary(id(n), BinaryOp::And, e(n), id(n + 1)));
            } else if n + 1 == k {
                nodes.push(ExprNode::binary(id(n), BinaryOp::And, e(n), e(k)));
            }
            nodes.push(ExprNode::binary(e(n), BinaryOp::Equal, c(n), c(n)));
            nodes.push(ExprNode::call(
                c(n),
                FunctionId::Concat,
                &[shared, suffix(n)],
            ));
            nodes.push(ExprNode::literal(
                suffix(n),
                TypedValue::String(n.to_string()),
            ));
        }
        ExpressionAst::new(id(1), nodes).expect("유효(k ≥ 2)")
    }

    #[test]
    fn cached_shared_results_count_toward_value_limit() {
        let half = 512 * 1024;
        let limits = EvalLimits {
            max_value_bytes: 4 << 20,
            ..EvalLimits::DEFAULT
        };
        // 5개: 캐시 5 × 512 KiB + 비교 중 stack 2개 ≈ 3.5 MiB → 통과.
        let program = compile(&cached_concat_chain(5, half));
        let (value, trace) = program
            .evaluate_with_limits(&BTreeMap::new(), limits)
            .expect("한도 안");
        assert_eq!(value, Some(TypedValue::Boolean(true)));
        assert!(trace.peak_value_bytes <= limits.max_value_bytes);
        assert!(
            trace.peak_value_bytes >= 5 * half,
            "캐시 보유분도 계량: {}",
            trace.peak_value_bytes
        );
        // 10개: 캐시가 누적되어 한도 초과(캐시를 계량하지 않으면 stack만으로는 1 MiB 남짓).
        let program = compile(&cached_concat_chain(10, half));
        let error = program
            .evaluate_with_limits(&BTreeMap::new(), limits)
            .expect_err("캐시 누적 초과");
        assert_eq!(
            error.kind,
            EvalErrorKind::ValueMemoryExceeded { limit: 4 << 20 }
        );
        // 기본 한도(16 MiB)에서는 같은 program이 정상 종료합니다.
        let (_, trace) = program
            .evaluate_traced(&BTreeMap::new())
            .expect("기본 한도");
        assert!(trace.peak_value_bytes <= EvalLimits::DEFAULT.max_value_bytes);
    }

    #[test]
    fn value_heap_bytes_measures_heap_payloads() {
        use crate::eval::value_heap_bytes;
        assert_eq!(value_heap_bytes(&None), 0);
        assert_eq!(value_heap_bytes(&Some(one())), 0);
        assert_eq!(
            value_heap_bytes(&Some(TypedValue::String("가나".to_string()))),
            6
        );
        assert_eq!(
            EvalLimits::DEFAULT.max_value_bytes,
            16 * crate::eval::MAX_STRING_BYTES
        );
    }
}
