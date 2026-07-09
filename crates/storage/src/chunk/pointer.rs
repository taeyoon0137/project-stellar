//! `ChunkPointer` — 논리 ChunkId → 물리 content hash 매핑 (V.3.1).
//!
//! generation manifest(V.4.1)의 항목입니다. 논리 chunk가 어떤 물리 payload를
//! 가리키는지, 어떤 kind·schema version으로 해석해야 하는지를 담습니다.

use std::fmt;

use crate::canonical::decode::{CanonicalDecoder, DecodeError};
use crate::canonical::encode::{CanonicalEncoder, EncodeError};
use crate::hash::{ContentHash, ParseContentHashError};

use super::envelope::ChunkKindCode;
use super::id::ChunkId;

/// pointer 인코딩·디코딩 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PointerCodecError {
    /// canonical 디코딩 실패.
    Decode(DecodeError),
    /// canonical 인코딩 실패.
    Encode(EncodeError),
    /// content hash 문자열이 ADR-0004 형식이 아닙니다.
    InvalidHash {
        /// 파싱 오류 메시지.
        message: String,
    },
}

impl fmt::Display for PointerCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PointerCodecError::Decode(e) => write!(f, "pointer 디코딩 실패: {e}"),
            PointerCodecError::Encode(e) => write!(f, "pointer 인코딩 실패: {e}"),
            PointerCodecError::InvalidHash { message } => {
                write!(f, "content hash 형식이 잘못되었습니다: {message}")
            }
        }
    }
}

impl std::error::Error for PointerCodecError {}

impl From<DecodeError> for PointerCodecError {
    fn from(e: DecodeError) -> PointerCodecError {
        PointerCodecError::Decode(e)
    }
}

impl From<EncodeError> for PointerCodecError {
    fn from(e: EncodeError) -> PointerCodecError {
        PointerCodecError::Encode(e)
    }
}

/// 논리 chunk → 물리 payload 매핑 항목.
///
/// 정렬(`Ord`)은 (chunk_id, content_hash, kind, schema_version) 순서 — manifest
/// 안에서 chunk_id 기준 결정론적 정렬에 사용합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChunkPointer {
    /// 불변 논리 chunk 식별자.
    pub chunk_id: ChunkId,
    /// payload content hash (ADR-0004).
    pub content_hash: ContentHash,
    /// chunk kind code.
    pub kind: ChunkKindCode,
    /// payload 스키마 version.
    pub schema_version: u32,
}

impl ChunkPointer {
    /// pointer를 canonical bytes로 인코딩합니다.
    ///
    /// 형식: chunk_id 16바이트(big-endian raw) + content hash(길이 프리픽스
    /// UTF-8, ADR-0004) + kind `u32` LE + schema version `u32` LE.
    pub fn encode_into(&self, enc: &mut CanonicalEncoder) -> Result<(), PointerCodecError> {
        enc.put_fixed_raw(&self.chunk_id.to_bytes());
        enc.put_string(&self.content_hash.to_string())?;
        enc.put_u32(self.kind.0);
        enc.put_u32(self.schema_version);
        Ok(())
    }

    /// 단독 bytes로 인코딩합니다.
    pub fn encode(&self) -> Result<Vec<u8>, PointerCodecError> {
        let mut enc = CanonicalEncoder::new();
        self.encode_into(&mut enc)?;
        Ok(enc.into_bytes())
    }

    /// decoder 현재 위치에서 pointer를 디코딩합니다.
    pub fn decode(dec: &mut CanonicalDecoder<'_>) -> Result<ChunkPointer, PointerCodecError> {
        let mut id_bytes = [0u8; 16];
        id_bytes.copy_from_slice(dec.take_raw(16)?);
        let chunk_id = ChunkId::from_bytes(id_bytes);
        let hash_text = dec.take_string()?;
        let content_hash: ContentHash = hash_text.parse().map_err(|e: ParseContentHashError| {
            PointerCodecError::InvalidHash {
                message: e.to_string(),
            }
        })?;
        let kind = ChunkKindCode(dec.take_u32()?);
        let schema_version = dec.take_u32()?;
        Ok(ChunkPointer {
            chunk_id,
            content_hash,
            kind,
            schema_version,
        })
    }
}

/// pointer 집합 검증 실패 (V.3.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PointerSetError {
    /// 같은 ChunkId가 서로 다른 content hash를 가리킵니다.
    ConflictingChunkId {
        /// 충돌한 논리 id.
        chunk_id: ChunkId,
        /// 먼저 나온 hash.
        first: ContentHash,
        /// 나중에 나온 hash.
        second: ContentHash,
    },
    /// 같은 ChunkId 항목이 두 번 이상 나타납니다(hash까지 동일한 중복).
    DuplicateChunkId {
        /// 중복된 논리 id.
        chunk_id: ChunkId,
    },
}

impl fmt::Display for PointerSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PointerSetError::ConflictingChunkId {
                chunk_id,
                first,
                second,
            } => {
                write!(
                    f,
                    "ChunkId {chunk_id}가 한 manifest에서 두 hash를 가리킵니다: {first} / {second}"
                )
            }
            PointerSetError::DuplicateChunkId { chunk_id } => {
                write!(f, "ChunkId {chunk_id} 항목이 중복됩니다")
            }
        }
    }
}

impl std::error::Error for PointerSetError {}

