# 작업 보고서: XIV.8.5 trusted에서만 commit, hook 실패 우회 없음

- WBS: XIV.8.5
- 작업명: trusted에서만 commit, hook 실패 우회 없음
- 상태: 완료 (`[x]`) — 2026-11-23 근거 커밋 기록(ledger-refresh): 이 행의 남은 범위가 없고, `[~]`의 이유였던 선행 XIV.8.1 사슬이 풀렸습니다. XIV.8.1의 판정 이중 경로는 integration-followups lane(`2ac9bd6f`)이 `stellar_ops::validate::validate_records` 하나로 합쳤습니다. 작업 커밋 제목이 `Task(<ID>)` 형식이 아니어서 원장이 근거 커밋을 찾지 못했습니다. 선행 XIV.8.4도 같은 판정으로 `[x]`입니다. 아래 "근거 커밋" 절에 실제 작업 커밋을 적었습니다. 이전 기록: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 선행 XIV.8.4(`[~]`)가 `[x]`가 아님(계획 1.2 규칙 4). 보고 당시 기록: 완료 (`[x]` 후보)
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

## 남은 위험 / 한계

- 없음.

## 근거 커밋 (2026-11-23 기록)

이 행의 작업은 아래 커밋에 있습니다. 이 기록 커밋(`Task(XIV.8.5)`)은 원장이 근거 커밋을 찾도록 남긴 것이고 코드 변경은 없습니다.

- `b14ea25f` Feat: 의미 단위 Git 엔진을 ops에 두고 commit 직전 재검증을 더함
- `7753af02` Test: Git 엔진 통합 시나리오와 작업 보고서 추가
