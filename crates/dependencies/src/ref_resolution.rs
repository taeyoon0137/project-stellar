//! 외부 참조 해석과 전이 resource ref (XV.5.4).
//!
//! 외부 참조(XV.5.2)는 선언 이름 + LibraryId + StableId만 갖습니다. 어떤 commit의 resource인지는 그
//! 참조가 **속한 scope**가 정합니다.
//!
//! - 사용 프로젝트 문서의 참조는 [`RefScope::Project`]: 사용 프로젝트 `stellar.yaml` 선언 이름을 lock의
//!   root edge로 찾습니다.
//! - library resource 안의 참조는 [`RefScope::Library`]`(그 node)`: 그 library commit의 선언 이름을 같은
//!   dependency graph의 그 node에서 나가는 edge로 찾습니다.
//!
//! 그래서 같은 그래프 instance 안에서 `a → c v1`, `b → c v2`이면 a의 resource가 가리키는 c는 v1, b의
//! resource가 가리키는 c는 v2입니다. 같은 StableId라도 섞이지 않습니다(아키텍처 30.3 버전별 격리). 사용
//! 프로젝트는 직접 선언하지 않은 전이 library를 참조할 수 없습니다.
//!
//! scope 표([`DependencyScopes`])는 resolver 그래프나 `stellar.lock.yaml`에서 만듭니다. lock에서는 각
//! entry의 canonical dependency path(XV.2.6)로 부모를 찾습니다. 전이 closure는 명시적 stack과 방문
//! 집합으로 계산하므로 깊은 chain에서도 호출 stack을 쓰지 않습니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::declaration::{LibraryId, LogicalName};
use crate::external_ref::{ExternalComponentRef, ExternalResourceRef, ExternalTokenRef};
use crate::graph::{DependencyGraph, DependencyNodeKey, DependencyPath};
use crate::library_snapshot::{
    LibraryComponent, LibraryResourceKey, LibraryResourceKind, LibrarySnapshot, LibraryToken,
};
use crate::lock::LockDocument;

/// 참조가 속한 scope.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RefScope {
    /// 사용 프로젝트 문서.
    Project,
    /// library node의 resource.
    Library(DependencyNodeKey),
}

impl fmt::Display for RefScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RefScope::Project => f.write_str("<project>"),
            RefScope::Library(node) => node.fmt(f),
        }
    }
}

/// lock에서 scope 표를 만들 수 없음.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeError {
    /// entry 경로의 부모 경로를 가진 entry가 없음.
    MissingParent {
        /// entry 경로.
        path: DependencyPath,
    },
    /// 같은 scope의 같은 선언 이름이 서로 다른 node를 가리킴.
    Conflict {
        /// entry 경로.
        path: DependencyPath,
    },
    /// 같은 경로가 서로 다른 node를 가리킴.
    DuplicatePath {
        /// 경로.
        path: DependencyPath,
    },
}

impl fmt::Display for ScopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScopeError::MissingParent { path } => {
                write!(f, "{path}: lock에 부모 경로 entry가 없습니다")
            }
            ScopeError::Conflict { path } => {
                write!(
                    f,
                    "{path}: 같은 선언 이름이 서로 다른 library node를 가리킵니다"
                )
            }
            ScopeError::DuplicatePath { path } => {
                write!(f, "{path}: 같은 경로가 서로 다른 library node를 가리킵니다")
            }
        }
    }
}

impl std::error::Error for ScopeError {}

/// (scope, 선언 이름) → node 표.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DependencyScopes {
    children: BTreeMap<(RefScope, LogicalName), DependencyNodeKey>,
}

