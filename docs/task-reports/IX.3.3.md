# 작업 보고서: IX.3.3 Slot content command

- WBS: IX.3.3
- 작업명: slot content 삽입·삭제·재정렬 command, allowed kind/cardinality 검증
- 상태: 완료 (`[x]`)
- 선행: IX.3.2, Phase VII(phase 단위 선행 — 완화 적용, 아래 "차이" 참조)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/slot_commands.rs`(신규) — `SlotContentCommand`(Insert·Remove·Reorder),
  `SlotNodeRecord`, `SlotCommandError`, `validate`/`apply`(inverse 반환).
- `crates/document/src/component/mod.rs` — `pub mod slot_commands;` 등록.
- `docs/task-reports/IX.3.3.md`(신규), 계획서(IX.3.3 `[ ]`→`[x]`).

## 구현 요약

- **closed enum command**: `Insert`(content subtree를 인스턴스 소유 node로 만들고 slot 위치에 투영),
  `Remove`(content subtree 삭제 + 투영 제거, `restore_default`면 마지막 content 제거 시 override entry를
  지워 default content로 복귀), `Reorder`(투영 순서 변경).
- **검증**: 인스턴스 해석, slot 존재, 삽입 root 종류가 slot 허용 종류인지(`DisallowedKind`), 최대 개수
  초과(`CardinalityViolation`), 삭제 시 최소 개수 미달, 위치 범위, 중복 StableId, record 부모 규칙(root =
  인스턴스, 자손 = 앞선 record), 인스턴스 자식 OrderKey 중복, 투영되지 않은 node 대상을 거부합니다.
  `apply`는 전부 검증한 뒤에만 scene을 바꿉니다.
- **inverse**: Insert ↔ Remove(override entry 생성 여부를 `restore_default`로 보존), Remove는 삭제한
  subtree(필드·payload)를 담은 Insert, Reorder는 원래 위치로의 Reorder를 반환합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 179 passed, 통합 4 passed, doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| Text 전용 footer에 Frame 삽입 거부, body(최대 3)·footer(최대 1) 초과 삽입 거부, 거부 시 scene snapshot 불변(`invalid_kind_and_overflow_are_rejected`) | 통과 |
| subtree 삽입 → 투영·소유 관계·투영 검증 통과, inverse로 snapshot 원복, inverse의 inverse 재적용(`insert_apply_and_inverse_round_trip`) | 통과 |
| 최소 개수 미달 삭제 거부, default 복귀 삭제(subtree·entry 제거, default 표시) 후 inverse 원복(`remove_respects_minimum_and_restores_default`) | 통과 |
| 재정렬 inverse, override 없던 slot 삽입의 inverse가 entry까지 제거해 payload 원복(`reorder_and_footer_insert_inverse_restore_sparse_payload`) | 통과 |
| 없는 slot·빈 삽입·범위 밖·중복 id·잘못된 부모·OrderKey 중복·비투영 node·scene 밖 대상 거부(`malformed_commands_are_rejected`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 `Phase VII`은 phase 단위 선행이라 사용자 지시(2026-11-14)에 따라 완화했습니다. 3002e6d의
  `stellar-commands`는 골격뿐이라, command를 document 계층의 closed enum + validate/apply(inverse 반환)로
  구현했습니다. VII.1.2 closed enum·revision·history 연동은 후속입니다.
- 정의 tree 안 nested 인스턴스의 slot content 편집은 정의 편집이므로 이 command 대상(scene 인스턴스)에서
  제외했습니다.

## 남은 위험 / 한계

- 여러 content를 한 번에 넣어야 최소 개수를 만족하는 경우의 원자 적용은 CommandTransaction(VII.1.6)이
  필요합니다. 현재 Insert는 최대 개수만 검사하고, 최소 개수 미달 상태는 IX.3.2 validator가 보고합니다.
- 삭제 content를 가리키던 다른 참조(ReferenceIndex) 처리 정책은 command 계층(VII.2.2)의 몫입니다.

## 후속 수정 (리뷰 MAJOR 2: 적용 후 투영 기준 개수·가림 검사)

- **결함**: `Insert`가 최대 개수만 검사해, default content를 보이던 slot에 처음 넣으면 1개짜리 override가 되어
  최소 개수(2 이상)를 어기는 상태를 만들 수 있었습니다(위 "남은 위험"의 최소 개수 미달이 command로 생김).
- **수정**: `Insert`·`Remove`는 **적용 후 content 목록**으로 cardinality(최소·최대)를 검사하고, 적용 후 payload에
  nested slot host 가림 규칙(`check_hidden_slot_content`)을 적용해 위반이면 적용 전에 거부합니다
  (`SlotCommandError::Projection`). 최소 개수가 2 이상인 slot은 이제 단일 `Insert`로 override를 시작할 수 없으며,
  여러 content를 한 번에 넣는 원자 command는 VII.1.6 이후입니다.

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | unit 240, `component_detach` 6, `component_expansion_scale` 1, `component_slot_projection` 1, doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| default 2개를 보이던 최소 2 slot에 첫 삽입 거부(`count: 1`)·scene 불변, 2개 override에 세 번째 삽입 통과(`first_insert_into_default_slot_checks_minimum_on_result`) | 통과 |
| 안쪽 content가 있는데 바깥 slot 삽입 거부, 가려진 안쪽 slot 삽입 거부(각각 불변), 바깥을 default로 되돌린 뒤 안쪽 삽입 통과(`inserts_creating_hidden_slot_content_are_rejected`) | 통과 |

## 후속 수정 (리뷰 minor: Insert subtree 형태 검증)

- **결함**: `Insert`는 content root와 인스턴스 자식 사이 OrderKey만 검사해, 삽입 subtree 안 같은 부모의 형제 OrderKey
  중복과 leaf 종류(예: Text) 아래 자식을 받아들였습니다.
- **수정**: subtree 안 부모별 형제 OrderKey 중복을 `DuplicateOrderKey`로, leaf(Shape·Path·Text·Image) 부모를
  `ParentNotContainer`로 거부합니다. ComponentInstance 부모는 허용하되(그 자식 = slot content), subtree 안 인스턴스의
  slot override가 subtree 안 자기 자식만 투영 규칙대로 가리키는지 검사합니다(`Projection`). `Remove`의 inverse가 nested
  인스턴스와 그 slot content를 되살려야 하므로 인스턴스 부모 자체를 금지하지 않았습니다. 투영할 것이 없는 인스턴스는
  참조 컴포넌트가 미해결이어도 이전처럼 허용합니다(III.2.4).

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | unit 243, `component_detach` 6, `component_expansion_scale` 1, `component_slot_projection` 1, doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| Text 부모 거부, subtree 안 형제 key 중복 거부, 다른 부모 아래 같은 key 허용(`insert_subtree_parents_and_sibling_keys_are_validated`) | 통과 |
| subtree 안 nested 인스턴스의 미투영 자식·subtree 밖 content 거부, 유효 subtree 삽입 후 두 인스턴스 투영 통과, Remove inverse로 snapshot 복원(`nested_instance_subtree_is_validated_and_remove_inverse_restores_it`) | 통과 |

## 후속 수정 (2026-11-24 instance-integrity: 일반 구조 command와 Slot content 투영, ADR-0133 결정 1·2)

### 배경

ADR-0129 결정 3으로 ComponentInstance 자식(투영된 Slot content)을 문서에 둘 수 있게 된 뒤, 일반 구조 command가 투영을
몰라 다음이 깨졌습니다. `delete-subtree`로 Slot content를 지우면 slot override가 지워진 node를 가리킨 채 남고, 그 역
`insert-node`는 부모가 인스턴스라 거부되어 undo가 실패했습니다. `move-node`로 인스턴스 밖으로 옮겨도 같았습니다. Slot
content를 가진 인스턴스를 지운 뒤의 undo, 그런 인스턴스의 복제·붙여넣기도 `insert-node`가 인스턴스 아래 자식을 거부해
실패했습니다.

### 변경 파일

- `crates/commands/src/node_ops.rs` — `slot_content_owner`(Slot content 판정 하나), `check_external_inbound`(삭제 정책을
  함수로 분리). `delete-subtree`·`move-node`가 Slot content를 투영 밖으로 지우거나 옮기면 거부, `insert-node`가 삽입하는
  인스턴스 아래 자식을 받고 투영(`validate::instance_projection_issues`)을 검사.
- `crates/commands/src/detach.rs` — `plan_slot_content_delete`·`SlotContentDeleteError`(투영에서 빼고 subtree를 지우는
  `place-slot-content` 하나).
- `crates/commands/src/detach_integrity_tests.rs`(신규), `crates/commands/src/detach_tests.rs`(fixture 공유).
- `docs/adr/ADR-0133-instance-structure-integrity.md`(신규).

### 실행한 명령

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-commands --lib` | 통과(257 passed, 6 ignored) |
| `cargo clippy -p stellar-commands --all-targets --locked -- -D warnings` | 통과 |

