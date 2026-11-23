# 작업 보고서: XVI.4.9 토큰·binding graph 증분 갱신

- WBS: XVI.4.9
- 작업명: 토큰·binding·orphan command 검증이 호출마다 다시 만드는 토큰 dependency graph와 통합 binding graph를 편집과 함께 증분 갱신
- 상태: 완료 (`[x]`)
- 선행: VIII.5.5, VIII.6.6, VIII.8.4
- 결정 기록: ADR-0108 결정 3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, release 측정 포함).

## 변경 파일 목록

- `crates/bindings/src/graph_view.rs`(신규) — `BindingGraphView`(통합 graph view), `TokenGraphView`(토큰 graph view),
  공용 BFS `first_cycle_path`.
- `crates/bindings/src/graph.rs` — `DependencyGraph::cycle_if_dependencies`가 같은 공용 BFS를 씀(판정·경로 동일).
- `crates/bindings/src/token_graph.rs` — `TokenDependencyGraph::cycle_if_references`가 같은 공용 BFS를 씀(판정·경로 동일).
- `crates/bindings/src/binding_commands.rs`, `crates/bindings/src/orphan_commands.rs`, `crates/bindings/src/token_commands.rs` —
  검증의 `DependencyGraph::build(..)`·`TokenDependencyGraph::build(..)`를 view로 바꿈(각 한 줄과 import).
- `crates/bindings/src/graph_view_tests.rs`(신규, test 전용), `crates/bindings/src/lib.rs`(module 등록).

## 구현 요약

- 두 graph의 간선은 producer 하나의 참조입니다. 토큰 node는 그 토큰 정의, 속성 node는 그 속성을 target으로 하는 binding이
  간선을 정합니다. `TokenRegistry`(토큰 ID → 정의)와 `BindingSet`(target 속성 → binding)은 이 producer 조회 index를 이미
  편집과 함께 갱신하므로, graph의 인접 목록을 따로 들지 않고 두 저장소를 그대로 인접 목록으로 쓰는 view를 두었습니다.
  편집 하나가 바꾸는 간선은 그 producer의 간선뿐이고, 저장소가 바뀌는 순간 view도 바뀝니다. 그래서 별도 graph를 맞추는
  갱신 코드나 어긋날 상태가 없습니다(ADR-0108 결정 3).
- cycle 검사는 새 참조에서 도달하는 부분만 BFS로 훑습니다. 전체 graph 두 개와 view가 같은 BFS(`first_cycle_path`)를 쓰므로
  같은 인접 목록이면 같은 판정·같은 경로입니다. 전체 graph에 없는 node(아무도 읽지 않고 producer도 없는 node)는 들어오는
  간선이 없어 view에서도 자연히 cycle이 없습니다.
- 토큰 view의 간선은 전체 graph와 같은 규칙(registry에 있거나 tombstone이 있는 대상만)입니다.
- cycle·참조 판정은 바꾸지 않았습니다.

## 성능

`cargo test -p stellar-bindings --release --lib graph_view_tests -- --ignored --nocapture`. synthetic fixture(VIII.9.1, 토큰
1,000 + binding 1,000·10,000). 다른 작업 12개가 동시에 도는 부하 상태에서 잰 값입니다. "이전"은 같은 검사를 호출마다 전체
graph를 다시 만들어 한 값(graph 구성 + cycle 질의만, type 검사 제외)입니다.

| command validate | binding 1,000 | binding 10,000 | 비율 | 이전(1,000 → 10,000) |
|---|---:|---:|---:|---:|
| binding Replace(새 source가 토큰 하나를 읽음) | 11.8 µs | 7.9 µs | 0.67배 | 1.30 ms → 7.29 ms |
| binding Replace(원래 참조 유지, 앞선 속성 포함) | 24.2 µs | 24.4 µs | 1.01배 | 1.30 ms → 7.29 ms |
| 토큰 SetExpression | 4.97 µs | 4.74 µs | 0.95배 | 0.30 ms → 0.28 ms |
| orphan Reconnect | 8.1 µs | 8.0 µs | 0.99배 | 1.09 ms → 7.30 ms |

모든 command에서 binding 10배에 시간 2배 이하입니다. 시험은 binding Replace·토큰 SetExpression·orphan Reconnect 중 하나라도
2배를 넘으면 실패합니다. cycle 검사 비용은 새 참조가 도달하는 subgraph 크기에 비례합니다. 도달 범위가 큰 편집(긴 binding
사슬 끝을 읽는 source)은 그만큼 더 걸리며, 전체 graph 재구성 비용을 넘지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-bindings` | unit 56 passed(3 ignored는 release 측정), `fallback_golden_corpus` 2, `token_change_benchmark` 3 passed |
| `cargo test -p stellar-bindings --release --lib graph_view_tests -- --ignored --nocapture` | 위 표, 통과 |
| `cargo test -p stellar-prototype -p stellar-semantic-diff` | 통과(binding command를 쓰는 다른 crate) |
| `cargo clippy -p stellar-bindings -p stellar-prototype -p stellar-integrity --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-bindings -- --check` | 통과 |

## 테스트

| 검사 | 결과 |
|---|---|
| random command sequence(seed 30 × 토큰·binding command 40단계 + orphan command 20단계, cycle 거부 20건 이상 포함) 뒤 view의 모든 node 간선·producer, 토큰 간선이 전체 재구성 graph와 같고, 단계마다 random cycle 질의 12개(graph 밖 node 포함)의 결과·경로가 같음(`views_match_full_rebuilds_after_random_command_sequences`) | 통과 |
| 변이 확인: 토큰 view가 tombstone 대상 간선을 빠뜨리게 바꾸면 위 시험이 실패함 | 확인 후 원복 |
| 기존 VIII.5.5(token_commands·token_graph), VIII.6.6(binding_commands·graph), VIII.8.4(orphan_commands) test | 통과 |

## 차이(작업 지시와 다른 점)

작업 지시와 다른 점은 아래 "남은 위험 / 한계"에 함께 적었습니다.

## 남은 위험 / 한계

- 저장 전 전체 검증(`validate_for_save`의 `find_cycle`)과 평가기(`BindingEvaluator`)는 graph 전체가 필요한 작업이라 전체
  graph를 그대로 만듭니다(command 검증 경로가 아님).
