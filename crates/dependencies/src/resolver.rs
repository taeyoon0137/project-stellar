//! 전이 의존성 resolver (XV.2.2).
//!
//! 사용 프로젝트 선언에서 시작해 각 선택자를 exact commit으로 해석하고([`SelectorResolver`]), 그
//! commit의 library 매니페스트([`LibraryManifestSource`])에서 전이 선언을 읽어 [`DependencyGraph`]를
//! 만듭니다. 같은 [`DependencyNodeKey`]에 다시 도달하면 node는 재사용(dedup)하고 edge만 그 경로로
//! 추가합니다. 탐색은 명시적 stack을 쓰는 깊이 우선이라 깊은 체인에서도 호출 stack을 소모하지
//! 않습니다.
//!
//! 매니페스트 읽기(`git show <commit>:stellar.yaml` + 제한 YAML 파싱)는 project crate가 구현합니다.
//! library 매니페스트 안의 `local_override`는 무시합니다(override는 사용 프로젝트 선언에만 적용).

use std::collections::BTreeMap;
use std::fmt;

use crate::declaration::{
    CommitId, DependencyDeclaration, LibraryId, RepositoryUrl, VersionSelector,
};
use crate::graph::{
    DependencyEdge, DependencyGraph, DependencyNode, DependencyNodeKey, DependencyPath,
};
use crate::selection::{SelectorError, SelectorResolver};

/// exact commit에서 읽은 library 매니페스트 요약.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryManifest {
    /// library 프로젝트의 `project.id`.
    pub library_id: LibraryId,
    /// 매니페스트 schema version.
    pub schema_version: u32,
    /// library가 선언한 전이 의존성.
    pub dependencies: Vec<DependencyDeclaration>,
}

/// 매니페스트 읽기 실패.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestSourceError {
    /// commit 내용을 읽을 수 없음(캐시 없음·오프라인 등).
    Unavailable,
    /// commit root에 `stellar.yaml`이 없음(일반 Git 저장소).
    NotAStellarProject,
    /// `stellar.yaml`이 손상되었거나 스키마를 위반함.
    InvalidManifest,
}

impl fmt::Display for ManifestSourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestSourceError::Unavailable => f.write_str("library 내용을 읽을 수 없습니다"),
            ManifestSourceError::NotAStellarProject => {
                f.write_str("Stellar 프로젝트가 아닌 저장소입니다")
            }
            ManifestSourceError::InvalidManifest => {
                f.write_str("library 매니페스트가 잘못되었습니다")
            }
        }
    }
}

impl std::error::Error for ManifestSourceError {}

/// exact commit의 library 매니페스트 공급자.
pub trait LibraryManifestSource {
    /// `repository`의 `commit`에서 매니페스트를 읽습니다.
    fn manifest_at(
        &mut self,
        repository: &RepositoryUrl,
        commit: &CommitId,
    ) -> Result<LibraryManifest, ManifestSourceError>;
}

/// resolve 실패. 모든 변형은 실패 지점까지의 dependency path를 담습니다(URL은 담지 않음).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// 선택자 해석 실패.
    Selector {
        /// 실패한 선언의 경로.
        path: DependencyPath,
        /// 원인.
        error: SelectorError,
    },
    /// 매니페스트 읽기 실패.
    Manifest {
        /// 실패한 선언의 경로.
        path: DependencyPath,
        /// 원인.
        error: ManifestSourceError,
    },
    /// 선언한 library id와 해석된 commit의 `project.id`가 다름.
    LibraryIdMismatch {
        /// 선언 경로.
        path: DependencyPath,
        /// 선언한 id.
        declared: LibraryId,
        /// commit 매니페스트의 id.
        found: LibraryId,
    },
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveError::Selector { path, error } => write!(f, "{path}: {error}"),
            ResolveError::Manifest { path, error } => write!(f, "{path}: {error}"),
            ResolveError::LibraryIdMismatch {
                path,
                declared,
                found,
            } => write!(
                f,
                "{path}: 선언한 library id {declared}와 저장소의 project id {found}가 다릅니다"
            ),
        }
    }
}

