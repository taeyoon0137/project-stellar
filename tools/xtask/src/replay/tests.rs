//! core replay 테스트 (XI.2.6, XII.3.7, XVI.2.1, XVI.2.2, XVI.2.5, XVI.2.6, XXIV.3.3).

use super::*;
use std::path::PathBuf;

use stellar_diagnostics::trace_schema::{RENDER_GRAPH_COMPILE, SCENE_BUILD};
use stellar_renderer_core::graph_trace::RenderGraphCompileTrace;
use stellar_scene::incremental::SceneBuildTrace;

/// 테스트용 판정 sample 수(CLI 기본은 1,000).
const SAMPLES: u64 = 12;

/// core replay가 실행할 수 있는 시나리오.
fn supported() -> Vec<ReplayScenario> {
    ReplayScenario::ALL
        .into_iter()
        .filter(|s| s.core_replay() == CoreReplay::Supported)
        .collect()
}

fn replay(
    scenario: ReplayScenario,
    fixture: &stellar_bench_fixtures::limits::VisibleFixture,
) -> ReplayFile {
    standard_replay(scenario, SMALL, fixture, SAMPLES, CacheState::Warm).unwrap()
}

/// 테스트용 작은 fixture(가시 node 2,000).
const SMALL: FixtureSpec = FixtureSpec {
    name: FixtureName::VisibleNodes,
    seed: 3,
    size: 2_000,
};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repo root")
        .to_path_buf()
}

fn environment_fixed() -> ResultEnvironment {
    environment(
        &EnvironmentArgs {
            commit: Some("0123456789abcdef0123456789abcdef01234567".to_owned()),
            model: Some("Mac14,2".to_owned()),
            cpu: Some("Apple M1".to_owned()),
            memory_gib: Some(16),
            os_build: Some("25C56".to_owned()),
            app_build: Some("0.0.0".to_owned()),
        },
        &repo_root(),
    )
    .unwrap()
}

#[test]
fn trace_schemas_match_the_emitting_crates() {
    // 검증 기준(XI.2.6): scene·render graph trace field가 diagnostics schema와 같습니다.
    assert_eq!(SceneBuildTrace::EVENT, SCENE_BUILD.event);
    assert_eq!(SceneBuildTrace::FIELDS.as_slice(), SCENE_BUILD.fields);
    assert_eq!(RenderGraphCompileTrace::EVENT, RENDER_GRAPH_COMPILE.event);
    let names: Vec<&str> = RenderGraphCompileTrace {
        scene_revision: 0,
        start_nanos: 0,
        end_nanos: 0,
        cpu_nanos: 0,
        pass_count: 0,
        draw_count: 0,
        batch_count: 0,
        layer_count: 0,
    }
    .fields()
    .iter()
    .map(|(name, _)| *name)
    .collect();
    assert_eq!(names.as_slice(), RENDER_GRAPH_COMPILE.fields);
}

#[test]
fn replaying_twice_gives_the_same_semantics_and_trace_count() {
    // 검증 기준(XVI.2.2): 동일 replay 두 번 semantic/trace event count 동일.
    let fixture = load_fixture(&SMALL).unwrap();
    for scenario in supported() {
        for cache in CACHE_STATES {
            let file = standard_replay(scenario, SMALL, &fixture, SAMPLES, cache).unwrap();
            let first = run_replay(&file, &fixture).unwrap();
            let second = run_replay(&file, &fixture).unwrap();
            assert_eq!(first.traces.len() + first.setup_events, file.events.len());
            assert_eq!(first.traces.len(), second.traces.len());
            assert_eq!(
                first.scene_hash, second.scene_hash,
                "{scenario:?} {cache:?}"
            );
            assert_eq!(first.document_revision, second.document_revision);
            assert_eq!(first.scene_builds, second.scene_builds);
            assert_eq!(first.graph_compiles, file.events.len());
            // T0은 ManualClock timeline 시각입니다.
            for (trace, event) in first.traces.iter().zip(&file.events[file.measure_from..]) {
                assert_eq!(
                    trace.stamp(stellar_diagnostics::interaction::Stage::T0),
                    event.at_nanos
                );
                assert_eq!(trace.dimensions().kind, scenario.kind());
                assert_eq!(trace.dimensions().cache, cache);
            }
        }
    }
}

