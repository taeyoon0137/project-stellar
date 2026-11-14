//! 컴포넌트 CLI는 같은 원본 폴더를 변경하는 협조적 writer와 저장 잠금을 공유합니다.
use std::path::{Path, PathBuf};

use crate::apply::ApplyError;
use crate::core_types::{
    ids::StableId,
    value::{Size2, Vec2},
};
use crate::lock::{default_lock_dir, SaveLock};
use crate::text_source::TextSourceOperationError as Error;
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::record_codec::decode_payload;
use stellar_document::records::NodePayload;
use stellar_project::document_io::component_source::{self, ComponentFiles, ComponentSource};
use stellar_storage::hash::ContentHash;

/// CLI에 노출하는 component/root의 실제 식별자와 저장된 Frame 크기.
pub struct ComponentInfo {
    /// 전체 component ID.
    pub id: StableId,
    /// DefinitionTree root ID.
    pub root: StableId,
    /// 컴포넌트 이름.
    pub name: String,
    /// root Frame의 논리 픽셀 크기.
    pub size: Size2,
    /// root를 포함한 실제 Frame 수.
    pub node_count: usize,
    /// 원본 경로와 bytes hash.
    pub file: PathBuf,
    /// 다음 쓰기에 사용할 실제 bytes hash.
    pub hash: ContentHash,
}

fn lock(dir: &Path) -> Result<SaveLock, Error> {
    // 라이브러리 디렉터리 자체에 프로젝트 ID가 없으므로 canonical 디렉터리를 잠금 identity로 씁니다.
    SaveLock::try_acquire(
        &default_lock_dir().map_err(ApplyError::Lock)?,
        StableId::NIL,
        dir,
    )
    .map_err(|e| ApplyError::Lock(e).into())
}

fn info(source: &ComponentSource, file: PathBuf, hash: ContentHash) -> ComponentInfo {
    let NodePayload::Frame(frame) = &source.root().payload else {
        unreachable!("validated root Frame")
    };
    ComponentInfo {
        id: source.definition().id,
        root: source.root().id,
        node_count: source.node_count(),
        name: source.definition().display_name().into(),
        size: frame.size,
        file,
        hash,
    }
}

/// 독립 원본 한 개를 새로 만들며 같은 폴더의 중복 component/root ID를 먼저 검사합니다.
pub fn create(
    dir: &Path,
    name: String,
    id: StableId,
    root: StableId,
) -> Result<ComponentInfo, Error> {
    let source = ComponentSource::new(id, name, root)?;
    let _guard = lock(dir)?;
    let (file, hash) = component_source::create(dir, &source)?;
    Ok(info(&source, file, hash))
}

/// 열린 프로젝트의 현재 generation과 원본 폴더를 다시 확인한 뒤 한 개의 독립 정의를 추가합니다.
pub fn create_in_project(
    root: &Path,
    project_id: StableId,
    name: String,
) -> Result<ComponentInfo, Error> {
    let [id, root_id] = stellar_project::new_ids::new_ids().map_err(|e| Error::Io(e.kind()))?;
    let source = ComponentSource::new(id, name, root_id)?;
    create_project_source(root, project_id, &source, None)
}

/// 선택한 Frame에서 복제한 정의는 창에서 읽은 것과 같은 generation에만 추가합니다.
pub fn create_project_source(
    root: &Path,
    project_id: StableId,
    source: &ComponentSource,
    expected: Option<crate::revision::ProjectRevision>,
) -> Result<ComponentInfo, Error> {
    let _project = lock_source_project(root, project_id, expected)?;
    let design = root
        .join("design")
        .canonicalize()
        .map_err(|e| Error::Io(e.kind()))?;
    if !source_directory(root, &design) {
        return Err(Error::Invalid(
            "프로젝트 내부의 design 폴더가 필요합니다".into(),
        ));
    }
    let folder = design.join("components");
    let created = match std::fs::create_dir(&folder) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => false,
        Err(e) => return Err(Error::Io(e.kind())),
    };
    let result = (|| {
        let target = folder.canonicalize().map_err(|e| Error::Io(e.kind()))?;
        if !target.is_dir() || !source_directory(root, &target) {
            return Err(Error::Invalid(
                "프로젝트 내부의 컴포넌트 폴더가 필요합니다".into(),
            ));
        }
        let _folder = lock(&target)?;
        let (file, hash) = component_source::create_in_project(root, &target, source)?;
        Ok(info(source, file, hash))
    })();
    if result.is_err() && created {
        // 동시 writer가 추가한 파일이 있으면 디렉터리는 보존됩니다.
        let _ = std::fs::remove_dir(folder);
    }
    result
}

