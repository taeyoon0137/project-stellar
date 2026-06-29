# 작업 보고서: IV.4.4 git init -b main

- WBS: IV.4.4
- 작업명: `git init -b main` 실행, `-b` 미지원 시 대체 추측 없이 지원 오류
- 상태: 완료 (`[x]`)
- 선행: IV.3.4(GitCommandResult), IV.4.3(scaffold)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 git, 로컬 검증).

## 변경 파일 목록

- `crates/git-porcelain/src/repo.rs` (신규: `git_init_main`, `RepoError`, 테스트)
- `crates/git-porcelain/src/lib.rs` (`pub mod repo;`)
- `docs/task-reports/IV.4.4.md`(신규), 계획서(IV.4.4 `[ ]`→`[x]`)

## 구현 요약

- `git_init_main(runner, version, dir)`: `GitRunner`로 `git init -b main`(argv, shell 없음)을
  실행하고 `GitCommandResult::require_success`로 성공을 강제합니다.
- `-b`는 Git 2.28+에서만 지원하므로, 버전이 2.28 미만이면 **Git을 실행하지도 않고**
  `InitBranchFlagUnsupported`를 반환합니다(**대체 명령 추측 없음**).
- git 연산이므로 git-porcelain의 `repo` 모듈에 두었습니다(GitRunner/Version/Result 사용).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-git-porcelain` (전체) | 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | 0 |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| **실제 `git init -b main` → `.git` 생성 + HEAD가 정확히 `main`** | 통과 |
| 구버전(2.27, 가짜) → `InitBranchFlagUnsupported`, **git 미실행**(.git 미생성) | 통과 |

## 차이(작업 지시와 다른 점)

- git init은 git 타입에 의존하므로 `project`가 아니라 `git-porcelain`의 `repo` 모듈에
  구현했습니다(프로젝트 생성 오케스트레이션이 이를 호출).
- `-b` 지원 판정은 버전 비교(2.28+)로 선제 처리해, 미지원 시 git을 실행하지 않고 대체 명령을
  추측하지 않습니다.

## 남은 위험 / 한계

- `git add`/커밋·identity 검사는 IV.4.5~IV.4.6, 단계별 rollback은 IV.4.7에서 추가합니다.
- 실제 git 테스트는 git 2.28+ 환경을 가정하며, 미설치/구버전 환경에서는 해당 분기를 건너뜁니다.
