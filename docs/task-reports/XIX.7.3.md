# 작업 보고서: XIX.7.3 non-overlap external change는 agent 작업을 유지한 채 적용

- WBS: XIX.7.3
- 작업명: non-overlap external change는 agent 작업을 유지한 채 적용
- 상태: 완료 (`[x]`)
- 선행: XIX.7.1
- 결정: [ADR-0100](../adr/ADR-0100-agent-change-history-and-revert.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 셸 UI와 FFI는 범위가 아닙니다(2026-11-23 엔진 우선 방향).

## 변경 파일 목록

- `crates/commands/src/agent_external_tests.rs` — 분할 test.
- `crates/mcp-core/src/agent_history/external_tests.rs` — non-overlap test.

## 구현 요약

- 겹치지 않으면 세션과 lease를 유지하고, base→외부 변경 중 창에서 바뀌지 않은 레코드만 System 출처 `ApplyExternalSnapshot`으로 반영합니다. 에이전트의 저장 전 변경은 덮어쓰지 않습니다.

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
| independent subtree fixture: 외부 변경 반영, agent 작업·세션·lease 유지, 표시 상태 유지(`non_overlapping_external_change_is_applied_keeping_agent_work`, `partition_keeps_local_agent_changes_and_applies_independent_external_changes`) | 통과 |

## 차이(작업 지시와 다른 점)

- 반영 transaction은 호출자(앱 창 history의 외부 반영 경로)가 적용합니다.

## 남은 위험 / 한계

- 사용자의 저장 전 변경과 겹치는 외부 변경은 이 행 범위가 아니며 `divergent`로 XIII.3.4에 넘깁니다.

## 근거 커밋

- 이 보고서를 담은 `Task(XIX.7.3)` 커밋(브랜치 `lane/agent-history`).
