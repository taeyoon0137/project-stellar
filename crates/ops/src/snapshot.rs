//! 앱 없이 연 읽기 전용 프로젝트 snapshot (XXIV.4.2).
//!
//! [`open_snapshot`]은 실행 중 앱 없이 프로젝트 폴더를 직접 엽니다.
//!
//! 1. 입력 경로가 디렉터리인지 확인합니다.
//! 2. root 검증(IV.2.2, `stellar.yaml`+`.git`)과 `stellar.yaml` probe(IV.5.1, loader 상한까지만 읽음).
//! 3. `project.id`(IV.2.3)로 current generation을 엽니다(V.4.4 검증, V.6.6 lazy: DocumentManifest chunk만 읽음).
//!    current pointer가 없으면 저장된 generation이 없는 프로젝트([`ProjectRevision::Unsaved`])입니다.
//! 4. generation schema를 제품 migration 정책으로 판정합니다(IX.11.21, [`decide`]). 같은 schema는 그대로,
//!    낮은 schema는 메모리에서만 migration해 읽고(디스크는 그대로), 높은 schema·경로 없는 schema는 열기는
//!    하되(상태 보고·`stellar migrate` 판정용) 문서 내용 읽기를 [`DocumentReadError::Schema`]로 거부합니다.
//!    이 경로는 읽기 전용이므로 메모리 migration 결과도 쓰지 않습니다.
//! 5. revision은 열린 generation 번호와 manifest content hash입니다([`crate::revision`]).
//!
//! 이 경로는 파일을 만들거나 바꾸지 않습니다. 문서 내용(node 레코드)은 요청할 때
//! [`ProjectSnapshot::document_records`]로 그 문서의 chunk만 읽습니다. 모든 결과는 열기 시점에
//! 고정한 generation 하나에서 나오므로, 다른 writer가 그 뒤에 저장해도 같은 snapshot의 결과는
//! 섞이지 않습니다(chunk는 content-addressed라 불변).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use stellar_document::definitions::DocumentDefinition;
use stellar_document::model::DocumentModel;
use stellar_document::records::DocumentRecords;
use stellar_project::document_io::load::LoadDocumentError;
use stellar_project::document_io::manifest::DocumentManifestChunk;
use stellar_project::identity::parse_project_id;
use stellar_project::manifest::Manifest;
use stellar_project::migration::{decide, MigratedGeneration, MigrationPolicy, SchemaDecision};
use stellar_project::probe::{probe_project, ProbeError};
use stellar_project::root::GitLink;
use stellar_storage::chunk::kinds::INITIAL_KINDS;
use stellar_storage::generation::current::CurrentPointerError;
use stellar_storage::generation::lazy::{LazyGeneration, LazyLoadError, NoopRecorder};
use stellar_storage::generation::loader::LoadError;

use crate::core_types::ids::StableId;
use crate::error::{DocumentReadError, OpenError};
use crate::revision::ProjectRevision;

/// generation에 기록된 chunk kind별 개수.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkKindCount {
    /// kind 이름(V.6.1 registry). 등록되지 않은 kind는 `None`.
    pub name: Option<&'static str>,
    /// kind code(저장 포맷 값).
    pub code: u32,
    /// 이 kind의 chunk pointer 수.
    pub count: usize,
}

/// 앱 없이 연 읽기 전용 프로젝트 snapshot.
#[derive(Debug)]
pub struct ProjectSnapshot {
    root: PathBuf,
    git_link: GitLink,
    manifest: Manifest,
    project_id: StableId,
    revision: ProjectRevision,
    documents: Vec<DocumentDefinition>,
    chunk_kinds: Vec<ChunkKindCount>,
    generation: Option<LazyGeneration>,
    /// 낮은 schema generation을 메모리에서 migration한 결과(디스크는 원본 그대로).
    migrated: Option<MigratedGeneration>,
    schema: SchemaDecision,
}

impl ProjectSnapshot {
    /// 연 프로젝트 root 경로(진단 문맥용, 결과 출력에는 넣지 않습니다).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `.git` 링크 종류.
    pub fn git_link(&self) -> GitLink {
        self.git_link
    }

    /// 검증한 `stellar.yaml`.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// 매니페스트의 project id.
    pub fn project_id(&self) -> StableId {
        self.project_id
    }

    /// 열린 generation 기준 revision.
    pub fn revision(&self) -> ProjectRevision {
        self.revision
    }

