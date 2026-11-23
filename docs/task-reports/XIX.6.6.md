# 작업 보고서: XIX.6.6 Interrupted Agent Task에 session id, duration, error code만 local diagnostic으로 남김

- WBS: XIX.6.6
- 작업명: Interrupted Agent Task에 session id, duration, error code만 local diagnostic으로 남김
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 XVI.8.1, XIX.6.4이 `[x]`가 아닙니다. 그 조건이 풀리면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]`)
- 선행: XVI.8.1, XIX.6.4
- 결정: [ADR-0100](../adr/ADR-0100-agent-change-history-and-revert.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 셸 UI와 FFI는 범위가 아닙니다(2026-11-23 엔진 우선 방향).

## 변경 파일 목록

- `crates/mcp-core/src/agent_history/mod.rs` — `InterruptedAgentTask`(XIX.6.2 커밋에 함께 들어감).
- `crates/mcp-core/src/agent_history/tests.rs` — 진단 내용 test.

## 구현 요약

- 종료 절차가 `InterruptedAgentTask { session, duration_ms, error_code }`를 남깁니다. 표시 문자열도 이 셋만 씁니다. 오류 코드는 `EndReason::code`이고 AgentSession과 같은 이유는 같은 문자열입니다.

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
| 진단 필드가 셋뿐이고 표시 문자열에 문서 이름·세션 표시 이름·경로·node id 없음(`interrupted_diagnostic_has_only_session_duration_and_code`) | 통과 |

## 차이(작업 지시와 다른 점)

- 로컬 diagnostic store(XVI.8.1)가 아직 없어 기록 타입만 만들었습니다. store가 생기면 그 schema에 넣습니다.

## 남은 위험 / 한계

- agent-host `SessionDiagnostic`과 모양이 같은 타입이 둘입니다. agent-protocol로 합치는 일이 후속입니다(소유 밖).

## 근거 커밋

- 이 보고서를 담은 `Task(XIX.6.6)` 커밋(브랜치 `lane/agent-history`).
