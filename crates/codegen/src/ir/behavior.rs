//! 선언형 상태 머신과 `MotionSpec`의 IR (XXII.1.5, ADR-0106 결정 1.2·1.3).
//!
//! 문서 값(IX.5 상태 머신, XXX.1.1 `MotionSpec`)을 그대로 옮긴 지역 타입입니다. timing function parameter와 reduced
//! motion 대체 동작은 값 그대로(f32 bit 그대로) 보존합니다. 표현식은 닫힌 트리 [`IrExpr`]이고 연산 기호·함수 이름은
//! expressions crate가 정한 문자열을 그대로 씁니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_core_types::typed_value::TypedValue;

/// 표현식 최대 깊이(root = 1). 이보다 깊은 식은 IR에 넣지 않습니다.
pub const MAX_EXPR_DEPTH: usize = 128;

/// 닫힌 표현식 트리.
#[derive(Debug, Clone, PartialEq)]
pub enum IrExpr {
    /// 값.
    Literal(TypedValue),
    /// `Optional`의 값 없음.
    Null,
    /// 상태 변수.
    State(StableId),
    /// 토큰.
    Token(StableId),
    /// 다른 속성 값.
    Property(PropertyPath),
    /// Component Property.
    ComponentProperty(StableId),
    /// 환경 입력(numeric key).
    Environment(u32),
    /// Context key(XXII.1.6).
    Context(StableId),
    /// 단항 연산(`neg`·`not`).
    Unary {
        /// 연산 기호.
        op: &'static str,
        /// 피연산자.
        operand: Box<IrExpr>,
    },
    /// 이항 연산(`+`·`-`·`*`·`/`·`%`·`==`·`!=`·`<`·`<=`·`>`·`>=`·`and`·`or`).
    Binary {
        /// 연산 기호.
        op: &'static str,
        /// 왼쪽.
        lhs: Box<IrExpr>,
        /// 오른쪽.
        rhs: Box<IrExpr>,
    },
    /// 내장 함수 호출.
    Call {
        /// 함수 이름.
        function: &'static str,
        /// 인자.
        args: Vec<IrExpr>,
    },
    /// `if`.
    If {
        /// 조건.
        condition: Box<IrExpr>,
        /// 참.
        then: Box<IrExpr>,
        /// 거짓.
        otherwise: Box<IrExpr>,
    },
    /// `match`.
    Match {
        /// 검사 값.
        scrutinee: Box<IrExpr>,
        /// (pattern 값, 결과).
        arms: Vec<(TypedValue, IrExpr)>,
        /// 기본 결과.
        default: Box<IrExpr>,
    },
}

impl IrExpr {
    /// 직접 자식(순서대로).
    pub fn children(&self) -> Vec<&IrExpr> {
        match self {
            IrExpr::Unary { operand, .. } => vec![operand],
            IrExpr::Binary { lhs, rhs, .. } => vec![lhs, rhs],
            IrExpr::Call { args, .. } => args.iter().collect(),
            IrExpr::If {
                condition,
                then,
                otherwise,
            } => vec![condition, then, otherwise],
            IrExpr::Match {
                scrutinee,
                arms,
                default,
            } => {
                let mut out: Vec<&IrExpr> = vec![scrutinee];
                out.extend(arms.iter().map(|(_, e)| e));
                out.push(default);
                out
            }
            _ => Vec::new(),
        }
    }

    /// 깊이(root = 1). 재귀 없이 셉니다.
    pub fn depth(&self) -> usize {
        let mut max = 0;
        let mut stack = vec![(self, 1usize)];
        while let Some((expr, depth)) = stack.pop() {
            max = max.max(depth);
            stack.extend(expr.children().into_iter().map(|c| (c, depth + 1)));
        }
        max
    }

    /// 식이 읽는 상태 변수(중복 포함, 방문 순서).
    pub fn states(&self) -> Vec<StableId> {
        self.collect(|e| match e {
            IrExpr::State(id) => Some(*id),
            _ => None,
        })
    }

    /// 식이 읽는 Context key.
    pub fn contexts(&self) -> Vec<StableId> {
        self.collect(|e| match e {
            IrExpr::Context(id) => Some(*id),
            _ => None,
        })
    }

    /// 식이 읽는 토큰.
    pub fn tokens(&self) -> Vec<StableId> {
        self.collect(|e| match e {
            IrExpr::Token(id) => Some(*id),
            _ => None,
        })
    }

    fn collect(&self, pick: impl Fn(&IrExpr) -> Option<StableId>) -> Vec<StableId> {
        let mut out = Vec::new();
        let mut stack = vec![self];
        while let Some(expr) = stack.pop() {
            if let Some(id) = pick(expr) {
                out.push(id);
            }
            stack.extend(expr.children().into_iter().rev());
        }
        out
    }
}

/// 상태 머신·MotionSpec 소유자.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IrOwner {
    /// ComponentDefinition.
    Component(StableId),
    /// ViewDefinition.
    View(StableId),
}

