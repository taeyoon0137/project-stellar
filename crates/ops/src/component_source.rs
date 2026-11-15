//! 컴포넌트 CLI는 같은 원본 폴더를 변경하는 협조적 writer와 저장 잠금을 공유합니다.
use std::path::{Path, PathBuf};

use crate::apply::ApplyError;
use crate::core_types::{ids::StableId, value::Size2};
use crate::lock::{default_lock_dir, SaveLock};
use crate::text_source::TextSourceOperationError as Error;
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::record_codec::decode_payload;
use stellar_document::records::NodePayload;
use stellar_project::document_io::component_source::{self, ComponentFiles, ComponentSource};
pub use stellar_project::document_io::component_source::{color_source_value, ComponentRootEdit};
pub use stellar_project::path_resolver::ExternalAccess;
use stellar_storage::hash::ContentHash;

/// 검증된 원본의 Frame 속성 값입니다. 수치와 색은 텍스트 원본의 정확한 표현을 유지합니다.
pub use stellar_project::yaml::YamlValue as SourceValue;

/// 단일 정의에 소속된 Frame의 공통 메타데이터와 정규 원본 속성.
#[derive(Debug)]
pub struct DefinitionFrame {
    /// 정의 안에서 안정적인 node ID.
    pub id: StableId,
    /// 부모 ID. 정의 root에는 없습니다.
    pub parent: Option<StableId>,
    /// 형제 간 화면 순서를 정하는 hex key.
    pub order_key: String,
    /// 사용자가 지정한 이름.
    pub display_name: String,
    /// 저장된 자체 표시 여부.
    pub visible: bool,
    /// 저장된 자체 잠금 여부.
    pub locked: bool,
    /// 크기, local affine, 채우기·테두리 등 원본 Frame payload.
    pub payload: SourceValue,
}

/// 현재 프로젝트의 한 공유 정의. node 순서는 부모 우선, 형제는 OrderKey 순입니다.
#[derive(Debug)]
pub struct ComponentDetail {
    /// project ID. 페이지가 속한 프로젝트를 판별합니다.
    pub project: StableId,
    /// 저장된 프로젝트 revision.
    pub revision: crate::revision::ProjectRevision,
    /// 요청한 컴포넌트 정의의 안정 식별자.
    pub component: StableId,
    /// 컴포넌트 이름. 서로 같은 이름을 허용합니다.
    pub name: String,
    /// 루트 Frame ID.
    pub root: StableId,
    /// 선택한 파일의 원본 bytes hash.
    pub source_hash: ContentHash,
    /// 읽을 때 검증한 전체 원본 집합. 다른 파일의 ID 충돌도 다시 검증합니다.
    pub library_hash: ContentHash,
    /// 정의 tree 전체.
    pub nodes: Vec<DefinitionFrame>,
}

/// 승인된 snapshot의 프로젝트 폴더에서 정의를 찾습니다. 다른 프로젝트의 ID는 찾을 수 없습니다.
pub fn detail(
    snapshot: &crate::snapshot::ProjectSnapshot,
    component: StableId,
) -> Result<Option<ComponentDetail>, Error> {
    let (library, files) = project_library(snapshot)?;
    let Some(entry) = library.get(crate::core_types::reference::ComponentRef::new(component))
    else {
        return Ok(None);
    };
    let root = entry.definition().root.id();
    let tree = entry.tree();
    let mut nodes = Vec::with_capacity(tree.len());
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        let node = tree.node(id).expect("validated component tree");
        let NodePayload::Frame(frame) = decode_payload(node.kind, &node.payload)
            .map_err(|err| Error::Invalid(err.to_string()))?
        else {
            return Err(Error::Unsupported(
                "컴포넌트 원본은 Free Frame만 지원합니다".into(),
            ));
        };
        nodes.push(DefinitionFrame {
            id,
            parent: node.parent,
            order_key: node
                .order_key
                .as_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            display_name: node.display_name.clone(),
            visible: node.visible,
            locked: node.locked,
            payload: component_source::frame_source_properties(&frame),
        });
        pending.extend(tree.children(id).iter().rev().copied());
    }
    Ok(Some(ComponentDetail {
        project: snapshot.project_id(),
        revision: snapshot.revision(),
        component,
        name: entry.definition().display_name().into(),
        root,
        source_hash: files[&component].1,
        library_hash: component_source::source_set_hash(files.into_values())?,
        nodes,
    }))
}

