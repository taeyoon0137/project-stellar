# 작업 보고서: III.5.1 OrderKey 요구사항 fixture(red)

- WBS: III.5.1
- 작업명: `OrderKey` 요구사항 문서 + 미구현 fixture(현재 실패, red checkpoint)
- 상태: 완료 (`[x]`)
- 선행: III.1.1(`StableId`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `docs/architecture/order-key-requirements.md` (신규: 요구사항 R1~R5)
- `crates/core-types/src/order_key.rs` (신규: `OrderKey`/`OrderKeyError` 미구현 stub + 요구사항 fixture)
- `crates/core-types/src/lib.rs` (`pub mod order_key;`)
- `docs/task-reports/III.5.1.md`(신규), 계획서(III.5.1 `[ ]`→`[x]`)

## 구현 요약

- 요구사항 문서에 R1~R5를 정의: 비교 가능한 byte sequence(사전식 = 시각 순서, 정규형은
  비어 있지 않고 `0x00` 미종료), 앞·뒤·사이 삽입, 서로 다른 브랜치 동시 삽입 + 동일 키의
  `StableId` tie-break, 결정론(난수 미사용), 키 길이 상한과 재균형 신호.
- `order_key.rs`에 `OrderKey`/`OrderKeyError` 타입과 API 시그니처(`between`, `from_bytes`,
  `as_bytes`, `byte_len`, `needs_rebalance`, `REBALANCE_THRESHOLD`)를 두되, 생성 API
  (`between`/`from_bytes`)는 `unimplemented!`입니다. **알고리즘은 구현하지 않았습니다.**
- 요구사항을 인코딩한 7개 fixture는 생성 API 호출에서 패닉하여 **현재 실패(red)**합니다.
  III.5.2 구현이 green으로 전환합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo fmt -p stellar-core-types -- --check` | 0 |
| `cargo clippy -p stellar-core-types --all-targets -- -D warnings` | 0 |
| `cargo test -p stellar-core-types` | 비0(의도된 실패) |

테스트 결과: `93 passed; 7 failed`. 실패한 7개는 모두 `order_key::tests::*`로, 생성 API
미구현 패닉입니다(요구사항 fixture가 현재 실패 — 완료 기준 충족). 다른 93개는 통과.

| fixture | 요구사항 | 현재 상태 |
|---|---|---|
| `insert_front_back_between` | R2 | red(미구현) |
| `concurrent_branch_inserts_independent` | R3 | red |
| `equal_key_tie_break_by_stable_id` | R3 | red |
| `between_is_deterministic` | R4 | red |
| `order_matches_byte_lexicographic` | R1 | red |
| `from_bytes_rejects_non_canonical` | R1 | red |
| `key_length_cap_signals_rebalance` | R5 | red |

## 차이(작업 지시와 다른 점)

- 지시대로 알고리즘은 구현하지 않았습니다. fixture가 컴파일되도록 타입·시그니처는 두되 생성
  API는 `unimplemented!`로 남겨 패닉(실패)하게 했습니다. 이는 "fixture가 현재 미구현으로
  실패"라는 완료 기준의 직접 구현입니다.
- CI(`bootstrap.yml`)는 `cargo test -p xtask`만 실행하고 core-types 테스트는 실행하지 않으므로,
  이 red 상태가 CI 게이트를 깨지 않습니다. III.5.2에서 즉시 green으로 전환합니다.

## 남은 위험 / 한계

- core-types 단위 테스트 스위트가 III.5.2 구현 전까지 red입니다(의도된 checkpoint). 후속
  III.5.2가 알고리즘을 구현해 7개 fixture를 통과시키면 스위트가 green으로 복귀합니다.
- 구체 알고리즘과 근거는 III.5.2의 ADR에서 확정합니다.
