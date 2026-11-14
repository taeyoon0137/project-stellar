//! expression AST·typechecker·evaluator fuzz harness (XVI.6.4).
//!
//! XVI.6.1·XVI.6.2·XVI.6.5와 같은 방식으로, 외부 fuzz 도구(cargo-fuzz/libFuzzer) 대신 저장소 안의 **결정론적**
//! harness를 `#[test]`로 실행합니다(새 의존성·nightly 없음). 같은 seed는 같은 입력열을 만듭니다.
//!
//! - **입력**
//!   1. 타입을 아는 생성기가 만든 AST: node 종류 7개·이항 13·단항 2·함수 16개 전부, DAG 공유(이미 만든 subtree
//!      재사용), Optional·null, 토큰·환경 참조(값 누락·null·타입이 다른 값 포함). 일부는 일부러 타입을 어긋나게
//!      만듭니다.
//!   2. 구조 변형: child 대상 바꾸기(cycle·도달 불가), slot 바꾸기·삭제·복제, payload 교체, ID 중복, root 변경.
//!   3. canonical AST bytes 변형: bit 뒤집기·경계 byte·삽입·삭제·`u16`/`u32` 경계값 덮어쓰기·절단·구간 복제.
//!   4. 고정 stress 입력: 왼쪽 중첩(전체 규모 60만 단계 = 명령 120만 개로 기본 budget 초과), 부호 반전 사슬, 300단계
//!      공유 block 중첩, 7만 단계 오른쪽 중첩, `if` 중첩, arm이 많은 `match`, 공유 `concat` 증폭. release 빌드 또는
//!      `STELLAR_EXPRESSION_FUZZ_STRESS=full`이면 전체 규모, debug 기본 실행은 축소 규모입니다.
//! - **판정(입력마다)**
//!   - panic 없음(`catch_unwind`), hang 없음(입력당 시간 상한 + 전체 watchdog). 모든 대상은 기본 2 MiB stack
//!     worker thread에서 실행합니다.
//!   - 구조: 생성 AST는 검증 통과, canonical bytes round-trip 동일, 수용한 변형 bytes는 재인코딩이 byte-identical,
//!     검증된 AST는 cycle 없음.
//!   - typechecker: 컴파일 성공 ⇔ type 검사 오류 0, program 결과 타입 = root 추론 타입, 타입을 지켜 만든 AST는 type
//!     검사 통과. 검증된 program 평가에서 방어 오류(`OperandTypeMismatch`·`MalformedProgram`·`IllTyped`) 0, 값은
//!     추론 타입과 일치.
//!   - evaluator: 한도 오류가 아니면 bytecode 결과 = AST 기준 결과(오류 node 포함).
//!   - **instruction budget**: 성공한 평가의 명령 수가 `n`이면 budget `n`은 같은 결과, `n − 1`은
//!     `InstructionBudgetExceeded { budget: n − 1 }`. stack 한도도 peak로 같은 검사. 무작위 한도에서 결과는 기본
//!     한도 결과 또는 그 한도의 오류뿐이며, 보고한 한도 값은 설정 값과 같습니다.
//!
//! 발견한 입력은 `CARGO_TARGET_TMPDIR/expression-fuzz/`에 hex로 저장하고 테스트를 실패시킵니다. 반복 수 기본값은
//! 고정이며 `STELLAR_EXPRESSION_FUZZ_ITERATIONS`로 늘릴 수 있습니다.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{mpsc, Once};
use std::time::{Duration, Instant};

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::reference::TokenRef;
use stellar_core_types::typed_value::{EnumValue, TypedValue};
use stellar_core_types::value::{Angle, Duration as DurationValue, FiniteF32, Length, Percentage};
use stellar_expressions::arithmetic::{ArithmeticOp, ARITHMETIC_RULES};
use stellar_expressions::ast::{
    ChildRef, ChildSlot, EnvironmentKey, ExprNode, ExprNodeId, ExpressionAst, Literal, NodePayload,
    ReferenceTarget,
};
use stellar_expressions::ast_eval::evaluate_ast_with_limits;
use stellar_expressions::bytecode::Program;
use stellar_expressions::cycle::find_cycle;
use stellar_expressions::eval::{EvalError, EvalErrorKind};
use stellar_expressions::functions::FunctionId;
use stellar_expressions::ops::{BinaryOp, UnaryOp};
use stellar_expressions::typecheck::type_check;
use stellar_expressions::types::{EnumTypeId, ExpressionType, TypeSpec};
use stellar_expressions::vm::{EvalLimits, EvalTrace};

const SEED: u64 = 0x4558_5052_4655_5A5A;
const DEFAULT_ITERATIONS: usize = 1_200;
const STRUCTURE_MUTATIONS: usize = 3;
const BYTE_MUTATIONS: usize = 6;
const LIMIT_TRIALS: usize = 3;
/// 생성·변형 입력 하나의 시간 상한(debug 빌드 기준, 측정 최대는 1 s 미만). 부하가 큰 기기에서도 오탐하지 않게
/// 넉넉히 둡니다.
const INPUT_TIME_LIMIT: Duration = Duration::from_secs(30);
/// stress 입력 하나의 시간 상한(수만~수십만 node, debug 기준).
const STRESS_TIME_LIMIT: Duration = Duration::from_secs(240);
/// 캠페인 전체 watchdog. 이 시간 안에 끝나지 않으면 hang으로 실패합니다.
const WATCHDOG: Duration = Duration::from_secs(1_200);

const ENUM_TYPE: u32 = 5;

// --- 난수 ---

/// SplitMix64.
#[derive(Clone)]
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// `0..n` (n > 0).
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    /// 확률 1/n.
    fn one_in(&mut self, n: usize) -> bool {
        self.below(n) == 0
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())]
    }
}

// --- panic 기록 ---

thread_local! {
    static QUIET: Cell<bool> = const { Cell::new(false) };
    static LAST_PANIC_AT: RefCell<String> = const { RefCell::new(String::new()) };
}

fn install_quiet_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if QUIET.with(Cell::get) {
                let at = info
                    .location()
                    .map(|l| format!("{}:{}", l.file(), l.line()))
                    .unwrap_or_default();
                LAST_PANIC_AT.with(|slot| *slot.borrow_mut() = at);
            } else {
                default(info);
            }
        }));
    });
}

/// `f`를 panic 출력 없이 실행하고 panic을 `Err`(메시지·위치)로 돌려줍니다.
fn quietly<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    let previous = QUIET.with(|q| q.replace(true));
    let result = catch_unwind(AssertUnwindSafe(f));
    QUIET.with(|q| q.set(previous));
    result.map_err(|payload| {
        let message = if let Some(s) = payload.downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic>".to_string()
        };
        let at = LAST_PANIC_AT.with(|slot| std::mem::take(&mut *slot.borrow_mut()));
        format!("{message} (위치 {at})")
    })
}

// --- 값 ---

const NUMERIC: [ExpressionType; 5] = [
    ExpressionType::Number,
    ExpressionType::Length,
    ExpressionType::Percentage,
    ExpressionType::Angle,
    ExpressionType::Duration,
];

const VALUE_TYPES: [ExpressionType; 9] = [
    ExpressionType::Boolean,
    ExpressionType::Number,
    ExpressionType::String,
    ExpressionType::Enum(EnumTypeId::new(ENUM_TYPE)),
    ExpressionType::Length,
    ExpressionType::Percentage,
    ExpressionType::Angle,
    ExpressionType::Duration,
    ExpressionType::Color,
];

