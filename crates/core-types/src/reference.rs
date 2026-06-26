//! 강한 타입 참조 (III.2.3).
//!
//! `NodeRef`, `TokenRef`, `ComponentRef`, `AssetRef`는 각각 [`StableId`]를 감싸지만 서로
//! 다른 타입입니다. 서로 암묵 변환·대입·비교가 불가능해, 참조 종류를 컴파일 단계에서 강제합니다.

use crate::ids::StableId;
use crate::property::PropertyPath;

/// 노드 참조.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeRef(StableId);

/// 디자인 토큰 참조.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TokenRef(StableId);

/// 컴포넌트 참조.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ComponentRef(StableId);

/// 에셋 참조.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssetRef(StableId);

impl NodeRef {
    /// `StableId`로부터 노드 참조를 만듭니다.
    pub const fn new(id: StableId) -> NodeRef {
        NodeRef(id)
    }
    /// 대상 `StableId`.
    pub const fn id(&self) -> StableId {
        self.0
    }
}

impl TokenRef {
    /// `StableId`로부터 토큰 참조를 만듭니다.
    pub const fn new(id: StableId) -> TokenRef {
        TokenRef(id)
    }
    /// 대상 `StableId`.
    pub const fn id(&self) -> StableId {
        self.0
    }
}

impl ComponentRef {
    /// `StableId`로부터 컴포넌트 참조를 만듭니다.
    pub const fn new(id: StableId) -> ComponentRef {
        ComponentRef(id)
    }
    /// 대상 `StableId`.
    pub const fn id(&self) -> StableId {
        self.0
    }
}

impl AssetRef {
    /// `StableId`로부터 에셋 참조를 만듭니다.
    pub const fn new(id: StableId) -> AssetRef {
        AssetRef(id)
    }
    /// 대상 `StableId`.
    pub const fn id(&self) -> StableId {
        self.0
    }
}

/// 참조가 기대하는 종류(III.2.4). 미해결 참조가 어떤 참조였는지 보존합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExpectedRefType {
    /// 노드 참조였음.
    Node,
    /// 토큰 참조였음.
    Token,
    /// 컴포넌트 참조였음.
    Component,
    /// 에셋 참조였음.
    Asset,
}

/// 미해결(dangling) 참조(III.2.4).
///
/// resolve에 실패해도 원래 `StableId`, 기대 타입, 발견 위치(`PropertyPath`)를 보존해
/// 정보 손실 없이 보관·재구성할 수 있습니다.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UnresolvedRef {
    original_id: StableId,
    expected: ExpectedRefType,
    location: PropertyPath,
}

impl UnresolvedRef {
    /// 미해결 참조를 만듭니다.
    pub fn new(
        original_id: StableId,
        expected: ExpectedRefType,
        location: PropertyPath,
    ) -> UnresolvedRef {
        UnresolvedRef {
            original_id,
            expected,
            location,
        }
    }

    /// 원래 대상 `StableId`.
    pub fn original_id(&self) -> StableId {
        self.original_id
    }

    /// 기대했던 참조 종류.
    pub fn expected(&self) -> ExpectedRefType {
        self.expected
    }

    /// 참조가 발견된 위치.
    pub fn location(&self) -> &PropertyPath {
        &self.location
    }
}

impl NodeRef {
    /// resolve 실패 시 위치를 붙여 미해결 참조로 변환합니다(기대 타입 Node).
    pub fn unresolved_at(&self, location: PropertyPath) -> UnresolvedRef {
        UnresolvedRef::new(self.0, ExpectedRefType::Node, location)
    }
}

impl TokenRef {
    /// resolve 실패 시 위치를 붙여 미해결 참조로 변환합니다(기대 타입 Token).
    pub fn unresolved_at(&self, location: PropertyPath) -> UnresolvedRef {
        UnresolvedRef::new(self.0, ExpectedRefType::Token, location)
    }
}

