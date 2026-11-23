# 작업 보고서: XVI.8.6 꺼진 기능(터미널·MCP) payload field 부재 검사

- WBS: XVI.8.6
- 작업명: 터미널·MCP가 초기 출시 feature off인 빌드에서도 해당 payload 필드가 schema에 존재하지 않음을 검사
- 상태: 완료 (`[x]`) — 검증 기준(feature inventory 통과)을 끝냈습니다.
- 결정: ADR-0109 결정 8, 9
- 선행: XVI.8.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/diagnostics/src/store_inventory.rs`(신규).
- `crates/diagnostics/src/lib.rs`(module 등록 한 줄).
- `tools/stellar-cli/src/diagnostics.rs`(`stellar diagnostics schema`).
- `tools/stellar-cli/src/cli.rs`, `tools/stellar-cli/tests/fixtures/help.txt`(명령 설명).
- `docs/task-reports/XVI.8.6.md`(신규).

## 구현 요약

- `schema_inventory`가 진단 파일 형식 이름 4개(store, 번들, 동의, 대기열)와 줄 schema 8개(env, meta, crash, renderer_error, perf, feature_failure, end, consent)의 tag, field 이름, 닫힌 enum 이름을 모읍니다. 번들 schema 목록(`BUNDLE_SCHEMAS`)은 scanner 구조 검사와 같은 것입니다.
- `check_feature_inventory`는 항목마다 꺼진 기능 단어 20개(`terminal`, `pty`, `tty`, `shell`, `console`, `command_line`, `stdin`, `stdout`, `stderr`, `mcp`, `jsonrpc`, `json_rpc`, `ipc`, `tool_call`, `agent`, `assistant`, `prompt`, `transcript`, `chat`, `session`)를 ASCII 대소문자 무시 부분 문자열로 찾습니다. 빈 목록이 통과입니다.
- 기능 목록(`Feature`)에도 터미널·MCP·에이전트 값이 없어서 그 기능의 실패 카운트를 남길 자리도 없습니다(XVI.8.1).
- 기능을 끈 빌드와 켠 빌드의 schema가 같다는 근거: 진단 schema 소스에 Cargo feature 조건부 컴파일이 없고, `stellar-diagnostics` manifest에 `[features]`가 없습니다. 테스트가 소스와 manifest를 읽어 확인합니다.
- CLI `stellar diagnostics schema`: 형식 이름, schema(field 이름·종류·허용 값), 검사 단어, 위반 목록을 냅니다. 위반이 있으면 1(`disabled-feature-field`)입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-diagnostics --lib` | 72 passed(store_inventory 3개 포함), 0 failed |
| `cargo test -p stellar-cli --lib` | 32 passed(diagnostics 6개 포함), 0 failed |
| `cargo test -p stellar-cli --test headless_help` | 6 passed |
| `cargo clippy -p stellar-cli -p stellar-diagnostics --all-targets -- -D warnings` | 경고 0 |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `diagnostic_schema_has_no_disabled_feature_fields`: 검증 기준(feature inventory 통과). 위반 0, 모든 schema·형식 이름이 inventory에 있음.
- `violations_are_detected`: 꺼진 기능 단어가 든 형식 이름·field·enum 값(대문자 포함)을 위반으로 잡음.
- `schema_does_not_depend_on_build_features`: schema 소스 6개에 feature 조건부 컴파일 없음, manifest에 `[features]` 없음.
- CLI `schema_lists_every_field_and_no_disabled_feature`: 위반 0, schema tag 순서, 출력 전체(검사 단어 목록 제외)에 꺼진 기능 단어 없음.

## 성능

- 해당 없음(정적 목록 검사).

## 차이(작업 지시와 다른 점)

- 계획 문구는 터미널·MCP만 말하지만 아키텍처 §39가 에이전트 입출력도 수집 금지로 두므로 에이전트 단어도 검사합니다. XXIX.6.4가 더할 에이전트 field 이름도 이 목록과 XVI.8.2 corpus에 더하면 됩니다.

## 남은 위험 / 한계

- 검사는 단어 목록 기반입니다. 꺼진 기능을 다른 이름으로 부르는 field(예: 줄임말)는 목록에 더해야 잡힙니다. 지금 schema에는 문자열 field가 없어 그런 field가 생기려면 schema 변경과 ADR이 필요합니다.
- 앱·릴리스 빌드 전체의 feature inventory(명령·메뉴·API)는 XVI.11.8 범위입니다.
