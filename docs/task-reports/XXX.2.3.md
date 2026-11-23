# 작업 보고서: XXX.2.3 그래프 편집 command plan

- WBS: XXX.2.3
- 작업명: node 추가·연결·해제·삭제를 binding·expression·transition·MotionSpec command로 낮춘 plan.
- 상태: 완료 (`[x]`)
- 결정: ADR-0094 결정 4
- 선행: XXX.2.1, XXX.1.6, VIII.3.3, VIII.6.6

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/prototype/src/plan.rs`(신규). transaction(`edit.rs`)은 XXX.2.1 commit에 들어갔습니다.
- `crates/prototype/src/lib.rs`: module 등록.
- `crates/prototype/tests/graph_plan.rs`(신규).

## 구현 요약

- `Planner::plan(GraphEdit)`이 사용자 동작 하나를 `PrototypeTransaction` 하나로 만듭니다(binding Create·Replace·Delete, 상태 머신 교체, MotionSpec Create·Replace·Delete, hint 고정).
- `PrototypeTransaction::apply`는 전부 적용하거나 snapshot으로 되돌리고, inverse transaction 하나(Undo 1회)를 돌려줍니다.
- 변환 node 토글·카운터는 `not`·`+ 1` transition으로 낮춥니다. 상태 변화→펄스는 상태 머신에 trigger가 없어 `Unsupported`입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-prototype` | 통과(lib 13, 통합 test 11개 파일 24개). |
| `cargo clippy -p stellar-document -p stellar-prototype --all-targets -- -D warnings` | 통과. |

시험:

- `graph_plan.rs::graph_edits_lower_to_commands_and_undo_once`: 연결·변환 node 둘·motion 대상 추가·해제·node 삭제 셋·위치 고정 각각 적용 후 상태 변경, Undo 1회로 bytes 복원. 쓰는 곳 있는 상태 변수 삭제 거부.
- `socket_kinds_and_port_types_are_enforced`.

## 차이(작업 지시와 다른 점)

- Phase VII `CommandTransaction` journal 대신 같은 원자성·inverse 규칙을 지키는 `PrototypeTransaction`을 썼습니다(ADR-0094 결정 4.3).
- expression block 삭제는 기존 expression 편집 plan(VIII.3.3)으로 하도록 거부합니다.

## 남은 위험 / 한계

- 창별 Undo stack·DocumentThread 연결은 UI(XXX.5) 몫입니다.
