# 작업 보고서: XIX.4.6 여러 agent는 겹치지 않는 subtree만 동시에 lock, 동일·ancestor/descendant 요청 거부

- WBS: XIX.4.6
- 작업명: 여러 agent는 겹치지 않는 subtree만 동시에 lock, 동일·ancestor/descendant 요청 거부
- 상태: 부분 완료 (`[~]`)
- 선행: XIX.4.5
- 결정: [ADR-0093](../adr/ADR-0093-agent-permission-and-edit-lease.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/src/lock/lease.rs` — `SubtreeLock::overlaps`, `Lease::{conflict_with, check_shape}`, `LeaseTable::{first_conflict, insert}`.
- `crates/commands/src/lock/coordinator.rs` — 획득 때 같은 겹침 판정.

## 구현 요약

- 같은 문서에서 root가 같거나 한쪽 root가 다른 쪽 조상 목록에 있으면 겹칩니다. 조상 목록은 잠금 시점에 기록해 다른 프로세스도 문서 모델 없이 같은 판정을 합니다.
- 한 요청 안의 대상끼리 겹치거나 resource가 중복이면 형식 오류입니다.

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
| overlap matrix: 동일·조상·자손(깊이 2 포함)은 거부, 형제·사촌은 허용, 다른 문서는 겹치지 않음, 요청 내부 겹침·빈 요청 오류(`overlap_matrix`) | 통과 |
| 조정자·요청 관리자에서도 같은 판정(`windows_of_one_process_share_leases`, `lease_is_created_only_after_approval`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 XIX.4.5가 `[~]`라 규칙 4에 따라 `[~]`입니다. 판정 자체는 완료입니다.

## 남은 위험 / 한계

- 잠금 뒤 대상의 조상이 바뀌면(다른 lease 밖 이동) 기록한 조상 목록이 낡을 수 있습니다. 사용자 판정이 lease root 조상 이동을 거부하므로 사용자 편집으로는 생기지 않고, 외부 변경 반영 때는 세션을 끝냅니다.

## 근거 커밋

- `537dbbd7`
- `286d8c4c`
