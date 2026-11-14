//! 양 플랫폼 evaluator golden corpus (VIII.9.3).
//!
//! 연산·함수 registry 전체를 표로 훑어 만든 case마다 결과 bytes(`TypedValue::to_bytes`)를 고정 golden 파일
//! `tests/fixtures/evaluator_golden_corpus_v1.golden`과 **파일 bytes 단위로** 비교합니다. 다른 플랫폼에서 같은
//! 테스트를 실행하면 모든 결과 bytes가 golden과 같아야 합니다.
//!
//! case마다 다음이 모두 같아야 golden과 비교합니다.
//!
//! 1. bytecode evaluator(같은 program 2회),
//! 2. canonical AST bytes(VIII.2.5)에서 복원해 다시 컴파일한 program,
//! 3. AST 기준 evaluator.
//!
//! 각 줄은 `<이름> <분류> <입력 digest> <결과>`입니다.
//!
//! - 분류: `ieee`(IEEE-754 기본 연산·정확 연산·정수 연산만) | `libm`(색 transfer의 `powf` 사용). AST의 함수 호출로
//!   계산합니다.
//! - 입력 digest: canonical AST bytes와 참조 입력(대상·타입·값 bytes)의 FNV-1a 64. case 정의가 바뀌면 달라집니다.
//! - 결과: `value:<hex>` | `null` | `error:<EvalErrorKind(공백 제거)>@<오류 node 번호>`.
//!
//! golden 갱신(의도적 변경만): `STELLAR_UPDATE_EVALUATOR_CORPUS=1 cargo test -p stellar-expressions --test
//! evaluator_golden_corpus`.

use std::collections::{BTreeMap, BTreeSet};

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::reference::TokenRef;
use stellar_core_types::typed_value::{EnumValue, ResourceRef, TypedValue};
use stellar_core_types::value::{Angle, Duration, FiniteF32, Length, Percentage};
use stellar_expressions::ast::{
    EnvironmentKey, ExprNode, ExprNodeId, ExpressionAst, NodePayload, ReferenceTarget,
};
use stellar_expressions::ast_eval::evaluate_ast;
use stellar_expressions::bytecode::Program;
use stellar_expressions::eval::{EvalError, MAX_STRING_BYTES};
use stellar_expressions::functions::FunctionId;
use stellar_expressions::ops::{BinaryOp, NodeKind, UnaryOp};
use stellar_expressions::types::{EnumTypeId, ExpressionType, TypeSpec};

const GOLDEN_PATH: &str = "tests/fixtures/evaluator_golden_corpus_v1.golden";
const GOLDEN: &str = include_str!("fixtures/evaluator_golden_corpus_v1.golden");

const HEADER: &str = "\
# 양 플랫폼 evaluator golden corpus (VIII.9.3, 버전 1).
#
# 각 줄: <case 이름> <분류> <입력 digest> <결과>
# - 분류 ieee: IEEE-754 기본 연산·정확 연산(fmod, abs, round, floor, ceil)·정수 연산만 사용.
#   구성상 플랫폼과 무관하게 같은 bytes여야 합니다(ADR-0006 규칙 4).
# - 분류 libm: 색 transfer 곡선의 powf(libm 초월함수)를 사용. 현재 macOS 기준 bytes이며 Windows 재개 시
#   같은 bytes인지 다시 검증해야 합니다.
# - 입력 digest: canonical AST bytes + 참조 입력(대상·타입·값 bytes)의 FNV-1a 64(hex).
# - 결과: value:<TypedValue::to_bytes 소문자 hex> | null | error:<EvalErrorKind 공백 제거>@<오류 node 번호>
#
# case 정의와 검사: crates/expressions/tests/evaluator_golden_corpus.rs
# 갱신: STELLAR_UPDATE_EVALUATOR_CORPUS=1 cargo test -p stellar-expressions --test evaluator_golden_corpus
";

/// 수치·단위 타입(함수 `T` 제약).
const NUMERIC: [ExpressionType; 5] = [
    ExpressionType::Number,
    ExpressionType::Length,
    ExpressionType::Percentage,
    ExpressionType::Angle,
    ExpressionType::Duration,
];

const ENUM_TYPE: u32 = 7;

fn short(t: ExpressionType) -> &'static str {
    match t {
        ExpressionType::Boolean => "bool",
        ExpressionType::Number => "num",
        ExpressionType::String => "str",
        ExpressionType::Enum(_) => "enum",
        ExpressionType::Length => "len",
        ExpressionType::Percentage => "pct",
        ExpressionType::Angle => "deg",
        ExpressionType::Duration => "dur",
        ExpressionType::Color => "color",
        ExpressionType::Gradient => "gradient",
        ExpressionType::Shadow => "shadow",
        ExpressionType::Typography => "typo",
        ExpressionType::ResourceRef => "res",
    }
}

