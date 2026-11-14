//! 비활성 문서 chunk unload와 재로드 (XVI.5.5).
//!
//! 메모리 예산을 위해 **열려 있지 않은(비활성) 문서**의 node 데이터(ViewTree·subtree chunk에서
//! 만든 런타임 모델)를 내려놓고, 다시 필요할 때 storage lazy loader(V.6.6)로 그 문서의 chunk만
//! 읽어 복원합니다.
//!
//! 내려놓지 않는 것:
//!
//! - **문서 index**: generation을 열 때 읽은 DocumentManifest(문서·페이지·View 정의)는 별도로
//!   보관하며 unload 대상이 아닙니다. 문서 목록 조회에는 chunk 읽기가 필요 없습니다.
//! - **열린 문서**: [`OpenDocumentSet`](VI.1.4)에 열려 있는 문서.
//! - **저장되지 않은 dirty chunk가 있는 문서**: 현재 모델을 직렬화한 chunk 중 하나라도 기준
//!   generation의 content hash와 다르면 그 문서는 unload하지 않습니다. 판정은 flag가 아니라
//!   내용 비교라 편집 경로가 dirty 표시를 빠뜨려도 미저장 내용을 잃지 않습니다. 직렬화가
//!   실패하는 문서(편집 중 불변식 위반)도 보수적으로 dirty로 취급합니다.
//!
//! 재로드는 저장 bytes에서 모델을 다시 만들므로 unload 전과 의미가 같습니다(VI.10.3 round-trip).

use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_document::definitions::ProjectDocumentIndex;
use stellar_document::model::DocumentModel;
use stellar_document::open_set::OpenDocumentSet;
use stellar_storage::chunk::id::ChunkId;
use stellar_storage::generation::lazy::{ChunkReadRecorder, LazyGeneration};
use stellar_storage::hash::ContentHash;

use super::load::{load_document_from_generation, LoadDocumentError};
use super::manifest::{DocumentManifestChunk, ManifestChunkError};
use super::serialize::serialize_document;

/// residency 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResidencyError {
    /// generation의 DocumentManifest를 해석하지 못했습니다.
    Manifest {
        /// manifest chunk.
        chunk: ChunkId,
        /// 원인.
        error: ManifestChunkError,
    },
    /// index에 없는 문서입니다.
    UnknownDocument {
        /// 요청 문서.
        document: StableId,
    },
    /// 문서 로드 실패.
    Load(LoadDocumentError),
}

impl fmt::Display for ResidencyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResidencyError::Manifest { chunk, error } => {
                write!(f, "DocumentManifest {chunk}: {error}")
            }
            ResidencyError::UnknownDocument { document } => {
                write!(f, "문서 index에 없는 문서입니다: {document}")
            }
            ResidencyError::Load(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ResidencyError {}

/// 문서의 dirty 판정.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirtyState {
    /// 모든 chunk가 기준 generation과 같습니다(unload 가능).
    Clean,
    /// 기준 generation과 다른 chunk들(ChunkId 순).
    Dirty(Vec<ChunkId>),
    /// 현재 모델을 직렬화할 수 없습니다(편집 중 불변식 위반 등) — unload하지 않습니다.
    Unserializable,
}

/// [`ResidentDocuments::unload_inactive`] 결과.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UnloadReport {
    /// 내려놓은 문서.
    pub unloaded: Vec<StableId>,
    /// 열려 있어 유지한 문서.
    pub kept_active: Vec<StableId>,
    /// 미저장 변경이 있어 유지한 문서와 그 판정.
    pub kept_dirty: Vec<(StableId, DirtyState)>,
}

/// 문서 index와 메모리에 올라온 문서 모델.
#[derive(Debug)]
pub struct ResidentDocuments {
    /// 문서 index: generation manifest 순서의 DocumentManifest. unload하지 않습니다.
    index: Vec<DocumentManifestChunk>,
    /// 기준 generation의 ChunkId → content hash.
    saved: BTreeMap<ChunkId, ContentHash>,
    /// 메모리에 올라온 문서.
    resident: BTreeMap<StableId, DocumentModel>,
}

fn read_index(
    lazy: &LazyGeneration,
) -> Result<(Vec<DocumentManifestChunk>, BTreeMap<ChunkId, ContentHash>), ResidencyError> {
    let mut index = Vec::new();
    for (pointer, payload) in lazy.document_manifest_records() {
        let manifest = DocumentManifestChunk::decode_for_schema(pointer.schema_version, payload)
            .map_err(|error| ResidencyError::Manifest {
                chunk: pointer.chunk_id,
                error,
            })?;
        index.push(manifest);
    }
    let saved = lazy
        .manifest()
        .chunk_pointers
        .iter()
        .map(|p| (p.chunk_id, p.content_hash))
        .collect();
    Ok((index, saved))
}

