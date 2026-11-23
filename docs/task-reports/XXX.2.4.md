# 작업 보고서: XXX.2.4 포트 공개

- WBS: XXX.2.4
- 작업명: 공개 ComponentProperty·Event·ContextRequirement 생성 plan과 원자 적용, nested 직접 edge 거부.
- 상태: 부분 완료 (`[~]`) — 속성 입력 공개(공개 Property + binding 교체)와 이벤트 공개(semantic action + emit transition), nested 내부 직접 edge 거부, fault injection 원자성은 끝났습니다. ContextRequirement 공개는 XXX.7.8 이후입니다.
- 결정: ADR-0094 결정 4.4·4.5
- 선행: XXX.2.3, IX.4.5, XXX.7.8

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/prototype/src/expose.rs`(신규).
- `crates/prototype/src/lib.rs`: module 등록.
- `crates/prototype/tests/graph_expose.rs`(신규).

## 구현 요약

- `plan_expose`가 미리보기 목록과 transaction 하나를 만들고, 적용은 `PrototypeTransaction`이 원자적으로 합니다.
- 다른 컴포넌트 정의 안의 포트는 `NestedInternalPort`로 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-prototype` | 통과(lib 13, 통합 test 11개 파일 24개). |
| `cargo clippy -p stellar-document -p stellar-prototype --all-targets -- -D warnings` | 통과. |

시험:

- `graph_expose.rs::expose_port_is_atomic_and_rejects_nested_internal_edges`: 둘째 step 실패 주입 시 문서 bytes 불변·생성된 Property 0, 정상 적용과 Undo, 이벤트 공개, nested 직접 edge·공개 거부, Context 요구는 Unsupported.

## 차이(작업 지시와 다른 점)

- ContextRequirement는 원본이 없어 Unsupported로 둡니다.

## 남은 위험 / 한계

- ContextRequirement(XXX.7.8)가 생기면 공개 plan에 더해야 합니다.

## 후속 보완 (2026-11-24, Context 제품 경로 연결)

- 결정: ADR-0114 결정 7.
- 포트 공개에 ContextRequirement를 더했습니다. `PrototypeDocument`가 문서 `ContextSet`을 들고, step
  `ContextRequirement { component, key, add }`(역은 반대 연산)를 둡니다. `ExposeRequest::ContextRequirement { component, key }`는
  미리보기 항목 하나와 transaction 하나입니다. 이미 선언한 key는 `InUse`, 문서에 없는 key는 `Missing`입니다.
- 시험: `graph_expose::exposing_a_context_port_declares_the_requirement_atomically`(fault injection 시 생성 0, Undo 1회 복원,
  중복·없는 key 거부).
- 남은 위험: PrototypeTransaction을 Phase VII `CommandTransaction` journal에 등록하는 일은 그대로 영속 command registry 몫입니다.
