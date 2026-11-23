# 작업 보고서: XXIX.3.4 앱 Agent Host·IPC I/O 스레드

- WBS: XXIX.3.4
- 작업명: 문서를 쓰지 않는 에이전트 I/O 스레드가 검증한 요청을 사용자 입력보다 낮은 priority로 document thread에 넣고 넘치면 busy, 큰 batch는 단일 점유 상한 안에서 나눠 적용
- 상태: 부분 완료 (`[~]`) — 스레드·검증·priority·busy 코어(ADR-0112), bridge frame 앞단, 큰 batch 분할, 20만 node 단일 점유 측정은 끝났습니다. 문서 대부분을 덮는 lease의 판정 비용(아래 "남은 위험")과 bridge 쪽 frame 쓰기·연결 감시 연결이 남았습니다.
- 선행: XXIX.3.1, XII.1.2, XIX.4.2
- 설계 기준: ADR-0112 결정 5, ADR-0124 결정 3·6, ADR-0005 §8, 아키텍처 §6.3(UI·문서 스레드 단일 점유 2 ms)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

1차(ADR-0112, lane `agent-edit-crate`):

- `crates/runtime/src/agent_io.rs`·`agent_io_tests.rs`(신규): `AgentIo`, `AgentIoThread`, `AgentRequest`, `AgentReply`, `AgentIoError`, `DEFAULT_IN_FLIGHT_LIMIT`, `DEFAULT_INBOX_CAPACITY`, `AppRuntime::{agent_io, agent_io_with_limit}`
- `crates/agent-edit/src/attached/edit.rs`: 문서 없이 만드는 `decode_proposal`·`DecodedProposal`

2차(ADR-0124, lane `agent-runtime-2`):

- `crates/local-ipc/src/frame.rs`·`frame_tests.rs`(신규): handshake 뒤 bridge↔app binary frame(`Frame`, `read_frame`, `write_frame`, `encode`, `decode`, `MAX_FRAME_BYTES`)
- `crates/runtime/src/agent_frame.rs`·`agent_frame_tests.rs`(신규): frame → `AgentRequest` 앞단 `AgentFrontEnd`(`convert`, `handle`, `serve`, `connect_external`, `bind_hosted`, `disconnected`), `reply_frame`
- `crates/runtime/src/agent_io.rs`: lease 요청·제안 전 첫 연결 승인 확인(`AgentIoError::ConnectionNotApproved`), 끝낸 앱 시작 세션을 AgentSession에 알림
- `crates/agent-edit/src/agent_history/mod.rs`·`tests.rs`: `AgentEditSessions::apply_proposal_chunk`, `ChunkApplied`(기존 `apply_proposal`은 나누지 않는 같은 함수)
- `crates/runtime/src/agent_edit.rs`: `AgentWindow::apply_proposal_chunk`, `AppRuntime::apply_agent_proposal`, `PROPOSAL_BATCH_COMMANDS`, `ProposalApplied`, `AgentApplyError`
- `crates/agent-edit/src/occupancy_trace.rs`(신규, release 측정 test), `crates/agent-edit/src/lib.rs`(등록)

## 구현 요약

- **문서를 쓰지 않음·검증·priority·busy**(1차): I/O 스레드는 document thread job 접수기만 갖고, proposal은 닫힌 enum으로 바꾼 뒤에만 `Command` lane에 넣습니다. 처리 전 요청 상한(64)이나 lane이 차면 `Busy`입니다. `base_revision`이 창의 저장 generation과 다르면 `StaleBase`입니다.
- **frame 앞단**(2차): local-ipc frame(`"SF"`, version 1, 상한 1 MiB, fail closed)을 runtime 앞단이 `AgentRequest`로 바꿉니다. 값 해석은 기존 공개 함수(`ProjectRevision::from_str`, `ExternalKind::from_name`, `CommandInput::Canonical` → `decode_commands`)만 씁니다. lease·transaction·완료 항목 id는 앱이 만들고, 연결마다 접수된 lease를 기억합니다. 권한 상한은 host trait(XXIX.3.1)로 먼저 봅니다. `serve`가 연결 하나를 읽어 응답 frame을 쓰고, 끝나면 `Disconnected`를 넣습니다.
- **첫 연결 승인 확인**(2차): 승인 전 `BeginEdit`·`Propose`는 document thread에서 거부합니다(XIX.3.3).
- **큰 batch 분할**(2차): 승인한 제안을 `PROPOSAL_BATCH_COMMANDS`(512) command씩 batch로 나눠 batch 하나를 job 하나로 실행합니다. 남으면 다음 batch job을 `Command` lane 뒤에 다시 넣으므로 그 사이 `Input` lane 사용자 입력이 먼저 처리됩니다. batch마다 출처 기록이 생기고 완료는 history 항목 하나라 Undo 한 번이 제안 전체를 되돌립니다. batch가 거부되면 남은 부분을 버리고 앞 batch는 표시와 함께 남습니다.
- **lease 대상 계산**: `LeaseTargets::resolve`는 대상 root마다 조상 사슬만 봅니다. 아래 측정에서 나눌 필요가 없어 나누지 않았습니다.

