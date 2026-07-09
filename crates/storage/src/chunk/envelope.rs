//! chunk envelope (V.2.2).
//!
//! `.starchunk` 파일 header입니다. 압축은 아직 사용하지 않습니다(압축 필드 없음 —
//! 도입 시 format version을 올립니다).
//!
//! header 형식(canonical 인코딩, V.1.2):
//!
//! | 필드 | 형식 |
//! |---|---|
//! | magic | 8바이트 raw `STARCHNK` |
//! | format version | `u32` LE (현재 1) |
//! | chunk kind | `u32` LE code |
//! | uncompressed length | `u64` LE |
//! | payload hash | 길이 프리픽스 UTF-8, ADR-0004 문자열 형식 |

use std::fmt;

use crate::canonical::decode::{CanonicalDecoder, DecodeError};
use crate::canonical::encode::{CanonicalEncoder, EncodeError};
use crate::hash::ContentHash;

/// `.starchunk` magic bytes.
pub const CHUNK_MAGIC: [u8; 8] = *b"STARCHNK";

/// 현재 chunk format version.
pub const CHUNK_FORMAT_VERSION: u32 = 1;

/// chunk kind의 숫자 code (registry는 V.2.7).
///
/// 도메인 타입과 섞이지 않게 newtype으로 둡니다. 이름·정책 매핑은 kind registry가
/// 담당합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChunkKindCode(pub u32);

/// envelope 인코딩·디코딩 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvelopeError {
    /// magic bytes가 `STARCHNK`가 아닙니다.
    BadMagic {
        /// 실제로 읽은 8바이트.
        found: [u8; 8],
    },
    /// 지원하지 않는 format version입니다.
    UnsupportedVersion {
        /// 실제로 읽은 version.
        found: u32,
    },
    /// payload hash 문자열이 ADR-0004 형식이 아닙니다.
    InvalidHash {
        /// 파싱 오류 메시지.
        message: String,
    },
    /// canonical 디코딩 실패.
    Decode(DecodeError),
    /// canonical 인코딩 실패.
    Encode(EncodeError),
}

impl fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EnvelopeError::BadMagic { found } => {
                let hex: String = found.iter().map(|b| format!("{b:02x}")).collect();
                write!(f, "chunk magic이 STARCHNK가 아닙니다: 0x{hex}")
            }
            EnvelopeError::UnsupportedVersion { found } => {
                write!(
                    f,
                    "지원하지 않는 chunk format version입니다: {found} (지원: {CHUNK_FORMAT_VERSION})"
                )
            }
            EnvelopeError::InvalidHash { message } => {
                write!(f, "payload hash 형식이 잘못되었습니다: {message}")
            }
            EnvelopeError::Decode(e) => write!(f, "envelope 디코딩 실패: {e}"),
            EnvelopeError::Encode(e) => write!(f, "envelope 인코딩 실패: {e}"),
        }
    }
}

impl std::error::Error for EnvelopeError {}

impl From<DecodeError> for EnvelopeError {
    fn from(e: DecodeError) -> EnvelopeError {
        EnvelopeError::Decode(e)
    }
}

impl From<EncodeError> for EnvelopeError {
    fn from(e: EncodeError) -> EnvelopeError {
        EnvelopeError::Encode(e)
    }
}

/// chunk envelope header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkEnvelope {
    /// chunk kind code.
    pub kind: ChunkKindCode,
    /// 압축 전(=현재는 그대로) payload 길이(byte).
    pub uncompressed_len: u64,
    /// payload의 content hash (ADR-0004).
    pub payload_hash: ContentHash,
}

impl ChunkEnvelope {
    /// payload로부터 envelope를 만듭니다(길이·hash 계산).
    pub fn for_payload(kind: ChunkKindCode, payload: &[u8]) -> ChunkEnvelope {
        ChunkEnvelope {
            kind,
            uncompressed_len: payload.len() as u64,
            payload_hash: ContentHash::compute(payload),
        }
    }

    /// header bytes를 인코딩합니다(현재 format version).
    pub fn encode(&self) -> Result<Vec<u8>, EnvelopeError> {
        let mut enc = CanonicalEncoder::new();
        enc.put_raw_magic(&CHUNK_MAGIC);
        enc.put_u32(CHUNK_FORMAT_VERSION);
        enc.put_u32(self.kind.0);
        enc.put_u64(self.uncompressed_len);
        enc.put_string(&self.payload_hash.to_string())?;
        Ok(enc.into_bytes())
    }

