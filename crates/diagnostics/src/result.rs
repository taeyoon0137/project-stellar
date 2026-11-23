//! 성능 측정 결과 보존 schema (XVI.4.5, ADR-0084 결정 11).
//!
//! 결과는 commit, hardware profile, OS build, app build와 함께 JSON 한 개로 남깁니다. 사용자 project 정보(경로·
//! 이름·text·Git URL·branch)는 담지 않습니다.
//!
//! - 문자열 field는 생성할 때 형식을 검사합니다([`ResultEnvironment::new`]). commit은 소문자 hex 40·64자, OS·app
//!   build는 영문·숫자·`.`·`+`·`-`·`_`, hardware 이름은 그에 공백·`,`·`(`·`)`만 더합니다. `/`·`\`·`:`·`@`는 허용하지
//!   않으므로 경로·URL·이메일을 넣을 수 없습니다.
//! - fixture는 저장소 밖에서 생성한 synthetic fixture의 이름(닫힌 목록)·seed·규모·content hash입니다. 사용자 문서를
//!   재생하는 경로는 없습니다.
//! - JSON key는 [`RESULT_KEYS`]로 고정합니다.
//! - v2(ADR-0096 결정 5): scenario마다 cache setup(`warm`·`cold`), 측정 구간의 실제 cache hit·miss, setup 일치 여부,
//!   측정 전 setup event 수를 남깁니다.

use std::fmt;
use std::fmt::Write as _;

use crate::analyzer::{ExclusionCode, GateViolation, LatencyStats};
use crate::interaction::{CacheState, InteractionKind, StageSpan};
use crate::latency_scenario::CacheObservation;

/// 결과 schema 이름과 version.
pub const RESULT_SCHEMA: &str = "stellar.performance-result.v2";

/// 결과 문자열 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultFieldError {
    /// 형식이 맞지 않습니다.
    Invalid {
        /// field.
        field: &'static str,
    },
}

impl fmt::Display for ResultFieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResultFieldError::Invalid { field } => {
                write!(f, "결과 field {field}의 형식이 잘못되었습니다")
            }
        }
    }
}

impl std::error::Error for ResultFieldError {}

fn check(
    field: &'static str,
    value: &str,
    max: usize,
    allowed: impl Fn(char) -> bool,
) -> Result<String, ResultFieldError> {
    if value.is_empty() || value.len() > max || !value.chars().all(allowed) {
        return Err(ResultFieldError::Invalid { field });
    }
    Ok(value.to_owned())
}

fn build_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-' | '_')
}

fn hardware_char(c: char) -> bool {
    build_char(c) || matches!(c, ' ' | ',' | '(' | ')')
}

/// 기준 장비 profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HardwareProfile {
    model: String,
    cpu: String,
    memory_gib: u32,
}

impl HardwareProfile {
    /// 검사한 profile(예: `MacBookPro17,1`, `Apple M1`, 16).
    pub fn new(
        model: &str,
        cpu: &str,
        memory_gib: u32,
    ) -> Result<HardwareProfile, ResultFieldError> {
        Ok(HardwareProfile {
            model: check("hardware.model", model, 64, hardware_char)?,
            cpu: check("hardware.cpu", cpu, 64, hardware_char)?,
            memory_gib,
        })
    }
}

/// 측정 환경.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultEnvironment {
    commit: String,
    hardware: HardwareProfile,
    os_build: String,
    app_build: String,
}

impl ResultEnvironment {
    /// 검사한 환경.
    pub fn new(
        commit: &str,
        hardware: HardwareProfile,
        os_build: &str,
        app_build: &str,
    ) -> Result<ResultEnvironment, ResultFieldError> {
        let hex = |c: char| c.is_ascii_digit() || ('a'..='f').contains(&c);
        if !(commit.len() == 40 || commit.len() == 64) {
            return Err(ResultFieldError::Invalid { field: "commit" });
        }
        Ok(ResultEnvironment {
            commit: check("commit", commit, 64, hex)?,
            hardware,
            os_build: check("os_build", os_build, 32, build_char)?,
            app_build: check("app_build", app_build, 40, build_char)?,
        })
    }
}

