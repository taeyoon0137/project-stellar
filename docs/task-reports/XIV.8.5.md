# 작업 보고서: XIV.8.5 trusted에서만 commit, hook 실패 우회 없음

- WBS: XIV.8.5
- 작업명: trusted에서만 commit, hook 실패 우회 없음
- 상태: 완료 (`[x]`) — 2026-11-23 cli-mcp-git-cleanup 재판정: 선행 XIV.1.1·XIV.8.4가 `[x]`입니다(XIV.8.4는 같은 날 XIV.8.1 사슬이 풀려 `[x]`). 이전 판정: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 선행 XIV.8.4(`[~]`)가 `[x]`가 아님(계획 1.2 규칙 4). 보고 당시 기록: 완료 (`[x]` 후보)
- 선행: XIV.1.1, XIV.8.4
- 결정: ADR-0080

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, XIV.14.1 fixture: 로컬 bare 저장소, 네트워크 없음).

## 변경 파일 목록

- `crates/ops/src/git.rs`
- `crates/runtime/src/staging/engine_tests.rs`

## 구현 요약

- 세션은 `GatedGitRunner`로만 Git을 실행합니다. 신뢰하지 않으면 세션 열기부터 프로세스 없이 `NotTrusted`입니다.
- commit은 시스템 Git `git commit --file=- --cleanup=verbatim`(XIV.8.2)을 씁니다. hook 실패는 `GitError::Commit(CommitError::Git)`로 종료 코드·stderr를 그대로 돌려줍니다. `--no-verify`나 `commit-tree` 우회 경로가 없습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops -p stellar-runtime -p stellar-git-porcelain -p stellar-cli -p stellar-mcp-core` | 모두 통과(실패 0) |
| `cargo clippy -p stellar-ops -p stellar-runtime -p stellar-cli -p xtask --all-targets --features stellar-ops/test-support -- -D warnings` | exit 0 |
| `cargo xtask validate-crate-graph` | exit 0 |
| `cargo check --workspace --all-targets` | exit 0 |

| 검사 | 결과 |
|---|---|
| 실패하는 pre-commit hook: `CommitError::Git`(비영 종료, stderr에 hook 출력), HEAD 불변, staged 변경 유지 (`failing_hook_blocks_commit_and_result_is_returned_as_is`) | 통과 |

## 테스트

- 위 검사 표의 테스트는 `cargo test -p stellar-runtime --lib staging`(19개)과 `cargo test -p stellar-cli --test git`(4개)에 포함되며 모두 통과했습니다.

## 성능

- 성능 예산 대상 경로를 바꾸지 않았고 수치를 측정하지 않았습니다. status·commit 재검증은 HEAD·index를 임시 디렉터리에 materialize합니다(ADR-0080 Performance).

## 차이(작업 지시와 다른 점)

- 셸 FFI와 Swift UI는 사용자 방향(2026-11-23)에 따라 다루지 않았습니다. 엔진은 Rust 테스트와 `stellar git` CLI로 검증했습니다.

## 2026-11-23 재판정 (cli-mcp-git-cleanup)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib staging` | 19개 통과 |
| `cargo test -p stellar-cli --test git` | 4개 통과 |
| `cargo test -p stellar-ops` | 83개 통과 |

- commit 인자는 `commit --file=- --cleanup=verbatim`(`stellar_git_porcelain::commit::COMMIT_FROM_STDIN_ARGS`)이고 `--no-verify`·`commit-tree` 호출이 코드에 없음을 다시 확인했습니다.
- `failing_hook_blocks_commit_and_result_is_returned_as_is`를 다시 실행해 통과했습니다.

## 남은 위험 / 한계

- 없음.