/// 수치·단위 값. 비유한 입력은 corpus 정의 오류입니다.
fn numeric(t: ExpressionType, v: f32) -> TypedValue {
    let finite = FiniteF32::new(v).expect("corpus 값은 유한");
    match t {
        ExpressionType::Number => TypedValue::Number(finite),
        ExpressionType::Length => TypedValue::Length(Length::from_raw(finite)),
        ExpressionType::Percentage => TypedValue::Percentage(Percentage::from_raw(finite)),
        ExpressionType::Angle => TypedValue::Angle(Angle::from_raw(finite)),
        ExpressionType::Duration => TypedValue::Duration(Duration::from_raw(finite)),
        other => panic!("수치 타입이 아닙니다: {other}"),
    }
}

fn num(v: f32) -> TypedValue {
    numeric(ExpressionType::Number, v)
}

fn text(s: &str) -> TypedValue {
    TypedValue::String(s.to_string())
}

fn color(space: ColorSpace, r: f32, g: f32, b: f32, a: f32) -> TypedValue {
    TypedValue::Color(Color::new(space, r, g, b, a).expect("corpus 색은 유효"))
}

fn enum_value(variant: u32) -> TypedValue {
    TypedValue::Enum(EnumValue::new(ENUM_TYPE, variant))
}

/// 타입별 비교용 값 세 개(작음, 같음 기준, 큼).
fn ordered_triplet(t: ExpressionType) -> Option<[TypedValue; 3]> {
    Some(match t {
        ExpressionType::Boolean => [
            TypedValue::Boolean(false),
            TypedValue::Boolean(true),
            TypedValue::Boolean(true),
        ],
        ExpressionType::String => [text("가나"), text("abc"), text("abd")],
        ExpressionType::Enum(_) => [enum_value(0), enum_value(2), enum_value(3)],
        ExpressionType::Color => [
            color(ColorSpace::Srgb, 0.1, 0.2, 0.3, 1.0),
            color(ColorSpace::DisplayP3, 0.1, 0.2, 0.3, 1.0),
            color(ColorSpace::Srgb, 0.1, 0.2, 0.3, 0.5),
        ],
        ExpressionType::ResourceRef => [
            TypedValue::ResourceRef(ResourceRef::new(StableId::from_bytes([1; 16]))),
            TypedValue::ResourceRef(ResourceRef::new(StableId::from_bytes([2; 16]))),
            TypedValue::ResourceRef(ResourceRef::new(StableId::from_bytes([3; 16]))),
        ],
        t if NUMERIC.contains(&t) => [numeric(t, -1.5), numeric(t, 0.1), numeric(t, 2.75)],
        _ => return None,
    })
}

fn token(n: u64) -> ReferenceTarget {
    ReferenceTarget::Token(TokenRef::new(StableId::new_v7(0xA0, 0, n)))
}

fn environment(n: u32) -> ReferenceTarget {
    ReferenceTarget::Environment(EnvironmentKey::new(n))
}

/// 입력 환경 항목: 참조 대상, 타입, 값.
type Input = (ReferenceTarget, TypeSpec, Option<TypedValue>);

/// corpus case 하나.
struct Case {
    name: String,
    root: ExprNodeId,
    nodes: Vec<ExprNode>,
    inputs: Vec<Input>,
    /// 타입은 알려 주되 값은 주지 않는 참조 대상(값 누락 경로).
    withheld: Vec<ReferenceTarget>,
}

/// node ID를 순서대로 발급하며 AST를 만드는 도구.
struct Builder {
    nodes: Vec<ExprNode>,
    inputs: Vec<Input>,
    next: u64,
}

fn node_id(n: u64) -> ExprNodeId {
    ExprNodeId::new(StableId::new_v7(0xC0, 0, n))
}

impl Builder {
    fn new() -> Builder {
        Builder {
            nodes: Vec::new(),
            inputs: Vec::new(),
            next: 0,
        }
    }

    fn id(&mut self) -> ExprNodeId {
        self.next += 1;
        node_id(self.next)
    }

    fn push(&mut self, node: ExprNode) -> ExprNodeId {
        let id = node.id();
        self.nodes.push(node);
        id
    }

    fn lit(&mut self, value: TypedValue) -> ExprNodeId {
        let id = self.id();
        self.push(ExprNode::literal(id, value))
    }

    fn null(&mut self, t: ExpressionType) -> ExprNodeId {
        let id = self.id();
        self.push(ExprNode::null(id, t))
    }

    /// 참조 node와 입력 항목. 같은 대상은 한 번만 입력에 넣습니다.
    fn reference(
        &mut self,
        target: ReferenceTarget,
        spec: TypeSpec,
        value: Option<TypedValue>,
    ) -> ExprNodeId {
        if !self.inputs.iter().any(|(t, _, _)| *t == target) {
            self.inputs.push((target.clone(), spec, value));
        }
        let id = self.id();
        self.push(ExprNode::reference(id, target))
    }

    /// 값을 토큰 참조로 넣습니다(번호는 입력 순서).
    fn input(&mut self, value: TypedValue) -> ExprNodeId {
        let target = token(self.inputs.len() as u64 + 1);
        let spec = TypeSpec::required(ExpressionType::of(&value));
        self.reference(target, spec, Some(value))
    }

