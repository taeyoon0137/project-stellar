# 작업 보고서: XIX.1.5 MCP protocol version config 고정

- WBS: XIX.1.5
- 작업명: MCP protocol version을 config에 고정하고 unsupported client version에 명확한 오류 반환
- 상태: 완료 (`[x]`)
- 선행: XIX.1.1(같은 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `config/mcp-protocol.yaml`(신규) — `supported_versions: 2025-11-25, 2025-06-18`.
- `tools/stellar-mcp/src/protocol.rs`(신규) — config 내장·파싱·검증, version 협상, 빌드 시 선호 version 계산.
- `tools/stellar-mcp/src/server.rs` — initialize가 config 목록으로 협상, `Server::with_config`,
  `PROTOCOL_VERSION`을 config 첫 항목에서 가져오도록 변경.
- `tools/stellar-mcp/src/main.rs` — 시작 시 내장 config 검사(오류면 종료 코드 78), 지원 목록 로그.
- `tools/stellar-mcp/src/lib.rs` — `protocol` module 등록.
- `tools/stellar-mcp/tests/protocol_version_matrix.rs`, `tests/fixtures/protocol-version-matrix.jsonl`(신규).
- `docs/task-reports/XIX.1.5.md`(신규), 계획서(XIX.1.5 `[ ]`→`[x]`).

## 구현 요약

- **config 고정**: 지원 version 목록은 `config/mcp-protocol.yaml` 한 곳에만 있습니다. 실행 파일은
  `include_str!`로 빌드 시 내장하므로 설치 위치와 무관합니다. 형식은 `key: value` 한 줄(기존 config와
  같음), 값은 쉼표 구분 `YYYY-MM-DD` 정확 일치 목록(범위·와일드카드 없음, 첫 항목이 선호)입니다.
  빈 목록·형식 오류(월·일 범위 포함)·중복 version·중복 key·알 수 없는 key를 거부합니다.
- **코드에 version 값 없음**: `PROTOCOL_VERSION`은 const fn이 빌드 시 config 첫 항목에서 계산합니다
  (key가 없으면 빌드 실패). 테스트가 런타임 parser 결과와 일치를 확인합니다.
- **협상**: 요청 `protocolVersion`이 목록에 정확히 있으면 그 version으로 응답합니다. 없으면 `-32602`와
  메시지 `지원하지 않는 MCP protocol version입니다(지원: 2025-11-25, 2025-06-18)`, `data`에
  `requested`(64자 초과 시 잘라냄)·`supported`를 반환합니다. 문자열이 아니거나 없으면 `-32602`
  `protocolVersion 문자열이 필요합니다`입니다. 거부 후에도 initialize 전 상태를 유지해 같은 연결에서
  재시도할 수 있습니다.
- **목록 선정**: batch를 요구하는 2025-03-26 이전 version은 넣지 않았습니다(XIX.1.1이 batch를
  거부). 현재 구현(initialize·initialized·ping)은 두 version에서 동작이 같습니다.
- **시작 검사**: 내장 config가 잘못되면 실행 파일이 protocol 처리 전에 종료 코드 78(`EX_CONFIG`)로
  끝납니다. 라이브러리 경로에서는 panic 없이 initialize가 `-32603`으로 실패합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-mcp` | unit 24, protocol_smoke 5, protocol_version_matrix 2 passed, 0 failed |
| `cargo clippy -p stellar-mcp --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-mcp -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **version matrix fixture**(11행, case마다 새 프로세스): 2025-11-25·2025-06-18 수락(같은 version으로 응답, 이어지는 ping 성공), 2025-03-26·2024-11-05·2099-01-01·`1.0.0`·빈 문자열·앞 공백 version 거부(-32602, 요청·지원 목록), 숫자·null·누락 거부(-32602), 거부 후 같은 연결 재시도 성공(`version_matrix_fixture`) | 통과 |
| matrix의 수락 행 = config 목록, repo config 파일 = 내장 config(`matrix_accepted_rows_equal_config_versions`) | 통과 |
| 내장 config 목록·선호 version, 빌드 시 상수와 parser 일치(`builtin_config_pins_exact_versions`) | 통과 |
| 정확 일치만 수락(공백·잘린 version 거부), 긴 요청 version 잘라서 echo(`negotiation_accepts_only_exact_matches`) | 통과 |
| 잘못된 config 13종 거부(`malformed_configs_are_rejected`) | 통과 |
| 다른 config 주입 시 그 목록으로 협상·거부 후 상태 유지, 깨진 config는 -32603(`server::tests`) | 통과 |
| XIX.1.1 protocol smoke fixture 변경 없이 통과 | 통과 |

## 차이(작업 지시와 다른 점)

- "version 고정"을 단일 값이 아니라 정확 일치 version 목록(2개)으로 구현했습니다. 범위·자동 최신
  추종은 없고, 목록을 바꾸려면 config 변경이 필요합니다.
- 미지원 version 오류 형식은 JSON-RPC `-32602`와 `data.requested`·`data.supported`입니다.

## 남은 위험 / 한계

- 이후 resources·tools(XIX.2)가 특정 version에만 있는 기능을 쓰게 되면 목록을 다시 검토해야 합니다.
- 더 새로운 MCP version만 지원하는 client는 명확한 오류를 받지만 연결할 수 없습니다. 새 version 지원은
  config 갱신과 호환성 검증이 필요합니다.
- 오류 메시지는 한국어 고정 문자열입니다. client는 `code`와 `data`로 처리해야 합니다.
