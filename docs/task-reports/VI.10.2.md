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
