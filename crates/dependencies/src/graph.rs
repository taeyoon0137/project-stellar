//! 의존성 resolver 그래프 (XV.2).
//!
//! node는 [`DependencyNodeKey`](LibraryId + 해석된 exact commit)로 식별합니다. 같은 library의 여러
//! 버전은 서로 다른 node입니다(아키텍처 30.3 "여러 버전 동시 로드", 자동 단일 버전 통합 금지).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::declaration::{CommitId, LibraryId, LogicalName, RepositoryUrl, VersionSelector};
use crate::selection::LockPin;

/// 해석된 의존성 node 식별자(XV.2.1).
///
/// 정렬은 LibraryId → commit 순서입니다. 같은 LibraryId라도 commit이 다르면 다른 key입니다.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DependencyNodeKey {
    /// 라이브러리 식별자.
    pub library_id: LibraryId,
    /// 해석된 exact commit.
    pub commit: CommitId,
}

impl DependencyNodeKey {
    /// key를 만듭니다.
    pub fn new(library_id: LibraryId, commit: CommitId) -> DependencyNodeKey {
        DependencyNodeKey { library_id, commit }
    }

    /// lock pin의 node key.
    pub fn of_pin(pin: &LockPin) -> DependencyNodeKey {
        DependencyNodeKey::new(pin.library_id, pin.commit.clone())
    }
}

impl fmt::Display for DependencyNodeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.library_id, self.commit)
    }
}

/// 사용 프로젝트 root에서 한 의존성까지의 logical name 경로(XV.2.2).
///
/// 첫 요소는 사용 프로젝트 `stellar.yaml`의 선언, 이후 요소는 각 library의 전이 선언입니다.
/// 정렬은 요소별 사전식입니다.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DependencyPath(Vec<LogicalName>);

impl DependencyPath {
    /// 빈 경로(사용 프로젝트 root).
    pub fn root() -> DependencyPath {
        DependencyPath(Vec::new())
    }

    /// 요소 목록으로 만듭니다.
    pub fn from_names(names: Vec<LogicalName>) -> DependencyPath {
        DependencyPath(names)
    }

    /// 끝에 이름을 붙인 새 경로.
    pub fn child(&self, name: &LogicalName) -> DependencyPath {
        let mut names = self.0.clone();
        names.push(name.clone());
        DependencyPath(names)
    }

    /// 요소.
    pub fn names(&self) -> &[LogicalName] {
        &self.0
    }

    /// 마지막 요소(root면 `None`).
    pub fn last(&self) -> Option<&LogicalName> {
        self.0.last()
    }

    /// 요소 수.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// root 경로인지.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for DependencyPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return f.write_str("<root>");
        }
        for (i, name) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(" > ")?;
            }
            f.write_str(name.as_str())?;
        }
        Ok(())
    }
}

/// 의존성 edge 하나: 부모(사용 프로젝트 root 또는 library node)의 선언 한 건(XV.2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyEdge {
    /// 부모 node(`None`이면 사용 프로젝트 root).
    pub parent: Option<DependencyNodeKey>,
    /// 해석된 자식 node.
    pub child: DependencyNodeKey,
    /// root에서 자식까지의 경로(마지막 요소 = 부모가 선언한 logical name).
    pub path: DependencyPath,
    /// 부모가 선언한 repository.
    pub repository: RepositoryUrl,
    /// 부모가 선언한 선택자.
    pub requested: VersionSelector,
}

/// 해석된 library node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyNode {
    /// node key.
    pub key: DependencyNodeKey,
    /// 해당 commit의 library 매니페스트 schema version.
    pub schema_version: u32,
    /// 직접 의존하는 node(전이 의존성, 정렬·중복 제거).
    pub dependencies: BTreeSet<DependencyNodeKey>,
}

/// resolver 그래프. 같은 key의 node는 하나만 존재하고(dedup), edge는 도달한 경로마다 기록합니다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DependencyGraph {
    pub(crate) nodes: BTreeMap<DependencyNodeKey, DependencyNode>,
    pub(crate) edges: Vec<DependencyEdge>,
}

impl DependencyGraph {
    /// node 조회.
    pub fn node(&self, key: &DependencyNodeKey) -> Option<&DependencyNode> {
        self.nodes.get(key)
    }

    /// 모든 node(key 순).
    pub fn nodes(&self) -> impl Iterator<Item = &DependencyNode> {
        self.nodes.values()
    }

    /// node 수.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// 모든 edge(해석 순서).
    pub fn edges(&self) -> &[DependencyEdge] {
        &self.edges
    }

    /// 특정 node로 들어오는 edge.
    pub fn edges_to<'a>(
        &'a self,
        key: &'a DependencyNodeKey,
    ) -> impl Iterator<Item = &'a DependencyEdge> + 'a {
        self.edges.iter().filter(move |e| &e.child == key)
    }

    /// 같은 LibraryId의 resolved commit이 둘 이상인 library마다 경고 하나(XV.2.5).
    ///
    /// 다중 버전은 오류가 아닙니다(아키텍처 30.3: 버전별 격리, 자동 단일 버전 통합 금지). 결과는
    /// LibraryId 순, 각 경고의 버전은 commit 순, 경로는 경로 순으로 정렬합니다.
    pub fn duplicate_library_versions(&self) -> Vec<DuplicateLibraryVersionWarning> {
        let mut by_library: BTreeMap<LibraryId, Vec<&DependencyNodeKey>> = BTreeMap::new();
        for key in self.nodes.keys() {
            by_library.entry(key.library_id).or_default().push(key);
        }
        by_library
            .into_iter()
            .filter(|(_, keys)| keys.len() > 1)
            .map(|(library_id, keys)| DuplicateLibraryVersionWarning {
                library_id,
                versions: keys
                    .into_iter()
                    .map(|key| {
                        let mut paths: Vec<DependencyPath> =
                            self.edges_to(key).map(|e| e.path.clone()).collect();
                        paths.sort();
                        paths.dedup();
                        DuplicateVersion {
                            commit: key.commit.clone(),
                            paths,
                        }
                    })
                    .collect(),
            })
            .collect()
    }
}

