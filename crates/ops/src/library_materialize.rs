//! 외부 library 제품 명령: 의존성 추가·update·cache 받기·외부 component 참조 저장 (XV.4.3·XV.7.4·XV.7.5, ADR-0137).
//!
//! CLI `stellar library add|update|fetch|import`가 이 함수들을 그대로 부릅니다. 판정과 기록은 기존 공개 함수가 합니다.
//!
//! | 단계 | 함수 |
//! |---|---|
//! | 선택자 → exact commit | [`LockPreferringResolver`](lock commit 우선, XV.4.3) → `GitSelectorResolver`(시스템 Git, XV.2.4) |
//! | 전이 의존성 graph | `resolve_dependency_graph`(XV.2.2) + [`CachedManifestSource`](cache entry 먼저, 없으면 fetch한 mirror) |
//! | cache entry | `LibraryEntryStore::materialize`(XV.3.3, 같은 commit·같은 내용은 재사용) |
//! | entry 검증·export | `load_cached_snapshot` + `ProjectLibraryValidator::cache_entry`·`SourceExports`(XV.3.4·XV.5.1) |
//! | lock | `LockDocument::from_graph`(XV.4.1) → `write_resolved_lockfile`(XV.4.5, 바뀌지 않으면 쓰지 않음) |
//! | `stellar.yaml` | [`ManifestEdit`](읽은 bytes hash가 그대로일 때만 씀) |
//! | update 전 검증 | `check_candidate_usages`(XV.7.3, `LibraryUpdateCandidate::check_usages`와 같은 함수) |
//!
//! ## 원자성 (XV.7.4·XV.7.5)
//!
//! 모든 명령은 resolve·fetch·materialize·entry 검증·후보 검증·텍스트 생성을 **파일을 쓰기 전에** 끝냅니다. 그 가운데 하나라도
//! 실패하면(network·cache·schema 오류, orphan 거부) `stellar.yaml`과 `stellar.lock.yaml`은 바뀌지 않습니다. 공용 cache에는 새
//! entry가 남을 수 있지만 entry는 immutable·content-addressed라 이전 lock이 가리키는 entry를 바꾸지 않습니다. 쓰기는
//! `stellar.yaml` → lock 순서이고, lock 쓰기가 실패하면 `stellar.yaml`을 연 시점 bytes로 되돌립니다([`ManifestEdit::restore`]).
//!
//! 열린 창은 lock·cache가 바뀐 것을 보고 library를 다시 로드합니다(`stellar_runtime::library::WindowLibraries`,
//! [`crate::library_watch`]). instance override는 property StableId로 저장되어 있으므로 새 snapshot에서 같은 StableId로 다시
//! 해석됩니다(문서를 다시 쓰지 않음).
//!
//! ## network
//!
//! lock이 있고 선택자가 바뀌지 않은 의존성은 lock commit을 쓰고, 그 commit의 cache entry가 있으면 매니페스트·entry를 cache에서
//! 읽습니다. 이때 Git을 실행하지 않습니다(XV.4.3). cache에 없는 commit만 mirror를 fetch합니다(저장소마다 한 번).

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use stellar_dependencies::cache_entry::{EntryReuse, MaterializeError};
use stellar_dependencies::cache_root::LibraryCacheRoot;
use stellar_dependencies::candidate_check::{check_candidate_usages, CandidateDiagnostic};
use stellar_dependencies::content_hash::ContentHashText;
use stellar_dependencies::declaration::{
    CommitId, DependencyDeclaration, LogicalName, RepositoryUrl, SourceKind, VersionSelector,
};
use stellar_dependencies::git_refs::{FetchedRefs, GitSelectorResolver};
use stellar_dependencies::graph::{DependencyNodeKey, DependencyPath};
use stellar_dependencies::library_snapshot::{
    load_cached_snapshot, LibrarySnapshot, SnapshotLimits, SnapshotLoadError, SnapshotRequest,
};
use stellar_dependencies::lock::{LockBuildError, LockDocument};
use stellar_dependencies::lock_preference::LockPreferringResolver;
use stellar_dependencies::mirror::MirrorCache;
use stellar_dependencies::resolver::{
    resolve_dependency_graph, LibraryManifest, LibraryManifestSource, ManifestSourceError,
    ResolveError,
};
use stellar_dependencies::resource_diff::{diff_library_snapshots, ChangeKind};
use stellar_dependencies::selection::{SelectorError, SelectorResolver};
use stellar_dependencies::validation::validate_declarations;
use stellar_project::library_entry::ProjectLibraryValidator;
use stellar_project::library_exports::SourceExports;
use stellar_project::library_git::SystemGitCommands;
use stellar_project::library_open::{read_lock, LibraryOpenProblem};
use stellar_project::library_refs::external_instance_usages;
use stellar_project::library_resolve::{CachedManifestSource, ManifestEdit, ManifestEditError};
use stellar_project::lockfile_write::{
    write_resolved_lockfile, LockfileWriteError, LockfileWriteOutcome,
};
use stellar_project::manifest_source::GitManifestSource;

