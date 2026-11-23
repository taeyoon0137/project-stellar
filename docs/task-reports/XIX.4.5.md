# 작업 보고서: XIX.4.5 같은 project의 모든 창·프로세스가 ProjectLockCoordinator로 lease 공유, 다른 worktree는 별도 identity

- WBS: XIX.4.5
- 작업명: 같은 project의 모든 창·프로세스가 ProjectLockCoordinator로 lease 공유, 다른 worktree는 별도 identity
- 상태: 부분 완료 (`[~]`)
- 선행: IV.2.3, XII.2.4
- 결정: [ADR-0093](../adr/ADR-0093-agent-permission-and-edit-lease.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/src/lock/coordinator.rs`(신규) — `ProjectLockCoordinator`, `HeldLease`, `LEASE_DIR`, 항목 파일 형식.
- `crates/commands/src/lock/coordinator_tests.rs`(신규) — 창·프로세스·stale·key test.
- `crates/mcp-core/src/lease.rs` — `open_project_coordinator`(저장 잠금과 같은 identity key).
- `crates/ops/src/lib.rs` — `stellar_commands` 재수출을 상시 공개(ADR-0093 결정 12).

## 구현 요약

- 위치는 `<저장 잠금 디렉터리>/agent-leases/<identity key>/`이고 key는 `stellar_ops::lock::lock_key`(project id ‖ canonical target)입니다. symlink로 연 같은 프로젝트는 같은 공간, 다른 worktree는 다른 공간입니다.
- lease 하나 = 파일 하나이고 쥔 쪽이 `flock(2)` 배타 잠금을 유지합니다. 조회는 잠기지 않은 항목(쥔 프로세스 종료, 비정상 종료 포함)을 지웁니다. 조회·획득·해제는 `coordinator.lock` 안에서만 합니다.

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
| multi-window fixture: 같은 프로세스의 두 조정자가 lease를 공유하고 겹침·중복 id 거부, 해제 뒤 획득(`windows_of_one_process_share_leases`) | 통과 |
| multi-process fixture: 자식 프로세스(같은 test binary)가 쥔 lease를 부모가 보고 겹침 거부, 정상 해제와 비정상 종료(`exit(3)`, 소멸자 없음) 뒤 모두 lease가 사라짐(`other_process_lease_is_shared_and_freed_when_it_exits`) | 통과 |
| 다른 worktree는 별도 identity, symlink 경로는 같은 identity(`worktrees_are_separate_project_identities`, `different_project_keys_do_not_share`) | 통과 |
| 잠금 없이 남은 항목은 조회 때 지움(`stale_entry_from_ended_owner_is_removed`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 XII.2.4가 `[x]`가 아니어서 계획 1.2 규칙 4에 따라 `[~]`입니다. 앱 창(runtime)이 이 조정자를 여는 연결은 하지 않았습니다(소유 밖, UI 재설계 대상).
- `crates/ops/src/lib.rs`의 재수출 한 줄은 소유 밖 파일이지만, mcp-core 제품 빌드가 commands 타입을 쓰려면 필요해 바꿨습니다(ADR-0093 결정 12).

## 남은 위험 / 한계

- runtime 창과 `stellar apply`가 이 조정자를 열어 lease 목록을 읽도록 연결해야 합니다.

## 근거 커밋

- `286d8c4c`
- `52aa2c2a`
- `94ba5be7`
