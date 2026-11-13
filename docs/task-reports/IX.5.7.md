# 작업 보고서: IX.5.7 상태 머신 진단

- WBS: IX.5.7
- 작업명: state machine의 unreachable transition, duplicate condition, missing target 진단
- 상태: 완료 (`[x]`)
- 선행: IX.5.3(이 lane에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/state_diagnostics.rs`(신규) — `diagnose_state_machine`, `StateDiagnostic`,
  `MissingTarget`.
- `crates/document/src/component/typed_expr.rs` — `CheckedExpression::structural_bytes`(node id를 무시한 구조
  비교)와 단위 테스트.
- `crates/document/src/component/mod.rs` — 모듈 등록.
- `docs/task-reports/IX.5.7.md`(신규), 계획서(IX.5.7 `[ ]`→`[x]`).

## 구현 요약

- 저장을 막지 않는 작성 단계 진단입니다. 결과는 transition 목록 순서 → 진단 종류 순서라 결정적입니다.
- **unreachable**: 같은 trigger의 앞 transition이 항상 참(조건 없음, 또는 참조 없는 조건이 `true`)이면
  `Unreachable { shadowed_by: Some(앞 transition) }`. 참조 없는 조건이 `false`면
  `Unreachable { shadowed_by: None }`. dispatch(IX.5.5)가 처음 참인 transition 하나만 실행하기 때문입니다.
- **duplicate condition**: 같은 trigger의 앞 transition과 조건이 같으면 `DuplicateCondition`. 같은 식을 GUI가 다른
  node id로 만들 수 있으므로 node id를 canonical 순서 위치로 바꾼 구조 bytes(`structural_bytes`)로 비교합니다.
- **missing target**: source node가 정의 tree에 없음(`SourceNode`), nested instance node 없음(`NestedInstance`),
  nested instance의 컴포넌트가 library에 없음(`NestedComponent`, 미해결 참조), nested 컴포넌트가 action을
  선언하지 않음(`NestedAction`).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 284 passed, 통합 7+1+1+2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| transition 9개(정상·중복 조건·무조건·가려진·상수 거짓·없는 source·미선언 nested action·없는 nested instance·정상 chain) → 진단 6건 순서·내용 정확, 진단 있는 library도 round-trip, 정상 머신·없는 컴포넌트는 0건(`diagnostic_fixture_is_exact`) | 통과 |
| nested instance가 library에 없는 컴포넌트 참조 → `NestedComponent`(`unresolved_nested_component_is_a_missing_target`) | 통과 |
| 같은 식을 다른 node id로 만든 두 표현식의 구조 bytes 동일, 값이 다르면 다름(`structural_bytes_ignore_node_ids`) | 통과 |

## 차이(작업 지시와 다른 점)

- 상태 머신 내부 대상(`SetState` 변수, `Emit` action)은 IX.5.3이 저장 단계에서 거부하므로 missing target 진단은
  정의 tree·다른 컴포넌트에 걸친 대상만 다룹니다.

## 남은 위험 / 한계

- 조건식의 논리적 동치(예: `a && b`와 `b && a`)나 서로 배타적이지 않은 조건 조합 분석은 하지 않습니다. 구조가
  같은 조건과 상수 조건만 판정합니다.
