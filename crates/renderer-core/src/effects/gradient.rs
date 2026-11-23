//! gradient 수식 (XI.10.3, ADR-0102 결정 4).
//!
//! ## stop 순서
//!
//! renderer는 stop을 **시각 순서로 받은 그대로** 씁니다. 시각 순서는 core-types `Gradient::visual_order`(III.4.3,
//! `(position, 안정 ID)`)가 정하는 유일한 규칙이고, renderer는 다시 정렬하지 않습니다. [`GradientRamp::resolve`]는
//! position이 줄지 않는지만 확인합니다(같은 position은 받은 순서가 tie 순서).
//!
//! ## 값
//!
//! stop 색은 working 값([`crate::color::to_working`], 선형 premultiplied)으로 바꾼 뒤 보간합니다.
//!
//! - `t < p₀`: 첫 stop.
//! - `t ≥ pₙ₋₁`: 마지막 stop.
//! - 그 사이: `pᵢ ≤ t < pᵢ₊₁`인 마지막 `i`와 `i + 1`의 선형 보간. 같은 position stop들은 `t = p`에서 **뒤 stop**이
//!   이깁니다(hard edge).
//!
//! ## 기하
//!
//! - 선형: `t = dot(p − start, end − start) / |end − start|²`.
//! - 원형: `t = |p − center| / radius`.
//! - 길이 0(start = end, radius = 0)은 `t = +∞`, 즉 마지막 stop 단색입니다.
//!
//! ## GPU ramp
//!
//! [`GradientRamp::ramp`]는 `[t_min, t_max]`(stop 범위와 `[0, 1]`의 합집합)를 [`RAMP_WIDTH`] texel로 나눈 texel 중심의
//! 정확한 값입니다. 선형 gradient는 `t`가 위치의 affine 함수라 정점 uv.x = [`GradientRamp::ramp_coordinate`]를 보간하고
//! linear filter·clamp로 읽으면 같은 값을 얻습니다. 원형 gradient는 정점 보간으로 표현되지 않아 GPU 경로가 없습니다.

use crate::color::{to_working, WorkingColor};
use crate::output_color::{OutputColor, OutputColorPlan};

/// GPU ramp texel 수.
pub const RAMP_WIDTH: u32 = 1024;

/// 시각 순서 stop(position과 문서 색).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    /// position.
    pub position: f32,
    /// 색.
    pub color: OutputColor,
}

/// gradient 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradientError {
    /// stop이 없음.
    Empty,
    /// position이 유한하지 않음.
    NonFinitePosition,
    /// position이 앞 stop보다 작음(시각 순서가 아님).
    OutOfOrder {
        /// 처음 어긋난 stop index.
        index: usize,
    },
}

/// working 값으로 푼 stop 목록.
#[derive(Debug, Clone, PartialEq)]
pub struct GradientRamp {
    positions: Vec<f32>,
    colors: Vec<WorkingColor>,
}

impl GradientRamp {
    /// 시각 순서 stop을 working 값으로 풉니다.
    pub fn resolve(
        plan: &OutputColorPlan,
        stops: &[GradientStop],
    ) -> Result<GradientRamp, GradientError> {
        if stops.is_empty() {
            return Err(GradientError::Empty);
        }
        let mut previous = f32::NEG_INFINITY;
        for (index, stop) in stops.iter().enumerate() {
            if !stop.position.is_finite() {
                return Err(GradientError::NonFinitePosition);
            }
            if stop.position < previous {
                return Err(GradientError::OutOfOrder { index });
            }
            previous = stop.position;
        }
        Ok(GradientRamp {
            positions: stops.iter().map(|s| s.position).collect(),
            colors: stops.iter().map(|s| to_working(plan, s.color)).collect(),
        })
    }

    /// `t`의 값(모듈 문서 규칙).
    pub fn evaluate(&self, t: f32) -> WorkingColor {
        let n = self.positions.len();
        if t.is_nan() || t < self.positions[0] {
            return self.colors[0];
        }
        // pᵢ ≤ t인 마지막 i.
        let i = self.positions.partition_point(|&p| p <= t) - 1;
        if i + 1 >= n {
            return self.colors[n - 1];
        }
        let (p0, p1) = (self.positions[i], self.positions[i + 1]);
        let k = (t - p0) / (p1 - p0);
        let (a, b) = (self.colors[i], self.colors[i + 1]);
        WorkingColor::premultiplied(std::array::from_fn(|c| {
            a.rgba[c] + (b.rgba[c] - a.rgba[c]) * k
        }))
    }