    fn unary(&mut self, op: UnaryOp, a: ExprNodeId) -> ExprNodeId {
        let id = self.id();
        self.push(ExprNode::unary(id, op, a))
    }

    fn binary(&mut self, op: BinaryOp, a: ExprNodeId, b: ExprNodeId) -> ExprNodeId {
        let id = self.id();
        self.push(ExprNode::binary(id, op, a, b))
    }

    fn call(&mut self, function: FunctionId, args: &[ExprNodeId]) -> ExprNodeId {
        let id = self.id();
        self.push(ExprNode::call(id, function, args))
    }

    fn if_else(&mut self, c: ExprNodeId, t: ExprNodeId, e: ExprNodeId) -> ExprNodeId {
        let id = self.id();
        self.push(ExprNode::if_else(id, c, t, e))
    }

    fn match_on(
        &mut self,
        scrutinee: ExprNodeId,
        arms: Vec<(TypedValue, ExprNodeId)>,
        default: ExprNodeId,
    ) -> ExprNodeId {
        let id = self.id();
        self.push(ExprNode::match_on(id, scrutinee, arms, default))
    }

    fn finish(self, name: String, root: ExprNodeId) -> Case {
        Case {
            name,
            root,
            nodes: self.nodes,
            inputs: self.inputs,
            withheld: Vec::new(),
        }
    }
}

/// 첫 인자는 토큰 참조, 나머지는 literal인 호출 case.
fn call_case(name: String, function: FunctionId, args: Vec<TypedValue>) -> Case {
    let mut b = Builder::new();
    let ids: Vec<ExprNodeId> = args
        .into_iter()
        .enumerate()
        .map(
            |(i, value)| {
                if i == 0 {
                    b.input(value)
                } else {
                    b.lit(value)
                }
            },
        )
        .collect();
    let root = b.call(function, &ids);
    b.finish(name, root)
}

/// 산술 피연산자 쌍: 일반, 음수·분수, 반올림, 0 나누기, overflow, subnormal.
const ARITH_PAIRS: [(f32, f32); 6] = [
    (12.5, 4.0),
    (-7.25, 2.5),
    (0.1, 0.2),
    (5.0, 0.0),
    (3.0e38, 2.0e38),
    (1.0e-38, 3.0),
];

fn arithmetic_cases(out: &mut Vec<Case>) {
    use stellar_expressions::arithmetic::{ArithmeticOp, ARITHMETIC_RULES};
    for rule in ARITHMETIC_RULES {
        let op = match rule.op {
            ArithmeticOp::Add => BinaryOp::Add,
            ArithmeticOp::Subtract => BinaryOp::Subtract,
            ArithmeticOp::Multiply => BinaryOp::Multiply,
            ArithmeticOp::Divide => BinaryOp::Divide,
            ArithmeticOp::Remainder => BinaryOp::Remainder,
        };
        for (i, (a, c)) in ARITH_PAIRS.iter().enumerate() {
            let mut b = Builder::new();
            let lhs = b.input(numeric(rule.lhs, *a));
            let rhs = b.lit(numeric(rule.rhs, *c));
            let root = b.binary(op, lhs, rhs);
            out.push(b.finish(
                format!("arith.{op:?}.{}_{}.{i}", short(rule.lhs), short(rule.rhs)).to_lowercase(),
                root,
            ));
        }
    }
    for t in NUMERIC {
        for (i, v) in [0.0f32, 3.5, -1.0e-30, 3.4e38].iter().enumerate() {
            let mut b = Builder::new();
            let operand = b.input(numeric(t, *v));
            let root = b.unary(UnaryOp::Negate, operand);
            out.push(b.finish(format!("neg.{}.{i}", short(t)), root));
        }
    }
}

const ALL_TYPES: [ExpressionType; 10] = [
    ExpressionType::Boolean,
    ExpressionType::Number,
    ExpressionType::String,
    ExpressionType::Enum(EnumTypeId::new(ENUM_TYPE)),
    ExpressionType::Length,
    ExpressionType::Percentage,
    ExpressionType::Angle,
    ExpressionType::Duration,
    ExpressionType::Color,
    ExpressionType::ResourceRef,
];

const COMPARISONS: [BinaryOp; 6] = [
    BinaryOp::Equal,
    BinaryOp::NotEqual,
    BinaryOp::Less,
    BinaryOp::LessEqual,
    BinaryOp::Greater,
    BinaryOp::GreaterEqual,
];

