# 작업 보고서: XIV.3.6 component definition·instance override 출처 구분

- WBS: XIV.3.6
- 작업명: component definition과 instance override diff를 별도 provenance와 함께 생성
- 상태: 완료 (`[x]`)
- 선행: IX.1, IX.2(절 단위 — 완화 적용). 필요한 코드 IX.1.1·IX.2.1·IX.2.2·IX.3.x는 `[x]`.
  IX.1.2~IX.1.4·IX.2.3·IX.2.4는 `[ ]`이지만 이 작업에 필요한 정의 resource·instance payload는
  이미 있습니다.

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-diff/src/build.rs` — `InstanceNode` 입력, `SnapshotBuilder::add_instance`.
- `crates/semantic-diff/src/diff.rs` — `property_provenance`, 출처 규칙 적용.
- `docs/task-reports/XIV.3.6.md`(신규), 계획서(XIV.3.6 `[ ]`→`[x]`).

## 구현 요약

- **scene instance 입력**: `DocumentRecords`가 아직 ComponentInstance payload variant를 갖지
  않아 `InstanceNode`(공통 node 필드 + `ComponentInstancePayload`)로 받습니다. override는 항목마다
  속성 하나입니다: `PROPERTY_OVERRIDES` + override key segment(nested 경로/Property id),
  `SLOT_OVERRIDES` + slot id segment.
- **출처 규칙**: 정의 resource·정의 tree node·Slot(Origin::Component)의 변경은
  `ComponentDefinition { document, component }`, scene 인스턴스의 override 경로 변경은
  `InstanceOverride { document, instance, component }`(참조 컴포넌트는 이후 값 우선)입니다.
  인스턴스의 정의 참조 교체와 공통 필드(이름·visible 등)는 scene 변경(`Document`)입니다.
  정의 tree 안 nested instance의 override는 정의의 일부라 ComponentDefinition입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-diff` | 32 passed, 0 failed |
| `cargo fmt -p stellar-semantic-diff -- --check` | 통과 |
| `cargo clippy -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 정의 root 크기 변경 + 인스턴스 override 추가가 각각 ComponentDefinition·InstanceOverride 출처, override 항목 단위 경로, override 값만 변경 시 1개(`definition_change_and_instance_override_have_distinct_provenance`) | 통과 |
| 인스턴스 이름 변경은 Document 출처(`instance_scene_fields_are_document_changes`) | 통과 |

## 차이(작업 지시와 다른 점)

- scene instance는 문서 레코드 대신 별도 입력(`InstanceNode`)으로 받습니다. 문서 레코드가
  instance variant를 갖게 되면 `add_document`가 직접 처리하도록 옮깁니다.

## 남은 위험 / 한계

- ComponentProperty record(IX.1.2)가 아직 없어 정의의 Property 변경은 Property id 목록
  (`COMPONENT_PROPERTIES`) 변경으로만 보입니다. 이름·타입·default 변경 diff는 IX.1.2 이후 추가.
