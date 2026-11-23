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