const MATCHABLE: [ExpressionType; 4] = [
    ExpressionType::Boolean,
    ExpressionType::Number,
    ExpressionType::String,
    ExpressionType::Enum(EnumTypeId::new(ENUM_TYPE)),
];

const INTERESTING: [f32; 20] = [
    0.0,
    1.0,
    -1.0,
    0.5,
    -2.5,
    2.0,
    3.0,
    10.0,
    0.1,
    100.0,
    255.0,
    1.0e-40,
    -1.0e-38,
    1.0e20,
    -3.0e38,
    3.4e38,
    8_388_609.0,
    0.499_999_97,
    9.0,
    12.0,
];

fn random_f32(rng: &mut Rng) -> f32 {
    if rng.one_in(3) {
        // 작은 범위의 임의 값.
        let raw = (rng.next() >> 40) as f32 / (1u64 << 24) as f32;
        (raw - 0.5) * 2_000.0
    } else {
        rng.pick(&INTERESTING)
    }
}

fn numeric_value(t: ExpressionType, v: f32) -> TypedValue {
    let finite = FiniteF32::new(v).unwrap_or(FiniteF32::ZERO);
    match t {
        ExpressionType::Length => TypedValue::Length(Length::from_raw(finite)),
        ExpressionType::Percentage => TypedValue::Percentage(Percentage::from_raw(finite)),
        ExpressionType::Angle => TypedValue::Angle(Angle::from_raw(finite)),
        ExpressionType::Duration => TypedValue::Duration(DurationValue::from_raw(finite)),
        _ => TypedValue::Number(finite),
    }
}

const STRINGS: [&str; 9] = [
    "", "a", "ab", "{n}", "간격", "🙂", "aaaa", "x{n}y{n}", "\u{0}",
];

fn random_value(rng: &mut Rng, t: ExpressionType) -> TypedValue {
    match t {
        ExpressionType::Boolean => TypedValue::Boolean(rng.one_in(2)),
        ExpressionType::String => {
            if rng.one_in(40) {
                TypedValue::String("가".repeat(rng.below(4_000) + 1))
            } else {
                TypedValue::String(rng.pick(&STRINGS).to_string())
            }
        }
        ExpressionType::Enum(id) => TypedValue::Enum(EnumValue::new(id.raw(), rng.below(4) as u32)),
        ExpressionType::Color => {
            let space = if rng.one_in(2) {
                ColorSpace::Srgb
            } else {
                ColorSpace::DisplayP3
            };
            let mut channel = || {
                if rng.one_in(6) {
                    random_f32(rng)
                } else {
                    rng.below(11) as f32 / 10.0
                }
            };
            let (r, g, b, a) = (channel(), channel(), channel(), channel());
            Color::new(space, r, g, b, a)
                .or_else(|_| Color::new(space, 0.0, 0.0, 0.0, 1.0))
                .map(TypedValue::Color)
                .unwrap_or(TypedValue::Boolean(false))
        }
        numeric => numeric_value(numeric, random_f32(rng)),
    }
}

// --- 생성기 ---

/// 생성 중 AST와 입력 환경.
struct Gen<'a> {
    rng: &'a mut Rng,
    nodes: Vec<ExprNode>,
    next: u64,
    budget: usize,
    /// 재사용 가능한 완성 subtree(타입, ID).
    pool: Vec<(TypeSpec, ExprNodeId)>,
    types: BTreeMap<ReferenceTarget, TypeSpec>,
    values: BTreeMap<ReferenceTarget, Option<TypedValue>>,
    /// 일부러 타입을 어긋나게 만든 위치가 있으면 `true`.
    ill_typed: bool,
    /// 타입 오염 확률(1/n, 0이면 없음).
    poison_one_in: usize,
}

