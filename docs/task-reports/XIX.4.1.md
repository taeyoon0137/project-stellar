# 작업 보고서: XIX.4.1 BeginAgentEdit tool 정의(target root StableId 목록과 작업 설명 제출)

- WBS: XIX.4.1
- 작업명: BeginAgentEdit tool 정의(target root StableId 목록과 작업 설명 제출)
- 상태: 완료 (`[x]`)
- 선행: XIX.2.3, XIX.3.4
- 결정: [ADR-0093](../adr/ADR-0093-agent-permission-and-edit-lease.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/mcp-core/src/lease.rs`(신규) — `BEGIN_AGENT_EDIT` ToolDef, `parse_begin_agent_edit`, `BeginAgentEdit::resolve`.
- `crates/mcp-core/src/lease.rs`의 test module — 인자·대상 거부 test(브리지 파일 변경 API 검사가 파일 끝 test module만 제외하므로 같은 파일에 둠).
- `crates/mcp-core/src/lib.rs` — module 등록.

## 구현 요약

- tool 이름 `document.begin_agent_edit`, attached 전용, 권한 Propose. 인자는 `document`, `targets`(1\~1,000 StableId), `externals`(선택, `{kind, id}`), `description`(1\~2,000자). schema는 `additionalProperties: false`.
- 빈 `targets`는 `empty-targets`, 문서에 없는 node는 `missing-node`(not-found), 다른 문서는 not-found, 중복·서로 겹치는 대상·빈 설명·모르는 resource 종류·선언 밖 key는 invalid-arguments입니다.
- 해석 결과는 `Lease`(subtree root와 잠금 시점 조상, 전역 resource)로 바뀝니다. 잠금은 승인 뒤에만 합니다(XIX.4.2).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-commands --locked` | 185 passed, 5 ignored(자식 프로세스 전용 1건 포함) |
| `cargo test -p stellar-mcp-core --locked` | 모든 test target 통과(lib 78 passed) |
| `cargo clippy -p stellar-commands -p stellar-mcp-core --all-targets --locked -- -D warnings` | 경고 0 |
| `cargo xtask validate-adr --base edfea9db` | 0 |
| `cargo xtask generate-ffi-header --check` | 0, drift 없음 |

| 검사 | 결과 |
|---|---|
| empty/missing node 거부(`begin_agent_edit_rejects_empty_and_missing_targets`) | 통과 |
| tool 이름 규칙·attached 전용·Propose 권한·엄격 schema(`tool_definition_is_attached_only_and_well_formed`) | 통과 |

## 차이(작업 지시와 다른 점)

- tool 정의는 `stellar_mcp::lease::BEGIN_AGENT_EDIT`에 있습니다. agent-protocol catalog 목록(`DOCUMENT_TOOLS`)과 catalog golden 등록은 agent-protocol 소유라 하지 않았습니다.

## 남은 위험 / 한계

- catalog에 넣기 전까지 attached `tools/list`에 나타나지 않습니다. 등록할 때 정의를 agent-protocol로 옮기고 이 상수를 지웁니다.

## 근거 커밋

- `52aa2c2a`
