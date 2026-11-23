//! 계산 중 memo·cache map의 빠른 hasher (crate 내부).
//!
//! 배치 요청 key는 짧은 정수 묶음이라 표준 SipHash의 비용이 크게 보입니다. 여기의 hasher는 Fx 계열(곱셈·회전)
//! 혼합입니다. map 순회 순서는 결과에 쓰지 않으므로(조회만) hash 값이 결과에 영향을 주지 않습니다. 신뢰할 수 없는
//! 입력의 충돌 공격은 상한 검사(node 수·깊이)로 막힌 범위 안에서만 생깁니다.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

/// Fx 계열 hasher.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FastHasher {
    hash: u64,
}

impl FastHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FastHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            let mut word = [0u8; 8];
            word.copy_from_slice(chunk);
            self.add(u64::from_le_bytes(word));
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut word = [0u8; 8];
            word[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(word));
        }
    }

    #[inline]
    fn write_u8(&mut self, value: u8) {
        self.add(u64::from(value));
    }

    #[inline]
    fn write_u16(&mut self, value: u16) {
        self.add(u64::from(value));
    }

    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.add(u64::from(value));
    }

    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }

    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }

    #[inline]
    fn write_i32(&mut self, value: i32) {
        self.add(u64::from(value as u32));
    }

    /// 곱셈은 낮은 bit를 잘 섞지 못하므로 잘 섞인 높은 bit를 낮은 쪽으로 돌려 bucket index에 씁니다.
    #[inline]
    fn finish(&self) -> u64 {
        self.hash.rotate_left(26)
    }
}

/// [`FastHasher`]를 쓰는 HashMap.
pub(crate) type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FastHasher>>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::Hash;

    fn hash<T: Hash>(value: &T) -> u64 {
        let mut hasher = FastHasher::default();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn hash_is_deterministic_and_distinguishes_values() {
        assert_eq!(hash(&(1u64, 2i32)), hash(&(1u64, 2i32)));
        assert_ne!(hash(&(1u64, 2i32)), hash(&(2u64, 1i32)));
        assert_ne!(hash(&[1u8, 2, 3]), hash(&[1u8, 2, 4]));
        let mut map: FastMap<u32, &str> = FastMap::default();
        map.insert(7, "a");
        assert_eq!(map.get(&7), Some(&"a"));
    }
}
