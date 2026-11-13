//! 표현식 evaluator 결정론 golden corpus (VIII.4.6).
//!
//! 같은 입력 환경과 같은 bytecode는 bit 단위로 같은 `TypedValue`를 반환해야 합니다. 각 case를
//!
//! 1. 같은 program으로 반복 평가,
//! 2. 같은 AST를 다시 컴파일,
//! 3. canonical AST bytes(VIII.2.5)에서 복원해 컴파일,
//! 4. 참조 값 제공 순서를 바꾼 입력 환경,
//! 5. AST 기준 evaluator
//!
//! 로 평가해 결과 bytes(`TypedValue::to_bytes`)가 모두 같고, 고정 golden 파일
//! `tests/fixtures/evaluator_determinism_v1.golden`과 같음을 확인합니다.
//!
//! case는 `ieee`(IEEE-754 기본 연산·정확 연산·정수 연산만 사용)와 `libm`(색 transfer의 `powf` 사용)으로
//! 나뉩니다. 분류는 AST의 함수 호출로 다시 계산해 golden 표기와 같아야 합니다.

use std::collections::BTreeMap;

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::reference::TokenRef;
use stellar_core_types::typed_value::{EnumValue, TypedValue};
use stellar_core_types::value::{Angle, FiniteF32, Length, Percentage};
use stellar_expressions::ast::{
    EnvironmentKey, ExprNode, ExprNodeId, ExpressionAst, NodePayload, ReferenceTarget,
};
use stellar_expressions::ast_eval::evaluate_ast;
use stellar_expressions::bytecode::Program;
use stellar_expressions::eval::{EvalError, ReferenceValues};
use stellar_expressions::functions::FunctionId;
use stellar_expressions::ops::{BinaryOp, UnaryOp};
use stellar_expressions::types::{EnumTypeId, ExpressionType, TypeSpec};

const GOLDEN: &str = include_str!("fixtures/evaluator_determinism_v1.golden");

fn id(n: u8) -> ExprNodeId {
    ExprNodeId::new(StableId::from_bytes([n; 16]))
}

fn num(v: f32) -> TypedValue {
    TypedValue::Number(FiniteF32::new(v).expect("유한"))
}

fn len(v: f32) -> TypedValue {
    TypedValue::Length(Length::new(v).expect("유한"))
}

fn deg(v: f32) -> TypedValue {
    TypedValue::Angle(Angle::new(v).expect("유한"))
}

fn pct(v: f32) -> TypedValue {
    TypedValue::Percentage(Percentage::new(v).expect("유한"))
}

fn text(s: &str) -> TypedValue {
    TypedValue::String(s.to_string())
}

fn color(space: ColorSpace, r: f32, g: f32, b: f32, a: f32) -> TypedValue {
    TypedValue::Color(Color::new(space, r, g, b, a).expect("색"))
}

fn token(n: u8) -> ReferenceTarget {
    ReferenceTarget::Token(TokenRef::new(StableId::from_bytes([0xA0 + n; 16])))
}

fn env(n: u32) -> ReferenceTarget {
    ReferenceTarget::Environment(EnvironmentKey::new(n))
}

/// 입력 환경: 참조 대상의 타입과 값(제공 순서는 시험마다 바꿉니다).
struct Case {
    name: &'static str,
    root: ExprNodeId,
    nodes: Vec<ExprNode>,
    inputs: Vec<(ReferenceTarget, TypeSpec, Option<TypedValue>)>,
}

/// 선형 탐색 값 제공자. 제공 순서가 결과에 영향을 주지 않음을 보이기 위해 순서를 섞어 씁니다.
struct OrderedValues(Vec<(ReferenceTarget, Option<TypedValue>)>);

impl ReferenceValues for OrderedValues {
    fn reference_value(&self, target: &ReferenceTarget) -> Option<Option<TypedValue>> {
        self.0
            .iter()
            .find(|(candidate, _)| candidate == target)
            .map(|(_, value)| value.clone())
    }
}

