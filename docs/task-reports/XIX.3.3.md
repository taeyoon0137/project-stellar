# 작업 보고서: XIX.3.3 attached 첫 연결 승인 전 Read 불허, headless 신뢰 프로젝트 읽기·검증·dry-run만, untrusted 전 tool 거부

- WBS: XIX.3.3
- 작업명: attached 첫 연결 승인 전 Read 불허, headless 신뢰 프로젝트 읽기·검증·dry-run만, untrusted 전 tool 거부
- 상태: 완료 (`[x]`)
- 선행: XVI.7.2, XIX.3.2
- 결정: [ADR-0093](../adr/ADR-0093-agent-permission-and-edit-lease.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/mcp-core/src/permission.rs` — `BackendPolicy`, `ConnectionApproval`, `PermissionStore::{connect, respond_connection, decide}`.
- `crates/mcp-core/src/permission_tests.rs` — 정책 test와 headless untrusted fixture.

## 구현 요약

- attached: 신뢰된 프로젝트라도 세션 등록 직후 첫 연결 승인 상태는 `Pending`이고, 이때 Read를 포함한 모든 권한이 `AwaitingConnection`(prompt 대기)입니다. 거부하면 `Denied(ConnectionDenied)`, 승인하면 Read만 허용됩니다. 첫 응답만 유효합니다.
- headless: 신뢰된 고정 프로젝트에서 `Permission::is_non_mutating`(Read·Propose)만 `Allow`, 나머지는 `Denied(NotOffered)`입니다. headless `tools/list`(모든 namespace를 켠 경우 포함)의 tool이 요구하는 권한이 모두 이 정책에서 허용되는지 test가 확인해, 목록과 정책이 같은 판정을 씁니다.
- 신뢰되지 않은 프로젝트는 attached·headless 모두 모든 권한 `Denied(NotTrusted)`입니다. headless backend는 신뢰 등록이 없으면 선택 자체가 실패해 어떤 tool도 응답하지 않습니다(기존 `TrustedProject::select`).
- 앱이 시작한 내장 세션의 첫 연결 승인 기록(XXIX.3.5)은 `respond_connection`을 쓰면 됩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-commands --locked` | 185 passed, 5 ignored(자식 프로세스 전용 1건 포함) |
| `cargo test -p stellar-mcp-core --locked` | 모든 test target 통과(lib 82 passed) |
| `cargo clippy -p stellar-commands -p stellar-mcp-core --all-targets --locked -- -D warnings` | 경고 0 |
| `cargo xtask validate-adr --base edfea9db` | 0 |
| `cargo xtask generate-ffi-header --check` | 0, drift 없음 |

| 검사 | 결과 |
|---|---|
| attached 첫 연결 denied/prompt state: 승인 전 모든 권한 `AwaitingConnection`, 거부 뒤 `ConnectionDenied`, 중복 응답 `ConnectionAnswered`(`attached_connection_needs_first_approval_even_for_read`) | 통과 |
| headless untrusted fixture 전 tool 거부: 신뢰 등록 없는 프로젝트 선택 실패(`untrusted_headless_fixture_rejects_selection`), 정책 전부 `NotTrusted` | 통과 |
| headless 정책 결과에 쓰기 권한 0: 허용 집합이 정확히 Read·Propose, headless tool 목록의 권한이 모두 허용 집합 안(`headless_policy_allows_only_non_mutating_permissions`) | 통과 |

## 차이(작업 지시와 다른 점)

- attached backend 진입(XIX.9.4)이 아직 없어 이 판정을 부르는 MCP 요청 dispatcher는 없습니다. 판정 함수와 상태만 구현했습니다.

## 남은 위험 / 한계

- attached dispatcher가 생기면 요청마다 `decide`·`authorize`를 부르도록 연결해야 합니다.

## 근거 커밋

- `967b2fd3`
