//! document → chunk model serializer (VI.10.2).
//!
//! 순서: 런타임 모델 → 영속 레코드(`to_records`, runtime 핸들 제거) → 전체 validator(VI.10.1)
//! → View별 subtree 분할 → chunk payload 인코딩. 검증에 실패한 문서는 chunk를 하나도 만들지
//! 않습니다. 결과는 입력 의미에만 의존합니다 — runtime 핸들 값·삽입 이력이 달라도 같은
//! bytes입니다.

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_document::context::set::ContextSet;
use stellar_document::model::{DocumentModel, ModelError};
use stellar_document::record_codec::PartitionError;
use stellar_document::records::DocumentRecords;
use stellar_document::validate::{validate_document, DocumentValidationError};
use stellar_storage::chunk::envelope::ChunkKindCode;
use stellar_storage::chunk::id::ChunkId;
use stellar_storage::chunk::kinds::{KIND_DOCUMENT_MANIFEST, KIND_VIEW_TREE};
use stellar_storage::chunk::planner::LogicalChunkState;
use stellar_storage::chunk::view_tree::{SubtreeRef, ViewTreeChunk, ViewTreeError};
use stellar_storage::hash::ContentHash;

use super::manifest::{DocumentManifestChunk, ManifestChunkError, ViewTreeRef};
use super::DOCUMENT_CHUNK_SCHEMA_VERSION;

/// 직렬화된 chunk 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerializedChunk {
    /// 논리 id.
    pub chunk_id: ChunkId,
    /// chunk kind.
    pub kind: ChunkKindCode,
    /// payload 스키마 version.
    pub schema_version: u32,
    /// canonical payload.
    pub payload: Vec<u8>,
}

/// 한 문서의 chunk model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerializedDocument {
    /// 문서 StableId.
    pub document_id: StableId,
    /// DocumentManifest chunk.
    pub manifest: SerializedChunk,
    /// View별 ViewTree chunk(정의 순서).
    pub view_trees: Vec<SerializedChunk>,
    /// subtree 내용 chunk(View 정의 순서, View 안에서는 subtree 순서).
    pub subtrees: Vec<SerializedChunk>,
    /// Component Context `context-set` chunk(XXX.7, ADR-0099 결정 2). 문서에 Context가 없으면 `None`.
    pub context: Option<SerializedChunk>,
}

impl SerializedDocument {
    /// 모든 chunk(manifest → ViewTree → subtree 순).
    pub fn chunks(&self) -> Vec<&SerializedChunk> {
        std::iter::once(&self.manifest)
            .chain(self.view_trees.iter())
            .chain(self.subtrees.iter())
            .chain(self.context.iter())
            .collect()
    }

    /// 저장 계획(V.5.1)에 넣을 dirty logical chunk 상태.
    pub fn to_dirty_states(&self) -> Vec<LogicalChunkState> {
        self.chunks()
            .into_iter()
            .map(|chunk| LogicalChunkState::Dirty {
                chunk_id: chunk.chunk_id,
                kind: chunk.kind,
                schema_version: chunk.schema_version,
                payload: chunk.payload.clone(),
            })
            .collect()
    }
}

/// 직렬화 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SerializeError {
    /// 런타임 모델 내부 불일치.
    Model(ModelError),
    /// 문서가 불변식을 위반합니다(chunk를 만들지 않음).
    Invalid(DocumentValidationError),
    /// subtree 분할 실패(데이터 손실 방지).
    Partition(PartitionError),
    /// ViewTree payload 인코딩 실패.
    ViewTree(ViewTreeError),
    /// DocumentManifest payload 인코딩 실패.
    Manifest(ManifestChunkError),
}

impl fmt::Display for SerializeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SerializeError::Model(e) => write!(f, "문서 모델: {e}"),
            SerializeError::Invalid(e) => write!(f, "{e}"),
            SerializeError::Partition(e) => write!(f, "subtree 분할: {e}"),
            SerializeError::ViewTree(e) => write!(f, "{e}"),
            SerializeError::Manifest(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SerializeError {}

/// 문서 ChunkId 유도의 domain 분리 prefix. 바꾸면 모든 문서 ChunkId가 바뀌는 저장 형식의
/// 일부입니다.
const CHUNK_ID_DOMAIN: &[u8] = b"stellar/document-chunk-id/v1\0";

/// 문서 chunk 계열(family) tag. 숫자 값은 저장 형식의 일부입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum ChunkFamily {
    /// DocumentManifest.
    Manifest = 1,
    /// View의 ViewTree.
    ViewTree = 2,
    /// subtree 내용.
    Subtree = 3,
    /// Component Context `context-set`(XXX.7).
    Context = 4,
}