### 테스트

- `structure_commands_refuse_to_drop_slot_content_out_of_its_projection`: Slot content 31의 `delete-subtree`, 인스턴스 밖
  `move-node`, 인스턴스 아래로의 이동·삽입을 거부하고 문서는 그대로입니다. 같은 인스턴스 안 OrderKey 변경은 받고 문서
  validator를 통과하며 undo가 정확합니다.
- `slot_content_delete_is_one_restructure_and_undoes_exactly`: content subtree(31과 자식 32)를 자식부터 지우는 command 하나,
  codec 왕복, 결과 문서 validator 통과, `restore_default`에 따라 빈 override 또는 override 없음, undo 한 번에 처음 문서와
  같습니다.
- `slot_content_delete_refuses_non_content_locks_and_missing_definitions`: Slot content가 아닌 node, 미해결 컴포넌트,
  잠긴 인스턴스.
- `instance_subtree_delete_duplicate_and_paste_keep_projected_children`: 인스턴스 삭제의 역 command(codec 왕복 뒤) 적용이
  처음 문서와 같고, 복제·붙여넣기 결과 인스턴스의 slot override가 새 content id를 가리키며 문서 validator를 통과합니다.
- `insert_rejects_instance_children_that_are_not_projected_exactly_once`: 투영되지 않은 자식이 섞인 삽입은 거부, 투영된
  자식만이면 받습니다.

### 남은 범위

- runtime 선택 삭제·끌어 내기 제스처가 Slot content를 `plan_slot_content_delete`·`plan_slot_take_out`으로 보내는 연결은
  UI 재설계 때 합니다. 지금 선택 삭제에 Slot content가 섞이면 command가 거부합니다.
