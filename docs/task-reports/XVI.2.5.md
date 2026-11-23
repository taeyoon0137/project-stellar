# 작업 보고서: XVI.2.5 성능 게이트 시나리오 replay

- WBS: XVI.2.5
- 작업명: Pan, Zoom, Drag, Resize, Rotate, Hover, Select, Text Edit, Token Propagation, Component Update, Variant Change,
  Undo, Redo replay를 각각 만들기
- 상태: 부분 완료 (`[~]`) — core replay 엔진 부분만 했습니다. 선행 XVI.2.3(macOS full replay)이 없고, 13개 중 3개는 엔진
  경로가 문서 → scene에 이어지지 않아 재생할 수 없습니다.
- 결정: [ADR-0096](../adr/ADR-0096-interaction-latency-probe-and-scenario-gate.md) 결정 3\~6
- 선행: XVI.2.3(`[ ]`, macOS full replay)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 측정 장비는 개발 장비(Apple M3 Max, 36GB)이고 기준 장비(M1 16GB)가 아닙니다.

## 변경 파일 목록

- `crates/diagnostics/src/latency_scenario.rs`(신규) — §38.1 시나리오 13개 목록, core replay 실행 가능 여부, cache setup 판정.
- `crates/diagnostics/src/lib.rs` — 모듈 등록.
- `crates/diagnostics/src/analyzer.rs` — gate의 sample 수·cache setup 위반.
- `crates/diagnostics/src/result.rs` — 결과 schema v2(cache metadata, setup event 수).
- `tools/xtask/src/replay/file.rs` — 재생 파일 v2(XVI.2.1).
- `tools/xtask/src/replay/runner.rs` — 시나리오 10개 실행, history 기록·재생, View Variant 교체, cold cache.
- `tools/xtask/src/replay.rs` — `record --samples --cache`, `record-suite`, 빠진 required 조합 gate 실패.
- `tools/xtask/src/replay/tests.rs` — 시나리오·cache 테스트.
- `tools/xtask/Cargo.toml`, `Cargo.lock` — `stellar-history` 의존 등록.
- `docs/task-reports/XVI.2.5.md`(신규).

## 구현 요약

- 시나리오 목록은 diagnostics `LatencyScenario` 하나입니다. 재생 파일 `scenario` 값과 xtask `ReplayScenario`(별칭)가 같은
  목록을 씁니다. 모두 required이고 trace 종류(`InteractionKind`)와 1:1입니다.
- `standard_replay(scenario, fixture, samples, cache)`가 판정 sample `samples`개(CLI 기본 1,000) 이상인 재생 파일을
  만듭니다. 제스처는 삼각파 경로라 화면 밖으로 나가지 않습니다.

  | 시나리오 | core replay 입력 | 지나는 엔진 경로 |
  |---|---|---|
  | pan | Hand 누르고 이동 | 상태 기계, camera, render graph |
  | zoom | Zoom 도구 scrub | 상태 기계, camera, render graph |
  | drag | 선택한 card 이동 | hit test, preview, 확정 transaction(history 기록), layout, scene commit |
  | resize | 오른쪽 아래 handle | 위와 같음(크기 command 포함) |
  | rotate | 오른쪽 위 회전 영역, 중심 둘레 이동 | 위와 같음(회전 transform) |
  | hover | 누르지 않은 이동 | 이동마다 canvas hit test |
  | select | 화면 안 card 차례로 클릭 | hit test, Click 결과, 선택 상자 handle |
  | variant-change | `commit n` | View Variant 교체 → `scene_edits` variant option, Subtree 무효화 |
  | undo | setup: card를 `samples`번 끌기, 측정: `cmd+z` | `History::undo`, change set → layout → scene |
  | redo | setup: 위 + 모두 undo, 측정: `cmd+shift+z` | `History::redo` |