impl ComponentRef {
    /// resolve 실패 시 위치를 붙여 미해결 참조로 변환합니다(기대 타입 Component).
    pub fn unresolved_at(&self, location: PropertyPath) -> UnresolvedRef {
        UnresolvedRef::new(self.0, ExpectedRefType::Component, location)
    }
}

impl AssetRef {
    /// resolve 실패 시 위치를 붙여 미해결 참조로 변환합니다(기대 타입 Asset).
    pub fn unresolved_at(&self, location: PropertyPath) -> UnresolvedRef {
        UnresolvedRef::new(self.0, ExpectedRefType::Asset, location)
    }
}

/// 서로 다른 참조 타입은 대입할 수 없습니다(컴파일 실패).
///
/// ```compile_fail
/// use stellar_core_types::ids::StableId;
/// use stellar_core_types::reference::{NodeRef, TokenRef};
/// let node = NodeRef::new(StableId::NIL);
/// let _token: TokenRef = node;
/// ```
///
/// 서로 다른 참조 타입은 비교할 수 없습니다(컴파일 실패).
///
/// ```compile_fail
/// use stellar_core_types::ids::StableId;
/// use stellar_core_types::reference::{NodeRef, AssetRef};
/// let node = NodeRef::new(StableId::NIL);
/// let asset = AssetRef::new(StableId::NIL);
/// let _ = node == asset;
/// ```
///
/// `StableId`가 자동으로 참조로 변환되지 않습니다(컴파일 실패).
///
/// ```compile_fail
/// use stellar_core_types::ids::StableId;
/// use stellar_core_types::reference::NodeRef;
/// let _node: NodeRef = StableId::NIL;
/// ```
#[cfg(doctest)]
struct ReferenceCompileFailDocs;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::property::PropertyKey;

    #[test]
    fn unresolved_ref_round_trip_without_loss() {
        let id = StableId::new_v7(7, 0, 7);
        let root = StableId::new_v7(1, 0, 1);
        let location = PropertyPath::new(root).push(PropertyKey(3)).with_index(2);

        // 노드 참조가 resolve에 실패 → 미해결 참조로 변환.
        let unresolved = NodeRef::new(id).unresolved_at(location.clone());
        assert_eq!(unresolved.original_id(), id);
        assert_eq!(unresolved.expected(), ExpectedRefType::Node);
        assert_eq!(unresolved.location(), &location);

        // 보존된 정보로 재구성하면 동일(정보 손실 없음).
        let rebuilt = UnresolvedRef::new(
            unresolved.original_id(),
            unresolved.expected(),
            unresolved.location().clone(),
        );
        assert_eq!(rebuilt, unresolved);
    }

    #[test]
    fn expected_type_matches_source_ref() {
        let id = StableId::new_v7(2, 0, 2);
        let loc = PropertyPath::new(id);
        assert_eq!(
            TokenRef::new(id).unresolved_at(loc.clone()).expected(),
            ExpectedRefType::Token
        );
        assert_eq!(
            ComponentRef::new(id).unresolved_at(loc.clone()).expected(),
            ExpectedRefType::Component
        );
        assert_eq!(
            AssetRef::new(id).unresolved_at(loc).expected(),
            ExpectedRefType::Asset
        );
    }

    #[test]
    fn refs_wrap_and_return_same_id() {
        let id = StableId::new_v7(123, 4, 5);
        assert_eq!(NodeRef::new(id).id(), id);
        assert_eq!(TokenRef::new(id).id(), id);
        assert_eq!(ComponentRef::new(id).id(), id);
        assert_eq!(AssetRef::new(id).id(), id);
    }

    #[test]
    fn same_type_refs_compare() {
        let id = StableId::new_v7(1, 0, 1);
        let other = StableId::new_v7(2, 0, 2);
        assert_eq!(NodeRef::new(id), NodeRef::new(id));
        assert_ne!(NodeRef::new(id), NodeRef::new(other));
    }
}
