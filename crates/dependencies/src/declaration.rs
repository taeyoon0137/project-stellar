//! `stellar.yaml` 의존성 선언 모델 (XV.1.1).
//!
//! 한 의존성 entry는 logical name(매핑 키), library id, source kind(Git), repository URL,
//! version 선택자(tag/branch/commit), 선택적 local override로 구성됩니다(기술 아키텍처 20·30장).
//! [`DeclNode`] 형식은 다음과 같습니다.
//!
//! ```yaml
//! dependencies:
//!   design-system:
//!     source: git
//!     library_id: 0195c530-3acd-7bc1-8414-c213f61cb965
//!     repository: git@github.com:example/design-system.git
//!     version: v2.4.0            # tag(아키텍처 20.3 예시와 같은 스칼라 형식)
//!   icons:
//!     source: git
//!     library_id: 0195c530-3acd-7bc1-8414-c213f61cb966
//!     repository: https://example.com/icons.git
//!     version:
//!       branch: main             # 또는 commit: <40/64자리 hex>
//!     local_override: ../icons
//!     components:                # 이 프로젝트가 쓰는 외부 component(StableId, ADR-0137)
//!       - 0195c530-3acd-7bc1-8414-c213f61cb9a0
//! ```
//!
//! tag는 canonical 형식에서 스칼라로, branch·commit은 한 키짜리 매핑으로 기록합니다. 입력에서는
//! `version: {tag: ...}` 매핑도 받아 스칼라로 정규화합니다. 알 수 없는 키는 거부합니다.
//!
//! `components`는 사용 프로젝트 문서의 instance가 가리키는 외부 component 참조를 저장합니다(ADR-0137 결정 1).
//! 한 항목은 [`ExternalComponentRef`](crate::external_ref::ExternalComponentRef)의 세 값(선언 이름 = 매핑 키, LibraryId
//! = `library_id`, component StableId = 항목) 가운데 StableId입니다. canonical 형식은 StableId 오름차순이고 비어 있으면
//! 키를 쓰지 않습니다. 입력의 중복·canonical이 아닌 UUID 표기는 거부하고 순서는 정규화합니다.

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::ComponentRef;

use crate::node::{
    expect_mapping, expect_scalar, invalid, join, optional, reject_unknown, require, DeclNode,
    SchemaError, ValueError,
};

/// logical name 최대 byte 수.
pub const MAX_LOGICAL_NAME_LEN: usize = 128;
/// repository URL 최대 byte 수.
pub const MAX_REPOSITORY_URL_LEN: usize = 4096;
/// Git ref 이름(tag·branch) 최대 byte 수.
pub const MAX_REF_NAME_LEN: usize = 255;
/// local override 경로 최대 byte 수.
pub const MAX_LOCAL_OVERRIDE_LEN: usize = 4096;
/// 의존성 하나가 저장하는 외부 component 참조 최대 수(ADR-0137 결정 1).
pub const MAX_COMPONENT_IMPORTS: usize = 4096;

/// 라이브러리 식별자: 라이브러리 프로젝트 `stellar.yaml`의 `project.id`(UUID).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LibraryId(StableId);

impl LibraryId {
    /// `StableId`로부터 만듭니다.
    pub const fn from_stable_id(id: StableId) -> LibraryId {
        LibraryId(id)
    }

    /// 내부 `StableId`.
    pub const fn stable_id(&self) -> StableId {
        self.0
    }
}

impl fmt::Display for LibraryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Debug for LibraryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LibraryId({})", self.0)
    }
}

impl FromStr for LibraryId {
    type Err = ValueError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<StableId>()
            .map(LibraryId)
            .map_err(|_| ValueError::InvalidLibraryId)
    }
}

/// 의존성 logical name(`dependencies` 매핑 키, dependency path 구성 요소).
///
/// ASCII 영숫자로 시작하고 영숫자·`-`·`_`·`.`만 씁니다. lock key(`name@...`)와 경로 구분에
/// 쓰는 `@`·`/`·공백은 허용하지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LogicalName(String);

