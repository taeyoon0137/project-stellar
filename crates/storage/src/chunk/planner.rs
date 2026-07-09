//! chunk write planner (V.3.3).
//!
//! 저장 시 **변경된 logical chunk만** 새 payload로 기록하고, 나머지는 base
//! generation의 pointer를 그대로 재사용합니다. 문서 계층이 dirty 여부를 알고
//! 있으므로 planner는 clean chunk의 payload를 다시 인코딩하지 않습니다.
//! dirty로 표시됐지만 내용이 같은 chunk(no-op 변경)는 hash 비교로 걸러 재사용합니다.

use std::collections::BTreeMap;
use std::fmt;

use crate::hash::ContentHash;

use super::envelope::ChunkKindCode;
use super::id::ChunkId;
use super::pointer::{validate_pointer_set, ChunkPointer, PointerSetError};

/// 저장 시점의 logical chunk 상태.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogicalChunkState {
    /// 변경 없음 — base generation pointer를 재사용합니다.
    Clean {
        /// 논리 id.
        chunk_id: ChunkId,
    },
    /// 변경됨(또는 신규) — payload를 새로 인코딩했습니다.
    Dirty {
        /// 논리 id.
        chunk_id: ChunkId,
        /// chunk kind.
        kind: ChunkKindCode,
        /// payload 스키마 version.
        schema_version: u32,
        /// canonical 인코딩된 payload.
        payload: Vec<u8>,
    },
}

impl LogicalChunkState {
    fn chunk_id(&self) -> ChunkId {
        match self {
            LogicalChunkState::Clean { chunk_id } => *chunk_id,
            LogicalChunkState::Dirty { chunk_id, .. } => *chunk_id,
        }
    }
}

/// 실제로 기록해야 하는 chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedWrite {
    /// 논리 id.
    pub chunk_id: ChunkId,
    /// chunk kind.
    pub kind: ChunkKindCode,
    /// payload 스키마 version.
    pub schema_version: u32,
    /// 기록할 payload.
    pub payload: Vec<u8>,
}

/// planner 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkWritePlan {
    /// 새 generation manifest의 pointer 목록(chunk_id 오름차순).
    pub pointers: Vec<ChunkPointer>,
    /// 실제로 기록해야 하는 chunk(새 hash가 된 것만).
    pub writes: Vec<PlannedWrite>,
}

/// planner 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// clean chunk의 base pointer가 없습니다.
    MissingBasePointer {
        /// 해당 논리 id.
        chunk_id: ChunkId,
    },
    /// 입력에 같은 ChunkId가 두 번 나타납니다.
    DuplicateInput {
        /// 중복 논리 id.
        chunk_id: ChunkId,
    },
    /// 결과 pointer 집합 검증 실패(V.3.2).
    InvalidPointerSet(PointerSetError),
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanError::MissingBasePointer { chunk_id } => {
                write!(f, "clean chunk의 base pointer가 없습니다: {chunk_id}")
            }
            PlanError::DuplicateInput { chunk_id } => {
                write!(f, "입력에 ChunkId가 중복됩니다: {chunk_id}")
            }
            PlanError::InvalidPointerSet(e) => write!(f, "pointer 집합 검증 실패: {e}"),
        }
    }
}

impl std::error::Error for PlanError {}

