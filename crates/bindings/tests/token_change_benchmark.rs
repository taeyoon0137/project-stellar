//! 단일 token 변경 재평가 benchmark (VIII.9.2).
//!
//! synthetic graph(VIII.9.1)를 실체화해 [`BindingEvaluator`](VIII.6.4)를 만든 뒤, 고정 seed로 고른 토큰마다 표현식을
//! 하나씩 바꿔 증분 재평가의 count(affected·visited·evaluated)와 duration을 측정하고 결과 JSON을 씁니다.
//!
//! 토큰마다 세 번의 단일 변경을 순서대로 측정합니다.
//!
//! - `raise`: 표현식을 `원래 + 1000`으로 바꿉니다. 모든 값이 0~999라 영향 범위 전체로 값이 전파됩니다.
//! - `revert`: 원래 표현식으로 되돌립니다.
//! - `same_value`: 표현식을 `원래 + 0`으로 바꿉니다. 값이 같아 early cutoff로 그 토큰만 평가합니다.
//!
//! count는 graph·선택 seed로 정해지는 결정적 값이고, duration은 실행 환경에 따른 측정값입니다. 기본 실행(`cargo
//! test`)은 작은 설정으로 count를 독립 계산(역방향 도달 집합)과 비교하고 증분 결과를 전체 재평가와 비교합니다.
//! 대규모 측정은 release 빌드에서 환경 변수로 preset을 고릅니다.
//!
//! ```text
//! STELLAR_TOKEN_BENCH_PRESET=edges_1m cargo test -p stellar-bindings --release \
//!     --test token_change_benchmark -- --nocapture
//! ```
//!
//! - `STELLAR_TOKEN_BENCH_PRESET`: `small`(기본) | `edges_1m` | `active_200k`
//! - `STELLAR_TOKEN_BENCH_SAMPLES`: 무작위로 고를 토큰 수(기본 small 8, 그 밖 32). hub(직접 dependents 최다)·sparse(최소) 토큰은
//!   항상 포함합니다.
//! - `STELLAR_TOKEN_BENCH_OUTPUT`: 결과 JSON 경로(기본 `CARGO_TARGET_TMPDIR/token-change-benchmark/<preset>.json`).

use std::collections::{BTreeSet, VecDeque};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use stellar_bindings::binding::BindingSet;
use stellar_bindings::environment_index::EnvironmentSources;
use stellar_bindings::graph::{DependencyNode, NodeIndex, Producer};
use stellar_bindings::registry::TokenRegistry;
use stellar_bindings::scheduler::{
    token_specs, BindingEvaluator, NodeValue, ProducerSpec, StaticSources, UpdateReport,
};
use stellar_bindings::synthetic::{generate, SyntheticConfig, SyntheticGraph, SyntheticRef};
use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::FiniteF32;
use stellar_expressions::ast::{ExprNode, ExprNodeId, ExpressionAst};
use stellar_expressions::environment::EvaluationEnvironment;
use stellar_expressions::ops::BinaryOp;
use stellar_expressions::vm::EvalLimits;

/// 결과 JSON 형식 이름과 버전.
const FORMAT: &str = "stellar.bench.token-change";
const FORMAT_VERSION: u32 = 1;

/// 참고 예산: 기술 아키텍처 §6.2 "의존성·토큰·Binding 재평가" 단계(목표 0.60ms, 절대 상한 1.00ms). 판정 gate가
/// 아니라 결과 해석용 기준입니다.
const BUDGET_TARGET_NS: u64 = 600_000;
const BUDGET_LIMIT_NS: u64 = 1_000_000;

/// `raise` 변경이 더하는 값. synthetic 값(0~999, 환경 기본값 0·1)보다 커서 영향 범위 전체로 전파됩니다.
const RAISE: f32 = 1_000.0;

/// 작은 기본 설정(debug `cargo test`용).
const SMALL: SyntheticConfig = SyntheticConfig {
    seed: 0x5354_454C_4C41_5233,
    tokens: 600,
    bindings: 1_800,
    edges: 6_000,
    max_token_refs: 3,
    environment_one_in: 8,
};

/// 토큰 선택 seed.
const SELECTION_SEED: u64 = 0x5345_4C45_4354_0001;