impl LogicalName {
    /// 규칙을 검사해 만듭니다.
    pub fn new(text: &str) -> Result<LogicalName, ValueError> {
        if text.is_empty() {
            return Err(ValueError::Empty);
        }
        if text.len() > MAX_LOGICAL_NAME_LEN {
            return Err(ValueError::TooLong {
                max: MAX_LOGICAL_NAME_LEN,
            });
        }
        let bytes = text.as_bytes();
        if !bytes[0].is_ascii_alphanumeric() {
            return Err(ValueError::InvalidCharacter);
        }
        if !bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        {
            return Err(ValueError::InvalidCharacter);
        }
        Ok(LogicalName(text.to_string()))
    }

    /// 문자열.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LogicalName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 의존성 source 종류. 현재는 Git만 지원합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceKind {
    /// 독립 Git 저장소(기술 아키텍처 30.1).
    Git,
}

impl SourceKind {
    /// 안정 이름.
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Git => "git",
        }
    }
}

impl FromStr for SourceKind {
    type Err = ValueError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "git" => Ok(SourceKind::Git),
            _ => Err(ValueError::UnknownSource),
        }
    }
}

/// repository URL(시스템 Git이 해석하는 원격 주소).
///
/// 제어 문자·공백을 금지하고, Git option으로 해석될 수 있는 `-` 시작과 명령을 실행하는 remote
/// helper transport(`ext::`, `fd::`)를 거부합니다.
///
/// telemetry·진단 민감값입니다(XV.1.4): `Debug`는 값을 가리고 `Display`는 구현하지 않으며, 원문은
/// [`RepositoryUrl::reveal`]로만 얻습니다.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepositoryUrl(String);

impl RepositoryUrl {
    /// 규칙을 검사해 만듭니다.
    pub fn new(text: &str) -> Result<RepositoryUrl, ValueError> {
        if text.is_empty() {
            return Err(ValueError::Empty);
        }
        if text.len() > MAX_REPOSITORY_URL_LEN {
            return Err(ValueError::TooLong {
                max: MAX_REPOSITORY_URL_LEN,
            });
        }
        if text.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(ValueError::InvalidCharacter);
        }
        if text.starts_with('-') {
            return Err(ValueError::LeadingDash);
        }
        let lower = text.to_ascii_lowercase();
        if lower.starts_with("ext::") || lower.starts_with("fd::") {
            return Err(ValueError::ForbiddenTransport);
        }
        Ok(RepositoryUrl(text.to_string()))
    }

    /// 원문 문자열. Git 인자·로컬 UI 표시처럼 명시적 경로에서만 사용합니다(XV.1.4).
    pub fn reveal(&self) -> &str {
        &self.0
    }
}

/// Git commit object id(SHA-1 40자 또는 SHA-256 64자, 소문자 hex로 정규화).
///
/// 축약 hash는 허용하지 않습니다. lockfile은 항상 이 타입만 기록합니다(XV.1.2).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommitId(String);

impl CommitId {
    /// 소문자 hex 문자열.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for CommitId {
    type Err = ValueError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 40 && s.len() != 64 {
            return Err(ValueError::InvalidCommitLength);
        }
        if !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ValueError::InvalidHexDigit);
        }
        Ok(CommitId(s.to_ascii_lowercase()))
    }
}

impl fmt::Display for CommitId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for CommitId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CommitId({})", self.0)
    }
}

/// tag·branch 이름(`git check-ref-format` 규칙의 보수적 부분집합).
///
/// 제어 문자·공백·`~^:?*[\`, `..`, `@{`, `//`, `-`·`/` 시작, `/`·`.` 끝, `.` 시작 component,
/// `.lock` 끝 component, 단독 `@`를 거부합니다.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RefName(String);

impl RefName {
    /// 규칙을 검사해 만듭니다.
    pub fn new(text: &str) -> Result<RefName, ValueError> {
        if text.is_empty() {
            return Err(ValueError::Empty);
        }
        if text.len() > MAX_REF_NAME_LEN {
            return Err(ValueError::TooLong {
                max: MAX_REF_NAME_LEN,
            });
        }
        if text
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || "~^:?*[\\".contains(c))
        {
            return Err(ValueError::InvalidCharacter);
        }
        if text.starts_with('-') {
            return Err(ValueError::LeadingDash);
        }
        let bad_shape = text == "@"
            || text.starts_with('/')
            || text.ends_with('/')
            || text.ends_with('.')
            || text.contains("..")
            || text.contains("@{")
            || text.contains("//")
            || text
                .split('/')
                .any(|part| part.starts_with('.') || part.ends_with(".lock"));
        if bad_shape {
            return Err(ValueError::InvalidRefName);
        }
        Ok(RefName(text.to_string()))
    }

    /// 문자열.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RefName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// version 선택자 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SelectorKind {
    /// tag.
    Tag,
    /// branch.
    Branch,
    /// exact commit.
    Commit,
}

