//! 외부 library 종단 시험 공용 helper (XV.3.4·XV.4·XV.5·XV.6.2·XV.7.3, ADR-0119).
//!
//! 실제 Git 저장소에 텍스트 원본(ADR-0088) library를 release하고, 제품 의존성 엔진(resolver·mirror·cache entry·
//! lock)과 제품 연결(`stellar_project::library_*`)로 사용 프로젝트를 만듭니다. 시험 안에서 판정을 다시 구현하지
//! 않습니다. 시험이 정하는 것은 fixture 내용(저장소·component·override)과 repository 비교 키 함수뿐입니다(제품
//! keyer는 privacy key 설정 범위라 시험 keyer를 씁니다).
#![allow(dead_code)]

/// 제품 library cache 경로 fixture(ADR-0127).
pub mod product;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::ComponentRef;
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::FiniteF32;
use stellar_dependencies::cache_entry::LibraryEntryStore;
use stellar_dependencies::cache_root::LibraryCacheRoot;
use stellar_dependencies::content_hash::ContentHashText;
use stellar_dependencies::declaration::{
    CommitId, DependencyDeclaration, LibraryId, RepositoryUrl,
};
use stellar_dependencies::git_refs::GitSelectorResolver;
use stellar_dependencies::graph::{DependencyGraph, DependencyNodeKey};
use stellar_dependencies::lock::LockDocument;
use stellar_dependencies::mirror::{MirrorCache, RepositoryKey, RepositoryKeyer};
use stellar_dependencies::resolver::resolve_dependency_graph;
use stellar_document::component::definition::{ComponentLibrary, ComponentPropertyId};
use stellar_document::component::drilling::DrillingSet;
use stellar_document::component::property::{ComponentProperty, PropertyVisibility};
use stellar_document::component::property_commands::PropertyCommand;
use stellar_document::context::fixtures::{literal, ComponentBuilder, SceneBuilder};
use stellar_document::expressions::types::{ExpressionType, TypeSpec};
use stellar_document::model::DocumentModel;
use stellar_git_porcelain::trust::GatedGitRunner;
use stellar_project::document_io::source_v4::project::{encode, write_new, TextProject};
use stellar_project::document_io::source_v4::token::{TokenSetSource, TokenSource};
use stellar_project::library_git::{StorageSha256, SystemGitCommands};
use stellar_project::lockfile_write::write_resolved_lockfile;
use stellar_project::manifest_source::{library_manifest_from_text, GitManifestSource};
use stellar_storage::chunk::id::ChunkId;
use stellar_storage::chunk::kinds::KIND_COMPONENT_LIBRARY;
use stellar_storage::chunk::planner::LogicalChunkState;
use stellar_storage::hash::ContentHash;
use stellar_storage::save::cancel::{run_save, CancelToken};
use stellar_storage::save::commit::SavedGenerationState;
use stellar_storage::save::plan::build_save_plan;

use crate::e2e_common::{git, test_home};

/// repository 비교 키의 SHA-256 앞 32 hex(시험 keyer).
#[derive(Clone, Copy)]
pub struct Sha256Keyer;

impl RepositoryKeyer for Sha256Keyer {
    fn repository_key(&self, repository: &RepositoryUrl) -> RepositoryKey {
        let digest = ContentHash::compute(repository.comparison_key().as_bytes());
        let hex: String = digest.digest().iter().map(|b| format!("{b:02x}")).collect();
        RepositoryKey::from_hex(&hex[..32]).expect("hex")
    }
}

/// 제품 SHA-256·시험 keyer로 연 entry store.
pub type Store = LibraryEntryStore<StorageSha256, Sha256Keyer>;

/// 시스템 Git 실행기(시험 HOME 환경).
pub fn commands() -> SystemGitCommands {
    test_home();
    SystemGitCommands(stellar_ops::git::system_git_runner().expect("시스템 Git"))
}

/// library id.
pub fn library_id(n: u64) -> LibraryId {
    format!("0195c530-3acd-7bc1-8414-{n:012}").parse().unwrap()
}

/// fixture StableId(공간 `space`).
pub fn sid(space: u64, n: u64) -> StableId {
    StableId::new_v7(5_000_000 + space * 1_000 + n, 0x0119, n)
}

/// `stellar.yaml` 텍스트(schema 4). `dependencies`: (이름, id, URL, tag).
pub fn manifest_text(
    id: LibraryId,
    name: &str,
    dependencies: &[(&str, LibraryId, &str, &str)],
) -> String {
    let mut text = format!(
        "schema_version: 4\nproject:\n  id: \"{id}\"\n  name: \"{name}\"\ntarget_profiles: []\n"
    );
    if dependencies.is_empty() {
        text.push_str("dependencies: {}\n");
    } else {
        text.push_str("dependencies:\n");
        for (dep, dep_id, url, tag) in dependencies {
            text.push_str(&format!(
                "  {dep}:\n    source: git\n    library_id: \"{dep_id}\"\n    repository: \"{url}\"\n    version: \"{tag}\"\n"
            ));
        }
    }
    text
}

