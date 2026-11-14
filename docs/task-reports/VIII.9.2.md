# 작업 보고서: VIII.9.2 단일 token 변경 재평가 benchmark

- WBS: VIII.9.2
- 작업명: 단일 token 변경의 evaluation count와 duration을 측정하는 benchmark(결과 JSON에 affected/visited/evaluated count)
- 상태: 완료 (`[x]`)
- 선행: VIII.9.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증. 기본 실행은 debug, 대규모 측정은 release 빌드).

## 변경 파일 목록

- `crates/bindings/tests/token_change_benchmark.rs`(신규) — benchmark harness(synthetic graph 실체화 → `BindingEvaluator`
  → 토큰별 단일 표현식 변경 측정), 결과 JSON 출력, count 독립 검산, `#[test]` 3개.
- `docs/task-reports/VIII.9.2.md`(신규), 계획서(VIII.9.2 `[ ]`→`[x]`).

## 구현 요약

- **대상**: VIII.9.1 synthetic graph를 실체화한 registry·binding으로 VIII.6.4 `BindingEvaluator`를 만들고, 토큰 하나의
  표현식을 `update_expression`으로 바꾼 한 번의 증분 재평가를 단일 token 변경으로 측정합니다. count는 scheduler의
  `UpdateReport`(affected·visited·evaluated, changed_inputs)를 그대로 기록하고, duration은 `update_expression` 호출
  전체(새 표현식 컴파일 + 영향 범위 BFS + 위상 순 재평가)의 `Instant` 측정값입니다.
- **토큰 선택**: hub(직접 dependents 최다), sparse(최소, 동률은 큰 번호), 고정 seed SplitMix64 무작위 N개(서로 다름).
- **토큰당 변경 3종**: `raise`(`원래 + 1000`, 값 0~999보다 커서 영향 범위 전체로 전파), `revert`(원래 표현식),
  `same_value`(`원래 + 0`, 값이 같아 early cutoff로 그 토큰만 평가). 참조 집합이 같아 graph 재구성은 없습니다.
- **결과 JSON**(`stellar.bench.token-change` v1, ASCII·키 순서 고정): profile·preset·config·graph(node·edge·digest)·setup
  시간(generate·materialize·build)·참고 예산(아키텍처 §6.2 재평가 단계 목표 0.60ms/상한 1.00ms, gate 아님)·sample마다
  `affected`/`visited`/`evaluated`/`changed_inputs`/`duration_ns`·summary(count min/max/total, duration
  min/p50/p95/p99/max/mean(nearest-rank), 예산 이내 수).
- **실행**: 기본 `cargo test`는 작은 설정(토큰 600·binding 1,800·간선 6,000)입니다. `STELLAR_TOKEN_BENCH_PRESET=edges_1m`
  ·`active_200k`, `STELLAR_TOKEN_BENCH_SAMPLES`, `STELLAR_TOKEN_BENCH_OUTPUT`로 preset·표본 수·출력 경로를 고릅니다(기본
  출력 `target/tmp/token-change-benchmark/<preset>.json`).
- **검산(모든 preset)**: affected·visited = spec에서 독립 계산한 역방향 도달 producer 수, raise·revert의 evaluated =
  affected, same_value의 evaluated = 1, 토큰별 3변경 후 전체 node 값 = 초기 값. 작은 설정은 raise마다 증분 결과를 새
  evaluator 전체 평가와 비교합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-bindings` | 종료 코드 0 |
| `cargo test -p stellar-bindings` | 첫 실행 1건 실패(작은 설정에 dependents 0인 토큰이 없어 `leaf` 선택 가정 오류) → 선택을 `sparse`(최소 dependents)로 바꾼 뒤 종료 코드 0 (unit 51 + `token_change_benchmark` 3 passed, 0 failed) |
| `cargo clippy -p stellar-bindings --all-targets -- -D warnings` | 첫 실행 `map_identity`·`write_with_newline` 2건 → 수정 후 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-bindings -- --check` | 종료 코드 0 |
| `STELLAR_TOKEN_BENCH_PRESET=edges_1m cargo test -p stellar-bindings --release --test token_change_benchmark single_token -- --nocapture` | 종료 코드 0, 1 passed(14.62 s), `target/tmp/token-change-benchmark/edges_1m.json` 생성 |
| `STELLAR_TOKEN_BENCH_PRESET=active_200k cargo test -p stellar-bindings --release --test token_change_benchmark single_token -- --nocapture` | 종료 코드 0, 1 passed(5.05 s), `active_200k.json` 생성 |