impl DependencyScopes {
    /// resolver 그래프의 edge에서 만듭니다.
    ///
    /// [`from_lock`](Self::from_lock)과 같은 규칙으로, 같은 scope의 같은 선언 이름이 서로 다른 node를
    /// 가리키면 덮어쓰지 않고 [`ScopeError::Conflict`]로 거부합니다. 같은 node를 가리키는 중복 edge(같은
    /// 부모 node에 여러 경로로 도달)는 허용합니다.
    pub fn from_graph(graph: &DependencyGraph) -> Result<DependencyScopes, ScopeError> {
        let mut children = BTreeMap::new();
        for edge in graph.edges() {
            let scope = match &edge.parent {
                None => RefScope::Project,
                Some(parent) => RefScope::Library(parent.clone()),
            };
            let Some(name) = edge.path.last() else {
                continue;
            };
            match children.insert((scope, name.clone()), edge.child.clone()) {
                Some(existing) if existing != edge.child => {
                    return Err(ScopeError::Conflict {
                        path: edge.path.clone(),
                    })
                }
                _ => {}
            }
        }
        Ok(DependencyScopes { children })
    }

    /// lock entry에서 만듭니다. 부모는 entry 경로에서 마지막 이름을 뺀 경로의 entry입니다.
    pub fn from_lock(lock: &LockDocument) -> Result<DependencyScopes, ScopeError> {
        let mut by_path: BTreeMap<&DependencyPath, DependencyNodeKey> = BTreeMap::new();
        for entry in lock.entries() {
            let key = entry.key();
            if let Some(existing) = by_path.insert(&entry.dependency_path, key.clone()) {
                if existing != key {
                    return Err(ScopeError::DuplicatePath {
                        path: entry.dependency_path.clone(),
                    });
                }
            }
        }
        let mut children = BTreeMap::new();
        for entry in lock.entries() {
            let names = entry.dependency_path.names();
            let Some((name, parent_names)) = names.split_last() else {
                continue;
            };
            let scope = if parent_names.is_empty() {
                RefScope::Project
            } else {
                let parent_path = DependencyPath::from_names(parent_names.to_vec());
                let parent =
                    by_path
                        .get(&parent_path)
                        .ok_or_else(|| ScopeError::MissingParent {
                            path: entry.dependency_path.clone(),
                        })?;
                RefScope::Library(parent.clone())
            };
            let key = entry.key();
            match children.insert((scope, name.clone()), key.clone()) {
                Some(existing) if existing != key => {
                    return Err(ScopeError::Conflict {
                        path: entry.dependency_path.clone(),
                    })
                }
                _ => {}
            }
        }
        Ok(DependencyScopes { children })
    }

    /// scope에서 선언 이름이 가리키는 node.
    pub fn child(&self, scope: &RefScope, name: &LogicalName) -> Option<&DependencyNodeKey> {
        self.children.get(&(scope.clone(), name.clone()))
    }

    /// `scope`가 직접 선언한 (선언 이름, node)(이름 순).
    pub fn children_of<'a>(
        &'a self,
        scope: &'a RefScope,
    ) -> impl Iterator<Item = (&'a LogicalName, &'a DependencyNodeKey)> + 'a {
        self.children
            .iter()
            .filter(move |((owner, _), _)| owner == scope)
            .map(|((_, name), node)| (name, node))
    }

    /// 표에 나오는 모든 node(정렬, 중복 제거).
    pub fn nodes(&self) -> BTreeSet<DependencyNodeKey> {
        self.children.values().cloned().collect()
    }
}

/// 참조 해석 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefResolutionError {
    /// scope가 그 이름의 의존성을 선언하지 않음(전이 library 직접 참조 포함).
    UnknownDependency {
        /// 참조 scope.
        scope: RefScope,
        /// 선언 이름.
        dependency: LogicalName,
    },
    /// 선언 이름이 참조와 다른 library를 가리킴.
    LibraryMismatch {
        /// 참조 scope.
        scope: RefScope,
        /// 선언 이름.
        dependency: LogicalName,
        /// 참조가 기대한 library.
        expected: LibraryId,
        /// 선언이 가리키는 library.
        found: LibraryId,
    },
    /// 대상 node의 snapshot이 로드되지 않음.
    SnapshotNotLoaded {
        /// 대상 node.
        node: DependencyNodeKey,
    },
    /// snapshot에 resource가 없음.
    MissingResource {
        /// 대상 전역 key.
        key: LibraryResourceKey,
        /// 기대 종류.
        expected: LibraryResourceKind,
    },
    /// StableId가 다른 종류의 resource임.
    WrongKind {
        /// 대상 전역 key.
        key: LibraryResourceKey,
        /// 기대 종류.
        expected: LibraryResourceKind,
        /// 실제 종류.
        found: LibraryResourceKind,
    },
}

