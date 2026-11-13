//! 산술 type rule table (VIII.1.2).
//!
//! 산술 연산은 아래 표에 **등록된 조합만** 허용합니다. 표에 없는 조합(예: `Length + Color`,
//! `Length × Length`, `Length + Number`, 서로 다른 단위끼리의 연산, 문자열·색상 산술)은 모두 type
//! error입니다. 암묵적 단위 변환이나 승격은 없습니다. 피연산자는 값이 반드시 있어야 하며
//! (`Required`), nullable(`Optional<T>`) 피연산자는 허용하지 않습니다.
//!
//! "단위"는 `Length`, `Percentage`, `Angle`, `Duration`이고, 무차원 scalar는 `Number`입니다.
//! 단위 `U`마다 `U + U → U`, `U − U → U`, `U × Number → U`, `Number × U → U`, `U ÷ Number → U`,
//! `U ÷ U → Number`, `U % U → U`를 등록합니다.
//!
//! 아래 표는 코드의 [`ARITHMETIC_RULES`]·[`NEGATION_RULES`]와 같아야 하며, 단위 테스트가 이
//! 문서 표를 직접 읽어 코드 표와 비교합니다.
//!
//! 이항 산술 규칙:
//!
//! | 연산 | 왼쪽 | 오른쪽 | 결과 |
//! |---|---|---|---|
//! | `+` | Number | Number | Number |
//! | `-` | Number | Number | Number |
//! | `*` | Number | Number | Number |
//! | `/` | Number | Number | Number |
//! | `%` | Number | Number | Number |
//! | `+` | Length | Length | Length |
//! | `-` | Length | Length | Length |
//! | `*` | Length | Number | Length |
//! | `*` | Number | Length | Length |
//! | `/` | Length | Number | Length |
//! | `/` | Length | Length | Number |
//! | `%` | Length | Length | Length |
//! | `+` | Percentage | Percentage | Percentage |
//! | `-` | Percentage | Percentage | Percentage |
//! | `*` | Percentage | Number | Percentage |
//! | `*` | Number | Percentage | Percentage |
//! | `/` | Percentage | Number | Percentage |
//! | `/` | Percentage | Percentage | Number |
//! | `%` | Percentage | Percentage | Percentage |
//! | `+` | Angle | Angle | Angle |
//! | `-` | Angle | Angle | Angle |
//! | `*` | Angle | Number | Angle |
//! | `*` | Number | Angle | Angle |
//! | `/` | Angle | Number | Angle |
//! | `/` | Angle | Angle | Number |
//! | `%` | Angle | Angle | Angle |
//! | `+` | Duration | Duration | Duration |
//! | `-` | Duration | Duration | Duration |
//! | `*` | Duration | Number | Duration |
//! | `*` | Number | Duration | Duration |
//! | `/` | Duration | Number | Duration |
//! | `/` | Duration | Duration | Number |
//! | `%` | Duration | Duration | Duration |
//!
//! 단항 부호 반전(`-x`) 규칙:
//!
//! | 연산 | 피연산자 | 결과 |
//! |---|---|---|
//! | `neg` | Number | Number |
//! | `neg` | Length | Length |
//! | `neg` | Percentage | Percentage |
//! | `neg` | Angle | Angle |
//! | `neg` | Duration | Duration |
//!
//! 오류 위치 규칙: 왼쪽 타입이 해당 연산의 어떤 규칙에도 왼쪽으로 등장하지 않으면(또는 nullable이면)
//! [`OperandPosition::Lhs`], 왼쪽은 가능하지만 그 왼쪽과 짝이 되는 오른쪽이 아니면(또는 nullable이면)
//! [`OperandPosition::Rhs`]를 보고합니다.

use std::fmt;

use crate::rules::{OperandPosition, TypeRuleError};
use crate::types::{ExpressionType, TypeSpec};

