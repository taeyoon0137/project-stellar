//! opt-in 단일 View 텍스트 원본 export·읽기 전용 검사 (ADR-0059).
//!
//! 기존 generation을 변경하지 않습니다. export는 재직렬화한 문서의 chunk 집합이 저장된
//! generation 전체와 정확히 같을 때만 새 파일을 만듭니다. 미지원 chunk를 버리지 않습니다.

use std::fmt;
use std::io::ErrorKind;
use std::path::Path;

use stellar_project::document_io::serialize::serialize_document;
use stellar_project::document_io::text_source::{self, TextSourceError};
use stellar_storage::chunk::pointer::ChunkPointer;
use stellar_storage::hash::ContentHash;

use crate::error::{DocumentReadError, OpenError};
use crate::snapshot::open_snapshot;

/// export·검사 결과. 경로와 프로젝트 내용은 포함하지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextSourceOutcome {
    /// 파일의 실제 텍스트 bytes hash.
    pub source_hash: ContentHash,
    /// 검증한 문서의 node 수.
    pub node_count: usize,
}

/// 텍스트 원본 작업 실패.
#[derive(Debug)]
pub enum TextSourceOperationError {
    /// 입력 프로젝트 열기 실패.
    Open(OpenError),
    /// 저장 문서 읽기 실패.
    Document(DocumentReadError),
    /// 지원 범위 밖의 프로젝트·문서 의미.
    Unsupported(String),
    /// 문서 또는 텍스트 형식 오류.
    Invalid(String),
    /// 작업 중 파일이 바뀌었습니다.
    Changed,
    /// 파일 접근 실패. 경로는 출력 계층이 별도로 다룹니다.
    Io(ErrorKind),
}

impl fmt::Display for TextSourceOperationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open(error) => write!(f, "{error}"),
            Self::Document(error) => write!(f, "{error}"),
            Self::Unsupported(message) => write!(f, "지원하지 않는 텍스트 원본 의미: {message}"),
            Self::Invalid(message) => write!(f, "텍스트 원본 검증: {message}"),
            Self::Changed => f.write_str("작업 중 텍스트 원본이 변경되었습니다"),
            Self::Io(kind) => write!(f, "텍스트 원본 I/O: {kind:?}"),
        }
    }
}

impl std::error::Error for TextSourceOperationError {}

impl From<TextSourceError> for TextSourceOperationError {
    fn from(error: TextSourceError) -> Self {
        match error {
            TextSourceError::Invalid(message) => Self::Invalid(message),
            TextSourceError::Unsupported(message) => Self::Unsupported(message),
            TextSourceError::Changed => Self::Changed,
            TextSourceError::Io(error) => Self::Io(error.kind()),
        }
    }
}

/// 저장된 단일 문서·page·View 프로젝트를 새 텍스트 파일로 내보냅니다.
/// 기존 출력 파일은 덮어쓰지 않습니다.
pub fn export(
    project: &Path,
    output: &Path,
) -> Result<TextSourceOutcome, TextSourceOperationError> {
    let snapshot = open_snapshot(project).map_err(TextSourceOperationError::Open)?;
    let generation = snapshot.generation().ok_or_else(|| {
        TextSourceOperationError::Unsupported("저장된 generation이 필요합니다".into())
    })?;
    let [document] = snapshot.documents() else {
        return Err(TextSourceOperationError::Unsupported(
            "정확히 한 문서가 필요합니다".into(),
        ));
    };
    let manifest = generation.manifest();
    if manifest.schema_version != crate::migrate::CURRENT_SCHEMA_VERSION
        || manifest.root_document_ids.as_slice() != [document.id]
    {
        return Err(TextSourceOperationError::Unsupported(
            "지원하는 generation 스키마와 단일 root 문서가 필요합니다".into(),
        ));
    }
    let model = snapshot
        .document_model(document.id)
        .map_err(TextSourceOperationError::Document)?;
    let serialized = serialize_document(&model)
        .map_err(|error| TextSourceOperationError::Invalid(error.to_string()))?;
    let mut expected: Vec<_> = serialized
        .chunks()
        .into_iter()
        .map(|chunk| ChunkPointer {
            chunk_id: chunk.chunk_id,
            content_hash: ContentHash::compute(&chunk.payload),
            kind: chunk.kind,
            schema_version: chunk.schema_version,
        })
        .collect();
    expected.sort();
    let mut saved = manifest.chunk_pointers.clone();
    saved.sort();
    if saved != expected {
        return Err(TextSourceOperationError::Unsupported(
            "문서 밖 chunk 또는 재직렬화로 보존할 수 없는 chunk가 있습니다".into(),
        ));
    }
    Ok(TextSourceOutcome {
        source_hash: text_source::save_new(output, &model)?,
        node_count: model.node_count(),
    })
}

