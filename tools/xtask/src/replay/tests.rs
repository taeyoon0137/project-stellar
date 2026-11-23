//! core replay 테스트 (XI.2.6, XII.3.7, XVI.2.1, XVI.2.2, XXIV.3.3).

use super::*;
use std::path::PathBuf;

use stellar_diagnostics::trace_schema::{RENDER_GRAPH_COMPILE, SCENE_BUILD};
use stellar_renderer_core::graph_trace::RenderGraphCompileTrace;
use stellar_scene::incremental::SceneBuildTrace;

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
    for scenario in ReplayScenario::ALL {
        let file = standard_replay(scenario, SMALL, &fixture).unwrap();
        let first = run_replay(&file, &fixture).unwrap();
        let second = run_replay(&file, &fixture).unwrap();
        assert_eq!(first.traces.len(), file.events.len());
        assert_eq!(first.traces.len(), second.traces.len());
        assert_eq!(first.scene_hash, second.scene_hash, "{scenario:?}");
        assert_eq!(first.document_revision, second.document_revision);
        assert_eq!(first.scene_builds, second.scene_builds);
        assert_eq!(first.graph_compiles, file.events.len());
        // T0은 ManualClock timeline 시각입니다.
        for (trace, event) in first.traces.iter().zip(&file.events) {
            assert_eq!(
                trace.stamp(stellar_diagnostics::interaction::Stage::T0),
                event.at_nanos
            );
        }
    }
}

#[test]
fn drag_commits_once_and_previews_keep_the_document_revision() {
    // 검증 기준(XII.3.7): drag 중 document revision 증가 없음, pointer up에서만 확정.
    let fixture = load_fixture(&SMALL).unwrap();
    let pan = run_replay(
        &standard_replay(ReplayScenario::Pan, SMALL, &fixture).unwrap(),
        &fixture,
    )
    .unwrap();
    assert_eq!(pan.document_revision.0, 0);
    // pan은 scene을 다시 만들지 않습니다(첫 build만).
    assert_eq!(pan.scene_builds, 1);
    for scenario in [ReplayScenario::Drag, ReplayScenario::Resize] {
        let file = standard_replay(scenario, SMALL, &fixture).unwrap();
        let run = run_replay(&file, &fixture).unwrap();
        assert_eq!(run.document_revision.0, 1, "{scenario:?}");
        // 첫 build + threshold를 넘은 이동마다 preview 1회 + 확정 1회.
        assert!(run.scene_builds > 50, "{scenario:?}: {}", run.scene_builds);
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
        let runs: Vec<ReplayRun> = ReplayScenario::ALL
            .iter()
            .map(|s| run_replay(&standard_replay(*s, SMALL, &fixture).unwrap(), &fixture).unwrap())
            .collect();
        let result = build_result(environment_fixed(), identity, &runs, &budget);
        assert_eq!(result.scenarios.len(), 3);
        assert!(result.scenarios.iter().all(|s| s.latency.count == 62));
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
        ]),
        &repo_root(),
    )
    .unwrap();
    let text = std::fs::read_to_string(&replay).unwrap();
    assert!(text.starts_with("stellar-replay v1\nfixture visible-nodes 3 2000\n"));
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
    assert!(json.contains("\"schema\": \"stellar.performance-result.v1\""));
    assert!(json.contains("\"kind\": \"drag\""));
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
