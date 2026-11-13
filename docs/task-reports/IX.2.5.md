# 작업 보고서: IX.2.5 가상 subtree 구조 편집 validator

- WBS: IX.2.5
- 작업명: 자유 구조 편집 금지, definition이 허용한 Slot만 변경 가능한 virtual subtree mutation validator
- 상태: 완료 (`[x]`)
- 선행: IX.2.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/mutation.rs`(신규) — `VirtualMutation`(Delete·Move·Insert),
  `validate_virtual_mutation`, `VirtualMutationError`.
- `crates/document/src/component/expansion.rs` — `chain_payload`를 crate 내부 공개(`pub(crate)`)로 변경.
- `crates/document/src/component/mod.rs` — `pub mod mutation;` 등록.
- `docs/task-reports/IX.2.5.md`(신규), 계획서(IX.2.5 `[ ]`→`[x]`).

## 구현 요약

- **대상 분류**: 정의에서 온 가상 node(`Virtual`, chain 1단계)는 `DefinitionNodeImmutable`, nested
  인스턴스 내부(chain 2단계 이상, 그 slot host 포함)는 `NestedInstanceInternal`로 삭제·이동을
  거부합니다. 인스턴스 소유 실제 node(`Owned`)만 떼어낼 수 있습니다.
- **목적지 분류**: 이 인스턴스(또는 slot에 놓인 실제 인스턴스)의 공개 slot host이거나 소유 실제 node
  (자유 구조) 아래만 허용합니다. 그 외 정의 node 아래, 실제 인스턴스 바로 아래는 `NotASlot`입니다.
- **slot 규칙**: 목적지 slot은 허용 종류·최대 개수, 출발지 slot은 최소 개수를 검사합니다. 같은
  slot 안 재정렬은 개수 변화가 없습니다. 자기 자손 아래로의 이동은 `CycleMove`입니다.
- 검사 전용이며 상태를 바꾸지 않습니다. 자유 편집은 detach(IX.2.6)로 합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 171 passed, 통합 2 passed, doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 정의 자식(Text·nested 인스턴스 node·slot host) delete/move, nested Icon 내부 자식 delete/move, slot의 실제 인스턴스 정의 자식 delete, Page 안 nested Card의 slot content·slot host 편집 거부(`nested_child_delete_and_move_are_rejected`) | 통과 |
| 공개 slot host만 삽입 허용, 정의 node·실제 인스턴스 아래 금지, 소유 node 아래 자유, 금지 종류(`only_public_slots_accept_structure`) | 통과 |
| 최대 개수 초과·최소 개수 미달·금지 종류·cycle 거부, slot content 삭제·재정렬·slot 간 이동 허용(`slot_content_edits_follow_cardinality`) | 통과 |

## 차이(작업 지시와 다른 점)

- 실행 순서: Slot host 정의(IX.3.1)와 투영(IX.3.2)이 필요해 그 뒤에 수행했습니다. 선행(IX.2.2)은 충족합니다.

## 남은 위험 / 한계

- 실제 적용(command)은 slot content command(IX.3.3)와 Phase VII command 계층의 몫입니다.
- variant 구조 override(IX.7.7)가 이 validator를 재사용하도록 연결하는 것은 후속입니다.

## 후속 수정 (리뷰 MAJOR 2: 적용 후 투영 기준 개수·가림 검사)

- **결함**: `Insert`·slot으로의 `Move`가 최대 개수만 검사해, default content를 보이던 slot에 처음 넣는 편집(1개짜리
  override)이 최소 개수 2 이상을 어겨도 통과했습니다.
- **수정**: 목적지·출발지 모두 **적용 후 content 목록**으로 최소·최대를 검사하고, 적용 후 payload에 nested slot host
  가림 규칙을 적용합니다(`VirtualMutationError::Projection`). 같은 인스턴스의 다른 slot 사이 이동은 출발 slot에서 뺀
  상태까지 합쳐 판정하므로, 안쪽 slot의 마지막 content를 바깥 slot으로 옮기는 편집은 허용됩니다.

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | unit 240 포함 전부 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| default slot(최소 2)으로의 첫 삽입·이동 거부(`count: 1`), 2개 override에 삽입 통과·3개에서 초과 거부(`first_insert_or_move_into_default_slot_checks_minimum_on_result`) | 통과 |
| 가려진 host 삽입·안쪽 content를 가리는 바깥 삽입 거부, 안쪽 마지막 content의 바깥 이동 허용·남으면 거부(`edits_creating_hidden_slot_content_are_rejected`) | 통과 |
| 기존 IX.2.5 fixture(`nested_child_delete_and_move_are_rejected` 등 3개) | 통과(변경 없음) |
