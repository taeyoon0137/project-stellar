//! `ChunkId` — StableId 기반 불변 논리 chunk 식별자 (V.2.1).
//!
//! chunk의 **논리** 식별자입니다. 내용이 바뀌어 content hash(V.1.5)가 달라져도
//! ChunkId는 유지됩니다(V.3 pointer 매핑). Node 등 다른 도메인의 StableId와
//! 타입이 섞이지 않도록 newtype으로 감쌉니다.

use std::fmt;
use std::str::FromStr;

use stellar_core_types::ids::{ParseStableIdError, StableId};

/// 불변 논리 chunk 식별자.
///
/// # 다른 StableId 도메인과 혼용은 컴파일 실패
///
/// `ChunkId`를 요구하는 자리에 원시 `StableId`를 넘길 수 없습니다.
///
/// ```compile_fail
/// use stellar_storage::chunk::ChunkId;
/// use stellar_core_types::ids::StableId;
/// fn wants_chunk(id: ChunkId) {}
/// let raw = StableId::NIL;
/// wants_chunk(raw); // StableId ≠ ChunkId — 컴파일 실패
/// ```
///
/// 반대로 `ChunkId`를 원시 `StableId` 자리에도 넘길 수 없습니다.
///
/// ```compile_fail
/// use stellar_storage::chunk::ChunkId;
/// use stellar_core_types::ids::StableId;
/// fn wants_stable(id: StableId) {}
/// let chunk = ChunkId::from_stable_id(StableId::NIL);
/// wants_stable(chunk); // ChunkId ≠ StableId — 컴파일 실패
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChunkId(StableId);

impl ChunkId {
    /// StableId로부터 chunk 식별자를 만듭니다.
    pub const fn from_stable_id(id: StableId) -> ChunkId {
        ChunkId(id)
    }

    /// 내부 StableId.
    pub const fn stable_id(&self) -> StableId {
        self.0
    }

    /// 16바이트(big-endian) 직렬화.
    pub const fn to_bytes(&self) -> [u8; 16] {
        self.0.to_bytes()
    }

    /// 16바이트(big-endian)로부터 복원합니다.
    pub const fn from_bytes(bytes: [u8; 16]) -> ChunkId {
        ChunkId(StableId::from_bytes(bytes))
    }
}

impl fmt::Display for ChunkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for ChunkId {
    type Err = ParseStableIdError;

    fn from_str(s: &str) -> Result<ChunkId, ParseStableIdError> {
        Ok(ChunkId(s.parse()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ChunkId {
        ChunkId::from_stable_id(StableId::new_v7(
            0x0192_3c4d_5e6f,
            0x0abc,
            0x0123_4567_89ab_cdef,
        ))
    }

    #[test]
    fn binary_round_trip() {
        let id = sample();
        assert_eq!(ChunkId::from_bytes(id.to_bytes()), id);
    }

    #[test]
    fn string_round_trip() {
        let id = sample();
        let text = id.to_string();
        let parsed: ChunkId = text.parse().unwrap();
        assert_eq!(parsed, id);
        assert_eq!(parsed.to_string(), text);
    }

    #[test]
    fn ordering_follows_stable_id_bytes() {
        let a = ChunkId::from_bytes([0u8; 16]);
        let mut high = [0u8; 16];
        high[0] = 1;
        let b = ChunkId::from_bytes(high);
        assert!(a < b);
        assert_eq!(a.cmp(&b), a.to_bytes().cmp(&b.to_bytes()));
    }
}