## 단일 점유 측정 (20만 node, release, 개발 장비)

`cargo test --release -p stellar-agent-edit --lib occupancy_trace -- --ignored --nocapture`. 문서는 `stellar_ops::test_support::large_document(200_000)`(root 아래 group 400개, group마다 Fragment 499개)입니다. 다른 lane 빌드가 함께 도는 부하 상태에서 잰 값입니다.

| 측정 | 값 |
|---|---|
| lease 대상 확인 `resolve`(root 1개) | 39~55 µs |
| lease 대상 확인 `resolve`(group 400개) | 372~377 µs |
| lease 판정 snapshot(lease root = 문서 root, 20만 node subtree) | 10.9~11.0 ms |
| lease 판정 snapshot(group 400개, 약 20만 node) | 13.5~13.6 ms |
| batch 적용 + 출처 기록, lease = group 하나(500 node): 1 / 64 / 256 / 499 command | 33 / 114 / 379 / 694 µs |
| batch 적용 + 출처 기록, lease = 문서 root: 1 / 1024 / 4096 / 10000 command | 10.6 / 11.9 / 16.5 / 25.5 ms |

- lease가 작은 subtree이면 command 하나가 약 1.4 µs(적용 0.9 + 출처 기록 0.5)이고 512개 batch가 약 0.7 ms입니다. 분할 뒤 batch job 하나는 2 ms 안입니다.
- lease가 문서 대부분을 덮으면 batch 크기와 관계없이 `LeaseTable::snapshot`이 subtree 전체를 걸어 약 10 ms입니다. 분할로 해결되지 않습니다(남은 위험).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-local-ipc --lib frame` | 통과 |
| `cargo test -p stellar-agent-edit` | 통과(측정 test 1개 ignored) |
| `cargo test -p stellar-runtime --lib agent_` | 통과 |
| `cargo test --release -p stellar-agent-edit --lib occupancy_trace -- --ignored --nocapture` | 위 표 |

| 검증 기준 | test | 결과 |
|---|---|---|
| 스레드는 문서를 쓰지 않음 | `io_thread_queues_validated_requests_without_writing_the_document` | 통과 |
| 사용자 입력보다 낮은 priority | `agent_requests_run_after_user_input` | 통과 |
| 넘치면 busy | `in_flight_limit_returns_busy_without_waiting` | 통과 |
| frame 해석 fail closed·상한 | `malformed_frames_fail_closed`, `oversized_frames_are_not_written`, `frame_limit_matches_the_mcp_message_limit` | 통과 |
| frame → 요청, 첫 연결 승인 전 거부, 기준 revision 대조 | `frames_become_agent_requests_through_the_host` | 통과 |
| 연결 하나 처리와 응답 frame, 연결 종료 시 종료 절차 | `serve_reads_frames_and_writes_replies_until_the_bridge_closes` | 통과 |
| batch 적용을 나눠 처리 | `large_approved_proposal_is_applied_in_batch_jobs`, `large_proposal_is_applied_in_batches_and_completes_as_one_entry`, `rejected_batch_drops_the_rest_and_keeps_earlier_batches` | 통과 |
| 20만 node lease·batch trace에서 UI·Document 단일 점유가 예산 이내 | `occupancy_trace_200k`(release 측정) | 작은 lease는 예산 이내, 문서 전체 lease는 초과(남은 위험) |
| 사용자 입력 지연 증가가 예산 이내 | 없음 | 측정 안 함(앱 입력 경로 trace는 UI 재설계 뒤) |

## 차이(작업 지시와 다른 점)

- XII.1.2 queue에 에이전트 전용 lane을 더하지 않고 처리 전 요청 상한과 batch job 재접수로 사용자 입력 자리를 지켰습니다(ADR-0112 결정 5, ADR-0124 결정 6).
- 제안 단위 원자성 대신 batch 단위 원자성입니다. 제안 전체를 사본에 먼저 적용해 검증하면 20만 node 사본 하나가 단일 점유 상한을 넘기 때문입니다(ADR-0124 Alternatives).
- 원문 버퍼링·표시용 텍스트 준비는 `stellar-agent-host`(XXIX.2.x)에 있고 이 스레드에 연결하지 않았습니다.

## 남은 위험 / 한계

- lease가 문서 대부분을 덮으면 `LeaseTable::snapshot`(commands `lock::lease`, 이 lane 소유 밖)이 batch마다 약 10 ms입니다. 변경 node의 조상만 보는 판정으로 바꾸는 일이 필요합니다. 그 lease가 있는 동안 lease 판정을 거치는 사용자 편집도 같은 비용입니다.
- bridge 쪽(mcp-core attached backend, XIX.9.4)이 frame을 쓰고, 앱 연결 감시(`stellar_local_ipc::session`)가 data를 받으면 닫는 대신 앞단 `serve`로 넘기는 연결이 남았습니다.
- 사용자 입력 지연 증가는 재지 않았습니다. 처리 전 요청 상한(64)과 batch 크기(512)는 위 측정만 근거입니다. 기준 장비(M1 16GB)에서 다시 재야 합니다.
