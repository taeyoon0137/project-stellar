# 작업 보고서: XIV.8.4 commit 뒤 HEAD·index·status 재조회, 자동 push 없음

- WBS: XIV.8.4
- 작업명: commit 뒤 HEAD·index·status 재조회, 자동 push 없음
- 상태: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 선행 XIV.8.1(`[~]`)가 `[x]`가 아님(계획 1.2 규칙 4). 보고 당시 기록: 완료 (`[x]` 후보)
- 선행: XIV.8.1~XIV.8.3
- 결정: ADR-0080

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, XIV.14.1 fixture: 로컬 bare 저장소, 네트워크 없음).

## 변경 파일 목록

- `crates/ops/src/git.rs`(`CommitOutcome`)
- `crates/runtime/src/staging/engine_tests.rs`

## 구현 요약

- commit 성공 뒤 HEAD commit, HEAD tree, index, `RepositoryStatus`, 계산 revision을 다시 읽어 `CommitOutcome`으로 돌려줍니다.
- `staged_remaining`은 HEAD tree와 index가 다른 경로 수입니다. `tree_matches_validated`는 새 HEAD tree가 재검증한 index와 같은지입니다(hook이 index를 바꾸면 거짓, 되돌리지 않음).
- 엔진·CLI에 push 경로가 없습니다(코드 검토: `ops::git`·CLI `git.rs`에 push·remote 쓰기 호출 없음).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops -p stellar-runtime -p stellar-git-porcelain -p stellar-cli -p stellar-mcp-core` | 모두 통과(실패 0) |
| `cargo clippy -p stellar-ops -p stellar-runtime -p stellar-cli -p xtask --all-targets --features stellar-ops/test-support -- -D warnings` | exit 0 |
| `cargo xtask validate-crate-graph` | exit 0 |
| `cargo check --workspace --all-targets` | exit 0 |

| 검사 | 결과 |
|---|---|
| 속성 하나 stage 후 commit: HEAD 바뀜, `staged_remaining` 0, `git diff --cached` 비어 있음, bare 저장소에 `main` ref 없음(push 없음) (`partial_same_chunk_property_stage_unstage_commit`) | 통과 |

## 테스트

- 위 검사 표의 테스트는 `cargo test -p stellar-runtime --lib staging`(19개)과 `cargo test -p stellar-cli --test git`(4개)에 포함되며 모두 통과했습니다.

## 성능

- 성능 예산 대상 경로를 바꾸지 않았고 수치를 측정하지 않았습니다. status·commit 재검증은 HEAD·index를 임시 디렉터리에 materialize합니다(ADR-0080 Performance).

## 차이(작업 지시와 다른 점)

- 셸 FFI와 Swift UI는 사용자 방향(2026-11-23)에 따라 다루지 않았습니다. 엔진은 Rust 테스트와 `stellar git` CLI로 검증했습니다.

## 남은 위험 / 한계

- 없음.
