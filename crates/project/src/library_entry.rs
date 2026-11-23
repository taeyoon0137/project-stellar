//! library root 검증기: ProjectProbe와 storage validator의 제품 구현 (XV.3.4, XV.6.2, ADR-0119 결정 1).
//!
//! [`stellar_dependencies::entry_validation::LibraryProjectValidator`]를 이 crate의 실제 매니페스트 parser·root
//! 검증기·storage loader·텍스트 원본 codec으로 구현합니다. 의존성 crate는 core-types에만 의존하므로 검증 순서와 거부
//! 정책만 갖고, 실제 판정은 여기서 합니다.
//!
//! root 종류([`LibraryRootKind`])마다 probe가 다릅니다.
//!
//! | 종류 | probe | 이유 |
//! |---|---|---|
//! | [`LibraryRootKind::CacheEntry`] | `stellar.yaml` 정규 파일(symlink 거부) → 제한 loader → 매니페스트 스키마 | materialize한 cache entry에는 `.git`이 없음(XV.3.3). root 검증(IV.2.2)의 `.git` 요구만 뺀 같은 parser |
//! | [`LibraryRootKind::Checkout`] | [`probe_project`](IV.5.1) 그대로 | local override·worktree는 실제 Git 작업 폴더 |
//!
//! 두 probe 모두 `stellar.yaml`이 없으면 일반 Git 저장소(또는 일반 폴더)로 거부하고, 문법·스키마 오류와 UUID가 아닌
//! `project.id`는 손상 매니페스트로 거부합니다.
//!
//! storage validator는 library가 가진 저장 데이터를 모두 검사합니다.
//!
//! 1. **chunk generation**(`design/current`가 있을 때): current pointer → manifest(hash 주소·project id) → 모든
//!    chunk를 [`ChunkStore::read_chunk`]로 읽어 envelope·payload hash까지 확인합니다(V.4.4 loader는 chunk 존재만
//!    확인하므로 bytes 손상까지 보려면 읽어야 함). component library chunk는 해석까지 합니다(Property·Slot을 가진
//!    정의는 아직 텍스트 원본으로 쓸 수 없어 이 chunk에 있음, ADR-0088 결정 2).
//! 2. **텍스트 원본**(ADR-0088, Git 원본): `design/document.stellar.yaml`이 있으면 원본 파일 집합 전체를 해석하고
//!    ([`session::decode_source`]), 없으면 컴포넌트 원본(`design/components/`)만 해석합니다
//!    ([`component_source::read_project_library`], 외부 symlink 내용은 읽지 않음).
//!
//! generation component library와 텍스트 컴포넌트 원본이 함께 정의를 담으면 손상입니다. 저장 데이터가 하나도 없는
//! library(매니페스트만 있음)는 export가 없는 정상 library입니다.

use std::io;
use std::path::Path;

use stellar_core_types::ids::StableId;
use stellar_dependencies::declaration::LibraryId;
use stellar_dependencies::entry_validation::{
    LibraryProbeFailure, LibraryProjectValidator, ProbedLibrary, StorageValidationFailure,
};
use stellar_document::component::definition::ComponentLibrary;
use stellar_storage::chunk::kinds::KIND_COMPONENT_LIBRARY;
use stellar_storage::chunk::store::ChunkStore;
use stellar_storage::generation::current::current_pointer_path;
use stellar_storage::generation::loader::{load_current_generation, LoadError};

use crate::document_io::component_source;
use crate::document_io::session;
use crate::identity::parse_project_id;
use crate::manifest::Manifest;
use crate::path_resolver::ExternalAccess;
use crate::probe::{probe_project, read_at_most, ProbeError};
use crate::root::{RootError, MANIFEST_FILE};
use crate::yaml::load_restricted;
use crate::yaml::loader::MAX_INPUT_BYTES;

/// 검증할 library root의 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryRootKind {
    /// 공용 cache에 materialize한 entry(`.git` 없음, 읽기 전용).
    CacheEntry,
    /// 실제 Git 작업 폴더(local override·linked worktree).
    Checkout,
}

/// 제품 library root 검증기.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectLibraryValidator {
    kind: LibraryRootKind,
}

impl ProjectLibraryValidator {
    /// cache entry 검증기.
    pub fn cache_entry() -> ProjectLibraryValidator {
        ProjectLibraryValidator {
            kind: LibraryRootKind::CacheEntry,
        }
    }

    /// Git 작업 폴더 검증기.
    pub fn checkout() -> ProjectLibraryValidator {
        ProjectLibraryValidator {
            kind: LibraryRootKind::Checkout,
        }
    }

    /// root 종류.
    pub fn kind(&self) -> LibraryRootKind {
        self.kind
    }
}