/// CLI에 노출하는 component/root의 실제 식별자와 저장된 Frame 크기.
#[derive(Debug)]
pub struct ComponentInfo {
    /// 전체 component ID.
    pub id: StableId,
    /// DefinitionTree root ID.
    pub root: StableId,
    /// 컴포넌트 이름.
    pub name: String,
    /// root Frame의 논리 픽셀 크기.
    pub size: Size2,
    /// 루트 Frame에 저장된 비연관 RGBA 채우기.
    pub background: Option<crate::core_types::color::Color>,
    /// root를 포함한 실제 Frame 수.
    pub node_count: usize,
    /// 원본 경로와 bytes hash.
    pub file: PathBuf,
    /// 다음 쓰기에 사용할 실제 bytes hash.
    pub hash: ContentHash,
}

impl ComponentInfo {
    /// 파일명의 이름 부분이 정의 이름과 다르면 정규 파일명을 돌려줍니다. 정의는 ID로 조립됩니다.
    pub fn renamed_file(&self) -> Option<String> {
        let expected = component_source::source_filename(&self.name, self.id).ok()?;
        (self.file.file_name() != Some(expected.as_ref())).then_some(expected)
    }
}

/// 원본 폴더가 Stellar 프로젝트 안에 있으면(조상에 `stellar.yaml`) 그 프로젝트의 저장 잠금을 잡고
/// component-library chunk가 없는지 다시 확인합니다(ADR-0061 결정 7). manifest를 읽을 수 없으면 쓰지
/// 않습니다.
fn project_guard(dir: &Path) -> Result<Option<SaveLock>, Error> {
    let folder = dir.canonicalize().map_err(|e| Error::Io(e.kind()))?;
    let Some(root) = folder.ancestors().find(|ancestor| {
        ancestor
            .join(stellar_project::root::MANIFEST_FILE)
            .is_file()
    }) else {
        return Ok(None);
    };
    let probe = crate::snapshot::probe(root).map_err(Error::Open)?;
    let project_id = stellar_project::identity::parse_project_id(&probe.manifest.project.id)
        .ok_or_else(|| Error::Invalid("프로젝트 ID 형식이 잘못되었습니다".into()))?;
    lock_source_project(root, project_id, None).map(Some)
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
        background: frame.background,
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
    let _project = project_guard(dir)?;
    let _guard = lock(dir)?;
    let (file, hash) = component_source::create(dir, &source)?;
    Ok(info(&source, file, hash))
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

/// 원본 파일만 읽어 ComponentLibrary를 조립하고 ID 순서로 정의를 반환합니다.
pub fn check(dir: &Path) -> Result<Vec<ComponentInfo>, Error> {
    let (library, index) = component_source::read_library(dir)?;
    entries(&library, index)
}

/// 프로젝트 안에 저장된 바이너리 chunk 개수와 텍스트 원본에서 실제로 읽은 컴포넌트들.
#[derive(Debug)]
pub struct ProjectComponents {
    /// 문서와 바이너리 chunk가 속한 generation revision.
    pub revision: crate::revision::ProjectRevision,
    /// generation의 component-library chunk 개수.
    pub chunk_count: usize,
    /// 아래 항목을 읽은 파일명과 원본 bytes 전체의 hash. generation과 별도로 바뀝니다.
    pub source_hash: ContentHash,
    /// 전체 component StableId 순서로 조립한 텍스트 원본 메타데이터.
    pub entries: Vec<ComponentInfo>,
}

/// 프로젝트 컴포넌트 조회. generation과 별도로 편집되는 YAML은 매번 다시 읽습니다.
pub fn query(snapshot: &crate::snapshot::ProjectSnapshot) -> Result<ProjectComponents, Error> {
    let chunks = crate::query::components(snapshot);
    let (library, files) = project_library(snapshot)?;
    let source_hash = component_source::source_set_hash(files.values().cloned())?;
    Ok(ProjectComponents {
        revision: chunks.revision,
        chunk_count: chunks.chunk_count,
        source_hash,
        entries: entries(&library, files)?,
    })
}

/// 프로젝트의 신뢰 상태에서 정한 외부 symlink 읽기 정책입니다(ADR-0061 결정 8). CLI와 headless
/// MCP는 앱과 같은 OS 사용자 데이터의 신뢰 저장소를 읽고, 신뢰를 확인할 수 없으면 외부 target 내용을
/// 읽지 않습니다. 저장소 파일을 만들거나 바꾸지 않습니다.
pub fn external_access(project_id: StableId, root: &Path) -> ExternalAccess {
    use stellar_project::{identity::ProjectIdentity, trust::TrustStatus};
    let status = ProjectIdentity::new(project_id, root)
        .ok()
        .map_or(TrustStatus::NotTrusted, |identity| {
            crate::trusted::os_trust_status(&identity)
        });
    ExternalAccess::from_trust(status)
}

/// 검증된 프로젝트의 원본 정의와 파일별 hash를 함께 반환합니다. symlink 정책은 신뢰 저장소의 상태입니다.
pub fn project_library(
    snapshot: &crate::snapshot::ProjectSnapshot,
) -> Result<(ComponentLibrary, ComponentFiles), Error> {
    project_library_with(
        snapshot,
        external_access(snapshot.project_id(), snapshot.root()),
    )
}

/// 호출자가 이미 아는 신뢰 상태(예: 앱 창 세션)의 정책으로 원본을 읽습니다.
pub fn project_library_with(
    snapshot: &crate::snapshot::ProjectSnapshot,
    access: ExternalAccess,
) -> Result<(ComponentLibrary, ComponentFiles), Error> {
    let (library, files) = component_source::read_project_library(snapshot.root(), access)?;
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
            background: frame.background,
            file,
            hash,
        });
    }
    Ok(result)
}