    /// decoder 현재 위치에서 header를 디코딩합니다.
    ///
    /// magic·format version·hash 형식을 검증합니다. payload 자체 검증(길이·hash
    /// 일치)은 chunk read(V.2.6)가 수행합니다.
    pub fn decode(dec: &mut CanonicalDecoder<'_>) -> Result<ChunkEnvelope, EnvelopeError> {
        let mut magic = [0u8; 8];
        magic.copy_from_slice(dec.take_raw(8)?);
        if magic != CHUNK_MAGIC {
            return Err(EnvelopeError::BadMagic { found: magic });
        }
        let version = dec.take_u32()?;
        if version != CHUNK_FORMAT_VERSION {
            return Err(EnvelopeError::UnsupportedVersion { found: version });
        }
        let kind = ChunkKindCode(dec.take_u32()?);
        let uncompressed_len = dec.take_u64()?;
        let hash_text = dec.take_string()?;
        let payload_hash: ContentHash =
            hash_text
                .parse()
                .map_err(
                    |e: crate::hash::ParseContentHashError| EnvelopeError::InvalidHash {
                        message: e.to_string(),
                    },
                )?;
        Ok(ChunkEnvelope {
            kind,
            uncompressed_len,
            payload_hash,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn sample() -> ChunkEnvelope {
        ChunkEnvelope::for_payload(ChunkKindCode(1), b"test")
    }

    #[test]
    fn golden_header_bytes_are_fixed() {
        // kind=1, payload=b"test" (len=4, sha256 test vector).
        // 이 bytes는 format version 1의 golden header입니다. 변경은 format version
        // 상승과 함께만 허용됩니다.
        let expected = "5354415243484e4b\
                        01000000\
                        01000000\
                        0400000000000000\
                        47000000\
                        7368613235363a39663836643038313838346337643635396132666561613063353561643031356133626634663162326230623832326364313564366331356230663030613038";
        assert_eq!(hex(&sample().encode().unwrap()), expected.replace(' ', ""));
    }

    #[test]
    fn round_trip() {
        let envelope = sample();
        let bytes = envelope.encode().unwrap();
        let mut dec = CanonicalDecoder::new(&bytes);
        let decoded = ChunkEnvelope::decode(&mut dec).unwrap();
        dec.expect_end().unwrap();
        assert_eq!(decoded, envelope);
    }

    #[test]
    fn bad_magic_is_rejected() {
        let mut bytes = sample().encode().unwrap();
        bytes[0] = b'X';
        let mut dec = CanonicalDecoder::new(&bytes);
        let err = ChunkEnvelope::decode(&mut dec).unwrap_err();
        assert!(matches!(err, EnvelopeError::BadMagic { .. }));
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let mut bytes = sample().encode().unwrap();
        // magic(8) 뒤 4바이트가 format version.
        bytes[8] = 0xff;
        let mut dec = CanonicalDecoder::new(&bytes);
        let err = ChunkEnvelope::decode(&mut dec).unwrap_err();
        assert_eq!(
            err,
            EnvelopeError::UnsupportedVersion { found: 0x0000_00ff }
        );
    }

    #[test]
    fn invalid_hash_string_is_rejected() {
        // hash 문자열의 알고리즘 이름을 손상.
        let mut bytes = sample().encode().unwrap();
        let pos = bytes.len() - 71; // "sha256:..." 시작
        bytes[pos] = b'x';
        let mut dec = CanonicalDecoder::new(&bytes);
        let err = ChunkEnvelope::decode(&mut dec).unwrap_err();
        assert!(matches!(err, EnvelopeError::InvalidHash { .. }));
    }

    #[test]
    fn truncated_header_is_rejected() {
        let bytes = sample().encode().unwrap();
        for cut in [0, 7, 8, 11, 15, 20, 27, 30, bytes.len() - 1] {
            let mut dec = CanonicalDecoder::new(&bytes[..cut]);
            assert!(
                ChunkEnvelope::decode(&mut dec).is_err(),
                "절단 길이 {cut}에서 오류가 아님"
            );
        }
    }
}