/// 변경 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChangeKind {
    Raise,
    Revert,
    SameValue,
}

impl ChangeKind {
    const fn name(self) -> &'static str {
        match self {
            ChangeKind::Raise => "raise",
            ChangeKind::Revert => "revert",
            ChangeKind::SameValue => "same_value",
        }
    }
}

/// 토큰을 고른 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Selection {
    /// 직접 dependents가 가장 많은 토큰.
    Hub,
    /// 직접 dependents가 가장 적은 토큰(동률은 큰 번호).
    Sparse,
    /// 고정 seed 무작위.
    Random,
}

impl Selection {
    const fn name(self) -> &'static str {
        match self {
            Selection::Hub => "hub",
            Selection::Sparse => "sparse",
            Selection::Random => "random",
        }
    }
}

/// 단일 변경 측정 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Sample {
    token: u32,
    selection: Selection,
    kind: ChangeKind,
    changed_inputs: usize,
    affected: usize,
    visited: usize,
    evaluated: usize,
    duration_ns: u64,
}

impl Sample {
    /// 결정적인 부분(count)만.
    fn counts(&self) -> (u32, &'static str, &'static str, usize, usize, usize, usize) {
        (
            self.token,
            self.selection.name(),
            self.kind.name(),
            self.changed_inputs,
            self.affected,
            self.visited,
            self.evaluated,
        )
    }
}

/// benchmark 한 번의 결과.
struct BenchResult {
    preset: &'static str,
    config: SyntheticConfig,
    node_count: u64,
    edge_count: u64,
    digest: u64,
    generate: Duration,
    materialize: Duration,
    build: Duration,
    samples: Vec<Sample>,
}

/// SplitMix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// spec에서 직접 계산한 역방향 간선: 토큰 `i`와 binding `j` target 속성을 읽는 producer 목록.
///
/// producer 번호: 토큰 `i` → `i`, binding `j` → `tokens + j`.
struct ReverseSpec {
    tokens: usize,
    dependents: Vec<Vec<usize>>,
}

impl ReverseSpec {
    fn new(graph: &SyntheticGraph) -> ReverseSpec {
        let tokens = graph.tokens.len();
        let mut dependents = vec![Vec::new(); tokens + graph.bindings.len()];
        let producers = graph.tokens.iter().enumerate().chain(
            graph
                .bindings
                .iter()
                .enumerate()
                .map(|(j, p)| (tokens + j, p)),
        );
        for (reader, producer) in producers {
            for r in &producer.refs {
                match r {
                    SyntheticRef::Token(i) => dependents[*i as usize].push(reader),
                    SyntheticRef::Property(j) => dependents[tokens + *j as usize].push(reader),
                    SyntheticRef::Environment(_) => {}
                }
            }
        }
        ReverseSpec { tokens, dependents }
    }

    /// 토큰 `i`에서 역방향으로 도달 가능한 producer 수(자기 자신 포함).
    fn reachable_from_token(&self, i: u32) -> usize {
        let mut seen: BTreeSet<usize> = BTreeSet::new();
        let mut queue: VecDeque<usize> = VecDeque::new();
        seen.insert(i as usize);
        queue.push_back(i as usize);
        while let Some(at) = queue.pop_front() {
            for &next in &self.dependents[at] {
                if seen.insert(next) {
                    queue.push_back(next);
                }
            }
        }
        seen.len()
    }

    /// 직접 dependents가 가장 많은 토큰(동률은 작은 번호).
    fn hub(&self) -> u32 {
        let mut best = 0usize;
        for i in 0..self.tokens {
            if self.dependents[i].len() > self.dependents[best].len() {
                best = i;
            }
        }
        best as u32
    }

    /// 직접 dependents가 가장 적은 토큰(동률은 큰 번호).
    fn sparse(&self) -> u32 {
        let mut best = self.tokens.saturating_sub(1);
        for i in (0..self.tokens).rev() {
            if self.dependents[i].len() < self.dependents[best].len() {
                best = i;
            }
        }
        best as u32
    }
}