/// `SHA-256(domain ‖ family ‖ 문서 id ‖ 객체 id)`의 앞 16바이트를 RFC 9562 UUIDv8(사용자
/// 정의) 형식으로 만든 ChunkId.
///
/// - **계열 분리**: family tag가 달라 manifest·ViewTree·subtree는 같은 StableId에서 유도돼도
///   서로 다른 ChunkId가 됩니다.
/// - **문서 분리**: 문서 StableId를 함께 넣어, 같은 View·node id가 다른 문서에 있어도(복제 등)
///   ChunkId가 겹치지 않습니다.
/// - **다른 도메인과 분리**: version nibble 8은 StableId(v7)를 그대로 쓰는 다른 ChunkId와 같은
///   값이 될 수 없습니다.
fn derive_chunk_id(family: ChunkFamily, document: StableId, object: StableId) -> ChunkId {
    let mut input = Vec::with_capacity(CHUNK_ID_DOMAIN.len() + 1 + 32);
    input.extend_from_slice(CHUNK_ID_DOMAIN);
    input.push(family as u8);
    input.extend_from_slice(&document.to_bytes());
    input.extend_from_slice(&object.to_bytes());
    let digest = ContentHash::compute(&input);
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest.digest()[..16]);
    bytes[6] = (bytes[6] & 0x0F) | 0x80; // version 8
    bytes[8] = (bytes[8] & 0x3F) | 0x80; // variant 0b10
    ChunkId::from_bytes(bytes)
}

/// DocumentManifest chunk의 논리 id(manifest 계열, 문서 StableId에서 유도).
pub fn manifest_chunk_id(document: StableId) -> ChunkId {
    derive_chunk_id(ChunkFamily::Manifest, document, document)
}

/// ViewTree chunk의 논리 id(ViewTree 계열, 문서·View StableId에서 유도).
pub fn view_tree_chunk_id(document: StableId, view: StableId) -> ChunkId {
    derive_chunk_id(ChunkFamily::ViewTree, document, view)
}

/// subtree chunk의 논리 id(subtree 계열, 문서·subtree root node StableId에서 유도).
pub fn subtree_chunk_id(document: StableId, root_node: StableId) -> ChunkId {
    derive_chunk_id(ChunkFamily::Subtree, document, root_node)
}

/// `context-set` chunk의 논리 id(Context 계열, 문서 StableId에서 유도).
pub fn context_chunk_id(document: StableId) -> ChunkId {
    derive_chunk_id(ChunkFamily::Context, document, document)
}

/// `context-set` chunk kind(IX.12.1 예약 `0x8000_0007`, required).
pub const KIND_CONTEXT_SET: ChunkKindCode =
    ChunkKindCode(stellar_core_types::reserved::context::CHUNK_KIND_CONTEXT_SET);

fn chunk(chunk_id: ChunkId, kind: ChunkKindCode, payload: Vec<u8>) -> SerializedChunk {
    SerializedChunk {
        chunk_id,
        kind,
        schema_version: DOCUMENT_CHUNK_SCHEMA_VERSION,
        payload,
    }
}

/// 런타임 문서 모델을 chunk model로 직렬화합니다.
///
/// 모델의 Component Context(살아 있는 provider의 것만, [`ContextSet::live`])가 비어 있지 않으면 `context-set` chunk를 더합니다.
pub fn serialize_document(model: &DocumentModel) -> Result<SerializedDocument, SerializeError> {
    let records = model.to_records().map_err(SerializeError::Model)?;
    serialize_records_with_context(&records, &model.context().live(model))
}

/// 영속 레코드를 검증한 뒤 chunk model로 직렬화합니다(Context 없음).
pub fn serialize_records(records: &DocumentRecords) -> Result<SerializedDocument, SerializeError> {
    serialize_records_with_context(records, &ContextSet::new())
}

/// 영속 레코드와 문서 Context를 chunk model로 직렬화합니다. Context가 비면 [`serialize_records`]와 같은 bytes입니다.
pub fn serialize_records_with_context(
    records: &DocumentRecords,
    context: &ContextSet,
) -> Result<SerializedDocument, SerializeError> {
    validate_document(records).map_err(SerializeError::Invalid)?;
    let document_id = records.definition.id;
    let mut view_trees = Vec::new();
    let mut subtrees = Vec::new();
    let mut refs = Vec::new();
    for (view, subtree) in records.view_subtrees().map_err(SerializeError::Partition)? {
        let subtree_id = subtree_chunk_id(document_id, subtree.root);
        let tree = ViewTreeChunk {
            view_root: subtree.root,
            subtrees: vec![SubtreeRef {
                root_node: subtree.root,
                chunk: subtree_id,
            }],
        };
        let tree_id = view_tree_chunk_id(document_id, view);
        view_trees.push(chunk(
            tree_id,
            KIND_VIEW_TREE,
            tree.encode().map_err(SerializeError::ViewTree)?,
        ));
        subtrees.push(chunk(subtree_id, KIND_VIEW_TREE, subtree.encode()));
        refs.push(ViewTreeRef {
            view,
            chunk: tree_id,
        });
    }
    let context_chunk = (!context.is_empty()).then(|| {
        chunk(
            context_chunk_id(document_id),
            KIND_CONTEXT_SET,
            context.encode(),
        )
    });
    let manifest = DocumentManifestChunk {
        definition: records.definition.clone(),
        view_trees: refs,
        context: context_chunk.as_ref().map(|c| c.chunk_id),
    };
    let version = manifest.schema_version();
    let mut manifest = chunk(
        manifest_chunk_id(document_id),
        KIND_DOCUMENT_MANIFEST,
        manifest.encode().map_err(SerializeError::Manifest)?,
    );
    manifest.schema_version = version;
    Ok(SerializedDocument {
        document_id,
        manifest,
        view_trees,
        subtrees,
        context: context_chunk,
    })
}
