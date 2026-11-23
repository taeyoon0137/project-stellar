# 작업 보고서: XIV.14.5 linked worktree open·status·stage·commit

- WBS: XIV.14.5
- 작업명: linked worktree open·status·stage·commit
- 상태: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 계획 1.1의 "올바른 커밋"이 없음: 구현 커밋이 `Feat:`·`Fix:`·`Test:` 제목이라 `Task(<ID>)` 제목 커밋이 없고, 원장(`cargo xtask generate-ledger`)이 근거 커밋과 검증 시각을 채우지 못합니다. 구현·검증과 선행 판정은 `[x]` 조건을 충족합니다. 보고 당시 기록: 완료 (`[x]` 후보)
- 선행: XIV.1.2, XIV.6.6
- 결정: ADR-0080

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, XIV.14.1 fixture: 로컬 bare 저장소, 네트워크 없음).

## 변경 파일 목록

- `crates/runtime/src/staging/engine_tests.rs`

## 구현 요약

- `git worktree add`로 만든 linked worktree에서 세션을 열고 status·stage·commit을 실행합니다.
- 세션은 worktree root에 고정하고 그 worktree의 Git dir·index를 씁니다. common Git dir은 공유합니다. 신뢰 등록은 실제 경로 단위라 linked worktree는 별도 project identity입니다(ADR-0080 결정 9).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops -p stellar-runtime -p stellar-git-porcelain -p stellar-cli -p stellar-mcp-core` | 모두 통과(실패 0) |
| `cargo clippy -p stellar-ops -p stellar-runtime -p stellar-cli -p xtask --all-targets --features stellar-ops/test-support -- -D warnings` | exit 0 |
| `cargo xtask validate-crate-graph` | exit 0 |
| `cargo check --workspace --all-targets` | exit 0 |

| 검사 | 결과 |
|---|---|
| `kind=Linked`, common dir 같음·git dir 다름, detached HEAD 표시, 주 worktree `.git/index` bytes와 HEAD 불변, linked HEAD = 새 commit (`linked_worktree_uses_its_own_index_and_leaves_main_index_alone`) | 통과 |

## 테스트

- 위 검사 표의 테스트는 `cargo test -p stellar-runtime --lib staging`(19개)과 `cargo test -p stellar-cli --test git`(4개)에 포함되며 모두 통과했습니다.

## 성능

- 성능 예산 대상 경로를 바꾸지 않았고 수치를 측정하지 않았습니다. status·commit 재검증은 HEAD·index를 임시 디렉터리에 materialize합니다(ADR-0080 Performance).

## 차이(작업 지시와 다른 점)

- 셸 FFI와 Swift UI는 사용자 방향(2026-11-23)에 따라 다루지 않았습니다. 엔진은 Rust 테스트와 `stellar git` CLI로 검증했습니다.

## 남은 위험 / 한계

- 없음.
