//! 비교·논리·조건식 type rule (VIII.1.3).
//!
//! | 대상 | 규칙 | 결과 |
//! |---|---|---|
//! | `==`, `!=` | 왼쪽은 [`TypeClass::Equatable`](nullable 허용), 오른쪽은 왼쪽과 같은 값 타입(nullable 허용) | `Boolean` |
//! | `<`, `<=`, `>`, `>=` | 양쪽 모두 비-nullable이고 같은 [`TypeClass::Ordered`] 타입 | `Boolean` |
//! | `and`, `or` | 양쪽 모두 비-nullable `Boolean` | `Boolean` |
//! | `not` | 비-nullable `Boolean` | `Boolean` |
//! | `if c then a else b` | `c`는 비-nullable `Boolean`, `a`·`b`는 [`common_type`]이 있어야 함 | 공통 타입 |
//! | `match s { p0 => a0, … } else d` | `s`는 비-nullable [`TypeClass::Matchable`], 모든 `pᵢ`는 `s`와 같은 타입·서로 다른 값, arm 1개 이상, `a0…`·`d`는 공통 타입 | 공통 타입 |
//!
//! **명시적 공통 타입**([`common_type`])은 아래 두 경우만 등록합니다. 수치 승격·단위 변환 같은
//! 암묵 공통 타입은 없습니다.
//!
//! - `T`와 `T` → `T` (nullable 여부까지 같음)
//! - `T`와 `Optional<T>`(순서 무관) → `Optional<T>`
//!
//! 오류 위치: 비교·논리는 산술(VIII.1.2)과 같이 `Lhs`/`Rhs`, `if`는 조건이 틀리면 `Condition`,
//! branch가 맞지 않으면 뒤 branch(`Else`), `match`는 대상(`Scrutinee`), 틀린 pattern
//! (`Pattern(i)`), 앞선 arm들과 공통 타입이 없는 첫 arm(`Arm(i)`) 또는 기본 branch(`Default`)를
//! 보고합니다.

use std::fmt;

use stellar_core_types::typed_value::TypedValue;

use crate::rules::{Expectation, OperandPosition, TypeClass, TypeRuleError};
use crate::types::{ExpressionType, TypeSpec};

/// 비교 연산자 (VIII.1.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ComparisonOp {
    /// `==`.
    Equal,
    /// `!=`.
    NotEqual,
    /// `<`.
    Less,
    /// `<=`.
    LessEqual,
    /// `>`.
    Greater,
    /// `>=`.
    GreaterEqual,
}

impl ComparisonOp {
    /// 모든 비교 연산자(선언 순서).
    pub const ALL: [ComparisonOp; 6] = [
        ComparisonOp::Equal,
        ComparisonOp::NotEqual,
        ComparisonOp::Less,
        ComparisonOp::LessEqual,
        ComparisonOp::Greater,
        ComparisonOp::GreaterEqual,
    ];

    /// 동등 비교(`==`, `!=`)이면 `true`, 순서 비교이면 `false`.
    pub const fn is_equality(&self) -> bool {
        matches!(self, ComparisonOp::Equal | ComparisonOp::NotEqual)
    }

    /// 진단용 기호.
    pub const fn symbol(&self) -> &'static str {
        match self {
            ComparisonOp::Equal => "==",
            ComparisonOp::NotEqual => "!=",
            ComparisonOp::Less => "<",
            ComparisonOp::LessEqual => "<=",
            ComparisonOp::Greater => ">",
            ComparisonOp::GreaterEqual => ">=",
        }
    }
}

impl fmt::Display for ComparisonOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.symbol())
    }
}

/// 이항 논리 연산자 (VIII.1.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LogicalOp {
    /// 논리곱 `and`.
    And,
    /// 논리합 `or`.
    Or,
}

impl LogicalOp {
    /// 모든 논리 연산자.
    pub const ALL: [LogicalOp; 2] = [LogicalOp::And, LogicalOp::Or];

    /// 진단용 기호.
    pub const fn symbol(&self) -> &'static str {
        match self {
            LogicalOp::And => "and",
            LogicalOp::Or => "or",
        }
    }
}

impl fmt::Display for LogicalOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.symbol())
    }
}

