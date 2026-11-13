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

    /// edge 경로를 선언 순서와 무관한 canonical 경로로 다시 씁니다(XV.2.6).
    ///
    /// dedup된 node의 하위 edge는 처음 도달한 경로를 물려받으므로 탐색 순서에 따라 달라질 수
    /// 있습니다. 각 node의 canonical 경로를 root에서의 최소 경로(길이 → 사전식)로 정하고, edge
    /// 경로를 `canonical(부모) + 선언 이름`으로 바꿉니다. edge 집합(부모, 이름, 자식)은 각 node를
    /// 한 번만 전개하므로 선언 순서와 무관하고, 따라서 결과 경로도 결정적입니다.
    pub(crate) fn canonicalize_paths(&mut self) {
        let mut children: BTreeMap<Option<&DependencyNodeKey>, Vec<usize>> = BTreeMap::new();
        for (i, edge) in self.edges.iter().enumerate() {
            children.entry(edge.parent.as_ref()).or_default().push(i);
        }
        let mut best: BTreeMap<DependencyNodeKey, DependencyPath> = BTreeMap::new();
        let mut queue: BTreeSet<(usize, DependencyPath, DependencyNodeKey)> = BTreeSet::new();
        let push = |queue: &mut BTreeSet<_>, prefix: &DependencyPath, edge: &DependencyEdge| {
            if let Some(name) = edge.path.last() {
                let path = prefix.child(name);
                queue.insert((path.len(), path, edge.child.clone()));
            }
        };
        for &i in children.get(&None).into_iter().flatten() {
            push(&mut queue, &DependencyPath::root(), &self.edges[i]);
        }
        // (길이, 경로) 최소 우선 — 경로를 늘리면 항상 커지므로 처음 확정된 경로가 최소입니다.
        while let Some((_, path, node)) = queue.pop_first() {
            if best.contains_key(&node) {
                continue;
            }
            for &i in children.get(&Some(&node)).into_iter().flatten() {
                if !best.contains_key(&self.edges[i].child) {
                    push(&mut queue, &path, &self.edges[i]);
                }
            }
            best.insert(node, path);
        }
        for edge in &mut self.edges {
            let prefix = match &edge.parent {
                None => Some(DependencyPath::root()),
                Some(parent) => best.get(parent).cloned(),
            };
            if let (Some(prefix), Some(name)) = (prefix, edge.path.last().cloned()) {
                edge.path = prefix.child(&name);
            }
        }
    }

    /// resolver 결과를 LibraryId → commit → dependency path 순으로 정렬한 entry 목록(XV.2.6).
    ///
    /// edge 하나가 entry 하나입니다(같은 library가 서로 다른 경로·버전으로 여러 번 등장 가능).
    pub fn sorted_entries(&self) -> Vec<ResolvedEntry> {
        let mut entries: Vec<ResolvedEntry> = self
            .edges
            .iter()
            .map(|edge| {
                let node = self.nodes.get(&edge.child);
                ResolvedEntry {
                    key: edge.child.clone(),
                    path: edge.path.clone(),
                    requested: edge.requested.clone(),
                    repository: edge.repository.clone(),
                    schema_version: node.map(|n| n.schema_version).unwrap_or_default(),
                    dependencies: node
                        .map(|n| n.dependencies.iter().cloned().collect())
                        .unwrap_or_default(),
                }
            })
            .collect();
        entries.sort_by(|a, b| {
            (&a.key.library_id, &a.key.commit, &a.path).cmp(&(
                &b.key.library_id,
                &b.key.commit,
                &b.path,
            ))
        });
        entries
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

/// 정렬된 resolver 결과 한 건(lockfile resolved entry의 원천, XV.2.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedEntry {
    /// resolved node.
    pub key: DependencyNodeKey,
    /// canonical dependency path.
    pub path: DependencyPath,
    /// 선언 선택자.
    pub requested: VersionSelector,
    /// 선언 repository.
    pub repository: RepositoryUrl,
    /// commit 매니페스트 schema version.
    pub schema_version: u32,
    /// 직접 의존 node(전이 의존성, 정렬).
    pub dependencies: Vec<DependencyNodeKey>,
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

    const D: &str = "https://e.com/d.git";

    /// root → a, b; a → c v1; b → (c v1, x = c v2); c v1 → d. `reverse`면 모든 선언 순서를 뒤집습니다.
    fn shared_descendant_catalog(reverse: bool) -> Catalog {
        let order = |mut v: Vec<crate::declaration::DependencyDeclaration>| {
            if reverse {
                v.reverse();
            }
            v
        };
        let mut catalog = Catalog::default();
        catalog.publish(D, 4, "v1", 0xd1, vec![]);
        catalog.publish(C, 3, "v1", 0xc1, vec![decl("d", 4, D, "v1")]);
        catalog.publish(C, 3, "v2", 0xc2, vec![]);
        catalog.publish(A, 1, "v1", 0xa1, vec![decl("c", 3, C, "v1")]);
        catalog.publish(
            B,
            2,
            "v1",
            0xb1,
            order(vec![decl("c", 3, C, "v1"), decl("x", 3, C, "v2")]),
        );
        catalog
    }

    fn summary(
        entries: &[ResolvedEntry],
    ) -> Vec<(DependencyNodeKey, String, Vec<DependencyNodeKey>)> {
        entries
            .iter()
            .map(|e| (e.key.clone(), e.path.to_string(), e.dependencies.clone()))
            .collect()
    }

    #[test]
    fn sorted_output_is_independent_of_declaration_order() {
        let forward_root = [decl("a", 1, A, "v1"), decl("b", 2, B, "v1")];
        let reverse_root = [decl("b", 2, B, "v1"), decl("a", 1, A, "v1")];
        let forward = resolve(&shared_descendant_catalog(false), &forward_root).unwrap();
        let reversed = resolve(&shared_descendant_catalog(true), &reverse_root).unwrap();
        // 탐색 순서(edge 기록 순서)는 다르지만 정렬 결과·경고는 동일.
        assert_ne!(forward.edges(), reversed.edges());
        assert_eq!(forward.sorted_entries(), reversed.sorted_entries());
        assert_eq!(
            forward.duplicate_library_versions(),
            reversed.duplicate_library_versions()
        );

        let key = |lib: u8, c: u8| DependencyNodeKey::new(library(lib), commit(c));
        // LibraryId → commit → path 정렬, d의 경로는 최소 경로 `a > c > d`(b 경유 선 탐색이어도).
        assert_eq!(
            summary(&forward.sorted_entries()),
            [
                (key(1, 0xa1), "a".to_string(), vec![key(3, 0xc1)]),
                (
                    key(2, 0xb1),
                    "b".to_string(),
                    vec![key(3, 0xc1), key(3, 0xc2)]
                ),
                (key(3, 0xc1), "a > c".to_string(), vec![key(4, 0xd1)]),
                (key(3, 0xc1), "b > c".to_string(), vec![key(4, 0xd1)]),
                (key(3, 0xc2), "b > x".to_string(), vec![]),
                (key(4, 0xd1), "a > c > d".to_string(), vec![]),
            ]
        );
    }

    #[test]
    fn canonical_path_prefers_shorter_then_lexicographic() {
        // root → z → q, root → q 직접: q 하위 r의 경로는 짧은 `q > r`.
        const Q: &str = "https://e.com/q.git";
        const R: &str = "https://e.com/r.git";
        const Z: &str = "https://e.com/z.git";
        let mut catalog = Catalog::default();
        catalog.publish(R, 6, "v1", 0x61, vec![]);
        catalog.publish(Q, 5, "v1", 0x51, vec![decl("r", 6, R, "v1")]);
        catalog.publish(Z, 7, "v1", 0x71, vec![decl("q", 5, Q, "v1")]);
        let graph = resolve(&catalog, &[decl("z", 7, Z, "v1"), decl("q", 5, Q, "v1")]).unwrap();
        let r = DependencyNodeKey::new(library(6), commit(0x61));
        let into_r: Vec<String> = graph.edges_to(&r).map(|e| e.path.to_string()).collect();
        assert_eq!(into_r, ["q > r"]);
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
