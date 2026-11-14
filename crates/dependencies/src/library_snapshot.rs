//! 읽기 전용 library snapshot (XV.5.1).
//!
//! library 프로젝트가 export한 ComponentDefinition·TokenSet index를 한 resolved node(LibraryId +
//! exact commit) 단위로 메모리에 고정합니다(아키텍처 30.1 "읽기 전용으로 로드", 30.3 "여러 버전
//! 동시 로드").
//!
//! - **resource key**: 모든 resource는 [`LibraryResourceKey`] = LibraryId + resolved commit +
//!   resource StableId로 식별합니다. 같은 library의 v1·v2가 같은 StableId를 export해도 commit이
//!   달라 key가 충돌하지 않습니다(버전별 격리, 자동 단일 버전 통합 금지).
//! - **형식 중립 export record**: ComponentDefinition(document crate)·TokenSet(bindings crate)은 이
//!   crate가 의존할 수 없는 crate에 있습니다(crate 의존 방향: dependencies → core-types만). library
//!   저장 데이터를 읽어 [`LibraryExports`]로 바꾸는 일은 [`LibraryExportSource`] 구현이 담당하고,
//!   이 crate는 그 결과를 검증해 snapshot으로 만듭니다(ADR-0023).
//! - **읽기 전용**: snapshot 필드는 비공개이고 변경 API가 없습니다. 검증을 통과한 결과만
//!   [`LibrarySnapshot::new`]로 만들 수 있습니다.
//! - **비신뢰 입력 상한**: export는 외부 저장소에서 옵니다. resource 수·이름 길이·fingerprint
//!   크기를 [`SnapshotLimits`]로 제한하고, 같은 library 안 StableId 중복을 거부합니다.
//! - **cache 읽기**: [`load_cached_snapshot`]은 cache entry를 공유 read lock(XV.3.6)을 잡은 채
//!   읽고, snapshot을 만든 뒤 lock을 풉니다. snapshot은 cache 파일을 참조하지 않으므로 이후 entry
//!   정리(XV.8)와 무관합니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::{ComponentRef, TokenRef};

use crate::cache_entry::{LibraryEntryStore, MaterializeError, Sha256Digest};
use crate::content_hash::ContentHashText;
use crate::declaration::{CommitId, LibraryId, RepositoryUrl};
use crate::graph::DependencyNodeKey;
use crate::mirror::RepositoryKeyer;

/// library resource 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LibraryResourceKind {
    /// ComponentDefinition.
    Component,
    /// TokenSet.
    TokenSet,
    /// TokenDefinition.
    Token,
}

impl LibraryResourceKind {
    /// 종류 이름.
    pub fn as_str(self) -> &'static str {
        match self {
            LibraryResourceKind::Component => "component",
            LibraryResourceKind::TokenSet => "token-set",
            LibraryResourceKind::Token => "token",
        }
    }
}

/// library resource 전역 key: LibraryId + resolved commit + resource StableId.
///
/// 정렬은 LibraryId → commit → StableId 순입니다.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LibraryResourceKey {
    /// resolved library node(LibraryId + exact commit).
    pub node: DependencyNodeKey,
    /// library 안 resource StableId.
    pub resource: StableId,
}

impl LibraryResourceKey {
    /// key를 만듭니다.
    pub fn new(node: DependencyNodeKey, resource: StableId) -> LibraryResourceKey {
        LibraryResourceKey { node, resource }
    }

    /// library 식별자.
    pub fn library_id(&self) -> LibraryId {
        self.node.library_id
    }

    /// resolved commit.
    pub fn commit(&self) -> &CommitId {
        &self.node.commit
    }
}

impl fmt::Display for LibraryResourceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.node, self.resource)
    }
}

/// producer가 정한 canonical encoding bytes. 이 crate는 같음 비교(diff, XV.7.2)에만 씁니다.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExportFingerprint(Vec<u8>);

impl ExportFingerprint {
    /// bytes로 만듭니다(상한은 snapshot 검증에서 확인).
    pub fn new(bytes: Vec<u8>) -> ExportFingerprint {
        ExportFingerprint(bytes)
    }

    /// bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// component property 공개 여부.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExportVisibility {
    /// 사용 프로젝트 instance가 override할 수 있음.
    Public,
    /// library 내부 전용(override 불가).
    Private,
}

/// export된 component property 하나(ComponentProperty 요약).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedProperty {
    /// ComponentPropertyId의 StableId.
    pub id: StableId,
    /// 표시 이름(식별에 사용 금지).
    pub name: String,
    /// 선언 타입의 canonical 표기(같은 표기 = 같은 타입).
    pub type_signature: String,
    /// 공개 여부.
    pub visibility: ExportVisibility,
    /// default expression의 canonical encoding.
    pub default: ExportFingerprint,
}

/// export된 ComponentDefinition 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedComponent {
    /// component StableId.
    pub id: ComponentRef,
    /// 표시 이름(식별에 사용 금지).
    pub name: String,
    /// property(선언 순서).
    pub properties: Vec<ExportedProperty>,
    /// slot StableId(선언 순서).
    pub slots: Vec<StableId>,
}

impl ExportedComponent {
    /// property를 StableId로 찾습니다.
    pub fn property(&self, id: StableId) -> Option<&ExportedProperty> {
        self.properties.iter().find(|p| p.id == id)
    }
}

