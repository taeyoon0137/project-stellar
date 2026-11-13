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
