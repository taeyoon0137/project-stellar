//! 표현식 bytecode 내부 포맷과 compiler (VIII.4.1~VIII.4.4).
//!
//! 검증·type 검사를 통과한 AST를 stack bytecode [`Program`]으로 컴파일합니다. bytecode는 **저장하지 않는
//! 실행 캐시**입니다. 직렬화 형식·안정 번호가 없고, 원본은 항상 canonical AST(VIII.2.5)이며 필요할 때마다
//! 다시 컴파일합니다(아키텍처 §14.1 "검증된 AST 또는 bytecode만 평가").
//!
//! 형식:
//!
//! - **block**: 값 하나를 계산해 반환하는 명령 열. block 0은 root입니다. 여러 부모가 공유하는 연산 node
//!   (DAG)는 별도 block으로 한 번만 컴파일하고, 부모는 [`Instr::Eval`]로 부릅니다. evaluator는 block 결과를
//!   캐시해 공유 node를 한 번만 평가합니다. Literal·Reference 공유 node는 load 명령이라 그대로 복제합니다.
//! - **constant pool**: 리터럴 값(null 포함)과 `match` pattern 값. 같은 값은 한 번만 들어갑니다(첫 등장 순서).
//! - **reference table**: 참조 대상과 type 검사 때의 타입. 평가 시 입력 값을 이 타입으로 검증합니다.
//! - 명령마다 원인 AST node를 기록해 평가 오류가 node StableId를 가리키게 합니다.
//!
//! **short-circuit(VIII.4.3)**: `if`·`match`·`and`·`or`는 block 안 jump로 컴파일해 선택된 branch만 실행합니다.
//! 선택되지 않은 branch의 연산·참조·공유 block은 실행되지 않으므로 그 안의 오류(0 나누기, 누락 참조 등)도
//! 발생하지 않습니다.
//!
//! ```text
//! if c then a else b :  c; JumpIfFalse(L1); a; Jump(L2); L1: b; L2:
//! match s {p0 => a0, …} else d :
//!                       s; MatchCase(p0, M1); a0; Jump(E); M1: MatchCase(p1, M2); a1; Jump(E); … ; Mn: Pop; d; E:
//! x and y            :  x; JumpIfFalse(F); y; Jump(E); F: Const(false); E:
//! x or y             :  x; JumpIfFalse(R); Const(true); Jump(E); R: y; E:
//! ```
//!
//! 컴파일은 명시적 stack으로 수행해 재귀하지 않습니다.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::fmt::Write as _;

use stellar_core_types::typed_value::TypedValue;

use crate::ast::{ChildSlot, ExprNodeId, ExpressionAst, Literal, NodePayload, ReferenceTarget};
use crate::functions::FunctionId;
use crate::ops::{BinaryOp, UnaryOp};
use crate::typecheck::{type_check, ReferenceTypes, TypeError};
use crate::types::TypeSpec;

/// stack 명령.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Instr {
    /// constant pool `index`의 값을 push.
    Const(u32),
    /// reference table `index`의 참조 값을 검증해 push.
    LoadRef(u32),
    /// 공유 block `index`의 값을 push(첫 호출에서 평가, 이후 캐시).
    Eval(u32),
    /// Boolean 조건을 pop하고 `false`면 같은 block의 명령 `target`으로 이동(VIII.4.3).
    JumpIfFalse(u32),
    /// 같은 block의 명령 `target`으로 이동(VIII.4.3).
    Jump(u32),
    /// stack top(match 대상)이 constant `pattern`과 같으면 pop하고 다음 명령, 다르면 남겨 두고 `miss`로
    /// 이동(VIII.4.3).
    MatchCase {
        /// pattern 값의 constant pool index.
        pattern: u32,
        /// 불일치 시 이동할 명령 index.
        miss: u32,
    },
    /// stack top을 버림(일치 arm이 없을 때 match 대상 제거, VIII.4.3).
    Pop,
    /// operand 하나를 pop하고 단항 연산 결과를 push(VIII.4.2).
    Unary(UnaryOp),
    /// `[lhs, rhs]`를 pop하고 이항 연산 결과를 push(VIII.4.2).
    Binary(BinaryOp),
    /// 인자 `argc`개를 pop(signature 순서)하고 함수 결과를 push(VIII.4.4).
    Call {
        /// 등록 함수.
        function: FunctionId,
        /// 인자 수.
        argc: u16,
    },
    /// stack top을 block 결과로 반환.
    Return,
}