impl LibraryProjectValidator for ProjectLibraryValidator {
    fn probe(&mut self, root: &Path) -> Result<ProbedLibrary, LibraryProbeFailure> {
        let manifest = match self.kind {
            LibraryRootKind::CacheEntry => probe_cache_entry(root)?,
            LibraryRootKind::Checkout => probe_project(root).map_err(probe_failure)?.manifest,
        };
        let project_id = manifest
            .project
            .id
            .parse::<LibraryId>()
            .map_err(|_| LibraryProbeFailure::CorruptManifest)?;
        Ok(ProbedLibrary {
            project_id,
            schema_version: manifest.schema_version,
        })
    }

    fn validate_storage(
        &mut self,
        root: &Path,
        project_id: LibraryId,
    ) -> Result<(), StorageValidationFailure> {
        validate_library_storage(root, project_id)
    }
}

/// [`ProbeError`] → probe 실패 분류.
fn probe_failure(error: ProbeError) -> LibraryProbeFailure {
    match error {
        ProbeError::Root(RootError::MissingManifest) => LibraryProbeFailure::NotAStellarProject,
        // 매니페스트는 있지만 Git 작업 폴더가 아님: local override는 Git 작업 폴더여야 합니다.
        ProbeError::Root(RootError::MissingGit) => LibraryProbeFailure::NotAStellarProject,
        ProbeError::ManifestRead { kind } => LibraryProbeFailure::Io { kind },
        ProbeError::ManifestSyntax(_) | ProbeError::ManifestSchema(_) => {
            LibraryProbeFailure::CorruptManifest
        }
    }
}

/// cache entry probe: `stellar.yaml` 정규 파일을 loader 상한까지만 읽어 실제 parser로 해석합니다.
fn probe_cache_entry(root: &Path) -> Result<Manifest, LibraryProbeFailure> {
    let path = root.join(MANIFEST_FILE);
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(LibraryProbeFailure::NotAStellarProject)
        }
        Err(e) => return Err(LibraryProbeFailure::Io { kind: e.kind() }),
    };
    // cache entry는 symlink를 만들지 않습니다(XV.3.3). 정규 파일이 아니면 손상입니다.
    if !meta.is_file() {
        return Err(LibraryProbeFailure::CorruptManifest);
    }
    let file =
        std::fs::File::open(&path).map_err(|e| LibraryProbeFailure::Io { kind: e.kind() })?;
    let bytes = read_at_most(file, MAX_INPUT_BYTES)
        .map_err(|e| LibraryProbeFailure::Io { kind: e.kind() })?
        .ok_or(LibraryProbeFailure::CorruptManifest)?;
    let text = String::from_utf8(bytes).map_err(|_| LibraryProbeFailure::CorruptManifest)?;
    let value = load_restricted(&text).map_err(|_| LibraryProbeFailure::CorruptManifest)?;
    Manifest::from_yaml(&value).map_err(|_| LibraryProbeFailure::CorruptManifest)
}

/// library 저장 데이터(chunk generation·텍스트 원본)를 검증합니다.
pub fn validate_library_storage(
    root: &Path,
    project_id: LibraryId,
) -> Result<(), StorageValidationFailure> {
    let generation = generation_components(root, project_id)?;
    let text = validate_text_source(root)?;
    if generation.is_some_and(|g| !g.is_empty()) && !text.is_empty() {
        return Err(StorageValidationFailure::TextSource);
    }
    Ok(())
}

/// current generation을 검증하며 읽고 component library chunk(있으면)를 해석합니다.
///
/// 모든 chunk를 읽어 hash를 확인합니다. component library chunk(`KIND_COMPONENT_LIBRARY`, schema 1)는 하나까지이고
/// 해석할 수 있어야 합니다. Property·Slot·상태 머신을 가진 컴포넌트 정의는 아직 텍스트 원본으로 쓸 수 없어 이 chunk에
/// 저장됩니다(ADR-0088 결정 2).
fn read_generation(
    root: &Path,
    project_id: StableId,
) -> Result<Option<ComponentLibrary>, StorageValidationFailure> {
    let loaded = load_current_generation(root, project_id).map_err(|error| match error {
        LoadError::CurrentPointer(_) => StorageValidationFailure::CurrentPointer,
        LoadError::WrongProjectId { .. } => StorageValidationFailure::ProjectIdMismatch,
        LoadError::MissingChunk { .. } => StorageValidationFailure::Chunk,
        _ => StorageValidationFailure::Manifest,
    })?;
    let store = ChunkStore::new(root);
    let mut library = None;
    for pointer in &loaded.manifest.chunk_pointers {
        let payload = store
            .read_chunk(&pointer.content_hash, pointer.kind)
            .map_err(|_| StorageValidationFailure::Chunk)?;
        if pointer.kind == KIND_COMPONENT_LIBRARY {
            if pointer.schema_version != 1 || library.is_some() {
                return Err(StorageValidationFailure::Chunk);
            }
            library = Some(
                ComponentLibrary::decode(&payload).map_err(|_| StorageValidationFailure::Chunk)?,
            );
        }
    }
    Ok(library)
}

