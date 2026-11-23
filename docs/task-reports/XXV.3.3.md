# 작업 보고서: XXV.3.3 remediation은 일반 CommandTransaction 또는 명시 navigation만

- WBS: XXV.3.3
- 작업명: remediation action은 일반 CommandTransaction 또는 명시 navigation만 실행, 임의 script 금지
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 XXV.2.2이 `[x]`가 아닙니다. 선행이 끝나면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]` 후보). 선행 XXV.2.2 판정을 따릅니다.
- 결정: ADR-0095 결정 5

## 필수 실행 환경

- `DEV-MAC`(Rust 단위 test).

## 변경 파일 목록

- `crates/integrity/src/remediation.rs`
  - action inventory 7개(형태 navigation·command).
  - `NavigationTarget`, `ProjectSection`.
  - closed enum `RemediationPlan { Navigate, Transaction }`, `plan`.
- `crates/integrity/src/tests.rs`: command remediation을 일반 `DocumentState::execute_transaction`으로 실행하고 undo로 복원.

## 구현 요약

- navigation 대상: 원인 node, 원인 속성, 프로젝트 설정 절(의존성·font·Target Profile·migration).
- command는 `remove-dangling-modifier` 하나입니다. source가 없는 Mask·Clip modifier를 `RemoveModifier` 요청 하나의
  transaction으로 지웁니다(출처 사용자, 기준 revision 지정). Boolean operand 누락에는 command를 주지 않습니다.
- 다음은 오류입니다: 진단이 제공하지 않은 action, 대상이 없는 action, 조건이 맞지 않는 action.
- OrphanedBinding 재연결·삭제는 binding 전용 command라 넣지 않았습니다(결정 5).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-integrity remediation` | 통과(4 passed) |

| 검사 | 결과 |
|---|---|
| action inventory: 모든 ID가 inventory에 한 번, 숫자·이름 고유, command 형태는 1개 | 통과(`remediation::tests::inventory_is_complete_and_each_action_has_one_form`) |
| 모든 종류 × 대표 위치에서 제공한 action의 계획이 inventory 형태와 같고, 모든 action이 어떤 진단에 쓰임 | 통과(`remediation::tests::every_offered_action_plans_to_navigation_or_command_transaction`) |
| Boolean operand 누락에는 command 없음, 제공하지 않은 action 거부 | 통과(`remediation::tests::boolean_operand_gets_no_command_and_unoffered_action_is_refused`) |
| command 계획을 일반 transaction 경로로 실행하면 진단이 사라지고, 역 transaction으로 다시 생김 | 통과(`tests::remediation_runs_as_ordinary_transaction_and_undo_restores_diagnostic`) |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- `stellar apply` 입력 형식에 `remove-modifier`가 없습니다. 그래서 CLI는 remediation을 실행하지 않고 action 이름·형태만
  보여 줍니다. apply 입력 확장은 ops command 입력(`mcp_command_input`) 소유 작업입니다.
