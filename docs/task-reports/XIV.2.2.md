# 작업 보고서: XIV.2.2 `RepositoryStatus`

- WBS: XIV.2.2
- 작업명: `RepositoryStatus`에 HEAD, upstream 상태, staged/unstaged/untracked counts, conflict state 정의
- 상태: 완료 (`[x]`)
- 선행: XIV.2.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 검증; 실제 Git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/repo_status.rs` (신규: `RepositoryStatus`, `HeadState`, `UpstreamState`, `ChangeCounts`, `ConflictState`, `InProgressOperation`, `read_repository_status`, `read_in_progress_operations`, 테스트)
- `crates/git-porcelain/src/lib.rs` (`repo_status` 모듈)
- `docs/task-reports/XIV.2.2.md`(신규), 계획서(XIV.2.2 `[ ]`→`[x]`)

## 구현 요약

- `HeadState`: `Branch{name, commit}`, `Detached{commit}`, `Unborn{branch}`. status header의
  `branch.oid (initial)`이 unborn, `branch.head (detached)`가 detached입니다. commit 없는 detached
  조합은 Git이 만들지 않으므로 해석 오류로 거부합니다.
- `UpstreamState`: `None`, `Tracking{name, ahead, behind}`, `Gone{name}`(upstream 설정은 있으나 Git이
  `branch.ab`를 생략 = ref가 사라짐).
- `ChangeCounts`: staged(index 쪽 변경), unstaged(worktree 쪽 변경), untracked, ignored,
  conflicted(unmerged). `MM`은 staged·unstaged 모두에 셉니다.
- `ConflictState`: unmerged 경로 수 + 진행 중 작업(merge·cherry-pick·revert·rebase·am). merge 등은
  `git rev-parse -q --verify MERGE_HEAD|CHERRY_PICK_HEAD|REVERT_HEAD`(ref backend와 무관), rebase·am은
  Git이 알려 주는 `--git-path rebase-merge|rebase-apply` 디렉터리 존재로 확인합니다(읽기만 함).
- 모든 실행은 trust gate를 거칩니다. 상태는 저장된 파일 기준입니다(모듈 문서에 명시).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 132 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **unborn branch fixture**: commit 없는 저장소 + staged 1 + untracked 1 → `Unborn{main}`, upstream `None`, counts 정확(`unborn_branch_fixture`) | 통과 |
| **detached HEAD fixture**: `switch --detach` + unstaged 1 → `Detached{HEAD commit}`(`detached_head_fixture`) | 통과 |
| upstream 추적(ahead 2/behind 0) → upstream ref 삭제 후 `Gone`(`upstream_tracking_and_gone_fixture`) | 통과 |
| merge conflict → `unmerged_paths 1` + `Merge`, cherry-pick conflict → `CherryPick`, rebase conflict → `Rebase`, abort 후 비활성(`merge_and_cherry_pick_conflict_fixture`) | 통과 |
| 합성 header·항목 counts(`MM`·`M.`·`.D`·rename·`UU`·`AA`·untracked·ignored), 중복 작업 정규화, header 누락·불가능 조합 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- conflict state에 진행 중 작업 종류(merge·cherry-pick·revert·rebase·am)를 함께 담았습니다. XIV.12.5
  (자동 continue 금지)와 UI 안내가 같은 상태를 쓰도록 하기 위해서입니다.

## 남은 위험 / 한계

- `rebase-apply`가 `git am`인지 rebase인지는 Git이 쓰는 `applying` 표지 파일 존재로 구분합니다(Git
  내부 동작 의존, 읽기 전용).
- 진행 작업 확인은 실행 5회(pseudo-ref 3, git-path 2)가 추가됩니다. 대형 저장소 성능은 측정하지 않았습니다.
