# 작업 보고서: XXX.2.1 읽기 전용 PrototypeGraphView

- WBS: XXX.2.1
- 작업명: binding·expression AST·상태 머신·MotionSpec에서 node·edge를 만드는 결정론적 projection.
- 상태: 부분 완료 (`[~]`) — binding·expression·상태 머신(상태 변수·이벤트 원천·Transition·semantic emit)·MotionSpec projection은 끝났습니다. Context 읽기(XXX.7.4 이후)와 Control Contract 이벤트 표 projection이 남았습니다.
- 결정: ADR-0094 결정 3
- 선행: XXX.1.3, VIII.3.1, IX.5.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/prototype/src/graph.rs`(신규).
- `crates/prototype/src/edit.rs`(신규): projection·편집·프리뷰가 함께 읽는 문서 상태 묶음 `PrototypeDocument`와 원자 transaction(XXX.2.3이 씀).
- `crates/prototype/src/lib.rs`: module 등록.
- `crates/prototype/tests/support/mod.rs`(신규): 토글 컴포넌트 fixture.
- `crates/prototype/tests/graph_projection.rs`(신규).

## 구현 요약

- node는 원본 StableId와 종류 tag 순, edge는 출력·입력 socket 순입니다. `to_bytes`가 결정론 확인용 canonical bytes입니다.
- 상태 변수·이벤트 payload 참조 block에는 원천에서 `reference` edge가, MotionSpec에서 대상 속성 출력으로 `drive` edge가 이어집니다.
- `lower_bindings`·`lower_transitions`가 projection에서 원본을 다시 만들어 의미 보존을 확인합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-prototype` | 통과(lib 13, 통합 test 11개 파일 24개). |
| `cargo clippy -p stellar-document -p stellar-prototype --all-targets -- -D warnings` | 통과. |

시험:

- `graph_projection.rs::projection_is_deterministic_and_lowers_to_same_meaning`: 같은 입력 bytes 동일, binding canonical AST·transition spec 재구성 일치, 모든 edge가 registry socket 종류 규칙을 지킴.

## 차이(작업 지시와 다른 점)

- Control Contract 이벤트는 상태 머신 trigger(EventKind)로만 나타납니다. Contract 표 자체를 node로 projection하지 않았습니다.

## 남은 위험 / 한계

- Context 원본(XXX.7.4)이 생기면 ContextRead node를 더해야 합니다.
- projection은 문서 전역 StableId 유일성을 전제합니다.