use crate::core_types::ids::StableId;
use crate::core_types::reference::ComponentRef;
use crate::library::{LibraryCache, LibraryCacheError, ProductKeyer};

/// 명령 입력·결과 타입 재수출(CLI·runtime은 의존성 crate에 직접 의존하지 않음, ADR-0127 결정 6).
pub use stellar_dependencies::cache_entry::EntryReuse as Reuse;
pub use stellar_dependencies::declaration::{
    CommitId as Commit, LogicalName as DependencyName, RefName, RepositoryUrl as Repository,
    VersionSelector as Selector,
};
pub use stellar_dependencies::node::ValueError as SelectorValueError;

/// 제품 명령이 쓰는 cache와 시스템 Git.
pub struct LibraryTools {
    cache: LibraryCache,
    git: SystemGitCommands,
}

impl fmt::Debug for LibraryTools {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LibraryTools(..)")
    }
}

impl LibraryTools {
    /// OS 사용자 데이터의 공용 cache(없으면 만듦)와 시스템 Git으로 엽니다.
    pub fn os_default() -> Result<LibraryTools, LibraryCommandError> {
        let cache = LibraryCache::create_os_default().map_err(LibraryCommandError::Cache)?;
        LibraryTools::with_cache(cache)
    }

    /// `app_data_dir` 아래 cache로 엽니다(시험·cache 위치를 아는 호출자).
    pub fn in_app_data(app_data_dir: &Path) -> Result<LibraryTools, LibraryCommandError> {
        let cache =
            LibraryCache::create_in_app_data(app_data_dir).map_err(LibraryCommandError::Cache)?;
        LibraryTools::with_cache(cache)
    }

    fn with_cache(cache: LibraryCache) -> Result<LibraryTools, LibraryCommandError> {
        let runner =
            crate::git::system_git_runner().map_err(|_| LibraryCommandError::GitUnavailable)?;
        Ok(LibraryTools {
            cache,
            git: SystemGitCommands(runner),
        })
    }

    /// cache.
    pub fn cache(&self) -> &LibraryCache {
        &self.cache
    }
}

/// 제품 명령 실패. URL·경로 원문은 담지 않습니다(XV.1.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryCommandError {
    /// 공용 cache를 열지 못함.
    Cache(LibraryCacheError),
    /// 공용 cache가 사용 프로젝트 안에 있음.
    CacheInsideProject,
    /// 시스템 Git이 없음.
    GitUnavailable,
    /// `stellar.yaml`을 읽거나 쓰지 못함.
    Manifest(ManifestEditError),
    /// `stellar.lock.yaml`을 읽지 못함.
    Lock(LibraryOpenProblem),
    /// lock이 없음(`fetch`는 lock이 필요).
    NotLocked,
    /// 같은 이름의 의존성이 이미 있음.
    DuplicateDependency {
        /// 이름.
        name: LogicalName,
    },
    /// 그 이름의 의존성이 없음.
    UnknownDependency {
        /// 이름.
        name: LogicalName,
    },
    /// 선언 충돌(XV.1.3 오류 심각도).
    Declarations {
        /// 충돌 설명.
        conflicts: Vec<String>,
    },
    /// 새 의존성의 선택자를 해석하지 못함.
    Selector(SelectorError),
    /// 새 의존성의 매니페스트를 읽지 못함.
    LibraryManifest(ManifestSourceError),
    /// 전이 의존성 해석 실패.
    Resolve(ResolveError),
    /// node 하나를 cache에 받지 못함.
    Materialize {
        /// 처음 도달한 경로.
        path: DependencyPath,
        /// 원인.
        error: MaterializeError,
    },
    /// 받은 entry가 올바른 library가 아니거나 export를 읽지 못함.
    Rejected {
        /// 처음 도달한 경로.
        path: DependencyPath,
        /// 원인.
        error: SnapshotLoadError,
    },
    /// lock content hash와 받은 entry hash가 다름(lock 무결성, XV.4.4).
    ContentHashMismatch {
        /// lock 경로.
        path: DependencyPath,
    },
    /// lock 생성 실패.
    LockBuild(LockBuildError),
    /// lock 쓰기 실패(`stellar.yaml`은 되돌림).
    LockWrite(LockfileWriteError),
    /// lock 쓰기 실패 뒤 `stellar.yaml`도 되돌리지 못함.
    RestoreFailed {
        /// lock 쓰기 원인.
        lock: LockfileWriteError,
        /// 되돌리기 원인.
        manifest: ManifestEditError,
    },
    /// 의존성 library가 export하지 않는 component.
    UnknownComponents {
        /// 의존성 이름.
        name: LogicalName,
        /// export되지 않은 component(정렬).
        components: Vec<StableId>,
    },
    /// 의존성 library를 cache에서 읽을 수 없음(`stellar library fetch` 필요).
    LibraryUnavailable {
        /// 의존성 이름.
        name: LogicalName,
    },
    /// 후보를 적용하면 사용 프로젝트 참조가 끊기거나, 이 의존성의 것일 수 있는 모호한 instance를 검증하지 못함(`allow_orphans`
    /// 없이 거부, fail closed).
    OrphansBlocked {
        /// orphan 진단(정렬).
        diagnostics: Vec<CandidateDiagnostic>,
        /// 이 의존성이 후보에 들어 있어 검증하지 못한 모호한 instance(정렬).
        ambiguous: Vec<StableId>,
    },
}

