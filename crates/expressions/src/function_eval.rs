//! color·string·수치 함수 evaluator (VIII.4.4).
//!
//! [`apply_function`]은 함수 registry(VIII.1.4)의 각 함수에 대한 값 의미입니다. bytecode evaluator와 AST 기준
//! evaluator가 함께 씁니다. 인자 타입은 type 검사(VIII.2.4)로 보장되고, 값 수준 제약 위반은 typed 오류입니다.
//!
//! **수치**(`min`·`max`·`clamp`·`abs`·`round`·`floor`·`ceil`·`lerp`·`remap`): 단위 타입을 유지하며 f32 기본
//! 연산과 정확 연산(abs·round·floor·ceil — 결과가 구현과 무관하게 정해지는 연산)만 씁니다.
//!
//! - `clamp(v, lo, hi)`: `lo > hi`면 `InvalidArgument(InvertedRange)`.
//! - `round`: 0.5는 0에서 먼 쪽으로.
//! - `lerp(a, b, t) = a + (b − a) × t` (t 범위 제한 없음, 외삽 허용).
//! - `remap(v, i0, i1, o0, o1) = ((v − i0) ÷ (i1 − i0)) × (o1 − o0) + o0`. `i0 == i1`이면 `DivisionByZero`.
//!
//! **색상**(`mix`·`with_alpha`·`to_srgb`·`to_display_p3`): 아키텍처 §10.3대로 선형 색공간에서 계산합니다.
//!
//! - `mix(from, to, t)`: t는 [0, 1]. `to`를 `from`의 색공간으로 변환한 뒤 선형광·premultiplied alpha로 보간하고
//!   다시 인코딩합니다. 결과 색공간은 `from`. t = 0이면 `from`, t = 1이면 변환한 `to`를 그대로 돌려줍니다.
//! - `with_alpha(color, alpha)`: alpha는 [0, 1]. 비연관 RGB는 그대로 둡니다.
//! - `to_srgb`/`to_display_p3`: 같은 색공간이면 그대로, 다르면 transfer decode(III.4.2) → D65 primaries 선형 변환
//!   행렬 → encode. alpha는 유지하고 gamut 밖 값(음수·1 초과)도 자르지 않습니다.
//!
//! **문자열**(`concat`·`replace`·`format_number`): 결과는 [`MAX_STRING_BYTES`] 이하여야 합니다.
//!
//! - `replace(text, search, replacement)`: 겹치지 않는 모든 일치를 왼쪽부터 치환. 빈 `search`는 `EmptySearch`.
//! - `format_number(value, digits)`: digits는 0~9 정수. 고정 소수점 10진 표기(정확한 2진 값의 10진 반올림),
//!   `-0`으로 표기될 결과는 부호를 붙이지 않습니다.

use stellar_core_types::color::{Color, ColorError, ColorSpace};
use stellar_core_types::typed_value::TypedValue;

use crate::eval::{make_numeric, numeric, ArgumentIssue, EvalErrorKind, MAX_STRING_BYTES};
use crate::functions::FunctionId;

/// 선형 Display P3 → 선형 sRGB (D65). 행 우선.
///
/// renderer-core `output_color`도 같은 값을 따로 보관합니다(crate 의존 정책상 공유 불가). 두 사본은
/// `tests/integration/tests/constant_copies.rs`가 대조합니다.
pub(crate) const P3_TO_SRGB: [[f32; 3]; 3] = [
    [1.224_940_1, -0.224_940_4, 0.0],
    [-0.042_056_9, 1.042_057_1, 0.0],
    [-0.019_637_6, -0.078_636_1, 1.098_273_5],
];

/// 선형 sRGB → 선형 Display P3 (D65). 행 우선. 대조는 [`P3_TO_SRGB`]와 같습니다.
pub(crate) const SRGB_TO_P3: [[f32; 3]; 3] = [
    [0.822_462_1, 0.177_538, 0.0],
    [0.033_194_1, 0.966_805_8, 0.0],
    [0.017_082_7, 0.072_397_4, 0.910_519_9],
];

