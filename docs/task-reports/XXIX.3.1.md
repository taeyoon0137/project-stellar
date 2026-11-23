# 작업 보고서: XXIX.3.1 앱 쪽 에이전트 편집 crate와 runtime 연결(엔진 부분)

- WBS: XXIX.3.1
- 작업명: 에이전트 편집 구현 crate `stellar-agent-edit`를 만들어 mcp-core의 앱 소유 module을 옮기고, runtime이 창 history·Undo·Redo·자동 저장·Undo 메뉴 이름에 연결
- 상태: 부분 완료 (`[~]`) — 엔진 부분(crate, crate graph, runtime 연결)은 끝났습니다. 셸 FFI 확장(세션·승인·채팅·출처·되돌리기 함수, 에이전트 event message)은 사용자 방향(2026-11-23)에 따라 하지 않았습니다.
- 선행: XXIX.2.8, XXIX.2.10, XXIX.2.12, XIX.6.10
- 설계 기준: ADR-0112(신규), ADR-0093 결정 12, ADR-0100 결정 2·3·7, ADR-0005 §3·§8

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/agent-edit/`(신규 crate `stellar-agent-edit`): `Cargo.toml`, `src/lib.rs`, `src/lease.rs`·`src/lease_tests.rs`(신규: `LeaseTargets`, `TargetError`, `AgentEditLeases`, `LeaseState`, `LeaseServiceError`, `open_project_coordinator`)
- mcp-core에서 이름 그대로 옮긴 module: `permission.rs`(+test), `provenance.rs`·`provenance_fixture.rs`·`provenance_tests.rs`, `agent_history/**`, `attached/{mod, edit, edit_tests, revert, revert_tests}.rs`
  - `attached/edit.rs`: JSON 대신 `ApplyRequest`를 받는 `decode_proposal`·`DecodedProposal::transaction`·`proposal_transaction`
  - `agent_history/mod.rs`: `AgentEditSessions::active_ids`
  - `agent_history/inventory_tests.rs`: XIX.8.1 검사 root에 `crates/agent-edit`
- `crates/mcp-core/src/lease.rs`: tool 정의와 인자 해석만 남김(`resolve`·요청 관리 제거), `crates/mcp-core/src/lib.rs`: 옮긴 module 등록 제거
- `crates/runtime/src/agent_edit.rs`·`agent_edit_tests.rs`(신규): `WindowAgentEdit`, `AgentWindow`, `replay`, `HistoryNames`, `AppRuntime::{with_agent_edit, agent_history_names}`
- `crates/runtime/src/app.rs`: 창 표 항목 `WindowEntry.agent`, Undo·Redo를 `agent_edit::replay`로, 창·프로젝트 닫기 때 `agent_edit::close_window`, 상태를 게시하는 job 접수기 `JobPublisher`
- `crates/runtime/src/window_session.rs`: `replay_history`(재생 항목과 change set을 돌려줌, `undo`·`redo`가 부름)
- `crates/runtime/Cargo.toml`, `Cargo.toml`, `Cargo.lock`, `crates/runtime/src/lib.rs`: crate·의존·module 등록
- `tools/xtask/src/crate_graph.rs`, `tools/xtask/src/crate_graph/reach.rs`, `docs/architecture/crate-dependency-graph.md`: 허용 edge와 `FORBIDDEN_REACH`
- `crates/agent-host/tests/immutable_paths.rs`: `APP_SIDE_MODULES` 예외 삭제
- `docs/adr/ADR-0112-app-agent-edit-crate-and-runtime-wiring.md`(신규)

## 구현 요약

- **crate 경계**: runtime은 mcp-core에 닿을 수 없어서(ADR-0005 §8) 앱 소유 상태를 새 crate로 옮겼습니다.
  - 허용 직접 의존은 ops와 agent-protocol이고, runtime이 이 crate에 의존합니다.
  - 에이전트 경로 crate(mcp-core, stellar-mcp, agent-protocol, local-ipc, stellar-cli, agent-host, ops)는 이 crate에 직접·전이로 닿지 않습니다. 이 crate도 mcp-core, stellar-mcp, local-ipc, agent-host, runtime, ffi에 닿지 않습니다.
- **브리지와 앱의 경계**: mcp-core에는 `document.begin_agent_edit` tool 정의와 인자 형식 해석만 남았습니다. 문서 기준 대상 확인과 요청 관리는 `stellar_agent_edit::lease` 한 곳입니다. proposal은 브리지 해석 결과(`ApplyRequest`)를 같은 `decode_commands`로 닫힌 enum으로 바꿉니다.
- **창 상태**: `WindowAgentEdit`가 창마다 세션 관리자, 출처 색인, `AgentEntryLabels`, 권한 저장소, lease 요청 관리자를 갖습니다. 앱 창 표 항목의 필드라 창과 함께 버립니다. `AgentWindow`가 `WindowAgentHistory{window, doc, labels}`를 만들어 완료·종료 절차와 되돌리기에 씁니다.
- **Undo·Redo**: 앱 `edit.undo`·`edit.redo`는 `agent_edit::replay`를 거칩니다. 재생한 항목 sequence로 `on_history`를 부르고, 재생 change set을 그 항목 출처로 `observe`에 넘깁니다.
- **다른 세션 batch**: 승인한 제안을 적용하면 그 change set을 batch 출처로 `observe`에 넘겨 다른 세션의 표시된 batch를 다시 판정합니다(같은 세션은 건너뜀).
- **되돌리기와 자동 저장**: 되돌리기는 작업 문서 revision을 올립니다. 사용자 UI 동작은 `AppRuntime::with_agent_edit`(job 뒤 상태 게시)로 실행되어 자동 저장 예약이 그 revision을 관찰합니다.
- **Undo 메뉴 이름**: `AppRuntime::agent_history_names`가 `AgentEntryLabels::action_name`으로 Undo·Redo 맨 위 항목 이름을 냅니다.
- **창 닫기**: 남은 세션을 사용자 중지로 끝내 lease·권한을 풀고 출처 색인을 버립니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-agent-edit` | 40 passed |
| `cargo test -p stellar-mcp-core` | 64 passed |
| `cargo test -p stellar-runtime --lib agent_` | 9 passed(agent_edit 6, agent_io 3) |
| `cargo test -p stellar-agent-host --test immutable_paths` | 4 passed |
| `cargo test -p xtask --lib crate_graph` | 47 passed |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |
| `cargo xtask generate-ffi-header --check` | drift 없음 |
| `cargo clippy -p stellar-runtime -p stellar-agent-edit -p stellar-mcp-core -p xtask -p stellar-agent-host --all-targets --locked -- -D warnings` | 경고 0 |

| 검증 기준 | test | 결과 |
|---|---|---|
| `cargo xtask validate-crate-graph` 통과 | `validate-crate-graph`, `agent_paths_never_reach_the_app_agent_edit_crate` | 통과 |
| stellar-ffi 직접 의존이 core-types·runtime뿐 | crate graph 정책(변경 없음) | 통과 |
| 생성 header drift 없음 | `generate-ffi-header --check` | 통과(FFI 변경 없음) |
| 에이전트 경로의 출처 표시 정리·되돌리기 호출 0(예외 없이) | `agent_paths_do_not_call_the_app_agent_edit_modules`, `agent_paths_cannot_write_immutable_state` | 통과 |
| 창 Undo·Redo 뒤 표시 전이 | `completed_group_undo_redo_moves_provenance_and_names`, `user_edit_undo_redo_is_observed_as_changed_later`, `io_thread_queues_validated_requests_without_writing_the_document`(앱 `edit.undo`) | 통과 |
| 되돌리기 change set이 자동 저장 입력 | `revert_is_one_history_entry_and_feeds_autosave` | 통과(revision 증가, 저장 전 변경) |
| Undo 메뉴 이름 | `completed_group_undo_redo_moves_provenance_and_names`, `ending_and_closing_release_leases_and_permissions` | 통과 |
| 다른 세션 batch가 같은 속성을 바꾸면 이후 변경됨 | `another_session_batch_on_the_same_property_is_changed_later` | 통과 |
| 창 닫기 때 lease·권한 해제 | `ending_and_closing_release_leases_and_permissions` | 통과 |
| FFI smoke 세션 handle 누수 0 | 없음 | 확인 안 함(FFI 범위 밖) |

## 차이(작업 지시와 다른 점)

- 계획서 행의 FFI 확장과 FFI smoke는 하지 않았습니다(사용자 방향 2026-11-23). runtime의 agent-protocol host trait 구현도 하지 않았습니다.
- crate graph 정책의 runtime 허용 의존에는 이 crate만 더했습니다. agent-protocol·ops는 이미 전이로 닿고(agent-host, ops 직접 edge) 직접 쓸 곳이 없습니다.
- lease 소유 파일 목록 밖인 `crates/mcp-core/src/lease.rs`·`permission*`, `tools/xtask/src/crate_graph*`, `docs/architecture/crate-dependency-graph.md`, `crates/runtime/src/app.rs`·`window_session.rs`를 고쳤습니다. 작업 정의가 lease 관리 이동, crate graph·문서 갱신, Undo·Redo 연결을 요구하기 때문입니다.

## 남은 위험 / 한계

- 사용자 편집 경로마다 change set을 `observe`에 넘기는 연결은 아직 없습니다. 지금은 Undo·Redo 재생과 다른 세션 batch 적용만 표시 상태를 다시 판정합니다. 사용자가 표시된 속성을 직접 고친 경우 "이후 변경됨"은 그 편집을 Undo·Redo하거나 되돌리기 미리보기를 만들 때 드러납니다.
- `stellar-agent-host` `AgentSession` 종료 효과를 이 세션 관리자에 잇는 일, 두 `AgentSessionId`(u64와 StableId)의 대응, `FirstConnectionApproval`을 권한 저장소 첫 연결 응답으로 넣는 일이 남았습니다.
- IPC frame에서 `BeginAgentEdit`·proposal을 앱 입력(`LeaseTargets`, `ApplyRequest`)으로 바꾸는 앞단이 없습니다.
- 되돌리기 미리보기 validation의 컴포넌트 정의 목록은 호출자가 줍니다. 창 컴포넌트 원본 cache와의 연결은 후속입니다.
