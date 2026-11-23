# 작업 보고서: XXIII.2.4 검증한 C만 쓰기 가능

- WBS: XXIII.2.4
- 작업명: C full validation과 unresolved conflict count가 0일 때만 write 가능하게 함
- 상태: 완료 (`[x]`)
- 선행: XXIII.2.3, VI.10.1
- 결정: [ADR-0081](../adr/ADR-0081-semantic-merge-resolution-and-git-driver.md) 결정 4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-merge/src/candidate.rs` — `ValidatedCandidate`(비공개 필드, `validate`만 생성).
- `crates/semantic-merge/tests/candidate_resolution.rs`.

## 구현 요약

- `CandidateSnapshot::validate`의 성공 값을 `ValidatedCandidate`로 바꿨습니다. 필드가 비공개라 다른 crate는 검증 없이 만들 수
  없습니다. 이후의 쓰기 API(merge driver 파일 쓰기, WorkingTree 쓰기 XIV.12.4)는 `ValidatedCandidate`만 받으므로, 미해결
  conflict나 새 snapshot 위반이 있는 C는 타입 단계에서 쓰기 경로에 들어가지 못합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-merge` | 전체 통과(candidate_resolution 8) |
| `cargo clippy -p stellar-semantic-merge --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 미해결·invalid candidate는 `CandidateBlocked`, 통과한 C의 `ValidatedCandidate::snapshot`이 해결 결과를 담음 | 통과 |

## 차이(작업 지시와 다른 점)

- 쓰기 대상(현재·기존·새 worktree)은 XXIII.3 범위입니다. 이 행은 쓰기 전제 조건의 타입 gate입니다.

## 남은 위험 / 한계

- `ValidatedCandidate`는 C를 빌린 값이라 검증 뒤 C가 바뀌면 다시 검증해야 합니다(borrow 규칙이 강제).
