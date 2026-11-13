//! 표현식 평가 공통 타입과 값 의미 (VIII.4.1, VIII.4.2).
//!
//! bytecode evaluator([`crate::vm`])와 AST 기준 evaluator([`crate::ast_eval`])가 같은 오류 타입·참조 입력·
//! 값 의미 함수를 공유합니다. 두 evaluator가 이 모듈의 함수만으로 값을 계산하므로 결과가 같아야 합니다.
//!
//! 평가 값은 `Option<TypedValue>`입니다. `None`은 `Optional<T>`의 값 없음(null)입니다.

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::{Angle, Duration, FiniteF32, Length, Percentage};

use crate::arithmetic::{arithmetic_rule, ArithmeticOp};
use crate::ast::{ExprNodeId, ReferenceTarget};
use crate::functions::FunctionId;
use crate::logic::{ComparisonOp, LogicalOp};
use crate::ops::{BinaryOp, BinaryOpClass, UnaryOp};
use crate::types::{ExpressionType, TypeSpec};

/// 참조 대상의 현재 값 제공자.
///
/// 반환값: `None` = 모르는 대상, `Some(None)` = 값 없음(null), `Some(Some(v))` = 값.
pub trait ReferenceValues {
    /// 참조 대상의 현재 값.
    fn reference_value(&self, target: &ReferenceTarget) -> Option<Option<TypedValue>>;
}

impl ReferenceValues for BTreeMap<ReferenceTarget, Option<TypedValue>> {
    fn reference_value(&self, target: &ReferenceTarget) -> Option<Option<TypedValue>> {
        self.get(target).cloned()
    }
}

/// 평가 오류 종류.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EvalErrorKind {
    /// 참조 대상 값이 입력에 없습니다.
    MissingReference(ReferenceTarget),
    /// 참조 값이 type 검사 때의 타입과 다릅니다(값 없음이 허용되지 않는 경우 포함).
    ReferenceTypeMismatch {
        /// 참조 대상.
        target: ReferenceTarget,
        /// 기대 타입.
        expected: TypeSpec,
    },
    /// 실행 중 값의 타입이 연산 규칙과 맞지 않습니다(검증된 program에서는 발생하지 않는 방어 오류).
    OperandTypeMismatch,
    /// program 내부 불변식 위반(stack 부족·잘못된 index). 검증된 program에서는 발생하지 않습니다.
    MalformedProgram,
    /// AST 기준 evaluator에 type 오류가 있는 AST가 주어졌습니다.
    IllTyped,
    /// 0으로 나누기 또는 0에 대한 나머지(VIII.4.2). `remap`의 빈 입력 범위 포함(VIII.4.4).
    DivisionByZero,
    /// 결과가 유한 범위를 벗어났습니다(overflow, VIII.4.2).
    NonFiniteResult,
    /// 함수 인자 값이 허용 범위 밖입니다(VIII.4.4).
    InvalidArgument {
        /// 호출한 함수.
        function: FunctionId,
        /// 인자 위치.
        argument: u16,
        /// 위반 내용.
        issue: ArgumentIssue,
    },
    /// 문자열 결과가 [`MAX_STRING_BYTES`]를 넘습니다(VIII.4.4).
    StringTooLong,
    /// 실행 명령 수가 instruction budget에 도달했습니다(VIII.4.5).
    InstructionBudgetExceeded {
        /// 적용한 budget.
        budget: u64,
    },
    /// 공유 block 중첩(frame) 깊이 한도를 넘었습니다(VIII.4.5).
    CallDepthExceeded {
        /// 적용한 한도.
        limit: usize,
    },
    /// operand stack 깊이 한도를 넘었습니다(VIII.4.5).
    StackLimitExceeded {
        /// 적용한 한도.
        limit: usize,
    },
}

/// 문자열 함수 결과의 최대 UTF-8 byte 수(VIII.4.4). 공유 node로 결합을 반복해도 메모리가 폭증하지 않게 합니다.
pub const MAX_STRING_BYTES: usize = 1 << 20;

/// 함수 인자 값 위반 내용(VIII.4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ArgumentIssue {
    /// 범위 하한이 상한보다 큽니다(`clamp`).
    InvertedRange,
    /// 값이 허용 구간 밖입니다(`mix`의 t·`with_alpha`의 alpha는 [0, 1], `format_number` 자릿수는 0~9).
    OutOfRange,
    /// 정수여야 하는 값이 정수가 아닙니다(`format_number` 자릿수).
    NotInteger,
    /// 빈 검색 문자열(`replace`).
    EmptySearch,
}

