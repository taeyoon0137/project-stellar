# 작업 보고서: IV.4.6 최초 커밋 생성

- WBS: IV.4.6
- 작업명: `New Stellar project initialized` 메시지로 최초 커밋
- 상태: 완료 (`[x]`)
- 선행: IV.4.5(add + identity)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 git, 로컬 검증; `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/repo.rs` (`INITIAL_COMMIT_MESSAGE`, `git_commit`, `create_initial_commit`, 테스트)
- `docs/task-reports/IV.4.6.md`(신규), 계획서(IV.4.6 `[ ]`→`[x]`)

## 구현 요약

- `git_commit(runner, dir, message)`: `git commit -m <message>`(argv)로 커밋.
- `create_initial_commit`: 고정 메시지 `INITIAL_COMMIT_MESSAGE`("New Stellar project
  initialized")로 최초 커밋.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-git-porcelain` (전체) | 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | 0 |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| **`git log -1 --format=%B` == "New Stellar project initialized"** | 통과 |
| **working tree clean**(`git status --porcelain` 빈 출력) | 통과 |

## 차이(작업 지시와 다른 점)

- `--format=%B`는 본문 뒤에 개행을 덧붙이므로 `trim_end` 후 메시지와 정확히 비교합니다.

## 남은 위험 / 한계

- 단계별 rollback 정책(Git init 이전/이후 실패 처리)은 IV.4.7에서 종합합니다.
- 실제 git 커밋 테스트는 git 설치 + hermetic config(로컬 identity)로 수행하며, 미설치 환경에서는
  건너뜁니다.
