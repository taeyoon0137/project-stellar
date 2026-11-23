# 작업 보고서: XIX.2.5 MCP payload 프로젝트·세션 범위 제한

- WBS: XIX.2.5
- 작업명: 모든 MCP payload에서 agent에 전달되는 프로젝트 내용을 사용자가 승인한 프로젝트·세션으로 제한
- 상태: `[x]` 판정 — 통합 때 계획서 표기를 `[x]`로 바꿉니다(2026-11-23 재판정, 아래 "재판정" 절). 이전 기록: 부분 완료 (`[~]`) — 범위 검사(요청 URI·tool 인자·응답 payload)와 cross-project 거부 fixture는 끝났습니다. 작업 지시의 "모든 MCP payload" 중 headless 읽기 backend 일부만 이 검사를 거치고, attached와 나머지 resource·tool 연결은 남았습니다. 아래 "2026-11-15 검토 메모"를 참고합니다.
- 선행: XIX.1.4(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `tools/stellar-mcp/src/catalog/scope.rs`(신규) — `ApprovedScope`(`check_resource_uri`, `check_tool_arguments`, `release`), `ProjectDocuments` trait, `ScopedPayload`, `ResourceTarget`, `ScopeError`.
- `tools/stellar-mcp/src/catalog/mod.rs` — `scope` 모듈.
- `docs/task-reports/XIX.2.5.md`(신규), 계획서(XIX.2.5 `[ ]`→`[x]`).

## 구현 요약

- 승인 범위는 프로젝트 하나(프로젝트 식별 key, IV.2.3)와 세션 하나입니다. headless는 실행 인자로 고정한 프로젝트(ADR-0005 §3), attached는 결합 창 세션이 범위를 정합니다.
- **요청 resource URI**: `stellar` scheme의 정의된 resource·template(XIX.2.1)만 받습니다. 다른 scheme(`file:`, `https:` 등) `ForeignScheme`, 정의 밖 경로(`stellar://projects/<다른 프로젝트>/...` 포함) `UnknownResource`, `..`·`.`·percent encoding·빈 segment·끝 `/`·page가 아닌 resource의 query·page resource query의 `cursor`·`limit` 밖 key·중복·형식 오류(XIX.2.2 `parse_page_query`) `MalformedUri`, 승인 프로젝트에 없는 문서 id `ForeignDocument`입니다. URI 자체에는 프로젝트를 고르는 수단이 없습니다.
- **tool 인자**: object여야 하고 input schema `properties`에 선언된 key만 받습니다(`project`·`path` 등 선언 밖 key `UndeclaredArgument`). `document` 인자는 승인 프로젝트 소속이어야 합니다. 정의된 어떤 tool도 프로젝트·경로·URI 인자를 선언하지 않음을 test로 고정했습니다.
- **응답 payload**: backend가 만든 payload에 출처 프로젝트·세션 표지를 붙이고, 승인 범위와 다르면 내보내지 않습니다(`ForeignProject`·`ForeignSession`, 심층 방어).
- 문서 소속 판정은 `ProjectDocuments` trait로 backend가 구현합니다(headless: `stellar-ops` snapshot 문서 목록, attached: 앱).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-mcp` | lib 51 passed, 통합 test 전부 passed, 0 failed |
| `cargo clippy -p stellar-mcp --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-mcp -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| cross-project access 거부: 다른 scheme 2, 프로젝트 선택 경로 2, 다른 프로젝트 문서, `..`·`%2e%2e`·빈 segment·잘못된 query·빈 URI 각각 정해진 오류(`cross_project_access_is_rejected`) | 통과 |
| tool 인자 `project`·`path`·다른 프로젝트 문서·object 아님 거부, 승인 문서는 허용 | 통과 |
| 다른 프로젝트·다른 세션 payload 내보내기 거부, 같은 범위는 허용 | 통과 |
| 정의 resource 5종 URI와 승인 문서 template 해석(`defined_resources_of_the_approved_project_are_accepted`) | 통과 |
| page resource query의 정의 밖 key(`project`·`root`)·중복 `limit`·`limit=010`·빈 `cursor`·값 없는 key 거부, `limit`·`cursor` 조합 허용(`paginated_query_accepts_only_cursor_and_limit`) | 통과 |
| 어떤 tool도 project·path·root·uri 인자를 선언하지 않음(`no_tool_declares_a_project_or_path_argument`) | 통과 |

- 성능 작업이 아니며 자동 생성 파일 변경은 없습니다.

## 차이(작업 지시와 다른 점)

- 작업 지시는 "모든 MCP payload"의 범위 제한이지만, 이 작업은 검사 함수(`ApprovedScope`)까지만 만들었습니다. catalog(XIX.2.1~XIX.2.4)가 아직 server에 광고·연결되지 않아 `resources/read`·`tools/call`·응답 경로에서 이 검사를 부르는 지점이 없고, test도 순수 함수만 다룹니다. server 연결은 XIX.9.3·XIX.9.4에서 합니다.

## 남은 위험 / 한계

- **후속(`[x]`로 올리기 위한 남은 일)**: server가 catalog를 광고할 때 모든 `resources/read` 요청 URI에 `check_resource_uri`, 모든 `tools/call` 인자에 `check_tool_arguments`, agent로 나가는 모든 resource·tool 응답에 `release`를 거치게 연결하고, 실제 server 왕복 fixture로 cross-project 요청 거부를 확인한 뒤 `[x]`로 바꿉니다.

- 승인(첫 연결 승인 XIX.3.3)과 trust 확인(XVI.7)은 이 모듈의 범위가 아닙니다. 범위 검사는 승인된 범위를 넘지 않게 할 뿐 승인을 만들지 않습니다.
- 응답 표지 검사는 backend가 표지를 정직하게 붙인다는 전제의 심층 방어입니다.

## 2026-11-15 검토 메모

- 범위 검사 모듈은 XIX.9.1 이동으로 `crates/mcp-core/src/catalog/scope.rs`에 있습니다.
- headless backend(`crates/mcp-core/src/headless.rs`)의 resource 읽기는 `check_resource_uri`와 `release`, tool 호출은
  `check_tool_arguments`를 거칩니다. 이 backend의 행 XIX.9.3은 선행 미완료로 `[!]`입니다.
- 이 절에 처음 적었던 stdio 프로세스 검사 결과와 "tools/list는 비어 있음"은 Codex 보고였고 지금 코드와도 맞지 않아 지웠습니다.

## 재판정 (2026-11-23, ledger-verification)

- 판정: `[x]` 판정 — 통합 때 계획서 표기를 `[x]`로 바꿉니다.
- 근거: 프로젝트 내용을 agent에 내보내는 모든 MCP 경로가 승인 범위 검사를 거칩니다. 이전 `[~]` 사유(일부 backend만 연결)는 지금 코드와 맞지 않습니다. 프로젝트 내용을 내보내는 backend는 headless 하나이고, 기본·attach 세션은 ping만 제공합니다.
- 판정 기준: 행 지시·검증 열의 불변식을 구현 코드에서 확인했고, 테스트 통과는 보조 근거로만 썼습니다. 계획서 §1.2 규칙 4(선행이 모두 `[x]`여야 완료)를 적용했습니다. 기준 커밋은 `main` `ce60bb92`입니다.

### 확인한 코드 위치

- `crates/mcp-core/src/server.rs` 175\~189행: `resources/read`·`tools/call` 등은 headless backend가 있을 때만 처리하고, 없으면 내용 method를 제공하지 않습니다.
- `crates/mcp-core/src/headless.rs`: `call`은 `check_tool_arguments`(123행), `read_cached`는 `check_resource_uri`(229행), 모든 응답은 `with_snapshot` 끝의 `release`(206행)를 거칩니다. 응답 직전에 신뢰·snapshot 최신성을 다시 확인합니다.
- `crates/mcp-core/src/catalog/scope.rs` `ApprovedScope`: 다른 scheme·정의 밖 경로·다른 프로젝트 문서·선언 밖 인자·다른 프로젝트·세션 표지 payload를 거부합니다.

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test --workspace`(DEV-MAC, 샌드박스 해제) | 테스트 binary 142개 모두 ok, 4136 passed, 0 failed |
| `cargo test -p stellar-mcp-core` | 58 passed |
| `cargo test -p stellar-mcp --test headless_read` | 11 passed, 1 ignored(`release_page_rtt` 측정). `scope_revision_limits_and_write_requests_return_normal_errors`가 실제 stdio server 왕복으로 다른 프로젝트 URI·`project` 인자·쓰기 tool을 -32602로 거부하고 파일 digest가 같음을 확인합니다. |

### 남은 조건과 finding

- attached backend가 프로젝트 내용을 내보내기 시작하면(agent-session-core lane) 같은 `ApprovedScope`를 거치게 해야 합니다. 그 전까지 이 판정은 유지됩니다.