impl SelectorKind {
    /// 안정 이름(`version` 매핑 키와 같음).
    pub fn as_str(self) -> &'static str {
        match self {
            SelectorKind::Tag => "tag",
            SelectorKind::Branch => "branch",
            SelectorKind::Commit => "commit",
        }
    }
}

/// version 선택자.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VersionSelector {
    /// tag 이름.
    Tag(RefName),
    /// branch 이름.
    Branch(RefName),
    /// exact commit.
    Commit(CommitId),
}

impl VersionSelector {
    /// 선택자 종류.
    pub fn kind(&self) -> SelectorKind {
        match self {
            VersionSelector::Tag(_) => SelectorKind::Tag,
            VersionSelector::Branch(_) => SelectorKind::Branch,
            VersionSelector::Commit(_) => SelectorKind::Commit,
        }
    }

    /// 선택자 값 문자열(tag·branch 이름 또는 commit hex).
    pub fn value(&self) -> &str {
        match self {
            VersionSelector::Tag(name) | VersionSelector::Branch(name) => name.as_str(),
            VersionSelector::Commit(commit) => commit.as_str(),
        }
    }

    /// canonical [`DeclNode`] 형식.
    pub fn to_node(&self) -> DeclNode {
        match self {
            VersionSelector::Tag(name) => DeclNode::scalar(name.as_str()),
            other => DeclNode::Mapping(vec![(
                other.kind().as_str().to_string(),
                DeclNode::scalar(other.value()),
            )]),
        }
    }

    /// [`DeclNode`]에서 읽습니다. 스칼라는 tag, 매핑은 정확히 한 키(`tag`/`branch`/`commit`).
    pub fn from_node(node: &DeclNode, path: &str) -> Result<VersionSelector, SchemaError> {
        if let Some(text) = node.as_scalar() {
            return RefName::new(text)
                .map(VersionSelector::Tag)
                .map_err(|e| invalid(path, e));
        }
        let pairs = node.as_mapping().ok_or_else(|| SchemaError::WrongType {
            path: path.to_string(),
            expected: "scalar 또는 mapping",
        })?;
        let [(key, value)] = pairs else {
            return Err(invalid(path, ValueError::InvalidSelector));
        };
        let value_path = join(path, key);
        let text = expect_scalar(value, &value_path)?;
        match key.as_str() {
            "tag" => RefName::new(text)
                .map(VersionSelector::Tag)
                .map_err(|e| invalid(&value_path, e)),
            "branch" => RefName::new(text)
                .map(VersionSelector::Branch)
                .map_err(|e| invalid(&value_path, e)),
            "commit" => text
                .parse::<CommitId>()
                .map(VersionSelector::Commit)
                .map_err(|e| invalid(&value_path, e)),
            _ => Err(SchemaError::UnknownKey { path: value_path }),
        }
    }
}

/// local override 경로(사용 프로젝트 root 기준 상대 경로 또는 절대 경로, 구분자 `/`).
///
/// telemetry·진단 민감값입니다(XV.1.4): `Debug`는 값을 가리고 `Display`는 구현하지 않으며, 원문은
/// [`LocalOverridePath::reveal`]로만 얻습니다.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalOverridePath(String);

