//! lock 기준 library 로드와 부분 열기 (XV.5.5).
//!
//! 프로젝트를 열 때 `stellar.lock.yaml`의 모든 node를 공용 cache에서 읽기 전용 snapshot으로 로드합니다.
//! Git·network를 쓰지 않습니다. cache에 entry가 없거나(offline·미 materialize·정리됨) 읽을 수 없는 node는
//! **오류가 아니라** [`LibraryUnavailableDiagnostic`]으로 남기고, 나머지 library와 프로젝트 기본 데이터는
//! 그대로 엽니다(부분 열기, 아키텍처 30.1 "오프라인에서는 캐시 사용").
//!
//! 사용할 수 없는 library를 가리키는 참조는 [`ResourceResolution::Unavailable`]이 되고, 문서 쪽에는 원래
//! 참조 전체(선언 이름·LibraryId·종류·StableId)와 발견 위치를 보존한 [`UnresolvedExternalRef`]로 남깁니다.
//! core-types `UnresolvedRef`(III.2.4)는 선언 이름과 LibraryId를 담지 못해 cache가 돌아온 뒤 원래
//! 의존성에 다시 연결할 수 없으므로 문서 저장에 쓰지 않습니다(진단 집계용 변환만 제공). 참조를 지우거나
//! 다른 버전으로 바꾸지 않습니다. 오류·진단 문자열에는 repository URL을 넣지 않습니다(XV.1.4).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::property::PropertyPath;
use stellar_core_types::reference::{ExpectedRefType, UnresolvedRef};

use crate::cache_entry::{EntryCorruption, LibraryEntryStore, Sha256Digest};
use crate::entry_validation::LibraryProjectValidator;
use crate::external_ref::ExternalResourceRef;
use crate::graph::{DependencyNodeKey, DependencyPath};
use crate::library_snapshot::{
    load_cached_snapshot, ExportReadFailure, LibraryExportSource, SnapshotError, SnapshotLimits,
    SnapshotLoadError, SnapshotRequest,
};
use crate::lock::LockDocument;
use crate::mirror::RepositoryKeyer;
use crate::ref_resolution::{
    DependencyScopes, LibraryGraphInstance, RefResolutionError, RefScope, ResolvedTarget,
    ScopeError,
};

/// library를 사용할 수 없는 이유.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnavailableReason {
    /// cache에 entry가 없음(offline·미 materialize·정리됨).
    MissingCacheEntry,
    /// cache entry content hash가 lock 값과 다르거나, 디스크 bytes에서 다시 계산한 hash가 entry 주소(= lock 값)와
    /// 다름(손상, XV.4.4). 다른 commit을 고르지 않습니다.
    ContentHashMismatch,
    /// 같은 node의 lock entry들이 content hash·schema version·전이 의존성에서 서로 다름(손상).
    InconsistentLockEntries,
    /// cache index·lock 읽기 실패.
    CacheFailure,
    /// export 읽기 실패.
    ExportUnreadable(ExportReadFailure),
    /// export 검증 실패.
    InvalidExports(SnapshotError),
}

impl fmt::Display for UnavailableReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnavailableReason::MissingCacheEntry => {
                f.write_str("공용 cache에 없습니다(offline 또는 미 다운로드)")
            }
            UnavailableReason::ContentHashMismatch => {
                f.write_str("cache 내용이 lockfile content hash와 다릅니다(손상)")
            }
            UnavailableReason::InconsistentLockEntries => {
                f.write_str("lockfile에서 같은 library node의 entry들이 서로 다릅니다(손상)")
            }
            UnavailableReason::CacheFailure => f.write_str("cache를 읽지 못했습니다"),
            UnavailableReason::ExportUnreadable(e) => write!(f, "{e}"),
            UnavailableReason::InvalidExports(e) => write!(f, "export 검증 실패: {e}"),
        }
    }
}

impl UnavailableReason {
    fn from_load_error(error: SnapshotLoadError) -> UnavailableReason {
        match error {
            SnapshotLoadError::MissingEntry => UnavailableReason::MissingCacheEntry,
            SnapshotLoadError::ContentHashMismatch { .. } => UnavailableReason::ContentHashMismatch,
            SnapshotLoadError::Cache(_) => UnavailableReason::CacheFailure,
            SnapshotLoadError::Corrupted(EntryCorruption::Io { .. }) => {
                UnavailableReason::CacheFailure
            }
            // 디스크 bytes가 entry 주소(= lock content hash)와 다름: lock content hash 불일치와 같은 손상입니다.
            SnapshotLoadError::Corrupted(_) => UnavailableReason::ContentHashMismatch,
            SnapshotLoadError::Rejected(e) => {
                UnavailableReason::ExportUnreadable(ExportReadFailure::InvalidLibrary(e))
            }
            SnapshotLoadError::Export(e) => UnavailableReason::ExportUnreadable(e),
            SnapshotLoadError::Invalid(e) => UnavailableReason::InvalidExports(e),
        }
    }
}

