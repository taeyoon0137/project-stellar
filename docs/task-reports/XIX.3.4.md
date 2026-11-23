# 작업 보고서: XIX.3.4 Apply, Terminal, GitCommit, GitPush 요청의 독립 확인 상태

- WBS: XIX.3.4
- 작업명: Apply, Terminal, GitCommit, GitPush 요청의 독립 확인 상태
- 상태: 완료 (`[x]`)
- 선행: XIX.3.1
- 결정: [ADR-0093](../adr/ADR-0093-agent-permission-and-edit-lease.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/mcp-core/src/permission.rs` — `Confirmation`, `RequestId`, `PermissionStore::{request, respond, authorize}`, `is_session_grant`.
- `crates/mcp-core/src/permission_tests.rs` — escalation fixture.

## 구현 요약

- Read를 뺀 권한은 권한마다 확인 요청 id와 상태(`NotRequested`·`Pending`·`Approved`·`Denied`)를 따로 가집니다. 한 응답은 그 요청의 권한 하나만 바꾸고, 첫 응답만 유효합니다.
- Propose·Apply·Terminal·FileDelete 승인은 세션 동안 유지되고, GitCommit·GitPush 승인은 `authorize` 한 번에 쓰여 다음 실행은 다시 확인해야 합니다(§31.3 Git 개별 확인).
- 응답 함수는 앱 사용자 UI만 부르는 경로로 문서화했습니다(ADR-0005 §3).

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
| permission escalation fixture 실패: Apply 승인 뒤 Terminal·FileDelete·GitCommit·GitPush·Propose가 모두 `AwaitingConfirmation`이고 상태가 바뀌지 않음(`confirmations_are_independent`) | 통과 |
| 요청마다 다른 id, Commit 승인이 Push를 허용하지 않음, Terminal 거부는 그 권한만, 중복 응답 거부, Commit 승인 1회 소비, Apply 승인 유지 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- XXIX.3.2(승인 요청 보관, 창 닫힘·기한 초과 거부)는 이 상태를 바탕으로 따로 구현해야 합니다.

## 근거 커밋

- `967b2fd3`