impl Gen<'_> {
    fn id(&mut self) -> ExprNodeId {
        self.next += 1;
        ExprNodeId::new(StableId::new_v7(0xF0, 0, self.next))
    }

    fn push(&mut self, node: ExprNode, spec: TypeSpec) -> (ExprNodeId, TypeSpec) {
        let id = node.id();
        self.nodes.push(node);
        self.budget = self.budget.saturating_sub(1);
        self.pool.push((spec, id));
        (id, spec)
    }

    fn literal(&mut self, t: ExpressionType) -> (ExprNodeId, TypeSpec) {
        let value = random_value(self.rng, t);
        let spec = TypeSpec::required(ExpressionType::of(&value));
        let id = self.id();
        self.push(ExprNode::literal(id, value), spec)
    }

    fn reference(&mut self, want: TypeSpec) -> (ExprNodeId, TypeSpec) {
        // 같은 타입으로 선언된 기존 대상을 재사용하거나 새 대상을 만듭니다.
        let existing: Vec<ReferenceTarget> = self
            .types
            .iter()
            .filter(|(_, spec)| **spec == want)
            .map(|(target, _)| target.clone())
            .collect();
        let target = if !existing.is_empty() && self.rng.one_in(2) {
            existing[self.rng.below(existing.len())].clone()
        } else {
            let n = self.types.len() as u64;
            let target = if self.rng.one_in(4) {
                ReferenceTarget::Environment(EnvironmentKey::new(100 + n as u32))
            } else {
                ReferenceTarget::Token(TokenRef::new(StableId::new_v7(0xA0, 0, n)))
            };
            self.types.insert(target.clone(), want);
            let value = match self.rng.below(24) {
                // 값 누락.
                0 => None,
                // null(Optional이 아니면 평가 시 타입 불일치).
                1 => Some(None),
                // 선언과 다른 타입의 값.
                2 => {
                    let other = self.rng.pick(&VALUE_TYPES);
                    Some(Some(random_value(self.rng, other)))
                }
                _ if want.is_optional() && self.rng.one_in(3) => Some(None),
                _ => Some(Some(random_value(self.rng, want.value_type()))),
            };
            if let Some(value) = value {
                self.values.insert(target.clone(), value);
            }
            target
        };
        let id = self.id();
        self.push(ExprNode::reference(id, target), want)
    }

    /// 이미 만든 subtree 중 타입이 맞는 것(DAG 공유).
    fn reuse(&mut self, want: TypeSpec) -> Option<(ExprNodeId, TypeSpec)> {
        let candidates: Vec<(TypeSpec, ExprNodeId)> = self
            .pool
            .iter()
            .filter(|(spec, _)| {
                *spec == want || (want.is_optional() && spec.value_type() == want.value_type())
            })
            .copied()
            .collect();
        if candidates.is_empty() {
            return None;
        }
        let (spec, id) = candidates[self.rng.below(candidates.len())];
        Some((id, spec))
    }

    fn leaf(&mut self, want: TypeSpec) -> (ExprNodeId, TypeSpec) {
        if want.is_optional() && self.rng.one_in(3) {
            let id = self.id();
            return self.push(ExprNode::null(id, want.value_type()), want);
        }
        match self.rng.below(6) {
            0 | 1 => self.reference(want),
            2 => self
                .reuse(want)
                .unwrap_or_else(|| self.literal(want.value_type())),
            _ => self.literal(want.value_type()),
        }
    }

    /// 필수 값 타입 child.
    fn required(&mut self, t: ExpressionType, depth: u32) -> ExprNodeId {
        self.expr(TypeSpec::required(t), depth).0
    }

    /// `want` 타입의 subtree. 필수 타입을 원하면 필수 타입만 만듭니다(오염 제외).
    fn expr(&mut self, want: TypeSpec, depth: u32) -> (ExprNodeId, TypeSpec) {
        let want = if self.poison_one_in > 0 && self.rng.one_in(self.poison_one_in) {
            self.ill_typed = true;
            let t = self.rng.pick(&VALUE_TYPES);
            if self.rng.one_in(3) {
                TypeSpec::optional(t)
            } else {
                TypeSpec::required(t)
            }
        } else {
            want
        };
        if depth == 0 || self.budget == 0 || self.rng.one_in(5) {
            return self.leaf(want);
        }
        let d = depth - 1;
        let t = want.value_type();
        // 공통 생성: if·match(약 1/6).
        match self.rng.below(12) {
            0 => {
                let cond = self.required(ExpressionType::Boolean, d);
                let (then, a) = self.expr(want, d);
                let (otherwise, b) = self.expr(want, d);
                let spec = if a.is_optional() || b.is_optional() {
                    want.to_optional()
                } else {
                    TypeSpec::required(t)
                };
                let id = self.id();
                return self.push(ExprNode::if_else(id, cond, then, otherwise), spec);
            }
            1 => return self.match_expr(want, d),
            _ => {}
        }
        let id_spec = TypeSpec::required(t);
        match t {
            ExpressionType::Boolean => self.boolean(d),
            ExpressionType::String => match self.rng.below(3) {
                0 => {
                    let a = self.required(ExpressionType::String, d);
                    let b = self.required(ExpressionType::String, d);
                    self.call(FunctionId::Concat, &[a, b], id_spec)
                }
                1 => {
                    let a = self.required(ExpressionType::String, d);
                    let b = self.required(ExpressionType::String, d);
                    let c = self.required(ExpressionType::String, d);
                    self.call(FunctionId::Replace, &[a, b, c], id_spec)
                }
                _ => {
                    let a = self.required(ExpressionType::Number, d);
                    let digits = if self.rng.one_in(2) {
                        let digits = TypedValue::Number(
                            FiniteF32::new(self.rng.below(12) as f32).unwrap_or(FiniteF32::ZERO),
                        );
                        let id = self.id();
                        self.push(
                            ExprNode::literal(id, digits),
                            TypeSpec::required(ExpressionType::Number),
                        )
                        .0
                    } else {
                        self.required(ExpressionType::Number, d)
                    };
                    self.call(FunctionId::FormatNumber, &[a, digits], id_spec)
                }
            },
            ExpressionType::Color => match self.rng.below(4) {
                0 => {
                    let a = self.required(ExpressionType::Color, d);
                    let b = self.required(ExpressionType::Color, d);
                    let w = self.required(ExpressionType::Number, d);
                    self.call(FunctionId::Mix, &[a, b, w], id_spec)
                }
                1 => {
                    let a = self.required(ExpressionType::Color, d);
                    let w = self.required(ExpressionType::Number, d);
                    self.call(FunctionId::WithAlpha, &[a, w], id_spec)
                }
                2 => {
                    let a = self.required(ExpressionType::Color, d);
                    self.call(FunctionId::ToSrgb, &[a], id_spec)
                }
                _ => {
                    let a = self.required(ExpressionType::Color, d);
                    self.call(FunctionId::ToDisplayP3, &[a], id_spec)
                }
            },
            t if NUMERIC.contains(&t) => self.numeric(t, d),
            _ => self.leaf(want),
        }
    }

    fn call(
        &mut self,
        function: FunctionId,
        args: &[ExprNodeId],
        spec: TypeSpec,
    ) -> (ExprNodeId, TypeSpec) {
        let id = self.id();
        self.push(ExprNode::call(id, function, args), spec)
    }

    fn numeric(&mut self, t: ExpressionType, d: u32) -> (ExprNodeId, TypeSpec) {
        let spec = TypeSpec::required(t);
        match self.rng.below(9) {
            0..=2 => {
                let rules: Vec<_> = ARITHMETIC_RULES
                    .iter()
                    .filter(|rule| rule.result == t)
                    .collect();
                let rule = rules[self.rng.below(rules.len())];
                let op = match rule.op {
                    ArithmeticOp::Add => BinaryOp::Add,
                    ArithmeticOp::Subtract => BinaryOp::Subtract,
                    ArithmeticOp::Multiply => BinaryOp::Multiply,
                    ArithmeticOp::Divide => BinaryOp::Divide,
                    ArithmeticOp::Remainder => BinaryOp::Remainder,
                };
                let a = self.required(rule.lhs, d);
                let b = self.required(rule.rhs, d);
                let id = self.id();
                self.push(ExprNode::binary(id, op, a, b), spec)
            }
            3 => {
                let a = self.required(t, d);
                let id = self.id();
                self.push(ExprNode::unary(id, UnaryOp::Negate, a), spec)
            }
            4 => {
                let f = self.rng.pick(&[FunctionId::Min, FunctionId::Max]);
                let a = self.required(t, d);
                let b = self.required(t, d);
                self.call(f, &[a, b], spec)
            }
            5 => {
                let a = self.required(t, d);
                let b = self.required(t, d);
                let c = self.required(t, d);
                self.call(FunctionId::Clamp, &[a, b, c], spec)
            }
            6 => {
                let f = self.rng.pick(&[
                    FunctionId::Abs,
                    FunctionId::Round,
                    FunctionId::Floor,
                    FunctionId::Ceil,
                ]);
                let a = self.required(t, d);
                self.call(f, &[a], spec)
            }
            7 => {
                let a = self.required(t, d);
                let b = self.required(t, d);
                let w = self.required(ExpressionType::Number, d);
                self.call(FunctionId::Lerp, &[a, b, w], spec)
            }
            _ => {
                let u = self.rng.pick(&NUMERIC);
                let v = self.required(u, d);
                let i0 = self.required(u, d);
                let i1 = self.required(u, d);
                let o0 = self.required(t, d);
                let o1 = self.required(t, d);
                self.call(FunctionId::Remap, &[v, i0, i1, o0, o1], spec)
            }
        }
    }

    fn boolean(&mut self, d: u32) -> (ExprNodeId, TypeSpec) {
        let spec = TypeSpec::required(ExpressionType::Boolean);
        match self.rng.below(5) {
            0 => {
                // 동등 비교(nullable 허용).
                let t = self.rng.pick(&VALUE_TYPES);
                let lhs_spec = if self.rng.one_in(3) {
                    TypeSpec::optional(t)
                } else {
                    TypeSpec::required(t)
                };
                let (a, _) = self.expr(lhs_spec, d);
                let rhs_spec = if self.rng.one_in(3) {
                    TypeSpec::optional(t)
                } else {
                    TypeSpec::required(t)
                };
                let (b, _) = self.expr(rhs_spec, d);
                let op = self.rng.pick(&[BinaryOp::Equal, BinaryOp::NotEqual]);
                let id = self.id();
                self.push(ExprNode::binary(id, op, a, b), spec)
            }
            1 => {
                let t = self.rng.pick(&NUMERIC);
                let a = self.required(t, d);
                let b = self.required(t, d);
                let op = self.rng.pick(&[
                    BinaryOp::Less,
                    BinaryOp::LessEqual,
                    BinaryOp::Greater,
                    BinaryOp::GreaterEqual,
                ]);
                let id = self.id();
                self.push(ExprNode::binary(id, op, a, b), spec)
            }
            2 | 3 => {
                let a = self.required(ExpressionType::Boolean, d);
                let b = self.required(ExpressionType::Boolean, d);
                let op = self.rng.pick(&[BinaryOp::And, BinaryOp::Or]);
                let id = self.id();
                self.push(ExprNode::binary(id, op, a, b), spec)
            }
            _ => {
                let a = self.required(ExpressionType::Boolean, d);
                let id = self.id();
                self.push(ExprNode::unary(id, UnaryOp::Not, a), spec)
            }
        }
    }

    fn match_expr(&mut self, want: TypeSpec, d: u32) -> (ExprNodeId, TypeSpec) {
        let scrutinee_type = self.rng.pick(&MATCHABLE);
        let scrutinee = self.required(scrutinee_type, d);
        let arm_count = 1 + self.rng.below(4);
        let mut patterns: Vec<TypedValue> = Vec::new();
        for _ in 0..arm_count * 3 {
            if patterns.len() == arm_count {
                break;
            }
            let p = random_value(self.rng, scrutinee_type);
            if !patterns.contains(&p) {
                patterns.push(p);
            }
        }
        let mut optional = false;
        let mut arms = Vec::new();
        for p in patterns {
            let (arm, spec) = self.expr(want, d);
            optional |= spec.is_optional();
            arms.push((p, arm));
        }
        let (default, spec) = self.expr(want, d);
        optional |= spec.is_optional();
        let result = if optional {
            want.to_optional()
        } else {
            TypeSpec::required(want.value_type())
        };
        let id = self.id();
        self.push(ExprNode::match_on(id, scrutinee, arms, default), result)
    }
}

