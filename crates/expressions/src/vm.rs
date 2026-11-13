//! bytecode evaluator (VIII.4.1).
//!
//! [`Program`]을 operand stack과 명시적 frame stack으로 실행합니다. Rust 재귀를 쓰지 않으며, 공유 block은
//! 처음 [`Instr::Eval`]에서 한 번 평가한 뒤 결과를 캐시합니다. 값 의미는 [`crate::eval`]의 공용 함수를 써서
//! AST 기준 evaluator([`crate::ast_eval`])와 같은 결과를 냅니다.

use stellar_core_types::typed_value::TypedValue;

use crate::bytecode::{Instr, Program};
use crate::eval::{
    load_reference, match_arm, select_if, EvalError, EvalErrorKind, ReferenceValues,
};

/// 평가 실행 기록.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EvalTrace {
    /// 실행한 명령 수.
    pub instructions: u64,
    /// 실행한 block 수(공유 block 캐시 적중은 제외).
    pub block_runs: u64,
}

struct Frame {
    block: usize,
    pc: usize,
}

impl Program {
    /// 참조 값으로 program을 평가합니다.
    pub fn evaluate(&self, values: &dyn ReferenceValues) -> Result<Option<TypedValue>, EvalError> {
        self.evaluate_traced(values).map(|(value, _)| value)
    }

    /// 평가하고 실행 기록을 함께 돌려줍니다.
    pub fn evaluate_traced(
        &self,
        values: &dyn ReferenceValues,
    ) -> Result<(Option<TypedValue>, EvalTrace), EvalError> {
        let mut trace = EvalTrace::default();
        let mut stack: Vec<Option<TypedValue>> = Vec::new();
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
            frame.pc += 1;
            trace.instructions += 1;
            let fail = |kind| EvalError::new(node, kind);
            match instr {
                Instr::Const(index) => {
                    let value = self
                        .constants()
                        .get(index as usize)
                        .ok_or_else(|| malformed(node))?;
                    stack.push(value.clone());
                }
                Instr::LoadRef(index) => {
                    let (target, expected) = self
                        .references()
                        .get(index as usize)
                        .ok_or_else(|| malformed(node))?;
                    stack.push(load_reference(target, *expected, values).map_err(fail)?);
                }
                Instr::Eval(index) => {
                    let slot = cache.get(index as usize).ok_or_else(|| malformed(node))?;
                    match slot {
                        Some(value) => stack.push(value.clone()),
                        None => {
                            trace.block_runs += 1;
                            frames.push(Frame {
                                block: index as usize,
                                pc: 0,
                            });
                        }
                    }
                }
                Instr::Select => {
                    let else_value = stack.pop().ok_or_else(|| malformed(node))?;
                    let then_value = stack.pop().ok_or_else(|| malformed(node))?;
                    let condition = stack.pop().ok_or_else(|| malformed(node))?;
                    stack.push(select_if(&condition, then_value, else_value).map_err(fail)?);
                }
                Instr::MatchSelect(set) => {
                    let patterns = self
                        .patterns()
                        .get(set as usize)
                        .ok_or_else(|| malformed(node))?;
                    let start = stack
                        .len()
                        .checked_sub(patterns.len() + 2)
                        .ok_or_else(|| malformed(node))?;
                    let mut operands = stack.split_off(start);
                    let default = operands.pop().ok_or_else(|| malformed(node))?;
                    let scrutinee = operands.remove(0);
                    let chosen = match match_arm(&scrutinee, patterns) {
                        Some(arm) => operands.swap_remove(arm),
                        None => default,
                    };
                    stack.push(chosen);
                }
                Instr::Return => {
                    let value = stack.pop().ok_or_else(|| malformed(node))?;
                    let finished = frames.pop().ok_or_else(|| malformed(node))?;
                    if frames.is_empty() {
                        return if stack.is_empty() {
                            Ok((value, trace))
                        } else {
                            Err(malformed(node))
                        };
                    }
                    if let Some(slot) = cache.get_mut(finished.block) {
                        *slot = Some(value.clone());
                    }
                    stack.push(value);
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
    use crate::ops::{BinaryOp, NodeKind};
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
        assert_eq!(trace.instructions as usize, program.instruction_count());
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
        let binary = ExpressionAst::new(
            id(1),
            vec![
                ExprNode::binary(id(1), BinaryOp::Add, id(2), id(2)),
                ExprNode::literal(id(2), len(1.0)),
            ],
        )
        .expect("유효");
        assert_eq!(
            Program::compile(&binary, &types()),
            Err(CompileError::UnsupportedNode {
                node: id(1),
                kind: NodeKind::Binary
            })
        );
    }
}
