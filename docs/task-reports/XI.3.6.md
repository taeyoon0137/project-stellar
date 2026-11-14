# 작업 보고서: XI.3.6 2만 가시 node hit test p99 benchmark

- WBS: XI.3.6
- 작업명: 2만 가시 node 기준 hit test p99 benchmark, 결과 JSON에 query count·candidate count·duration
- 상태: 완료 (`[x]`)
- 선행: XI.3.4 (이 lane에서 완료: `1737788`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 실행). 측정값은 실행 기기·부하에 따라 달라집니다.

## 변경 파일 목록

- `crates/scene/tests/hit_test_benchmark.rs`(신규) — benchmark scene 생성기, 측정·집계(nearest-rank 백분위수),
  JSON 출력, 기본 테스트 1개(JSON 구성·집계 일관성)와 `#[ignore]` 측정 테스트 1개.
- `docs/task-reports/XI.3.6.md`(신규), 계획서(XI.3.6 `[ ]`→`[x]`).

## 구현 요약

- **scene**: 1920×1080 viewport를 50×40 card로 채웁니다. card는 Frame(둥근 모서리, bounds clip) 1개와 자식 9개
  (Ellipse·사각형·stroke만 있는 원·작은 사각형 Shape 4, EvenOdd 삼각형·열린 stroke Path 2, Text 2, Image 1)로
  10 node입니다. 가시 card 2,000개 = 가시 node 정확히 20,000(XI.3.2 `visible_candidates`로 확인), 가시 card 위에
  겹친 숨김 card 100개(1,000 node), viewport 밖 card 100개(1,000 node)를 더해 22,000 node입니다. 공개 API(scene
  builder)만 씁니다.
- **측정**: seed 고정 SplitMix64 view 점마다 `hit_test_view_narrow`(broad + path clip 정밀 + content narrow-phase,
  view 허용 거리 2 px)를 한 번씩 `Instant`로 재고, 같은 점의 `hit_test_broad`(허용 거리 0) 시간을 기준선으로 함께 잽니다.
  warmup query는 집계에서 뺍니다.
- **결과 JSON**(한 줄, stdout; `STELLAR_HIT_TEST_BENCH_OUT=<path>`면 파일에도 씀): `query_count`, `warmup_count`,
  `candidate_count`(`total`, query당 p50/p95/p99/max/mean), `hit_count`(적중 query 수, narrow/broad 적중 수, 숨김·
  통과·content 밖·path clip 제외 수), `duration_ns`·`broad_only_duration_ns`(p50/p95/p99/max/mean/total), `index_build_ns`,
  scene 규모(`nodes`, `indexed_nodes`, `visible_nodes`), 아키텍처 §6.2 hit test 예산(`target` 0.35 ms, `limit` 0.75 ms).
