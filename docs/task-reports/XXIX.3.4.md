# 작업 보고서: XXIX.3.4 앱 Agent Host·IPC I/O 스레드

- WBS: XXIX.3.4
- 작업명: 문서를 쓰지 않는 에이전트 I/O 스레드가 검증한 요청을 사용자 입력보다 낮은 priority로 document thread에 넣고 넘치면 busy, 큰 batch는 단일 점유 상한 안에서 나눠 적용
- 상태: 부분 완료 (`[~]`) — 스레드·검증·priority·busy 코어(ADR-0112), bridge frame 앞단, 큰 batch 분할, 20만 node 단일 점유 측정은 끝났습니다. 문서 대부분을 덮는 lease의 판정 비용과 bridge 쪽 frame 쓰기·연결 감시 연결은 lane `agent-lease-perf`가 고쳤습니다(아래 추가 작업 1·2). 사용자 입력 지연 측정, MCP 서버 루프의 link 연결, 앱 listener 연결 수명 코드가 남았습니다.
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
| lease 대상 확인 `resolve`(root 1개) | 39–55 µs |
| lease 대상 확인 `resolve`(group 400개) | 372–377 µs |
| lease 판정 snapshot(lease root = 문서 root, 20만 node subtree) | 10.9–11.0 ms |
| lease 판정 snapshot(group 400개, 약 20만 node) | 13.5–13.6 ms |
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
| batch 적용을 나눠 처리, batch 사이에 사용자 입력·다른 job이 먼저 실행 | `large_approved_proposal_is_applied_in_batch_jobs`, `large_proposal_is_applied_in_batches_and_completes_as_one_entry`, `rejected_batch_drops_the_rest_and_keeps_earlier_batches` | 통과 |
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

## 추가 작업 1: 변경 node의 조상만 보는 lease 판정 (ADR-0132 결정 1, lane `agent-lease-perf`)

위 "남은 위험" 첫 항목을 고쳤습니다. 상태는 부분 완료(`[~]`) 그대로입니다(사용자 입력 지연 증가는 앱 입력 경로가 없어 재지 못함).

### 변경 파일

- `crates/commands/src/lock/lease.rs`: `LeaseTable::snapshot(before, changes)`가 바뀐·삭제 node의 조상 사슬만 걸어 적용 전 소속을 만듭니다. lease root의 진조상은 부모·순서 변경이 있을 때만 걷습니다. `execute_leased`는 적용 뒤 역 transaction을 `TransactionGuard` 안에서 적용해 적용 전 문서를 잠깐 되살리고, 소속을 기록한 뒤 guard journal로 되돌립니다(문서 복제 없음). 그 단계가 실패하면 판정하지 않고 거부합니다(`ViolationReason::Unjudged`). 미리보기는 손대지 않은 원본 문서로 소속을 만듭니다.
- `crates/commands/src/lock/lease_tests.rs`: 옛 subtree 걷기 판정을 비교 기준으로 둔 전수 대조 test.
- 검토 보완: guard 되돌리기마저 실패하면 문서가 적용 후 상태라는 보장이 없으므로 역 transaction을 다시 얹지 않고 `LeasedExecutionError::Restore`(위반 `Unjudged`, 원인 `CommandError::RollbackFailed`)로 알립니다. 역 transaction 적용만 실패하면 guard를 명시적으로 되돌린 뒤 `Unjudged`로 거부합니다(drop 경로의 조용한 되돌리기 실패 없음).
- `crates/agent-edit/src/occupancy_trace.rs`: 새 판정 입력, 512 command batch, lease 안 사용자 편집 거부 측정을 더했습니다.

### 단일 점유 측정 (20만 node, release, 개발 장비, 다른 lane 빌드가 함께 도는 부하 상태)

`cargo test --release -p stellar-agent-edit --lib occupancy_trace -- --ignored --nocapture`를 여섯 번 돌린 값입니다. 부하로 크게 튄 값(같은 실행의 다른 항목보다 2배 이상, 예: 4096 command 14 ms)은 범위에서 뺐습니다. group 400개 판정 입력은 진조상을 부모·순서 변경 때만 걷도록 고친 뒤 두 번 잰 값입니다(고치기 전 279–353 µs).

| 측정 | 전(위 표) | 후 |
|---|---|---|
| lease 판정 입력(lease root = 문서 root, 변경 node 1개) | 10.9–11.0 ms(subtree 전체) | 23–104 µs |
| lease 판정 입력(group 400개, 변경 node 1개) | 13.5–13.6 ms | 53–64 µs |
| batch 적용 + 출처 기록, lease = 문서 root: 1 / 512 / 1024 / 4096 command | 10.6 / – / 11.9 / 16.5 ms | 0.03–0.11 / 1.1–1.2 / 2.2–2.4 / 8.8–9.3 ms |
| batch 적용 + 출처 기록, lease = group 하나(500 node): 64 / 256 / 499 command | 114 / 379 / 694 µs | 127–148 / 520–566 / 980–1,100 µs |
| lease 안 사용자 편집 하나(이름 변경, 거부: 적용·판정·되돌리기) | 약 11 ms(snapshot) | 71 µs |

