//! 성능 sample analyzer와 release gate (XVI.4.2~XVI.4.4, ADR-0084 결정 11).
//!
//! - **통계**(XVI.4.2): 통제 가능한 sample의 p50·p95·p99(nearest-rank, 정렬 후 `ceil(p × n / 100)`번째)와 최댓값,
//!   통제 가능 상한(`controllable_max_us`) 초과 수를 냅니다. 평균은 판정에 쓰지 않습니다(아키텍처 §6.3).
//! - **제외**(XVI.4.3): 운영체제 선점·장치 상실·절전 전환·드라이버 정지처럼 Stellar가 통제할 수 없는 지연은 사전
//!   정의한 [`ExclusionCode`]를 단 sample만 뺍니다. 코드가 없는 sample은 아무리 커도 그대로 셉니다(임의 outlier
//!   제거 금지).
//! - **release gate**(XVI.4.4): required scenario가 모두 `p50 ≤ 5`, `p95 ≤ 7`, `p99 ≤ 9`, `controllable max ≤ 10`
//!   ms를 지켜야 통과입니다. 경계값(같음)은 통과입니다. sample이 하나도 없는 required scenario는 실패입니다.
//!   판정 sample이 `scenario_min_samples`(1,000)보다 적거나, 선언한 cache setup이 실제 hit·miss와 맞지 않는 required
//!   scenario도 실패입니다(ADR-0096 결정 5·6). 통계가 그 scenario를 대표하지 못하기 때문입니다.

use std::fmt;

use crate::budget::InteractionBudget;

/// 통제할 수 없는 지연의 사전 정의 코드. 이 코드가 있는 sample만 판정에서 뺍니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExclusionCode {
    /// 운영체제 선점(스레드가 실행되지 못한 구간이 측정됨).
    OsPreemption,
    /// GPU 장치 상실·재생성.
    DeviceLost,
    /// 절전·깨우기 전환.
    SleepTransition,
    /// GPU 드라이버 정지.
    DriverStall,
}

impl ExclusionCode {
    /// 모든 코드.
    pub const ALL: [ExclusionCode; 4] = [
        ExclusionCode::OsPreemption,
        ExclusionCode::DeviceLost,
        ExclusionCode::SleepTransition,
        ExclusionCode::DriverStall,
    ];

    /// 고정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            ExclusionCode::OsPreemption => "os_preemption",
            ExclusionCode::DeviceLost => "device_lost",
            ExclusionCode::SleepTransition => "sleep_transition",
            ExclusionCode::DriverStall => "driver_stall",
        }
    }
}

/// sample 하나(Logical Interaction Latency).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LatencySample {
    /// T5 − T0(ns).
    pub latency_nanos: u64,
    /// 통제할 수 없는 지연 코드(있으면 제외).
    pub exclusion: Option<ExclusionCode>,
}

impl LatencySample {
    /// 통제 가능한 sample.
    pub const fn controllable(latency_nanos: u64) -> LatencySample {
        LatencySample {
            latency_nanos,
            exclusion: None,
        }
    }
}

/// 통계 결과(ns).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LatencyStats {
    /// 판정에 쓴 sample 수.
    pub count: u64,
    /// 제외한 sample 수(코드별, [`ExclusionCode::ALL`] 순서).
    pub excluded: [u64; 4],
    /// p50.
    pub p50_nanos: u64,
    /// p95.
    pub p95_nanos: u64,
    /// p99.
    pub p99_nanos: u64,
    /// 통제 가능한 최댓값.
    pub max_nanos: u64,
    /// 통제 가능 상한 초과 수.
    pub over_limit: u64,
}

/// 정렬된 값의 nearest-rank 백분위.
pub fn percentile(sorted: &[u64], percent: u64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let n = sorted.len() as u64;
    let rank = (percent * n).div_ceil(100).clamp(1, n);
    sorted[rank as usize - 1]
}

/// sample을 분석합니다(XVI.4.2, XVI.4.3).
pub fn analyze(samples: &[LatencySample], budget: &InteractionBudget) -> LatencyStats {
    let mut stats = LatencyStats::default();
    let mut values: Vec<u64> = Vec::with_capacity(samples.len());
    let limit = budget.controllable_max_us.saturating_mul(1_000);
    for sample in samples {
        match sample.exclusion {
            Some(code) => stats.excluded[code as usize] += 1,
            None => {
                values.push(sample.latency_nanos);
                if sample.latency_nanos > limit {
                    stats.over_limit += 1;
                }
            }
        }
    }
    values.sort_unstable();
    stats.count = values.len() as u64;
    stats.p50_nanos = percentile(&values, 50);
    stats.p95_nanos = percentile(&values, 95);
    stats.p99_nanos = percentile(&values, 99);
    stats.max_nanos = values.last().copied().unwrap_or(0);
    stats
}

