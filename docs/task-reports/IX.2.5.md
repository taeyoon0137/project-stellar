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