/// 생성된 case.
#[derive(Clone)]
struct Case {
    root: ExprNodeId,
    nodes: Vec<ExprNode>,
    types: BTreeMap<ReferenceTarget, TypeSpec>,
    values: BTreeMap<ReferenceTarget, Option<TypedValue>>,
    /// 타입 규칙을 지켜 만들었는지.
    well_typed: bool,
}

fn generate(rng: &mut Rng) -> Case {
    let max_nodes = if rng.one_in(10) {
        200 + rng.below(3_000)
    } else {
        1 + rng.below(120)
    };
    let depth = 1 + rng.below(14) as u32;
    let poison = if rng.one_in(4) { 6 + rng.below(20) } else { 0 };
    let want = {
        let t = rng.pick(&VALUE_TYPES);
        if rng.one_in(4) {
            TypeSpec::optional(t)
        } else {
            TypeSpec::required(t)
        }
    };
    let mut generator = Gen {
        rng,
        nodes: Vec::new(),
        next: 0,
        budget: max_nodes,
        pool: Vec::new(),
        types: BTreeMap::new(),
        values: BTreeMap::new(),
        ill_typed: false,
        poison_one_in: poison,
    };
    let (root, _) = generator.expr(want, depth);
    // 도달 불가 node 제거: 생성은 상향식이라 root에서 닿지 않는 subtree가 생기지 않지만, 재사용 후보만 남은
    // 경우를 대비해 root에서 닿는 node만 남깁니다.
    let nodes = reachable(root, generator.nodes);
    Case {
        root,
        nodes,
        types: generator.types,
        values: generator.values,
        well_typed: !generator.ill_typed,
    }
}

/// root에서 닿는 node만(원래 순서) 남깁니다.
fn reachable(root: ExprNodeId, nodes: Vec<ExprNode>) -> Vec<ExprNode> {
    let index: BTreeMap<ExprNodeId, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (node.id(), i))
        .collect();
    let mut keep = vec![false; nodes.len()];
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if let Some(&i) = index.get(&id) {
            if !keep[i] {
                keep[i] = true;
                stack.extend(nodes[i].children().iter().map(|child| child.target));
            }
        }
    }
    nodes
        .into_iter()
        .zip(keep)
        .filter_map(|(node, keep)| keep.then_some(node))
        .collect()
}

// --- 구조·byte 변형 ---

const SLOTS: [ChildSlot; 12] = [
    ChildSlot::Operand,
    ChildSlot::Lhs,
    ChildSlot::Rhs,
    ChildSlot::Argument(0),
    ChildSlot::Argument(1),
    ChildSlot::Argument(5),
    ChildSlot::Condition,
    ChildSlot::Then,
    ChildSlot::Else,
    ChildSlot::Scrutinee,
    ChildSlot::Arm(0),
    ChildSlot::Default,
];

fn random_payload(rng: &mut Rng) -> NodePayload {
    match rng.below(7) {
        0 => {
            let t = rng.pick(&VALUE_TYPES);
            NodePayload::Literal(Literal::Value(random_value(rng, t)))
        }
        1 => NodePayload::Literal(Literal::Null(rng.pick(&VALUE_TYPES))),
        2 => NodePayload::Unary(rng.pick(&UnaryOp::ALL)),
        3 => NodePayload::Binary(rng.pick(&BinaryOp::ALL)),
        4 => NodePayload::Call(rng.pick(&FunctionId::ALL)),
        5 => NodePayload::If,
        _ => {
            let t = rng.pick(&MATCHABLE);
            NodePayload::Match(vec![random_value(rng, t)])
        }
    }
}

/// node 목록에 구조 변형을 1~3회 적용합니다.
fn mutate_structure(rng: &mut Rng, case: &Case) -> (ExprNodeId, Vec<ExprNode>) {
    let mut nodes = case.nodes.clone();
    let mut root = case.root;
    if nodes.is_empty() {
        return (root, nodes);
    }
    for _ in 0..1 + rng.below(3) {
        let i = rng.below(nodes.len());
        let node = nodes[i].clone();
        let mut children: Vec<ChildRef> = node.children().to_vec();
        let mut payload = node.payload().clone();
        let mut id = node.id();
        match rng.below(8) {
            // child 대상을 임의 node로(cycle·도달 불가 유발).
            0 if !children.is_empty() => {
                let c = rng.below(children.len());
                children[c].target = nodes[rng.below(nodes.len())].id();
            }
            // slot 바꾸기.
            1 if !children.is_empty() => {
                let c = rng.below(children.len());
                children[c].slot = rng.pick(&SLOTS);
            }
            // child 삭제.
            2 if !children.is_empty() => {
                children.remove(rng.below(children.len()));
            }
            // child 복제·추가.
            3 => {
                let target = nodes[rng.below(nodes.len())].id();
                children.push(ChildRef::new(rng.pick(&SLOTS), target));
            }
            // payload 교체.
            4 => payload = random_payload(rng),
            // ID 중복.
            5 => id = nodes[rng.below(nodes.len())].id(),
            // root 변경.
            6 => root = nodes[rng.below(nodes.len())].id(),
            // 없는 child 대상.
            _ => children.push(ChildRef::new(
                rng.pick(&SLOTS),
                ExprNodeId::new(StableId::new_v7(0xEE, 0, rng.next() & 0xFFFF)),
            )),
        }
        nodes[i] = ExprNode::new(id, payload, children);
    }
    (root, nodes)
}

const BOUNDARY_BYTES: [u8; 6] = [0x00, 0x01, 0x7F, 0x80, 0xFE, 0xFF];

fn mutate_bytes(rng: &mut Rng, input: &[u8]) -> Vec<u8> {
    let mut bytes = input.to_vec();
    for _ in 0..1 + rng.below(6) {
        let len = bytes.len();
        match rng.below(8) {
            0 if len > 0 => {
                let i = rng.below(len);
                bytes[i] ^= 1 << rng.below(8);
            }
            1 if len > 0 => {
                let i = rng.below(len);
                bytes[i] = rng.pick(&BOUNDARY_BYTES);
            }
            2 => {
                let i = rng.below(len + 1);
                bytes.insert(i, rng.next() as u8);
            }
            3 if len > 0 => {
                let start = rng.below(len);
                let end = (start + 1 + rng.below(8)).min(len);
                bytes.drain(start..end);
            }
            4 if len >= 2 => {
                let i = rng.below(len - 1);
                let value = rng.pick(&[0u16, 1, 0x7FFF, 0xFFFF]);
                bytes[i..i + 2].copy_from_slice(&value.to_le_bytes());
            }
            5 if len >= 4 => {
                let i = rng.below(len - 3);
                let value = rng.pick(&[0u32, 1, 0xFFFF, 0x7FFF_FFFF, u32::MAX]);
                bytes[i..i + 4].copy_from_slice(&value.to_le_bytes());
            }
            6 if len > 0 => bytes.truncate(rng.below(len)),
            _ if len > 0 => {
                let start = rng.below(len);
                let end = (start + 1 + rng.below(16)).min(len);
                let chunk: Vec<u8> = bytes[start..end].to_vec();
                let at = rng.below(bytes.len() + 1);
                for (k, b) in chunk.into_iter().enumerate() {
                    bytes.insert(at + k, b);
                }
            }
            _ => {}
        }
        if bytes.len() > 1 << 20 {
            bytes.truncate(1 << 20);
        }
    }
    bytes
}

