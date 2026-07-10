//! `SavePlan` (V.5.1).
//!
//! 한 번의 저장이 수행할 일을 **실행 전에** 완전히 기술합니다: base generation,
//! target generation, 기록할 dirty chunk들, 새 manifest. 동일한 문서 상태에서는
//! 항상 동일한 SavePlan이 나옵니다(결정론).

use std::fmt;

use stellar_core_types::ids::StableId;

use crate::chunk::planner::{plan_chunk_writes, LogicalChunkState, PlanError, PlannedWrite};
use crate::generation::loader::LoadedGeneration;
use crate::generation::manifest::{GenerationManifest, ManifestError};
use crate::hash::ContentHash;

/// SavePlan 생성 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SavePlanError {
    /// chunk write 계획 실패(V.3.3).
    Plan(PlanError),
    /// 새 manifest 검증 실패(V.4.1).
    Manifest(ManifestError),
}

impl fmt::Display for SavePlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SavePlanError::Plan(e) => write!(f, "chunk 계획 실패: {e}"),
            SavePlanError::Manifest(e) => write!(f, "manifest 검증 실패: {e}"),
        }
    }
}

impl std::error::Error for SavePlanError {}

impl From<PlanError> for SavePlanError {
    fn from(e: PlanError) -> SavePlanError {
        SavePlanError::Plan(e)
    }
}

impl From<ManifestError> for SavePlanError {
    fn from(e: ManifestError) -> SavePlanError {
        SavePlanError::Manifest(e)
    }
}

/// 한 번의 저장 계획.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavePlan {
    /// base generation number. 첫 저장은 `None`.
    pub base_generation: Option<u64>,
    /// base generation manifest hash. 첫 저장은 `None`.
    /// optimistic check(V.5.2)가 current pointer와 비교하는 값입니다.
    pub base_manifest_hash: Option<ContentHash>,
    /// target generation number(base+1, 첫 저장은 1).
    pub target_generation: u64,
    /// 실제로 기록해야 하는 dirty chunk들.
    pub dirty_chunks: Vec<PlannedWrite>,
    /// 새 generation manifest(검증 완료).
    pub manifest: GenerationManifest,
}

