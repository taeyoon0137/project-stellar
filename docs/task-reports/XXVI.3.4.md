# 작업 보고서: XXVI.3.4 untracked worktree remove 거부 결과 그대로 표시

- WBS: XXVI.3.4
- 작업명: untracked file이 있는 worktree remove를 Git이 거부하면 결과를 그대로 표시하고 force를 기본 제공하지 않음
- 상태: 완료 (`[x]`)
- 선행: XXVI.3.2(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 검증; 실제 Git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/advanced/worktree_ops.rs` (`remove_worktree`, `RemoveError`, 테스트)
- `docs/task-reports/XXVI.3.4.md`(신규), 계획서(XXVI.3.4 `[ ]`→`[x]`)

## 구현 요약

- `remove_worktree`는 확인된 `worktree remove` plan만 실행합니다. Git이 거부(untracked·변경 파일, lock 등)하면
  `RemoveError::RefusedByGit{exit_code, message}`로 Git stderr(UTF-8 우선 표시)를 그대로 돌려줍니다.
- force 경로가 없습니다: `plan_remove`는 `--force`를 만들지 않고, `--force`/`-f`가 든 plan을 수동으로 조립해도
  `remove_worktree`가 실행 전에 거부합니다. 사용자는 파일을 직접 정리하거나 외부 터미널에서 Git을 사용합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 221 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **untracked fixture**: linked worktree에 untracked 파일 → 확인된 remove를 Git이 거부, 비영 종료·비어 있지 않은 Git 메시지가 오류 표시에 그대로 포함, 파일·worktree 유지; tracked 변경도 거부; force plan 수동 조립 거부(worktree 유지); 정리 후 제거 성공(`untracked_or_modified_worktree_remove_is_refused_as_is`) | 통과 |

## 차이(작업 지시와 다른 점)

- "기본 제공하지 않음"을 force API 미제공으로 구현했습니다(선택적 force도 두지 않음).

## 남은 위험 / 한계

- Git 거부 메시지는 locale에 따라 달라질 수 있으며 해석하지 않고 표시만 합니다.
