# 작업 보고서: XIV.10.6 OrderKey 독립 삽입 병합과 상충 재정렬

- WBS: XIV.10.6
- 작업명: OrderKey independent insertion을 자동 병합하고 같은 child conflicting reorder를 conflict로 분류
- 상태: 완료 (`[x]`)
- 선행: III.5.4 `[x]`, XIV.10.2 `[x]`(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-merge/src/merge.rs` — `rekey_independent_insertions`, `merge_cycle`,
  `position_conflict`, III.5.4 fixture·재정렬 fixture.
- `docs/task-reports/XIV.10.6.md`(신규), 계획서(XIV.10.6 `[ ]`→`[x]`).

## 구현 요약

- **독립 삽입**: 서로 다른 StableId의 자식 추가는 겹치지 않아 모두 받아들입니다. 같은 간격에
  삽입해 `OrderKey::between`이 같은 key를 낸 경우(III.5.4 fixture 1), 순서는 `(OrderKey,
  StableId)`로 이미 정해지므로 묶음의 가장 작은 StableId는 key를 유지하고 다른 쪽에서 온 나머지만
  다음 다른 key와의 사이 key로 다시 매깁니다. 시각 순서는 fixture 기대값과 같고 형제 key가
  유일해집니다(문서 validator VI.10.1 OrderKey 유일성). base 자식과 입력에 이미 있던 충돌은
  바꾸지 않습니다.
- **같은 child 상충 재정렬**: 양쪽이 같은 객체를 다른 위치(parent 또는 OrderKey)로 옮기면
  `conflicting-reorder`(대상 위치, base·ours·theirs 위치 보존, candidate는 base 위치). 같은 위치면
  한 변경입니다(XIV.10.3).
- **이동 cycle**: 서로 다른 객체의 독립 이동이 합쳐져 parent cycle이 되면(A를 B 아래로 / B를 A
  아래로) cycle 구성 객체의 이동을 되돌리고 각각 `conflicting-reorder`로 기록합니다. parent 사슬은
  반복문 + 방문 집합으로 따라갑니다(재귀 없음).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-merge` | 20 passed, 0 failed |
| `cargo fmt -p stellar-semantic-merge -- --check` | 통과 |
| `cargo clippy -p stellar-semantic-merge --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| III.5.4 fixture 전부(같은 간격 동일 key·앞/뒤 독립 삽입): schema 검증, conflict 0, 병합 자식 순서 = `expected_merged`, 형제 key 유일, base key 불변(`order_key_merge_fixtures_pass`) | 통과 |
| 같은 child 다른 key → conflicting-reorder·위치 3종 기록·candidate base 위치; 같은 재정렬·다른 child 재정렬은 clean(`conflicting_reorder_of_same_child`) | 통과 |
| 독립 이동 cycle → 두 객체 conflicting-reorder, candidate = base; cycle 없는 독립 이동은 clean(`independent_moves_forming_a_cycle_are_conflicts`) | 통과 |
| 기존 fixture(`same_edits_on_both_sides_property` 등)에서 입력에 이미 있던 key 충돌을 바꾸지 않음 | 통과 |

## 차이(작업 지시와 다른 점)

- 같은 key 독립 삽입의 key 재배정과 이동 cycle 검사를 넣었습니다. III.5.4 fixture는 순서만
  요구하지만, 그대로 두면 병합 결과가 문서 validator의 OrderKey 유일성·parent 불변식을 어기므로
  자동 병합 결과가 조용히 invalid해지지 않도록 했습니다(full validator 연결 XIV.10.8은 차단).

## 남은 위험 / 한계

- key 재배정은 merge 결과에만 적용되며 다른 쪽의 원래 key와 달라집니다(변경 목록에 반영).
- 객체 종류(Node/Modifier/View)를 가리지 않고 같은 규칙을 씁니다. 페이지·View 목록 순서는
  OrderKey가 아니라 목록 속성이라 이 규칙의 대상이 아닙니다(XIV.3.1 한계).
