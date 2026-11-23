//! 성능 입력 재생과 결과 요약 (XVI.2.1, XVI.2.2, ADR-0084 결정 9·11).
//!
//! core replay runner([`runner`])가 재생 파일을 실행하고, [`build_result`]가 trace를 scenario 통계·release gate·
//! 결과 schema로 요약합니다. 환경 값은 인자로 주거나, 없으면 `git rev-parse HEAD`·`sysctl`·`sw_vers`에서 읽습니다.

pub mod file;
pub mod layout;
pub mod runner;

use std::path::Path;
use std::process::Command;

use stellar_diagnostics::analyzer::{
    analyze, percentile, release_gate, GateScenario, LatencySample,
};
use stellar_diagnostics::budget::InteractionBudget;
use stellar_diagnostics::interaction::StageSpan;
use stellar_diagnostics::result::{
    BuildProfile, FixtureIdentity, FixtureName, HardwareProfile, PerformanceResult,
    ResultEnvironment, ScenarioResult,
};

use file::FixtureSpec;
use runner::ReplayRun;
#[cfg(test)]
use file::ReplayScenario;
#[cfg(test)]
use runner::{load_fixture, run_replay, standard_replay};

/// 기본 fixture: 가시 node 20,000(아키텍처 §5.3).
pub const DEFAULT_FIXTURE: FixtureSpec = FixtureSpec {
    name: FixtureName::VisibleNodes,
    seed: 1,
    size: 20_000,
};

/// 측정 환경 인자.
#[derive(Debug, Clone, Default)]
pub struct EnvironmentArgs {
    /// commit.
    pub commit: Option<String>,
    /// hardware model.
    pub model: Option<String>,
    /// CPU.
    pub cpu: Option<String>,
    /// 메모리(GiB).
    pub memory_gib: Option<u32>,
    /// OS build.
    pub os_build: Option<String>,
    /// app build.
    pub app_build: Option<String>,
}

fn command_output(program: &str, args: &[&str], dir: &Path) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// 환경을 정합니다(인자 우선, 없으면 시스템에서 읽음).
pub fn environment(args: &EnvironmentArgs, repo_root: &Path) -> Result<ResultEnvironment, String> {
    let commit = args
        .commit
        .clone()
        .or_else(|| command_output("git", &["rev-parse", "HEAD"], repo_root))
        .unwrap_or_else(|| "0".repeat(40));
    let model = args
        .model
        .clone()
        .or_else(|| command_output("sysctl", &["-n", "hw.model"], repo_root))
        .unwrap_or_else(|| "unknown".to_owned());
    let cpu = args
        .cpu
        .clone()
        .or_else(|| command_output("sysctl", &["-n", "machdep.cpu.brand_string"], repo_root))
        .unwrap_or_else(|| "unknown".to_owned());
    let memory = args.memory_gib.unwrap_or_else(|| {
        command_output("sysctl", &["-n", "hw.memsize"], repo_root)
            .and_then(|bytes| bytes.parse::<u64>().ok())
            .map_or(0, |bytes| u32::try_from(bytes >> 30).unwrap_or(u32::MAX))
    });
    let os_build = args
        .os_build
        .clone()
        .or_else(|| command_output("sw_vers", &["-buildVersion"], repo_root))
        .unwrap_or_else(|| "unknown".to_owned());
    let app_build = args
        .app_build
        .clone()
        .unwrap_or_else(|| format!("0.0.0+{}", &commit[..commit.len().min(8)]));
    let hardware = HardwareProfile::new(&model, &cpu, memory).map_err(|e| e.to_string())?;
    ResultEnvironment::new(&commit, hardware, &os_build, &app_build).map_err(|e| e.to_string())
}

/// 재생 결과를 scenario 결과로 요약합니다.
pub fn summarize(run: &ReplayRun, budget: &InteractionBudget) -> ScenarioResult {
    let samples: Vec<LatencySample> = run
        .traces
        .iter()
        .map(|t| LatencySample::controllable(t.logical_latency_nanos()))
        .collect();
    let latency = analyze(&samples, budget);
    let mut span_p99_nanos = [0u64; 5];
    for (slot, span) in span_p99_nanos.iter_mut().zip(StageSpan::ALL) {
        let mut values: Vec<u64> = run.traces.iter().map(|t| t.span_nanos(span)).collect();
        values.sort_unstable();
        *slot = percentile(&values, 99);
    }
    ScenarioResult {
        kind: run.scenario.kind(),
        required: true,
        latency,
        span_p99_nanos,
        traces: run.traces.len() as u64,
        scene_hash: run.scene_hash,
    }
}

/// 여러 재생 결과로 결과 문서를 만듭니다.
pub fn build_result(
    environment: ResultEnvironment,
    fixture: FixtureIdentity,
    runs: &[ReplayRun],
    budget: &InteractionBudget,
) -> PerformanceResult {
    let scenarios: Vec<ScenarioResult> = runs.iter().map(|run| summarize(run, budget)).collect();
    let gate: Vec<GateScenario> = scenarios
        .iter()
        .map(|s| GateScenario {
            required: s.required,
            stats: s.latency,
        })
        .collect();
    PerformanceResult {
        environment,
        fixture,
        profile: BuildProfile::current(),
        violations: release_gate(&gate, budget),
        scenarios,
    }
}

fn budget(repo_root: &Path) -> Result<InteractionBudget, String> {
    let text = std::fs::read_to_string(repo_root.join("config/performance-budgets.yaml"))
        .map_err(|e| format!("performance-budgets.yaml: {e}"))?;
    InteractionBudget::parse(&text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