impl fmt::Display for ArgumentIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ArgumentIssue::InvertedRange => "하한이 상한보다 큽니다",
            ArgumentIssue::OutOfRange => "허용 구간 밖입니다",
            ArgumentIssue::NotInteger => "정수가 아닙니다",
            ArgumentIssue::EmptySearch => "빈 검색 문자열입니다",
        })
    }
}

impl fmt::Display for EvalErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EvalErrorKind::MissingReference(target) => write!(f, "참조 값이 없습니다: {target}"),
            EvalErrorKind::ReferenceTypeMismatch { target, expected } => {
                write!(f, "참조 {target}의 값이 {expected} 타입이 아닙니다")
            }
            EvalErrorKind::OperandTypeMismatch => f.write_str("연산 값의 타입이 규칙과 다릅니다"),
            EvalErrorKind::MalformedProgram => f.write_str("program 내부 불변식 위반"),
            EvalErrorKind::IllTyped => f.write_str("type 오류가 있는 AST는 평가할 수 없습니다"),
            EvalErrorKind::DivisionByZero => f.write_str("0으로 나눌 수 없습니다"),
            EvalErrorKind::NonFiniteResult => f.write_str("결과가 유한 범위를 벗어났습니다"),
            EvalErrorKind::InvalidArgument {
                function,
                argument,
                issue,
            } => write!(f, "{function}의 인자 {argument}: {issue}"),
            EvalErrorKind::StringTooLong => {
                write!(f, "문자열 결과가 {MAX_STRING_BYTES} bytes를 넘습니다")
            }
            EvalErrorKind::InstructionBudgetExceeded { budget } => {
                write!(f, "명령 {budget}개 budget을 넘었습니다")
            }
            EvalErrorKind::CallDepthExceeded { limit } => {
                write!(f, "공유 block 중첩 깊이 {limit}을 넘었습니다")
            }
            EvalErrorKind::StackLimitExceeded { limit } => {
                write!(f, "operand stack 깊이 {limit}을 넘었습니다")
            }
        }
    }
}

/// 평가 오류 (VIII.4.1). 원인 AST node를 함께 기록합니다.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EvalError {
    /// 오류가 난 AST node(알 수 있으면).
    pub node: Option<ExprNodeId>,
    /// 오류 종류.
    pub kind: EvalErrorKind,
}

impl EvalError {
    /// 오류를 만듭니다.
    pub const fn new(node: Option<ExprNodeId>, kind: EvalErrorKind) -> EvalError {
        EvalError { node, kind }
    }
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.node {
            Some(node) => write!(f, "node {node}: {}", self.kind),
            None => write!(f, "{}", self.kind),
        }
    }
}

impl std::error::Error for EvalError {}

/// 참조 값을 읽고 type 검사 때의 타입과 맞는지 확인합니다.
pub fn load_reference(
    target: &ReferenceTarget,
    expected: TypeSpec,
    values: &dyn ReferenceValues,
) -> Result<Option<TypedValue>, EvalErrorKind> {
    let value = values
        .reference_value(target)
        .ok_or_else(|| EvalErrorKind::MissingReference(target.clone()))?;
    if expected.accepts(value.as_ref()) {
        Ok(value)
    } else {
        Err(EvalErrorKind::ReferenceTypeMismatch {
            target: target.clone(),
            expected,
        })
    }
}

/// 깊이 한도를 지키는 operand stack (VIII.4.5).
///
/// push 전에 깊이를 검사하므로 stack은 어떤 순간에도 `max_depth`개를 넘지 않습니다. bytecode evaluator와
/// AST 기준 evaluator가 같은 한도 의미를 공유합니다.
pub(crate) struct BoundedStack {
    values: Vec<Option<TypedValue>>,
    max_depth: usize,
    peak_depth: usize,
}

impl BoundedStack {
    /// 빈 stack을 만듭니다.
    pub(crate) fn new(max_depth: usize) -> BoundedStack {
        BoundedStack {
            values: Vec::new(),
            max_depth,
            peak_depth: 0,
        }
    }

    /// 값을 push합니다. push하면 `max_depth`를 넘는 경우 push하지 않고 오류입니다.
    pub(crate) fn push(&mut self, value: Option<TypedValue>) -> Result<(), EvalErrorKind> {
        if self.values.len() >= self.max_depth {
            return Err(EvalErrorKind::StackLimitExceeded {
                limit: self.max_depth,
            });
        }
        self.values.push(value);
        self.peak_depth = self.peak_depth.max(self.values.len());
        Ok(())
    }

    /// 맨 위 값을 꺼냅니다.
    pub(crate) fn pop(&mut self) -> Option<Option<TypedValue>> {
        self.values.pop()
    }

