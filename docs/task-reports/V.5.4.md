# 작업 보고서: V.5.4 pointer 전환 commit

- WBS: V.5.4
- 작업명: 마지막 단계 current pointer 전환, 성공 후에만 in-memory saved generation 갱신
- 상태: 완료 (`[x]`)
- 선행: V.5.3, V.4.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/save/commit.rs`(신규) — `commit_save`,
  `commit_save_observed`, `SavedGenerationState`, `CommitError`.
- `crates/storage/src/save/mod.rs` — `commit` 모듈 등록.
- `docs/task-reports/V.5.4.md`(신규), 계획서(V.5.4 `[ ]`→`[x]`).

## 구현 요약

- **`commit_save`**: prepare(V.5.3) 산출물의 manifest hash로 current pointer를
  atomic replace(V.4.3)합니다. 저장 파이프라인에서 디스크의 current를 바꾸는
  유일한 단계입니다.
- **`SavedGenerationState`**: 마지막으로 디스크 전환까지 성공한 (generation
  number, manifest hash)의 in-memory 추적자. **pointer 전환이 성공한 뒤에만**
  갱신되며, 전환 실패 경로에서는 어떤 단계 실패든 값이 바뀌지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 138 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| pointer 전환 각 단계 실패 시 memory generation 미변경 + 디스크 pointer base 유지(`pointer_switch_failure_leaves_memory_unchanged`) | 통과 |
| 성공 commit 시 pointer·memory 동시 갱신, 첫 저장 commit | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 취소 token(V.5.5)과 전체 fault matrix(V.5.6)는 후속 작업입니다.
