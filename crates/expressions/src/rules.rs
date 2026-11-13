//! type rule 공통 오류 (VIII.1.2, VIII.1.3, VIII.1.4).
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
    /// `if`의 조건(VIII.1.3).
    Condition,
    /// `if`의 참 branch(VIII.1.3).
    Then,
    /// `if`의 거짓 branch(VIII.1.3).
    Else,
    /// `match`의 검사 대상(VIII.1.3).
    Scrutinee,
    /// `match`의 `index`번째 arm pattern(VIII.1.3).
    Pattern(u16),
    /// `match`의 `index`번째 arm 값(VIII.1.3).
    Arm(u16),
    /// `match`의 기본 branch(VIII.1.3).
    Default,
    /// 함수 호출의 `index`번째 인자(VIII.1.4).
    Argument(u16),
}

impl fmt::Display for OperandPosition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OperandPosition::Operand => f.write_str("피연산자"),
            OperandPosition::Lhs => f.write_str("왼쪽 피연산자"),
            OperandPosition::Rhs => f.write_str("오른쪽 피연산자"),
            OperandPosition::Condition => f.write_str("조건"),
            OperandPosition::Then => f.write_str("참 branch"),
            OperandPosition::Else => f.write_str("거짓 branch"),
            OperandPosition::Scrutinee => f.write_str("match 대상"),
            OperandPosition::Pattern(index) => write!(f, "match pattern {index}"),
            OperandPosition::Arm(index) => write!(f, "match arm {index}"),
            OperandPosition::Default => f.write_str("match 기본 branch"),
            OperandPosition::Argument(index) => write!(f, "인자 {index}"),
        }
    }
}

/// 타입 분류(VIII.1.3). 개별 타입을 나열할 수 없는 조건(예: 모든 enum 타입)을 표현합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TypeClass {
    /// `==`·`!=`로 비교할 수 있는 타입. 합성 값(Gradient·Shadow·Typography)을 제외한 전부이며
    /// nullable도 허용합니다.
    Equatable,
    /// `<`·`<=`·`>`·`>=`로 비교할 수 있는 비-nullable 수치·단위 타입.
    Ordered,
    /// `match` 대상이 될 수 있는 비-nullable 이산 타입(Boolean·Number·String·Enum).
    Matchable,
}

impl TypeClass {
    /// 비-nullable 타입이 이 분류에 속하는지 여부.
    pub const fn contains(&self, value_type: ExpressionType) -> bool {
        match self {
            TypeClass::Equatable => !matches!(
                value_type,
                ExpressionType::Gradient | ExpressionType::Shadow | ExpressionType::Typography
            ),
            TypeClass::Ordered => matches!(
                value_type,
                ExpressionType::Number
                    | ExpressionType::Length
                    | ExpressionType::Percentage
                    | ExpressionType::Angle
                    | ExpressionType::Duration
            ),
            TypeClass::Matchable => matches!(
                value_type,
                ExpressionType::Boolean
                    | ExpressionType::Number
                    | ExpressionType::String
                    | ExpressionType::Enum(_)
            ),
        }
    }

    /// 진단용 이름.
    pub const fn name(&self) -> &'static str {
        match self {
            TypeClass::Equatable => "동등 비교 가능 타입",
            TypeClass::Ordered => "순서 비교 가능 수치·단위 타입",
            TypeClass::Matchable => "match 가능 이산 타입(Boolean·Number·String·Enum)",
        }
    }
}

/// 위반 위치가 만족해야 했던 조건.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Expectation {
    /// 나열된 비-nullable 타입 중 하나여야 합니다(오름차순, 중복 없음). 빈 목록은 이 연산에서
    /// 앞선 피연산자와 짝이 되는 타입이 없음을 뜻합니다.
    OneOf(Vec<ExpressionType>),
    /// 이 분류의 타입이어야 합니다(VIII.1.3). `Equatable` 외에는 비-nullable이어야 합니다.
    Class(TypeClass),
    /// 앞선 branch의 타입과 같거나 명시적 공통 타입이 있어야 합니다(VIII.1.3).
    CommonWith(TypeSpec),
    /// 앞선 pattern과 다른 값이어야 합니다(VIII.1.3).
    DistinctPattern,
    /// `match`에는 arm이 하나 이상 있어야 합니다(VIII.1.3).
    AtLeastOneArm,
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
            Expectation::Class(class) => f.write_str(class.name()),
            Expectation::CommonWith(spec) => write!(f, "{spec}와 같은 타입 또는 공통 타입"),
            Expectation::DistinctPattern => f.write_str("앞선 pattern과 다른 값"),
            Expectation::AtLeastOneArm => f.write_str("arm 1개 이상"),
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

    /// 임의 조건 위반을 만듭니다.
    pub const fn new(
        position: OperandPosition,
        found: TypeSpec,
        expected: Expectation,
    ) -> TypeRuleError {
        TypeRuleError {
            position,
            found,
            expected,
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
    use crate::types::EnumTypeId;

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

    #[test]
    fn type_classes_are_stable() {
        let all = [
            ExpressionType::Boolean,
            ExpressionType::Number,
            ExpressionType::String,
            ExpressionType::Enum(EnumTypeId::new(4)),
            ExpressionType::Length,
            ExpressionType::Percentage,
            ExpressionType::Angle,
            ExpressionType::Duration,
            ExpressionType::Color,
            ExpressionType::Gradient,
            ExpressionType::Shadow,
            ExpressionType::Typography,
            ExpressionType::ResourceRef,
        ];
        let names = |class: TypeClass| -> Vec<&'static str> {
            all.iter()
                .filter(|t| class.contains(**t))
                .map(|t| t.name())
                .collect()
        };
        assert_eq!(
            names(TypeClass::Equatable),
            vec![
                "Boolean",
                "Number",
                "String",
                "Enum",
                "Length",
                "Percentage",
                "Angle",
                "Duration",
                "Color",
                "ResourceRef"
            ]
        );
        assert_eq!(
            names(TypeClass::Ordered),
            vec!["Number", "Length", "Percentage", "Angle", "Duration"]
        );
        assert_eq!(
            names(TypeClass::Matchable),
            vec!["Boolean", "Number", "String", "Enum"]
        );
        assert_eq!(OperandPosition::Arm(2).to_string(), "match arm 2");
        assert_eq!(
            Expectation::CommonWith(TypeSpec::required(ExpressionType::Length)).to_string(),
            "Length와 같은 타입 또는 공통 타입"
        );
    }
}