fn corpus() -> Vec<Case> {
    let srgb = ColorSpace::Srgb;
    let p3 = ColorSpace::DisplayP3;
    vec![
        Case {
            // (token1 * 2 + 4px) / (token1 - 1px)
            name: "arith_length_ratio",
            root: id(1),
            nodes: vec![
                ExprNode::binary(id(1), BinaryOp::Divide, id(2), id(3)),
                ExprNode::binary(id(2), BinaryOp::Add, id(4), id(5)),
                ExprNode::binary(id(3), BinaryOp::Subtract, id(6), id(7)),
                ExprNode::binary(id(4), BinaryOp::Multiply, id(6), id(8)),
                ExprNode::literal(id(5), len(4.0)),
                ExprNode::reference(id(6), token(1)),
                ExprNode::literal(id(7), len(1.0)),
                ExprNode::literal(id(8), num(2.0)),
            ],
            inputs: vec![(
                token(1),
                TypeSpec::required(ExpressionType::Length),
                Some(len(5.3)),
            )],
        },
        Case {
            name: "angle_remainder_negative",
            root: id(1),
            nodes: vec![
                ExprNode::binary(id(1), BinaryOp::Remainder, id(2), id(3)),
                ExprNode::unary(id(2), UnaryOp::Negate, id(4)),
                ExprNode::literal(id(3), deg(360.0)),
                ExprNode::literal(id(4), deg(450.75)),
            ],
            inputs: vec![],
        },
        Case {
            name: "division_by_zero",
            root: id(1),
            nodes: vec![
                ExprNode::binary(id(1), BinaryOp::Divide, id(2), id(3)),
                ExprNode::literal(id(2), num(1.0)),
                ExprNode::literal(id(3), num(0.0)),
            ],
            inputs: vec![],
        },
        Case {
            name: "overflow",
            root: id(1),
            nodes: vec![
                ExprNode::binary(id(1), BinaryOp::Multiply, id(2), id(2)),
                ExprNode::literal(id(2), num(1.0e20)),
            ],
            inputs: vec![],
        },
        Case {
            // round(remap(token1, 0, 10, 0%, 100%)) 를 clamp·min·max·abs·floor·ceil·lerp로 감쌈.
            name: "numeric_functions",
            root: id(1),
            nodes: vec![
                ExprNode::call(id(1), FunctionId::Lerp, &[id(2), id(3), id(4)]),
                ExprNode::call(id(2), FunctionId::Round, &[id(5)]),
                ExprNode::call(id(3), FunctionId::Clamp, &[id(6), id(7), id(8)]),
                ExprNode::literal(id(4), num(0.375)),
                ExprNode::call(
                    id(5),
                    FunctionId::Remap,
                    &[id(9), id(10), id(11), id(7), id(8)],
                ),
                ExprNode::call(id(6), FunctionId::Max, &[id(12), id(13)]),
                ExprNode::literal(id(7), pct(0.0)),
                ExprNode::literal(id(8), pct(100.0)),
                ExprNode::reference(id(9), token(1)),
                ExprNode::literal(id(10), num(0.0)),
                ExprNode::literal(id(11), num(10.0)),
                ExprNode::call(id(12), FunctionId::Abs, &[id(14)]),
                ExprNode::call(id(13), FunctionId::Floor, &[id(15)]),
                ExprNode::call(id(14), FunctionId::Min, &[id(16), id(17)]),
                ExprNode::call(id(15), FunctionId::Ceil, &[id(18)]),
                ExprNode::literal(id(16), pct(-33.3)),
                ExprNode::literal(id(17), pct(12.5)),
                ExprNode::literal(id(18), pct(41.01)),
            ],
            inputs: vec![(
                token(1),
                TypeSpec::required(ExpressionType::Number),
                Some(num(3.27)),
            )],
        },
        Case {
            // match theme { 0 => if contrast then null else token2, 1 => token2 } else 0px
            name: "conditional_optional",
            root: id(1),
            nodes: vec![
                ExprNode::match_on(
                    id(1),
                    id(2),
                    vec![
                        (TypedValue::Enum(EnumValue::new(9, 0)), id(3)),
                        (TypedValue::Enum(EnumValue::new(9, 1)), id(4)),
                    ],
                    id(5),
                ),
                ExprNode::reference(id(2), env(1)),
                ExprNode::if_else(id(3), id(6), id(7), id(4)),
                ExprNode::reference(id(4), token(2)),
                ExprNode::literal(id(5), len(0.0)),
                ExprNode::reference(id(6), env(2)),
                ExprNode::null(id(7), ExpressionType::Length),
            ],
            inputs: vec![
                (
                    env(1),
                    TypeSpec::required(ExpressionType::Enum(EnumTypeId::new(9))),
                    Some(TypedValue::Enum(EnumValue::new(9, 0))),
                ),
                (
                    env(2),
                    TypeSpec::required(ExpressionType::Boolean),
                    Some(TypedValue::Boolean(false)),
                ),
                (
                    token(2),
                    TypeSpec::optional(ExpressionType::Length),
                    Some(len(12.25)),
                ),
            ],
        },
        Case {
            name: "conditional_null_branch",
            root: id(1),
            nodes: vec![
                ExprNode::if_else(id(1), id(2), id(3), id(4)),
                ExprNode::reference(id(2), env(2)),
                ExprNode::null(id(3), ExpressionType::Length),
                ExprNode::reference(id(4), token(2)),
            ],
            inputs: vec![
                (
                    env(2),
                    TypeSpec::required(ExpressionType::Boolean),
                    Some(TypedValue::Boolean(true)),
                ),
                (token(2), TypeSpec::optional(ExpressionType::Length), None),
            ],
        },
        Case {
            // not (a < b) or (token2 == null) and (a >= b)
            name: "comparison_logic",
            root: id(1),
            nodes: vec![
                ExprNode::binary(id(1), BinaryOp::Or, id(2), id(3)),
                ExprNode::unary(id(2), UnaryOp::Not, id(4)),
                ExprNode::binary(id(3), BinaryOp::And, id(5), id(6)),
                ExprNode::binary(id(4), BinaryOp::Less, id(7), id(8)),
                ExprNode::binary(id(5), BinaryOp::Equal, id(9), id(10)),
                ExprNode::binary(id(6), BinaryOp::GreaterEqual, id(7), id(8)),
                ExprNode::literal(id(7), len(2.0)),
                ExprNode::literal(id(8), len(3.0)),
                ExprNode::reference(id(9), token(2)),
                ExprNode::null(id(10), ExpressionType::Length),
            ],
            inputs: vec![(token(2), TypeSpec::optional(ExpressionType::Length), None)],
        },
        Case {
            name: "string_functions",
            root: id(1),
            nodes: vec![
                ExprNode::call(id(1), FunctionId::Replace, &[id(2), id(3), id(4)]),
                ExprNode::call(id(2), FunctionId::Concat, &[id(5), id(6)]),
                ExprNode::literal(id(3), text("{n}")),
                ExprNode::call(id(4), FunctionId::FormatNumber, &[id(7), id(8)]),
                ExprNode::literal(id(5), text("간격 {n}px, ")),
                ExprNode::literal(id(6), text("배율 {n}")),
                ExprNode::reference(id(7), token(1)),
                ExprNode::literal(id(8), num(3.0)),
            ],
            inputs: vec![(
                token(1),
                TypeSpec::required(ExpressionType::Number),
                Some(num(2.0 / 3.0)),
            )],
        },
        Case {
            name: "format_number_exact_decimal",
            root: id(1),
            nodes: vec![
                ExprNode::call(id(1), FunctionId::FormatNumber, &[id(2), id(3)]),
                ExprNode::literal(id(2), num(0.1)),
                ExprNode::literal(id(3), num(9.0)),
            ],
            inputs: vec![],
        },
        Case {
            name: "with_alpha",
            root: id(1),
            nodes: vec![
                ExprNode::call(id(1), FunctionId::WithAlpha, &[id(2), id(3)]),
                ExprNode::literal(id(2), color(p3, 0.25, 0.5, 0.75, 1.0)),
                ExprNode::literal(id(3), num(0.4)),
            ],
            inputs: vec![],
        },
        Case {
            name: "mix_linear_light",
            root: id(1),
            nodes: vec![
                ExprNode::call(id(1), FunctionId::Mix, &[id(2), id(3), id(4)]),
                ExprNode::literal(id(2), color(srgb, 0.9, 0.2, 0.1, 1.0)),
                ExprNode::literal(id(3), color(srgb, 0.1, 0.4, 0.8, 0.6)),
                ExprNode::literal(id(4), num(0.3)),
            ],
            inputs: vec![],
        },
        Case {
            name: "to_display_p3",
            root: id(1),
            nodes: vec![
                ExprNode::call(id(1), FunctionId::ToDisplayP3, &[id(2)]),
                ExprNode::literal(id(2), color(srgb, 0.8, 0.3, 0.5, 0.9)),
            ],
            inputs: vec![],
        },
        Case {
            name: "to_srgb_out_of_gamut",
            root: id(1),
            nodes: vec![
                ExprNode::call(id(1), FunctionId::ToSrgb, &[id(2)]),
                ExprNode::literal(id(2), color(p3, 0.0, 1.0, 0.2, 1.0)),
            ],
            inputs: vec![],
        },
    ]
}

