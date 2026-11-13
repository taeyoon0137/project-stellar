# 작업 보고서: XIX.1.4 한 bridge = 한 MCP 세션, 연결 종료 event 1회

- WBS: XIX.1.4
- 작업명: 한 bridge process는 한 MCP 연결 세션만 담당. bridge가 끝나면 앱은 연결 종료 event를 한 번 기록하고, 외부 클라이언트 세션은 그 시점에 XIX.6.4 종료 절차를 시작하며, 앱이 시작한 세션의 권한·lease 수명은 AgentSession(XXIX.2.8)이 정함
- 상태: 완료 (`[x]`)
- 선행: XIX.1.3(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (Rust + macOS Unix domain socket, 로컬 검증). macOS 전용(ADR-0006).

## 변경 파일 목록

- `crates/local-ipc/src/session.rs`(신규) — 앱 측 연결 수명: `ConnectionMonitor`, `ConnectionClosed`(정확히 1회), `SessionOrigin`(External·AppStarted), `CloseCause`, 종료 절차 hook `SessionTermination`.
- `crates/local-ipc/src/lib.rs` — module 등록.
- `tools/stellar-mcp/src/attach.rs` — bridge 측 앱 연결 감시 `watch_app_connection`, `AppConnectionEnd`와 종료 상태, "한 bridge = 한 세션" 문서.
- `tools/stellar-mcp/src/main.rs` — `--attach` 연결 뒤 앱 연결 감시 시작, 앱이 닫으면 MCP 세션 종료.
- `tools/stellar-mcp/tests/session_lifecycle.rs`(신규) — 실제 bridge 프로세스 disconnect fixture.
- `tools/stellar-mcp/tests/no_network.rs` — Unix socket `shutdown(2)` 방향 값 `std::net::Shutdown`만 network API 금지 검사에서 제외.
- `docs/task-reports/XIX.1.4.md`(신규), 계획서(XIX.1.4 `[ ]`→`[x]`).

## 구현 요약

- **한 bridge = 한 세션**: bridge는 프로세스 수명 동안 앱 연결을 한 번만 맺고(`--attach`, XIX.1.7) 다시 연결하지 않습니다. MCP `initialize`는 한 번만 받습니다(XIX.1.1, 두 번째는 `-32600`). 앱이 연결을 닫으면 bridge의 감시 thread가 EOF를 보고 MCP 세션을 69(`unavailable`)로 끝냅니다. handshake 뒤 bridge↔app 메시지는 아직 없으므로(XIX.9.4) data를 받으면 76(`protocol`)으로 끝냅니다.
- **연결 종료 event 1회**: 앱은 인증된 연결을 `ConnectionMonitor::watch`에 넘깁니다. 감시 thread(`stellar-ipc-conn`)가 bridge 종료(EOF; 정상 종료·SIGKILL 모두 OS가 socket을 닫음), 읽기 오류, 정의되지 않은 data 중 하나를 보거나 앱이 먼저 닫으면(`close`·drop) `ConnectionClosed`를 기록합니다. 기록은 `AtomicBool` swap으로 첫 경로만 통과하므로 경로가 겹쳐도 연결마다 정확히 한 번입니다.
- **종료 절차 시작**: `SessionOrigin::External`이면 event를 보내기 전에 `SessionTermination::begin_external_termination`을 한 번 호출합니다. 앱은 여기서 XIX.6.4 절차(권한·lease 즉시 해제, 대기 요청 거부, 자동 rollback 없음)를 시작합니다. `SessionOrigin::AppStarted`는 호출하지 않고 event만 기록합니다(AgentSession이 수명 소유, XXIX.2.8).
- **문서 무변경**: `stellar-local-ipc`는 crate graph 정책상 core-types만 허용되어 document·commands에 도달하지 못하므로, 감시와 event 기록이 문서 상태를 바꿀 수단이 없습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-local-ipc -p stellar-mcp` | local-ipc unit 47·foreign_peer 1, stellar-mcp unit 35·attach_auth 4·connection_info 4·json_copy_sync 2·no_network 3·parent_exit 4·protocol_smoke 5·protocol_version_matrix 2·session_lifecycle 3 passed, 0 failed |
| `cargo test -q -p stellar-mcp -p stellar-local-ipc` 10회 반복 | 실패 0 |
| `cargo clippy -p stellar-local-ipc -p stellar-mcp --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-local-ipc -p stellar-mcp -- --check` | 통과 |

| 검사(fixture) | 결과 |
|---|---|
| **외부 bridge disconnect(실제 `stellar-mcp --attach` 프로세스)**: initialize 응답 확인 후 (a) client가 stdin을 닫음(종료 코드 0), (b) bridge SIGKILL → 각각 연결 종료 event 1회(`PeerClosed`, External), **session permission 0**(2 → 0), 종료 절차 호출 1회, **문서 revision 변화 0**(7 → 7), 이후 앱 쪽 정리로 event 추가 0(`fixture_external_bridge_disconnect_releases_permissions_once`) | 통과 |
| 앱이 시작한 세션: disconnect 시 event 1회, 종료 절차 호출 0, 권한은 AgentSession 몫으로 유지(`app_started_session_disconnect_leaves_lifetime_to_agent_session`) | 통과 |
| 한 bridge = 한 세션: 두 번째 initialize `-32600`, 앱이 연결을 닫으면 bridge가 69로 끝나고 재연결 0, event 1회(`AppShutdown`), 권한 0(`app_closing_the_connection_ends_the_single_bridge_session`) | 통과 |
| 앱 종료와 bridge 종료가 동시에 일어나는 경합 20회: 매회 event 1개, 종료 절차 1회(`app_shutdown_and_peer_close_race_records_once`) | 통과 |
| bridge 종료 → event 1회·종료 절차 1회, 이후 `close`는 `false`(`bridge_close_records_one_event_and_starts_external_termination_once`), 정의되지 않은 data → `UnexpectedData`로 닫고 bridge는 EOF(`unexpected_data_closes_connection_once`), 연결 id 유일 | 통과 |
| bridge 측 감시: 앱이 닫으면 `Closed`(69), data `UnexpectedData`(76), 읽기 오류(74)(`app_connection_end_is_reported_once_with_table_exit`) | 통과 |

## 차이(작업 지시와 다른 점)

- 세션 권한 저장소(XIX.3)와 XIX.6.4 종료 절차, 문서 revision은 아직 없어 fixture의 앱 쪽 권한·revision은 `SessionTermination`을 구현한 test double(`TestApp`)입니다. 앱 측 연결 감시·event 기록·hook 호출은 실제 구현(`stellar-local-ipc`)이고, bridge는 실제 프로세스입니다. XIX.6.4가 이 hook에 실제 권한·lease 해제를 연결해야 합니다(XIX.6.4의 선행이 XIX.1.4인 계획 순서와 같음).
- 앱 runtime(`crates/runtime`)은 다른 lane 범위라 `AppListener`·`ConnectionMonitor`를 시작하는 연결은 하지 않았습니다.
- 작업 순서: XIX.1.4 검증에 실제 연결된 bridge가 필요해 XIX.1.6·XIX.1.7 다음에 진행했습니다.
- 연결 감시는 ADR-0005 §6의 IPC I/O 계층 안에서 연결마다 thread 하나(`stellar-ipc-conn`)로 합니다. bridge는 세션당 연결 하나라 연결 수가 적습니다. 단일 thread 다중화는 bridge↔app 메시지 protocol(XIX.9.4)과 함께 정합니다.

## 남은 위험 / 한계

- 재연결 유예와 앱이 시작한 세션의 bridge 재결합은 XIX.1.8·XXIX.2.8 범위입니다. 현재 앱이 시작한 세션은 `SessionOrigin::AppStarted`로 event만 받습니다.
- `begin_external_termination`은 연결 감시 thread에서 호출되므로, 실제 구현은 UI·Document 스레드 작업을 자기 queue로 넘겨야 합니다(trait 문서에 명시).