fn lock_source_project(
    root: &Path,
    project_id: StableId,
    expected: Option<crate::revision::ProjectRevision>,
) -> Result<SaveLock, Error> {
    let locks = default_lock_dir().map_err(ApplyError::Lock)?;
    let guard = SaveLock::try_acquire(&locks, project_id, root).map_err(ApplyError::Lock)?;
    let snapshot = crate::snapshot::open_snapshot(root).map_err(Error::Open)?;
    if snapshot.project_id() != project_id
        || snapshot.root() != root
        || expected.is_some_and(|r| r != snapshot.revision())
    {
        return Err(Error::Changed);
    }
    if crate::query::components(&snapshot).chunk_count != 0 {
        return Err(Error::Unsupported(
            "기존 component-library chunk를 가진 프로젝트입니다".into(),
        ));
    }
    Ok(guard)
}

/// 현재 창이 읽은 component ID와 원본 hash가 일치할 때 루트 Frame의 크기를 저장합니다.
pub fn resize_project_root(
    root: &Path,
    project_id: StableId,
    id: StableId,
    hash: ContentHash,
    size: Size2,
) -> Result<ComponentInfo, Error> {
    let _project = lock_source_project(root, project_id, None)?;
    let folder = root
        .join("design/components")
        .canonicalize()
        .map_err(|e| Error::Io(e.kind()))?;
    if !source_directory(root, &folder) {
        return Err(Error::Invalid(
            "프로젝트 내부의 컴포넌트 폴더가 필요합니다".into(),
        ));
    }
    let _folder = lock(&folder)?;
    let (source, file, hash) =
        component_source::resize_project_root(root, &folder, id, hash, size)?;
    Ok(info(&source, file, hash))
}

fn source_directory(root: &Path, dir: &Path) -> bool {
    let Ok(relative) = dir.strip_prefix(root) else {
        return false;
    };
    !relative.as_os_str().is_empty()
        && !relative
            .iter()
            .any(|part| part.eq_ignore_ascii_case(".git") || part.eq_ignore_ascii_case(".stellar"))
        && !relative.starts_with("design/objects")
        && !relative.starts_with("design/generations")
}

/// 원본 파일만 읽어 ComponentLibrary를 조립하고 ID 순서로 정의를 반환합니다.
pub fn check(dir: &Path) -> Result<Vec<ComponentInfo>, Error> {
    let (library, index) = component_source::read_library(dir)?;
    entries(&library, index)
}

/// 프로젝트 안에 저장된 바이너리 chunk 개수와 텍스트 원본에서 실제로 읽은 컴포넌트들.
pub struct ProjectComponents {
    /// 문서와 바이너리 chunk가 속한 generation revision.
    pub revision: crate::revision::ProjectRevision,
    /// generation의 component-library chunk 개수.
    pub chunk_count: usize,
    /// 전체 component StableId 순서로 조립한 텍스트 원본 메타데이터.
    pub entries: Vec<ComponentInfo>,
}

/// 프로젝트 컴포넌트 조회. generation과 별도로 편집되는 YAML은 매번 다시 읽습니다.
pub fn query(snapshot: &crate::snapshot::ProjectSnapshot) -> Result<ProjectComponents, Error> {
    let chunks = crate::query::components(snapshot);
    let (library, files) = project_library(snapshot)?;
    Ok(ProjectComponents {
        revision: chunks.revision,
        chunk_count: chunks.chunk_count,
        entries: entries(&library, files)?,
    })
}

/// 검증된 프로젝트의 원본 정의와 파일별 hash를 함께 반환합니다.
pub fn project_library(
    snapshot: &crate::snapshot::ProjectSnapshot,
) -> Result<(ComponentLibrary, ComponentFiles), Error> {
    let (library, files) = component_source::read_project_library(snapshot.root())?;
    if !files.is_empty() && crate::query::components(snapshot).chunk_count != 0 {
        return Err(Error::Unsupported(
            "component-library chunk와 텍스트 원본이 함께 있습니다".into(),
        ));
    }
    Ok((library, files))
}

