# 작업 보고서: IX.3.2 Slot override 투영

- WBS: IX.3.2
- 작업명: Slot override는 instance가 소유한 nodes를 slot에 투영하고 definition tree 안으로 parent를 직접 바꾸지 않음
- 상태: 완료 (`[x]`)
- 선행: IX.3.1, IX.2.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/slot_override.rs`(신규) — `validate_scene_slot_overrides`,
  `validate_definition_slot_overrides`, `SlotOverrideError`.
- `crates/document/src/component/expansion.rs` — `ExpandedNodeId::Owned` 추가, slot host의 투영 자식,
  투영 node의 부모(slot host) 계산, 소유 node가 인스턴스인 경우 재펼침, `ExpansionError::NotProjected`.
- `crates/document/src/component/mod.rs` — `pub mod slot_override;` 등록.
- `crates/document/tests/component_slot_projection.rs`(신규) — save/load 소유 관계 검증.
- `docs/task-reports/IX.3.2.md`(신규), 계획서(IX.3.2 `[ ]`→`[x]`).

## 구현 요약

- **투영 의미**: slot override payload는 node 참조 목록뿐입니다. scene 인스턴스의 content는 scene에서
  부모가 **인스턴스 node 그대로**이고, expansion에서만 `ExpandedNodeId::Owned`로 slot host 아래에
  투영됩니다. 정의 tree는 바뀌지 않습니다. override가 없으면 host의 default content가 보입니다.
- **nested 인스턴스**: 정의 tree 안 인스턴스의 content는 같은 정의 tree에서 부모가 그 인스턴스인
  node이고, nested 정의의 slot host 아래에 투영됩니다.
- **expansion 확장**: slot host 자식 = 투영 목록(override 순서), 투영 node의 부모 = slot host,
  소유 node가 ComponentInstance면 그 정의를 다시 펼칩니다. expansion 범위는 인스턴스의 scene 후손으로
  한정합니다.
- **validator**: content가 없는 node(정의 tree node 포함)·부모가 인스턴스가 아닌 node(소유 자손 포함)·
  중복 투영·소유 자식 누락·없는 slot·slot 종류/개수 위반을 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 168 passed, `component_expansion_scale` 1, `component_slot_projection` 1 passed, doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| library·scene(+instance payload)을 bytes로 저장 후 역순 삽입으로 재구성해도 content의 scene 부모 = 인스턴스, 정의 host 자식 불변, validator 통과, 투영 subtree·expansion 부모 동일, 재저장 byte-identical(`ownership_survives_save_and_load`) | 통과 |
| 투영 순서·expansion 부모(slot host)·scene 부모(인스턴스) 분리, 소유 인스턴스 재펼침, 정의 tree 불변(`owned_nodes_are_projected_without_reparenting`) | 통과 |
| override 없으면 default content(`default_content_shows_when_not_overridden`) | 통과 |
| 비소유 node·정의 node·소유 자손 투영, 누락, 중복, 금지 종류, 없는 slot, 개수 초과 거부(`projection_rule_violations_are_reported`) | 통과 |
| nested 인스턴스 content는 바깥 정의 node에서 투영, 미투영 거부(`nested_instance_slot_content_comes_from_outer_definition`) | 통과 |

## 차이(작업 지시와 다른 점)

- `ExpandedNodeId`에 `Owned` variant를 추가하면서 IX.2.2 unit test helper(`sources`)의 `match`에
  컴파일용 arm 한 줄을 추가했습니다. 해당 fixture에는 slot이 없어 그 arm은 실행되지 않으며 기존 단언은
  바꾸지 않았습니다.
- 문서 serializer(VI.10.2)가 아직 없어 scene node 저장은 테스트 전용 단순 형식으로 했고, library와
  instance payload는 실제 codec을 사용했습니다.

## 남은 위험 / 한계

- scene 전체 validator(VI.10.1)가 이 검사들을 호출하도록 연결하는 것은 그 작업의 몫입니다.
- 소유 node의 자손 구조는 자유 편집 대상이며, 그 경계는 IX.2.5 validator가 다룹니다.