impl fmt::Display for RefResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RefResolutionError::UnknownDependency { scope, dependency } => {
                write!(f, "{scope}에 의존성 {dependency} 선언이 없습니다")
            }
            RefResolutionError::LibraryMismatch {
                scope,
                dependency,
                expected,
                found,
            } => write!(
                f,
                "{scope}의 의존성 {dependency}는 library {found}이지만 참조는 {expected}를 기대합니다"
            ),
            RefResolutionError::SnapshotNotLoaded { node } => {
                write!(f, "library {node}가 로드되지 않았습니다")
            }
            RefResolutionError::MissingResource { key, expected } => {
                write!(f, "library {} {key}가 없습니다", expected.as_str())
            }
            RefResolutionError::WrongKind {
                key,
                expected,
                found,
            } => write!(
                f,
                "{key}는 {}가 아니라 {}입니다",
                expected.as_str(),
                found.as_str()
            ),
        }
    }
}

impl std::error::Error for RefResolutionError {}

/// 해석된 resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedTarget<'a> {
    /// component.
    Component(LibraryComponent<'a>),
    /// token.
    Token(LibraryToken<'a>),
}

impl<'a> ResolvedTarget<'a> {
    /// 전역 key.
    pub fn key(&self) -> LibraryResourceKey {
        match self {
            ResolvedTarget::Component(c) => c.key(),
            ResolvedTarget::Token(t) => t.key(),
        }
    }

    /// 이 resource가 자기 library scope에서 가진 외부 참조.
    pub fn references(&self) -> &'a [ExternalResourceRef] {
        match self {
            ResolvedTarget::Component(c) => &c.export().references,
            ResolvedTarget::Token(t) => &t.export().references,
        }
    }
}

/// 한 dependency graph instance: scope 표와 node별 읽기 전용 snapshot, 사용 프로젝트가 저장한 외부 component 참조.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryGraphInstance {
    scopes: DependencyScopes,
    snapshots: BTreeMap<DependencyNodeKey, LibrarySnapshot>,
    imports: crate::component_imports::ComponentImports,
}

impl LibraryGraphInstance {
    /// scope 표와 snapshot으로 만듭니다. 같은 node의 snapshot이 여럿이면 마지막 것을 씁니다.
    pub fn new(
        scopes: DependencyScopes,
        snapshots: impl IntoIterator<Item = LibrarySnapshot>,
    ) -> LibraryGraphInstance {
        LibraryGraphInstance {
            scopes,
            snapshots: snapshots
                .into_iter()
                .map(|s| (s.node().clone(), s))
                .collect(),
            imports: Default::default(),
        }
    }

    /// 사용 프로젝트 `stellar.yaml`이 저장한 외부 component 참조를 붙입니다(ADR-0137 결정 1).
    pub fn with_imports(
        mut self,
        imports: crate::component_imports::ComponentImports,
    ) -> LibraryGraphInstance {
        self.imports = imports;
        self
    }

    /// 사용 프로젝트가 저장한 외부 component 참조.
    pub fn imports(&self) -> &crate::component_imports::ComponentImports {
        &self.imports
    }

    /// scope 표.
    pub fn scopes(&self) -> &DependencyScopes {
        &self.scopes
    }

    /// node snapshot.
    pub fn snapshot(&self, node: &DependencyNodeKey) -> Option<&LibrarySnapshot> {
        self.snapshots.get(node)
    }

    /// 모든 snapshot(node 순).
    pub fn snapshots(&self) -> impl Iterator<Item = &LibrarySnapshot> {
        self.snapshots.values()
    }