// --- 판정 ---

/// 통계.
#[derive(Debug, Default)]
struct Stats {
    inputs: usize,
    generated: usize,
    generated_well_typed: usize,
    structure_accepted: usize,
    structure_rejected: usize,
    bytes_accepted: usize,
    bytes_rejected: usize,
    compiled: usize,
    type_rejected: usize,
    evaluated_ok: usize,
    evaluated_err: usize,
    limit_errors: usize,
    budget_boundary_checks: usize,
    /// 무작위 한도 평가 중 한도 오류로 끝난 수.
    trial_limit_errors: usize,
    /// 무작위 한도 평가 중 기본 한도 결과와 같았던 수.
    trial_unchanged: usize,
    slowest: Duration,
}

/// 발견 하나.
struct Finding {
    kind: String,
    detail: String,
    bytes: Vec<u8>,
}

fn is_limit(kind: &EvalErrorKind) -> bool {
    matches!(
        kind,
        EvalErrorKind::InstructionBudgetExceeded { .. }
            | EvalErrorKind::CallDepthExceeded { .. }
            | EvalErrorKind::StackLimitExceeded { .. }
            | EvalErrorKind::ValueMemoryExceeded { .. }
    )
}

/// 검증된 program에서 나오면 안 되는 방어 오류.
fn is_defensive(kind: &EvalErrorKind) -> bool {
    matches!(
        kind,
        EvalErrorKind::OperandTypeMismatch
            | EvalErrorKind::MalformedProgram
            | EvalErrorKind::IllTyped
    )
}

/// 한도 오류가 설정한 한도 값을 보고하는지.
fn limit_matches(kind: &EvalErrorKind, limits: EvalLimits) -> bool {
    match kind {
        EvalErrorKind::InstructionBudgetExceeded { budget } => *budget == limits.instruction_budget,
        EvalErrorKind::CallDepthExceeded { limit } => *limit == limits.max_call_depth,
        EvalErrorKind::StackLimitExceeded { limit } => *limit == limits.max_stack_depth,
        EvalErrorKind::ValueMemoryExceeded { limit } => *limit == limits.max_value_bytes,
        _ => false,
    }
}

type VmResult = Result<(Option<TypedValue>, EvalTrace), EvalError>;
type AstResult = Result<Option<TypedValue>, EvalError>;

fn random_limits(rng: &mut Rng) -> EvalLimits {
    EvalLimits {
        instruction_budget: rng.below(400) as u64,
        max_call_depth: 1 + rng.below(8),
        max_stack_depth: 1 + rng.below(16),
        max_value_bytes: rng.below(4_096),
    }
}

/// 검증된 AST 하나에 typechecker·compiler·두 evaluator를 적용하고 성질을 확인합니다.
fn check_validated(
    rng: &mut Rng,
    ast: &ExpressionAst,
    case: &Case,
    expect_well_typed: bool,
    stats: &mut Stats,
) -> Result<(), String> {
    if let Some(path) = find_cycle(ast) {
        return Err(format!("검증된 AST에 cycle: {path:?}"));
    }
    let report = type_check(ast, &case.types);
    let compiled = Program::compile(ast, &case.types);
    if compiled.is_ok() != report.is_ok() {
        return Err(format!(
            "컴파일 결과({})와 type 검사 결과({})가 다름",
            compiled.is_ok(),
            report.is_ok()
        ));
    }
    let program = match compiled {
        Ok(program) => program,
        Err(error) => {
            stats.type_rejected += 1;
            if expect_well_typed {
                return Err(format!("타입을 지켜 만든 AST가 거부됨: {error}"));
            }
            return Ok(());
        }
    };
    stats.compiled += 1;
    if Some(program.result_type()) != report.root_type() {
        return Err(format!(
            "program 결과 타입 {:?} ≠ 추론 타입 {:?}",
            program.result_type(),
            report.root_type()
        ));
    }
    let defaults = EvalLimits::DEFAULT;
    let vm: VmResult = program.evaluate_with_limits(&case.values, defaults);
    let tree: AstResult = evaluate_ast_with_limits(ast, &case.types, &case.values, defaults);
    for (label, result) in [
        (
            "bytecode",
            vm.as_ref().map(|(v, _)| v.clone()).map_err(Clone::clone),
        ),
        ("AST", tree.clone()),
    ] {
        match &result {
            Ok(value) => {
                let spec = program.result_type();
                let conforms = match value {
                    None => spec.is_optional(),
                    Some(v) => spec.value_type().matches(v),
                };
                if !conforms {
                    return Err(format!(
                        "{label} 결과 {value:?}가 추론 타입 {spec:?}와 다름"
                    ));
                }
            }
            Err(error) => {
                if is_defensive(&error.kind) {
                    return Err(format!("{label} 평가에서 방어 오류: {error:?}"));
                }
                if is_limit(&error.kind) && !limit_matches(&error.kind, defaults) {
                    return Err(format!("{label} 한도 오류 값이 설정과 다름: {error:?}"));
                }
            }
        }
    }
    let vm_value: AstResult = vm.as_ref().map(|(v, _)| v.clone()).map_err(Clone::clone);
    let either_limit = matches!(&vm_value, Err(e) if is_limit(&e.kind))
        || matches!(&tree, Err(e) if is_limit(&e.kind));
    if either_limit {
        stats.limit_errors += 1;
    } else if vm_value != tree {
        return Err(format!(
            "bytecode 결과 {vm_value:?} ≠ AST 기준 결과 {tree:?}"
        ));
    }
    match &vm {
        Ok((_, trace)) => {
            stats.evaluated_ok += 1;
            budget_boundary(&program, case, &vm_value, trace, stats)?;
        }
        Err(_) => stats.evaluated_err += 1,
    }
    // 무작위 한도: 결과는 기본 한도 결과 또는 그 한도의 오류뿐.
    for _ in 0..LIMIT_TRIALS {
        let limits = random_limits(rng);
        let limited: AstResult = program
            .evaluate_with_limits(&case.values, limits)
            .map(|(v, _)| v);
        allowed_under_limits("bytecode", &limited, &vm_value, limits)?;
        let limited_tree = evaluate_ast_with_limits(ast, &case.types, &case.values, limits);
        allowed_under_limits("AST", &limited_tree, &tree, limits)?;
        for result in [&limited, &limited_tree] {
            match result {
                Err(error) if is_limit(&error.kind) => stats.trial_limit_errors += 1,
                _ => stats.trial_unchanged += 1,
            }
        }
    }
    Ok(())
}

