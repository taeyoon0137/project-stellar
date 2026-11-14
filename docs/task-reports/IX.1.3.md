# 작업 보고서: IX.1.3 Property 생성·rename·default 변경·삭제 command

- WBS: IX.1.3
- 작업명: Property 생성·rename·default 변경·삭제 command, inbound drilling이 있는 삭제는 orphan 정보 생성
- 상태: 완료 (`[x]`)
- 선행: IX.1.2(`[x]`), VIII.8.1(`[x]`), Phase VII(phase 단위 선행 — 완화 적용)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/property_commands.rs`(신규) — `PropertyCommand`(Create/Rename/SetDefault/Delete,
  inverse 전용 Restore), `PropertyCommandOutcome`, `PropertyCommandError`, library property record 삽입·삭제·교체
  helper, 상태 머신·정의 tree override 참조 검사.
- `crates/document/src/component/drilling.rs` — orphan 상태(`OrphanCause`, `BreakPoint`, `DrillingOrphan`),
  `DrillingSet`의 orphan 표시·해제·조회(`orphan`, `orphans`, `live`, `uses_of`, `target_type`), orphan 정보 canonical
  저장, 검증·중간 link 판정에서 orphan 제외.
- `crates/document/src/component/definition.rs` — `ComponentDefinition::insert_property_at`, `ComponentEntry::definition_mut`
  (`pub(super)`, IX.4.1의 테스트 전용 표시 해제).
- `crates/document/src/component/mod.rs` — 모듈 등록.
- `docs/task-reports/IX.1.3.md`(신규), 계획서(IX.1.3 `[ ]`→`[x]`).

## 구현 요약

- **command**(closed enum, 검증 후 적용·실패 시 library와 drilling 모음 불변, inverse 반환):
  `Create`(지정 위치 선언, 이름 유일) ↔ `Delete`, `Rename` ↔ 이전 이름, `SetDefault`(IX.1.2 default 규칙 재검사) ↔ 이전
  AST, `Delete` → `Restore`(원래 선언 위치 + 그 삭제가 만든 orphan만 해제).
- **orphan 정보**: 삭제 property를 쓰는 live drilling binding(owner source = `BreakPoint::Source`, 경계 chain =
  `BreakPoint::Boundary(k)`)을 지우지 않고 `DrillingOrphan { cause: PropertyDeleted, break_point, last_valid_type }`을
  붙여 남기며 결과 `orphaned`로 돌려줍니다. 이미 다른 원인으로 orphan인 binding은 덮어쓰지 않아 undo가 정확합니다.
  orphan 정보는 `DrillingSet` bytes에 저장됩니다.
- **삭제 거부**: 같은 컴포넌트 상태 머신 표현식이 참조하거나 다른 정의 tree의 nested 인스턴스가 정적 override하면
  거부합니다(둘 다 library 저장 검증이 거부하는 상태). 매 단계 `ComponentLibrary::decode(encode())`가 통과합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 324 passed, 통합 7+1+1, doc 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| Create(index 0)→Rename→SetDefault→Delete 적용, inverse 역순 적용 시 매 단계 library·drilling bytes 정확 복원, redo 재현, 매 단계 library 저장 검증 통과(`create_rename_default_delete_apply_and_inverse`) | 통과 |
| 가장 안쪽 property 삭제 → link 1개 `Boundary(0)` orphan(원인·last valid type 정확, binding 보존, 저장 round-trip), 중간 property 삭제 → `Source`·`Boundary(0)` 2건, 중복 orphan 미덮어쓰기, undo 후 bytes 동일(`deleting_a_drilled_property_creates_orphan_information`) | 통과 |
| 이름·id 중복, 범위 밖 위치, 빈 이름, default 타입 불일치, 없는 대상, 잘못된 revive 거부와 상태 불변(`invalid_commands_leave_state_unchanged`) | 통과 |
| 상태 머신 참조·정의 tree 정적 override 시 삭제 거부, 미참조 property 삭제 후 저장 검증 통과(`delete_is_rejected_when_state_machine_or_definition_override_uses_it`) | 통과 |

## 차이(작업 지시와 다른 점)

- 작업 순서: inbound drilling 표현(IX.4.1)이 먼저 필요해 IX.4.1을 먼저 커밋했습니다(계획상 IX.1.3은 IX.4.1에
  의존하지 않으며 둘 다 선행은 충족).
- 선행 `Phase VII`은 phase 단위 선행이라 완화했습니다. `crates/commands`는 이 lane의 허용 경로가 아니어서 document
  계층 closed enum + apply(inverse 반환)로 구현했습니다. command 계층 등록·history 연동은 후속입니다.
- inverse 전용 `Restore` variant를 추가했습니다(삭제가 만든 orphan만 정확히 해제).
- orphan 표현은 `stellar-bindings` `OrphanedBinding`(VIII.8.1)을 쓸 수 없어(crate 그래프) document drilling 모음의
  orphan 정보로 정의했습니다. 원래 ID·path·chain·마지막 유효 타입·끊긴 위치·원인을 보존하는 규칙은 같습니다.

## 남은 위험 / 한계

- 삭제는 scene을 보지 않습니다. scene 인스턴스 override는 orphan override로 남으며 IX.2.4 reset으로 정리합니다.
- 상위 chain 전체의 orphan 전파와 타입 변경은 IX.4.6 범위입니다(여기서는 삭제 property를 직접 쓰는 binding만).
