# 작업 보고서: VIII.4.1 bytecode 내부 포맷과 compiler

- WBS: VIII.4.1
- 작업명: 검증된 AST를 stack bytecode로 컴파일하는 내부 포맷 정의 — bytecode는 저장하지 않고 캐시로만 사용
- 상태: 완료 (`[x]`)
- 선행: VIII.2.4(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/expressions/src/bytecode.rs`(신규) — `Instr`, `CodeBlock`, `Program`(`compile`, table 접근자, `disassemble`), `CompileError`.
- `crates/expressions/src/vm.rs`(신규) — `Program::evaluate`, `evaluate_traced`, `EvalTrace`, 비교 fixture 테스트.
- `crates/expressions/src/ast_eval.rs`(신규) — AST 기준 evaluator `evaluate_ast`.
- `crates/expressions/src/eval.rs`(신규) — `ReferenceValues`, `EvalError`/`EvalErrorKind`, 공용 값 의미(`load_reference`, `select_if`, `match_arm`).
- `crates/expressions/src/lib.rs` — 모듈 선언.
- `docs/task-reports/VIII.4.1.md`(신규), 계획서(VIII.4.1 `[ ]`→`[x]`).

## 구현 요약

- **포맷**: `Program` = block 목록(0 = root) + constant pool(null 포함, 값 중복 제거) + reference table(대상, type 검사
  타입) + match pattern table. 명령은 `Const`, `LoadRef`, `Eval`(공유 block 호출·결과 캐시), `Select`(if),
  `MatchSelect`(match), `Return`이며 명령마다 원인 AST node를 기록합니다. 여러 부모가 공유하는 연산 node는 별도 block으로
  한 번만 컴파일합니다. 직렬화 형식·안정 번호가 없는 **저장하지 않는 캐시**이며 원본은 canonical AST입니다.
- **compiler**: `Program::compile(ast, types)`는 먼저 type 검사(VIII.2.4)를 통과해야 하고, 명시적 task stack으로 재귀 없이 컴파일합니다.
- **evaluator**: VM은 operand stack과 명시적 frame stack으로 실행하고, 공유 block은 첫 `Eval`에서 실행 후 캐시합니다.
  참조 값은 reference table 타입으로 검증합니다(`MissingReference`, `ReferenceTypeMismatch`). 오류는 원인 node StableId를 담습니다.
- **AST 기준 evaluator**: `evaluate_ast`는 AST를 직접(명시적 stack, 공유 node 1회) 평가하는 의미 기준이며 VM과 같은 공용 값 의미 함수를 씁니다.
- **단계 범위**: 이 작업은 Literal·Reference·If·Match를 컴파일·평가합니다. 단항·이항 연산 의미는 VIII.4.2, 함수 호출은
  VIII.4.4에서 추가하며 그 전까지는 `CompileError::UnsupportedNode` / `EvalErrorKind::UnsupportedNode`로 명시 거부합니다.
  if/match는 이 단계에서 모든 branch를 계산한 뒤 선택하며, short-circuit은 VIII.4.3에서 구현합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-expressions` | 종료 코드 0 |
| `cargo test -p stellar-expressions` | 종료 코드 0 (95 unit passed + 3 doctest passed, 0 failed) |
| `cargo clippy -p stellar-expressions --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-expressions -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **AST와 bytecode evaluator 결과 동일 fixture**(`ast_and_bytecode_results_match_on_fixture_corpus`): 공유 if·중첩 if·match·null·Optional 참조를 가진 11 node AST를 입력 조합 32개(enum 4 × env 2 × env 2 × null/값)로 평가해 두 evaluator 결과 동일, 서로 다른 결과 6종 이상 | 통과 |
| 선택 입력의 기대 값(공유 if 두 경로, null branch, 기본 branch)(`expected_values_for_selected_inputs`) | 통과 |
| 공유 node = block 1개, 두 arm이 `Eval` 호출, 실행 block 2개(두 번째 호출은 캐시), 명령별 원인 node, `Return` 종료(`shared_node_is_one_block_evaluated_once`) | 통과 |
| constant·reference 중복 제거와 결과 동일(`constants_and_references_are_deduplicated`) | 통과 |
| 누락·타입 불일치 참조 오류가 두 evaluator에서 같고 원인 node 정확(`errors_match_and_point_to_nodes`) | 통과 |
| type 오류 AST 컴파일 거부, 미지원 node 명시 거부(`compile_rejects_ill_typed_and_unsupported_nodes`) | 통과 |

## 차이(작업 지시와 다른 점)

- 연산·함수 의미를 이후 행(VIII.4.2·VIII.4.4)에 맞춰 단계적으로 추가하므로, 이 작업의 비교 fixture는 Literal·Reference·If·Match로
  구성했습니다. 미지원 node는 typed 오류로 거부하며 이후 행에서 제거합니다.

## 남은 위험 / 한계

- instruction budget과 call depth 제한은 VIII.4.5에서 추가합니다. 현재는 깊은 공유 block 중첩에 frame 수 제한이 없습니다.
- program 캐시의 무효화 정책(토큰·binding 변경 시 재컴파일)은 VIII.5.4·VIII.6.4 소관입니다.