impl fmt::Display for LibraryCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LibraryCommandError::Cache(e) => write!(f, "{e}"),
            LibraryCommandError::CacheInsideProject => {
                f.write_str("공용 library cache가 사용 프로젝트 안에 있습니다")
            }
            LibraryCommandError::GitUnavailable => f.write_str("시스템 Git을 찾지 못했습니다"),
            LibraryCommandError::Manifest(e) => write!(f, "{e}"),
            LibraryCommandError::Lock(e) => write!(f, "{e}"),
            LibraryCommandError::NotLocked => {
                f.write_str("stellar.lock.yaml이 없습니다(stellar library update로 먼저 해석하세요)")
            }
            LibraryCommandError::DuplicateDependency { name } => {
                write!(f, "의존성 {name}이 이미 stellar.yaml에 있습니다")
            }
            LibraryCommandError::UnknownDependency { name } => {
                write!(f, "stellar.yaml에 의존성 {name}이 없습니다")
            }
            LibraryCommandError::Declarations { conflicts } => {
                write!(f, "의존성 선언 충돌: {}", conflicts.join("; "))
            }
            LibraryCommandError::Selector(e) => write!(f, "선택자 해석 실패: {e}"),
            LibraryCommandError::LibraryManifest(e) => write!(f, "library 매니페스트: {e}"),
            LibraryCommandError::Resolve(e) => write!(f, "의존성 해석 실패: {e}"),
            LibraryCommandError::Materialize { path, error } => {
                write!(f, "{path}: library를 받지 못했습니다: {error}")
            }
            LibraryCommandError::Rejected { path, error } => {
                write!(f, "{path}: library를 사용할 수 없습니다: {error}")
            }
            LibraryCommandError::ContentHashMismatch { path } => write!(
                f,
                "{path}: 받은 library 내용이 stellar.lock.yaml content hash와 다릅니다(lock 손상)"
            ),
            LibraryCommandError::LockBuild(e) => write!(f, "{e}"),
            LibraryCommandError::LockWrite(e) => {
                write!(f, "{e} (stellar.yaml은 이전 상태로 되돌렸습니다)")
            }
            LibraryCommandError::RestoreFailed { lock, manifest } => write!(
                f,
                "{lock}; stellar.yaml도 되돌리지 못했습니다({manifest}). 두 파일을 확인하세요"
            ),
            LibraryCommandError::UnknownComponents { name, components } => {
                let ids: Vec<String> = components.iter().map(ToString::to_string).collect();
                write!(
                    f,
                    "의존성 {name}의 library가 export하지 않는 component입니다: {}",
                    ids.join(", ")
                )
            }
            LibraryCommandError::LibraryUnavailable { name } => write!(
                f,
                "의존성 {name}의 library를 cache에서 읽을 수 없습니다(stellar library fetch로 받으세요)"
            ),
            LibraryCommandError::OrphansBlocked {
                diagnostics,
                ambiguous,
            } => {
                write!(
                    f,
                    "update하면 사용 프로젝트 참조 {}개가 끊깁니다",
                    diagnostics.len()
                )?;
                if !ambiguous.is_empty() {
                    write!(
                        f,
                        ", 어느 의존성의 것인지 정할 수 없어 검증하지 못한 instance가 {}개입니다",
                        ambiguous.len()
                    )?;
                }
                f.write_str("(확인했으면 --allow-orphans)")
            }
        }
    }
}

impl std::error::Error for LibraryCommandError {}

/// node 하나를 받은 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeReport {
    /// 처음 도달한 dependency path.
    pub path: DependencyPath,
    /// node.
    pub node: DependencyNodeKey,
    /// entry content hash.
    pub content_hash: ContentHashText,
    /// cache 재사용 방식.
    pub reuse: EntryReuse,
}

/// lock을 다시 만든 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockReport {
    /// node별 결과(lock 순서).
    pub nodes: Vec<NodeReport>,
    /// lock commit으로 답한 선택자 해석 수(Git 없음, XV.4.3).
    pub pinned: usize,
    /// cache entry에서 읽은 매니페스트 수(Git 없음).
    pub manifests_from_cache: usize,
    /// `stellar.yaml`을 바꿨는지.
    pub manifest_written: bool,
    /// lock을 바꿨는지(같으면 쓰지 않음).
    pub lock_written: bool,
}

/// `stellar library add` 요청.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddRequest {
    /// 의존성 이름.
    pub name: LogicalName,
    /// repository.
    pub repository: RepositoryUrl,
    /// 선택자.
    pub version: VersionSelector,
    /// 함께 저장할 외부 component 참조.
    pub components: BTreeSet<ComponentRef>,
}

