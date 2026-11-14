//! 외부 library resource 참조 (XV.5.2).
//!
//! 사용 프로젝트(또는 다른 library)의 문서가 의존성 library의 component·token을 가리키는 참조입니다.
//! 참조는 **선언 이름 + LibraryId + resource StableId**만 저장하고 commit은 저장하지 않습니다. commit은
//! 그 참조가 속한 scope의 `stellar.lock.yaml`이 정합니다. 그래서 의존성을 update해도 문서의 참조를 다시
//! 쓰지 않고, 같은 StableId로 새 버전 resource에 다시 연결됩니다(아키텍처 30.2 "StableId 기준으로 기존
//! Override 재적용").
//!
//! - `dependency`: 참조를 가진 프로젝트의 `stellar.yaml` 의존성 logical name(아키텍처 30.3의
//!   DependencyInstanceId). 같은 library를 두 이름으로 다른 버전에 선언해도 참조가 어느 쪽인지 정해집니다.
//! - `library_id`: 그 이름이 가리켜야 하는 library. 선언이 다른 library로 바뀌면 해석 단계(XV.5.4)가
//!   불일치로 거부합니다.
//!
//! canonical 텍스트 형식(ADR-0023):
//!
//! ```text
//! component:<logical name>@<library id>/<component StableId>
//! token:<logical name>@<library id>/<token StableId>
//! ```
//!
//! logical name은 `@`·`/`·공백을 쓸 수 없으므로(XV.1.1) 구분자가 모호하지 않습니다. UUID는 소문자
//! hyphen 형식만 받습니다. canonical이 아닌 입력(대문자 UUID 등)은 거부해 같은 참조가 언제나 같은 bytes가
//! 되게 합니다.

use std::fmt;
use std::str::FromStr;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::{ComponentRef, ExpectedRefType, TokenRef};

use crate::declaration::{LibraryId, LogicalName, MAX_LOGICAL_NAME_LEN};
use crate::library_snapshot::LibraryResourceKind;
use crate::node::ValueError;

/// component 참조 접두사.
pub const COMPONENT_REF_PREFIX: &str = "component:";
/// token 참조 접두사.
pub const TOKEN_REF_PREFIX: &str = "token:";
/// UUID 텍스트 길이.
const UUID_TEXT_LEN: usize = 36;
/// canonical 참조 텍스트 최대 byte 수.
pub const MAX_EXTERNAL_REF_LEN: usize =
    COMPONENT_REF_PREFIX.len() + MAX_LOGICAL_NAME_LEN + 1 + UUID_TEXT_LEN + 1 + UUID_TEXT_LEN;

/// 참조 텍스트 해석 실패. 입력 원문은 담지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalRefParseError {
    /// [`MAX_EXTERNAL_REF_LEN`]을 넘음.
    TooLong,
    /// 알 수 없는 참조 종류 접두사.
    UnknownKind,
    /// 기대한 종류와 다른 참조(예: component 자리에 token 참조).
    KindMismatch {
        /// 기대한 종류.
        expected: LibraryResourceKind,
        /// 텍스트의 종류.
        found: LibraryResourceKind,
    },
    /// `@`·`/` 구분 구조가 아님.
    Malformed,
    /// 선언 이름 규칙 위반.
    InvalidDependency(ValueError),
    /// library id가 UUID가 아님.
    InvalidLibraryId,
    /// resource id가 UUID가 아님.
    InvalidResourceId,
    /// canonical 형식이 아님(대문자 UUID 등).
    NotCanonical,
}

impl fmt::Display for ExternalRefParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExternalRefParseError::TooLong => {
                write!(f, "외부 참조가 {MAX_EXTERNAL_REF_LEN} byte를 넘습니다")
            }
            ExternalRefParseError::UnknownKind => f.write_str("알 수 없는 외부 참조 종류입니다"),
            ExternalRefParseError::KindMismatch { expected, found } => write!(
                f,
                "{} 참조를 기대했지만 {} 참조입니다",
                expected.as_str(),
                found.as_str()
            ),
            ExternalRefParseError::Malformed => f.write_str(
                "외부 참조는 '<종류>:<이름>@<library id>/<resource id>' 형식이어야 합니다",
            ),
            ExternalRefParseError::InvalidDependency(e) => write!(f, "의존성 이름 오류: {e}"),
            ExternalRefParseError::InvalidLibraryId => f.write_str("library id는 UUID여야 합니다"),
            ExternalRefParseError::InvalidResourceId => {
                f.write_str("resource id는 UUID여야 합니다")
            }
            ExternalRefParseError::NotCanonical => {
                f.write_str("외부 참조가 canonical 형식(소문자 UUID)이 아닙니다")
            }
        }
    }
}

