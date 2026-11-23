# 작업 보고서: XXX.7.2 `ContextProvision`·`ContextRequirement`·Slot 공개 목록

- WBS: XXX.7.2
- 작업명: node에 붙는 provision, 컴포넌트 requirement, Slot 공개 Context key 목록과 저장.
- 상태: 부분 완료 (`[~]` 후보) — 행 검증 기준(round-trip, 범위 밖 거부, instance node 허용, Slot 목록 round-trip)은 채웠지만, 앱 창 저장 경로가 Context를 넘기지 않아 저장 시 Context 원본이 지워집니다(아래, 통합 검토에서 확인). 보고 당시 기록: 완료(`[x]` 후보).
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

- 앱 창 저장(`stellar_runtime::window_session`)은 `SharedSnapshot`에 레코드만 담아 `serialize_records`와
  `DocumentModel::from_records` 뒤 텍스트 원본을 인코딩합니다. 그래서 `design/context.stellar.yaml`이 있는 프로젝트를 앱에서
  저장하면 그 파일이 삭제 변경으로 계획되고 context-set chunk도 빠집니다(손실 저장). generation→텍스트 원본 전환(`convert`)도
  레코드 사본이라 Context를 옮기지 않습니다. 고치려면 snapshot에 `ContextSet::live` 결과를 함께 담아
  `serialize_records_with_context`와 `replace_context`로 넘기면 됩니다(runtime, 소유 밖, 통합 전 후속). `stellar apply`(ops)는
  모델을 `serialize_document`로 쓰므로 Context를 보존합니다.
- 외부 원본 반영(`ApplyExternalSnapshot`, runtime `sync_external`)은 문서 레코드만 옮기므로 Context 파일을 밖에서 고치면
  열린 창에 반영되지 않습니다(소유 밖).
- storage `INITIAL_KINDS`에는 kind 7을 등록하지 않았습니다(보호 경로, ADR-0099 결정 12).

## 통합 보완 (2026-11-23, 앱 저장의 Context 손실)

- 결함: 앱 창 저장 snapshot(`stellar_runtime::window_session::SharedSnapshot`)이 문서 레코드만 담아, 저장이 텍스트 원본
  `design/context.stellar.yaml`을 삭제로 계획하고 context-set chunk도 빠뜨렸습니다. chunk 프로젝트를 텍스트 원본으로
  옮기는 변환(`window_session/source.rs`)도 Context를 옮기지 않았습니다.
- 수정: snapshot이 `model.context().live(model)`을 함께 담고, chunk 직렬화는 `serialize_records_with_context`, 텍스트
  원본 인코딩과 변환은 레코드로 다시 만든 model에 Context를 넣은 뒤 씁니다.
- 시험: `window_session::source_tests::save_keeps_the_document_context_in_the_source_and_the_cache`. 수정 전 코드에서는
  첫 저장 뒤 Context 원본 파일이 없어 실패하는 것을 확인했습니다.
- 남은 범위: 외부 변경 반영(`ApplyExternalSnapshot`)이 Context를 옮기지 않아, 밖에서 바꾼 Context 원본은 열린 창에
  반영되지 않고 그 창의 다음 저장이 덮어씁니다.