/// export된 TokenDefinition 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedToken {
    /// token StableId.
    pub id: TokenRef,
    /// 표시 이름(한 set 안에서 유일).
    pub name: String,
    /// token 타입의 canonical 표기.
    pub type_signature: String,
    /// default expression의 canonical encoding.
    pub value: ExportFingerprint,
}

/// export된 TokenSet 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedTokenSet {
    /// TokenSet StableId.
    pub id: StableId,
    /// token(검증 후 StableId 순).
    pub tokens: Vec<ExportedToken>,
}

/// library 프로젝트 export index(검증 전 원본).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryExports {
    /// export된 ComponentDefinition.
    pub components: Vec<ExportedComponent>,
    /// export된 TokenSet.
    pub token_sets: Vec<ExportedTokenSet>,
}

/// snapshot 입력 상한.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotLimits {
    /// 최대 component 수.
    pub max_components: usize,
    /// component당 최대 property 수.
    pub max_properties_per_component: usize,
    /// component당 최대 slot 수.
    pub max_slots_per_component: usize,
    /// 최대 TokenSet 수.
    pub max_token_sets: usize,
    /// 모든 set을 합한 최대 token 수.
    pub max_tokens: usize,
    /// 이름 최대 byte 수.
    pub max_name_bytes: usize,
    /// 타입 표기 최대 byte 수.
    pub max_type_signature_bytes: usize,
    /// fingerprint 하나의 최대 byte 수.
    pub max_fingerprint_bytes: usize,
}

impl Default for SnapshotLimits {
    fn default() -> SnapshotLimits {
        SnapshotLimits {
            max_components: 100_000,
            max_properties_per_component: 4_096,
            max_slots_per_component: 4_096,
            max_token_sets: 10_000,
            max_tokens: 1_000_000,
            max_name_bytes: 1_024,
            max_type_signature_bytes: 256,
            max_fingerprint_bytes: 1024 * 1024,
        }
    }
}

/// export 검증 실패. 이름·값 원문은 담지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    /// component 수 상한 초과.
    TooManyComponents {
        /// 상한.
        max: usize,
    },
    /// component의 property 수 상한 초과.
    TooManyProperties {
        /// component.
        component: StableId,
        /// 상한.
        max: usize,
    },
    /// component의 slot 수 상한 초과.
    TooManySlots {
        /// component.
        component: StableId,
        /// 상한.
        max: usize,
    },
    /// TokenSet 수 상한 초과.
    TooManyTokenSets {
        /// 상한.
        max: usize,
    },
    /// token 수 상한 초과.
    TooManyTokens {
        /// 상한.
        max: usize,
    },
    /// 같은 library 안에서 두 resource가 같은 StableId를 씀.
    DuplicateResource {
        /// 중복 StableId.
        id: StableId,
    },
    /// 한 component 안 property StableId 중복.
    DuplicateProperty {
        /// component.
        component: StableId,
        /// 중복 property.
        property: StableId,
    },
    /// 한 component 안 slot StableId 중복.
    DuplicateSlot {
        /// component.
        component: StableId,
        /// 중복 slot.
        slot: StableId,
    },
    /// 한 TokenSet 안 token 이름 중복.
    DuplicateTokenName {
        /// TokenSet.
        set: StableId,
        /// 뒤에 나온 token.
        token: StableId,
    },
    /// 이름이 상한을 넘음.
    NameTooLong {
        /// 대상 resource·property.
        id: StableId,
    },
    /// 타입 표기가 비었거나 상한을 넘거나 제어 문자를 포함함.
    InvalidTypeSignature {
        /// 대상 property·token.
        id: StableId,
    },
    /// fingerprint가 상한을 넘음.
    FingerprintTooLarge {
        /// 대상 property·token.
        id: StableId,
    },
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SnapshotError::TooManyComponents { max } => {
                write!(f, "component 수가 {max}개를 넘습니다")
            }
            SnapshotError::TooManyProperties { component, max } => {
                write!(
                    f,
                    "component {component}의 property 수가 {max}개를 넘습니다"
                )
            }
            SnapshotError::TooManySlots { component, max } => {
                write!(f, "component {component}의 slot 수가 {max}개를 넘습니다")
            }
            SnapshotError::TooManyTokenSets { max } => {
                write!(f, "TokenSet 수가 {max}개를 넘습니다")
            }
            SnapshotError::TooManyTokens { max } => write!(f, "token 수가 {max}개를 넘습니다"),
            SnapshotError::DuplicateResource { id } => {
                write!(f, "library 안에서 resource StableId {id}가 중복됩니다")
            }
            SnapshotError::DuplicateProperty {
                component,
                property,
            } => write!(
                f,
                "component {component}에서 property {property}가 중복됩니다"
            ),
            SnapshotError::DuplicateSlot { component, slot } => {
                write!(f, "component {component}에서 slot {slot}이 중복됩니다")
            }
            SnapshotError::DuplicateTokenName { set, token } => {
                write!(f, "TokenSet {set}에서 token {token}의 이름이 중복됩니다")
            }
            SnapshotError::NameTooLong { id } => write!(f, "{id}의 이름이 너무 깁니다"),
            SnapshotError::InvalidTypeSignature { id } => {
                write!(f, "{id}의 타입 표기가 잘못되었습니다")
            }
            SnapshotError::FingerprintTooLarge { id } => {
                write!(f, "{id}의 값 encoding이 너무 큽니다")
            }
        }
    }
}