impl LocalOverridePath {
    /// 규칙을 검사해 만듭니다.
    ///
    /// 빈 값·길이·제어 문자 외에 경로 형식(XV.6.1)을 검사합니다: 내부 구분자는 `/`만(아키텍처
    /// 19.2, `\` 금지), 빈 component(`a//b`)·끝 `/`·`.` component 금지. `..`은 사용 프로젝트 밖을
    /// 가리키기 위해 허용합니다. 프로젝트 root 기준 위치 검사는 [`crate::local_override`]가 합니다.
    pub fn new(text: &str) -> Result<LocalOverridePath, ValueError> {
        if text.is_empty() {
            return Err(ValueError::Empty);
        }
        if text.len() > MAX_LOCAL_OVERRIDE_LEN {
            return Err(ValueError::TooLong {
                max: MAX_LOCAL_OVERRIDE_LEN,
            });
        }
        if text.chars().any(|c| c.is_control() || c == '\\') {
            return Err(ValueError::InvalidCharacter);
        }
        let body = text.strip_prefix('/').unwrap_or(text);
        if body.is_empty() || body.split('/').any(|part| part.is_empty() || part == ".") {
            return Err(ValueError::InvalidOverridePath);
        }
        Ok(LocalOverridePath(text.to_string()))
    }

    /// 절대 경로(`/` 시작)인지.
    pub fn is_absolute(&self) -> bool {
        self.0.starts_with('/')
    }

    /// `/`로 나눈 component(절대 경로의 첫 `/` 제외).
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.strip_prefix('/').unwrap_or(&self.0).split('/')
    }

    /// 원문 문자열. 로컬 파일 시스템 접근·로컬 UI 표시에서만 사용합니다(XV.1.4).
    pub fn reveal(&self) -> &str {
        &self.0
    }
}

/// `stellar.yaml` 의존성 entry 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyDeclaration {
    /// logical name(매핑 키).
    pub name: LogicalName,
    /// 라이브러리 식별자.
    pub library_id: LibraryId,
    /// source 종류.
    pub source: SourceKind,
    /// repository URL.
    pub repository: RepositoryUrl,
    /// version 선택자.
    pub version: VersionSelector,
    /// 선택적 local override.
    pub local_override: Option<LocalOverridePath>,
    /// 사용 프로젝트가 이 의존성에서 쓰는 외부 component(저장된 외부 참조, ADR-0137 결정 1).
    pub components: BTreeSet<ComponentRef>,
}

/// entry 매핑의 허용 키(canonical 기록 순서).
const ENTRY_KEYS: &[&str] = &[
    "source",
    "library_id",
    "repository",
    "version",
    "local_override",
    "components",
];

/// `components` 항목 하나(canonical 소문자 UUID).
fn parse_component_id(text: &str, path: &str) -> Result<ComponentRef, SchemaError> {
    let id = text
        .parse::<StableId>()
        .map_err(|_| invalid(path, ValueError::InvalidComponentId))?;
    if id.to_string() != text {
        return Err(invalid(path, ValueError::InvalidComponentId));
    }
    Ok(ComponentRef::new(id))
}

/// `components` 시퀀스를 읽습니다. 중복·상한 초과·잘못된 UUID를 거부합니다.
fn parse_components(node: &DeclNode, path: &str) -> Result<BTreeSet<ComponentRef>, SchemaError> {
    let items = node.as_sequence().ok_or_else(|| SchemaError::WrongType {
        path: path.to_string(),
        expected: "sequence",
    })?;
    if items.len() > MAX_COMPONENT_IMPORTS {
        return Err(invalid(
            path,
            ValueError::TooManyEntries {
                max: MAX_COMPONENT_IMPORTS,
            },
        ));
    }
    let mut out = BTreeSet::new();
    for (index, item) in items.iter().enumerate() {
        let item_path = format!("{path}[{index}]");
        let text = expect_scalar(item, &item_path)?;
        if !out.insert(parse_component_id(text, &item_path)?) {
            return Err(invalid(&item_path, ValueError::DuplicateEntry));
        }
    }
    Ok(out)
}

