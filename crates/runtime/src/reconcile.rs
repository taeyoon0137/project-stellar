//! 열린 창의 외부 변경 semantic reconciliation (XIII.3.4, ADR-0110 결정 1·2).
//!
//! 미저장 편집이 있는 창의 base를 외부 writer(CLI·MCP·Git·다른 프로세스)가 바꾸면, 창 session은
//! 세 문서를 비교합니다(아키텍처 §22.5).
//!
//! | 이름 | 읽는 곳 |
//! |---|---|
//! | LoadedBase | 창이 마지막으로 읽거나 쓴 generation(`loaded`)에서 이 창 문서만 다시 읽음 |
//! | CurrentMemory | 창의 메모리 문서 |
//! | ExternalDisk | 디스크의 새 원본·cache(`read_disk_state`, 열기와 같은 경로) |
//!
//! 병합 판정과 역투영은 [`stellar_ops::merge_apply::document::merge_document`] 하나입니다(새 병합 규칙 없음). 결과:
//!
//! - **쓸 수 있는 결과**: 창 session이 결과를 `ApplyExternalSnapshot` transaction 하나로 history에 기록하고 base를
//!   ExternalDisk로 바꿉니다. Undo 한 번이 병합 반영 전체를 되돌립니다. 결과가 디스크 문서와 같으면 clean, 다르면
//!   dirty로 남아 자동 저장이 그 결과를 씁니다(자동 저장 재개).
//! - **겹침·결과 위반·병합 불가**: 문서는 그대로 두고 [`ExternalConflict`]를 기록합니다. 그 디스크 관찰값에 한해
//!   stale 다음 할 일은 두 선택지(`StaleResolution::Choose`)입니다. 사용자가 UseCurrentWindow·UseLatestDisk를
//!   고릅니다.
//!
//! LoadedBase를 믿을 수 없으면(텍스트 원본에서 연 뒤 cache를 아직 맞춰 쓰지 않음, generation이 지워짐 등) 병합하지
//! 않고 [`ReconcileBlock::BaseUnavailable`]입니다. 잘못된 base로 병합하면 로컬 편집이나 외부 변경을 잃기 때문입니다.

use std::collections::BTreeMap;
use std::path::Path;

use stellar_document::model::DocumentModel;
use stellar_ops::diff::DocumentSource;
use stellar_project::document_io::load::{load_document, ChunkSource, ChunkSourceError};
use stellar_project::document_io::manifest::DocumentManifestChunk;
use stellar_storage::chunk::id::ChunkId;
use stellar_storage::chunk::kinds::KIND_DOCUMENT_MANIFEST;
use stellar_storage::chunk::pointer::ChunkPointer;
use stellar_storage::chunk::store::ChunkStore;
use stellar_storage::generation::loader::LoadedGeneration;

use stellar_core_types::ids::StableId;
pub use stellar_ops::compare::ConflictEntry;
use stellar_ops::merge_apply::document::{merge_document, DocumentMerge};

/// 자동 적용하지 않은 이유.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcileBlock {
    /// 겹치는 변경이 있거나 병합 결과가 새 snapshot 위반을 만듭니다.
    Conflicts,
    /// LoadedBase를 읽을 수 없거나 믿을 수 없습니다.
    BaseUnavailable(String),
    /// 외부 변경이 이 창 문서를 지웠습니다.
    DocumentRemoved,
    /// 무손실 병합을 증명할 수 없습니다(snapshot 왕복 손실 등).
    Unsupported(String),
}

impl ReconcileBlock {
    /// 안정 code.
    pub fn code(&self) -> &'static str {
        match self {
            ReconcileBlock::Conflicts => "conflicts",
            ReconcileBlock::BaseUnavailable(_) => "base-unavailable",
            ReconcileBlock::DocumentRemoved => "document-removed",
            ReconcileBlock::Unsupported(_) => "unsupported",
        }
    }
}

/// 자동 적용하지 않은 외부 변경 병합. 창 문서·history는 그대로이고 두 선택지를 기다립니다.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalConflict {
    /// 이유.
    pub reason: ReconcileBlock,
    /// 겹친 변경(엔진 순서, [`ReconcileBlock::Conflicts`]일 때).
    pub conflicts: Vec<ConflictEntry>,
    /// 병합 결과가 새로 만든 snapshot 위반 수.
    pub violations: usize,
}

impl ExternalConflict {
    fn blocked(reason: ReconcileBlock) -> ExternalConflict {
        ExternalConflict {
            reason,
            conflicts: Vec::new(),
            violations: 0,
        }
    }
}

