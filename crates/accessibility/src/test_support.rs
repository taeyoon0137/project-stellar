//! 테스트 전용 fixture 도우미.

use stellar_core_types::ids::StableId;

/// 마지막 바이트가 `n`인 결정론 StableId.
pub(crate) fn id(n: u8) -> StableId {
    let mut bytes = [0u8; 16];
    bytes[15] = n;
    StableId::from_bytes(bytes)
}