/// 새 generation의 chunk 기록 계획을 만듭니다.
///
/// - `base`: 이전 generation manifest의 pointer들.
/// - `current`: 저장 시점의 모든 logical chunk 상태. 여기 없는 base pointer는
///   새 generation에서 제거된 것으로 간주해 계획에 포함하지 않습니다.
///
/// Dirty chunk는 payload hash를 계산해 base와 같으면(no-op 변경) 재사용하고,
/// 다르면 [`PlannedWrite`]로 계획합니다. 결과 pointer 집합은 V.3.2 validator로
/// 검증하고 chunk_id 오름차순으로 정렬합니다.
pub fn plan_chunk_writes(
    base: &[ChunkPointer],
    current: &[LogicalChunkState],
) -> Result<ChunkWritePlan, PlanError> {
    let base_by_id: BTreeMap<ChunkId, &ChunkPointer> =
        base.iter().map(|p| (p.chunk_id, p)).collect();

    let mut seen: std::collections::BTreeSet<ChunkId> = std::collections::BTreeSet::new();
    let mut pointers: Vec<ChunkPointer> = Vec::new();
    let mut writes: Vec<PlannedWrite> = Vec::new();

    for state in current {
        let chunk_id = state.chunk_id();
        if !seen.insert(chunk_id) {
            return Err(PlanError::DuplicateInput { chunk_id });
        }
        match state {
            LogicalChunkState::Clean { chunk_id } => {
                let pointer = base_by_id
                    .get(chunk_id)
                    .ok_or(PlanError::MissingBasePointer {
                        chunk_id: *chunk_id,
                    })?;
                pointers.push(**pointer);
            }
            LogicalChunkState::Dirty {
                chunk_id,
                kind,
                schema_version,
                payload,
            } => {
                let hash = ContentHash::compute(payload);
                let unchanged = base_by_id
                    .get(chunk_id)
                    .is_some_and(|p| p.content_hash == hash);
                pointers.push(ChunkPointer {
                    chunk_id: *chunk_id,
                    content_hash: hash,
                    kind: *kind,
                    schema_version: *schema_version,
                });
                if !unchanged {
                    writes.push(PlannedWrite {
                        chunk_id: *chunk_id,
                        kind: *kind,
                        schema_version: *schema_version,
                        payload: payload.clone(),
                    });
                }
            }
        }
    }

    pointers.sort();
    validate_pointer_set(&pointers).map_err(PlanError::InvalidPointerSet)?;
    Ok(ChunkWritePlan { pointers, writes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::ids::StableId;

    fn id(ts: u64) -> ChunkId {
        ChunkId::from_stable_id(StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab))
    }

    const KIND: ChunkKindCode = ChunkKindCode(0x8000_0001);

    fn base_pointer(ts: u64, payload: &[u8]) -> ChunkPointer {
        ChunkPointer {
            chunk_id: id(ts),
            content_hash: ContentHash::compute(payload),
            kind: KIND,
            schema_version: 1,
        }
    }

    /// base 5개 chunk fixture.
    fn base_fixture() -> Vec<ChunkPointer> {
        (0..5u64)
            .map(|i| base_pointer(1_000 + i, format!("chunk-{i}").as_bytes()))
            .collect()
    }

    #[test]
    fn single_property_change_touches_exactly_one_chunk() {
        // 검증 기준(V.3.3): 단일 속성 변경 fixture에서 예상 chunk 하나만 hash 변경.
        let base = base_fixture();
        // chunk 2만 변경, 나머지는 clean.
        let current: Vec<LogicalChunkState> = (0..5u64)
            .map(|i| {
                if i == 2 {
                    LogicalChunkState::Dirty {
                        chunk_id: id(1_000 + i),
                        kind: KIND,
                        schema_version: 1,
                        payload: b"chunk-2-modified".to_vec(),
                    }
                } else {
                    LogicalChunkState::Clean {
                        chunk_id: id(1_000 + i),
                    }
                }
            })
            .collect();

        let plan = plan_chunk_writes(&base, &current).unwrap();
        assert_eq!(plan.writes.len(), 1, "정확히 chunk 하나만 기록");
        assert_eq!(plan.writes[0].chunk_id, id(1_002));

        // hash가 바뀐 pointer는 정확히 하나.
        let changed: Vec<&ChunkPointer> =
            plan.pointers.iter().filter(|p| !base.contains(p)).collect();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].chunk_id, id(1_002));
        assert_eq!(
            changed[0].content_hash,
            ContentHash::compute(b"chunk-2-modified")
        );
        // 나머지 4개 pointer는 base와 동일(재사용).
        assert_eq!(plan.pointers.len(), 5);
    }

    #[test]
    fn noop_dirty_chunk_is_reused_without_write() {
        // dirty로 표시됐지만 내용이 같으면 기록하지 않습니다.
        let base = base_fixture();
        let current = vec![LogicalChunkState::Dirty {
            chunk_id: id(1_000),
            kind: KIND,
            schema_version: 1,
            payload: b"chunk-0".to_vec(),
        }];
        let plan = plan_chunk_writes(&base, &current).unwrap();
        assert!(plan.writes.is_empty());
        assert_eq!(plan.pointers, vec![base[0]]);
    }

    #[test]
    fn new_chunk_is_written() {
        let base = base_fixture();
        let mut current: Vec<LogicalChunkState> = base
            .iter()
            .map(|p| LogicalChunkState::Clean {
                chunk_id: p.chunk_id,
            })
            .collect();
        current.push(LogicalChunkState::Dirty {
            chunk_id: id(9_000),
            kind: KIND,
            schema_version: 1,
            payload: b"brand new".to_vec(),
        });
        let plan = plan_chunk_writes(&base, &current).unwrap();
        assert_eq!(plan.writes.len(), 1);
        assert_eq!(plan.pointers.len(), 6);
    }

    #[test]
    fn removed_chunk_is_dropped_from_plan() {
        let base = base_fixture();
        // chunk 4를 목록에서 제외 → 새 generation에서 제거.
        let current: Vec<LogicalChunkState> = (0..4u64)
            .map(|i| LogicalChunkState::Clean {
                chunk_id: id(1_000 + i),
            })
            .collect();
        let plan = plan_chunk_writes(&base, &current).unwrap();
        assert!(plan.writes.is_empty());
        assert_eq!(plan.pointers.len(), 4);
        assert!(!plan.pointers.iter().any(|p| p.chunk_id == id(1_004)));
    }

    #[test]
    fn clean_chunk_without_base_pointer_is_error() {
        let plan = plan_chunk_writes(&[], &[LogicalChunkState::Clean { chunk_id: id(1) }]);
        assert_eq!(
            plan.unwrap_err(),
            PlanError::MissingBasePointer { chunk_id: id(1) }
        );
    }

    #[test]
    fn duplicate_input_is_error() {
        let base = base_fixture();
        let current = vec![
            LogicalChunkState::Clean {
                chunk_id: id(1_000),
            },
            LogicalChunkState::Clean {
                chunk_id: id(1_000),
            },
        ];
        assert_eq!(
            plan_chunk_writes(&base, &current).unwrap_err(),
            PlanError::DuplicateInput {
                chunk_id: id(1_000)
            }
        );
    }

    #[test]
    fn identical_state_produces_identical_plan() {
        // 같은 입력 → 같은 계획(결정론).
        let base = base_fixture();
        let current: Vec<LogicalChunkState> = base
            .iter()
            .map(|p| LogicalChunkState::Clean {
                chunk_id: p.chunk_id,
            })
            .collect();
        let a = plan_chunk_writes(&base, &current).unwrap();
        let b = plan_chunk_writes(&base, &current).unwrap();
        assert_eq!(a, b);
        assert!(a.writes.is_empty());
        assert_eq!(a.pointers, base);
    }
}
