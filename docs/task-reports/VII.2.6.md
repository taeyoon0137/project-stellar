# 작업 보고서: VII.2.6 InsertModifier·RemoveModifier·MoveModifier·SetModifierProperty

- WBS: VII.2.6
- 작업명: `InsertModifier`, `RemoveModifier`, `MoveModifier`, `SetModifierProperty`를 각각 구현
- 상태: 완료 (`[x]`)
- 선행: VI.9.1~VI.9.7, VII.1.6

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/src/modifier_ops.rs`(신규) — 네 command(validate/invert/apply/codec), 공통 stack 검증, registry
  값 타입 대응, payload 속성 읽기·쓰기, 테스트 5건.
- `crates/commands/src/property.rs` — modifier 값 타입 8종(`BooleanOp`, `NodeRefList`, `NodeRef`, `NonNegative`, `Color`,
  `Offset`, `UnitInterval`, `BlendMode`)과 값 tag `10..=17`, 테스트 helper의 표본 값 확장.
- `crates/commands/src/command.rs`, `crates/commands/src/kind.rs`, `crates/commands/src/codec.rs`,
  `crates/commands/src/lib.rs` — `0x0020`~`0x0023` 등록·직렬화·dispatch.
- `docs/task-reports/VII.2.6.md`(신규), 계획서(VII.2.6 `[ ]`→`[x]`).

## 구현 요약

- **InsertModifier**: caller가 정한 id·OrderKey·종류·payload의 항목을 `ModifierStack::insert_on`(VI.9.6 소유 종류 검사
  포함)으로 넣습니다. id는 NIL 금지·문서 영속 id 공간에서 미사용. 역은 `RemoveModifier`.
- **RemoveModifier**: 항목을 빼고(빈 stack은 제거) 역으로 원래 항목(같은 OrderKey 위치)을 넣는 `InsertModifier`.
- **MoveModifier**: 순서 key를 바꿉니다(stack 안 key 충돌 거부). 역은 원래 key로의 `MoveModifier`.
- **SetModifierProperty**: VI.9.7 registry로 key를 찾고, 그 modifier 종류가 가지지 않는 key는 `UnknownProperty`, 값 타입이
  registry 타입과 다르면 `PropertyTypeMismatch`, `ORDER`는 `PropertyNotSettable`(MoveModifier 전용). `ENABLED`는 stack의
  활성 여부, 나머지는 payload 필드를 바꾸고 `ModifierPayload::validate`로 값 범위(비음수, 0..=1)를 검사합니다.
- **공통 검증**: 소유 node 존재, Fragment 금지, 참조 대상이 문서에 있으면 node, 문서 전체 modifier 참조 cycle 금지(VI.9.5).
- **change set**: modifier id를 root로 한 속성 경로(삽입·삭제는 modifier 전체, 이동은 `ORDER`, 속성은 해당 key)와 registry
  무효화(`kind_invalidation`/`invalidation_of`).
- **직렬화**: 항목은 단일 항목 `ModifierStack` canonical bytes(document 형식 재사용)로 기록합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-commands` | 완료 |
| `cargo test -p stellar-commands` | unit 48 passed, doctest 3 passed, 0 failed |
| `cargo clippy -p stellar-commands --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-commands -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| Insert→inverse(Remove) snapshot 원복(빈 stack 제거), Remove의 역이 같은 항목, 가운데 항목 제거→원복(`insert_and_remove_modifier_round_trip`) | 통과 |
| Move→순서·경로 확인→undo 원복, key 충돌 거부(`move_modifier_round_trip`) | 통과 |
| 12종 modifier의 payload 속성 16개 + ENABLED 각각 설정→값·경로·registry 무효화 일치→undo 원복, codec round-trip(`set_modifier_property_round_trip_for_every_field`) | 통과 |
| Fragment 소유, id 중복(modifier·node), stack key 충돌, 불투명도 1.5, 타입 불일치, ORDER 설정, 종류에 없는 key, self·간접 cycle, View id 참조, 없는 modifier·node 거부, 문서 불변(`modifier_commands_reject_invalid_edits`) | 통과 |
| kind 0x0020~0x0023 codec round-trip(`modifier_command_codec_round_trips`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- cycle·id 검사는 문서의 모든 stack을 훑습니다(O(N)).
