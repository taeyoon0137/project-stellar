# 작업 보고서: XIX.2.4 terminal·commit·push tool namespace 분리와 기본 비활성

- WBS: XIX.2.4
- 작업명: terminal, commit, push 권한을 문서 apply 권한과 별도 tool namespace로 구분하고 commit·push tool을 기본 비활성으로 둠
- 상태: 완료 (`[x]`)
- 선행: XIX.2.3(이 lane에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `tools/stellar-mcp/src/catalog/tools.rs` — `ToolNamespace`(document·terminal·git), `EnabledNamespaces`(기본 문서만), `SYSTEM_TOOLS`(`terminal.run`, `git.commit`, `git.push`), `RequiredGrant`에 terminal·git-commit·git-push, `all_tools`·`listed_tools`·`tools_list`, permission matrix test.
- `tools/stellar-mcp/src/catalog/mod.rs` — catalog snapshot에 모든 tool의 namespace·기본 활성 여부.
- `tools/stellar-mcp/tests/fixtures/catalog.json` — golden 갱신.
- `docs/task-reports/XIX.2.4.md`(신규), 계획서(XIX.2.4 `[ ]`→`[x]`).

## 구현 요약

- **namespace**: tool 이름 접두어가 namespace입니다(`document.*`, `terminal.*`, `git.*`). 아키텍처 §31.3 "문서 편집 권한과 Git·터미널 권한을 결합하지 않습니다"에 따라 namespace마다 따로 켭니다.
- **permission matrix**

| tool | namespace | 필요 권한 | backend | 기본 |
|---|---|---|---|---|
| `document.semantic_diff` | document | read | all | 켜짐 |
| `document.render_preview` | document | read | attached | 켜짐 |
| `document.validate` | document | read | all | 켜짐 |
| `document.propose_commands` | document | propose | all | 켜짐 |
| `document.apply_commands` | document | apply | attached | 켜짐 |
| `terminal.run` | terminal | terminal | attached | **꺼짐** |
| `git.commit` | git | git-commit | attached | **꺼짐** |
| `git.push` | git | git-push | attached | **꺼짐** |

- **기본 비활성**: `EnabledNamespaces::default()`는 문서 namespace만 켭니다. Git·터미널은 `with_git`·`with_terminal`로 명시적으로만 켜며, 문서 apply가 보여도 commit·push는 목록에 나타나지 않습니다. headless backend는 켜도 terminal·git·쓰기 tool이 없습니다(모두 attached 전용).
- 목록 노출은 권한이 아닙니다. 노출된 tool 호출도 각 권한의 독립 확인(XIX.3.4)을 거칩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-mcp` | lib 48 passed, 통합 test 전부 passed(`catalog_fixtures` 포함), 0 failed |
| `cargo clippy -p stellar-mcp --all-targets -- -D warnings` | 경고 0(첫 실행의 `derivable_impls` 경고 → `#[derive(Default)]`로 해결) |
| `cargo fmt -p stellar-mcp -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| permission matrix: 8개 tool의 (namespace, 필요 권한) 고정, 문서 namespace만 read·propose·apply, 터미널·Git namespace만 terminal·git 권한, 이름 고유·규칙 준수(`permission_matrix_separates_document_terminal_and_git`) | 통과 |
| 기본 목록은 문서 tool 5개뿐, Git을 켜면 commit·push만 추가, 터미널을 켜면 run만 추가, headless는 둘 다 켜도 읽기 3개뿐, 기본 `tools/list`에 `git.`·`terminal.` 없음(`commit_push_and_terminal_are_disabled_by_default`) | 통과 |
| catalog golden에 namespace·기본 활성 여부 포함 | 통과 |

- 성능 작업이 아니며 자동 생성 파일 변경은 없습니다.

## 차이(작업 지시와 다른 점)

- 작업 지시는 commit·push 기본 비활성을 요구합니다. 터미널 namespace도 기본 비활성으로 두었습니다. 내장 세션 카탈로그에는 터미널 실행이 없다는 아키텍처 §31.3과 맞추고, MCP 경로의 터미널 실행 자체가 아직 구현되지 않았기 때문입니다.

## 남은 위험 / 한계

- namespace를 켜는 사용자 설정 경로와 저장은 아직 없습니다. 켜는 수단은 앱 UI에만 두어야 하며 MCP 경로에서 바꿀 수 없어야 합니다(XIX.9.7 inventory).
- 파일 삭제 권한(§31.3)에 해당하는 tool은 정의하지 않았습니다.