/// type 규칙을 통과하는 비교만 남깁니다(통과 여부는 compile로 판정).
fn comparison_cases(out: &mut Vec<Case>) {
    for op in COMPARISONS {
        for t in ALL_TYPES {
            let Some(values) = ordered_triplet(t) else {
                continue;
            };
            for (i, (l, r)) in [(1usize, 1usize), (0, 2), (2, 0), (1, 2)]
                .iter()
                .enumerate()
            {
                let mut b = Builder::new();
                let lhs = b.input(values[*l].clone());
                let rhs = b.lit(values[*r].clone());
                let root = b.binary(op, lhs, rhs);
                let case = b.finish(format!("cmp.{op:?}.{}.{i}", short(t)).to_lowercase(), root);
                if compiles(&case) {
                    out.push(case);
                }
            }
        }
    }
    // nullable 동등 비교: Optional 참조 vs null literal, 값 있는 Optional vs 값.
    let optional = TypeSpec::optional(ExpressionType::Length);
    for (i, (value, op)) in [
        (None, BinaryOp::Equal),
        (None, BinaryOp::NotEqual),
        (Some(numeric(ExpressionType::Length, 4.0)), BinaryOp::Equal),
        (
            Some(numeric(ExpressionType::Length, 4.0)),
            BinaryOp::NotEqual,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut b = Builder::new();
        let lhs = b.reference(token(1), optional, value.clone());
        let rhs = b.null(ExpressionType::Length);
        let root = b.binary(op, lhs, rhs);
        out.push(b.finish(format!("cmp.optional_null.{i}"), root));
        let mut b = Builder::new();
        let lhs = b.reference(token(1), optional, value);
        let rhs = b.lit(numeric(ExpressionType::Length, 4.0));
        let root = b.binary(op, lhs, rhs);
        out.push(b.finish(format!("cmp.optional_value.{i}"), root));
    }
}

fn logic_cases(out: &mut Vec<Case>) {
    for op in [BinaryOp::And, BinaryOp::Or] {
        for (i, (l, r)) in [(false, false), (false, true), (true, false), (true, true)]
            .iter()
            .enumerate()
        {
            let mut b = Builder::new();
            let lhs = b.input(TypedValue::Boolean(*l));
            let rhs = b.lit(TypedValue::Boolean(*r));
            let root = b.binary(op, lhs, rhs);
            out.push(b.finish(format!("logic.{op:?}.{i}").to_lowercase(), root));
        }
        // 오른쪽이 0 나누기를 포함한 비교: and는 false에서, or는 true에서 오른쪽을 평가하지 않습니다.
        for (i, lhs_value) in [false, true].iter().enumerate() {
            let mut b = Builder::new();
            let lhs = b.input(TypedValue::Boolean(*lhs_value));
            let one = b.lit(num(1.0));
            let zero = b.lit(num(0.0));
            let div = b.binary(BinaryOp::Divide, one, zero);
            let rhs = b.binary(BinaryOp::Equal, div, one);
            let root = b.binary(op, lhs, rhs);
            out.push(b.finish(
                format!("logic.{op:?}.short_circuit.{i}").to_lowercase(),
                root,
            ));
        }
    }
    for (i, v) in [false, true].iter().enumerate() {
        let mut b = Builder::new();
        let operand = b.input(TypedValue::Boolean(*v));
        let root = b.unary(UnaryOp::Not, operand);
        out.push(b.finish(format!("logic.not.{i}"), root));
    }
}

fn conditional_cases(out: &mut Vec<Case>) {
    // if: 선택되지 않은 branch의 0 나누기는 실행하지 않습니다.
    for (i, c) in [false, true].iter().enumerate() {
        let mut b = Builder::new();
        let cond = b.input(TypedValue::Boolean(*c));
        let then = b.lit(numeric(ExpressionType::Length, 8.0));
        let one = b.lit(numeric(ExpressionType::Length, 1.0));
        let zero = b.lit(num(0.0));
        let otherwise = b.binary(BinaryOp::Divide, one, zero);
        let root = b.if_else(cond, then, otherwise);
        out.push(b.finish(format!("if.short_circuit.{i}"), root));
    }
    // if: T와 Optional<T> 공통 타입, null branch 선택.
    for (i, c) in [false, true].iter().enumerate() {
        let mut b = Builder::new();
        let cond = b.input(TypedValue::Boolean(*c));
        let then = b.null(ExpressionType::Angle);
        let otherwise = b.lit(numeric(ExpressionType::Angle, 45.0));
        let root = b.if_else(cond, then, otherwise);
        out.push(b.finish(format!("if.optional.{i}"), root));
    }
    // match: Boolean·Number·String·Enum 대상, 일치 arm·기본 branch.
    let scrutinees: [(&str, Vec<TypedValue>, TypedValue); 4] = [
        (
            "bool",
            vec![TypedValue::Boolean(true)],
            TypedValue::Boolean(false),
        ),
        ("num", vec![num(1.0), num(2.5), num(-3.0)], num(2.5)),
        ("str", vec![text("a"), text("한글"), text("")], text("")),
        (
            "enum",
            vec![enum_value(0), enum_value(1), enum_value(4)],
            enum_value(4),
        ),
    ];
    for (label, patterns, hit) in scrutinees {
        for (i, scrutinee_value) in [hit.clone(), patterns[0].clone(), miss_of(&hit)]
            .into_iter()
            .enumerate()
        {
            let mut b = Builder::new();
            let scrutinee = b.input(scrutinee_value);
            let arms: Vec<(TypedValue, ExprNodeId)> = patterns
                .iter()
                .enumerate()
                .map(|(k, p)| {
                    let arm = b.lit(text(&format!("arm{k}")));
                    (p.clone(), arm)
                })
                .collect();
            let default = b.lit(text("default"));
            let root = b.match_on(scrutinee, arms, default);
            out.push(b.finish(format!("match.{label}.{i}"), root));
        }
    }
    // match: 선택되지 않은 arm의 오류는 실행하지 않습니다.
    let mut b = Builder::new();
    let scrutinee = b.input(num(2.0));
    let one = b.lit(num(1.0));
    let zero = b.lit(num(0.0));
    let failing = b.binary(BinaryOp::Remainder, one, zero);
    let fine = b.lit(num(9.0));
    let root = b.match_on(scrutinee, vec![(num(1.0), failing), (num(2.0), fine)], one);
    out.push(b.finish("match.short_circuit.0".to_string(), root));
}

/// pattern과 다른 값(기본 branch 선택용).
fn miss_of(value: &TypedValue) -> TypedValue {
    match value {
        TypedValue::Boolean(_) => TypedValue::Boolean(false),
        TypedValue::Number(_) => num(99.0),
        TypedValue::String(_) => text("없음"),
        _ => enum_value(9),
    }
}

fn numeric_function_cases(out: &mut Vec<Case>) {
    for t in NUMERIC {
        let v = |x: f32| numeric(t, x);
        let s = short(t);
        for (i, (a, c)) in [
            (1.5f32, -2.0f32),
            (3.0, 3.0),
            (-0.0, 0.0),
            (1.0e-40, 2.0e-40),
        ]
        .iter()
        .enumerate()
        {
            for f in [FunctionId::Min, FunctionId::Max] {
                out.push(call_case(
                    format!("fn.{f:?}.{s}.{i}").to_lowercase(),
                    f,
                    vec![v(*a), v(*c)],
                ));
            }
        }
        for (i, (x, lo, hi)) in [
            (5.0f32, 0.0f32, 10.0f32),
            (-1.0, 0.0, 10.0),
            (11.0, 0.0, 10.0),
            (2.0, 2.0, 2.0),
            (1.0, 10.0, 0.0),
        ]
        .iter()
        .enumerate()
        {
            out.push(call_case(
                format!("fn.clamp.{s}.{i}"),
                FunctionId::Clamp,
                vec![v(*x), v(*lo), v(*hi)],
            ));
        }
        for (i, x) in [
            2.5f32,
            -2.5,
            0.499_999_97,
            -0.5,
            8_388_609.0,
            -1.0e-45,
            123.456,
        ]
        .iter()
        .enumerate()
        {
            for f in [
                FunctionId::Abs,
                FunctionId::Round,
                FunctionId::Floor,
                FunctionId::Ceil,
            ] {
                out.push(call_case(
                    format!("fn.{f:?}.{s}.{i}").to_lowercase(),
                    f,
                    vec![v(*x)],
                ));
            }
        }
        for (i, (a, c, w)) in [
            (0.0f32, 10.0f32, 0.25f32),
            (-5.0, 5.0, 1.5),
            (1.0, 2.0, -0.5),
            (0.1, 0.7, 0.3),
            (3.0e38, -3.0e38, 0.5),
        ]
        .iter()
        .enumerate()
        {
            out.push(call_case(
                format!("fn.lerp.{s}.{i}"),
                FunctionId::Lerp,
                vec![v(*a), v(*c), num(*w)],
            ));
        }
        for u in NUMERIC {
            let w = |x: f32| numeric(u, x);
            for (i, (x, i0, i1, o0, o1)) in [
                (2.5f32, 0.0f32, 10.0f32, 100.0f32, 200.0f32),
                (-1.0, 0.0, 4.0, 0.0, 1.0),
                (3.0, 3.0, 3.0, 0.0, 1.0),
                (0.3, 0.1, 0.7, -1.0, 1.0),
            ]
            .iter()
            .enumerate()
            {
                out.push(call_case(
                    format!("fn.remap.{s}_{}.{i}", short(u)),
                    FunctionId::Remap,
                    vec![v(*x), v(*i0), v(*i1), w(*o0), w(*o1)],
                ));
            }
        }
    }
}

fn color_function_cases(out: &mut Vec<Case>) {
    use ColorSpace::{DisplayP3, Srgb};
    let colors = [
        ("srgb_red", color(Srgb, 0.9, 0.2, 0.1, 1.0)),
        ("srgb_half", color(Srgb, 0.1, 0.4, 0.8, 0.6)),
        ("p3_green", color(DisplayP3, 0.0, 1.0, 0.2, 1.0)),
        ("p3_dark", color(DisplayP3, 0.02, 0.03, 0.04, 0.25)),
        ("srgb_clear", color(Srgb, 0.0, 0.0, 0.0, 0.0)),
        ("srgb_wide", color(Srgb, -0.2, 1.3, 0.5, 1.0)),
    ];
    for (name, c) in &colors {
        out.push(call_case(
            format!("fn.to_srgb.{name}"),
            FunctionId::ToSrgb,
            vec![c.clone()],
        ));
        out.push(call_case(
            format!("fn.to_display_p3.{name}"),
            FunctionId::ToDisplayP3,
            vec![c.clone()],
        ));
        for (i, alpha) in [0.0f32, 0.5, 1.0, -0.1, 1.1].iter().enumerate() {
            out.push(call_case(
                format!("fn.with_alpha.{name}.{i}"),
                FunctionId::WithAlpha,
                vec![c.clone(), num(*alpha)],
            ));
        }
    }
    for (from_name, from) in &colors[..4] {
        for (to_name, to) in &colors[..4] {
            if from_name == to_name {
                continue;
            }
            for (i, t) in [0.0f32, 0.3, 1.0, 1.5].iter().enumerate() {
                out.push(call_case(
                    format!("fn.mix.{from_name}_{to_name}.{i}"),
                    FunctionId::Mix,
                    vec![from.clone(), to.clone(), num(*t)],
                ));
            }
        }
    }
}

fn string_function_cases(out: &mut Vec<Case>) {
    let strings = ["", "abc", "간격", "🙂 emoji", "a\u{0}b\n\"q\""];
    for (i, a) in strings.iter().enumerate() {
        for (j, c) in strings.iter().enumerate() {
            out.push(call_case(
                format!("fn.concat.{i}_{j}"),
                FunctionId::Concat,
                vec![text(a), text(c)],
            ));
        }
    }
    let big = "x".repeat(MAX_STRING_BYTES / 2 + 1);
    out.push(call_case(
        "fn.concat.too_long".to_string(),
        FunctionId::Concat,
        vec![text(&big), text(&big)],
    ));
    for (i, (t, search, with)) in [
        ("aaaa", "aa", "b"),
        ("banana", "an", "AN"),
        ("간격 {n}px {n}", "{n}", "12"),
        ("abc", "", "x"),
        ("abc", "zz", "y"),
        ("🙂🙂", "🙂", ""),
        ("aaa", "a", "aa"),
    ]
    .iter()
    .enumerate()
    {
        out.push(call_case(
            format!("fn.replace.{i}"),
            FunctionId::Replace,
            vec![text(t), text(search), text(with)],
        ));
    }
    for (i, x) in [
        0.0f32,
        -0.0049,
        2.0 / 3.0,
        1.0e10,
        123.456,
        -1.5,
        0.125,
        3.4e38,
        1.0e-45,
    ]
    .iter()
    .enumerate()
    {
        for digits in [0.0f32, 2.0, 9.0] {
            out.push(call_case(
                format!("fn.format_number.{i}.{digits}"),
                FunctionId::FormatNumber,
                vec![num(*x), num(digits)],
            ));
        }
    }
    for (i, digits) in [10.0f32, -1.0, 1.5].iter().enumerate() {
        out.push(call_case(
            format!("fn.format_number.invalid_digits.{i}"),
            FunctionId::FormatNumber,
            vec![num(1.0), num(*digits)],
        ));
    }
}

/// 참조 입력 경로: 누락·null·타입 불일치·환경 입력.
fn reference_cases(out: &mut Vec<Case>) {
    let length = TypeSpec::required(ExpressionType::Length);
    // 타입은 있으나 값이 주어지지 않은 참조.
    let mut b = Builder::new();
    let root = b.reference(token(9), length, None);
    let mut case = b.finish("ref.missing".to_string(), root);
    case.withheld.push(token(9));
    out.push(case);
    // Required 참조에 null.
    let mut b = Builder::new();
    let root = b.reference(token(1), length, None);
    out.push(b.finish("ref.required_null".to_string(), root));
    // 값 타입이 선언 타입과 다름.
    let mut b = Builder::new();
    let root = b.reference(token(1), length, Some(num(3.0)));
    out.push(b.finish("ref.type_mismatch".to_string(), root));
    // 환경 입력 참조와 산술.
    let mut b = Builder::new();
    let width = b.reference(
        environment(8),
        TypeSpec::required(ExpressionType::Length),
        Some(numeric(ExpressionType::Length, 390.0)),
    );
    let half = b.lit(num(2.0));
    let root = b.binary(BinaryOp::Divide, width, half);
    out.push(b.finish("ref.environment".to_string(), root));
}

/// DAG 공유: `x(k+1) = x(k) + x(k)`로 같은 node를 두 번 참조하는 사슬. 공유 block 캐시 경로를 씁니다.
fn dag_cases(out: &mut Vec<Case>) {
    for (depth, start) in [(20u32, 1.5f32), (30, 0.1), (130, 1.0)] {
        let mut b = Builder::new();
        let mut current = b.input(numeric(ExpressionType::Length, start));
        for _ in 0..depth {
            current = b.binary(BinaryOp::Add, current, current);
        }
        out.push(b.finish(format!("dag.doubling.{depth}"), current));
    }
}

fn corpus() -> Vec<Case> {
    let mut out = Vec::new();
    arithmetic_cases(&mut out);
    comparison_cases(&mut out);
    logic_cases(&mut out);
    conditional_cases(&mut out);
    numeric_function_cases(&mut out);
    color_function_cases(&mut out);
    string_function_cases(&mut out);
    reference_cases(&mut out);
    dag_cases(&mut out);
    out
}

fn ast_of(case: &Case) -> ExpressionAst {
    ExpressionAst::new(case.root, case.nodes.clone())
        .unwrap_or_else(|error| panic!("{}: 유효한 AST여야 합니다: {error}", case.name))
}

fn types_of(case: &Case) -> BTreeMap<ReferenceTarget, TypeSpec> {
    case.inputs
        .iter()
        .map(|(target, spec, _)| (target.clone(), *spec))
        .collect()
}

fn values_of(case: &Case) -> BTreeMap<ReferenceTarget, Option<TypedValue>> {
    case.inputs
        .iter()
        .filter(|(target, _, _)| !case.withheld.contains(target))
        .map(|(target, _, value)| (target.clone(), value.clone()))
        .collect()
}

fn compiles(case: &Case) -> bool {
    Program::compile(&ast_of(case), &types_of(case)).is_ok()
}

/// libm 초월함수(색 transfer의 powf)를 쓰는지.
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

/// FNV-1a 64.
fn fnv(parts: &[&[u8]]) -> u64 {
    let mut hash: u64 = 0xCBF2_9CE4_8422_2325;
    for part in parts {
        hash ^= part.len() as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        for byte in *part {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    hash
}

fn input_digest(case: &Case, ast: &ExpressionAst) -> u64 {
    let ast_bytes = ast.to_canonical_bytes().expect("canonical 직렬화");
    let mut inputs: Vec<&Input> = case.inputs.iter().collect();
    inputs.sort_by(|a, b| a.0.cmp(&b.0));
    let mut parts: Vec<Vec<u8>> = vec![ast_bytes];
    for (target, spec, value) in inputs {
        parts.push(format!("{target:?}|{spec:?}").into_bytes());
        parts.push(value.as_ref().map(TypedValue::to_bytes).unwrap_or_default());
        parts.push(vec![u8::from(case.withheld.contains(target))]);
    }
    let refs: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
    fnv(&refs)
}

/// 오류 node를 builder 발급 번호(1부터)로 표기합니다. node가 없으면 `-`.
fn node_number(case: &Case, id: Option<ExprNodeId>) -> String {
    id.and_then(|id| case.nodes.iter().position(|node| node.id() == id))
        .map(|position| (position + 1).to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn encode(case: &Case, result: &Result<Option<TypedValue>, EvalError>) -> String {
    match result {
        Ok(Some(value)) => format!("value:{}", hex(&value.to_bytes())),
        Ok(None) => "null".to_string(),
        Err(error) => {
            let kind: String = format!("{:?}", error.kind)
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            format!("error:{kind}@{}", node_number(case, error.node))
        }
    }
}

/// case 하나를 모든 경로로 평가해 같은 결과인지 확인하고 golden 줄을 만듭니다.
fn golden_line(case: &Case) -> String {
    let ast = ast_of(case);
    let types = types_of(case);
    let values = values_of(case);
    let program = Program::compile(&ast, &types)
        .unwrap_or_else(|error| panic!("{}: 컴파일 실패: {error}", case.name));
    let reference = program.evaluate(&values);
    assert_eq!(program.evaluate(&values), reference, "{}: 반복", case.name);
    let bytes = ast.to_canonical_bytes().expect("직렬화");
    let restored = ExpressionAst::from_canonical_bytes(&bytes).expect("역직렬화");
    assert_eq!(restored, ast, "{}: canonical round-trip", case.name);
    let recompiled = Program::compile(&restored, &types).expect("재컴파일");
    assert_eq!(
        recompiled.evaluate(&values),
        reference,
        "{}: 복원 후 컴파일",
        case.name
    );
    assert_eq!(
        evaluate_ast(&ast, &types, &values),
        reference,
        "{}: AST 기준 evaluator",
        case.name
    );
    format!(
        "{} {} {:016x} {}",
        case.name,
        class_of(&ast),
        input_digest(case, &ast),
        encode(case, &reference)
    )
}

fn render() -> String {
    let mut out = String::from(HEADER);
    for case in corpus() {
        out.push_str(&golden_line(&case));
        out.push('\n');
    }
    out
}

#[test]
fn every_result_matches_golden_bytes() {
    let rendered = render();
    if std::env::var_os("STELLAR_UPDATE_EVALUATOR_CORPUS").is_some() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDEN_PATH);
        std::fs::write(&path, &rendered).expect("golden 쓰기");
        return;
    }
    if rendered != GOLDEN {
        let expected: Vec<&str> = GOLDEN.lines().collect();
        let actual: Vec<&str> = rendered.lines().collect();
        let diff: Vec<String> = actual
            .iter()
            .zip(expected.iter())
            .filter(|(a, e)| a != e)
            .take(20)
            .map(|(a, e)| format!("- {e}\n+ {a}"))
            .collect();
        panic!(
            "golden 불일치(줄 수 실제 {} / golden {}). 처음 차이:\n{}",
            actual.len(),
            expected.len(),
            diff.join("\n")
        );
    }
}

#[test]
fn corpus_covers_every_operator_function_kind_and_outcome() {
    let cases = corpus();
    let names: BTreeSet<&str> = cases.iter().map(|case| case.name.as_str()).collect();
    assert_eq!(names.len(), cases.len(), "case 이름은 유일");
    assert!(names.iter().all(|name| !name.contains(char::is_whitespace)));
    let mut kinds = BTreeSet::new();
    let mut functions = BTreeSet::new();
    let mut binaries = BTreeSet::new();
    let mut unaries = BTreeSet::new();
    for case in &cases {
        for node in &case.nodes {
            kinds.insert(node.kind());
            match node.payload() {
                NodePayload::Call(f) => {
                    functions.insert(*f);
                }
                NodePayload::Binary(op) => {
                    binaries.insert(*op);
                }
                NodePayload::Unary(op) => {
                    unaries.insert(*op);
                }
                _ => {}
            }
        }
    }
    assert_eq!(kinds, NodeKind::ALL.into_iter().collect());
    assert_eq!(functions, FunctionId::ALL.into_iter().collect());
    assert_eq!(binaries, BinaryOp::ALL.into_iter().collect());
    assert_eq!(unaries, UnaryOp::ALL.into_iter().collect());
    let lines: Vec<&str> = GOLDEN
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect();
    assert_eq!(lines.len(), cases.len());
    for marker in [" ieee ", " libm "] {
        assert!(lines.iter().any(|line| line.contains(marker)), "{marker}");
    }
    for outcome in [" value:", " null", " error:"] {
        assert!(
            lines.iter().any(|line| line.contains(outcome)),
            "{outcome} 결과 포함"
        );
    }
    // 산술 규칙 33개 × 피연산자 6쌍은 모두 type 규칙을 통과해 들어갑니다.
    assert_eq!(
        cases
            .iter()
            .filter(|case| case.name.starts_with("arith."))
            .count(),
        33 * ARITH_PAIRS.len()
    );
}

#[test]
fn hand_checked_golden_entries() {
    let result = |name: &str| -> String {
        GOLDEN
            .lines()
            .find(|line| line.split(' ').next() == Some(name))
            .and_then(|line| line.split(' ').nth(3))
            .unwrap_or_else(|| panic!("{name} 없음"))
            .to_string()
    };
    let value = |v: TypedValue| format!("value:{}", hex(&v.to_bytes()));
    // 12.5 + 4 = 16.5 (정확), 12.5 % 4 = 0.5, 0.1 + 0.2 = f32 0.3(0x3e99999a).
    assert_eq!(result("arith.add.num_num.0"), value(num(16.5)));
    assert_eq!(result("arith.remainder.num_num.0"), value(num(0.5)));
    assert_eq!(
        result("arith.add.num_num.2"),
        value(TypedValue::Number(
            FiniteF32::new(f32::from_bits(0x3e99_999a)).expect("유한")
        ))
    );
    assert!(result("arith.divide.num_num.3").starts_with("error:DivisionByZero@"));
    assert!(result("arith.multiply.num_num.4").starts_with("error:NonFiniteResult@"));
    // round는 0.5를 0에서 먼 쪽으로: round(-2.5) = -3, round(0.49999997) = 0.
    assert_eq!(result("fn.round.num.1"), value(num(-3.0)));
    assert_eq!(result("fn.round.num.2"), value(num(0.0)));
    // 1.5 × 2^130 = 2^130.58… > f32::MAX → overflow.
    assert!(result("dag.doubling.130").starts_with("error:NonFiniteResult@"));
    // and/or short-circuit는 오른쪽 오류를 실행하지 않습니다.
    assert_eq!(
        result("logic.and.short_circuit.0"),
        value(TypedValue::Boolean(false))
    );
    assert_eq!(
        result("logic.or.short_circuit.1"),
        value(TypedValue::Boolean(true))
    );
    assert!(result("logic.and.short_circuit.1").starts_with("error:DivisionByZero@"));
    // with_alpha는 연산 없이 정해집니다.
    let mut expected = vec![0x09, 0x01];
    for channel in [0.9f32, 0.2, 0.1, 0.5] {
        expected.extend_from_slice(&channel.to_le_bytes());
    }
    assert_eq!(
        result("fn.with_alpha.srgb_red.1"),
        format!("value:{}", hex(&expected))
    );
    assert_eq!(result("fn.concat.1_2"), value(text("abc간격")));
    assert!(result("fn.concat.too_long").starts_with("error:StringTooLong@"));
}