#[test]
fn every_supported_scenario_yields_the_requested_samples() {
    // 검증 기준(XVI.2.5): 시나리오마다 sample을 요청한 수 이상 만듭니다(CLI 기본 1,000).
    let fixture = load_fixture(&SMALL).unwrap();
    let scenarios = supported();
    assert_eq!(scenarios.len(), 10);
    for scenario in scenarios {
        let run = run_replay(&replay(scenario, &fixture), &fixture).unwrap();
        assert!(
            run.traces.len() as u64 >= SAMPLES,
            "{scenario:?}: {}",
            run.traces.len()
        );
    }
    // 엔진 경로가 이어지지 않은 시나리오는 재생 파일을 만들지 않습니다.
    for scenario in [
        ReplayScenario::TextEdit,
        ReplayScenario::TokenPropagation,
        ReplayScenario::ComponentUpdate,
    ] {
        let err =
            standard_replay(scenario, SMALL, &fixture, SAMPLES, CacheState::Warm).unwrap_err();
        assert!(
            err.contains("core replay가 아직 실행할 수 없습니다"),
            "{err}"
        );
    }
}

#[test]
fn scenarios_exercise_their_engine_paths() {
    // 검증 기준(XVI.2.5): 시나리오마다 해당 상호작용 경로를 실제로 지납니다.
    let fixture = load_fixture(&SMALL).unwrap();
    let run = |scenario| run_replay(&replay(scenario, &fixture), &fixture).unwrap();
    let pan = run(ReplayScenario::Pan);
    // pan·zoom·hover·select는 문서를 바꾸지 않고 scene도 다시 만들지 않습니다(첫 build만).
    for scenario in [
        ReplayScenario::Pan,
        ReplayScenario::Zoom,
        ReplayScenario::Hover,
        ReplayScenario::Select,
    ] {
        let r = run(scenario);
        assert_eq!(r.document_revision.0, 0, "{scenario:?}");
        assert_eq!(r.scene_builds, 1, "{scenario:?}");
        assert_eq!(r.scene_hash, pan.scene_hash, "{scenario:?}");
    }
    // 제스처는 preview 뒤 한 번 확정합니다.
    for scenario in [
        ReplayScenario::Drag,
        ReplayScenario::Resize,
        ReplayScenario::Rotate,
    ] {
        let r = run(scenario);
        assert_eq!(r.document_revision.0, 1, "{scenario:?}");
        assert!(r.scene_builds > 2, "{scenario:?}: {}", r.scene_builds);
        assert_ne!(r.scene_hash, pan.scene_hash, "{scenario:?}");
    }
    // undo: setup의 편집 SAMPLES개를 모두 되돌려 처음 scene으로 돌아갑니다.
    let undo = run(ReplayScenario::Undo);
    assert_eq!(undo.setup_events as u64, 4 * SAMPLES);
    assert_eq!(undo.document_revision.0, 2 * SAMPLES);
    assert_eq!(undo.scene_hash, pan.scene_hash);
    // redo: 다시 적용하면 setup 편집 뒤와 같은 scene입니다(짝수 번 왕복이라 처음 위치).
    let redo = run(ReplayScenario::Redo);
    assert_eq!(redo.setup_events as u64, 5 * SAMPLES);
    assert_eq!(redo.document_revision.0, 3 * SAMPLES);
    assert_eq!(redo.scene_hash, pan.scene_hash);
    // variant: 짝수 단계는 card를 숨기고 홀수 단계는 되돌립니다. 문서 revision은 그대로입니다.
    let variant = run(ReplayScenario::VariantChange);
    assert_eq!(variant.document_revision.0, 0);
    assert_eq!(variant.scene_builds as u64, 1 + SAMPLES);
    // 마지막 단계(홀수)는 Variant 없음이라 처음 scene입니다.
    assert_eq!(variant.scene_hash, pan.scene_hash);
    let odd = standard_replay(
        ReplayScenario::VariantChange,
        SMALL,
        &fixture,
        SAMPLES + 1,
        CacheState::Warm,
    )
    .unwrap();
    assert_ne!(
        run_replay(&odd, &fixture).unwrap().scene_hash,
        pan.scene_hash
    );
}