const BOOLEAN: TypeSpec = TypeSpec::required(ExpressionType::Boolean);

/// 명시적 공통 타입 (VIII.1.3). 모듈 문서의 두 경우가 아니면 `None`입니다.
pub fn common_type(a: TypeSpec, b: TypeSpec) -> Option<TypeSpec> {
    if a == b {
        Some(a)
    } else if a.value_type() == b.value_type() {
        Some(a.to_optional())
    } else {
        None
    }
}

/// 비교식의 결과 타입을 검사합니다.
pub fn check_comparison(
    op: ComparisonOp,
    lhs: TypeSpec,
    rhs: TypeSpec,
) -> Result<TypeSpec, TypeRuleError> {
    if op.is_equality() {
        if !TypeClass::Equatable.contains(lhs.value_type()) {
            return Err(TypeRuleError::new(
                OperandPosition::Lhs,
                lhs,
                Expectation::Class(TypeClass::Equatable),
            ));
        }
        if rhs.value_type() != lhs.value_type() {
            return Err(TypeRuleError::one_of(
                OperandPosition::Rhs,
                rhs,
                vec![lhs.value_type()],
            ));
        }
        return Ok(BOOLEAN);
    }
    if lhs.is_optional() || !TypeClass::Ordered.contains(lhs.value_type()) {
        return Err(TypeRuleError::new(
            OperandPosition::Lhs,
            lhs,
            Expectation::Class(TypeClass::Ordered),
        ));
    }
    if rhs != TypeSpec::required(lhs.value_type()) {
        return Err(TypeRuleError::one_of(
            OperandPosition::Rhs,
            rhs,
            vec![lhs.value_type()],
        ));
    }
    Ok(BOOLEAN)
}

/// 이항 논리식의 결과 타입을 검사합니다. 연산자 종류와 무관하게 같은 규칙입니다.
pub fn check_logical(
    _op: LogicalOp,
    lhs: TypeSpec,
    rhs: TypeSpec,
) -> Result<TypeSpec, TypeRuleError> {
    require_boolean(OperandPosition::Lhs, lhs)?;
    require_boolean(OperandPosition::Rhs, rhs)?;
    Ok(BOOLEAN)
}

/// `not`의 결과 타입을 검사합니다.
pub fn check_not(operand: TypeSpec) -> Result<TypeSpec, TypeRuleError> {
    require_boolean(OperandPosition::Operand, operand)?;
    Ok(BOOLEAN)
}

/// `if`의 결과 타입을 검사합니다. 두 branch는 같거나 명시적 공통 타입이어야 합니다.
pub fn check_if(
    condition: TypeSpec,
    then_branch: TypeSpec,
    else_branch: TypeSpec,
) -> Result<TypeSpec, TypeRuleError> {
    require_boolean(OperandPosition::Condition, condition)?;
    common_type(then_branch, else_branch).ok_or(TypeRuleError::new(
        OperandPosition::Else,
        else_branch,
        Expectation::CommonWith(then_branch),
    ))
}

/// `match` arm 하나: pattern 값과 arm 값의 타입.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchArmType<'a> {
    /// arm pattern(리터럴 값).
    pub pattern: &'a TypedValue,
    /// arm 값의 타입.
    pub value: TypeSpec,
}

