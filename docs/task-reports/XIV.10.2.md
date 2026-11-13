# 작업 보고서: XIV.10.2 겹치지 않는 변경 자동 병합

- WBS: XIV.10.2
- 작업명: 서로 다른 StableId 변경과 같은 object의 서로 다른 PropertyPath 변경 자동 병합
- 상태: 완료 (`[x]`)
- 선행: XIV.10.1 `[x]`, XIV.3.9 `[x]`(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-merge/src/merge.rs`(신규) — `merge` 엔진.
- `crates/semantic-merge/src/lib.rs` — module 선언.
- `docs/task-reports/XIV.10.2.md`(신규), 계획서(XIV.10.2 `[ ]`→`[x]`).

## 구현 요약

- **대상 단위 비교**: base → ours, base → theirs semantic diff를 객체별로 묶고 대상(객체 구조:
  추가·삭제·교체 / 위치: 이동·재정렬 / 속성: `StableId + PropertyPath`)으로 나눕니다.
- **자동 병합**: 한쪽만 바꾼 대상은 받아들입니다 — 서로 다른 StableId, 같은 객체의 서로 다른
  PropertyPath, 같은 객체의 위치 변경 + 속성 변경은 conflict가 없습니다. 받아들인 변경을
  semantic-diff `apply`로 base에 적용해 candidate를 만들고, `changes = diff(base, candidate)`.
- **겹침은 조용히 병합하지 않음**: 양쪽이 같은 대상을 바꾸면 conflict(속성 → property-value,
  위치 → conflicting-reorder, 구조 → 삭제가 있으면 delete-vs-modify, 아니면
  inconsistent-definition)이며 candidate는 base 상태를 유지합니다. 세부 분류·동일 변경 처리는
  XIV.10.3~XIV.10.6에서 다듬습니다.
- **구조 정합**: 한쪽이 삭제한 parent 아래에 다른 쪽이 객체를 둬 candidate에 고아가 생기면 그
  parent 삭제를 되돌리고 delete-vs-modify conflict로 기록합니다(입력에 이미 있던 고아는 제외).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-merge` | 11 passed, 0 failed |
| `cargo fmt -p stellar-semantic-merge -- --check` | 통과 |
| `cargo clippy -p stellar-semantic-merge --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 서로 다른 객체 이름 변경 → conflict 0, 양쪽 반영(`different_objects_merge_without_conflict`) | 통과 |
| 같은 객체 VISIBLE/DISPLAY_NAME, 위치/이름 → conflict 0(`same_object_different_paths_merge_without_conflict`) | 통과 |
| 독립 추가·삭제 → conflict 0, 자식 순서 정확(`independent_additions_and_deletions_merge`) | 통과 |
| 한쪽만 변경하면 결과 = 그쪽, 변경 없음 = base(`one_sided_merge_returns_that_side`) | 통과 |
| 100 seed 무작위 tree에서 node를 양쪽에 나눠 편집 → conflict 0, 기대 결과와 동일(`disjoint_random_edits_merge_without_conflict`) | 통과 |
| 같은 속성 다른 값은 conflict이고 candidate는 base 유지(`overlapping_edits_are_conflicts_not_silent`) | 통과 |
| 삭제된 parent 아래 추가 → delete-vs-modify, candidate 고아 0(`child_added_under_deleted_parent_is_conflict`) | 통과 |

## 차이(작업 지시와 다른 점)

- 고아 parent 검사를 이 단계에 넣었습니다. full validator 연결(XIV.10.8)은 선행 XIV.10.7이
  차단되어 진행하지 못하므로, 지시 범위의 자동 병합이 parent 없는 객체를 조용히 만들지 않도록
  최소 구조 검사를 둡니다.

## 남은 위험 / 한계

- 참조 무결성(삭제된 node를 가리키는 modifier operand 등)과 parent cycle은 아직 검사하지
  않습니다. cycle은 XIV.10.6, 나머지 문서 불변식은 XIV.10.8(차단)에서 다룹니다.
