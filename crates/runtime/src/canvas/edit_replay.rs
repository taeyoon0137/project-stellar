//! 캔버스 편집 성능 replay: scenario, 예산, 결과 JSON (XII.10.43, ADR-0117 결정 10).
//!
//! 2만 가시 node fixture에서 scenario마다 최소 [`MIN_SAMPLES`]개 입력의 코어 처리 시간(hit·선택·preview 계산)을 재고
//! [`judge`]로 판정합니다. hover·클릭 선택·핸들 표시는 목표 5ms·상한 8ms, 나머지는 Present 예약 10ms 이하(p99,
//! nearest-rank)입니다. 결과는 `stellar.canvas-edit-replay.v1` JSON([`to_json`])입니다.
//!
//! IME 조합 입력은 Text 편집 경로가 scene 입력에 이어지지 않아 [`ScenarioResult::not_connected`]로 남고 통과가
//! 아닙니다(판정에서 빼지 않음). 측정 실행은 release test `measure_canvas_edit_replay_20k`(ignored)입니다. 기준 장비
//! 판정과 xtask analyzer 연결은 XVI.2.3·XVI.4.2 경로가 합니다.

use std::fmt::Write as _;

/// scenario당 최소 sample 수(ADR-0096 `scenario_min_samples`와 같은 값).
pub const MIN_SAMPLES: usize = 1_000;
/// 결과 schema 이름.
pub const SCHEMA: &str = "stellar.canvas-edit-replay.v1";

/// 캔버스 편집 scenario.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CanvasEditScenario {
    /// 누르지 않은 이동마다 hit test와 hover 대상.
    Hover,
    /// 클릭 선택(hit test와 클릭 규칙).
    ClickSelect,
    /// 선택 핸들 layer 만들기.
    HandleDisplay,
    /// 마퀴 preview 갱신.
    Marquee,
    /// 스냅 포함 이동 drag update.
    SnapMove,
    /// flex 컨테이너 안 크기 조절 update.
    FlexResize,
    /// 회전 update.
    Rotate,
    /// Path point drag update.
    PathPointDrag,
    /// 팬(viewport와 camera).
    Pan,
    /// 줌(viewport와 camera).
    Zoom,
    /// IME 조합 입력(연결 안 됨).
    ImeComposition,
}

/// scenario 예산(µs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    /// 목표(p99). 없으면 상한만 판정합니다.
    pub target_us: Option<u64>,
    /// 상한(p99).
    pub limit_us: u64,
}

impl CanvasEditScenario {
    /// 전체 scenario(결과 순서).
    pub const ALL: [CanvasEditScenario; 11] = [
        CanvasEditScenario::Hover,
        CanvasEditScenario::ClickSelect,
        CanvasEditScenario::HandleDisplay,
        CanvasEditScenario::Marquee,
        CanvasEditScenario::SnapMove,
        CanvasEditScenario::FlexResize,
        CanvasEditScenario::Rotate,
        CanvasEditScenario::PathPointDrag,
        CanvasEditScenario::Pan,
        CanvasEditScenario::Zoom,
        CanvasEditScenario::ImeComposition,
    ];

    /// 결과 JSON 이름.
    pub const fn name(self) -> &'static str {
        match self {
            CanvasEditScenario::Hover => "hover",
            CanvasEditScenario::ClickSelect => "click_select",
            CanvasEditScenario::HandleDisplay => "handle_display",
            CanvasEditScenario::Marquee => "marquee",
            CanvasEditScenario::SnapMove => "snap_move",
            CanvasEditScenario::FlexResize => "flex_resize",
            CanvasEditScenario::Rotate => "rotate",
            CanvasEditScenario::PathPointDrag => "path_point_drag",
            CanvasEditScenario::Pan => "pan",
            CanvasEditScenario::Zoom => "zoom",
            CanvasEditScenario::ImeComposition => "ime_composition",
        }
    }

    /// 예산: hover·선택·핸들 표시는 목표 5ms·상한 8ms, 나머지는 10ms.
    pub const fn budget(self) -> Budget {
        match self {
            CanvasEditScenario::Hover
            | CanvasEditScenario::ClickSelect
            | CanvasEditScenario::HandleDisplay => Budget {
                target_us: Some(5_000),
                limit_us: 8_000,
            },
            _ => Budget {
                target_us: None,
                limit_us: 10_000,
            },
        }
    }
}