/// 함수 하나를 평가합니다(VIII.4.4). `args`는 signature 순서이며 type 검사를 통과한 값입니다.
pub fn apply_function(
    function: FunctionId,
    args: &[Option<TypedValue>],
) -> Result<Option<TypedValue>, EvalErrorKind> {
    if args.len() != function.signature().arity() {
        return Err(EvalErrorKind::MalformedProgram);
    }
    let invalid = |argument: u16, issue| EvalErrorKind::InvalidArgument {
        function,
        argument,
        issue,
    };
    match function {
        FunctionId::Min | FunctionId::Max => {
            let (value_type, a) = numeric(&args[0])?;
            let (_, b) = numeric(&args[1])?;
            let pick_a = if function == FunctionId::Min {
                a <= b
            } else {
                a >= b
            };
            make_numeric(value_type, if pick_a { a } else { b })
        }
        FunctionId::Clamp => {
            let (value_type, v) = numeric(&args[0])?;
            let (_, lo) = numeric(&args[1])?;
            let (_, hi) = numeric(&args[2])?;
            if lo > hi {
                return Err(invalid(1, ArgumentIssue::InvertedRange));
            }
            let clamped = if v < lo {
                lo
            } else if v > hi {
                hi
            } else {
                v
            };
            make_numeric(value_type, clamped)
        }
        FunctionId::Abs | FunctionId::Round | FunctionId::Floor | FunctionId::Ceil => {
            let (value_type, v) = numeric(&args[0])?;
            let result = match function {
                FunctionId::Abs => v.abs(),
                FunctionId::Round => v.round(),
                FunctionId::Floor => v.floor(),
                _ => v.ceil(),
            };
            make_numeric(value_type, result)
        }
        FunctionId::Lerp => {
            let (value_type, a) = numeric(&args[0])?;
            let (_, b) = numeric(&args[1])?;
            let (_, t) = numeric(&args[2])?;
            make_numeric(value_type, a + (b - a) * t)
        }
        FunctionId::Remap => {
            let (_, v) = numeric(&args[0])?;
            let (_, in_min) = numeric(&args[1])?;
            let (_, in_max) = numeric(&args[2])?;
            let (out_type, out_min) = numeric(&args[3])?;
            let (_, out_max) = numeric(&args[4])?;
            let span = in_max - in_min;
            if span == 0.0 {
                return Err(EvalErrorKind::DivisionByZero);
            }
            make_numeric(
                out_type,
                ((v - in_min) / span) * (out_max - out_min) + out_min,
            )
        }
        FunctionId::Mix => {
            let from = color(&args[0])?;
            let to = color(&args[1])?;
            let (_, t) = numeric(&args[2])?;
            if !(0.0..=1.0).contains(&t) {
                return Err(invalid(2, ArgumentIssue::OutOfRange));
            }
            Ok(Some(TypedValue::Color(mix(from, to, t)?)))
        }
        FunctionId::WithAlpha => {
            let base = color(&args[0])?;
            let (_, alpha) = numeric(&args[1])?;
            if !(0.0..=1.0).contains(&alpha) {
                return Err(invalid(1, ArgumentIssue::OutOfRange));
            }
            let result = Color::new(base.space(), base.r(), base.g(), base.b(), alpha)
                .map_err(color_error)?;
            Ok(Some(TypedValue::Color(result)))
        }
        FunctionId::ToSrgb | FunctionId::ToDisplayP3 => {
            let target = if function == FunctionId::ToSrgb {
                ColorSpace::Srgb
            } else {
                ColorSpace::DisplayP3
            };
            Ok(Some(TypedValue::Color(convert(color(&args[0])?, target)?)))
        }
        FunctionId::Concat => {
            let first = string(&args[0])?;
            let second = string(&args[1])?;
            let len = first
                .len()
                .checked_add(second.len())
                .ok_or(EvalErrorKind::StringTooLong)?;
            if len > MAX_STRING_BYTES {
                return Err(EvalErrorKind::StringTooLong);
            }
            let mut out = String::with_capacity(len);
            out.push_str(first);
            out.push_str(second);
            Ok(Some(TypedValue::String(out)))
        }
        FunctionId::Replace => {
            let text = string(&args[0])?;
            let search = string(&args[1])?;
            let replacement = string(&args[2])?;
            if search.is_empty() {
                return Err(invalid(1, ArgumentIssue::EmptySearch));
            }
            let count = text.matches(search).count();
            let removed = count
                .checked_mul(search.len())
                .ok_or(EvalErrorKind::StringTooLong)?;
            let added = count
                .checked_mul(replacement.len())
                .ok_or(EvalErrorKind::StringTooLong)?;
            let len = (text.len() - removed)
                .checked_add(added)
                .ok_or(EvalErrorKind::StringTooLong)?;
            if len > MAX_STRING_BYTES {
                return Err(EvalErrorKind::StringTooLong);
            }
            Ok(Some(TypedValue::String(text.replace(search, replacement))))
        }
        FunctionId::FormatNumber => {
            let (_, value) = numeric(&args[0])?;
            let (_, digits) = numeric(&args[1])?;
            if digits.fract() != 0.0 {
                return Err(invalid(1, ArgumentIssue::NotInteger));
            }
            if !(0.0..=9.0).contains(&digits) {
                return Err(invalid(1, ArgumentIssue::OutOfRange));
            }
            // 0~9로 확인했으므로 손실 없는 변환입니다.
            let digits = digits as usize;
            let mut text = format!("{value:.digits$}");
            if text.starts_with('-') && text[1..].chars().all(|c| c == '0' || c == '.') {
                text.remove(0);
            }
            Ok(Some(TypedValue::String(text)))
        }
    }
}