impl std::error::Error for SnapshotError {}

/// resolved node 하나의 읽기 전용 export index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibrarySnapshot {
    node: DependencyNodeKey,
    content_hash: ContentHashText,
    components: BTreeMap<StableId, ExportedComponent>,
    token_sets: BTreeMap<StableId, ExportedTokenSet>,
    /// token → 소속 TokenSet.
    token_owner: BTreeMap<StableId, StableId>,
}

/// snapshot 안 component 하나와 전역 key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LibraryComponent<'a> {
    snapshot: &'a LibrarySnapshot,
    export: &'a ExportedComponent,
}

impl<'a> LibraryComponent<'a> {
    /// 전역 key.
    pub fn key(&self) -> LibraryResourceKey {
        self.snapshot.key_of(self.export.id.id())
    }

    /// export record.
    pub fn export(&self) -> &'a ExportedComponent {
        self.export
    }
}

/// snapshot 안 token 하나와 전역 key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LibraryToken<'a> {
    snapshot: &'a LibrarySnapshot,
    set: StableId,
    export: &'a ExportedToken,
}

impl<'a> LibraryToken<'a> {
    /// 전역 key.
    pub fn key(&self) -> LibraryResourceKey {
        self.snapshot.key_of(self.export.id.id())
    }

    /// 소속 TokenSet의 전역 key.
    pub fn set_key(&self) -> LibraryResourceKey {
        self.snapshot.key_of(self.set)
    }

    /// export record.
    pub fn export(&self) -> &'a ExportedToken {
        self.export
    }
}

fn check_name(name: &str, id: StableId, limits: &SnapshotLimits) -> Result<(), SnapshotError> {
    if name.len() > limits.max_name_bytes {
        return Err(SnapshotError::NameTooLong { id });
    }
    Ok(())
}

fn check_type(signature: &str, id: StableId, limits: &SnapshotLimits) -> Result<(), SnapshotError> {
    if signature.is_empty()
        || signature.len() > limits.max_type_signature_bytes
        || signature.chars().any(char::is_control)
    {
        return Err(SnapshotError::InvalidTypeSignature { id });
    }
    Ok(())
}

fn check_fingerprint(
    value: &ExportFingerprint,
    id: StableId,
    limits: &SnapshotLimits,
) -> Result<(), SnapshotError> {
    if value.as_bytes().len() > limits.max_fingerprint_bytes {
        return Err(SnapshotError::FingerprintTooLarge { id });
    }
    Ok(())
}

impl LibrarySnapshot {
    /// export index를 검증해 snapshot을 만듭니다.
    ///
    /// 거부: 상한 초과, 같은 library 안 resource(component·TokenSet·token) StableId 중복, component 안
    /// property·slot 중복, TokenSet 안 token 이름 중복, 잘못된 타입 표기. token은 set 안에서
    /// StableId 순으로 정렬합니다(property·slot은 선언 순서 유지).
    pub fn new(
        node: DependencyNodeKey,
        content_hash: ContentHashText,
        exports: LibraryExports,
        limits: &SnapshotLimits,
    ) -> Result<LibrarySnapshot, SnapshotError> {
        if exports.components.len() > limits.max_components {
            return Err(SnapshotError::TooManyComponents {
                max: limits.max_components,
            });
        }
        if exports.token_sets.len() > limits.max_token_sets {
            return Err(SnapshotError::TooManyTokenSets {
                max: limits.max_token_sets,
            });
        }
        exports
            .token_sets
            .iter()
            .try_fold(0usize, |acc, set| acc.checked_add(set.tokens.len()))
            .filter(|n| *n <= limits.max_tokens)
            .ok_or(SnapshotError::TooManyTokens {
                max: limits.max_tokens,
            })?;

        let mut seen: BTreeSet<StableId> = BTreeSet::new();
        let mut claim = |id: StableId| {
            if seen.insert(id) {
                Ok(())
            } else {
                Err(SnapshotError::DuplicateResource { id })
            }
        };

        let mut components = BTreeMap::new();
        for component in exports.components {
            let id = component.id.id();
            claim(id)?;
            check_name(&component.name, id, limits)?;
            if component.properties.len() > limits.max_properties_per_component {
                return Err(SnapshotError::TooManyProperties {
                    component: id,
                    max: limits.max_properties_per_component,
                });
            }
            if component.slots.len() > limits.max_slots_per_component {
                return Err(SnapshotError::TooManySlots {
                    component: id,
                    max: limits.max_slots_per_component,
                });
            }
            let mut properties = BTreeSet::new();
            for property in &component.properties {
                if !properties.insert(property.id) {
                    return Err(SnapshotError::DuplicateProperty {
                        component: id,
                        property: property.id,
                    });
                }
                check_name(&property.name, property.id, limits)?;
                check_type(&property.type_signature, property.id, limits)?;
                check_fingerprint(&property.default, property.id, limits)?;
            }
            let mut slots = BTreeSet::new();
            for slot in &component.slots {
                if !slots.insert(*slot) {
                    return Err(SnapshotError::DuplicateSlot {
                        component: id,
                        slot: *slot,
                    });
                }
            }
            components.insert(id, component);
        }

        let mut token_sets = BTreeMap::new();
        let mut token_owner = BTreeMap::new();
        for mut set in exports.token_sets {
            claim(set.id)?;
            {
                let mut names: BTreeSet<&str> = BTreeSet::new();
                for token in &set.tokens {
                    let id = token.id.id();
                    claim(id)?;
                    check_name(&token.name, id, limits)?;
                    check_type(&token.type_signature, id, limits)?;
                    check_fingerprint(&token.value, id, limits)?;
                    if !names.insert(token.name.as_str()) {
                        return Err(SnapshotError::DuplicateTokenName {
                            set: set.id,
                            token: id,
                        });
                    }
                    token_owner.insert(id, set.id);
                }
            }
            set.tokens.sort_by_key(|t| t.id);
            token_sets.insert(set.id, set);
        }

        Ok(LibrarySnapshot {
            node,
            content_hash,
            components,
            token_sets,
            token_owner,
        })
    }

