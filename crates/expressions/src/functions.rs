//! 표현식 함수 signature registry (VIII.1.4).
//!
//! 표현식에서 부를 수 있는 함수는 이 registry의 **닫힌 목록**뿐입니다. 사용자 정의 함수·외부 함수는
//! 없으며(아키텍처 §14.1 "등록되지 않은 외부 함수" 금지), 이름이나 numeric ID가 목록에 없으면
//! [`FunctionLookupError`]로 거부합니다. numeric ID([`FunctionId::tag`])는 직렬화 포맷의 일부이므로
//! 한 번 정한 값을 바꾸거나 재사용하지 않습니다.
//!
//! | ID | 이름 | 인자 | 결과 |
//! |---|---|---|---|
//! | `0x0001` | `min` | `(a: T, b: T)` | `T` |
//! | `0x0002` | `max` | `(a: T, b: T)` | `T` |
//! | `0x0003` | `clamp` | `(value: T, min: T, max: T)` | `T` |
//! | `0x0004` | `abs` | `(value: T)` | `T` |
//! | `0x0005` | `round` | `(value: T)` | `T` |
//! | `0x0006` | `floor` | `(value: T)` | `T` |
//! | `0x0007` | `ceil` | `(value: T)` | `T` |
//! | `0x0008` | `lerp` | `(from: T, to: T, t: Number)` | `T` |
//! | `0x0009` | `remap` | `(value: T, in_min: T, in_max: T, out_min: U, out_max: U)` | `U` |
//! | `0x0101` | `mix` | `(from: Color, to: Color, t: Number)` | `Color` |
//! | `0x0102` | `with_alpha` | `(color: Color, alpha: Number)` | `Color` |
//! | `0x0103` | `to_srgb` | `(color: Color)` | `Color` |
//! | `0x0104` | `to_display_p3` | `(color: Color)` | `Color` |
//! | `0x0201` | `concat` | `(first: String, second: String)` | `String` |
//! | `0x0202` | `replace` | `(text: String, search: String, replacement: String)` | `String` |
//! | `0x0203` | `format_number` | `(value: Number, fraction_digits: Number)` | `String` |
//!
//! `T`·`U`는 수치 타입 변수입니다([`TypeConstraint::Numeric`]: Number·Length·Percentage·Angle·
//! Duration). 같은 변수는 한 호출 안에서 같은 타입이어야 합니다. 모든 인자는 비-nullable이어야
//! 합니다.
//!
//! 검사 순서와 오류 위치: 인자 수가 다르면 [`CallTypeError::Arity`]. 그 외에는 인자를 왼쪽부터
//! 검사해 첫 위반 인자를 [`OperandPosition::Argument`]로 보고합니다. 타입 변수는 처음 만난 인자의
//! 타입에 묶이고, 뒤 인자는 그 타입이어야 합니다.

use std::fmt;

use crate::rules::{OperandPosition, TypeRuleError};
use crate::types::{ExpressionType, TypeSpec};

/// 타입 변수 제약.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TypeConstraint {
    /// Number·Length·Percentage·Angle·Duration.
    Numeric,
}

impl TypeConstraint {
    /// 제약을 만족하는 비-nullable 타입 목록(오름차순).
    pub fn members(&self) -> Vec<ExpressionType> {
        match self {
            TypeConstraint::Numeric => vec![
                ExpressionType::Number,
                ExpressionType::Length,
                ExpressionType::Percentage,
                ExpressionType::Angle,
                ExpressionType::Duration,
            ],
        }
    }

    /// 타입이 제약을 만족하는지 여부.
    pub fn contains(&self, value_type: ExpressionType) -> bool {
        self.members().contains(&value_type)
    }
}

/// 인자·결과 타입: 고정 타입 또는 타입 변수.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ParamType {
    /// 고정 비-nullable 타입.
    Exact(ExpressionType),
    /// 타입 변수(`index`는 signature 안에서의 변수 번호).
    Var {
        /// 변수 번호(0 = `T`, 1 = `U`).
        index: u8,
        /// 변수 제약.
        constraint: TypeConstraint,
    },
}

/// 함수 인자 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Param {
    /// GUI·진단용 인자 이름.
    pub name: &'static str,
    /// 인자 타입.
    pub param_type: ParamType,
}