- 문서 전체 lease에서도 분할 단위(`PROPOSAL_BATCH_COMMANDS` 512) batch job 하나가 약 1.1–1.2 ms로 단일 점유 상한 2 ms 안입니다. 판정 비용이 batch 크기에 비례하므로 분할로 해결됩니다.
- 역 transaction을 guard 안에서 한 번 더 적용하므로 작은 lease의 batch는 약 1.4–1.6배가 됐습니다(499 command 0.69 → 0.98–1.10 ms). 상한 안이고 분할 단위를 바꾸지 않았습니다.

### 검증

| 명령 / test | 결과 |
|---|---|
| `cargo test -p stellar-commands --lib lock::` | 통과 |
| `ancestor_walk_judgement_matches_the_subtree_walk_for_every_case` | 통과: lease 배치 12가지 × command(이름·삭제·이동·추가·여러 command 묶음) × 주체 4가지(사용자, 두 lease, 없는 lease) 1,000건 이상에서 판정 결과·거부 뒤 문서·revision이 옛 판정과 같고, 소속 표가 옛 표의 변경 node 부분과 같음 |
| 기존 lease test(사용자 편집 matrix, 범위 밖 거부, 새 자손 자동 잠금, 시스템 출처 touched_leases, 미리보기) | 통과 |
| `occupancy_trace_200k`(release) | 위 표 |

## 추가 작업 2: 연결 감시가 frame을 앞단에 넘기고, bridge 쪽이 frame을 씀 (ADR-0132 결정 2·3, lane `agent-lease-perf`)

위 "남은 위험" 둘째 항목의 엔진 부분입니다.

### 변경 파일

- `crates/local-ipc/src/session.rs`: `FrameService` trait, `ConnectionMonitor::serve`(감시 스레드가 handshake 뒤 연결을 서비스에 넘기고 돌아오면 종료 event 한 번), `CloseCause::of_frame_error`. 기존 `watch`는 그대로입니다. frame 연결은 종료 절차를 서비스가 맡아 `SessionTermination`을 받지 않습니다.
- `crates/runtime/src/agent_frame.rs`·`agent_frame_tests.rs`: `FrameConnection`(연결마다 앞단 하나, `ConnectionBinding`대로 묶은 뒤 `AgentFrontEnd::serve`), `ConnectionBinding`.
- `crates/mcp-core/src/attached.rs`·`attached_tests.rs`(신규), `crates/mcp-core/src/lib.rs`(등록): bridge 쪽 `AttachedLink`(요청 frame 쓰기와 같은 id의 응답 하나 읽기, `call_tool`로 MCP tool 결과 만들기), `begin_frame`, `propose_frame`.

### 구현 요약

- 앱: 인증·handshake를 마친 연결을 `ConnectionMonitor::serve`에 `FrameConnection`과 함께 넘기면 감시 스레드가 frame을 읽어 앞단에 넘깁니다. 결합이 거부된 연결(없는 앱 시작 세션 등)은 frame을 읽지 않고 앱 쪽에서 닫습니다(`AppShutdown`). 연결이 끝나면 앞단이 `Disconnected` 하나를 넣어 외부 클라이언트 세션은 종료 절차, 앱 시작 세션은 재연결 유예로 갑니다.
- bridge: tool 인자는 `parse_begin_agent_edit`·`parse_transaction`(기존 함수)으로 해석하고, command는 `decode_commands` 뒤 commands codec canonical bytes로 싣습니다. 앱 응답의 결과 종류 이름을 ADR-0085 `ResultKind`로 읽어 tool 결과로 바꿉니다. 응답 id 불일치·응답 아닌 frame·모르는 결과 종류는 연결을 더 쓰지 않습니다. 입력 오류와 1 MiB frame 상한 초과는 보내기 전에 거부하고 연결을 유지합니다.

### 검증

| 명령 / test | 결과 |
|---|---|
| `cargo test -p stellar-local-ipc --lib session` | 통과: `frame_connection_is_served_and_closes_once`, `malformed_frame_closes_served_connection_as_unexpected_data`, `frame_errors_map_to_close_causes` |
| `cargo test -p stellar-runtime --lib agent_frame` | 통과: `connection_monitor_hands_bridge_frames_to_the_front_end`(실제 IPC handshake 연결, 승인 전 거부 응답, 승인 뒤 lease 요청 응답, bridge 종료 시 event 1회·세션 종료·lease 0), `unbound_hosted_connection_is_closed_by_the_app_without_replies` |
| `cargo test -p stellar-mcp-core --lib attached` | 통과: tool 호출 → frame(canonical bytes가 같은 codec으로 같은 command로 읽힘) → 응답 → tool 결과, fail closed, 보내기 전 입력 거부 |

### 남은 위험

- MCP 서버 루프(`server`)·실행 진입점(`entry`)이 attached 모드에서 `AttachedLink::call_tool`로 `tools/call`을 보내고, bridge의 앱 연결 감시(`attach::watch_app_connection`)를 link의 응답 읽기로 바꾸는 일이 남았습니다(XIX.9.4, mcp-core server·entry 소유). 지금 bridge는 frame을 보내지 않으므로 기존 감시가 그대로 맞습니다.
- 앱 listener 연결 event를 `ConnectionMonitor::serve`·`FrameConnection`에 잇는 앱 수명 코드와 외부 클라이언트가 결합할 창을 고르는 화면은 UI 재설계 뒤입니다.