/// property 하나의 선언.
#[derive(Debug, Clone)]
pub struct PropertySpec {
    /// property StableId.
    pub id: StableId,
    /// 이름.
    pub name: &'static str,
    /// 선언 타입.
    pub value_type: TypeSpec,
    /// default 값.
    pub default: TypedValue,
    /// 공개 여부.
    pub visibility: PropertyVisibility,
}

/// String 값.
pub fn text_value(s: &str) -> TypedValue {
    TypedValue::String(s.to_string())
}

/// Number 값.
pub fn number_value(n: f32) -> TypedValue {
    TypedValue::Number(FiniteF32::new(n).expect("finite"))
}

/// 공개 String property.
pub fn public_string(id: StableId, name: &'static str, default: &str) -> PropertySpec {
    PropertySpec {
        id,
        name,
        value_type: TypeSpec::required(ExpressionType::String),
        default: text_value(default),
        visibility: PropertyVisibility::Public,
    }
}

/// component 하나를 property와 함께 library에 등록합니다. property는 제품 command(`PropertyCommand::Create`)로 더합니다.
pub fn add_component(
    library: &mut ComponentLibrary,
    id: StableId,
    root: StableId,
    name: &str,
    properties: &[PropertySpec],
) {
    ComponentBuilder::new(id, name, root).register(library);
    let mut drilling = DrillingSet::new();
    for (n, spec) in properties.iter().enumerate() {
        let property = ComponentProperty::new(
            ComponentPropertyId::new(spec.id),
            spec.name.to_string(),
            spec.value_type,
            literal(9_000 + n as u64, spec.default.clone()),
            spec.visibility,
        )
        .expect("property");
        PropertyCommand::Create {
            component: ComponentRef::new(id),
            property,
            index: None,
        }
        .apply(library, &mut drilling)
        .expect("property 추가");
    }
}

/// library 저장소 하나(일반 작업 저장소, tag로 버전 표시).
pub struct LibraryRepo {
    /// 작업 폴더.
    pub dir: PathBuf,
    /// `file://` URL.
    pub url: String,
}

/// 빈 library 저장소를 만듭니다.
pub fn library_repo(gate: &GatedGitRunner, parent: &Path, name: &str) -> LibraryRepo {
    let dir = parent.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    git(gate, &dir, &["init", "-q", "-b", "main"]);
    LibraryRepo {
        url: format!("file://{}", dir.display()),
        dir,
    }
}

/// release 내용.
pub struct Release<'a> {
    /// `stellar.yaml`.
    pub manifest: &'a str,
    /// 텍스트 원본 문서(없으면 컴포넌트만 담은 빈 문서).
    pub document: Option<DocumentModel>,
    /// 컴포넌트 정의.
    pub components: ComponentLibrary,
    /// token set.
    pub token_sets: Vec<TokenSetSource>,
}

/// 빈 원본 문서(View 하나, root Frame).
pub fn empty_document(space: u64) -> DocumentModel {
    SceneBuilder::new(
        sid(space, 900),
        sid(space, 901),
        sid(space, 902),
        sid(space, 903),
    )
    .build()
}

/// release 하나를 commit하고 tag를 붙입니다.
///
/// `design/` 텍스트 원본은 제품 encoder로 다시 씁니다. 컴포넌트 정의는 Property·Slot을 텍스트 원본으로 아직 쓸 수
/// 없으므로(ADR-0088 결정 2) 제품 저장 경로(save plan → run_save)로 generation component library chunk에 씁니다.
pub fn release(
    gate: &GatedGitRunner,
    repo: &LibraryRepo,
    release: Release<'_>,
    tag: &str,
) -> CommitId {
    let manifest = library_manifest_from_text(release.manifest).expect("library stellar.yaml");
    std::fs::write(repo.dir.join("stellar.yaml"), release.manifest).unwrap();
    let design = repo.dir.join("design");
    let _ = std::fs::remove_dir_all(&design);
    let project = TextProject {
        document: release.document.unwrap_or_else(|| empty_document(77)),
        components: ComponentLibrary::new(),
        token_sets: release.token_sets,
        images: BTreeMap::new(),
    };
    let encoded = encode(&project).expect("텍스트 원본 encode");
    write_new(&design, &encoded).expect("텍스트 원본 쓰기");
    if !release.components.is_empty() {
        write_component_generation(&repo.dir, manifest.library_id, &release.components);
    }
    // materialize 대상이 아닌 파일(entry에 들어가지 않음).
    std::fs::write(repo.dir.join("CHANGELOG.md"), format!("{tag}\n")).unwrap();
    git(gate, &repo.dir, &["add", "-A"]);
    git(gate, &repo.dir, &["commit", "-q", "-m", tag]);
    git(gate, &repo.dir, &["tag", tag]);
    git(gate, &repo.dir, &["rev-parse", "HEAD"])
        .trim()
        .parse()
        .unwrap()
}

