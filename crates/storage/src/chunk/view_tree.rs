//! ViewTree chunk payload 스키마 (V.6.2).
//!
//! ViewTree chunk(`KIND_VIEW_TREE`)는 view 전체 노드를 직접 담지 않고,
//! **의미 있는 subtree root 목록**과 각 subtree 내용을 담은 **개별 subtree
//! chunk 참조**(ChunkId)를 담습니다. 지연 로딩(V.6.6)은 이 목록을 읽고 필요한
//! subtree chunk만 open합니다.
//!
//! payload 형식(canonical, schema version은 ChunkPointer가 보관):
//!
//! | 필드 | 형식 |
//! |---|---|
//! | view root node id | 16바이트 raw StableId |
//! | subtree count | `u32` LE |
//! | subtree 항목들 | root node id 16바이트 + subtree ChunkId 16바이트 |

use std::fmt;

use stellar_core_types::ids::StableId;

use crate::canonical::decode::{CanonicalDecoder, DecodeError};
use crate::canonical::encode::{CanonicalEncoder, EncodeError};

use super::id::ChunkId;

/// subtree 참조: subtree root 노드와 내용 chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubtreeRef {
    /// subtree root node의 StableId.
    pub root_node: StableId,
    /// subtree 내용을 담은 chunk의 논리 id.
    pub chunk: ChunkId,
}

/// ViewTree chunk payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewTreeChunk {
    /// 이 view의 root node StableId.
    pub view_root: StableId,
    /// 의미 있는 subtree root 목록(순서 의미 있음 — 자식 순서).
    pub subtrees: Vec<SubtreeRef>,
}

/// ViewTree payload 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewTreeError {
    /// subtree root node가 중복됩니다.
    DuplicateSubtreeRoot {
        /// 중복된 node id.
        root_node: StableId,
    },
    /// canonical 디코딩 실패.
    Decode(DecodeError),
    /// canonical 인코딩 실패.
    Encode(EncodeError),
}

impl fmt::Display for ViewTreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ViewTreeError::DuplicateSubtreeRoot { root_node } => {
                write!(f, "subtree root node가 중복됩니다: {root_node}")
            }
            ViewTreeError::Decode(e) => write!(f, "ViewTree 디코딩 실패: {e}"),
            ViewTreeError::Encode(e) => write!(f, "ViewTree 인코딩 실패: {e}"),
        }
    }
}

impl std::error::Error for ViewTreeError {}

impl From<DecodeError> for ViewTreeError {
    fn from(e: DecodeError) -> ViewTreeError {
        ViewTreeError::Decode(e)
    }
}

impl From<EncodeError> for ViewTreeError {
    fn from(e: EncodeError) -> ViewTreeError {
        ViewTreeError::Encode(e)
    }
}

impl ViewTreeChunk {
    /// 구조 불변식(중복 subtree root 금지)을 검증합니다.
    pub fn validate(&self) -> Result<(), ViewTreeError> {
        let mut seen = std::collections::BTreeSet::new();
        for subtree in &self.subtrees {
            if !seen.insert(subtree.root_node) {
                return Err(ViewTreeError::DuplicateSubtreeRoot {
                    root_node: subtree.root_node,
                });
            }
        }
        Ok(())
    }

    /// payload bytes로 인코딩합니다.
    pub fn encode(&self) -> Result<Vec<u8>, ViewTreeError> {
        self.validate()?;
        let mut enc = CanonicalEncoder::new();
        enc.put_fixed_raw(&self.view_root.to_bytes());
        let count = u32::try_from(self.subtrees.len()).map_err(|_| {
            ViewTreeError::Encode(EncodeError::LengthOverflow {
                len: self.subtrees.len(),
            })
        })?;
        enc.put_u32(count);
        for subtree in &self.subtrees {
            enc.put_fixed_raw(&subtree.root_node.to_bytes());
            enc.put_fixed_raw(&subtree.chunk.to_bytes());
        }
        Ok(enc.into_bytes())
    }

    /// payload bytes에서 디코딩하고 검증합니다.
    pub fn decode(bytes: &[u8]) -> Result<ViewTreeChunk, ViewTreeError> {
        let mut dec = CanonicalDecoder::new(bytes);
        let mut id_bytes = [0u8; 16];
        id_bytes.copy_from_slice(dec.take_raw(16)?);
        let view_root = StableId::from_bytes(id_bytes);

        let count = dec.take_u32()? as usize;
        let mut subtrees = Vec::with_capacity(count.min(1024));
        for _ in 0..count {
            let mut node_bytes = [0u8; 16];
            node_bytes.copy_from_slice(dec.take_raw(16)?);
            let mut chunk_bytes = [0u8; 16];
            chunk_bytes.copy_from_slice(dec.take_raw(16)?);
            subtrees.push(SubtreeRef {
                root_node: StableId::from_bytes(node_bytes),
                chunk: ChunkId::from_bytes(chunk_bytes),
            });
        }
        dec.expect_end()?;

        let tree = ViewTreeChunk {
            view_root,
            subtrees,
        };
        tree.validate()?;
        Ok(tree)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    fn sample() -> ViewTreeChunk {
        ViewTreeChunk {
            view_root: stable(100),
            subtrees: vec![
                SubtreeRef {
                    root_node: stable(200),
                    chunk: ChunkId::from_stable_id(stable(1_000)),
                },
                SubtreeRef {
                    root_node: stable(300),
                    chunk: ChunkId::from_stable_id(stable(2_000)),
                },
            ],
        }
    }

    #[test]
    fn two_subtree_fixture_round_trips() {
        // 검증 기준(V.6.2): 2개 subtree fixture round-trip.
        let tree = sample();
        let bytes = tree.encode().unwrap();
        let decoded = ViewTreeChunk::decode(&bytes).unwrap();
        assert_eq!(decoded, tree);
        assert_eq!(decoded.subtrees.len(), 2);
        // 재인코딩 byte-identical.
        assert_eq!(decoded.encode().unwrap(), bytes);
    }

    #[test]
    fn empty_subtree_list_round_trips() {
        let tree = ViewTreeChunk {
            view_root: stable(100),
            subtrees: vec![],
        };
        let bytes = tree.encode().unwrap();
        assert_eq!(ViewTreeChunk::decode(&bytes).unwrap(), tree);
        // 16(view root) + 4(count).
        assert_eq!(bytes.len(), 20);
    }

    #[test]
    fn subtree_order_is_preserved() {
        let mut tree = sample();
        tree.subtrees.reverse();
        let bytes = tree.encode().unwrap();
        let decoded = ViewTreeChunk::decode(&bytes).unwrap();
        assert_eq!(decoded.subtrees, tree.subtrees, "자식 순서 보존");
    }

    #[test]
    fn duplicate_subtree_root_is_rejected() {
        let mut tree = sample();
        tree.subtrees.push(tree.subtrees[0]);
        assert!(matches!(
            tree.encode().unwrap_err(),
            ViewTreeError::DuplicateSubtreeRoot { .. }
        ));
    }

    #[test]
    fn truncated_payload_is_rejected() {
        let bytes = sample().encode().unwrap();
        for cut in [0, 8, 16, 19, 20, 35, 51, bytes.len() - 1] {
            assert!(
                ViewTreeChunk::decode(&bytes[..cut]).is_err(),
                "절단 길이 {cut}에서 오류가 아님"
            );
        }
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let mut bytes = sample().encode().unwrap();
        bytes.push(0xff);
        assert!(ViewTreeChunk::decode(&bytes).is_err());
    }
}
