# 작업 보고서: X.7.1 command invalidation → layout dirty set

- WBS: X.7.1
- 작업명: command invalidation을 layout dirty set으로 변환(PaintOnly는 layout queue 제외)
- 상태: 완료 (`[x]`)
- 선행: VII.1.4(`[x]`), X.1.3(`[x]`) — 명시 행 선행 모두 기준 commit에서 `[x]`, 완화 없음

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/dirty.rs`(신규) — `LayoutPropertySet`, `changed_properties`, `node_dirty_classes`,
  `ALL_LAYOUT_CLASSES`, `LayoutDirtySet`, `PropertyLayoutEffect`, `LayoutPropertyMap`, `CommandInvalidation`,
  `LayoutChangeSource`, `LayoutNodeLookup`, `LayoutInputIndex`, `dirty_set_from_changes`, 테스트.
- `crates/layout/src/lib.rs` — `dirty` 모듈 등록.
- `docs/task-reports/X.7.1.md`(신규), 계획서(X.7.1 `[ ]`→`[x]`).

## 구현 요약

- **crate 경계**: layout은 core-types만 의존하므로 VII.1.4 `ChangeSet`·문서 속성 registry를 직접 쓰지 않습니다.
  adapter가 `LayoutChangeSource`(바뀐 node와 무효화, 바뀐 `(node, PropertyKey)`, 삽입, 삭제 — id는 layout id)와
  `LayoutPropertyMap`(key → `None`/`PaintOnly`/`Layout { node, parent }`)을 구현합니다. 무효화 class는
  `CommandInvalidation`(VI.9.7 geometry·layout·paint의 layout 쪽 사본)로 받습니다.
- **변환 규칙**(`dirty_set_from_changes`): node 속성은 그 node에, 부모 쪽 속성(형제 순서·부모 변경)은 변경 전·후 부모
  각각에 class를 더합니다. class는 X.1.3 `dirty_classes`를 변경 전·후 상태 **양쪽**으로 구한 합집합입니다(콘텐츠 크기
  node의 DescendantConstraint는 SelfSize 포함). 삽입 node는 세 class, 삽입·삭제 node의 부모는 자식 변경(Children)을
  받습니다. layout 무효화가 기록된 layout 속성·자식 구조 변경으로 설명되지 않는 node(종류 변환, 알 수 없는 key)는
  세 class 모두로 보수 처리하고, layout 없이 paint·geometry만 있으면 렌더 dirty입니다.
- **PaintOnly 제외**: `LayoutDirtySet::mark`는 PaintOnly class를 렌더 집합(`paint()`)에만 넣고, `layout_queue()`는
  레이아웃에 영향을 주는 class가 있는 node만 돌려줍니다(StableId 순, 결정적).
- **입력 차이**: `changed_properties(before, after)`는 `LayoutNode`를 `..` 없이 구조 분해해 바뀐 속성을 구합니다(필드
  추가 시 컴파일 실패). 자식 목록은 visual order로 비교합니다. 증분 레이아웃(X.7.3)이 이 함수로 정확한 class를 구합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | unit 153 + `layout_corpus` 2 + doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| 속성별 기대 dirty set: 크기·텍스트 → SelfSize, Fixed node padding → DescendantConstraint, Hug node padding·layout mode → SelfSize+DescendantConstraint, 표시 여부·flex → ParentFlow, 형제 순서 → Hug 부모 SelfSize+DescendantConstraint, transform(PaintOnly class)·색 → layout queue 없음·paint만, 잠금 → 없음(`property_changes_map_to_expected_dirty_sets`) | 통과 |
| 변경 전·후 상태 합집합(Fixed↔Hug 전환과 padding 동시 변경, 양방향 동일)(`classes_use_both_before_and_after_states`) | 통과 |
| 삽입(부모 Fixed → DescendantConstraint, 새 node 세 class), 삭제, 이동(전·후 부모), 설명되지 않은 구조 무효화 보수 처리(`structure_insert_remove_and_move_mark_parents`) | 통과 |
| 알 수 없는 key: layout 무효화 → 세 class, paint만 → 렌더, layout node가 아닌 path root → layout queue 없음, 정밀 속성과 섞이면 보수(`unknown_keys_fall_back_to_node_invalidation`) | 통과 |
| `changed_properties`가 14개 필드 각각을 정확히 하나의 속성으로 검출, 자식 삽입 순서 무시(`changed_properties_detects_each_field`) | 통과 |
| 집합 연산·bit 위치 = registry tag(`dirty_set_and_property_set_operations`) | 통과 |

## 차이(작업 지시와 다른 점)

- `ChangeSet`·문서 속성 registry를 `LayoutChangeSource`·`LayoutPropertyMap`으로 감싸는 adapter는 layout crate가 상위
  crate에 의존할 수 없어 포함하지 않았습니다. 테스트는 문서 key 대역을 흉내 낸 fixture registry로 검증했습니다.

## 남은 위험 / 한계

- 같은 command에서 종류 변환과 layout 속성 변경이 함께 기록된 node는 기록된 속성의 class만 얻습니다(layout queue에는
  들어감). 정확한 class는 다시 읽은 입력의 `changed_properties` 차이(X.7.3)가 정합니다.
- Fragment·virtual node의 문서 id → layout id 대응은 adapter 책임입니다(X.1.2 `LayoutTreeView::layout_id`·
  `fragment_host` 사용).

## 후속 수정 (리뷰 minor: 무효화가 보고되지 않은 node의 알 수 없는 key)

- **결함**: `dirty_set_from_changes`는 알 수 없는 key를 그 node의 무효화(`changed_nodes`)로만 보수 처리했습니다. change
  source가 `changed_nodes`에 없는 node의 알 수 없는 key를 보고하면 그 변경이 dirty set에서 조용히 빠졌습니다.
- **수정**: 변환 규칙 5를 추가했습니다. `changed_nodes`에 무효화가 없는 node의 알 수 없는 key는 레이아웃 node면 세 class
  모두, 아니면 렌더 dirty로 표시합니다.
- **재현 테스트**(수정 전 실패 확인): `unknown_keys_without_node_invalidation_are_conservative` — 무효화 없는 알 수 없는 key,
  정밀 속성과 섞인 경우, 레이아웃 node가 아닌 path root, 다른 node 무효화만 있는 경우.
- **유지한 동작**: 무효화에 `layout`이 없는 node의 알 수 없는 key는 렌더 dirty로 둡니다. 무효화 class는 문서 속성 metadata
  registry(VI.9.7, `stellar_commands::change`의 `ChangeRecorder`가 속성마다 node 무효화를 기록)가 정한 값이라, layout 쪽
  key 표에 없다는 이유만으로 이를 뒤집지 않습니다(`unknown_keys_fall_back_to_node_invalidation`이 고정).
- 검증(`DEV-MAC`): `cargo test -p stellar-layout` unit 172 + `component_layout_corpus` 4 + `layout_corpus` 2 + doctest 2
  passed, 0 failed. `cargo clippy -p stellar-layout --all-targets -- -D warnings` 경고 0,
  `cargo fmt -p stellar-layout -- --check` 종료 코드 0.