/// synthetic fixture 이름(닫힌 목록).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FixtureName {
    /// 가시 node 목표 fixture(XVI.3.2).
    VisibleNodes,
}

impl FixtureName {
    /// 고정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            FixtureName::VisibleNodes => "visible-nodes",
        }
    }

    /// 이름에서 찾습니다.
    pub fn parse(text: &str) -> Option<FixtureName> {
        (text == "visible-nodes").then_some(FixtureName::VisibleNodes)
    }
}

/// fixture 식별.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixtureIdentity {
    /// 이름.
    pub name: FixtureName,
    /// seed.
    pub seed: u64,
    /// 규모(가시 node 목표).
    pub size: u64,
    /// content hash(32 bytes).
    pub content_hash: [u8; 32],
}

/// scenario 하나의 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioResult {
    /// 상호작용 종류.
    pub kind: InteractionKind,
    /// required scenario인지.
    pub required: bool,
    /// Logical Interaction Latency 통계.
    pub latency: LatencyStats,
    /// 구간별 p99(ns, [`StageSpan::ALL`] 순서).
    pub span_p99_nanos: [u64; 5],
    /// trace event 수.
    pub traces: u64,
    /// 마지막 scene semantic hash(재생 결정성 비교용).
    pub scene_hash: u128,
    /// 선언한 cache setup.
    pub cache: CacheState,
    /// 측정 구간에서 관측한 cache hit·miss.
    pub cache_observation: CacheObservation,
    /// 선언한 setup과 관측이 맞는지.
    pub cache_consistent: bool,
    /// 측정 전에 실행한 setup event 수(sample 아님).
    pub setup_events: u64,
}

/// 결과 전체.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerformanceResult {
    /// 환경.
    pub environment: ResultEnvironment,
    /// fixture.
    pub fixture: FixtureIdentity,
    /// build profile(`debug`·`release`).
    pub profile: BuildProfile,
    /// scenario 결과.
    pub scenarios: Vec<ScenarioResult>,
    /// release gate 위반.
    pub violations: Vec<GateViolation>,
}

/// build profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildProfile {
    /// debug.
    Debug,
    /// release.
    Release,
}

impl BuildProfile {
    /// 고정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            BuildProfile::Debug => "debug",
            BuildProfile::Release => "release",
        }
    }

    /// 현재 build.
    pub const fn current() -> BuildProfile {
        if cfg!(debug_assertions) {
            BuildProfile::Debug
        } else {
            BuildProfile::Release
        }
    }
}

/// JSON에 쓸 수 있는 key 전체(privacy schema).
pub const RESULT_KEYS: [&str; 50] = [
    "schema",
    "environment",
    "commit",
    "hardware",
    "model",
    "cpu",
    "memory_gib",
    "os_build",
    "app_build",
    "fixture",
    "name",
    "seed",
    "size",
    "content_hash",
    "profile",
    "scenarios",
    "kind",
    "required",
    "count",
    "excluded",
    "p50_ns",
    "p95_ns",
    "p99_ns",
    "max_ns",
    "over_limit",
    "span_p99_ns",
    "traces",
    "scene_hash",
    "gate",
    "pass",
    "violations",
    "scenario",
    "metric",
    "value_ns",
    "limit_ns",
    "input",
    "command",
    "layout",
    "scene",
    "present",
    "cache",
    "state",
    "hits",
    "misses",
    "full_builds",
    "first_hits",
    "first_misses",
    "first_full",
    "consistent",
    "setup_events",
];

