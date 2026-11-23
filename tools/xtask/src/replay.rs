//! `cargo xtask replay` — 성능 입력 재생 CLI (XVI.2.1, XVI.2.2, XVI.2.5, XVI.2.6, XXIV.3.3, ADR-0084 결정 9·11,
//! ADR-0096).
//!
//! ```text
//! cargo xtask replay record <scenario> --out <file> [--seed N] [--size N] [--samples N] [--cache warm|cold]
//! cargo xtask replay record-suite --dir <dir> [--seed N] [--size N] [--samples N]
//! cargo xtask replay run <file>... [--out <result.json>] [--gate]
//!     [--commit <hex>] [--hardware-model <s>] [--hardware-cpu <s>] [--memory-gib <n>]
//!     [--os-build <s>] [--app-build <s>]
//! ```
//!
//! - `record`는 synthetic fixture(`visible-nodes`, 기본 seed 1, 가시 node 20,000)를 만들어 표준 scenario의 재생
//!   파일을 씁니다. 판정 sample은 `--samples`(기본 1,000)개 이상, cache setup은 `--cache`(기본 warm)입니다. 파일은
//!   프로젝트 밖 아무 경로에나 둡니다.
//! - `record-suite`는 core replay가 실행할 수 있는 시나리오마다 warm·cold 재생 파일
//!   (`<scenario>.<cache>.replay`)을 `--dir`에 씁니다. 엔진 경로가 이어지지 않은 시나리오는 이유를 표준 오류에
//!   적고 건너뜁니다.
//! - `run`은 파일마다 fixture를 다시 만들고 content hash를 확인한 뒤 core replay를 실행합니다. 결과 JSON
//!   (`stellar.performance-result.v2`)을 표준 출력이나 `--out`에 씁니다. `--gate`면 release gate 위반 때 종료
//!   코드 65입니다. release gate는 §38.1 시나리오 13개 각각의 warm·cold 결과가 모두 있어야 통과합니다. 실행에
//!   없는 조합은 sample 0개 결과로 넣어 실패합니다(빼지 않음).
//! - 환경 값은 인자로 주거나, 없으면 `git rev-parse HEAD`·`sysctl`·`sw_vers`에서 읽습니다. 읽지 못하면 `unknown`입니다.
//!   기준 장비(reference runner)에서 같은 명령을 실행하면 같은 schema의 결과가 나옵니다.

pub mod file;
pub mod layout;
pub mod runner;

use std::path::Path;
use std::process::Command;

use stellar_diagnostics::analyzer::{
    analyze, percentile, release_gate, GateScenario, LatencySample,
};
use stellar_diagnostics::budget::InteractionBudget;
use stellar_diagnostics::interaction::{CacheState, StageSpan};
use stellar_diagnostics::latency_scenario::{CacheObservation, CoreReplay};
use stellar_diagnostics::result::{
    BuildProfile, FixtureIdentity, FixtureName, HardwareProfile, PerformanceResult,
    ResultEnvironment, ScenarioResult,
};

use file::{FixtureSpec, ReplayFile, ReplayScenario};
use runner::{load_fixture, run_replay, standard_replay, ReplayRun, DEFAULT_SAMPLES};

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
        required: run.scenario.required(),
        latency,
        span_p99_nanos,
        traces: run.traces.len() as u64,
        scene_hash: run.scene_hash,
        cache: run.cache,
        cache_observation: run.cache_observation,
        cache_consistent: run.cache_consistent,
        setup_events: run.setup_events as u64,
    }
}

/// 두 cache setup.
pub const CACHE_STATES: [CacheState; 2] = [CacheState::Warm, CacheState::Cold];

/// 실행에 없는 required (시나리오, cache) 조합의 sample 0개 결과(gate에서 NoSamples로 실패).
fn missing_required(scenarios: &[ScenarioResult]) -> Vec<ScenarioResult> {
    let mut out = Vec::new();
    for scenario in ReplayScenario::ALL {
        if !scenario.required() {
            continue;
        }
        for cache in CACHE_STATES {
            if scenarios
                .iter()
                .any(|s| s.kind == scenario.kind() && s.cache == cache)
            {
                continue;
            }
            out.push(ScenarioResult {
                kind: scenario.kind(),
                required: true,
                latency: Default::default(),
                span_p99_nanos: [0; 5],
                traces: 0,
                scene_hash: 0,
                cache,
                cache_observation: CacheObservation::default(),
                cache_consistent: true,
                setup_events: 0,
            });
        }
    }
    out
}

