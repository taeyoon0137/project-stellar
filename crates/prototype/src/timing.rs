//! timing function evaluator와 값 보간 (XXX.3.2, ADR-0025 결정 1.2·2.7, ADR-0094 결정 6).
//!
//! 결과는 (MotionSpec, 경과 논리 시간)의 순수 함수입니다. 계산은 IEEE-754 기본 연산(사칙연산·비교)만 쓰며, libm
//! 초월함수(지수·삼각·거듭제곱·로그)는 플랫폼마다 결과가 다를 수 있어 쓰지 않습니다(ADR-0006). 모든 중간 계산은 `f64`이고
//! 결과 값을 저장 타입(`f32`)으로 한 번 반올림합니다.
//!
//! - `linear`: 진행률 그대로.
//! - `cubic-bezier`: x(t) = p를 [`BEZIER_ITERATIONS`]번 고정 이분법으로 풀고 y(t)를 씁니다.
//! - `steps`: CSS Easing 1의 step 공식(before flag 없음).
//! - `spring`: 끝값까지 거리 1을 0→1로 놓고 [`SPRING_STEP_MS`] 고정 step의 semi-implicit Euler로 적분합니다. step 사이는
//!   선형 보간하고, 지속 시간이 끝나면 정확히 1입니다.
//!
//! 보간([`interpolate`])은 같은 타입 끝값 사이에서만 합니다. Color는 저장 색공간의 encoded 채널을 alpha로 곱한
//! (premultiplied) 값을 선형 보간하고 결과 alpha로 나눕니다(결과 alpha가 0이면 채널 0). 진행률이 [0, 1]을 벗어나면
//! (overshoot) alpha는 [0, 1]로 자릅니다.

use stellar_core_types::color::Color;
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::{Angle, Duration, FiniteF32, Length, Percentage};
use stellar_document::motion::{
    check_endpoints, EndpointError, MotionSpec, StepPosition, TimingFunction,
};

/// cubic-bezier 이분법 반복 횟수(고정).
pub const BEZIER_ITERATIONS: u32 = 64;
/// spring 적분 step(ms).
pub const SPRING_STEP_MS: f64 = 1.0;

fn bezier(a: f64, b: f64, t: f64) -> f64 {
    // B(t) = 3(1-t)^2 t a + 3(1-t) t^2 b + t^3
    let u = 1.0 - t;
    3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
}