impl std::error::Error for ExternalRefParseError {}

/// 참조 텍스트를 (종류, 이름, library, resource)로 나눕니다.
fn parse_parts(
    text: &str,
) -> Result<(LibraryResourceKind, LogicalName, LibraryId, StableId), ExternalRefParseError> {
    if text.len() > MAX_EXTERNAL_REF_LEN {
        return Err(ExternalRefParseError::TooLong);
    }
    let (kind, rest) = if let Some(rest) = text.strip_prefix(COMPONENT_REF_PREFIX) {
        (LibraryResourceKind::Component, rest)
    } else if let Some(rest) = text.strip_prefix(TOKEN_REF_PREFIX) {
        (LibraryResourceKind::Token, rest)
    } else {
        return Err(ExternalRefParseError::UnknownKind);
    };
    let (name, rest) = rest
        .split_once('@')
        .ok_or(ExternalRefParseError::Malformed)?;
    let (library, resource) = rest
        .split_once('/')
        .ok_or(ExternalRefParseError::Malformed)?;
    let dependency = LogicalName::new(name).map_err(ExternalRefParseError::InvalidDependency)?;
    let library_id: LibraryId = library
        .parse()
        .map_err(|_| ExternalRefParseError::InvalidLibraryId)?;
    let resource_id: StableId = resource
        .parse()
        .map_err(|_| ExternalRefParseError::InvalidResourceId)?;
    if library_id.to_string() != library || resource_id.to_string() != resource {
        return Err(ExternalRefParseError::NotCanonical);
    }
    Ok((kind, dependency, library_id, resource_id))
}

fn expect_kind(
    found: LibraryResourceKind,
    expected: LibraryResourceKind,
) -> Result<(), ExternalRefParseError> {
    if found == expected {
        Ok(())
    } else {
        Err(ExternalRefParseError::KindMismatch { expected, found })
    }
}

/// 외부 library component 참조.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExternalComponentRef {
    dependency: LogicalName,
    library_id: LibraryId,
    component: ComponentRef,
}

impl ExternalComponentRef {
    /// 참조를 만듭니다.
    pub fn new(
        dependency: LogicalName,
        library_id: LibraryId,
        component: ComponentRef,
    ) -> ExternalComponentRef {
        ExternalComponentRef {
            dependency,
            library_id,
            component,
        }
    }

    /// 참조하는 쪽 `stellar.yaml`의 의존성 logical name.
    pub fn dependency(&self) -> &LogicalName {
        &self.dependency
    }

    /// 기대 library.
    pub fn library_id(&self) -> LibraryId {
        self.library_id
    }

    /// library 안 component.
    pub fn component(&self) -> ComponentRef {
        self.component
    }
}

impl fmt::Display for ExternalComponentRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{COMPONENT_REF_PREFIX}{}@{}/{}",
            self.dependency,
            self.library_id,
            self.component.id()
        )
    }
}

impl FromStr for ExternalComponentRef {
    type Err = ExternalRefParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (kind, dependency, library_id, id) = parse_parts(s)?;
        expect_kind(kind, LibraryResourceKind::Component)?;
        Ok(ExternalComponentRef::new(
            dependency,
            library_id,
            ComponentRef::new(id),
        ))
    }
}

/// 외부 library token 참조.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExternalTokenRef {
    dependency: LogicalName,
    library_id: LibraryId,
    token: TokenRef,
}

impl ExternalTokenRef {
    /// 참조를 만듭니다.
    pub fn new(
        dependency: LogicalName,
        library_id: LibraryId,
        token: TokenRef,
    ) -> ExternalTokenRef {
        ExternalTokenRef {
            dependency,
            library_id,
            token,
        }
    }

    /// 참조하는 쪽 `stellar.yaml`의 의존성 logical name.
    pub fn dependency(&self) -> &LogicalName {
        &self.dependency
    }

    /// 기대 library.
    pub fn library_id(&self) -> LibraryId {
        self.library_id
    }

    /// library 안 token.
    pub fn token(&self) -> TokenRef {
        self.token
    }
}

impl fmt::Display for ExternalTokenRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{TOKEN_REF_PREFIX}{}@{}/{}",
            self.dependency,
            self.library_id,
            self.token.id()
        )
    }
}

