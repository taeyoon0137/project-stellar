# 작업 보고서: XXX.2.5 진단의 그래프 위치 매핑

- WBS: XXX.2.5
- 작업명: expression·상태 머신·Context·MotionSpec 진단을 그래프 node·socket으로.
- 상태: 부분 완료 (`[~]`) — expression 타입 오류, 포트 타입, 상태 머신, MotionSpec 진단 매핑은 끝났습니다. Context 진단(XXX.7.5)이 남았습니다.
- 결정: ADR-0094 결정 3
- 선행: XXX.2.1, VIII.3.4, IX.5.7, XXX.7.5

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/prototype/src/diagnostics.rs`(신규).
- `crates/prototype/src/lib.rs`: module 등록.
- `crates/prototype/tests/graph_diagnostics.rs`(신규).

## 구현 요약

- `graph_diagnostics`가 진단마다 `GraphLocation{node, socket}`을 돌려줍니다(표는 module 문서).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-prototype` | 통과(lib 13, 통합 test 11개 파일 24개). |
| `cargo clippy -p stellar-document -p stellar-prototype --all-targets -- -D warnings` | 통과. |

시험:

- `graph_diagnostics.rs::diagnostics_map_to_graph_nodes_and_sockets`: orphan 대상 → drive socket 1, 없는 시작 변수 → start, source 없는 transition → trigger, `not 5px` → block operand slot. 원래 fixture는 진단 0.

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- Context 진단 매핑은 XXX.7.5 이후입니다.

## 후속 보완 (2026-11-24, Context 제품 경로 연결)

- 결정: ADR-0114 결정 7.
- projection이 `Context(K)` 참조 block마다 key 단위 ContextRead node(StableId = key)와 `value → reference` edge를 만듭니다.
  Context를 읽지 않는 그래프의 bytes는 그대로입니다.
- `context_graph_diagnostics`가 문서 validator 진단을 그래프 위치로 옮깁니다: 선언 없는 소비·없는 key 소비·default·shadowing은
  참조 block의 `reference` socket, 타입 불일치는 PropertyOutput `value` socket, 쓰지 않는·없는 requirement는 ContextRead node.
  provision·Slot 공개 목록 진단은 그래프 위치가 없습니다.
- 시험: `graph_context::context_reads_project_as_one_node_per_key`(node·edge, lower 왕복, bytes 결정성, Context 없는 그래프에
  ContextRead 0), `graph_context::context_diagnostics_map_to_graph_positions`(위치 3건, 다른 컴포넌트·provision 진단 제외).
- 남은 위험: validator는 문서 모델이 필요해 호출자가 진단을 넘깁니다(PrototypeDocument에는 문서 모델이 없음).