/// case가 libm 초월함수(색 transfer의 powf)를 쓰는지 AST에서 계산합니다.
fn class_of(ast: &ExpressionAst) -> &'static str {
    let uses_transfer = ast.nodes().iter().any(|node| {
        matches!(
            node.payload(),
            NodePayload::Call(FunctionId::Mix | FunctionId::ToSrgb | FunctionId::ToDisplayP3)
        )
    });
    if uses_transfer {
        "libm"
    } else {
        "ieee"
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 결과의 안정 문자열: 값 bytes hex, null, 또는 오류 종류.
fn encode_result(result: &Result<Option<TypedValue>, EvalError>) -> String {
    match result {
        Ok(Some(value)) => format!("value:{}", hex(&value.to_bytes())),
        Ok(None) => "null".to_string(),
        Err(error) => format!("error:{:?}", error.kind),
    }
}

/// 한 case를 다섯 경로로 평가해 모두 같은 결과 문자열인지 확인하고 그 문자열을 돌려줍니다.
fn evaluate_all_paths(case: &Case) -> (String, &'static str) {
    let ast = ExpressionAst::new(case.root, case.nodes.clone()).expect("유효한 AST");
    let types: BTreeMap<ReferenceTarget, TypeSpec> = case
        .inputs
        .iter()
        .map(|(target, spec, _)| (target.clone(), *spec))
        .collect();
    let values: BTreeMap<ReferenceTarget, Option<TypedValue>> = case
        .inputs
        .iter()
        .map(|(target, _, value)| (target.clone(), value.clone()))
        .collect();
    let program = Program::compile(&ast, &types).expect("컴파일");
    let reference = encode_result(&program.evaluate(&values));
    // 1. 반복 평가.
    for _ in 0..3 {
        assert_eq!(
            encode_result(&program.evaluate(&values)),
            reference,
            "{}: 반복",
            case.name
        );
    }
    // 2. 재컴파일 program 동일·결과 동일.
    let recompiled = Program::compile(&ast, &types).expect("재컴파일");
    assert_eq!(recompiled, program, "{}: bytecode 결정적", case.name);
    // 3. canonical bytes에서 복원.
    let bytes = ast.to_canonical_bytes().expect("직렬화");
    let restored = ExpressionAst::from_canonical_bytes(&bytes).expect("역직렬화");
    let from_bytes = Program::compile(&restored, &types).expect("컴파일");
    assert_eq!(
        encode_result(&from_bytes.evaluate(&values)),
        reference,
        "{}: 복원",
        case.name
    );
    // 4. 참조 값 제공 순서 정방향·역방향.
    let mut ordered: Vec<(ReferenceTarget, Option<TypedValue>)> =
        values.clone().into_iter().collect();
    assert_eq!(
        encode_result(&program.evaluate(&OrderedValues(ordered.clone()))),
        reference,
        "{}: 제공 순서",
        case.name
    );
    ordered.reverse();
    assert_eq!(
        encode_result(&program.evaluate(&OrderedValues(ordered))),
        reference,
        "{}: 역순 제공",
        case.name
    );
    // 5. AST 기준 evaluator.
    assert_eq!(
        encode_result(&evaluate_ast(&ast, &types, &values)),
        reference,
        "{}: AST 기준 evaluator",
        case.name
    );
    (reference, class_of(&ast))
}

fn golden() -> Vec<(String, String, String)> {
    GOLDEN
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|line| {
            let mut parts = line.split_whitespace();
            let name = parts.next().expect("이름").to_string();
            let class = parts.next().expect("분류").to_string();
            let result = parts.next().expect("결과").to_string();
            assert!(parts.next().is_none(), "golden 열 수: {line}");
            (name, class, result)
        })
        .collect()
}