/// 함수 signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FunctionSignature {
    /// 인자 목록(순서 고정).
    pub params: &'static [Param],
    /// 결과 타입.
    pub result: ParamType,
}

impl FunctionSignature {
    /// 인자 수.
    pub const fn arity(&self) -> usize {
        self.params.len()
    }
}

/// 함수 분류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FunctionCategory {
    /// 수치 함수.
    Numeric,
    /// 색상 함수.
    Color,
    /// 문자열 함수.
    String,
}

/// 등록된 표현식 함수 (VIII.1.4). 닫힌 목록입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FunctionId {
    /// `min(a, b)`.
    Min,
    /// `max(a, b)`.
    Max,
    /// `clamp(value, min, max)`.
    Clamp,
    /// `abs(value)`.
    Abs,
    /// `round(value)` — 0.5는 0에서 먼 쪽으로.
    Round,
    /// `floor(value)`.
    Floor,
    /// `ceil(value)`.
    Ceil,
    /// `lerp(from, to, t)`.
    Lerp,
    /// `remap(value, in_min, in_max, out_min, out_max)`.
    Remap,
    /// `mix(from, to, t)` — 색상 혼합.
    Mix,
    /// `with_alpha(color, alpha)` — 투명도 지정.
    WithAlpha,
    /// `to_srgb(color)` — sRGB로 색공간 변환.
    ToSrgb,
    /// `to_display_p3(color)` — Display P3로 색공간 변환.
    ToDisplayP3,
    /// `concat(first, second)` — 문자열 결합.
    Concat,
    /// `replace(text, search, replacement)` — 문자열 치환.
    Replace,
    /// `format_number(value, fraction_digits)` — 고정 소수 자릿수 포맷.
    FormatNumber,
}

const T: ParamType = ParamType::Var {
    index: 0,
    constraint: TypeConstraint::Numeric,
};
const U: ParamType = ParamType::Var {
    index: 1,
    constraint: TypeConstraint::Numeric,
};
const NUMBER: ParamType = ParamType::Exact(ExpressionType::Number);
const COLOR: ParamType = ParamType::Exact(ExpressionType::Color);
const STRING: ParamType = ParamType::Exact(ExpressionType::String);

const fn param(name: &'static str, param_type: ParamType) -> Param {
    Param { name, param_type }
}

const PARAMS_A_B: [Param; 2] = [param("a", T), param("b", T)];
const PARAMS_CLAMP: [Param; 3] = [param("value", T), param("min", T), param("max", T)];
const PARAMS_VALUE: [Param; 1] = [param("value", T)];
const PARAMS_LERP: [Param; 3] = [param("from", T), param("to", T), param("t", NUMBER)];
const PARAMS_REMAP: [Param; 5] = [
    param("value", T),
    param("in_min", T),
    param("in_max", T),
    param("out_min", U),
    param("out_max", U),
];
const PARAMS_MIX: [Param; 3] = [param("from", COLOR), param("to", COLOR), param("t", NUMBER)];
const PARAMS_WITH_ALPHA: [Param; 2] = [param("color", COLOR), param("alpha", NUMBER)];
const PARAMS_COLOR: [Param; 1] = [param("color", COLOR)];
const PARAMS_CONCAT: [Param; 2] = [param("first", STRING), param("second", STRING)];
const PARAMS_REPLACE: [Param; 3] = [
    param("text", STRING),
    param("search", STRING),
    param("replacement", STRING),
];
const PARAMS_FORMAT_NUMBER: [Param; 2] = [param("value", NUMBER), param("fraction_digits", NUMBER)];

impl FunctionId {
    /// 등록된 함수 전체(numeric ID 오름차순).
    pub const ALL: [FunctionId; 16] = [
        FunctionId::Min,
        FunctionId::Max,
        FunctionId::Clamp,
        FunctionId::Abs,
        FunctionId::Round,
        FunctionId::Floor,
        FunctionId::Ceil,
        FunctionId::Lerp,
        FunctionId::Remap,
        FunctionId::Mix,
        FunctionId::WithAlpha,
        FunctionId::ToSrgb,
        FunctionId::ToDisplayP3,
        FunctionId::Concat,
        FunctionId::Replace,
        FunctionId::FormatNumber,
    ];