impl std::error::Error for ResolveError {}

/// 탐색 stack frame: 한 부모의 선언 목록과 다음 처리 위치.
struct Frame {
    parent: Option<DependencyNodeKey>,
    path: DependencyPath,
    declarations: Vec<DependencyDeclaration>,
    next: usize,
}

/// 사용 프로젝트 선언에서 전이 의존성 그래프를 만듭니다.
///
/// 같은 `(repository 비교 키, 선택자)`는 한 번만 해석합니다(세션 memo).
pub fn resolve_dependency_graph<S, M>(
    root: &[DependencyDeclaration],
    selectors: &mut S,
    manifests: &mut M,
) -> Result<DependencyGraph, ResolveError>
where
    S: SelectorResolver + ?Sized,
    M: LibraryManifestSource + ?Sized,
{
    let mut graph = DependencyGraph::default();
    let mut memo: BTreeMap<(String, VersionSelector), CommitId> = BTreeMap::new();
    let mut stack = vec![Frame {
        parent: None,
        path: DependencyPath::root(),
        declarations: root.to_vec(),
        next: 0,
    }];

    while let Some(frame) = stack.last_mut() {
        let Some(declaration) = frame.declarations.get(frame.next).cloned() else {
            stack.pop();
            continue;
        };
        frame.next += 1;
        let parent = frame.parent.clone();
        let path = frame.path.child(&declaration.name);

        let memo_key = (
            declaration.repository.comparison_key(),
            declaration.version.clone(),
        );
        let commit = match memo.get(&memo_key) {
            Some(commit) => commit.clone(),
            None => {
                let commit = selectors
                    .resolve(&declaration.repository, &declaration.version)
                    .map_err(|error| ResolveError::Selector {
                        path: path.clone(),
                        error,
                    })?;
                memo.insert(memo_key, commit.clone());
                commit
            }
        };
        let key = DependencyNodeKey::new(declaration.library_id, commit);

        graph.edges.push(DependencyEdge {
            parent: parent.clone(),
            child: key.clone(),
            path: path.clone(),
            repository: declaration.repository.clone(),
            requested: declaration.version.clone(),
        });
        if let Some(parent) = &parent {
            if let Some(node) = graph.nodes.get_mut(parent) {
                node.dependencies.insert(key.clone());
            }
        }
        if graph.nodes.contains_key(&key) {
            // 같은 resolved node: 재사용하고 하위는 다시 탐색하지 않습니다.
            continue;
        }

        let manifest = manifests
            .manifest_at(&declaration.repository, &key.commit)
            .map_err(|error| ResolveError::Manifest {
                path: path.clone(),
                error,
            })?;
        if manifest.library_id != declaration.library_id {
            return Err(ResolveError::LibraryIdMismatch {
                path,
                declared: declaration.library_id,
                found: manifest.library_id,
            });
        }
        graph.nodes.insert(
            key.clone(),
            DependencyNode {
                key: key.clone(),
                schema_version: manifest.schema_version,
                dependencies: Default::default(),
            },
        );
        stack.push(Frame {
            parent: Some(key),
            path,
            declarations: manifest.dependencies,
            next: 0,
        });
    }
    Ok(graph)
}

#[cfg(test)]
pub(crate) mod fake {
    //! 테스트용 in-memory library 저장소 목록(선택자 해석 + 매니페스트).

    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use super::*;
    use crate::declaration::fixtures::{commit, decl, library};

    /// 저장소 URL별 tag → commit, commit → 매니페스트 표. 호출 횟수를 기록합니다.
    #[derive(Default)]
    pub(crate) struct Catalog {
        refs: BTreeMap<(String, VersionSelector), CommitId>,
        manifests: BTreeMap<(String, CommitId), LibraryManifest>,
        pub(crate) resolve_calls: RefCell<usize>,
        pub(crate) manifest_calls: RefCell<BTreeMap<CommitId, usize>>,
    }

