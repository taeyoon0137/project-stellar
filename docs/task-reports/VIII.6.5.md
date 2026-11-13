# 작업 보고서: VIII.6.5 평가 값 provenance chain

- WBS: VIII.6.5
- 작업명: evaluated value provenance chain에 token, expression node, component property, environment branch 기록
- 상태: 완료 (`[x]`)
- 선행: VIII.6.4(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/expressions/src/ast_eval.rs` — `evaluate_ast_traced`, `EvaluationPath`, `ReferenceRead`, `BranchDecision`
  (평가 중 실제로 읽은 참조와 선택한 branch 기록, 실패해도 실패 지점까지의 경로 반환)와 테스트 1개.
- `crates/bindings/src/provenance.rs`(신규) — `ProvenanceStep`, `ProvenanceChain`, `BindingEvaluator::provenance`와 테스트.
- `crates/bindings/src/lib.rs` — `provenance` 모듈 등록.
- `docs/task-reports/VIII.6.5.md`(신규), 계획서(VIII.6.5 `[ ]`→`[x]`).

## 구현 요약

- **평가 경로 기록**(expressions): AST 기준 evaluator(VIII.4.1, 한도 VIII.4.5 보완)를 그대로 쓰며 참조 node를 평가할 때
  (참조 node StableId, 대상)을, `if`(Then/Else)·`match`(Arm/Default)·`and`/`or`(왼쪽에서 결정 `Lhs`/오른쪽 평가 `Rhs`)
  결정을 순서대로 기록합니다. short-circuit으로 평가하지 않은 branch의 참조는 기록되지 않습니다. 기록 없는 기존 경로
  (`evaluate_ast_with_limits`)는 동작이 같습니다.
- **chain**(bindings): 요청 node에서 너비 우선으로 producer 표현식을 현재 값으로 다시 평가해 실제로 읽은 node를 따라가며
  각 node를 한 번씩 기록합니다. 단계마다 node, producer(토큰·binding·없음), 현재 값, 읽은 (표현식 node, node) 목록,
  선택 branch를 둡니다. source(환경 입력·Component Property·binding 없는 속성)가 원본입니다. 요청 시에만 계산합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-bindings -p stellar-expressions` | 종료 코드 0 |
| `cargo test -p stellar-expressions` | 첫 실행 1 실패(새 테스트 fixture가 `or`에 Number 참조를 써 type 오류) → fixture를 Boolean 참조로 수정 후 종료 코드 0 (123 unit + 9 integration + 3 doctest passed) |
| `cargo test -p stellar-bindings` | 종료 코드 0 (34 unit passed) |
| `cargo clippy -p stellar-bindings -p stellar-expressions --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-bindings -p stellar-expressions -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **최종값→원본 chain fixture**(`final_value_chain_reaches_token_branch_and_environment`): `p1 ← t3`, `t3 = if dark then t1 else t2`, dark=true에서 chain `[p1, t3, env dark, t1]`(t2 없음), p1 producer=binding·값·읽기 `(x40, t3)`, t3 읽기 `(x31, dark)`,`(x32, t1)`·branch `(x30, Then)`, 원본 = dark, t1은 상수 | 통과 |
| Component Property·branch 변화(`chain_follows_branch_change_and_component_property`): `p2 ← with_alpha(t3, cp)`, dark=false에서 `[p2, t3, cp, dark, t2]`, branch Else, 원본 `[cp, dark]`, 반복 동일, 환경 변경 후 chain이 t1로 바뀜 | 통과 |
| 평가 경로 기록(`traced_evaluation_records_reads_and_branches`): 읽기 3개(Else·`or` 오른쪽 참조 없음), branch `[if Then, and Rhs, or Lhs]`, 값은 기록 없는 평가와 동일 | 통과 |

## 차이(작업 지시와 다른 점)

- provenance는 저장하지 않고 요청 시 현재 값으로 다시 계산합니다(값 저장 경로에 비용 없음). 다시 평가는 AST 기준
  evaluator를 써서 bytecode 결과와 같은 값 의미(VIII.4.1)를 따릅니다.
- 신규 모듈이므로 bindings 테스트는 구현과 함께 작성했습니다.

## 남은 위험 / 한계

- Component Property의 출처(어느 instance override·상위 Property Drilling에서 왔는지)는 Phase IX 값 공급자가 없어
  source 단계로만 기록합니다.
- provenance 계산 비용은 chain 크기에 비례하며 각 producer 표현식을 한 번 더 평가합니다.