/// 창 문서 병합 결과.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum WindowMerge {
    /// 쓸 수 있는 결과.
    Merged {
        /// 결과 문서 레코드와 Component Context(ADR-0114 결정 2).
        document: Box<DocumentSource>,
        /// 결과가 ExternalDisk 문서와 같은지(같으면 반영 뒤 clean).
        matches_disk: bool,
    },
    /// 자동 적용하지 않습니다.
    Blocked(ExternalConflict),
}

/// 한 generation의 chunk만 읽는 원천(pointer가 가리키는 content-addressed 파일, 전체 검증).
struct GenerationChunks<'a> {
    store: ChunkStore,
    pointers: BTreeMap<ChunkId, &'a ChunkPointer>,
}

impl ChunkSource for GenerationChunks<'_> {
    fn read_chunk(&mut self, chunk: ChunkId) -> Result<Vec<u8>, ChunkSourceError> {
        let pointer = self.pointers.get(&chunk).ok_or(ChunkSourceError {
            chunk,
            message: "generation에 없는 chunk입니다".into(),
        })?;
        self.store
            .read_chunk(&pointer.content_hash, pointer.kind)
            .map_err(|e| ChunkSourceError {
                chunk,
                message: e.to_string(),
            })
    }
}

/// LoadedBase: `loaded` generation에서 문서 `document`(레코드와 Context)를 다시 읽습니다. current pointer와 관계없이
/// 그 generation의 manifest가 가리키는 chunk만 읽습니다(content-addressed라 불변). 읽지 못하면 이유 문장입니다.
pub(crate) fn base_records(
    root: &Path,
    loaded: &LoadedGeneration,
    document: StableId,
) -> Result<DocumentSource, String> {
    let mut chunks = GenerationChunks {
        store: ChunkStore::new(root),
        pointers: loaded
            .manifest
            .chunk_pointers
            .iter()
            .map(|p| (p.chunk_id, p))
            .collect(),
    };
    for pointer in &loaded.manifest.chunk_pointers {
        if pointer.kind != KIND_DOCUMENT_MANIFEST {
            continue;
        }
        let payload = chunks.read_chunk(pointer.chunk_id).map_err(|e| e.message)?;
        let manifest = DocumentManifestChunk::decode_for_schema(pointer.schema_version, &payload)
            .map_err(|e| format!("{e:?}"))?;
        if manifest.definition.id != document {
            continue;
        }
        let loaded = load_document(&payload, &mut chunks).map_err(|e| e.to_string())?;
        return DocumentSource::of_model(&loaded.model).map_err(|e| e.to_string());
    }
    Err("base generation에 이 문서가 없습니다".into())
}

/// LoadedBase·CurrentMemory·ExternalDisk를 병합합니다(모듈 문서). `base`는 [`base_records`] 결과, `theirs`가 `None`이면
/// 외부 변경이 문서를 지웠습니다.
pub(crate) fn merge_window(
    base: Result<DocumentSource, String>,
    ours: &DocumentModel,
    theirs: Option<&DocumentModel>,
) -> WindowMerge {
    let Some(theirs) = theirs else {
        return WindowMerge::Blocked(ExternalConflict::blocked(ReconcileBlock::DocumentRemoved));
    };
    let base = match base {
        Ok(base) => base,
        Err(reason) => {
            return WindowMerge::Blocked(ExternalConflict::blocked(
                ReconcileBlock::BaseUnavailable(reason),
            ))
        }
    };
    let unsupported =
        |e: String| WindowMerge::Blocked(ExternalConflict::blocked(ReconcileBlock::Unsupported(e)));
    let ours = match DocumentSource::of_model(ours) {
        Ok(document) => document,
        Err(e) => return unsupported(e.to_string()),
    };
    let theirs = match DocumentSource::of_model(theirs) {
        Ok(document) => document,
        Err(e) => return unsupported(e.to_string()),
    };
    match merge_document(&base, &ours, &theirs) {
        Ok(DocumentMerge::Merged { document, .. }) => WindowMerge::Merged {
            matches_disk: *document == theirs,
            document,
        },
        Ok(DocumentMerge::Conflicted {
            conflicts,
            violations,
        }) => WindowMerge::Blocked(ExternalConflict {
            reason: ReconcileBlock::Conflicts,
            conflicts,
            violations,
        }),
        Err(e) => unsupported(e.to_string()),
    }
}

#[cfg(test)]
mod tests;
