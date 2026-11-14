//! DocumentManifest chunk payload (VI.10.2).
//!
//! 형식(canonical, V.1):
//!
//! | 필드 | 형식 |
//! |---|---|
//! | 문서 정의 | `u32` 길이 + [`DocumentDefinition::encode`] bytes |
//! | View 수 | `u32` LE |
//! | View 항목 | View StableId 16바이트 + ViewTree ChunkId 16바이트 |
//!
//! View 항목은 정의의 View 순서(페이지 → View)와 정확히 같아야 합니다.

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_document::definitions::{DefinitionError, DocumentDefinition};
use stellar_storage::canonical::decode::{CanonicalDecoder, DecodeError};
use stellar_storage::canonical::encode::{CanonicalEncoder, EncodeError};
use stellar_storage::chunk::id::ChunkId;

/// View 하나의 ViewTree chunk 참조.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewTreeRef {
    /// View StableId.
    pub view: StableId,
    /// 그 View의 ViewTree chunk.
    pub chunk: ChunkId,
}

/// DocumentManifest chunk payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentManifestChunk {
    /// 문서·페이지·View 정의.
    pub definition: DocumentDefinition,
    /// View별 ViewTree chunk(정의의 View 순서).
    pub view_trees: Vec<ViewTreeRef>,
}

/// manifest payload 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestChunkError {
    /// View 항목이 정의의 View 순서·구성과 다릅니다.
    ViewMismatch {
        /// 문제 위치(항목 index).
        index: usize,
    },
    /// pointer의 schema version 또는 version과 payload의 조합을 해석할 수 없습니다.
    UnsupportedSchema(u32),
    /// 문서 정의 디코딩 실패.
    Definition(DefinitionError),
    /// canonical 디코딩 실패.
    Decode(DecodeError),
    /// canonical 인코딩 실패.
    Encode(EncodeError),
}

impl fmt::Display for ManifestChunkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestChunkError::ViewMismatch { index } => {
                write!(f, "View 항목 {index}가 문서 정의의 View 순서와 다릅니다")
            }
            ManifestChunkError::UnsupportedSchema(version) => {
                write!(f, "DocumentManifest schema {version} 해석 불가")
            }
            ManifestChunkError::Definition(e) => write!(f, "문서 정의: {e}"),
            ManifestChunkError::Decode(e) => write!(f, "DocumentManifest 디코딩: {e}"),
            ManifestChunkError::Encode(e) => write!(f, "DocumentManifest 인코딩: {e}"),
        }
    }
}

impl std::error::Error for ManifestChunkError {}

impl From<DecodeError> for ManifestChunkError {
    fn from(e: DecodeError) -> ManifestChunkError {
        ManifestChunkError::Decode(e)
    }
}

impl From<EncodeError> for ManifestChunkError {
    fn from(e: EncodeError) -> ManifestChunkError {
        ManifestChunkError::Encode(e)
    }
}

impl DocumentManifestChunk {
    /// View 항목이 정의의 View 순서와 1:1인지 검사합니다.
    pub fn validate(&self) -> Result<(), ManifestChunkError> {
        let views: Vec<StableId> = self
            .definition
            .pages
            .iter()
            .flat_map(|p| p.views.iter().map(|v| v.id))
            .collect();
        if views.len() != self.view_trees.len() {
            return Err(ManifestChunkError::ViewMismatch {
                index: views.len().min(self.view_trees.len()),
            });
        }
        for (index, (view, entry)) in views.iter().zip(&self.view_trees).enumerate() {
            if *view != entry.view {
                return Err(ManifestChunkError::ViewMismatch { index });
            }
        }
        Ok(())
    }

    /// payload bytes로 인코딩합니다.
    pub fn encode(&self) -> Result<Vec<u8>, ManifestChunkError> {
        self.validate()?;
        let mut enc = CanonicalEncoder::new();
        enc.put_bytes(&self.definition.encode())?;
        let count = u32::try_from(self.view_trees.len()).map_err(|_| {
            ManifestChunkError::Encode(EncodeError::LengthOverflow {
                len: self.view_trees.len(),
            })
        })?;
        enc.put_u32(count);
        for entry in &self.view_trees {
            enc.put_fixed_raw(&entry.view.to_bytes());
            enc.put_fixed_raw(&entry.chunk.to_bytes());
        }
        Ok(enc.into_bytes())
    }