/// `stellar library update` 요청.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateRequest {
    /// 의존성 이름.
    pub name: LogicalName,
    /// 새 선택자(없으면 선언 선택자를 다시 해석: branch 이동 확인).
    pub version: Option<VersionSelector>,
    /// orphan 진단이 있어도 적용합니다(사용자 승인).
    pub allow_orphans: bool,
    /// 검토만 하고 쓰지 않습니다.
    pub dry_run: bool,
}

/// resource diff 개수.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiffCounts {
    /// 추가.
    pub added: usize,
    /// 삭제.
    pub removed: usize,
    /// 변경.
    pub modified: usize,
}

/// `stellar library update` 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateReport {
    /// 의존성 이름.
    pub name: LogicalName,
    /// 이전 lock commit(lock이 없었으면 `None`).
    pub previous: Option<CommitId>,
    /// 새 commit.
    pub candidate: CommitId,
    /// 이전 → 새 snapshot resource diff 개수(이전 snapshot을 cache에서 읽을 수 없으면 `None`).
    pub diff: Option<DiffCounts>,
    /// 사용 프로젝트 참조 검증 결과(XV.7.3).
    pub orphans: Vec<CandidateDiagnostic>,
    /// 이 의존성이 후보 선언에 들어 있지만 어느 의존성인지 정할 수 없어 검증에서 빠진 instance. 있으면 orphan처럼 승인 없이는
    /// 적용하지 않습니다(fail closed).
    pub ambiguous: Vec<StableId>,
    /// 적용했는지(dry-run이면 `false`).
    pub applied: bool,
    /// lock 결과.
    pub lock: LockReport,
}

/// `stellar library fetch` 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchReport {
    /// node별 결과(lock 순서).
    pub nodes: Vec<NodeReport>,
}

/// `stellar library import` 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    /// 의존성 이름.
    pub name: LogicalName,
    /// 저장 뒤 그 의존성의 외부 component 참조(정렬).
    pub components: Vec<StableId>,
    /// `stellar.yaml`을 바꿨는지.
    pub manifest_written: bool,
}

// ───────────────────────────── 공용 단계 ─────────────────────────────

/// 한 명령 동안 선택자 해석·매니페스트·materialize가 함께 쓰는 mirror(저장소마다 fetch 한 번, XV.3.2).
struct SharedMirrors<'a>(&'a RefCell<MirrorCache<SystemGitCommands, ProductKeyer>>);

impl FetchedRefs for SharedMirrors<'_> {
    fn fetched_git_dir(&mut self, repository: &RepositoryUrl) -> Result<PathBuf, SelectorError> {
        self.0.borrow_mut().fetched_git_dir(repository)
    }
}

/// mirror에서 매니페스트를 읽되, mirror가 없거나 commit이 없으면 한 번 fetch하고 다시 읽습니다.
struct FetchingManifests<'a> {
    git: GitManifestSource<SystemGitCommands, ProductKeyer>,
    mirrors: SharedMirrors<'a>,
}

impl LibraryManifestSource for FetchingManifests<'_> {
    fn manifest_at(
        &mut self,
        repository: &RepositoryUrl,
        commit: &CommitId,
    ) -> Result<LibraryManifest, ManifestSourceError> {
        match self.git.manifest_at(repository, commit) {
            Err(ManifestSourceError::Unavailable) => {
                self.mirrors
                    .fetched_git_dir(repository)
                    .map_err(|_| ManifestSourceError::Unavailable)?;
                self.git.manifest_at(repository, commit)
            }
            other => other,
        }
    }
}

/// resolve·materialize·검증까지 끝낸 결과(파일은 아직 쓰지 않음).
struct Prepared {
    lock: LockDocument,
    nodes: Vec<NodeReport>,
    snapshots: BTreeMap<DependencyNodeKey, LibrarySnapshot>,
    pinned: usize,
    manifests_from_cache: usize,
}

/// 선택자 하나를 해석하고 그 commit의 매니페스트를 읽습니다(새 의존성의 library id 확인).
fn probe_library(
    tools: &LibraryTools,
    mirrors: &RefCell<MirrorCache<SystemGitCommands, ProductKeyer>>,
    repository: &RepositoryUrl,
    version: &VersionSelector,
) -> Result<LibraryManifest, LibraryCommandError> {
    let root = tools.cache.root().clone();
    let mut selectors =
        GitSelectorResolver::new(tools.git.clone(), SharedMirrors(mirrors), root.path());
    let commit = selectors
        .resolve(repository, version)
        .map_err(LibraryCommandError::Selector)?;
    let mut manifests = CachedManifestSource::new(
        tools.cache.store(),
        FetchingManifests {
            git: GitManifestSource::new(tools.git.clone(), root, tools.cache.keyer().clone()),
            mirrors: SharedMirrors(mirrors),
        },
    );
    manifests
        .manifest_at(repository, &commit)
        .map_err(LibraryCommandError::LibraryManifest)
}