/// scenario 하나의 결과.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScenarioResult {
    /// scenario.
    pub scenario: CanvasEditScenario,
    /// sample 수.
    pub samples: usize,
    /// p50(ns).
    pub p50_ns: u64,
    /// p99(ns, nearest-rank).
    pub p99_ns: u64,
    /// 최대(ns).
    pub max_ns: u64,
    /// 경로가 이어지지 않아 재지 못함.
    pub not_connected: bool,
}

impl ScenarioResult {
    /// sample(ns)로 결과를 만듭니다.
    pub fn from_samples(scenario: CanvasEditScenario, mut samples: Vec<u64>) -> ScenarioResult {
        samples.sort_unstable();
        let rank = |q: f64| -> u64 {
            if samples.is_empty() {
                return 0;
            }
            let index = ((q * samples.len() as f64).ceil() as usize).clamp(1, samples.len()) - 1;
            samples[index]
        };
        ScenarioResult {
            scenario,
            samples: samples.len(),
            p50_ns: rank(0.5),
            p99_ns: rank(0.99),
            max_ns: samples.last().copied().unwrap_or(0),
            not_connected: false,
        }
    }

    /// 연결되지 않은 scenario.
    pub fn not_connected(scenario: CanvasEditScenario) -> ScenarioResult {
        ScenarioResult {
            scenario,
            samples: 0,
            p50_ns: 0,
            p99_ns: 0,
            max_ns: 0,
            not_connected: true,
        }
    }
}

/// 판정.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// 상한 안(목표가 있으면 목표 안 여부도 함께).
    Pass {
        /// 목표 안.
        within_target: bool,
    },
    /// sample 부족.
    TooFewSamples,
    /// 상한 초과.
    OverLimit,
    /// 경로 연결 안 됨(통과 아님).
    NotConnected,
}

impl Verdict {
    /// 결과 JSON 이름.
    pub const fn name(self) -> &'static str {
        match self {
            Verdict::Pass { .. } => "pass",
            Verdict::TooFewSamples => "too_few_samples",
            Verdict::OverLimit => "over_limit",
            Verdict::NotConnected => "not_connected",
        }
    }

    /// 통과인지.
    pub const fn passed(self) -> bool {
        matches!(self, Verdict::Pass { .. })
    }
}

/// 결과 하나를 예산으로 판정합니다.
pub fn judge(result: &ScenarioResult) -> Verdict {
    if result.not_connected {
        return Verdict::NotConnected;
    }
    if result.samples < MIN_SAMPLES {
        return Verdict::TooFewSamples;
    }
    let budget = result.scenario.budget();
    let p99_us = result.p99_ns.div_ceil(1_000);
    if p99_us > budget.limit_us {
        return Verdict::OverLimit;
    }
    Verdict::Pass {
        within_target: budget.target_us.is_none_or(|t| p99_us <= t),
    }
}

/// 결과 JSON(닫힌 이름과 숫자만). `fixture`는 fixture 이름입니다.
pub fn to_json(fixture: &str, results: &[ScenarioResult]) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "{{\"schema\":\"{SCHEMA}\",\"fixture\":\"{fixture}\",\"scenarios\":["
    );
    for (i, r) in results.iter().enumerate() {
        let budget = r.scenario.budget();
        let target = budget
            .target_us
            .map_or_else(|| "null".to_string(), |t| t.to_string());
        let _ = write!(
            out,
            "{}{{\"scenario\":\"{}\",\"samples\":{},\"p50_us\":{:.3},\"p99_us\":{:.3},\"max_us\":{:.3},\
             \"target_us\":{target},\"limit_us\":{},\"verdict\":\"{}\"}}",
            if i == 0 { "" } else { "," },
            r.scenario.name(),
            r.samples,
            r.p50_ns as f64 / 1_000.0,
            r.p99_ns as f64 / 1_000.0,
            r.max_ns as f64 / 1_000.0,
            budget.limit_us,
            judge(r).name()
        );
    }
    out.push_str("]}");
    out
}

#[cfg(test)]
mod tests;
