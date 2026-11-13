//! layout revision newtype (X.1.4).
//!
//! layout pass가 결과를 확정할 때마다 증가하는 카운터입니다. 문서·scene·storage revision
//! (II.3.4)과 서로 다른 타입이라 혼용을 컴파일 단계에서 막습니다.

use std::fmt;

/// layout revision. layout 결과를 확정한 pass 번호입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LayoutRevision(
    /// revision 번호.
    pub u64,
);

impl LayoutRevision {
    /// 최초 revision(아직 layout pass가 확정되지 않음).
    pub const INITIAL: LayoutRevision = LayoutRevision(0);

    /// 다음 revision. `u64` 범위를 넘으면 `None`을 반환합니다(감싸지 않음).
    pub const fn checked_next(self) -> Option<LayoutRevision> {
        match self.0.checked_add(1) {
            Some(next) => Some(LayoutRevision(next)),
            None => None,
        }
    }
}

impl fmt::Display for LayoutRevision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "layout#{}", self.0)
    }
}

/// `LayoutRevision`은 문서 revision과 비교할 수 없습니다(컴파일 실패).
///
/// ```compile_fail
/// use stellar_core_types::revision::DocumentRevision;
/// use stellar_layout::revision::LayoutRevision;
/// let l = LayoutRevision(1);
/// let d = DocumentRevision(1);
/// let _ = l == d;
/// ```
///
/// `LayoutRevision`에 scene revision을 대입할 수 없습니다(컴파일 실패).
///
/// ```compile_fail
/// use stellar_core_types::revision::SceneRevision;
/// use stellar_layout::revision::LayoutRevision;
/// let s = SceneRevision(1);
/// let _l: LayoutRevision = s;
/// ```
#[cfg(doctest)]
struct LayoutRevisionCompileFailDocs;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_type_compares_and_orders() {
        assert_eq!(LayoutRevision(3), LayoutRevision(3));
        assert!(LayoutRevision::INITIAL < LayoutRevision(1));
        assert_eq!(LayoutRevision::INITIAL.0, 0);
    }

    #[test]
    fn checked_next_increments_and_stops_at_max() {
        assert_eq!(LayoutRevision(4).checked_next(), Some(LayoutRevision(5)));
        assert_eq!(LayoutRevision(u64::MAX).checked_next(), None);
    }

    #[test]
    fn display_shows_number() {
        assert_eq!(LayoutRevision(12).to_string(), "layout#12");
    }
}