fn cubic_bezier(x1: f64, y1: f64, x2: f64, y2: f64, p: f64) -> f64 {
    if p <= 0.0 {
        return 0.0;
    }
    if p >= 1.0 {
        return 1.0;
    }
    // x1·x2 ∈ [0, 1]이므로 x(t)는 [0, 1]에서 단조 증가합니다.
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    for _ in 0..BEZIER_ITERATIONS {
        let mid = (lo + hi) * 0.5;
        if bezier(x1, x2, mid) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bezier(y1, y2, (lo + hi) * 0.5)
}

fn steps(count: u32, position: StepPosition, p: f64) -> f64 {
    let n = count as f64;
    let mut step = (p * n).floor();
    if matches!(position, StepPosition::JumpStart | StepPosition::JumpBoth) {
        step += 1.0;
    }
    let jumps = match position {
        StepPosition::JumpStart | StepPosition::JumpEnd => n,
        StepPosition::JumpNone => n - 1.0,
        StepPosition::JumpBoth => n + 1.0,
    };
    if p >= 0.0 && step < 0.0 {
        step = 0.0;
    }
    if p <= 1.0 && step > jumps {
        step = jumps;
    }
    step / jumps
}

fn spring(mass: f64, stiffness: f64, damping: f64, velocity: f64, elapsed_ms: f64) -> f64 {
    let h = SPRING_STEP_MS / 1000.0;
    let mut x = 0.0f64;
    let mut v = velocity;
    let whole = elapsed_ms.floor();
    let steps = whole as u64;
    let step = |x: &mut f64, v: &mut f64| {
        let a = (-stiffness * (*x - 1.0) - damping * *v) / mass;
        *v += a * h;
        *x += *v * h;
    };
    for _ in 0..steps {
        step(&mut x, &mut v);
    }
    let frac = elapsed_ms - whole;
    if frac > 0.0 {
        let (mut nx, mut nv) = (x, v);
        step(&mut nx, &mut nv);
        x + (nx - x) * frac
    } else {
        x
    }
}

/// `timing`으로 진행률을 계산합니다. `elapsed_ms`는 지연을 뺀 경과 시간입니다. 결과는 overshoot 때 [0, 1]을 벗어날 수
/// 있습니다.
pub fn eased_progress(timing: TimingFunction, duration_ms: f64, elapsed_ms: f64) -> f64 {
    if elapsed_ms <= 0.0 {
        return match timing {
            TimingFunction::Steps { count, position } => steps(count, position, 0.0),
            _ => 0.0,
        };
    }
    if duration_ms <= 0.0 || elapsed_ms >= duration_ms {
        return 1.0;
    }
    let p = elapsed_ms / duration_ms;
    match timing {
        TimingFunction::Linear => p,
        TimingFunction::CubicBezier { x1, y1, x2, y2 } => {
            cubic_bezier(x1 as f64, y1 as f64, x2 as f64, y2 as f64, p)
        }
        TimingFunction::Steps { count, position } => steps(count, position, p),
        TimingFunction::Spring {
            mass,
            stiffness,
            damping,
            initial_velocity,
        } => spring(
            mass as f64,
            stiffness as f64,
            damping as f64,
            initial_velocity as f64,
            elapsed_ms,
        ),
    }
}

fn lerp(a: f32, b: f32, q: f64) -> f64 {
    a as f64 + (b as f64 - a as f64) * q
}

fn finite(v: f64) -> Result<FiniteF32, EndpointError> {
    FiniteF32::new(v as f32).map_err(|_| EndpointError::Missing)
}

/// 같은 타입 끝값 `start`·`end`를 진행률 `q`로 보간합니다.
pub fn interpolate(
    start: &TypedValue,
    end: &TypedValue,
    q: f64,
) -> Result<TypedValue, EndpointError> {
    check_endpoints(Some(start), Some(end))?;
    Ok(match (start, end) {
        (TypedValue::Number(a), TypedValue::Number(b)) => {
            TypedValue::Number(finite(lerp(a.get(), b.get(), q))?)
        }
        (TypedValue::Length(a), TypedValue::Length(b)) => {
            TypedValue::Length(Length::from_raw(finite(lerp(a.get(), b.get(), q))?))
        }
        (TypedValue::Percentage(a), TypedValue::Percentage(b)) => {
            TypedValue::Percentage(Percentage::from_raw(finite(lerp(a.get(), b.get(), q))?))
        }
        (TypedValue::Angle(a), TypedValue::Angle(b)) => {
            TypedValue::Angle(Angle::from_raw(finite(lerp(a.get(), b.get(), q))?))
        }
        (TypedValue::Duration(a), TypedValue::Duration(b)) => {
            TypedValue::Duration(Duration::from_raw(finite(lerp(a.get(), b.get(), q))?))
        }
        (TypedValue::Color(a), TypedValue::Color(b)) => {
            TypedValue::Color(interpolate_color(*a, *b, q)?)
        }
        _ => {
            return Err(EndpointError::NotInterpolable(
                stellar_expressions::types::ExpressionType::of(start),
            ))
        }
    })
}

/// premultiplied encoded 채널 선형 보간(같은 색공간).
pub fn interpolate_color(a: Color, b: Color, q: f64) -> Result<Color, EndpointError> {
    if a.space() != b.space() {
        return Err(EndpointError::ColorSpaceMismatch);
    }
    let (aa, ba) = (a.alpha() as f64, b.alpha() as f64);
    let alpha = (aa + (ba - aa) * q).clamp(0.0, 1.0);
    let channel = |ca: f32, cb: f32| -> f32 {
        let pa = ca as f64 * aa;
        let pb = cb as f64 * ba;
        let p = pa + (pb - pa) * q;
        if alpha > 0.0 {
            (p / alpha) as f32
        } else {
            0.0
        }
    };
    Color::new(
        a.space(),
        channel(a.r(), b.r()),
        channel(a.g(), b.g()),
        channel(a.b(), b.b()),
        alpha as f32,
    )
    .map_err(|_| EndpointError::Missing)
}

/// MotionSpec 한 대상의 표본: 시작 뒤 `elapsed_ms`(지연 포함)에서의 값과 끝났는지.
pub fn sample(
    spec: &MotionSpec,
    start: &TypedValue,
    end: &TypedValue,
    elapsed_ms: f64,
) -> Result<(TypedValue, bool), EndpointError> {
    let active = elapsed_ms - spec.delay_ms() as f64;
    let duration = spec.duration_ms() as f64;
    if active >= duration {
        check_endpoints(Some(start), Some(end))?;
        return Ok((end.clone(), true));
    }
    if active < 0.0 {
        // 지연 동안은 시작값을 유지합니다(CSS transition before phase).
        check_endpoints(Some(start), Some(end))?;
        return Ok((start.clone(), false));
    }
    let q = eased_progress(spec.timing(), duration, active);
    Ok((interpolate(start, end, q)?, false))
}

#[cfg(test)]
mod tests {
    use stellar_core_types::color::ColorSpace;

    use super::*;

    fn bits(v: f64) -> u64 {
        v.to_bits()
    }

    #[test]
    fn linear_and_boundaries() {
        assert_eq!(eased_progress(TimingFunction::Linear, 200.0, 50.0), 0.25);
        assert_eq!(eased_progress(TimingFunction::Linear, 200.0, -5.0), 0.0);
        assert_eq!(eased_progress(TimingFunction::Linear, 200.0, 200.0), 1.0);
        assert_eq!(eased_progress(TimingFunction::Linear, 0.0, 1.0), 1.0);
    }

    #[test]
    fn cubic_bezier_boundaries() {
        // 검증 기준(XXX.3.2): 경계 parameter fixture.
        let identity = TimingFunction::CubicBezier {
            x1: 0.0,
            y1: 0.0,
            x2: 1.0,
            y2: 1.0,
        };
        for i in 0..=10 {
            let p = i as f64 / 10.0;
            let y = eased_progress(identity, 1000.0, p * 1000.0);
            assert!((y - p).abs() < 1e-9, "{p} → {y}");
        }
        let overshoot = TimingFunction::CubicBezier {
            x1: 0.34,
            y1: 1.56,
            x2: 0.64,
            y2: 1.0,
        };
        let mid = eased_progress(overshoot, 100.0, 60.0);
        assert!(mid > 1.0, "overshoot {mid}");
        let x_edges = TimingFunction::CubicBezier {
            x1: 1.0,
            y1: 0.0,
            x2: 0.0,
            y2: 1.0,
        };
        let v = eased_progress(x_edges, 100.0, 50.0);
        // x(t)의 기울기가 0인 지점이라 t 정밀도가 떨어집니다(삼차 근). 결과는 결정론적입니다.
        assert!((v - 0.5).abs() < 1e-4, "{v}");
    }

    #[test]
    fn steps_follow_css_easing() {
        let f = |count, position, p: f64| {
            eased_progress(TimingFunction::Steps { count, position }, 1.0, p)
        };
        assert_eq!(f(4, StepPosition::JumpEnd, 0.3), 0.25);
        assert_eq!(f(4, StepPosition::JumpStart, 0.3), 0.5);
        assert_eq!(f(4, StepPosition::JumpStart, 0.0), 0.25);
        assert_eq!(f(3, StepPosition::JumpNone, 0.5), 0.5);
        assert_eq!(f(3, StepPosition::JumpBoth, 0.0), 0.25);
        assert_eq!(f(3, StepPosition::JumpBoth, 0.99), 0.75);
        assert_eq!(f(1, StepPosition::JumpEnd, 0.99), 0.0);
        assert_eq!(f(1, StepPosition::JumpEnd, 1.0), 1.0);
    }

    #[test]
    fn spring_settles_and_ends_exactly() {
        let timing = TimingFunction::Spring {
            mass: 1.0,
            stiffness: 170.0,
            damping: 26.0,
            initial_velocity: 0.0,
        };
        let early = eased_progress(timing, 1000.0, 10.0);
        let late = eased_progress(timing, 1000.0, 900.0);
        assert!(early > 0.0 && early < 0.2, "{early}");
        assert!((late - 1.0).abs() < 1e-3, "{late}");
        assert_eq!(eased_progress(timing, 1000.0, 1000.0), 1.0);
        // 감쇠 0: 진동이 그대로 남아 overshoot.
        let undamped = TimingFunction::Spring {
            mass: 1.0,
            stiffness: 100.0,
            damping: 0.0,
            initial_velocity: 0.0,
        };
        let peak = (1..1000)
            .map(|t| eased_progress(undamped, 10_000.0, t as f64))
            .fold(f64::MIN, f64::max);
        assert!(peak > 1.9, "{peak}");
    }

    #[test]
    fn color_interpolates_premultiplied() {
        let red = Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 1.0).expect("색");
        let clear_blue = Color::new(ColorSpace::Srgb, 0.0, 0.0, 1.0, 0.0).expect("색");
        let mid = interpolate_color(red, clear_blue, 0.5).expect("보간");
        // premultiplied: 파랑은 alpha 0이라 기여 0 → 채널은 순수 빨강.
        assert_eq!(
            (mid.r(), mid.g(), mid.b(), mid.alpha()),
            (1.0, 0.0, 0.0, 0.5)
        );
        // 검증 기준(XXX.3.2): 결과 alpha 0 Color 보간 fixture.
        let clear_red = Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 0.0).expect("색");
        let zero = interpolate_color(clear_red, clear_blue, 0.5).expect("보간");
        assert_eq!(
            (zero.r(), zero.g(), zero.b(), zero.alpha()),
            (0.0, 0.0, 0.0, 0.0)
        );
        let p3 = Color::new(ColorSpace::DisplayP3, 1.0, 0.0, 0.0, 1.0).expect("색");
        assert_eq!(
            interpolate_color(red, p3, 0.5),
            Err(EndpointError::ColorSpaceMismatch)
        );
    }

    /// reference corpus: 고정 입력의 결과 bits를 한 줄씩 적은 텍스트.
    pub(crate) fn corpus_text() -> String {
        let timings = [
            TimingFunction::Linear,
            TimingFunction::EASE,
            TimingFunction::CubicBezier {
                x1: 0.68,
                y1: -0.55,
                x2: 0.265,
                y2: 1.55,
            },
            TimingFunction::Steps {
                count: 5,
                position: StepPosition::JumpStart,
            },
            TimingFunction::Spring {
                mass: 1.0,
                stiffness: 170.0,
                damping: 26.0,
                initial_velocity: 0.0,
            },
            TimingFunction::Spring {
                mass: 2.0,
                stiffness: 300.0,
                damping: 5.0,
                initial_velocity: 3.0,
            },
        ];
        let red = Color::new(ColorSpace::Srgb, 1.0, 0.2, 0.0, 0.8).expect("색");
        let blue = Color::new(ColorSpace::Srgb, 0.0, 0.4, 1.0, 0.3).expect("색");
        let mut lines = Vec::new();
        for (index, timing) in timings.into_iter().enumerate() {
            for tick in (0..=48).step_by(4) {
                let elapsed = crate::clock::ticks_to_ms(tick);
                let q = eased_progress(timing, 400.0, elapsed);
                let c = interpolate_color(red, blue, q).expect("보간");
                lines.push(format!(
                    "{index} {tick} {:016x} {:08x} {:08x} {:08x} {:08x}",
                    bits(q),
                    c.r().to_bits(),
                    c.g().to_bits(),
                    c.b().to_bits(),
                    c.alpha().to_bits()
                ));
            }
        }
        lines.join("\n") + "\n"
    }

    #[test]
    fn reference_corpus_is_byte_identical() {
        // 검증 기준(XXX.3.2): reference corpus bytes 동일(Color 보간 포함).
        let text = corpus_text();
        let golden = include_str!("timing_corpus.txt");
        if text != golden {
            panic!("timing corpus가 다릅니다. 새 값:\n{text}");
        }
        assert_eq!(corpus_text(), text, "같은 입력 같은 결과");
    }
}
