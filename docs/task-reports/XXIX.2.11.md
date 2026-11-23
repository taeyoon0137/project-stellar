# 작업 보고서: XXIX.2.11 표시 전용 출력 디코더

- WBS: XXIX.2.11
- 작업명: launch profile별 버전 decoder profile로 공식 구조화 출력만 해석하고 실패하면 닫히는 디코더
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 XXIX.2.9이 `[x]`가 아닙니다. 선행이 끝나면 `[x]`. 구현 보고 당시 기록: `[x]` 완료
- 선행: XXIX.2.9, XXIX.2.10, XXIX.1.1
- 설계 기준: ADR-0005 §4·§6·§8, ADR-0023 결정 9, ADR-0092

## 필수 실행 환경

- `DEV-MAC` (macOS 프로세스·파이프·process group·Code Signing Services, 로컬 검증). 네트워크와 실제 공급자 계정은 쓰지 않습니다.

## 변경 파일 목록

- `crates/agent-host/src/decoder/mod.rs`, `crates/agent-host/src/decoder/claude.rs`, `crates/agent-host/src/decoder/codex.rs`(신규)
- `crates/agent-host/tests/decoder_fixtures.rs`, `crates/agent-host/tests/inventory.rs`

## 구현 요약

- **모듈 구성**:
  - `decoder::DecoderKind`: decoder profile id + 버전. 정확히 같을 때만 resolve.
  - `decoder::Decoder`: 줄 framing, 상태, 실패 판정
  - profile별 줄 해석 `claude.rs`(`claude-stream-json@1`), `codex.rs`(`codex-exec-jsonl@1`)
- **입출력**: 입력은 CLI stdout byte 조각(순서대로)이고, 출력은 조각마다 `DecodeOutput { items, signals, failure }`입니다. `finish()`는 줄바꿈 없는 마지막 조각을 처리합니다. stderr는 디코드하지 않습니다.
- **실패 판정 기준**: `invalid-utf8`, `line-too-long`(1 MiB), `invalid-json`(엄격 JSON, 자원 상한), `not-object`, `missing-type`, `shape`(표시하는 문서화된 type의 필수 field 모양 위반). 판정 하나를 내고 닫으며, 실패한 줄의 부분 결과는 버립니다. 닫힌 뒤 입력은 item을 만들지 않습니다.
- **건너뜀**: 문서에 있지만 보이지 않는 type(예: `stream_event`, `rate_limit_event`, reasoning)과 모르는 type은 item 없이 건너뛰고 수를 셉니다. 잘못된 내용을 보이지 않으므로 닫힘 원칙과 맞습니다. 모르는 type을 실패로 두면 지원 버전 안에서 type이 늘 때 정상 세션이 닫힙니다(ADR-0092 Alternatives).
- 사용량·비용 field(`usage`, `total_cost_usd`, `modelUsage`)는 읽지 않습니다.
- **결정론**: 시각·난수 입력 없음. 같은 원문은 입력 분할과 관계없이 같은 결과입니다.
- **격리**: 공급자 출력 해석은 이 crate 안에만 있고, 다른 crate·도구·macOS 셸에는 해석 표지 0입니다.

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
| XXIX.2.9 fixture 19개 디코드 golden 일치(한 번에·8가지 조각 크기) | 통과 |
| 해석 불가 fixture(`fail-*` 9개)별 실패 판정 정확히 1 | 통과 |
| 반복 실행 결과 동일 | 통과 |
| agent-host 밖의 공급자 출력 해석 코드 0(`crates`·`tools`·`apps`의 `.rs`·`.swift`·`.h`·`.m`에서 `"thread.started"`·`"item.completed"`·`"tool_use"`·`"tool_result"`·`stream-json`·`"agent_message"`·`SDKMessage` 0) | 통과 |
| 버전이 다른 decoder profile은 resolve 거부, 줄 상한 초과 실패(단위 test) | 통과 |

- 성능 예산 작업이 아니며 자동 생성 파일 변경은 없습니다.

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- Codex 공식 문서가 사용 한도·인증 오류를 구분하는 field를 정하지 않아, Codex decoder는 `CliSignal`을 내지 않습니다. Codex 세션은 한 turn 뒤 프로세스 종료로 멈춥니다.
