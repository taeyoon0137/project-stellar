//! `MotionSpec` record와 검증 (XXX.1.1, ADR-0025 결정 1.2·2.3, ADR-0094 결정 1).
//!
//! `MotionSpec`은 CSS `transition`과 같은 뜻의 시간축 원본입니다. 대상 속성 목록(`transition-property`), 지속 시간
//! (`transition-duration`), 지연(`transition-delay`), timing function(`transition-timing-function`)을 가지고, 시작 조건은
//! CSS의 "계산 값이 바뀜" 대신 상태 변수 하나의 값 조건입니다. 보간 시작값은 그 순간 표시 중인 값, 끝값은 상태가
//! 바뀐 뒤 대상 binding이 계산한 값입니다(ADR-0094 결정 6).
//!
//! timing function은 닫힌 집합입니다: `linear`, `cubic-bezier(x1, y1, x2, y2)`, `steps(n, <position>)`, 그리고 CSS에
//! 없는 `spring(mass stiffness damping velocity)`. 범위를 벗어난 parameter는 [`MotionSpec::new`]가 거부합니다.

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_core_types::reference::ComponentRef;
use stellar_core_types::typed_value::TypedValue;
use stellar_expressions::types::ExpressionType;

use crate::component::state_machine::StateVariableId;

/// MotionSpec 식별자.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MotionSpecId(StableId);

impl MotionSpecId {
    /// `StableId`로부터 만듭니다.
    pub const fn new(id: StableId) -> MotionSpecId {
        MotionSpecId(id)
    }

    /// 내부 `StableId`.
    pub const fn stable_id(&self) -> StableId {
        self.0
    }
}

impl fmt::Display for MotionSpecId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// MotionSpec 소유자.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MotionOwner {
    /// ComponentDefinition(대상은 정의 tree node).
    Component(ComponentRef),
    /// ViewDefinition(대상은 View 문서 node).
    View(StableId),
}

impl MotionOwner {
    /// 소유자 StableId.
    pub const fn id(&self) -> StableId {
        match self {
            MotionOwner::Component(component) => component.id(),
            MotionOwner::View(view) => *view,
        }
    }

    /// 저장 tag.
    pub const fn tag(&self) -> u8 {
        match self {
            MotionOwner::Component(_) => 0x01,
            MotionOwner::View(_) => 0x02,
        }
    }
}

/// `steps()`의 jump 위치(CSS Easing 1 `<step-position>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StepPosition {
    /// `jump-start`(`start`).
    JumpStart,
    /// `jump-end`(`end`, 기본).
    JumpEnd,
    /// `jump-none`.
    JumpNone,
    /// `jump-both`.
    JumpBoth,
}

impl StepPosition {
    /// 모든 위치(tag 순).
    pub const ALL: [StepPosition; 4] = [
        StepPosition::JumpStart,
        StepPosition::JumpEnd,
        StepPosition::JumpNone,
        StepPosition::JumpBoth,
    ];

    /// 저장 tag.
    pub const fn tag(self) -> u8 {
        match self {
            StepPosition::JumpStart => 0x01,
            StepPosition::JumpEnd => 0x02,
            StepPosition::JumpNone => 0x03,
            StepPosition::JumpBoth => 0x04,
        }
    }

    /// tag에서 복원합니다.
    pub fn from_tag(tag: u8) -> Option<StepPosition> {
        StepPosition::ALL.into_iter().find(|p| p.tag() == tag)
    }

    /// CSS 정규 이름.
    pub const fn css_name(self) -> &'static str {
        match self {
            StepPosition::JumpStart => "jump-start",
            StepPosition::JumpEnd => "jump-end",
            StepPosition::JumpNone => "jump-none",
            StepPosition::JumpBoth => "jump-both",
        }
    }
}

/// timing function(닫힌 집합).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimingFunction {
    /// `linear`.
    Linear,
    /// `cubic-bezier(x1, y1, x2, y2)`. `x1`·`x2`는 `[0, 1]`.
    CubicBezier {
        /// 첫 제어점 x.
        x1: f32,
        /// 첫 제어점 y.
        y1: f32,
        /// 둘째 제어점 x.
        x2: f32,
        /// 둘째 제어점 y.
        y2: f32,
    },
    /// `steps(count, position)`.
    Steps {
        /// 단계 수(1 이상, `jump-none`이면 2 이상).
        count: u32,
        /// jump 위치.
        position: StepPosition,
    },
    /// 감쇠 spring. 질량·강성·감쇠 계수와 초기 속도(끝값까지 거리 1 기준, 1/s)로 정의합니다.
    Spring {
        /// 질량(양수).
        mass: f32,
        /// 강성(양수).
        stiffness: f32,
        /// 감쇠 계수(0 이상).
        damping: f32,
        /// 초기 속도.
        initial_velocity: f32,
    },
}