fn allowed_under_limits(
    label: &str,
    limited: &AstResult,
    unlimited: &AstResult,
    limits: EvalLimits,
) -> Result<(), String> {
    if limited == unlimited {
        return Ok(());
    }
    match limited {
        Err(error) if is_limit(&error.kind) && limit_matches(&error.kind, limits) => Ok(()),
        // 기본 한도에서 한도 오류였다면 작은 한도에서는 다른 한도 오류가 먼저 날 수 있습니다.
        Err(error)
            if is_limit(&error.kind)
                && matches!(unlimited, Err(e) if is_limit(&e.kind)) =>
        {
            Ok(())
        }
        _ => Err(format!(
            "{label}: 한도 {limits:?}에서 결과 {limited:?}가 기본 결과 {unlimited:?}와 다르고 한도 오류도 아님"
        )),
    }
}

/// 성공한 평가의 명령 수·stack peak 경계에서 한도가 정확히 작동하는지.
fn budget_boundary(
    program: &Program,
    case: &Case,
    expected: &AstResult,
    trace: &EvalTrace,
    stats: &mut Stats,
) -> Result<(), String> {
    stats.budget_boundary_checks += 1;
    let n = trace.instructions;
    let exact = EvalLimits {
        instruction_budget: n,
        ..EvalLimits::DEFAULT
    };
    let at = program.evaluate_with_limits(&case.values, exact);
    if at.as_ref().map(|(v, _)| v.clone()).map_err(Clone::clone) != *expected {
        return Err(format!("budget = 명령 수 {n}에서 결과가 달라짐: {at:?}"));
    }
    if let Ok((_, t)) = &at {
        if t.instructions != n {
            return Err(format!(
                "같은 program의 명령 수가 다름: {n} vs {}",
                t.instructions
            ));
        }
    }
    if n > 0 {
        let below = EvalLimits {
            instruction_budget: n - 1,
            ..EvalLimits::DEFAULT
        };
        match program.evaluate_with_limits(&case.values, below) {
            Err(EvalError {
                kind: EvalErrorKind::InstructionBudgetExceeded { budget },
                ..
            }) if budget == n - 1 => {}
            other => {
                return Err(format!(
                    "budget = 명령 수 − 1 ({})에서 budget 오류가 아님: {other:?}",
                    n - 1
                ))
            }
        }
    }
    let peak = trace.peak_stack_depth;
    let stack_exact = EvalLimits {
        max_stack_depth: peak,
        ..EvalLimits::DEFAULT
    };
    let at_peak: AstResult = program
        .evaluate_with_limits(&case.values, stack_exact)
        .map(|(v, _)| v);
    if at_peak != *expected {
        return Err(format!(
            "stack 한도 = peak {peak}에서 결과가 달라짐: {at_peak:?}"
        ));
    }
    if peak > 0 {
        let stack_below = EvalLimits {
            max_stack_depth: peak - 1,
            ..EvalLimits::DEFAULT
        };
        match program.evaluate_with_limits(&case.values, stack_below) {
            Err(EvalError {
                kind: EvalErrorKind::StackLimitExceeded { limit },
                ..
            }) if limit == peak - 1 => {}
            other => {
                return Err(format!(
                    "stack 한도 = peak − 1 ({})에서 stack 오류가 아님: {other:?}",
                    peak - 1
                ))
            }
        }
    }
    if trace.instructions > EvalLimits::DEFAULT.instruction_budget
        || trace.peak_stack_depth > EvalLimits::DEFAULT.max_stack_depth
        || trace.peak_value_bytes > EvalLimits::DEFAULT.max_value_bytes
    {
        return Err(format!("trace가 기본 한도를 넘음: {trace:?}"));
    }
    Ok(())
}

/// 생성 case 하나와 그 변형들을 검사합니다. 발견은 `findings`에 쌓습니다.
fn run_case(rng: &mut Rng, case: &Case, stats: &mut Stats, findings: &mut Vec<Finding>) {
    let mut record = |kind: &str, detail: String, bytes: Vec<u8>| {
        findings.push(Finding {
            kind: kind.to_string(),
            detail,
            bytes,
        });
    };
    // 1. 생성 AST.
    stats.generated += 1;
    stats.inputs += 1;
    if case.well_typed {
        stats.generated_well_typed += 1;
    }
    let started = Instant::now();
    let mut local = rng.clone();
    let outcome = quietly(|| -> Result<Option<Vec<u8>>, String> {
        let ast = ExpressionAst::new(case.root, case.nodes.clone())
            .map_err(|error| format!("생성 AST 검증 실패: {error}"))?;
        let bytes = ast
            .to_canonical_bytes()
            .map_err(|error| format!("직렬화 실패: {error}"))?;
        let restored = ExpressionAst::from_canonical_bytes(&bytes)
            .map_err(|error| format!("canonical bytes 복원 실패: {error}"))?;
        if restored != ast {
            return Err("canonical round-trip AST 불일치".to_string());
        }
        if restored.to_canonical_bytes().ok().as_deref() != Some(bytes.as_slice()) {
            return Err("canonical 재직렬화 bytes 불일치".to_string());
        }
        check_validated(&mut local, &ast, case, case.well_typed, stats)?;
        Ok(Some(bytes))
    });
    *rng = local;
    stats.slowest = stats.slowest.max(started.elapsed());
    if started.elapsed() > INPUT_TIME_LIMIT {
        record(
            "slow",
            format!("생성 AST {:?}", started.elapsed()),
            Vec::new(),
        );
    }
    let bytes = match outcome {
        Ok(Ok(Some(bytes))) => bytes,
        Ok(Ok(None)) => return,
        Ok(Err(detail)) => {
            record("property", detail, Vec::new());
            return;
        }
        Err(panic) => {
            record("panic", panic, Vec::new());
            return;
        }
    };
    // 2. 구조 변형.
    for _ in 0..STRUCTURE_MUTATIONS {
        stats.inputs += 1;
        let (root, nodes) = mutate_structure(rng, case);
        let started = Instant::now();
        let mut local = rng.clone();
        let outcome = quietly(|| -> Result<(), String> {
            for node in &nodes {
                if let Ok(node_bytes) = node.to_bytes() {
                    let decoded = ExprNode::from_bytes(&node_bytes)
                        .map_err(|error| format!("node bytes 복원 실패: {error}"))?;
                    if decoded != *node {
                        return Err("node bytes round-trip 불일치".to_string());
                    }
                }
            }
            match ExpressionAst::new(root, nodes.clone()) {
                Ok(ast) => {
                    stats.structure_accepted += 1;
                    let bytes = ast
                        .to_canonical_bytes()
                        .map_err(|error| format!("직렬화 실패: {error}"))?;
                    let restored = ExpressionAst::from_canonical_bytes(&bytes)
                        .map_err(|error| format!("복원 실패: {error}"))?;
                    if restored != ast {
                        return Err("변형 AST canonical round-trip 불일치".to_string());
                    }
                    check_validated(&mut local, &ast, case, false, stats)
                }
                Err(_) => {
                    stats.structure_rejected += 1;
                    Ok(())
                }
            }
        });
        *rng = local;
        stats.slowest = stats.slowest.max(started.elapsed());
        match outcome {
            Ok(Ok(())) => {}
            Ok(Err(detail)) => record("property", format!("구조 변형: {detail}"), bytes.clone()),
            Err(panic) => record("panic", format!("구조 변형: {panic}"), bytes.clone()),
        }
    }
    // 3. canonical bytes 변형.
    for _ in 0..BYTE_MUTATIONS {
        stats.inputs += 1;
        let mutated = mutate_bytes(rng, &bytes);
        let started = Instant::now();
        let mut local = rng.clone();
        let outcome = quietly(|| -> Result<(), String> {
            let _ = ExprNode::from_bytes(&mutated);
            match ExpressionAst::from_canonical_bytes(&mutated) {
                Ok(ast) => {
                    stats.bytes_accepted += 1;
                    let again = ast
                        .to_canonical_bytes()
                        .map_err(|error| format!("수용 입력 재직렬화 실패: {error}"))?;
                    if again != mutated {
                        return Err(
                            "수용한 bytes의 재인코딩이 입력과 다름(canonical 아님)".to_string()
                        );
                    }
                    check_validated(&mut local, &ast, case, false, stats)
                }
                Err(_) => {
                    stats.bytes_rejected += 1;
                    Ok(())
                }
            }
        });
        *rng = local;
        let elapsed = started.elapsed();
        stats.slowest = stats.slowest.max(elapsed);
        if elapsed > INPUT_TIME_LIMIT {
            record("slow", format!("bytes 변형 {elapsed:?}"), mutated.clone());
        }
        match outcome {
            Ok(Ok(())) => {}
            Ok(Err(detail)) => record("property", format!("bytes 변형: {detail}"), mutated),
            Err(panic) => record("panic", format!("bytes 변형: {panic}"), mutated),
        }
    }
}

