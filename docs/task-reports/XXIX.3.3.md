# 작업 보고서: XXIX.3.3 에이전트 모드를 권한 상한으로 매핑

- WBS: XXIX.3.3
- 작업명: 모드별 Read·Propose·Apply 상한, 내장 세션 카탈로그(Terminal·FileDelete 제외, Git 쓰기 개별 확인), 사용자 전용 모드 변경
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 XXIX.3.2, XIX.9.7이 `[x]`가 아닙니다. 그 조건이 풀리면 `[x]`. 구현 보고 당시 기록: `[x]` 완료(엔진 부분). 앱 attached host에 카탈로그를 적용하는 일은 XXIX.3.1입니다.
- 선행: XXIX.3.2, XIX.9.7
- 설계 기준: ADR-0005 §4.5·Security, ADR-0092 결정 11, ADR-0101 결정 2

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/agent-host/src/mode.rs`(신규): `GrantRoute`, `grant_route`, `builtin_tools`, `builtin_tool`, `ToolRefusal`, `SessionMode::{ALL, name, default}`
- `crates/agent-host/src/session.rs`: 권한 받기를 `grant_route`로 판정, `AgentSession::change_mode_by_user`, `SessionEffect::RevokePermissions`, `SessionEffect::kind`, `SessionState::code`
- `crates/agent-host/tests/mode_matrix.rs`(신규), `crates/agent-host/tests/session_machine.rs`(새 효과 variant 이름 한 줄)
- `crates/agent-host/src/lib.rs`: module 등록

## 구현 요약

| 권한 | 읽기 전용 | 검토 후 적용(기본) | 승인 범위 내 바로 적용 |
|---|---|---|---|
| Read | 세션 | 세션 | 세션 |
| Propose | 상한 밖 | 세션 | 세션 |
| Apply | 상한 밖 | 상한 밖 | 세션 |
| GitCommit·GitPush | 상한 밖 | 실행 한 번 개별 확인 | 실행 한 번 개별 확인 |
| Terminal·FileDelete | 제외 | 제외 | 제외 |

- **판정 함수 하나**: `grant_route(mode, permission)`입니다. 세션 상태 기계의 권한 받기(`PermissionGranted`), `tools/list`(`builtin_tools`), `tools/call`(`builtin_tool`)이 모두 이 함수를 씁니다.
- **모드는 상한**입니다. 세션은 권한 0에서 시작하고 XIX.3 승인을 받은 권한 중 표의 `세션` 칸만 보유합니다. GitCommit·GitPush는 세션 권한이 아니라 실행 한 번 확인(XIX.3.4 `is_session_grant`)이라 세션이 보유하지 않습니다.
- **내장 세션 카탈로그**: attached backend와 문서·Git namespace에서 시작합니다. 터미널 namespace는 켜지 않습니다.
  - 표의 `세션`·`실행 한 번` 칸 권한의 tool만 보입니다.
  - 읽기 전용에는 상태를 바꾸는 tool이 없고, 검토 후 적용에는 `document.apply_commands`가 없습니다(batch는 사용자 승인으로 앱이 적용).
- **모드 변경**: `AgentSession::change_mode_by_user` 하나입니다.
  - 낮추면 새 상한 밖 세션 권한만 `RevokePermissions`로 폐기하고 lease·요청·적용 변경은 그대로입니다.
  - 올려도 권한은 생기지 않습니다. 끝나는 중·끝난 세션은 바꾸지 않습니다.
  - 카탈로그(agent-protocol)에는 모드·권한·신뢰를 쓰는 tool이 없어 그런 이름의 호출은 `Unknown`입니다. 세션 event에는 모드 변경 event가 없습니다(XXIX.6.5 inventory).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-agent-host --test mode_matrix` | 5 passed |
| `cargo test -p stellar-agent-host --test session_machine` | 통과(기존 test 그대로, `AboveModeCap` 판정 결과 동일) |
| `cargo clippy -p stellar-agent-host --all-targets -- -D warnings` | 경고 0 |

| 검증 기준 | test | 결과 |
|---|---|---|
| 모드별 permission matrix | `permission_matrix_per_mode`, `mode_is_a_ceiling_not_a_grant` | 통과(표 golden, `세션` 칸 = `SessionMode::cap`, 시작 권한 0) |
| 내장 세션 tools/list의 terminal·파일 삭제 계열 0 | `builtin_catalog_has_no_terminal_or_file_delete_tools` | 통과(세 모드 모두 0, 호출도 `Excluded`) |
| tool 호출로 모드 변경 시도 거부 | `tool_calls_cannot_change_mode` | 통과(카탈로그 이름·schema에 mode·permission·trust·provenance·revert 0, 모드 변경 이름 호출 `Unknown`) |
| 사용자 모드 변경 | `user_mode_change_lowers_ceiling_and_never_raises_grants` | 통과 |

## 차이(작업 지시와 다른 점)

- 읽기 전용 모드에서는 GitCommit·GitPush도 상한 밖으로 정했습니다. ADR-0005 §4.5 표의 읽기 전용은 Read만이고, Git 쓰기는 상태를 바꾸기 때문입니다(ADR-0101 결정 2).

## 남은 위험 / 한계

- 내장 세션 bridge의 attached `tools/list`·`tools/call`은 지금 mcp-core가 처리합니다. mcp-core는 agent-host에 닿을 수 없어, 앱 host trait 구현(XXIX.3.1, ADR-0005 §8)에서 `builtin_tools`·`builtin_tool`을 적용해야 합니다.
- 모드 변경 UI는 UI 재설계 때 만듭니다.