impl TimingFunction {
    /// 저장 tag.
    pub const fn tag(&self) -> u8 {
        match self {
            TimingFunction::Linear => 0x01,
            TimingFunction::CubicBezier { .. } => 0x02,
            TimingFunction::Steps { .. } => 0x03,
            TimingFunction::Spring { .. } => 0x04,
        }
    }

    /// CSS `ease`(`cubic-bezier(0.25, 0.1, 0.25, 1)`).
    pub const EASE: TimingFunction = TimingFunction::CubicBezier {
        x1: 0.25,
        y1: 0.1,
        x2: 0.25,
        y2: 1.0,
    };
}

/// 시작 조건.
#[derive(Debug, Clone, PartialEq)]
pub enum StartCondition {
    /// 변수 값이 `value`가 아니다가 `value`가 되는 tick에 시작합니다.
    Becomes {
        /// 상태 변수.
        variable: StateVariableId,
        /// 값.
        value: TypedValue,
    },
    /// 변수 값이 바뀌는 모든 tick에 시작합니다.
    Changes {
        /// 상태 변수.
        variable: StateVariableId,
    },
}

impl StartCondition {
    /// 조건의 상태 변수.
    pub const fn variable(&self) -> StateVariableId {
        match self {
            StartCondition::Becomes { variable, .. } | StartCondition::Changes { variable } => {
                *variable
            }
        }
    }

    /// 저장 tag.
    pub const fn tag(&self) -> u8 {
        match self {
            StartCondition::Becomes { .. } => 0x01,
            StartCondition::Changes { .. } => 0x02,
        }
    }

    /// 이전 값 `before`에서 `after`로 바뀐 tick에 조건이 성립하는지.
    pub fn fires(&self, before: Option<&TypedValue>, after: Option<&TypedValue>) -> bool {
        match self {
            StartCondition::Changes { .. } => before != after,
            StartCondition::Becomes { value, .. } => after == Some(value) && before != Some(value),
        }
    }
}

/// reduced motion 대체 동작(닫힌 집합).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum ReducedMotion {
    /// 즉시 끝값으로 바꿉니다(기본).
    #[default]
    Instant,
    /// 그대로 재생합니다(불투명도·색 전환처럼 움직임이 없는 motion).
    Play,
}

impl ReducedMotion {
    /// 저장 tag.
    pub const fn tag(self) -> u8 {
        match self {
            ReducedMotion::Instant => 0x01,
            ReducedMotion::Play => 0x02,
        }
    }

    /// tag에서 복원합니다.
    pub fn from_tag(tag: u8) -> Option<ReducedMotion> {
        match tag {
            0x01 => Some(ReducedMotion::Instant),
            0x02 => Some(ReducedMotion::Play),
            _ => None,
        }
    }

    /// 텍스트 이름.
    pub const fn name(self) -> &'static str {
        match self {
            ReducedMotion::Instant => "instant",
            ReducedMotion::Play => "play",
        }
    }
}

/// 지속 시간·지연 상한(ms, 1시간).
pub const MAX_TIME_MS: f32 = 3_600_000.0;
/// spring motion 지속 시간 상한(ms). spring은 고정 step으로 적분하므로 평가 비용을 묶습니다.
pub const MAX_SPRING_DURATION_MS: f32 = 10_000.0;
/// cubic-bezier y 절댓값 상한.
pub const MAX_BEZIER_Y: f32 = 1_000.0;
/// `steps()` 단계 수 상한.
pub const MAX_STEPS: u32 = 10_000;
/// spring 고유 각진동수 제곱(`stiffness / mass`) 상한(1/s²). 1ms 고정 step 적분이 안정한 범위입니다.
pub const MAX_SPRING_OMEGA_SQUARED: f32 = 1_000_000.0;
/// spring 감쇠율(`damping / mass`) 상한(1/s).
pub const MAX_SPRING_DAMPING_RATE: f32 = 1_000.0;
/// spring 초기 속도 절댓값 상한(1/s).
pub const MAX_SPRING_VELOCITY: f32 = 1_000.0;
/// 대상 속성 수 상한.
pub const MAX_TARGETS: usize = 64;

