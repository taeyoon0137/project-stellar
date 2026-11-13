# 작업 보고서: VI.10.2 document → chunk model serializer

- WBS: VI.10.2
- 작업명: document→chunk model serializer, runtime ID 비저장
- 상태: 완료 (`[x]`)
- 선행: VI.10.1, V.6.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/model.rs`(신규) — 런타임 `DocumentModel`(NodeColumns + IdRegistry +
  StableId 키 payload·modifier map), `ModelError`, `to_records`.
- `crates/document/src/record_codec.rs`(신규) — node 레코드 canonical 인코딩, `SubtreeRecords`
  (subtree chunk payload), `DocumentRecords::view_subtrees`·`from_subtrees`, `PartitionError`,
  `RecordCodecError`.
- `crates/document/src/shape.rs` — `ShapeGeometry::encode/decode`, `PolygonParams` 접근자.
- `crates/document/src/path.rs` — `IdentifiedPath::encode/decode`(point·handle id 보존).
- `crates/document/src/definitions.rs` — 단일 `DocumentDefinition::encode/decode`.
- `crates/document/src/lib.rs` — 모듈 등록.
- `crates/project/src/document_io/{mod.rs, manifest.rs, serialize.rs}`(신규) — chunk model 매핑,
  `DocumentManifestChunk`, `serialize_document`·`serialize_records`, `SerializedDocument`,
  ChunkId 유도 함수, `DOCUMENT_CHUNK_SCHEMA_VERSION`.
- `crates/project/src/lib.rs` — `pub mod document_io;` 한 줄.
- `crates/project/tests/common/mod.rs`, `crates/project/tests/document_serialize.rs`(신규).
- `docs/task-reports/VI.10.2.md`(신규), 계획서(VI.10.2 `[ ]`→`[x]`).

## 구현 요약

- **crate 경계**: document는 storage에 의존할 수 없으므로 storage 독립 레코드 인코딩
  (node 레코드·subtree payload·shape·path·문서 정의)은 document에, storage chunk 타입
  (kind, `ViewTreeChunk`, `ChunkId`, canonical encoder)으로의 매핑은 project에 두었습니다.
- **chunk model**: 문서당 DocumentManifest(문서 정의 + View별 ViewTree ChunkId), View당
  ViewTree(storage `ViewTreeChunk`: root Frame + subtree 참조), View당 subtree 내용 chunk
  (`SubtreeRecords`: pre-order node 레코드 + 분리된 modifier stack). subtree 내용 chunk는
  기존 lazy loader 관례처럼 `KIND_VIEW_TREE`를 씁니다(storage kind registry 무변경).
- **ChunkId**: 담는 의미 객체(문서·View·subtree root)의 StableId에서 유도해 같은 문서는 항상
  같은 ChunkId 집합을 만듭니다. loader는 유도 규칙이 아니라 저장된 참조를 따릅니다.
- **runtime ID 비저장**: 직렬화 입력은 `to_records()`가 부모 핸들을 StableId로 바꾼 레코드뿐이며
  레코드 타입에 `RuntimeEntityId`가 없습니다. 직렬화 전 validator(VI.10.1) 실패 시 chunk를
  만들지 않고, subtree 분할은 모든 node·stack이 정확히 한 번 배치되지 않으면 거부합니다.
- **분할 정책**: View당 subtree 하나. 크기 기반 분할(V.6.4)은 임계치 ADR 승인 대기라 추가하지
  않았습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 177 passed + doctest 2 passed, 0 failed |
| `cargo test -p stellar-project` | unit 138 passed, `document_serialize` 4 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check`, `cargo fmt -p stellar-project -- --check` | 통과 |
| `cargo run -q -p xtask -- validate-crate-graph` | 통과(exit 0) |

