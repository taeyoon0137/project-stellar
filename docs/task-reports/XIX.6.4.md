# 작업 보고서: XIX.6.4 세션 종료 시 자동 rollback 없는 종료 절차

- WBS: XIX.6.4
- 작업명: 세션 종료 시 자동 rollback 없는 종료 절차
- 상태: 완료 (`[x]`)
- 선행: XIX.1.4, XIX.6.2, XIX.6.7
- 결정: [ADR-0100](../adr/ADR-0100-agent-change-history-and-revert.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 셸 UI와 FFI는 범위가 아닙니다(2026-11-23 엔진 우선 방향).

## 변경 파일 목록

- `crates/mcp-core/src/agent_history/mod.rs` — `end`, `on_bridge_disconnected`, `EndReason`, `SessionEndReport`(XIX.6.2 커밋에 함께 들어감).
- `crates/mcp-core/src/agent_history/tests.rs` — 종료 절차 test.

## 구현 요약

- 종료 이유(MCP 연결 종료, process 종료, 사용자 중지, 재연결 유예 초과, 비정상 종료, 신뢰 철회, lease 겹침 외부 변경, 사용자의 lease 해제)마다 같은 절차를 실행합니다: 진행 중 batch를 "중단됨" entry 하나로 확정, batch·출처 표시 유지, lease와 대기 요청 해제, 응답 대기 확인 요청 거부 보고와 세션 권한 폐기, 적용 전 제안 폐기, 내용 없는 진단.
- history 확정이 실패해도 lease·권한 해제는 반드시 실행합니다. 확정하지 못한 batch는 진행 중으로 남고, 끝난 세션에 대해 `confirm_interrupted`가 다음 확정 기회에 같은 경로로 "중단됨" entry 하나로 확정합니다(검토 보완).
- 앱 시작 세션의 재연결 유예는 AgentSession(XXIX.2.8)이 판정하고, 이 관리자는 bridge 끊김에서 세션을 유지하다가 유예 초과 이유로 같은 절차를 실행합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

브랜치 `lane/agent-history` 최종 상태에서 실행했습니다.

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-commands --locked` | lib 205 passed, 5 ignored; 통합 test 3 passed |
| `cargo test -p stellar-history --locked` | lib 55 passed |
| `cargo test -p stellar-mcp-core --locked` | lib 101 passed |
| `cargo clippy -p stellar-commands -p stellar-history -p stellar-mcp-core --all-targets --locked -- -D warnings` | 경고 0 |
| `cargo xtask validate-adr --base 682c7b0c` | 0 |
| `cargo xtask generate-ffi-header --check` | 0, drift 없음 |
| `cargo xtask validate-crate-graph` | 0 |

| 검사 | 결과 |
|---|---|
| kill fixture: 적용 batch 유지, 출처 표시 존재, lease 0, permission 0, 대기 요청 0, history entry 1(중단)(`kill_fixture_keeps_batches_and_clears_lease_permission_and_pending`) | 통과 |
| 앱 시작 세션 bridge disconnect: 유예 안 세션·lease 유지, 유예 초과 후 같은 종료 결과(`external_client_disconnect_ends_but_app_started_waits_for_grace`) | 통과 |
| batch 없는 종료는 entry 0, lease 해제(`end_without_batches_creates_no_entry_and_still_releases`) | 통과 |
| history 거부 시에도 lease·권한 해제, 이후 `confirm_interrupted`로 남은 batch를 entry 하나로 확정·끝나지 않은 세션 거부(`history_refusal_still_releases_lease_and_permission`) | 통과 |

## 차이(작업 지시와 다른 점)

- "적용 중인 transaction"은 DocumentThread의 원자 실행이라 종료 절차가 transaction 경계에서만 실행됩니다. 별도 대기 상태는 AgentSession(`Ending`)이 가집니다.
- kill은 process 종료 이유로 재현했습니다(실제 process kill은 agent-host XXIX.2.8 범위).

## 남은 위험 / 한계

- 앱 runtime이 AgentSession 효과를 이 절차에 잇는 일은 XXIX.3.1입니다.

## 근거 커밋

- 이 보고서를 담은 `Task(XIX.6.4)` 커밋(브랜치 `lane/agent-history`).