    /// ramp가 덮는 `t` 범위 `[t_min, t_max]`.
    pub fn domain(&self) -> (f32, f32) {
        let first = self.positions[0].min(0.0);
        let last = self.positions[self.positions.len() - 1].max(1.0);
        (first, last)
    }

    /// `t`의 ramp texture 좌표(`[0, 1]` 밖은 clamp가 첫·마지막 값을 줌).
    pub fn ramp_coordinate(&self, t: f32) -> f32 {
        let (lo, hi) = self.domain();
        if t == f32::INFINITY {
            return 2.0;
        }
        (t - lo) / (hi - lo)
    }

    /// GPU ramp texel(texel 중심의 정확한 값).
    pub fn ramp(&self, width: u32) -> Vec<WorkingColor> {
        let (lo, hi) = self.domain();
        (0..width)
            .map(|i| self.evaluate(lo + (hi - lo) * (i as f32 + 0.5) / width as f32))
            .collect()
    }
}

/// gradient 기하.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GradientGeometry {
    /// 선형(시작점 → 끝점).
    Linear {
        /// 시작점(`t = 0`).
        start: [f32; 2],
        /// 끝점(`t = 1`).
        end: [f32; 2],
    },
    /// 원형(중심, 반지름).
    Radial {
        /// 중심(`t = 0`).
        center: [f32; 2],
        /// 반지름(`t = 1`).
        radius: f32,
    },
}

impl GradientGeometry {
    /// 점 `p`의 `t`(길이 0이면 `+∞`).
    pub fn parameter(&self, p: [f32; 2]) -> f32 {
        match *self {
            GradientGeometry::Linear { start, end } => {
                let d = [f64::from(end[0] - start[0]), f64::from(end[1] - start[1])];
                let len2 = d[0] * d[0] + d[1] * d[1];
                if len2 == 0.0 {
                    return f32::INFINITY;
                }
                let v = [f64::from(p[0] - start[0]), f64::from(p[1] - start[1])];
                ((v[0] * d[0] + v[1] * d[1]) / len2) as f32
            }
            GradientGeometry::Radial { center, radius } => {
                if radius <= 0.0 {
                    return f32::INFINITY;
                }
                let dx = f64::from(p[0] - center[0]);
                let dy = f64::from(p[1] - center[1]);
                ((dx * dx + dy * dy).sqrt() / f64::from(radius)) as f32
            }
        }
    }

