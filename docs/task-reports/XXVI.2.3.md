# 작업 보고서: XXVI.2.3 reset/restore 파괴 수준별 확인

- WBS: XXVI.2.3
- 작업명: reset/restore는 destructive level별 확인을 요구하고 command preview를 표시
- 상태: 완료 (`[x]`)
- 선행: Phase XIV(phase 수준, 완화 적용)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 검증; 실제 Git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/advanced/reset.rs` (신규: `plan_reset`, `plan_restore`, `ResetMode`, `RestoreScope`, `is_valid_target`, 테스트)
- `crates/git-porcelain/src/advanced/mod.rs` (`reset` 모듈)
- `docs/task-reports/XXVI.2.3.md`(신규), 계획서(XXVI.2.3 `[ ]`→`[x]`)

## 구현 요약

- reset·restore는 모두 `CommandPlan`(미리보기 `git reset -q --hard HEAD~1 --` 등 + 파괴 수준)으로 만들고,
  `execute_plan`이 plan digest에 묶인 `Confirmation` 없이는 프로세스를 만들지 않습니다.
- 수준: `reset --soft`·`--mixed`·`restore --staged` = `DiscardsRefs`(branch 이동·staging 폐기, reflog·dangling으로만
  복구), `reset --hard`·`restore --worktree`(또는 둘 다) = `DiscardsWork`(커밋되지 않은 작업 폐기).
- 대상은 `HEAD`, `HEAD^`, `HEAD~<n>`, 안전한 ref 이름, object ID만 받습니다(option 주입·reflog 문법 거부).
  restore 경로는 `:(literal)` pathspec으로 넘겨 glob 문자를 해석하지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 213 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **hard reset confirm 없이는 실행 0**: 더러운 작업 트리에서 `reset --hard HEAD~1` plan을 확인 없이·다른 plan 확인으로 실행 → `ConfirmationRequired(DiscardsWork)`, spawn 증가 0, HEAD·파일 그대로; 맞는 확인 후 HEAD 이동·파일 복원(`hard_reset_without_confirmation_spawns_nothing`) | 통과 |
| soft(HEAD만 이동)·restore --staged(unstage)·restore --worktree(변경 폐기)·mixed(staging 폐기, 작업 트리 유지)·`:(literal)` 경로(`b*.txt`만 복원, `bz.txt` 유지)(`soft_mixed_and_restore_levels_execute_after_confirmation`) | 통과 |
| 수준·미리보기 문자열, 잘못된 대상 8종·경로 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- `reset --soft`·`restore --staged`도 확인 대상(`DiscardsRefs`)으로 두었습니다. branch 이동·staged 내용 폐기는 작업
  트리를 지우지 않지만 되돌리려면 reflog가 필요하기 때문입니다.
- UI 없이 core API만 제공합니다(lane 지시).

## 남은 위험 / 한계

- `reset --keep`·`--merge`, `restore --patch`는 제공하지 않습니다.
- 파괴 수준은 명령 종류로 정합니다. 실제로 잃는 내용이 없는 경우(깨끗한 작업 트리의 hard reset)도 확인을 요구합니다.