| 검사 | 결과 |
|---|---|
| 결과 JSON의 모든 sample 줄과 summary에 `affected`/`visited`/`evaluated` count 존재, 값이 측정값과 일치(`single_token_change_benchmark_writes_counts_and_durations`) | 통과 |
| count 독립 검산(역방향 도달 집합)·same_value cutoff·초기 값 복원·작은 설정 전체 재평가 일치 | 통과(small·edges_1m·active_200k) |
| 같은 설정 두 번 실행 count 동일, 다른 graph seed는 count 다름(`counts_are_deterministic_across_runs`) | 통과 |
| nearest-rank 백분위수(`percentile_uses_nearest_rank`) | 통과 |

## 성능 수치 (`DEV-MAC`, release, 토큰 34개 × 변경 3종 = 102회)

| preset | node / 간선 | setup(generate / materialize / build) | duration p50 / p95 / p99 / max | affected max | 상한 1.00ms 이내 |
|---|---|---|---|---|---|
| `edges_1m` | 300,003 / 1,000,000 | 16.4 ms / 1.41 s / 7.23 s | 151 µs / 270.6 ms / 1.07 s / 1.42 s | 271,603(hub = 토큰 0) | 72 / 102 |
| `active_200k` | 200,000 / 500,000 | 5.6 ms / 533 ms / 3.01 s | 75 µs / 15.4 ms / 438 ms / 532 ms | 176,218(hub = 토큰 5) | 87 / 102 |

- `edges_1m` 대표 sample: sparse 토큰 99855 affected 1 → raise 30.8 µs, 무작위 토큰 52158 affected 237 → raise 717 µs·
  revert 224 µs·same_value 16.6 µs, hub 토큰 0 same_value(evaluated 1)도 affected 271,603을 순회해 217 ms.
- duration은 이 기기의 1회 측정값이며 결정적이지 않습니다. count는 결정적입니다.

## 차이(작업 지시와 다른 점)

- benchmark를 별도 bench crate나 `cargo bench`(nightly·criterion 의존성) 대신 bindings crate의 integration test로
  두었습니다. 기본 `cargo test`는 작은 설정으로 검산만 하고, 대규모 측정은 release + 환경 변수로 실행합니다.
- 단일 token 변경을 "토큰 표현식 교체"로 정의했습니다. synthetic graph의 토큰은 모두 producer라 source 값 변경 대상이
  없습니다.

## 남은 위험 / 한계

- 측정상 hub 토큰 변경은 영향 범위가 graph의 90%(27만 node)에 달해 1 s 수준이며, 값이 같아도(same_value) 영향 범위
  BFS만으로 217 ms가 걸립니다. 아키텍처 §6.2 재평가 상한(1.00 ms)은 영향 범위가 수백 node 이하일 때만 만족합니다.
  BFS 전에 early cutoff를 적용하는 lazy 전파(평가하면서 dependents를 확장)와 `HashSet`·`BTreeMap` index 대신 dense
  bitset·배열 index 사용이 후속 최적화 후보입니다. 성능 gate 판정은 이 작업 범위가 아닙니다.
- `BindingEvaluator::new` 구성은 1M 간선에서 7.2 s입니다(node마다 `BTreeMap` index 조회·전체 컴파일·전체 평가). 문서
  열기 경로 예산은 별도 작업에서 다뤄야 합니다.
- synthetic graph는 수치 `max` 표현식만 써서 실제 문서의 표현식 비용(색·문자열 함수)을 반영하지 않습니다.
- 결과 JSON은 CI artifact로 보존하지 않습니다(성능 추이 기록은 Phase XVI 범위).
