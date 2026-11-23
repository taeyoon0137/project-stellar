//! CandidateIndexSnapshot → 기존 generation 파일(XIV.5.5).
//!
//! Git이나 파일 쓰기는 담당하지 않습니다. 기존 loader·serialize_records·SavePlan·envelope를
//! 재사용합니다. 문서가 설명하지 못하는 chunk, 스키마 또는 semantic 역투영에서 소실되는 필드는
//! Unsupported입니다. ADR-0059의 텍스트 원본 전환·외부 library 저장은 이 경로에 포함하지 않습니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use stellar_project::document_io::serialize::serialize_records_with_context;
use stellar_project::saved_project::SavedProject;
use stellar_semantic_diff::diff::diff;
use stellar_storage::chunk::envelope::ChunkEnvelope;
use stellar_storage::chunk::path::ChunkObjectPath;
use stellar_storage::chunk::pointer::ChunkPointer;
use stellar_storage::generation::loader::{manifest_path, LoadedGeneration};
use stellar_storage::hash::ContentHash;
use stellar_storage::save::plan::build_save_plan;

use crate::core_types::ids::StableId;
use crate::diff::{document_from_snapshot, snapshot_of, DocumentInput, DocumentSource};

pub use stellar_semantic_diff::diff::SemanticDiff;
pub use stellar_semantic_diff::keys::DISPLAY_NAME;
pub use stellar_semantic_diff::snapshot::SemanticSnapshot;
pub use stellar_semantic_diff::stage::{
    build_stage_candidate, build_unstage_candidate, CandidateIndexSnapshot, ChangeId,
    SelectionOperation, StageError, StageSelectionRequest,
};

/// 후보 직렬화 실패. 무손실 증명이 없는 입력은 성공으로 축소하지 않습니다.
#[derive(Debug)]
pub enum CandidateStorageError {
    /// 현재 저장/semantic 모델로 무손실 변환을 증명할 수 없습니다.
    Unsupported(&'static str),
    /// 문서·chunk·generation 검증 실패.
    Invalid(String),
    /// candidate가 현재 index에서 나온 변경과 맞지 않습니다.
    StaleBase,
}

impl fmt::Display for CandidateStorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(reason) => write!(f, "지원하지 않는 staging 저장 의미: {reason}"),
            Self::Invalid(reason) => write!(f, "staging 저장 검증: {reason}"),
            Self::StaleBase => f.write_str("candidate와 현재 index의 기준이 다릅니다"),
        }
    }
}

impl std::error::Error for CandidateStorageError {}

fn invalid(error: impl fmt::Display) -> CandidateStorageError {
    CandidateStorageError::Invalid(error.to_string())
}

/// 저장된 문서마다 영속 레코드와 Context(ADR-0114 결정 2).
fn records(
    project: &SavedProject,
) -> Result<BTreeMap<StableId, DocumentSource>, CandidateStorageError> {
    let mut records = BTreeMap::new();
    for id in project.document_ids().map_err(invalid)? {
        let model = project.load_document(id).map_err(invalid)?.model;
        let document = DocumentSource::of_model(&model).map_err(invalid)?;
        if records.insert(id, document).is_some() {
            return Err(CandidateStorageError::Unsupported("중복 문서 manifest"));
        }
    }
    Ok(records)
}

/// 문서들의 문서 범위 snapshot.
fn documents_snapshot(
    documents: &BTreeMap<StableId, DocumentSource>,
) -> Result<SemanticSnapshot, CandidateStorageError> {
    let inputs: Vec<DocumentInput<'_>> = documents.values().map(DocumentSource::input).collect();
    snapshot_of(&inputs).map_err(invalid)
}

/// 문서 하나의 chunk(레코드 chunk와 비어 있지 않은 Context의 context-set chunk).
fn document_chunks(
    document: &DocumentSource,
) -> Result<stellar_project::document_io::serialize::SerializedDocument, CandidateStorageError> {
    serialize_records_with_context(&document.records, &document.context).map_err(invalid)
}