/// 파일을 열 때 관측한 hash가 일치하면 root Frame 크기를 바꿉니다.
pub fn resize(file: &Path, expected: &str, size: Size2) -> Result<ComponentInfo, Error> {
    edit_root(file, expected, ComponentRootEdit::Size(size))
}

/// 파일을 열 때 관측한 hash로 단일 root의 크기·채우기를 조건부 저장합니다.
pub fn edit_root(
    file: &Path,
    expected: &str,
    edit: ComponentRootEdit,
) -> Result<ComponentInfo, Error> {
    let (_guard, hash) = lock_file(file, expected)?;
    let (source, hash) = component_source::edit_root(file, hash, edit)?;
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

type FileGuard = (Option<SaveLock>, SaveLock);

fn lock_file(file: &Path, expected: &str) -> Result<(FileGuard, ContentHash), Error> {
    let hash = expected
        .parse()
        .map_err(|_| Error::Invalid("원본 hash 형식".into()))?;
    let dir = file
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let project = project_guard(dir)?;
    Ok(((project, lock(dir)?), hash))
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

    #[cfg(unix)]
    #[test]
    fn project_symlink_reads_follow_one_trust_based_policy() {
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
        let read = |access| project_library_with(&snap, access).map(|(_, files)| files);
        // 프로젝트 안 별칭은 신뢰 여부와 무관하게 읽습니다.
        for access in [ExternalAccess::Allowed, ExternalAccess::MetadataOnly] {
            assert_eq!(read(access).unwrap()[&source.definition().id].1, hash);
        }
        // 이 임시 프로젝트는 신뢰 저장소에 없으므로 기본 조회는 미신뢰 정책입니다.
        assert_eq!(
            external_access(snap.project_id(), snap.root()),
            ExternalAccess::MetadataOnly
        );
        assert_eq!(query(&snap).unwrap().entries[0].hash, hash);

        // 프로젝트 밖 파일 target: 신뢰하면 읽고(명시 폴더 check와 같은 결과), 아니면 내용을 읽지 않습니다.
        let external = outside.path.join(file.file_name().unwrap());
        fs::rename(&file, &external).unwrap();
        symlink(&external, &file).unwrap();
        assert_eq!(
            read(ExternalAccess::Allowed).unwrap()[&source.definition().id].1,
            hash
        );
        assert_eq!(check(&folder).unwrap()[0].hash, hash);
        assert!(matches!(
            read(ExternalAccess::MetadataOnly),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(query(&snap), Err(Error::Invalid(_))));

        // 프로젝트 밖 폴더 target도 같은 정책입니다.
        fs::remove_file(&file).unwrap();
        fs::rename(&external, &file).unwrap();
        fs::remove_file(&folder).unwrap();
        let outer = outside.path.join("library");
        fs::rename(&dedicated, &outer).unwrap();
        symlink(&outer, &folder).unwrap();
        assert_eq!(read(ExternalAccess::Allowed).unwrap().len(), 1);
        assert!(matches!(
            read(ExternalAccess::MetadataOnly),
            Err(Error::Invalid(_))
        ));
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

        let folder = root.join("design/components");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("README.md"), "unrelated").unwrap();
        assert!(query(&snapshot).unwrap().entries.is_empty());
        // CLI 폴더 명령도 프로젝트 안에서는 저장 잠금과 chunk 검사를 거쳐 이중 원본을 만들지 않습니다.
        assert!(matches!(
            create(&folder, "Card".into(), id(2), id(3)),
            Err(Error::Unsupported(_))
        ));
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1);
        let source = ComponentSource::new(id(2), "Card".into(), id(3)).unwrap();
        let (file, hash) = component_source::create(&folder, &source).unwrap();
        let before = file_digest(root);
        assert!(matches!(query(&snapshot), Err(Error::Unsupported(_))));
        assert!(matches!(
            resize(&file, &hash.to_string(), Size2::new(4., 4.).unwrap()),
            Err(Error::Unsupported(_))
        ));
        assert_eq!(file_digest(root), before);
    }
}