fn color(value: &Option<TypedValue>) -> Result<Color, EvalErrorKind> {
    match value {
        Some(TypedValue::Color(color)) => Ok(*color),
        _ => Err(EvalErrorKind::OperandTypeMismatch),
    }
}

fn string(value: &Option<TypedValue>) -> Result<&str, EvalErrorKind> {
    match value {
        Some(TypedValue::String(text)) => Ok(text),
        _ => Err(EvalErrorKind::OperandTypeMismatch),
    }
}

fn color_error(error: ColorError) -> EvalErrorKind {
    match error {
        ColorError::NonFiniteChannel => EvalErrorKind::NonFiniteResult,
        ColorError::AlphaOutOfRange | ColorError::UnknownColorSpace(_) => {
            EvalErrorKind::OperandTypeMismatch
        }
    }
}

/// 색을 `target` 색공간으로 변환합니다(같으면 그대로).
fn convert(color: Color, target: ColorSpace) -> Result<Color, EvalErrorKind> {
    if color.space() == target {
        return Ok(color);
    }
    let source = color.space();
    let linear = [
        source.decode_channel(color.r()),
        source.decode_channel(color.g()),
        source.decode_channel(color.b()),
    ];
    let matrix = match target {
        ColorSpace::Srgb => &P3_TO_SRGB,
        ColorSpace::DisplayP3 => &SRGB_TO_P3,
    };
    let mut out = [0.0f32; 3];
    for (row, slot) in matrix.iter().zip(out.iter_mut()) {
        let value = row[0] * linear[0] + row[1] * linear[1] + row[2] * linear[2];
        *slot = target.encode_channel(value);
    }
    Color::new(target, out[0], out[1], out[2], color.alpha()).map_err(color_error)
}

