# 작업 보고서: IX.2.2 instance expansion view

- WBS: IX.2.2
- 작업명: definition tree와 instance override를 합성한 read-only virtual subtree(실제 node 복제 금지)
- 상태: 완료 (`[x]`)
- 선행: IX.2.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/expansion.rs`(신규) — `SceneIndex`, `ScenePayloads`,
  `ExpandedNodeId`, `ExpandedNode`, `InstanceExpansion`, `ExpansionError`, `MAX_INSTANCE_NESTING`.
- `crates/document/src/component/mod.rs` — `pub mod expansion;` 등록.
- `crates/document/tests/component_expansion_scale.rs`(신규) — 5만 인스턴스 검증.
- `docs/task-reports/IX.2.2.md`(신규), 계획서(IX.2.2 `[ ]`→`[x]`).

## 구현 요약

- **가상 node 식별**: `ExpandedNodeId::Virtual { instances, source }` — scene 인스턴스부터 정의
  tree 안 nested 인스턴스까지의 chain과 정의 node StableId 쌍입니다. 실제 node·id를 새로 만들지
  않습니다.
- **복제 금지**: `ExpandedNode`는 library에 저장된 `DefinitionNode`의 **참조**만 보유합니다. 종류·
  표시·잠금·이름·payload를 그 참조에서 읽습니다.
- **합성**: 자식은 정의 tree의 OrderKey 순, nested ComponentInstance의 자식은 그 정의 root의 가상
  투영입니다. 인스턴스의 PropertyOverride는 `property_override`로 합성해 읽습니다(값을 node 속성에
  반영하는 binding은 Phase VIII). `parent`는 인스턴스 경계를 넘어 부모를 계산하고, `walk`는 전위
  순회입니다.
- **거부**: ComponentInstance가 아닌 node, 손상 payload, 정의 없는 컴포넌트(미해결 참조 —
  `PropertyPath(instance, COMPONENT_REF)`), 자기 자신을 중첩하는 재귀 정의, 중첩 64 초과, 다른
  expansion의 id.
- **SceneIndex**: 런타임 핸들 기반 `NodeColumns`를 StableId ↔ 핸들, 정렬 자식(OrderKey →
  StableId, VI.2.4와 같은 규칙)으로 한 번 색인하는 읽기 전용 index입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 159 passed, `component_expansion_scale` 1 passed, doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 5만 인스턴스(정의 node 11개, nested 포함)를 전부 펼쳐 가상 node 55만 개를 읽어도 서로 다른 정의 node 저장소 주소는 11개, scene node 수·정의 node 수 불변(`fifty_thousand_instances_share_one_definition_copy`) | 통과 |
| 가상 subtree가 정의 순서·nested 정의를 그대로 반영하고 정의 node 참조가 library 저장소와 동일 주소(`virtual_subtree_mirrors_definition_and_nested_instance`) | 통과 |
| 인스턴스 경계를 넘는 parent 계산, 다른 chain id 거부(`parent_links_cross_instance_boundaries`) | 통과 |
| PropertyOverride 합성 조회(`property_overrides_are_composed_from_instance`) | 통과 |
| 미해결 컴포넌트·비인스턴스·없는 node·손상 payload 거부(`invalid_instances_are_rejected`) | 통과 |
| 재귀 정의 거부(`recursive_component_is_rejected`) | 통과 |

## 차이(작업 지시와 다른 점)

- slot override의 투영은 Slot 정의(IX.3.1)가 필요해 IX.3.2에서 이 view에 연결합니다. 이 작업은
  정의 tree·nested 인스턴스·PropertyOverride 합성까지입니다.

## 남은 위험 / 한계

- PropertyOverride 값이 실제 node 속성에 반영되는 것은 binding 평가(Phase VIII)와 property 캐시
  (IX.1.4)가 생긴 뒤입니다.
- `SceneIndex` 구축은 O(n log n)입니다. 대규모 문서에서 증분 갱신은 후속 성능 작업입니다.
- nested 인스턴스 payload는 조회마다 디코딩합니다(중첩 깊이만큼). 필요하면 library 단위 캐시를
  추가합니다.