/// library root의 component 정의를 읽습니다(검증 포함): generation component library chunk와 텍스트 컴포넌트 원본.
///
/// 둘 다 정의를 담으면 어느 쪽이 맞는지 정할 수 없으므로 손상으로 거부합니다(텍스트 원본 변환 규칙
/// `source_v4::project::from_saved`와 같은 정책).
pub fn generation_components(
    root: &Path,
    project_id: LibraryId,
) -> Result<Option<ComponentLibrary>, StorageValidationFailure> {
    if !current_pointer_path(root).exists() {
        return Ok(None);
    }
    let id = parse_project_id(&project_id.to_string())
        .ok_or(StorageValidationFailure::ProjectIdMismatch)?;
    read_generation(root, id)
}

/// 텍스트 원본을 해석하고 원본이 담은 component 정의를 돌려줍니다.
fn validate_text_source(root: &Path) -> Result<ComponentLibrary, StorageValidationFailure> {
    let invalid =
        |_: crate::document_io::text_source::TextSourceError| StorageValidationFailure::TextSource;
    match session::read_source(root).map_err(invalid)? {
        Some(files) => session::decode_source(&files)
            .map(|project| project.components)
            .map_err(invalid),
        None => component_source::read_project_library(root, ExternalAccess::MetadataOnly)
            .map(|(library, _)| library)
            .map_err(invalid),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "stellar-library-entry-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const ID: &str = "0195c530-3acd-7bc1-8414-c213f61cb969";

    fn manifest(id: &str) -> String {
        format!(
            "schema_version: 4\nproject:\n  id: \"{id}\"\n  name: \"Lib\"\ntarget_profiles: []\ndependencies: {{}}\n"
        )
    }

    #[test]
    fn cache_entry_probe_needs_no_git_but_rejects_plain_and_corrupt_roots() {
        let mut v = ProjectLibraryValidator::cache_entry();
        let plain = temp("plain");
        std::fs::write(plain.join("README.md"), "x").unwrap();
        assert_eq!(
            v.probe(&plain),
            Err(LibraryProbeFailure::NotAStellarProject)
        );

        let bad = temp("bad");
        std::fs::write(bad.join(MANIFEST_FILE), "schema_version: [\n").unwrap();
        assert_eq!(v.probe(&bad), Err(LibraryProbeFailure::CorruptManifest));

        let not_uuid = temp("not-uuid");
        std::fs::write(not_uuid.join(MANIFEST_FILE), manifest("not-a-uuid")).unwrap();
        assert!(v.probe(&not_uuid).is_err());

        let good = temp("good");
        std::fs::write(good.join(MANIFEST_FILE), manifest(ID)).unwrap();
        let probed = v.probe(&good).unwrap();
        assert_eq!(probed.project_id.to_string(), ID);
        assert_eq!(probed.schema_version, 4);
        assert_eq!(v.validate_storage(&good, probed.project_id), Ok(()));

        // 같은 폴더도 Git 작업 폴더 probe는 `.git`을 요구합니다.
        let mut checkout = ProjectLibraryValidator::checkout();
        assert_eq!(
            checkout.probe(&good),
            Err(LibraryProbeFailure::NotAStellarProject)
        );
        for dir in [plain, bad, not_uuid, good] {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn broken_text_source_and_generation_pointer_are_corrupt_storage() {
        let mut v = ProjectLibraryValidator::cache_entry();
        let root = temp("broken-source");
        std::fs::write(root.join(MANIFEST_FILE), manifest(ID)).unwrap();
        std::fs::create_dir_all(root.join("design")).unwrap();
        std::fs::write(
            root.join("design/document.stellar.yaml"),
            "format: nonsense\n",
        )
        .unwrap();
        let id: LibraryId = ID.parse().unwrap();
        assert_eq!(
            v.validate_storage(&root, id),
            Err(StorageValidationFailure::TextSource)
        );
        std::fs::remove_file(root.join("design/document.stellar.yaml")).unwrap();
        std::fs::write(current_pointer_path(&root), "garbage\n").unwrap();
        assert_eq!(
            v.validate_storage(&root, id),
            Err(StorageValidationFailure::CurrentPointer)
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
