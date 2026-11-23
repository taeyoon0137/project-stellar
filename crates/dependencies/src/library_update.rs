//! library update 후보 준비 (XV.7.1).
//!
//! 사용 프로젝트의 의존성 하나를 update하려면 먼저 **현재 snapshot**(lock의 exact commit)과 **후보
//! snapshot**(선택자를 다시 해석한 commit)을 나란히 만듭니다. 이 단계는 검토용입니다.
//!
//! - lockfile·문서·in-memory 로드 상태를 바꾸지 않습니다. 적용(XV.7.4)은 사용자 승인 뒤의 별도 단계입니다.
//! - 선택자는 시스템 Git으로 다시 해석합니다([`SelectorResolver`], XV.2.4). 요청에 새 선택자가 있으면 그
//!   선택자를, 없으면 `stellar.yaml` 선언 선택자를 씁니다(branch 이동 확인).
//! - 현재 snapshot은 cache에서만 읽습니다(network 없음). 후보는 필요하면 fetch·materialize하고
//!   ProjectProbe·storage validator(XV.3.4)를 통과해야 합니다. 후보 materialize는 immutable 공용 cache에
//!   entry를 추가할 뿐 기존 entry를 바꾸지 않습니다.

use std::fmt;

use crate::cache_entry::{LibraryEntryStore, MaterializeError, Sha256Digest};
use crate::declaration::{
    CommitId, DependencyDeclaration, LogicalName, RepositoryUrl, VersionSelector,
};
use crate::entry_validation::{LibraryEntryRejection, LibraryProjectValidator};
use crate::git::GitCommandRunner;
use crate::git_refs::FetchedRefs;
use crate::graph::{DependencyNodeKey, DependencyPath};
use crate::library_snapshot::{
    load_cached_snapshot, LibraryExportSource, LibrarySnapshot, SnapshotLimits, SnapshotLoadError,
    SnapshotRequest,
};
use crate::lock::{LockDocument, LockEntry};
use crate::mirror::RepositoryKeyer;
use crate::selection::{SelectorError, SelectorResolver};

/// 의존성 update 요청.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryUpdateRequest {
    /// 사용 프로젝트 `stellar.yaml`의 의존성 logical name.
    pub dependency: LogicalName,
    /// 새 선택자(`None`이면 선언 선택자를 다시 해석).
    pub selector: Option<VersionSelector>,
}

/// lock에 고정된 현재 버전.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentLibrarySnapshot {
    /// 현재 lock entry(사용 프로젝트 root 선언).
    pub lock_entry: LockEntry,
    /// 현재 snapshot.
    pub snapshot: LibrarySnapshot,
}

/// 선택자를 다시 해석한 후보 버전.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateLibrarySnapshot {
    /// 선언 repository.
    pub repository: RepositoryUrl,
    /// 해석에 쓴 선택자.
    pub requested: VersionSelector,
    /// 후보 snapshot.
    pub snapshot: LibrarySnapshot,
}

/// update 검토 후보.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryUpdateCandidate {
    /// 의존성 logical name.
    pub dependency: LogicalName,
    /// 현재 버전.
    pub current: CurrentLibrarySnapshot,
    /// 후보 버전.
    pub candidate: CandidateLibrarySnapshot,
}

impl LibraryUpdateCandidate {
    /// 현재 commit.
    pub fn current_commit(&self) -> &CommitId {
        self.current.snapshot.commit()
    }

    /// 후보 commit.
    pub fn candidate_commit(&self) -> &CommitId {
        self.candidate.snapshot.commit()
    }

    /// 후보가 현재와 같은 commit인지(바꿀 것 없음).
    pub fn is_same_commit(&self) -> bool {
        self.current_commit() == self.candidate_commit()
    }

    /// 선택자가 lock의 선언 선택자와 다른지.
    pub fn selector_changed(&self) -> bool {
        self.candidate.requested != self.current.lock_entry.requested
    }
}