    impl Catalog {
        /// `url`의 tag `tag`를 commit 번호 `commit_no`로 publish합니다(library `lib`, 전이 선언 `deps`).
        pub(crate) fn publish(
            &mut self,
            url: &str,
            lib: u8,
            tag: &str,
            commit_no: u8,
            deps: Vec<DependencyDeclaration>,
        ) {
            self.publish_ref(url, lib, tag, commit_no);
            self.manifests.insert(
                (url.to_string(), commit(commit_no)),
                LibraryManifest {
                    library_id: library(lib),
                    schema_version: 1,
                    dependencies: deps,
                },
            );
        }

        /// tag → commit만 등록합니다(매니페스트 없음).
        pub(crate) fn publish_ref(&mut self, url: &str, lib: u8, tag: &str, commit_no: u8) {
            let selector = decl("x", lib, url, tag).version;
            self.refs
                .insert((url.to_string(), selector), commit(commit_no));
        }

        /// 매니페스트를 직접 덮어씁니다.
        pub(crate) fn set_manifest(&mut self, url: &str, commit_no: u8, manifest: LibraryManifest) {
            self.manifests
                .insert((url.to_string(), commit(commit_no)), manifest);
        }

        pub(crate) fn selectors(&self) -> CatalogSelectors<'_> {
            CatalogSelectors(self)
        }