fn first<T: fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(|| "null".to_owned(), |v| v.to_string())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl PerformanceResult {
    /// gate 통과 여부.
    pub fn passed(&self) -> bool {
        self.violations.is_empty()
    }

    /// 결정적 JSON(key 순서 고정, 들여쓰기 2칸).
    pub fn to_json(&self) -> String {
        let env = &self.environment;
        let mut out = String::new();
        let _ = writeln!(out, "{{");
        let _ = writeln!(out, "  \"schema\": \"{RESULT_SCHEMA}\",");
        let _ = writeln!(
            out,
            "  \"environment\": {{\"commit\": \"{}\", \"hardware\": {{\"model\": \"{}\", \"cpu\": \"{}\", \
             \"memory_gib\": {}}}, \"os_build\": \"{}\", \"app_build\": \"{}\"}},",
            env.commit, env.hardware.model, env.hardware.cpu, env.hardware.memory_gib, env.os_build, env.app_build
        );
        let _ = writeln!(
            out,
            "  \"fixture\": {{\"name\": \"{}\", \"seed\": {}, \"size\": {}, \"content_hash\": \"{}\"}},",
            self.fixture.name.name(),
            self.fixture.seed,
            self.fixture.size,
            hex(&self.fixture.content_hash)
        );
        let _ = writeln!(out, "  \"profile\": \"{}\",", self.profile.name());
        let _ = writeln!(out, "  \"scenarios\": [");
        for (i, s) in self.scenarios.iter().enumerate() {
            let excluded: Vec<String> = ExclusionCode::ALL
                .iter()
                .zip(s.latency.excluded)
                .map(|(code, n)| format!("\"{}\": {n}", code.name()))
                .collect();
            let spans: Vec<String> = StageSpan::ALL
                .iter()
                .zip(s.span_p99_nanos)
                .map(|(span, n)| format!("\"{}\": {n}", span.name()))
                .collect();
            let _ = write!(
                out,
                "    {{\"kind\": \"{}\", \"required\": {}, \"count\": {}, \"excluded\": {{{}}}, \"p50_ns\": {}, \
                 \"p95_ns\": {}, \"p99_ns\": {}, \"max_ns\": {}, \"over_limit\": {}, \"span_p99_ns\": {{{}}}, \
                 \"traces\": {}, \"scene_hash\": \"{:032x}\", \"setup_events\": {}, \"cache\": {{\"state\": \"{}\", \
                 \"hits\": {}, \"misses\": {}, \"full_builds\": {}, \"first_hits\": {}, \"first_misses\": {}, \
                 \"first_full\": {}, \"consistent\": {}}}}}",
                s.kind.name(),
                s.required,
                s.latency.count,
                excluded.join(", "),
                s.latency.p50_nanos,
                s.latency.p95_nanos,
                s.latency.p99_nanos,
                s.latency.max_nanos,
                s.latency.over_limit,
                spans.join(", "),
                s.traces,
                s.scene_hash,
                s.setup_events,
                s.cache.name(),
                s.cache_observation.hits,
                s.cache_observation.misses,
                s.cache_observation.full_builds,
                first(s.cache_observation.first.map(|b| b.hits)),
                first(s.cache_observation.first.map(|b| b.misses)),
                first(s.cache_observation.first.map(|b| b.full)),
                s.cache_consistent
            );
            let _ = writeln!(
                out,
                "{}",
                if i + 1 < self.scenarios.len() {
                    ","
                } else {
                    ""
                }
            );
        }
        let _ = writeln!(out, "  ],");
        let violations: Vec<String> = self
            .violations
            .iter()
            .map(|v| {
                format!(
                    "{{\"scenario\": {}, \"metric\": \"{}\", \"value_ns\": {}, \"limit_ns\": {}}}",
                    v.scenario,
                    v.metric.name(),
                    v.value_nanos,
                    v.limit_nanos
                )
            })
            .collect();
        let _ = writeln!(
            out,
            "  \"gate\": {{\"pass\": {}, \"violations\": [{}]}}",
            self.passed(),
            violations.join(", ")
        );
        let _ = writeln!(out, "}}");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::GateMetric;

    fn result() -> PerformanceResult {
        PerformanceResult {
            environment: ResultEnvironment::new(
                "0123456789abcdef0123456789abcdef01234567",
                HardwareProfile::new("MacBookPro17,1", "Apple M1", 16).unwrap(),
                "25C56",
                "0.0.0+d2bf97b3",
            )
            .unwrap(),
            fixture: FixtureIdentity {
                name: FixtureName::VisibleNodes,
                seed: 1,
                size: 20_000,
                content_hash: [0xab; 32],
            },
            profile: BuildProfile::Release,
            scenarios: vec![ScenarioResult {
                kind: InteractionKind::Drag,
                required: true,
                latency: LatencyStats {
                    count: 3,
                    excluded: [0, 0, 1, 0],
                    p50_nanos: 1,
                    p95_nanos: 2,
                    p99_nanos: 3,
                    max_nanos: 3,
                    over_limit: 0,
                },
                span_p99_nanos: [1, 2, 3, 4, 5],
                traces: 4,
                scene_hash: 7,
                cache: CacheState::Cold,
                cache_observation: CacheObservation {
                    hits: 0,
                    misses: 20,
                    full_builds: 1,
                    first: Some(crate::latency_scenario::CacheBuild {
                        hits: 0,
                        misses: 20,
                        full: true,
                    }),
                },
                cache_consistent: true,
                setup_events: 2,
            }],
            violations: vec![GateViolation {
                scenario: 0,
                metric: GateMetric::P99,
                value_nanos: 10,
                limit_nanos: 9,
            }],
        }
    }

    /// JSON에서 모든 key 문자열(`"key":`)을 꺼냅니다.
    fn keys(json: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = json;
        while let Some(start) = rest.find('"') {
            let after = &rest[start + 1..];
            let Some(end) = after.find('"') else { break };
            let token = &after[..end];
            let tail = &after[end + 1..];
            if tail.starts_with(':') {
                out.push(token.to_owned());
            }
            rest = tail;
        }
        out
    }

    #[test]
    fn result_json_uses_only_allowed_keys() {
        // 검증 기준(XVI.4.5): result schema privacy 검사.
        let json = result().to_json();
        let excluded_names: Vec<&str> = ExclusionCode::ALL.iter().map(|c| c.name()).collect();
        for key in keys(&json) {
            assert!(
                RESULT_KEYS.contains(&key.as_str()) || excluded_names.contains(&key.as_str()),
                "허용하지 않은 key {key}"
            );
        }
        assert!(json.contains("\"commit\": \"0123456789abcdef0123456789abcdef01234567\""));
        assert!(json.contains("\"gate\": {\"pass\": false"));
        assert!(json.contains("\"schema\": \"stellar.performance-result.v2\""));
        assert!(json.contains(
            "\"setup_events\": 2, \"cache\": {\"state\": \"cold\", \"hits\": 0, \"misses\": 20, \"full_builds\": 1, \
             \"first_hits\": 0, \"first_misses\": 20, \"first_full\": true, \"consistent\": true}"
        ));
        // 경로·URL·이메일 문자가 없습니다.
        for forbidden in ['/', '\\', '@'] {
            assert!(!json.contains(forbidden), "{forbidden}");
        }
    }

    #[test]
    fn rejects_project_like_strings() {
        let hw = || HardwareProfile::new("Mac", "Apple M1", 16).unwrap();
        for (commit, os, app) in [
            ("main", "25C56", "1.0"),
            ("0123456789ABCDEF0123456789ABCDEF01234567", "25C56", "1.0"),
            (
                "0123456789abcdef0123456789abcdef01234567",
                "/Users/me",
                "1.0",
            ),
            (
                "0123456789abcdef0123456789abcdef01234567",
                "25C56",
                "git@github.com:me/x",
            ),
            ("0123456789abcdef0123456789abcdef01234567", "25C56", ""),
        ] {
            assert!(
                ResultEnvironment::new(commit, hw(), os, app).is_err(),
                "{commit} {os} {app}"
            );
        }
        assert!(HardwareProfile::new("~/Projects/My App", "M1", 8).is_err());
        assert!(HardwareProfile::new("Mac", "https:x", 8).is_err());
    }
}
