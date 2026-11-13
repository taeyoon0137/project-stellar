//! 결정론적 64-bit digest (내부).
//!
//! cache key의 빠른 hash·로그·fixture 비교용 FNV-1a 64입니다. 암호학적 보장이 없으므로
//! key 동등성 판정은 항상 원본 bytes 비교와 함께 사용합니다.

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a 64 digest.
pub(crate) fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_reference_vectors() {
        // FNV-1a 64 공개 참조값.
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
    }
}
