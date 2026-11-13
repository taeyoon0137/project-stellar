//! resolver 그래프 → `stellar.lock.yaml` resolved entry 변환 (XV.4.1)과 entry 필드 validator (XV.4.2).
//!
//! lock 문서는 resolver 결과([`DependencyGraph::sorted_entries`])와 검증된 cache entry의 content
//! hash에서만 만듭니다(IV.1.7: 사용자 편집 필드 없음). entry는 edge(부모 선언 한 건)마다 하나이고
//! LibraryId → commit → dependency path 순으로 정렬되므로 선언 순서와 무관하게 같은 문서가 됩니다
//! (XV.2.6). [`DeclNode`] 형식은 다음과 같고, 텍스트 기록은 project crate의 canonical YAML
//! writer(IV.1.5)가 담당합니다.
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
//!     content_hash: sha256:<64 hex>       # 검증된 cache entry content hash (XV.4.2)
//!     schema_version: 4                   # library 매니페스트 schema version (XV.4.2)
//!     dependency_path: [a, c]
//!     dependencies:                       # 전이 의존 node (XV.4.2)
//!       - library_id: <uuid>
//!         exact_commit: <hex>
//! ```

use std::collections::BTreeMap;
use std::fmt;

use crate::content_hash::ContentHashText;
use crate::declaration::{
    CommitId, LibraryId, LogicalName, RepositoryUrl, SourceKind, VersionSelector,
};
use crate::graph::{DependencyGraph, DependencyNodeKey, DependencyPath};
use crate::node::{
    expect_mapping, expect_scalar, expect_sequence, invalid, join, parse_u32_strict,
    reject_unknown, require, DeclNode, SchemaError, ValueError,
};

/// 현재 lock schema version.
pub const LOCK_SCHEMA_VERSION: u32 = 1;

/// lock entry 매핑의 필수 키(canonical 기록 순서). 모두 필수입니다.
pub const LOCK_ENTRY_KEYS: &[&str] = &[
    "logical_id",
    "library_id",
    "source",
    "repository",
    "requested",
    "exact_commit",
    "content_hash",
    "schema_version",
    "dependency_path",
    "dependencies",
];

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
    /// 검증된 cache entry content hash.
    pub content_hash: ContentHashText,
    /// library 매니페스트 schema version.
    pub schema_version: u32,
    /// root에서의 canonical dependency path(비어 있지 않음).
    pub dependency_path: DependencyPath,
    /// 전이 의존 node(정렬).
    pub dependencies: Vec<DependencyNodeKey>,
}

impl LockEntry {
    /// logical name(dependency path 마지막 요소).
    pub fn logical_id(&self) -> Option<&LogicalName> {
        self.dependency_path.last()
    }

