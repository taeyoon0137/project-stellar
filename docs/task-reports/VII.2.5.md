# 작업 보고서: VII.2.5 SetVisibility·SetLocked·RenameNode specialization

- WBS: VII.2.5
- 작업명: `SetVisibility`, `SetLocked`, `RenameNode`를 별도 command가 아니라 SetProperty specialization으로 노출
- 상태: 완료 (`[x]`)
- 선행: VII.2.4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/src/property.rs` — `SetProperty::{visibility, locked, rename}`, 테스트 전용 core patch 호출 카운터, 테스트.
- `crates/commands/src/command.rs` — `Command::{set_visibility, set_locked, rename_node}` 생성자.
- `docs/task-reports/VII.2.5.md`(신규), 계획서(VII.2.5 `[ ]`→`[x]`).

## 구현 요약

- 세 편집은 `CommandKind`에 새 종류를 추가하지 않고, 공통 필드 key(`VISIBLE`·`LOCKED`·`DISPLAY_NAME`, VII.1.4)를 채운
  `SetProperty`를 만드는 생성자로만 노출합니다. 직렬화(kind `0x0010`)·검증(registry 타입)·적용(`SetProperty::apply`의 core
  patch → `replace_record`)·undo(역 `SetProperty`)가 모두 같은 경로입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-commands` | 완료 |
| `cargo test -p stellar-commands` | unit 43 passed, doctest 3 passed, 0 failed |
| `cargo clippy -p stellar-commands --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-commands -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 세 specialization 모두 kind SetProperty·key 일치, 별도 registry 종류 없음, kind 0x0010 직렬화 round-trip, 실행 시 core patch 호출 정확히 1회, 일반 SetProperty와 snapshot·change set 동일, undo도 SetProperty, 의미 반영(`specializations_use_the_same_core_patch_path`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 없음(VII.2.4의 표시·잠금 변경 비용 한계를 그대로 가집니다).