/// MotionSpec 검증 오류.
#[derive(Debug, Clone, PartialEq)]
pub enum MotionSpecError {
    /// 범위를 벗어난 parameter.
    ParameterOutOfRange {
        /// parameter 이름.
        parameter: &'static str,
        /// 값.
        value: f32,
    },
    /// 대상이 없습니다.
    NoTargets,
    /// 대상이 너무 많습니다.
    TooManyTargets(usize),
    /// 같은 대상이 두 번 나옵니다.
    DuplicateTarget(PropertyPath),
    /// 대상 경로가 속성 하나가 아닙니다(key 하나, tail 없음).
    InvalidTargetPath(PropertyPath),
    /// 보간할 수 없는 타입의 대상.
    NotInterpolable {
        /// 대상.
        target: PropertyPath,
        /// 포트 타입 중 보간할 수 없는 것.
        found: ExpressionType,
    },
}

impl fmt::Display for MotionSpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MotionSpecError::ParameterOutOfRange { parameter, value } => {
                write!(f, "{parameter} 값 {value}이(가) 허용 범위 밖입니다")
            }
            MotionSpecError::NoTargets => f.write_str("대상 속성이 없습니다"),
            MotionSpecError::TooManyTargets(n) => write!(f, "대상 속성이 너무 많습니다({n})"),
            MotionSpecError::DuplicateTarget(path) => {
                write!(f, "같은 대상이 두 번 있습니다: {path:?}")
            }
            MotionSpecError::InvalidTargetPath(path) => {
                write!(f, "대상은 속성 하나를 가리켜야 합니다: {path:?}")
            }
            MotionSpecError::NotInterpolable { target, found } => {
                write!(f, "{found:?} 값은 보간할 수 없습니다: {target:?}")
            }
        }
    }
}

impl std::error::Error for MotionSpecError {}

/// MotionSpec 입력(검증 전).
#[derive(Debug, Clone, PartialEq)]
pub struct MotionSpecInput {
    /// 식별자.
    pub id: MotionSpecId,
    /// 소유자.
    pub owner: MotionOwner,
    /// 대상 속성(순서 보존).
    pub targets: Vec<PropertyPath>,
    /// 시작 조건.
    pub start: StartCondition,
    /// 지속 시간(ms).
    pub duration_ms: f32,
    /// 지연(ms, 음수면 중간부터 시작).
    pub delay_ms: f32,
    /// timing function.
    pub timing: TimingFunction,
    /// reduced motion 대체 동작.
    pub reduced_motion: ReducedMotion,
}

/// 검증된 MotionSpec.
#[derive(Debug, Clone, PartialEq)]
pub struct MotionSpec {
    input: MotionSpecInput,
}

fn range(parameter: &'static str, value: f32, ok: bool) -> Result<(), MotionSpecError> {
    if value.is_finite() && ok {
        Ok(())
    } else {
        Err(MotionSpecError::ParameterOutOfRange { parameter, value })
    }
}

fn check_timing(timing: TimingFunction, duration_ms: f32) -> Result<(), MotionSpecError> {
    match timing {
        TimingFunction::Linear => Ok(()),
        TimingFunction::CubicBezier { x1, y1, x2, y2 } => {
            range("cubic-bezier x1", x1, (0.0..=1.0).contains(&x1))?;
            range("cubic-bezier x2", x2, (0.0..=1.0).contains(&x2))?;
            range("cubic-bezier y1", y1, y1.abs() <= MAX_BEZIER_Y)?;
            range("cubic-bezier y2", y2, y2.abs() <= MAX_BEZIER_Y)
        }
        TimingFunction::Steps { count, position } => {
            let min = if position == StepPosition::JumpNone {
                2
            } else {
                1
            };
            range(
                "steps count",
                count as f32,
                count >= min && count <= MAX_STEPS,
            )
        }
        TimingFunction::Spring {
            mass,
            stiffness,
            damping,
            initial_velocity,
        } => {
            range("spring mass", mass, mass > 0.0)?;
            range("spring stiffness", stiffness, stiffness > 0.0)?;
            range("spring damping", damping, damping >= 0.0)?;
            range(
                "spring stiffness/mass",
                stiffness / mass,
                stiffness / mass <= MAX_SPRING_OMEGA_SQUARED,
            )?;
            range(
                "spring damping/mass",
                damping / mass,
                damping / mass <= MAX_SPRING_DAMPING_RATE,
            )?;
            range(
                "spring velocity",
                initial_velocity,
                initial_velocity.abs() <= MAX_SPRING_VELOCITY,
            )?;
            range(
                "spring duration",
                duration_ms,
                duration_ms <= MAX_SPRING_DURATION_MS,
            )
        }
    }
}