/// 선언으로 graph를 해석하고 모든 node를 받아 검증합니다. `refresh`는 lock commit을 쓰지 않을 선언입니다.
fn prepare(
    tools: &mut LibraryTools,
    mirrors: &RefCell<MirrorCache<SystemGitCommands, ProductKeyer>>,
    declarations: &[DependencyDeclaration],
    previous: Option<&LockDocument>,
    refresh: Option<&DependencyDeclaration>,
    now_unix_ms: u64,
) -> Result<Prepared, LibraryCommandError> {
    let report = validate_declarations(declarations);
    if report.has_errors() {
        return Err(LibraryCommandError::Declarations {
            conflicts: report.errors().map(ToString::to_string).collect(),
        });
    }
    let root: LibraryCacheRoot = tools.cache.root().clone();
    let (graph, pinned, manifests_from_cache) = {
        let inner =
            GitSelectorResolver::new(tools.git.clone(), SharedMirrors(mirrors), root.path());
        let mut selectors = LockPreferringResolver::new(previous, inner);
        if let Some(declaration) = refresh {
            selectors = selectors.refresh(&declaration.repository, &declaration.version);
        }
        let mut manifests = CachedManifestSource::new(
            tools.cache.store(),
            FetchingManifests {
                git: GitManifestSource::new(
                    tools.git.clone(),
                    root.clone(),
                    tools.cache.keyer().clone(),
                ),
                mirrors: SharedMirrors(mirrors),
            },
        );
        let graph = resolve_dependency_graph(declarations, &mut selectors, &mut manifests)
            .map_err(LibraryCommandError::Resolve)?;
        (graph, selectors.pinned_hits(), manifests.cache_hits())
    };

    let mut hashes: BTreeMap<DependencyNodeKey, ContentHashText> = BTreeMap::new();
    let mut nodes = Vec::new();
    let mut runner = tools.git.clone();
    for entry in graph.sorted_entries() {
        if hashes.contains_key(&entry.key) {
            continue;
        }
        let materialized = tools
            .cache
            .store_mut()
            .materialize(
                &mut runner,
                &mut SharedMirrors(mirrors),
                &entry.repository,
                &entry.key.commit,
                now_unix_ms,
            )
            .map_err(|error| LibraryCommandError::Materialize {
                path: entry.path.clone(),
                error,
            })?;
        hashes.insert(entry.key.clone(), materialized.content_hash.clone());
        nodes.push(NodeReport {
            path: entry.path.clone(),
            node: entry.key.clone(),
            content_hash: materialized.content_hash,
            reuse: materialized.reuse,
        });
    }
    let snapshots = validate_nodes(tools, &graph_repositories(&graph), &nodes, now_unix_ms)?;
    let lock = LockDocument::from_graph(&graph, &hashes).map_err(LibraryCommandError::LockBuild)?;
    Ok(Prepared {
        lock,
        nodes,
        snapshots,
        pinned,
        manifests_from_cache,
    })
}

/// node → 처음 도달한 entry의 repository.
fn graph_repositories(
    graph: &stellar_dependencies::graph::DependencyGraph,
) -> BTreeMap<DependencyNodeKey, RepositoryUrl> {
    let mut out = BTreeMap::new();
    for entry in graph.sorted_entries() {
        out.entry(entry.key).or_insert(entry.repository);
    }
    out
}

/// 받은 entry를 제품 검증기·export 공급자로 읽어 snapshot으로 만듭니다(XV.3.4·XV.5.1).
fn validate_nodes(
    tools: &LibraryTools,
    repositories: &BTreeMap<DependencyNodeKey, RepositoryUrl>,
    nodes: &[NodeReport],
    now_unix_ms: u64,
) -> Result<BTreeMap<DependencyNodeKey, LibrarySnapshot>, LibraryCommandError> {
    let mut validator = ProjectLibraryValidator::cache_entry();
    let mut exports = SourceExports;
    let mut snapshots = BTreeMap::new();
    for node in nodes {
        let Some(repository) = repositories.get(&node.node) else {
            continue;
        };
        let snapshot = load_cached_snapshot(
            tools.cache.store(),
            SnapshotRequest {
                repository,
                node: &node.node,
                expected_content_hash: Some(&node.content_hash),
            },
            &mut validator,
            &mut exports,
            &SnapshotLimits::default(),
            now_unix_ms,
        )
        .map_err(|error| LibraryCommandError::Rejected {
            path: node.path.clone(),
            error,
        })?;
        snapshots.insert(node.node.clone(), snapshot);
    }
    Ok(snapshots)
}

fn read_previous_lock(root: &Path) -> Result<Option<LockDocument>, LibraryCommandError> {
    read_lock(root).map_err(LibraryCommandError::Lock)
}

fn check_cache_location(tools: &LibraryTools, root: &Path) -> Result<(), LibraryCommandError> {
    tools
        .cache
        .root()
        .ensure_outside_project(root)
        .map_err(|_| LibraryCommandError::CacheInsideProject)
}

fn new_mirrors(tools: &LibraryTools) -> RefCell<MirrorCache<SystemGitCommands, ProductKeyer>> {
    RefCell::new(MirrorCache::new(
        tools.cache.root().clone(),
        tools.git.clone(),
        tools.cache.keyer().clone(),
    ))
}

