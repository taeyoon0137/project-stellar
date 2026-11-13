# 작업 보고서: XIV.10.5 delete/move/type conflict 분류

- WBS: XIV.10.5
- 작업명: delete-vs-modify, delete-vs-move, type-change-vs-value-change conflict를 각각 다른 kind로 분류
- 상태: 완료 (`[x]`)
- 선행: XIV.10.1 `[x]`(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-merge/src/merge.rs` — `delete_conflict_kind`, `property_conflict_kind`,
  고아 parent 분류(`merge_orphan`), conflict matrix fixture.
- `docs/task-reports/XIV.10.5.md`(신규), 계획서(XIV.10.5 `[ ]`→`[x]`).

## 구현 요약

- **삭제 대 편집**: 한쪽이 객체를 삭제하고 다른 쪽이 위치만 바꿨으면 `delete-vs-move`, 속성 또는
  구조를 바꿨으면(이동 + 속성 포함) `delete-vs-modify`. 양쪽 같은 삭제는 conflict가 아닙니다
  (XIV.10.3).
- **삭제된 parent**: 다른 쪽이 그 parent 아래로 **옮겨 온** 객체 때문에 고아가 생기면
  `delete-vs-move`, **새로 추가한** 객체 때문이면 `delete-vs-modify`(subtree 수정).
- **타입 대 값**: 같은 속성에서 한쪽은 값 타입(`ValueType` = 값 종류 tag + TypedValue tag)을
  바꾸고 다른 쪽은 같은 타입 안에서 값만 바꿨으면 `type-change-vs-value-change`. 양쪽이 서로 다른
  타입으로 바꾸거나 같은 타입 다른 값이면 `property-value`. 속성 추가·삭제는 타입 변경이 아닙니다.
- **교체**: 같은 StableId를 다른 종류로 다시 정의한 쪽과 수정한 쪽은 `inconsistent-definition`.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-merge` | 17 passed, 0 failed |
| `cargo fmt -p stellar-semantic-merge -- --check` | 통과 |
| `cargo clippy -p stellar-semantic-merge --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| conflict matrix 14행(삭제×수정/이동/재정렬/이동+수정 양방향, 타입×값 양방향, 타입×타입, 값×값, 삭제 parent×자식 추가/이동, 교체×수정, 삭제×삭제, 이동×이동) 기대 kind 정확, matrix가 6개 kind를 모두 서로 다르게 생성(`conflict_matrix`) | 통과 |
| delete-vs-move 기록: 대상 객체, base 객체·ours Missing·theirs 이동 객체, candidate는 base 객체 유지(`delete_conflict_keeps_base_object_and_records_sides`) | 통과 |

## 차이(작업 지시와 다른 점)

- 삭제된 parent 아래 추가·이동을 같은 분류 규칙으로 delete-vs-modify/delete-vs-move에 넣었습니다
  (XIV.10.2 구조 정합 검사의 분류 세분).

## 남은 위험 / 한계

- token 타입(`TOKEN_TYPE`) 변경 대 token 값(`TOKEN_VALUE`) 변경은 서로 다른 경로라 자동
  병합됩니다. token 전용 conflict는 XIV.10.7(차단: IX.1.3·IX.4.6·VIII.5.5 미완료) 범위입니다.
