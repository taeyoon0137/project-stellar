# 작업 보고서: XVI.4.7 문서 command 검증 index

- WBS: XVI.4.7
- 작업명: command 검증이 호출마다 다시 만드는 문서 전체 index를 모델이 편집과 함께 증분 유지하는 id 종류·자식·inbound 참조 index로 바꿈
- 상태: 완료 (`[x]`)
- 선행: VII.2.7, VI.2.5
- 결정 기록: ADR-0108 결정 1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, release 측정 포함).

## 변경 파일 목록

- `crates/commands/src/tree.rs` — `ChildIndex`를 모델의 부모별 형제 index를 읽는 view로 바꿈(만들 때 문서를 훑지 않음),
  `id_use`·`IdUse`(영속 id 공간 조회) 추가, 문서 전체 재구성 함수(`ChildIndex::rebuild`, `reference_index`,
  `id_owners`)를 test oracle(`#[cfg(test)]`)로 내림.
- `crates/commands/src/convert.rs` — `ConvertShapeToPath`·`RestoreNodeRecords` 검증이 `id_owners` 대신 `id_use`를 씀.
- `crates/commands/src/node_ops.rs` — `InsertNode` 참조 종류 검사가 `id_use`를 씀. `ChildIndex::children`이 `Vec`을
  돌려주게 된 데 맞춘 반복 두 곳과 기존 index 비교 test.
- `crates/commands/src/property.rs` — Frame clip 대상 검사가 `id_use`를 씀.
- `crates/commands/src/page_view.rs`, `crates/commands/src/test_support.rs` — `ChildIndex<'_>` 수명 표기.
- `crates/commands/src/index_tests.rs`(신규), `crates/commands/src/lib.rs`(test module 등록).

## 구현 요약

- `DocumentModel`(document crate)은 이미 부모별 `(OrderKey, StableId)` 형제 집합, inbound/outbound 참조 index
  (`references()`), 영속 id 소유 수(`contains_persistent_id`)를 편집 API(`insert_node`·`remove_leaf`·`set_payload`·
  `set_modifier_stack`·`replace_definition`·`replace_text_styles`)와 함께 증분 갱신합니다. 검증이 문서 전체 index를
  다시 만들던 원인은 commands 쪽 helper(`id_owners`·`ChildIndex::build`·test용 `reference_index`)였으므로, helper가
  모델의 증분 index를 읽도록 바꿨습니다. document crate는 바꾸지 않았습니다.
- **id 종류**: command 검증은 "node이고 종류가 무엇인가"와 "node가 아닌 영속 id인가"만 구분합니다(clip 대상은
  `None | Node(Path)`, modifier 참조는 `None | Node(_)`, 새 path element는 사용 여부). 그래서 node가 아닌 쪽은 한
  값(`IdUse::Other`)으로 묶고 `payload(id)`와 `contains_persistent_id(id)` 두 조회로 답합니다(O(log N)).
- **자식**: `ChildIndex`는 모델을 빌리는 view이고 `children`은 `children_in_order`(해당 부모의 자식만 복사)입니다.
  DeleteSubtree·MoveNode·ConvertFragmentToFrame·편집 문맥의 subtree 순회가 subtree 크기에만 비례합니다.
- **inbound 참조**: 검증은 이미 `model.references()`를 썼고, 전체 재구성 `reference_index`는 test oracle입니다.
- **검사 규칙**: 오류 값과 검사 순서는 그대로입니다. 한 가지 차이는 아래 "판정이 바뀐 경우"에 적었습니다.

### 판정이 바뀐 경우 (ADR-0108 결정 1)

옛 `id_owners`는 주석에 "문서 validator VI.10.1의 id 공간과 같음"이라고 적혀 있었지만 typography resource id
(ADR-0067)를 넣지 않았습니다. 그래서 typography id를 새 path element id로 쓰는 ConvertShapeToPath·RestoreNodeRecords와,
typography id를 clip 대상으로 두는 SetProperty·InsertNode를 받아들인 뒤 결과 문서가 validator의 `DuplicateId`·
`RefTypeMismatch`를 어겼습니다. 모델의 영속 id 공간(validator와 같음)을 쓰면서 이 경우를 `DuplicateId`·
`ReferenceTypeMismatch`로 거부합니다. validator가 거부하는 결과를 막는 쪽으로만 바뀌었고, 검증 통과 ⇒ validator 통과
property test(`scoped_restore_validation_never_accepts_what_the_full_validator_rejects`)도 그대로 통과합니다.
같은 id를 node와 다른 영속 id가 함께 쓰는 문서(이미 validator 위반)에서는 node 종류를 우선합니다. 옛 map은 삽입
순서에 따라 달랐습니다.

## 성능

`cargo test -p stellar-commands --release index_tests::measure -- --ignored --nocapture`. 문서는 root ← group 100개 ←
leaf Frame(VII.2.7 사후 수정 측정과 같은 모양), 이름만 바꾸는 RestoreNodeRecords 1건의 `validate` 2,000회. 다른 작업
12개가 동시에 도는 부하 상태(load average 약 16)에서 잰 값입니다.

| leaf | node | 평균 | p50 | p99 | 최대 | 이전(VII.2.7 사후 수정) |
|---:|---:|---:|---:|---:|---:|---:|
| 1,000 | 1,101 | 254 ns | 250 ns | 333 ns | 1.4 µs | 0.27 ms |
| 10,000 | 10,101 | 256 ns | 250 ns | 334 ns | 0.6 µs | 3.70 ms |
| 50,000 | 50,101 | 300 ns | 292 ns | 417 ns | 3.7 µs | 28.9 ms |

50,000 node 값 0.3 µs로 기준 1 ms 이하(아키텍처 6.3 단일 점유 2 ms, 6.4 Undo/Redo 10 ms 안)를 만족합니다. 시험은
평균이 1 ms를 넘으면 실패합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-commands` | unit 196 passed, 6 ignored(기존 장기 fuzz·측정 포함), doctest 3 passed |
| `cargo test -p stellar-commands --release index_tests::measure -- --ignored --nocapture` | 위 표, 통과 |
| `cargo clippy -p stellar-commands --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-commands -- --check` | 통과 |

## 테스트

| 검사 | 결과 |
|---|---|
| random command sequence(문서 40개 × 40단계, 셋 중 하나는 직전 역 command 적용) 뒤 증분 index(id 종류·자식·참조)가 전체 재구성 index와 같음(`incremental_indexes_match_full_rebuilds_after_random_command_sequences`, 적용 900건·역 적용 200건 이상 확인) | 통과 |
| 범위 검사와 전체 validator oracle 비교(`scoped_restore_validation_never_accepts_what_the_full_validator_rejects`, 기존) | 통과 |
| typography id가 영속 id 공간에 들고 clip 대상이면 거부(`typography_ids_occupy_the_id_space_like_the_document_validator`) | 통과 |
| 기존 index 비교(`insert_indexes_match_full_rebuilds_after_command_sequences`) | 통과 |

## 차이(작업 지시와 다른 점)

작업 지시와 다른 점은 아래 "남은 위험 / 한계"에 함께 적었습니다.

## 남은 위험 / 한계

- `InsertNode`가 modifier stack을 함께 넣을 때 쓰는 `existing_stacks`(문서의 모든 modifier stack 복사, modifier 참조
  cycle 검사)는 이 행의 세 index 밖이라 O(N) 그대로입니다. modifier 의존 graph를 모델이 증분 유지해야 없앨 수 있습니다.
- `ApplyExternalSnapshot`은 외부 문서 전체를 비교하는 command라 O(N)이 본래 비용입니다(바꾸지 않음).
