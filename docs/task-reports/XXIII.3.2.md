# 작업 보고서: XXIII.3.2 새 branch·worktree는 시스템 Git으로만 생성

- WBS: XXIII.3.2
- 작업명: new branch/worktree 생성은 시스템 Git으로만 실행
- 상태: 완료 (`[x]`)
- 선행: XIV.1.3, XXIII.3.1
- 결정: [ADR-0110](../adr/ADR-0110-window-reconciliation-and-merge-result-apply.md) 결정 5

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git fixture).

## 변경 파일 목록

- `crates/ops/src/merge_apply/target.rs` — `create_branch_worktree`.
- `crates/ops/src/merge_apply/tests/target.rs` — 생성·실패 시험.

## 구현 요약

- 검증한 `NewBranchWorktree` target을 `git worktree add -b <branch> -- <path> <start>` **하나**로 만듭니다. 명령은 git-porcelain의
  `plan_add`(고정 argv, 경로 앞 `--`, Safe 수준)와 `execute_plan`(gate 실행기)으로 실행합니다. `.git` 아래 파일을 직접 만들거나
  고치는 코드는 없습니다(이 모듈에 파일 쓰기 호출이 없음).
- 만든 뒤 target 프로젝트를 열어 project id·Git 상태를 확인하고 revision을 기대 revision으로 기록합니다(이후 generation save의
  기준).
- 실패해도 만든 branch·worktree를 지우지 않습니다. 지우기는 파괴적 명령이라 자동 실행하지 않습니다.
- 다른 target(current·existing)은 Git을 실행하지 않고 그대로 돌려줍니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops --lib merge_apply` | 통과(target 3, document 4) |
| `cargo clippy -p stellar-ops --all-targets -- -D warnings` | 경고 0 |

| 검사(`.git` 직접 수정 0) | 결과 |
|---|---|
| 새 경로의 `.git`이 Git이 쓴 gitfile, `refs/heads/result` = 시작 commit, 새 worktree HEAD = 시작 commit, 원래 worktree HEAD 그대로 | 통과 |
| `git worktree list --porcelain`에 새 branch worktree가 있음 | 통과 |
| 만든 프로젝트를 열어 기대 revision 기록 | 통과 |
| 다른 target은 Git 실행 0 | 통과 |
| 검증 뒤 경로가 점유되면 Git 거부를 `TargetError::Git`으로 알림, 점유 파일 그대로 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 생성 뒤 프로젝트 확인이 실패하면 branch·worktree가 남습니다. 오류가 경로를 알리고 정리는 사용자가 합니다.
- 생성 시점은 C 쓰기 조건과 source 재검사를 통과한 뒤입니다(XXIII.3.3·XXIII.3.4가 호출 순서를 고정).