/// 텍스트 원본을 읽기 전용으로 열고 문서 불변식을 검사합니다.
pub fn check(path: &Path) -> Result<TextSourceOutcome, TextSourceOperationError> {
    let opened = text_source::open(path)?;
    Ok(TextSourceOutcome {
        source_hash: opened.source_hash,
        node_count: opened.model.node_count(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_types::ids::StableId;
    use crate::test_support::{
        file_digest, large_document, project_id, save_documents, scaffold_project, TempDir,
    };
    use stellar_storage::chunk::envelope::ChunkKindCode;
    use stellar_storage::chunk::id::ChunkId;
    use stellar_storage::chunk::kinds::{KIND_DOCUMENT_MANIFEST, KIND_VIEW_TREE};
    use stellar_storage::chunk::store::ChunkStore;
    use stellar_storage::generation::current::write_current_pointer;
    use stellar_storage::generation::loader::{load_current_generation, write_manifest_file};

    #[test]
    fn export_rejects_unconsumed_chunks_and_changed_pointer_metadata_without_writing() {
        let dir = TempDir::new("text-source-chunks");
        scaffold_project(&dir.path);
        save_documents(&dir.path, &[&large_document(4)]);
        let original = load_current_generation(&dir.path, project_id()).unwrap();
        let output = dir.path.join("view.stellar.yaml");
        for case in 0..6 {
            let mut manifest = original.manifest.clone();
            match case {
                0 | 1 => {
                    let mut extra = *manifest
                        .chunk_pointers
                        .iter()
                        .find(|p| p.kind == KIND_VIEW_TREE)
                        .unwrap();
                    extra.chunk_id = ChunkId::from_stable_id(StableId::new_v7(99, 0, 99));
                    if case == 0 {
                        extra.kind = ChunkKindCode(999);
                        let payload = b"unconsumed optional project data";
                        ChunkStore::new(&dir.path)
                            .write_chunk(extra.kind, payload)
                            .unwrap();
                        extra.content_hash = ContentHash::compute(payload);
                    }
                    manifest.chunk_pointers.push(extra);
                }
                2 => manifest.chunk_pointers[0].schema_version += 1,
                3 => manifest.root_document_ids.clear(),
                4 => manifest.schema_version += 1,
                5 => {
                    manifest
                        .chunk_pointers
                        .iter_mut()
                        .find(|p| p.kind == KIND_DOCUMENT_MANIFEST)
                        .unwrap()
                        .chunk_id = ChunkId::from_stable_id(StableId::new_v7(99, 0, 99))
                }
                _ => unreachable!(),
            }
            let hash = write_manifest_file(&dir.path, &manifest).unwrap();
            write_current_pointer(&dir.path, &hash).unwrap();
            let before = file_digest(&dir.path);
            assert!(export(&dir.path, &output).is_err(), "case {case}");
            assert!(!output.exists(), "case {case}");
            assert_eq!(file_digest(&dir.path), before, "case {case}");
        }
    }
}