/// 선형광·premultiplied alpha 혼합.
fn mix(from: Color, to: Color, t: f32) -> Result<Color, EvalErrorKind> {
    let space = from.space();
    let to = convert(to, space)?;
    if t == 0.0 {
        return Ok(from);
    }
    if t == 1.0 {
        return Ok(to);
    }
    let lerp = |a: f32, b: f32| a + (b - a) * t;
    let alpha = lerp(from.alpha(), to.alpha()).clamp(0.0, 1.0);
    let channel = |a: f32, b: f32| {
        let (la, lb) = (space.decode_channel(a), space.decode_channel(b));
        let linear = if alpha > 0.0 {
            lerp(la * from.alpha(), lb * to.alpha()) / alpha
        } else {
            lerp(la, lb)
        };
        space.encode_channel(linear)
    };
    Color::new(
        space,
        channel(from.r(), to.r()),
        channel(from.g(), to.g()),
        channel(from.b(), to.b()),
        alpha,
    )
    .map_err(color_error)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use stellar_core_types::ids::StableId;
    use stellar_core_types::value::{Angle, Duration, FiniteF32, Length, Percentage};

    use super::*;
    use crate::ast::{ExprNode, ExprNodeId, ExpressionAst, ReferenceTarget};
    use crate::ast_eval::evaluate_ast;
    use crate::bytecode::Program;
    use crate::eval::EvalError;
    use crate::types::TypeSpec;

    /// 색상 transfer 허용 오차(`config/performance-budgets.yaml`의 `color.transfer_abs_tolerance`).
    const TRANSFER_TOLERANCE: f32 = 1.0e-5;
    /// 공개 참조 값(소수 4자리 반올림) 비교 허용 오차.
    const PUBLISHED_TOLERANCE: f32 = 1.0e-3;

    /// f64 기준 구현: sRGB transfer(IEC 61966-2-1)와 D65 primaries 행렬로 sRGB↔Display P3 변환.
    fn reference_convert(rgb: [f32; 3], to_p3: bool) -> [f32; 3] {
        let decode = |v: f64| {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        let encode = |v: f64| {
            if v <= 0.003_130_8 {
                v * 12.92
            } else {
                1.055 * v.powf(1.0 / 2.4) - 0.055
            }
        };
        let matrix: [[f64; 3]; 3] = if to_p3 {
            [
                [0.822_462_1, 0.177_538_0, 0.0],
                [0.033_194_1, 0.966_805_8, 0.0],
                [0.017_082_7, 0.072_397_4, 0.910_519_9],
            ]
        } else {
            [
                [1.224_940_1, -0.224_940_4, 0.0],
                [-0.042_056_9, 1.042_057_1, 0.0],
                [-0.019_637_6, -0.078_636_1, 1.098_273_5],
            ]
        };
        let linear = rgb.map(|v| decode(f64::from(v)));
        let mut out = [0.0f32; 3];
        for (row, slot) in matrix.iter().zip(out.iter_mut()) {
            *slot = encode(row[0] * linear[0] + row[1] * linear[1] + row[2] * linear[2]) as f32;
        }
        out
    }

    fn id(n: u8) -> ExprNodeId {
        ExprNodeId::new(StableId::from_bytes([n; 16]))
    }

    fn num(v: f32) -> TypedValue {
        TypedValue::Number(FiniteF32::new(v).expect("유한"))
    }

    fn len(v: f32) -> TypedValue {
        TypedValue::Length(Length::new(v).expect("유한"))
    }

    fn pct(v: f32) -> TypedValue {
        TypedValue::Percentage(Percentage::new(v).expect("유한"))
    }

    fn deg(v: f32) -> TypedValue {
        TypedValue::Angle(Angle::new(v).expect("유한"))
    }

    fn secs(v: f32) -> TypedValue {
        TypedValue::Duration(Duration::new(v).expect("유한"))
    }

    fn text(s: &str) -> TypedValue {
        TypedValue::String(s.to_string())
    }

    fn rgba(space: ColorSpace, r: f32, g: f32, b: f32, a: f32) -> TypedValue {
        TypedValue::Color(Color::new(space, r, g, b, a).expect("색"))
    }

    fn srgb(r: f32, g: f32, b: f32, a: f32) -> TypedValue {
        rgba(ColorSpace::Srgb, r, g, b, a)
    }

    fn p3(r: f32, g: f32, b: f32, a: f32) -> TypedValue {
        rgba(ColorSpace::DisplayP3, r, g, b, a)
    }

    /// 함수 호출 literal AST를 VM·AST evaluator로 평가하고 의미 함수 결과와 같음을 확인합니다.
    fn call(
        function: FunctionId,
        args: &[TypedValue],
    ) -> Result<Option<TypedValue>, EvalErrorKind> {
        let direct = apply_function(
            function,
            &args.iter().cloned().map(Some).collect::<Vec<_>>(),
        );
        let arg_ids: Vec<ExprNodeId> = (0..args.len()).map(|i| id(10 + i as u8)).collect();
        let mut nodes = vec![ExprNode::call(id(1), function, &arg_ids)];
        for (arg, &arg_id) in args.iter().zip(&arg_ids) {
            nodes.push(ExprNode::literal(arg_id, arg.clone()));
        }
        let ast = ExpressionAst::new(id(1), nodes).expect("유효");
        let types: BTreeMap<ReferenceTarget, TypeSpec> = BTreeMap::new();
        let program = Program::compile(&ast, &types).expect("컴파일");
        let values = BTreeMap::new();
        let by_bytecode = program.evaluate(&values);
        assert_eq!(
            by_bytecode,
            evaluate_ast(&ast, &types, &values),
            "{function}: AST/bytecode"
        );
        let expected = direct
            .clone()
            .map_err(|kind| EvalError::new(Some(id(1)), kind));
        assert_eq!(by_bytecode, expected, "{function}: 의미 함수/bytecode");
        direct
    }

    fn channels(value: &Option<TypedValue>) -> (ColorSpace, [f32; 4]) {
        match value {
            Some(TypedValue::Color(c)) => (c.space(), [c.r(), c.g(), c.b(), c.alpha()]),
            other => panic!("색 기대: {other:?}"),
        }
    }

    fn assert_color_near(
        actual: &Result<Option<TypedValue>, EvalErrorKind>,
        space: ColorSpace,
        expected: [f32; 4],
        tolerance: f32,
    ) {
        let value = actual.as_ref().expect("평가 성공");
        let (actual_space, actual) = channels(value);
        assert_eq!(actual_space, space);
        for (index, (a, e)) in actual.iter().zip(expected).enumerate() {
            assert!(
                (a - e).abs() <= tolerance,
                "채널 {index}: {a} vs {e} (허용 {tolerance})"
            );
        }
    }

    #[test]
    fn numeric_reference_values() {
        let cases: Vec<(FunctionId, Vec<TypedValue>, TypedValue)> = vec![
            (FunctionId::Min, vec![len(3.0), len(-2.0)], len(-2.0)),
            (FunctionId::Max, vec![deg(90.0), deg(45.0)], deg(90.0)),
            (FunctionId::Min, vec![num(1.0), num(1.0)], num(1.0)),
            (
                FunctionId::Clamp,
                vec![len(150.0), len(0.0), len(100.0)],
                len(100.0),
            ),
            (
                FunctionId::Clamp,
                vec![len(-5.0), len(0.0), len(100.0)],
                len(0.0),
            ),
            (
                FunctionId::Clamp,
                vec![pct(42.0), pct(0.0), pct(100.0)],
                pct(42.0),
            ),
            (
                FunctionId::Clamp,
                vec![num(3.0), num(3.0), num(3.0)],
                num(3.0),
            ),
            (FunctionId::Abs, vec![secs(-0.25)], secs(0.25)),
            (FunctionId::Round, vec![num(2.5)], num(3.0)),
            (FunctionId::Round, vec![num(-2.5)], num(-3.0)),
            (FunctionId::Round, vec![num(0.499_999_97)], num(0.0)),
            (FunctionId::Round, vec![len(-0.4)], len(0.0)),
            (FunctionId::Floor, vec![len(-1.5)], len(-2.0)),
            (FunctionId::Ceil, vec![len(-1.5)], len(-1.0)),
            (FunctionId::Ceil, vec![deg(0.1)], deg(1.0)),
            (
                FunctionId::Lerp,
                vec![len(0.0), len(10.0), num(0.25)],
                len(2.5),
            ),
            (
                FunctionId::Lerp,
                vec![deg(0.0), deg(90.0), num(2.0)],
                deg(180.0),
            ),
            (
                FunctionId::Lerp,
                vec![num(4.0), num(4.0), num(0.7)],
                num(4.0),
            ),
            (
                FunctionId::Remap,
                vec![num(5.0), num(0.0), num(10.0), len(100.0), len(200.0)],
                len(150.0),
            ),
            (
                FunctionId::Remap,
                vec![secs(0.5), secs(0.0), secs(1.0), deg(0.0), deg(360.0)],
                deg(180.0),
            ),
            (
                FunctionId::Remap,
                vec![pct(25.0), pct(100.0), pct(0.0), num(0.0), num(1.0)],
                num(0.75),
            ),
        ];
        for (function, args, expected) in cases {
            assert_eq!(
                call(function, &args),
                Ok(Some(expected)),
                "{function}{args:?}"
            );
        }
    }

    #[test]
    fn numeric_argument_errors() {
        assert_eq!(
            call(FunctionId::Clamp, &[len(1.0), len(10.0), len(0.0)]),
            Err(EvalErrorKind::InvalidArgument {
                function: FunctionId::Clamp,
                argument: 1,
                issue: ArgumentIssue::InvertedRange
            })
        );
        assert_eq!(
            call(
                FunctionId::Remap,
                &[num(1.0), num(2.0), num(2.0), num(0.0), num(1.0)]
            ),
            Err(EvalErrorKind::DivisionByZero)
        );
        assert_eq!(
            call(FunctionId::Lerp, &[num(-f32::MAX), num(f32::MAX), num(1.0)]),
            Err(EvalErrorKind::NonFiniteResult)
        );
    }

    #[test]
    fn color_reference_values() {
        // with_alpha: RGB 유지, alpha 교체.
        assert_eq!(
            call(FunctionId::WithAlpha, &[srgb(0.2, 0.4, 0.6, 1.0), num(0.5)]),
            Ok(Some(srgb(0.2, 0.4, 0.6, 0.5)))
        );
        // mix 끝점은 정확히 입력(t=0 → from, t=1 → to를 from 색공간으로).
        let a = srgb(0.1, 0.2, 0.3, 1.0);
        let b = srgb(0.9, 0.8, 0.7, 0.5);
        assert_eq!(
            call(FunctionId::Mix, &[a.clone(), b.clone(), num(0.0)]),
            Ok(Some(a.clone()))
        );
        assert_eq!(
            call(FunctionId::Mix, &[a.clone(), b.clone(), num(1.0)]),
            Ok(Some(b))
        );
        // 검정·흰색 중간은 선형광 0.5의 sRGB 인코딩 값(≈ 0.735357, 지각 중간 0.5가 아님).
        let half = (1.055 * 0.5f64.powf(1.0 / 2.4) - 0.055) as f32;
        assert_color_near(
            &call(
                FunctionId::Mix,
                &[srgb(0.0, 0.0, 0.0, 1.0), srgb(1.0, 1.0, 1.0, 1.0), num(0.5)],
            ),
            ColorSpace::Srgb,
            [half, half, half, 1.0],
            TRANSFER_TOLERANCE,
        );
        // premultiplied: 빨강과 투명 검정의 중간은 어두워지지 않은 반투명 빨강.
        assert_color_near(
            &call(
                FunctionId::Mix,
                &[srgb(1.0, 0.0, 0.0, 1.0), srgb(0.0, 0.0, 0.0, 0.0), num(0.5)],
            ),
            ColorSpace::Srgb,
            [1.0, 0.0, 0.0, 0.5],
            TRANSFER_TOLERANCE,
        );
        // 서로 다른 색공간: 결과는 from 색공간.
        let mixed = call(
            FunctionId::Mix,
            &[p3(1.0, 1.0, 1.0, 1.0), srgb(1.0, 1.0, 1.0, 1.0), num(0.5)],
        );
        assert_color_near(
            &mixed,
            ColorSpace::DisplayP3,
            [1.0, 1.0, 1.0, 1.0],
            TRANSFER_TOLERANCE,
        );
    }

    #[test]
    fn color_space_conversion_reference_values() {
        // f64 독립 기준 구현(같은 정의: sRGB transfer + D65 행렬)과 transfer 허용 오차 안에서 일치.
        let reference_cases: [([f32; 3], bool); 6] = [
            ([1.0, 0.0, 0.0], true),
            ([0.0, 1.0, 0.0], true),
            ([0.0, 0.0, 1.0], true),
            ([0.25, 0.5, 0.75], true),
            ([1.0, 0.0, 0.0], false),
            ([0.2, 0.9, 0.4], false),
        ];
        for (rgb, to_p3) in reference_cases {
            let expected = reference_convert(rgb, to_p3);
            let (function, input, space) = if to_p3 {
                (
                    FunctionId::ToDisplayP3,
                    srgb(rgb[0], rgb[1], rgb[2], 0.25),
                    ColorSpace::DisplayP3,
                )
            } else {
                (
                    FunctionId::ToSrgb,
                    p3(rgb[0], rgb[1], rgb[2], 0.25),
                    ColorSpace::Srgb,
                )
            };
            assert_color_near(
                &call(function, &[input]),
                space,
                [expected[0], expected[1], expected[2], 0.25],
                TRANSFER_TOLERANCE,
            );
        }
        // 공개 근사값(CSS Color 4 변환 예, 소수 3자리 수준)과도 일치. gamut 밖 값은 자르지 않음.
        assert_color_near(
            &call(FunctionId::ToDisplayP3, &[srgb(1.0, 0.0, 0.0, 1.0)]),
            ColorSpace::DisplayP3,
            [0.9175, 0.2003, 0.1386, 1.0],
            PUBLISHED_TOLERANCE,
        );
        assert_color_near(
            &call(FunctionId::ToSrgb, &[p3(1.0, 0.0, 0.0, 1.0)]),
            ColorSpace::Srgb,
            [1.0931, -0.5434, -0.2538, 1.0],
            PUBLISHED_TOLERANCE,
        );
        // 무채색은 색공간과 무관하게 거의 같음, 같은 색공간 변환은 그대로.
        assert_color_near(
            &call(FunctionId::ToDisplayP3, &[srgb(0.5, 0.5, 0.5, 1.0)]),
            ColorSpace::DisplayP3,
            [0.5, 0.5, 0.5, 1.0],
            TRANSFER_TOLERANCE,
        );
        let same = srgb(0.3, 0.6, 0.9, 0.8);
        assert_eq!(
            call(FunctionId::ToSrgb, std::slice::from_ref(&same)),
            Ok(Some(same))
        );
        // sRGB → P3 → sRGB 왕복은 transfer 허용 오차 안.
        for step in 0..=10 {
            let v = step as f32 / 10.0;
            let original = srgb(v, 1.0 - v, (v * 7.0) % 1.0, 1.0);
            let there =
                call(FunctionId::ToDisplayP3, std::slice::from_ref(&original)).expect("변환");
            let back = call(FunctionId::ToSrgb, &[there.expect("값")]);
            let (_, expected) = channels(&Some(original));
            assert_color_near(&back, ColorSpace::Srgb, expected, TRANSFER_TOLERANCE);
        }
    }

    #[test]
    fn color_argument_errors() {
        for (function, args, argument) in [
            (
                FunctionId::Mix,
                vec![srgb(0.0, 0.0, 0.0, 1.0), srgb(1.0, 1.0, 1.0, 1.0), num(1.5)],
                2,
            ),
            (
                FunctionId::Mix,
                vec![
                    srgb(0.0, 0.0, 0.0, 1.0),
                    srgb(1.0, 1.0, 1.0, 1.0),
                    num(-0.1),
                ],
                2,
            ),
            (
                FunctionId::WithAlpha,
                vec![srgb(0.0, 0.0, 0.0, 1.0), num(2.0)],
                1,
            ),
        ] {
            assert_eq!(
                call(function, &args),
                Err(EvalErrorKind::InvalidArgument {
                    function,
                    argument,
                    issue: ArgumentIssue::OutOfRange
                })
            );
        }
    }

    #[test]
    fn string_reference_values() {
        assert_eq!(
            call(FunctionId::Concat, &[text("가나"), text("다")]),
            Ok(Some(text("가나다")))
        );
        assert_eq!(
            call(FunctionId::Concat, &[text(""), text("")]),
            Ok(Some(text("")))
        );
        assert_eq!(
            call(
                FunctionId::Replace,
                &[text("a-b-c"), text("-"), text(" / ")]
            ),
            Ok(Some(text("a / b / c")))
        );
        assert_eq!(
            call(FunctionId::Replace, &[text("aaaa"), text("aa"), text("b")]),
            Ok(Some(text("bb"))),
            "겹치지 않는 일치를 왼쪽부터"
        );
        assert_eq!(
            call(FunctionId::Replace, &[text("없음"), text("x"), text("y")]),
            Ok(Some(text("없음")))
        );
        let formats: Vec<(f32, f32, &str)> = vec![
            (1.5, 2.0, "1.50"),
            (2.345, 2.0, "2.35"),
            (2.5, 0.0, "2"),
            (3.5, 0.0, "4"),
            (-0.001, 2.0, "0.00"),
            (-1.25, 1.0, "-1.2"),
            (1.0e10, 0.0, "10000000000"),
            (0.1, 9.0, "0.100000001"),
        ];
        for (value, digits, expected) in formats {
            assert_eq!(
                call(FunctionId::FormatNumber, &[num(value), num(digits)]),
                Ok(Some(text(expected))),
                "format_number({value}, {digits})"
            );
        }
    }

    #[test]
    fn string_argument_errors_and_limits() {
        let invalid = |function, argument, issue| {
            Err(EvalErrorKind::InvalidArgument {
                function,
                argument,
                issue,
            })
        };
        assert_eq!(
            call(FunctionId::Replace, &[text("abc"), text(""), text("x")]),
            invalid(FunctionId::Replace, 1, ArgumentIssue::EmptySearch)
        );
        assert_eq!(
            call(FunctionId::FormatNumber, &[num(1.0), num(1.5)]),
            invalid(FunctionId::FormatNumber, 1, ArgumentIssue::NotInteger)
        );
        assert_eq!(
            call(FunctionId::FormatNumber, &[num(1.0), num(10.0)]),
            invalid(FunctionId::FormatNumber, 1, ArgumentIssue::OutOfRange)
        );
        assert_eq!(
            call(FunctionId::FormatNumber, &[num(1.0), num(-1.0)]),
            invalid(FunctionId::FormatNumber, 1, ArgumentIssue::OutOfRange)
        );
        // 길이 한도: 한도 정확히는 허용, 1 byte 초과는 거부.
        let half = "a".repeat(MAX_STRING_BYTES / 2);
        assert!(call(FunctionId::Concat, &[text(&half), text(&half)]).is_ok());
        assert_eq!(
            call(
                FunctionId::Concat,
                &[text(&half), text(&format!("{half}a"))]
            ),
            Err(EvalErrorKind::StringTooLong)
        );
        let many = "x".repeat(1_000);
        assert_eq!(
            call(
                FunctionId::Replace,
                &[text(&many), text("x"), text(&"y".repeat(2_000))]
            ),
            Err(EvalErrorKind::StringTooLong)
        );
    }

    #[test]
    fn shared_concat_chain_hits_string_limit_instead_of_exploding() {
        // s0 = "ab", s(i+1) = concat(s(i), s(i)) 를 공유 node로 40단 → 2^41 bytes 대신 한도 오류.
        let mut nodes = vec![ExprNode::literal(id(100), text("ab"))];
        for level in 0..40u8 {
            nodes.push(ExprNode::call(
                id(level),
                FunctionId::Concat,
                &[
                    if level == 0 { id(100) } else { id(level - 1) },
                    if level == 0 { id(100) } else { id(level - 1) },
                ],
            ));
        }
        let ast = ExpressionAst::new(id(39), nodes).expect("유효");
        let types: BTreeMap<ReferenceTarget, TypeSpec> = BTreeMap::new();
        let program = Program::compile(&ast, &types).expect("컴파일");
        let result = program.evaluate(&BTreeMap::new());
        assert!(matches!(
            result,
            Err(EvalError {
                kind: EvalErrorKind::StringTooLong,
                ..
            })
        ));
        assert_eq!(result, evaluate_ast(&ast, &types, &BTreeMap::new()));
    }
}
