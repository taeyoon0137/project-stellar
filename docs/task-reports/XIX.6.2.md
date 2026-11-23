# 작업 보고서: XIX.6.2 정상 CompleteAgentEdit에서 변경 그룹을 history entry로 확정하고 lease 해제

- WBS: XIX.6.2
- 작업명: 정상 CompleteAgentEdit에서 변경 그룹을 history entry로 확정하고 lease 해제
- 상태: 완료 (`[x]`)
- 선행: XIX.6.1
- 결정: [ADR-0100](../adr/ADR-0100-agent-change-history-and-revert.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 셸 UI와 FFI는 범위가 아닙니다(2026-11-23 엔진 우선 방향).

## 변경 파일 목록

- `crates/commands/src/agent_sink.rs`(신규) — `AgentHistorySink`, `AgentGroup`.
- `crates/history/src/agent.rs`(신규) — `WindowAgentHistory`, `AgentEntryLabels`. `crates/history/src/agent_tests.rs`(신규).
- `crates/history/src/history.rs` — crate 내부 `push_applied_document`(이미 적용된 변경을 항목 하나로 확정). 소유 범위 밖 파일이며 이유는 ADR-0100 결정 2.
- `crates/mcp-core/src/agent_history/mod.rs`(신규) — `AgentEditSessions`(start, begin_edit, queue/apply/reject proposal, complete, end).
- `crates/mcp-core/src/agent_history/tests.rs`(신규), `crates/mcp-core/src/provenance_fixture.rs`(test 대역 `TestHistory`, `World`).
- `crates/commands/src/lib.rs`, `crates/history/src/lib.rs`, `crates/mcp-core/src/lib.rs` — module 등록.

## 구현 요약

- 완료하면 진행 중 batch의 정방향(이어 붙임)·역방향(역순)을 다시 실행하지 않고 "에이전트 편집" history entry 하나로 넣고, 그 lease를 조정자에서 해제합니다. 세션과 권한은 남습니다.
- mcp-core와 history가 서로 닿지 않아 commands에 연결 지점 trait를 두고 history가 창 history로 구현합니다.

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
| 완료 후 history entry 1, 출처 FutureAgent, revision 변화 0, lease 0, 같은 범위 사용자 편집 성공(`complete_confirms_one_entry_and_releases_lease_for_user_edit`) | 통과 |
| 완료 전 lease 안 사용자 편집 거부(`user_edit_inside_lease_is_rejected_before_completion`) | 통과 |
| 창 history 그룹 항목 Undo로 baseline, Redo로 재적용(`completed_group_is_one_undoable_entry_without_reexecution`) | 통과 |

## 차이(작업 지시와 다른 점)

- `History`에 crate 내부 함수 하나를 더했습니다(소유 범위 밖). 그룹을 되돌린 뒤 다시 실행하는 우회는 확정이 실패할 수 있어 채택하지 않았습니다(ADR-0100 Alternatives).

## 남은 위험 / 한계

- Undo 메뉴 이름은 `AgentEntryLabels::action_name`이 정하고 `HistoryEntry::action_name`은 그대로입니다. 메뉴 연결은 UI 재설계 때 합니다.
- 그룹 항목은 확정 시점에 stack 맨 위에 들어갑니다(ADR-0100 결정 2).

## 근거 커밋

- 이 보고서를 담은 `Task(XIX.6.2)` 커밋(브랜치 `lane/agent-history`).