/// 실행 단위 block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeBlock {
    node: ExprNodeId,
    code: Vec<Instr>,
    sources: Vec<ExprNodeId>,
}

impl CodeBlock {
    /// block이 계산하는 AST node.
    pub const fn node(&self) -> ExprNodeId {
        self.node
    }

    /// 명령 열.
    pub fn code(&self) -> &[Instr] {
        &self.code
    }

    /// 명령별 원인 AST node(`code`와 같은 길이).
    pub fn sources(&self) -> &[ExprNodeId] {
        &self.sources
    }
}

/// 컴파일 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileError {
    /// type 오류가 있는 AST는 컴파일하지 않습니다.
    TypeErrors(Vec<TypeError>),
    /// table 크기가 `u32`를 넘습니다.
    TooLarge,
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompileError::TypeErrors(errors) => {
                write!(
                    f,
                    "type 오류 {}개가 있어 컴파일할 수 없습니다",
                    errors.len()
                )
            }
            CompileError::TooLarge => f.write_str("program table이 너무 큽니다"),
        }
    }
}

impl std::error::Error for CompileError {}

/// 컴파일된 표현식 program (VIII.4.1). 저장하지 않는 캐시입니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    blocks: Vec<CodeBlock>,
    constants: Vec<Option<TypedValue>>,
    references: Vec<(ReferenceTarget, TypeSpec)>,
    result_type: TypeSpec,
}

impl Program {
    /// AST를 type 검사한 뒤 bytecode로 컴파일합니다.
    pub fn compile(
        ast: &ExpressionAst,
        types: &dyn ReferenceTypes,
    ) -> Result<Program, CompileError> {
        let report = type_check(ast, types);
        if !report.is_ok() {
            return Err(CompileError::TypeErrors(report.errors().to_vec()));
        }
        let result_type = report
            .root_type()
            .ok_or(CompileError::TypeErrors(Vec::new()))?;
        let mut compiler = Compiler {
            ast,
            report_types: report.types(),
            shared: ast
                .parent_counts()
                .into_iter()
                .filter(|(_, count)| *count > 1)
                .map(|(id, _)| id)
                .collect(),
            block_index: BTreeMap::new(),
            pending: VecDeque::new(),
            constants: Vec::new(),
            constant_index: BTreeMap::new(),
            references: Vec::new(),
            reference_index: BTreeMap::new(),
        };
        compiler.block_for(ast.root())?;
        let mut blocks = Vec::new();
        while let Some(node) = compiler.pending.pop_front() {
            blocks.push(compiler.compile_block(node)?);
        }
        Ok(Program {
            blocks,
            constants: compiler.constants,
            references: compiler.references,
            result_type,
        })
    }

    /// block 목록(0 = root).
    pub fn blocks(&self) -> &[CodeBlock] {
        &self.blocks
    }

    /// constant pool.
    pub fn constants(&self) -> &[Option<TypedValue>] {
        &self.constants
    }

    /// reference table.
    pub fn references(&self) -> &[(ReferenceTarget, TypeSpec)] {
        &self.references
    }

    /// 결과 타입.
    pub const fn result_type(&self) -> TypeSpec {
        self.result_type
    }

    /// 전체 명령 수.
    pub fn instruction_count(&self) -> usize {
        self.blocks.iter().map(|block| block.code.len()).sum()
    }

    /// 사람이 읽는 명령 목록(디버깅·구조 검사용, 형식 계약 아님).
    pub fn disassemble(&self) -> String {
        let mut out = String::new();
        for (index, block) in self.blocks.iter().enumerate() {
            let _ = writeln!(out, "block {index}:");
            for instr in &block.code {
                let _ = writeln!(out, "  {instr:?}");
            }
        }
        out
    }
}

/// 이동 대상이 label인 미해결 명령.
#[derive(Clone, Copy)]
enum Pending {
    /// 이미 확정된 명령.
    Ready(Instr),
    /// `JumpIfFalse(label)`.
    JumpIfFalse(usize),
    /// `Jump(label)`.
    Jump(usize),
    /// `MatchCase { pattern, miss: label }`.
    MatchCase { pattern: u32, miss: usize },
}

/// 컴파일 작업.
enum Task {
    /// node 값을 stack에 남기는 코드 생성.
    Visit(ExprNodeId),
    /// 명령 하나 기록.
    Emit(Pending, ExprNodeId),
    /// label 위치를 현재 명령 위치로 확정.
    Label(usize),
}

