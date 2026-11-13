//! `stellar.lock.yaml` 잠금 파일 데이터 모델 (IV.1.6).
//!
//! 매니페스트([`crate::manifest`])와 **별도**로, resolver가 확정한 의존성 해석 결과를
//! 표현합니다. `resolved`는 **시퀀스**라 같은 라이브러리가 서로 다른 경로/버전으로 여러 번 등장할 수
//! 있습니다(다중 버전).
//!
//! 잠금 파일 형식은 하나뿐입니다(ADR-0021). resolved entry의 스키마와 값 규칙은 의존성 모델
//! [`stellar_dependencies::lock`](XV.4.1·XV.4.2)이 정의하고, 이 모듈은 제한 YAML 텍스트와 그 모델
//! 사이를 연결합니다([`crate::manifest_node`]). IV.1.6의 여섯 필드(logical ID, source, exact
//! commit, content hash, schema version, dependency path)는 같은 이름으로 유지하고, resolver가
//! 기록하는 `library_id`·`repository`·`requested`·`dependencies`를 더한 열 개 필드가 모두
//! 필수입니다.
//!
//! ```yaml
//! schema_version: 1
//! resolved:
//!   - logical_id: c
//!     library_id: <uuid>
//!     source: git
//!     repository: https://example.com/c.git
//!     requested: v1
//!     exact_commit: <40/64 hex>
//!     content_hash: sha256:<64 hex>
//!     schema_version: 4
//!     dependency_path: [a, c]
//!     dependencies: []
//! ```
//!
//! 잠금 파일은 전부 resolver 결과에서 생성됩니다(사용자 편집 필드 금지는 IV.1.7). entry는
//! LibraryId → commit → dependency path 순 canonical 순서여야 하며, 알 수 없는 키·누락 필드·값
//! 규칙 위반·비 canonical 순서를 거부합니다.

use std::fmt;

use stellar_dependencies::lock::{LockDocument, LockEntry, LOCK_SCHEMA_VERSION};
use stellar_dependencies::node::{DeclNode, SchemaError, ValueError};

use crate::manifest_node::{node_to_yaml, yaml_to_node, NodeConversionError};
use crate::yaml::{WriteError, YamlValue};

/// 해석된 의존성 한 건(의존성 모델의 lock entry, XV.4.1·XV.4.2).
pub use stellar_dependencies::lock::LockEntry as ResolvedDependency;

/// 현재 잠금 파일 스키마 버전.
pub const CURRENT_LOCK_SCHEMA_VERSION: u32 = LOCK_SCHEMA_VERSION;

/// 잠금 파일 파싱 오류. 입력 값은 담지 않고 경로만 담습니다(URL 민감값, XV.1.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockError {
    /// 필수 필드 누락.
    MissingField {
        /// 누락된 필드 경로.
        path: String,
    },
    /// 타입 불일치.
    WrongType {
        /// 필드 경로.
        path: String,
        /// 기대 종류(`mapping`/`sequence`/`scalar`).
        expected: &'static str,
    },
    /// 알 수 없는 키(제한 스키마).
    UnknownKey {
        /// 키 경로.
        path: String,
    },
    /// 값 규칙 위반(정수·UUID·commit·content hash 형식, 지원하지 않는 schema version, 필드 모순,
    /// 비 canonical entry 순서).
    InvalidValue {
        /// 필드 경로.
        path: String,
        /// 위반한 규칙.
        error: ValueError,
    },
    /// 컨테이너 중첩 깊이 초과.
    TooDeep {
        /// 상한을 넘은 위치.
        path: String,
    },
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LockError::MissingField { path } => write!(f, "필수 필드 누락: {path}"),
            LockError::WrongType { path, expected } => {
                write!(f, "타입 불일치: {path}는 {expected}여야 합니다")
            }
            LockError::UnknownKey { path } => write!(f, "알 수 없는 키: {path}"),
            LockError::InvalidValue { path, error } => write!(f, "잘못된 값({path}): {error}"),
            LockError::TooDeep { path } => write!(f, "중첩 깊이 상한 초과: {path}"),
        }
    }
}

impl std::error::Error for LockError {}

impl From<SchemaError> for LockError {
    fn from(error: SchemaError) -> LockError {
        match error {
            SchemaError::MissingField { path } => LockError::MissingField { path },
            SchemaError::WrongType { path, expected } => LockError::WrongType { path, expected },
            SchemaError::UnknownKey { path } => LockError::UnknownKey { path },
            SchemaError::InvalidValue { path, error } => LockError::InvalidValue { path, error },
        }
    }
}

