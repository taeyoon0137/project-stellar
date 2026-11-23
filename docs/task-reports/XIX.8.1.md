# 작업 보고서: XIX.8.1 MCP/agent model에 ShadowWorktree 전용 타입이 없는지 inventory test

- WBS: XIX.8.1
- 작업명: MCP/agent model에 ShadowWorktree 전용 타입이 없는지 inventory test
- 상태: 완료 (`[x]`)
- 선행: XIX.5.1
- 결정: [ADR-0100](../adr/ADR-0100-agent-change-history-and-revert.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 셸 UI와 FFI는 범위가 아닙니다(2026-11-23 엔진 우선 방향).

## 변경 파일 목록

- `crates/mcp-core/src/agent_history/inventory_tests.rs`(신규).
- `crates/mcp-core/src/agent_history/mod.rs` — test module 등록.

## 구현 요약

- mcp-core, agent-protocol, agent-host, stellar-mcp가 작업 사본 개념(worktree·checkout·workspace 등) 타입을 선언하지 않는지, 세션 key가 일반 프로젝트 identity(project id + canonical target)인지 확인합니다. 위반 fixture(ShadowWorktree 선언)는 검출합니다.

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
| MCP·에이전트 crate 작업 사본 타입 0(`agent_crates_declare_no_worktree_types`) | 통과 |
| 위반 fixture 검출(`detector_rejects_shadow_worktree_fixture`) | 통과 |
| 세션 key가 project id·canonical target·세션뿐(`agent_session_key_is_general_project_identity`) | 통과 |

## 차이(작업 지시와 다른 점)

- workspace 전체 금지는 XVIII.5.3 test가 이미 맡아, 이 test는 에이전트 crate만 더 엄격하게 봅니다.

## 남은 위험 / 한계

- 없음.

## 근거 커밋

- 이 보고서를 담은 `Task(XIX.8.1)` 커밋(브랜치 `lane/agent-history`).
