//! 강한 타입 참조 (III.2.3).
//!
//! `NodeRef`, `TokenRef`, `ComponentRef`, `AssetRef`는 각각 [`StableId`]를 감싸지만 서로
//! 다른 타입입니다. 서로 암묵 변환·대입·비교가 불가능해, 참조 종류를 컴파일 단계에서 강제합니다.

use crate::ids::StableId;

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
