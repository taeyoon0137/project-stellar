# 작업 보고서: XIX.4.4 외부 token, component definition, asset 수정은 별도 root lock 필요(자동 연쇄 잠금 없음)

- WBS: XIX.4.4
- 작업명: 외부 token, component definition, asset 수정은 별도 root lock 필요(자동 연쇄 잠금 없음)
- 상태: 부분 완료 (`[~]`)
- 선행: XIX.4.2
- 결정: [ADR-0093](../adr/ADR-0093-agent-permission-and-edit-lease.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/src/lock/lease.rs` — `ExternalKind`, `ExternalResource`, `LeaseTable::{check_external, external_holder}`.
- `crates/mcp-core/src/lease.rs` — `externals` 인자 해석.

## 구현 요약

- 전역 resource는 lease 요청에 `{kind, id}`로 명시합니다. node lease는 instance가 참조하는 정의를 자동으로 잠그지 않습니다.
- `check_external`: 사용자는 어느 lease도 쥐지 않은 resource만, 에이전트는 자기 lease가 명시적으로 잠근 resource만 바꿉니다. 같은 resource를 두 lease가 잠글 수 없습니다.

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
| unrequested external edit 거부: 잠그지 않은 component definition·token 편집은 `ExternalNotLocked`, 다른 lease의 resource는 `ExternalLeased`, 사용자도 잠긴 resource는 거부(`external_resources_need_their_own_lock`) | 통과 |

## 차이(작업 지시와 다른 점)

- token·component 원본·asset을 고치는 기존 쓰기 경로(component-source CLI 등)는 아직 `check_external`을 부르지 않습니다(소유 밖). 그래서 상태는 `[~]`입니다.

## 남은 위험 / 한계

- 전역 resource 쓰기 경로마다 `check_external` 호출을 연결해야 합니다.

## 근거 커밋

- `537dbbd7`
- `52aa2c2a`