    /// resolved node(LibraryId + commit).
    pub fn node(&self) -> &DependencyNodeKey {
        &self.node
    }

    /// library 식별자.
    pub fn library_id(&self) -> LibraryId {
        self.node.library_id
    }

    /// resolved commit.
    pub fn commit(&self) -> &CommitId {
        &self.node.commit
    }

    /// 읽은 cache entry content hash.
    pub fn content_hash(&self) -> &ContentHashText {
        &self.content_hash
    }

    /// 이 snapshot 안 StableId의 전역 key.
    pub fn key_of(&self, resource: StableId) -> LibraryResourceKey {
        LibraryResourceKey::new(self.node.clone(), resource)
    }

    /// StableId의 resource 종류(없으면 `None`).
    pub fn kind_of(&self, resource: StableId) -> Option<LibraryResourceKind> {
        if self.components.contains_key(&resource) {
            Some(LibraryResourceKind::Component)
        } else if self.token_sets.contains_key(&resource) {
            Some(LibraryResourceKind::TokenSet)
        } else if self.token_owner.contains_key(&resource) {
            Some(LibraryResourceKind::Token)
        } else {
            None
        }
    }

    /// component를 찾습니다.
    pub fn component(&self, id: ComponentRef) -> Option<LibraryComponent<'_>> {
        self.components
            .get(&id.id())
            .map(|export| LibraryComponent {
                snapshot: self,
                export,
            })
    }

    /// 모든 component(StableId 순).
    pub fn components(&self) -> impl Iterator<Item = LibraryComponent<'_>> {
        self.components
            .values()
            .map(move |export| LibraryComponent {
                snapshot: self,
                export,
            })
    }

    /// TokenSet을 찾습니다.
    pub fn token_set(&self, id: StableId) -> Option<&ExportedTokenSet> {
        self.token_sets.get(&id)
    }

    /// 모든 TokenSet(StableId 순).
    pub fn token_sets(&self) -> impl Iterator<Item = &ExportedTokenSet> {
        self.token_sets.values()
    }

    /// token을 찾습니다.
    pub fn token(&self, id: TokenRef) -> Option<LibraryToken<'_>> {
        let set = *self.token_owner.get(&id.id())?;
        let tokens = &self.token_sets.get(&set)?.tokens;
        let index = tokens.binary_search_by_key(&id, |t| t.id).ok()?;
        Some(LibraryToken {
            snapshot: self,
            set,
            export: &tokens[index],
        })
    }

    /// 모든 token(StableId 순).
    pub fn tokens(&self) -> impl Iterator<Item = LibraryToken<'_>> {
        self.token_owner
            .keys()
            .filter_map(move |id| self.token(TokenRef::new(*id)))
    }

    /// 모든 resource key(component·TokenSet·token, key 순).
    pub fn resource_keys(&self) -> Vec<LibraryResourceKey> {
        let mut ids: Vec<StableId> = self
            .components
            .keys()
            .chain(self.token_sets.keys())
            .chain(self.token_owner.keys())
            .copied()
            .collect();
        ids.sort();
        ids.into_iter().map(|id| self.key_of(id)).collect()
    }
}

/// export index 읽기 실패. 경로·원문은 담지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportReadFailure {
    /// export 저장 데이터가 손상됨.
    Corrupt,
    /// 지원하지 않는 library 저장 schema.
    UnsupportedSchema,
    /// 읽기 실패.
    Io {
        /// io 오류 종류.
        kind: std::io::ErrorKind,
    },
}

impl fmt::Display for ExportReadFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExportReadFailure::Corrupt => f.write_str("library export 데이터가 손상되었습니다"),
            ExportReadFailure::UnsupportedSchema => {
                f.write_str("지원하지 않는 library 저장 schema입니다")
            }
            ExportReadFailure::Io { kind } => write!(f, "library export 읽기 실패: {kind:?}"),
        }
    }
}

impl std::error::Error for ExportReadFailure {}

/// 읽기 전용 library project root에서 export index를 읽는 공급자.
///
/// 운영 구현은 storage generation loader와 document·bindings codec을 쓰는 쪽(project 통합)이
/// 제공합니다. 구현은 `root` 아래를 쓰지 않아야 합니다(cache entry는 읽기 전용).
pub trait LibraryExportSource {
    /// `root`의 library 프로젝트(`library`)에서 export index를 읽습니다.
    fn read_exports(
        &mut self,
        root: &Path,
        library: LibraryId,
    ) -> Result<LibraryExports, ExportReadFailure>;
}

