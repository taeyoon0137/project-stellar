# 작업 보고서: XXIX.3.4 앱 Agent Host·IPC I/O 스레드 코어

- WBS: XXIX.3.4
- 작업명: 문서를 쓰지 않는 에이전트 I/O 스레드가 검증한 요청을 사용자 입력보다 낮은 priority로 document thread에 넣고 넘치면 busy
- 상태: 부분 완료 (`[~]`) — 스레드·검증·priority·busy 코어는 끝났습니다. 20만 node 문서 lease·batch trace와 IPC frame 읽기 앞단은 남았습니다.
- 선행: XXIX.3.1, XII.1.2, XIX.4.2
- 설계 기준: ADR-0112 결정 5, ADR-0005 §8, XII.1.2 queue 정책

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/runtime/src/agent_io.rs`·`agent_io_tests.rs`(신규): `AgentIo`, `AgentIoThread`, `AgentRequest`, `AgentReply`, `AgentIoError`, `DEFAULT_IN_FLIGHT_LIMIT`, `DEFAULT_INBOX_CAPACITY`, `AppRuntime::{agent_io, agent_io_with_limit}`
- `crates/runtime/src/lib.rs`: module 등록
- `crates/agent-edit/src/attached/edit.rs`: 문서 없이 만드는 `decode_proposal`·`DecodedProposal`(이 스레드의 검증)

## 구현 요약

- **문서를 쓰지 않음**: `AgentIo`와 `AgentIoThread`(스레드 이름 `stellar-agent-io`, bounded 수신함)는 document thread job 접수기만 갖습니다. 문서·history·출처 색인은 document thread job 안에서만 바뀝니다.
- **이 스레드의 검증**: proposal은 `decode_proposal`(`stellar apply`와 같은 `decode_commands`)로 닫힌 enum command가 된 뒤에만 document thread에 들어갑니다. 바꿀 수 없으면 넣지 않고 `Invalid`로 거부합니다.
- **priority**: 검증한 요청은 XII.1.2 `Command` lane에 들어가 사용자 입력(`Input`) 뒤에 처리됩니다. `Background` lane은 넘칠 때 오래된 항목을 밀어내므로 쓰지 않습니다.
- **busy**: 에이전트 요청은 처리 전 요청 수 상한(기본 64)을 따로 둡니다. `Command` lane 기본 용량 1024보다 작아 사용자 command 자리를 다 쓰지 못합니다. 상한, lane, 수신함 중 하나가 차면 `Busy`로 바로 거부하고 이미 접수한 요청은 버리지 않습니다. 처리하거나 버린 job은 자리를 돌려줍니다(drop guard).
- **document thread 안**: 세션 등록, lease 요청(승인 대기, 잠그지 않음), 제안 대기(문서 그대로), 완료, 연결 종료, 종료 절차만 합니다. 문서 적용은 사용자가 승인한 뒤 `AgentWindow::apply_proposal`입니다.
- 스레드는 document thread 처리를 기다리지 않고 접수 결과(ticket 또는 거부)를 돌려줘 다음 요청을 바로 받습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib agent_io` | 3 passed |
| `cargo clippy -p stellar-runtime --all-targets --locked -- -D warnings` | 경고 0 |

| 검증 기준 | test | 결과 |
|---|---|---|
| 스레드는 문서를 쓰지 않음 | `io_thread_queues_validated_requests_without_writing_the_document` | 통과(제안 대기 전후 revision 같음, 승인 뒤에만 바뀜) |
| 검증 실패는 queue에 넣지 않음 | 같은 test(빈 proposal은 스레드에서 `Invalid`) | 통과 |
| 사용자 입력보다 낮은 priority | `agent_requests_run_after_user_input` | 통과(입력 job이 먼저 실행) |
| 넘치면 busy | `in_flight_limit_returns_busy_without_waiting` | 통과(기다리지 않고 거부, 접수한 요청은 처리, 자리 반환) |
| 20만 node 문서 lease·batch trace에서 단일 점유와 입력 지연 증가가 예산 이내 | 없음 | 측정 안 함 |

## 차이(작업 지시와 다른 점)

- 계획서는 "lease 대상 계산과 batch 적용은 단일 점유 상한 안에서 나눠 처리"를 요구합니다. 지금 batch 적용은 transaction 하나(전부 적용 또는 전부 미적용)라 한 job 안에서 끝나고, 나눠 처리하지 않았습니다. 측정도 하지 않았습니다.
- XII.1.2 queue에 에이전트 전용 lane을 더하지 않고 처리 전 요청 상한으로 사용자 command 자리를 지켰습니다(ADR-0112 결정 5).
- 원문 버퍼링·표시용 텍스트 준비는 `stellar-agent-host`(XXIX.2.x)에 있고 이 스레드에 연결하지 않았습니다.

## 남은 위험 / 한계

- IPC frame 읽기와 tool 인자를 `AgentRequest`로 바꾸는 앞단(local-ipc 연결)이 없습니다. 지금은 Rust 호출로만 요청을 넣습니다.
- 20만 node 문서에서 lease 대상 subtree 순회와 큰 batch 적용의 document thread 점유 시간을 재지 않았습니다. 상한을 넘으면 batch를 나누는 설계가 필요합니다.
- 처리 전 요청 상한(64)은 측정 없이 정한 값입니다.