/// hub·sparse와 고정 seed 무작위 토큰(서로 다름, 선택 순서).
fn select_tokens(spec: &ReverseSpec, random: usize) -> Vec<(u32, Selection)> {
    let mut picked: Vec<(u32, Selection)> = vec![(spec.hub(), Selection::Hub)];
    if spec.sparse() != spec.hub() {
        picked.push((spec.sparse(), Selection::Sparse));
    }
    let mut rng = Rng(SELECTION_SEED);
    let wanted = random.min(spec.tokens.saturating_sub(picked.len()));
    let target = picked.len() + wanted;
    while picked.len() < target {
        let candidate = (rng.next() % spec.tokens as u64) as u32;
        if !picked.iter().any(|(t, _)| *t == candidate) {
            picked.push((candidate, Selection::Random));
        }
    }
    picked
}

fn number(v: f32) -> TypedValue {
    TypedValue::Number(FiniteF32::new(v).expect("유한"))
}

/// `original + delta`. 새 node ID는 synthetic 표현식 ID 공간(`new_v7(2, 0, k)`)과 겹치지 않습니다.
fn plus(original: &ExpressionAst, delta: f32) -> ExpressionAst {
    let root = ExprNodeId::new(StableId::new_v7(7, 0, 0));
    let literal = ExprNodeId::new(StableId::new_v7(7, 0, 1));
    let mut nodes: Vec<ExprNode> = original.nodes().to_vec();
    nodes.push(ExprNode::literal(literal, number(delta)));
    nodes.push(ExprNode::binary(
        root,
        BinaryOp::Add,
        original.root(),
        literal,
    ));
    ExpressionAst::new(root, nodes).expect("유효한 표현식")
}

/// 모든 node 값.
fn snapshot(evaluator: &BindingEvaluator) -> Vec<NodeValue> {
    evaluator
        .graph()
        .indices()
        .map(|index| evaluator.value_at(index).clone())
        .collect()
}

/// 토큰 `i`의 표현식을 `expression`으로 바꾼 명세로 전체 평가를 새로 합니다(검산용).
fn full_evaluation(
    registry: &TokenRegistry,
    bindings: &BindingSet,
    sources: &EnvironmentSources<'_>,
    token: u32,
    expression: &ExpressionAst,
) -> Vec<NodeValue> {
    let target = SyntheticGraph::token_ref(token);
    let mut specs: Vec<ProducerSpec> = token_specs(registry)
        .into_iter()
        .map(|mut spec| {
            if spec.producer == Producer::Token(target) {
                spec.expression = expression.clone();
            }
            spec
        })
        .collect();
    specs.extend(bindings.iter().map(|binding| ProducerSpec {
        node: DependencyNode::Property(binding.target().clone()),
        producer: Producer::Binding(binding.id()),
        value_type: Some(binding.expected_type()),
        expression: binding.source().clone(),
    }));
    let fresh =
        BindingEvaluator::from_specs(specs, sources, EvalLimits::DEFAULT).expect("전체 평가");
    snapshot(&fresh)
}

/// 단일 변경 하나를 측정합니다.
fn measure(
    evaluator: &mut BindingEvaluator,
    node: &DependencyNode,
    expression: ExpressionAst,
) -> (UpdateReport, Duration) {
    let started = Instant::now();
    let report = evaluator
        .update_expression(node, expression)
        .expect("같은 dependency의 표현식 교체");
    (report, started.elapsed())
}

fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