/// 컴포넌트 정의를 제품 저장 경로로 generation 1의 component library chunk에 씁니다.
pub fn write_component_generation(root: &Path, id: LibraryId, components: &ComponentLibrary) {
    let project_id = stellar_project::identity::parse_project_id(&id.to_string()).unwrap();
    let chunks = [LogicalChunkState::Dirty {
        chunk_id: ChunkId::from_stable_id(sid(0, 999)),
        kind: KIND_COMPONENT_LIBRARY,
        schema_version: 1,
        payload: components.encode(),
    }];
    let plan = build_save_plan(None, project_id, 1, &chunks, vec![]).expect("save plan");
    run_save(
        root,
        &plan,
        &mut SavedGenerationState::new(None),
        &CancelToken::new(),
    )
    .expect("generation 저장");
}

/// token set 하나(Color token 하나).
pub fn token_set(space: u64, name: &str, value: TypedValue) -> TokenSetSource {
    TokenSetSource {
        id: sid(space, 800),
        name: "Brand".into(),
        tokens: vec![TokenSource {
            id: sid(space, 801),
            name: name.into(),
            expression: sid(space, 802),
            value,
        }],
    }
}

/// 사용 프로젝트 선언(실제 `stellar.yaml` 파서).
pub fn declarations(text: &str) -> Vec<DependencyDeclaration> {
    library_manifest_from_text(text)
        .expect("사용 프로젝트 stellar.yaml")
        .dependencies
}

/// 시험 공용 cache root.
pub fn cache_root(name: &str) -> LibraryCacheRoot {
    LibraryCacheRoot::in_app_data(&test_home().join(format!("library-cache-{name}"))).unwrap()
}

/// 제품 resolver로 의존성 graph를 해석합니다(시스템 Git).
pub fn resolve(
    cache: &LibraryCacheRoot,
    work: &Path,
    root: &[DependencyDeclaration],
) -> DependencyGraph {
    let mut selectors = GitSelectorResolver::new(
        commands(),
        MirrorCache::new(cache.clone(), commands(), Sha256Keyer),
        work,
    );
    let mut manifests = GitManifestSource::new(commands(), cache.clone(), Sha256Keyer);
    resolve_dependency_graph(root, &mut selectors, &mut manifests).expect("의존성 해석")
}

/// graph의 모든 node를 cache entry로 materialize하고 lock을 만들어 `project_root`에 씁니다.
///
/// 반환: lock과 node별 entry 디렉터리.
pub fn materialize_and_lock(
    store: &mut Store,
    cache: &LibraryCacheRoot,
    graph: &DependencyGraph,
    project_root: &Path,
) -> (LockDocument, BTreeMap<DependencyNodeKey, PathBuf>) {
    let mut mirrors = MirrorCache::new(cache.clone(), commands(), Sha256Keyer);
    let mut runner = commands();
    let mut hashes: BTreeMap<DependencyNodeKey, ContentHashText> = BTreeMap::new();
    let mut dirs = BTreeMap::new();
    for entry in graph.sorted_entries() {
        if hashes.contains_key(&entry.key) {
            continue;
        }
        let created = store
            .materialize(
                &mut runner,
                &mut mirrors,
                &entry.repository,
                &entry.key.commit,
                1,
            )
            .expect("materialize");
        hashes.insert(entry.key.clone(), created.content_hash.clone());
        dirs.insert(entry.key.clone(), created.dir.clone());
    }
    let lock = LockDocument::from_graph(graph, &hashes).expect("lock");
    write_resolved_lockfile(project_root, &lock).expect("lock 기록");
    (lock, dirs)
}

/// 사용 프로젝트 폴더(일반 폴더 + `stellar.yaml`).
pub fn using_project(parent: &Path, manifest: &str) -> PathBuf {
    let dir = parent.join("app-project");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("stellar.yaml"), manifest).unwrap();
    dir
}

/// 읽기 전용(0444/0555) entry 파일을 변조 시험용으로 쓰기 가능하게 합니다.
pub fn make_writable(path: &Path) {
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(path, perms).unwrap();
}