/// `match`의 결과 타입을 검사합니다.
pub fn check_match(
    scrutinee: TypeSpec,
    arms: &[MatchArmType<'_>],
    default: TypeSpec,
) -> Result<TypeSpec, TypeRuleError> {
    if scrutinee.is_optional() || !TypeClass::Matchable.contains(scrutinee.value_type()) {
        return Err(TypeRuleError::new(
            OperandPosition::Scrutinee,
            scrutinee,
            Expectation::Class(TypeClass::Matchable),
        ));
    }
    if arms.is_empty() {
        return Err(TypeRuleError::new(
            OperandPosition::Scrutinee,
            scrutinee,
            Expectation::AtLeastOneArm,
        ));
    }
    let scrutinee_type = scrutinee.value_type();
    for (index, arm) in arms.iter().enumerate() {
        let position = OperandPosition::Pattern(arm_index(index));
        let pattern_type = ExpressionType::of(arm.pattern);
        if pattern_type != scrutinee_type {
            return Err(TypeRuleError::one_of(
                position,
                TypeSpec::required(pattern_type),
                vec![scrutinee_type],
            ));
        }
        if arms[..index]
            .iter()
            .any(|earlier| earlier.pattern == arm.pattern)
        {
            return Err(TypeRuleError::new(
                position,
                TypeSpec::required(pattern_type),
                Expectation::DistinctPattern,
            ));
        }
    }
    let mut result = arms[0].value;
    for (index, arm) in arms.iter().enumerate().skip(1) {
        result = common_type(result, arm.value).ok_or(TypeRuleError::new(
            OperandPosition::Arm(arm_index(index)),
            arm.value,
            Expectation::CommonWith(result),
        ))?;
    }
    common_type(result, default).ok_or(TypeRuleError::new(
        OperandPosition::Default,
        default,
        Expectation::CommonWith(result),
    ))
}

/// arm index를 위치 번호로 바꿉니다. `u16`을 넘는 arm 수는 AST 단계에서 거부되므로 포화합니다.
fn arm_index(index: usize) -> u16 {
    u16::try_from(index).unwrap_or(u16::MAX)
}

fn require_boolean(position: OperandPosition, found: TypeSpec) -> Result<(), TypeRuleError> {
    if found == BOOLEAN {
        Ok(())
    } else {
        Err(TypeRuleError::one_of(
            position,
            found,
            vec![ExpressionType::Boolean],
        ))
    }
}

#[cfg(test)]
mod tests {
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::typed_value::EnumValue;
    use stellar_core_types::value::{FiniteF32, Length};

    use super::*;
    use crate::types::EnumTypeId;

    fn req(value_type: ExpressionType) -> TypeSpec {
        TypeSpec::required(value_type)
    }

    fn opt(value_type: ExpressionType) -> TypeSpec {
        TypeSpec::optional(value_type)
    }

    fn universe() -> Vec<ExpressionType> {
        vec![
            ExpressionType::Boolean,
            ExpressionType::Number,
            ExpressionType::String,
            ExpressionType::Enum(EnumTypeId::new(1)),
            ExpressionType::Enum(EnumTypeId::new(2)),
            ExpressionType::Length,
            ExpressionType::Percentage,
            ExpressionType::Angle,
            ExpressionType::Duration,
            ExpressionType::Color,
            ExpressionType::Gradient,
            ExpressionType::Shadow,
            ExpressionType::Typography,
            ExpressionType::ResourceRef,
        ]
    }

    fn all_specs() -> Vec<TypeSpec> {
        universe()
            .into_iter()
            .flat_map(|t| [req(t), opt(t)])
            .collect()
    }

    fn number(value: f32) -> TypedValue {
        TypedValue::Number(FiniteF32::new(value).expect("유한"))
    }

    fn error_at(result: Result<TypeSpec, TypeRuleError>) -> OperandPosition {
        result.expect_err("type error 기대").position
    }

    #[test]
    fn comparison_rules_are_exhaustive() {
        for op in ComparisonOp::ALL {
            for lhs in all_specs() {
                for rhs in all_specs() {
                    let result = check_comparison(op, lhs, rhs);
                    let same = lhs.value_type() == rhs.value_type();
                    let allowed = if op.is_equality() {
                        same && TypeClass::Equatable.contains(lhs.value_type())
                    } else {
                        same && !lhs.is_optional()
                            && !rhs.is_optional()
                            && TypeClass::Ordered.contains(lhs.value_type())
                    };
                    assert_eq!(result.is_ok(), allowed, "{lhs} {op} {rhs}");
                    if let Ok(spec) = result {
                        assert_eq!(spec, BOOLEAN);
                    }
                }
            }
        }
    }

    #[test]
    fn logical_and_not_rules_are_exhaustive() {
        for op in LogicalOp::ALL {
            for lhs in all_specs() {
                for rhs in all_specs() {
                    let allowed = lhs == BOOLEAN && rhs == BOOLEAN;
                    assert_eq!(check_logical(op, lhs, rhs).is_ok(), allowed);
                }
            }
        }
        for operand in all_specs() {
            assert_eq!(check_not(operand).is_ok(), operand == BOOLEAN);
        }
    }

    #[test]
    fn common_type_table() {
        for a in all_specs() {
            for b in all_specs() {
                let expected = if a == b {
                    Some(a)
                } else if a.value_type() == b.value_type() {
                    Some(opt(a.value_type()))
                } else {
                    None
                };
                assert_eq!(common_type(a, b), expected, "{a} ~ {b}");
                // 교환 법칙.
                assert_eq!(common_type(a, b), common_type(b, a));
            }
        }
        // 서로 다른 enum 타입·수치/단위 사이에는 공통 타입이 없음(암묵 승격 없음).
        let e1 = ExpressionType::Enum(EnumTypeId::new(1));
        let e2 = ExpressionType::Enum(EnumTypeId::new(2));
        assert_eq!(common_type(req(e1), req(e2)), None);
        assert_eq!(
            common_type(req(ExpressionType::Number), req(ExpressionType::Length)),
            None
        );
    }

    #[test]
    fn if_branches_must_match_or_have_common_type() {
        let length = req(ExpressionType::Length);
        assert_eq!(check_if(BOOLEAN, length, length), Ok(length));
        assert_eq!(
            check_if(BOOLEAN, length, opt(ExpressionType::Length)),
            Ok(opt(ExpressionType::Length))
        );
        assert_eq!(
            check_if(
                BOOLEAN,
                opt(ExpressionType::Color),
                req(ExpressionType::Color)
            ),
            Ok(opt(ExpressionType::Color))
        );
    }

    #[test]
    fn mismatch_fixtures_report_exact_position() {
        let length = req(ExpressionType::Length);
        let color = req(ExpressionType::Color);
        let number = req(ExpressionType::Number);
        let string = req(ExpressionType::String);
        let e1 = ExpressionType::Enum(EnumTypeId::new(1));

        // (설명, 실제 결과, 기대 위치)
        let fixtures: Vec<(&str, Result<TypeSpec, TypeRuleError>, OperandPosition)> = vec![
            (
                "if: 조건이 Number",
                check_if(number, length, length),
                OperandPosition::Condition,
            ),
            (
                "if: 조건이 Optional<Boolean>",
                check_if(opt(ExpressionType::Boolean), length, length),
                OperandPosition::Condition,
            ),
            (
                "if: Length vs Color branch",
                check_if(BOOLEAN, length, color),
                OperandPosition::Else,
            ),
            (
                "if: Number vs Length branch(암묵 승격 없음)",
                check_if(BOOLEAN, number, length),
                OperandPosition::Else,
            ),
            (
                "비교: Color < Color",
                check_comparison(ComparisonOp::Less, color, color),
                OperandPosition::Lhs,
            ),
            (
                "비교: Length < Percentage",
                check_comparison(ComparisonOp::Less, length, req(ExpressionType::Percentage)),
                OperandPosition::Rhs,
            ),
            (
                "비교: Optional<Length> < Length",
                check_comparison(ComparisonOp::Less, opt(ExpressionType::Length), length),
                OperandPosition::Lhs,
            ),
            (
                "비교: Length >= Optional<Length>",
                check_comparison(
                    ComparisonOp::GreaterEqual,
                    length,
                    opt(ExpressionType::Length),
                ),
                OperandPosition::Rhs,
            ),
            (
                "비교: Gradient == Gradient",
                check_comparison(
                    ComparisonOp::Equal,
                    req(ExpressionType::Gradient),
                    req(ExpressionType::Gradient),
                ),
                OperandPosition::Lhs,
            ),
            (
                "비교: Enum(1) == Enum(2)",
                check_comparison(
                    ComparisonOp::Equal,
                    req(e1),
                    req(ExpressionType::Enum(EnumTypeId::new(2))),
                ),
                OperandPosition::Rhs,
            ),
            (
                "논리: Number and Boolean",
                check_logical(LogicalOp::And, number, BOOLEAN),
                OperandPosition::Lhs,
            ),
            (
                "논리: Boolean or Optional<Boolean>",
                check_logical(LogicalOp::Or, BOOLEAN, opt(ExpressionType::Boolean)),
                OperandPosition::Rhs,
            ),
            ("not: String", check_not(string), OperandPosition::Operand),
        ];
        for (name, result, expected) in fixtures {
            assert_eq!(error_at(result), expected, "{name}");
        }
    }

    #[test]
    fn equality_accepts_nullable_operands() {
        let length = req(ExpressionType::Length);
        let maybe = opt(ExpressionType::Length);
        for op in [ComparisonOp::Equal, ComparisonOp::NotEqual] {
            assert_eq!(check_comparison(op, maybe, length), Ok(BOOLEAN));
            assert_eq!(check_comparison(op, length, maybe), Ok(BOOLEAN));
            assert_eq!(check_comparison(op, maybe, maybe), Ok(BOOLEAN));
        }
    }

    #[test]
    fn match_rules_and_error_positions() {
        let e1 = ExpressionType::Enum(EnumTypeId::new(1));
        let a = TypedValue::Enum(EnumValue::new(1, 0));
        let b = TypedValue::Enum(EnumValue::new(1, 1));
        let foreign = TypedValue::Enum(EnumValue::new(2, 0));
        let length = req(ExpressionType::Length);
        let color = req(ExpressionType::Color);
        fn arm(pattern: &TypedValue, value: TypeSpec) -> MatchArmType<'_> {
            MatchArmType { pattern, value }
        }

        // 정상: 결과는 arm/기본 branch의 공통 타입.
        assert_eq!(
            check_match(req(e1), &[arm(&a, length), arm(&b, length)], length),
            Ok(length)
        );
        assert_eq!(
            check_match(
                req(e1),
                &[arm(&a, length), arm(&b, opt(ExpressionType::Length))],
                length
            ),
            Ok(opt(ExpressionType::Length))
        );

        let one = number(1.0);
        let two = number(2.0);
        let three = number(3.0);
        let text = TypedValue::String("a".to_string());
        let color_value =
            TypedValue::Color(Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0).expect("유효한 색"));
        let fixtures: Vec<(&str, Result<TypeSpec, TypeRuleError>, OperandPosition)> = vec![
            (
                "대상이 Color",
                check_match(color, &[arm(&color_value, length)], length),
                OperandPosition::Scrutinee,
            ),
            (
                "대상이 Optional<Number>",
                check_match(opt(ExpressionType::Number), &[arm(&one, length)], length),
                OperandPosition::Scrutinee,
            ),
            (
                "arm 없음",
                check_match(req(ExpressionType::Number), &[], length),
                OperandPosition::Scrutinee,
            ),
            (
                "pattern 1이 다른 enum 타입",
                check_match(req(e1), &[arm(&a, length), arm(&foreign, length)], length),
                OperandPosition::Pattern(1),
            ),
            (
                "pattern 2가 앞선 pattern과 중복",
                check_match(
                    req(ExpressionType::Number),
                    &[arm(&one, length), arm(&two, length), arm(&one, length)],
                    length,
                ),
                OperandPosition::Pattern(2),
            ),
            (
                "pattern 0이 String(대상 Number)",
                check_match(req(ExpressionType::Number), &[arm(&text, length)], length),
                OperandPosition::Pattern(0),
            ),
            (
                "arm 2가 Color",
                check_match(
                    req(ExpressionType::Number),
                    &[arm(&one, length), arm(&two, length), arm(&three, color)],
                    length,
                ),
                OperandPosition::Arm(2),
            ),
            (
                "기본 branch가 Color",
                check_match(req(ExpressionType::Number), &[arm(&one, length)], color),
                OperandPosition::Default,
            ),
        ];
        for (name, result, expected) in fixtures {
            let err = result.expect_err(name);
            assert_eq!(err.position, expected, "{name}");
            if name == "pattern 2가 앞선 pattern과 중복" {
                assert_eq!(err.expected, Expectation::DistinctPattern);
            }
            if name == "arm 2가 Color" {
                assert_eq!(err.expected, Expectation::CommonWith(length));
                assert_eq!(err.found, color);
            }
        }
        // 단위 값 pattern은 Matchable이 아님(Length 대상 거부).
        let len_value = TypedValue::Length(Length::new(1.0).expect("유한"));
        assert_eq!(
            error_at(check_match(length, &[arm(&len_value, length)], length)),
            OperandPosition::Scrutinee
        );
    }
}
