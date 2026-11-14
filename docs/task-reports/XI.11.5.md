# 작업 보고서: XI.11.5 benchmark 결과의 present queue 깊이·dropped stale frame 수

- WBS: XI.11.5
- 작업명: Present queue depth와 dropped stale frame count를 benchmark 결과에 포함하고 queue depth ≤ 1 assertion 제공
- 상태: 완료 (`[x]`)
- 선행: XI.5.5 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/present_metrics.rs`(신규) — `PresentRecorder`, `PresentMetrics`(`assert_queue_depth`,
  `check_accounting`, `fields`, `to_json`), `PresentMetricsError`, `DEPTH_BUCKETS`.
- `crates/renderer-core/src/present_metrics/tests.rs`(신규) — slow GPU benchmark 모사와 assertion 테스트.
- `crates/renderer-core/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XI.11.5.md`(신규), 계획서(XI.11.5 `[ ]`→`[x]`).

## 구현 요약

- **기록**: `PresentRecorder::record(&scheduler)`를 매 `tick` 뒤 호출해 present queue 깊이 표본을 깊이별 분포(`0..=1` +
  상한 초과 칸)와 최대값으로 모읍니다. `finish`는 분포와 XI.5.5 `SchedulerStats`(offer·submit·encode 전/후 폐기·throttle·
  skip·scheduler 관측 최대 깊이), 끝난 시점 pending 수를 `PresentMetrics`로 합칩니다.
- **assertion**: `assert_queue_depth`는 최대 깊이 > 1이거나 초과 칸에 표본이 있으면 `QueueDepthExceeded`입니다.
  `check_accounting`은 offer = 제출 + 폐기(전·후) + pending을 확인합니다(지표 누락 방지).
- **결과 JSON**: `to_json`은 key 순서가 고정된 정수 전용 객체
  `{"present_queue":{"limit","max_depth","ticks","offered","submitted","dropped_stale_frames","dropped_before_encode",
  "dropped_after_encode","throttled_ticks","skipped_ticks","pending_at_end","depth_histogram":[...]}}`를 만들고, `fields`는 같은
  값을 (정적 key, 값) 배열로 줍니다(XI.4.6 trace와 같은 비민감 기록 규칙). 실제 benchmark 결과 JSON에 이 객체를 넣는 연결은
  단계별 timing benchmark를 만드는 XI.11.4(`[ ]`)에서 합니다. 지금 renderer-core 밖에서 `PresentRecorder`를 쓰는 benchmark는
  없습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-renderer-core` | 종료 코드 0 |
| `cargo test -p stellar-renderer-core` | 147 unit passed(신규 2) + 3 doctest passed, 0 failed |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-core -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| slow GPU benchmark 모사(300 tick, GPU 3 tick당 1 frame, 5 tick마다 snapshot 2개, 11 tick마다 encode 중 새 세대): queue depth ≤ 1 assertion 통과, 집계 일치, 최대 깊이 1, 분포 합 300·초과 칸 0, encode 전·후 폐기·throttle 발생, 결과 JSON golden `{"present_queue":{"limit":1,"max_depth":1,"ticks":300,"offered":360,"submitted":100,"dropped_stale_frames":259,"dropped_before_encode":250,"dropped_after_encode":9,"throttled_ticks":191,"skipped_ticks":0,"pending_at_end":1,"depth_histogram":[9,291,0]}}`, 재실행 동일(`benchmark_result_includes_queue_depth_and_dropped_frames`) | 통과 |
| 깊이 2 표본·초과 칸만 있는 결과는 `QueueDepthExceeded`, offer 수 변조는 `AccountingMismatch`(`queue_depth_violation_fails_the_assertion`) | 통과 |

## 차이(작업 지시와 다른 점)

- 실제 GPU benchmark 실행 파일과 2만 node 단계 benchmark(XI.11.4)는 아직 없습니다. 이 작업은 benchmark 결과에 들어갈
  지표·assertion·JSON 형식과 mock backend 기반 결정적 모사까지입니다. Metal backend가 생기면(XI.8.5) 같은 recorder로
  실측 결과를 만듭니다.
- 테스트용 mock backend는 `#[cfg(test)]` 전용이라 benchmark 바이너리에서 쓰지 않습니다.

## 남은 위험 / 한계

- 실제 Metal 실행에서는 `GpuProgress.presented`가 드라이버 present 완료 시점에 의존하므로 throttle·폐기 수가 모사와 다릅니다.
  queue depth ≤ 1 assertion은 그대로 적용됩니다.
- CI benchmark 실행·G6 판정(XI.11.6)은 후속 작업입니다.

## 상태 확인(통합 검증 반영, 2026-11-14)

- 구현 요약의 "XI.11.4 결과 JSON에 포함" 문장이 차이 절("XI.11.4는 아직 없음")과 모순되어 사실대로 고쳤습니다. 실제
  benchmark JSON 연결은 XI.11.4 범위입니다.
- 상태는 `[x]`를 유지합니다. 같은 통합에서 XI.8.2를 `[~]`로 낮춘 기준은 계획 §1.3 규칙 5(필수 환경·조건을 모방한 증거로
  `[x]` 금지)였습니다. XI.8.2의 검증 열은 실제 창 상태(최소화·가림)를 요구했는데 재현으로만 확인했습니다. 이 행의 검증 열은
  "queue depth≤1 assertion"이고 필수 환경은 `DEV-MAC`(순수 Rust)입니다. assertion은 실제 XI.5.5 frame scheduler 출력에 대해
  `DEV-MAC`에서 실행했고, mock backend는 GPU 진행 시점만 대신합니다. 검증 열이 실제 GPU나 실제 benchmark 실행을 요구하지
  않습니다. 또 계획 선행상 이 행(선행 XI.5.5)은 XI.11.4보다 먼저 완료될 수 있습니다. 그래서 XI.8.2와 같은 모방 증거 문제로
  보지 않았습니다.
