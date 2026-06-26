//! 자식 순서 키 `OrderKey` (III.5).
//!
//! 자식 순서는 배열 인덱스가 아니라 결정론적 [`OrderKey`]로 결정합니다(기술 아키텍처 12.7).
//! 요구사항은 `docs/architecture/order-key-requirements.md`에 정의되어 있습니다.
//!
//! 알고리즘은 **결정론적 fractional indexing**입니다(III.5.2, ADR-0003): 키를 base-256 소수
//! byte string으로 보고, [`OrderKey::between`]이 좌/우 이웃 사이의 결정론적 중간값을
//! 생성합니다(난수 미사용). 동일 키 충돌은 상위 정렬에서 `StableId`로 tie-break 합니다.
//! 재균형은 III.5.3입니다.

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

    /// 좌/우 이웃 사이의 키를 결정론적으로 생성합니다(난수 미사용). `left`/`right`가 `None`
    /// 이면 각각 하한(0.0)/상한(1.0)입니다. `left < right`(존재 시)를 가정합니다.
    ///
    /// 결과는 `left < 결과 < right`를 만족하는 정규형 키입니다(비어 있지 않고 `0x00` 미종료).
    pub fn between(left: Option<&OrderKey>, right: Option<&OrderKey>) -> OrderKey {
        let lower: &[u8] = left.map_or(&[], |k| k.0.as_slice());
        let upper: Option<&[u8]> = right.map(|k| k.0.as_slice());
        OrderKey(between_bytes(lower, upper))
    }

    /// byte sequence로부터 키를 검증·생성합니다(비어 있거나 `0x00`으로 끝나면 거부).
    pub fn from_bytes(bytes: Vec<u8>) -> Result<OrderKey, OrderKeyError> {
        match bytes.last() {
            None => Err(OrderKeyError::Empty),
            Some(0) => Err(OrderKeyError::TrailingZero),
            Some(_) => Ok(OrderKey(bytes)),
        }
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

/// `lower`(빈 슬라이스=0.0)와 `upper`(`None`=1.0) 소수 사이의 결정론적 중간값 byte열.
///
/// 자릿수를 앞에서부터 비교합니다. 두 자릿수 사이에 간격(≥2)이 있으면 그 중간 자릿수로
/// 종료하고, 간격이 1이면 좌측 자릿수를 따라 내려가며 상한 제약을 풉니다(이후 상한은 1.0).
/// 항상 0이 아닌 자릿수로 끝나 정규형(후행 0 없음)을 유지합니다. `lower < upper`를 가정합니다.
fn between_bytes(lower: &[u8], mut upper: Option<&[u8]>) -> Vec<u8> {
    let mut result = Vec::new();
    let mut i = 0;
    loop {
        let l = u16::from(lower.get(i).copied().unwrap_or(0));
        // 상한 자릿수. None(=1.0)이면 256 sentinel. `lower < upper` 가정하에 항상 l <= u.
        let u = match upper {
            Some(up) => up.get(i).copied().map_or(256, u16::from),
            None => 256,
        };
        if u - l >= 2 {
            // 간격이 있으면 중간 자릿수(>l, <u, 따라서 ≥1)로 종료.
            result.push((l + (u - l) / 2) as u8);
            return result;
        }
        // 간격이 없음(u == l 또는 u == l+1): 좌측 자릿수를 따라 내려간다.
        result.push(l as u8);
        i += 1;
        if u == l + 1 {
            // 이 자릿수에서 이미 상한보다 작아졌으므로 이후 상한은 1.0.
            upper = None;
        }
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

    // --- III.5.2 구현 검증 ---

    #[test]
    fn between_results_are_canonical() {
        // 생성된 키는 비어 있지 않고 0x00으로 끝나지 않음(정규형).
        let a = OrderKey::between(None, None);
        assert!(!a.as_bytes().is_empty());
        assert_ne!(a.as_bytes().last(), Some(&0));

        let keys = [
            OrderKey::between(None, Some(&a)),
            OrderKey::between(Some(&a), None),
            OrderKey::between(None, None),
        ];
        for k in &keys {
            assert!(!k.as_bytes().is_empty());
            assert_ne!(k.as_bytes().last(), Some(&0), "후행 0: {:?}", k.as_bytes());
        }
    }

    #[test]
    fn between_handles_tight_neighbors() {
        // 인접 byte 차이가 1인 이웃 사이에도 키를 만들 수 있음(자릿수 확장).
        let a = OrderKey::from_bytes(vec![0x10]).unwrap();
        let b = OrderKey::from_bytes(vec![0x11]).unwrap();
        let mid = OrderKey::between(Some(&a), Some(&b));
        assert!(a < mid && mid < b, "{:?} < {:?} < {:?}", a, mid, b);

        // 동일 prefix + 인접: [5,3] 과 [5,3,1] 사이.
        let c = OrderKey::from_bytes(vec![5, 3]).unwrap();
        let d = OrderKey::from_bytes(vec![5, 3, 1]).unwrap();
        let mid2 = OrderKey::between(Some(&c), Some(&d));
        assert!(c < mid2 && mid2 < d);
    }

    #[test]
    fn repeated_back_insert_stays_ordered() {
        // 같은 끝에 반복 삽입해도 순서 유지(키가 점진적으로 자랄 수 있음).
        let mut last: Option<OrderKey> = None;
        let mut prev: Option<OrderKey> = None;
        for _ in 0..1000 {
            let next = OrderKey::between(last.as_ref(), None);
            if let Some(p) = &prev {
                assert!(p < &next, "뒤 삽입이 단조 증가해야 함");
            }
            prev = Some(next.clone());
            last = Some(next);
        }
    }

    /// 결정론적 xorshift64(테스트 전용). 키 생성이 아니라 삽입 위치 선택에만 사용합니다.
    fn xorshift(state: &mut u64) -> u64 {
        let mut x = *state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *state = x;
        x
    }

    /// 무작위 probe 키(정규형). 삽입 위치를 정하기 위한 임시 키입니다.
    fn random_probe(state: &mut u64) -> OrderKey {
        let r = xorshift(state);
        let mut bytes = r.to_be_bytes()[..3].to_vec();
        while bytes.last() == Some(&0) {
            bytes.pop();
        }
        if bytes.is_empty() {
            bytes.push(0x80);
        }
        OrderKey(bytes)
    }

    #[test]
    fn property_million_inserts_no_duplicate_or_inversion() {
        use std::collections::BTreeSet;

        // 100만 회 임의 삽입: 중복·순서 역전 없음(III.5.2 완료 기준).
        let mut set: BTreeSet<OrderKey> = BTreeSet::new();
        let mut rng: u64 = 0x9E37_79B9_7F4A_7C15;

        for _ in 0..1_000_000 {
            let probe = random_probe(&mut rng);
            let pred = set.range(..probe.clone()).next_back().cloned();
            let succ = set.range(probe..).next().cloned();

            let new = OrderKey::between(pred.as_ref(), succ.as_ref());

            // 순서 역전 없음.
            if let Some(p) = &pred {
                assert!(p < &new, "pred 순서 역전");
            }
            if let Some(s) = &succ {
                assert!(&new < s, "succ 순서 역전");
            }
            // 중복 없음(인접 구간에 기존 키가 없으므로 항상 새 키).
            assert!(set.insert(new), "중복 키 생성");
        }

        assert_eq!(set.len(), 1_000_000);
    }
}