- 시간 판정(예산 초과 실패)은 하지 않습니다. 기본 `cargo test`는 같은 20,000 가시 scene에서 500 query로 JSON key와
  집계 결정론만 확인합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-scene` | 완료 |
| `cargo test -p stellar-scene` | unit 95, `hit_test_benchmark` 1 passed·1 ignored, doc 2 passed, 0 failed |
| `cargo clippy -p stellar-scene --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-scene -- --check` | 통과 |
| `cargo test --release -p stellar-scene --test hit_test_benchmark -- --ignored --nocapture`(3회, 1회는 `STELLAR_HIT_TEST_BENCH_OUT` 지정) | 3회 모두 통과, JSON 출력·파일 기록 확인 |

| 검사(검증 기준: 결과 JSON에 query count·candidate count·duration) | 결과 |
|---|---|
| 가시 node 20,000·전체 22,000, JSON에 `query_count`·`candidate_count`·`duration_ns`(p99 포함)·예산 key 존재, 괄호 균형, 같은 seed 재실행 시 후보·적중 집계 동일(`benchmark_json_reports_counts_and_durations_on_20k_visible_scene`) | 통과 |

release 측정 1회 결과(JSON 원문, 20,000 query, warmup 2,000):

```json
{"benchmark":"scene_hit_test","wbs":"XI.3.6","profile":"release","seed":13830,"scene":{"nodes":22000,"indexed_nodes":22000,"visible_nodes":20000,"hidden_cards":100,"offscreen_cards":100,"viewport":[1920,1080],"view_tolerance_px":2},"query_count":20000,"warmup_count":2000,"candidate_count":{"total":61038,"per_query":{"p50":3,"p95":5,"p99":8,"max":18,"mean":3,"total":61038}},"hit_count":{"queries_with_hit":19865,"narrow_hits":46028,"broad_hits":32497,"hidden":2870,"pass_through":0,"content_missed":3438,"path_clipped":0},"duration_ns":{"p50":1083,"p95":1792,"p99":2291,"max":61708,"mean":1145,"total":22911362},"broad_only_duration_ns":{"p50":417,"p95":584,"p99":708,"max":16791,"mean":437,"total":8749708},"index_build_ns":4273000,"budget_ns":{"target":350000,"limit":750000}}
```

추가 3회 실행의 scene hit test p50/p95/p99/max(ns): 916/1,500/1,792/17,000, 958/1,667/4,791/9,666, 959/1,666/2,000/16,791.
네 번 모두 p99가 §6.2 목표 0.35 ms(350,000 ns)의 약 1.4% 이하였습니다. 후보 수 합계(61,038)는 seed가 같아 매번 같습니다.

## 차이(작업 지시와 다른 점)

- benchmark는 별도 bench target이 아니라 `#[ignore]` 통합 테스트로 둡니다(외부 benchmark crate 없이 기존 측정
  테스트 관례와 같은 방식). 측정 구간은 scene hit test(공간 인덱스 질의 → broad → narrow)이며 입력 이벤트 수신(T0)부터의
  queue latency는 XII.3.5 범위입니다.
- `narrow_hits`가 `broad_hits`보다 많은 것은 narrow 측정이 view 허용 거리 2 px로 후보를 넓히고, broad 기준선은 허용 거리
  0이기 때문입니다(두 수는 서로 비교하는 값이 아님).

## 남은 위험 / 한계

- 측정은 DEV-MAC 단일 기기·짧은 실행이며 CI 성능 게이트·회귀 추적(§38.1, §38.3) 연결은 하지 않았습니다. max는 OS
  선점 등으로 크게 튑니다(9.7~61.7 µs).
- scene은 작은 card 격자라 query당 후보가 적습니다(p99 8). 큰 node가 많이 겹치는 문서나 복잡한 path(cubic 수천 개)는
  비용이 다를 수 있어 XVI.3 fixture로 추가 측정이 필요합니다.
- Text는 layout 상자 정책(`LayoutBoxesOnly`)으로 측정했습니다. line rect 제공자 비용은 포함하지 않습니다.

## 후속 측정 (XI.3.4 narrow-phase 후속 수정 뒤)

XI.3.4 후속 수정(허용 거리를 Butt cap 끝 너머·Inside/Outside 띠 반대쪽·열린 fill의 닫는 변까지 적용, 폭 0 stroke만 있는
path 미적중, 반복 끝점 cap)으로 같은 seed의 적중 집계가 바뀌었습니다. benchmark 코드와 scene 구성은 그대로입니다.

- `hit_count`: `narrow_hits` 46,028 → 46,214, `content_missed` 3,438 → 3,252. `queries_with_hit` 19,865, `broad_hits`
  32,497, `hidden` 2,870, 후보 수 합계 61,038은 같습니다.
- release 3회(`cargo test --release -p stellar-scene --test hit_test_benchmark -- --ignored --nocapture`)의 scene hit test
  p50/p95/p99/max(ns): 1,125/2,166/5,792/28,875, 1,083/1,875/4,208/27,708, 1,083/1,833/2,333/36,417. p99는 모두 §6.2 목표
  350,000 ns의 약 1.7% 이하입니다.
