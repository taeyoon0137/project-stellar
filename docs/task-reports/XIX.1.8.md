# 작업 보고서: XIX.1.8 앱이 시작한 bridge의 세션 결합

- WBS: XIX.1.8
- 작업명: 앱이 시작한 bridge를 그 세션에만 결합하고, 세션 종료 시 결합 수단을 무효화하며, 결합이 권한을 주지 않게 함
- 상태: 부분 완료 (`[~]`) — 결합 수단·검증·무효화·거부와 bridge 측 전달은 구현·검증했으나, 작업 지시의 "각 CLI 공식 문서 근거"는 Claude Code 문서만 이번 실행에서 확인했고 다른 공급자 CLI와 Apple 문서 근거는 확인하지 못했습니다. 앱 runtime이 이 결합을 쓰도록 연결하는 일은 이 lane 경로(`crates/runtime`) 밖입니다.
- 선행: XIX.1.3(`[x]`), XVI.7.5(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (macOS Unix domain socket·코드 서명 peer 인증, 로컬 검증).

## 변경 파일 목록

- `crates/local-ipc/src/binding.rs`(신규) — `BindingToken`(256 bit, 16진수 64자, `Debug` 값 숨김, 상수 시간 비교), `SessionBindings`(issue·end·verify), `BindingSession`, `BindingError`, `BINDING_ENV`, `token_from_env_value`.
- `crates/local-ipc/src/handshake.rs` — `Message::BoundHello`(`STELLAR-IPC/1 HELLO <client> BIND <token>`), `HandshakeError::BindingRejected`, `bridge_handshake_bound`, `connect_bound`, 메시지 test.
- `crates/local-ipc/src/listener.rs` — `AppListener::spawn_with_bindings`, handshake에서 token 검증·`REJECT binding-rejected`, `AppConnection::bound_session`·`origin`, 결합 test 2개.
- `crates/local-ipc/src/lib.rs` — `binding` 모듈.
- `tools/stellar-mcp/src/attach.rs` — 환경 변수 `STELLAR_MCP_SESSION_BINDING`을 읽어 `connect_bound`로 전송, `AttachError::Binding`(78), 결합 거부 77, test.
- `docs/task-reports/XIX.1.8.md`(신규), 계획서(XIX.1.8 `[ ]`→`[~]`).

## 구현 요약

- **결합 수단**: 앱은 세션을 시작할 때 `SessionBindings::issue`로 그 세션 전용 token(OS CSPRNG `/dev/urandom` 256 bit)을 만들고, 공급자 CLI의 MCP 서버 설정 환경 변수로 bridge에 넘깁니다. bridge는 앱 peer 인증(XIX.1.7)을 통과한 **뒤에만** token을 HELLO에 담아 보냅니다(인증 실패 시 token 포함 전송 0). 한 줄은 123 bytes로 handshake 한 줄 상한 128 bytes 안입니다.
- **환경 변수를 고른 근거**: Claude Code 공식 MCP 문서(`code.claude.com/docs/en/mcp`, 이번 실행에서 확인)는 stdio MCP 서버 항목의 `env` field와 `claude mcp add --env`로 서버 프로세스 환경 변수를 지정하는 방법을 문서화합니다. 명령줄 인자 대신 환경 변수를 쓰는 이유는 macOS에서 인자 목록은 다른 사용자도 `ps`로 볼 수 있지만 다른 프로세스의 환경은 같은 사용자·root만 읽을 수 있다는 점입니다(아래 "남은 위험"의 미확인 항목).
- **검증**: 앱 listener는 peer 인증(XIX.1.6)과 nonce 검사(XIX.1.3)를 통과한 연결에서만 token을 봅니다. 모든 활성 token과 상수 시간으로 비교해 만든 세션을 찾고, 모르는 token(`Unknown`)·끝난 세션 token(`Ended`)·결합을 받지 않는 listener(`Unsupported`)는 `REJECT binding-rejected`로 거부하고 연결을 앱에 넘기지 않습니다.
- **무효화**: `SessionBindings::end`가 세션의 token을 즉시 무효로 합니다. 같은 세션을 다시 시작하거나 재발급하면 이전 token은 계속 거부됩니다. 끝난 token 기억은 1024개 상한이며 잊은 token도 `Unknown`으로 거부됩니다.
- **결합은 권한이 아님**: 결합 결과는 `BindingSession`(세션 식별)뿐이며 권한·승인 상태를 담는 field나 API가 없습니다. 결합된 연결의 `origin()`은 `AppStarted`(XIX.1.4 수명 규칙 구분)일 뿐이고 XIX.3 승인을 대체하지 않습니다.
- **bridge 종료 코드**: 앱이 결합을 거부하면 77 `noperm`, 환경 변수 형식 오류는 연결하지 않고 78 `config`(진단에 값을 쓰지 않음)입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-local-ipc` | 54 passed, 0 failed |
| `cargo test -p stellar-mcp` | 80 passed, 0 failed |
| `cargo clippy -p stellar-local-ipc -p stellar-mcp --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-local-ipc -p stellar-mcp -- --check` | 통과 |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| 실제 socket·peer 인증: 세션 10의 token 연결은 세션 10에만 결합(`bound_session` = 10, 20 아님), `origin` AppStarted, IPC I/O 스레드에서 처리(`bound_bridge_is_tied_to_the_issuing_session_only`) | 통과 |
| 다른 세션·종료된 세션 결합 거부: 끝난 세션 token `Ended`, 위조 token `Unknown` → bridge `REJECT binding-rejected`, 앱 `HandshakeFailed(BindingRejected)`, 연결 전달 0 | 통과 |
| 결합 없는 연결은 `bound_session` None·origin External | 통과 |
| 결합을 받지 않는 listener는 token을 `Unsupported`로 거부(`listener_without_bindings_rejects_any_binding`) | 통과 |
| 결합 표: 세션별 token, 종료 즉시 무효, 재발급 시 이전 token 무효, 끝난 token 기억 상한(`token_binds_only_its_own_session_and_dies_with_it`, `ended_memory_is_bounded`) | 통과 |
| `BoundHello` 왕복·한 줄 상한·잘못된 BIND 형식 거부(`bound_hello_round_trips_within_the_line_limit`) | 통과 |
| bridge 종료 코드 77·78과 환경 변수 값 비노출(`binding_env_value_is_parsed_without_echoing_it`, 종료 코드 표 test) | 통과 |
| 결합만으로 얻는 권한 0 | 구조로 확인(결합 결과 타입에 권한 field 없음). XIX.3 권한 저장소가 아직 없어 런타임 권한 집합 비교는 하지 않았음 |

- 성능 작업이 아니며 자동 생성 파일 변경은 없습니다.

## 차이(작업 지시와 다른 점)

- 공급자 CLI 문서 근거는 Claude Code 하나만 이번 실행에서 확인했습니다. Codex CLI 문서는 다른 host로 redirect되어 따라가지 않았고, 그 밖의 CLI는 확인하지 않았습니다.
- 앱 runtime이 세션 시작 시 `SessionBindings::issue`를 부르고 listener를 `spawn_with_bindings`로 여는 연결은 `crates/runtime` 범위라 하지 않았습니다(XXIX.2.x 내장 세션 작업).

## 남은 위험 / 한계

- **후속(`[x]`로 올리기 위한 남은 일)**: 다른 공급자 CLI(Codex, Gemini 등)의 MCP 서버 환경 변수 지정 방법을 각 공식 문서로 확인해 이 보고서에 기록하고, macOS에서 다른 사용자가 프로세스 환경을 읽을 수 없다는 근거를 Apple 공식 문서(`ps(1)`, `sysctl(3)` `KERN_PROCARGS2`)로 확인합니다. 지원하지 않는 CLI가 있으면 다른 전달 수단을 정해야 합니다.
- 같은 OS 사용자의 다른 프로세스는 환경 변수를 읽을 수 있습니다. 그 경계는 같은 사용자 신뢰(아키텍처 §37)와 peer 코드 서명 인증(XIX.1.6·XIX.1.7)이 담당하며, token만으로 다른 사용자의 세션에 붙을 수는 없습니다.
- `libc` 승인 항목(`planned_for: XIX.1.8, XXIX.2.3`)은 이 작업에서 쓰지 않았습니다(std와 기존 최소 extern으로 충분).