/// 값 타입을 보간할 수 있는지(ADR-0094 결정 1.3). 수·길이·백분율·각도·시간·색만 보간합니다.
pub const fn is_interpolable(value_type: ExpressionType) -> bool {
    matches!(
        value_type,
        ExpressionType::Number
            | ExpressionType::Length
            | ExpressionType::Percentage
            | ExpressionType::Angle
            | ExpressionType::Duration
            | ExpressionType::Color
    )
}

impl MotionSpec {
    /// 입력을 검증합니다. 대상 속성 타입 검사는 대상 문서를 아는 [`super::world::MotionWorld`]가 합니다
    /// ([`super::world::check_targets`]).
    pub fn new(input: MotionSpecInput) -> Result<MotionSpec, MotionSpecError> {
        range(
            "duration",
            input.duration_ms,
            (0.0..=MAX_TIME_MS).contains(&input.duration_ms),
        )?;
        range("delay", input.delay_ms, input.delay_ms.abs() <= MAX_TIME_MS)?;
        check_timing(input.timing, input.duration_ms)?;
        if input.targets.is_empty() {
            return Err(MotionSpecError::NoTargets);
        }
        if input.targets.len() > MAX_TARGETS {
            return Err(MotionSpecError::TooManyTargets(input.targets.len()));
        }
        for (index, target) in input.targets.iter().enumerate() {
            if target.keys().len() != 1 || target.tail().is_some() {
                return Err(MotionSpecError::InvalidTargetPath(target.clone()));
            }
            if input.targets[..index].contains(target) {
                return Err(MotionSpecError::DuplicateTarget(target.clone()));
            }
        }
        let mut input = input;
        // `-0`은 `0`으로 정규화합니다(저장 bytes 결정론).
        input.duration_ms += 0.0;
        input.delay_ms += 0.0;
        input.timing = match input.timing {
            TimingFunction::CubicBezier { x1, y1, x2, y2 } => TimingFunction::CubicBezier {
                x1: x1 + 0.0,
                y1: y1 + 0.0,
                x2: x2 + 0.0,
                y2: y2 + 0.0,
            },
            TimingFunction::Spring {
                mass,
                stiffness,
                damping,
                initial_velocity,
            } => TimingFunction::Spring {
                mass: mass + 0.0,
                stiffness: stiffness + 0.0,
                damping: damping + 0.0,
                initial_velocity: initial_velocity + 0.0,
            },
            other => other,
        };
        Ok(MotionSpec { input })
    }

    /// 식별자.
    pub fn id(&self) -> MotionSpecId {
        self.input.id
    }

    /// 소유자.
    pub fn owner(&self) -> MotionOwner {
        self.input.owner
    }

    /// 대상 속성.
    pub fn targets(&self) -> &[PropertyPath] {
        &self.input.targets
    }

    /// 시작 조건.
    pub fn start(&self) -> &StartCondition {
        &self.input.start
    }

    /// 지속 시간(ms).
    pub fn duration_ms(&self) -> f32 {
        self.input.duration_ms
    }

    /// 지연(ms).
    pub fn delay_ms(&self) -> f32 {
        self.input.delay_ms
    }

    /// timing function.
    pub fn timing(&self) -> TimingFunction {
        self.input.timing
    }

    /// reduced motion 대체 동작.
    pub fn reduced_motion(&self) -> ReducedMotion {
        self.input.reduced_motion
    }

