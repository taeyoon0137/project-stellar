# 작업 보고서: XIX.1.9 stellar-mcp의 stdin EOF·부모 종료 시 자체 종료

- WBS: XIX.1.9
- 작업명: `stellar-mcp`가 표준 입력 EOF나 부모 프로세스 종료를 감지하면 스스로 종료
- 상태: 완료 (`[x]`)
- 선행: XIX.1.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + macOS `kqueue`, 로컬 검증). macOS 전용(ADR-0006).

## 변경 파일 목록

- `tools/stellar-mcp/src/parent.rs`(신규) — `kqueue`/`EVFILT_PROC`/`NOTE_EXIT` 부모 감시(`watch_parent`, `watch_pid`, `Watch`), 최소 `extern "C"` 선언.
- `tools/stellar-mcp/src/main.rs` — 서버 시작 전 부모 감시 시작, 종료 코드 문서·도움말 갱신.
- `tools/stellar-mcp/src/lib.rs` — `parent` module 등록(macOS).
- `tools/stellar-mcp/tests/parent_exit.rs`(신규) — 부모 강제 종료 fixture, stdin EOF 종료 검사.
- `docs/task-reports/XIX.1.9.md`(신규), 계획서(XIX.1.9 `[ ]`→`[x]`).

## 구현 요약

- **근거(MCP 명세 2025-11-25)**: stdio transport에서 client가 서버를 subprocess로 실행하고(Transports "stdio"), 종료는 client가 서버 stdin을 닫고 서버 종료를 기다리는 방식입니다(Lifecycle "Shutdown"). stdin EOF 종료는 XIX.1.1 `server::serve`가 이미 처리하며 이 작업에서 검사를 추가했습니다.
- **부모 종료 감지(Apple `kqueue(2)`, `getpid(2)` man page)**: client가 강제 종료되면 stdin 쓰기 끝이 다른 프로세스에 남아 EOF가 오지 않을 수 있습니다. 그래서 시작 시 `getppid()`로 부모 pid를 기록하고 `kqueue`에 `EVFILT_PROC`·`NOTE_EXIT`로 등록한 뒤, 전용 thread(`stellar-mcp-parent-watch`)가 `kevent`에서 대기합니다. `NOTE_EXIT`를 받으면 stderr에 한 줄 남기고 종료 코드 0으로 끝납니다.
- **경쟁 조건 처리**: 등록이 `ESRCH`(kqueue(2) ERRORS "The specified process to attach to does not exist")이면 부모가 이미 끝난 것으로 보고 바로 종료합니다. 등록 뒤 `getppid()`를 다시 읽어 값이 바뀌었으면(이미 launchd로 재부모화) 역시 종료합니다. 이렇게 하면 pid 재사용으로 다른 프로세스를 감시하는 경우가 없습니다. 시작 시 부모가 launchd(pid 1)이면 감시하지 않고 stdin EOF로만 끝납니다.
- **종료 코드**: 입력 끝·부모 종료는 0, 감시를 시작할 수 없으면(`kqueue` 실패 등) 70(`EX_SOFTWARE`, XXIV.1.2 표 "내부 오류")로 서버를 시작하지 않습니다. 잔존 프로세스를 남기지 않는 쪽을 택했습니다.
- **앱 측**: bridge 프로세스가 끝나면 OS가 IPC socket을 닫으므로 앱은 EOF로 연결 종료를 봅니다. 연결 종료 event 기록은 XIX.1.4 범위입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-mcp` | unit 30, json_copy_sync 2, no_network 3, parent_exit 2, protocol_smoke 5, protocol_version_matrix 2 passed, 0 failed |
| `cargo test -q -p stellar-mcp --test parent_exit` 6회 반복 | 매회 2 passed, 0 failed |
| `cargo clippy -p stellar-mcp --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-mcp -- --check` | 통과 |
| 음성 대조 스크립트(같은 fixture 구조, 대상만 교체) | `/bin/cat`: 부모 SIGKILL 2초 뒤 잔존(`S` 상태), `target/debug/stellar-mcp`: 잔존 0 |

| 검사 | 결과 |
|---|---|
| **부모 강제 종료 fixture**: test가 stdin pipe 쓰기 끝을 쥔 채 `sh`(→ `exec sleep`, 부모)가 띄운 bridge에 ping 응답 확인 → 부모 SIGKILL → bridge 잔존 0(`ps` 기준 없음 또는 zombie), 로그에 감시 시작·부모 종료 감지 있음, `입력 종료` 로그 없음(EOF가 원인이 아님)(`fixture_parent_force_kill_leaves_no_bridge`) | 통과 |
| stdin EOF → 종료 코드 0, stdout 비어 있음, `입력 종료` 로그(`stdin_eof_ends_bridge_with_exit_zero`) | 통과 |
| `kevent` 구조체 크기·정렬 32/8 byte(`kevent_layout_matches_the_c_struct`) | 통과 |
| 감시 대상 `sleep` 프로세스 종료 시 콜백 정확히 1회, 생존 중 0회(`watched_process_exit_calls_back_once`) | 통과 |
| 이미 회수된 pid → `ESRCH`로 감시 안 함(`already_reaped_process_is_reported_gone`), 음수 pid 거부(`invalid_pid_is_rejected`) | 통과 |
| 살아 있는 부모(test runner) → `Watching(getppid())`, 콜백 0회(`test_runner_parent_is_watched`) | 통과 |
| 기존 protocol smoke·version matrix fixture(stderr 줄 형식 포함) 계속 통과 | 통과 |

## 차이(작업 지시와 다른 점)

- 작업 지시의 "앱은 이를 연결 종료로 처리하고 XIX.1.4를 따릅니다"는 앱 측 동작이므로 이 작업에서는 bridge 종료가 IPC socket을 닫는다는 사실만 기록하고, 앱 측 연결 종료 event는 XIX.1.4에서 구현합니다.

## 남은 위험 / 한계

- 시작 시 부모가 이미 launchd인 경우(부모가 브리지 시작 직후 `getppid()` 전에 죽은 경우 포함)는 감시하지 않으며 stdin EOF가 유일한 종료 신호입니다. MCP client가 subprocess로 실행하는 정상 경로에서는 해당하지 않습니다.
- 부모가 살아 있지만 응답하지 않는 경우(멈춘 client)는 종료 조건이 아닙니다. MCP 명세상 그 경우 client가 SIGTERM·SIGKILL로 서버를 끝냅니다.
