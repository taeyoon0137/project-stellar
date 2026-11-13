# 작업 보고서: XXVI.1.1 branch list/switch/create/delete core API

- WBS: XXVI.1.1
- 작업명: branch list/switch/create/delete UI API를 시스템 Git 명령에 연결
- 상태: 완료 (`[x]`)
- 선행: Phase XIV(phase 수준, 완화 적용)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 검증; 실제 Git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/advanced/mod.rs` (신규: 고급 Git core API 공통 — `DestructiveLevel`, `CommandPlan`(argv 미리보기·digest), `Confirmation`, `PlanError`, `execute_plan`, 테스트)
- `crates/git-porcelain/src/advanced/branch.rs` (신규: `list_branches`, `parse_branch_list`, `switch_branch`, `create_branch`, `plan_delete_branch`, `delete_branch`, `BranchInfo`, `TrackState`, 테스트)
- `crates/git-porcelain/src/refname.rs` (신규: 안전한 ref 이름 부분집합 검증)
- `crates/git-porcelain/src/remote/push.rs` (refspec 검증을 `refname`으로 위임, 규칙 동일 + `-`·`@` 시작 추가 거부)
- `crates/git-porcelain/src/lib.rs` (`refname`, `advanced` 모듈)
- `docs/task-reports/XXVI.1.1.md`(신규), 계획서(XXVI.1.1 `[ ]`→`[x]`)

## 구현 요약

- **UI 없이 core API만** 제공합니다(로드맵 기능 노출 정책은 XIV.13.7 UI 단계).
- 목록: `git for-each-ref --format=%(refname)%00%(objectname)%00%(HEAD)%00%(upstream:short)%00%(upstream:track,nobracket) refs/heads`
  → `BranchInfo{name, commit, is_head, upstream, track}`(`NoUpstream`/`InSync`/`Diverged{ahead,behind}`/`Gone`).
- 전환: `git switch --no-guess <branch>`(remote branch 자동 생성 없음, 작업 트리 충돌은 Git 거부 결과 그대로,
  변경 폐기 옵션 미제공). 생성: `git branch --no-track -- <name> [<start>]`.
- 삭제: 기본 `-d`(Safe, 병합 안 된 branch는 Git이 거부). 강제 `-D`는 `DiscardsRefs` plan으로만 만들고
  사용자 확인(`Confirmation::confirm(&plan)`, plan argv·level digest에 묶임) 없이는 프로세스를 만들지 않습니다.
- 파괴적 명령 확인을 데이터로 표현하는 `CommandPlan`/`Confirmation`/`execute_plan`은 이후 XXVI 작업이 공통으로 씁니다.
- 이름·시작 지점은 Git 실행 전에 검증합니다(`-`로 시작하는 option 주입 차단).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 199 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **local repo fixture**: 목록(이름 순·HEAD·upstream·ahead 1), 생성(시작 지점 지정), 전환, 작업 트리 충돌 전환은 Git 거부·파일 보존, 병합 branch `-d` 삭제, 미병합 `-d`는 Git 거부, `-D`는 확인 없이 spawn 0·branch 유지, 확인 후 삭제(`local_repo_branch_fixture`) | 통과 |
| 목록 해석(5종 track 상태)·형식 오류 5종 | 통과 |
| 잘못된 이름·option 형태 시작 지점 → spawn 0 | 통과 |
| plan 확인: 확인 없음·다른 plan 확인 → spawn 0, 맞는 확인만 실행, 미리보기 quoting(`advanced::tests`) | 통과 |
| ref 이름 규칙 표(`refname::tests`), push refspec 기존 테스트 유지 | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 "Phase XIV"는 phase 수준 선행이라 완화 규칙을 적용했습니다(필요한 Git 실행·gate 코드는 이미 있음).
- "UI API"는 core API로 구현했습니다(lane 지시: XXVI는 UI 없이 core API, 파괴 수준 확인은 데이터).

## 남은 위험 / 한계

- remote branch 목록·삭제(`git push --delete`)는 다루지 않습니다.
- 확인 digest는 FNV-1a 64로 plan 일치 확인용이며 보안 서명이 아닙니다(같은 프로세스 안의 UI 확인 흐름 대상).
