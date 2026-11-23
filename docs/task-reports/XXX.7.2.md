# 작업 보고서: XXX.7.2 `ContextProvision`·`ContextRequirement`·Slot 공개 목록

- WBS: XXX.7.2
- 작업명: node에 붙는 provision, 컴포넌트 requirement, Slot 공개 Context key 목록과 저장.
- 상태: 완료 (`[x]` 후보) — 검증 기준을 채웠습니다. 외부 원본 반영 경로 한계는 아래에 적었습니다.
- 결정: ADR-0099 결정 1·2, ADR-0025 결정 3.2·3.3·3.11
- 선행: XXX.7.1, IX.1.1, IX.2.1, IX.3.1

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/document/src/context/{provision,set,set_tests,fixtures}.rs`(신규).
- `crates/document/src/model.rs`: `DocumentModel`에 `ContextSet` 거울(`context`, `context_mut`, `replace_context`).
- `crates/project/src/document_io/{manifest,serialize,load}.rs`: `context-set` chunk(kind `0x8000_0007`)와 manifest 선택 절
  (pointer schema 3).
- `crates/project/src/document_io/source_v4/{context,mod,project}.rs`: 텍스트 원본 `design/context.stellar.yaml`.

## 구현 요약

- provision은 id·owner(View 또는 컴포넌트)·provider node·key·값 식입니다. `check_provision_scope`가 provider를 owner의 tree
  안 node로 제한하고, 자기 tree 안의 ComponentInstance node는 허용하며, 다른 컴포넌트 내부 node·다른 View node는 거부합니다.
- requirement(컴포넌트 → key 집합)와 Slot 공개 목록(Slot → key 집합)은 같은 `ContextSet`에 둡니다(ADR-0099 결정 1: 컴포넌트
  interface가 텍스트 원본에 아직 없어 ComponentDefinition bytes를 바꾸지 않음).
- 삭제된 scene node의 View provision은 모델에 남고 영속 출력(`live`)에서만 빠집니다(삭제 undo 대비).
- Context가 없는 문서의 manifest·chunk·텍스트 파일 bytes는 그대로입니다(시험으로 확인).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document --lib context::` | 통과 |
| `cargo test -p stellar-project` | 통과(lib 442개 등 전체) |

시험:

- `context::set_tests::set_round_trips_with_requirements_and_slot_keys`: round-trip, Slot 공개 목록 round-trip, 꼬리·version 거부.
- `context::set_tests::provision_scope_rejects_nodes_outside_owner_and_allows_own_instance_nodes`.
- `context::set_tests::set_rejects_duplicates`, `live_set_hides_provisions_of_deleted_scene_nodes`.
- `document_io::source_v4::context::tests::context_source_round_trips_literal_and_ast_expressions`,
  `project_source_and_chunk_cache_carry_context`(텍스트 원본·chunk cache, Context 없을 때 bytes 불변).

## 차이(작업 지시와 다른 점)

- requirement·Slot 목록의 저장 위치를 IX.12.1이 예약한 ComponentDefinition·Slot 확장 section 대신 `ContextSet` 안 같은 tag의
  section으로 정했습니다(ADR-0099 결정 1, 대안 절).
- 소유 범위 밖 owner 소속 판정은 document 계층에 있고, command 계층은 컴포넌트 library를 모르므로 View owner만 직접
  거부합니다(컴포넌트 owner는 validator, XXX.7.5).

## 남은 위험 / 한계

- 외부 원본 반영(`ApplyExternalSnapshot`, runtime `sync_external`)은 문서 레코드만 옮기므로 Context 파일을 밖에서 고치면
  열린 창에 반영되지 않고, 다음 저장이 창의 Context로 덮어쓸 수 있습니다. layout record와 같은 종류의 남은 일입니다(소유 밖).
- storage `INITIAL_KINDS`에는 kind 7을 등록하지 않았습니다(보호 경로, ADR-0099 결정 12).