impl DependencyDeclaration {
    /// entry를 `(logical name, 매핑)` 쌍으로 변환합니다(키 순서 고정).
    pub fn to_node_entry(&self) -> (String, DeclNode) {
        let mut pairs = vec![
            ("source".to_string(), DeclNode::scalar(self.source.as_str())),
            (
                "library_id".to_string(),
                DeclNode::scalar(self.library_id.to_string()),
            ),
            (
                "repository".to_string(),
                DeclNode::scalar(self.repository.reveal()),
            ),
            ("version".to_string(), self.version.to_node()),
        ];
        if let Some(path) = &self.local_override {
            pairs.push((
                "local_override".to_string(),
                DeclNode::scalar(path.reveal()),
            ));
        }
        if !self.components.is_empty() {
            pairs.push((
                "components".to_string(),
                DeclNode::Sequence(
                    self.components
                        .iter()
                        .map(|c| DeclNode::scalar(c.id().to_string()))
                        .collect(),
                ),
            ));
        }
        (self.name.as_str().to_string(), DeclNode::Mapping(pairs))
    }

    /// `(logical name, 매핑)` 쌍에서 entry를 읽습니다. `parent`는 오류 경로 접두사입니다.
    pub fn from_node_entry(
        name: &str,
        node: &DeclNode,
        parent: &str,
    ) -> Result<DependencyDeclaration, SchemaError> {
        let path = join(parent, name);
        let logical = LogicalName::new(name).map_err(|e| invalid(&path, e))?;
        let pairs = expect_mapping(node, &path)?;
        reject_unknown(pairs, &path, ENTRY_KEYS)?;

        let source_path = join(&path, "source");
        let source = expect_scalar(require(pairs, "source", &path)?, &source_path)?
            .parse::<SourceKind>()
            .map_err(|e| invalid(&source_path, e))?;

        let id_path = join(&path, "library_id");
        let library_id = expect_scalar(require(pairs, "library_id", &path)?, &id_path)?
            .parse::<LibraryId>()
            .map_err(|e| invalid(&id_path, e))?;

        let repo_path = join(&path, "repository");
        let repository = RepositoryUrl::new(expect_scalar(
            require(pairs, "repository", &path)?,
            &repo_path,
        )?)
        .map_err(|e| invalid(&repo_path, e))?;

        let version =
            VersionSelector::from_node(require(pairs, "version", &path)?, &join(&path, "version"))?;

        let local_override = match optional(pairs, "local_override") {
            None => None,
            Some(value) => {
                let override_path = join(&path, "local_override");
                let text = expect_scalar(value, &override_path)?;
                Some(LocalOverridePath::new(text).map_err(|e| invalid(&override_path, e))?)
            }
        };

        let components = match optional(pairs, "components") {
            None => BTreeSet::new(),
            Some(value) => parse_components(value, &join(&path, "components"))?,
        };

        Ok(DependencyDeclaration {
            name: logical,
            library_id,
            source,
            repository,
            version,
            local_override,
            components,
        })
    }
}

/// 선언 목록을 `dependencies` 매핑으로 변환합니다(선언 순서 보존).
pub fn dependencies_to_node(declarations: &[DependencyDeclaration]) -> DeclNode {
    DeclNode::Mapping(
        declarations
            .iter()
            .map(DependencyDeclaration::to_node_entry)
            .collect(),
    )
}