// --- 고정 stress 입력 ---

fn sid(n: u64) -> ExprNodeId {
    ExprNodeId::new(StableId::new_v7(0xB0, 0, n))
}

fn number(v: f32) -> TypedValue {
    TypedValue::Number(FiniteF32::new(v).unwrap_or(FiniteF32::ZERO))
}

/// stress 규모. release 빌드 또는 `STELLAR_EXPRESSION_FUZZ_STRESS=full`이면 전체 규모(기본 한도를 넘는 60만 단계
/// 등), debug 기본 실행은 축소 규모입니다(debug에서 60만 node 검증·평가는 1분 이상 걸립니다).
fn full_stress() -> bool {
    !cfg!(debug_assertions)
        || std::env::var("STELLAR_EXPRESSION_FUZZ_STRESS").is_ok_and(|v| v == "full")
}

/// 고정 stress case 하나: 이름, root, node, 주 평가 한도.
struct StressCase {
    name: &'static str,
    root: ExprNodeId,
    nodes: Vec<ExprNode>,
    limits: EvalLimits,
}

fn stress_cases(full: bool) -> Vec<StressCase> {
    let mut out = Vec::new();
    let defaults = EvalLimits::DEFAULT;
    // 왼쪽 중첩 `((c + c) + c) …`: 단계마다 명령 2개. 전체 규모 60만 단계(명령 120만 개)는 기본 budget(100만)을,
    // 축소 규모 6만 단계(12만 개)는 budget 10만을 넘습니다.
    let (levels, budget) = if full {
        (600_000u64, defaults.instruction_budget)
    } else {
        (60_000u64, 100_000)
    };
    let mut nodes: Vec<ExprNode> = (1..=levels)
        .map(|n| ExprNode::binary(sid(n), BinaryOp::Add, sid(n + 1), sid(0)))
        .collect();
    nodes.push(ExprNode::literal(sid(levels + 1), number(1.0)));
    nodes.push(ExprNode::literal(sid(0), number(0.0)));
    out.push(StressCase {
        name: "left_nested",
        root: sid(1),
        nodes,
        limits: EvalLimits {
            instruction_budget: budget,
            ..defaults
        },
    });
    // 부호 반전 사슬(전체 15만, 축소 3만).
    let levels = if full { 150_000u64 } else { 30_000 };
    let mut nodes: Vec<ExprNode> = (1..=levels)
        .map(|n| ExprNode::unary(sid(n), UnaryOp::Negate, sid(n + 1)))
        .collect();
    nodes.push(ExprNode::literal(sid(levels + 1), number(2.5)));
    out.push(StressCase {
        name: "negate_chain",
        root: sid(1),
        nodes,
        limits: defaults,
    });
    // 300단계 공유 block 중첩(frame 깊이 300 > 256).
    let levels = 300u64;
    let mut nodes: Vec<ExprNode> = (1..=levels)
        .map(|n| ExprNode::binary(sid(n), BinaryOp::Add, sid(n + 1), sid(n + 1)))
        .collect();
    nodes.push(ExprNode::literal(sid(levels + 1), number(0.0)));
    out.push(StressCase {
        name: "shared_300",
        root: sid(1),
        nodes,
        limits: defaults,
    });
    // 7만 단계 오른쪽 중첩(operand stack 7만 > 기본 65,536).
    let levels = 70_000u64;
    let mut nodes: Vec<ExprNode> = (1..=levels)
        .map(|n| ExprNode::binary(sid(n), BinaryOp::Add, sid(0), sid(n + 1)))
        .collect();
    nodes.push(ExprNode::literal(sid(levels + 1), number(1.0)));
    nodes.push(ExprNode::literal(sid(0), number(1.0)));
    out.push(StressCase {
        name: "right_nested_70k",
        root: sid(1),
        nodes,
        limits: defaults,
    });
    // `if` 중첩(전체 10만, 축소 3만).
    let levels = if full { 100_000u64 } else { 30_000 };
    let mut nodes: Vec<ExprNode> = (1..=levels)
        .map(|n| ExprNode::if_else(sid(n), sid(0), sid(n + 1), sid(1_000_000)))
        .collect();
    nodes.push(ExprNode::literal(sid(levels + 1), number(4.0)));
    nodes.push(ExprNode::literal(sid(0), TypedValue::Boolean(true)));
    nodes.push(ExprNode::literal(sid(1_000_000), number(5.0)));
    out.push(StressCase {
        name: "if_chain",
        root: sid(1),
        nodes,
        limits: defaults,
    });
    // arm이 많은 `match`(전체 6만, 축소 2만). 마지막 arm이 일치합니다.
    let arms = if full { 60_000u64 } else { 20_000 };
    let nodes = vec![
        ExprNode::match_on(
            sid(1),
            sid(2),
            (0..arms).map(|k| (number(k as f32), sid(3))).collect(),
            sid(4),
        ),
        ExprNode::literal(sid(2), number((arms - 1) as f32)),
        ExprNode::literal(sid(3), number(7.0)),
        ExprNode::literal(sid(4), number(8.0)),
    ];
    out.push(StressCase {
        name: "match_arms",
        root: sid(1),
        nodes,
        limits: defaults,
    });
    // 공유 concat 증폭: s(k+1) = concat(s(k), s(k)), 64 bytes × 2^24.
    let levels = 24u64;
    let mut nodes: Vec<ExprNode> = (1..=levels)
        .map(|n| ExprNode::call(sid(n), FunctionId::Concat, &[sid(n + 1), sid(n + 1)]))
        .collect();
    nodes.push(ExprNode::literal(
        sid(levels + 1),
        TypedValue::String("가".repeat(21) + "x"),
    ));
    out.push(StressCase {
        name: "concat_doubling_24",
        root: sid(1),
        nodes,
        limits: defaults,
    });
    out
}

