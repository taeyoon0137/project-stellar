# 작업 보고서: XXVI.3.2 worktree 관리 명령 연결

- WBS: XXVI.3.2
- 작업명: add/remove/move/lock/unlock/prune/repair를 시스템 Git command로 연결
- 상태: 완료 (`[x]`)
- 선행: XXVI.3.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 검증; 실제 Git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/advanced/worktree_ops.rs` (신규: `plan_add`·`plan_remove`·`plan_move`·`plan_lock`·`plan_unlock`·`plan_prune`·`plan_repair`, `preview_prune`, `AddTarget`, 테스트)
- `crates/git-porcelain/src/advanced/mod.rs` (`worktree_ops` 모듈)
- `docs/task-reports/XXVI.3.2.md`(신규), 계획서(XXVI.3.2 `[ ]`→`[x]`)

## 구현 요약

- 각 연산은 `git worktree <sub>` `CommandPlan`입니다(경로는 `--` 뒤 argv, `.git` 직접 수정 없음).
  - Safe: add(`-b <new> [start]`·기존 branch·`--detach <rev>`), move, lock(`--reason`), unlock, repair.
  - 확인 필요: remove(`DiscardsWork` — ignore된 파일과 디렉터리를 지움, `--force` 없음), prune(`DiscardsRefs` — 사라진
    worktree 관리 정보 삭제). prune은 `preview_prune`(`--dry-run --verbose`, 변경 없음)으로 대상을 먼저 보여 줄 수 있습니다.
- branch·rev·lock 이유는 실행 전에 검증합니다(option 주입·개행 거부).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 220 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **command matrix fixture**(실제 Git): add(새 branch·detached) → 3개; move → 새 경로; lock(한글 이유) → 이유 표시, lock 상태 move는 Git 거부; unlock; remove는 확인 없이 spawn 0, 확인 후 디렉터리 제거; 디렉터리 삭제 후 prune 미리보기 비어 있지 않고 dry-run은 정리하지 않음, prune 확인 없이 거부, 확인 후 주 worktree만 남음; repair 성공(`command_matrix_fixture`) | 통과 |
| plan 표: 8개 명령 미리보기 문자열·수준, remove에 force 없음, 잘못된 입력 거부(`plan_matrix_levels_and_argv`) | 통과 |

## 차이(작업 지시와 다른 점)

- remove와 prune을 확인 대상으로 두었습니다(lane 지시: 파괴 수준 확인은 데이터). UI 없이 core API만 제공합니다.

## 남은 위험 / 한계

- `worktree add --lock`·`--orphan`, `move`/`remove`의 force는 제공하지 않습니다(XXVI.3.4).
- prune 미리보기 문구는 Git 출력(locale 의존 가능)을 그대로 줄 단위로 보여 줍니다.
