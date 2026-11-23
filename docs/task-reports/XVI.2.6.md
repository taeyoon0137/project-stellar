# 작업 보고서: XVI.2.6 warm·cold cache setup

- WBS: XVI.2.6
- 작업명: warm cache와 cold cache setup을 분리하고 결과 metadata에 명시
- 상태: 부분 완료 (`[~]`) — core replay의 cache(증분 scene·공간 인덱스·layout)만 했습니다. GPU cache(glyph atlas XI.6.5,
  texture cache XI.7.3)는 macOS full replay(XVI.2.3)가 같은 형식으로 더합니다. 선행 XVI.2.5도 `[~]`입니다.
- 결정: [ADR-0096](../adr/ADR-0096-interaction-latency-probe-and-scenario-gate.md) 결정 4·5
- 선행: XI.6.5(`[x]`), XI.7.3(`[x]`), XVI.2.5(`[~]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 측정 장비는 개발 장비(Apple M3 Max, 36GB)이고 기준 장비(M1 16GB)가 아닙니다.

## 변경 파일 목록

- `crates/diagnostics/src/latency_scenario.rs` — `CacheBuild`, `CacheObservation`, `check_cache_setup`.
- `crates/diagnostics/src/result.rs` — 결과 schema v2의 scenario별 `cache`·`setup_events`.
- `tools/xtask/src/replay/file.rs` — 재생 파일 `cache warm|cold` 줄.
- `tools/xtask/src/replay/runner.rs` — cold 측정 시작 때 scene·공간 인덱스·layout을 처음부터 만들고, 측정 구간 scene
  build의 hit·miss를 모음.
- `docs/task-reports/XVI.2.6.md`(신규).

## 구현 요약

- setup은 재생 파일에 적습니다(`cache warm|cold`). `record-suite`는 시나리오마다 warm·cold 파일을 따로 씁니다.
- hit은 이전 결과를 그대로 쓴 scene node(`SceneBuildStats::shared`), miss는 다시 평가한 node(`rebuilt`)입니다.
- `warm`: 문서를 연 뒤의 결과와 setup event가 남긴 결과를 그대로 둡니다. `cold`: 측정 첫 event에서 결과를 버리고 처음부터
  만듭니다.
- 판정(`check_cache_setup`): cold는 측정 시작 build가 처음부터 만든 build이고 hit 0·miss 1 이상, warm은 측정 구간에
  처음부터 만든 build가 없고 측정 시작 build가 있으면 hit 1 이상입니다.
- 결과 JSON v2의 scenario마다 `"cache": {"state", "hits", "misses", "full_builds", "first_hits", "first_misses",
  "first_full", "consistent"}`와 `"setup_events"`를 남깁니다. 불일치는 release gate `cache_setup` 위반입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-diagnostics` | 통과(XVI.2.5 보고서의 최종 검증 절) |
| `cargo test -p xtask --lib replay` | 11 passed, 0 failed |
| `cargo run --release -p xtask -- replay run <scratch>/suite/*.replay --gate` | 20개 결과 모두 `consistent: true` |

## 테스트

- `cache_setup_must_match_observed_hits_and_misses`(diagnostics): cold·warm 일치·불일치 경우 9개.
- `cache_setup_matches_observed_hits_and_misses`(xtask): 시나리오 10개 × warm·cold가 모두 일치, cold 첫 build는 full·hit 0,
  warm은 full build 0, 두 setup의 마지막 scene semantic hash 같음. cold 관측을 warm으로 판정하면 불일치.

## 성능

release 측정(XVI.2.5 보고서와 같은 실행, 부하 상태)의 cache 관측입니다.

| 시나리오 | warm 측정 첫 build (hit, miss) | cold 측정 첫 build (hit, miss, full) |
|---|---|---|
| drag·rotate·undo·redo | (19,496, 4) | (0, 19,501, full) |
| resize | (19,499, 1) | (0, 19,501, full) |
| variant-change | (16,978, 2,522) | (0, 19,501, full) |
| pan·zoom·hover·select | build 없음 | (0, 19,501, full) |

- cold의 첫 event는 문서 전체 scene build(약 19,500 node)를 포함해 max가 105\~487ms로 통제 가능 max 10ms를 넘습니다.
  부하가 섞인 값이지만 전체 build 비용 자체가 단일 interaction 예산을 넘는다는 뜻입니다.

## 차이(작업 지시와 다른 점)

- GPU cache를 core replay에서 흉내 내지 않았습니다. synthetic fixture image에는 asset 내용이 없어 texture cache의 content
  hash key를 만들 수 없기 때문입니다(ADR-0096 Alternatives).

## 남은 위험 / 한계

- glyph atlas·texture cache의 setup과 hit·miss는 macOS full replay(XVI.2.3)에서 더해야 합니다.
- cold 첫 interaction 비용을 줄이는 일(문서 열기 때 scene을 미리 만들어 두는 등)은 이 행 밖입니다.

## 근거 커밋

- `c6caa47c` Task(XVI.2.5): 성능 게이트 시나리오 목록과 core replay 시나리오 10개 재생(cache setup 판정·결과 schema v2 포함)
- `4d48e5d6` Task(XVI.2.6): 이 보고서.