#[test]
fn evaluator_results_are_bit_stable_and_match_golden() {
    let actual: Vec<(String, String, String)> = corpus()
        .iter()
        .map(|case| {
            let (result, class) = evaluate_all_paths(case);
            (case.name.to_string(), class.to_string(), result)
        })
        .collect();
    let rendered: Vec<String> = actual
        .iter()
        .map(|(name, class, result)| format!("{name} {class} {result}"))
        .collect();
    assert_eq!(
        actual,
        golden(),
        "golden 불일치. 현재 결과:\n{}",
        rendered.join("\n")
    );
}

#[test]
fn golden_covers_both_classes_and_all_outcomes() {
    let golden = golden();
    assert_eq!(golden.len(), corpus().len());
    let ieee = golden
        .iter()
        .filter(|(_, class, _)| class == "ieee")
        .count();
    let libm = golden
        .iter()
        .filter(|(_, class, _)| class == "libm")
        .count();
    assert_eq!(ieee + libm, golden.len(), "분류는 ieee/libm뿐");
    assert!(ieee >= 10 && libm >= 3);
    for prefix in ["value:", "null", "error:"] {
        assert!(
            golden
                .iter()
                .any(|(_, _, result)| result.starts_with(prefix)),
            "{prefix} 결과 포함"
        );
    }
}

#[test]
fn golden_header_bytes_are_hand_checked() {
    // with_alpha 결과는 IEEE 연산 없이 정해집니다: Color tag 0x09 ++ P3 tag 0x02 ++ r,g,b,a f32 LE.
    let golden = golden();
    let (_, class, result) = golden
        .iter()
        .find(|(name, _, _)| name == "with_alpha")
        .expect("with_alpha case");
    assert_eq!(class, "ieee");
    let mut expected = vec![0x09, 0x02];
    for channel in [0.25f32, 0.5, 0.75, 0.4] {
        expected.extend_from_slice(&channel.to_le_bytes());
    }
    assert_eq!(result, &format!("value:{}", hex(&expected)));
}