/// `dependencies` 매핑에서 선언 목록을 읽습니다(순서 보존).
///
/// 같은 logical name의 중복은 여기서 거부하지 않고 그대로 보존합니다. 중복·충돌 판정은
/// 선언 validation(XV.1.3)이 담당합니다.
pub fn dependencies_from_node(node: &DeclNode) -> Result<Vec<DependencyDeclaration>, SchemaError> {
    let pairs = expect_mapping(node, "dependencies")?;
    pairs
        .iter()
        .map(|(name, entry)| DependencyDeclaration::from_node_entry(name, entry, "dependencies"))
        .collect()
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! 테스트 공용 선언 fixture.

    use super::*;

    /// 번호로 결정적 LibraryId를 만듭니다.
    pub(crate) fn library(n: u8) -> LibraryId {
        LibraryId::from_stable_id(StableId::new_v7(0x0195_c530_3acd, 0x0bc1, n as u64))
    }

    /// 번호로 결정적 40자리 commit을 만듭니다.
    pub(crate) fn commit(n: u8) -> CommitId {
        format!("{n:02x}")
            .repeat(20)
            .parse()
            .expect("commit fixture")
    }

    /// tag 선택자 선언.
    pub(crate) fn decl(name: &str, lib: u8, url: &str, tag: &str) -> DependencyDeclaration {
        DependencyDeclaration {
            name: LogicalName::new(name).expect("name"),
            library_id: library(lib),
            source: SourceKind::Git,
            repository: RepositoryUrl::new(url).expect("url"),
            version: VersionSelector::Tag(RefName::new(tag).expect("tag")),
            local_override: None,
            components: BTreeSet::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{commit, decl, library};
    use super::*;

    fn sample() -> Vec<DependencyDeclaration> {
        let mut icons = decl("icons", 2, "https://example.com/icons.git", "unused");
        icons.version = VersionSelector::Branch(RefName::new("feature/next").unwrap());
        icons.local_override = Some(LocalOverridePath::new("../icons").unwrap());
        let mut pinned = decl("tokens", 3, "file:///srv/tokens", "unused");
        pinned.version = VersionSelector::Commit(commit(0xab));
        vec![
            decl(
                "design-system",
                1,
                "git@github.com:example/design-system.git",
                "v2.4.0",
            ),
            icons,
            pinned,
        ]
    }

    fn entry(node: &DeclNode, name: &str) -> DeclNode {
        node.get(name).cloned().expect("entry")
    }

    #[test]
    fn schema_round_trip_preserves_every_field_and_order() {
        let declarations = sample();
        let node = dependencies_to_node(&declarations);
        let parsed = dependencies_from_node(&node).expect("parse");
        assert_eq!(parsed, declarations);
        // 노드 → 모델 → 노드도 동일(canonical 형식 고정).
        assert_eq!(dependencies_to_node(&parsed), node);

        // canonical 형식: tag는 스칼라, branch·commit은 한 키 매핑, 키 순서 고정.
        let ds = entry(&node, "design-system");
        assert_eq!(ds.get("version"), Some(&DeclNode::scalar("v2.4.0")));
        let keys: Vec<&str> = ds
            .as_mapping()
            .unwrap()
            .iter()
            .map(|(k, _)| k.as_str())
            .collect();
        assert_eq!(keys, ["source", "library_id", "repository", "version"]);
        assert_eq!(
            entry(&node, "icons").get("version"),
            Some(&DeclNode::Mapping(vec![(
                "branch".to_string(),
                DeclNode::scalar("feature/next")
            )]))
        );
        assert_eq!(
            entry(&node, "icons")
                .get("local_override")
                .and_then(DeclNode::as_scalar),
            Some("../icons")
        );
        assert_eq!(
            entry(&node, "tokens")
                .get("version")
                .and_then(|v| v.get("commit"))
                .and_then(DeclNode::as_scalar),
            Some("abababababababababababababababababababab")
        );
        assert_eq!(
            ds.get("library_id").and_then(DeclNode::as_scalar),
            Some(library(1).to_string().as_str())
        );
    }

    #[test]
    fn tag_mapping_form_and_uppercase_commit_normalize() {
        let node = DeclNode::Mapping(vec![(
            "lib".to_string(),
            DeclNode::Mapping(vec![
                ("source".to_string(), DeclNode::scalar("git")),
                (
                    "library_id".to_string(),
                    DeclNode::scalar(library(9).to_string()),
                ),
                ("repository".to_string(), DeclNode::scalar("https://e/x")),
                (
                    "version".to_string(),
                    DeclNode::Mapping(vec![("tag".to_string(), DeclNode::scalar("v1"))]),
                ),
            ]),
        )]);
        let parsed = dependencies_from_node(&node).unwrap();
        assert_eq!(
            parsed[0].version,
            VersionSelector::Tag(RefName::new("v1").unwrap())
        );
        assert_eq!(
            entry(&dependencies_to_node(&parsed), "lib").get("version"),
            Some(&DeclNode::scalar("v1"))
        );

        let upper: CommitId = "ABABABABABABABABABABABABABABABABABABABAB".parse().unwrap();
        assert_eq!(upper, commit(0xab));
        let sha256: CommitId = "a".repeat(64).parse().unwrap();
        assert_eq!(sha256.as_str().len(), 64);
    }

    fn with_field(key: &str, value: Option<DeclNode>) -> DeclNode {
        let (name, DeclNode::Mapping(mut pairs)) =
            decl("lib", 1, "https://e/x", "v1").to_node_entry()
        else {
            unreachable!("entry는 항상 매핑");
        };
        pairs.retain(|(k, _)| k != key);
        if let Some(value) = value {
            pairs.push((key.to_string(), value));
        }
        DeclNode::Mapping(vec![(name, DeclNode::Mapping(pairs))])
    }

    fn invalid_value(node: &DeclNode) -> (String, ValueError) {
        match dependencies_from_node(node) {
            Err(SchemaError::InvalidValue { path, error }) => (path, error),
            other => panic!("InvalidValue 기대, 실제 {other:?}"),
        }
    }

    #[test]
    fn schema_violations_are_rejected_with_paths() {
        assert_eq!(
            dependencies_from_node(&with_field("library_id", None)),
            Err(SchemaError::MissingField {
                path: "dependencies.lib.library_id".to_string()
            })
        );
        assert_eq!(
            dependencies_from_node(&with_field("mirror", Some(DeclNode::scalar("x")))),
            Err(SchemaError::UnknownKey {
                path: "dependencies.lib.mirror".to_string()
            })
        );
        assert_eq!(
            dependencies_from_node(&with_field(
                "repository",
                Some(DeclNode::Sequence(Vec::new()))
            )),
            Err(SchemaError::WrongType {
                path: "dependencies.lib.repository".to_string(),
                expected: "scalar"
            })
        );
        assert_eq!(
            invalid_value(&with_field("source", Some(DeclNode::scalar("svn")))),
            (
                "dependencies.lib.source".to_string(),
                ValueError::UnknownSource
            )
        );
        assert_eq!(
            invalid_value(&with_field("library_id", Some(DeclNode::scalar("x")))).1,
            ValueError::InvalidLibraryId
        );
        assert_eq!(
            invalid_value(&with_field(
                "repository",
                Some(DeclNode::scalar("--upload-pack=touch"))
            ))
            .1,
            ValueError::LeadingDash
        );
        assert_eq!(
            invalid_value(&with_field(
                "repository",
                Some(DeclNode::scalar("ext::sh -c id"))
            ))
            .1,
            ValueError::InvalidCharacter
        );
        assert_eq!(
            invalid_value(&with_field(
                "repository",
                Some(DeclNode::scalar("EXT::sh%20-c%20id"))
            ))
            .1,
            ValueError::ForbiddenTransport
        );
        // 축약 commit, 두 키 version, 빈 version 매핑, 잘못된 ref 이름.
        let short = DeclNode::Mapping(vec![("commit".to_string(), DeclNode::scalar("abc1234"))]);
        assert_eq!(
            invalid_value(&with_field("version", Some(short))),
            (
                "dependencies.lib.version.commit".to_string(),
                ValueError::InvalidCommitLength
            )
        );
        let two = DeclNode::Mapping(vec![
            ("tag".to_string(), DeclNode::scalar("v1")),
            ("branch".to_string(), DeclNode::scalar("main")),
        ]);
        assert_eq!(
            invalid_value(&with_field("version", Some(two))).1,
            ValueError::InvalidSelector
        );
        assert_eq!(
            invalid_value(&with_field("version", Some(DeclNode::Mapping(Vec::new())))).1,
            ValueError::InvalidSelector
        );
        let unknown = DeclNode::Mapping(vec![("semver".to_string(), DeclNode::scalar("^1"))]);
        assert_eq!(
            dependencies_from_node(&with_field("version", Some(unknown))),
            Err(SchemaError::UnknownKey {
                path: "dependencies.lib.version.semver".to_string()
            })
        );
        for bad in [
            "-v1", "v1..2", "a b", "v1.lock", ".hidden", "x/", "@", "a@{1}",
        ] {
            assert!(
                RefName::new(bad).is_err(),
                "ref 이름 {bad:?}는 거부되어야 합니다"
            );
        }
        assert!(RefName::new("release/2026.11").is_ok());
        // logical name 규칙.
        for bad in ["", "-x", "a@b", "a/b", "a b"] {
            assert!(LogicalName::new(bad).is_err(), "name {bad:?}");
        }
        let bad_name = DeclNode::Mapping(vec![(
            "a@b".to_string(),
            with_field("version", Some(DeclNode::scalar("v1")))
                .get("lib")
                .cloned()
                .unwrap(),
        )]);
        assert_eq!(
            invalid_value(&bad_name),
            ("dependencies.a@b".to_string(), ValueError::InvalidCharacter)
        );
        assert_eq!(
            dependencies_from_node(&DeclNode::scalar("x")),
            Err(SchemaError::WrongType {
                path: "dependencies".to_string(),
                expected: "mapping"
            })
        );
    }

    #[test]
    fn duplicate_names_are_preserved_for_validation() {
        let (name, body) = decl("lib", 1, "https://e/x", "v1").to_node_entry();
        let node = DeclNode::Mapping(vec![(name.clone(), body.clone()), (name, body)]);
        assert_eq!(dependencies_from_node(&node).unwrap().len(), 2);
    }

    #[test]
    fn component_imports_round_trip_in_canonical_order_and_reject_bad_items() {
        let a = ComponentRef::new(StableId::new_v7(0x0195_c530_3acd, 0x0bc1, 2));
        let b = ComponentRef::new(StableId::new_v7(0x0195_c530_3acd, 0x0bc1, 1));
        let mut d = decl("ds", 1, "https://e.com/ds.git", "v1");
        d.components = [a, b].into_iter().collect();
        let (name, body) = d.to_node_entry();
        let items = body
            .get("components")
            .and_then(DeclNode::as_sequence)
            .unwrap();
        // StableId 오름차순(b < a)으로 씁니다.
        assert_eq!(
            items,
            [
                DeclNode::scalar(b.id().to_string()),
                DeclNode::scalar(a.id().to_string())
            ]
        );
        let node = DeclNode::Mapping(vec![(name.clone(), body.clone())]);
        assert_eq!(dependencies_from_node(&node).unwrap(), vec![d.clone()]);

        // 입력 순서는 정규화합니다.
        let mut reversed = body.clone();
        if let DeclNode::Mapping(pairs) = &mut reversed {
            let last = pairs.last_mut().unwrap();
            last.1 = DeclNode::Sequence(vec![
                DeclNode::scalar(a.id().to_string()),
                DeclNode::scalar(b.id().to_string()),
            ]);
        }
        let parsed =
            dependencies_from_node(&DeclNode::Mapping(vec![(name.clone(), reversed)])).unwrap();
        assert_eq!(dependencies_to_node(&parsed), node);

        // 비어 있으면 키를 쓰지 않습니다.
        let (_, plain) = decl("ds", 1, "https://e.com/ds.git", "v1").to_node_entry();
        assert!(plain.get("components").is_none());

        let with = |items: DeclNode| {
            let mut body = plain.clone();
            if let DeclNode::Mapping(pairs) = &mut body {
                pairs.push(("components".to_string(), items));
            }
            dependencies_from_node(&DeclNode::Mapping(vec![("ds".to_string(), body)]))
        };
        let upper = a.id().to_string().to_ascii_uppercase();
        assert_eq!(
            with(DeclNode::Sequence(vec![DeclNode::scalar(upper)])),
            Err(SchemaError::InvalidValue {
                path: "dependencies.ds.components[0]".to_string(),
                error: ValueError::InvalidComponentId
            })
        );
        assert_eq!(
            with(DeclNode::Sequence(vec![
                DeclNode::scalar(a.id().to_string()),
                DeclNode::scalar(a.id().to_string())
            ])),
            Err(SchemaError::InvalidValue {
                path: "dependencies.ds.components[1]".to_string(),
                error: ValueError::DuplicateEntry
            })
        );
        assert_eq!(
            with(DeclNode::scalar("x")),
            Err(SchemaError::WrongType {
                path: "dependencies.ds.components".to_string(),
                expected: "sequence"
            })
        );
        let many = DeclNode::Sequence(
            (0..=MAX_COMPONENT_IMPORTS as u64)
                .map(|n| DeclNode::scalar(StableId::new_v7(1, 1, n).to_string()))
                .collect(),
        );
        assert_eq!(
            with(many),
            Err(SchemaError::InvalidValue {
                path: "dependencies.ds.components".to_string(),
                error: ValueError::TooManyEntries {
                    max: MAX_COMPONENT_IMPORTS
                }
            })
        );
    }
}
