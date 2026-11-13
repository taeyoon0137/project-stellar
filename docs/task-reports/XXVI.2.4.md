# 작업 보고서: XXVI.2.4 reflog 조회와 선택 ref 복구

- WBS: XXVI.2.4
- 작업명: reflog 조회와 선택 ref 복구 command 제공
- 상태: 완료 (`[x]`)
- 선행: XIV.1.3(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 검증; 실제 Git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/advanced/reflog.rs` (신규: `read_reflog`, `parse_reflog`, `plan_recover_branch`, `recover_branch`, `ReflogEntry`, 테스트)
- `crates/git-porcelain/src/advanced/mod.rs` (`reflog` 모듈)
- `docs/task-reports/XXVI.2.4.md`(신규), 계획서(XXVI.2.4 `[ ]`→`[x]`)

## 구현 요약

- 조회: `git log -g --format=%H%x00%gD%x00%gs <ref> --` → `ReflogEntry{commit, selector, subject}`(최신 먼저).
  대상은 `HEAD` 또는 안전한 branch 이름만 받습니다(option 주입 거부).
- 복구: 선택한 항목의 commit에 새 branch를 만드는 `CommandPlan`(`git branch --no-track -- <name> <oid>`, Safe).
  현재 branch·작업 트리를 바꾸지 않으며, 같은 이름이 있으면 Git이 거부합니다(덮어쓰지 않음). 현재 branch를
  되돌리는 복구는 XXVI.2.3 reset plan(확인 필요)을 씁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 215 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **reflog fixture**: commit 후 `reset --hard HEAD~1`로 잃은 commit을 `HEAD` reflog(`HEAD@{1}`)와 `main` reflog에서 찾음 → `rescued` branch로 복구, branch가 잃은 commit을 가리킴, HEAD·작업 트리 불변, 같은 이름 재복구는 Git 거부, 없는 reflog 대상 오류·option 형태 대상 거부(`reflog_fixture_recovers_lost_commit`) | 통과 |
| reflog 줄 해석·형식 오류, 잘못된 branch 이름 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- "선택 ref 복구"를 새 branch 생성(비파괴)으로 구현했습니다. 현재 branch를 옮기는 복구는 파괴적이므로 reset
  plan(확인 필요)에 맡깁니다.

## 남은 위험 / 한계

- reflog 만료(`gc.reflogExpire`)·`git gc` 이후의 항목은 Git에 없으므로 복구할 수 없습니다.
- stash·remote-tracking ref의 reflog 조회는 대상 검증 규칙상 제공하지 않습니다(HEAD·branch만).
