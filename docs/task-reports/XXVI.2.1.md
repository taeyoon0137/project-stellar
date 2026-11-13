# 작업 보고서: XXVI.2.1 stash core API

- WBS: XXVI.2.1
- 작업명: stash list/create/apply/pop/drop UI를 시스템 Git으로 연결(core API)
- 상태: 완료 (`[x]`)
- 선행: Phase XIV(phase 수준, 완화 적용)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 검증; 실제 Git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/advanced/stash.rs` (신규: `list_stashes`, `parse_stash_list`, `create_stash`, `apply_stash`, `pop_stash`, `plan_drop_stash`, `drop_stash`, `StashEntry`, `StashOptions`, `StashDropPlan`, `StashError`, 테스트)
- `crates/git-porcelain/src/advanced/mod.rs` (`stash` 모듈)
- `docs/task-reports/XXVI.2.1.md`(신규), 계획서(XXVI.2.1 `[ ]`→`[x]`)

## 구현 요약

- 목록: `git stash list --format=%gd%x00%H%x00%gs` → `StashEntry{index, selector, commit, subject}`(selector 순서 검증).
- 만들기: `git stash push [--include-untracked] [-m <message>]`(message는 argv, NUL 거부). 저장할 변경이 없으면 `None`.
- 적용: `apply`(stash 유지), `pop`(적용 성공 시 Git이 제거). 충돌·거부는 Git 결과 그대로.
- 삭제: `drop`은 `DiscardsRefs` plan으로 사용자 확인이 필요합니다(없으면 spawn 0). 실행 직전에 목록을 다시 읽어
  확인한 selector가 여전히 같은 stash(commit·subject)를 가리킬 때만 지웁니다(`StashChanged`).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 209 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **stash fixture**: 변경 없음 → `None`; untracked 포함 + 한글·shell 문자 message stash → 작업 트리 복원·subject 보존; apply → 내용 복원·목록 유지; 중복 apply는 Git 거부; pop → 복원·목록 0; 두 stash 중 오래된 것 drop은 확인 없이 spawn 0, 확인 후 목록 변화 시 `StashChanged`(3개 유지), 재계획·확인 후 삭제(`stash_lifecycle_fixture`) | 통과 |
| 목록 해석·형식 오류 3종 | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 "Phase XIV"는 phase 수준 선행이라 완화 규칙을 적용했습니다(필요한 실행·gate 코드는 이미 있음).
- UI 없이 core API만 제공하고, 파괴 수준 확인은 데이터(`StashDropPlan` + `Confirmation`)로 표현했습니다.

## 남은 위험 / 한계

- `stash clear`(전체 삭제)와 `stash branch`는 제공하지 않습니다.
- drop 직전 재확인과 실제 drop 사이의 짧은 경쟁(외부 터미널 동시 조작)은 막을 수 없습니다.