impl From<NodeConversionError> for LockError {
    fn from(error: NodeConversionError) -> LockError {
        match error {
            NodeConversionError::NonScalarKey { path } => LockError::WrongType {
                path: format!("{path} 키"),
                expected: "scalar",
            },
            NodeConversionError::TooDeep { path } => LockError::TooDeep { path },
        }
    }
}

/// 잠금 파일 기록 실패. 이 오류가 나면 텍스트를 만들지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockWriteError {
    /// 모델이 lock 규칙을 위반함(지원하지 않는 schema version, 비 canonical entry 순서 등).
    Invalid(LockError),
    /// 제한 loader가 다시 읽지 못할 출력(크기 상한 초과 등).
    Yaml(WriteError),
}

impl fmt::Display for LockWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LockWriteError::Invalid(e) => write!(f, "잠금 파일 모델 오류: {e}"),
            LockWriteError::Yaml(e) => write!(f, "잠금 파일 기록 실패: {e}"),
        }
    }
}

impl std::error::Error for LockWriteError {}

/// `stellar.lock.yaml` 잠금 파일.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lockfile {
    /// 잠금 파일 스키마 버전.
    pub schema_version: u32,
    /// 해석된 의존성(canonical 순서, 같은 라이브러리 다중 등장 가능).
    pub resolved: Vec<ResolvedDependency>,
}

impl Lockfile {
    /// resolver 결과 문서([`LockDocument::from_graph`])에서 잠금 파일을 만듭니다.
    pub fn from_document(document: LockDocument) -> Lockfile {
        Lockfile {
            schema_version: LOCK_SCHEMA_VERSION,
            resolved: document.into_entries(),
        }
    }

    /// 형식 중립 트리(키 순서 고정).
    fn to_node(&self) -> DeclNode {
        DeclNode::Mapping(vec![
            (
                "schema_version".to_string(),
                DeclNode::scalar(self.schema_version.to_string()),
            ),
            (
                "resolved".to_string(),
                DeclNode::Sequence(self.resolved.iter().map(LockEntry::to_node).collect()),
            ),
        ])
    }

    /// 잠금 파일을 [`YamlValue`] 트리로 변환합니다(키 순서 고정).
    pub fn to_yaml(&self) -> YamlValue {
        node_to_yaml(&self.to_node())
    }

    /// 잠금 파일을 canonical 텍스트로 기록합니다.
    ///
    /// 입력은 resolver 결과를 담은 [`Lockfile`]뿐이며, 사용자 편집 필드를 받는 경로가 없습니다.
    /// 고정 필드 모델만 직렬화하므로 알려진 필드 외에는 기록될 수 없습니다(IV.1.7).
    /// 모델 규칙·loader 제한은 검사하지 않으므로 파일로 기록할 때는 [`Lockfile::write_checked`]를
    /// 씁니다.
    pub fn write(&self) -> String {
        crate::yaml::write_canonical(&self.to_yaml())
    }

    /// 잠금 파일을 모델 규칙과 loader 제한을 강제하며 canonical 텍스트로 기록합니다.
    ///
    /// 파일로 기록하는 경로는 이 함수를 씁니다. 다시 읽으면 거부될 모델(지원하지 않는 schema
    /// version, 비 canonical entry 순서 등)과 제한 loader가 다시 읽지 못할 출력(크기 상한 초과
    /// 등)은 텍스트를 만들지 않고 [`LockWriteError`]로 거부합니다.
    pub fn write_checked(&self) -> Result<String, LockWriteError> {
        let node = self.to_node();
        LockDocument::from_node(&node).map_err(|e| LockWriteError::Invalid(LockError::from(e)))?;
        crate::yaml::write_canonical_checked(&node_to_yaml(&node)).map_err(LockWriteError::Yaml)
    }

    /// [`YamlValue`] 트리에서 잠금 파일을 만듭니다. 알 수 없는 키·누락 필드·타입 불일치·값 규칙
    /// 위반·비 canonical entry 순서를 거부합니다(XV.4.2 validator).
    pub fn from_yaml(value: &YamlValue) -> Result<Lockfile, LockError> {
        let node = yaml_to_node(value, "")?;
        let document = LockDocument::from_node(&node)?;
        Ok(Lockfile::from_document(document))
    }

