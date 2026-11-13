//! 문서 ↔ chunk model 입출력 (VI.10.2·VI.10.3·XVI.5.5).
//!
//! document crate는 storage에 의존하지 않으므로, 문서 영속 레코드(VI.10)를 storage의 chunk
//! 단위(V.6.1 kind, V.6.2 ViewTree)로 옮기는 매핑은 둘 다 의존할 수 있는 이 crate가
//! 담당합니다.
//!
//! 한 문서의 chunk model:
//!
//! | chunk | kind | 논리 ChunkId | payload |
//! |---|---|---|---|
//! | DocumentManifest | `KIND_DOCUMENT_MANIFEST` | 문서 StableId | 문서 정의 + View별 ViewTree ChunkId ([`manifest`]) |
//! | ViewTree | `KIND_VIEW_TREE` | View StableId | storage `ViewTreeChunk`(root Frame + subtree 참조) |
//! | subtree | `KIND_VIEW_TREE` | subtree root node StableId | document `SubtreeRecords`(node 레코드 + modifier stack) |
//!
//! subtree 내용 chunk는 별도 kind 없이 ViewTree 계열 kind를 씁니다(V.6.2 lazy loader와 같은
//! 관례). 어떤 payload로 해석할지는 참조 경로(manifest → ViewTree → subtree)가 정합니다.
//!
//! **ChunkId 유도**: 각 chunk의 논리 id는 그 chunk가 담는 의미 객체의 StableId에서 유도합니다.
//! 같은 문서는 언제나 같은 ChunkId 집합을 만들므로 바뀐 chunk만 새 hash가 됩니다(V.3.3).
//! validator(VI.10.1)가 문서 안 id 유일성을 보장하므로 한 문서의 ChunkId는 서로 다릅니다.
//! loader는 유도 규칙을 가정하지 않고 저장된 참조만 따릅니다.
//!
//! **runtime ID 비저장**: 직렬화는 [`DocumentModel::to_records`](stellar_document::model::DocumentModel::to_records)가
//! 만든 StableId 레코드만 인코딩합니다. runtime 핸들(`RuntimeEntityId`)은 레코드 타입에
//! 존재하지 않습니다.

/// DocumentManifest chunk payload (VI.10.2).
pub mod manifest;

/// document → chunk model serializer (VI.10.2).
pub mod serialize;

/// chunk model → document loader (VI.10.3).
pub mod load;

/// 비활성 문서 chunk unload와 재로드 (XVI.5.5).
pub mod residency;

/// 문서 chunk payload 스키마 version. chunk pointer(V.3.1)에 기록됩니다.
pub const DOCUMENT_CHUNK_SCHEMA_VERSION: u32 = 1;
