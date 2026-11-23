# 작업 보고서: XXIX.2.9 출력 디코더 테스트 fixture

- WBS: XXIX.2.9
- 작업명: launch profile마다 원문 입력과 기대 AgentChatItem 목록(golden) fixture
- 상태: `[x]` 완료
- 선행: XXIX.2.2, XXIX.2.6
- 설계 기준: ADR-0005 §4·§6·§8, ADR-0023 결정 9, ADR-0092

## 필수 실행 환경

- `DEV-MAC` (macOS 프로세스·파이프·process group·Code Signing Services, 로컬 검증). 네트워크와 실제 공급자 계정은 쓰지 않습니다.

## 변경 파일 목록

- `crates/agent-host/tests/fixtures/decoder/INDEX`(fixture 목록 golden)
- `crates/agent-host/tests/fixtures/decoder/claude-stream-json/*.jsonl`·`*.golden`(12쌍)
- `crates/agent-host/tests/fixtures/decoder/codex-exec-jsonl/*.jsonl`·`*.golden`(7쌍)
- `crates/agent-host/tests/decoder_fixtures.rs`

## 구현 요약

- decoder profile(= launch profile의 `decoder`)마다 디렉터리 하나에 원문 `<case>.jsonl`과 기대 결과 `<case>.golden`(item golden 줄, 신호, `ok` 또는 `failure <code> line <n>`)을 둡니다. 목록은 `INDEX` golden으로 고정합니다.
- **`claude-stream-json` 사례**(근거: Agent SDK TypeScript reference의 `SDKMessage`·`SDKAssistantMessage.error` 값·`SDKResultMessage` subtype):
  - 정상(`normal`: init·text·tool_use·tool_result 성공·실패·result success, 한글·emoji)
  - `usage-limit`(`error: rate_limit` + `error_during_execution`), `auth-error`(`authentication_failed`)
  - `skipped-types`(`stream_event`·`rate_limit_event`·hook system·빈 줄), `crlf`, `no-final-newline`
  - 해석 불가: `fail-invalid-json`, `fail-invalid-utf8`, `fail-not-object`, `fail-missing-type`, `fail-shape`, `fail-trailing-garbage`
- **`codex-exec-jsonl` 사례**(근거: Codex non-interactive mode 문서의 event type·item type):
  - 정상(`thread.started`, reasoning 건너뜀, `mcp_tool_call`, `command_execution` 실패, `agent_message` updated·completed, `turn.completed` usage 무시)
  - `turn-failed`, `error-event`, `unknown-item`
  - 해석 불가: `fail-shape`, `fail-agent-message-without-text`, `fail-invalid-json`
- **입력 경계**: 모든 fixture를 1·2·3·5·7·13·64·1000 byte 조각과 한 번에 넣어 같은 golden이 나와야 합니다(한글·emoji 다중 byte 문자, 줄 경계 포함).
- fixture의 id는 고정 가짜 UUID(`00000000-…`)이고 cwd는 `/tmp/work`입니다. 사용자 경로·계정·토큰이 없습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-agent-host` | 통과(아래 표의 대상별 수, 실패 0) |
| `cargo clippy -p stellar-agent-host --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-agent-host -- --check` | 통과 |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |
| `cargo xtask validate-adr --base edfea9db` | 종료 코드 0 |
| `STELLAR_CHECK_BASE=edfea9db cargo xtask check-core` | lane 최종 실행 결과는 lane 보고(통합 기록)에 적음 |

| 검사 | 결과 |
|---|---|
| fixture 목록 golden(`INDEX`, 19개) 일치 | 통과 |
| 모든 fixture에 기대 결과 파일 존재 | 통과 |
| 모든 launch profile(공식 2, 대역 2)에 정상·실패 fixture 존재 | 통과 |
| fixture privacy scan(XVI.8: 메일 `@`, `/Users/`, `/home/`, `sk-`, `Bearer `, api key, password) 0건 | 통과 |

- 성능 예산 작업이 아니며 자동 생성 파일 변경은 없습니다.

## 차이(작업 지시와 다른 점)

- 사례 근거는 위 각 CLI 공식 문서입니다. 실제 CLI 출력 녹화본이 아니라 문서 정의로 만든 합성 fixture입니다.

## 남은 위험 / 한계

- 실제 CLI 출력과의 차이(문서에 없는 field·순서)는 릴리스 전 수동 검증(XXIX.6.8)에서 확인하고, 필요하면 fixture를 추가합니다.
- golden 갱신은 `STELLAR_BLESS_DECODER_GOLDEN=1`로만 하고 사람이 diff를 검토해야 합니다.