    /// 저장된 문서 정의(DocumentManifest chunk 순서 = chunk id 오름차순).
    pub fn documents(&self) -> &[DocumentDefinition] {
        &self.documents
    }

    /// generation의 chunk kind별 개수(kind code 오름차순). 저장된 generation이 없으면 빈 목록.
    pub fn chunk_kinds(&self) -> &[ChunkKindCount] {
        &self.chunk_kinds
    }

    /// 열린 generation(저장된 generation이 없으면 `None`).
    pub fn generation(&self) -> Option<&LazyGeneration> {
        self.generation.as_ref()
    }

    /// 열린 generation의 schema 판정(IX.11.21). 저장된 generation이 없으면 [`SchemaDecision::Current`]입니다.
    pub fn schema(&self) -> SchemaDecision {
        self.schema
    }

    /// 문서 하나의 런타임 모델을 그 문서의 chunk만 읽어 만듭니다(VI.10.3).
    pub fn document_model(&self, document: StableId) -> Result<DocumentModel, DocumentReadError> {
        if let SchemaDecision::Reject(rejection) = self.schema {
            return Err(DocumentReadError::Schema(rejection));
        }
        let generation = self
            .generation
            .as_ref()
            .filter(|_| self.documents.iter().any(|d| d.id == document))
            .ok_or(DocumentReadError::UnknownDocument { document })?;
        let loaded = match &self.migrated {
            Some(migrated) => migrated.load_document(document),
            None => stellar_project::document_io::load::load_document_from_generation(
                generation,
                document,
                &mut NoopRecorder,
            ),
        }
        .map_err(|e| match e {
            LoadDocumentError::UnknownDocument { document } => {
                DocumentReadError::UnknownDocument { document }
            }
            other => DocumentReadError::Load(other),
        })?;
        Ok(loaded.model)
    }

    /// 문서 하나의 영속 레코드를 그 문서의 chunk만 읽어 만듭니다(VI.10.3, canonical 정렬).
    pub fn document_records(
        &self,
        document: StableId,
    ) -> Result<DocumentRecords, DocumentReadError> {
        let mut records = self
            .document_model(document)?
            .to_records()
            .map_err(|e| DocumentReadError::Records(e.to_string()))?;
        records.sort_canonical();
        Ok(records)
    }
}

/// 문서 chunk를 읽지 않는 가벼운 검사 결과(IV.5.1 probe).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectProbe {
    /// 프로젝트 root 경로(진단 문맥용).
    pub root: PathBuf,
    /// `.git` 링크 종류.
    pub git_link: GitLink,
    /// 검증한 `stellar.yaml`.
    pub manifest: Manifest,
}

/// 경로를 가볍게 검사합니다: 디렉터리 확인 → root 검증 → `stellar.yaml` probe(IV.5.1).
///
/// 문서 chunk(`design/`)는 읽지 않고 파일을 만들거나 바꾸지 않습니다.
pub fn probe(path: &Path) -> Result<ProjectProbe, OpenError> {
    let metadata = std::fs::metadata(path).map_err(|e| OpenError::Path { kind: e.kind() })?;
    if !metadata.is_dir() {
        return Err(OpenError::NotADirectory);
    }
    let probe = probe_project(path).map_err(|e| match e {
        ProbeError::Root(root) => OpenError::NotAProject(root),
        ProbeError::ManifestRead { kind } => OpenError::ManifestRead { kind },
        ProbeError::ManifestSyntax(e) => OpenError::ManifestSyntax(e),
        ProbeError::ManifestSchema(e) => OpenError::ManifestSchema(e),
    })?;
    Ok(ProjectProbe {
        root: probe.root,
        git_link: probe.git_link,
        manifest: probe.manifest,
    })
}