/// root 선언의 snapshot(그 이름의 root lock entry node).
fn root_snapshot<'a>(prepared: &'a Prepared, name: &LogicalName) -> Option<&'a LibrarySnapshot> {
    let path = DependencyPath::from_names(vec![name.clone()]);
    prepared
        .lock
        .entries()
        .iter()
        .find(|e| e.dependency_path == path)
        .and_then(|e| prepared.snapshots.get(&e.key()))
}

/// `components` 가운데 snapshot이 export하지 않는 것.
fn unknown_components(
    snapshot: &LibrarySnapshot,
    components: &BTreeSet<ComponentRef>,
) -> Vec<StableId> {
    components
        .iter()
        .filter(|c| snapshot.component(**c).is_none())
        .map(|c| c.id())
        .collect()
}

/// `stellar.yaml`(바뀌었으면) → lock 순서로 씁니다. lock 쓰기가 실패하면 `stellar.yaml`을 되돌립니다(모듈 문서).
fn commit_files(
    root: &Path,
    edit: &ManifestEdit,
    lock: &LockDocument,
) -> Result<(bool, bool), LibraryCommandError> {
    let manifest = edit.encoded().map_err(LibraryCommandError::Manifest)?;
    if let Some(bytes) = &manifest {
        edit.write(root, bytes)
            .map_err(LibraryCommandError::Manifest)?;
    }
    match write_resolved_lockfile(root, lock) {
        Ok(outcome) => Ok((manifest.is_some(), outcome == LockfileWriteOutcome::Written)),
        Err(lock_error) => match &manifest {
            Some(bytes) => match edit.restore(root, bytes) {
                Ok(()) => Err(LibraryCommandError::LockWrite(lock_error)),
                Err(manifest_error) => Err(LibraryCommandError::RestoreFailed {
                    lock: lock_error,
                    manifest: manifest_error,
                }),
            },
            None => Err(LibraryCommandError::LockWrite(lock_error)),
        },
    }
}

// ───────────────────────────── 명령 ─────────────────────────────

/// 의존성을 `stellar.yaml`에 더하고 lock·cache를 갱신합니다. library id는 해석한 commit의 매니페스트(`project.id`)입니다.
/// `components`가 있으면 새 library가 그 component를 모두 export해야 합니다.
pub fn add(
    root: &Path,
    tools: &mut LibraryTools,
    request: &AddRequest,
    now_unix_ms: u64,
) -> Result<LockReport, LibraryCommandError> {
    check_cache_location(tools, root)?;
    let mut edit = ManifestEdit::open(root).map_err(LibraryCommandError::Manifest)?;
    if edit.declarations().iter().any(|d| d.name == request.name) {
        return Err(LibraryCommandError::DuplicateDependency {
            name: request.name.clone(),
        });
    }
    let previous = read_previous_lock(root)?;
    let mirrors = new_mirrors(tools);
    let library = probe_library(tools, &mirrors, &request.repository, &request.version)?;
    edit.declarations_mut().push(DependencyDeclaration {
        name: request.name.clone(),
        library_id: library.library_id,
        source: SourceKind::Git,
        repository: request.repository.clone(),
        version: request.version.clone(),
        local_override: None,
        components: request.components.clone(),
    });
    let declarations = edit.declarations().to_vec();
    let prepared = prepare(
        tools,
        &mirrors,
        &declarations,
        previous.as_ref(),
        None,
        now_unix_ms,
    )?;
    if !request.components.is_empty() {
        let snapshot = root_snapshot(&prepared, &request.name).ok_or_else(|| {
            LibraryCommandError::LibraryUnavailable {
                name: request.name.clone(),
            }
        })?;
        let unknown = unknown_components(snapshot, &request.components);
        if !unknown.is_empty() {
            return Err(LibraryCommandError::UnknownComponents {
                name: request.name.clone(),
                components: unknown,
            });
        }
    }
    let (manifest_written, lock_written) = commit_files(root, &edit, &prepared.lock)?;
    Ok(LockReport {
        nodes: prepared.nodes,
        pinned: prepared.pinned,
        manifests_from_cache: prepared.manifests_from_cache,
        manifest_written,
        lock_written,
    })
}

