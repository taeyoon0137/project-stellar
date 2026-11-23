# 작업 보고서: XIX.9.9 tool 결과 종류와 종료 코드 대응

- WBS: XIX.9.9
- 작업명: tool 결과 종류와 `stellar`·`stellar-mcp` 종료 코드의 대응을 XXIV.1.2 종료 코드 표에 맞춰 고정
- 상태: 부분 완료 (`[~]`) — 2026-11-23 ledger-refresh 판정: 구현 판정 통과, 선행 미충족(계획 1.2 규칙 4): 선행 XIX.9.5(`[ ]`)가 `[x]`가 아닙니다. XIX.9.5가 attached 승인·lease 판정 범주를 더하면 이 표에 행을 더해야 합니다. 이전 기록: 구현 완료, 원장 판정 보류 (`[~]`) — 선행 XIX.9.5(결과 범주, `[ ]`)가 이 lane 범위 밖이라 `[x]`로 올리지 않습니다. XIX.9.5가 attached 승인·lease 판정 범주를 더하면 이 표에 행을 더해야 합니다.
- 선행: XIX.9.5 `[ ]`, XXIV.1.2 `[x]`

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/agent-protocol/src/outcome.rs`(신규) — `ResultKind` 13종, `exit()`, `retryable()`, `is_protocol_error()`, golden test.
- `crates/agent-protocol/src/lib.rs` — 모듈 등록.
- `crates/mcp-core/src/failure.rs`(신규) — `fail`(종류·고정 문구·`exit`), `kind_of`, `WithDetail`.
- `crates/mcp-core/src/result.rs` — `tool_failure`(`isError: true` 결과).
- `crates/mcp-core/src/exit.rs` — XXIV.1.2 표 13행 전체(1·65·66·73 추가), `for_kind`.
- `crates/mcp-core/src/headless.rs`, `headless/{comparison,components,validation}.rs` — 자체 실패 종류 enum을 지우고 `ResultKind`를 씀, page·scope 오류 분류, tool 실행 실패를 `isError` 결과로, 시작 시 신뢰 실패 종료 코드를 대응표에서.
- `tools/stellar-mcp/tests/exit_table_sync.rs`(신규), `tools/stellar-mcp/tests/fixtures/result-exit-map.json`(신규 golden).
- `tools/stellar-mcp/tests/{support/mod.rs,headless_read.rs}`, `tools/stellar-mcp/tests/fixtures/trust-boundary.jsonl` — 실패 확인 helper와 golden.

## 구현 요약

| 결과 종류 | 종료 코드 | MCP 표현 |
|---|---|---|
| `ok` | 0 `ok` | tool 결과 `structuredContent.kind` |
| `rejected` | 1 `failure` | tool `isError` 결과 |
| `unknown-tool` | 64 `usage` | JSON-RPC `error`(-32602) |
| `invalid-arguments`·`stale`·`too-large` | 65 `dataerr` | tool `isError` 결과 / resource JSON-RPC `error` |
| `not-found` | 66 `noinput` | 같음 |
| `unavailable` | 69 `unavailable` | 같음 |
| `internal` | 70 `software` | 같음 |
| `busy`·`timeout` | 75 `tempfail`(재시도 가능) | 같음 |
| `not-trusted`·`out-of-scope` | 77 `noperm` | 같음 |

- CLI가 같은 실패에서 내는 코드와 같습니다: base revision 불일치·형식 오류 65, validator 거부 1, 없는 node 66, untrusted 77, 잠금 사용 중 75.
- 실패 결과 `structuredContent`는 `{"kind", "exit": {"code", "name"}, "message", ...}`이고 보조 field(`index`, `code`)를 옮깁니다. JSON-RPC 실패의 `error.data`도 같은 `kind`·`exit`를 갖습니다.
- MCP 명세 2025-11-25 "Tools" Error Handling대로 모르는 tool과 params 형식 오류만 protocol 오류이고, 인자 검증을 포함한 실행 실패는 `isError` 결과입니다.
- `stellar-mcp` 프로세스 종료 코드 표(`exit::ALL`)는 이제 XXIV.1.2 표 전체이고, headless 시작 시 신뢰 실패는 `for_kind(not-trusted)` = 77입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-agent-protocol` | 통과(`result_exit_table_golden` 포함) |
| `cargo test -p stellar-mcp --test exit_table_sync` | 2 passed |
| `cargo test -p stellar-mcp -p stellar-mcp-core` | 실패 0 |

| 검사 | 결과 |
|---|---|
| 대응표 golden 일치: Rust 표(`result_exit_table_golden`)와 fixture(`result_exit_map_matches_golden`) | 통과 |
| `stellar-mcp` 종료 코드 표 = `tools/stellar-cli/src/exit.rs` XXIV.1.2 표(값·이름, 소스 비교)(`bridge_exit_table_equals_the_cli_table`) | 통과 |
| 모든 결과 종류의 종료 코드가 `stellar-mcp` 표에 있음(`every_result_kind_maps_into_this_table_with_the_same_name`) | 통과 |
| 통합 test의 모든 실패 응답에서 `exit`가 `kind`의 대응과 같음(`support::failure_kind`가 매번 확인) | 통과 |
| 검토 보완: HEAD 없음·컴포넌트 원본·문서 snapshot 읽기 실패도 `unavailable` 종류를 가져 tool 실패는 `isError` 결과, resource 실패는 `data.kind`·`exit`가 있는 JSON-RPC 오류(`headless_read`의 `assert_tool_error`·`assert_error`가 확인) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 XIX.9.5(XIX.3·XIX.4 승인·lease 판정 범주)가 없어 headless backend에서 나는 결과 종류만 표에 넣었습니다.

## 남은 위험 / 한계

- attached backend의 승인 거부·lease 충돌 범주(XIX.9.5)가 생기면 `ResultKind`와 golden을 함께 고쳐야 합니다.
- MCP 명세의 resource not found 코드(-32002)는 이 서버에서 initialize 전 요청 코드로 이미 쓰여 있어, 정의되지 않은 resource URI는 `invalid-arguments`(-32602)로 둡니다.