/// library 하나를 사용할 수 없음(경고, 프로젝트는 계속 열림).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryUnavailableDiagnostic {
    /// 대상 node.
    pub node: DependencyNodeKey,
    /// 이 node에 도달한 lock 경로(정렬).
    pub paths: Vec<DependencyPath>,
    /// 이유.
    pub reason: UnavailableReason,
}

impl fmt::Display for LibraryUnavailableDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let paths: Vec<String> = self.paths.iter().map(ToString::to_string).collect();
        write!(
            f,
            "library {} ({})를 사용할 수 없습니다: {}",
            self.node,
            paths.join(", "),
            self.reason
        )
    }
}

/// 사용할 수 없는 library의 resource를 가리킨 참조(library resource unavailable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceUnavailable<'a> {
    /// 참조가 속한 scope.
    pub scope: RefScope,
    /// 참조.
    pub reference: ExternalResourceRef,
    /// 대상 library 진단.
    pub library: &'a LibraryUnavailableDiagnostic,
}

impl ResourceUnavailable<'_> {
    /// 문서에 남길 미해결 외부 참조(원래 참조 전체·위치 보존).
    pub fn unresolved_at(&self, location: PropertyPath) -> UnresolvedExternalRef {
        unresolved_external(&self.reference, location)
    }
}

impl fmt::Display for ResourceUnavailable<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}의 library resource {}를 사용할 수 없습니다: {}",
            self.scope, self.reference, self.library
        )
    }
}

/// 해석할 수 없는 외부 참조(III.2.4 미해결 참조의 외부 library판).
///
/// 원래 [`ExternalResourceRef`] 전체(선언 이름·LibraryId·종류·StableId)와 발견 위치를 그대로 보존합니다.
/// 그래서 cache가 돌아오거나 의존성을 다시 받은 뒤 [`reference`](Self::reference)를 그대로 다시 해석하면
/// 원래 resource에 연결됩니다. 저장할 때는 참조의 canonical 텍스트(ADR-0027 결정 7)와 위치를 함께
/// 씁니다.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnresolvedExternalRef {
    reference: ExternalResourceRef,
    location: PropertyPath,
}

impl UnresolvedExternalRef {
    /// 미해결 외부 참조를 만듭니다.
    pub fn new(reference: ExternalResourceRef, location: PropertyPath) -> UnresolvedExternalRef {
        UnresolvedExternalRef {
            reference,
            location,
        }
    }

    /// 원래 외부 참조(선언 이름·LibraryId·StableId).
    pub fn reference(&self) -> &ExternalResourceRef {
        &self.reference
    }

    /// 참조가 발견된 위치.
    pub fn location(&self) -> &PropertyPath {
        &self.location
    }

    /// 기대했던 참조 종류.
    pub fn expected(&self) -> ExpectedRefType {
        self.reference.expected_ref_type()
    }

    /// 진단 집계용 core-types 미해결 참조.
    ///
    /// **손실 변환**입니다. 선언 이름과 LibraryId가 빠지므로 문서에 저장하거나 다시 연결하는 데 쓰지
    /// 않습니다.
    pub fn diagnostic_ref(&self) -> UnresolvedRef {
        UnresolvedRef::new(
            self.reference.resource(),
            self.expected(),
            self.location.clone(),
        )
    }
}

/// 외부 참조를 미해결 외부 참조로 바꿉니다(원래 참조 전체·위치 보존).
pub fn unresolved_external(
    reference: &ExternalResourceRef,
    location: PropertyPath,
) -> UnresolvedExternalRef {
    UnresolvedExternalRef::new(reference.clone(), location)
}

/// 참조 해석 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceResolution<'a> {
    /// 해석됨.
    Resolved(ResolvedTarget<'a>),
    /// 대상 library를 사용할 수 없음(offline·손상).
    Unavailable(ResourceUnavailable<'a>),
    /// 해석 실패(선언 없음·없는 resource 등).
    Unresolved(RefResolutionError),
}

/// 부분 전이 해석 결과.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PartialClosure<'a> {
    /// 해석된 resource(key 순).
    pub resolved: Vec<ResolvedTarget<'a>>,
    /// 사용할 수 없는 library를 가리킨 참조(scope·참조 순).
    pub unavailable: Vec<ResourceUnavailable<'a>>,
    /// 해석 실패(scope·참조·오류).
    pub unresolved: Vec<(RefScope, ExternalResourceRef, RefResolutionError)>,
}

