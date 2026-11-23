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

## 추가 작업: 앱 창의 외부 변경 반영·reconciliation에 연결 (ADR-0124 결정 5, lane `agent-runtime-2`)

XIII.3.4 semantic reconciliation(ADR-0110)이 창 session에 생겨, 그 경로에 lease 겹침 판정을 이었습니다.

### 변경 파일

- `crates/agent-edit/src/agent_history/external.rs`·`external_tests.rs`: `respond_to_external`은 문서를 바꾸지 않고 lease 겹침 판정과 겹친 세션 종료만 합니다. 창 반영 경로와 겹치던 반영 transaction(`apply`)과 `divergent`를 지웠습니다. 분할·semantic conflict 후보는 `analysis`에 판정 근거로 남습니다.
- `crates/runtime/src/agent_observe.rs`·`agent_observe_tests.rs`: 외부 변경 관찰값(base 원본, 외부 원본)을 비울 때 `respond_to_external`로 겹친 세션을 `LeaseConflict`로 끝내고 앱 시작 세션이면 AgentSession에 알립니다.
- `crates/runtime/src/window_session.rs`: 창 세션이 lease를 쥘 때만 외부 변경 관찰값을 넣습니다. clean 반영은 지금 문서와 외부 원본, 미저장 편집의 reconciliation은 LoadedBase와 외부 원본이고, 병합하지 못해 두 선택지로 멈춘 경우에도 넣습니다.

### 구현 요약

- 외부 변경을 창 문서에 넣는 경로는 창 session 하나입니다(clean이면 `ApplyExternalSnapshot`, 미저장 편집이면 reconciliation, 겹치면 두 선택지). 적용된 에이전트 변경은 그 경로에서 출처 표시를 유지한 로컬 변경으로 다뤄지고, 같은 속성을 서로 다르게 바꾸면 reconciliation이 semantic conflict로 멈춥니다.
- lease와 겹치면 반영 여부와 관계없이 그 세션을 종료 절차로 끝냅니다(자동 rollback 없음, lease 해제, 대기 제안 버림). 겹치지 않으면 세션과 lease를 유지합니다(XIX.7.3).
- 반영한 외부 변경 change set은 System 출처로 출처 색인 `observe`에 넘깁니다(XIX.6.7 관찰 대기열).

### 검증

| 명령 / test | 결과 |
|---|---|
| `cargo test -p stellar-agent-edit` | 통과 |
| `cargo test -p stellar-runtime --lib agent_observe` | 통과 |
| `overlapping_external_change_ends_the_session_and_keeps_agent_work`(실제 `stellar apply` 외부 writer) | 통과: reconciliation 두 선택지(semantic conflict 경로), `LeaseConflict` 종료, lease 0, 문서 revision 그대로, 에이전트 변경 자동 삭제 0, 출처 표시 유지 |
| `non_overlapping_external_change_keeps_the_session` | 통과: 외부 변경 반영, 세션·lease 유지, 표시 그대로 |
| `overlapping_external_change_ends_session_without_deleting_agent_change`(agent-edit) | 통과 |

### 남은 위험

- 사용자가 두 선택지에서 디스크 쪽(UseLatestDisk)을 고르면 창 문서를 디스크로 다시 엽니다(`reload_latest_disk`). 그때 끝나지 않은 세션과 표시의 처리는 정하지 않았습니다.
- 다른 프로세스가 쥔 겹친 lease는 그 프로세스가 끝낸다고 보고 `foreign_overlaps`로만 알립니다.
