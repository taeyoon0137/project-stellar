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
| runtime job으로 한 되돌리기가 실제 자동 저장에 기록 | `agent_io::tests::revert_through_the_runtime_job_is_autosaved` | 통과(상태 게시 뒤 `Pending`, 다음 generation 기록 뒤 clean) |
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

## 추가 작업: 앱 시작 세션 연결과 host trait (ADR-0124 결정 1·2, lane `agent-runtime-2`)

위 "남은 위험"의 앞 세 항목(AgentSession 종료 효과 연결, 두 `AgentSessionId` 대응, `FirstConnectionApproval` 기록)과 runtime의 agent-protocol host trait 구현을 했습니다. 셸 FFI는 여전히 범위 밖이라 행은 부분 완료입니다.

### 변경 파일

- `crates/runtime/src/agent_session.rs`·`agent_session_tests.rs`(신규): `HostedSessions`(agent-host id ↔ 편집 id, AgentSession 상태), `AgentWindow::{start_hosted, host_event, change_hosted_mode, take_host_effects}`, `end_reason`, `end_event`, `fresh_id`, `new_edit_session_id`, `LinkError`, `HostEffectReport`
- `crates/runtime/src/agent_edit.rs`: `WindowAgentEdit.hosted`, `AgentEditError::Link`, 창 닫기에서 앱 시작 세션은 `UserStop` event로 끝냄
- `crates/agent-edit/src/agent_history/mod.rs`: `EndReason::{UsageLimitReported, AuthErrorReported}`
- `crates/agent-edit/src/permission.rs`·`permission_tests.rs`: `PermissionStore::revoke`
- `crates/agent-protocol/src/host.rs`(신규): `AttachedHost`, `HostRefusal`
- `crates/runtime/src/agent_attached.rs`·`agent_attached_tests.rs`(신규): `AppAgentHost`(`AttachedHost` 구현), `AgentConnection`, `BoundConnection`, `refusal`
- `crates/runtime/Cargo.toml`, `Cargo.lock`, `tools/xtask/src/crate_graph.rs`, `docs/architecture/crate-dependency-graph.md`: runtime 직접 의존 agent-protocol·local-ipc
- `docs/adr/ADR-0124-agent-runtime-host-link-frames-and-observation.md`(신규)

### 구현 요약

- **id 대응**: 창의 `HostedSessions` 한 표가 agent-host `AgentSessionId(u64)`와 편집 `AgentSessionId(StableId)`를 1:1로 잇습니다. 편집 id는 `new_edit_session_id`(UUIDv7)가 만듭니다.
- **시작과 첫 연결 승인**: `start_hosted`가 시작 확인(`StartedSession`)의 프로젝트·창을 이 창과 대조하고, 편집 세션을 `AppStarted`로 시작한 뒤 `FirstConnectionApproval`을 권한 저장소의 첫 연결 승인으로 기록합니다.
- **효과 실행**: 종료 효과 묶음은 세션 관리자의 종료 절차 하나로, 정상 완료 효과는 `complete` 하나로, 모드 낮춤은 `PermissionStore::revoke`로 실행합니다. 효과의 batch 수·lease 목록은 쓰지 않고 편집 쪽 기록을 기준으로 합니다. 프로세스 종료·실행 흔적 정리·승인 화면 닫기·결합 해제·진단은 창 대기열로 넘깁니다. 편집 쪽이 먼저 끝낸 세션은 같은 이유의 event로 AgentSession 상태를 맞춥니다.
- **host trait**: `AttachedHost`(`listed_tools`, `permits`, `submit`, 기본 `callable_tool`)를 runtime `AppAgentHost`가 구현합니다. 내장 세션은 agent-host `builtin_tools`·`grant_route`(모드 상한, 터미널·파일 삭제 제외), 외부 클라이언트는 attached catalog이고, 접수는 `AgentIo::submit`입니다. 거부는 ADR-0085 결과 종류입니다.

### 검증

| 명령 / test | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib agent_` | 통과 |
| `cargo test -p stellar-agent-edit` | 통과 |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |
| `cargo test -p xtask --lib crate_graph` | 통과 |
| `start_records_first_connection_and_maps_both_ids` | 통과(첫 연결 승인 기록, 양방향 대응, 중복 연결 거부) |
| `start_for_another_project_or_window_changes_nothing` | 통과 |
| `process_exit_runs_the_session_end_procedure_once` | 통과(중단됨 항목 1, lease 0, 권한 폐기, 자동 rollback 없음, 문서 밖 효과 대기열, 끝난 세션 재실행 없음) |
| `edit_completed_and_mode_lowering_reach_the_session_manager` | 통과 |
| `closing_the_window_stops_hosted_sessions_with_process_termination` | 통과 |
| `reasons_map_with_the_same_diagnostic_codes` | 통과 |
| `builtin_sessions_follow_the_mode_cap`, `external_clients_use_the_attached_catalog_and_unbound_is_refused`, `refusals_use_the_result_kinds` | 통과 |
| FFI smoke 세션 handle 누수 0 | 확인 안 함(FFI 범위 밖) |

### 남은 위험

- 셸 FFI 확장(세션·승인·채팅·출처·되돌리기 함수, 에이전트 event message)은 사용자 방향(2026-11-23)에 따라 하지 않았습니다.
- 문서 밖 효과 대기열(`take_host_effects`)을 꺼내 프로세스를 끝내는 호출자(프로세스 관리자)와 승인 화면은 아직 없습니다.
- 모드 변경은 document thread(`change_hosted_mode`)와 연결 I/O 쪽 host(`AppAgentHost::set_mode`)에 각각 알려야 합니다. 둘을 함께 부르는 앱 UI 동작은 UI 재설계 때 만듭니다.