/// 저장된 파일 → 문서 레코드 → semantic → 문서 레코드 전체 왕복을 증명합니다.
/// 지원하지 않는 optional chunk·pointer metadata·opaque field를 버리지 않고 거부합니다.
pub fn saved_semantic_snapshot(
    project: &SavedProject,
) -> Result<SemanticSnapshot, CandidateStorageError> {
    let manifest = project.generation.manifest();
    if manifest.schema_version != crate::migrate::CURRENT_SCHEMA_VERSION {
        return Err(CandidateStorageError::Unsupported("generation 스키마"));
    }
    let records = records(project)?;
    let roots: BTreeSet<_> = manifest.root_document_ids.iter().copied().collect();
    if roots != records.keys().copied().collect() || roots.len() != manifest.root_document_ids.len()
    {
        return Err(CandidateStorageError::Unsupported("root 문서 집합"));
    }
    let semantic = documents_snapshot(&records)?;
    let mut pointers = Vec::new();
    for (id, record) in &records {
        let projected = document_from_snapshot(&semantic, *id).map_err(invalid)?;
        if &projected != record {
            return Err(CandidateStorageError::Unsupported(
                "문서 semantic 역투영 손실",
            ));
        }
        for chunk in document_chunks(record)?.chunks() {
            pointers.push(ChunkPointer {
                chunk_id: chunk.chunk_id,
                content_hash: ContentHash::compute(&chunk.payload),
                kind: chunk.kind,
                schema_version: chunk.schema_version,
            });
        }
    }
    pointers.sort();
    let mut saved = manifest.chunk_pointers.clone();
    saved.sort();
    if pointers != saved {
        return Err(CandidateStorageError::Unsupported(
            "재직렬화로 보존할 수 없는 chunk 또는 opaque field",
        ));
    }
    Ok(semantic)
}

/// index의 기존 파일을 모두 유지하면서 덮어쓰거나 추가할 상대 경로와 bytes입니다.
/// 호출자는 목록에 없는 파일을 삭제하면 안 됩니다. 빈 변경은 빈 목록입니다.
pub fn serialize_candidate(
    index: &SavedProject,
    candidate: &CandidateIndexSnapshot,
) -> Result<BTreeMap<PathBuf, Vec<u8>>, CandidateStorageError> {
    let base_semantic = saved_semantic_snapshot(index)?;
    if &diff(&base_semantic, candidate.snapshot()) != candidate.applied() {
        return Err(CandidateStorageError::StaleBase);
    }
    let mut documents = records(index)?;
    for projection in candidate.documents() {
        let id = projection.records.definition.id;
        if !documents.contains_key(&id) {
            return Err(CandidateStorageError::Unsupported("문서 추가"));
        }
        // 레코드와 Context를 함께 되돌립니다(Context 변경도 stage 대상).
        documents.insert(
            id,
            document_from_snapshot(candidate.snapshot(), id).map_err(invalid)?,
        );
    }
    let actual = documents_snapshot(&documents)?;
    if &actual != candidate.snapshot() {
        return Err(CandidateStorageError::Unsupported(
            "문서 삭제 또는 레코드 밖 semantic 데이터",
        ));
    }
    if candidate.applied().is_empty() {
        return Ok(BTreeMap::new());
    }
    let manifest = index.generation.manifest();
    if manifest.generation == u64::MAX {
        return Err(CandidateStorageError::Unsupported("generation 번호 소진"));
    }
    let (_, manifest_hash) = manifest.encode_with_hash().map_err(invalid)?;
    let base = LoadedGeneration {
        manifest_hash,
        manifest: manifest.clone(),
    };
    let mut states = Vec::new();
    for record in documents.values() {
        states.extend(document_chunks(record)?.to_dirty_states());
    }
    let plan = build_save_plan(
        Some(&base),
        index.project_id,
        manifest.schema_version,
        &states,
        manifest.root_document_ids.clone(),
    )
    .map_err(invalid)?;
    let mut files = BTreeMap::new();
    for chunk in plan.dirty_chunks {
        let envelope = ChunkEnvelope::for_payload(chunk.kind, &chunk.payload);
        let mut bytes = envelope.encode().map_err(invalid)?;
        bytes.extend_from_slice(&chunk.payload);
        files.insert(
            ChunkObjectPath::for_hash(&envelope.payload_hash).to_relative_path(),
            bytes,
        );
    }
    let (bytes, hash) = plan.manifest.encode_with_hash().map_err(invalid)?;
    files.insert(manifest_path(Path::new(""), &hash), bytes);
    files.insert(
        PathBuf::from("design/current"),
        format!("{hash}\n").into_bytes(),
    );
    Ok(files)
}

