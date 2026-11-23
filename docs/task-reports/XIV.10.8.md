# 작업 보고서: XIV.10.8 병합 결과 full validator 검증

- WBS: XIV.10.8
- 작업명: conflict 없는 merged snapshot을 full validator로 검증하고 invalid result를 자동 merge 성공으로 반환하지 않음
- 상태: 완료 (`[x]`)
- 선행: XIV.10.2~XIV.10.7, VI.10.1
- 결정: [ADR-0081](../adr/ADR-0081-semantic-merge-resolution-and-git-driver.md) 결정 2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-merge/src/model.rs` — `ConflictKind::InvalidResult`(code 10).
- `crates/semantic-merge/src/merge.rs` — `Merger::settle`(고아·cycle·key 재배정·정의 목록을 한 함수로), 위반 단위
  `Unit`·`violation_units`, `revert_violations`, 검증 반복.
- `crates/semantic-merge/tests/result_validation.rs`(신규).

## 구현 요약

- candidate를 만든 뒤 `introduced_violations(merged, [base, ours, theirs])`로 세 입력 어디에도 없던 위반만 봅니다(snapshot에
  담지 않은 asset 참조 같은 입력 공통 위반은 제외).
- 위반마다 관련 단위(객체 추가·삭제, 위치, 속성 경로, interface 목록 member 제거)의 받아들인 변경을 되돌리고 그 단위를
  `invalid-result` conflict로 기록한 뒤 다시 적용합니다. 되돌릴 때마다 받아들인 변경이 줄어 유한 번에 끝납니다.
- 더 되돌릴 변경이 없는데 위반이 남으면 위반의 대표 대상으로 conflict를 남겨 clean이 되지 않게 합니다.
- 정의 목록 conflict는 settle마다 새로 계산해 반복 실행에도 중복되지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-merge` | lib 28, dedicated 3, result_validation 4 passed, 0 failed |
| `cargo test -p stellar-integration-tests --test git_semantic_merge` | 4 passed |
| `cargo clippy -p stellar-semantic-merge --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 삭제한 node 대 새 clipping 참조: 양방향 모두 not clean, 두 단위 base 유지, 새 위반 0 | 통과 |
| 삭제한 컴포넌트 대 새 인스턴스: 정의·root 복원, 새 인스턴스 보류, 새 위반 0 | 통과 |
| View·root 삭제 대 View 이름 변경(SM-1 남은 한계): root Frame 복원, 새 위반 0 | 통과 |
| clean 병합은 새 위반 0 | 통과 |

## 차이(작업 지시와 다른 점)

- 검증 기준은 semantic snapshot validator(XIV.5.3)입니다. 문서 validator(VI.10.1)는 레코드 역투영이 필요한 쓰기 경로
  (XIV.12.4, merge driver)에서 다시 검사합니다.

## 남은 위험 / 한계

- 관련 단위를 모두 되돌리는 보수적 규칙이라 위반과 덜 직접적인 변경(예: 참조 대상 삭제와 참조 추가 둘 다)도 함께 보류됩니다.