/// 후보 snapshot 준비 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateLoadError {
    /// fetch·materialize 실패.
    Materialize(MaterializeError),
    /// ProjectProbe·storage validator 거부(일반 Git 저장소·다른 library id·손상).
    Rejected(LibraryEntryRejection),
    /// snapshot 읽기 실패.
    Load(SnapshotLoadError),
}

impl fmt::Display for CandidateLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CandidateLoadError::Materialize(e) => write!(f, "후보 library 다운로드 실패: {e}"),
            CandidateLoadError::Rejected(e) => write!(f, "후보 library 거부: {e}"),
            CandidateLoadError::Load(e) => write!(f, "후보 library 읽기 실패: {e}"),
        }
    }
}

impl std::error::Error for CandidateLoadError {}

/// update 후보 준비 실패. URL 원문은 담지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    /// 사용 프로젝트 `stellar.yaml`에 그 이름의 선언이 없음.
    UnknownDependency {
        /// 의존성 이름.
        dependency: LogicalName,
    },
    /// lock에 그 선언의 root entry가 없음(먼저 resolve 필요).
    NotLocked {
        /// 의존성 이름.
        dependency: LogicalName,
    },
    /// 선언 library id와 lock entry library id가 다름(lock이 선언보다 오래됨).
    LockMismatch {
        /// 의존성 이름.
        dependency: LogicalName,
    },
    /// 선택자 해석 실패.
    Selector {
        /// 의존성 이름.
        dependency: LogicalName,
        /// 원인.
        error: SelectorError,
    },
    /// 현재 버전 snapshot을 cache에서 읽지 못함.
    CurrentUnavailable(SnapshotLoadError),
    /// 후보 버전 snapshot을 준비하지 못함.
    Candidate(CandidateLoadError),
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpdateError::UnknownDependency { dependency } => {
                write!(f, "stellar.yaml에 의존성 {dependency}가 없습니다")
            }
            UpdateError::NotLocked { dependency } => {
                write!(
                    f,
                    "lockfile에 의존성 {dependency}가 없습니다(먼저 resolve하세요)"
                )
            }
            UpdateError::LockMismatch { dependency } => write!(
                f,
                "의존성 {dependency}의 선언 library id와 lockfile이 다릅니다(먼저 resolve하세요)"
            ),
            UpdateError::Selector { dependency, error } => {
                write!(f, "의존성 {dependency} 선택자 해석 실패: {error}")
            }
            UpdateError::CurrentUnavailable(e) => write!(f, "현재 library를 읽지 못했습니다: {e}"),
            UpdateError::Candidate(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for UpdateError {}

/// update 검토용 snapshot 공급자.
pub trait UpdateSnapshotSource {
    /// lock에 기록된 현재 node snapshot(cache만, network 없음).
    fn current(&mut self, entry: &LockEntry) -> Result<LibrarySnapshot, SnapshotLoadError>;

    /// 후보 node snapshot(필요하면 fetch·materialize·검증).
    fn candidate(
        &mut self,
        repository: &RepositoryUrl,
        node: &DependencyNodeKey,
    ) -> Result<LibrarySnapshot, CandidateLoadError>;
}

/// 의존성 하나의 현재·후보 snapshot을 만듭니다. lock·문서는 바꾸지 않습니다.
pub fn prepare_library_update(
    declarations: &[DependencyDeclaration],
    lock: &LockDocument,
    request: &LibraryUpdateRequest,
    selectors: &mut dyn SelectorResolver,
    snapshots: &mut dyn UpdateSnapshotSource,
) -> Result<LibraryUpdateCandidate, UpdateError> {
    let dependency = request.dependency.clone();
    let declaration = declarations
        .iter()
        .find(|d| d.name == dependency)
        .ok_or_else(|| UpdateError::UnknownDependency {
            dependency: dependency.clone(),
        })?;
    let root_path = DependencyPath::from_names(vec![dependency.clone()]);
    let lock_entry = lock
        .entries()
        .iter()
        .find(|e| e.dependency_path == root_path)
        .ok_or_else(|| UpdateError::NotLocked {
            dependency: dependency.clone(),
        })?;
    if lock_entry.library_id != declaration.library_id {
        return Err(UpdateError::LockMismatch { dependency });
    }

    let requested = request
        .selector
        .clone()
        .unwrap_or_else(|| declaration.version.clone());
    let commit = selectors
        .resolve(&declaration.repository, &requested)
        .map_err(|error| UpdateError::Selector {
            dependency: dependency.clone(),
            error,
        })?;

    let current = snapshots
        .current(lock_entry)
        .map_err(UpdateError::CurrentUnavailable)?;
    let candidate_node = DependencyNodeKey::new(declaration.library_id, commit);
    let candidate = snapshots
        .candidate(&declaration.repository, &candidate_node)
        .map_err(UpdateError::Candidate)?;
    Ok(LibraryUpdateCandidate {
        dependency,
        current: CurrentLibrarySnapshot {
            lock_entry: lock_entry.clone(),
            snapshot: current,
        },
        candidate: CandidateLibrarySnapshot {
            repository: declaration.repository.clone(),
            requested,
            snapshot: candidate,
        },
    })
}

/// 공용 cache 기반 [`UpdateSnapshotSource`].
pub struct CacheUpdateSource<'a, H, K, R: ?Sized, F: ?Sized> {
    /// cache entry store.
    pub store: &'a mut LibraryEntryStore<H, K>,
    /// materialize용 Git 실행기.
    pub runner: &'a mut R,
    /// fetch된 mirror 공급자.
    pub mirrors: &'a mut F,
    /// 후보 entry 검증기(ProjectProbe·storage validator).
    pub validator: &'a mut dyn LibraryProjectValidator,
    /// export 공급자.
    pub exports: &'a mut dyn LibraryExportSource,
    /// snapshot 상한.
    pub limits: SnapshotLimits,
    /// 현재 시각(cache 최근 사용 기록).
    pub now_unix_ms: u64,
}

