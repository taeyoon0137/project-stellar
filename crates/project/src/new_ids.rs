//! 새 프로젝트 원본에 사용할 UUIDv7 생성.
use std::io;
use std::time::{SystemTime, UNIX_EPOCH};
use stellar_core_types::ids::StableId;

/// 현재 시각과 OS 난수로 서로 독립적인 UUIDv7을 생성합니다.
pub fn new_ids<const N: usize>() -> io::Result<[StableId; N]> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?;
    let ms = u64::try_from(now.as_millis()).map_err(io::Error::other)?;
    let mut entropy = [[0u8; 10]; N];
    fill_entropy(entropy.as_flattened_mut())?;
    Ok(entropy.map(|bytes| {
        StableId::new_v7(
            ms,
            u16::from_be_bytes([bytes[0], bytes[1]]),
            u64::from_be_bytes(bytes[2..].try_into().expect("8 bytes")),
        )
    }))
}

/// Unix 시스템 난수 adapter. 다른 플랫폼 구현은 ADR-0006의 지원 재개 시 추가합니다.
#[cfg(unix)]
fn fill_entropy(bytes: &mut [u8]) -> io::Result<()> {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")?.read_exact(bytes)
}

#[cfg(not(unix))]
fn fill_entropy(_: &mut [u8]) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}
