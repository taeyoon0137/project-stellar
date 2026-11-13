//! type rule 공통 오류 (VIII.1.2).
//!
//! 산술·비교·논리·조건식 rule은 위반 시 [`TypeRuleError`]를 반환합니다. 오류는 **어느 피연산자
//! 위치**([`OperandPosition`])가 잘못되었는지, 그 위치의 실제 타입과 기대 조건을 함께 담습니다.
//! type checker(VIII.2.4)는 이 위치를 AST child slot과 node StableId로 바꿔 보고합니다.

use std::fmt;

use crate::types::{ExpressionType, TypeSpec};

/// rule 위반이 발생한 피연산자 위치.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OperandPosition {
    /// 단항 연산의 피연산자.
    Operand,
    /// 이항 연산의 왼쪽 피연산자.
    Lhs,
    /// 이항 연산의 오른쪽 피연산자.
    Rhs,
}

impl fmt::Display for OperandPosition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OperandPosition::Operand => f.write_str("피연산자"),
            OperandPosition::Lhs => f.write_str("왼쪽 피연산자"),
            OperandPosition::Rhs => f.write_str("오른쪽 피연산자"),
        }
    }
}

/// 위반 위치가 만족해야 했던 조건.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Expectation {
    /// 나열된 비-nullable 타입 중 하나여야 합니다(오름차순, 중복 없음). 빈 목록은 이 연산에서
    /// 앞선 피연산자와 짝이 되는 타입이 없음을 뜻합니다.
    OneOf(Vec<ExpressionType>),
}

impl fmt::Display for Expectation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expectation::OneOf(types) if types.is_empty() => f.write_str("허용 타입 없음"),
            Expectation::OneOf(types) => {
                for (index, value_type) in types.iter().enumerate() {
                    if index > 0 {
                        f.write_str(" | ")?;
                    }
                    write!(f, "{value_type}")?;
                }
                Ok(())
            }
        }
    }
}

/// type rule 위반 (VIII.1.2).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeRuleError {
    /// 잘못된 피연산자 위치.
    pub position: OperandPosition,
    /// 그 위치의 실제 타입.
    pub found: TypeSpec,
    /// 그 위치가 만족해야 했던 조건.
    pub expected: Expectation,
}

impl TypeRuleError {
    /// "나열된 타입 중 하나" 조건 위반을 만듭니다. `allowed`는 정렬·중복 제거됩니다.
    pub fn one_of(
        position: OperandPosition,
        found: TypeSpec,
        mut allowed: Vec<ExpressionType>,
    ) -> TypeRuleError {
        allowed.sort();
        allowed.dedup();
        TypeRuleError {
            position,
            found,
            expected: Expectation::OneOf(allowed),
        }
    }
}

impl fmt::Display for TypeRuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} 타입은 허용되지 않습니다(기대: {})",
            self.position, self.found, self.expected
        )
    }
}

impl std::error::Error for TypeRuleError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_of_sorts_and_dedups() {
        let err = TypeRuleError::one_of(
            OperandPosition::Rhs,
            TypeSpec::required(ExpressionType::Color),
            vec![
                ExpressionType::Number,
                ExpressionType::Length,
                ExpressionType::Number,
            ],
        );
        assert_eq!(
            err.expected,
            Expectation::OneOf(vec![ExpressionType::Number, ExpressionType::Length])
        );
        assert_eq!(
            err.to_string(),
            "오른쪽 피연산자: Color 타입은 허용되지 않습니다(기대: Number | Length)"
        );
        let none = TypeRuleError::one_of(
            OperandPosition::Lhs,
            TypeSpec::optional(ExpressionType::Length),
            Vec::new(),
        );
        assert_eq!(
            none.to_string(),
            "왼쪽 피연산자: Optional<Length> 타입은 허용되지 않습니다(기대: 허용 타입 없음)"
        );
    }
}