- Text Edit·Token Propagation·Component Update는 `CoreReplay::NotConnected`입니다. 재생 파일을 만들지 않고(`record`는 사용법
  오류, `record-suite`는 이유를 적고 건너뜀) gate에서 sample 0개로 실패합니다. 빠진 연결은 ADR-0096 결정 3에 있습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-diagnostics` | 통과(아래 최종 검증 절) |
| `cargo test -p xtask --lib replay` | 11 passed, 0 failed |
| `cargo run --release -p xtask -- replay record-suite --dir <scratch>/suite` | 재생 파일 20개(시나리오 10 × warm·cold), 각 판정 sample 1,000 이상 |
| `cargo run --release -p xtask -- replay run <scratch>/suite/*.replay --gate --out result.json` | 결과 JSON 기록, gate 실패(종료 65, 위반 58개) |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `every_supported_scenario_yields_the_requested_samples`: 시나리오 10개 모두 요청한 sample 수 이상, 3개는 재생 파일 거부.
- `scenarios_exercise_their_engine_paths`: pan·zoom·hover·select는 문서·scene 불변, drag·resize·rotate는 확정 1회, undo는
  setup 편집을 모두 되돌려 처음 scene, redo는 다시 적용, variant는 짝수 단계에서 scene이 바뀌고 홀수 단계에서 돌아옴.
- `replaying_twice_gives_the_same_semantics_and_trace_count`: 시나리오 10개 × warm·cold 두 번 실행 결과 같음.
- `catalog_lists_the_thirteen_gate_scenarios`(diagnostics): 13개, 이름·종류 서로 다름, 모두 required.

## 성능

2만 가시 node fixture(seed 1), release, 개발 장비, 다른 작업 14개 동시 실행 중 1회 측정입니다. 부하 때문에 같은
시나리오의 warm이 cold보다 나쁘게 나온 경우가 있어 판정용 값이 아닙니다(ms, 판정 sample 1,000\~1,002개).

| 시나리오 | cache | p50 | p95 | p99 | max |
|---|---|---|---|---|---|
| pan | warm | 3.88 | 6.00 | 49.77 | 80.58 |
| zoom | warm | 5.10 | 12.57 | 34.09 | 94.01 |
| drag | warm | 4.28 | 16.39 | 36.13 | 81.15 |
| resize | warm | 4.36 | 5.41 | 6.05 | 7.76 |
| rotate | warm | 6.86 | 68.87 | 128.64 | 176.51 |
| hover | warm | 3.61 | 14.95 | 38.90 | 107.09 |
| select | warm | 3.62 | 68.00 | 192.52 | 377.99 |
| variant-change | warm | 16.88 | 73.47 | 145.81 | 263.21 |
| undo | warm | 5.93 | 20.36 | 76.20 | 203.00 |
| redo | warm | 5.78 | 6.84 | 7.39 | 11.45 |

- 지배 비용은 ADR-0084 Performance와 같이 render graph compile(frame마다 화면 전체)입니다. 이 실행의 render graph p99는
  4.7\~192ms로 부하에 크게 흔들렸습니다. 증분 compile은 render-incremental 작업(ADR-0090) 몫입니다.
- variant-change는 단계마다 card 10개 중 1개의 표시를 바꿔 scene node 약 2,500개를 다시 평가합니다(측정 첫 build miss
  2,522, p50 16.9ms).
  부하와 무관하게 예산을 넘는 구조적 비용입니다. `scene_edits`가 바뀐 card subtree마다 문서 subtree를 다시 변환하기
  때문입니다. 후속은 아래 남은 위험에 적었습니다.

## 차이(작업 지시와 다른 점)

- 13개 중 10개만 재생합니다. 나머지 3개는 엔진 경로가 없어 core replay 전용 경로를 만들면 이중 경로가 됩니다.
- full replay(XVI.2.3)가 없어 T5 = T4이고 GPU 구간이 없습니다.

## 남은 위험 / 한계

- Text Edit: runtime의 text layout hook(`canvas::text`)이 crate 밖에 공개되지 않아 Text node가 scene에 들어가지 않습니다.
  hook을 공개하거나 runtime에 문서 → scene 변환 진입점을 두면 `ReplaceText` 경로로 재생할 수 있습니다.
- Token Propagation: `stellar_bindings` 평가 결과를 문서 node 값·scene에 쓰는 제품 경로가 없습니다.
- Component Update: 컴포넌트 정의 편집이 command change set을 내지 않습니다.
- variant-change 비용: 표시 patch만 바뀐 node는 subtree 재변환 없이 표시 여부만 바꾸는 증분 경로가 필요합니다(runtime
  `scene_source`, 소유 밖).
- 측정값은 부하 상태 1회입니다. 기준 장비에서 다시 측정해야 합니다(XVI.4.6).
- 제스처 시나리오의 sample은 down·이동·up을 모두 셉니다(이동 1,000 + 2).