    /// 입력으로 되돌립니다.
    pub fn to_input(&self) -> MotionSpecInput {
        self.input.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::property::PropertyKey;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn input() -> MotionSpecInput {
        MotionSpecInput {
            id: MotionSpecId::new(sid(1)),
            owner: MotionOwner::Component(ComponentRef::new(sid(2))),
            targets: vec![PropertyPath::new(sid(3)).push(PropertyKey(0x0104))],
            start: StartCondition::Changes {
                variable: StateVariableId::new(sid(4)),
            },
            duration_ms: 200.0,
            delay_ms: 0.0,
            timing: TimingFunction::EASE,
            reduced_motion: ReducedMotion::Instant,
        }
    }

    fn rejects(change: impl FnOnce(&mut MotionSpecInput)) -> MotionSpecError {
        let mut value = input();
        change(&mut value);
        MotionSpec::new(value).expect_err("거부")
    }

    #[test]
    fn parameter_ranges_are_enforced() {
        // 검증 기준(XXX.1.1): parameter 범위 위반 fixture 거부.
        assert!(MotionSpec::new(input()).is_ok());
        type Change = Box<dyn FnOnce(&mut MotionSpecInput)>;
        let cases: Vec<Change> = vec![
            Box::new(|i| i.duration_ms = -1.0),
            Box::new(|i| i.duration_ms = f32::NAN),
            Box::new(|i| i.duration_ms = MAX_TIME_MS + 1.0),
            Box::new(|i| i.delay_ms = f32::INFINITY),
            Box::new(|i| {
                i.timing = TimingFunction::CubicBezier {
                    x1: 1.5,
                    y1: 0.0,
                    x2: 0.5,
                    y2: 1.0,
                }
            }),
            Box::new(|i| {
                i.timing = TimingFunction::CubicBezier {
                    x1: 0.5,
                    y1: 0.0,
                    x2: -0.1,
                    y2: 1.0,
                }
            }),
            Box::new(|i| {
                i.timing = TimingFunction::Steps {
                    count: 0,
                    position: StepPosition::JumpEnd,
                }
            }),
            Box::new(|i| {
                i.timing = TimingFunction::Steps {
                    count: 1,
                    position: StepPosition::JumpNone,
                }
            }),
            Box::new(|i| {
                i.timing = TimingFunction::Spring {
                    mass: 0.0,
                    stiffness: 100.0,
                    damping: 10.0,
                    initial_velocity: 0.0,
                }
            }),
            Box::new(|i| {
                i.timing = TimingFunction::Spring {
                    mass: 1.0,
                    stiffness: -1.0,
                    damping: 10.0,
                    initial_velocity: 0.0,
                }
            }),
            Box::new(|i| {
                i.timing = TimingFunction::Spring {
                    mass: 1.0,
                    stiffness: 100.0,
                    damping: -0.5,
                    initial_velocity: 0.0,
                }
            }),
            Box::new(|i| {
                i.timing = TimingFunction::Spring {
                    mass: 0.001,
                    stiffness: 10_000.0,
                    damping: 10.0,
                    initial_velocity: 0.0,
                }
            }),
            Box::new(|i| {
                i.duration_ms = MAX_SPRING_DURATION_MS + 1.0;
                i.timing = TimingFunction::Spring {
                    mass: 1.0,
                    stiffness: 100.0,
                    damping: 10.0,
                    initial_velocity: 0.0,
                }
            }),
        ];
        for case in cases {
            assert!(matches!(
                rejects(case),
                MotionSpecError::ParameterOutOfRange { .. }
            ));
        }
    }

    #[test]
    fn target_list_rules() {
        assert_eq!(rejects(|i| i.targets.clear()), MotionSpecError::NoTargets);
        let dup = rejects(|i| {
            let t = i.targets[0].clone();
            i.targets.push(t);
        });
        assert!(matches!(dup, MotionSpecError::DuplicateTarget(_)));
        let bad = rejects(|i| i.targets = vec![PropertyPath::new(sid(3))]);
        assert!(matches!(bad, MotionSpecError::InvalidTargetPath(_)));
        let indexed = rejects(|i| {
            i.targets = vec![PropertyPath::new(sid(3)).push(PropertyKey(1)).with_index(0)]
        });
        assert!(matches!(indexed, MotionSpecError::InvalidTargetPath(_)));
    }

    #[test]
    fn negative_zero_is_normalized() {
        let mut value = input();
        value.delay_ms = -0.0;
        value.timing = TimingFunction::CubicBezier {
            x1: -0.0,
            y1: -0.0,
            x2: 1.0,
            y2: 1.0,
        };
        let spec = MotionSpec::new(value).expect("유효");
        assert!(spec.delay_ms().is_sign_positive());
        let TimingFunction::CubicBezier { x1, y1, .. } = spec.timing() else {
            panic!("cubic-bezier");
        };
        assert!(x1.is_sign_positive() && y1.is_sign_positive());
        let mut spring = input();
        spring.timing = TimingFunction::Spring {
            mass: 1.0,
            stiffness: 100.0,
            damping: -0.0,
            initial_velocity: -0.0,
        };
        let TimingFunction::Spring {
            damping,
            initial_velocity,
            ..
        } = MotionSpec::new(spring).expect("유효").timing()
        else {
            panic!("spring");
        };
        assert!(damping.is_sign_positive() && initial_velocity.is_sign_positive());
    }

    #[test]
    fn start_condition_edges() {
        let var = StateVariableId::new(sid(9));
        let on = TypedValue::Boolean(true);
        let off = TypedValue::Boolean(false);
        let becomes = StartCondition::Becomes {
            variable: var,
            value: on.clone(),
        };
        assert!(becomes.fires(Some(&off), Some(&on)));
        assert!(!becomes.fires(Some(&on), Some(&on)));
        assert!(!becomes.fires(Some(&on), Some(&off)));
        let changes = StartCondition::Changes { variable: var };
        assert!(changes.fires(Some(&on), Some(&off)));
        assert!(!changes.fires(Some(&on), Some(&on)));
    }
}