/// benchmark를 실행합니다. `verify_full`이면 `raise`마다 전체 재평가와 비교합니다(작은 설정 전용).
fn run(
    preset: &'static str,
    config: SyntheticConfig,
    random: usize,
    verify_full: bool,
) -> BenchResult {
    let started = Instant::now();
    let synthetic = generate(config).expect("synthetic graph 생성");
    let generate_time = started.elapsed();
    let started = Instant::now();
    let (registry, bindings) = synthetic.materialize().expect("실체화");
    let materialize_time = started.elapsed();
    let environment = EvaluationEnvironment::new();
    let others = StaticSources::new();
    let sources = EnvironmentSources {
        environment: &environment,
        others: &others,
    };
    let started = Instant::now();
    let mut evaluator = BindingEvaluator::new(&registry, &bindings, &sources).expect("evaluator");
    let build_time = started.elapsed();
    let initial = snapshot(&evaluator);
    assert!(
        initial.iter().all(Result::is_ok),
        "초기 평가는 모두 성공해야 합니다"
    );

    let spec = ReverseSpec::new(&synthetic);
    let mut samples = Vec::new();
    for (token, selection) in select_tokens(&spec, random) {
        let node = DependencyNode::Token(SyntheticGraph::token_ref(token));
        let index: NodeIndex = evaluator.graph().index_of(&node).expect("토큰 node");
        let original = evaluator
            .expression_at(index)
            .cloned()
            .expect("토큰 표현식");
        let reachable = spec.reachable_from_token(token);
        let plan = [
            (ChangeKind::Raise, plus(&original, RAISE)),
            (ChangeKind::Revert, original.clone()),
            (ChangeKind::SameValue, plus(&original, 0.0)),
        ];
        for (kind, expression) in plan {
            let check = (verify_full && kind == ChangeKind::Raise).then(|| expression.clone());
            let (report, elapsed) = measure(&mut evaluator, &node, expression);
            // count 검산: 영향 범위 = spec에서 독립 계산한 역방향 도달 집합.
            assert_eq!(report.changed_inputs, 1, "{token} {kind:?}");
            assert_eq!(report.affected, reachable, "{token} {kind:?} affected");
            assert_eq!(
                report.visited, reachable,
                "{token} {kind:?} visited(토큰 변경은 producer만 방문)"
            );
            assert_eq!(report.evaluated, report.order.len());
            match kind {
                // 모든 값이 RAISE보다 작으므로 영향 범위 전체가 바뀌고, 되돌리면 전체가 다시 바뀝니다.
                ChangeKind::Raise | ChangeKind::Revert => {
                    assert_eq!(report.evaluated, reachable, "{token} {kind:?} evaluated")
                }
                // 값이 같으면 바꾼 토큰만 평가하고 위로 전파하지 않습니다.
                ChangeKind::SameValue => {
                    assert_eq!(report.evaluated, 1, "{token} same_value cutoff")
                }
            }
            if let Some(expression) = check {
                let expected = full_evaluation(&registry, &bindings, &sources, token, &expression);
                assert_eq!(
                    snapshot(&evaluator),
                    expected,
                    "{token}: 증분 = 전체 재평가"
                );
            }
            samples.push(Sample {
                token,
                selection,
                kind,
                changed_inputs: report.changed_inputs,
                affected: report.affected,
                visited: report.visited,
                evaluated: report.evaluated,
                duration_ns: duration_ns(elapsed),
            });
        }
        // revert 후 same_value만 남았으므로 모든 값은 초기값과 같아야 합니다.
        assert!(snapshot(&evaluator) == initial, "{token}: 초기 값 복원");
    }
    BenchResult {
        preset,
        config,
        node_count: synthetic.node_count(),
        edge_count: synthetic.edge_count(),
        digest: synthetic.digest(),
        generate: generate_time,
        materialize: materialize_time,
        build: build_time,
        samples,
    }
}

/// 정렬된 값의 nearest-rank 백분위수.
fn percentile(sorted: &[u64], p: u64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (p * sorted.len() as u64).div_ceil(100).max(1) as usize;
    sorted[rank.min(sorted.len()) - 1]
}

fn count_stats(out: &mut String, name: &str, values: impl Iterator<Item = usize> + Clone) {
    let min = values.clone().min().unwrap_or(0);
    let max = values.clone().max().unwrap_or(0);
    let total: usize = values.sum();
    let _ = writeln!(
        out,
        "    \"{name}\": {{ \"min\": {min}, \"max\": {max}, \"total\": {total} }},"
    );
}

