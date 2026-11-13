# 작업 보고서: VI.10.3 chunk model → document loader

- WBS: VI.10.3
- 작업명: chunk model→document loader, ID registry와 parent order 재구축
- 상태: 완료 (`[x]`)
- 선행: VI.10.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증. 저장 round-trip은 실제 파일시스템 temp dir 사용).

## 변경 파일 목록

- `crates/document/src/model.rs` — `DocumentModel::from_records`, `LoadModelError`.
- `crates/project/src/document_io/load.rs`(신규) — `load_document`,
  `load_document_from_generation`, `find_document_manifest`, `ChunkSource` trait,
  `MemoryChunks`, `LazySource`(storage lazy loader 래퍼), `LoadDocumentError`, `LoadedDocument`.
- `crates/project/src/document_io/mod.rs` — `load` 모듈 등록.
- `crates/project/tests/common/golden.rs`(신규) — index→고정 문서 golden 생성기.
- `crates/project/tests/common/mod.rs` — `golden` 모듈 등록.
- `crates/project/tests/document_round_trip.rs`(신규).
- `docs/task-reports/VI.10.3.md`(신규), 계획서(VI.10.3 `[ ]`→`[x]`).

## 구현 요약

- **loader 경로**: DocumentManifest payload → 정의의 View 순서대로 ViewTree chunk → 그 ViewTree가
  가리키는 subtree chunk(여러 개 가능)를 **저장된 참조만** 따라 읽습니다(ChunkId 유도 규칙 비가정).
  ViewTree root ≠ 정의 root, subtree root ≠ 참조 root, 읽기 실패, payload 손상은 각각 typed
  error입니다.
- **재구축**: 합친 레코드를 `DocumentModel::from_records`가 먼저 전체 validator(VI.10.1)로 검증하고,
  View 정의 순서로 root부터 pre-order(형제는 OrderKey, StableId 순)로 node를 넣어 새 runtime 핸들,
  StableId↔핸들 ID registry, 부모·자식 순서를 입력 레코드 순서와 무관하게 결정론적으로 만듭니다.
  modifier stack은 마지막에 붙입니다(Fragment 금지 등 모델 규칙 재적용).
- **lazy 연동**: `load_document_from_generation`은 열기 때 읽힌 DocumentManifest(V.6.6)를 쓰고
  요청 문서의 ViewTree·subtree chunk만 `LazyGeneration::read_chunk_payload`로 읽습니다.
- **golden 문서**: 파일 대신 index→문서 생성기(정수 연산과 1/4 단위 f32만 사용)로 100개를 만듭니다.
  페이지 1~2·View 1~3, 모든 node 종류와 Shape 4종, 미해결 참조, modifier 12종(비활성 포함, 참조는
  먼저 만든 node만 가리켜 DAG), depth 64 chain 문서 4개를 포함합니다. 형식 고정을 위해 100개 chunk
  model 전체 content hash를 golden 값으로 고정했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 179 passed + doctest 2 passed, 0 failed |
| `cargo test -p stellar-project` | unit 138, `document_round_trip` 4, `document_serialize` 4 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check`, `cargo fmt -p stellar-project -- --check` | 통과 |
| `cargo run -q -p xtask -- validate-crate-graph` | 통과(exit 0) |

| 검사 | 결과 |
|---|---|
| 100개 golden 문서: 직렬화→로드 후 레코드 동일, 모든 node 자식 순서 동일, registry 왕복, 진단 동일, 재직렬화 byte-identical, 전체 chunk hash golden 일치(`hundred_golden_documents_round_trip_semantically_equal`) | 통과 |
| 100개 문서를 실제 저장 파이프라인(`build_save_plan`+`run_save`)으로 한 generation에 저장 → `LazyGeneration`에서 문서별 로드 의미 동일, 읽은 chunk = 그 문서의 ViewTree+subtree뿐(`saved_generation_loads_each_golden_document_lazily`) | 통과 |
| ViewTree subtree 2개로 나눈 chunk model 병합(`view_tree_with_two_subtrees_is_merged`) | 통과 |
| 없는 chunk·ViewTree root 불일치·subtree root 불일치·subtree 손상·불변식 위반(형제 key 중복)·manifest 손상 typed error(`corrupted_chunk_models_are_rejected_with_typed_errors`) | 통과 |
| `from_records` 입력 순서 무관 재구축·registry·자식 순서, 위반 레코드 거부(document unit) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. "100개 golden document"는 지시에 따라 체크인 파일 대신 결정론적 생성기로 만들고 전체
  chunk hash 하나만 고정했습니다.

## 남은 위험 / 한계

- 저장 round-trip 테스트는 chunk마다 fsync를 수행해 약 6~7초 걸립니다(디버그, `DEV-MAC`).
- golden hash는 저장 형식 변경 감지용입니다. 의도한 스키마 변경 시 schema version 상승과
  migration(V.8)을 함께 하고 값을 갱신해야 합니다.

## 후속 수정 (리뷰 minor: 저장된 빈 modifier stack 레코드)

- **결함**: 런타임 모델은 빈 stack을 보관하지 않지만(`set_modifier_stack`이 제거), validator가 빈 stack 레코드를 허용해
  그런 chunk가 로드되면 재직렬화 bytes가 저장 bytes와 달라졌습니다(load→save 불안정, residency dirty 판정이 영구 dirty).
- **수정**: validator가 `DocumentViolation::EmptyModifierStack`으로 거부합니다(로드·저장 양쪽). 모델은 빈 stack을 만들지
  않으므로 유효한 문서의 저장 결과는 바뀌지 않습니다(golden chunk hash 불변).
- **기존 테스트 변경(명세 동작 변경)**: VI.10.1 테스트 `modifier_cycle_and_fragment_modifier_report_locations`의 "빈 stack은
  Fragment도 허용" 단언을 `EmptyModifierStack` 위반으로 바꾸고, `modifier_stack_owner_violations`의 소유 node 없는 stack에
  항목 하나를 넣어 같은 위반(소유 node 없음·중복)만 검사하게 했습니다.

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | unit 245 포함 전부 passed, 0 failed |
| `cargo test -p stellar-project` | unit 168, `document_residency` 3, `document_round_trip` 5, `document_serialize` 4, `restricted_yaml_fuzz` 4 passed, 0 failed |
| `cargo clippy -p stellar-document -p stellar-project --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -p stellar-project -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 빈 stack 레코드 위치 보고, 소유 node 없음과 함께 둘 다 보고(`empty_modifier_stack_record_is_rejected`) | 통과 |
| 빈 stack 레코드를 넣은 subtree chunk 로드가 `Invalid(EmptyModifierStack)`, 정상 chunk는 로드 후 재직렬화 byte-identical(`stored_empty_modifier_stack_record_is_rejected`) | 통과 |
| 100개 golden round-trip·golden chunk hash(`hundred_golden_documents_round_trip_semantically_equal`) | 통과(hash 불변) |