/// 이항 산술 연산자 (VIII.1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ArithmeticOp {
    /// 덧셈 `+`.
    Add,
    /// 뺄셈 `-`.
    Subtract,
    /// 곱셈 `×`.
    Multiply,
    /// 나눗셈 `÷`.
    Divide,
    /// 나머지 `%`.
    Remainder,
}

impl ArithmeticOp {
    /// 모든 산술 연산자(선언 순서).
    pub const ALL: [ArithmeticOp; 5] = [
        ArithmeticOp::Add,
        ArithmeticOp::Subtract,
        ArithmeticOp::Multiply,
        ArithmeticOp::Divide,
        ArithmeticOp::Remainder,
    ];

    /// 문서·진단용 기호(`+`, `-`, `*`, `/`, `%`).
    pub const fn symbol(&self) -> &'static str {
        match self {
            ArithmeticOp::Add => "+",
            ArithmeticOp::Subtract => "-",
            ArithmeticOp::Multiply => "*",
            ArithmeticOp::Divide => "/",
            ArithmeticOp::Remainder => "%",
        }
    }
}

impl fmt::Display for ArithmeticOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.symbol())
    }
}

/// 등록된 이항 산술 조합 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArithmeticRule {
    /// 연산자.
    pub op: ArithmeticOp,
    /// 왼쪽 피연산자 타입.
    pub lhs: ExpressionType,
    /// 오른쪽 피연산자 타입.
    pub rhs: ExpressionType,
    /// 결과 타입.
    pub result: ExpressionType,
}

/// 등록된 단항 부호 반전 조합 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NegationRule {
    /// 피연산자 타입.
    pub operand: ExpressionType,
    /// 결과 타입.
    pub result: ExpressionType,
}

/// 표 한 줄을 만드는 내부 helper.
const fn rule(
    op: ArithmeticOp,
    lhs: ExpressionType,
    rhs: ExpressionType,
    result: ExpressionType,
) -> ArithmeticRule {
    ArithmeticRule {
        op,
        lhs,
        rhs,
        result,
    }
}

/// 단위 `U` 하나의 7개 규칙을 만듭니다.
const fn unit_rules(unit: ExpressionType) -> [ArithmeticRule; 7] {
    use ArithmeticOp::{Add, Divide, Multiply, Remainder, Subtract};
    use ExpressionType::Number;
    [
        rule(Add, unit, unit, unit),
        rule(Subtract, unit, unit, unit),
        rule(Multiply, unit, Number, unit),
        rule(Multiply, Number, unit, unit),
        rule(Divide, unit, Number, unit),
        rule(Divide, unit, unit, Number),
        rule(Remainder, unit, unit, unit),
    ]
}

/// `[Number 5개] ++ [Length 7] ++ [Percentage 7] ++ [Angle 7] ++ [Duration 7]`를 이어 붙입니다.
const fn build_arithmetic_rules() -> [ArithmeticRule; 33] {
    use ArithmeticOp::{Add, Divide, Multiply, Remainder, Subtract};
    use ExpressionType::{Angle, Duration, Length, Number, Percentage};
    let scalar = [
        rule(Add, Number, Number, Number),
        rule(Subtract, Number, Number, Number),
        rule(Multiply, Number, Number, Number),
        rule(Divide, Number, Number, Number),
        rule(Remainder, Number, Number, Number),
    ];
    let units = [
        unit_rules(Length),
        unit_rules(Percentage),
        unit_rules(Angle),
        unit_rules(Duration),
    ];
    let mut out = [scalar[0]; 33];
    let mut index = 0;
    while index < scalar.len() {
        out[index] = scalar[index];
        index += 1;
    }
    let mut unit = 0;
    while unit < units.len() {
        let mut entry = 0;
        while entry < 7 {
            out[scalar.len() + unit * 7 + entry] = units[unit][entry];
            entry += 1;
        }
        unit += 1;
    }
    out
}

/// 등록된 이항 산술 조합 전체(모듈 문서 표와 같은 순서).
pub const ARITHMETIC_RULES: [ArithmeticRule; 33] = build_arithmetic_rules();

