# 작업 보고서: XXII.1.5 상태 머신과 `MotionSpec`을 CodegenIR에 포함

## WBS

- WBS: XXII.1.5
- 작업명: 선언형 상태 머신과 `MotionSpec`을 CodegenIR에 포함, timing function과 reduced motion 대체 동작을 그대로 보존
- 상태: 완료 (`[x]`) — 선행 XXII.1.1이 차단 상태라 원장 표기는 통합 때 정합니다.
- 결정: ADR-0106 결정 1.2·1.3, ADR-0094 결정 1, ADR-0025 결정 4.1
- 선행: XXII.1.1, XXX.1.1, IX.5.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust 로컬 검증).

## 변경 파일

- `crates/codegen/src/ir/behavior.rs`(신규): `IrExpr`(닫힌 표현식 트리), `IrStateMachine`·`IrTransition`·`IrTrigger`·
  `IrAction`, `IrMotion`·`IrTiming`·`IrStepPosition`·`IrReducedMotion`·`IrMotionStart`.
- `crates/codegen/src/ir/behavior_build.rs`(신규): 참조 무결성 검사와 결정론 정렬.
- `crates/codegen/src/ir/{mod,build,snapshot,tests}.rs`, `crates/codegen/src/fixture.rs`.
- `crates/codegen/tests/fixtures/ir_{site,ios,android}.snapshot`: `machines`·`motions` 절.
- `crates/ops/src/codegen.rs`: 문서 `MotionSpec` → `IrMotion` 변환 `motion`과 test.

## 구현 요약

- `CodegenSources::machines`·`motions`를 받아 `IrTree::machines`(소유자 오름차순)·`motions`(id 오름차순)에 담습니다.
- timing function parameter·지속 시간·지연은 f32 bit 그대로, reduced motion은 `instant`·`play` 그대로 둡니다.
  snapshot은 f32 최단 왕복 표기라 값이 그대로 드러납니다.
- 검사: 같은 소유자 머신 중복, transition id 중복, 선언되지 않은 상태 변수(머신 변수·set 대상·식 참조), 선언되지
  않은 action emit, View 밖 source, 선언되지 않은 토큰, 표현식 깊이 상한(128), motion id 중복, View 밖 대상,
  선언되지 않은 시작 조건 변수.
- profile에서 제외한 node가 source인 transition과 제외 node 대상은 버리고, 대상이 모두 빠진 motion도 버립니다.
- 문서 `MotionSpec`은 ops `motion`이 그대로 옮깁니다. 상태 머신 조건식·값 식(`CheckedExpression`)을 `IrExpr`로 낮추는
  일은 expressions 타입을 이름으로 써야 해서 ops에서 할 수 없습니다(crate 그래프). IR 입력 타입은 준비했고, 문서 →
  IR 연결(XXII.1.1 재개)이 expressions에 닿는 위치에서 채워야 합니다.

## 실행한 명령 / 검증

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-codegen` | 통과(64 + 2) |
| `cargo test -p stellar-ops --lib codegen` | 통과(4) |
| `cargo clippy -p stellar-codegen -p stellar-ops --all-targets -- -D warnings` | 경고 0 |

검증 기준 대응:

- semantic snapshot fixture: `ir::tests::semantic_snapshot_fixtures_per_profile`(machines·motions 절 포함),
  `state_machine_and_motion_are_kept_in_ir_with_exact_parameters`.
- 같은 입력 IR bytes 동일: `state_machine_and_motion_ir_bytes_are_identical_for_the_same_input`(입력 순서를 바꿔도 같음).
- 입력 오류: `behavior_reference_errors_are_rejected`(11 case).
- 문서 값 보존: `ops::codegen::tests::motion_spec_values_are_copied_bit_for_bit`.

## 차이(작업 지시와 다른 점)

- 문서 상태 머신 → IR 변환은 만들지 않았습니다(ops가 expressions 타입에 닿지 못함, ADR-0106 결정 1.2).

## 남은 위험

- 문서 상태 머신 → `IrStateMachine` 변환은 없습니다(위 이유). fixture 입력으로만 검증했습니다.