    /// 안정 numeric ID. 직렬화 포맷 계약입니다.
    pub const fn tag(&self) -> u16 {
        match self {
            FunctionId::Min => 0x0001,
            FunctionId::Max => 0x0002,
            FunctionId::Clamp => 0x0003,
            FunctionId::Abs => 0x0004,
            FunctionId::Round => 0x0005,
            FunctionId::Floor => 0x0006,
            FunctionId::Ceil => 0x0007,
            FunctionId::Lerp => 0x0008,
            FunctionId::Remap => 0x0009,
            FunctionId::Mix => 0x0101,
            FunctionId::WithAlpha => 0x0102,
            FunctionId::ToSrgb => 0x0103,
            FunctionId::ToDisplayP3 => 0x0104,
            FunctionId::Concat => 0x0201,
            FunctionId::Replace => 0x0202,
            FunctionId::FormatNumber => 0x0203,
        }
    }

    /// numeric ID로 함수를 찾습니다. 미등록 ID는 거부합니다.
    pub fn from_tag(tag: u16) -> Result<FunctionId, FunctionLookupError> {
        FunctionId::ALL
            .into_iter()
            .find(|function| function.tag() == tag)
            .ok_or(FunctionLookupError::UnknownTag(tag))
    }

    /// 표현식·GUI에서 쓰는 이름(대소문자 구분).
    pub const fn name(&self) -> &'static str {
        match self {
            FunctionId::Min => "min",
            FunctionId::Max => "max",
            FunctionId::Clamp => "clamp",
            FunctionId::Abs => "abs",
            FunctionId::Round => "round",
            FunctionId::Floor => "floor",
            FunctionId::Ceil => "ceil",
            FunctionId::Lerp => "lerp",
            FunctionId::Remap => "remap",
            FunctionId::Mix => "mix",
            FunctionId::WithAlpha => "with_alpha",
            FunctionId::ToSrgb => "to_srgb",
            FunctionId::ToDisplayP3 => "to_display_p3",
            FunctionId::Concat => "concat",
            FunctionId::Replace => "replace",
            FunctionId::FormatNumber => "format_number",
        }
    }

    /// 이름으로 함수를 찾습니다. 미등록 이름은 거부합니다.
    pub fn from_name(name: &str) -> Result<FunctionId, FunctionLookupError> {
        FunctionId::ALL
            .into_iter()
            .find(|function| function.name() == name)
            .ok_or_else(|| FunctionLookupError::UnknownName(name.to_string()))
    }

    /// 함수 분류.
    pub const fn category(&self) -> FunctionCategory {
        match self {
            FunctionId::Min
            | FunctionId::Max
            | FunctionId::Clamp
            | FunctionId::Abs
            | FunctionId::Round
            | FunctionId::Floor
            | FunctionId::Ceil
            | FunctionId::Lerp
            | FunctionId::Remap => FunctionCategory::Numeric,
            FunctionId::Mix
            | FunctionId::WithAlpha
            | FunctionId::ToSrgb
            | FunctionId::ToDisplayP3 => FunctionCategory::Color,
            FunctionId::Concat | FunctionId::Replace | FunctionId::FormatNumber => {
                FunctionCategory::String
            }
        }
    }

    /// 함수 signature.
    pub const fn signature(&self) -> FunctionSignature {
        let (params, result): (&'static [Param], ParamType) = match self {
            FunctionId::Min | FunctionId::Max => (&PARAMS_A_B, T),
            FunctionId::Clamp => (&PARAMS_CLAMP, T),
            FunctionId::Abs | FunctionId::Round | FunctionId::Floor | FunctionId::Ceil => {
                (&PARAMS_VALUE, T)
            }
            FunctionId::Lerp => (&PARAMS_LERP, T),
            FunctionId::Remap => (&PARAMS_REMAP, U),
            FunctionId::Mix => (&PARAMS_MIX, COLOR),
            FunctionId::WithAlpha => (&PARAMS_WITH_ALPHA, COLOR),
            FunctionId::ToSrgb | FunctionId::ToDisplayP3 => (&PARAMS_COLOR, COLOR),
            FunctionId::Concat => (&PARAMS_CONCAT, STRING),
            FunctionId::Replace => (&PARAMS_REPLACE, STRING),
            FunctionId::FormatNumber => (&PARAMS_FORMAT_NUMBER, STRING),
        };
        FunctionSignature { params, result }
    }
}