    /// 맨 위 값.
    pub(crate) fn last(&self) -> Option<&Option<TypedValue>> {
        self.values.last()
    }

    /// 위에서 `count`개를 순서대로 꺼냅니다. 값이 모자라면 `None`(stack 불변).
    pub(crate) fn pop_many(&mut self, count: usize) -> Option<Vec<Option<TypedValue>>> {
        let start = self.values.len().checked_sub(count)?;
        Some(self.values.split_off(start))
    }

    /// 비어 있으면 `true`.
    pub(crate) fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// 지금까지의 최대 깊이.
    pub(crate) fn peak_depth(&self) -> usize {
        self.peak_depth
    }
}

/// 조건 값의 참·거짓(`if` 조건, `and`/`or` 왼쪽). Boolean이 아니면 방어 오류입니다.
pub fn truth(condition: &Option<TypedValue>) -> Result<bool, EvalErrorKind> {
    match condition {
        Some(TypedValue::Boolean(value)) => Ok(*value),
        _ => Err(EvalErrorKind::OperandTypeMismatch),
    }
}

/// `and`/`or`의 왼쪽 값만으로 결과가 정해지면 그 결과(short-circuit, VIII.4.3).
pub fn short_circuit(op: LogicalOp, lhs: bool) -> Option<bool> {
    match (op, lhs) {
        (LogicalOp::And, false) => Some(false),
        (LogicalOp::Or, true) => Some(true),
        _ => None,
    }
}

/// `match` arm 선택 의미: 대상과 같은 첫 pattern의 index. 없으면 `None`(기본 branch).
pub fn match_arm(scrutinee: &Option<TypedValue>, patterns: &[TypedValue]) -> Option<usize> {
    let scrutinee = scrutinee.as_ref()?;
    patterns.iter().position(|pattern| pattern == scrutinee)
}

// --- 연산 의미 (VIII.4.2) ---
//
// 수치·단위 값은 f32 IEEE-754 기본 연산(+, −, ×, ÷)과 정확(exact) 연산인 나머지(fmod)·부호 반전만 씁니다.
// 결과가 비유한(overflow)이면 `NonFiniteResult`, 0으로 나누기·0에 대한 나머지는 `DivisionByZero`입니다.
// 결과 `-0.0`은 `FiniteF32` 규칙대로 `+0.0`으로 정규화합니다.

/// 수치·단위 값의 내부 f32와 타입.
pub(crate) fn numeric(value: &Option<TypedValue>) -> Result<(ExpressionType, f32), EvalErrorKind> {
    match value {
        Some(TypedValue::Number(v)) => Ok((ExpressionType::Number, v.get())),
        Some(TypedValue::Length(v)) => Ok((ExpressionType::Length, v.get())),
        Some(TypedValue::Percentage(v)) => Ok((ExpressionType::Percentage, v.get())),
        Some(TypedValue::Angle(v)) => Ok((ExpressionType::Angle, v.get())),
        Some(TypedValue::Duration(v)) => Ok((ExpressionType::Duration, v.get())),
        _ => Err(EvalErrorKind::OperandTypeMismatch),
    }
}

/// f32 결과를 `value_type`의 값으로 만듭니다. 비유한이면 `NonFiniteResult`.
pub(crate) fn make_numeric(
    value_type: ExpressionType,
    raw: f32,
) -> Result<Option<TypedValue>, EvalErrorKind> {
    let finite = FiniteF32::new(raw).map_err(|_| EvalErrorKind::NonFiniteResult)?;
    let value = match value_type {
        ExpressionType::Number => TypedValue::Number(finite),
        ExpressionType::Length => TypedValue::Length(Length::from_raw(finite)),
        ExpressionType::Percentage => TypedValue::Percentage(Percentage::from_raw(finite)),
        ExpressionType::Angle => TypedValue::Angle(Angle::from_raw(finite)),
        ExpressionType::Duration => TypedValue::Duration(Duration::from_raw(finite)),
        _ => return Err(EvalErrorKind::OperandTypeMismatch),
    };
    Ok(Some(value))
}

fn boolean(value: &Option<TypedValue>) -> Result<bool, EvalErrorKind> {
    match value {
        Some(TypedValue::Boolean(b)) => Ok(*b),
        _ => Err(EvalErrorKind::OperandTypeMismatch),
    }
}

/// 단항 연산 의미(VIII.4.2).
pub fn apply_unary(
    op: UnaryOp,
    operand: &Option<TypedValue>,
) -> Result<Option<TypedValue>, EvalErrorKind> {
    match op {
        UnaryOp::Negate => {
            let (value_type, raw) = numeric(operand)?;
            make_numeric(value_type, -raw)
        }
        UnaryOp::Not => Ok(Some(TypedValue::Boolean(!boolean(operand)?))),
    }
}

