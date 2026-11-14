# 작업 보고서: XIX.2.5 MCP payload 프로젝트·세션 범위 제한

- WBS: XIX.2.5
- 작업명: 모든 MCP payload에서 agent에 전달되는 프로젝트 내용을 사용자가 승인한 프로젝트·세션으로 제한
- 상태: 부분 완료 (`[~]`) — 범위 검사(요청 URI·tool 인자·응답 payload)와 cross-project 거부 fixture는 끝났으나, 작업 지시의 "모든 MCP payload"에 적용하려면 server의 `resources/list`·`resources/read`·`tools/call`이 이 검사를 거쳐야 하는데 catalog가 아직 server에 연결되지 않았습니다. 지금은 검사를 강제하는 호출 지점이 없습니다.
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
