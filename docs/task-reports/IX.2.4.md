# 작업 보고서: IX.2.4 override 생성·변경·reset command

- WBS: IX.2.4
- 작업명: 인스턴스 PropertyOverride 생성·변경·reset command, reset은 map entry 삭제로 default 복귀
- 상태: 완료 (`[x]`)
- 선행: IX.2.3(이 lane에서 `[x]`), Phase VII(phase 단위 선행 — 완화 적용, 아래 "차이" 참조)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/override_commands.rs`(신규) — `PropertyOverrideCommand`(Set·Reset·Restore),
  `OverrideCommandError`, `validate`/`apply`(inverse 반환).
- `crates/document/src/component/mod.rs` — 모듈 등록.
- `docs/task-reports/IX.2.4.md`(신규), 계획서(IX.2.4 `[ ]`→`[x]`).

## 구현 요약

- **Set**: 공개 property override를 만들거나 바꿉니다. IX.2.3 `check_override`(공개·선언·타입 일치)를 통과해야
  합니다. inverse는 이전 값이 있으면 그 값의 `Set`, 없으면 `Reset`입니다.
- **Reset**: map entry를 삭제합니다(희소 저장 — reset 후 payload bytes는 override를 만든 적 없는 인스턴스와
  같습니다). entry가 없으면 `NotOverridden`. 정의에서 사라진 property의 orphan override도 지울 수 있습니다.
- **Restore**: `Reset`의 inverse 전용으로, 규칙을 재검사하지 않고 지운 값을 되살립니다. orphan override처럼
  `Set`으로는 다시 만들 수 없는 값도 undo가 정확히 복원합니다. entry가 이미 있으면 `AlreadyOverridden`.
- 인자는 인스턴스 정의의 `ComponentPropertyId`뿐이라 nested instance 경로를 표현할 수 없습니다(IX.4.2의
  "직접 수정 API 미제공"과 같은 경계). 검증 실패 시 scene payload는 바뀌지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 261 passed, 통합 7+1+1+2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 생성→변경→변경 undo→reset(bytes = override 없는 인스턴스, 24 bytes)→restore→redo, 생성 undo 원복(`set_change_reset_apply_and_inverse_with_sparse_storage`) | 통과 |
| 타입 불일치·다른 컴포넌트 property Set, 없는 override Reset, 인스턴스 아닌 node 거부, 거부 시 payload 불변(`invalid_commands_leave_scene_unchanged`) | 통과 |
| orphan override reset과 Restore 정확 복원, entry가 있으면 Restore 거부(`orphan_override_reset_is_exactly_undoable`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 `Phase VII`은 phase 단위 선행이라 완화했습니다. `stellar-commands`는 다른 lane이 동시에 구현 중이고 이
  lane은 `crates/commands`를 수정하지 않으므로, document 계층 closed enum + validate/apply(inverse 반환)로
  구현했습니다(IX.3.3과 같은 방식). command 계층 등록·revision·history 연동은 후속입니다.
- inverse 전용 `Restore` variant를 추가했습니다(orphan override undo의 정확성).

## 남은 위험 / 한계

- 정의 tree 안 nested 인스턴스의 override 편집은 정의 편집이므로 대상에서 제외했습니다.
- 여러 override를 한 번에 바꾸는 원자 적용은 CommandTransaction(VII.1.6)의 몫입니다.