/// 이항 연산 의미(VIII.4.2). 결과 타입은 산술 규칙 표(VIII.1.2)를 따릅니다.
pub fn apply_binary(
    op: BinaryOp,
    lhs: &Option<TypedValue>,
    rhs: &Option<TypedValue>,
) -> Result<Option<TypedValue>, EvalErrorKind> {
    match op.class() {
        BinaryOpClass::Arithmetic(arith) => {
            let (lhs_type, a) = numeric(lhs)?;
            let (rhs_type, b) = numeric(rhs)?;
            let rule = arithmetic_rule(arith, lhs_type, rhs_type)
                .ok_or(EvalErrorKind::OperandTypeMismatch)?;
            let raw = match arith {
                ArithmeticOp::Add => a + b,
                ArithmeticOp::Subtract => a - b,
                ArithmeticOp::Multiply => a * b,
                ArithmeticOp::Divide | ArithmeticOp::Remainder if b == 0.0 => {
                    return Err(EvalErrorKind::DivisionByZero)
                }
                ArithmeticOp::Divide => a / b,
                ArithmeticOp::Remainder => a % b,
            };
            make_numeric(rule.result, raw)
        }
        BinaryOpClass::Comparison(comparison) => {
            let result = match comparison {
                ComparisonOp::Equal => lhs == rhs,
                ComparisonOp::NotEqual => lhs != rhs,
                ordered => {
                    let (lhs_type, a) = numeric(lhs)?;
                    let (rhs_type, b) = numeric(rhs)?;
                    if lhs_type != rhs_type {
                        return Err(EvalErrorKind::OperandTypeMismatch);
                    }
                    match ordered {
                        ComparisonOp::Less => a < b,
                        ComparisonOp::LessEqual => a <= b,
                        ComparisonOp::Greater => a > b,
                        _ => a >= b,
                    }
                }
            };
            Ok(Some(TypedValue::Boolean(result)))
        }
        BinaryOpClass::Logical(logical) => {
            let (a, b) = (boolean(lhs)?, boolean(rhs)?);
            Ok(Some(TypedValue::Boolean(match logical {
                LogicalOp::And => a && b,
                LogicalOp::Or => a || b,
            })))
        }
    }
}

#[cfg(test)]
mod tests {
    use stellar_core_types::ids::StableId;
    use stellar_core_types::reference::TokenRef;
    use stellar_core_types::value::FiniteF32;

    use super::*;
    use crate::types::ExpressionType;

    #[test]
    fn reference_loading_validates_type_and_null() {
        let token = ReferenceTarget::Token(TokenRef::new(StableId::from_bytes([1; 16])));
        let number = TypedValue::Number(FiniteF32::ONE);
        let mut values: BTreeMap<ReferenceTarget, Option<TypedValue>> = BTreeMap::new();
        let required = TypeSpec::required(ExpressionType::Number);
        let optional = TypeSpec::optional(ExpressionType::Number);
        assert_eq!(
            load_reference(&token, required, &values),
            Err(EvalErrorKind::MissingReference(token.clone()))
        );
        values.insert(token.clone(), Some(number.clone()));
        assert_eq!(load_reference(&token, required, &values), Ok(Some(number)));
        values.insert(token.clone(), None);
        assert_eq!(load_reference(&token, optional, &values), Ok(None));
        assert_eq!(
            load_reference(&token, required, &values),
            Err(EvalErrorKind::ReferenceTypeMismatch {
                target: token.clone(),
                expected: required
            })
        );
        values.insert(token.clone(), Some(TypedValue::Boolean(true)));
        assert!(load_reference(&token, optional, &values).is_err());
    }

    #[test]
    fn selection_semantics() {
        let a = Some(TypedValue::Boolean(true));
        let b = Some(TypedValue::Boolean(false));
        assert_eq!(truth(&a), Ok(true));
        assert_eq!(truth(&b), Ok(false));
        assert_eq!(truth(&None), Err(EvalErrorKind::OperandTypeMismatch));
        assert_eq!(short_circuit(LogicalOp::And, false), Some(false));
        assert_eq!(short_circuit(LogicalOp::And, true), None);
        assert_eq!(short_circuit(LogicalOp::Or, true), Some(true));
        assert_eq!(short_circuit(LogicalOp::Or, false), None);
        let patterns = [TypedValue::Boolean(false), TypedValue::Boolean(true)];
        assert_eq!(match_arm(&a, &patterns), Some(1));
        assert_eq!(match_arm(&None, &patterns), None);
    }
}