/// cache snapshot 읽기 요청.
#[derive(Debug, Clone, Copy)]
pub struct SnapshotRequest<'a> {
    /// node를 선언한 repository.
    pub repository: &'a RepositoryUrl,
    /// 읽을 node.
    pub node: &'a DependencyNodeKey,
    /// 기대 content hash(lock entry 값). 있으면 cache index 값과 같아야 합니다.
    pub expected_content_hash: Option<&'a ContentHashText>,
}

/// cache snapshot 읽기 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotLoadError {
    /// cache에 entry가 없음(미 materialize·offline·정리됨).
    MissingEntry,
    /// cache entry content hash가 기대 값과 다름(손상, 다른 commit 자동 선택 금지).
    ContentHashMismatch {
        /// 기대 값.
        expected: ContentHashText,
        /// cache index 값.
        found: ContentHashText,
    },
    /// cache index·lock 실패.
    Cache(MaterializeError),
    /// export 읽기 실패.
    Export(ExportReadFailure),
    /// export 검증 실패.
    Invalid(SnapshotError),
}

impl fmt::Display for SnapshotLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SnapshotLoadError::MissingEntry => f.write_str("cache에 library entry가 없습니다"),
            SnapshotLoadError::ContentHashMismatch { expected, found } => write!(
                f,
                "cache entry content hash {found}가 기대 값 {expected}와 다릅니다"
            ),
            SnapshotLoadError::Cache(e) => write!(f, "library cache 오류: {e}"),
            SnapshotLoadError::Export(e) => write!(f, "{e}"),
            SnapshotLoadError::Invalid(e) => write!(f, "library export 검증 실패: {e}"),
        }
    }
}

impl std::error::Error for SnapshotLoadError {}