/// 프로젝트를 앱 없이 읽기 전용으로 엽니다(모듈 문서 순서).
pub fn open_snapshot(path: &Path) -> Result<ProjectSnapshot, OpenError> {
    let probe = probe(path)?;
    let project_id = parse_project_id(&probe.manifest.project.id).ok_or_else(|| {
        OpenError::InvalidProjectId {
            id: probe.manifest.project.id.clone(),
        }
    })?;

    let generation = match LazyGeneration::open(&probe.root, project_id, &mut NoopRecorder) {
        Ok(generation) => Some(generation),
        Err(LazyLoadError::Load(LoadError::CurrentPointer(CurrentPointerError::Missing {
            ..
        }))) => None,
        Err(e) => return Err(OpenError::Storage(e)),
    };

    let schema = generation
        .as_ref()
        .map_or(SchemaDecision::Current, |generation| {
            decide(
                generation.manifest().schema_version,
                &MigrationPolicy::PRODUCT,
            )
        });
    let migrated = match (schema, &generation) {
        (SchemaDecision::MigrateInMemory { .. }, Some(generation)) => Some(
            MigratedGeneration::from_lazy(generation, &MigrationPolicy::PRODUCT)
                .map_err(|e| OpenError::Migration(e.to_string()))?,
        ),
        _ => None,
    };

    let (revision, mut documents, chunk_kinds) = match &generation {
        None => (ProjectRevision::Unsaved, Vec::new(), Vec::new()),
        Some(generation) => {
            let manifest = generation.manifest();
            let (_, manifest_hash) = manifest
                .encode_with_hash()
                .map_err(|e| OpenError::Storage(LazyLoadError::Load(LoadError::Manifest(e))))?;
            let revision = ProjectRevision::Saved {
                generation: manifest.generation,
                manifest_hash,
            };
            let mut documents = Vec::new();
            for (pointer, payload) in generation.document_manifest_records() {
                let chunk =
                    DocumentManifestChunk::decode_for_schema(pointer.schema_version, payload)
                        .map_err(|e| {
                            OpenError::Document(DocumentReadError::Load(
                                LoadDocumentError::Manifest(e),
                            ))
                        })?;
                documents.push(chunk.definition);
            }
            let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
            for pointer in &manifest.chunk_pointers {
                *counts.entry(pointer.kind.0).or_default() += 1;
            }
            let chunk_kinds = counts
                .into_iter()
                .map(|(code, count)| ChunkKindCount {
                    name: INITIAL_KINDS
                        .iter()
                        .find(|(kind, _)| kind.0 == code)
                        .map(|(_, name)| *name),
                    code,
                    count,
                })
                .collect();
            (revision, documents, chunk_kinds)
        }
    };
    if let Some(migrated) = &migrated {
        // 문서 정의는 migration한 DocumentManifest에서 다시 읽습니다.
        documents = Vec::new();
        for chunk in migrated
            .chunks()
            .iter()
            .filter(|chunk| chunk.kind == stellar_storage::chunk::kinds::KIND_DOCUMENT_MANIFEST)
        {
            let manifest =
                DocumentManifestChunk::decode_for_schema(chunk.schema_version, &chunk.payload)
                    .map_err(|e| {
                        OpenError::Document(DocumentReadError::Load(LoadDocumentError::Manifest(e)))
                    })?;
            documents.push(manifest.definition);
        }
    }

    Ok(ProjectSnapshot {
        root: probe.root,
        git_link: probe.git_link,
        manifest: probe.manifest,
        project_id,
        revision,
        documents,
        chunk_kinds,
        generation,
        migrated,
        schema,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        file_digest, save_documents, scaffold_project, TempDir, PROJECT_ID_TEXT,
    };
    use stellar_document::test_support::random_document::random_valid_document;
    use stellar_storage::generation::current::read_current_pointer;

    #[test]
    fn unsaved_project_has_g0_revision_and_no_documents() {
        let dir = TempDir::new("unsaved");
        scaffold_project(&dir.path);
        let snapshot = open_snapshot(&dir.path).unwrap();
        assert_eq!(snapshot.revision(), ProjectRevision::Unsaved);
        assert!(snapshot.documents().is_empty());
        assert!(snapshot.chunk_kinds().is_empty());
        assert!(snapshot.generation().is_none());
        assert_eq!(
            snapshot.project_id(),
            parse_project_id(PROJECT_ID_TEXT).unwrap()
        );
        assert_eq!(snapshot.git_link(), GitLink::Directory);
    }

    #[test]
    fn revision_matches_current_pointer_and_generation_without_app() {
        // 검증 기준(XXIV.4.2): 앱 미실행 snapshot fixture revision 일치.
        let dir = TempDir::new("saved");
        scaffold_project(&dir.path);
        let first = random_valid_document(21);
        let second = random_valid_document(22);
        save_documents(&dir.path, &[&first]);

        let snapshot = open_snapshot(&dir.path).unwrap();
        let pointer = read_current_pointer(&dir.path).unwrap();
        assert_eq!(
            snapshot.revision(),
            ProjectRevision::Saved {
                generation: 1,
                manifest_hash: pointer
            }
        );
        assert_eq!(snapshot.documents().len(), 1);
        assert_eq!(snapshot.documents()[0].id, first.document_id());

        save_documents(&dir.path, &[&first, &second]);
        let reopened = open_snapshot(&dir.path).unwrap();
        let pointer = read_current_pointer(&dir.path).unwrap();
        assert_eq!(reopened.revision().generation(), 2);
        assert_eq!(reopened.revision().manifest_hash(), Some(pointer));
        assert_ne!(reopened.revision(), snapshot.revision());
        let mut ids: Vec<StableId> = reopened.documents().iter().map(|d| d.id).collect();
        ids.sort();
        let mut expected = vec![first.document_id(), second.document_id()];
        expected.sort();
        assert_eq!(ids, expected);
        let manifests = reopened
            .chunk_kinds()
            .iter()
            .find(|k| k.name == Some("document-manifest"))
            .unwrap();
        assert_eq!(manifests.count, 2);
    }

    #[test]
    fn document_records_round_trip_and_unknown_document_is_rejected() {
        let dir = TempDir::new("records");
        scaffold_project(&dir.path);
        let model = random_valid_document(31);
        save_documents(&dir.path, &[&model]);
        let snapshot = open_snapshot(&dir.path).unwrap();
        let records = snapshot.document_records(model.document_id()).unwrap();
        let mut expected = model.to_records().unwrap();
        expected.sort_canonical();
        assert_eq!(records, expected);
        let unknown = StableId::new_v7(1, 2, 3);
        assert!(matches!(
            snapshot.document_records(unknown),
            Err(DocumentReadError::UnknownDocument { document }) if document == unknown
        ));
    }

    #[test]
    fn open_snapshot_never_writes_project_files() {
        let dir = TempDir::new("readonly");
        scaffold_project(&dir.path);
        save_documents(&dir.path, &[&random_valid_document(41)]);
        let before = file_digest(&dir.path);
        let snapshot = open_snapshot(&dir.path).unwrap();
        for document in snapshot.documents() {
            snapshot.document_records(document.id).unwrap();
        }
        assert_eq!(file_digest(&dir.path), before);
    }

    #[test]
    fn newer_schema_generation_is_rejected_and_current_schema_opens() {
        // IX.11.21: open_snapshot은 migration::decide 판정을 따릅니다.
        let dir = TempDir::new("schema-newer");
        scaffold_project(&dir.path);
        let model = random_valid_document(51);
        save_documents(&dir.path, &[&model]);
        assert_eq!(
            open_snapshot(&dir.path).unwrap().schema(),
            SchemaDecision::Current
        );
        let before = file_digest(&dir.path);
        crate::test_support::save_documents_with_schema(&dir.path, &[&model], 2);
        let after = file_digest(&dir.path);
        assert_ne!(before, after);
        let newer = open_snapshot(&dir.path).unwrap();
        let rejection = stellar_project::migration::SchemaRejection::Newer {
            found: 2,
            target: 1,
        };
        assert_eq!(newer.schema(), SchemaDecision::Reject(rejection));
        assert!(matches!(
            newer.document_records(model.document_id()),
            Err(DocumentReadError::Schema(found)) if found == rejection
        ));
        assert_eq!(file_digest(&dir.path), after, "거부한 읽기는 쓰지 않음");
    }

    #[test]
    fn open_errors_are_typed() {
        let dir = TempDir::new("errors");
        assert!(matches!(
            open_snapshot(&dir.path.join("missing")),
            Err(OpenError::Path {
                kind: std::io::ErrorKind::NotFound
            })
        ));
        std::fs::write(dir.path.join("file"), b"x").unwrap();
        assert!(matches!(
            open_snapshot(&dir.path.join("file")),
            Err(OpenError::NotADirectory)
        ));
        assert!(matches!(
            open_snapshot(&dir.path),
            Err(OpenError::NotAProject(_))
        ));
        scaffold_project(&dir.path);
        // 손상된 current pointer는 저장소 오류입니다(Unsaved로 숨기지 않음).
        let pointer = stellar_storage::generation::current::current_pointer_path(&dir.path);
        std::fs::write(&pointer, b"not a hash\n").unwrap();
        assert!(matches!(
            open_snapshot(&dir.path),
            Err(OpenError::Storage(_))
        ));
    }
}