/// gate 위반 지표.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GateMetric {
    /// sample 없음.
    NoSamples,
    /// p50.
    P50,
    /// p95.
    P95,
    /// p99.
    P99,
    /// 통제 가능한 최댓값.
    ControllableMax,
    /// 판정 sample이 `scenario_min_samples`보다 적음.
    TooFewSamples,
    /// 선언한 cache setup과 실제 hit·miss가 다름.
    CacheSetup,
}

impl GateMetric {
    /// 고정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            GateMetric::NoSamples => "no_samples",
            GateMetric::P50 => "p50",
            GateMetric::P95 => "p95",
            GateMetric::P99 => "p99",
            GateMetric::ControllableMax => "controllable_max",
            GateMetric::TooFewSamples => "too_few_samples",
            GateMetric::CacheSetup => "cache_setup",
        }
    }
}

/// gate 위반 하나. sample 수 지표는 `value_nanos`·`limit_nanos`에 ns 대신 개수를, cache setup은 0을 씁니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateViolation {
    /// scenario index.
    pub scenario: usize,
    /// 지표.
    pub metric: GateMetric,
    /// 측정값(ns).
    pub value_nanos: u64,
    /// 기준(ns).
    pub limit_nanos: u64,
}

impl fmt::Display for GateViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.metric {
            GateMetric::TooFewSamples => {
                return write!(
                    f,
                    "scenario {} {} sample {}개 < {}개",
                    self.scenario,
                    self.metric.name(),
                    self.value_nanos,
                    self.limit_nanos
                )
            }
            GateMetric::NoSamples | GateMetric::CacheSetup => {
                return write!(f, "scenario {} {}", self.scenario, self.metric.name())
            }
            _ => {}
        }
        write!(
            f,
            "scenario {} {} {}ns > {}ns",
            self.scenario,
            self.metric.name(),
            self.value_nanos,
            self.limit_nanos
        )
    }
}

/// scenario 하나의 gate 입력.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateScenario {
    /// required scenario인지.
    pub required: bool,
    /// 통계.
    pub stats: LatencyStats,
    /// 선언한 cache setup이 실제 hit·miss와 맞는지([`crate::latency_scenario::check_cache_setup`]).
    pub cache_consistent: bool,
}