#[test]
fn cache_setup_matches_observed_hits_and_misses() {
    // 검증 기준(XVI.2.6): cache setup이 실제 hit/miss count와 일치하고 결과 metadata에 남습니다.
    let fixture = load_fixture(&SMALL).unwrap();
    for scenario in supported() {
        let warm = run_replay(
            &standard_replay(scenario, SMALL, &fixture, SAMPLES, CacheState::Warm).unwrap(),
            &fixture,
        )
        .unwrap();
        let cold = run_replay(
            &standard_replay(scenario, SMALL, &fixture, SAMPLES, CacheState::Cold).unwrap(),
            &fixture,
        )
        .unwrap();
        assert!(
            warm.cache_consistent,
            "{scenario:?} {:?}",
            warm.cache_observation
        );
        assert!(
            cold.cache_consistent,
            "{scenario:?} {:?}",
            cold.cache_observation
        );
        assert_eq!(warm.cache_observation.full_builds, 0, "{scenario:?}");
        let first = cold.cache_observation.first.expect("cold 첫 build");
        assert!(
            first.full && first.hits == 0 && first.misses > 0,
            "{scenario:?}"
        );
        // cache 상태는 결과만 바꾸지 scene 의미는 같습니다.
        assert_eq!(warm.scene_hash, cold.scene_hash, "{scenario:?}");
        if let Some(first) = warm.cache_observation.first {
            assert!(first.hits > 0, "{scenario:?}");
        }
    }
    // 선언과 다른 관측은 불일치로 판정합니다: warm 파일을 cold처럼 측정한 결과를 warm으로 판정하면 실패.
    let file = standard_replay(
        ReplayScenario::Drag,
        SMALL,
        &fixture,
        SAMPLES,
        CacheState::Cold,
    )
    .unwrap();
    let cold = run_replay(&file, &fixture).unwrap();
    assert!(stellar_diagnostics::latency_scenario::check_cache_setup(
        CacheState::Warm,
        &cold.cache_observation
    )
    .is_err());
}

#[test]
fn inputs_a_scenario_does_not_define_are_rejected() {
    // 해석하지 않는 key나 시나리오가 정의하지 않은 commit 단계는 아무 일도 하지 않는 sample이 되므로 거부합니다.
    let fixture = load_fixture(&SMALL).unwrap();
    let mut undo = replay(ReplayScenario::Undo, &fixture);
    let last = undo.events.last().unwrap().at_nanos;
    undo.events.push(super::file::ReplayEvent {
        at_nanos: last,
        input: super::file::ReplayInput::Key {
            key: "a".to_owned(),
            modifiers: super::file::Modifiers::NONE,
        },
    });
    let err = run_replay(&undo, &fixture).unwrap_err();
    assert!(err.contains("해석하지 않는 key"), "{err}");
    let mut pan = replay(ReplayScenario::Pan, &fixture);
    let last = pan.events.last().unwrap().at_nanos;
    pan.events.push(super::file::ReplayEvent {
        at_nanos: last,
        input: super::file::ReplayInput::Commit { step: 0 },
    });
    let err = run_replay(&pan, &fixture).unwrap_err();
    assert!(err.contains("commit 단계를 정의하지 않습니다"), "{err}");
}

#[test]
fn drag_commits_once_and_previews_keep_the_document_revision() {
    // 검증 기준(XII.3.7): drag 중 document revision 증가 없음, pointer up에서만 확정.
    let fixture = load_fixture(&SMALL).unwrap();
    for scenario in [ReplayScenario::Drag, ReplayScenario::Resize] {
        let file = replay(scenario, &fixture);
        let run = run_replay(&file, &fixture).unwrap();
        assert_eq!(run.document_revision.0, 1, "{scenario:?}");
        // 첫 build + threshold를 넘은 이동마다 preview 1회 + 확정 1회.
        assert!(
            run.scene_builds as u64 > SAMPLES / 2,
            "{scenario:?}: {}",
            run.scene_builds
        );
    }
}