fn entries(
    library: &ComponentLibrary,
    mut index: ComponentFiles,
) -> Result<Vec<ComponentInfo>, Error> {
    let mut result = Vec::new();
    for entry in library.iter() {
        let definition = entry.definition();
        let root = entry
            .tree()
            .node(definition.root.id())
            .expect("validated tree");
        let NodePayload::Frame(frame) =
            decode_payload(root.kind, &root.payload).map_err(|e| Error::Invalid(e.to_string()))?
        else {
            unreachable!("validated root Frame")
        };
        let (file, hash) = index.remove(&definition.id).expect("loaded source");
        result.push(ComponentInfo {
            id: definition.id,
            root: root.id,
            node_count: entry.tree().len(),
            name: definition.display_name().into(),
            size: frame.size,
            file,
            hash,
        });
    }
    Ok(result)
}

/// 파일을 열 때 관측한 hash가 일치하면 root Frame 크기를 바꿉니다.
pub fn resize(file: &Path, expected: &str, size: Size2) -> Result<ComponentInfo, Error> {
    let (_guard, hash) = lock_file(file, expected)?;
    let (source, hash) = component_source::resize(file, hash, size)?;
    Ok(info(&source, file.into(), hash))
}

/// 컴포넌트 한 개의 지정 부모에 Free Frame을 추가합니다.
pub fn add_frame(
    file: &Path,
    expected: &str,
    parent: StableId,
    id: StableId,
    name: String,
    size: Size2,
) -> Result<ComponentInfo, Error> {
    let (_guard, hash) = lock_file(file, expected)?;
    let (source, hash) = component_source::add_frame(file, hash, parent, id, name, size)?;
    Ok(info(&source, file.into(), hash))
}

/// 원본 hash가 일치하면 root 또는 자식 Frame의 표시 이름을 변경합니다.
pub fn rename_frame(
    file: &Path,
    expected: &str,
    id: StableId,
    name: String,
) -> Result<ComponentInfo, Error> {
    let (_guard, hash) = lock_file(file, expected)?;
    let (source, hash) = component_source::rename_frame(file, hash, id, name)?;
    Ok(info(&source, file.into(), hash))
}

/// 원본 hash가 일치하면 root 또는 자식 Frame의 표시 여부을 변경합니다.
pub fn set_visible(
    file: &Path,
    expected: &str,
    id: StableId,
    visible: bool,
) -> Result<ComponentInfo, Error> {
    let (_guard, hash) = lock_file(file, expected)?;
    let (source, hash) = component_source::set_visible(file, hash, id, visible)?;
    Ok(info(&source, file.into(), hash))
}

/// 원본 hash가 일치하면 root 또는 자식 Frame의 잠금 여부를 변경합니다.
pub fn set_locked(
    file: &Path,
    expected: &str,
    id: StableId,
    locked: bool,
) -> Result<ComponentInfo, Error> {
    let (_guard, hash) = lock_file(file, expected)?;
    let (source, hash) = component_source::set_locked(file, hash, id, locked)?;
    Ok(info(&source, file.into(), hash))
}

/// 원본 hash가 일치하면 Frame의 부모 좌표계 위치를 변경합니다.
pub fn set_position(
    file: &Path,
    expected: &str,
    id: StableId,
    position: Vec2,
) -> Result<ComponentInfo, Error> {
    let (_guard, hash) = lock_file(file, expected)?;
    let (source, hash) = component_source::set_position(file, hash, id, position)?;
    Ok(info(&source, file.into(), hash))
}

/// 원본 hash가 일치하면 지정 Frame의 크기를 변경합니다.
pub fn resize_frame(
    file: &Path,
    expected: &str,
    id: StableId,
    size: Size2,
) -> Result<ComponentInfo, Error> {
    let (_guard, hash) = lock_file(file, expected)?;
    let (source, hash) = component_source::resize_frame(file, hash, id, size)?;
    Ok(info(&source, file.into(), hash))
}

/// 원본 hash가 일치하면 자식 Frame과 하위 트리를 삭제합니다.
pub fn remove_frame(file: &Path, expected: &str, id: StableId) -> Result<ComponentInfo, Error> {
    let (_guard, hash) = lock_file(file, expected)?;
    let (source, hash) = component_source::remove_frame(file, hash, id)?;
    Ok(info(&source, file.into(), hash))
}

