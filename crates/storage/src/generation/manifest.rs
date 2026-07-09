//! generation manifest (V.4.1).
//!
//! 한 저장 세대의 스냅샷 기술서입니다. binary 형식(canonical, format version 1):
//!
//! | 필드 | 형식 |
//! |---|---|
//! | magic | 8바이트 raw `STARMFST` |
//! | format version | `u32` LE (현재 1) |
//! | generation number | `u64` LE (1부터) |
//! | parent 존재 flag | `bool` 1바이트 |
//! | parent generation | flag=true일 때만 `u64` LE |
//! | project id | 16바이트 raw (StableId, big-endian) |
//! | schema version | `u32` LE |
//! | chunk pointers | `u32` LE count + chunk_id 오름차순 pointer들(V.3.1) |
//! | root document ids | `u32` LE count + 16바이트 raw StableId들(순서 의미 있음) |

use std::fmt;

use stellar_core_types::ids::StableId;

use crate::canonical::decode::{CanonicalDecoder, DecodeError};
use crate::canonical::encode::{CanonicalEncoder, EncodeError};
use crate::chunk::pointer::{
    validate_pointer_set, ChunkPointer, PointerCodecError, PointerSetError,
};

/// manifest magic bytes.
pub const MANIFEST_MAGIC: [u8; 8] = *b"STARMFST";

/// 현재 manifest format version.
pub const MANIFEST_FORMAT_VERSION: u32 = 1;

/// manifest 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    /// magic bytes가 `STARMFST`가 아닙니다.
    BadMagic {
        /// 실제로 읽은 8바이트.
        found: [u8; 8],
    },
    /// 지원하지 않는 format version입니다.
    UnsupportedVersion {
        /// 실제로 읽은 version.
        found: u32,
    },
    /// generation number가 0입니다(1부터 시작).
    ZeroGeneration,
    /// parent generation이 자신보다 크거나 같습니다.
    InvalidParent {
        /// generation number.
        generation: u64,
        /// parent generation.
        parent: u64,
    },
    /// root document id가 중복됩니다.
    DuplicateRootDocument {
        /// 중복된 id.
        id: StableId,
    },
    /// pointer 집합 검증 실패(V.3.2).
    PointerSet(PointerSetError),
    /// pointer 목록이 chunk_id 오름차순이 아닙니다(canonical 위반).
    MisorderedPointers,
    /// pointer 인코딩·디코딩 실패.
    Pointer(PointerCodecError),
    /// canonical 디코딩 실패.
    Decode(DecodeError),
    /// canonical 인코딩 실패.
    Encode(EncodeError),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestError::BadMagic { found } => {
                let hex: String = found.iter().map(|b| format!("{b:02x}")).collect();
                write!(f, "manifest magic이 STARMFST가 아닙니다: 0x{hex}")
            }
            ManifestError::UnsupportedVersion { found } => {
                write!(
                    f,
                    "지원하지 않는 manifest format version입니다: {found} (지원: {MANIFEST_FORMAT_VERSION})"
                )
            }
            ManifestError::ZeroGeneration => {
                f.write_str("generation number는 1부터 시작해야 합니다")
            }
            ManifestError::InvalidParent { generation, parent } => {
                write!(
                    f,
                    "parent generation({parent})은 generation({generation})보다 작아야 합니다"
                )
            }
            ManifestError::DuplicateRootDocument { id } => {
                write!(f, "root document id가 중복됩니다: {id}")
            }
            ManifestError::PointerSet(e) => write!(f, "pointer 집합 검증 실패: {e}"),
            ManifestError::MisorderedPointers => {
                f.write_str("chunk pointer가 chunk_id 오름차순이 아닙니다")
            }
            ManifestError::Pointer(e) => write!(f, "pointer 오류: {e}"),
            ManifestError::Decode(e) => write!(f, "manifest 디코딩 실패: {e}"),
            ManifestError::Encode(e) => write!(f, "manifest 인코딩 실패: {e}"),
        }
    }
}

impl std::error::Error for ManifestError {}

impl From<DecodeError> for ManifestError {
    fn from(e: DecodeError) -> ManifestError {
        ManifestError::Decode(e)
    }
}

impl From<EncodeError> for ManifestError {
    fn from(e: EncodeError) -> ManifestError {
        ManifestError::Encode(e)
    }
}

impl From<PointerCodecError> for ManifestError {
    fn from(e: PointerCodecError) -> ManifestError {
        ManifestError::Pointer(e)
    }
}

/// generation manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationManifest {
    /// 세대 번호(1부터 단조 증가).
    pub generation: u64,
    /// 부모 세대 번호. 첫 세대는 `None`.
    pub parent_generation: Option<u64>,
    /// 프로젝트 StableId (`stellar.yaml`의 project id).
    pub project_id: StableId,
    /// 문서 스키마 version.
    pub schema_version: u32,
    /// 논리 chunk pointer들.
    pub chunk_pointers: Vec<ChunkPointer>,
    /// root document StableId들(순서 의미 있음).
    pub root_document_ids: Vec<StableId>,
}