/// 여러 재생 결과로 결과 문서를 만듭니다.
pub fn build_result(
    environment: ResultEnvironment,
    fixture: FixtureIdentity,
    runs: &[ReplayRun],
    budget: &InteractionBudget,
) -> PerformanceResult {
    let mut scenarios: Vec<ScenarioResult> =
        runs.iter().map(|run| summarize(run, budget)).collect();
    let missing = missing_required(&scenarios);
    scenarios.extend(missing);
    let gate: Vec<GateScenario> = scenarios
        .iter()
        .map(|s| GateScenario {
            required: s.required,
            stats: s.latency,
            cache_consistent: s.cache_consistent,
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

fn flag_value<'a>(args: &'a [String], i: &mut usize, flag: &str) -> Result<&'a str, String> {
    *i += 1;
    args.get(*i)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} 뒤에 값이 없습니다"))
}

/// CLI 실행. 오류는 (종료 코드, 메시지)입니다.
pub fn run_cli(args: &[String], repo_root: &Path) -> Result<(), (i32, String)> {
    const USAGE: i32 = 64;
    const POLICY: i32 = 65;
    const INTERNAL: i32 = 70;
    let usage = || {
        (
            USAGE,
            "사용법: cargo xtask replay record <scenario> --out <file> [--seed N] [--size N] [--samples N] \
             [--cache warm|cold]\n\
             \x20      cargo xtask replay record-suite --dir <dir> [--seed N] [--size N] [--samples N]\n\
             \x20      cargo xtask replay run <file>... [--out <result.json>] [--gate] [--commit <hex>] \
             [--hardware-model <s>] [--hardware-cpu <s>] [--memory-gib <n>] [--os-build <s>] [--app-build <s>]"
                .to_owned(),
        )
    };
    match args.first().map(String::as_str) {
        Some(command @ ("record" | "record-suite")) => {
            let suite = command == "record-suite";
            let scenario = if suite {
                None
            } else {
                Some(
                    args.get(1)
                        .and_then(|s| ReplayScenario::parse(s))
                        .ok_or_else(usage)?,
                )
            };
            let mut spec = DEFAULT_FIXTURE;
            let mut out = None;
            let mut samples = DEFAULT_SAMPLES;
            let mut cache = CacheState::Warm;
            let mut i = if suite { 1 } else { 2 };
            let number = |text: &str, flag: &str| {
                text.parse::<u64>()
                    .map_err(|_| (USAGE, format!("{flag}는 정수입니다")))
            };
            while i < args.len() {
                match args[i].as_str() {
                    flag @ ("--out" | "--dir") if (flag == "--dir") == suite => {
                        out = Some(flag_value(args, &mut i, flag).map_err(|e| (USAGE, e))?)
                    }
                    "--seed" => {
                        spec.seed = number(
                            flag_value(args, &mut i, "--seed").map_err(|e| (USAGE, e))?,
                            "--seed",
                        )?
                    }
                    "--size" => {
                        spec.size = number(
                            flag_value(args, &mut i, "--size").map_err(|e| (USAGE, e))?,
                            "--size",
                        )?
                    }
                    "--samples" => {
                        samples = number(
                            flag_value(args, &mut i, "--samples").map_err(|e| (USAGE, e))?,
                            "--samples",
                        )?
                    }
                    "--cache" if !suite => {
                        cache = match flag_value(args, &mut i, "--cache").map_err(|e| (USAGE, e))? {
                            "warm" => CacheState::Warm,
                            "cold" => CacheState::Cold,
                            _ => return Err((USAGE, "--cache는 warm·cold입니다".to_owned())),
                        }
                    }
                    _ => return Err(usage()),
                }
                i += 1;
            }
            let out = out.ok_or_else(usage)?;
            let fixture = load_fixture(&spec).map_err(|e| (INTERNAL, e))?;
            let write = |path: &Path, file: &ReplayFile| {
                std::fs::write(path, file.to_text())
                    .map_err(|e| (INTERNAL, format!("{}: {e}", path.display())))
            };
            match scenario {
                Some(scenario) => {
                    let file = standard_replay(scenario, spec, &fixture, samples, cache)
                        .map_err(|e| (USAGE, e))?;
                    write(Path::new(out), &file)?;
                    println!("재생 파일 기록 완료: {out} ({} event)", file.events.len());
                }
                None => {
                    let dir = Path::new(out);
                    std::fs::create_dir_all(dir)
                        .map_err(|e| (INTERNAL, format!("{}: {e}", dir.display())))?;
                    let mut written = 0;
                    for scenario in ReplayScenario::ALL {
                        if let CoreReplay::NotConnected { missing } = scenario.core_replay() {
                            eprintln!("{}: 건너뜀({missing})", scenario.name());
                            continue;
                        }
                        for cache in CACHE_STATES {
                            let file = standard_replay(scenario, spec, &fixture, samples, cache)
                                .map_err(|e| (INTERNAL, e))?;
                            let path =
                                dir.join(format!("{}.{}.replay", scenario.name(), cache.name()));
                            write(&path, &file)?;
                            written += 1;
                        }
                    }
                    println!("재생 파일 {written}개 기록 완료: {out}");
                }
            }
            Ok(())
        }
        Some("run") => {
            let mut files = Vec::new();
            let mut out = None;
            let mut gate = false;
            let mut env = EnvironmentArgs::default();
            let mut i = 1;
            while i < args.len() {
                let arg = args[i].as_str();
                let mut value = |flag: &str| {
                    flag_value(args, &mut i, flag)
                        .map(str::to_owned)
                        .map_err(|e| (USAGE, e))
                };
                match arg {
                    "--out" => out = Some(value("--out")?),
                    "--gate" => gate = true,
                    "--commit" => env.commit = Some(value("--commit")?),
                    "--hardware-model" => env.model = Some(value("--hardware-model")?),
                    "--hardware-cpu" => env.cpu = Some(value("--hardware-cpu")?),
                    "--memory-gib" => {
                        env.memory_gib = Some(
                            value("--memory-gib")?
                                .parse()
                                .map_err(|_| (USAGE, "--memory-gib는 정수입니다".to_owned()))?,
                        )
                    }
                    "--os-build" => env.os_build = Some(value("--os-build")?),
                    "--app-build" => env.app_build = Some(value("--app-build")?),
                    flag if flag.starts_with("--") => return Err(usage()),
                    path => files.push(path.to_owned()),
                }
                i += 1;
            }
            if files.is_empty() {
                return Err(usage());
            }
            let budget = budget(repo_root).map_err(|e| (INTERNAL, e))?;
            let mut replays = Vec::new();
            for path in &files {
                let text =
                    std::fs::read_to_string(path).map_err(|e| (USAGE, format!("{path}: {e}")))?;
                replays
                    .push(ReplayFile::parse(&text).map_err(|e| (USAGE, format!("{path}: {e}")))?);
            }
            let first = replays[0].fixture;
            if replays
                .iter()
                .any(|r| r.fixture != first || r.fixture_hash != replays[0].fixture_hash)
            {
                return Err((
                    USAGE,
                    "한 번 실행의 재생 파일은 같은 fixture를 써야 합니다".to_owned(),
                ));
            }
            let fixture = load_fixture(&first).map_err(|e| (INTERNAL, e))?;
            let mut runs = Vec::new();
            for replay in &replays {
                runs.push(run_replay(replay, &fixture).map_err(|e| (INTERNAL, e))?);
            }
            let environment = environment(&env, repo_root).map_err(|e| (USAGE, e))?;
            let identity = FixtureIdentity {
                name: first.name,
                seed: first.seed,
                size: first.size,
                content_hash: replays[0].fixture_hash,
            };
            for run in &runs {
                // 구간 T3→T4 안의 scene build와 render graph 시간(진단용, 결과 schema 밖).
                eprintln!(
                    "{}.{}: scene build p99 {}ns, render graph p99 {}ns, scene build {}회",
                    run.scenario.name(),
                    run.cache.name(),
                    run.scene_build_p99_nanos,
                    run.graph_compile_p99_nanos,
                    run.scene_builds
                );
            }
            let result = build_result(environment, identity, &runs, &budget);
            let json = result.to_json();
            match out {
                Some(path) => {
                    std::fs::write(&path, &json).map_err(|e| (INTERNAL, format!("{path}: {e}")))?;
                    println!("재생 결과 기록 완료: {path}");
                }
                None => print!("{json}"),
            }
            if gate && !result.passed() {
                let reasons: Vec<String> =
                    result.violations.iter().map(ToString::to_string).collect();
                return Err((POLICY, format!("release gate 실패: {}", reasons.join(", "))));
            }
            Ok(())
        }
        _ => Err(usage()),
    }
}

#[cfg(test)]
mod tests;
