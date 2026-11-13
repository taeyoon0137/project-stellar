# 작업 보고서: VIII.4.5 instruction budget과 call depth limit

- WBS: VIII.4.5
- 작업명: evaluator에 instruction budget과 recursion 없는 call depth limit 적용
- 상태: 완료 (`[x]`)
- 선행: VIII.4.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/expressions/src/vm.rs` — `EvalLimits`(`DEFAULT`, `Default`), `Program::evaluate_with_limits`, 한도 검사, 한도 fixture 테스트.
- `crates/expressions/src/eval.rs` — `EvalErrorKind::{InstructionBudgetExceeded, CallDepthExceeded, StackLimitExceeded}`.
- `docs/task-reports/VIII.4.5.md`(신규), 계획서(VIII.4.5 `[ ]`→`[x]`).

## 구현 요약

- **한도**: `EvalLimits { instruction_budget, max_call_depth, max_stack_depth }`, 기본값 명령 1,000,000개·frame 256개·operand
  65,536개. `evaluate`/`evaluate_traced`는 기본 한도를, `evaluate_with_limits`는 지정 한도를 씁니다.
- **instruction budget**: 명령을 실행하기 전에 실행 수가 budget에 도달했으면 `InstructionBudgetExceeded`로 즉시 종료합니다
  (budget = 명령 수면 성공, 1 모자라면 실패).
- **recursion 없는 call depth**: 공유 block 호출은 Rust 재귀가 아니라 명시적 frame stack push이며, frame 수가 한도에 도달한
  상태에서 새 block을 열려 하면 `CallDepthExceeded`입니다.
- **operand stack**: 깊게 오른쪽으로 중첩된 식처럼 값이 쌓이는 경우 `StackLimitExceeded`로 종료합니다.
- 모든 한도 오류는 중단 지점의 원인 node를 기록합니다. 컴파일·type 검사·구조 검증도 이미 비재귀라 큰 AST에서 panic·stack
  overflow 없이 제한 오류로 끝납니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-expressions` | 종료 코드 0 |
| `cargo test -p stellar-expressions` | 종료 코드 0 (116 unit passed + 3 doctest passed, 0 failed) |
| `cargo clippy -p stellar-expressions --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-expressions -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **비정상적으로 큰 AST가 제한 오류로 종료**(`huge_ast_stops_at_instruction_budget`): 30만 node(명령 300,001개) program이 budget 10만에서 `InstructionBudgetExceeded`(node 기록), budget 300,001이면 정상, 300,000이면 실패(경계 정확) | 통과 |
| 기본 한도에서 100만+1 node program이 `InstructionBudgetExceeded { budget: 1_000_000 }`(`default_budget_stops_abnormally_large_program`) | 통과 |
| **call depth**(`deeply_nested_shared_blocks_stop_at_call_depth`): 공유 block 300단 중첩이 `CallDepthExceeded { limit: 256 }`, 100단은 정상(공유 block 각 1회 실행, 결과 2^100), 한도 50으로 낮추면 거부 | 통과 |
| operand stack(`right_nested_expression_stops_at_stack_limit`): 7만 단 오른쪽 중첩 덧셈이 `StackLimitExceeded { limit: 65_536 }`, 한도 상향 시 70,001 | 통과 |
| 일반 크기 식은 기본 한도 안(`ordinary_expressions_fit_default_limits`) | 통과 |

## 차이(작업 지시와 다른 점)

- 행은 instruction budget과 call depth를 요구합니다. operand stack 깊이 한도를 함께 두었습니다. budget보다 작은 program도 값이
  깊게 쌓이면 메모리를 크게 쓰기 때문입니다.
- 기본 한도 값은 이 crate 상수입니다. `config/performance-budgets.yaml`로 옮기는 것은 이 lane의 수정 허용 범위 밖이라 후속 결정입니다.

## 남은 위험 / 한계

- AST 기준 evaluator(`evaluate_ast`)는 검증용 기준 구현이라 한도를 적용하지 않습니다(비재귀, 메모리는 AST 크기에 비례). 런타임
  평가는 VM만 사용해야 합니다.
- 기본 한도 값의 성능 근거(10만 node 표현식 평가 시간 등)는 측정하지 않았습니다. benchmark(VIII.9.2)에서 확인이 필요합니다.
