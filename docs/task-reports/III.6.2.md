# 작업 보고서: III.6.2 SoA NodeColumns

- WBS: III.6.2
- 작업명: 공통 node 필드를 SoA column으로 저장하는 최소 `NodeColumns`
- 상태: 완료 (`[x]`)
- 선행: III.6.1(arena·`live_slot`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/node_columns.rs` (신규: `NodeColumns`, `NodeFields`, invariant test)
- `crates/document/src/lib.rs` (`pub mod node_columns;`)
- `docs/task-reports/III.6.2.md`(신규), 계획서(III.6.2 `[ ]`→`[x]`)

## 구현 요약

- `NodeColumns`: 모든 node 공통 필드를 **SoA column**(필드별 별도 `Vec`)으로 저장합니다.
  최소 공통 필드는 `stable_id`(StableId), `parent`(Option\<RuntimeEntityId>),
  `order_key`(OrderKey)입니다. 종류별 payload는 없습니다(Phase VI).
- 핸들 수명은 `RuntimeEntityAllocator`, 인덱싱은 `live_slot`을 사용합니다(III.6.1과 동일 패턴).
- 길이 불변식: 새 slot은 모든 column에 함께 push, 재사용 slot은 모든 column을 덮어쓰며, 제거는
  길이를 줄이지 않습니다. 따라서 **모든 column 길이가 항상 일치**합니다.
- API: `insert`/`get`(스냅샷)/`set_parent`/`set_order_key`/`contains`/`remove`/`len`/`is_empty`.
  stale 핸들 접근·갱신은 실패합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-document` (8 unit) | 0 |
| `cargo fmt -p stellar-document -- --check` | 0 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| insert/get/remove round-trip, 중복 제거 실패 | 통과 |
| setter 갱신 + stale 핸들 거부 | 통과 |
| **1만 회 임의 삽입·삭제 후 매 연산마다 모든 column 길이 일치** | 통과 |
| slot 재사용 후 column 정렬 유지 + 옛 핸들 무효 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 공통 필드는 현재 정의된 core 타입(StableId·OrderKey)과 tree 부모 핸들로 최소 구성했고,
  종류별 payload는 의도대로 포함하지 않았습니다.

## 남은 위험 / 한계

- `NodeColumns`와 III.6.1의 generic `Arena<T>`는 둘 다 `RuntimeEntityAllocator`+`live_slot`
  패턴을 쓰지만 별도 구조입니다(`Arena<T>`는 재사용 가능한 AoS 원시 타입, `NodeColumns`는
  node 전용 SoA 저장소). 노드 저장은 `NodeColumns`를 사용합니다.
- 외부 crate에 mutable 참조를 노출하지 않는 읽기 전용 view는 III.6.3에서 추가합니다.
- 제거된 slot의 column 데이터는 재사용 전까지 옛 값을 남기지만, 접근은 항상 liveness를 먼저
  검사하므로 관측되지 않습니다.