fn lock_file(file: &Path, expected: &str) -> Result<(SaveLock, ContentHash), Error> {
    let hash = expected
        .parse()
        .map_err(|_| Error::Invalid("원본 hash 형식".into()))?;
    let dir = file
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok((lock(dir)?, hash))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::open_snapshot;
    use crate::test_support::{file_digest, project_id, scaffold_project, TempDir};
    use stellar_storage::chunk::{
        id::ChunkId, kinds::KIND_COMPONENT_LIBRARY, planner::LogicalChunkState,
    };
    use stellar_storage::save::{
        cancel::{run_save, CancelToken},
        commit::SavedGenerationState,
        plan::build_save_plan,
    };

    #[test]
    fn renamed_root_and_child_reopen_with_unchanged_identity_and_geometry() {
        let dir = TempDir::new("component-rename-roundtrip");
        let id = |n| StableId::new_v7(10, 0, n);
        let source = ComponentSource::new(id(1), "Card".into(), id(2)).unwrap();
        let (file, hash) = component_source::create(&dir.path, &source).unwrap();
        let size = Size2::new(32.0, 16.0).unwrap();
        let (_, hash) =
            component_source::add_frame(&file, hash, id(2), id(3), "Child".into(), size).unwrap();
        let name = "이름 / : # \"버튼\"\n다음 줄";
        let (_, hash) = component_source::rename_frame(&file, hash, id(3), name.into()).unwrap();
        let (_, hash) = component_source::rename_frame(&file, hash, id(2), String::new()).unwrap();
        assert_eq!(component_source::open(&file).unwrap().1, hash);
        let (library, _) = component_source::read_library(&dir.path).unwrap();
        let entry = library.iter().next().unwrap();
        assert_eq!(entry.definition().id, id(1));
        assert_eq!(entry.definition().display_name(), "Card");
        assert_eq!(entry.tree().node(id(2)).unwrap().display_name, "");
        let child = entry.tree().node(id(3)).unwrap();
        assert_eq!(child.display_name, name);
        assert_eq!(child.parent, Some(id(2)));
        let NodePayload::Frame(frame) = decode_payload(child.kind, &child.payload).unwrap() else {
            panic!("Frame expected")
        };
        assert_eq!(frame.size, size);
    }

    #[cfg(unix)]
    #[test]
    fn project_query_reads_an_internal_alias_and_rejects_an_external_component() {
        use std::{fs, os::unix::fs::symlink};
        let dir = TempDir::new("component-query-links");
        let outside = TempDir::new("component-query-neighbor");
        let root = &dir.path;
        scaffold_project(root);
        let dedicated = root.join("shared-library");
        fs::create_dir(&dedicated).unwrap();
        let source = ComponentSource::new(
            StableId::new_v7(10, 0, 1),
            "Card".into(),
            StableId::new_v7(10, 0, 2),
        )
        .unwrap();
        let (file, hash) = component_source::create(&dedicated, &source).unwrap();
        let folder = root.join("design/components");
        symlink("../shared-library", &folder).unwrap();
        let snap = open_snapshot(root).unwrap();
        assert_eq!(query(&snap).unwrap().entries[0].hash, hash);
        let external = outside.path.join(file.file_name().unwrap());
        fs::rename(&file, &external).unwrap();
        symlink(&external, &file).unwrap();
        assert!(matches!(query(&snap), Err(Error::Invalid(_))));
        assert_eq!(check(&folder).unwrap()[0].hash, hash);
    }

    #[cfg(unix)]
    #[test]
    fn project_component_creation_rejects_redirected_and_invalid_sources() {
        use std::{fs, os::unix::fs::symlink};
        let dir = TempDir::new("component-app-boundary");
        let neighbor = TempDir::new("component-app-neighbor");
        let root = dir.path.canonicalize().unwrap();
        scaffold_project(&root);
        let sources = root.join("design/components");
        let add = || create_in_project(&root, project_id(), "Toolbar".into());
        // Both metadata and an external directory are invalid write targets, including through design.
        for target in [&neighbor.path, &root.join(".git")] {
            symlink(target, &sources).unwrap();
            assert!(add().is_err());
            assert_eq!(fs::read_dir(target).unwrap().count(), 0);
            fs::remove_file(&sources).unwrap();
        }
        fs::create_dir(&sources).unwrap();
        let unchanged = sources.join("broken.stellar.yaml");
        fs::write(&unchanged, "broken").unwrap();
        assert!(add().is_err());
        assert_eq!(fs::read(&unchanged).unwrap(), b"broken");
        assert_eq!(fs::read_dir(&sources).unwrap().count(), 1);
        fs::remove_file(&unchanged).unwrap();
        fs::remove_dir(&sources).unwrap();
        let alias = root.join("shared-library");
        fs::create_dir(&alias).unwrap();
        symlink(&alias, &sources).unwrap();
        let first = add().unwrap();
        let second = add().unwrap();
        assert_ne!(first.id, second.id);
        let read = query(&open_snapshot(&root).unwrap()).unwrap();
        assert_eq!(read.entries.len(), 2);
        assert!(read
            .entries
            .iter()
            .any(|e| e.id == first.id && e.hash == first.hash));
    }

    #[test]
    fn project_root_resize_is_conditional_and_preserves_unrelated_source_bytes() {
        use std::fs;
        let dir = TempDir::new("component-app-root-size");
        let root = dir.path.canonicalize().unwrap();
        scaffold_project(&root);
        let first = create_in_project(&root, project_id(), "First".into()).unwrap();
        let other = create_in_project(&root, project_id(), "Other".into()).unwrap();
        let second_before = fs::read(&other.file).unwrap();
        let new_size = Size2::new(230.5, 41.0).unwrap();
        let edit =
            |id, expected, size| resize_project_root(&root, project_id(), id, expected, size);
        let saved = edit(first.id, first.hash, new_size).unwrap();
        assert_eq!(second_before, fs::read(&other.file).unwrap());
        assert_eq!(
            query(&open_snapshot(&root).unwrap())
                .unwrap()
                .entries
                .iter()
                .find(|e| e.id == first.id)
                .unwrap()
                .size,
            new_size
        );
        assert!(matches!(
            edit(first.id, first.hash, new_size),
            Err(Error::Changed)
        ));
        let bytes = fs::read_to_string(&saved.file).unwrap();
        let commented = format!("# keep comment\n{bytes}");
        fs::write(&saved.file, &commented).unwrap();
        let current = ContentHash::compute(commented.as_bytes());
        assert_eq!(edit(first.id, current, new_size).unwrap().hash, current);
        assert_eq!(fs::read_to_string(&saved.file).unwrap(), commented);
        let locked = commented.replace("locked: \"false\"", "locked: \"true\"");
        assert_ne!(locked, commented);
        fs::write(&saved.file, &locked).unwrap();
        let expected = ContentHash::compute(locked.as_bytes());
        assert!(edit(first.id, expected, Size2::new(1., 1.).unwrap()).is_err());
        assert_eq!(fs::read_to_string(&saved.file).unwrap(), locked);
    }

    #[test]
    fn generation_only_lists_chunks_and_mixed_project_preserves_both_sources() {
        let dir = TempDir::new("component-query-sources");
        let root = &dir.path;
        scaffold_project(root);
        let id = |n| StableId::new_v7(10, 0, n);
        let lib = ComponentLibrary::new();
        let chunks = [LogicalChunkState::Dirty {
            chunk_id: ChunkId::from_stable_id(id(1)),
            kind: KIND_COMPONENT_LIBRARY,
            schema_version: 1,
            payload: lib.encode(),
        }];
        let plan = build_save_plan(None, project_id(), 1, &chunks, vec![]).unwrap();
        run_save(
            root,
            &plan,
            &mut SavedGenerationState::new(None),
            &CancelToken::new(),
        )
        .unwrap();
        let snapshot = open_snapshot(root).unwrap();
        let generation_only = query(&snapshot).unwrap();
        assert_eq!(generation_only.chunk_count, 1);
        assert!(generation_only.entries.is_empty());

        let canonical = root.canonicalize().unwrap();
        let before = file_digest(root);
        assert!(matches!(
            create_in_project(&canonical, project_id(), "Button".into()),
            Err(Error::Unsupported(_))
        ));
        assert_eq!(file_digest(root), before);
        assert!(!root.join("design/components").exists());

        let folder = root.join("design/components");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("README.md"), "unrelated").unwrap();
        assert!(query(&snapshot).unwrap().entries.is_empty());
        let source = ComponentSource::new(id(2), "Card".into(), id(3)).unwrap();
        component_source::create(&folder, &source).unwrap();
        let before = file_digest(root);
        assert!(matches!(query(&snapshot), Err(Error::Unsupported(_))));
        assert_eq!(file_digest(root), before);
    }
}
