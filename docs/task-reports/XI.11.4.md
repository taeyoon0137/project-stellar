# 작업 보고서: XI.11.4 2만 가시 node scene 구간 benchmark

- WBS: XI.11.4
- 작업명: 2만 가시 node scene에서 scene build, render graph encode, submit까지 구간 benchmark 추가
- 상태: 완료 (`[x]`)
- 선행: XI.2.6 (`[x]`), XI.4.6 (`[x]`), XI.8.5 (`[x]`)
- 결정 기록: ADR-0102 결정 7

## 필수 실행 환경

- `DEV-MAC` (Apple Silicon GPU, 창에 붙지 않은 CAMetalLayer, 로컬 검증).

## 변경 파일 목록

- `benchmarks/fixtures/tests/render_stage_benchmark.rs`(신규) — 2만 가시 node scene, 단계별 측정, 대리 frame encoder, 결과 JSON.
- `benchmarks/fixtures/Cargo.toml`, `Cargo.lock` — `stellar-scene`·`stellar-renderer-core` 의존, macOS dev 의존 `stellar-renderer-metal`.

## 구현 요약

- scene: XI.3.6과 같은 card 2,000개(Frame + 자식 9) = 가시 20,000 node, 숨김·화면 밖 card 각 100개(전체 22,000 node).
- 단계: `scene_build`(SceneBuilder 전체 생성), `visibility`(공간 index + 가시 후보), `render_graph`(`compile_frame` + `batch_frame`), `encode`(batch →
  draw command·정점·uniform, buffer 갱신, Metal `encode`), `submit`(`FrameScheduler::tick`의 encode 밖: poll·present 예약·`submit`), `total`.
  frame마다 nearest-rank p50/p95/p99·max·mean(ns)을 냅니다.
- 결과 JSON에 XI.11.5 `present_queue` 지표(queue 깊이 ≤ 1 assertion, 집계 검사 포함)와 아키텍처 §6.2 단계 예산(µs, 참고)을 넣습니다.
- batch → draw command는 대리 구현입니다(node world bounds 사각형, 단색은 첫 paint 색, texture program은 1×1 대체 texture). 제품 frame encoder가
  없기 때문이며 생기면 바꿉니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-bench-fixtures --test render_stage_benchmark` | 1 passed, 1 ignored(측정 전용) |
| `cargo test --release -p stellar-bench-fixtures --test render_stage_benchmark -- --ignored --nocapture` | 통과, 아래 JSON |
| `cargo clippy -p stellar-bench-fixtures --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| frame 3개(warmup 1): 전체 22,000·가시 20,000 node, draw ≥ 가시 node, 단계 6개마다 표본 3개, JSON에 단계별 p50/p95/p99·`present_queue`·`budget_us`, 중괄호 짝, queue 깊이 ≤ 1(`stage_json_reports_percentiles_for_each_stage_on_20k_visible_scene`) | 통과 |

release 측정(120 frame, warmup 10, 다른 작업 12개가 함께 도는 개발 장비, 참고 값):

| 단계 | p50 | p95 | p99 |
|---|---|---|---|
| scene_build | 3.03 ms | 4.11 ms | 5.48 ms |
| visibility | 6.64 ms | 10.02 ms | 14.17 ms |
| render_graph | 2.32 ms | 2.99 ms | 4.58 ms |
| encode | 5.98 ms | 7.88 ms | 8.80 ms |
| submit | 0.04 ms | 0.13 ms | 0.15 ms |
| total | 18.12 ms | 23.27 ms | 29.36 ms |

work: pass 2, batch 8,000, draw 20,000, command 64,000. present queue: 최대 깊이 1, 130 tick 모두 제출, 폐기·throttle 0.

## 차이(작업 지시와 다른 점)

- `scene_build`는 매 frame 전체 재빌드(최악 경우)입니다. 증분 build(XI.2.5)·증분 compile(ADR-0090)의 재사용 효과는 이 benchmark가 보지 않습니다.
  상호작용 경로 측정은 `cargo xtask replay`(ADR-0084·0090)가 맡습니다.
- encode 단계의 command 생성은 대리 구현이라 제품 frame encoder 비용과 다릅니다.

## 남은 위험 / 한계

- 측정 값은 부하가 있는 개발 장비 값입니다. 기준 장비(M1 16GB)에서 다시 재야 하며, §6.2 예산(scene 1.75 ms, render graph 1.25 ms 상한)과 비교하면
  전체 재빌드 경로는 예산을 넘습니다. 상호작용 판정은 증분 경로로 해야 합니다.
- visibility 단계(공간 index 전체 build)가 가장 큽니다. frame마다 index를 다시 만드는 대신 증분 갱신(XI.3.1 update)을 쓰는 연결이 후속입니다.
- CI 실행과 G6 판정은 XI.11.6입니다.
