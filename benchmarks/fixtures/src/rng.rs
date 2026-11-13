//! 결정론적 PRNG(SplitMix64).
//!
//! fixture는 seed만으로 재현돼야 하므로 외부 crate나 OS 난수를 쓰지 않습니다. document
//! crate의 테스트 전용 생성기(VI.10.5)와 같은 알고리즘이지만, 그 모듈은 `test-support`
//! feature 뒤에 있어 일반 의존으로 쓸 수 없으므로 이 crate에 따로 둡니다. 출력 값은
//! [`GENERATOR_VERSION`](crate::GENERATOR_VERSION)의 일부입니다 — 바꾸면 version을 올립니다.

/// SplitMix64 상태.
#[derive(Debug, Clone)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// seed로 만듭니다.
    pub fn new(seed: u64) -> SplitMix64 {
        SplitMix64 { state: seed }
    }

    /// 다음 64비트 값.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// `0..n` 값(`n == 0`이면 0). modulo 편향은 fixture 용도에서 허용합니다.
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next_u64() % n
        }
    }

    /// `lo..=hi` 값(`hi < lo`이면 `lo`).
    pub fn range(&mut self, lo: u64, hi: u64) -> u64 {
        if hi <= lo {
            lo
        } else {
            lo + self.below(hi - lo + 1)
        }
    }

    /// 확률 `1/n`으로 true(`n == 0`이면 항상 true).
    pub fn one_in(&mut self, n: u64) -> bool {
        self.below(n) == 0
    }

    /// 비어 있지 않은 고정 배열에서 하나를 고릅니다.
    pub fn pick<T: Copy, const N: usize>(&mut self, items: &[T; N]) -> T {
        const { assert!(N > 0, "빈 배열에서 고를 수 없습니다") };
        items[self.below(N as u64) as usize]
    }

    /// 가중치 `(값, 가중치)` 목록에서 하나를 고릅니다. 가중치 합이 0이면 첫 값입니다.
    pub fn weighted<T: Copy, const N: usize>(&mut self, items: &[(T, u32); N]) -> T {
        const { assert!(N > 0, "빈 배열에서 고를 수 없습니다") };
        let total: u64 = items.iter().map(|(_, w)| u64::from(*w)).sum();
        let mut roll = self.below(total);
        for (value, weight) in items {
            let weight = u64::from(*weight);
            if roll < weight {
                return *value;
            }
            roll -= weight;
        }
        items[0].0
    }

    /// 독립 하위 stream. 같은 부모 상태와 `stream`은 언제나 같은 하위 stream을 냅니다.
    pub fn fork(&mut self, stream: u64) -> SplitMix64 {
        let base = self.next_u64();
        SplitMix64::new(base ^ stream.wrapping_mul(0xD1B5_4A32_D192_ED03))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream() {
        let mut a = SplitMix64::new(7);
        let mut b = SplitMix64::new(7);
        let xs: Vec<u64> = (0..16).map(|_| a.next_u64()).collect();
        let ys: Vec<u64> = (0..16).map(|_| b.next_u64()).collect();
        assert_eq!(xs, ys);
        let mut c = SplitMix64::new(8);
        assert_ne!(xs[0], c.next_u64());
    }

    #[test]
    fn helpers_stay_in_range() {
        let mut rng = SplitMix64::new(1);
        for _ in 0..1_000 {
            assert!(rng.below(5) < 5);
            let v = rng.range(3, 9);
            assert!((3..=9).contains(&v));
            assert!([1, 2, 3].contains(&rng.pick(&[1, 2, 3])));
        }
        assert_eq!(rng.below(0), 0);
        assert_eq!(rng.range(4, 2), 4);
        // 가중치 0인 값은 나오지 않습니다.
        for _ in 0..1_000 {
            assert_ne!(rng.weighted(&[('a', 1), ('b', 0), ('c', 3)]), 'b');
        }
        assert_eq!(rng.weighted(&[('z', 0)]), 'z');
    }

    #[test]
    fn fork_is_deterministic_and_distinct() {
        let mut a = SplitMix64::new(42);
        let mut b = SplitMix64::new(42);
        let mut fa = a.fork(3);
        let mut fb = b.fork(3);
        assert_eq!(fa.next_u64(), fb.next_u64());
        let mut c = SplitMix64::new(42);
        let mut fc = c.fork(4);
        let mut d = SplitMix64::new(42);
        let mut fd = d.fork(3);
        assert_ne!(fc.next_u64(), fd.next_u64());
    }
}