    /// 이 payload를 가리키는 pointer의 schema. 기존 정의는 v1 bytes를 그대로 유지합니다.
    pub fn schema_version(&self) -> u32 {
        if self.definition.text_styles.is_empty() {
            1
        } else {
            2
        }
    }

    /// 제품 generation에서 읽을 때는 저장된 pointer version을 먼저 확인합니다.
    pub fn decode_for_schema(version: u32, bytes: &[u8]) -> Result<Self, ManifestChunkError> {
        if !matches!(version, 1 | 2) {
            return Err(ManifestChunkError::UnsupportedSchema(version));
        }
        let manifest = Self::decode(bytes)?;
        if version != manifest.schema_version() {
            return Err(ManifestChunkError::UnsupportedSchema(version));
        }
        Ok(manifest)
    }

    /// payload bytes에서 디코딩하고 검증합니다.
    pub fn decode(bytes: &[u8]) -> Result<DocumentManifestChunk, ManifestChunkError> {
        let mut dec = CanonicalDecoder::new(bytes);
        let definition = DocumentDefinition::decode(dec.take_bytes()?)
            .map_err(ManifestChunkError::Definition)?;
        let count = dec.take_u32()? as usize;
        let mut view_trees = Vec::with_capacity(count.min(1024));
        for _ in 0..count {
            let mut view = [0u8; 16];
            view.copy_from_slice(dec.take_raw(16)?);
            let mut chunk = [0u8; 16];
            chunk.copy_from_slice(dec.take_raw(16)?);
            view_trees.push(ViewTreeRef {
                view: StableId::from_bytes(view),
                chunk: ChunkId::from_bytes(chunk),
            });
        }
        dec.expect_end()?;
        let manifest = DocumentManifestChunk {
            definition,
            view_trees,
        };
        manifest.validate()?;
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_document::definitions::{PageDefinition, ViewDefinition};

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn sample() -> DocumentManifestChunk {
        let mut definition = DocumentDefinition::new(sid(1), "문서".to_string());
        let mut page = PageDefinition::new(sid(2), "페이지".to_string());
        page.add_view(ViewDefinition::new(sid(3), String::new(), sid(10)))
            .unwrap();
        page.add_view(ViewDefinition::new(sid(4), "뷰".into(), sid(20)))
            .unwrap();
        definition.add_page(page).unwrap();
        DocumentManifestChunk {
            definition,
            view_trees: vec![
                ViewTreeRef {
                    view: sid(3),
                    chunk: ChunkId::from_stable_id(sid(3)),
                },
                ViewTreeRef {
                    view: sid(4),
                    chunk: ChunkId::from_stable_id(sid(4)),
                },
            ],
        }
    }

    #[test]
    fn manifest_payload_round_trips() {
        let manifest = sample();
        let bytes = manifest.encode().unwrap();
        let decoded = DocumentManifestChunk::decode(&bytes).unwrap();
        assert_eq!(decoded, manifest);
        assert_eq!(decoded.encode().unwrap(), bytes);
    }

    #[test]
    fn view_entries_must_match_definition_order() {
        let mut manifest = sample();
        manifest.view_trees.swap(0, 1);
        assert_eq!(
            manifest.encode().unwrap_err(),
            ManifestChunkError::ViewMismatch { index: 0 }
        );
        let mut manifest = sample();
        manifest.view_trees.pop();
        assert!(matches!(
            manifest.encode().unwrap_err(),
            ManifestChunkError::ViewMismatch { .. }
        ));
    }

    #[test]
    fn malformed_payload_is_rejected() {
        let bytes = sample().encode().unwrap();
        for cut in [0, 3, 4, bytes.len() - 32, bytes.len() - 1] {
            assert!(
                DocumentManifestChunk::decode(&bytes[..cut]).is_err(),
                "절단 {cut}"
            );
        }
        let mut extended = bytes;
        extended.push(0);
        assert!(DocumentManifestChunk::decode(&extended).is_err());
    }
}
