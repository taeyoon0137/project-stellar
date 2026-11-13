//! 타입이 확인된 표현식 슬롯 (IX.1.2·IX.5.1·IX.5.3 공용).
//!
//! Component Property default, 상태 변수 initial, transition 조건·action 값처럼 **선언된 타입**을
//! 갖는 표현식 위치를 [`CheckedExpression`]으로 표현합니다. 만들 때 한 번 검사합니다:
//!
//! 1. AST 구조는 [`ExpressionAst`]가 이미 보장합니다(ID 유일성·도달성·cycle 금지, VIII.2.1·VIII.2.3).
//! 2. type checker(VIII.2.4)가 호출자의 참조 타입 환경으로 오류 없이 통과해야 합니다.
//! 3. root 추론 타입이 선언 타입에 대입 가능해야 합니다([`assignable`]).
//!
//! 저장은 canonical AST bytes(VIII.2.5)이며, 디코딩은 같은 검사를 다시 거칩니다. 평가는 bytecode
//! program(VIII.4.1)을 [`EvalLimits`] 한도(VIII.4.5)로 실행합니다 — 재귀 없이 끝나며 비정상적으로 큰
//! AST는 제한 오류로 종료합니다.

use std::fmt;

use stellar_core_types::typed_value::TypedValue;
use stellar_expressions::ast::{ExpressionAst, ReferenceTarget};
use stellar_expressions::bytecode::{CompileError, Program};
use stellar_expressions::codec::{value_type_tag, CodecError};
use stellar_expressions::eval::{EvalError, ReferenceValues};
use stellar_expressions::typecheck::{ReferenceTypes, TypeError};
use stellar_expressions::types::{EnumTypeId, ExpressionType, TypeSpec};
use stellar_expressions::vm::EvalLimits;

use super::codec::{put_bytes, put_u32, put_u8, Malformed, Reader};

/// 타입 확인 표현식 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpressionCheckError {
    /// type checker가 오류를 보고했습니다(오류 node StableId·경로 포함, VIII.2.4).
    TypeErrors(Vec<TypeError>),
    /// root 추론 타입이 선언 타입에 대입되지 않습니다.
    TypeMismatch {
        /// 선언 타입.
        expected: TypeSpec,
        /// 추론 타입.
        found: TypeSpec,
    },
    /// 허용되지 않은 참조 대상을 씁니다(위치별 규칙).
    ForbiddenReference(ReferenceTarget),
    /// bytecode table이 너무 큽니다.
    TooLarge,
    /// canonical AST bytes가 잘못됐습니다.
    Codec(CodecError),
}

impl fmt::Display for ExpressionCheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExpressionCheckError::TypeErrors(errors) => match errors.first() {
                Some(first) => write!(f, "표현식 type 오류 {}건(첫 오류: {first})", errors.len()),
                None => f.write_str("표현식 root 타입을 추론할 수 없습니다"),
            },
            ExpressionCheckError::TypeMismatch { expected, found } => {
                write!(
                    f,
                    "표현식 타입 {found}는 선언 타입 {expected}에 맞지 않습니다"
                )
            }
            ExpressionCheckError::ForbiddenReference(target) => {
                write!(f, "이 위치에서 쓸 수 없는 참조: {target}")
            }
            ExpressionCheckError::TooLarge => f.write_str("표현식이 너무 큽니다"),
            ExpressionCheckError::Codec(err) => write!(f, "표현식 bytes 오류: {err}"),
        }
    }
}

impl std::error::Error for ExpressionCheckError {}

/// `found` 타입 값을 `expected` 타입 위치에 둘 수 있는지.
///
/// 비-nullable 값 타입이 같아야 하고(`Enum`은 enum 타입 ID까지), nullable 값(`Optional<T>`)은
/// nullable 위치에만 둘 수 있습니다. `T`는 `Optional<T>` 위치에 둘 수 있습니다.
pub fn assignable(found: TypeSpec, expected: TypeSpec) -> bool {
    found.value_type() == expected.value_type() && (expected.is_optional() || !found.is_optional())
}

/// 선언 타입에 대해 검사를 통과한 표현식.
///
/// 동등성은 선언 타입과 canonical AST bytes로 판단합니다(GUI 생성 순서와 무관, VIII.2.5).
#[derive(Debug, Clone)]
pub struct CheckedExpression {
    ast: ExpressionAst,
    canonical: Vec<u8>,
    declared: TypeSpec,
    program: Program,
}

impl PartialEq for CheckedExpression {
    fn eq(&self, other: &CheckedExpression) -> bool {
        self.declared == other.declared && self.canonical == other.canonical
    }
}

impl Eq for CheckedExpression {}