impl fmt::Display for FunctionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// 함수 조회 실패: registry에 없는 함수.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FunctionLookupError {
    /// 등록되지 않은 이름.
    UnknownName(String),
    /// 등록되지 않은 numeric ID.
    UnknownTag(u16),
}

impl fmt::Display for FunctionLookupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FunctionLookupError::UnknownName(name) => {
                write!(f, "등록되지 않은 함수 이름: {name:?}")
            }
            FunctionLookupError::UnknownTag(tag) => {
                write!(f, "등록되지 않은 함수 ID: 0x{tag:04X}")
            }
        }
    }
}

impl std::error::Error for FunctionLookupError {}

/// 함수 호출 type 오류 (VIII.1.4).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CallTypeError {
    /// 인자 수 불일치.
    Arity {
        /// 호출한 함수.
        function: FunctionId,
        /// signature 인자 수.
        expected: usize,
        /// 실제 인자 수.
        found: usize,
    },
    /// 인자 타입 위반. `error.position`은 [`OperandPosition::Argument`]입니다.
    Argument {
        /// 호출한 함수.
        function: FunctionId,
        /// 위반 내용.
        error: TypeRuleError,
    },
}

impl fmt::Display for CallTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallTypeError::Arity {
                function,
                expected,
                found,
            } => write!(
                f,
                "{function}: 인자 {expected}개가 필요하지만 {found}개가 주어졌습니다"
            ),
            CallTypeError::Argument { function, error } => write!(f, "{function}: {error}"),
        }
    }
}

impl std::error::Error for CallTypeError {}