impl GenerationManifest {
    /// 구조 불변식을 검증합니다.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.generation == 0 {
            return Err(ManifestError::ZeroGeneration);
        }
        if let Some(parent) = self.parent_generation {
            if parent >= self.generation {
                return Err(ManifestError::InvalidParent {
                    generation: self.generation,
                    parent,
                });
            }
        }
        validate_pointer_set(&self.chunk_pointers).map_err(ManifestError::PointerSet)?;
        let mut seen = std::collections::BTreeSet::new();
        for id in &self.root_document_ids {
            if !seen.insert(*id) {
                return Err(ManifestError::DuplicateRootDocument { id: *id });
            }
        }
        Ok(())
    }

    /// manifest를 canonical bytes로 인코딩합니다.
    ///
    /// pointer는 chunk_id 오름차순으로 정렬해 기록합니다(입력 순서 무관).
    pub fn encode(&self) -> Result<Vec<u8>, ManifestError> {
        self.validate()?;
        let mut sorted = self.chunk_pointers.clone();
        sorted.sort();

        let mut enc = CanonicalEncoder::new();
        enc.put_fixed_raw(&MANIFEST_MAGIC);
        enc.put_u32(MANIFEST_FORMAT_VERSION);
        enc.put_u64(self.generation);
        match self.parent_generation {
            Some(parent) => {
                enc.put_bool(true);
                enc.put_u64(parent);
            }
            None => enc.put_bool(false),
        }
        enc.put_fixed_raw(&self.project_id.to_bytes());
        enc.put_u32(self.schema_version);

        enc.put_u32(u32::try_from(sorted.len()).map_err(|_| {
            ManifestError::Encode(EncodeError::LengthOverflow { len: sorted.len() })
        })?);
        for pointer in &sorted {
            pointer.encode_into(&mut enc)?;
        }

        enc.put_u32(u32::try_from(self.root_document_ids.len()).map_err(|_| {
            ManifestError::Encode(EncodeError::LengthOverflow {
                len: self.root_document_ids.len(),
            })
        })?);
        for id in &self.root_document_ids {
            enc.put_fixed_raw(&id.to_bytes());
        }
        Ok(enc.into_bytes())
    }

    /// bytes에서 manifest를 디코딩하고 전부 검증합니다.
    pub fn decode(bytes: &[u8]) -> Result<GenerationManifest, ManifestError> {
        let mut dec = CanonicalDecoder::new(bytes);

        let mut magic = [0u8; 8];
        magic.copy_from_slice(dec.take_raw(8)?);
        if magic != MANIFEST_MAGIC {
            return Err(ManifestError::BadMagic { found: magic });
        }
        let version = dec.take_u32()?;
        if version != MANIFEST_FORMAT_VERSION {
            return Err(ManifestError::UnsupportedVersion { found: version });
        }
        let generation = dec.take_u64()?;
        let parent_generation = if dec.take_bool()? {
            Some(dec.take_u64()?)
        } else {
            None
        };
        let mut project_bytes = [0u8; 16];
        project_bytes.copy_from_slice(dec.take_raw(16)?);
        let project_id = StableId::from_bytes(project_bytes);
        let schema_version = dec.take_u32()?;

        let pointer_count = dec.take_u32()? as usize;
        let mut chunk_pointers = Vec::with_capacity(pointer_count.min(1024));
        for _ in 0..pointer_count {
            chunk_pointers.push(ChunkPointer::decode(&mut dec)?);
        }
        // canonical: 오름차순 강제.
        if chunk_pointers.windows(2).any(|w| w[0] >= w[1]) {
            return Err(ManifestError::MisorderedPointers);
        }

        let root_count = dec.take_u32()? as usize;
        let mut root_document_ids = Vec::with_capacity(root_count.min(1024));
        for _ in 0..root_count {
            let mut id_bytes = [0u8; 16];
            id_bytes.copy_from_slice(dec.take_raw(16)?);
            root_document_ids.push(StableId::from_bytes(id_bytes));
        }
        dec.expect_end()?;

        let manifest = GenerationManifest {
            generation,
            parent_generation,
            project_id,
            schema_version,
            chunk_pointers,
            root_document_ids,
        };
        manifest.validate()?;
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::envelope::ChunkKindCode;
    use crate::chunk::id::ChunkId;
    use crate::hash::ContentHash;

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    fn pointer(ts: u64, payload: &[u8]) -> ChunkPointer {
        ChunkPointer {
            chunk_id: ChunkId::from_stable_id(stable(ts)),
            content_hash: ContentHash::compute(payload),
            kind: ChunkKindCode::required(1),
            schema_version: 1,
        }
    }

    fn sample() -> GenerationManifest {
        GenerationManifest {
            generation: 7,
            parent_generation: Some(6),
            project_id: stable(42),
            schema_version: 3,
            chunk_pointers: vec![pointer(2_000, b"b"), pointer(1_000, b"a")],
            root_document_ids: vec![stable(500), stable(400)],
        }
    }

    #[test]
    fn golden_round_trip() {
        // 검증 기준(V.4.1): golden manifest round-trip.
        let manifest = sample();
        let bytes = manifest.encode().unwrap();
        let decoded = GenerationManifest::decode(&bytes).unwrap();
        // encode는 pointer를 정렬하므로 round-trip 비교도 정렬 후 수행.
        let mut expected = manifest.clone();
        expected.chunk_pointers.sort();
        assert_eq!(decoded, expected);
        // 재인코딩은 byte-identical(결정론).
        assert_eq!(decoded.encode().unwrap(), bytes);
    }

    #[test]
    fn golden_header_layout_is_fixed() {
        // 필드 순서·형식을 사양대로 수동 구성해 인코더 출력과 비교합니다.
        let manifest = GenerationManifest {
            generation: 1,
            parent_generation: None,
            project_id: stable(42),
            schema_version: 2,
            chunk_pointers: vec![],
            root_document_ids: vec![],
        };
        let mut expected = CanonicalEncoder::new();
        expected.put_fixed_raw(b"STARMFST");
        expected.put_u32(1); // format version
        expected.put_u64(1); // generation
        expected.put_bool(false); // parent 없음
        expected.put_fixed_raw(&stable(42).to_bytes());
        expected.put_u32(2); // schema version
        expected.put_u32(0); // pointer count
        expected.put_u32(0); // root doc count
        assert_eq!(manifest.encode().unwrap(), expected.into_bytes());
    }

    #[test]
    fn first_generation_has_no_parent() {
        let mut manifest = sample();
        manifest.generation = 1;
        manifest.parent_generation = None;
        let bytes = manifest.encode().unwrap();
        let decoded = GenerationManifest::decode(&bytes).unwrap();
        assert_eq!(decoded.parent_generation, None);
    }

    #[test]
    fn zero_generation_is_rejected() {
        let mut manifest = sample();
        manifest.generation = 0;
        manifest.parent_generation = None;
        assert_eq!(
            manifest.encode().unwrap_err(),
            ManifestError::ZeroGeneration
        );
    }

    #[test]
    fn parent_not_less_than_generation_is_rejected() {
        let mut manifest = sample();
        manifest.parent_generation = Some(7);
        assert_eq!(
            manifest.encode().unwrap_err(),
            ManifestError::InvalidParent {
                generation: 7,
                parent: 7
            }
        );
    }

    #[test]
    fn duplicate_chunk_id_is_rejected() {
        let mut manifest = sample();
        let mut dup = manifest.chunk_pointers[0];
        dup.content_hash = ContentHash::compute(b"other");
        manifest.chunk_pointers.push(dup);
        assert!(matches!(
            manifest.encode().unwrap_err(),
            ManifestError::PointerSet(_)
        ));
    }

    #[test]
    fn duplicate_root_document_is_rejected() {
        let mut manifest = sample();
        manifest
            .root_document_ids
            .push(manifest.root_document_ids[0]);
        assert!(matches!(
            manifest.encode().unwrap_err(),
            ManifestError::DuplicateRootDocument { .. }
        ));
    }

    #[test]
    fn misordered_pointers_in_bytes_are_rejected() {
        // 인코더는 항상 정렬하므로, 정렬 위반 bytes를 수동으로 만듭니다.
        let manifest = sample();
        let sorted_bytes = manifest.encode().unwrap();
        let decoded = GenerationManifest::decode(&sorted_bytes).unwrap();
        // pointer 2개를 역순으로 다시 조립.
        let mut enc = CanonicalEncoder::new();
        enc.put_fixed_raw(&MANIFEST_MAGIC);
        enc.put_u32(MANIFEST_FORMAT_VERSION);
        enc.put_u64(decoded.generation);
        enc.put_bool(true);
        enc.put_u64(decoded.parent_generation.unwrap());
        enc.put_fixed_raw(&decoded.project_id.to_bytes());
        enc.put_u32(decoded.schema_version);
        enc.put_u32(2);
        decoded.chunk_pointers[1].encode_into(&mut enc).unwrap();
        decoded.chunk_pointers[0].encode_into(&mut enc).unwrap();
        enc.put_u32(0);
        let err = GenerationManifest::decode(&enc.into_bytes()).unwrap_err();
        assert_eq!(err, ManifestError::MisorderedPointers);
    }

    #[test]
    fn truncated_manifest_is_rejected() {
        let bytes = sample().encode().unwrap();
        for cut in [0, 7, 12, 20, 21, 37, 41, bytes.len() - 1] {
            assert!(
                GenerationManifest::decode(&bytes[..cut]).is_err(),
                "절단 길이 {cut}에서 오류가 아님"
            );
        }
    }

    #[test]
    fn bad_magic_and_version_are_rejected() {
        let mut bytes = sample().encode().unwrap();
        bytes[0] = b'X';
        assert!(matches!(
            GenerationManifest::decode(&bytes).unwrap_err(),
            ManifestError::BadMagic { .. }
        ));

        let mut bytes = sample().encode().unwrap();
        bytes[8] = 0x63;
        assert!(matches!(
            GenerationManifest::decode(&bytes).unwrap_err(),
            ManifestError::UnsupportedVersion { .. }
        ));
    }
}
