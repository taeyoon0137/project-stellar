# 작업 보고서: XIX.3.2 권한을 ProjectIdentity+AgentSessionId별로 저장하고 app 재시작 후 유지하지 않음

- WBS: XIX.3.2
- 작업명: 권한을 ProjectIdentity+AgentSessionId별로 저장하고 app 재시작 후 유지하지 않음
- 상태: 완료 (`[x]`)
- 선행: XIX.3.1, IV.2.3
- 결정: [ADR-0093](../adr/ADR-0093-agent-permission-and-edit-lease.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/mcp-core/src/permission.rs`(신규) — `AgentSessionId`, `PermissionKey`(project id + canonical target + 세션), 메모리 전용 `PermissionStore`.
- `crates/mcp-core/src/permission_tests.rs`(신규) — 재시작·key 분리 test.
- `crates/mcp-core/src/lib.rs` — module 등록.

## 구현 요약

- 권한 상태는 `PermissionKey`별 `BTreeMap`에만 있습니다. 파일·직렬화 경로가 없어서 새 `PermissionStore`(앱 재시작)는 항상 빈 상태입니다.
- key는 IV.2.3 프로젝트 identity(project id, canonical target)와 에이전트 세션 id입니다. 같은 프로젝트의 다른 worktree나 다른 세션은 서로 권한을 보지 않습니다.
- `end_session`은 세션 권한을 모두 버립니다(§31.5 세션 종료).

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
| restart 후 permission 없음: 승인 뒤 저장소를 버리고 새로 만들면 모든 권한이 `AwaitingConnection`, 확인 상태 `NotRequested`(`permissions_do_not_survive_restart`) | 통과 |
| 다른 세션·다른 worktree key는 권한을 공유하지 않고, 세션 종료는 권한을 버림(`permissions_are_keyed_by_project_identity_and_session`) | 통과 |

## 차이(작업 지시와 다른 점)

- `PermissionKey::new`는 `ProjectIdentity` 값 대신 그 두 필드(project id, canonical target)를 받습니다. mcp-core 제품 빌드에서 `ProjectIdentity` 타입이 공개 경로로 재수출되지 않기 때문입니다.

## 남은 위험 / 한계

- 앱(runtime)은 ADR-0005 §8에 따라 mcp-core에 닿을 수 없으므로, 앱이 이 저장소를 소유하려면 XXIX.3.1의 에이전트 편집 구현 crate로 옮겨야 합니다(ADR-0093 결정 12).

## 근거 커밋

- `967b2fd3`