fn run_stress(stats: &mut Stats, findings: &mut Vec<Finding>) -> Vec<(String, String, Duration)> {
    let mut summary = Vec::new();
    let mut rng = Rng(SEED ^ 0x5354_5245_5353);
    for stress in stress_cases(full_stress()) {
        let name = stress.name;
        let limits = stress.limits;
        stats.inputs += 1;
        let case = Case {
            root: stress.root,
            nodes: stress.nodes,
            types: BTreeMap::new(),
            values: BTreeMap::new(),
            well_typed: true,
        };
        let started = Instant::now();
        let outcome = quietly(|| -> Result<String, String> {
            let ast = ExpressionAst::new(case.root, case.nodes.clone())
                .map_err(|error| format!("검증 실패: {error}"))?;
            let bytes = ast
                .to_canonical_bytes()
                .map_err(|error| format!("직렬화 실패: {error}"))?;
            let restored = ExpressionAst::from_canonical_bytes(&bytes)
                .map_err(|error| format!("복원 실패: {error}"))?;
            if restored != ast {
                return Err("round-trip 불일치".to_string());
            }
            let program = Program::compile(&ast, &case.types)
                .map_err(|error| format!("컴파일 실패: {error}"))?;
            let vm = program.evaluate_with_limits(&case.values, limits);
            let outcome = match &vm {
                Ok((value, trace)) => {
                    format!("ok({value:?}, 명령 {})", trace.instructions)
                }
                Err(error) => format!("{:?}", error.kind),
            };
            if let Err(error) = &vm {
                if is_limit(&error.kind) && !limit_matches(&error.kind, limits) {
                    return Err(format!("한도 오류 값이 설정과 다름: {error:?}"));
                }
            }
            // 작은 budget에서도 결과는 주 한도 결과 또는 그 budget 오류뿐입니다.
            let small = EvalLimits {
                instruction_budget: 1_000,
                ..limits
            };
            let unlimited: AstResult = vm.as_ref().map(|(v, _)| v.clone()).map_err(Clone::clone);
            let limited: AstResult = program
                .evaluate_with_limits(&case.values, small)
                .map(|(v, _)| v);
            allowed_under_limits("bytecode budget 1,000", &limited, &unlimited, small)?;
            check_validated(&mut rng, &ast, &case, true, stats)?;
            Ok(outcome)
        });
        let elapsed = started.elapsed();
        stats.slowest = stats.slowest.max(elapsed);
        if elapsed > STRESS_TIME_LIMIT {
            findings.push(Finding {
                kind: "slow".to_string(),
                detail: format!("{name}: {elapsed:?}"),
                bytes: Vec::new(),
            });
        }
        match outcome {
            Ok(Ok(outcome)) => summary.push((name.to_string(), outcome, elapsed)),
            Ok(Err(detail)) => findings.push(Finding {
                kind: "property".to_string(),
                detail: format!("{name}: {detail}"),
                bytes: Vec::new(),
            }),
            Err(panic) => findings.push(Finding {
                kind: "panic".to_string(),
                detail: format!("{name}: {panic}"),
                bytes: Vec::new(),
            }),
        }
    }
    summary
}

// --- 캠페인 ---

fn iterations() -> usize {
    std::env::var("STELLAR_EXPRESSION_FUZZ_ITERATIONS")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(DEFAULT_ITERATIONS)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn save(findings: &[Finding]) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("expression-fuzz");
    // 이전 실행의 발견 파일과 섞이지 않게 비웁니다.
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    for (i, finding) in findings.iter().enumerate() {
        let _ = std::fs::write(
            dir.join(format!("{i:04}-{}.txt", finding.kind)),
            format!("{}\n{}\n", finding.detail, hex(&finding.bytes)),
        );
    }
    dir
}

/// watchdog 아래 worker thread(기본 2 MiB stack)에서 `f`를 실행합니다.
fn with_watchdog<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    install_quiet_panic_hook();
    let (sender, receiver) = mpsc::channel();
    let worker = std::thread::Builder::new()
        .name("expression-fuzz".to_string())
        .spawn(move || {
            let _ = sender.send(f());
        })
        .expect("worker thread");
    match receiver.recv_timeout(WATCHDOG) {
        Ok(value) => {
            let _ = worker.join();
            value
        }
        Err(_) => panic!("fuzz campaign이 {WATCHDOG:?} 안에 끝나지 않았습니다(hang)"),
    }
}

#[test]
fn expression_fuzz_campaign_has_no_findings() {
    let iterations = iterations();
    let (stats, findings) = with_watchdog(move || {
        let mut stats = Stats::default();
        let mut findings = Vec::new();
        let mut rng = Rng(SEED);
        for _ in 0..iterations {
            let case = generate(&mut rng);
            run_case(&mut rng, &case, &mut stats, &mut findings);
        }
        (stats, findings)
    });
    println!("expression fuzz: {iterations}회 {stats:?}");
    if !findings.is_empty() {
        let dir = save(&findings);
        let listed: Vec<String> = findings
            .iter()
            .take(10)
            .map(|f| format!("[{}] {}", f.kind, f.detail))
            .collect();
        panic!(
            "발견 {}건(저장: {}):\n{}",
            findings.len(),
            dir.display(),
            listed.join("\n")
        );
    }
    assert!(
        stats.compiled > iterations / 2,
        "평가까지 간 입력이 충분해야 합니다"
    );
    assert!(stats.budget_boundary_checks > iterations / 4);
    assert!(stats.trial_limit_errors > 0 && stats.trial_unchanged > 0);
    assert!(stats.bytes_accepted > 0 && stats.bytes_rejected > 0);
    assert!(stats.structure_accepted > 0 && stats.structure_rejected > 0);
    assert!(stats.type_rejected > 0, "타입 오류 경로도 실행");
}

#[test]
fn stress_inputs_stop_at_limits_without_panic() {
    let (stats, findings, summary) = with_watchdog(|| {
        let mut stats = Stats::default();
        let mut findings = Vec::new();
        let summary = run_stress(&mut stats, &mut findings);
        (stats, findings, summary)
    });
    for (name, outcome, elapsed) in &summary {
        println!("stress {name}: {outcome} ({elapsed:?})");
    }
    if !findings.is_empty() {
        let dir = save(&findings);
        let listed: Vec<String> = findings
            .iter()
            .map(|f| format!("[{}] {}", f.kind, f.detail))
            .collect();
        panic!(
            "stress 발견(저장: {}):\n{}",
            dir.display(),
            listed.join("\n")
        );
    }
    let outcome = |name: &str| -> &str {
        summary
            .iter()
            .find(|(n, _, _)| n == name)
            .map(|(_, o, _)| o.as_str())
            .unwrap_or("")
    };
    // 한도가 비정상적으로 큰 표현식을 typed 오류로 끝냅니다(전체 규모의 left_nested는 기본 budget).
    assert!(outcome("left_nested").starts_with("InstructionBudgetExceeded"));
    assert!(outcome("shared_300").starts_with("CallDepthExceeded { limit: 256 }"));
    assert!(outcome("right_nested_70k").starts_with("StackLimitExceeded { limit: 65536 }"));
    assert!(
        outcome("concat_doubling_24").starts_with("StringTooLong")
            || outcome("concat_doubling_24").starts_with("ValueMemoryExceeded")
    );
    assert!(outcome("negate_chain").starts_with("ok("));
    assert!(outcome("if_chain").starts_with("ok("));
    assert!(outcome("match_arms").starts_with("ok(Some(Number(FiniteF32(7.0)))"));
    assert_eq!(stats.inputs, summary.len());
}

#[test]
fn campaign_inputs_are_deterministic() {
    let digest = |seed: u64| {
        let mut rng = Rng(seed);
        let mut out = Vec::new();
        for _ in 0..40 {
            let case = generate(&mut rng);
            let bytes = ExpressionAst::new(case.root, case.nodes.clone())
                .ok()
                .and_then(|ast| ast.to_canonical_bytes().ok())
                .unwrap_or_default();
            out.push(mutate_bytes(&mut rng, &bytes));
            out.push(bytes);
        }
        out
    };
    assert_eq!(digest(SEED), digest(SEED));
    assert_ne!(digest(SEED), digest(SEED ^ 1));
}
