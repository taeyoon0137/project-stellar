# 작업 보고서: XXI.4.6 플러그인 command의 일반 Command·Git WorkingTree 경로 사용

- WBS: XXI.4.6
- 작업명: plugin command도 일반 Command/Undo/Git WorkingTree 경로를 사용
- 상태: 부분 완료 (`[~]`) — headless 경로(Command·저장·Git WorkingTree)는 완료. 앱 Undo history 경로는 앱 attached 경로가 생길 때 연결합니다.
- 선행: Phase VII, XXI.4.2
- 결정: [ADR-0097](../adr/ADR-0097-plugin-host-core.md) 결정 9

## 필수 실행 환경

- `DEV-MAC` (Rust, 시스템 Git 2.x, 로컬 검증).

## 변경 파일 목록

- `crates/plugin-host/tests/plugin_command_path.rs`(신규).
- 구현은 XXI.4.2의 `command.apply` handler(`crates/plugin-host/src/host.rs`)입니다. 이 행에서 코드를 더 바꾸지 않았습니다.

## 구현 요약

- `command.apply`는 command object를 `ops::mcp_command_input::command_input`으로 해석하고 `stellar apply`와 같은 `ops::apply::apply`를
  같은 저장 잠금(`LockPolicy::Required`)으로 부릅니다. 플러그인 전용 편집·저장 경로가 없습니다.
- 결과는 generation 저장(작업 트리)에 기록되므로 Git status가 사용자·CLI 편집과 같은 방식으로 봅니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-plugin-host --test plugin_command_path` | 1 passed(시스템 Git 있음, 건너뜀 없음) |

| 검사 | 결과 |
|---|---|
| 같은 내용 두 프로젝트에 같은 transaction(set-visible, rename): 플러그인 경로와 CLI 경로의 결과 revision 같음 | 통과 |
| semantic diff 항목 수·종류 같음 | 통과 |
| 커밋한 기준 위 `git.status` unstaged semantic 변경 목록이 같음(비어 있지 않음) | 통과 |

## 차이(작업 지시와 다른 점)

- "user edit와 동일 semantic diff"를 headless 사용자 편집 경로(`stellar apply`)와 비교했습니다. 앱 창의 사용자 편집은 앱 DocumentThread를
  지나므로, 앱 attached 경로가 생기면 같은 비교를 그 경로로 다시 해야 합니다.

## 남은 위험 / 한계

- 플러그인 편집을 앱 Undo history에 넣는 연결이 없습니다. 앱이 열린 동안 플러그인이 쓰면 앱은 외부 변경으로 봅니다(ADR-0083).