#[test]
fn result_schema_is_identical_across_runs() {
    // 검증 기준(XXIV.3.3): result schema identical.
    let fixture = load_fixture(&SMALL).unwrap();
    let budget = budget(&repo_root()).unwrap();
    let identity = FixtureIdentity {
        name: SMALL.name,
        seed: SMALL.seed,
        size: SMALL.size,
        content_hash: *fixture.content_hash.digest(),
    };
    let shape = |json: &str| -> Vec<String> {
        // 숫자·hash 값을 지운 구조. gate 위반 목록의 길이는 측정값에 따라 달라지므로 그 앞까지 비교합니다.
        let json = &json[..json.find("\"violations\"").expect("violations key")];
        json.split('"')
            .enumerate()
            .filter(|(i, _)| i % 2 == 1)
            .map(|(_, token)| token.to_owned())
            .filter(|token| !token.chars().all(|c| c.is_ascii_hexdigit()))
            .collect()
    };
    let mut shapes = Vec::new();
    for _ in 0..2 {
        let runs: Vec<ReplayRun> = [ReplayScenario::Pan, ReplayScenario::Drag]
            .iter()
            .map(|s| run_replay(&replay(*s, &fixture), &fixture).unwrap())
            .collect();
        let result = build_result(environment_fixed(), identity, &runs, &budget);
        // 실행한 2개 + 없는 required 조합(13 × 2 − 2)은 sample 0개로 들어가 gate에서 실패합니다.
        assert_eq!(result.scenarios.len(), 26);
        assert!(result.scenarios[..2]
            .iter()
            .all(|s| s.latency.count == SAMPLES + 2));
        assert!(result.scenarios[2..].iter().all(|s| s.latency.count == 0));
        assert!(!result.passed());
        let no_samples = result
            .violations
            .iter()
            .filter(|v| v.metric == stellar_diagnostics::analyzer::GateMetric::NoSamples)
            .count();
        assert_eq!(no_samples, 24);
        // 실행한 시나리오도 sample이 scenario_min_samples보다 적어 실패합니다.
        assert!(result.violations.iter().any(|v| {
            v.metric == stellar_diagnostics::analyzer::GateMetric::TooFewSamples && v.scenario < 2
        }));
        shapes.push(shape(&result.to_json()));
    }
    assert_eq!(shapes[0], shapes[1]);
}

#[test]
fn cli_records_and_runs_replay_files() {
    // lane 검증: `cargo xtask replay` CLI.
    let dir = std::env::temp_dir().join(format!("stellar-replay-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let replay = dir.join("drag.replay");
    let suite = dir.join("suite");
    let result = dir.join("result.json");
    let args = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    run_cli(
        &args(&[
            "record",
            "drag",
            "--out",
            replay.to_str().unwrap(),
            "--seed",
            "3",
            "--size",
            "2000",
            "--samples",
            "5",
            "--cache",
            "cold",
        ]),
        &repo_root(),
    )
    .unwrap();
    let text = std::fs::read_to_string(&replay).unwrap();
    assert!(text.starts_with("stellar-replay v2\nfixture visible-nodes 3 2000\n"));
    assert!(text.contains("\ncache cold\nmeasure-from 0\n"));
    // 엔진 경로가 이어지지 않은 시나리오는 사용법 오류입니다.
    let refused = run_cli(
        &args(&["record", "text-edit", "--out", replay.to_str().unwrap()]),
        &repo_root(),
    )
    .unwrap_err();
    assert_eq!(refused.0, 64, "{}", refused.1);
    run_cli(
        &args(&[
            "record-suite",
            "--dir",
            suite.to_str().unwrap(),
            "--seed",
            "3",
            "--size",
            "2000",
            "--samples",
            "3",
        ]),
        &repo_root(),
    )
    .unwrap();
    let mut names: Vec<String> = std::fs::read_dir(&suite)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names.len(), 20);
    assert!(names.contains(&"undo.cold.replay".to_owned()));
    assert!(!names.iter().any(|n| n.starts_with("text-edit")));
    run_cli(
        &args(&[
            "run",
            replay.to_str().unwrap(),
            "--out",
            result.to_str().unwrap(),
            "--commit",
            "0123456789abcdef0123456789abcdef01234567",
            "--hardware-model",
            "Mac14,2",
            "--hardware-cpu",
            "Apple M1",
            "--memory-gib",
            "16",
            "--os-build",
            "25C56",
            "--app-build",
            "0.0.0",
        ]),
        &repo_root(),
    )
    .unwrap();
    let json = std::fs::read_to_string(&result).unwrap();
    assert!(json.contains("\"schema\": \"stellar.performance-result.v2\""));
    assert!(json.contains("\"kind\": \"drag\""));
    assert!(json.contains("\"cache\": {\"state\": \"cold\""));
    assert!(!json.contains(dir.to_str().unwrap()));
    // 잘못된 인자와 다른 fixture hash는 거부합니다.
    assert_eq!(run_cli(&args(&["spin"]), &repo_root()).unwrap_err().0, 64);
    let tampered = text.replace(
        "fixture visible-nodes 3 2000",
        "fixture visible-nodes 4 2000",
    );
    std::fs::write(&replay, tampered).unwrap();
    let err = run_cli(&args(&["run", replay.to_str().unwrap()]), &repo_root()).unwrap_err();
    assert_eq!(err.0, 70, "{}", err.1);
    let _ = std::fs::remove_dir_all(&dir);
}
