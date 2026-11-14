# 작업 보고서: X.7.4 20만 node 단일 leaf 변경 방문 수 benchmark

- WBS: X.7.4
- 작업명: 20만 node synthetic document에서 단일 leaf 변경 방문 node count를 기록하는 benchmark
- 상태: 완료 (`[x]`)
- 선행: X.7.3(`[x]`, 이 lane `51dc6b1`) — 명시 행 선행 충족, 완화 없음

## 필수 실행 환경

- `DEV-MAC` (Apple M3 Max, release build, 로컬 실행).

## 변경 파일 목록

- `crates/layout/src/bench.rs`(신규) — 결정적 합성 문서 `synthetic_input`(`LeafEditConfig::DEFAULT` = 200 section × 100 row
  × 9 leaf = 200,201 node), `run_leaf_edit_benchmark`, `LeafEditBenchmark::to_json`, 테스트.
- `crates/layout/examples/incremental_leaf_bench.rs`(신규) — 실행기(`--out`, `--edits`, `--sections`, `--rows`).
- `crates/layout/src/incremental.rs` — `IncrementalLayout::node`(index 조회) 추가.
- `crates/layout/src/lib.rs` — `bench` 모듈 등록.
- `crates/layout/bench-results/X.7.4-leaf-edit-DEV-MAC.json`(신규, 생성물) — 원시 결과.
- `docs/task-reports/X.7.4.md`(신규), 계획서(X.7.4 `[ ]`→`[x]`).

## 구현 요약

- **합성 문서**: root(Fixed Free) → section(Fixed 세로 Stack) → row(Hug 가로 Stack) → leaf(고정 box 7, Hug Text 1, 너비 고정
  Image 1). id·OrderKey·크기는 번호에서 결정적으로 만듭니다.
- **측정**: `IncrementalLayout`으로 전체 배치 1회 후, edit `i`마다 section `i % 200`·row `(7i) % 100`의 첫 leaf 너비를 1px
  늘리는 단일 edit를 적용하고 `UpdateStats`(visited = 배치를 다시 계산한 node, reused, cache_hits 등)와 적용 시간(ns)을
  기록합니다. 비교 기준으로 마지막 입력의 전체 계산(`compute_layout_with`) 시간도 기록합니다.
- **JSON**: `benchmark`, `profile`, `nodes`, 설정, `initial`(집계 + `duration_ns`), `full_compute_ns`, `summary`(`visited_max`,
  `visited_p50`, `reused_total`, `cache_hits_total`, `full_layouts_total`, `duration_ns_p50/p95/max`), `samples`(edit별 `leaf`,
  `visited`, `reused`, `cache_hits`, …, `duration_ns`). 외부 serializer 없이 key 순서를 고정해 씁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | unit 167 + `layout_corpus` 2 + doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 (example 포함) |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |
| `cargo run --release -q -p stellar-layout --example incremental_leaf_bench -- --out crates/layout/bench-results/X.7.4-leaf-edit-DEV-MAC.json` | 종료 코드 0(총 4회 실행: 이 경로 1회 + scratch 경로 3회, 4번째 실행 결과를 이 경로에 저장) |

| 검사 | 결과 |
|---|---|
| 합성 문서 검증 통과·node 수 = 설정·결정적, 기본 200,201 node(`synthetic_document_is_valid_and_sized_as_configured`) | 통과 |
| 작은 문서(727 node)에서 edit마다 visited 3·reused 19·측정 cache hit ≥ 11·전체 레이아웃 0, JSON에 visited/cache_hits/duration key(`single_leaf_edits_visit_three_nodes_and_reuse_the_rest`) | 통과 |
| benchmark edit 뒤 결과 = 전체 계산(`benchmark_edits_keep_results_equal_to_full_layout`), 백분위 nearest-rank(`percentile_uses_nearest_rank`) | 통과 |

### 원시 결과 요약 (`crates/layout/bench-results/X.7.4-leaf-edit-DEV-MAC.json`, 200,201 node, edit 64회)

| 지표 | 값 |
|---|---|
| visited(edit당, 64회 모두) | 3 (section·row·leaf) |
| reused(edit당) | 107 (같은 section의 다른 row 99 + 같은 row의 다른 leaf 8) |
| cache_hits(edit당) | 201 |
| full_layouts(edit 합) | 0 |
| edit duration p50 / p95 / max | 0.222 ms / 0.702 ms / 18.85 ms |
| 초기 전체 배치(증분 상태 구성 포함) | 2.48 s |
| 전체 계산 비교 기준(`full_compute_ns`) | 1.26 s |

- 4회 실행 모두 visited·reused·cache_hits는 같았고(결정적), 시간은 p50 0.22~0.38 ms, max 5~47 ms, 초기 배치 2.5~4.8 s로
  흔들렸습니다. 측정 시 다른 병렬 작업으로 load average가 약 106이었습니다.

## 차이(작업 지시와 다른 점)

- 실행기를 cargo example로 두었습니다(`benches/`·외부 harness 없음). 결과 JSON은 layout crate 아래
  `bench-results/`에 저장했습니다(허용 경로 안).

## 남은 위험 / 한계

- 시간 값은 부하가 큰 환경에서 1회씩 측정한 값이라 p95·max가 불안정합니다. 성능 게이트 판정에는 통제된 기준 장비(아키텍처
  6장)에서 반복 측정이 필요합니다.
- 20만 node **전체** 배치가 release에서 1.1~1.4 s(증분 상태 구성 포함 2.5 s 이상)로, 초기 로드·전체 대체 경로가 느립니다.
  일반 edit는 증분 경로라 영향이 없지만, 전체 배치 최적화는 별도 작업이 필요합니다.
- 합성 문서 하나(Stack 위주)만 측정합니다. Grid·Constraint 위주 문서와 구조 edit 측정은 포함하지 않았습니다.