impl CheckedExpression {
    /// `ast`를 참조 타입 환경 `references`로 검사하고 root 타입이 `declared`에 대입되는지 확인합니다.
    pub fn new(
        ast: ExpressionAst,
        declared: TypeSpec,
        references: &dyn ReferenceTypes,
    ) -> Result<CheckedExpression, ExpressionCheckError> {
        let program = Program::compile(&ast, references).map_err(|err| match err {
            CompileError::TypeErrors(errors) => ExpressionCheckError::TypeErrors(errors),
            CompileError::TooLarge => ExpressionCheckError::TooLarge,
        })?;
        let found = program.result_type();
        if !assignable(found, declared) {
            return Err(ExpressionCheckError::TypeMismatch {
                expected: declared,
                found,
            });
        }
        let canonical = ast
            .to_canonical_bytes()
            .map_err(ExpressionCheckError::Codec)?;
        Ok(CheckedExpression {
            ast,
            canonical,
            declared,
            program,
        })
    }

    /// 참조가 하나도 없는(닫힌) 표현식으로 검사합니다. 참조가 있으면 첫 참조를 거부합니다.
    pub fn closed(
        ast: ExpressionAst,
        declared: TypeSpec,
    ) -> Result<CheckedExpression, ExpressionCheckError> {
        if let Some(target) = ast.referenced_targets().into_iter().next() {
            return Err(ExpressionCheckError::ForbiddenReference(target.clone()));
        }
        let none: std::collections::BTreeMap<ReferenceTarget, TypeSpec> =
            std::collections::BTreeMap::new();
        CheckedExpression::new(ast, declared, &none)
    }

    /// 원본 AST.
    pub fn ast(&self) -> &ExpressionAst {
        &self.ast
    }

    /// canonical AST bytes(VIII.2.5).
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }

    /// 선언 타입.
    pub fn declared_type(&self) -> TypeSpec {
        self.declared
    }

    /// root 추론 타입(선언 타입에 대입 가능).
    pub fn result_type(&self) -> TypeSpec {
        self.program.result_type()
    }

    /// 참조 대상 목록(중복 없음, 정렬).
    pub fn references(&self) -> Vec<ReferenceTarget> {
        self.ast.referenced_targets().into_iter().cloned().collect()
    }

    /// 기본 한도([`EvalLimits::DEFAULT`])로 평가합니다.
    pub fn evaluate(&self, values: &dyn ReferenceValues) -> Result<Option<TypedValue>, EvalError> {
        self.evaluate_with_limits(values, EvalLimits::DEFAULT)
    }

    /// 주어진 한도로 평가합니다.
    pub fn evaluate_with_limits(
        &self,
        values: &dyn ReferenceValues,
        limits: EvalLimits,
    ) -> Result<Option<TypedValue>, EvalError> {
        self.program
            .evaluate_with_limits(values, limits)
            .map(|(value, _)| value)
    }
}

/// [`TypeSpec`]을 기록합니다: nullable flag(u8) + 값 타입 tag(u8, VIII.2.2 tag 표) + `Enum`이면
/// enum 타입 ID(u32 LE).
pub(crate) fn put_type_spec(buf: &mut Vec<u8>, spec: TypeSpec) {
    put_u8(buf, u8::from(spec.is_optional()));
    let value_type = spec.value_type();
    put_u8(buf, value_type_tag(value_type));
    if let ExpressionType::Enum(id) = value_type {
        put_u32(buf, id.raw());
    }
}

/// canonical AST bytes를 길이 프리픽스로 기록합니다.
pub(crate) fn put_expression(buf: &mut Vec<u8>, expression: &CheckedExpression) {
    put_bytes(buf, expression.canonical_bytes());
}

