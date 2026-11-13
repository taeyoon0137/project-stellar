//! document → chunk model serializer (VI.10.2).
//!
//! 순서: 런타임 모델 → 영속 레코드(`to_records`, runtime 핸들 제거) → 전체 validator(VI.10.1)
//! → View별 subtree 분할 → chunk payload 인코딩. 검증에 실패한 문서는 chunk를 하나도 만들지
//! 않습니다. 결과는 입력 의미에만 의존합니다 — runtime 핸들 값·삽입 이력이 달라도 같은
//! bytes입니다.

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_document::model::{DocumentModel, ModelError};
use stellar_document::record_codec::PartitionError;
use stellar_document::records::DocumentRecords;
use stellar_document::validate::{validate_document, DocumentValidationError};
use stellar_storage::chunk::envelope::ChunkKindCode;
use stellar_storage::chunk::id::ChunkId;
use stellar_storage::chunk::kinds::{KIND_DOCUMENT_MANIFEST, KIND_VIEW_TREE};
use stellar_storage::chunk::planner::LogicalChunkState;
use stellar_storage::chunk::view_tree::{SubtreeRef, ViewTreeChunk, ViewTreeError};

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
}

impl SerializedDocument {
    /// 모든 chunk(manifest → ViewTree → subtree 순).
    pub fn chunks(&self) -> Vec<&SerializedChunk> {
        std::iter::once(&self.manifest)
            .chain(self.view_trees.iter())
            .chain(self.subtrees.iter())
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

/// DocumentManifest chunk의 논리 id(문서 StableId에서 유도).
pub fn manifest_chunk_id(document: StableId) -> ChunkId {
    ChunkId::from_stable_id(document)
}

/// ViewTree chunk의 논리 id(View StableId에서 유도).
pub fn view_tree_chunk_id(view: StableId) -> ChunkId {
    ChunkId::from_stable_id(view)
}

/// subtree chunk의 논리 id(subtree root node StableId에서 유도).
pub fn subtree_chunk_id(root_node: StableId) -> ChunkId {
    ChunkId::from_stable_id(root_node)
}

fn chunk(chunk_id: ChunkId, kind: ChunkKindCode, payload: Vec<u8>) -> SerializedChunk {
    SerializedChunk {
        chunk_id,
        kind,
        schema_version: DOCUMENT_CHUNK_SCHEMA_VERSION,
        payload,
    }
}

/// 런타임 문서 모델을 chunk model로 직렬화합니다.
pub fn serialize_document(model: &DocumentModel) -> Result<SerializedDocument, SerializeError> {
    let records = model.to_records().map_err(SerializeError::Model)?;
    serialize_records(&records)
}

/// 영속 레코드를 검증한 뒤 chunk model로 직렬화합니다.
pub fn serialize_records(records: &DocumentRecords) -> Result<SerializedDocument, SerializeError> {
    validate_document(records).map_err(SerializeError::Invalid)?;
    let document_id = records.definition.id;
    let mut view_trees = Vec::new();
    let mut subtrees = Vec::new();
    let mut refs = Vec::new();
    for (view, subtree) in records.view_subtrees().map_err(SerializeError::Partition)? {
        let subtree_id = subtree_chunk_id(subtree.root);
        let tree = ViewTreeChunk {
            view_root: subtree.root,
            subtrees: vec![SubtreeRef {
                root_node: subtree.root,
                chunk: subtree_id,
            }],
        };
        let tree_id = view_tree_chunk_id(view);
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
    let manifest = DocumentManifestChunk {
        definition: records.definition.clone(),
        view_trees: refs,
    }
    .encode()
    .map_err(SerializeError::Manifest)?;
    Ok(SerializedDocument {
        document_id,
        manifest: chunk(
            manifest_chunk_id(document_id),
            KIND_DOCUMENT_MANIFEST,
            manifest,
        ),
        view_trees,
        subtrees,
    })
}
