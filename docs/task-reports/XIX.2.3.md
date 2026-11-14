# 작업 보고서: XIX.2.3 MCP 문서 tool 분리

- WBS: XIX.2.3
- 작업명: tool을 semantic diff 생성, render preview 요청, document validation, command proposal, command apply로 분리 정의
- 상태: 완료 (`[x]`)
- 선행: XIX.2.1(이 lane에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `tools/stellar-mcp/src/catalog/tools.rs`(신규) — `ToolDef`, `DOCUMENT_TOOLS`(5개), `RequiredGrant`, `valid_tool_name`, `descriptor`, `document_tools`.
- `tools/stellar-mcp/src/catalog/mod.rs` — `tools` 모듈, catalog snapshot에 tool 정의·필요 권한 추가.
- `tools/stellar-mcp/tests/fixtures/catalog.json` — tool inventory를 담도록 golden 갱신.
- `docs/task-reports/XIX.2.3.md`(신규), 계획서(XIX.2.3 `[ ]`→`[x]`).

## 구현 요약

| name | 동작 | annotation | backend | 필요 권한 |
|---|---|---|---|---|
| `document.semantic_diff` | 저장된 프로젝트와 Git 기준(기본 HEAD)의 semantic diff | readOnly, idempotent | all | read |
| `document.render_preview` | 결합된 창에서 node 렌더 미리보기 요청 | readOnly, idempotent | attached 전용 | read |
| `document.validate` | 문서 validator 검증 | readOnly, idempotent | all | read |
| `document.propose_commands` | closed enum command 검증 + semantic diff(dry-run, 미적용) | readOnly, idempotent | all | propose |
| `document.apply_commands` | 앱 lease·승인·DocumentThread 경로로 적용 | destructive | attached 전용 | apply |

- MCP 명세 2025-11-25 "Tools" 항목(`name`, `title`, `description`, `inputSchema`, `annotations`)을 씁니다. 이름은 명세 규칙(1~128자, ASCII 영문·숫자·`_`·`-`·`.`)과 `<namespace>.<동작>` 형태를 따르고, 모든 input schema는 `additionalProperties: false` object입니다.
- annotation은 client 표시용 힌트입니다. 허용 여부는 Stellar 승인(XIX.3)이 정하고, 각 tool의 필요 권한 이름(아키텍처 §31.3)을 정의에 함께 둡니다.
- headless backend는 읽기·검증·dry-run 세 tool만 제공합니다(ADR-0005 §3). render preview는 attached 전용(ADR-0005 §8), 적용은 앱 승인·lease가 필요해 attached 전용입니다.
- command 입력은 XXIV.4.4 `stellar-apply` command 형식(`kind` + 필드 또는 `canonical`)과 같은 모양이며, 종류별 schema 생성은 XIX.9.2(xtask가 closed enum에서 생성)의 범위입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-mcp --lib catalog::tools` | 3 passed |
| `cargo test -p stellar-mcp --test catalog_fixtures` | 1 passed(golden 갱신 후) |
| `cargo clippy -p stellar-mcp --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-mcp -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| tool inventory 정확: 문서 tool 정확히 다섯 개·고정 순서, 이름 규칙, input schema가 엄격 JSON object, 쓰기 tool만 destructive(`document_tools_are_the_five_planned_operations`) | 통과 |
| headless 목록은 읽기·검증·dry-run 3개만, 모두 readOnly(`headless_backend_lists_no_write_or_render_tool`) | 통과 |
| 이름 규칙 경계(128자 허용·129자 거부, 공백·`/`·`:` 거부)(`tool_name_rule_follows_the_spec`) | 통과 |
| catalog golden에 tool 정의·annotation·필요 권한 포함(`catalog_matches_golden_fixture`) | 통과 |

- 성능 작업이 아니며 자동 생성 파일 변경은 없습니다.

## 차이(작업 지시와 다른 점)

- 없음. 정의 작업이며 `tools/call` 처리와 backend 연결은 XIX.9.3·XIX.9.4, 결과 종류는 XIX.9.5에서 합니다.

## 남은 위험 / 한계

- 서버는 backend가 연결될 때까지 `tools` capability를 광고하지 않습니다.
- `document.semantic_diff`의 Git 기준 읽기에는 신뢰된 프로젝트의 시스템 Git 실행(XVI.7)이 필요합니다.
