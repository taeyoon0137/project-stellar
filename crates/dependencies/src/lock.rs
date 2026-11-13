//! resolver 그래프 → `stellar.lock.yaml` resolved entry 변환 (XV.4.1).
//!
//! lock 문서는 resolver 결과([`DependencyGraph::sorted_entries`])에서만 만듭니다(IV.1.7: 사용자 편집
//! 필드 없음). entry는 edge(부모 선언 한 건)마다 하나이고 LibraryId → commit → dependency path 순으로
//! 정렬되므로 선언 순서와 무관하게 같은 문서가 됩니다(XV.2.6). [`DeclNode`] 형식은 다음과 같고,
//! 텍스트 기록은 project crate의 canonical YAML writer(IV.1.5)가 담당합니다.
//!
//! ```yaml
//! schema_version: 1
//! resolved:
//!   - logical_id: c                      # dependency path 마지막 이름
//!     library_id: <uuid>
//!     source: git
//!     repository: https://example.com/c.git
//!     requested: v1                       # 선언 선택자(`version`과 같은 형식)
//!     exact_commit: <40/64 hex>
//!     dependency_path: [a, c]
//! ```

use crate::declaration::{
    CommitId, LibraryId, LogicalName, RepositoryUrl, SourceKind, VersionSelector,
};
use crate::graph::{DependencyGraph, DependencyPath};
use crate::node::DeclNode;

/// 현재 lock schema version.
pub const LOCK_SCHEMA_VERSION: u32 = 1;

/// lock resolved entry 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockEntry {
    /// 라이브러리 식별자.
    pub library_id: LibraryId,
    /// source 종류.
    pub source: SourceKind,
    /// 선언 repository.
    pub repository: RepositoryUrl,
    /// 선언 선택자.
    pub requested: VersionSelector,
    /// exact commit.
    pub exact_commit: CommitId,
    /// root에서의 canonical dependency path(비어 있지 않음).
    pub dependency_path: DependencyPath,
}

impl LockEntry {
    /// logical name(dependency path 마지막 요소).
    pub fn logical_id(&self) -> Option<&LogicalName> {
        self.dependency_path.last()
    }

    /// canonical [`DeclNode`] 매핑(키 순서 고정).
    pub fn to_node(&self) -> DeclNode {
        let logical = self
            .logical_id()
            .map(|n| n.as_str().to_string())
            .unwrap_or_default();
        DeclNode::Mapping(vec![
            ("logical_id".to_string(), DeclNode::scalar(logical)),
            (
                "library_id".to_string(),
                DeclNode::scalar(self.library_id.to_string()),
            ),
            ("source".to_string(), DeclNode::scalar(self.source.as_str())),
            (
                "repository".to_string(),
                DeclNode::scalar(self.repository.reveal()),
            ),
            ("requested".to_string(), self.requested.to_node()),
            (
                "exact_commit".to_string(),
                DeclNode::scalar(self.exact_commit.as_str()),
            ),
            (
                "dependency_path".to_string(),
                DeclNode::Sequence(
                    self.dependency_path
                        .names()
                        .iter()
                        .map(|n| DeclNode::scalar(n.as_str()))
                        .collect(),
                ),
            ),
        ])
    }
}

/// lock 문서(resolved entry 목록, 정렬 상태 유지).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockDocument {
    entries: Vec<LockEntry>,
}

impl LockDocument {
    /// resolver 그래프에서 lock 문서를 만듭니다.
    pub fn from_graph(graph: &DependencyGraph) -> LockDocument {
        let entries = graph
            .sorted_entries()
            .into_iter()
            .map(|e| LockEntry {
                library_id: e.key.library_id,
                source: SourceKind::Git,
                repository: e.repository,
                requested: e.requested,
                exact_commit: e.key.commit,
                dependency_path: e.path,
            })
            .collect();
        LockDocument { entries }
    }

    /// entry(정렬 순).
    pub fn entries(&self) -> &[LockEntry] {
        &self.entries
    }

    /// canonical [`DeclNode`] 문서(`schema_version`, `resolved`).
    pub fn to_node(&self) -> DeclNode {
        DeclNode::Mapping(vec![
            (
                "schema_version".to_string(),
                DeclNode::scalar(LOCK_SCHEMA_VERSION.to_string()),
            ),
            (
                "resolved".to_string(),
                DeclNode::Sequence(self.entries.iter().map(LockEntry::to_node).collect()),
            ),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declaration::fixtures::{commit, decl, library};
    use crate::resolver::fake::{resolve, Catalog};

    const A: &str = "https://e.com/a.git";
    const B: &str = "https://e.com/b.git";
    const C: &str = "https://e.com/c.git";

    /// root → a, b; a → c v1; b → c v2 (다중 버전).
    fn catalog() -> Catalog {
        let mut catalog = Catalog::default();
        catalog.publish(C, 3, "v1", 0xc1, vec![]);
        catalog.publish(C, 3, "v2", 0xc2, vec![]);
        catalog.publish(A, 1, "v1", 0xa1, vec![decl("c", 3, C, "v1")]);
        catalog.publish(B, 2, "v1", 0xb1, vec![decl("c", 3, C, "v2")]);
        catalog
    }

    fn entry_node(logical: &str, lib: u8, url: &str, tag: &str, c: u8, path: &[&str]) -> DeclNode {
        DeclNode::Mapping(vec![
            ("logical_id".to_string(), DeclNode::scalar(logical)),
            (
                "library_id".to_string(),
                DeclNode::scalar(library(lib).to_string()),
            ),
            ("source".to_string(), DeclNode::scalar("git")),
            ("repository".to_string(), DeclNode::scalar(url)),
            ("requested".to_string(), DeclNode::scalar(tag)),
            (
                "exact_commit".to_string(),
                DeclNode::scalar(commit(c).as_str()),
            ),
            (
                "dependency_path".to_string(),
                DeclNode::Sequence(path.iter().map(|p| DeclNode::scalar(*p)).collect()),
            ),
        ])
    }

    #[test]
    fn canonical_lock_golden() {
        let graph = resolve(&catalog(), &[decl("a", 1, A, "v1"), decl("b", 2, B, "v1")]).unwrap();
        let lock = LockDocument::from_graph(&graph);
        let expected = DeclNode::Mapping(vec![
            ("schema_version".to_string(), DeclNode::scalar("1")),
            (
                "resolved".to_string(),
                DeclNode::Sequence(vec![
                    entry_node("a", 1, A, "v1", 0xa1, &["a"]),
                    entry_node("b", 2, B, "v1", 0xb1, &["b"]),
                    entry_node("c", 3, C, "v1", 0xc1, &["a", "c"]),
                    entry_node("c", 3, C, "v2", 0xc2, &["b", "c"]),
                ]),
            ),
        ]);
        assert_eq!(lock.to_node(), expected);

        // 선언 순서를 바꿔도 같은 lock 문서.
        let reordered =
            resolve(&catalog(), &[decl("b", 2, B, "v1"), decl("a", 1, A, "v1")]).unwrap();
        assert_eq!(LockDocument::from_graph(&reordered).to_node(), expected);
        // exact commit만 기록(branch·tag 이름은 requested에만).
        assert!(lock
            .entries()
            .iter()
            .all(|e| e.exact_commit.as_str().len() == 40));
        assert_eq!(lock.entries()[2].logical_id().unwrap().as_str(), "c");
    }
}