impl ResidentDocuments {
    /// 열린 generation에서 문서 index만 만듭니다. node chunk는 읽지 않습니다(열기 때 이미 읽힌
    /// DocumentManifest만 사용).
    pub fn open(lazy: &LazyGeneration) -> Result<ResidentDocuments, ResidencyError> {
        let (index, saved) = read_index(lazy)?;
        Ok(ResidentDocuments {
            index,
            saved,
            resident: BTreeMap::new(),
        })
    }

    /// 저장 후 새로 연 generation을 기준으로 삼습니다. index와 기준 hash를 바꾸고, 메모리의
    /// 모델은 그대로 둡니다(새 기준과 같으면 이후 clean으로 판정됩니다).
    pub fn adopt_generation(&mut self, lazy: &LazyGeneration) -> Result<(), ResidencyError> {
        let (index, saved) = read_index(lazy)?;
        self.index = index;
        self.saved = saved;
        Ok(())
    }

    /// 문서 index(정의). 메모리 residency와 무관하게 항상 사용할 수 있습니다.
    pub fn document_index(&self) -> ProjectDocumentIndex {
        ProjectDocumentIndex {
            documents: self.index.iter().map(|m| m.definition.clone()).collect(),
        }
    }

    /// index에 있는 문서인지.
    pub fn contains(&self, document: StableId) -> bool {
        self.index.iter().any(|m| m.definition.id == document)
    }

    /// 문서가 메모리에 올라와 있는지.
    pub fn is_resident(&self, document: StableId) -> bool {
        self.resident.contains_key(&document)
    }

    /// 메모리에 올라온 문서 수.
    pub fn resident_count(&self) -> usize {
        self.resident.len()
    }

    /// 메모리에 올라온 문서(읽기).
    pub fn document(&self, document: StableId) -> Option<&DocumentModel> {
        self.resident.get(&document)
    }

    /// 메모리에 올라온 문서(편집). dirty 판정은 내용 비교라 별도 표시가 필요 없습니다.
    pub fn document_mut(&mut self, document: StableId) -> Option<&mut DocumentModel> {
        self.resident.get_mut(&document)
    }

    /// 문서를 메모리에 올립니다. 이미 있으면 chunk를 읽지 않고, 없으면 그 문서의 ViewTree·
    /// subtree chunk만 lazy loader로 읽어 복원합니다.
    pub fn ensure_loaded(
        &mut self,
        lazy: &LazyGeneration,
        document: StableId,
        recorder: &mut dyn ChunkReadRecorder,
    ) -> Result<&mut DocumentModel, ResidencyError> {
        if !self.contains(document) {
            return Err(ResidencyError::UnknownDocument { document });
        }
        match self.resident.entry(document) {
            Entry::Occupied(entry) => Ok(entry.into_mut()),
            Entry::Vacant(entry) => {
                let loaded = load_document_from_generation(lazy, document, recorder)
                    .map_err(ResidencyError::Load)?;
                Ok(entry.insert(loaded.model))
            }
        }
    }

    /// 메모리 문서의 dirty 판정. 메모리에 없으면 `None`.
    pub fn dirty_state(&self, document: StableId) -> Option<DirtyState> {
        let model = self.resident.get(&document)?;
        let Ok(serialized) = serialize_document(model) else {
            return Some(DirtyState::Unserializable);
        };
        let dirty: Vec<ChunkId> = serialized
            .chunks()
            .into_iter()
            .filter(|chunk| {
                self.saved.get(&chunk.chunk_id) != Some(&ContentHash::compute(&chunk.payload))
            })
            .map(|chunk| chunk.chunk_id)
            .collect();
        Some(if dirty.is_empty() {
            DirtyState::Clean
        } else {
            let mut dirty = dirty;
            dirty.sort();
            DirtyState::Dirty(dirty)
        })
    }

    /// 열려 있지 않고 미저장 변경이 없는 문서를 메모리에서 내립니다. 문서 index는 건드리지
    /// 않습니다.
    pub fn unload_inactive(&mut self, open: &OpenDocumentSet) -> UnloadReport {
        let mut report = UnloadReport::default();
        let documents: Vec<StableId> = self.resident.keys().copied().collect();
        for document in documents {
            if open.is_open(document) {
                report.kept_active.push(document);
                continue;
            }
            match self.dirty_state(document) {
                Some(DirtyState::Clean) => {
                    self.resident.remove(&document);
                    report.unloaded.push(document);
                }
                Some(state) => report.kept_dirty.push((document, state)),
                None => {}
            }
        }
        report
    }
}