/// 결과 JSON(키 순서 고정, ASCII, 2칸 들여쓰기, LF).
fn to_json(result: &BenchResult) -> String {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let c = &result.config;
    let mut out = String::new();
    out.push_str("{\n");
    let _ = writeln!(out, "  \"format\": \"{FORMAT}\",");
    let _ = writeln!(out, "  \"version\": {FORMAT_VERSION},");
    out.push_str("  \"task\": \"VIII.9.2\",\n");
    let _ = writeln!(out, "  \"profile\": \"{profile}\",");
    let _ = writeln!(out, "  \"preset\": \"{}\",", result.preset);
    let _ = writeln!(
        out,
        "  \"config\": {{ \"seed\": {}, \"tokens\": {}, \"bindings\": {}, \"edges\": {}, \"max_token_refs\": {}, \"environment_one_in\": {} }},",
        c.seed, c.tokens, c.bindings, c.edges, c.max_token_refs, c.environment_one_in
    );
    let _ = writeln!(
        out,
        "  \"graph\": {{ \"nodes\": {}, \"edges\": {}, \"digest\": \"0x{:016x}\" }},",
        result.node_count, result.edge_count, result.digest
    );
    let _ = writeln!(
        out,
        "  \"setup_ns\": {{ \"generate\": {}, \"materialize\": {}, \"build\": {} }},",
        duration_ns(result.generate),
        duration_ns(result.materialize),
        duration_ns(result.build)
    );
    let _ = writeln!(
        out,
        "  \"reference_budget_ns\": {{ \"target\": {BUDGET_TARGET_NS}, \"limit\": {BUDGET_LIMIT_NS}, \"source\": \"architecture 6.2 dependency/token/binding re-evaluation\" }},"
    );
    out.push_str("  \"samples\": [\n");
    for (n, s) in result.samples.iter().enumerate() {
        let comma = if n + 1 == result.samples.len() {
            ""
        } else {
            ","
        };
        let _ = writeln!(
            out,
            "    {{ \"token\": {}, \"selection\": \"{}\", \"kind\": \"{}\", \"changed_inputs\": {}, \"affected\": {}, \"visited\": {}, \"evaluated\": {}, \"duration_ns\": {} }}{comma}",
            s.token,
            s.selection.name(),
            s.kind.name(),
            s.changed_inputs,
            s.affected,
            s.visited,
            s.evaluated,
            s.duration_ns
        );
    }
    out.push_str("  ],\n");
    out.push_str("  \"summary\": {\n");
    let samples = &result.samples;
    let _ = writeln!(out, "    \"changes\": {},", samples.len());
    count_stats(&mut out, "affected", samples.iter().map(|s| s.affected));
    count_stats(&mut out, "visited", samples.iter().map(|s| s.visited));
    count_stats(&mut out, "evaluated", samples.iter().map(|s| s.evaluated));
    let mut durations: Vec<u64> = samples.iter().map(|s| s.duration_ns).collect();
    durations.sort_unstable();
    let mean = durations
        .iter()
        .sum::<u64>()
        .checked_div(durations.len() as u64)
        .unwrap_or(0);
    let _ = writeln!(
        out,
        "    \"duration_ns\": {{ \"min\": {}, \"p50\": {}, \"p95\": {}, \"p99\": {}, \"max\": {}, \"mean\": {mean} }},",
        durations.first().copied().unwrap_or(0),
        percentile(&durations, 50),
        percentile(&durations, 95),
        percentile(&durations, 99),
        durations.last().copied().unwrap_or(0),
    );
    let within = |limit: u64| durations.iter().filter(|d| **d <= limit).count();
    let _ = writeln!(
        out,
        "    \"within_budget\": {{ \"target\": {}, \"limit\": {} }}",
        within(BUDGET_TARGET_NS),
        within(BUDGET_LIMIT_NS)
    );
    out.push_str("  }\n");
    out.push_str("}\n");
    out
}

fn preset_from_env() -> (&'static str, SyntheticConfig, usize, bool) {
    let preset = std::env::var("STELLAR_TOKEN_BENCH_PRESET").unwrap_or_default();
    let (name, config, default_samples, verify_full) = match preset.as_str() {
        "" | "small" => ("small", SMALL, 8, true),
        "edges_1m" => ("edges_1m", SyntheticConfig::EDGES_1M, 32, false),
        "active_200k" => (
            "active_200k",
            SyntheticConfig::ACTIVE_DOCUMENT_200K,
            32,
            false,
        ),
        other => panic!("알 수 없는 STELLAR_TOKEN_BENCH_PRESET: {other}"),
    };
    let samples = std::env::var("STELLAR_TOKEN_BENCH_SAMPLES")
        .ok()
        .map(|raw| {
            raw.parse::<usize>()
                .expect("STELLAR_TOKEN_BENCH_SAMPLES는 정수")
        })
        .unwrap_or(default_samples);
    (name, config, samples, verify_full)
}

