//! 디스크에 저장된 프로젝트 loader (XIV.2.7, XIV.11.2 연결).
//!
//! Git snapshot provider(XIV.2)의 [`DiskProjectLoader`]를 Stellar 프로젝트 열기 경로로 구현합니다.
//! WorkingTree snapshot([`read_working_tree_project`])과 conflict 세 snapshot(base·ours·theirs)은 모두
//! 이 loader로 **디스크에 저장된 파일**만 읽어 엽니다. 편집기의 저장 대기 메모리 상태(autosave 전 편집)는
//! loader 입력에 존재하지 않으므로, snapshot과 Git status는 같은 기준
//! ([`SnapshotBasis::SavedFilesOnDisk`](stellar_git_porcelain::snapshot::working_tree::SnapshotBasis))입니다.
//!
//! 열기 순서([`SavedProjectLoader::open`]):
//! 1. `stellar.yaml`을 제한 YAML(IV.1.2)로 읽고 스키마(IV.1.1)를 검증해 project id(IV.2.3)를 얻습니다.
//! 2. current pointer → manifest → chunk 존재 순으로 검증하고(V.4.4) DocumentManifest chunk만
//!    읽습니다(V.6.6 lazy open).
//! 3. 문서 내용(ViewTree·subtree)은 요청할 때 [`SavedProject::load_document`]로 읽습니다(VI.10.3).
//!
//! `.git`은 요구하지 않습니다. materialize된 virtual project(XIV.2.6)에는 `.git`이 없기 때문입니다.
//! `stellar.yaml`이 symlink면 따르지 않고 거부합니다(snapshot이 프로젝트 밖 파일을 매니페스트로 읽지
//! 않게, IV.5.3 비신뢰 probe와 같은 규칙).
//!
//! 이 loader는 디스크 bytes를 바꾸지 않습니다(읽기 전용).

use std::fmt;
use std::io;
use std::path::Path;

use stellar_core_types::ids::StableId;
use stellar_git_porcelain::ops::GitOpError;
use stellar_git_porcelain::snapshot::working_tree::{
    read_working_tree_snapshot, DiskProjectLoader, WorkingTreeSnapshot,
};
use stellar_git_porcelain::trust::GatedGitRunner;
use stellar_storage::generation::lazy::{LazyGeneration, LazyLoadError, NoopRecorder};

use crate::document_io::load::{load_document_from_generation, LoadDocumentError, LoadedDocument};
use crate::document_io::manifest::DocumentManifestChunk;
use crate::identity::parse_project_id;
use crate::manifest::{Manifest, ManifestError};
use crate::root::MANIFEST_FILE;
use crate::yaml::{load_restricted, LoaderError};

/// 디스크에서 연 저장된 프로젝트.
#[derive(Debug)]
pub struct SavedProject {
    /// 검증한 `stellar.yaml`.
    pub manifest: Manifest,
    /// 매니페스트의 project id.
    pub project_id: StableId,
    /// 검증한 current generation(DocumentManifest chunk까지 읽음).
    pub generation: LazyGeneration,
}

impl SavedProject {
    /// current generation 번호.
    pub fn generation_number(&self) -> u64 {
        self.generation.manifest().generation
    }

    /// 저장된 문서 id(DocumentManifest chunk 순서).
    pub fn document_ids(&self) -> Result<Vec<StableId>, LoadDocumentError> {
        self.generation
            .document_manifests()
            .iter()
            .map(|(_, payload)| {
                DocumentManifestChunk::decode(payload)
                    .map(|manifest| manifest.definition.id)
                    .map_err(LoadDocumentError::Manifest)
            })
            .collect()
    }

    /// 저장된 문서 하나를 디스크 chunk에서 로드합니다(그 문서의 chunk만 읽음).
    pub fn load_document(&self, document: StableId) -> Result<LoadedDocument, LoadDocumentError> {
        load_document_from_generation(&self.generation, document, &mut NoopRecorder)
    }
}

/// 저장된 프로젝트 열기 실패.
#[derive(Debug)]
pub enum SavedProjectError {
    /// `stellar.yaml`이 symlink라 따르지 않음.
    ManifestSymlink,
    /// `stellar.yaml` 읽기 실패.
    ManifestRead {
        /// io 오류 종류.
        kind: io::ErrorKind,
    },
    /// `stellar.yaml` 제한 YAML 문법 오류.
    ManifestSyntax(LoaderError),
    /// 매니페스트 스키마 오류.
    ManifestSchema(ManifestError),
    /// `project.id`가 UUID 형식이 아님.
    InvalidProjectId {
        /// 매니페스트의 원문 값.
        id: String,
    },
    /// 저장소(generation) 열기 실패(V.4.4).
    Storage(LazyLoadError),
}

