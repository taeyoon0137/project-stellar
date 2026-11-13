# 작업 보고서: VIII.4.3 if·match short-circuit 평가

- WBS: VIII.4.3
- 작업명: if·match short-circuit 평가 구현 — 선택되지 않은 branch 오류가 실행되지 않게
- 상태: 완료 (`[x]`)
- 선행: VIII.4.2(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/expressions/src/bytecode.rs` — `Instr::{JumpIfFalse, Jump, MatchCase, Pop}`(eager `Select`/`MatchSelect` 제거), label 기반
  컴파일(if·match·and·or), match pattern을 constant pool로 통합(pattern table 제거), 모듈 문서의 컴파일 형태.
- `crates/expressions/src/vm.rs` — jump·match case 실행, short-circuit fixture 테스트, VIII.4.1 테스트 2곳 조정.
- `crates/expressions/src/ast_eval.rs` — 지연(lazy) 작업 stack 기반으로 재작성.
- `crates/expressions/src/eval.rs` — `truth`, `short_circuit`(`select_if` 대체).
- `docs/task-reports/VIII.4.3.md`(신규), 계획서(VIII.4.3 `[ ]`→`[x]`).

## 구현 요약

- **bytecode**: `if c then a else b` → `c; JumpIfFalse(L1); a; Jump(L2); L1: b; L2:`. `match`는 arm마다
  `MatchCase{pattern, miss}`(대상과 같으면 대상을 pop하고 arm 실행, 다르면 다음 case로 이동), 일치 arm이 없으면 `Pop` 후
  기본 branch. `and`/`or`도 같은 jump로 short-circuit(`false and …` → false, `true or …` → true)합니다. label은 block 끝에서
  절대 명령 위치로 해석합니다.
- **효과**: 선택되지 않은 branch의 연산·참조·공유 block(`Eval`)은 실행되지 않아 그 안의 0 나누기·누락 참조 오류가 나지 않습니다.
  선택된 branch의 오류는 원인 node StableId와 함께 그대로 보고합니다.
- **AST 기준 evaluator**: 조건·대상·왼쪽 값을 먼저 평가한 뒤 선택된 child만 평가하는 작업 stack 방식으로 바꿔 VM과 같은
  지연 의미를 갖습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-expressions` | 종료 코드 0 |
| `cargo test -p stellar-expressions` | 종료 코드 0 (103 unit passed + 3 doctest passed, 0 failed) |
| `cargo clippy -p stellar-expressions --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-expressions -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **if short-circuit**(`if_skips_unselected_branch_errors`): `if true then 7 else 1/0` → 7(실행 명령 수 < 전체), `if false then 누락참조 else 8` → 8; 선택된 `1/0`은 `DivisionByZero`(node 정확) | 통과 |
| **match short-circuit**(`match_skips_unselected_arm_and_default_errors`): 오류 arm·오류 기본 branch가 있어도 arm 1 선택 시 값, 오류 arm 선택 시 그 오류, 일치 없음 시 기본 branch 평가, 모든 arm이 오류여도 안전한 기본 branch 선택 시 값 | 통과 |
| **and/or short-circuit**(`and_or_short_circuit`): `false and (1/0 == 1)` → false, `true or (누락 == 1)` → true, 반대 입력은 오른쪽 오류 보고, 진리표 8건 | 통과 |
| 미선택 branch의 공유 `1/0` block 미실행(`block_runs` = 1), 선택 시 오류(`shared_block_in_unselected_branch_is_not_run`) | 통과 |
| 모든 fixture에서 VM 결과 = AST 기준 evaluator 결과(테스트 helper가 매번 비교) | 통과 |

## 차이(작업 지시와 다른 점)

- `and`/`or`도 short-circuit으로 바꿨습니다. 행은 if·match만 명시하지만 아키텍처의 "선택되지 않은 계산의 오류 비실행"
  원칙을 논리식에도 같게 적용하기 위해서입니다.
- VIII.4.1 테스트 중 eager 평가를 전제한 두 단언을 조정했습니다: 실행 명령 수 = 전체 명령 수 → 전체보다 적음, 누락 참조
  오류 검사는 그 참조가 있는 branch가 선택되도록 입력(env1 = true)을 지정. 두 evaluator 결과 동일·오류 node 정확이라는 검사
  의도는 유지됩니다.

## 남은 위험 / 한계

- 함수 호출은 VIII.4.4 전까지 미지원(typed 거부)입니다. 함수 인자는 short-circuit 대상이 아니며 모두 평가합니다.