impl FromStr for ExternalTokenRef {
    type Err = ExternalRefParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (kind, dependency, library_id, id) = parse_parts(s)?;
        expect_kind(kind, LibraryResourceKind::Token)?;
        Ok(ExternalTokenRef::new(
            dependency,
            library_id,
            TokenRef::new(id),
        ))
    }
}

/// 외부 library resource 참조(component 또는 token).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExternalResourceRef {
    /// component 참조.
    Component(ExternalComponentRef),
    /// token 참조.
    Token(ExternalTokenRef),
}

impl ExternalResourceRef {
    /// resource 종류.
    pub fn kind(&self) -> LibraryResourceKind {
        match self {
            ExternalResourceRef::Component(_) => LibraryResourceKind::Component,
            ExternalResourceRef::Token(_) => LibraryResourceKind::Token,
        }
    }

    /// 의존성 logical name.
    pub fn dependency(&self) -> &LogicalName {
        match self {
            ExternalResourceRef::Component(r) => r.dependency(),
            ExternalResourceRef::Token(r) => r.dependency(),
        }
    }

    /// 기대 library.
    pub fn library_id(&self) -> LibraryId {
        match self {
            ExternalResourceRef::Component(r) => r.library_id(),
            ExternalResourceRef::Token(r) => r.library_id(),
        }
    }

    /// library 안 resource StableId.
    pub fn resource(&self) -> StableId {
        match self {
            ExternalResourceRef::Component(r) => r.component().id(),
            ExternalResourceRef::Token(r) => r.token().id(),
        }
    }

    /// 해석 실패 시 [`UnresolvedRef`](stellar_core_types::reference::UnresolvedRef)에 남길 기대 타입.
    pub fn expected_ref_type(&self) -> ExpectedRefType {
        match self {
            ExternalResourceRef::Component(_) => ExpectedRefType::Component,
            ExternalResourceRef::Token(_) => ExpectedRefType::Token,
        }
    }
}

impl From<ExternalComponentRef> for ExternalResourceRef {
    fn from(r: ExternalComponentRef) -> ExternalResourceRef {
        ExternalResourceRef::Component(r)
    }
}

impl From<ExternalTokenRef> for ExternalResourceRef {
    fn from(r: ExternalTokenRef) -> ExternalResourceRef {
        ExternalResourceRef::Token(r)
    }
}

impl fmt::Display for ExternalResourceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExternalResourceRef::Component(r) => r.fmt(f),
            ExternalResourceRef::Token(r) => r.fmt(f),
        }
    }
}

impl FromStr for ExternalResourceRef {
    type Err = ExternalRefParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (kind, dependency, library_id, id) = parse_parts(s)?;
        Ok(match kind {
            LibraryResourceKind::Component => ExternalResourceRef::Component(
                ExternalComponentRef::new(dependency, library_id, ComponentRef::new(id)),
            ),
            LibraryResourceKind::Token => ExternalResourceRef::Token(ExternalTokenRef::new(
                dependency,
                library_id,
                TokenRef::new(id),
            )),
            LibraryResourceKind::TokenSet => return Err(ExternalRefParseError::UnknownKind),
        })
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! 테스트 공용 참조 fixture.

    use super::*;

    /// component 참조.
    pub(crate) fn component_ref(
        dependency: &str,
        lib: LibraryId,
        id: StableId,
    ) -> ExternalComponentRef {
        ExternalComponentRef::new(
            LogicalName::new(dependency).expect("name"),
            lib,
            ComponentRef::new(id),
        )
    }