        pub(crate) fn manifests(&self) -> CatalogManifests<'_> {
            CatalogManifests(self)
        }

        pub(crate) fn manifest_calls_for(&self, commit_no: u8) -> usize {
            self.manifest_calls
                .borrow()
                .get(&commit(commit_no))
                .copied()
                .unwrap_or(0)
        }
    }

    pub(crate) struct CatalogSelectors<'a>(&'a Catalog);
    pub(crate) struct CatalogManifests<'a>(&'a Catalog);

    impl SelectorResolver for CatalogSelectors<'_> {
        fn resolve(
            &mut self,
            repository: &RepositoryUrl,
            selector: &VersionSelector,
        ) -> Result<CommitId, SelectorError> {
            *self.0.resolve_calls.borrow_mut() += 1;
            self.0
                .refs
                .get(&(repository.reveal().to_string(), selector.clone()))
                .cloned()
                .ok_or(SelectorError::RefNotFound {
                    kind: selector.kind(),
                })
        }
    }

    impl LibraryManifestSource for CatalogManifests<'_> {
        fn manifest_at(
            &mut self,
            repository: &RepositoryUrl,
            commit: &CommitId,
        ) -> Result<LibraryManifest, ManifestSourceError> {
            *self
                .0
                .manifest_calls
                .borrow_mut()
                .entry(commit.clone())
                .or_default() += 1;
            self.0
                .manifests
                .get(&(repository.reveal().to_string(), commit.clone()))
                .cloned()
                .ok_or(ManifestSourceError::Unavailable)
        }
    }

    /// 사용 프로젝트 root 해석 헬퍼.
    pub(crate) fn resolve(
        catalog: &Catalog,
        root: &[DependencyDeclaration],
    ) -> Result<DependencyGraph, ResolveError> {
        resolve_dependency_graph(root, &mut catalog.selectors(), &mut catalog.manifests())
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{resolve, Catalog};
    use super::*;
    use crate::declaration::fixtures::{commit, decl, library};
    use crate::declaration::LogicalName;

    const A: &str = "https://e.com/a.git";
    const B: &str = "https://e.com/b.git";
    const C: &str = "https://e.com/c.git";

    fn key(lib: u8, commit_no: u8) -> DependencyNodeKey {
        DependencyNodeKey::new(library(lib), commit(commit_no))
    }

    fn path(names: &[&str]) -> DependencyPath {
        DependencyPath::from_names(names.iter().map(|n| LogicalName::new(n).unwrap()).collect())
    }

    /// root → a, b; a → c v1; b → c v1 (diamond).
    fn diamond() -> Catalog {
        let mut catalog = Catalog::default();
        catalog.publish(C, 3, "v1", 0xc1, vec![]);
        catalog.publish(A, 1, "v1", 0xa1, vec![decl("c", 3, C, "v1")]);
        catalog.publish(B, 2, "v1", 0xb1, vec![decl("c", 3, C, "v1")]);
        catalog
    }

    #[test]
    fn diamond_dependency_dedups_node_and_records_both_paths() {
        let catalog = diamond();
        let root = [decl("a", 1, A, "v1"), decl("b", 2, B, "v1")];
        let graph = resolve(&catalog, &root).expect("resolve");

        // node 3개(c는 하나), edge 4개(경로마다).
        assert_eq!(graph.node_count(), 3);
        assert_eq!(graph.edges().len(), 4);
        let edges: Vec<(Option<DependencyNodeKey>, DependencyNodeKey, DependencyPath)> = graph
            .edges()
            .iter()
            .map(|e| (e.parent.clone(), e.child.clone(), e.path.clone()))
            .collect();
        assert_eq!(
            edges,
            [
                (None, key(1, 0xa1), path(&["a"])),
                (Some(key(1, 0xa1)), key(3, 0xc1), path(&["a", "c"])),
                (None, key(2, 0xb1), path(&["b"])),
                (Some(key(2, 0xb1)), key(3, 0xc1), path(&["b", "c"])),
            ]
        );
        let c = key(3, 0xc1);
        let into_c: Vec<String> = graph.edges_to(&c).map(|e| e.path.to_string()).collect();
        assert_eq!(into_c, ["a > c", "b > c"]);

        // 전이 의존성 기록, dedup된 c 매니페스트는 한 번만 읽음, 선택자 memo로 c 해석 1회.
        assert_eq!(
            graph.node(&key(1, 0xa1)).unwrap().dependencies,
            [c.clone()].into()
        );
        assert_eq!(
            graph.node(&key(2, 0xb1)).unwrap().dependencies,
            [c.clone()].into()
        );
        assert!(graph.node(&c).unwrap().dependencies.is_empty());
        assert_eq!(catalog.manifest_calls_for(0xc1), 1);
        assert_eq!(*catalog.resolve_calls.borrow(), 3);
    }

    #[test]
    fn failures_carry_dependency_path() {
        let catalog = diamond();
        // a → c가 없는 tag를 요구.
        let mut broken = Catalog::default();
        broken.publish(A, 1, "v1", 0xa1, vec![decl("c", 3, C, "v9")]);
        assert_eq!(
            resolve(&broken, &[decl("a", 1, A, "v1")]),
            Err(ResolveError::Selector {
                path: path(&["a", "c"]),
                error: SelectorError::RefNotFound {
                    kind: crate::declaration::SelectorKind::Tag
                }
            })
        );

        // 선언한 library id와 저장소 project id 불일치.
        let wrong = resolve(&catalog, &[decl("a", 9, A, "v1")]).unwrap_err();
        assert_eq!(
            wrong,
            ResolveError::LibraryIdMismatch {
                path: path(&["a"]),
                declared: library(9),
                found: library(1),
            }
        );
        assert!(!wrong.to_string().contains("e.com"));

        // ref는 있지만 commit 내용을 읽을 수 없음 → 경로와 함께 매니페스트 오류.
        let mut unreadable = Catalog::default();
        unreadable.publish(A, 1, "v1", 0xa1, vec![decl("c", 3, C, "v1")]);
        unreadable.publish_ref(C, 3, "v1", 0xc1);
        assert_eq!(
            resolve(&unreadable, &[decl("a", 1, A, "v1")]),
            Err(ResolveError::Manifest {
                path: path(&["a", "c"]),
                error: ManifestSourceError::Unavailable
            })
        );
    }

    #[test]
    fn node_records_manifest_schema_version() {
        let mut catalog = Catalog::default();
        catalog.publish(A, 1, "v1", 0xa1, vec![]);
        catalog.set_manifest(
            A,
            0xa1,
            LibraryManifest {
                library_id: library(1),
                schema_version: 4,
                dependencies: vec![],
            },
        );
        let graph = resolve(&catalog, &[decl("a", 1, A, "v1")]).unwrap();
        assert_eq!(graph.node(&key(1, 0xa1)).unwrap().schema_version, 4);
    }
}