/// transition source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrTrigger {
    /// node에서 난 이벤트.
    Event {
        /// 이벤트 종류의 안정 이름(문서 `EventKind::name`).
        kind: &'static str,
        /// 이벤트를 내는 node.
        source: StableId,
    },
    /// nested instance가 emit한 공개 semantic action.
    NestedAction {
        /// nested instance node.
        instance: StableId,
        /// action.
        action: StableId,
    },
}

/// transition action.
#[derive(Debug, Clone, PartialEq)]
pub enum IrAction {
    /// 상태 변수 set.
    SetState {
        /// 변수.
        variable: StableId,
        /// 값 식.
        value: IrExpr,
    },
    /// semantic action emit.
    Emit {
        /// action.
        action: StableId,
    },
}

/// transition 하나.
#[derive(Debug, Clone, PartialEq)]
pub struct IrTransition {
    /// 식별자.
    pub id: StableId,
    /// source.
    pub trigger: IrTrigger,
    /// 조건식(없으면 항상 참).
    pub condition: Option<IrExpr>,
    /// 순서 있는 action.
    pub actions: Vec<IrAction>,
}

/// semantic action 선언.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrSemanticAction {
    /// 식별자.
    pub id: StableId,
    /// 이름.
    pub name: String,
}

/// 상태 머신 하나(입력이자 IR 값). 변수는 IR 상태 변수 선언([`super::IrState`])의 id입니다.
#[derive(Debug, Clone, PartialEq)]
pub struct IrStateMachine {
    /// 소유자.
    pub owner: IrOwner,
    /// 이 머신의 상태 변수(id 오름차순).
    pub variables: Vec<StableId>,
    /// 선언한 semantic action(id 오름차순).
    pub actions: Vec<IrSemanticAction>,
    /// transition(입력 순서 = 평가 순서).
    pub transitions: Vec<IrTransition>,
}

/// `steps()` jump 위치.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IrStepPosition {
    /// `jump-start`.
    JumpStart,
    /// `jump-end`.
    JumpEnd,
    /// `jump-none`.
    JumpNone,
    /// `jump-both`.
    JumpBoth,
}

impl IrStepPosition {
    /// CSS 정규 이름.
    pub const fn css_name(self) -> &'static str {
        match self {
            IrStepPosition::JumpStart => "jump-start",
            IrStepPosition::JumpEnd => "jump-end",
            IrStepPosition::JumpNone => "jump-none",
            IrStepPosition::JumpBoth => "jump-both",
        }
    }
}

/// timing function(문서 값 그대로).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IrTiming {
    /// `linear`.
    Linear,
    /// `cubic-bezier(x1, y1, x2, y2)`.
    CubicBezier {
        /// x1.
        x1: f32,
        /// y1.
        y1: f32,
        /// x2.
        x2: f32,
        /// y2.
        y2: f32,
    },
    /// `steps(count, position)`.
    Steps {
        /// 단계 수.
        count: u32,
        /// jump 위치.
        position: IrStepPosition,
    },
    /// 감쇠 spring.
    Spring {
        /// 질량.
        mass: f32,
        /// 강성.
        stiffness: f32,
        /// 감쇠 계수.
        damping: f32,
        /// 초기 속도.
        initial_velocity: f32,
    },
}

/// reduced motion 대체 동작.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IrReducedMotion {
    /// 즉시 끝값.
    Instant,
    /// 그대로 재생.
    Play,
}

impl IrReducedMotion {
    /// 안정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            IrReducedMotion::Instant => "instant",
            IrReducedMotion::Play => "play",
        }
    }
}

/// motion 시작 조건.
#[derive(Debug, Clone, PartialEq)]
pub enum IrMotionStart {
    /// 변수 값이 `value`가 되는 tick.
    Becomes {
        /// 상태 변수.
        state: StableId,
        /// 값.
        value: TypedValue,
    },
    /// 변수 값이 바뀌는 tick.
    Changes {
        /// 상태 변수.
        state: StableId,
    },
}

impl IrMotionStart {
    /// 조건의 상태 변수.
    pub const fn state(&self) -> StableId {
        match self {
            IrMotionStart::Becomes { state, .. } | IrMotionStart::Changes { state } => *state,
        }
    }
}

/// `MotionSpec` 하나(입력이자 IR 값).
#[derive(Debug, Clone, PartialEq)]
pub struct IrMotion {
    /// 식별자.
    pub id: StableId,
    /// 소유자.
    pub owner: IrOwner,
    /// 대상 속성(입력 순서, 제외 node 대상은 빠짐).
    pub targets: Vec<PropertyPath>,
    /// 시작 조건.
    pub start: IrMotionStart,
    /// 지속 시간(ms).
    pub duration_ms: f32,
    /// 지연(ms).
    pub delay_ms: f32,
    /// timing function.
    pub timing: IrTiming,
    /// reduced motion 대체 동작.
    pub reduced_motion: IrReducedMotion,
}