/// lock 기준으로 로드한 library 집합.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedLibraries {
    instance: LibraryGraphInstance,
    unavailable: BTreeMap<DependencyNodeKey, LibraryUnavailableDiagnostic>,
}

impl LoadedLibraries {
    /// 로드된 graph instance.
    pub fn instance(&self) -> &LibraryGraphInstance {
        &self.instance
    }

    /// 사용할 수 없는 library 진단(node 순).
    pub fn unavailable(&self) -> impl Iterator<Item = &LibraryUnavailableDiagnostic> {
        self.unavailable.values()
    }

    /// 모든 lock node를 로드했는지.
    pub fn is_complete(&self) -> bool {
        self.unavailable.is_empty()
    }

    /// 참조 하나를 해석합니다.
    pub fn resolve(
        &self,
        scope: &RefScope,
        reference: &ExternalResourceRef,
    ) -> ResourceResolution<'_> {
        match self.instance.resolve(scope, reference) {
            Ok(target) => ResourceResolution::Resolved(target),
            Err(error) => self.classify(scope, reference, error),
        }
    }

    fn classify(
        &self,
        scope: &RefScope,
        reference: &ExternalResourceRef,
        error: RefResolutionError,
    ) -> ResourceResolution<'_> {
        if let RefResolutionError::SnapshotNotLoaded { node } = &error {
            if let Some(library) = self.unavailable.get(node) {
                return ResourceResolution::Unavailable(ResourceUnavailable {
                    scope: scope.clone(),
                    reference: reference.clone(),
                    library,
                });
            }
        }
        ResourceResolution::Unresolved(error)
    }

    /// 참조의 전이 closure를 사용할 수 있는 부분까지 해석합니다.
    ///
    /// 사용할 수 없는 library를 만나면 그 참조를 `unavailable`에 남기고 나머지를 계속 해석합니다.
    pub fn resolve_transitive_partial(
        &self,
        scope: &RefScope,
        reference: &ExternalResourceRef,
    ) -> PartialClosure<'_> {
        let mut closure = PartialClosure::default();
        let mut found: BTreeMap<_, ResolvedTarget<'_>> = BTreeMap::new();
        let mut seen_refs: BTreeSet<(RefScope, ExternalResourceRef)> = BTreeSet::new();
        let mut pending = vec![(scope.clone(), reference.clone())];
        while let Some((scope, reference)) = pending.pop() {
            if !seen_refs.insert((scope.clone(), reference.clone())) {
                continue;
            }
            match self.resolve(&scope, &reference) {
                ResourceResolution::Resolved(target) => {
                    let key = target.key();
                    if found.contains_key(&key) {
                        continue;
                    }
                    let inner = RefScope::Library(key.node.clone());
                    for child in target.references() {
                        pending.push((inner.clone(), child.clone()));
                    }
                    found.insert(key, target);
                }
                ResourceResolution::Unavailable(missing) => closure.unavailable.push(missing),
                ResourceResolution::Unresolved(error) => {
                    closure.unresolved.push((scope, reference, error))
                }
            }
        }
        closure.resolved = found.into_values().collect();
        closure
            .unavailable
            .sort_by(|a, b| (&a.scope, &a.reference).cmp(&(&b.scope, &b.reference)));
        closure
            .unresolved
            .sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        closure
    }
}

