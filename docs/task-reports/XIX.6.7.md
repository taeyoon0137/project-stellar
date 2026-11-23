# 작업 보고서: XIX.6.7 에이전트 변경 출처 색인 AgentChangeProvenance를 창 세션 메모리에 정의

- WBS: XIX.6.7
- 작업명: 에이전트 변경 출처 색인 AgentChangeProvenance를 창 세션 메모리에 정의
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 XIX.5.5이 `[x]`가 아닙니다. 그 조건이 풀리면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]`)
- 선행: XIX.5.5, VII.1.1, VII.1.4, VII.4.1
- 결정: [ADR-0100](../adr/ADR-0100-agent-change-history-and-revert.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 셸 UI와 FFI는 범위가 아닙니다(2026-11-23 엔진 우선 방향).

## 변경 파일 목록

- `crates/mcp-core/src/provenance.rs`(신규) — `AgentChangeProvenance`, `BatchProvenance`, `BatchOutcome`, `DisplayState`.
- `crates/mcp-core/src/provenance_tests.rs`(신규), `crates/mcp-core/src/provenance_fixture.rs`(신규, test 전용).
- `crates/mcp-core/src/lib.rs` — module 등록.

## 구현 요약

- 적용 batch마다 AgentSessionId, CLI 세션 표시 이름, batch 순번(세션별 1부터), lease, 그룹 history sequence, `AgentBatchRecord`(change set의 StableId·PropertyPath, 생성·삭제 node, 지문), 결과(진행 중·완료·중단)를 기록합니다.
- batch command 출처는 VII.1.1의 `FutureAgent`이고 VII 타입은 바꾸지 않았습니다.
- 직렬화 경로가 없고 `close_window`로 창과 함께 버립니다. ADR-0093처럼 지금은 mcp-core에 두고 XXIX.3.1 에이전트 편집 구현 crate로 옮깁니다(ADR-0100 결정 3).

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
| batch→StableId·PropertyPath·생성 node 매핑 정확(`batch_maps_to_exact_stable_ids_and_property_paths`) | 통과 |
| 저장 bytes에 세션 표시 이름·세션 id·lease id 0, 창 닫기 후 색인 0(`provenance_is_not_in_saved_bytes_and_is_dropped_with_window`) | 통과 |
| 진단 기록에 출처 필드 없음(`interrupted_diagnostic_has_only_session_duration_and_code`) | 통과 |

## 차이(작업 지시와 다른 점)

- recovery snapshot(V.9.2)·텔레메트리는 이 crate에서 닿지 않아, 출처 타입에 직렬화 경로가 없음과 저장 bytes 검사로 대신했습니다.

## 남은 위험 / 한계

- runtime은 mcp-core에 닿지 못하므로 앱 창이 이 색인을 소유하는 연결은 XXIX.3.1에서 합니다.

## 근거 커밋

- 이 보고서를 담은 `Task(XIX.6.7)` 커밋(브랜치 `lane/agent-history`).

## 추가 작업: 사용자 편집을 앱 창 색인에 연결 (ADR-0124 결정 4, lane `agent-runtime-2`)

색인은 ADR-0112로 앱 창(`WindowAgentEdit`)이 소유합니다. 이번에는 사용자 편집이 표시된 batch의 "이후 변경됨"을 바로 바꾸게 했습니다.

### 변경 파일

- `crates/runtime/src/agent_observe.rs`·`agent_observe_tests.rs`(신규): 창 관찰 대기열 `AgentWatch`, `drain`, `drain_windows`
- `crates/runtime/src/window_session.rs`: 문서 기록 함수(`apply`, `apply_grouped`, `insert_grouped_and_select`, `insert_and_select`, `apply_and_select`, 외부 반영)가 change set과 출처를 대기열에 넣음, `end_preview`가 마지막 preview change set을 받음
- `crates/runtime/src/app/inspector.rs`: 속성 scrub 끝에서 마지막 change set을 `end_preview`에 넘김
- `crates/runtime/src/app.rs`: 상태 snapshot 게시(`build_state`) 앞에서 `drain_windows`
- `crates/runtime/src/agent_edit.rs`: 대기열이 쓰는 창 상태 field 공개 범위(`pub(crate)`)

### 구현 요약

- 기록 함수 한 층에서 change set을 모으고, 모든 document thread job 뒤 상태 게시 앞에서 창마다 `observe`에 넘깁니다. 호출 지점마다 관찰을 따로 부르지 않아 빠지는 편집 경로가 없습니다.
- 표시된 batch가 없으면 쌓지 않습니다(대기열을 비울 때 켜짐 여부를 다시 맞춤).
- 속성 scrub은 preview 중 쌓지 않고 끝에서 한 번 넣습니다. 매 단계 시작 상태에서 다시 계산하므로 마지막 change set이 제스처 전체입니다. 캔버스 drag는 끝에서 `apply` 한 번이라 같은 경로입니다.
- Undo·Redo 재생과 에이전트 batch 적용·되돌리기는 지금처럼 그 자리에서 판정하므로 대기열에 넣지 않습니다(두 번 판정하지 않음).

### 검증

| 명령 / test | 결과 |
|---|---|
| `cargo test -p stellar-runtime` | 통과 |
| `user_edits_reach_the_provenance_index_at_state_publication` | 통과(표시 batch 없을 때 대기열 0, 사용자 편집 뒤 이후 변경됨, 되돌리면 검토 대기, 무관한 node는 그대로) |
| `history_preview_is_observed_once_at_the_end` | 통과(preview 중 대기열 0, 끝에서 1) |

### 남은 위험

- `canvas::select_session`·`canvas::text::edit`처럼 `WindowHistory`를 직접 받는 캔버스 엔진 부품은 창 session 기록 함수를 거치지 않습니다. 지금 앱 경로에서는 쓰이지 않지만, UI 재설계 때 이 부품을 앱 경로에 연결하면 기록 함수 층을 거치게 해야 합니다.