/// 다중 버전 경고의 버전 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateVersion {
    /// resolved commit.
    pub commit: CommitId,
    /// 이 버전에 도달한 dependency path(정렬).
    pub paths: Vec<DependencyPath>,
}

/// 같은 LibraryId가 여러 resolved commit으로 그래프에 존재함(경고, XV.2.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateLibraryVersionWarning {
    /// library 식별자.
    pub library_id: LibraryId,
    /// 버전별 commit과 경로(commit 순).
    pub versions: Vec<DuplicateVersion>,
}

impl fmt::Display for DuplicateLibraryVersionWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "library {}가 {}개 버전으로 함께 로드됩니다",
            self.library_id,
            self.versions.len()
        )?;
        for version in &self.versions {
            let paths: Vec<String> = version.paths.iter().map(ToString::to_string).collect();
            write!(f, "; {} ← {}", version.commit, paths.join(", "))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashSet};

    use super::*;
    use crate::declaration::fixtures::{commit, decl, library};
    use crate::declaration::{LogicalName, RefName, VersionSelector};
    use crate::resolver::fake::{resolve, Catalog};

    const A: &str = "https://e.com/a.git";
    const B: &str = "https://e.com/b.git";
    const C: &str = "https://e.com/c.git";

    fn path(names: &[&str]) -> DependencyPath {
        DependencyPath::from_names(names.iter().map(|n| LogicalName::new(n).unwrap()).collect())
    }

    #[test]
    fn two_versions_of_one_library_yield_exactly_one_warning() {
        // root → a → c v1, root → b → c v2.
        let mut catalog = Catalog::default();
        catalog.publish(C, 3, "v1", 0xc1, vec![]);
        catalog.publish(C, 3, "v2", 0xc2, vec![]);
        catalog.publish(A, 1, "v1", 0xa1, vec![decl("c", 3, C, "v1")]);
        catalog.publish(B, 2, "v1", 0xb1, vec![decl("c", 3, C, "v2")]);
        let graph = resolve(&catalog, &[decl("a", 1, A, "v1"), decl("b", 2, B, "v1")])
            .expect("다중 버전은 오류가 아님");
        assert_eq!(graph.node_count(), 4);

        let warnings = graph.duplicate_library_versions();
        assert_eq!(
            warnings,
            [DuplicateLibraryVersionWarning {
                library_id: library(3),
                versions: vec![
                    DuplicateVersion {
                        commit: commit(0xc1),
                        paths: vec![path(&["a", "c"])],
                    },
                    DuplicateVersion {
                        commit: commit(0xc2),
                        paths: vec![path(&["b", "c"])],
                    },
                ],
            }]
        );
        let text = warnings[0].to_string();
        assert!(text.contains("2개 버전"));
        assert!(text.contains("a > c") && text.contains("b > c"));
        assert!(!text.contains("e.com"));
    }

    #[test]
    fn single_version_paths_produce_no_warning() {
        // diamond(같은 버전) → 경고 없음.
        let mut catalog = Catalog::default();
        catalog.publish(C, 3, "v1", 0xc1, vec![]);
        catalog.publish(A, 1, "v1", 0xa1, vec![decl("c", 3, C, "v1")]);
        catalog.publish(B, 2, "v1", 0xb1, vec![decl("c", 3, C, "v1")]);
        let graph = resolve(&catalog, &[decl("a", 1, A, "v1"), decl("b", 2, B, "v1")]).unwrap();
        assert!(graph.duplicate_library_versions().is_empty());
    }

    #[test]
    fn same_library_two_versions_are_distinct_keys() {
        let v1 = DependencyNodeKey::new(library(3), commit(0x01));
        let v2 = DependencyNodeKey::new(library(3), commit(0x02));
        assert_ne!(v1, v2);
        assert_eq!(v1, DependencyNodeKey::new(library(3), commit(0x01)));

        // 두 버전이 한 map에 동시에 존재.
        let mut nodes = BTreeMap::new();
        nodes.insert(v2.clone(), "v2");
        nodes.insert(v1.clone(), "v1");
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes.values().copied().collect::<Vec<_>>(), ["v1", "v2"]);
        let set: HashSet<_> = [v1.clone(), v2.clone(), v1.clone()].into_iter().collect();
        assert_eq!(set.len(), 2);

        // 정렬: LibraryId가 commit보다 우선.
        let other = DependencyNodeKey::new(library(4), commit(0x00));
        let mut keys = vec![other.clone(), v2.clone(), v1.clone()];
        keys.sort();
        assert_eq!(keys, [v1.clone(), v2, other]);

        assert_eq!(
            v1.to_string(),
            format!("{}@{}", library(3), "01".repeat(20))
        );
    }

    #[test]
    fn key_from_lock_pin() {
        let pin = LockPin {
            name: LogicalName::new("ds").unwrap(),
            library_id: library(1),
            requested: VersionSelector::Tag(RefName::new("v1").unwrap()),
            commit: commit(0x0a),
        };
        assert_eq!(
            DependencyNodeKey::of_pin(&pin),
            DependencyNodeKey::new(library(1), commit(0x0a))
        );
    }
}