/// lock의 모든 node를 cache에서 로드합니다(Git·network 없음).
///
/// 같은 node의 lock entry(다른 경로로 도달)는 content hash·schema version·전이 의존성이 모두 같아야
/// 합니다. 다르면 어느 값이 맞는지 정할 수 없으므로 첫 entry를 조용히 쓰지 않고 그 node를
/// [`UnavailableReason::InconsistentLockEntries`] 진단으로 남깁니다. 일치하면 entry를 순서대로 시도해
/// cache에서 찾은 첫 entry를 씁니다(같은 node를 다른 repository 표기로 선언한 경우). 모두 실패하면 node를
/// 진단으로 남깁니다. lock 구조(부모 경로)가 잘못된 경우만 오류입니다.
pub fn load_locked_libraries<H: Sha256Digest, K: RepositoryKeyer>(
    lock: &LockDocument,
    store: &LibraryEntryStore<H, K>,
    validator: &mut dyn LibraryProjectValidator,
    source: &mut dyn LibraryExportSource,
    limits: &SnapshotLimits,
    now_unix_ms: u64,
) -> Result<LoadedLibraries, ScopeError> {
    let scopes = DependencyScopes::from_lock(lock)?;
    let mut by_node: BTreeMap<DependencyNodeKey, Vec<&crate::lock::LockEntry>> = BTreeMap::new();
    for entry in lock.entries() {
        by_node.entry(entry.key()).or_default().push(entry);
    }
    let mut snapshots = Vec::new();
    let mut unavailable = BTreeMap::new();
    for (node, entries) in by_node {
        let mut outcome = Err(UnavailableReason::MissingCacheEntry);
        // 불일치 entry는 어느 것도 시도하지 않음.
        let candidates: &[&crate::lock::LockEntry] = if entries_agree(&entries) {
            &entries
        } else {
            outcome = Err(UnavailableReason::InconsistentLockEntries);
            &[]
        };
        for entry in candidates {
            let request = SnapshotRequest {
                repository: &entry.repository,
                node: &node,
                expected_content_hash: Some(&entry.content_hash),
            };
            match load_cached_snapshot(store, request, validator, source, limits, now_unix_ms) {
                Ok(snapshot) => {
                    outcome = Ok(snapshot);
                    break;
                }
                Err(SnapshotLoadError::MissingEntry) => continue,
                Err(other) => {
                    outcome = Err(UnavailableReason::from_load_error(other));
                    break;
                }
            }
        }
        match outcome {
            Ok(snapshot) => snapshots.push(snapshot),
            Err(reason) => {
                let mut paths: Vec<DependencyPath> =
                    entries.iter().map(|e| e.dependency_path.clone()).collect();
                paths.sort();
                paths.dedup();
                unavailable.insert(
                    node.clone(),
                    LibraryUnavailableDiagnostic {
                        node,
                        paths,
                        reason,
                    },
                );
            }
        }
    }
    Ok(LoadedLibraries {
        instance: LibraryGraphInstance::new(scopes, snapshots),
        unavailable,
    })
}