    /// 의존성 모델 문서로 바꿉니다(검증 포함).
    pub fn to_document(&self) -> Result<LockDocument, LockError> {
        Ok(LockDocument::from_node(&self.to_node())?)
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! 테스트 공용 lock entry fixture.

    use stellar_dependencies::content_hash::ContentHashText;
    use stellar_dependencies::declaration::{
        CommitId, LogicalName, RefName, RepositoryUrl, SourceKind, VersionSelector,
    };
    use stellar_dependencies::graph::{DependencyNodeKey, DependencyPath};

    use super::ResolvedDependency;
    use crate::manifest::fixtures::library;

    /// 번호로 결정적 40자리 commit.
    pub(crate) fn commit(n: u8) -> CommitId {
        format!("{n:02x}").repeat(20).parse().expect("commit")
    }

    /// 번호로 결정적 content hash.
    pub(crate) fn hash(n: u8) -> ContentHashText {
        ContentHashText::from_sha256_digest([n; 32])
    }

    /// lock entry(tag `v1` 요청, schema version 1).
    pub(crate) fn entry(
        lib: u8,
        commit_no: u8,
        path: &[&str],
        dependencies: &[(u8, u8)],
    ) -> ResolvedDependency {
        ResolvedDependency {
            library_id: library(lib),
            source: SourceKind::Git,
            repository: RepositoryUrl::new(&format!("https://example.com/lib{lib}.git"))
                .expect("url"),
            requested: VersionSelector::Tag(RefName::new("v1").expect("tag")),
            exact_commit: commit(commit_no),
            content_hash: hash(commit_no),
            schema_version: 1,
            dependency_path: DependencyPath::from_names(
                path.iter()
                    .map(|p| LogicalName::new(p).expect("name"))
                    .collect(),
            ),
            dependencies: dependencies
                .iter()
                .map(|(l, c)| DependencyNodeKey::new(library(*l), commit(*c)))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{commit, entry};
    use super::*;
    use crate::yaml::{load_restricted, write_canonical};
    use stellar_dependencies::lock::LOCK_ENTRY_KEYS;

    /// 같은 라이브러리(design-system, lib 1)가 서로 다른 경로/commit으로 두 번 등장(canonical 순).
    fn multi_version_lock() -> Lockfile {
        Lockfile {
            schema_version: CURRENT_LOCK_SCHEMA_VERSION,
            resolved: vec![
                entry(1, 0xaa, &["design-system"], &[]),
                entry(1, 0xbb, &["icons", "design-system"], &[]),
                entry(2, 0xcc, &["icons"], &[(1, 0xbb)]),
            ],
        }
    }

    fn parse(text: &str) -> Result<Lockfile, LockError> {
        Lockfile::from_yaml(&load_restricted(text).unwrap())
    }

    #[test]
    fn multi_version_same_library_round_trips() {
        let lock = multi_version_lock();
        let restored = Lockfile::from_yaml(&lock.to_yaml()).unwrap();
        assert_eq!(restored, lock);
        // 같은 library가 두 번, 서로 다른 commit·경로로 보존.
        let ds: Vec<&ResolvedDependency> = restored
            .resolved
            .iter()
            .filter(|d| d.logical_id().map(|n| n.as_str()) == Some("design-system"))
            .collect();
        assert_eq!(ds.len(), 2);
        assert_ne!(ds[0].exact_commit, ds[1].exact_commit);
        assert_ne!(ds[0].dependency_path, ds[1].dependency_path);
    }

    #[test]
    fn lock_text_round_trips_through_writer_and_loader() {
        let lock = multi_version_lock();
        let text = write_canonical(&lock.to_yaml());
        assert_eq!(parse(&text).unwrap(), lock);
    }

    #[test]
    fn empty_resolved_round_trips() {
        let lock = Lockfile {
            schema_version: 1,
            resolved: Vec::new(),
        };
        assert_eq!(Lockfile::from_yaml(&lock.to_yaml()).unwrap(), lock);
        // scaffold가 쓰는 빈 잠금 파일 텍스트는 그대로 유효(ADR-0021 호환).
        assert_eq!(
            parse("schema_version: \"1\"\nresolved: []\n").unwrap(),
            lock
        );
    }

    #[test]
    fn rejects_unknown_key() {
        let mut yaml = multi_version_lock().to_yaml();
        if let YamlValue::Mapping(pairs) = &mut yaml {
            pairs.push((YamlValue::scalar("extra"), YamlValue::scalar("x")));
        }
        assert_eq!(
            Lockfile::from_yaml(&yaml),
            Err(LockError::UnknownKey {
                path: "extra".to_string()
            })
        );
    }

    #[test]
    fn rejects_unknown_resolved_entry_key() {
        let mut yaml = multi_version_lock().to_yaml();
        if let YamlValue::Mapping(pairs) = &mut yaml {
            if let YamlValue::Sequence(items) = &mut pairs[1].1 {
                if let YamlValue::Mapping(first) = &mut items[0] {
                    first.push((YamlValue::scalar("oops"), YamlValue::scalar("y")));
                }
            }
        }
        assert_eq!(
            Lockfile::from_yaml(&yaml),
            Err(LockError::UnknownKey {
                path: "resolved[0].oops".to_string()
            })
        );
    }

    #[test]
    fn rejects_missing_field_in_resolved() {
        for field in LOCK_ENTRY_KEYS {
            let mut yaml = multi_version_lock().to_yaml();
            if let YamlValue::Mapping(pairs) = &mut yaml {
                if let YamlValue::Sequence(items) = &mut pairs[1].1 {
                    if let YamlValue::Mapping(first) = &mut items[0] {
                        first.retain(|(k, _)| k.as_scalar() != Some(field));
                    }
                }
            }
            assert_eq!(
                Lockfile::from_yaml(&yaml),
                Err(LockError::MissingField {
                    path: format!("resolved[0].{field}")
                }),
                "{field}"
            );
        }
    }

    #[test]
    fn rejects_pre_unification_six_field_entries() {
        // IV.1.6 초기 6필드 entry(library_id·repository·requested·dependencies 없음)는 받지 않음.
        let text = "schema_version: 1\nresolved:\n  - logical_id: design-system\n    \
                    source: git\n    exact_commit: \"0123456789abcdef0123456789abcdef01234567\"\n    \
                    content_hash: \"sha256:00ff\"\n    schema_version: 1\n    \
                    dependency_path:\n      - design-system\n";
        assert_eq!(
            parse(text),
            Err(LockError::MissingField {
                path: "resolved[0].library_id".to_string()
            })
        );
    }

    #[test]
    fn rejects_invalid_schema_version() {
        // leading-zero·비정수·지원하지 않는 버전.
        for (bad, error) in [
            ("\"01\"", ValueError::InvalidInteger),
            ("x", ValueError::InvalidInteger),
            ("\"2\"", ValueError::UnsupportedVersion),
        ] {
            assert_eq!(
                parse(&format!("schema_version: {bad}\nresolved: []\n")),
                Err(LockError::InvalidValue {
                    path: "schema_version".to_string(),
                    error
                }),
                "{bad}"
            );
        }
        // entry schema_version도 엄격 정수.
        let text = multi_version_lock().write().replacen(
            "schema_version: \"1\"\n    dependency_path",
            "schema_version: \"01\"\n    dependency_path",
            1,
        );
        assert_eq!(
            parse(&text),
            Err(LockError::InvalidValue {
                path: "resolved[0].schema_version".to_string(),
                error: ValueError::InvalidInteger
            })
        );
    }

    #[test]
    fn writer_emits_only_resolver_fields() {
        // 생성된 잠금 파일은 모델의 고정 필드만 포함합니다(사용자 편집 필드 없음).
        let value = load_restricted(&multi_version_lock().write()).unwrap();
        let top_keys: Vec<&str> = value
            .as_mapping()
            .unwrap()
            .iter()
            .filter_map(|(k, _)| k.as_scalar())
            .collect();
        assert_eq!(top_keys, vec!["schema_version", "resolved"]);

        for entry in value.get("resolved").unwrap().as_sequence().unwrap() {
            let entry_keys: Vec<&str> = entry
                .as_mapping()
                .unwrap()
                .iter()
                .filter_map(|(k, _)| k.as_scalar())
                .collect();
            assert_eq!(entry_keys, LOCK_ENTRY_KEYS);
            // IV.1.6 필드는 이름 그대로 포함.
            for kept in [
                "logical_id",
                "source",
                "exact_commit",
                "content_hash",
                "schema_version",
                "dependency_path",
            ] {
                assert!(entry_keys.contains(&kept), "{kept}");
            }
        }
    }

    #[test]
    fn write_is_canonical_and_idempotent_regeneration() {
        // resolver 결과 → 텍스트 → 모델 → 텍스트가 byte-identical(canonical 재생성).
        let lock = multi_version_lock();
        let text1 = lock.write_checked().unwrap();
        let text2 = parse(&text1).unwrap().write_checked().unwrap();
        assert_eq!(text1, text2);
        assert_eq!(text1, lock.write());
    }

    #[test]
    fn rejects_unknown_field_lockfile_without_modifying_input() {
        // 임의 unknown field가 있는 잠금 파일은 거부하고(자동 정리·재작성 없음) 원본을 수정하지 않음.
        let text = "schema_version: 1\nresolved: []\nextra_field: hacked\n";
        let value = load_restricted(text).unwrap();
        let before = value.clone();

        let result = Lockfile::from_yaml(&value);
        assert!(matches!(result, Err(LockError::UnknownKey { .. })));
        // from_yaml은 입력을 읽기만 하므로 원본 트리가 그대로 유지됨.
        assert_eq!(value, before);
    }

    #[test]
    fn rejects_wrong_type_for_resolved() {
        assert_eq!(
            parse("schema_version: 1\nresolved: oops\n"),
            Err(LockError::WrongType {
                path: "resolved".to_string(),
                expected: "sequence"
            })
        );
    }

    #[test]
    fn checked_write_rejects_lockfiles_that_would_not_reload() {
        let lock = multi_version_lock();
        let text = lock.write_checked().unwrap();
        assert_eq!(text, lock.write());
        assert_eq!(parse(&text).unwrap(), lock);

        // 비 canonical entry 순서·지원하지 않는 버전은 모델 오류로 거부(텍스트 없음).
        let mut unsorted = multi_version_lock();
        unsorted.resolved.swap(0, 2);
        assert_eq!(
            unsorted.write_checked(),
            Err(LockWriteError::Invalid(LockError::InvalidValue {
                path: "resolved".to_string(),
                error: ValueError::Inconsistent
            }))
        );
        let mut future = multi_version_lock();
        future.schema_version = 2;
        assert!(matches!(
            future.write_checked(),
            Err(LockWriteError::Invalid(LockError::InvalidValue {
                error: ValueError::UnsupportedVersion,
                ..
            }))
        ));

        // 입력 크기 상한을 넘는 잠금 파일(긴 경로 entry 다수).
        let segment = "p".repeat(stellar_dependencies::declaration::MAX_LOGICAL_NAME_LEN);
        let path: Vec<&str> = (0..64).map(|_| segment.as_str()).collect();
        let big = Lockfile {
            schema_version: CURRENT_LOCK_SCHEMA_VERSION,
            resolved: (0..140u8).map(|c| entry(1, c, &path, &[])).collect(),
        };
        assert!(load_restricted(&big.write()).is_err());
        assert!(matches!(
            big.write_checked(),
            Err(LockWriteError::Yaml(WriteError::OutputTooLarge { .. }))
        ));
        // 모델 값은 길이 상한이 있어 선읽기 상한을 넘는 스칼라를 만들 수 없음.
        assert!(stellar_dependencies::declaration::RepositoryUrl::new(
            &"h".repeat(crate::yaml::loader::MAX_EVENT_LOOKAHEAD_CHARS + 1)
        )
        .is_err());
        assert_eq!(commit(1).as_str().len(), 40);
    }

    #[test]
    fn document_round_trips_between_lockfile_and_dependency_model() {
        let lock = multi_version_lock();
        let document = lock.to_document().unwrap();
        assert_eq!(document.entries(), lock.resolved.as_slice());
        assert_eq!(Lockfile::from_document(document), lock);
    }

    const URL_A: &str = "https://example.com/a.git";
    const URL_B: &str = "https://example.com/b.git";
    const URL_C: &str = "https://example.com/c.git";

    /// root → a(tag v1), b(branch main); a → c v1; b → c v2. 실제 `stellar.yaml` 텍스트(사용
    /// 프로젝트·library 모두 IV.1.1 parser)에서 출발해 resolver 그래프를 만들고 canonical lock
    /// 텍스트로 기록합니다.
    fn resolved_lock_text(root_order_ba: bool) -> String {
        use crate::manifest_source::text_resolver::{
            branch, entry_text, library_text, resolve_root, root_text, tag, TableSelectors,
            TextManifests,
        };

        let mut manifests = TextManifests::default()
            .with(URL_C, commit(0xc1), library_text(0x0c, 1, ""))
            .with(URL_C, commit(0xc2), library_text(0x0c, 2, ""))
            .with(
                URL_A,
                commit(0xa1),
                library_text(0x0a, 1, &entry_text("c", 0x0c, URL_C, " v1")),
            )
            .with(
                URL_B,
                commit(0xb1),
                library_text(0x0b, 1, &entry_text("c", 0x0c, URL_C, " v2")),
            );
        let mut selectors = TableSelectors::default()
            .with(URL_A, tag("v1"), commit(0xa1))
            .with(URL_B, branch("main"), commit(0xb1))
            .with(URL_C, tag("v1"), commit(0xc1))
            .with(URL_C, tag("v2"), commit(0xc2));
        let a = entry_text("a", 0x0a, URL_A, " v1");
        let b = entry_text("b", 0x0b, URL_B, "\n      branch: main");
        let entries = if root_order_ba { b + &a } else { a + &b };
        let graph = resolve_root(&root_text(&entries), &mut selectors, &mut manifests).unwrap();
        // node content hash = commit 번호(실제 cache entry hash는 XV.3.3·XV.3.4 범위).
        let hashes: std::collections::BTreeMap<_, _> = graph
            .nodes()
            .map(|n| {
                let c = u8::from_str_radix(&n.key.commit.as_str()[..2], 16).unwrap();
                (n.key.clone(), super::fixtures::hash(c))
            })
            .collect();
        let document = LockDocument::from_graph(&graph, &hashes).unwrap();
        Lockfile::from_document(document).write_checked().unwrap()
    }

    #[test]
    fn canonical_lock_text_golden_from_resolver_graph() {
        // XV.4.1: resolver 그래프 → stellar.lock.yaml resolved entries·dependency paths 텍스트 golden.
        let text = resolved_lock_text(false);
        assert_eq!(text, CANONICAL_LOCK_GOLDEN);
        // 선언 순서를 바꿔도 byte 동일(XV.2.6), 다시 읽으면 같은 모델(IV.1.6 parser).
        assert_eq!(resolved_lock_text(true), text);
        let reloaded = Lockfile::from_yaml(&load_restricted(&text).unwrap()).unwrap();
        assert_eq!(reloaded.write_checked().unwrap(), text);
    }

    /// root → a(v1), b(branch main); a → c v1; b → c v2의 canonical `stellar.lock.yaml`.
    ///
    /// entry는 edge마다 하나이고 LibraryId → commit → dependency path 순입니다. branch 선언은
    /// `requested`에만 남고 `exact_commit`은 항상 40자 commit입니다(XV.1.2).
    const CANONICAL_LOCK_GOLDEN: &str = r#"schema_version: "1"
resolved:
  -
    logical_id: a
    library_id: "0195c530-3acd-7bc1-8414-00000000000a"
    source: git
    repository: "https://example.com/a.git"
    requested: v1
    exact_commit: a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1
    content_hash: "sha256:a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1"
    schema_version: "1"
    dependency_path:
      - a
    dependencies:
      -
        library_id: "0195c530-3acd-7bc1-8414-00000000000c"
        exact_commit: c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1
  -
    logical_id: b
    library_id: "0195c530-3acd-7bc1-8414-00000000000b"
    source: git
    repository: "https://example.com/b.git"
    requested:
      branch: main
    exact_commit: b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1
    content_hash: "sha256:b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1"
    schema_version: "1"
    dependency_path:
      - b
    dependencies:
      -
        library_id: "0195c530-3acd-7bc1-8414-00000000000c"
        exact_commit: c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2
  -
    logical_id: c
    library_id: "0195c530-3acd-7bc1-8414-00000000000c"
    source: git
    repository: "https://example.com/c.git"
    requested: v1
    exact_commit: c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1
    content_hash: "sha256:c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1"
    schema_version: "1"
    dependency_path:
      - a
      - c
    dependencies: []
  -
    logical_id: c
    library_id: "0195c530-3acd-7bc1-8414-00000000000c"
    source: git
    repository: "https://example.com/c.git"
    requested: v2
    exact_commit: c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2
    content_hash: "sha256:c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2"
    schema_version: "2"
    dependency_path:
      - b
      - c
    dependencies: []
"#;
}