fn output_path(preset: &str) -> PathBuf {
    match std::env::var_os("STELLAR_TOKEN_BENCH_OUTPUT") {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("token-change-benchmark")
            .join(format!("{preset}.json")),
    }
}

/// JSON 줄에서 `"key": <정수>` 값을 읽습니다.
fn field(line: &str, key: &str) -> Option<u64> {
    let marker = format!("\"{key}\": ");
    let start = line.find(&marker)? + marker.len();
    let digits: String = line[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

#[test]
fn single_token_change_benchmark_writes_counts_and_durations() {
    let (preset, config, random, verify_full) = preset_from_env();
    let result = run(preset, config, random, verify_full);
    let json = to_json(&result);
    let path = output_path(preset);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("결과 디렉터리");
    }
    std::fs::write(&path, &json).expect("결과 JSON 쓰기");
    println!("token change benchmark: {}", path.display());
    print!("{json}");

    // 결과 JSON: 모든 sample 줄과 summary에 affected/visited/evaluated count가 있고 sample 값과 같습니다.
    assert!(json.is_ascii());
    let sample_lines: Vec<&str> = json
        .lines()
        .filter(|line| line.contains("\"selection\""))
        .collect();
    assert_eq!(sample_lines.len(), result.samples.len());
    assert_eq!(result.samples.len() % 3, 0);
    assert!(result.samples.len() >= 3 * 2, "hub·sparse 포함");
    for (line, sample) in sample_lines.iter().zip(&result.samples) {
        for (key, expected) in [
            ("affected", sample.affected),
            ("visited", sample.visited),
            ("evaluated", sample.evaluated),
        ] {
            assert_eq!(field(line, key), Some(expected as u64), "{key}: {line}");
        }
        assert_eq!(field(line, "duration_ns"), Some(sample.duration_ns));
    }
    for key in [
        "\"affected\": { \"min\"",
        "\"visited\": { \"min\"",
        "\"evaluated\": { \"min\"",
        "\"duration_ns\": { \"min\"",
        "\"p99\"",
    ] {
        assert!(json.contains(key), "summary {key}");
    }
    // hub의 영향 범위는 sparse 이상이며 1보다 큽니다. same_value는 영향 범위와 무관하게 평가 1회입니다.
    let hub = &result.samples[0];
    assert_eq!(hub.selection, Selection::Hub);
    let sparse = &result.samples[3];
    assert_eq!(sparse.selection, Selection::Sparse);
    assert!(hub.affected > 1 && hub.affected >= sparse.affected);
    assert!(result
        .samples
        .iter()
        .filter(|s| s.kind == ChangeKind::SameValue)
        .all(|s| s.evaluated == 1));
}

#[test]
fn counts_are_deterministic_across_runs() {
    let a = run("small", SMALL, 6, false);
    let b = run("small", SMALL, 6, false);
    let counts = |r: &BenchResult| r.samples.iter().map(Sample::counts).collect::<Vec<_>>();
    assert_eq!(counts(&a), counts(&b));
    assert_eq!(a.digest, b.digest);
    // 다른 그래프 seed는 다른 count를 만듭니다(선택 seed는 같음).
    let other = run(
        "small",
        SyntheticConfig {
            seed: SMALL.seed ^ 1,
            ..SMALL
        },
        6,
        false,
    );
    assert_ne!(counts(&a), counts(&other));
}

#[test]
fn percentile_uses_nearest_rank() {
    let sorted: Vec<u64> = (1..=100).collect();
    assert_eq!(percentile(&sorted, 50), 50);
    assert_eq!(percentile(&sorted, 95), 95);
    assert_eq!(percentile(&sorted, 99), 99);
    assert_eq!(percentile(&[7], 99), 7);
    assert_eq!(percentile(&[], 50), 0);
    assert_eq!(percentile(&[1, 2, 3], 50), 2);
}
