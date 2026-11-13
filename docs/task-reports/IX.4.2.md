# 작업 보고서: IX.4.2 nested instance property 직접 수정 금지

- WBS: IX.4.2
- 작업명: nested instance property를 상위 instance에서 직접 수정하는 API를 제공하지 않고 validator에서도 거부
- 상태: 완료 (`[x]`)
- 선행: IX.2.3(이 lane에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/encapsulation.rs`(신규) — `resolve_property_edit`, `PropertyEditTarget`,
  `instance_override_path`, `classify_property_path`, `PathTarget`, `validate_scene_encapsulation`,
  `EncapsulationError`.
- `crates/document/src/component/mod.rs` — 모듈 등록.
- `docs/task-reports/IX.4.2.md`(신규), 계획서(IX.4.2 `[ ]`→`[x]`).

## 구현 요약

- **API 미제공**
  - `resolve_property_edit(expansion, node)`: property 편집 대상은 scene 인스턴스뿐입니다. 인스턴스 root
    투영은 scene 인스턴스 자신, 인스턴스가 소유한 scene ComponentInstance(slot content)는 그 인스턴스를
    돌려줍니다. 가상 subtree 안 nested instance를 고르면 nested 경로 대신
    `RequiresDrilling { chain, component }`를 돌려줍니다(IX.4.3 DrillingPlan의 입력이 될 chain).
  - override command(IX.2.4)는 인스턴스 정의의 property id만 받아 nested 경로를 표현할 수 없습니다.
- **validator 거부**
  - PropertyOverride map의 nested 경로 key: IX.2.3 `NestedAccess`를 scene 전체 검사
    `validate_scene_encapsulation`이 그대로 적용합니다(미해결 참조 인스턴스는 진단 대상이라 건너뜀).
  - `classify_property_path`(variant patch·binding target 등 PropertyPath 검사): root가 컴포넌트 정의 tree
    node면 `DefinitionInternalTarget`, 인스턴스 override 경로가 canonical 형식(keys = `[PROPERTY_OVERRIDES]`,
    tail = 소문자 property UUID)이 아니면 `NestedOverridePath`, `COMPONENT_REF`·`SLOT_OVERRIDES` key 경로는
    `ProtectedInstanceKey`입니다. 직접 override 경로는 공개·선언 property여야 합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 264 passed, 통합 7+1+1+2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 정의 node root 경로, 이어 붙인 nested override 경로, 대문자·chain tail, COMPONENT_REF·SLOT_OVERRIDES 경로, payload nested key 전부 거부(`prohibited_direct_path_fixtures_fail`) | 통과(거부) |
| expansion 안 nested Button 선택 → `RequiresDrilling{[1001, 106], Button}`, 인스턴스 root → scene 인스턴스(`nested_instance_property_has_no_direct_edit_api`) | 통과 |
| canonical 직접 경로 수용, 일반 node·인스턴스 공통 필드 경로는 Node 대상, private/없는 property 거부(`canonical_direct_paths_are_accepted`) | 통과 |

## 차이(작업 지시와 다른 점)

- 인스턴스 override를 가리키는 `PropertyPath` 형식(`instance_override_path`)을 이 작업에서 정했습니다.
  binding target(VIII.6.1)·variant patch(IX.7.7)가 같은 형식을 씁니다.

## 남은 위험 / 한계

- binding 계층(VIII.6.1, 다른 lane)이 target PropertyPath 검증에 `classify_property_path`를 연결해야 binding
  경로의 nested 직접 접근도 막힙니다.