/// 한 generation manifest의 pointer 집합을 검증합니다 (V.3.2).
///
/// 같은 ChunkId가 두 번 나타나면 거부합니다 — 다른 hash를 가리키면
/// [`PointerSetError::ConflictingChunkId`], 완전히 같은 항목이 반복되어도
/// [`PointerSetError::DuplicateChunkId`]입니다.
pub fn validate_pointer_set(pointers: &[ChunkPointer]) -> Result<(), PointerSetError> {
    let mut seen: std::collections::BTreeMap<ChunkId, ContentHash> =
        std::collections::BTreeMap::new();
    for pointer in pointers {
        if let Some(first) = seen.get(&pointer.chunk_id) {
            if *first != pointer.content_hash {
                return Err(PointerSetError::ConflictingChunkId {
                    chunk_id: pointer.chunk_id,
                    first: *first,
                    second: pointer.content_hash,
                });
            }
            return Err(PointerSetError::DuplicateChunkId {
                chunk_id: pointer.chunk_id,
            });
        }
        seen.insert(pointer.chunk_id, pointer.content_hash);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::ids::StableId;

    fn pointer(ts: u64, payload: &[u8]) -> ChunkPointer {
        ChunkPointer {
            chunk_id: ChunkId::from_stable_id(StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)),
            content_hash: ContentHash::compute(payload),
            kind: ChunkKindCode::required(1),
            schema_version: 3,
        }
    }

    #[test]
    fn binary_round_trip() {
        let original = pointer(0x0192_3c4d_5e6f, b"pointer payload");
        let bytes = original.encode().unwrap();
        let mut dec = CanonicalDecoder::new(&bytes);
        let decoded = ChunkPointer::decode(&mut dec).unwrap();
        dec.expect_end().unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn multiple_pointers_round_trip_in_sequence() {
        let pointers: Vec<ChunkPointer> = (0..5u64)
            .map(|i| pointer(1_000 + i, format!("payload-{i}").as_bytes()))
            .collect();
        let mut enc = CanonicalEncoder::new();
        for p in &pointers {
            p.encode_into(&mut enc).unwrap();
        }
        let bytes = enc.into_bytes();
        let mut dec = CanonicalDecoder::new(&bytes);
        let decoded: Vec<ChunkPointer> = (0..5)
            .map(|_| ChunkPointer::decode(&mut dec).unwrap())
            .collect();
        dec.expect_end().unwrap();
        assert_eq!(decoded, pointers);
    }

    #[test]
    fn ordering_is_by_chunk_id_first() {
        // UUIDv7의 timestamp가 최상위이므로 ts 순서가 곧 정렬 순서.
        let early = pointer(1_000, b"a");
        let late = pointer(2_000, b"a");
        assert!(early < late);

        let mut list = vec![late, early];
        list.sort();
        assert_eq!(list, vec![early, late]);
    }

    #[test]
    fn corrupted_hash_string_is_rejected() {
        let original = pointer(42, b"x");
        let mut bytes = original.encode().unwrap();
        // hash 문자열 시작(16 + 4바이트 길이 프리픽스 뒤)을 손상.
        bytes[20] = b'X';
        let mut dec = CanonicalDecoder::new(&bytes);
        let err = ChunkPointer::decode(&mut dec).unwrap_err();
        assert!(matches!(err, PointerCodecError::InvalidHash { .. }));
    }

    #[test]
    fn pointer_set_with_unique_ids_passes() {
        let pointers: Vec<ChunkPointer> = (0..10u64)
            .map(|i| pointer(1_000 + i, format!("p{i}").as_bytes()))
            .collect();
        assert_eq!(validate_pointer_set(&pointers), Ok(()));
        assert_eq!(validate_pointer_set(&[]), Ok(()));
    }

    #[test]
    fn same_chunk_id_with_two_hashes_is_rejected() {
        // 검증 기준(V.3.2): duplicate pointer fixture 실패.
        let a = pointer(1_000, b"content-a");
        let mut b = a;
        b.content_hash = ContentHash::compute(b"content-b");
        let err = validate_pointer_set(&[a, b]).unwrap_err();
        assert_eq!(
            err,
            PointerSetError::ConflictingChunkId {
                chunk_id: a.chunk_id,
                first: a.content_hash,
                second: b.content_hash,
            }
        );
    }

    #[test]
    fn exact_duplicate_pointer_is_rejected() {
        let a = pointer(1_000, b"content-a");
        let err = validate_pointer_set(&[a, a]).unwrap_err();
        assert_eq!(
            err,
            PointerSetError::DuplicateChunkId {
                chunk_id: a.chunk_id
            }
        );
    }

    #[test]
    fn two_ids_sharing_one_hash_is_allowed() {
        // 서로 다른 논리 chunk가 같은 내용(dedup)을 가리키는 것은 허용됩니다.
        let a = pointer(1_000, b"shared");
        let b = pointer(2_000, b"shared");
        assert_eq!(validate_pointer_set(&[a, b]), Ok(()));
    }

    #[test]
    fn truncated_pointer_is_rejected() {
        let bytes = pointer(42, b"x").encode().unwrap();
        for cut in [0, 8, 16, 19, 30, bytes.len() - 1] {
            let mut dec = CanonicalDecoder::new(&bytes[..cut]);
            assert!(
                ChunkPointer::decode(&mut dec).is_err(),
                "절단 길이 {cut}에서 오류가 아님"
            );
        }
    }
}