/// 함수 호출의 결과 타입을 검사합니다(VIII.1.4).
pub fn check_call(function: FunctionId, args: &[TypeSpec]) -> Result<TypeSpec, CallTypeError> {
    let signature = function.signature();
    if args.len() != signature.arity() {
        return Err(CallTypeError::Arity {
            function,
            expected: signature.arity(),
            found: args.len(),
        });
    }
    // 변수 번호 0·1의 바인딩.
    let mut bound: [Option<ExpressionType>; 2] = [None, None];
    for (index, (param, arg)) in signature.params.iter().zip(args).enumerate() {
        let position = OperandPosition::Argument(u16::try_from(index).unwrap_or(u16::MAX));
        let allowed = match param.param_type {
            ParamType::Exact(value_type) => vec![value_type],
            ParamType::Var {
                index: var,
                constraint,
            } => match bound.get(usize::from(var)).copied().flatten() {
                Some(value_type) => vec![value_type],
                None => constraint.members(),
            },
        };
        if arg.is_optional() || !allowed.contains(&arg.value_type()) {
            return Err(CallTypeError::Argument {
                function,
                error: TypeRuleError::one_of(position, *arg, allowed),
            });
        }
        if let ParamType::Var { index: var, .. } = param.param_type {
            if let Some(slot) = bound.get_mut(usize::from(var)) {
                *slot = Some(arg.value_type());
            }
        }
    }
    let result = match signature.result {
        ParamType::Exact(value_type) => value_type,
        // 결과 변수는 항상 인자에 등장하므로 위 루프에서 묶였습니다.
        ParamType::Var { index, .. } => bound
            .get(usize::from(index))
            .copied()
            .flatten()
            .unwrap_or(ExpressionType::Number),
    };
    Ok(TypeSpec::required(result))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::rules::Expectation;

    fn req(value_type: ExpressionType) -> TypeSpec {
        TypeSpec::required(value_type)
    }

    const LENGTH: TypeSpec = TypeSpec::required(ExpressionType::Length);
    const NUM: TypeSpec = TypeSpec::required(ExpressionType::Number);
    const COL: TypeSpec = TypeSpec::required(ExpressionType::Color);
    const STR: TypeSpec = TypeSpec::required(ExpressionType::String);

    fn argument_error(result: Result<TypeSpec, CallTypeError>) -> TypeRuleError {
        match result {
            Err(CallTypeError::Argument { error, .. }) => error,
            other => panic!("인자 type 오류 기대: {other:?}"),
        }
    }

    #[test]
    fn registry_is_closed_unique_and_round_trips() {
        let tags: BTreeSet<u16> = FunctionId::ALL.iter().map(|f| f.tag()).collect();
        let names: BTreeSet<&str> = FunctionId::ALL.iter().map(|f| f.name()).collect();
        assert_eq!(tags.len(), FunctionId::ALL.len());
        assert_eq!(names.len(), FunctionId::ALL.len());
        for function in FunctionId::ALL {
            assert_eq!(FunctionId::from_tag(function.tag()), Ok(function));
            assert_eq!(FunctionId::from_name(function.name()), Ok(function));
            // 결과 변수는 반드시 인자에 등장(묶이지 않은 결과 변수 없음).
            let signature = function.signature();
            if let ParamType::Var { index, .. } = signature.result {
                assert!(signature.params.iter().any(
                    |p| matches!(p.param_type, ParamType::Var { index: i, .. } if i == index)
                ));
            }
        }
        // ALL은 tag 오름차순.
        let ordered: Vec<u16> = FunctionId::ALL.iter().map(|f| f.tag()).collect();
        let mut sorted = ordered.clone();
        sorted.sort_unstable();
        assert_eq!(ordered, sorted);
    }

    #[test]
    fn numeric_ids_are_stable_golden() {
        let golden: Vec<(u16, &str)> = FunctionId::ALL
            .iter()
            .map(|f| (f.tag(), f.name()))
            .collect();
        assert_eq!(
            golden,
            vec![
                (0x0001, "min"),
                (0x0002, "max"),
                (0x0003, "clamp"),
                (0x0004, "abs"),
                (0x0005, "round"),
                (0x0006, "floor"),
                (0x0007, "ceil"),
                (0x0008, "lerp"),
                (0x0009, "remap"),
                (0x0101, "mix"),
                (0x0102, "with_alpha"),
                (0x0103, "to_srgb"),
                (0x0104, "to_display_p3"),
                (0x0201, "concat"),
                (0x0202, "replace"),
                (0x0203, "format_number"),
            ]
        );
    }

    #[test]
    fn unknown_function_fixtures_fail() {
        for name in [
            "",
            "sqrt",
            "pow",
            "MIN",
            " min",
            "min ",
            "now",
            "random",
            "read_file",
            "fetch",
            "eval",
            "exec",
            "git",
            "loop",
        ] {
            assert_eq!(
                FunctionId::from_name(name),
                Err(FunctionLookupError::UnknownName(name.to_string())),
                "{name:?}"
            );
        }
        for tag in [
            0x0000, 0x000A, 0x0100, 0x0105, 0x0200, 0x0204, 0x0F00, 0xFFFF,
        ] {
            assert_eq!(
                FunctionId::from_tag(tag),
                Err(FunctionLookupError::UnknownTag(tag))
            );
        }
        assert_eq!(
            FunctionLookupError::UnknownTag(0x0F00).to_string(),
            "등록되지 않은 함수 ID: 0x0F00"
        );
    }

    #[test]
    fn arity_fixtures_fail() {
        for function in FunctionId::ALL {
            let arity = function.signature().arity();
            let too_many = vec![NUM; arity + 1];
            assert_eq!(
                check_call(function, &too_many),
                Err(CallTypeError::Arity {
                    function,
                    expected: arity,
                    found: arity + 1
                })
            );
            let too_few = vec![NUM; arity - 1];
            assert_eq!(
                check_call(function, &too_few),
                Err(CallTypeError::Arity {
                    function,
                    expected: arity,
                    found: arity - 1
                })
            );
        }
        assert_eq!(
            check_call(FunctionId::Clamp, &[LENGTH, LENGTH])
                .expect_err("arity")
                .to_string(),
            "clamp: 인자 3개가 필요하지만 2개가 주어졌습니다"
        );
    }

    #[test]
    fn valid_calls_infer_result_type() {
        let angle = req(ExpressionType::Angle);
        let pct = req(ExpressionType::Percentage);
        let dur = req(ExpressionType::Duration);
        let cases: Vec<(FunctionId, Vec<TypeSpec>, TypeSpec)> = vec![
            (FunctionId::Min, vec![LENGTH, LENGTH], LENGTH),
            (FunctionId::Max, vec![NUM, NUM], NUM),
            (FunctionId::Clamp, vec![pct, pct, pct], pct),
            (FunctionId::Abs, vec![angle], angle),
            (FunctionId::Round, vec![LENGTH], LENGTH),
            (FunctionId::Floor, vec![dur], dur),
            (FunctionId::Ceil, vec![NUM], NUM),
            (FunctionId::Lerp, vec![angle, angle, NUM], angle),
            (
                FunctionId::Remap,
                vec![NUM, NUM, NUM, LENGTH, LENGTH],
                LENGTH,
            ),
            (FunctionId::Remap, vec![dur, dur, dur, pct, pct], pct),
            (FunctionId::Mix, vec![COL, COL, NUM], COL),
            (FunctionId::WithAlpha, vec![COL, NUM], COL),
            (FunctionId::ToSrgb, vec![COL], COL),
            (FunctionId::ToDisplayP3, vec![COL], COL),
            (FunctionId::Concat, vec![STR, STR], STR),
            (FunctionId::Replace, vec![STR, STR, STR], STR),
            (FunctionId::FormatNumber, vec![NUM, NUM], STR),
        ];
        for (function, args, expected) in cases {
            assert_eq!(check_call(function, &args), Ok(expected), "{function}");
        }
    }

    #[test]
    fn argument_type_fixtures_report_exact_index() {
        let numeric = TypeConstraint::Numeric.members();
        // (함수, 인자, 기대 위치 index, 기대 허용 목록)
        let fixtures: Vec<(FunctionId, Vec<TypeSpec>, u16, Vec<ExpressionType>)> = vec![
            (FunctionId::Abs, vec![STR], 0, numeric.clone()),
            (
                FunctionId::Min,
                vec![LENGTH, req(ExpressionType::Percentage)],
                1,
                vec![ExpressionType::Length],
            ),
            (
                FunctionId::Clamp,
                vec![LENGTH, LENGTH, req(ExpressionType::Angle)],
                2,
                vec![ExpressionType::Length],
            ),
            (
                FunctionId::Lerp,
                vec![LENGTH, LENGTH, LENGTH],
                2,
                vec![ExpressionType::Number],
            ),
            (
                FunctionId::Remap,
                vec![LENGTH, LENGTH, LENGTH, req(ExpressionType::Angle), LENGTH],
                4,
                vec![ExpressionType::Angle],
            ),
            (
                FunctionId::Remap,
                vec![LENGTH, NUM, LENGTH, NUM, NUM],
                1,
                vec![ExpressionType::Length],
            ),
            (
                FunctionId::Mix,
                vec![COL, LENGTH, NUM],
                1,
                vec![ExpressionType::Color],
            ),
            (
                FunctionId::WithAlpha,
                vec![COL, req(ExpressionType::Percentage)],
                1,
                vec![ExpressionType::Number],
            ),
            (
                FunctionId::ToSrgb,
                vec![req(ExpressionType::Gradient)],
                0,
                vec![ExpressionType::Color],
            ),
            (
                FunctionId::Concat,
                vec![STR, NUM],
                1,
                vec![ExpressionType::String],
            ),
            (
                FunctionId::Replace,
                vec![STR, STR, req(ExpressionType::Boolean)],
                2,
                vec![ExpressionType::String],
            ),
            (
                FunctionId::FormatNumber,
                vec![LENGTH, NUM],
                0,
                vec![ExpressionType::Number],
            ),
            // nullable 인자는 타입이 맞아도 거부.
            (
                FunctionId::Max,
                vec![LENGTH, TypeSpec::optional(ExpressionType::Length)],
                1,
                vec![ExpressionType::Length],
            ),
            (
                FunctionId::Floor,
                vec![TypeSpec::optional(ExpressionType::Number)],
                0,
                numeric.clone(),
            ),
        ];
        for (function, args, index, allowed) in fixtures {
            let error = argument_error(check_call(function, &args));
            assert_eq!(
                error.position,
                OperandPosition::Argument(index),
                "{function}"
            );
            assert_eq!(error.expected, Expectation::OneOf(allowed), "{function}");
            assert_eq!(error.found, args[usize::from(index)], "{function}");
        }
    }

    #[test]
    fn categories_and_param_names() {
        let numeric = FunctionId::ALL
            .iter()
            .filter(|f| f.category() == FunctionCategory::Numeric)
            .count();
        let color = FunctionId::ALL
            .iter()
            .filter(|f| f.category() == FunctionCategory::Color)
            .count();
        assert_eq!(
            (numeric, color, FunctionId::ALL.len() - numeric - color),
            (9, 4, 3)
        );
        let names: Vec<&str> = FunctionId::Remap
            .signature()
            .params
            .iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(
            names,
            vec!["value", "in_min", "in_max", "out_min", "out_max"]
        );
    }
}
