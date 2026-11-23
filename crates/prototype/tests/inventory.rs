//! 평가 경로 API inventory (XXX.3.1, XXX.3.2, XXX.6.3 일부).
//!
//! - 프로토타입 엔진과 문서 쪽 프로토타입·MotionSpec module에 시스템 시각·파일·네트워크·프로세스·환경 변수 접근이 없습니다.
//! - timing evaluator·제스처·tick 처리 경로(시험 module 제외)에 libm 초월함수 호출이 없습니다.
//! - 위반 fixture(금지 호출을 넣은 텍스트)는 검사가 잡아냅니다.

const ENGINE: &[(&str, &str)] = &[
    ("lib.rs", include_str!("../src/lib.rs")),
    ("graph.rs", include_str!("../src/graph.rs")),
    ("auto_layout.rs", include_str!("../src/auto_layout.rs")),
    ("edit.rs", include_str!("../src/edit.rs")),
    ("plan.rs", include_str!("../src/plan.rs")),
    ("expose.rs", include_str!("../src/expose.rs")),
    ("diagnostics.rs", include_str!("../src/diagnostics.rs")),
    ("clock.rs", include_str!("../src/clock.rs")),
    ("timing.rs", include_str!("../src/timing.rs")),
    ("gesture.rs", include_str!("../src/gesture.rs")),
    ("session.rs", include_str!("../src/session.rs")),
    ("context.rs", include_str!("../src/context.rs")),
    (
        "document motion/spec.rs",
        include_str!("../../document/src/motion/spec.rs"),
    ),
    (
        "document motion/set.rs",
        include_str!("../../document/src/motion/set.rs"),
    ),
    (
        "document motion/world.rs",
        include_str!("../../document/src/motion/world.rs"),
    ),
    (
        "document motion/command.rs",
        include_str!("../../document/src/motion/command.rs"),
    ),
    (
        "document prototype/socket.rs",
        include_str!("../../document/src/prototype/socket.rs"),
    ),
    (
        "document prototype/node_kind.rs",
        include_str!("../../document/src/prototype/node_kind.rs"),
    ),
    (
        "document prototype/layout_hint.rs",
        include_str!("../../document/src/prototype/layout_hint.rs"),
    ),
    (
        "document prototype/source.rs",
        include_str!("../../document/src/prototype/source.rs"),
    ),
];

/// 평가 경로(값이 tick의 순수 함수여야 하는 곳).
const EVALUATION: &[&str] = &["clock.rs", "timing.rs", "gesture.rs", "session.rs"];

const SIDE_CHANNELS: &[&str] = &[
    "std::time",
    "SystemTime",
    "Instant::now",
    "UNIX_EPOCH",
    "std::fs",
    "std::net",
    "std::process",
    "std::env",
    "std::thread::sleep",
];

const TRANSCENDENTAL: &[&str] = &[
    ".exp(",
    ".exp2(",
    ".exp_m1(",
    ".ln(",
    ".log(",
    ".log2(",
    ".log10(",
    ".ln_1p(",
    ".powf(",
    ".powi(",
    ".sin(",
    ".cos(",
    ".tan(",
    ".asin(",
    ".acos(",
    ".atan(",
    ".atan2(",
    ".sinh(",
    ".cosh(",
    ".tanh(",
    ".cbrt(",
    ".hypot(",
    ".sin_cos(",
    ".mul_add(",
];

/// 시험 module(`#[cfg(test)]` 뒤)을 뺀 본문.
fn production(source: &str) -> &str {
    source.split("#[cfg(test)]").next().unwrap_or(source)
}

fn violations<'a>(source: &str, patterns: &[&'a str]) -> Vec<&'a str> {
    let body = production(source);
    patterns
        .iter()
        .copied()
        .filter(|p| {
            // 초월함수는 method 호출(`x.exp(`)과 경로 호출(`f64::exp(`) 둘 다 셉니다.
            let path_form = p.strip_prefix('.').map(|name| format!("::{name}"));
            body.lines().any(|line| {
                !line.trim_start().starts_with("//")
                    && (line.contains(p) || path_form.as_deref().is_some_and(|f| line.contains(f)))
            })
        })
        .collect()
}

#[test]
fn inventory_lists_every_engine_source() {
    // 새 source file이 inventory에서 빠지지 않도록, 검사 대상 디렉터리의 `.rs` 파일이 모두 목록에 있는지 봅니다.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let dirs = [
        (root.join("src"), ""),
        (root.join("../document/src/motion"), "document motion/"),
        (
            root.join("../document/src/prototype"),
            "document prototype/",
        ),
    ];
    let listed: Vec<&str> = ENGINE.iter().map(|(name, _)| *name).collect();
    for (dir, prefix) in dirs {
        for entry in std::fs::read_dir(&dir).expect("디렉터리") {
            let path = entry.expect("항목").path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let file = path.file_name().and_then(|f| f.to_str()).expect("이름");
            if prefix.is_empty() || file != "mod.rs" {
                let name = format!("{prefix}{file}");
                assert!(listed.contains(&name.as_str()), "inventory에 없음: {name}");
            }
        }
    }
}

#[test]
fn engine_has_no_system_time_file_or_network_access() {
    // 검증 기준(XXX.3.1): 평가 경로 API inventory에 시스템 시각 호출 0. (XXX.6.3: 실행 코드·파일·네트워크·시스템 시각 0)
    for (name, source) in ENGINE {
        assert_eq!(
            violations(source, SIDE_CHANNELS),
            Vec::<&str>::new(),
            "{name}"
        );
    }
}

#[test]
fn evaluation_path_has_no_transcendental_calls() {
    // 검증 기준(XXX.3.2): evaluator 경로의 libm 초월함수 호출 inventory 0.
    for (name, source) in ENGINE.iter().filter(|(n, _)| EVALUATION.contains(n)) {
        assert_eq!(
            violations(source, TRANSCENDENTAL),
            Vec::<&str>::new(),
            "{name}"
        );
    }
}

#[test]
fn violation_fixtures_are_caught() {
    // 검증 기준(XXX.6.3): 위반 fixture 거부.
    let clock =
        "fn now() -> u64 { std::time::SystemTime::now().elapsed().unwrap().as_millis() as u64 }";
    assert!(!violations(clock, SIDE_CHANNELS).is_empty());
    let spring = "fn f(t: f64) -> f64 { (-t).exp() * t.cos() }";
    assert_eq!(violations(spring, TRANSCENDENTAL), vec![".exp(", ".cos("]);
    let path_call = "fn f(t: f64) -> f64 { f64::sin(t) }";
    assert_eq!(violations(path_call, TRANSCENDENTAL), vec![".sin("]);
    let file = "use std::fs::read; fn x() {}";
    assert!(!violations(file, SIDE_CHANNELS).is_empty());
    // 주석과 시험 module은 세지 않습니다.
    let commented = "// std::time은 쓰지 않습니다\nfn ok() {}\n#[cfg(test)]\nmod tests { fn t() { let _ = 1.0f64.exp(); } }";
    assert!(violations(commented, SIDE_CHANNELS).is_empty());
    assert!(violations(commented, TRANSCENDENTAL).is_empty());
}