struct Compiler<'a> {
    ast: &'a ExpressionAst,
    report_types: &'a BTreeMap<ExprNodeId, TypeSpec>,
    shared: std::collections::BTreeSet<ExprNodeId>,
    block_index: BTreeMap<ExprNodeId, u32>,
    pending: VecDeque<ExprNodeId>,
    constants: Vec<Option<TypedValue>>,
    constant_index: BTreeMap<Option<TypedValue>, u32>,
    references: Vec<(ReferenceTarget, TypeSpec)>,
    reference_index: BTreeMap<ReferenceTarget, u32>,
}

fn index_u32(len: usize) -> Result<u32, CompileError> {
    u32::try_from(len).map_err(|_| CompileError::TooLarge)
}

impl Compiler<'_> {
    /// node의 block 번호(처음이면 배정하고 컴파일 대기열에 넣음).
    fn block_for(&mut self, node: ExprNodeId) -> Result<u32, CompileError> {
        if let Some(&index) = self.block_index.get(&node) {
            return Ok(index);
        }
        let index = index_u32(self.block_index.len())?;
        self.block_index.insert(node, index);
        self.pending.push_back(node);
        Ok(index)
    }

    fn constant(&mut self, value: Option<TypedValue>) -> Result<u32, CompileError> {
        if let Some(&index) = self.constant_index.get(&value) {
            return Ok(index);
        }
        let index = index_u32(self.constants.len())?;
        self.constant_index.insert(value.clone(), index);
        self.constants.push(value);
        Ok(index)
    }

    fn reference(
        &mut self,
        target: &ReferenceTarget,
        node: ExprNodeId,
    ) -> Result<u32, CompileError> {
        if let Some(&index) = self.reference_index.get(target) {
            return Ok(index);
        }
        let spec = self
            .report_types
            .get(&node)
            .copied()
            .ok_or(CompileError::TypeErrors(Vec::new()))?;
        let index = index_u32(self.references.len())?;
        self.reference_index.insert(target.clone(), index);
        self.references.push((target.clone(), spec));
        Ok(index)
    }

    /// 공유 연산 node(별도 block 대상)인지.
    fn is_shared_operation(&self, node: ExprNodeId) -> bool {
        self.shared.contains(&node)
            && self.ast.node(node).is_some_and(|n| {
                !matches!(
                    n.payload(),
                    NodePayload::Literal(_) | NodePayload::Reference(_)
                )
            })
    }

    fn compile_block(&mut self, block_root: ExprNodeId) -> Result<CodeBlock, CompileError> {
        let mut code: Vec<Pending> = Vec::new();
        let mut sources = Vec::new();
        let mut labels: Vec<Option<u32>> = Vec::new();
        fn new_label(labels: &mut Vec<Option<u32>>) -> usize {
            labels.push(None);
            labels.len() - 1
        }
        let mut stack = vec![Task::Visit(block_root)];
        let mut first = true;
        while let Some(task) = stack.pop() {
            let id = match task {
                Task::Emit(pending, source) => {
                    code.push(pending);
                    sources.push(source);
                    continue;
                }
                Task::Label(label) => {
                    let position = index_u32(code.len())?;
                    if let Some(slot) = labels.get_mut(label) {
                        *slot = Some(position);
                    }
                    continue;
                }
                Task::Visit(id) => id,
            };
            let is_root = std::mem::replace(&mut first, false);
            if !is_root && self.is_shared_operation(id) {
                let index = self.block_for(id)?;
                code.push(Pending::Ready(Instr::Eval(index)));
                sources.push(id);
                continue;
            }
            let node = self
                .ast
                .node(id)
                .ok_or(CompileError::TypeErrors(Vec::new()))?;
            let child = |slot| node.child(slot).ok_or(CompileError::TypeErrors(Vec::new()));
            let emit = |instr: Instr| Task::Emit(Pending::Ready(instr), id);
            match node.payload() {
                NodePayload::Literal(literal) => {
                    let value = match literal {
                        Literal::Value(value) => Some(value.clone()),
                        Literal::Null(_) => None,
                    };
                    let index = self.constant(value)?;
                    stack.push(emit(Instr::Const(index)));
                }
                NodePayload::Reference(target) => {
                    let index = self.reference(target, id)?;
                    stack.push(emit(Instr::LoadRef(index)));
                }
                NodePayload::If => {
                    let otherwise = new_label(&mut labels);
                    let end = new_label(&mut labels);
                    // LIFO: 마지막에 push한 작업이 먼저 실행됩니다.
                    stack.push(Task::Label(end));
                    stack.push(Task::Visit(child(ChildSlot::Else)?));
                    stack.push(Task::Label(otherwise));
                    stack.push(Task::Emit(Pending::Jump(end), id));
                    stack.push(Task::Visit(child(ChildSlot::Then)?));
                    stack.push(Task::Emit(Pending::JumpIfFalse(otherwise), id));
                    stack.push(Task::Visit(child(ChildSlot::Condition)?));
                }
                NodePayload::Match(patterns) => {
                    let end = new_label(&mut labels);
                    let mut tasks = vec![Task::Visit(child(ChildSlot::Scrutinee)?)];
                    for (index, pattern) in patterns.iter().enumerate() {
                        let miss = new_label(&mut labels);
                        let pattern = self.constant(Some(pattern.clone()))?;
                        let slot = ChildSlot::Arm(
                            u16::try_from(index).map_err(|_| CompileError::TooLarge)?,
                        );
                        tasks.push(Task::Emit(Pending::MatchCase { pattern, miss }, id));
                        tasks.push(Task::Visit(child(slot)?));
                        tasks.push(Task::Emit(Pending::Jump(end), id));
                        tasks.push(Task::Label(miss));
                    }
                    tasks.push(emit(Instr::Pop));
                    tasks.push(Task::Visit(child(ChildSlot::Default)?));
                    tasks.push(Task::Label(end));
                    stack.extend(tasks.into_iter().rev());
                }
                NodePayload::Binary(op @ (BinaryOp::And | BinaryOp::Or)) => {
                    let alternative = new_label(&mut labels);
                    let end = new_label(&mut labels);
                    let lhs = child(ChildSlot::Lhs)?;
                    let rhs = child(ChildSlot::Rhs)?;
                    let tasks = if *op == BinaryOp::And {
                        let false_index = self.constant(Some(TypedValue::Boolean(false)))?;
                        vec![
                            Task::Visit(lhs),
                            Task::Emit(Pending::JumpIfFalse(alternative), id),
                            Task::Visit(rhs),
                            Task::Emit(Pending::Jump(end), id),
                            Task::Label(alternative),
                            emit(Instr::Const(false_index)),
                            Task::Label(end),
                        ]
                    } else {
                        let true_index = self.constant(Some(TypedValue::Boolean(true)))?;
                        vec![
                            Task::Visit(lhs),
                            Task::Emit(Pending::JumpIfFalse(alternative), id),
                            emit(Instr::Const(true_index)),
                            Task::Emit(Pending::Jump(end), id),
                            Task::Label(alternative),
                            Task::Visit(rhs),
                            Task::Label(end),
                        ]
                    };
                    stack.extend(tasks.into_iter().rev());
                }
                NodePayload::Unary(op) => {
                    stack.push(emit(Instr::Unary(*op)));
                    stack.push(Task::Visit(child(ChildSlot::Operand)?));
                }
                NodePayload::Binary(op) => {
                    stack.push(emit(Instr::Binary(*op)));
                    stack.push(Task::Visit(child(ChildSlot::Rhs)?));
                    stack.push(Task::Visit(child(ChildSlot::Lhs)?));
                }
                NodePayload::Call(function) => {
                    let arity = function.signature().arity();
                    let argc = u16::try_from(arity).map_err(|_| CompileError::TooLarge)?;
                    stack.push(emit(Instr::Call {
                        function: *function,
                        argc,
                    }));
                    for index in (0..argc).rev() {
                        stack.push(Task::Visit(child(ChildSlot::Argument(index))?));
                    }
                }
            }
        }
        code.push(Pending::Ready(Instr::Return));
        sources.push(block_root);
        let resolve = |label: usize| -> Result<u32, CompileError> {
            labels
                .get(label)
                .copied()
                .flatten()
                .ok_or(CompileError::TooLarge)
        };
        let code = code
            .into_iter()
            .map(|pending| {
                Ok(match pending {
                    Pending::Ready(instr) => instr,
                    Pending::JumpIfFalse(label) => Instr::JumpIfFalse(resolve(label)?),
                    Pending::Jump(label) => Instr::Jump(resolve(label)?),
                    Pending::MatchCase { pattern, miss } => Instr::MatchCase {
                        pattern,
                        miss: resolve(miss)?,
                    },
                })
            })
            .collect::<Result<Vec<Instr>, CompileError>>()?;
        Ok(CodeBlock {
            node: block_root,
            code,
            sources,
        })
    }
}