    /// 참조의 선언 이름을 scope에서 node로 바꿉니다(LibraryId 확인 포함).
    pub fn node_for(
        &self,
        scope: &RefScope,
        reference: &ExternalResourceRef,
    ) -> Result<&DependencyNodeKey, RefResolutionError> {
        let node = self
            .scopes
            .child(scope, reference.dependency())
            .ok_or_else(|| RefResolutionError::UnknownDependency {
                scope: scope.clone(),
                dependency: reference.dependency().clone(),
            })?;
        if node.library_id != reference.library_id() {
            return Err(RefResolutionError::LibraryMismatch {
                scope: scope.clone(),
                dependency: reference.dependency().clone(),
                expected: reference.library_id(),
                found: node.library_id,
            });
        }
        Ok(node)
    }

    /// 참조 하나를 scope 안에서 해석합니다.
    pub fn resolve(
        &self,
        scope: &RefScope,
        reference: &ExternalResourceRef,
    ) -> Result<ResolvedTarget<'_>, RefResolutionError> {
        let node = self.node_for(scope, reference)?;
        let snapshot = self
            .snapshots
            .get(node)
            .ok_or_else(|| RefResolutionError::SnapshotNotLoaded { node: node.clone() })?;
        let id = reference.resource();
        let expected = reference.kind();
        let found = match reference {
            ExternalResourceRef::Component(r) => snapshot
                .component(r.component())
                .map(ResolvedTarget::Component),
            ExternalResourceRef::Token(r) => snapshot.token(r.token()).map(ResolvedTarget::Token),
        };
        found.ok_or_else(|| match snapshot.kind_of(id) {
            Some(kind) => RefResolutionError::WrongKind {
                key: snapshot.key_of(id),
                expected,
                found: kind,
            },
            None => RefResolutionError::MissingResource {
                key: snapshot.key_of(id),
                expected,
            },
        })
    }

    /// 참조와 그 resource가 전이적으로 참조하는 모든 library resource를 해석합니다(key 순, 중복 제거).
    ///
    /// 각 library resource의 참조는 그 resource가 속한 node scope에서 해석합니다. 해석에 실패하면 실패한
    /// 참조의 scope를 담은 오류를 반환합니다.
    pub fn resolve_transitive(
        &self,
        scope: &RefScope,
        reference: &ExternalResourceRef,
    ) -> Result<Vec<ResolvedTarget<'_>>, RefResolutionError> {
        let first = self.resolve(scope, reference)?;
        let mut found: BTreeMap<LibraryResourceKey, ResolvedTarget<'_>> = BTreeMap::new();
        let mut stack = vec![first];
        while let Some(target) = stack.pop() {
            let key = target.key();
            if found.contains_key(&key) {
                continue;
            }
            let inner = RefScope::Library(key.node.clone());
            for child in target.references() {
                let resolved = self.resolve(&inner, child)?;
                if !found.contains_key(&resolved.key()) {
                    stack.push(resolved);
                }
            }
            found.insert(key, target);
        }
        Ok(found.into_values().collect())
    }

    /// component 참조를 해석합니다.
    pub fn component(
        &self,
        scope: &RefScope,
        reference: &ExternalComponentRef,
    ) -> Result<LibraryComponent<'_>, RefResolutionError> {
        match self.resolve(scope, &ExternalResourceRef::Component(reference.clone()))? {
            ResolvedTarget::Component(c) => Ok(c),
            ResolvedTarget::Token(t) => Err(RefResolutionError::WrongKind {
                key: t.key(),
                expected: LibraryResourceKind::Component,
                found: LibraryResourceKind::Token,
            }),
        }
    }

    /// token 참조를 해석합니다.
    pub fn token(
        &self,
        scope: &RefScope,
        reference: &ExternalTokenRef,
    ) -> Result<LibraryToken<'_>, RefResolutionError> {
        match self.resolve(scope, &ExternalResourceRef::Token(reference.clone()))? {
            ResolvedTarget::Token(t) => Ok(t),
            ResolvedTarget::Component(c) => Err(RefResolutionError::WrongKind {
                key: c.key(),
                expected: LibraryResourceKind::Token,
                found: LibraryResourceKind::Component,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::content_hash::ContentHashText;
    use crate::declaration::fixtures::{commit, decl, library};
    use crate::external_ref::fixtures::{component_ref, token_ref};
    use crate::library_snapshot::fixtures::{component, rid, snapshot, token, token_set};
    use crate::library_snapshot::LibraryExports;
    use crate::lock::LockDocument;
    use crate::resolver::fake::{resolve, Catalog};

    const A: &str = "https://e.com/a.git";
    const B: &str = "https://e.com/b.git";
    const C: &str = "https://e.com/c.git";

    const CHIP: u16 = 30; // c의 component(v1·v2 같은 StableId)
    const INK: u16 = 31; // c의 token(v1·v2 같은 StableId)
    const CARD: u16 = 10; // a의 component
    const TILE: u16 = 20; // b의 component

    fn key(lib: u8, c: u8) -> DependencyNodeKey {
        DependencyNodeKey::new(library(lib), commit(c))
    }

    /// root → a → c v1, root → b → c v2.
    fn graph() -> DependencyGraph {
        let mut catalog = Catalog::default();
        catalog.publish(C, 3, "v1", 0xc1, vec![]);
        catalog.publish(C, 3, "v2", 0xc2, vec![]);
        catalog.publish(A, 1, "v1", 0xa1, vec![decl("c", 3, C, "v1")]);
        catalog.publish(B, 2, "v1", 0xb1, vec![decl("c", 3, C, "v2")]);
        resolve(&catalog, &[decl("a", 1, A, "v1"), decl("b", 2, B, "v1")]).unwrap()
    }

    fn c_version(commit_no: u8, chip: &str, ink: &[u8]) -> LibrarySnapshot {
        snapshot(
            key(3, commit_no),
            LibraryExports {
                components: vec![component(CHIP, chip, vec![])],
                token_sets: vec![token_set(32, vec![token(INK, "ink", "Color", ink)])],
            },
        )
    }

    /// 부모 library: component 하나가 c의 component·token을 참조.
    fn parent(lib: u8, commit_no: u8, id: u16) -> LibrarySnapshot {
        let mut card = component(id, "Parent", vec![]);
        card.references = vec![
            component_ref("c", library(3), rid(CHIP)).into(),
            token_ref("c", library(3), rid(INK)).into(),
        ];
        snapshot(
            key(lib, commit_no),
            LibraryExports {
                components: vec![card],
                token_sets: vec![],
            },
        )
    }

    fn instance() -> LibraryGraphInstance {
        LibraryGraphInstance::new(
            DependencyScopes::from_graph(&graph()).unwrap(),
            [
                parent(1, 0xa1, CARD),
                parent(2, 0xb1, TILE),
                c_version(0xc1, "Chip v1", b"#111"),
                c_version(0xc2, "Chip v2", b"#222"),
            ],
        )
    }

    fn names(targets: &[ResolvedTarget<'_>]) -> Vec<(LibraryResourceKey, String)> {
        targets
            .iter()
            .map(|t| {
                let name = match t {
                    ResolvedTarget::Component(c) => c.export().name.clone(),
                    ResolvedTarget::Token(t) => {
                        String::from_utf8_lossy(t.export().value.as_bytes()).into_owned()
                    }
                };
                (t.key(), name)
            })
            .collect()
    }

    #[test]
    fn v1_parent_reaches_v1_child_and_v2_parent_reaches_v2_child() {
        let graph = instance();
        let from_a = graph
            .resolve_transitive(
                &RefScope::Project,
                &component_ref("a", library(1), rid(CARD)).into(),
            )
            .unwrap();
        let from_b = graph
            .resolve_transitive(
                &RefScope::Project,
                &component_ref("b", library(2), rid(TILE)).into(),
            )
            .unwrap();
        let chip_v1 = LibraryResourceKey::new(key(3, 0xc1), rid(CHIP));
        let chip_v2 = LibraryResourceKey::new(key(3, 0xc2), rid(CHIP));
        assert_eq!(
            names(&from_a),
            [
                (
                    LibraryResourceKey::new(key(1, 0xa1), rid(CARD)),
                    "Parent".to_string()
                ),
                (chip_v1.clone(), "Chip v1".to_string()),
                (
                    LibraryResourceKey::new(key(3, 0xc1), rid(INK)),
                    "#111".to_string()
                ),
            ]
        );
        assert_eq!(
            names(&from_b),
            [
                (
                    LibraryResourceKey::new(key(2, 0xb1), rid(TILE)),
                    "Parent".to_string()
                ),
                (chip_v2.clone(), "Chip v2".to_string()),
                (
                    LibraryResourceKey::new(key(3, 0xc2), rid(INK)),
                    "#222".to_string()
                ),
            ]
        );
        // 같은 참조 텍스트(`component:c@...`)도 scope에 따라 다른 버전.
        let chip = component_ref("c", library(3), rid(CHIP));
        assert_eq!(
            graph
                .component(&RefScope::Library(key(1, 0xa1)), &chip)
                .unwrap()
                .key(),
            chip_v1
        );
        assert_eq!(
            graph
                .component(&RefScope::Library(key(2, 0xb1)), &chip)
                .unwrap()
                .key(),
            chip_v2
        );
        // 사용 프로젝트는 선언하지 않은 전이 library(c)를 직접 참조할 수 없음.
        assert_eq!(
            graph.component(&RefScope::Project, &chip),
            Err(RefResolutionError::UnknownDependency {
                scope: RefScope::Project,
                dependency: LogicalName::new("c").unwrap(),
            })
        );
    }

    #[test]
    fn lock_scopes_match_graph_scopes() {
        let graph = graph();
        let hashes: BTreeMap<_, _> = graph
            .nodes()
            .map(|n| (n.key.clone(), ContentHashText::from_sha256_digest([7; 32])))
            .collect();
        let lock = LockDocument::from_graph(&graph, &hashes).unwrap();
        let from_lock = DependencyScopes::from_lock(&lock).unwrap();
        assert_eq!(from_lock, DependencyScopes::from_graph(&graph).unwrap());
        assert_eq!(
            from_lock.child(
                &RefScope::Library(key(2, 0xb1)),
                &LogicalName::new("c").unwrap()
            ),
            Some(&key(3, 0xc2))
        );
        assert_eq!(from_lock.nodes().len(), 4);
    }

    #[test]
    fn conflicting_graph_edges_are_rejected_like_lock() {
        // 같은 scope의 같은 선언 이름이 서로 다른 node를 가리키는 그래프. lock 경로(from_lock)는
        // `Conflict`로 거부하므로 그래프 경로도 덮어쓰지 않고 같은 오류를 돌려야 합니다(리뷰 P5).
        let mut broken = graph();
        let mut extra = broken
            .edges
            .iter()
            .find(|e| e.parent.is_none())
            .cloned()
            .unwrap();
        extra.child = key(9, 0x99);
        broken.edges.push(extra.clone());
        assert_eq!(
            DependencyScopes::from_graph(&broken),
            Err(ScopeError::Conflict { path: extra.path })
        );
        // 같은 node를 가리키는 중복 edge(diamond로 같은 부모 node에 두 경로로 도달)는 허용.
        let mut diamond = graph();
        let mut same = diamond.edges[0].clone();
        same.path = DependencyPath::from_names(vec![
            LogicalName::new("other").unwrap(),
            same.path.last().unwrap().clone(),
        ]);
        diamond.edges.push(same);
        assert!(DependencyScopes::from_graph(&diamond).is_ok());
    }

    #[test]
    fn lock_without_parent_entry_is_rejected() {
        let graph = graph();
        let hashes: BTreeMap<_, _> = graph
            .nodes()
            .map(|n| (n.key.clone(), ContentHashText::from_sha256_digest([7; 32])))
            .collect();
        let lock = LockDocument::from_graph(&graph, &hashes).unwrap();
        // `a` entry를 빼면 `a > c`의 부모를 찾을 수 없음.
        let entries: Vec<_> = lock
            .into_entries()
            .into_iter()
            .filter(|e| e.dependency_path.to_string() != "a")
            .collect();
        let node = crate::node::DeclNode::Mapping(vec![
            (
                "schema_version".to_string(),
                crate::node::DeclNode::scalar("1"),
            ),
            (
                "resolved".to_string(),
                crate::node::DeclNode::Sequence(entries.iter().map(|e| e.to_node()).collect()),
            ),
        ]);
        let broken = LockDocument::from_node(&node).unwrap();
        assert!(matches!(
            DependencyScopes::from_lock(&broken),
            Err(ScopeError::MissingParent { .. })
        ));
    }

    #[test]
    fn resolution_failures_are_structured() {
        let graph = instance();
        let a_scope = RefScope::Library(key(1, 0xa1));
        // a가 선언하지 않은 이름.
        assert!(matches!(
            graph.component(&a_scope, &component_ref("d", library(3), rid(CHIP))),
            Err(RefResolutionError::UnknownDependency { .. })
        ));
        // 선언 이름이 다른 library를 가리킴.
        assert_eq!(
            graph.component(&a_scope, &component_ref("c", library(9), rid(CHIP))),
            Err(RefResolutionError::LibraryMismatch {
                scope: a_scope.clone(),
                dependency: LogicalName::new("c").unwrap(),
                expected: library(9),
                found: library(3),
            })
        );
        // 없는 resource, 다른 종류.
        assert_eq!(
            graph.component(&a_scope, &component_ref("c", library(3), rid(99))),
            Err(RefResolutionError::MissingResource {
                key: LibraryResourceKey::new(key(3, 0xc1), rid(99)),
                expected: LibraryResourceKind::Component,
            })
        );
        assert_eq!(
            graph.component(&a_scope, &component_ref("c", library(3), rid(INK))),
            Err(RefResolutionError::WrongKind {
                key: LibraryResourceKey::new(key(3, 0xc1), rid(INK)),
                expected: LibraryResourceKind::Component,
                found: LibraryResourceKind::Token,
            })
        );
        // snapshot이 없는 node.
        let partial = LibraryGraphInstance::new(
            DependencyScopes::from_graph(&self::graph()).unwrap(),
            [parent(1, 0xa1, CARD)],
        );
        assert_eq!(
            partial.resolve_transitive(
                &RefScope::Project,
                &component_ref("a", library(1), rid(CARD)).into()
            ),
            Err(RefResolutionError::SnapshotNotLoaded { node: key(3, 0xc1) })
        );
    }

    #[test]
    fn transitive_closure_handles_shared_and_deep_chains_iteratively() {
        // 깊은 chain: L0 → L1 → ... → L{n}, 각 library component가 다음 library component를 참조.
        const DEPTH: u8 = 200;
        let mut scopes = DependencyScopes::default();
        let mut snapshots = Vec::new();
        for i in 0..DEPTH {
            let node = key(i, i);
            let scope = if i == 0 {
                RefScope::Project
            } else {
                RefScope::Library(key(i - 1, i - 1))
            };
            scopes.children.insert(
                (scope, LogicalName::new(&format!("l{i}")).unwrap()),
                node.clone(),
            );
            let mut c = component(1, "Link", vec![]);
            if i + 1 < DEPTH {
                // 같은 대상을 두 번 참조해도 closure에는 한 번.
                let next = component_ref(&format!("l{}", i + 1), library(i + 1), rid(1));
                c.references = vec![next.clone().into(), next.into()];
            }
            snapshots.push(snapshot(
                node,
                LibraryExports {
                    components: vec![c],
                    token_sets: vec![],
                },
            ));
        }
        let graph = LibraryGraphInstance::new(scopes, snapshots);
        let closure = graph
            .resolve_transitive(
                &RefScope::Project,
                &component_ref("l0", library(0), rid(1)).into(),
            )
            .unwrap();
        assert_eq!(closure.len(), DEPTH as usize);
        let keys: Vec<_> = closure.iter().map(ResolvedTarget::key).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
    }
}