/// 등록된 단항 부호 반전 조합 전체.
pub const NEGATION_RULES: [NegationRule; 5] = [
    NegationRule {
        operand: ExpressionType::Number,
        result: ExpressionType::Number,
    },
    NegationRule {
        operand: ExpressionType::Length,
        result: ExpressionType::Length,
    },
    NegationRule {
        operand: ExpressionType::Percentage,
        result: ExpressionType::Percentage,
    },
    NegationRule {
        operand: ExpressionType::Angle,
        result: ExpressionType::Angle,
    },
    NegationRule {
        operand: ExpressionType::Duration,
        result: ExpressionType::Duration,
    },
];

/// 비-nullable 타입 조합의 등록 규칙을 찾습니다. 없으면 `None`(type error).
pub fn arithmetic_rule(
    op: ArithmeticOp,
    lhs: ExpressionType,
    rhs: ExpressionType,
) -> Option<&'static ArithmeticRule> {
    ARITHMETIC_RULES
        .iter()
        .find(|rule| rule.op == op && rule.lhs == lhs && rule.rhs == rhs)
}

/// 이항 산술의 결과 타입을 검사합니다(VIII.1.2).
///
/// 성공하면 항상 `Required` 결과입니다. 오류 위치는 모듈 문서의 "오류 위치 규칙"을 따릅니다.
pub fn check_arithmetic(
    op: ArithmeticOp,
    lhs: TypeSpec,
    rhs: TypeSpec,
) -> Result<TypeSpec, TypeRuleError> {
    let allowed_lhs: Vec<ExpressionType> = ARITHMETIC_RULES
        .iter()
        .filter(|rule| rule.op == op)
        .map(|rule| rule.lhs)
        .collect();
    if lhs.is_optional() || !allowed_lhs.contains(&lhs.value_type()) {
        return Err(TypeRuleError::one_of(
            OperandPosition::Lhs,
            lhs,
            allowed_lhs,
        ));
    }
    let lhs_type = lhs.value_type();
    let allowed_rhs: Vec<ExpressionType> = ARITHMETIC_RULES
        .iter()
        .filter(|rule| rule.op == op && rule.lhs == lhs_type)
        .map(|rule| rule.rhs)
        .collect();
    match arithmetic_rule(op, lhs_type, rhs.value_type()) {
        Some(rule) if !rhs.is_optional() => Ok(TypeSpec::required(rule.result)),
        _ => Err(TypeRuleError::one_of(
            OperandPosition::Rhs,
            rhs,
            allowed_rhs,
        )),
    }
}