/// 의존성 하나의 선택자를 다시 해석(또는 바꿔)하고, 사용 프로젝트 참조를 후보에 대어 본 뒤 lock·`stellar.yaml`을 갱신합니다.
///
/// 다른 의존성은 lock commit을 그대로 씁니다(XV.4.3). orphan 진단이 있으면 `allow_orphans` 없이 거부하고 아무것도 쓰지
/// 않습니다. `dry_run`이면 검토 결과만 돌려줍니다.
pub fn update(
    root: &Path,
    tools: &mut LibraryTools,
    request: &UpdateRequest,
    now_unix_ms: u64,
) -> Result<UpdateReport, LibraryCommandError> {
    check_cache_location(tools, root)?;
    let mut edit = ManifestEdit::open(root).map_err(LibraryCommandError::Manifest)?;
    let Some(position) = edit
        .declarations()
        .iter()
        .position(|d| d.name == request.name)
    else {
        return Err(LibraryCommandError::UnknownDependency {
            name: request.name.clone(),
        });
    };
    if let Some(version) = &request.version {
        edit.declarations_mut()[position].version = version.clone();
    }
    let declaration = edit.declarations()[position].clone();
    let previous = read_previous_lock(root)?;
    let root_path = DependencyPath::from_names(vec![request.name.clone()]);
    let previous_entry = previous
        .as_ref()
        .and_then(|lock| {
            lock.entries()
                .iter()
                .find(|e| e.dependency_path == root_path)
        })
        .cloned();
    let declarations = edit.declarations().to_vec();
    let mirrors = new_mirrors(tools);
    let prepared = prepare(
        tools,
        &mirrors,
        &declarations,
        previous.as_ref(),
        Some(&declaration),
        now_unix_ms,
    )?;
    let candidate = root_snapshot(&prepared, &request.name)
        .cloned()
        .ok_or_else(|| LibraryCommandError::LibraryUnavailable {
            name: request.name.clone(),
        })?;

    // 이전 snapshot(있으면)과 resource diff 개수(XV.7.2).
    let current = previous_entry.as_ref().and_then(|entry| {
        load_cached_snapshot(
            tools.cache.store(),
            SnapshotRequest {
                repository: &entry.repository,
                node: &entry.key(),
                expected_content_hash: Some(&entry.content_hash),
            },
            &mut ProjectLibraryValidator::cache_entry(),
            &mut SourceExports,
            &SnapshotLimits::default(),
            now_unix_ms,
        )
        .ok()
    });
    let diff = current
        .as_ref()
        .and_then(|current| diff_library_snapshots(current, &candidate).ok())
        .map(|diff| {
            let mut counts = DiffCounts::default();
            for change in &diff.changes {
                match change.kind {
                    ChangeKind::Added => counts.added += 1,
                    ChangeKind::Removed => counts.removed += 1,
                    ChangeKind::Modified(_) => counts.modified += 1,
                }
            }
            counts
        });

    // 사용 프로젝트의 외부 instance·override를 후보에 대어 봅니다(XV.7.3). 출처 판정은 지금 열린 library 기준입니다.
    let (orphans, ambiguous) =
        check_project_usages(root, &tools.cache, &request.name, &candidate, now_unix_ms);
    // 이 의존성의 것일 수 있는 모호한 instance는 검증하지 못했으므로 끊기는 참조와 같이 승인을 요구합니다(fail closed).
    let blocked = !orphans.is_empty() || !ambiguous.is_empty();
    let applied = !request.dry_run && (!blocked || request.allow_orphans);
    if !request.dry_run && blocked && !request.allow_orphans {
        return Err(LibraryCommandError::OrphansBlocked {
            diagnostics: orphans,
            ambiguous,
        });
    }
    let (manifest_written, lock_written) = if applied {
        commit_files(root, &edit, &prepared.lock)?
    } else {
        (false, false)
    };
    Ok(UpdateReport {
        name: request.name.clone(),
        previous: previous_entry.map(|e| e.exact_commit),
        candidate: candidate.commit().clone(),
        diff,
        orphans,
        ambiguous,
        applied,
        lock: LockReport {
            nodes: prepared.nodes,
            pinned: prepared.pinned,
            manifests_from_cache: prepared.manifests_from_cache,
            manifest_written,
            lock_written,
        },
    })
}

/// 저장된 모든 문서의 외부 instance 사용을 `dependency`의 후보 snapshot에 대어 봅니다. 출처는 지금 lock과 명령의 cache로 연
/// library와 저장된 참조로 정합니다(`classify_component`, 진단·사전 검사와 같은 판정). 모호한 instance는 후보 선언 이름에
/// `dependency`가 있는 것만 돌려줍니다(다른 의존성끼리의 모호함은 이 update와 무관).
fn check_project_usages(
    root: &Path,
    cache: &LibraryCache,
    dependency: &LogicalName,
    candidate: &LibrarySnapshot,
    now_unix_ms: u64,
) -> (Vec<CandidateDiagnostic>, Vec<StableId>) {
    let Ok(snapshot) = crate::snapshot::open_snapshot(root) else {
        // 문서를 열 수 없는 프로젝트(문서 없음 포함)는 검증할 사용이 없습니다.
        return (Vec::new(), Vec::new());
    };
    let own = crate::component_source::project_library(&snapshot)
        .map(|(library, _)| library)
        .unwrap_or_default();
    let libraries = crate::library::open_libraries(root, Ok(cache), now_unix_ms).instance();
    let mut orphans = Vec::new();
    let mut ambiguous = Vec::new();
    for definition in snapshot.documents() {
        let Ok(model) = snapshot.document_model(definition.id) else {
            continue;
        };
        let usages = external_instance_usages(&model, &own, &libraries);
        let check = check_candidate_usages(dependency, candidate, &usages.instances, &[]);
        orphans.extend(check.diagnostics().iter().cloned());
        ambiguous.extend(
            usages
                .ambiguous
                .into_iter()
                .filter(|(_, names)| names.contains(dependency))
                .map(|(id, _)| id),
        );
    }
    orphans.sort();
    ambiguous.sort();
    (orphans, ambiguous)
}

