# 작업 보고서: XIX.2.1 MCP read resource 정의

- WBS: XIX.2.1
- 작업명: project structure, current document, selection, components, tokens, target profile, Git status read resource를 MCP resource·template과 내용 schema로 정의
- 상태: 완료 (`[x]`)
- 선행: XIX.1.4(`[x]`), G8(gate, 관련 코드 있음)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `tools/stellar-mcp/src/catalog/mod.rs`(신규) — catalog 모듈, `catalog_snapshot`(정의 golden용).
- `tools/stellar-mcp/src/catalog/resources.rs`(신규) — `ResourceDef`, `RESOURCES`(8개), `Availability`·`Backend`, `resources_list`·`templates_list`.
- `tools/stellar-mcp/src/lib.rs` — `catalog` 모듈.
- `tools/stellar-mcp/tests/catalog_fixtures.rs`(신규), `tests/fixtures/catalog.json`(신규) — catalog schema golden fixture.
- `docs/task-reports/XIX.2.1.md`(신규), 계획서(XIX.2.1 `[ ]`→`[x]`).

## 구현 요약

- MCP 명세 2025-11-25 "Resources" 형태로 정의했습니다. 고정 URI는 `resources/list`의 `uri`, 인자가 있는 것은 `resources/templates/list`의 `uriTemplate`(RFC 6570)입니다. 모든 내용은 `application/json`이며 resource마다 내용 JSON Schema(draft 2020-12)가 있습니다.

| name | URI | backend | page |
|---|---|---|---|
| `project-structure` | `stellar://project/structure` | all | 아님 |
| `current-document` | `stellar://document/current{?cursor,limit}` | attached 전용 | 예 |
| `document` | `stellar://documents/{document}{?cursor,limit}` | all | 예 |
| `selection` | `stellar://selection` | attached 전용 | 아님 |
| `components` | `stellar://components` | all | 아님 |
| `tokens` | `stellar://tokens` | all | 아님 |
| `target-profiles` | `stellar://target-profiles` | all | 아님 |
| `git-status` | `stellar://git/status` | all | 아님 |

- "현재 문서"와 선택 영역은 창 세션이 있어야 하므로 attached 전용이고, headless에서는 문서 id로 고르는 `document` template을 씁니다(ADR-0005 §3 backend).
- 모든 내용 schema는 `revision`(XXIV.4.2 표기 `g0`·`g<n>:sha256:<hex>`)을 필수로 갖고, page 내용은 `next_cursor`를 필수로 갖습니다(XIX.2.2).
- URI에는 프로젝트를 고르는 segment가 없습니다. 모든 resource는 세션에 결합된 승인 프로젝트 하나의 범위입니다(XIX.2.5).
- Git status 내용은 경로·URL 없이 개수와 branch만 담도록 정의했습니다.
- 이 작업은 정의입니다. 서버는 backend가 연결될 때까지 `resources` capability를 광고하지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-mcp` | lib 38 passed, 통합 test 전부 passed(`catalog_fixtures` 1 포함), 0 failed |
| `cargo clippy -p stellar-mcp --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-mcp -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| schema fixture: catalog 전체(정의 + 내용 schema)가 `tests/fixtures/catalog.json`과 byte 일치(`catalog_matches_golden_fixture`) | 통과 |
| 모든 내용 schema가 엄격 JSON reader로 읽히고 `type: object`, `revision` 필수, page resource는 `next_cursor` 필수·`{?cursor,limit}` template(`schemas_are_valid_json_and_require_revision`) | 통과 |
| §31.2 대표 resource 7종이 정확히 한 번 정의, URI 고유(`every_architecture_resource_is_defined_once`) | 통과 |
| headless 목록에 attached 전용(selection, current document) 없음(`headless_lists_exclude_attached_only_resources`) | 통과 |

- 성능 작업이 아니며 자동 생성 파일 변경은 없습니다.

## 차이(작업 지시와 다른 점)

- 작업 지시의 7종에 headless용 `document`(문서 id template)를 더했습니다. headless backend에는 "현재 문서"를 정할 창이 없기 때문입니다.
- §31.2의 "컴포넌트와 인스턴스", "토큰과 Binding"은 저장 포맷이 아직 없어 component·token 항목 목록만 정의했습니다.

## 남은 위험 / 한계

- `resources/read` 처리와 backend 연결은 XIX.9.3·XIX.9.4, 정의의 `stellar-agent-protocol` 이동과 생성은 XIX.9.2에서 합니다(현재 둘 다 선행 미충족).
- 프로젝트 유래 문자열(이름 등)을 지시로 해석되지 않게 표현하는 방식은 XIX.9.6에서 정합니다.