/// materialize된 cache entry에서 snapshot을 읽습니다.
///
/// entry 공유 read lock을 잡은 동안만 export를 읽고, snapshot을 만든 뒤 lock을 풉니다. Git·network를
/// 쓰지 않습니다(offline에서도 cache만 있으면 동작). 기대 content hash와 다르면 export를 읽지 않고
/// 실패합니다.
pub fn load_cached_snapshot<H: Sha256Digest, K: RepositoryKeyer>(
    store: &LibraryEntryStore<H, K>,
    request: SnapshotRequest<'_>,
    source: &mut dyn LibraryExportSource,
    limits: &SnapshotLimits,
    now_unix_ms: u64,
) -> Result<LibrarySnapshot, SnapshotLoadError> {
    let opened = store
        .open_entry(request.repository, &request.node.commit, now_unix_ms)
        .map_err(SnapshotLoadError::Cache)?
        .ok_or(SnapshotLoadError::MissingEntry)?;
    if let Some(expected) = request.expected_content_hash {
        if expected != opened.content_hash() {
            return Err(SnapshotLoadError::ContentHashMismatch {
                expected: expected.clone(),
                found: opened.content_hash().clone(),
            });
        }
    }
    let exports = source
        .read_exports(opened.dir(), request.node.library_id)
        .map_err(SnapshotLoadError::Export)?;
    let snapshot = LibrarySnapshot::new(
        request.node.clone(),
        opened.content_hash().clone(),
        exports,
        limits,
    )
    .map_err(SnapshotLoadError::Invalid)?;
    drop(opened);
    Ok(snapshot)
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! 테스트 공용 export fixture.

    use super::*;

    /// 번호로 결정적 resource StableId를 만듭니다.
    pub(crate) fn rid(n: u16) -> StableId {
        StableId::new_v7(0x0196_0000_0000, n, 0x5eed)
    }

    /// 공개 property.
    pub(crate) fn property(n: u16, name: &str, ty: &str, default: &[u8]) -> ExportedProperty {
        ExportedProperty {
            id: rid(n),
            name: name.to_string(),
            type_signature: ty.to_string(),
            visibility: ExportVisibility::Public,
            default: ExportFingerprint::new(default.to_vec()),
        }
    }

    /// component.
    pub(crate) fn component(
        n: u16,
        name: &str,
        properties: Vec<ExportedProperty>,
    ) -> ExportedComponent {
        ExportedComponent {
            id: ComponentRef::new(rid(n)),
            name: name.to_string(),
            properties,
            slots: Vec::new(),
        }
    }

    /// token.
    pub(crate) fn token(n: u16, name: &str, ty: &str, value: &[u8]) -> ExportedToken {
        ExportedToken {
            id: TokenRef::new(rid(n)),
            name: name.to_string(),
            type_signature: ty.to_string(),
            value: ExportFingerprint::new(value.to_vec()),
        }
    }

    /// TokenSet.
    pub(crate) fn token_set(n: u16, tokens: Vec<ExportedToken>) -> ExportedTokenSet {
        ExportedTokenSet { id: rid(n), tokens }
    }

    /// 결정적 content hash.
    pub(crate) fn hash(n: u8) -> ContentHashText {
        ContentHashText::from_sha256_digest([n; 32])
    }

    /// 검증된 snapshot(실패 시 panic, 테스트 전용).
    pub(crate) fn snapshot(node: DependencyNodeKey, exports: LibraryExports) -> LibrarySnapshot {
        LibrarySnapshot::new(node, hash(1), exports, &SnapshotLimits::default())
            .expect("fixture snapshot")
    }

    /// root 경로별 export 표 공급자.
    #[derive(Default)]
    pub(crate) struct TableExportSource {
        pub(crate) by_root: BTreeMap<std::path::PathBuf, LibraryExports>,
        pub(crate) calls: Vec<(std::path::PathBuf, LibraryId)>,
    }

    impl LibraryExportSource for TableExportSource {
        fn read_exports(
            &mut self,
            root: &Path,
            library: LibraryId,
        ) -> Result<LibraryExports, ExportReadFailure> {
            self.calls.push((root.to_path_buf(), library));
            self.by_root
                .get(root)
                .cloned()
                .ok_or(ExportReadFailure::Corrupt)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::cache_lock::{CacheLockError, CacheLocks};
    use crate::cache_root::LibraryCacheRoot;
    use crate::declaration::fixtures::{commit, library};
    use crate::mirror::MirrorCache;
    use crate::test_support::{FakeSha256, FnvKeyer, SystemGit, TempDir};

    fn key(lib: u8, c: u8) -> DependencyNodeKey {
        DependencyNodeKey::new(library(lib), commit(c))
    }

    fn version_exports(button_name: &str, color: &[u8]) -> LibraryExports {
        LibraryExports {
            components: vec![component(
                1,
                button_name,
                vec![property(2, "label", "String", b"\"OK\"")],
            )],
            token_sets: vec![token_set(3, vec![token(4, "brand", "Color", color)])],
        }
    }

    #[test]
    fn same_stable_id_in_v1_and_v2_does_not_collide() {
        // 같은 library(c)의 v1·v2가 같은 StableId(component 1, token 4)를 export.
        let v1 = snapshot(key(3, 0xc1), version_exports("Button", b"#f00"));
        let v2 = snapshot(key(3, 0xc2), version_exports("Button v2", b"#0f0"));

        let mut index: BTreeMap<LibraryResourceKey, String> = BTreeMap::new();
        for snap in [&v1, &v2] {
            for c in snap.components() {
                assert!(index.insert(c.key(), c.export().name.clone()).is_none());
            }
            for t in snap.tokens() {
                assert!(index.insert(t.key(), t.export().name.clone()).is_none());
            }
        }
        // component 2개 + token 2개, 충돌 없음.
        assert_eq!(index.len(), 4);
        let c1 = v1.component(ComponentRef::new(rid(1))).unwrap().key();
        let c2 = v2.component(ComponentRef::new(rid(1))).unwrap().key();
        assert_ne!(c1, c2);
        assert_eq!(c1.resource, c2.resource);
        assert_eq!(c1.library_id(), c2.library_id());
        assert_eq!(c1.commit(), &commit(0xc1));
        assert_eq!(c2.commit(), &commit(0xc2));
        assert_eq!(index[&c1], "Button");
        assert_eq!(index[&c2], "Button v2");
        let t2 = v2.token(TokenRef::new(rid(4))).unwrap();
        assert_eq!(t2.export().value.as_bytes(), b"#0f0");
        assert_eq!(t2.set_key(), v2.key_of(rid(3)));
        assert_eq!(
            c1.to_string(),
            format!("{}@{}#{}", library(3), commit(0xc1), rid(1))
        );
        // key 정렬은 LibraryId → commit → StableId.
        let keys: Vec<_> = index.keys().cloned().collect();
        assert_eq!(keys[0].commit(), &commit(0xc1));
        assert_eq!(keys[3].commit(), &commit(0xc2));
    }

    #[test]
    fn snapshot_indexes_kinds_and_sorts_tokens() {
        let exports = LibraryExports {
            components: vec![component(10, "Card", vec![])],
            token_sets: vec![token_set(
                20,
                vec![
                    token(22, "b", "Number", b"2"),
                    token(21, "a", "Number", b"1"),
                ],
            )],
        };
        let snap = snapshot(key(1, 1), exports);
        assert_eq!(snap.kind_of(rid(10)), Some(LibraryResourceKind::Component));
        assert_eq!(snap.kind_of(rid(20)), Some(LibraryResourceKind::TokenSet));
        assert_eq!(snap.kind_of(rid(21)), Some(LibraryResourceKind::Token));
        assert_eq!(snap.kind_of(rid(99)), None);
        let ids: Vec<_> = snap.tokens().map(|t| t.export().id.id()).collect();
        assert_eq!(ids, [rid(21), rid(22)]);
        let set_ids: Vec<_> = snap
            .token_set(rid(20))
            .unwrap()
            .tokens
            .iter()
            .map(|t| t.id.id())
            .collect();
        assert_eq!(set_ids, [rid(21), rid(22)]);
        assert_eq!(
            snap.resource_keys()
                .into_iter()
                .map(|k| k.resource)
                .collect::<Vec<_>>(),
            [rid(10), rid(20), rid(21), rid(22)]
        );
        assert!(snap.component(ComponentRef::new(rid(20))).is_none());
        assert!(snap.token(TokenRef::new(rid(10))).is_none());
    }

    fn build(
        exports: LibraryExports,
        limits: &SnapshotLimits,
    ) -> Result<LibrarySnapshot, SnapshotError> {
        LibrarySnapshot::new(key(1, 1), hash(1), exports, limits)
    }

    #[test]
    fn invalid_exports_are_rejected() {
        let limits = SnapshotLimits::default();
        let one = |c: ExportedComponent| LibraryExports {
            components: vec![c],
            token_sets: vec![],
        };
        // component와 token이 같은 StableId.
        let clash = LibraryExports {
            components: vec![component(1, "A", vec![])],
            token_sets: vec![token_set(2, vec![token(1, "t", "Number", b"1")])],
        };
        assert_eq!(
            build(clash, &limits),
            Err(SnapshotError::DuplicateResource { id: rid(1) })
        );
        // property 중복.
        let dup_prop = component(
            1,
            "A",
            vec![
                property(2, "x", "Number", b"1"),
                property(2, "y", "Number", b"1"),
            ],
        );
        assert_eq!(
            build(one(dup_prop), &limits),
            Err(SnapshotError::DuplicateProperty {
                component: rid(1),
                property: rid(2)
            })
        );
        // slot 중복.
        let mut dup_slot = component(1, "A", vec![]);
        dup_slot.slots = vec![rid(5), rid(5)];
        assert_eq!(
            build(one(dup_slot), &limits),
            Err(SnapshotError::DuplicateSlot {
                component: rid(1),
                slot: rid(5)
            })
        );
        // set 안 token 이름 중복.
        let dup_name = LibraryExports {
            components: vec![],
            token_sets: vec![token_set(
                2,
                vec![
                    token(3, "same", "Number", b"1"),
                    token(4, "same", "Number", b"2"),
                ],
            )],
        };
        assert_eq!(
            build(dup_name, &limits),
            Err(SnapshotError::DuplicateTokenName {
                set: rid(2),
                token: rid(4)
            })
        );
        // 잘못된 타입 표기(빈 값·제어 문자).
        for bad in ["", "Num\u{7}"] {
            let c = component(1, "A", vec![property(2, "x", bad, b"1")]);
            assert_eq!(
                build(one(c), &limits),
                Err(SnapshotError::InvalidTypeSignature { id: rid(2) })
            );
        }
    }

    #[test]
    fn export_limits_are_enforced() {
        let tight = SnapshotLimits {
            max_components: 1,
            max_properties_per_component: 1,
            max_slots_per_component: 1,
            max_token_sets: 1,
            max_tokens: 1,
            max_name_bytes: 4,
            max_type_signature_bytes: 6,
            max_fingerprint_bytes: 2,
        };
        let two_components = LibraryExports {
            components: vec![component(1, "A", vec![]), component(2, "B", vec![])],
            token_sets: vec![],
        };
        assert_eq!(
            build(two_components, &tight),
            Err(SnapshotError::TooManyComponents { max: 1 })
        );
        let props = component(
            1,
            "A",
            vec![property(2, "x", "N", b"1"), property(3, "y", "N", b"1")],
        );
        assert_eq!(
            build(
                LibraryExports {
                    components: vec![props],
                    token_sets: vec![]
                },
                &tight
            ),
            Err(SnapshotError::TooManyProperties {
                component: rid(1),
                max: 1
            })
        );
        let mut slots = component(1, "A", vec![]);
        slots.slots = vec![rid(5), rid(6)];
        assert_eq!(
            build(
                LibraryExports {
                    components: vec![slots],
                    token_sets: vec![]
                },
                &tight
            ),
            Err(SnapshotError::TooManySlots {
                component: rid(1),
                max: 1
            })
        );
        let two_sets = LibraryExports {
            components: vec![],
            token_sets: vec![token_set(1, vec![]), token_set(2, vec![])],
        };
        assert_eq!(
            build(two_sets, &tight),
            Err(SnapshotError::TooManyTokenSets { max: 1 })
        );
        let two_tokens = LibraryExports {
            components: vec![],
            token_sets: vec![token_set(
                1,
                vec![token(2, "a", "N", b"1"), token(3, "b", "N", b"1")],
            )],
        };
        assert_eq!(
            build(two_tokens, &tight),
            Err(SnapshotError::TooManyTokens { max: 1 })
        );
        let long_name = component(1, "Toolong", vec![]);
        assert_eq!(
            build(
                LibraryExports {
                    components: vec![long_name],
                    token_sets: vec![]
                },
                &tight
            ),
            Err(SnapshotError::NameTooLong { id: rid(1) })
        );
        let long_type = component(1, "A", vec![property(2, "x", "Numbers", b"1")]);
        assert_eq!(
            build(
                LibraryExports {
                    components: vec![long_type],
                    token_sets: vec![]
                },
                &tight
            ),
            Err(SnapshotError::InvalidTypeSignature { id: rid(2) })
        );
        let big_value = LibraryExports {
            components: vec![],
            token_sets: vec![token_set(1, vec![token(2, "a", "N", b"123")])],
        };
        assert_eq!(
            build(big_value, &tight),
            Err(SnapshotError::FingerprintTooLarge { id: rid(2) })
        );
        // 오류 메시지는 이름·값 원문을 담지 않음.
        let text = SnapshotError::NameTooLong { id: rid(1) }.to_string();
        assert!(!text.contains("Toolong"));
    }

    /// read lock 보유 여부를 확인하며 export를 돌려주는 공급자.
    struct LockProbeSource {
        locks: CacheLocks,
        hash: ContentHashText,
        exports: LibraryExports,
        delete_while_reading: Option<CacheLockError>,
    }

    impl LibraryExportSource for LockProbeSource {
        fn read_exports(
            &mut self,
            root: &Path,
            _: LibraryId,
        ) -> Result<LibraryExports, ExportReadFailure> {
            assert!(root.join("stellar.yaml").is_file());
            self.delete_while_reading = self.locks.try_delete_entry(&self.hash).err();
            Ok(self.exports.clone())
        }
    }

    #[test]
    fn cached_snapshot_is_read_under_shared_lock_and_detached_after() {
        let tmp = TempDir::new("library-snapshot");
        let Some(mut git) = SystemGit::find(tmp.path()) else {
            return;
        };
        let lib = tmp.path().join("lib");
        std::fs::create_dir_all(lib.join("design")).unwrap();
        git.ok(&lib, ["init", "-q", "-b", "main"]);
        git.ok(&lib, ["config", "user.name", "Fixture"]);
        git.ok(&lib, ["config", "user.email", "fixture@example.com"]);
        git.ok(&lib, ["config", "commit.gpgsign", "false"]);
        std::fs::write(lib.join("stellar.yaml"), "schema_version: 1\n").unwrap();
        std::fs::write(lib.join("design/chunk"), [1u8, 2, 3]).unwrap();
        git.ok(&lib, ["add", "-A"]);
        git.ok(&lib, ["commit", "-q", "-m", "v1"]);
        let head: CommitId = git.ok(&lib, ["rev-parse", "HEAD"]).trim().parse().unwrap();
        let url = RepositoryUrl::new(lib.to_str().unwrap()).unwrap();
        let root = LibraryCacheRoot::in_app_data(&tmp.path().join("app")).unwrap();
        let mut store = LibraryEntryStore::new(root.clone(), FakeSha256, FnvKeyer::new(5));
        let mut mirrors = MirrorCache::new(
            root.clone(),
            SystemGit::find(tmp.path()).unwrap(),
            FnvKeyer::new(5),
        );
        let mut runner = SystemGit::find(tmp.path()).unwrap();
        let node = DependencyNodeKey::new(library(7), head.clone());
        let limits = SnapshotLimits::default();

        // materialize 전: entry 없음(offline·미 cache) → export를 읽지 않음.
        let mut table = TableExportSource::default();
        let request = SnapshotRequest {
            repository: &url,
            node: &node,
            expected_content_hash: None,
        };
        assert_eq!(
            load_cached_snapshot(&store, request, &mut table, &limits, 1),
            Err(SnapshotLoadError::MissingEntry)
        );
        assert!(table.calls.is_empty());

        let entry = store
            .materialize(&mut runner, &mut mirrors, &url, &head, 2)
            .unwrap();
        let mut probe = LockProbeSource {
            locks: store.locks(),
            hash: entry.content_hash.clone(),
            exports: version_exports("Button", b"#f00"),
            delete_while_reading: None,
        };
        let git_before = runner.calls.clone();
        let snap = load_cached_snapshot(
            &store,
            SnapshotRequest {
                expected_content_hash: Some(&entry.content_hash),
                ..request
            },
            &mut probe,
            &limits,
            3,
        )
        .unwrap();
        // 읽는 동안 공유 read lock 보유 → 삭제(배타) 시도 Busy.
        assert_eq!(probe.delete_while_reading, Some(CacheLockError::Busy));
        // snapshot 이후 lock 해제, Git 실행 없음(offline 동작).
        assert!(store.locks().try_delete_entry(&entry.content_hash).is_ok());
        assert_eq!(runner.calls, git_before);
        assert_eq!(snap.node(), &node);
        assert_eq!(snap.content_hash(), &entry.content_hash);
        assert_eq!(
            snap.component(ComponentRef::new(rid(1))).unwrap().key(),
            LibraryResourceKey::new(node.clone(), rid(1))
        );

        // 기대 content hash가 다르면 export를 읽지 않고 실패(다른 entry 자동 선택 금지).
        let other = hash(9);
        let mut table = TableExportSource::default();
        assert_eq!(
            load_cached_snapshot(
                &store,
                SnapshotRequest {
                    expected_content_hash: Some(&other),
                    ..request
                },
                &mut table,
                &limits,
                4,
            ),
            Err(SnapshotLoadError::ContentHashMismatch {
                expected: other.clone(),
                found: entry.content_hash.clone(),
            })
        );
        assert!(table.calls.is_empty());

        // export 읽기 실패·검증 실패는 구조화 오류.
        let mut table = TableExportSource::default();
        assert_eq!(
            load_cached_snapshot(&store, request, &mut table, &limits, 5),
            Err(SnapshotLoadError::Export(ExportReadFailure::Corrupt))
        );
        assert_eq!(table.calls, [(entry.dir.clone(), library(7))]);
        table.by_root.insert(
            entry.dir.clone(),
            LibraryExports {
                components: vec![component(1, "A", vec![]), component(1, "B", vec![])],
                token_sets: vec![],
            },
        );
        assert_eq!(
            load_cached_snapshot(&store, request, &mut table, &limits, 6),
            Err(SnapshotLoadError::Invalid(
                SnapshotError::DuplicateResource { id: rid(1) }
            ))
        );
    }
}
