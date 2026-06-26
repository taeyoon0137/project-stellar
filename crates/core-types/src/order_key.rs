//! 자식 순서 키 `OrderKey` (III.5).
//!
//! 자식 순서는 배열 인덱스가 아니라 결정론적 [`OrderKey`]로 결정합니다(기술 아키텍처 12.7).
//! 요구사항은 `docs/architecture/order-key-requirements.md`에 정의되어 있습니다.
//!
//! **III.5.1 상태**: 요구사항 fixture만 작성하고 알고리즘은 구현하지 않았습니다. 생성 API는
//! `unimplemented!`이며, 아래 테스트는 현재 미구현으로 실패합니다(red). 알고리즘과 근거는
//! III.5.2의 ADR에서 확정하고 구현합니다. 재균형은 III.5.3입니다.

use std::fmt;

/// `OrderKey` 생성·검증 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderKeyError {
    /// 빈 키(비어 있을 수 없음).
    Empty,
    /// `0x00`으로 끝나는 비정규 키(후행 0 금지).
    TrailingZero,
}

impl fmt::Display for OrderKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OrderKeyError::Empty => f.write_str("OrderKey는 비어 있을 수 없습니다"),
            OrderKeyError::TrailingZero => f.write_str("OrderKey는 0x00으로 끝날 수 없습니다"),
        }
    }
}

impl std::error::Error for OrderKeyError {}

/// 자식 순서 키. 비교 가능한 byte sequence이며 사전식 순서가 곧 시각 순서입니다(III.5).
///
/// 0~1 사이 base-256 소수로 해석되며, 정규형은 비어 있지 않고 `0x00`으로 끝나지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OrderKey(Vec<u8>);

impl OrderKey {
    /// 키 길이가 이 값을 넘으면 부모 재균형이 필요합니다(III.5.3).
    pub const REBALANCE_THRESHOLD: usize = 16;

    /// 좌/우 이웃 사이의 키를 생성합니다. `left`/`right`가 `None`이면 각각 하한/상한입니다.
    ///
    /// **III.5.2에서 구현 예정**(현재 미구현).
    pub fn between(left: Option<&OrderKey>, right: Option<&OrderKey>) -> OrderKey {
        let _ = (left, right);
        unimplemented!("OrderKey 알고리즘은 III.5.2에서 구현합니다")
    }

    /// byte sequence로부터 키를 검증·생성합니다(비어 있거나 `0x00`으로 끝나면 거부).
    ///
    /// **III.5.2에서 구현 예정**(현재 미구현).
    pub fn from_bytes(bytes: Vec<u8>) -> Result<OrderKey, OrderKeyError> {
        let _ = bytes;
        unimplemented!("OrderKey 검증은 III.5.2에서 구현합니다")
    }

    /// 키의 byte sequence.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// 키 길이(byte).
    pub fn byte_len(&self) -> usize {
        self.0.len()
    }

    /// 키가 임계 길이를 넘어 재균형이 필요한지 여부.
    pub fn needs_rebalance(&self) -> bool {
        self.0.len() > Self::REBALANCE_THRESHOLD
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::StableId;

    // 아래 fixture는 요구사항(R1~R5)을 인코딩합니다. III.5.1 시점에는 생성 API가 미구현이라
    // 모두 실패(red)합니다. III.5.2 구현이 green으로 만듭니다.

    #[test]
    fn insert_front_back_between() {
        // R2: 앞·뒤·사이 삽입.
        let a = OrderKey::between(None, None);
        let front = OrderKey::between(None, Some(&a));
        let back = OrderKey::between(Some(&a), None);
        assert!(front < a, "앞 삽입 키는 기존보다 작아야 함");
        assert!(back > a, "뒤 삽입 키는 기존보다 커야 함");
        let mid = OrderKey::between(Some(&front), Some(&a));
        assert!(front < mid && mid < a, "사이 삽입 키는 두 이웃 사이여야 함");
    }

    #[test]
    fn concurrent_branch_inserts_independent() {
        // R3: 같은 이웃 사이에 두 브랜치가 독립 삽입. 결정론적이라 같은 키.
        let a = OrderKey::from_bytes(vec![0x40]).unwrap();
        let b = OrderKey::from_bytes(vec![0x80]).unwrap();
        let x = OrderKey::between(Some(&a), Some(&b));
        let y = OrderKey::between(Some(&a), Some(&b));
        assert!(a < x && x < b);
        assert!(a < y && y < b);
        assert_eq!(x, y, "동일 이웃 입력은 동일 키(결정론)");
    }

    #[test]
    fn equal_key_tie_break_by_stable_id() {
        // R3: 동일 키는 StableId로 tie-break. (OrderKey, StableId) 정렬로 전순서 확보.
        let key = OrderKey::between(None, None);
        let id_lo = StableId::new_v7(1, 0, 1);
        let id_hi = StableId::new_v7(2, 0, 2);
        let mut items = [(key.clone(), id_hi), (key, id_lo)];
        items.sort();
        assert_eq!(items[0].1, id_lo, "키 동일 시 StableId 오름차순");
        assert_eq!(items[1].1, id_hi);
    }

    #[test]
    fn between_is_deterministic() {
        // R4: 난수 미사용. 같은 입력 → 같은 키.
        let a = OrderKey::between(None, None);
        let b = OrderKey::between(None, None);
        assert_eq!(a, b);
    }

    #[test]
    fn order_matches_byte_lexicographic() {
        // R1: 비교 순서가 byte 사전식과 일치.
        let lo = OrderKey::from_bytes(vec![0x10]).unwrap();
        let hi = OrderKey::from_bytes(vec![0x20]).unwrap();
        assert!(lo < hi);
        assert!(lo.as_bytes() < hi.as_bytes());
    }

    #[test]
    fn from_bytes_rejects_non_canonical() {
        // R1: 빈 키·후행 0 거부.
        assert_eq!(OrderKey::from_bytes(Vec::new()), Err(OrderKeyError::Empty));
        assert_eq!(
            OrderKey::from_bytes(vec![0x10, 0x00]),
            Err(OrderKeyError::TrailingZero)
        );
    }

    #[test]
    fn key_length_cap_signals_rebalance() {
        // R5: 길이 상한 초과 시 재균형 신호.
        let short = OrderKey::from_bytes(vec![0x80]).unwrap();
        assert!(!short.needs_rebalance());
        let long = OrderKey::from_bytes(vec![0x80; OrderKey::REBALANCE_THRESHOLD + 1]).unwrap();
        assert!(long.needs_rebalance());
    }
}
