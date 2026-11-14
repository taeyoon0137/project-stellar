//! chunk model → document loader (VI.10.3).
//!
//! DocumentManifest payload에서 시작해 저장된 참조(manifest → ViewTree → subtree)만 따라가며
//! chunk를 읽고, 레코드를 합친 뒤 [`DocumentModel::from_records`]로 검증·재구축합니다(ID
//! registry와 부모·자식 순서). ChunkId 유도 규칙은 가정하지 않습니다.
//!
//! chunk 읽기는 [`ChunkSource`]로 추상화합니다. 저장된 프로젝트는 storage lazy loader
//! (V.6.6)를 감싼 [`LazySource`]로 요청한 문서의 chunk만 읽습니다.

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_document::model::{DocumentModel, LoadModelError};
use stellar_document::record_codec::{RecordCodecError, SubtreeRecords};
use stellar_document::records::DocumentRecords;
use stellar_document::validate::DocumentDiagnostics;
use stellar_storage::chunk::id::ChunkId;
use stellar_storage::chunk::view_tree::{ViewTreeChunk, ViewTreeError};
use stellar_storage::generation::lazy::{ChunkReadRecorder, LazyGeneration};

use super::manifest::{DocumentManifestChunk, ManifestChunkError};
use super::serialize::SerializedDocument;

/// chunk 하나를 읽지 못했습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkSourceError {
    /// 요청 chunk.
    pub chunk: ChunkId,
    /// 원인 설명.
    pub message: String,
}

/// 논리 ChunkId로 chunk payload를 읽는 원천.
pub trait ChunkSource {
    /// `chunk`의 payload를 읽습니다.
    fn read_chunk(&mut self, chunk: ChunkId) -> Result<Vec<u8>, ChunkSourceError>;
}

/// 메모리 chunk 집합(직렬화 결과 round-trip·테스트 용).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryChunks {
    chunks: BTreeMap<ChunkId, Vec<u8>>,
}

impl MemoryChunks {
    /// 빈 집합.
    pub fn new() -> MemoryChunks {
        MemoryChunks::default()
    }

    /// 직렬화된 문서의 모든 chunk를 넣습니다.
    pub fn insert_document(&mut self, document: &SerializedDocument) {
        for chunk in document.chunks() {
            self.chunks.insert(chunk.chunk_id, chunk.payload.clone());
        }
    }

    /// chunk 하나를 넣거나 바꿉니다.
    pub fn insert(&mut self, chunk: ChunkId, payload: Vec<u8>) {
        self.chunks.insert(chunk, payload);
    }
}

impl ChunkSource for MemoryChunks {
    fn read_chunk(&mut self, chunk: ChunkId) -> Result<Vec<u8>, ChunkSourceError> {
        self.chunks.get(&chunk).cloned().ok_or(ChunkSourceError {
            chunk,
            message: "chunk가 없습니다".to_string(),
        })
    }
}

/// storage lazy loader(V.6.6)를 감싼 chunk 원천. 모든 읽기가 recorder를 지납니다.
pub struct LazySource<'a> {
    lazy: &'a LazyGeneration,
    recorder: &'a mut dyn ChunkReadRecorder,
}

impl<'a> LazySource<'a> {
    /// lazy generation과 계측 recorder로 만듭니다.
    pub fn new(
        lazy: &'a LazyGeneration,
        recorder: &'a mut dyn ChunkReadRecorder,
    ) -> LazySource<'a> {
        LazySource { lazy, recorder }
    }
}

impl ChunkSource for LazySource<'_> {
    fn read_chunk(&mut self, chunk: ChunkId) -> Result<Vec<u8>, ChunkSourceError> {
        check_document_chunk_schema(self.lazy, chunk)?;
        self.lazy
            .read_chunk_payload(chunk, self.recorder)
            .map_err(|e| ChunkSourceError {
                chunk,
                message: e.to_string(),
            })
    }
}

fn check_document_chunk_schema(
    lazy: &LazyGeneration,
    chunk: ChunkId,
) -> Result<(), ChunkSourceError> {
    if let Some(pointer) = lazy.chunk_pointer(chunk) {
        if !(super::DOCUMENT_CHUNK_SCHEMA_VERSION..=super::INSTANCE_SUBTREE_SCHEMA_VERSION)
            .contains(&pointer.schema_version)
        {
            return Err(ChunkSourceError {
                chunk,
                message: format!("지원하지 않는 문서 chunk schema {}", pointer.schema_version),
            });
        }
    }
    Ok(())
}

/// 문서 로드 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadDocumentError {
    /// DocumentManifest payload 오류.
    Manifest(ManifestChunkError),
    /// 요청 문서의 DocumentManifest가 없습니다.
    UnknownDocument {
        /// 요청 문서.
        document: StableId,
    },
    /// chunk를 읽지 못했습니다.
    Read(ChunkSourceError),
    /// ViewTree payload 오류.
    ViewTree {
        /// View.
        view: StableId,
        /// 원인.
        error: ViewTreeError,
    },
    /// ViewTree의 root가 View 정의의 root Frame과 다릅니다.
    ViewRootMismatch {
        /// View.
        view: StableId,
        /// 정의의 root.
        expected: StableId,
        /// ViewTree의 root.
        found: StableId,
    },
    /// subtree payload 오류.
    Subtree {
        /// subtree chunk.
        chunk: ChunkId,
        /// 원인.
        error: RecordCodecError,
    },
    /// subtree chunk의 root가 ViewTree가 가리킨 root와 다릅니다.
    SubtreeRootMismatch {
        /// subtree chunk.
        chunk: ChunkId,
        /// ViewTree의 subtree root.
        expected: StableId,
        /// chunk에 기록된 root.
        found: StableId,
    },
    /// 합친 레코드가 문서 불변식을 위반하거나 모델을 만들지 못했습니다.
    Model(LoadModelError),
}