impl<H, K, R, F> UpdateSnapshotSource for CacheUpdateSource<'_, H, K, R, F>
where
    H: Sha256Digest,
    K: RepositoryKeyer,
    R: GitCommandRunner + ?Sized,
    F: FetchedRefs + ?Sized,
{
    fn current(&mut self, entry: &LockEntry) -> Result<LibrarySnapshot, SnapshotLoadError> {
        let node = entry.key();
        load_cached_snapshot(
            self.store,
            SnapshotRequest {
                repository: &entry.repository,
                node: &node,
                expected_content_hash: Some(&entry.content_hash),
            },
            self.validator,
            self.exports,
            &self.limits,
            self.now_unix_ms,
        )
    }

    fn candidate(
        &mut self,
        repository: &RepositoryUrl,
        node: &DependencyNodeKey,
    ) -> Result<LibrarySnapshot, CandidateLoadError> {
        let materialized = self
            .store
            .materialize(
                self.runner,
                self.mirrors,
                repository,
                &node.commit,
                self.now_unix_ms,
            )
            .map_err(CandidateLoadError::Materialize)?;
        // entry 검증(ProjectProbe·storage validator)과 디스크 bytes 재해시는 cache 읽기 경로 하나가 합니다.
        load_cached_snapshot(
            self.store,
            SnapshotRequest {
                repository,
                node,
                expected_content_hash: Some(&materialized.content_hash),
            },
            self.validator,
            self.exports,
            &self.limits,
            self.now_unix_ms,
        )
        .map_err(|error| match error {
            SnapshotLoadError::Rejected(rejection) => CandidateLoadError::Rejected(rejection),
            other => CandidateLoadError::Load(other),
        })
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! 실제 Git update fixture에서 쓰는 공급자.

    use std::path::Path;

    use super::*;
    use crate::declaration::LibraryId;
    use crate::entry_validation::{LibraryProbeFailure, ProbedLibrary, StorageValidationFailure};
    use crate::library_snapshot::fixtures::component;
    use crate::library_snapshot::{ExportReadFailure, LibraryExports};

    /// `stellar.yaml` 첫 줄을 component 이름으로 돌려주는 export 공급자(버전마다 내용이 다름).
    #[derive(Default)]
    pub(crate) struct ManifestEchoSource {
        pub(crate) reads: usize,
    }

    impl LibraryExportSource for ManifestEchoSource {
        fn read_exports(
            &mut self,
            root: &Path,
            _: LibraryId,
            _: &SnapshotLimits,
        ) -> Result<LibraryExports, ExportReadFailure> {
            self.reads += 1;
            let text = std::fs::read_to_string(root.join("stellar.yaml"))
                .map_err(|e| ExportReadFailure::Io { kind: e.kind() })?;
            let name = text.lines().next().unwrap_or_default().to_string();
            Ok(LibraryExports {
                components: vec![component(1, &name, vec![])],
                token_sets: vec![],
            })
        }
    }

    /// 첫 release(`schema_version: 1`, lock의 현재 버전)는 `current_id`, 그 밖의 release는 `project_id`의 library로
    /// 판정하는 검증기. 후보 저장소만 다른 library인 경우를 만듭니다.
    pub(crate) struct FixedValidator {
        pub(crate) current_id: LibraryId,
        pub(crate) project_id: LibraryId,
    }

    impl LibraryProjectValidator for FixedValidator {
        fn probe(&mut self, root: &Path) -> Result<ProbedLibrary, LibraryProbeFailure> {
            if let Ok(text) = std::fs::read_to_string(root.join("stellar.yaml")) {
                let project_id = if text.starts_with("schema_version: 1\n") {
                    self.current_id
                } else {
                    self.project_id
                };
                Ok(ProbedLibrary {
                    project_id,
                    schema_version: 1,
                })
            } else {
                Err(LibraryProbeFailure::NotAStellarProject)
            }
        }

        fn validate_storage(
            &mut self,
            _: &Path,
            _: LibraryId,
        ) -> Result<(), StorageValidationFailure> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{FixedValidator, ManifestEchoSource};
    use super::*;
    use crate::cache_root::LibraryCacheRoot;
    use crate::content_hash::ContentHashText;
    use crate::declaration::fixtures::{decl, library};
    use crate::declaration::{RefName, SourceKind};
    use crate::git_refs::GitSelectorResolver;
    use crate::library_snapshot::fixtures::rid;
    use crate::mirror::MirrorCache;
    use crate::node::DeclNode;
    use crate::test_support::{fixture_repo, FakeSha256, FnvKeyer, SystemGit, TempDir};
    use stellar_core_types::reference::ComponentRef;

    fn tag(name: &str) -> VersionSelector {
        VersionSelector::Tag(RefName::new(name).unwrap())
    }

    fn branch(name: &str) -> VersionSelector {
        VersionSelector::Branch(RefName::new(name).unwrap())
    }

    fn lock_with(entry: LockEntry) -> LockDocument {
        let node = DeclNode::Mapping(vec![
            ("schema_version".to_string(), DeclNode::scalar("1")),
            (
                "resolved".to_string(),
                DeclNode::Sequence(vec![entry.to_node()]),
            ),
        ]);
        LockDocument::from_node(&node).unwrap()
    }

    fn component_name(snapshot: &LibrarySnapshot) -> String {
        snapshot
            .component(ComponentRef::new(rid(1)))
            .map(|c| c.export().name.clone())
            .unwrap_or_default()
    }

    struct Env {
        tmp: TempDir,
        url: RepositoryUrl,
        first: CommitId,
        second: CommitId,
        root: LibraryCacheRoot,
        store: LibraryEntryStore<FakeSha256, FnvKeyer>,
        lock: LockDocument,
    }

    /// `ds` 저장소(v1=first, v2=second=main)와 first에 고정된 lock.
    fn env(requested: VersionSelector) -> Option<Env> {
        let tmp = TempDir::new("library-update");
        let mut git = SystemGit::find(tmp.path())?;
        let repo = fixture_repo(&mut git, tmp.path(), "ds");
        let url = RepositoryUrl::new(repo.path.to_str().unwrap()).unwrap();
        let first: CommitId = repo.first.parse().unwrap();
        let second: CommitId = repo.second.parse().unwrap();
        let root = LibraryCacheRoot::in_app_data(&tmp.path().join("app")).unwrap();
        let mut store = LibraryEntryStore::new(root.clone(), FakeSha256, FnvKeyer::new(4));
        let mut mirrors =
            MirrorCache::new(root.clone(), SystemGit::find(tmp.path())?, FnvKeyer::new(4));
        let mut runner = SystemGit::find(tmp.path())?;
        let current = store
            .materialize(&mut runner, &mut mirrors, &url, &first, 1)
            .unwrap();
        let lock = lock_with(LockEntry {
            library_id: library(1),
            source: SourceKind::Git,
            repository: url.clone(),
            requested,
            exact_commit: first.clone(),
            content_hash: current.content_hash,
            schema_version: 1,
            dependency_path: DependencyPath::from_names(vec![LogicalName::new("ds").unwrap()]),
            dependencies: vec![],
        });
        Some(Env {
            tmp,
            url,
            first,
            second,
            root,
            store,
            lock,
        })
    }

    fn run(
        env: &mut Env,
        declarations: &[DependencyDeclaration],
        request: &LibraryUpdateRequest,
        validator_id: u8,
    ) -> Result<LibraryUpdateCandidate, UpdateError> {
        let work = env.tmp.path().to_path_buf();
        let mut selectors = GitSelectorResolver::new(
            SystemGit::find(&work).unwrap(),
            MirrorCache::new(
                env.root.clone(),
                SystemGit::find(&work).unwrap(),
                FnvKeyer::new(4),
            ),
            &work,
        );
        let mut runner = SystemGit::find(&work).unwrap();
        let mut mirrors = MirrorCache::new(
            env.root.clone(),
            SystemGit::find(&work).unwrap(),
            FnvKeyer::new(4),
        );
        let mut validator = FixedValidator {
            current_id: library(1),
            project_id: library(validator_id),
        };
        let mut exports = ManifestEchoSource::default();
        let mut source = CacheUpdateSource {
            store: &mut env.store,
            runner: &mut runner,
            mirrors: &mut mirrors,
            validator: &mut validator,
            exports: &mut exports,
            limits: SnapshotLimits::default(),
            now_unix_ms: 10,
        };
        prepare_library_update(
            declarations,
            &env.lock,
            request,
            &mut selectors,
            &mut source,
        )
    }

    fn ds_decl(url: &RepositoryUrl, version: VersionSelector) -> DependencyDeclaration {
        let mut d = decl("ds", 1, url.reveal(), "v1");
        d.version = version;
        d
    }

    #[test]
    fn new_selector_yields_exact_current_and_candidate_commits() {
        let Some(mut env) = env(tag("v1")) else {
            return;
        };
        let declarations = [ds_decl(&env.url, tag("v1"))];
        let lock_before = env.lock.clone();
        let request = LibraryUpdateRequest {
            dependency: LogicalName::new("ds").unwrap(),
            selector: Some(tag("v2")),
        };
        let update = run(&mut env, &declarations, &request, 1).unwrap();
        assert_eq!(update.current_commit(), &env.first);
        assert_eq!(update.candidate_commit(), &env.second);
        assert!(!update.is_same_commit());
        assert!(update.selector_changed());
        assert_eq!(update.candidate.requested, tag("v2"));
        assert_eq!(
            component_name(&update.current.snapshot),
            "schema_version: 1"
        );
        assert_eq!(
            component_name(&update.candidate.snapshot),
            "schema_version: 2"
        );
        assert_ne!(
            update.current.snapshot.content_hash(),
            update.candidate.snapshot.content_hash()
        );
        // 검토 단계는 lock을 바꾸지 않음.
        assert_eq!(env.lock, lock_before);
    }

    #[test]
    fn declared_branch_is_reinterpreted_and_unchanged_tag_is_same_commit() {
        // lock은 main이 first였을 때 만들어졌고, 지금 main은 second.
        let Some(mut env) = env(branch("main")) else {
            return;
        };
        let request = LibraryUpdateRequest {
            dependency: LogicalName::new("ds").unwrap(),
            selector: None,
        };
        let declarations = [ds_decl(&env.url, branch("main"))];
        let update = run(&mut env, &declarations, &request, 1).unwrap();
        assert_eq!(update.current_commit(), &env.first);
        assert_eq!(update.candidate_commit(), &env.second);
        assert!(!update.selector_changed());

        // tag v1 그대로 다시 해석 → 같은 commit(바꿀 것 없음).
        let Some(mut env) = self::env(tag("v1")) else {
            return;
        };
        let declarations = [ds_decl(&env.url, tag("v1"))];
        let update = run(&mut env, &declarations, &request, 1).unwrap();
        assert!(update.is_same_commit());
        assert_eq!(update.candidate_commit(), &env.first);
        assert_eq!(update.current.snapshot, update.candidate.snapshot);
    }

    #[test]
    fn update_failures_are_structured() {
        let Some(mut env) = env(tag("v1")) else {
            return;
        };
        let ds = LogicalName::new("ds").unwrap();
        let declarations = [ds_decl(&env.url, tag("v1"))];
        let request = |name: &str, selector: Option<VersionSelector>| LibraryUpdateRequest {
            dependency: LogicalName::new(name).unwrap(),
            selector,
        };
        assert_eq!(
            run(&mut env, &declarations, &request("icons", None), 1),
            Err(UpdateError::UnknownDependency {
                dependency: LogicalName::new("icons").unwrap()
            })
        );
        let mut other = ds_decl(&env.url, tag("v1"));
        other.name = LogicalName::new("other").unwrap();
        assert_eq!(
            run(&mut env, &[other], &request("other", None), 1),
            Err(UpdateError::NotLocked {
                dependency: LogicalName::new("other").unwrap()
            })
        );
        let mut changed_library = ds_decl(&env.url, tag("v1"));
        changed_library.library_id = library(5);
        assert_eq!(
            run(&mut env, &[changed_library], &request("ds", None), 1),
            Err(UpdateError::LockMismatch {
                dependency: ds.clone()
            })
        );
        assert!(matches!(
            run(&mut env, &declarations, &request("ds", Some(tag("v9"))), 1),
            Err(UpdateError::Selector {
                error: SelectorError::RefNotFound { .. },
                ..
            })
        ));
        // 후보가 다른 library id의 저장소 → ProjectProbe 단계에서 거부.
        assert_eq!(
            run(&mut env, &declarations, &request("ds", Some(tag("v2"))), 7),
            Err(UpdateError::Candidate(CandidateLoadError::Rejected(
                LibraryEntryRejection::LibraryIdMismatch {
                    expected: library(1),
                    found: library(7),
                }
            )))
        );
        // 현재 버전 cache가 lock과 다름 → 현재 snapshot 없음으로 중단.
        let mut entry = env.lock.entries()[0].clone();
        entry.content_hash = ContentHashText::from_sha256_digest([0xab; 32]);
        env.lock = lock_with(entry);
        assert!(matches!(
            run(&mut env, &declarations, &request("ds", Some(tag("v2"))), 1),
            Err(UpdateError::CurrentUnavailable(
                SnapshotLoadError::ContentHashMismatch { .. }
            ))
        ));
        // 오류 문자열에 URL 원문 없음.
        let text = UpdateError::Selector {
            dependency: ds,
            error: SelectorError::RefNotFound {
                kind: crate::declaration::SelectorKind::Tag,
            },
        }
        .to_string();
        assert!(!text.contains(env.url.reveal()));
    }
}
