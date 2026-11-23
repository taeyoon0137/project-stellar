# 작업 보고서: XIV.12.3 해결 후 candidate full validation

- WBS: XIV.12.3
- 작업명: 모든 semantic conflict 해결 후 candidate snapshot full validation 실행
- 상태: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 계획 1.1의 "올바른 커밋"이 없음: 구현 커밋이 `Feat:`·`Fix:`·`Test:` 제목이라 `Task(<ID>)` 제목 커밋이 없고, 원장(`cargo xtask generate-ledger`)이 근거 커밋과 검증 시각을 채우지 못합니다. 구현·검증은 완료입니다; 선행 XIV.12.2(`[~]`)가 `[x]`가 아님(계획 1.2 규칙 4). 보고 당시 기록: 완료 (`[x]`)
- 선행: XIV.12.2, VI.10.1
- 결정: [ADR-0081](../adr/ADR-0081-semantic-merge-resolution-and-git-driver.md) 결정 4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-merge/src/candidate.rs` — `CandidateBlocked`, `CandidateSnapshot::validate`.
- `crates/semantic-merge/tests/candidate_resolution.rs` — 쓰기 조건 검사.

## 구현 요약

- `validate`는 미해결 conflict 단위 목록과 `introduced_violations(C, [base, A, B])`를 함께 봅니다. 둘 다 비어야 성공이고, 아니면
  두 목록을 모두 담은 `CandidateBlocked`를 돌려줍니다(엔진 결과 검증 XIV.10.8과 같은 기준).
- 문서 validator(VI.10.1)는 쓰기 경로의 레코드 역투영(XIV.12.4)에서 다시 거칩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-merge` | 전체 통과(candidate_resolution 8) |
| `cargo clippy -p stellar-semantic-merge --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 미해결 1 → 차단(목록 일치), 해결 후 통과, 지운 Property의 override를 B에서 다시 고르면 위반 1로 차단, base로 해결을 바꾸면 통과(`write_requires_zero_unresolved_and_no_new_violation`) | 통과 |

## 차이(작업 지시와 다른 점)

- XIV.12.2(선행)보다 먼저 커밋했습니다. XIV.12.2의 메모리 적용은 XXIII.2.3에 들어 있고, 남은 검증(preview 중 disk·index hash
  동일)은 Git 작업 트리 세션이 필요해 XIV.11.3 뒤에 커밋합니다.

## 남은 위험 / 한계

- 기준이 "세 입력 어디에도 없던 위반"이라 세 입력에 공통으로 있던 위반(예: snapshot에 담지 않은 asset 참조)은 통과합니다.
