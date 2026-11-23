# 작업 보고서: XIX.9.10 MCP 경로 자원 상한

- WBS: XIX.9.10
- 작업명: MCP 경로의 자원 사용(호출량, payload 크기, 대기 시간)에 상한을 두고 넘으면 거부하거나 busy로 응답
- 상태: 구현 완료, 원장 판정 보류 (`[~]`) — 선행 XIX.9.4(attached backend, `[ ]`)가 이 lane 범위 밖이라 `[x]`로 올리지 않습니다. headless 경로의 상한은 모두 구현했고 attached backend도 같은 서버 판정을 거칩니다.
- 선행: XIX.9.4 `[ ]`

## 필수 실행 환경

- `DEV-MAC` (Apple M3 Max, 실측 포함).

## 변경 파일 목록

- `crates/agent-protocol/src/limits.rs`(신규) — 상한 값 목록과 schema 일치 test.
- `crates/agent-protocol/src/catalog/tools.rs` — page 인자 schema 최대 2,000 → 1,000.
- `crates/agent-protocol/src/lib.rs` — 모듈 등록.
- `crates/mcp-core/src/limits.rs`(신규) — 고정 창 호출량 판정 `RateLimiter`.
- `crates/mcp-core/src/server.rs` — 요청마다 호출량·대기 시간·응답 크기 판정, 실행 실패를 `isError`로 보내는 응답 경로를 서버 한 곳으로.
- `crates/mcp-core/src/catalog/paging.rs` — page 상수를 agent-protocol 상한에서.
- `crates/mcp-core/src/headless.rs` — 대기·응답 크기 상한 단위 test, 실패 변환을 서버로 옮김.
- `tools/stellar-mcp/tests/limits.rs`(신규), `tools/stellar-mcp/tests/support/mod.rs`, `tools/stellar-mcp/tests/headless_read.rs`, `tools/stellar-mcp/tests/fixtures/{catalog.json,trust-boundary.jsonl}`.
- `docs/adr/ADR-0085-mcp-boundary-agent-protocol.md` — 결정 4를 구현에 맞춰 보완.

## 구현 요약

| 한도 | 값 | 초과 응답 |
|---|---|---|
| 수신 메시지 | 1 MiB(1,048,576 bytes) | 메시지를 버리고 `-32600`(기존 framing) |
| 송신 응답 메시지 | 1 MiB | 결과를 버리고 `too-large`(tool은 `isError`, 그 밖은 JSON-RPC 오류) |
| page 항목 | 기본 200, 최대 1,000 | `invalid-arguments` |
| page 내용 JSON | 384 KiB(1 MiB × 3/8) | 넘는 항목은 다음 page, 항목 하나가 넘으면 `too-large` |
| 호출 대기 | 30 초 | 결과를 버리고 `timeout`(재시도 가능) |
| 호출량 | 1 초 창에 100개(`ping`·`initialize` 제외) | 처리하지 않고 `busy`(재시도 가능, `retry_after_ms`) |
| 동시 실행 | 연결당 1(stdio 순서 처리) | — |
| proposal command 수 | 10,000(ops `MAX_COMMANDS`, 생성 schema `maxItems`) | `invalid-arguments` |
| CSS 선언 블록 | 16 KiB(CLI와 같음) | `too-large` |

실측(release build, 100,000 node 문서 한 개, `measure_round_trips`, `cargo test -p stellar-mcp --release --test limits measure_round_trips -- --ignored --nocapture`):

| 요청 | 시간 | 응답 크기 |
|---|---|---|
| `stellar://project/structure` | 0.65 ms | 778 bytes |
| 문서 트리 page 1,000개(처음, 트리 적재 포함) | 178.7 ms | 252,594 bytes |
| 문서 트리 page 1,000개(적재 후) | 5.2 ms | 252,594 bytes |
| `document.validate`(전체 문서, limit 1,000) | 260.8 ms | 876 bytes |
| `document.propose_commands`(rename 1개, dry-run) | 358.3 ms | 1,476 bytes |

- page 최대 1,000: 트리 page 1,000개가 약 247 KiB라 tool 결과로 두 번 실려도 1 MiB 안입니다. 2,000이면 단순 트리만으로 0.5 MiB에 가까워 tool 결과 상한을 넘을 수 있어 줄였습니다. 기본 200은 page 하나를 50 KiB 안팎으로 둡니다.
- 호출 대기 30 초: 실측 최대(0.36 s)의 약 80배입니다. 시스템 Git 명령 하나의 상한(`SYSTEM_GIT_TIMEOUT` 60 s)보다 짧게 두어, Git이 오래 걸릴 때 agent가 결과 없이 기다리는 시간을 줄입니다.
- 호출량 100/s: 적재 후 page 읽기(5 ms) 기준 한 코어의 약 절반입니다. 같은 기기의 앱(§6.3 단일 점유 2 ms)과 CPU를 나눠 쓰도록 agent의 빠른 반복 호출을 묶습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-mcp --test limits` | 3 passed, 1 ignored(실측) |
| `cargo test -p stellar-mcp-core --lib` | 62 passed |
| `cargo test -p stellar-agent-protocol` | 통과(`limits_follow_adr_0085`, `tool_schemas_use_the_page_limit`) |
| `cargo test -p stellar-mcp --release --test limits measure_round_trips -- --ignored --nocapture` | 위 실측 표 |

| 검사 | 결과 |
|---|---|
| 호출량 초과: 한 번에 보낸 150개 중 처음 100개는 처리, 창 안 초과 50개는 `busy`·`retry_after_ms` 0..=1000, `ping`은 세지 않음(`burst_over_the_rate_window_is_answered_busy_without_processing`) | 통과 |
| page 기본 200, `limit` 1,001은 resource·tool 모두 `invalid-arguments`, 2,500 node 문서를 1,000씩 끝까지 읽음, 저장 뒤 이전 cursor는 `stale`(`page_limit_default_and_maximum_follow_the_contract`) | 통과 |
| page byte 상한을 넘는 항목은 `too-large`, 같은 세션 계속 사용(`oversized_result_is_too_large_and_session_continues`) | 통과 |
| 호출 대기 상한 0이면 `timeout`(`isError`), 응답 상한을 넘으면 작은 `too-large` 응답(`call_deadline_and_response_limit_turn_results_into_retryable_failures`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 XIX.9.4(attached backend)가 없어 headless 경로에서 상한을 검증했습니다. 판정은 backend와 무관한 서버 단계에 있습니다.

## 남은 위험 / 한계

- 호출 대기 상한은 요청 처리가 끝난 뒤 확인합니다. ops 호출을 중간에 끊지 않으므로 30 초를 넘는 요청은 끝날 때까지 다음 요청을 막습니다. Git 실행은 ops의 60 초 상한을 따릅니다.
- 실측은 한 기기(M3 Max)·release build 한 번의 값입니다. 느린 기기와 큰 Git 저장소는 재측정이 필요합니다.
- 호출량 판정은 연결 단위입니다. 여러 bridge 프로세스를 동시에 띄우는 경우의 합산 상한은 없습니다.