/// 같은 node의 entry들이 node 자체를 같은 값으로 기록하는지(content hash·schema version·전이 의존성).
///
/// repository 표기와 선택자는 선언한 부모마다 다를 수 있으므로 비교하지 않습니다.
fn entries_agree(entries: &[&crate::lock::LockEntry]) -> bool {
    entries.windows(2).all(|pair| {
        let (a, b) = (pair[0], pair[1]);
        a.content_hash == b.content_hash
            && a.schema_version == b.schema_version
            && a.dependencies == b.dependencies
    })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use stellar_core_types::property::PropertyKey;

    use super::*;
    use crate::cache_root::LibraryCacheRoot;
    use crate::content_hash::ContentHashText;
    use crate::declaration::fixtures::library;
    use crate::declaration::{
        CommitId, LogicalName, RefName, RepositoryUrl, SourceKind, VersionSelector,
    };
    use crate::entry_validation::fake::TableValidator;
    use crate::entry_validation::ProbedLibrary;
    use crate::external_ref::fixtures::{component_ref, token_ref};
    use crate::library_snapshot::fixtures::{component, rid, token, token_set, TableExportSource};
    use crate::library_snapshot::{LibraryExports, LibraryResourceKey};
    use crate::lock::LockEntry;
    use crate::mirror::MirrorCache;
    use crate::node::DeclNode;
    use crate::test_support::{FakeSha256, FnvKeyer, SystemGit, TempDir};

    struct Repo {
        url: RepositoryUrl,
        commit: CommitId,
    }

    fn library_repo(git: &mut SystemGit, dir: &Path, name: &str) -> Repo {
        let path = dir.join(name);
        std::fs::create_dir_all(path.join("design")).unwrap();
        git.ok(&path, ["init", "-q", "-b", "main"]);
        git.ok(&path, ["config", "user.name", "Fixture"]);
        git.ok(&path, ["config", "user.email", "fixture@example.com"]);
        git.ok(&path, ["config", "commit.gpgsign", "false"]);
        std::fs::write(
            path.join("stellar.yaml"),
            format!("schema_version: 1\n# {name}\n"),
        )
        .unwrap();
        std::fs::write(path.join("design/chunk"), name.as_bytes()).unwrap();
        git.ok(&path, ["add", "-A"]);
        git.ok(&path, ["commit", "-q", "-m", name]);
        Repo {
            url: RepositoryUrl::new(path.to_str().unwrap()).unwrap(),
            commit: git.ok(&path, ["rev-parse", "HEAD"]).trim().parse().unwrap(),
        }
    }

    fn entry(
        lib: u8,
        repo: &Repo,
        hash: ContentHashText,
        path: &[&str],
        deps: Vec<DependencyNodeKey>,
    ) -> LockEntry {
        LockEntry {
            library_id: library(lib),
            source: SourceKind::Git,
            repository: repo.url.clone(),
            requested: VersionSelector::Tag(RefName::new("v1").unwrap()),
            exact_commit: repo.commit.clone(),
            content_hash: hash,
            schema_version: 1,
            dependency_path: DependencyPath::from_names(
                path.iter().map(|n| LogicalName::new(n).unwrap()).collect(),
            ),
            dependencies: deps,
        }
    }

    fn lock_of(mut entries: Vec<LockEntry>) -> LockDocument {
        entries.sort_by(|a, b| {
            (&a.library_id, &a.exact_commit, &a.dependency_path).cmp(&(
                &b.library_id,
                &b.exact_commit,
                &b.dependency_path,
            ))
        });
        let node = DeclNode::Mapping(vec![
            ("schema_version".to_string(), DeclNode::scalar("1")),
            (
                "resolved".to_string(),
                DeclNode::Sequence(entries.iter().map(LockEntry::to_node).collect()),
            ),
        ]);
        LockDocument::from_node(&node).expect("canonical lock")
    }

    const CARD: u16 = 10;
    const BADGE: u16 = 20;
    const CHIP: u16 = 30;
    const INK: u16 = 31;

    /// root → a(lib1) → c(lib3), root → b(lib2). a·b는 cache에 있고 c는 없음(offline).
    struct Fixture {
        _tmp: TempDir,
        store: LibraryEntryStore<FakeSha256, FnvKeyer>,
        runner: SystemGit,
        source: TableExportSource,
        validator: TableValidator,
        lock: LockDocument,
        a: DependencyNodeKey,
        b: DependencyNodeKey,
        c: DependencyNodeKey,
        c_url: RepositoryUrl,
        b_dir: PathBuf,
    }

    fn fixture() -> Option<Fixture> {
        let tmp = TempDir::new("library-load");
        let mut git = SystemGit::find(tmp.path())?;
        let a = library_repo(&mut git, tmp.path(), "a");
        let b = library_repo(&mut git, tmp.path(), "b");
        let c = library_repo(&mut git, tmp.path(), "c");
        let root = LibraryCacheRoot::in_app_data(&tmp.path().join("app")).unwrap();
        let mut store = LibraryEntryStore::new(root.clone(), FakeSha256, FnvKeyer::new(9));
        let mut mirrors = MirrorCache::new(root, SystemGit::find(tmp.path())?, FnvKeyer::new(9));
        let mut runner = SystemGit::find(tmp.path())?;
        let a_entry = store
            .materialize(&mut runner, &mut mirrors, &a.url, &a.commit, 1)
            .unwrap();
        let b_entry = store
            .materialize(&mut runner, &mut mirrors, &b.url, &b.commit, 1)
            .unwrap();
        let a_key = DependencyNodeKey::new(library(1), a.commit.clone());
        let b_key = DependencyNodeKey::new(library(2), b.commit.clone());
        let c_key = DependencyNodeKey::new(library(3), c.commit.clone());

        let mut card = component(CARD, "Card", vec![]);
        card.references = vec![
            component_ref("c", library(3), rid(CHIP)).into(),
            token_ref("c", library(3), rid(INK)).into(),
        ];
        let mut source = TableExportSource::default();
        source.by_root.insert(
            a_entry.dir.clone(),
            LibraryExports {
                components: vec![card],
                token_sets: vec![],
            },
        );
        source.by_root.insert(
            b_entry.dir.clone(),
            LibraryExports {
                components: vec![component(BADGE, "Badge", vec![])],
                token_sets: vec![token_set(21, vec![token(22, "accent", "Color", b"#0af")])],
            },
        );
        let lock = lock_of(vec![
            entry(
                1,
                &a,
                a_entry.content_hash.clone(),
                &["a"],
                vec![c_key.clone()],
            ),
            entry(2, &b, b_entry.content_hash.clone(), &["b"], vec![]),
            // c는 materialize하지 않음(offline에서 처음 열린 상태).
            entry(
                3,
                &c,
                ContentHashText::from_sha256_digest([3; 32]),
                &["a", "c"],
                vec![],
            ),
        ]);
        let mut validator = TableValidator::default();
        for (dir, lib) in [(&a_entry.dir, 1), (&b_entry.dir, 2)] {
            validator.probes.insert(
                dir.clone(),
                Ok(ProbedLibrary {
                    project_id: library(lib),
                    schema_version: 1,
                }),
            );
        }
        Some(Fixture {
            _tmp: tmp,
            store,
            runner,
            source,
            validator,
            lock,
            a: a_key,
            b: b_key,
            c: c_key,
            c_url: c.url,
            b_dir: b_entry.dir,
        })
    }

    #[test]
    fn offline_missing_library_opens_partially_with_unavailable_diagnostic() {
        let Some(mut f) = fixture() else {
            return;
        };
        let git_before = f.runner.calls.clone();
        let loaded = load_locked_libraries(
            &f.lock,
            &f.store,
            &mut f.validator,
            &mut f.source,
            &SnapshotLimits::default(),
            5,
        )
        .expect("lock 구조는 정상");
        // network·Git 없이 로드.
        assert_eq!(f.runner.calls, git_before);
        assert!(!loaded.is_complete());
        let diagnostics: Vec<_> = loaded.unavailable().cloned().collect();
        assert_eq!(
            diagnostics,
            [LibraryUnavailableDiagnostic {
                node: f.c.clone(),
                paths: vec![DependencyPath::from_names(vec![
                    LogicalName::new("a").unwrap(),
                    LogicalName::new("c").unwrap()
                ])],
                reason: UnavailableReason::MissingCacheEntry,
            }]
        );
        let text = diagnostics[0].to_string();
        assert!(text.contains("a > c") && text.contains("offline"));
        assert!(!text.contains(f.c_url.reveal()), "URL 원문 노출 금지");

        // 사용할 수 있는 library(a·b)의 resource는 그대로 해석.
        let badge = component_ref("b", library(2), rid(BADGE)).into();
        match loaded.resolve(&RefScope::Project, &badge) {
            ResourceResolution::Resolved(target) => {
                assert_eq!(
                    target.key(),
                    LibraryResourceKey::new(f.b.clone(), rid(BADGE))
                )
            }
            other => panic!("b component 해석 실패: {other:?}"),
        }
        let accent = token_ref("b", library(2), rid(22)).into();
        assert!(matches!(
            loaded.resolve(&RefScope::Project, &accent),
            ResourceResolution::Resolved(_)
        ));

        // a의 card는 열리고, card가 참조한 c resource는 unavailable 진단.
        let card: ExternalResourceRef = component_ref("a", library(1), rid(CARD)).into();
        let closure = loaded.resolve_transitive_partial(&RefScope::Project, &card);
        assert_eq!(
            closure
                .resolved
                .iter()
                .map(ResolvedTarget::key)
                .collect::<Vec<_>>(),
            [LibraryResourceKey::new(f.a.clone(), rid(CARD))]
        );
        assert!(closure.unresolved.is_empty());
        assert_eq!(closure.unavailable.len(), 2);
        for missing in &closure.unavailable {
            assert_eq!(missing.scope, RefScope::Library(f.a.clone()));
            assert_eq!(missing.library.node, f.c);
            assert!(missing.to_string().contains("사용할 수 없습니다"));
        }
        // 문서에는 원래 참조 전체(선언 이름·LibraryId·StableId)와 위치를 보존한 미해결 참조로 남김.
        let location = PropertyPath::with_keys(rid(900), vec![PropertyKey(0x0A01)]);
        let chip = &closure.unavailable[0];
        let unresolved = chip.unresolved_at(location.clone());
        assert_eq!(unresolved.reference(), &chip.reference);
        assert_eq!(unresolved.reference().dependency().as_str(), "c");
        assert_eq!(unresolved.reference().library_id(), library(3));
        assert_eq!(unresolved.reference().resource(), rid(CHIP));
        assert_eq!(unresolved.expected(), ExpectedRefType::Component);
        assert_eq!(unresolved.location(), &location);
        assert_eq!(closure.unavailable[1].reference.resource(), rid(INK));

        // 사용 프로젝트가 c를 직접 선언하지 않았으므로 직접 참조는 unavailable이 아니라 미선언 오류.
        let direct = component_ref("c", library(3), rid(CHIP)).into();
        assert!(matches!(
            loaded.resolve(&RefScope::Project, &direct),
            ResourceResolution::Unresolved(RefResolutionError::UnknownDependency { .. })
        ));
    }

    #[test]
    fn corrupted_or_unreadable_entries_become_diagnostics_not_errors() {
        let Some(mut f) = fixture() else {
            return;
        };
        // b의 lock content hash를 바꿈 → cache와 불일치, export를 읽지 않음.
        let entries: Vec<LockEntry> = f
            .lock
            .entries()
            .iter()
            .cloned()
            .map(|mut e| {
                if e.library_id == library(2) {
                    e.content_hash = ContentHashText::from_sha256_digest([0xee; 32]);
                }
                e
            })
            .collect();
        let tampered = lock_of(entries);
        let loaded = load_locked_libraries(
            &tampered,
            &f.store,
            &mut f.validator,
            &mut f.source,
            &SnapshotLimits::default(),
            6,
        )
        .unwrap();
        let reasons: BTreeMap<_, _> = loaded
            .unavailable()
            .map(|d| (d.node.clone(), d.reason.clone()))
            .collect();
        assert_eq!(
            reasons.get(&f.b),
            Some(&UnavailableReason::ContentHashMismatch)
        );
        assert_eq!(
            reasons.get(&f.c),
            Some(&UnavailableReason::MissingCacheEntry)
        );
        assert!(!f.source.calls.iter().any(|(root, _)| root == &f.b_dir));
        assert!(loaded.instance().snapshot(&f.a).is_some());

        // export를 읽지 못하면 그 library만 unavailable.
        f.source.by_root.remove(&f.b_dir);
        let loaded = load_locked_libraries(
            &f.lock,
            &f.store,
            &mut f.validator,
            &mut f.source,
            &SnapshotLimits::default(),
            7,
        )
        .unwrap();
        let reasons: BTreeMap<_, _> = loaded
            .unavailable()
            .map(|d| (d.node.clone(), d.reason.clone()))
            .collect();
        assert_eq!(
            reasons.get(&f.b),
            Some(&UnavailableReason::ExportUnreadable(
                ExportReadFailure::Corrupt
            ))
        );
        assert!(loaded.instance().snapshot(&f.a).is_some());
        let badge = component_ref("b", library(2), rid(BADGE)).into();
        assert!(matches!(
            loaded.resolve(&RefScope::Project, &badge),
            ResourceResolution::Unavailable(_)
        ));
    }

    #[test]
    fn tampered_bytes_and_rejected_entries_are_diagnostics_without_other_commit() {
        let Some(mut f) = fixture() else {
            return;
        };
        // b entry를 ProjectProbe가 일반 Git 저장소로 거부 → InvalidLibrary, export를 읽지 않음.
        f.validator.probes.insert(
            f.b_dir.clone(),
            Err(crate::entry_validation::LibraryProbeFailure::NotAStellarProject),
        );
        let loaded = load_locked_libraries(
            &f.lock,
            &f.store,
            &mut f.validator,
            &mut f.source,
            &SnapshotLimits::default(),
            10,
        )
        .unwrap();
        let reason = loaded.unavailable().find(|d| d.node == f.b).unwrap();
        assert_eq!(
            reason.reason,
            UnavailableReason::ExportUnreadable(ExportReadFailure::InvalidLibrary(
                crate::entry_validation::LibraryEntryRejection::NotAStellarProject
            ))
        );
        assert!(!f.source.calls.iter().any(|(root, _)| root == &f.b_dir));
        assert!(loaded.instance().snapshot(&f.a).is_some());

        // b entry의 디스크 bytes를 바꿈 → 다시 계산한 hash가 주소와 달라 손상. 다른 commit을 찾지 않음.
        f.validator.probes.insert(
            f.b_dir.clone(),
            Ok(ProbedLibrary {
                project_id: library(2),
                schema_version: 1,
            }),
        );
        let chunk = f.b_dir.join("design/chunk");
        let mut perms = std::fs::metadata(&chunk).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        std::fs::set_permissions(&chunk, perms).unwrap();
        std::fs::write(&chunk, b"tampered").unwrap();
        let calls_before = f.validator.storage_calls.len();
        let loaded = load_locked_libraries(
            &f.lock,
            &f.store,
            &mut f.validator,
            &mut f.source,
            &SnapshotLimits::default(),
            11,
        )
        .unwrap();
        let reason = loaded.unavailable().find(|d| d.node == f.b).unwrap();
        assert_eq!(reason.reason, UnavailableReason::ContentHashMismatch);
        assert!(reason.to_string().contains("손상"));
        assert!(loaded.instance().snapshot(&f.b).is_none());
        assert!(loaded.instance().snapshot(&f.a).is_some());
        // b는 검증기까지 가지 않음(a만 storage 검증).
        assert_eq!(f.validator.storage_calls.len(), calls_before + 1);
    }

    #[test]
    fn inconsistent_entries_for_one_node_make_it_unavailable() {
        // 같은 node(a)에 두 경로로 도달하는 lock에서 두 번째 entry만 다른 content hash·schema version을
        // 가짐. 어느 값이 맞는지 알 수 없으므로 첫 entry를 조용히 쓰지 않고 node를 진단으로 남김(리뷰 P5).
        let Some(mut f) = fixture() else {
            return;
        };
        let original = f
            .lock
            .entries()
            .iter()
            .find(|e| e.library_id == library(1))
            .cloned()
            .unwrap();
        let mut alias = original.clone();
        alias.dependency_path = DependencyPath::from_names(vec![
            LogicalName::new("b").unwrap(),
            LogicalName::new("a").unwrap(),
        ]);
        let variants: [fn(&mut LockEntry); 3] = [
            |e| e.content_hash = ContentHashText::from_sha256_digest([0xab; 32]),
            |e| e.schema_version += 1,
            |e| e.dependencies.clear(),
        ];
        for tamper in variants {
            let mut second = alias.clone();
            tamper(&mut second);
            let mut entries = f.lock.entries().to_vec();
            entries.push(second);
            let lock = lock_of(entries);
            let loaded = load_locked_libraries(
                &lock,
                &f.store,
                &mut f.validator,
                &mut f.source,
                &SnapshotLimits::default(),
                8,
            )
            .unwrap();
            assert!(
                loaded.instance().snapshot(&f.a).is_none(),
                "불일치 entry가 있는 node는 로드하지 않음"
            );
            assert!(loaded.instance().snapshot(&f.b).is_some());
            let diagnostic = loaded.unavailable().find(|d| d.node == f.a).unwrap();
            assert_eq!(
                diagnostic.reason,
                UnavailableReason::InconsistentLockEntries
            );
            assert_eq!(
                diagnostic
                    .paths
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
                ["a", "b > a"]
            );
            assert!(diagnostic.to_string().contains("손상"));
        }
        // 일치하는 두 번째 경로는 정상(같은 node를 두 경로로 선언).
        let mut entries = f.lock.entries().to_vec();
        entries.push(alias);
        let loaded = load_locked_libraries(
            &lock_of(entries),
            &f.store,
            &mut f.validator,
            &mut f.source,
            &SnapshotLimits::default(),
            9,
        )
        .unwrap();
        assert!(loaded.instance().snapshot(&f.a).is_some());
    }

    #[test]
    fn unresolved_external_keeps_dependency_name_and_library() {
        // 같은 StableId·종류라도 선언 이름이나 library가 다르면 다른 참조입니다. offline에서 남긴 미해결
        // 형태도 서로 구분되어야 cache가 돌아온 뒤 원래 의존성에 다시 연결할 수 있습니다(리뷰 P1).
        let path = PropertyPath::with_keys(rid(901), vec![PropertyKey(7)]);
        let ds: ExternalResourceRef = token_ref("ds", library(1), rid(5)).into();
        let renamed: ExternalResourceRef = token_ref("ds-next", library(1), rid(5)).into();
        let foreign: ExternalResourceRef = token_ref("ds", library(2), rid(5)).into();
        let kept = unresolved_external(&ds, path.clone());
        assert_ne!(kept, unresolved_external(&renamed, path.clone()));
        assert_ne!(kept, unresolved_external(&foreign, path));
    }

    #[test]
    fn unresolved_external_preserves_reference_identity() {
        let token: ExternalResourceRef = token_ref("ds", library(1), rid(5)).into();
        let path = PropertyPath::with_keys(rid(901), vec![PropertyKey(7)]);
        let unresolved = unresolved_external(&token, path.clone());
        assert_eq!(unresolved.reference(), &token);
        assert_eq!(unresolved.expected(), ExpectedRefType::Token);
        assert_eq!(unresolved.location(), &path);
        // 저장 형식(canonical 참조 텍스트 + 위치)으로 다시 만들어도 같음(정보 손실 없음).
        let rebuilt = UnresolvedExternalRef::new(
            unresolved.reference().to_string().parse().unwrap(),
            unresolved.location().clone(),
        );
        assert_eq!(rebuilt, unresolved);
        // 진단용 core-types 변환은 StableId·기대 타입·위치만 담음(손실 변환).
        let diagnostic = unresolved.diagnostic_ref();
        assert_eq!(diagnostic.original_id(), rid(5));
        assert_eq!(diagnostic.expected(), ExpectedRefType::Token);
        assert_eq!(diagnostic.location(), &path);
    }
}
