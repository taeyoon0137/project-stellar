# 작업 보고서: VIII.4.2 literal·산술·비교·논리 evaluator

- WBS: VIII.4.2
- 작업명: literal·산술·비교·논리 evaluator 구현 — division by zero는 typed evaluation error
- 상태: 완료 (`[x]`)
- 선행: VIII.4.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/expressions/src/eval.rs` — `apply_unary`, `apply_binary`, `make_numeric`, `EvalErrorKind::{DivisionByZero, NonFiniteResult}`.
- `crates/expressions/src/bytecode.rs` — `Instr::{Unary, Binary}`, Unary/Binary node 컴파일.
- `crates/expressions/src/vm.rs` — Unary/Binary 실행, table-driven 테스트.
- `crates/expressions/src/ast_eval.rs` — Unary/Binary 평가.
- `docs/task-reports/VIII.4.2.md`(신규), 계획서(VIII.4.2 `[ ]`→`[x]`).

## 구현 요약

- **산술**: 수치·단위 값을 f32 IEEE-754 기본 연산(`+ − × ÷`)과 정확 연산인 나머지(`%`, fmod)·부호 반전으로 계산하고,
  결과 타입은 산술 규칙 표(VIII.1.2)의 등록 규칙으로 정합니다(예: `Length ÷ Length → Number`).
- **typed 오류**: `÷`·`%`의 오른쪽이 0이면 `DivisionByZero`, 결과가 비유한(overflow)이면 `NonFiniteResult`입니다.
  core 단위 타입의 saturate 덧셈과 달리 표현식 평가는 무한대·saturate 값을 만들지 않습니다. `-0.0`은 `+0.0`으로 정규화됩니다.
- **비교**: `==`/`!=`는 `Option<TypedValue>` 동등성(null == null, 색공간 포함 비교), 순서 비교는 같은 타입 f32 비교.
- **논리**: `and`/`or`/`not`은 Boolean. 이 단계의 `and`/`or`는 양쪽을 모두 계산하며 short-circuit은 VIII.4.3에서 구현합니다.
- 두 evaluator(VM, AST 기준)가 같은 `apply_*` 함수를 쓰고 오류는 연산 node StableId를 가리킵니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-expressions` | 종료 코드 0 |
| `cargo test -p stellar-expressions` | 종료 코드 0 (99 unit passed + 3 doctest passed, 0 failed) |
| `cargo clippy -p stellar-expressions --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-expressions -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **산술 table**(`arithmetic_table`): 33개 case — 5개 단위 계열의 등록 연산 값, 음수 나머지(`-7 % 3 = -1`), `-0.0` 정규화, **0 나누기·0 나머지 6건 `DivisionByZero`**, overflow 4건 `NonFiniteResult`; 의미 함수·VM·AST evaluator 모두 같은 결과, 오류 node = 연산 node | 통과 |
| **비교 table**(`comparison_table`): 순서 비교 5, 동등 비교(`0 == -0`, 색공간이 다른 색, 문자열), nullable(`null == null`, `null == 값`, `값 != null`) 14건 | 통과 |
| **논리·단항 table**(`logic_and_unary_tables`): and/or 진리표 8, not·neg 8건(단위별, `-(-0) = 0`, `-MAX`) | 통과 |
| 공유 참조가 있는 중첩 산술의 입력 6종 VM=AST, 0 나누기 입력에서 `DivisionByZero`(`nested_arithmetic_with_references_matches_ast`) | 통과 |
| VIII.4.1 비교 fixture 전부 계속 통과 | 통과 |

## 차이(작업 지시와 다른 점)

- VIII.4.1의 "미지원 node 거부" 테스트가 Binary node를 예로 썼는데, 이 작업에서 Binary를 지원하므로 같은 검사를 아직
  미지원인 Call node(VIII.4.4에서 지원)로 바꿨습니다. 검사 의도(미지원 node의 typed 거부)는 유지됩니다.

## 남은 위험 / 한계

- 나머지(`%`)는 Rust `f32 %`(fmod)를 씁니다. fmod는 IEEE-754 정의상 정확 결과라 구현과 무관하게 같은 값이어야 하지만,
  교차 플랫폼 bytes 확인은 결정론 행(VIII.4.6)에서 다룹니다.
