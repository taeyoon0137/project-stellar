# 작업 보고서: VIII.3.3 block 추가·연결·삭제 command plan API

- WBS: VIII.3.3
- 작업명: block 추가·연결·삭제를 command plan으로 생성하는 API — 실제 UI는 Phase XII에서 연결
- 상태: 완료 (`[x]`)
- 선행: VIII.3.2(`[x]`), Phase VII(완화 — 아래 "차이" 참조)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/expressions/src/edit_plan.rs`(신규) — `EditOp`, `EditPlan`(`ops`, `apply`), `PlanError`, `NewBlock`, `InputSpec`,
  `Placement`, `DeleteReplacement`, `plan_add_block`, `plan_connect`, `plan_delete_block`, `default_literal`, 단위 테스트.
- `crates/expressions/src/lib.rs` — 모듈 선언.
- `docs/task-reports/VIII.3.3.md`(신규), 계획서(VIII.3.3 `[ ]`→`[x]`).

## 구현 요약

- **plan 형식**: `EditPlan`은 원시 변경 `InsertNode`·`SetChild{socket, target}`·`RemoveNode`·`SetRoot`의 순서열입니다.
  `apply`는 기준 AST에 순서대로 적용한 뒤 `ExpressionAst::new` 구조 검증을 거칩니다.
- **추가**: `plan_add_block(ast, refs, Placement::{Socket, Root}, NewBlock, new_id)` — 입력 slot마다 기존 block 연결
  (자리를 내준 기존 block으로 감싸기 포함), 새 literal, 또는 타입 초기값 literal(`default_literal`)을 지정합니다.
  slot 누락·중복·불필요 slot은 `InvalidInputs`.
- **연결**: `plan_connect` — VIII.3.2 `can_connect`로 먼저 확인(cycle·타입·하류 오류 거부) 후 `SetChild`.
- **삭제**: `plan_delete_block(…, DeleteReplacement::{DefaultLiteral, PromoteInput(slot)})` — 삭제 block을 참조하는 모든
  socket(공유 포함)과 root 자리를 추론 타입 초기값 literal 또는 끌어올린 입력으로 대체합니다.
- **공통 보장**: 모든 생성 함수는 변경 후 root에서 닿지 않는 node를 `RemoveNode`로 명시하고, plan을 실제로 적용해
  구조 검증과 type 검사(새 오류 node 없음)를 통과한 경우에만 반환합니다. 새 ID는 호출자 생성기로 받아 결정적입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-expressions` | 종료 코드 0 |
| `cargo test -p stellar-expressions` | 종료 코드 0 (84 unit passed + 3 doctest passed, 0 failed) |
| `cargo clippy -p stellar-expressions --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-expressions -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **각 plan 적용 후 valid AST**: 기존 입력 감싸기 추가(`add_block_wrapping_existing_input_yields_valid_ast`), root 감싸기·초기값 입력 추가와 대체된 subtree 제거 op(`add_block_at_root_and_with_defaults_yields_valid_ast`), 연결·도달 불가 제거(`connect_plan_yields_valid_ast_and_prunes`), 초기값·끌어올림·공유 block·root 삭제(`delete_plans_yield_valid_ast`) — 적용 결과마다 type 오류 0, canonical round-trip 동일 | 통과 |
| 결정적 무작위 편집 300회 중 생성된 plan(100건 초과)을 연속 적용해도 매번 valid AST(`random_edit_sequences_keep_ast_valid`) | 통과 |
| 잘못된 요청 거부: 타입 불일치 추가, slot 누락·중복, 없는 socket, cycle·타입 불일치 연결, 타입이 다른 입력 끌어올림(`invalid_add_requests_are_rejected` 외) | 통과 |
| 같은 입력·같은 ID 생성기 → 같은 plan, 타입별 초기값 literal 타입 일치(`default_literals_match_their_type`) | 통과 |

## 차이(작업 지시와 다른 점)

- **완화한 선행**: 행 선행의 `Phase VII`(Command·트랜잭션)는 사용자 지시(2026-11-14)에 따라 phase 수준 선행으로
  완화했습니다. Phase VII command 타입이 아직 없으므로 "command plan"은 이 crate 안의 원시 AST 변경 목록(`EditPlan`)으로
  정의했고, Phase VII command(예: 표현식 교체 command)가 이 plan을 payload로 감싸는 연결은 이후 작업입니다.
- 새 block 빈 입력의 초기값(`default_literal`)은 GUI 편의용 값이며, orphan binding의 "타입별 안전값"(VIII.8.3)과는 별개입니다.

## 남은 위험 / 한계

- plan에는 역(inverse) 변경 목록이 없습니다. undo는 Phase VII command가 전후 AST 또는 역 plan을 보관하는 방식으로 정해야 합니다.
- plan 생성마다 전체 type check를 2회 수행합니다. 큰 표현식의 GUI 반응성은 이후 측정이 필요합니다.
