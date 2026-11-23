# 작업 보고서: XIX.9.2 agent-protocol catalog와 생성 CommandTransaction schema

- WBS: XIX.9.2
- 작업명: resource·tool 목록을 `stellar-agent-protocol` Rust 정의에서 생성하고, CommandTransaction 입력 schema를 xtask가 closed enum에서 생성해 `schemas/`에 둠
- 상태: 구현 완료, 원장 판정 보류 (`[~]`) — 선행 XIX.9.1이 `[~]`(XXIX.1.1 문서 대기)라서 `[x]`로 올리지 않습니다. 행 범위는 모두 구현했습니다.
- 선행: XIX.9.1(`[~]`), XIX.2.1 `[x]`, XIX.2.3 `[x]`, XIX.5.1(이 lane `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/agent-protocol/src/catalog/{mod,resources,tools}.rs`(신규) — resource·tool 정의, backend·availability, namespace, 이름 규칙. mcp-core에서 옮겼고 CLI 동등성 tool을 더했습니다.
- `crates/agent-protocol/src/schema.rs`(신규) — `COMMAND_TRANSACTION_SCHEMA = include_str!("../../../schemas/command-transaction.schema.json")`.
- `crates/agent-protocol/src/instructions.rs`(신규) — MCP `instructions` 고정 문구.
- `crates/agent-protocol/src/lib.rs` — 모듈 등록.
- `schemas/command-transaction.schema.json`(신규, 생성 산출물, 보호 경로) — ADR-0085.
- `tools/xtask/src/command_schema.rs`(신규) — `cargo xtask generate-command-schema [--check]`와 drift test.
- `tools/xtask/src/{lib,commands}.rs`, `tools/xtask/Cargo.toml` — 명령 등록, `stellar-ops` 의존 한 줄.
- `crates/mcp-core/src/catalog/{mod,resources,tools}.rs` — 정의를 재노출하고 JSON 목록만 생성.
- `crates/mcp-core/src/server.rs` — `initialize` 결과에 `instructions`.
- `tools/stellar-mcp/tests/fixtures/{catalog.json,protocol-smoke.out.jsonl,protocol-version-matrix.jsonl}` — golden 갱신.
- `docs/adr/ADR-0085-mcp-boundary-agent-protocol.md`(신규).

## 구현 요약

- **catalog 생성**: `tools/list`·`resources/list`·`resources/templates/list` JSON은 mcp-core가 agent-protocol 정의(`RESOURCES`, `DOCUMENT_TOOLS`, `SYSTEM_TOOLS`)에서만 만듭니다. annotation(`readOnlyHint`·`destructiveHint`·`idempotentHint`·`openWorldHint`)은 정의 값 그대로입니다.
- **새 tool 정의(ADR-0085 결정 8)**: 읽기·dry-run `document.source_span`, `document.export_web_frame`, `document.copy_css`, `document.propose_css_paste`, `git.status`, `git.semantic_conflicts`. attached 전용 쓰기 `document.paste_css`, `git.stage`, `git.unstage`. 상태를 바꾸는 tool은 모두 attached 전용이고, 읽기 전용 여부와 권한(Read·Propose)이 일치함을 test가 강제합니다.
- **namespace**: 터미널·Git 기본 비활성은 상태를 바꾸는 tool에만 적용합니다(ADR-0085 결정 6).
- **생성 schema**: xtask가 `CommandKind::ALL`(VII.1.6 닫힌 enum)과 ops `FIELD_SPECS`에서 JSON Schema 2020-12를 결정적으로 씁니다. 필드 종류 11개는 `const` kind + 필드, 나머지 closed enum 종류 12개는 `enum` kind + `canonical`. `document.propose_commands`·`document.apply_commands`의 `inputSchema`가 이 산출물입니다.
- **instructions**: MCP Lifecycle `InitializeResult.instructions`에 backend 범위, CLI 적용 경로, 페이지·revision 규칙, 프로젝트 문자열 신뢰 경계를 적었습니다. backend 없는 bridge 세션은 별도 짧은 안내입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo xtask generate-command-schema --check` | drift 없음 |
| `cargo test -p xtask --lib command_schema` | 2 passed |
| `cargo test -p stellar-agent-protocol` | 7 passed |
| `cargo test -p stellar-mcp-core -p stellar-mcp` | 전부 통과 |
| `cargo xtask validate-crate-graph` | 종료 0 |
| `cargo xtask validate-adr --base d2bf97b3` | 종료 0 |

| 검사 | 결과 |
|---|---|
| catalog golden 일치(`catalog_matches_golden_fixture`, 의도한 변경으로 갱신) | 통과 |
| 생성 schema와 enum variant 일치: 저장된 산출물 = 생성 결과, 모든 closed enum 종류가 정확히 한 번(`checked_in_schema_has_no_drift`, `every_closed_enum_kind_appears_exactly_once`) | 통과 |
| agent-protocol의 commands 의존 0: 실제 manifest 내부 의존이 core-types 하나(`agent_protocol_and_mcp_core_follow_adr_0005`), 전이 도달 금지 규칙 | 통과 |
| 이름 규칙·annotation·권한 matrix, 쓰기 tool attached 전용(`permission_matrix_separates_document_terminal_and_git`) | 통과 |
| `initialize`의 `instructions`(protocol smoke·version matrix golden) | 통과 |

## 차이(작업 지시와 다른 점)

- 행은 "schemas/는 보호 경로이므로 같은 변경에 ADR을 갱신"이라 합니다. 기존 ADR이 아니라 새 ADR-0085를 같은 브랜치 범위에 두었습니다.
- headless backend가 이 정의의 `availability`로 목록을 고르는 연결은 XIX.9.3 커밋에서 합니다.

## 남은 위험 / 한계

- xtask가 `stellar-ops`에 의존하게 되어 xtask 첫 빌드가 길어졌습니다.
- `outputSchema`는 정의하지 않았습니다(명세상 선택).