/// lock의 모든 node를 공용 cache에 받습니다(lock·`stellar.yaml`은 바꾸지 않음). cache에 있는 commit은 Git을 실행하지 않습니다.
/// 받은 entry의 content hash가 lock 값과 다르면 실패합니다(lock 무결성, XV.4.4).
pub fn fetch(
    root: &Path,
    tools: &mut LibraryTools,
    now_unix_ms: u64,
) -> Result<FetchReport, LibraryCommandError> {
    check_cache_location(tools, root)?;
    let lock = read_previous_lock(root)?.ok_or(LibraryCommandError::NotLocked)?;
    let mirrors = new_mirrors(tools);
    let mut runner = tools.git.clone();
    let mut nodes: Vec<NodeReport> = Vec::new();
    let mut repositories = BTreeMap::new();
    for entry in lock.entries() {
        let node = entry.key();
        if nodes.iter().any(|n| n.node == node) {
            continue;
        }
        let materialized = tools
            .cache
            .store_mut()
            .materialize(
                &mut runner,
                &mut SharedMirrors(&mirrors),
                &entry.repository,
                &entry.exact_commit,
                now_unix_ms,
            )
            .map_err(|error| LibraryCommandError::Materialize {
                path: entry.dependency_path.clone(),
                error,
            })?;
        if materialized.content_hash != entry.content_hash {
            return Err(LibraryCommandError::ContentHashMismatch {
                path: entry.dependency_path.clone(),
            });
        }
        repositories.insert(node.clone(), entry.repository.clone());
        nodes.push(NodeReport {
            path: entry.dependency_path.clone(),
            node,
            content_hash: materialized.content_hash,
            reuse: materialized.reuse,
        });
    }
    validate_nodes(tools, &repositories, &nodes, now_unix_ms)?;
    Ok(FetchReport { nodes })
}

/// 의존성 하나에 외부 component 참조를 저장합니다. 저장할 component는 lock 기준으로 `cache`에서 로드한 그 의존성의 library가
/// export해야 합니다(cache에 없으면 실패, Git 없음). lock은 바꾸지 않습니다.
pub fn import(
    root: &Path,
    cache: &LibraryCache,
    name: &LogicalName,
    components: &BTreeSet<ComponentRef>,
    now_unix_ms: u64,
) -> Result<ImportReport, LibraryCommandError> {
    let mut edit = ManifestEdit::open(root).map_err(LibraryCommandError::Manifest)?;
    let position = declaration_position(&edit, name)?;
    let opened = crate::library::open_libraries(root, Ok(cache), now_unix_ms);
    let instance = opened.instance();
    let snapshot = instance
        .scopes()
        .child(
            &stellar_dependencies::ref_resolution::RefScope::Project,
            name,
        )
        .and_then(|node| instance.snapshot(node))
        .ok_or_else(|| LibraryCommandError::LibraryUnavailable { name: name.clone() })?;
    let unknown = unknown_components(snapshot, components);
    if !unknown.is_empty() {
        return Err(LibraryCommandError::UnknownComponents {
            name: name.clone(),
            components: unknown,
        });
    }
    edit.declarations_mut()[position]
        .components
        .extend(components.iter().copied());
    write_imports(root, &edit, position, name)
}

/// 의존성 하나에서 외부 component 참조를 지웁니다(library를 읽지 않음). 없는 참조는 무시합니다.
pub fn remove_imports(
    root: &Path,
    name: &LogicalName,
    components: &BTreeSet<ComponentRef>,
) -> Result<ImportReport, LibraryCommandError> {
    let mut edit = ManifestEdit::open(root).map_err(LibraryCommandError::Manifest)?;
    let position = declaration_position(&edit, name)?;
    for component in components {
        edit.declarations_mut()[position]
            .components
            .remove(component);
    }
    write_imports(root, &edit, position, name)
}

fn declaration_position(
    edit: &ManifestEdit,
    name: &LogicalName,
) -> Result<usize, LibraryCommandError> {
    edit.declarations()
        .iter()
        .position(|d| &d.name == name)
        .ok_or_else(|| LibraryCommandError::UnknownDependency { name: name.clone() })
}

fn write_imports(
    root: &Path,
    edit: &ManifestEdit,
    position: usize,
    name: &LogicalName,
) -> Result<ImportReport, LibraryCommandError> {
    let bytes = edit.encoded().map_err(LibraryCommandError::Manifest)?;
    if let Some(bytes) = &bytes {
        edit.write(root, bytes)
            .map_err(LibraryCommandError::Manifest)?;
    }
    Ok(ImportReport {
        name: name.clone(),
        components: edit.declarations()[position]
            .components
            .iter()
            .map(|c| c.id())
            .collect(),
        manifest_written: bytes.is_some(),
    })
}

#[cfg(test)]
#[path = "library_materialize_tests.rs"]
mod tests;