    /// token 참조.
    pub(crate) fn token_ref(dependency: &str, lib: LibraryId, id: StableId) -> ExternalTokenRef {
        ExternalTokenRef::new(
            LogicalName::new(dependency).expect("name"),
            lib,
            TokenRef::new(id),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::declaration::fixtures::library;
    use crate::library_snapshot::fixtures::rid;

    #[test]
    fn references_round_trip_through_canonical_text() {
        let component = component_ref("design-system", library(1), rid(7));
        let token = token_ref("tokens.v2", library(2), rid(8));
        let component_text = component.to_string();
        let token_text = token.to_string();
        assert_eq!(
            component_text,
            format!("component:design-system@{}/{}", library(1), rid(7))
        );
        assert_eq!(
            token_text,
            format!("token:tokens.v2@{}/{}", library(2), rid(8))
        );

        assert_eq!(
            component_text.parse::<ExternalComponentRef>(),
            Ok(component.clone())
        );
        assert_eq!(token_text.parse::<ExternalTokenRef>(), Ok(token.clone()));
        let any_component: ExternalResourceRef = component_text.parse().unwrap();
        let any_token: ExternalResourceRef = token_text.parse().unwrap();
        assert_eq!(any_component, ExternalResourceRef::from(component.clone()));
        assert_eq!(any_token, ExternalResourceRef::from(token.clone()));
        // 두 번째 왕복도 같은 bytes.
        assert_eq!(any_component.to_string(), component_text);
        assert_eq!(any_token.to_string(), token_text);

        assert_eq!(any_component.kind(), LibraryResourceKind::Component);
        assert_eq!(any_token.kind(), LibraryResourceKind::Token);
        assert_eq!(any_component.dependency().as_str(), "design-system");
        assert_eq!(any_token.library_id(), library(2));
        assert_eq!(any_token.resource(), rid(8));
        assert_eq!(
            any_component.expected_ref_type(),
            ExpectedRefType::Component
        );
        assert_eq!(any_token.expected_ref_type(), ExpectedRefType::Token);
        // 가장 긴 이름도 상한 안에서 왕복.
        let long = component_ref(&"a".repeat(MAX_LOGICAL_NAME_LEN), library(1), rid(1));
        assert_eq!(long.to_string().len(), MAX_EXTERNAL_REF_LEN);
        assert_eq!(long.to_string().parse::<ExternalComponentRef>(), Ok(long));
    }

    #[test]
    fn malformed_and_non_canonical_references_are_rejected() {
        let lib = library(1).to_string();
        let res = rid(3).to_string();
        let parse = |s: &str| s.parse::<ExternalResourceRef>();
        assert_eq!(parse(""), Err(ExternalRefParseError::UnknownKind));
        assert_eq!(
            parse(&format!("frame:ds@{lib}/{res}")),
            Err(ExternalRefParseError::UnknownKind)
        );
        assert_eq!(
            parse(&format!("component:ds{lib}/{res}")),
            Err(ExternalRefParseError::Malformed)
        );
        assert_eq!(
            parse(&format!("component:ds@{lib}{res}")),
            Err(ExternalRefParseError::Malformed)
        );
        assert_eq!(
            parse(&format!("component:-ds@{lib}/{res}")),
            Err(ExternalRefParseError::InvalidDependency(
                ValueError::InvalidCharacter
            ))
        );
        assert_eq!(
            parse(&format!("component:@{lib}/{res}")),
            Err(ExternalRefParseError::InvalidDependency(ValueError::Empty))
        );
        assert_eq!(
            parse(&format!("component:ds@not-a-uuid/{res}")),
            Err(ExternalRefParseError::InvalidLibraryId)
        );
        assert_eq!(
            parse(&format!("component:ds@{lib}/{res}/extra")),
            Err(ExternalRefParseError::InvalidResourceId)
        );
        assert_eq!(
            parse(&format!("component:ds@{lib}/{}", res.to_uppercase())),
            Err(ExternalRefParseError::NotCanonical)
        );
        assert_eq!(
            parse(&format!("component:ds@{}/{res}", lib.to_uppercase())),
            Err(ExternalRefParseError::NotCanonical)
        );
        assert_eq!(
            parse(&format!(
                "component:{}@{lib}/{res}",
                "a".repeat(MAX_LOGICAL_NAME_LEN + 1)
            )),
            Err(ExternalRefParseError::TooLong)
        );
        // 종류가 다른 참조는 타입별 parser가 거부.
        assert_eq!(
            format!("token:ds@{lib}/{res}").parse::<ExternalComponentRef>(),
            Err(ExternalRefParseError::KindMismatch {
                expected: LibraryResourceKind::Component,
                found: LibraryResourceKind::Token,
            })
        );
        assert_eq!(
            format!("component:ds@{lib}/{res}").parse::<ExternalTokenRef>(),
            Err(ExternalRefParseError::KindMismatch {
                expected: LibraryResourceKind::Token,
                found: LibraryResourceKind::Component,
            })
        );
        // 오류 메시지는 입력 원문을 담지 않음.
        let text =
            ExternalRefParseError::InvalidDependency(ValueError::InvalidCharacter).to_string();
        assert!(!text.contains("-ds"));
    }

    #[test]
    fn component_and_token_refs_are_distinct_types() {
        let c = component_ref("ds", library(1), rid(1));
        let t = token_ref("ds", library(1), rid(1));
        // 같은 이름·library·StableId라도 종류가 다르면 다른 참조.
        assert_ne!(ExternalResourceRef::from(c), ExternalResourceRef::from(t));
    }
}