    /// node key.
    pub fn key(&self) -> DependencyNodeKey {
        DependencyNodeKey::new(self.library_id, self.exact_commit.clone())
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
                "content_hash".to_string(),
                DeclNode::scalar(self.content_hash.as_str()),
            ),
            (
                "schema_version".to_string(),
                DeclNode::scalar(self.schema_version.to_string()),
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
            (
                "dependencies".to_string(),
                DeclNode::Sequence(
                    self.dependencies
                        .iter()
                        .map(|k| {
                            DeclNode::Mapping(vec![
                                (
                                    "library_id".to_string(),
                                    DeclNode::scalar(k.library_id.to_string()),
                                ),
                                (
                                    "exact_commit".to_string(),
                                    DeclNode::scalar(k.commit.as_str()),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }

    /// 매핑에서 entry를 읽습니다. 모든 필드가 필수이며 누락·알 수 없는 키·모순을 거부합니다.
    pub fn from_node(node: &DeclNode, path: &str) -> Result<LockEntry, SchemaError> {
        let pairs = expect_mapping(node, path)?;
        reject_unknown(pairs, path, LOCK_ENTRY_KEYS)?;
        for key in LOCK_ENTRY_KEYS {
            require(pairs, key, path)?;
        }
        let scalar = |key: &str| -> Result<(&str, String), SchemaError> {
            let field = join(path, key);
            let text = expect_scalar(require(pairs, key, path)?, &field)?;
            Ok((text, field))
        };

        let (text, field) = scalar("library_id")?;
        let library_id = text.parse::<LibraryId>().map_err(|e| invalid(&field, e))?;
        let (text, field) = scalar("source")?;
        let source = text.parse::<SourceKind>().map_err(|e| invalid(&field, e))?;
        let (text, field) = scalar("repository")?;
        let repository = RepositoryUrl::new(text).map_err(|e| invalid(&field, e))?;
        let requested = VersionSelector::from_node(
            require(pairs, "requested", path)?,
            &join(path, "requested"),
        )?;
        let (text, field) = scalar("exact_commit")?;
        let exact_commit = text.parse::<CommitId>().map_err(|e| invalid(&field, e))?;
        let (text, field) = scalar("content_hash")?;
        let content_hash = text
            .parse::<ContentHashText>()
            .map_err(|e| invalid(&field, e))?;
        let (text, field) = scalar("schema_version")?;
        let schema_version =
            parse_u32_strict(text).ok_or_else(|| invalid(&field, ValueError::InvalidInteger))?;

        let path_field = join(path, "dependency_path");
        let names = expect_sequence(require(pairs, "dependency_path", path)?, &path_field)?;
        if names.is_empty() {
            return Err(invalid(&path_field, ValueError::Empty));
        }
        let mut segments = Vec::with_capacity(names.len());
        for (i, name) in names.iter().enumerate() {
            let segment_path = format!("{path_field}[{i}]");
            let text = expect_scalar(name, &segment_path)?;
            segments.push(LogicalName::new(text).map_err(|e| invalid(&segment_path, e))?);
        }
        let dependency_path = DependencyPath::from_names(segments);

        let (text, field) = scalar("logical_id")?;
        if dependency_path.last().map(LogicalName::as_str) != Some(text) {
            return Err(invalid(&field, ValueError::Inconsistent));
        }

        let deps_field = join(path, "dependencies");
        let items = expect_sequence(require(pairs, "dependencies", path)?, &deps_field)?;
        let mut dependencies = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            let item_path = format!("{deps_field}[{i}]");
            let item_pairs = expect_mapping(item, &item_path)?;
            reject_unknown(item_pairs, &item_path, &["library_id", "exact_commit"])?;
            let id_field = join(&item_path, "library_id");
            let id = expect_scalar(require(item_pairs, "library_id", &item_path)?, &id_field)?
                .parse::<LibraryId>()
                .map_err(|e| invalid(&id_field, e))?;
            let commit_field = join(&item_path, "exact_commit");
            let commit = expect_scalar(
                require(item_pairs, "exact_commit", &item_path)?,
                &commit_field,
            )?
            .parse::<CommitId>()
            .map_err(|e| invalid(&commit_field, e))?;
            dependencies.push(DependencyNodeKey::new(id, commit));
        }
        if dependencies.windows(2).any(|w| w[0] >= w[1]) {
            return Err(invalid(&deps_field, ValueError::Inconsistent));
        }

        Ok(LockEntry {
            library_id,
            source,
            repository,
            requested,
            exact_commit,
            content_hash,
            schema_version,
            dependency_path,
            dependencies,
        })
    }
}

/// lock 문서 생성 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockBuildError {
    /// node의 검증된 cache content hash가 없음(lock에 기록할 수 없음).
    MissingContentHash {
        /// 대상 node.
        key: DependencyNodeKey,
    },
}

impl fmt::Display for LockBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LockBuildError::MissingContentHash { key } => {
                write!(f, "검증된 cache content hash가 없습니다: {key}")
            }
        }
    }
}

impl std::error::Error for LockBuildError {}

/// lock 문서(resolved entry 목록, 정렬 상태 유지).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockDocument {
    entries: Vec<LockEntry>,
}

impl LockDocument {
    /// resolver 그래프와 node별 검증된 cache content hash에서 lock 문서를 만듭니다.
    pub fn from_graph(
        graph: &DependencyGraph,
        content_hashes: &BTreeMap<DependencyNodeKey, ContentHashText>,
    ) -> Result<LockDocument, LockBuildError> {
        let mut entries = Vec::new();
        for e in graph.sorted_entries() {
            let content_hash = content_hashes
                .get(&e.key)
                .cloned()
                .ok_or_else(|| LockBuildError::MissingContentHash { key: e.key.clone() })?;
            entries.push(LockEntry {
                library_id: e.key.library_id,
                source: SourceKind::Git,
                repository: e.repository,
                requested: e.requested,
                exact_commit: e.key.commit,
                content_hash,
                schema_version: e.schema_version,
                dependency_path: e.path,
                dependencies: e.dependencies,
            });
        }
        Ok(LockDocument { entries })
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

    /// lock 문서를 읽고 검증합니다(XV.4.2 validator).
    ///
    /// 모든 entry 필드가 필수이고, 알 수 없는 키·잘못된 값·`logical_id` 모순·비 canonical 순서를
    /// 거부합니다(lock은 resolver 출력만 허용, IV.1.7).
    pub fn from_node(node: &DeclNode) -> Result<LockDocument, SchemaError> {
        let pairs = expect_mapping(node, "")?;
        reject_unknown(pairs, "", &["schema_version", "resolved"])?;
        let version = expect_scalar(require(pairs, "schema_version", "")?, "schema_version")?;
        match parse_u32_strict(version) {
            Some(LOCK_SCHEMA_VERSION) => {}
            Some(_) => return Err(invalid("schema_version", ValueError::UnsupportedVersion)),
            None => return Err(invalid("schema_version", ValueError::InvalidInteger)),
        }
        let items = expect_sequence(require(pairs, "resolved", "")?, "resolved")?;
        let mut entries = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            entries.push(LockEntry::from_node(item, &format!("resolved[{i}]"))?);
        }
        let sort_key = |e: &LockEntry| {
            (
                e.library_id,
                e.exact_commit.clone(),
                e.dependency_path.clone(),
            )
        };
        if entries
            .windows(2)
            .any(|w| sort_key(&w[0]) >= sort_key(&w[1]))
        {
            return Err(invalid("resolved", ValueError::Inconsistent));
        }
        Ok(LockDocument { entries })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache_index::fixtures::hash;
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

    fn key(lib: u8, c: u8) -> DependencyNodeKey {
        DependencyNodeKey::new(library(lib), commit(c))
    }

    /// 각 node의 content hash = hash(commit 번호).
    fn hashes() -> BTreeMap<DependencyNodeKey, ContentHashText> {
        [(1, 0xa1), (2, 0xb1), (3, 0xc1), (3, 0xc2)]
            .into_iter()
            .map(|(lib, c)| (key(lib, c), hash(c)))
            .collect()
    }

    fn lock() -> LockDocument {
        let graph = resolve(&catalog(), &[decl("a", 1, A, "v1"), decl("b", 2, B, "v1")]).unwrap();
        LockDocument::from_graph(&graph, &hashes()).unwrap()
    }

    fn dep_node(lib: u8, c: u8) -> DeclNode {
        DeclNode::Mapping(vec![
            (
                "library_id".to_string(),
                DeclNode::scalar(library(lib).to_string()),
            ),
            (
                "exact_commit".to_string(),
                DeclNode::scalar(commit(c).as_str()),
            ),
        ])
    }

    fn entry_node(lib: u8, url: &str, c: u8, path: &[&str], deps: Vec<DeclNode>) -> DeclNode {
        DeclNode::Mapping(vec![
            (
                "logical_id".to_string(),
                DeclNode::scalar(*path.last().unwrap()),
            ),
            (
                "library_id".to_string(),
                DeclNode::scalar(library(lib).to_string()),
            ),
            ("source".to_string(), DeclNode::scalar("git")),
            ("repository".to_string(), DeclNode::scalar(url)),
            (
                "requested".to_string(),
                DeclNode::scalar(if c == 0xc2 { "v2" } else { "v1" }),
            ),
            (
                "exact_commit".to_string(),
                DeclNode::scalar(commit(c).as_str()),
            ),
            (
                "content_hash".to_string(),
                DeclNode::scalar(hash(c).as_str()),
            ),
            ("schema_version".to_string(), DeclNode::scalar("1")),
            (
                "dependency_path".to_string(),
                DeclNode::Sequence(path.iter().map(|p| DeclNode::scalar(*p)).collect()),
            ),
            ("dependencies".to_string(), DeclNode::Sequence(deps)),
        ])
    }

    #[test]
    fn canonical_lock_golden() {
        let lock = lock();
        let expected = DeclNode::Mapping(vec![
            ("schema_version".to_string(), DeclNode::scalar("1")),
            (
                "resolved".to_string(),
                DeclNode::Sequence(vec![
                    entry_node(1, A, 0xa1, &["a"], vec![dep_node(3, 0xc1)]),
                    entry_node(2, B, 0xb1, &["b"], vec![dep_node(3, 0xc2)]),
                    entry_node(3, C, 0xc1, &["a", "c"], vec![]),
                    entry_node(3, C, 0xc2, &["b", "c"], vec![]),
                ]),
            ),
        ]);
        assert_eq!(lock.to_node(), expected);

        // 선언 순서를 바꿔도 같은 lock 문서.
        let reordered =
            resolve(&catalog(), &[decl("b", 2, B, "v1"), decl("a", 1, A, "v1")]).unwrap();
        assert_eq!(
            LockDocument::from_graph(&reordered, &hashes())
                .unwrap()
                .to_node(),
            expected
        );
        // exact commit만 기록(branch·tag 이름은 requested에만).
        assert!(lock
            .entries()
            .iter()
            .all(|e| e.exact_commit.as_str().len() == 40));
        assert_eq!(lock.entries()[2].logical_id().unwrap().as_str(), "c");
    }

    #[test]
    fn every_entry_records_commit_hash_schema_and_transitive_dependencies() {
        let lock = lock();
        let a = &lock.entries()[0];
        assert_eq!(a.exact_commit, commit(0xa1));
        assert_eq!(a.content_hash, hash(0xa1));
        assert_eq!(a.schema_version, 1);
        assert_eq!(a.dependencies, [key(3, 0xc1)]);
        // round-trip.
        assert_eq!(LockDocument::from_node(&lock.to_node()).unwrap(), lock);
        // 검증된 content hash가 없는 node는 lock을 만들 수 없음.
        let mut partial = hashes();
        partial.remove(&key(3, 0xc2));
        let graph = resolve(&catalog(), &[decl("a", 1, A, "v1"), decl("b", 2, B, "v1")]).unwrap();
        assert_eq!(
            LockDocument::from_graph(&graph, &partial),
            Err(LockBuildError::MissingContentHash { key: key(3, 0xc2) })
        );
    }

    /// 첫 entry의 필드를 바꾼 문서.
    fn tamper(f: impl FnOnce(&mut Vec<(String, DeclNode)>)) -> DeclNode {
        let DeclNode::Mapping(mut top) = lock().to_node() else {
            unreachable!()
        };
        let DeclNode::Sequence(items) = &mut top[1].1 else {
            unreachable!()
        };
        let DeclNode::Mapping(first) = &mut items[0] else {
            unreachable!()
        };
        f(first);
        DeclNode::Mapping(top)
    }

    #[test]
    fn missing_field_validator_fails() {
        for field in [
            "exact_commit",
            "content_hash",
            "schema_version",
            "dependencies",
            "dependency_path",
            "logical_id",
            "library_id",
            "source",
            "repository",
            "requested",
        ] {
            let node = tamper(|m| m.retain(|(k, _)| k != field));
            assert_eq!(
                LockDocument::from_node(&node),
                Err(SchemaError::MissingField {
                    path: format!("resolved[0].{field}")
                }),
                "{field}"
            );
        }
        // 잘못된 값·모순·알 수 없는 키·지원하지 않는 버전·비 canonical 순서.
        let set = |key: &'static str, value: DeclNode| {
            tamper(move |m| {
                for (k, v) in m.iter_mut() {
                    if k == key {
                        *v = value.clone();
                    }
                }
            })
        };
        let err = |node: DeclNode| match LockDocument::from_node(&node) {
            Err(SchemaError::InvalidValue { path, error }) => (path, error),
            other => panic!("InvalidValue 기대: {other:?}"),
        };
        assert_eq!(
            err(set("content_hash", DeclNode::scalar("sha256:abc"))),
            (
                "resolved[0].content_hash".to_string(),
                ValueError::InvalidContentHash
            )
        );
        assert_eq!(
            err(set("schema_version", DeclNode::scalar("01"))).1,
            ValueError::InvalidInteger
        );
        assert_eq!(
            err(set("logical_id", DeclNode::scalar("zz"))).1,
            ValueError::Inconsistent
        );
        assert_eq!(
            err(set("dependency_path", DeclNode::Sequence(vec![]))).1,
            ValueError::Empty
        );
        assert_eq!(
            err(set("exact_commit", DeclNode::scalar("a1a1a1"))).1,
            ValueError::InvalidCommitLength
        );
        assert_eq!(
            LockDocument::from_node(&tamper(
                |m| m.push(("note".to_string(), DeclNode::scalar("x")))
            )),
            Err(SchemaError::UnknownKey {
                path: "resolved[0].note".to_string()
            })
        );
        let DeclNode::Mapping(mut top) = lock().to_node() else {
            unreachable!()
        };
        top[0].1 = DeclNode::scalar("2");
        assert_eq!(
            LockDocument::from_node(&DeclNode::Mapping(top.clone())),
            Err(SchemaError::InvalidValue {
                path: "schema_version".to_string(),
                error: ValueError::UnsupportedVersion
            })
        );
        top[0].1 = DeclNode::scalar("1");
        if let DeclNode::Sequence(items) = &mut top[1].1 {
            items.swap(0, 1);
        }
        assert_eq!(
            LockDocument::from_node(&DeclNode::Mapping(top)),
            Err(SchemaError::InvalidValue {
                path: "resolved".to_string(),
                error: ValueError::Inconsistent
            })
        );
    }
}