/// release gate(XVI.4.4). 위반 목록이 비어 있으면 통과입니다.
pub fn release_gate(scenarios: &[GateScenario], budget: &InteractionBudget) -> Vec<GateViolation> {
    let mut violations = Vec::new();
    for (index, scenario) in scenarios.iter().enumerate() {
        if !scenario.required {
            continue;
        }
        let stats = &scenario.stats;
        if !scenario.cache_consistent {
            violations.push(GateViolation {
                scenario: index,
                metric: GateMetric::CacheSetup,
                value_nanos: 0,
                limit_nanos: 0,
            });
        }
        if stats.count == 0 {
            violations.push(GateViolation {
                scenario: index,
                metric: GateMetric::NoSamples,
                value_nanos: 0,
                limit_nanos: 0,
            });
            continue;
        }
        if stats.count < budget.scenario_min_samples {
            violations.push(GateViolation {
                scenario: index,
                metric: GateMetric::TooFewSamples,
                value_nanos: stats.count,
                limit_nanos: budget.scenario_min_samples,
            });
        }
        for (metric, value, limit_us) in [
            (GateMetric::P50, stats.p50_nanos, budget.p50_us),
            (GateMetric::P95, stats.p95_nanos, budget.p95_us),
            (GateMetric::P99, stats.p99_nanos, budget.p99_us),
            (
                GateMetric::ControllableMax,
                stats.max_nanos,
                budget.controllable_max_us,
            ),
        ] {
            let limit_nanos = limit_us.saturating_mul(1_000);
            if value > limit_nanos {
                violations.push(GateViolation {
                    scenario: index,
                    metric,
                    value_nanos: value,
                    limit_nanos,
                });
            }
        }
    }
    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget() -> InteractionBudget {
        InteractionBudget::parse(include_str!("../../../config/performance-budgets.yaml")).unwrap()
    }

    fn ms(value: f64) -> u64 {
        (value * 1_000_000.0) as u64
    }

    #[test]
    fn reference_statistics() {
        // 검증 기준(XVI.4.2): 통계 reference fixture. 1..=100 ms.
        let samples: Vec<LatencySample> = (1..=100)
            .map(|v| LatencySample::controllable(ms(v as f64)))
            .collect();
        let stats = analyze(&samples, &budget());
        assert_eq!(stats.count, 100);
        assert_eq!(stats.p50_nanos, ms(50.0));
        assert_eq!(stats.p95_nanos, ms(95.0));
        assert_eq!(stats.p99_nanos, ms(99.0));
        assert_eq!(stats.max_nanos, ms(100.0));
        // 10ms 초과: 11..=100.
        assert_eq!(stats.over_limit, 90);
        // 작은 표본의 nearest-rank.
        let few = [3, 1, 2].map(LatencySample::controllable);
        let stats = analyze(&few, &budget());
        assert_eq!(
            (stats.p50_nanos, stats.p95_nanos, stats.p99_nanos),
            (2, 3, 3)
        );
        assert_eq!(percentile(&[], 50), 0);
    }

    #[test]
    fn only_predefined_codes_exclude_samples() {
        // 검증 기준(XVI.4.3): arbitrary high sample이 제외되지 않습니다.
        let mut samples: Vec<LatencySample> = (0..99)
            .map(|_| LatencySample::controllable(ms(2.0)))
            .collect();
        samples.push(LatencySample::controllable(ms(500.0)));
        let stats = analyze(&samples, &budget());
        assert_eq!(stats.count, 100);
        assert_eq!(stats.max_nanos, ms(500.0));
        assert_eq!(stats.over_limit, 1);
        // 같은 값이라도 사전 정의 코드가 있으면 뺍니다.
        samples.push(LatencySample {
            latency_nanos: ms(800.0),
            exclusion: Some(ExclusionCode::SleepTransition),
        });
        samples.push(LatencySample {
            latency_nanos: ms(900.0),
            exclusion: Some(ExclusionCode::OsPreemption),
        });
        let stats = analyze(&samples, &budget());
        assert_eq!(stats.count, 100);
        assert_eq!(stats.max_nanos, ms(500.0));
        assert_eq!(stats.excluded, [1, 0, 1, 0]);
    }

    #[test]
    fn gate_threshold_boundaries() {
        // 검증 기준(XVI.4.4): threshold boundary fixture. 경계값은 통과, 1ns 넘으면 실패.
        let budget = budget();
        let at = LatencyStats {
            count: 1_000,
            p50_nanos: ms(5.0),
            p95_nanos: ms(7.0),
            p99_nanos: ms(9.0),
            max_nanos: ms(10.0),
            ..LatencyStats::default()
        };
        let pass = GateScenario {
            required: true,
            stats: at,
            cache_consistent: true,
        };
        assert!(release_gate(&[pass], &budget).is_empty());
        for (metric, bump) in [
            (
                GateMetric::P50,
                LatencyStats {
                    p50_nanos: ms(5.0) + 1,
                    ..at
                },
            ),
            (
                GateMetric::P95,
                LatencyStats {
                    p95_nanos: ms(7.0) + 1,
                    ..at
                },
            ),
            (
                GateMetric::P99,
                LatencyStats {
                    p99_nanos: ms(9.0) + 1,
                    ..at
                },
            ),
            (
                GateMetric::ControllableMax,
                LatencyStats {
                    max_nanos: ms(10.0) + 1,
                    ..at
                },
            ),
        ] {
            let violations = release_gate(
                &[
                    pass,
                    GateScenario {
                        required: true,
                        stats: bump,
                        cache_consistent: true,
                    },
                ],
                &budget,
            );
            assert_eq!(violations.len(), 1, "{metric:?}");
            assert_eq!(violations[0].metric, metric);
            assert_eq!(violations[0].scenario, 1);
        }
        // 필수가 아닌 scenario는 판정하지 않고, 필수인데 sample이 없으면 실패입니다.
        let slow = GateScenario {
            required: false,
            stats: LatencyStats {
                p50_nanos: ms(50.0),
                ..at
            },
            cache_consistent: false,
        };
        assert!(release_gate(&[slow], &budget).is_empty());
        let empty = GateScenario {
            required: true,
            stats: LatencyStats::default(),
            cache_consistent: true,
        };
        assert_eq!(
            release_gate(&[empty], &budget)[0].metric,
            GateMetric::NoSamples
        );
        // sample 수 경계: scenario_min_samples(1,000)는 통과, 하나 적으면 실패.
        let few = GateScenario {
            stats: LatencyStats { count: 999, ..at },
            ..pass
        };
        let violations = release_gate(&[few], &budget);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].metric, GateMetric::TooFewSamples);
        assert_eq!(
            (violations[0].value_nanos, violations[0].limit_nanos),
            (999, 1_000)
        );
        // cache setup이 실제와 다르면 통계가 좋아도 실패.
        let mismatched = GateScenario {
            cache_consistent: false,
            ..pass
        };
        let violations = release_gate(&[mismatched], &budget);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].metric, GateMetric::CacheSetup);
    }
}