    /// 정점 보간으로 정확히 표현되면(선형) 참.
    pub const fn is_affine(&self) -> bool {
        matches!(self, GradientGeometry::Linear { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::ColorSpace;
    use crate::effects::fixture::srgb;

    fn plan() -> OutputColorPlan {
        OutputColorPlan::negotiate(ColorSpace::Srgb, &crate::color::tests::caps(false))
    }

    fn stop(position: f32, r: f32, a: f32) -> GradientStop {
        GradientStop {
            position,
            color: srgb(r, 0.0, 0.0, a),
        }
    }

    #[track_caller]
    fn close(a: WorkingColor, b: WorkingColor) {
        for (x, y) in a.rgba.iter().zip(b.rgba.iter()) {
            assert!((x - y).abs() < 1e-6, "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn evaluation_pads_interpolates_and_breaks_ties_toward_later_stop() {
        let plan = plan();
        let ramp = GradientRamp::resolve(
            &plan,
            &[
                stop(0.25, 1.0, 1.0),
                stop(0.5, 0.0, 1.0),
                stop(0.5, 0.5, 1.0),
                stop(0.75, 1.0, 0.0),
            ],
        )
        .unwrap();
        let w = |r: f32, a: f32| to_working(&plan, srgb(r, 0.0, 0.0, a));
        close(ramp.evaluate(-3.0), w(1.0, 1.0));
        close(ramp.evaluate(0.25), w(1.0, 1.0));
        // 선형 working 값 사이 보간.
        let mid = ramp.evaluate(0.375);
        let expected = (w(1.0, 1.0).rgba[0] + w(0.0, 1.0).rgba[0]) / 2.0;
        assert!((mid.rgba[0] - expected).abs() < 1e-6);
        // 같은 position: 직전은 앞 stop 쪽, t = p에서는 뒤 stop.
        assert!((ramp.evaluate(0.499_999).rgba[0] - w(0.0, 1.0).rgba[0]).abs() < 1e-4);
        close(ramp.evaluate(0.5), w(0.5, 1.0));
        // premultiplied 보간: 투명 stop 쪽으로 가며 색과 alpha가 함께 줄어듭니다.
        let fading = ramp.evaluate(0.625);
        assert!((fading.alpha() - 0.5).abs() < 1e-6);
        close(ramp.evaluate(0.75), w(1.0, 0.0));
        close(ramp.evaluate(10.0), w(1.0, 0.0));
        close(ramp.evaluate(f32::INFINITY), w(1.0, 0.0));
    }

    #[test]
    fn stops_must_arrive_in_visual_order() {
        let plan = plan();
        assert_eq!(GradientRamp::resolve(&plan, &[]), Err(GradientError::Empty));
        assert_eq!(
            GradientRamp::resolve(&plan, &[stop(0.5, 1.0, 1.0), stop(0.25, 0.0, 1.0)]),
            Err(GradientError::OutOfOrder { index: 1 })
        );
        assert_eq!(
            GradientRamp::resolve(&plan, &[stop(f32::NAN, 1.0, 1.0)]),
            Err(GradientError::NonFinitePosition)
        );
        // 단일 stop은 단색입니다.
        let single = GradientRamp::resolve(&plan, &[stop(0.3, 1.0, 1.0)]).unwrap();
        close(single.evaluate(0.0), single.evaluate(1.0));
    }

    #[test]
    fn geometry_parameters_and_degenerate_cases() {
        let linear = GradientGeometry::Linear {
            start: [0.0, 0.0],
            end: [10.0, 0.0],
        };
        assert_eq!(linear.parameter([5.0, 3.0]), 0.5);
        assert_eq!(linear.parameter([-5.0, 0.0]), -0.5);
        assert!(linear.is_affine());
        let diagonal = GradientGeometry::Linear {
            start: [0.0, 0.0],
            end: [4.0, 4.0],
        };
        assert!((diagonal.parameter([4.0, 0.0]) - 0.5).abs() < 1e-6);
        let radial = GradientGeometry::Radial {
            center: [5.0, 5.0],
            radius: 5.0,
        };
        assert_eq!(radial.parameter([8.0, 9.0]), 1.0);
        assert!(!radial.is_affine());
        let zero = GradientGeometry::Linear {
            start: [1.0, 1.0],
            end: [1.0, 1.0],
        };
        assert_eq!(zero.parameter([3.0, 3.0]), f32::INFINITY);
        assert_eq!(
            GradientGeometry::Radial {
                center: [0.0, 0.0],
                radius: 0.0
            }
            .parameter([1.0, 1.0]),
            f32::INFINITY
        );
    }

    #[test]
    fn ramp_texels_cover_domain_including_stops_outside_unit_range() {
        let plan = plan();
        let ramp =
            GradientRamp::resolve(&plan, &[stop(-0.5, 1.0, 1.0), stop(1.5, 0.0, 1.0)]).unwrap();
        assert_eq!(ramp.domain(), (-0.5, 1.5));
        assert_eq!(ramp.ramp_coordinate(-0.5), 0.0);
        assert_eq!(ramp.ramp_coordinate(1.5), 1.0);
        let texels = ramp.ramp(4);
        close(texels[0], ramp.evaluate(-0.25));
        close(texels[3], ramp.evaluate(1.25));
        // 길이 0 gradient는 clamp 쪽(마지막 값)으로 갑니다.
        assert!(ramp.ramp_coordinate(f32::INFINITY) > 1.0);
        let unit =
            GradientRamp::resolve(&plan, &[stop(0.2, 1.0, 1.0), stop(0.8, 0.0, 1.0)]).unwrap();
        assert_eq!(unit.domain(), (0.0, 1.0));
    }
}