impl fmt::Display for SavedProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SavedProjectError::ManifestSymlink => {
                write!(f, "{MANIFEST_FILE}이 symlink라 따르지 않습니다")
            }
            SavedProjectError::ManifestRead { kind } => {
                write!(f, "{MANIFEST_FILE} 읽기 실패: {kind:?}")
            }
            SavedProjectError::ManifestSyntax(e) => write!(f, "{MANIFEST_FILE} 손상: {e}"),
            SavedProjectError::ManifestSchema(e) => write!(f, "매니페스트 스키마 오류: {e}"),
            SavedProjectError::InvalidProjectId { id } => {
                write!(f, "project.id가 UUID가 아닙니다: {id:?}")
            }
            SavedProjectError::Storage(e) => write!(f, "저장소 열기 실패: {e}"),
        }
    }
}

impl std::error::Error for SavedProjectError {}

/// 디스크에 저장된 파일로 프로젝트를 여는 loader(읽기 전용).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SavedProjectLoader;

impl SavedProjectLoader {
    /// loader를 만듭니다.
    pub fn new() -> SavedProjectLoader {
        SavedProjectLoader
    }

    /// `root`의 저장된 프로젝트를 엽니다: 매니페스트 → current generation(V.4.4).
    pub fn open(&self, root: &Path) -> Result<SavedProject, SavedProjectError> {
        let manifest = read_manifest(root)?;
        let project_id = parse_project_id(&manifest.project.id).ok_or_else(|| {
            SavedProjectError::InvalidProjectId {
                id: manifest.project.id.clone(),
            }
        })?;
        let generation = LazyGeneration::open(root, project_id, &mut NoopRecorder)
            .map_err(SavedProjectError::Storage)?;
        Ok(SavedProject {
            manifest,
            project_id,
            generation,
        })
    }
}

impl DiskProjectLoader for SavedProjectLoader {
    type Project = SavedProject;
    type Error = SavedProjectError;

    fn load_saved_project(&self, worktree_root: &Path) -> Result<SavedProject, SavedProjectError> {
        self.open(worktree_root)
    }
}

/// `stellar.yaml`을 symlink를 따르지 않고 읽어 검증합니다.
fn read_manifest(root: &Path) -> Result<Manifest, SavedProjectError> {
    let path = root.join(MANIFEST_FILE);
    let meta = std::fs::symlink_metadata(&path)
        .map_err(|e| SavedProjectError::ManifestRead { kind: e.kind() })?;
    if meta.file_type().is_symlink() {
        return Err(SavedProjectError::ManifestSymlink);
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| SavedProjectError::ManifestRead { kind: e.kind() })?;
    let value = load_restricted(&text).map_err(SavedProjectError::ManifestSyntax)?;
    Manifest::from_yaml(&value).map_err(SavedProjectError::ManifestSchema)
}

