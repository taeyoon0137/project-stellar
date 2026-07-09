//! chunk remap manifest (V.3.4).
//!
//! chunk split/merge가 일어나면 기존 ChunkId가 새 ChunkId로 이동합니다. 외부
//! 참조(다른 문서·의존 프로젝트)가 옛 ChunkId로도 대상을 찾을 수 있도록
//! `from → to` 재매핑을 기록합니다. **실제 split/merge는 후속 작업**(V.6.4,
//! V.6.5)이고, 여기서는 타입과 검증만 제공합니다.
//!
//! 검증 규칙:
//! - `from == to`(자기 재매핑)와 중복 `from`은 등록 시점에 거부합니다.
//! - remap chain(`a → b → c`)은 허용하되 **cycle은 거부**합니다.
//! - chain의 최종 대상은 살아 있는 ChunkId 집합에 존재해야 합니다(missing target 거부).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use super::id::ChunkId;

/// remap 등록 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemapInsertError {
    /// 자기 자신으로의 재매핑입니다.
    SelfRemap {
        /// 해당 id.
        chunk_id: ChunkId,
    },
    /// 같은 source가 이미 등록되어 있습니다.
    DuplicateSource {
        /// 중복 source id.
        from: ChunkId,
    },
}

impl fmt::Display for RemapInsertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RemapInsertError::SelfRemap { chunk_id } => {
                write!(f, "자기 자신으로 재매핑할 수 없습니다: {chunk_id}")
            }
            RemapInsertError::DuplicateSource { from } => {
                write!(f, "재매핑 source가 중복됩니다: {from}")
            }
        }
    }
}

impl std::error::Error for RemapInsertError {}

/// remap manifest 검증 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemapValidateError {
    /// remap chain에 cycle이 있습니다.
    Cycle {
        /// cycle이 탐지된 시작 id.
        start: ChunkId,
    },
    /// chain의 최종 대상이 살아 있는 chunk 집합에 없습니다.
    MissingTarget {
        /// 재매핑 시작 id.
        from: ChunkId,
        /// 존재하지 않는 최종 대상.
        target: ChunkId,
    },
}

impl fmt::Display for RemapValidateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RemapValidateError::Cycle { start } => {
                write!(f, "remap chain에 cycle이 있습니다: {start}에서 시작")
            }
            RemapValidateError::MissingTarget { from, target } => {
                write!(f, "remap 대상이 존재하지 않습니다: {from} → ... → {target}")
            }
        }
    }
}

impl std::error::Error for RemapValidateError {}

/// chunk remap manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChunkRemapManifest {
    map: BTreeMap<ChunkId, ChunkId>,
}

impl ChunkRemapManifest {
    /// 빈 manifest를 만듭니다.
    pub fn new() -> ChunkRemapManifest {
        ChunkRemapManifest::default()
    }

    /// `from → to` 재매핑을 등록합니다.
    pub fn insert(&mut self, from: ChunkId, to: ChunkId) -> Result<(), RemapInsertError> {
        if from == to {
            return Err(RemapInsertError::SelfRemap { chunk_id: from });
        }
        if self.map.contains_key(&from) {
            return Err(RemapInsertError::DuplicateSource { from });
        }
        self.map.insert(from, to);
        Ok(())
    }

    /// 등록 수.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// 비어 있는지 여부.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// id를 최종 대상까지 따라갑니다. 재매핑이 없으면 원본을 그대로 반환합니다.
    ///
    /// cycle이 있으면 `None`을 반환합니다(검증 전 데이터 방어).
    pub fn resolve(&self, id: ChunkId) -> Option<ChunkId> {
        let mut visited: BTreeSet<ChunkId> = BTreeSet::new();
        let mut current = id;
        while let Some(next) = self.map.get(&current) {
            if !visited.insert(current) {
                return None;
            }
            current = *next;
        }
        Some(current)
    }

    /// manifest 전체를 검증합니다.
    ///
    /// - 모든 chain이 cycle 없이 끝나야 합니다.
    /// - 모든 chain의 최종 대상이 `live`(살아 있는 ChunkId 집합)에 있어야 합니다.
    pub fn validate(&self, live: &BTreeSet<ChunkId>) -> Result<(), RemapValidateError> {
        for from in self.map.keys() {
            let Some(target) = self.resolve(*from) else {
                return Err(RemapValidateError::Cycle { start: *from });
            };
            if !live.contains(&target) {
                return Err(RemapValidateError::MissingTarget {
                    from: *from,
                    target,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::ids::StableId;

    fn id(ts: u64) -> ChunkId {
        ChunkId::from_stable_id(StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab))
    }

    #[test]
    fn resolve_follows_chain_to_final_target() {
        // a → b → c, c는 살아 있음.
        let mut manifest = ChunkRemapManifest::new();
        manifest.insert(id(1), id(2)).unwrap();
        manifest.insert(id(2), id(3)).unwrap();
        assert_eq!(manifest.resolve(id(1)), Some(id(3)));
        assert_eq!(manifest.resolve(id(2)), Some(id(3)));
        // 재매핑 없는 id는 그대로.
        assert_eq!(manifest.resolve(id(9)), Some(id(9)));
    }

    #[test]
    fn valid_chain_passes_validation() {
        let mut manifest = ChunkRemapManifest::new();
        manifest.insert(id(1), id(2)).unwrap();
        manifest.insert(id(2), id(3)).unwrap();
        let live: BTreeSet<ChunkId> = [id(3), id(4)].into_iter().collect();
        assert_eq!(manifest.validate(&live), Ok(()));
    }

    #[test]
    fn cycle_is_rejected() {
        // 검증 기준(V.3.4): remap chain cycle 검증.
        let mut manifest = ChunkRemapManifest::new();
        manifest.insert(id(1), id(2)).unwrap();
        manifest.insert(id(2), id(3)).unwrap();
        manifest.insert(id(3), id(1)).unwrap();
        let live: BTreeSet<ChunkId> = [id(4)].into_iter().collect();
        let err = manifest.validate(&live).unwrap_err();
        assert!(matches!(err, RemapValidateError::Cycle { .. }));
        assert_eq!(manifest.resolve(id(1)), None);
    }

    #[test]
    fn missing_target_is_rejected() {
        // 검증 기준(V.3.4): missing target 검증.
        let mut manifest = ChunkRemapManifest::new();
        manifest.insert(id(1), id(2)).unwrap();
        let live: BTreeSet<ChunkId> = [id(9)].into_iter().collect();
        assert_eq!(
            manifest.validate(&live).unwrap_err(),
            RemapValidateError::MissingTarget {
                from: id(1),
                target: id(2),
            }
        );
    }

    #[test]
    fn self_remap_and_duplicate_source_are_rejected_at_insert() {
        let mut manifest = ChunkRemapManifest::new();
        assert_eq!(
            manifest.insert(id(1), id(1)).unwrap_err(),
            RemapInsertError::SelfRemap { chunk_id: id(1) }
        );
        manifest.insert(id(1), id(2)).unwrap();
        assert_eq!(
            manifest.insert(id(1), id(3)).unwrap_err(),
            RemapInsertError::DuplicateSource { from: id(1) }
        );
        assert_eq!(manifest.len(), 1);
    }

    #[test]
    fn empty_manifest_is_valid() {
        let manifest = ChunkRemapManifest::new();
        assert!(manifest.is_empty());
        assert_eq!(manifest.validate(&BTreeSet::new()), Ok(()));
    }
}