/// 단항 부호 반전의 결과 타입을 검사합니다(VIII.1.2).
pub fn check_negation(operand: TypeSpec) -> Result<TypeSpec, TypeRuleError> {
    let found = NEGATION_RULES
        .iter()
        .find(|rule| rule.operand == operand.value_type());
    match found {
        Some(rule) if !operand.is_optional() => Ok(TypeSpec::required(rule.result)),
        _ => Err(TypeRuleError::one_of(
            OperandPosition::Operand,
            operand,
            NEGATION_RULES.iter().map(|rule| rule.operand).collect(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::rules::Expectation;
    use crate::types::EnumTypeId;

    /// 모든 비-nullable 타입(서로 다른 enum 타입 2개 포함).
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

    fn parse_type(name: &str) -> ExpressionType {
        match name {
            "Number" => ExpressionType::Number,
            "Length" => ExpressionType::Length,
            "Percentage" => ExpressionType::Percentage,
            "Angle" => ExpressionType::Angle,
            "Duration" => ExpressionType::Duration,
            other => panic!("문서 표의 알 수 없는 타입: {other}"),
        }
    }

    fn parse_op(symbol: &str) -> ArithmeticOp {
        ArithmeticOp::ALL
            .into_iter()
            .find(|op| format!("`{}`", op.symbol()) == symbol)
            .unwrap_or_else(|| panic!("문서 표의 알 수 없는 연산자: {symbol}"))
    }

    /// 모듈 문서의 표 행(`//! | ... |`)을 셀 목록으로 읽습니다. 헤더·구분선은 제외합니다.
    fn doc_table_rows() -> Vec<Vec<String>> {
        include_str!("arithmetic.rs")
            .lines()
            .filter_map(|line| line.strip_prefix("//! | `"))
            .map(|rest| {
                format!("`{rest}")
                    .trim_end_matches('|')
                    .split('|')
                    .map(|cell| cell.trim().to_string())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn documented_table_matches_code_table() {
        let mut binary = BTreeSet::new();
        let mut negation = BTreeSet::new();
        for row in doc_table_rows() {
            match row.len() {
                4 => {
                    binary.insert(ArithmeticRule {
                        op: parse_op(&row[0]),
                        lhs: parse_type(&row[1]),
                        rhs: parse_type(&row[2]),
                        result: parse_type(&row[3]),
                    });
                }
                3 => {
                    assert_eq!(row[0], "`neg`");
                    negation.insert(NegationRule {
                        operand: parse_type(&row[1]),
                        result: parse_type(&row[2]),
                    });
                }
                other => panic!("문서 표 열 수 오류: {other} ({row:?})"),
            }
        }
        let code_binary: BTreeSet<ArithmeticRule> = ARITHMETIC_RULES.into_iter().collect();
        let code_negation: BTreeSet<NegationRule> = NEGATION_RULES.into_iter().collect();
        assert_eq!(binary, code_binary, "문서 표와 코드 표가 다름");
        assert_eq!(negation, code_negation, "문서 부호 반전 표와 코드가 다름");
        // 표에 중복 행이 없음.
        assert_eq!(code_binary.len(), ARITHMETIC_RULES.len());
        assert_eq!(code_negation.len(), NEGATION_RULES.len());
    }

    #[test]
    fn every_combination_is_allowed_only_if_registered() {
        let registered: BTreeSet<(ArithmeticOp, ExpressionType, ExpressionType)> = ARITHMETIC_RULES
            .iter()
            .map(|rule| (rule.op, rule.lhs, rule.rhs))
            .collect();
        let mut allowed = 0;
        let mut rejected = 0;
        for op in ArithmeticOp::ALL {
            for lhs in universe() {
                for rhs in universe() {
                    let result = check_arithmetic(op, lhs.into(), rhs.into());
                    let is_registered = registered.contains(&(op, lhs, rhs));
                    assert_eq!(result.is_ok(), is_registered, "{lhs} {op} {rhs}");
                    match result {
                        Ok(spec) => {
                            allowed += 1;
                            let rule = arithmetic_rule(op, lhs, rhs).expect("등록됨");
                            assert_eq!(spec, TypeSpec::required(rule.result));
                        }
                        Err(_) => rejected += 1,
                    }
                    // nullable 피연산자는 등록 조합이어도 항상 거부.
                    let nullable_lhs = check_arithmetic(op, TypeSpec::optional(lhs), rhs.into());
                    let err = nullable_lhs.expect_err("nullable 왼쪽 거부");
                    assert_eq!(err.position, OperandPosition::Lhs);
                    if is_registered {
                        let nullable_rhs =
                            check_arithmetic(op, lhs.into(), TypeSpec::optional(rhs));
                        let err = nullable_rhs.expect_err("nullable 오른쪽 거부");
                        assert_eq!(err.position, OperandPosition::Rhs);
                    }
                }
            }
        }
        assert_eq!(allowed, 33);
        assert_eq!(allowed + rejected, 5 * 14 * 14);
    }

    #[test]
    fn architecture_examples() {
        // 아키텍처 §14.1 타입 규칙 예.
        let length = TypeSpec::required(ExpressionType::Length);
        let number = TypeSpec::required(ExpressionType::Number);
        let color = TypeSpec::required(ExpressionType::Color);
        assert_eq!(
            check_arithmetic(ArithmeticOp::Add, length, length),
            Ok(length)
        );
        assert_eq!(
            check_arithmetic(ArithmeticOp::Multiply, length, number),
            Ok(length)
        );
        assert_eq!(
            check_arithmetic(ArithmeticOp::Divide, length, length),
            Ok(number)
        );
        let err = check_arithmetic(ArithmeticOp::Add, length, color).expect_err("type error");
        assert_eq!(err.position, OperandPosition::Rhs);
        assert_eq!(err.found, color);
        assert_eq!(
            err.expected,
            Expectation::OneOf(vec![ExpressionType::Length])
        );
    }

    #[test]
    fn error_position_is_exact() {
        let req = TypeSpec::required;
        // 왼쪽이 이 연산의 어떤 규칙에도 왼쪽으로 나오지 않음 → Lhs.
        let err = check_arithmetic(
            ArithmeticOp::Add,
            req(ExpressionType::Color),
            req(ExpressionType::Length),
        )
        .expect_err("Color + Length");
        assert_eq!(err.position, OperandPosition::Lhs);
        assert_eq!(
            err.expected,
            Expectation::OneOf(vec![
                ExpressionType::Number,
                ExpressionType::Length,
                ExpressionType::Percentage,
                ExpressionType::Angle,
                ExpressionType::Duration,
            ])
        );
        // 서로 다른 단위 → Rhs.
        let err = check_arithmetic(
            ArithmeticOp::Add,
            req(ExpressionType::Length),
            req(ExpressionType::Percentage),
        )
        .expect_err("Length + Percentage");
        assert_eq!(err.position, OperandPosition::Rhs);
        // Length × Length는 미등록 → Rhs, 기대는 Number.
        let err = check_arithmetic(
            ArithmeticOp::Multiply,
            req(ExpressionType::Length),
            req(ExpressionType::Length),
        )
        .expect_err("Length * Length");
        assert_eq!(err.position, OperandPosition::Rhs);
        assert_eq!(
            err.expected,
            Expectation::OneOf(vec![ExpressionType::Number])
        );
        // Number ÷ Length: Number는 왼쪽으로 가능하지만 짝이 Number뿐 → Rhs.
        let err = check_arithmetic(
            ArithmeticOp::Divide,
            req(ExpressionType::Number),
            req(ExpressionType::Length),
        )
        .expect_err("Number / Length");
        assert_eq!(err.position, OperandPosition::Rhs);
        assert_eq!(
            err.expected,
            Expectation::OneOf(vec![ExpressionType::Number])
        );
        // Number × Length는 허용(교환 등록).
        assert_eq!(
            check_arithmetic(
                ArithmeticOp::Multiply,
                req(ExpressionType::Number),
                req(ExpressionType::Length)
            ),
            Ok(req(ExpressionType::Length))
        );
    }

    #[test]
    fn negation_rules_are_exhaustive() {
        let registered: BTreeSet<ExpressionType> =
            NEGATION_RULES.iter().map(|rule| rule.operand).collect();
        for operand in universe() {
            let result = check_negation(operand.into());
            assert_eq!(
                result.is_ok(),
                registered.contains(&operand),
                "neg {operand}"
            );
            if let Ok(spec) = result {
                assert_eq!(spec, TypeSpec::required(operand), "부호 반전은 타입 유지");
            } else {
                assert_eq!(result.expect_err("거부").position, OperandPosition::Operand);
            }
            let nullable = check_negation(TypeSpec::optional(operand)).expect_err("nullable");
            assert_eq!(nullable.position, OperandPosition::Operand);
        }
    }

    #[test]
    fn symbols_are_stable() {
        let symbols: Vec<&str> = ArithmeticOp::ALL.iter().map(|op| op.symbol()).collect();
        assert_eq!(symbols, vec!["+", "-", "*", "/", "%"]);
        assert_eq!(ArithmeticOp::Divide.to_string(), "/");
    }
}