/// 디스크 기준 WorkingTree snapshot을 실제 프로젝트 loader로 읽습니다(XIV.2.7).
///
/// 파일 목록은 Git이 아는 작업 트리 파일, 프로젝트는 [`SavedProjectLoader`]가 같은 root에서 연
/// 저장된 프로젝트입니다. 둘 다 디스크에 저장된 파일 기준이며 편집기 메모리 상태는 포함하지 않습니다.
pub fn read_working_tree_project(
    git: &GatedGitRunner,
    worktree_root: &Path,
) -> Result<WorkingTreeSnapshot<SavedProject, SavedProjectError>, GitOpError> {
    read_working_tree_snapshot(git, worktree_root, &SavedProjectLoader::new())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::document_io::serialize::serialize_document;
    use crate::manifest::{ProjectMeta, CURRENT_SCHEMA_VERSION};
    use crate::yaml::write_canonical;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use stellar_document::model::DocumentModel;
    use stellar_document::test_support::random_document::random_valid_document;
    use stellar_document::test_support::semantic_snapshot::model_snapshot_json;
    use stellar_storage::generation::loader::{load_current_generation, LoadError};
    use stellar_storage::save::cancel::{run_save, CancelToken};
    use stellar_storage::save::commit::SavedGenerationState;
    use stellar_storage::save::plan::build_save_plan;

    pub(crate) const PROJECT_ID_TEXT: &str = "0195c530-3acd-7bc1-8414-c213f61cb965";

    pub(crate) struct TempDir {
        pub(crate) path: PathBuf,
    }
    impl TempDir {
        pub(crate) fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "stellar-saved-project-test-{}-{n}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    pub(crate) fn project_id() -> StableId {
        parse_project_id(PROJECT_ID_TEXT).unwrap()
    }

    /// 유효한 `stellar.yaml`을 씁니다.
    pub(crate) fn write_manifest(root: &Path, id: &str) {
        let manifest = Manifest {
            schema_version: CURRENT_SCHEMA_VERSION,
            project: ProjectMeta {
                id: id.to_string(),
                name: "Saved".to_string(),
            },
            target_profiles: Vec::new(),
            dependencies: Vec::new(),
        };
        std::fs::write(
            root.join(MANIFEST_FILE),
            write_canonical(&manifest.to_yaml()),
        )
        .unwrap();
    }

    /// 문서들을 실제 저장 파이프라인(V.5)으로 current generation 위에 저장합니다.
    pub(crate) fn save_documents(root: &Path, documents: &[&DocumentModel]) {
        let base = load_current_generation(root, project_id()).ok();
        let serialized: Vec<_> = documents
            .iter()
            .map(|model| serialize_document(model).unwrap())
            .collect();
        let states: Vec<_> = serialized
            .iter()
            .flat_map(|d| d.to_dirty_states())
            .collect();
        let roots = serialized.iter().map(|d| d.document_id).collect();
        let plan = build_save_plan(base.as_ref(), project_id(), 1, &states, roots).unwrap();
        let mut memory = SavedGenerationState::new(
            base.as_ref()
                .map(|b| (b.manifest.generation, b.manifest_hash)),
        );
        run_save(root, &plan, &mut memory, &CancelToken::new()).unwrap();
    }

    #[test]
    fn opens_manifest_and_current_generation_and_loads_documents() {
        let dir = TempDir::new();
        write_manifest(&dir.path, PROJECT_ID_TEXT);
        let first = random_valid_document(11);
        let second = random_valid_document(12);
        save_documents(&dir.path, &[&first, &second]);

        let project = SavedProjectLoader::new().open(&dir.path).unwrap();
        assert_eq!(project.project_id, project_id());
        assert_eq!(project.manifest.project.name, "Saved");
        assert_eq!(project.generation_number(), 1);
        let mut ids = project.document_ids().unwrap();
        ids.sort();
        let mut expected = vec![first.document_id(), second.document_id()];
        expected.sort();
        assert_eq!(ids, expected);
        for model in [&first, &second] {
            let loaded = project.load_document(model.document_id()).unwrap();
            assert_eq!(
                model_snapshot_json(&loaded.model).unwrap(),
                model_snapshot_json(model).unwrap()
            );
        }
        // DiskProjectLoader로도 같은 결과(.git 없이 열림).
        let via_trait = SavedProjectLoader.load_saved_project(&dir.path).unwrap();
        assert_eq!(via_trait.generation_number(), 1);
    }

    #[test]
    fn invalid_projects_are_rejected_with_typed_errors() {
        let loader = SavedProjectLoader::new();
        let dir = TempDir::new();
        // 매니페스트 없음.
        assert!(matches!(
            loader.open(&dir.path),
            Err(SavedProjectError::ManifestRead {
                kind: io::ErrorKind::NotFound
            })
        ));
        // 문법 오류(anchor 금지).
        std::fs::write(dir.path.join(MANIFEST_FILE), "schema_version: &a 1\n").unwrap();
        assert!(matches!(
            loader.open(&dir.path),
            Err(SavedProjectError::ManifestSyntax(_))
        ));
        // 스키마 오류.
        std::fs::write(dir.path.join(MANIFEST_FILE), "schema_version: 1\n").unwrap();
        assert!(matches!(
            loader.open(&dir.path),
            Err(SavedProjectError::ManifestSchema(_))
        ));
        // UUID가 아닌 project id.
        write_manifest(&dir.path, "not-a-uuid");
        assert!(matches!(
            loader.open(&dir.path),
            Err(SavedProjectError::InvalidProjectId { id }) if id == "not-a-uuid"
        ));
        // 저장된 generation 없음(current pointer 없음).
        write_manifest(&dir.path, PROJECT_ID_TEXT);
        assert!(matches!(
            loader.open(&dir.path),
            Err(SavedProjectError::Storage(LazyLoadError::Load(
                LoadError::CurrentPointer(_)
            )))
        ));
        // 매니페스트와 저장소의 project id가 다름.
        save_documents(&dir.path, &[&random_valid_document(3)]);
        write_manifest(&dir.path, "0195c530-3acd-7bc1-8414-000000000000");
        assert!(matches!(
            loader.open(&dir.path),
            Err(SavedProjectError::Storage(LazyLoadError::Load(
                LoadError::WrongProjectId { .. }
            )))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_manifest_is_not_followed() {
        let dir = TempDir::new();
        let outside = TempDir::new();
        write_manifest(&outside.path, PROJECT_ID_TEXT);
        save_documents(&dir.path, &[&random_valid_document(4)]);
        std::os::unix::fs::symlink(
            outside.path.join(MANIFEST_FILE),
            dir.path.join(MANIFEST_FILE),
        )
        .unwrap();
        assert!(matches!(
            SavedProjectLoader::new().open(&dir.path),
            Err(SavedProjectError::ManifestSymlink)
        ));
    }
}