/// 문서 상태로부터 SavePlan을 만듭니다.
///
/// - `base`: 열려 있는 base generation. 첫 저장은 `None`.
/// - `current_chunks`: 저장 시점의 모든 logical chunk 상태(V.3.3).
/// - `root_document_ids`: 새 세대의 root document 목록.
///
/// schema version은 base를 따르거나 첫 저장 인자로 받습니다.
pub fn build_save_plan(
    base: Option<&LoadedGeneration>,
    project_id: StableId,
    schema_version: u32,
    current_chunks: &[LogicalChunkState],
    root_document_ids: Vec<StableId>,
) -> Result<SavePlan, SavePlanError> {
    let base_pointers = base.map_or(&[][..], |b| &b.manifest.chunk_pointers);
    let chunk_plan = plan_chunk_writes(base_pointers, current_chunks)?;

    let base_generation = base.map(|b| b.manifest.generation);
    let target_generation = base_generation.map_or(1, |g| g + 1);

    let manifest = GenerationManifest {
        generation: target_generation,
        parent_generation: base_generation,
        project_id,
        schema_version,
        chunk_pointers: chunk_plan.pointers,
        root_document_ids,
    };
    manifest.validate()?;

    Ok(SavePlan {
        base_generation,
        base_manifest_hash: base.map(|b| b.manifest_hash),
        target_generation,
        dirty_chunks: chunk_plan.writes,
        manifest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::envelope::ChunkKindCode;
    use crate::chunk::id::ChunkId;
    use crate::chunk::pointer::ChunkPointer;

    const KIND: ChunkKindCode = ChunkKindCode(0x8000_0001);

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    fn chunk_id(ts: u64) -> ChunkId {
        ChunkId::from_stable_id(stable(ts))
    }

    fn base_generation() -> LoadedGeneration {
        let manifest = GenerationManifest {
            generation: 3,
            parent_generation: Some(2),
            project_id: stable(42),
            schema_version: 1,
            chunk_pointers: vec![
                ChunkPointer {
                    chunk_id: chunk_id(1_000),
                    content_hash: ContentHash::compute(b"chunk-0"),
                    kind: KIND,
                    schema_version: 1,
                },
                ChunkPointer {
                    chunk_id: chunk_id(1_001),
                    content_hash: ContentHash::compute(b"chunk-1"),
                    kind: KIND,
                    schema_version: 1,
                },
            ],
            root_document_ids: vec![stable(7_000)],
        };
        let (_, hash) = manifest.encode_with_hash().unwrap();
        LoadedGeneration {
            manifest_hash: hash,
            manifest,
        }
    }

    fn current_state() -> Vec<LogicalChunkState> {
        vec![
            LogicalChunkState::Clean {
                chunk_id: chunk_id(1_000),
            },
            LogicalChunkState::Dirty {
                chunk_id: chunk_id(1_001),
                kind: KIND,
                schema_version: 1,
                payload: b"chunk-1-modified".to_vec(),
            },
        ]
    }

    #[test]
    fn identical_document_state_yields_identical_plan() {
        // 검증 기준(V.5.1): 동일 문서 상태에서 동일 SavePlan.
        let base = base_generation();
        let a = build_save_plan(
            Some(&base),
            stable(42),
            1,
            &current_state(),
            vec![stable(7_000)],
        )
        .unwrap();
        let b = build_save_plan(
            Some(&base),
            stable(42),
            1,
            &current_state(),
            vec![stable(7_000)],
        )
        .unwrap();
        assert_eq!(a, b);
        // manifest bytes·hash까지 동일.
        assert_eq!(
            a.manifest.encode_with_hash().unwrap(),
            b.manifest.encode_with_hash().unwrap()
        );
    }

    #[test]
    fn plan_records_base_and_target_generation() {
        let base = base_generation();
        let plan = build_save_plan(
            Some(&base),
            stable(42),
            1,
            &current_state(),
            vec![stable(7_000)],
        )
        .unwrap();
        assert_eq!(plan.base_generation, Some(3));
        assert_eq!(plan.base_manifest_hash, Some(base.manifest_hash));
        assert_eq!(plan.target_generation, 4);
        assert_eq!(plan.manifest.parent_generation, Some(3));
        // dirty chunk는 변경된 하나뿐.
        assert_eq!(plan.dirty_chunks.len(), 1);
        assert_eq!(plan.dirty_chunks[0].chunk_id, chunk_id(1_001));
    }

    #[test]
    fn first_save_has_no_base() {
        let current = vec![LogicalChunkState::Dirty {
            chunk_id: chunk_id(1_000),
            kind: KIND,
            schema_version: 1,
            payload: b"first chunk".to_vec(),
        }];
        let plan = build_save_plan(None, stable(42), 1, &current, vec![stable(7_000)]).unwrap();
        assert_eq!(plan.base_generation, None);
        assert_eq!(plan.base_manifest_hash, None);
        assert_eq!(plan.target_generation, 1);
        assert_eq!(plan.manifest.parent_generation, None);
        assert_eq!(plan.dirty_chunks.len(), 1);
    }

    #[test]
    fn invalid_state_is_rejected() {
        // clean인데 base 없음 → 계획 실패.
        let current = vec![LogicalChunkState::Clean {
            chunk_id: chunk_id(1_000),
        }];
        let err = build_save_plan(None, stable(42), 1, &current, vec![]).unwrap_err();
        assert!(matches!(err, SavePlanError::Plan(_)));

        // root document 중복 → manifest 검증 실패.
        let base = base_generation();
        let err = build_save_plan(
            Some(&base),
            stable(42),
            1,
            &current_state(),
            vec![stable(7_000), stable(7_000)],
        )
        .unwrap_err();
        assert!(matches!(err, SavePlanError::Manifest(_)));
    }
}