| 검사 | 결과 |
|---|---|
| 모든 node 핸들 generation을 0x1F2E로 churn한 뒤 (slot, generation) LE/BE 8바이트 조합과 generation 4바이트가 chunk payload와 실제 기록 파일(envelope 포함)에 없음 + scanner positive control(`save_bytes_contain_no_runtime_entity_id_pattern`) | 통과 |
| 삽입 순서·churn이 다른 두 모델(핸들 상이)의 chunk model byte-identical, 반복 직렬화 동일(`serialization_is_independent_of_runtime_layout`) | 통과 |
| manifest·ViewTree·subtree kind/ChunkId/schema version, 전체 node 1회 배치, stack 분리 저장(`chunk_model_has_manifest_view_trees_and_subtrees`) | 통과 |
| 불변식 위반·NIL 문서는 chunk 0개(`invalid_document_produces_no_chunks`) | 통과 |
| node 레코드 전 payload 종류 round-trip·malformed 거부, subtree 분할·재결합, shape/path/정의 codec round-trip | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. ComponentInstance는 payload 스키마(Phase IX) 전이라 레코드 codec이
  `UnsupportedNodeKind`로 거부합니다.

## 남은 위험 / 한계

- 핸들 slot·generation은 core-types에 공개 접근자가 없어 테스트가 `Debug` 출력에서 읽습니다.
  `Debug` 형식이 바뀌면 테스트 helper를 갱신해야 합니다.
- 문서 간 node id 충돌은 문서별 validator 범위 밖입니다. 같은 프로젝트의 두 문서가 같은 node
  id를 쓰면 subtree ChunkId가 겹쳐 generation manifest 검증(V.3.2)에서 저장이 거부됩니다.
- Text style 표·layout 정의는 아직 문서 resource 스키마가 없어 저장 대상이 아닙니다.

## 후속 수정 (리뷰 minor: ChunkId 계열·문서 domain 분리)

- **결함**: 문서 ChunkId를 원시 StableId에서 그대로 만들었습니다(manifest = 문서 id, ViewTree = View id, subtree = root
  node id). 계열 사이(같은 id가 다른 계열 객체에 쓰일 때)와 문서 사이(복제 문서처럼 같은 View·node id를 가진 두 문서)에서
  ChunkId가 겹칠 수 있었습니다(위 "남은 위험"의 문서 간 node id 충돌).
- **수정(저장 형식 변경)**: ChunkId = `SHA-256("stellar/document-chunk-id/v1\0" ‖ 계열 tag(manifest 1·ViewTree 2·subtree 3)
  ‖ 문서 StableId ‖ 객체 StableId)`의 앞 16바이트를 RFC 9562 UUIDv8(version 8, variant 0b10)로 만든 값입니다.
  `view_tree_chunk_id`·`subtree_chunk_id`는 문서 id를 함께 받습니다. version 8은 StableId(v7)를 그대로 쓰는 다른 ChunkId와
  같은 값이 될 수 없습니다. loader는 저장된 참조만 따르므로 로드 경로 변경은 없습니다.
- **golden 갱신(의도)**: `document_round_trip`의 `GOLDEN_CHUNKS_HASH`를 갱신했습니다(ChunkId와 그것을 담는 manifest·ViewTree
  payload 변경, subtree payload 불변). 유도 golden 3개는 독립 구현(Python `hashlib`)으로 같은 값을 교차 확인했습니다.

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-project` | unit 168, `document_residency` 3, `document_round_trip` 5, `document_serialize` 5, `restricted_yaml_fuzz` 4 passed, 0 failed |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-project -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 같은 StableId에서 계열·문서가 다르면 ChunkId 6개 모두 다름, 원시 StableId 미사용, UUIDv8·variant, 유도 golden 3개, View·node id가 같은 두 문서의 chunk 집합 불교차(`chunk_ids_are_domain_separated_by_family_and_document`) | 통과 |
| 100개 golden round-trip·재직렬화 byte-identical·갱신된 golden hash, lazy 로드(`document_round_trip`) | 통과 |
| residency dirty 판정·unload/reload(`document_residency`, 새 subtree ChunkId 기대값) | 통과 |
