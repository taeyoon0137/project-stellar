# 작업 보고서: XIX.1.1 stellar-mcp stdio JSON-RPC 프레이밍

- WBS: XIX.1.1
- 작업명: 별도 `tools/stellar-mcp` 실행 파일과 stdin/stdout JSON-RPC 프레이밍, stdout은 protocol 메시지 전용
- 상태: 완료 (`[x]`)
- 선행: G8(gate 수준 선행 — 사용자 지시 2026-11-14에 따라 완화, 아래 "차이" 참조)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `tools/stellar-mcp/Cargo.toml`(신규) — binary+lib package, 의존성 없음.
- `tools/stellar-mcp/src/lib.rs`, `src/main.rs`(신규) — 진입점(인자 없음, 도움말·사용법 오류는 stderr).
- `tools/stellar-mcp/src/json.rs`(신규) — 이 도구 전용 엄격 JSON reader·writer(크기·깊이 제한).
- `tools/stellar-mcp/src/framing.rs`(신규) — 개행 구분 frame reader(최대 크기 초과분 폐기)·writer.
- `tools/stellar-mcp/src/jsonrpc.rs`(신규) — JSON-RPC 2.0 분류, 오류 코드, response 생성.
- `tools/stellar-mcp/src/log.rs`(신규) — stderr 전용 한 줄 logger(제어 문자 이스케이프).
- `tools/stellar-mcp/src/server.rs`(신규) — 수명주기(initialize → initialized → ready)와 serve 루프.
- `tools/stellar-mcp/tests/protocol_smoke.rs`, `tests/fixtures/protocol-smoke.{in,out}.jsonl`(신규).
- `Cargo.toml` — workspace member에 `tools/stellar-mcp` 추가. `Cargo.lock` — cargo 자동 갱신.
- `tools/xtask/src/crate_graph.rs` — `stellar-mcp` 허용 edge(core-types, diagnostics)와 manifest 검사 테스트.
- `docs/task-reports/XIX.1.1.md`(신규), 계획서(XIX.1.1 `[ ]`→`[x]`).

## 구현 요약

- **프레이밍**: 메시지 하나 = 개행으로 끝나는 JSON 한 줄. reader는 `fill_buf` 단위로 읽어 1 MiB를
  넘는 줄을 메모리에 쌓지 않고 다음 개행까지 버린 뒤 `-32600` 오류로 응답하고 다음 메시지를 계속
  처리합니다. 입력 끝의 개행 없는 마지막 줄도 처리하고, 빈 줄은 건너뜁니다. writer는 개행 없는
  JSON에 개행 하나를 붙여 쓰고 메시지마다 flush합니다.
- **JSON-RPC 2.0**: object만 허용(batch 거부), `"jsonrpc":"2.0"` 필수, id는 문자열·정수만(null·소수
  거부, 정수 원문 그대로 echo), params는 object·array. 오류 코드 -32700/-32600/-32601/-32602/-32603과
  서버 정의 -32002(initialize 전 request)를 사용합니다. id를 알 수 없는 오류는 id `null`로 응답합니다.
- **수명주기**: `initialize` 전에는 `ping`만 허용, `initialize` 응답(`protocolVersion`, 빈
  `capabilities`, `serverInfo`) 후 `notifications/initialized`로 ready. 중복 initialize는 거부, client
  response와 알 수 없는 notification은 로그만 남기고 무시합니다. resources·tools는 XIX.2 범위입니다.
- **stdout 순수성**: stdout handle은 `main.rs`에서 한 번만 열고 serve 루프의 응답 frame만 씁니다.
  로그·도움말·사용법 오류는 모두 stderr이며, 로그에는 메시지 본문을 넣지 않고 method 이름·오류
  코드만 넣습니다(제어 문자 이스케이프로 로그 줄 위조 방지). 소스에 출력 macro가 없음을 테스트로
  강제합니다. 링크 결과는 `libSystem.B.dylib`뿐이고 network socket API를 쓰지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-mcp` | unit 18, protocol_smoke 5 passed, 0 failed |
| `cargo clippy -p stellar-mcp --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-mcp -- --check` | 통과 |
| `cargo test -p xtask` | 116 passed(+2, +2), 0 failed |
| `cargo clippy -p xtask --all-targets -- -D warnings`, `cargo fmt -p xtask -- --check` | 통과 |
| `cargo run -q -p xtask -- validate-crate-graph` | 종료 코드 0 |
| `otool -L target/debug/stellar-mcp` | `/usr/lib/libSystem.B.dylib`만 링크 |

| 검사 | 결과 |
|---|---|
| **protocol smoke fixture**: 16줄 입력(ping·초기화 전 거부·initialize·initialized·문자열 id·알 수 없는 method·잘린 JSON·batch·잘못된 jsonrpc·null id·중복 initialize·client response·잘못된 params·알 수 없는 notification·escape된 개행 포함 params·개행 없는 마지막 줄)에 대한 stdout이 golden 13줄과 byte 일치, 종료 코드 0(`protocol_smoke_fixture_matches_golden_output`) | 통과 |
| stdout 모든 줄이 JSON-RPC response(jsonrpc 2.0·id·result/error 중 하나), stderr 모든 줄이 `stellar-mcp: ` 로그 | 통과 |
| 메시지마다 flush: stdin을 닫기 전에 initialize·ping 응답을 읽음, stdin 종료 시 0으로 종료(`responses_are_flushed_per_message`) | 통과 |
| 2 MiB 메시지 거부 후 다음 메시지 정상 처리(`oversized_message_is_rejected_and_stream_recovers`) | 통과 |
| `--help`(0)·알 수 없는 인자(64)는 stdout에 쓰지 않음(`help_and_bad_arguments_never_touch_stdout`) | 통과 |
| 소스에 print/println/eprint/eprintln/dbg macro 없음, stdout handle은 main.rs 1회(`sources_have_no_stdout_print_macros`) | 통과 |
| frame 분할(버퍼 1·3·4·8192 byte)·크기 초과 폐기·정확히 최대 크기 허용(`framing::tests`) | 통과 |
| JSON-RPC 위반 11종 거부와 id 보존, response 형식(`jsonrpc::tests`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 G8(초기 릴리스 gate)은 gate 수준 선행이라 사용자 지시(2026-11-14)에 따라 완화했습니다.
  이 작업은 기존 코드에 의존하지 않는 신규 도구입니다.
- 응답 protocol version은 이 단계에서 코드 상수(`2025-11-25`)입니다. config 고정과 unsupported
  version 거부는 XIX.1.5에서 다룹니다.
- JSON reader·writer는 `stellar-cli`와 같은 설계의 이 도구 전용 사본입니다(지시대로 각 도구 내부에
  작성, 브리지↔headless 도구 간 의존 없음). 정수 id 보존을 위한 `JsonNumber::is_integer`가 추가돼
  있습니다.

## 남은 위험 / 한계

- `docs/architecture/crate-dependency-graph.md`(사람용 표)는 허용 경로 밖이라 갱신하지 않았습니다.
- 앱과의 IPC(XIX.1.2~1.4)와 MCP resources·tools(XIX.2)는 아직 없습니다. 현재 서버는 수명주기와
  `ping`만 제공합니다.
- 로그 메시지는 한국어 고정 문자열이며 기계 처리 대상이 아닙니다.