impl fmt::Display for LoadDocumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadDocumentError::Manifest(e) => write!(f, "{e}"),
            LoadDocumentError::UnknownDocument { document } => {
                write!(f, "문서 {document}의 DocumentManifest가 없습니다")
            }
            LoadDocumentError::Read(e) => write!(f, "chunk {} 읽기 실패: {}", e.chunk, e.message),
            LoadDocumentError::ViewTree { view, error } => write!(f, "View {view}: {error}"),
            LoadDocumentError::ViewRootMismatch {
                view,
                expected,
                found,
            } => write!(
                f,
                "View {view}: ViewTree root {found}가 정의의 root {expected}와 다릅니다"
            ),
            LoadDocumentError::Subtree { chunk, error } => {
                write!(f, "subtree chunk {chunk}: {error}")
            }
            LoadDocumentError::SubtreeRootMismatch {
                chunk,
                expected,
                found,
            } => write!(
                f,
                "subtree chunk {chunk}: root {found}가 ViewTree의 {expected}와 다릅니다"
            ),
            LoadDocumentError::Model(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for LoadDocumentError {}

/// 로드된 문서.
#[derive(Debug)]
pub struct LoadedDocument {
    /// 재구축된 런타임 모델.
    pub model: DocumentModel,
    /// 미해결 참조 진단(문서는 유효).
    pub diagnostics: DocumentDiagnostics,
}

/// DocumentManifest payload에서 문서를 로드합니다.
pub fn load_document(
    manifest_payload: &[u8],
    source: &mut dyn ChunkSource,
) -> Result<LoadedDocument, LoadDocumentError> {
    let manifest =
        DocumentManifestChunk::decode(manifest_payload).map_err(LoadDocumentError::Manifest)?;
    let roots: BTreeMap<StableId, StableId> = manifest
        .definition
        .pages
        .iter()
        .flat_map(|p| p.views.iter().map(|v| (v.id, v.root_frame)))
        .collect();
    let mut subtrees = Vec::new();
    for entry in &manifest.view_trees {
        let payload = source
            .read_chunk(entry.chunk)
            .map_err(LoadDocumentError::Read)?;
        let tree =
            ViewTreeChunk::decode(&payload).map_err(|error| LoadDocumentError::ViewTree {
                view: entry.view,
                error,
            })?;
        // manifest.validate()가 View 항목과 정의의 1:1을 보장합니다.
        let expected = roots.get(&entry.view).copied().unwrap_or(StableId::NIL);
        if tree.view_root != expected {
            return Err(LoadDocumentError::ViewRootMismatch {
                view: entry.view,
                expected,
                found: tree.view_root,
            });
        }
        for subtree_ref in &tree.subtrees {
            let payload = source
                .read_chunk(subtree_ref.chunk)
                .map_err(LoadDocumentError::Read)?;
            let subtree =
                SubtreeRecords::decode(&payload).map_err(|error| LoadDocumentError::Subtree {
                    chunk: subtree_ref.chunk,
                    error,
                })?;
            if subtree.root != subtree_ref.root_node {
                return Err(LoadDocumentError::SubtreeRootMismatch {
                    chunk: subtree_ref.chunk,
                    expected: subtree_ref.root_node,
                    found: subtree.root,
                });
            }
            subtrees.push(subtree);
        }
    }
    let records = DocumentRecords::from_subtrees(manifest.definition, subtrees);
    let (model, diagnostics) =
        DocumentModel::from_records(records).map_err(LoadDocumentError::Model)?;
    Ok(LoadedDocument { model, diagnostics })
}

/// lazy generation에서 `document`를 로드합니다. 열기 때 이미 읽은 DocumentManifest를 쓰고,
/// 그 문서의 ViewTree·subtree chunk만 추가로 읽습니다(다른 문서의 chunk는 읽지 않음).
pub fn load_document_from_generation(
    lazy: &LazyGeneration,
    document: StableId,
    recorder: &mut dyn ChunkReadRecorder,
) -> Result<LoadedDocument, LoadDocumentError> {
    let payload = find_document_manifest(lazy, document)?;
    let mut source = LazySource::new(lazy, recorder);
    load_document(payload, &mut source)
}

/// 열린 generation에서 `document`의 DocumentManifest payload를 찾습니다.
pub fn find_document_manifest(
    lazy: &LazyGeneration,
    document: StableId,
) -> Result<&[u8], LoadDocumentError> {
    for (pointer, payload) in lazy.document_manifest_records() {
        let manifest = DocumentManifestChunk::decode_for_schema(pointer.schema_version, payload)
            .map_err(LoadDocumentError::Manifest)?;
        if manifest.definition.id == document {
            return Ok(payload);
        }
    }
    Err(LoadDocumentError::UnknownDocument { document })
}
