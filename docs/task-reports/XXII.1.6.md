# 작업 보고서: XXII.1.6 Context 선언·provision·소비를 CodegenIR에 포함

## WBS

- WBS: XXII.1.6
- 작업명: Context 선언·provision·소비를 CodegenIR에 포함, 소비 지점마다 해석기가 정한 값의 출처와 slot 경계를 보존
- 상태: 부분 완료 (`[~]`) — XXX.7.4 해석기가 없어 해석기 대조 기준이 남았습니다.
- 결정: ADR-0106 결정 1.4, ADR-0025 결정 3·4.1
- 선행: XXII.1.1, XXX.7.4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust 로컬 검증).

## 변경 파일

- `crates/codegen/src/ir/context.rs`(신규): `IrContexts`, `IrContextKey`, `IrContextProvision`(펼친 트리 위치별 깊이
  집합), `IrContextConsumption`(출처 `IrContextSource`, 건넌 `IrSlotCrossing`), `IrSlotHost`(공개 key 목록), 조회 함수.
- `crates/codegen/src/ir/behavior_build.rs`: Context 참조 무결성 검사와 정렬, 표현식 상태 오류 생성기 분리.
- `crates/codegen/src/ir/{mod,build,snapshot,tests}.rs`, `crates/codegen/src/fixture.rs`.
- `crates/codegen/tests/fixtures/ir_{site,ios,android}.snapshot`: `contexts` 절.

## 구현 요약

- `CodegenSources::contexts`(해석 결과)를 받아 `IrTree::contexts`에 정렬해 담습니다. key는 id, provision은 (node, key),
  소비는 (node, path, key), slot host는 host 순입니다.
- IR은 소비 지점의 출처를 다시 계산하지 않습니다. `IrContexts::consumption_of`는 입력 결과를 그대로 돌려줍니다.
- 검사: key 중복·빈 이름, 선언되지 않은 key(provision·식·소비·Slot 공개 목록), 선언되지 않은 상태 읽기, View 밖 node,
  같은 node·key provision 중복, 비었거나 0을 담은 깊이 집합, IR에 없는 출처 provision, 같은 경로 소비 중복.
- profile에서 제외한 node의 provision·소비·slot host는 버립니다.

## 실행한 명령 / 검증

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-codegen` | 통과(67 + 2) |
| `cargo clippy -p stellar-codegen --all-targets -- -D warnings` | 경고 0 |

시험:

- `ir::tests::context_resolution_is_kept_per_consumption_without_reinterpretation`: 중첩(1 → 2), shadowing(6이 1을 가림
  → 8), 같은 key 재읽기(7), default 소비(12), slot 경계(17 ← 16)가 입력 그대로 남음.
- `context_ir_bytes_are_identical_for_the_same_input`: 입력 순서와 무관하게 같은 snapshot.
- `context_reference_errors_are_rejected`: 11 case.

## 차이(작업 지시와 다른 점)

- 해석기 결과 자리에 손으로 적은 합성 입력을 썼습니다.

## 남은 위험

- XXX.7.4 해석기가 생기면 같은 fixture를 해석기 결과로 만들고 IR과 대조하는 test를 더해야 합니다.
- IR View tree는 instance 내부를 펼치지 않습니다. slot host가 컴포넌트 내부 node일 때 그 node를 IR에 넣는 방법은
  instance 펼침 입력(XXII.1.1 재개)과 함께 정해야 합니다.
