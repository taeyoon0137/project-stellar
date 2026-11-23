# 작업 보고서: XIX.7.2 overlap이면 agent 세션을 종료 절차로 끝내고 reconciliation에 넣음

- WBS: XIX.7.2
- 작업명: overlap이면 agent 세션을 종료 절차로 끝내고 reconciliation에 넣음
- 상태: 부분 완료 (`[~]`)
- 선행: XIX.7.1, XIX.6.4, XIII.3.4
- 결정: [ADR-0100](../adr/ADR-0100-agent-change-history-and-revert.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 셸 UI와 FFI는 범위가 아닙니다(2026-11-23 엔진 우선 방향).

## 변경 파일 목록

- `crates/mcp-core/src/agent_history/external.rs` — `respond_to_external`, `SemanticConflictCandidate`(XIX.7.1 커밋에 함께 들어감).
- `crates/mcp-core/src/agent_history/external_tests.rs` — overlap test.

## 구현 요약

- 겹친 lease의 이 창 세션을 `LeaseConflict` 종료 절차로 끝냅니다(자동 rollback 없음, lease 해제, 대기 제안 거부). 적용된 에이전트 변경은 표시를 유지한 로컬 변경으로 남고, 창에서 바뀌지 않은 레코드의 외부 변경만 반영 transaction으로 돌려줍니다. 창과 외부가 모두 바꾼 레코드와 같은 속성을 다르게 바꾼 semantic conflict 후보는 reconciliation 입력으로 돌려줍니다.

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
| overlap fixture: agent 변경 자동 삭제 0, 겹친 속성 conflict 후보, lease 0, 중단 entry 1(`overlapping_external_change_ends_session_without_deleting_agent_change`) | 통과 |

## 차이(작업 지시와 다른 점)

- XIII.3.4 semantic reconciliation과 Semantic Conflict 경로가 아직 없어, 후보와 겹친 레코드를 넘기는 데까지 했습니다. 그래서 부분 완료입니다.

## 남은 위험 / 한계

- 다른 프로세스가 쥔 겹친 lease는 그 프로세스가 끝낸다고 보고 `foreign_overlaps`로만 알립니다.

## 근거 커밋

- 이 보고서를 담은 `Task(XIX.7.2)` 커밋(브랜치 `lane/agent-history`).
