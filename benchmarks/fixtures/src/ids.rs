//! fixture용 결정론적 StableId 발급기.
//!
//! UUIDv7의 timestamp 자리에 `seed`에서 유도한 기준값 + 단조 증가 counter를 넣어, 한 발급기가
//! 낸 id는 **구성상 전부 유일**하고 NIL이 아닙니다. 난수 비트(rand_a·rand_b)는 seed stream에서
//! 채워 id 분포가 실제 UUIDv7과 비슷하게 흩어지게 합니다. 실제 시계는 쓰지 않습니다.
//!
//! 프로젝트 fixture는 문서 사이에서도 id가 겹치지 않도록 발급기 하나를 모든 문서가 공유합니다.

use stellar_core_types::ids::StableId;

use crate::rng::SplitMix64;

/// timestamp 기준값(2025-06 무렵, ms).
const BASE_MS: u64 = 1_750_000_000_000;
/// seed별 timestamp 구간 수.
const SEED_SLOTS: u64 = 1 << 16;
/// seed 구간 하나의 폭(ms). 한 fixture가 이보다 많은 id를 내도 그 fixture 안에서는 유일합니다.
const SLOT_WIDTH: u64 = 1 << 26;

/// 결정론적 StableId 발급기.
#[derive(Debug, Clone)]
pub struct IdAllocator {
    rng: SplitMix64,
    base_ms: u64,
    issued: u64,
}

impl IdAllocator {
    /// `seed`의 발급기.
    pub fn new(seed: u64) -> IdAllocator {
        let mut rng = SplitMix64::new(seed ^ 0x1D5E_ED00_0000_0001);
        let slot = rng.next_u64() % SEED_SLOTS;
        IdAllocator {
            rng,
            base_ms: BASE_MS + slot * SLOT_WIDTH,
            issued: 0,
        }
    }

    /// 다음 id. 같은 seed와 같은 호출 순서는 언제나 같은 id를 냅니다.
    pub fn next_id(&mut self) -> StableId {
        self.issued += 1;
        let rand = self.rng.next_u64();
        let rand_b = self.rng.next_u64();
        StableId::new_v7(self.base_ms + self.issued, (rand & 0x0FFF) as u16, rand_b)
    }

    /// 지금까지 발급한 id 수.
    pub fn issued(&self) -> u64 {
        self.issued
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn ids_are_unique_deterministic_and_non_nil() {
        let mut a = IdAllocator::new(9);
        let mut b = IdAllocator::new(9);
        let mut seen = BTreeSet::new();
        for _ in 0..10_000 {
            let id = a.next_id();
            assert_eq!(id, b.next_id());
            assert_ne!(id, StableId::NIL);
            assert_eq!(id.version(), 7);
            assert!(seen.insert(id));
        }
        assert_eq!(a.issued(), 10_000);
        let mut c = IdAllocator::new(10);
        assert_ne!(c.next_id(), IdAllocator::new(9).next_id());
    }
}