impl Reader<'_> {
    /// [`put_type_spec`] 형식을 읽습니다. 알 수 없는 tag·flag는 거부합니다.
    pub(crate) fn type_spec(&mut self) -> Result<TypeSpec, Malformed> {
        let optional = self.bool()?;
        let value_type = match self.u8()? {
            0x01 => ExpressionType::Boolean,
            0x02 => ExpressionType::Number,
            0x03 => ExpressionType::String,
            0x04 => ExpressionType::Enum(EnumTypeId::new(self.u32()?)),
            0x05 => ExpressionType::Length,
            0x06 => ExpressionType::Percentage,
            0x07 => ExpressionType::Angle,
            0x08 => ExpressionType::Duration,
            0x09 => ExpressionType::Color,
            0x0A => ExpressionType::Gradient,
            0x0B => ExpressionType::Shadow,
            0x0C => ExpressionType::Typography,
            0x0D => ExpressionType::ResourceRef,
            _ => return Err(Malformed("알 수 없는 값 타입 tag")),
        };
        Ok(if optional {
            TypeSpec::optional(value_type)
        } else {
            TypeSpec::required(value_type)
        })
    }

    /// 길이 프리픽스 canonical AST bytes를 읽어 AST로 디코딩합니다.
    pub(crate) fn expression_ast(
        &mut self,
    ) -> Result<Result<ExpressionAst, CodecError>, Malformed> {
        let bytes = self.bytes()?;
        Ok(ExpressionAst::from_canonical_bytes(bytes))
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    //! 테스트용 AST 생성 도우미.

    use stellar_core_types::ids::StableId;
    use stellar_core_types::typed_value::TypedValue;
    use stellar_expressions::ast::{ExprNode, ExprNodeId, ExpressionAst, ReferenceTarget};
    use stellar_expressions::ops::BinaryOp;
    use stellar_expressions::types::ExpressionType;

    /// 테스트용 node id.
    pub(crate) fn expr_id(n: u64) -> ExprNodeId {
        ExprNodeId::new(StableId::new_v7(n, 0x0e0e, n))
    }

    /// 호출마다 다른 node id(AST 조합 시 id 충돌 방지).
    fn fresh_id() -> ExprNodeId {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(10_000);
        expr_id(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    /// 리터럴 하나짜리 AST.
    pub(crate) fn literal(value: TypedValue) -> ExpressionAst {
        let id = fresh_id();
        ExpressionAst::new(id, vec![ExprNode::literal(id, value)]).unwrap()
    }

    /// null 리터럴 하나짜리 AST.
    pub(crate) fn null(value_type: ExpressionType) -> ExpressionAst {
        let id = fresh_id();
        ExpressionAst::new(id, vec![ExprNode::null(id, value_type)]).unwrap()
    }

    /// 참조 하나짜리 AST.
    pub(crate) fn reference(target: ReferenceTarget) -> ExpressionAst {
        let id = fresh_id();
        ExpressionAst::new(id, vec![ExprNode::reference(id, target)]).unwrap()
    }

    /// `lhs op rhs` AST.
    pub(crate) fn binary(op: BinaryOp, lhs: ExpressionAst, rhs: ExpressionAst) -> ExpressionAst {
        let root = fresh_id();
        let mut nodes = vec![ExprNode::binary(root, op, lhs.root(), rhs.root())];
        nodes.extend(lhs.nodes().iter().cloned());
        nodes.extend(
            rhs.nodes()
                .iter()
                .filter(|n| lhs.node(n.id()).is_none())
                .cloned(),
        );
        ExpressionAst::new(root, nodes).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::test_util::*;
    use super::*;
    use std::collections::BTreeMap;
    use stellar_core_types::ids::StableId;
    use stellar_core_types::value::FiniteF32;
    use stellar_expressions::ops::BinaryOp;

    #[test]
    fn assignability_rules() {
        let num = TypeSpec::required(ExpressionType::Number);
        let opt = TypeSpec::optional(ExpressionType::Number);
        assert!(assignable(num, num));
        assert!(assignable(num, opt), "T → Optional<T>");
        assert!(!assignable(opt, num), "Optional<T> → T 금지");
        assert!(!assignable(num, TypeSpec::required(ExpressionType::String)));
        let e1 = TypeSpec::required(ExpressionType::Enum(EnumTypeId::new(1)));
        let e2 = TypeSpec::required(ExpressionType::Enum(EnumTypeId::new(2)));
        assert!(!assignable(e1, e2), "enum 타입 ID까지 같아야 함");
    }

    #[test]
    fn checked_expression_validates_type_and_evaluates() {
        let two = TypedValue::Number(FiniteF32::new(2.0).unwrap());
        let ast = binary(BinaryOp::Add, literal(two.clone()), literal(two));
        let checked =
            CheckedExpression::closed(ast.clone(), TypeSpec::required(ExpressionType::Number))
                .unwrap();
        let none: BTreeMap<ReferenceTarget, Option<TypedValue>> = BTreeMap::new();
        assert_eq!(
            checked.evaluate(&none).unwrap(),
            Some(TypedValue::Number(FiniteF32::new(4.0).unwrap()))
        );
        assert_eq!(
            CheckedExpression::closed(ast, TypeSpec::required(ExpressionType::String)),
            Err(ExpressionCheckError::TypeMismatch {
                expected: TypeSpec::required(ExpressionType::String),
                found: TypeSpec::required(ExpressionType::Number),
            })
        );
        // 참조가 있으면 닫힌 표현식이 아닙니다.
        let target = ReferenceTarget::ComponentProperty(StableId::new_v7(1, 0, 1));
        assert_eq!(
            CheckedExpression::closed(
                reference(target.clone()),
                TypeSpec::required(ExpressionType::Number)
            ),
            Err(ExpressionCheckError::ForbiddenReference(target))
        );
    }

    #[test]
    fn type_spec_codec_round_trips() {
        for spec in [
            TypeSpec::required(ExpressionType::Boolean),
            TypeSpec::optional(ExpressionType::String),
            TypeSpec::required(ExpressionType::Enum(EnumTypeId::new(77))),
            TypeSpec::optional(ExpressionType::ResourceRef),
        ] {
            let mut buf = Vec::new();
            put_type_spec(&mut buf, spec);
            let mut reader = Reader::new(&buf);
            assert_eq!(reader.type_spec().unwrap(), spec);
            reader.expect_end().unwrap();
        }
        assert!(Reader::new(&[0, 0xEE]).type_spec().is_err());
        assert!(Reader::new(&[2, 0x01]).type_spec().is_err());
    }
}
