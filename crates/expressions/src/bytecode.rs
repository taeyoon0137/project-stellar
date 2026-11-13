//! 표현식 bytecode 내부 포맷과 compiler (VIII.4.1).
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
//! - **constant pool**: 리터럴 값(null 포함). 같은 값은 한 번만 들어갑니다(첫 등장 순서).
//! - **reference table**: 참조 대상과 type 검사 때의 타입. 평가 시 입력 값을 이 타입으로 검증합니다.
//! - **pattern table**: `match` arm pattern 값 목록.
//! - 명령마다 원인 AST node를 기록해 평가 오류가 node StableId를 가리키게 합니다.
//!
//! 컴파일은 명시적 stack으로 수행해 재귀하지 않습니다.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::fmt::Write as _;

use stellar_core_types::typed_value::TypedValue;

use crate::ast::{ChildSlot, ExprNodeId, ExpressionAst, Literal, NodePayload, ReferenceTarget};
use crate::ops::NodeKind;
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
    /// `[condition, then, else]`를 pop하고 조건에 따라 하나를 push(`if`).
    Select,
    /// `[scrutinee, arm0, …, armN-1, default]`를 pop하고 pattern table `index`로 고른 값을 push(`match`).
    MatchSelect(u32),
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
    /// 이 단계에서 아직 지원하지 않는 node 종류입니다.
    UnsupportedNode {
        /// node.
        node: ExprNodeId,
        /// 종류.
        kind: NodeKind,
    },
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
            CompileError::UnsupportedNode { node, kind } => {
                write!(f, "아직 컴파일할 수 없는 node 종류 {kind}: {node}")
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
    patterns: Vec<Vec<TypedValue>>,
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
            patterns: Vec::new(),
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
            patterns: compiler.patterns,
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

    /// pattern table.
    pub fn patterns(&self) -> &[Vec<TypedValue>] {
        &self.patterns
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

/// 컴파일 작업.
enum Task {
    /// node 값을 stack에 남기는 코드 생성.
    Visit(ExprNodeId),
    /// 명령 하나 기록.
    Emit(Instr, ExprNodeId),
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
    patterns: Vec<Vec<TypedValue>>,
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
        let mut code = Vec::new();
        let mut sources = Vec::new();
        let mut stack = vec![Task::Visit(block_root)];
        let mut first = true;
        while let Some(task) = stack.pop() {
            let id = match task {
                Task::Emit(instr, source) => {
                    code.push(instr);
                    sources.push(source);
                    continue;
                }
                Task::Visit(id) => id,
            };
            let is_root = std::mem::replace(&mut first, false);
            if !is_root && self.is_shared_operation(id) {
                let index = self.block_for(id)?;
                code.push(Instr::Eval(index));
                sources.push(id);
                continue;
            }
            let node = self
                .ast
                .node(id)
                .ok_or(CompileError::TypeErrors(Vec::new()))?;
            let child = |slot| node.child(slot).ok_or(CompileError::TypeErrors(Vec::new()));
            match node.payload() {
                NodePayload::Literal(literal) => {
                    let value = match literal {
                        Literal::Value(value) => Some(value.clone()),
                        Literal::Null(_) => None,
                    };
                    let index = self.constant(value)?;
                    code.push(Instr::Const(index));
                    sources.push(id);
                }
                NodePayload::Reference(target) => {
                    let index = self.reference(target, id)?;
                    code.push(Instr::LoadRef(index));
                    sources.push(id);
                }
                NodePayload::If => {
                    stack.push(Task::Emit(Instr::Select, id));
                    stack.push(Task::Visit(child(ChildSlot::Else)?));
                    stack.push(Task::Visit(child(ChildSlot::Then)?));
                    stack.push(Task::Visit(child(ChildSlot::Condition)?));
                }
                NodePayload::Match(patterns) => {
                    let set = index_u32(self.patterns.len())?;
                    self.patterns.push(patterns.clone());
                    stack.push(Task::Emit(Instr::MatchSelect(set), id));
                    stack.push(Task::Visit(child(ChildSlot::Default)?));
                    for index in (0..patterns.len()).rev() {
                        let slot = ChildSlot::Arm(
                            u16::try_from(index).map_err(|_| CompileError::TooLarge)?,
                        );
                        stack.push(Task::Visit(child(slot)?));
                    }
                    stack.push(Task::Visit(child(ChildSlot::Scrutinee)?));
                }
                NodePayload::Unary(_) | NodePayload::Binary(_) | NodePayload::Call(_) => {
                    return Err(CompileError::UnsupportedNode {
                        node: id,
                        kind: node.kind(),
                    })
                }
            }
        }
        code.push(Instr::Return);
        sources.push(block_root);
        Ok(CodeBlock {
            node: block_root,
            code,
            sources,
        })
    }
}
