//! revision newtype.
//!
//! 문서·scene·storage의 버전 카운터를 서로 다른 타입으로 분리해 혼용을 컴파일 단계에서 막습니다.

/// 문서 revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocumentRevision(
    /// revision 번호.
    pub u64,
);

/// scene revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SceneRevision(
    /// revision 번호.
    pub u64,
);

/// storage generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StorageGeneration(
    /// generation 번호.
    pub u64,
);

/// 서로 다른 revision 타입은 비교할 수 없습니다(컴파일 실패).
///
/// ```compile_fail
/// use stellar_core_types::revision::{DocumentRevision, SceneRevision};
/// let d = DocumentRevision(1);
/// let s = SceneRevision(1);
/// let _ = d == s;
/// ```
///
/// 서로 다른 revision 타입은 대입할 수 없습니다(컴파일 실패).
///
/// ```compile_fail
/// use stellar_core_types::revision::{DocumentRevision, SceneRevision};
/// let d = DocumentRevision(1);
/// let _s: SceneRevision = d;
/// ```
#[cfg(doctest)]
struct RevisionCompileFailDocs;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_type_compares_and_orders() {
        assert_eq!(DocumentRevision(1), DocumentRevision(1));
        assert!(DocumentRevision(1) < DocumentRevision(2));
        assert_eq!(SceneRevision(5).0, 5);
        assert_eq!(StorageGeneration(7).0, 7);
    }
}
