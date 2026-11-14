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

## 알려진 결함 SM-1 (통합 테스트에서 발견, 2026-11-14)

- **증상**: 양쪽이 서로 다른 페이지·View를 독립적으로 추가하면(아키텍처 §27.1 "충돌하지 않는 자식 삽입", 자동 병합
  대상) semantic-merge가 부모 정의 객체의 `DOCUMENT_PAGES`·`PAGE_VIEWS`(Ids 목록 속성, `crates/semantic-diff/src/keys.rs`)를
  원자 값으로 비교해 목록 PropertyValue conflict를 냅니다. 같은 merge가 새 Page·View·root 객체의 AddObject는 받아들이므로
  candidate의 정의 목록과 객체가 어긋납니다(목록에 없는 Page·View, 어떤 View 목록에도 닿지 않는 root Frame). 이 상태는
  VI.10.1 root count 규칙으로 되돌릴 수 없습니다.
- **재현**: `tests/integration/tests/git_semantic_merge.rs`의
  `independent_page_and_view_insertions_merge_cleanly`(`#[ignore]`, 결함 사유 명시).
  `cargo test -p stellar-integration-tests --test git_semantic_merge -- --ignored independent_page_and_view_insertions_merge_cleanly`가
  실패합니다. `crates/semantic-merge`는 이 결함을 드러낸 통합 브랜치에서 바뀌지 않았으므로 기존 결함입니다.
- **이 행과의 관계**: 이 행의 범위는 OrderKey 자식 순서입니다. 페이지·View 순서는 OrderKey가 아니라 목록 속성이라 이 행의
  규칙 대상이 아니고(위 "남은 위험 / 한계"), 검증 열의 Phase III fixture는 그대로 통과합니다. 그래서 상태(`[x]`)는 바꾸지
  않았습니다. 목록 속성의 독립 삽입 병합을 맡은 WBS 행은 계획서에서 찾지 못했습니다. 후속 행 추가나 이 행 재검토 여부는
  계획 변경 결정이 필요합니다.
- **XIV.10.8 선행 조건**: XIV.10.8(`[ ]`, conflict 없는 merged snapshot의 full validator 검증)은 이 결함의 candidate처럼
  정의 목록과 객체가 어긋난 결과를 자동 merge 성공으로 돌려주지 않아야 합니다. XIV.10.8 착수 시 위 재현 테스트를 invalid
  candidate fixture로 함께 다뤄야 합니다. 수정되면 테스트의 `#[ignore]`를 지웁니다.

### SM-1 해소 (2026-11-14)

- **수정**: `crates/semantic-merge/src/merge.rs`. 양쪽이 모두 바꾼 정의 목록 속성(`DOCUMENT_PAGES`·`PAGE_VIEWS`·
  `COMPONENT_SLOTS`)은 객체 단계에서 원자 값으로 비교하지 않습니다. 구조·위치·고아·cycle 처리가 모두 끝난 뒤, candidate에
  실제로 남은 소속 객체(semantic validator XIV.5.3의 소속 규칙)를 원소로 목록을 다시 만듭니다(`reconcile_member_lists`,
  `merge_member_order`).
  - 순서: 양쪽 목록에 모두 있는 객체는 양쪽 순서가 같으면 그 순서, 한쪽만 base 순서와 다르면 그쪽 순서입니다. 한쪽 목록에만
    있는 객체(삽입, 또는 받아들이지 않은 삭제의 대상)는 그 목록에서 바로 앞 객체 뒤에 둡니다. 같은 자리에 양쪽이 삽입하면 첫
    StableId가 작은 묶음이 먼저입니다(좌우 교환에도 같은 결과).
  - 양쪽이 같은 목록을 서로 다른 순서로 재정렬하면 목록 속성 대상 `conflicting-reorder`입니다. conflict는 base·ours·theirs 목록
    값을 담고, candidate는 남은 객체의 base 순서를 유지합니다.
  - 목록 원소 집합은 conflict 여부와 무관하게 항상 candidate 소속 객체와 같습니다. 예: 한쪽 페이지 삭제 대 다른 쪽 페이지 이름
    변경이면 페이지는 delete-vs-modify conflict로 base 상태를 유지하고 문서 페이지 목록에도 남습니다. 받아들인 View 삭제는 그
    페이지의 View 목록에서 빠집니다.
  - 비용: 소속 map을 snapshot마다 한 번 순회해 만들고, 목록마다 정렬 집합·map만 써서 O(n log n)입니다.
- **테스트**:
  - 통합 테스트 `independent_page_and_view_insertions_merge_cleanly`의 `#[ignore]`를 지우고 좌우 교환 검사(`assert_symmetric`)를
    더했습니다.
  - `crates/semantic-merge/src/merge.rs` unit 테스트 8개: 독립 페이지·View 삽입, 앞 객체 기준 삽입과 삭제, 한쪽 재정렬과 다른
    쪽 삽입, 상충 재정렬 conflict, 페이지 삭제 대 이름 변경, Slot 독립 삽입, 무작위 300 seed 목록 편집(목록 정합, 좌우 교환,
    양쪽 재정렬이 아닌 목록 conflict 0), 목록과 소속 객체가 어긋난 입력. 수정 전 엔진에 앞 7개를 붙여 실행하면 모두 실패하고
    기존 20개는 통과하는 것을 확인했습니다.
- **XIV.10.8과의 관계**: 위 재현 입력의 candidate는 이제 정의 목록이 객체와 맞습니다. full validator 연결(XIV.10.8, `[ ]`)은
  그대로 남은 작업입니다.
- **남은 한계**:
  - 재정렬은 목록 단위로 판단합니다. 양쪽이 서로 다른 원소를 각자 옮긴 경우도 conflict로 남깁니다(보수적 판단).
  - 한쪽이 View와 그 root Frame을 지우고 다른 쪽이 그 View만 고치면 View는 delete-vs-modify conflict로 남고 View 목록에도
    남지만, root 삭제는 받아들여져 `VIEW_ROOT_FRAME`이 없는 객체를 가리킵니다. node subtree 삭제 대 수정과 같은 기존
    규칙(conflict 대상 객체만 base 유지)에서 나오는 문제이며 SM-1의 목록 정합 범위 밖입니다. XIV.10.8 full validator 연결에서
    다뤄야 합니다.