/// 검증한 병합 candidate(XXIII.2.4)의 문서를 ours 세대 위의 다음 generation 파일로 직렬화합니다(XIV.12.4,
/// ADR-0081 결정 6).
///
/// 기존 save API 경로(문서 역투영 → `serialize_records` → `build_save_plan` → envelope)를 그대로 씁니다. 결과는
/// 프로젝트 상대 경로와 bytes이며(chunk·manifest·`design/current`), 파일을 쓰지 않습니다. 병합 문서 집합이 ours와
/// 다르면(문서 추가·삭제) Unsupported입니다. 직렬화할 레코드를 다시 snapshot으로 올린 값이 candidate와 다르면
/// 무손실이 아니므로 거부합니다.
pub fn serialize_merged_generation(
    ours: &SavedProject,
    validated: stellar_semantic_merge::candidate::ValidatedCandidate<'_>,
) -> Result<BTreeMap<PathBuf, Vec<u8>>, CandidateStorageError> {
    let merged = validated.snapshot();
    let ours_documents = records(ours)?;
    let ids = stellar_semantic_diff::records::documents(merged);
    if ids != ours_documents.keys().copied().collect::<Vec<_>>() {
        return Err(CandidateStorageError::Unsupported("문서 추가·삭제 병합"));
    }
    let mut documents = BTreeMap::new();
    for id in ids {
        documents.insert(id, document_from_snapshot(merged, id).map_err(invalid)?);
    }
    let actual = documents_snapshot(&documents)?;
    if &actual != merged {
        return Err(CandidateStorageError::Unsupported(
            "문서 레코드 밖 semantic 데이터",
        ));
    }
    let manifest = ours.generation.manifest();
    if manifest.generation == u64::MAX {
        return Err(CandidateStorageError::Unsupported("generation 번호 소진"));
    }
    let (_, manifest_hash) = manifest.encode_with_hash().map_err(invalid)?;
    let base = LoadedGeneration {
        manifest_hash,
        manifest: manifest.clone(),
    };
    let mut states = Vec::new();
    for record in documents.values() {
        states.extend(document_chunks(record)?.to_dirty_states());
    }
    let plan = build_save_plan(
        Some(&base),
        ours.project_id,
        manifest.schema_version,
        &states,
        manifest.root_document_ids.clone(),
    )
    .map_err(invalid)?;
    let mut files = BTreeMap::new();
    for chunk in plan.dirty_chunks {
        let envelope = ChunkEnvelope::for_payload(chunk.kind, &chunk.payload);
        let mut bytes = envelope.encode().map_err(invalid)?;
        bytes.extend_from_slice(&chunk.payload);
        files.insert(
            ChunkObjectPath::for_hash(&envelope.payload_hash).to_relative_path(),
            bytes,
        );
    }
    let (bytes, hash) = plan.manifest.encode_with_hash().map_err(invalid)?;
    files.insert(manifest_path(Path::new(""), &hash), bytes);
    files.insert(
        PathBuf::from("design/current"),
        format!("{hash}\n").into_bytes(),
    );
    Ok(files)
}

/// 컴포넌트 원본 파일 하나의 bytes ↔ semantic snapshot (XXIV.2.1, ADR-0081).
pub mod component_file;

/// 텍스트 의미 파일 하나의 3-way semantic merge (XXIV.2.1·XXIV.2.2, ADR-0081).
pub mod file_merge;

/// Semantic Merge 해결·등록이 쓰는 Git 접근 포트 (ADR-0081 결정 7).
pub mod git_port;

/// merge driver 저장소 로컬 등록 (XXIV.2.3, ADR-0081).
pub mod install;

/// Git conflict 세션: unmerged 세 stage → Semantic Merge, raw 경로 ↔ 객체 대응 (XIV.11.3, ADR-0081).
pub mod session;

/// 검증한 해결 결과의 WorkingTree 쓰기와 resolved 처리 (XIV.12.4~XIV.12.6, ADR-0081).
pub mod apply;

#[cfg(test)]
mod tests;
