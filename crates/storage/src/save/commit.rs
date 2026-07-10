//! 저장 commit: current pointer 전환 (V.5.4).
//!
//! prepare(V.5.3)가 끝난 뒤 **마지막 단계**로 current pointer를 새 manifest
//! hash로 전환합니다. in-memory saved generation은 pointer 전환이 **성공한
//! 뒤에만** 갱신합니다 — 전환 실패 시 메모리 상태는 base generation을 그대로
//! 유지합니다.

use std::fmt;
use std::path::Path;

use crate::atomic_file::{NoopObserver, WriteObserver};
use crate::generation::current::{write_current_pointer_observed, CurrentPointerError};
use crate::hash::ContentHash;

use super::executor::PreparedSave;
use super::plan::SavePlan;

/// in-memory saved generation 상태.
///
/// 마지막으로 **디스크 전환까지 성공한** 세대를 추적합니다. 문서 dirty 판정과
/// 다음 SavePlan의 base가 이 값을 사용합니다.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SavedGenerationState {
    saved: Option<(u64, ContentHash)>,
}

impl SavedGenerationState {
    /// 열린 세대로 초기화합니다(저장 이력이 없으면 `None`).
    pub fn new(saved: Option<(u64, ContentHash)>) -> SavedGenerationState {
        SavedGenerationState { saved }
    }

    /// 마지막으로 저장 성공한 (generation number, manifest hash).
    pub fn saved(&self) -> Option<(u64, ContentHash)> {
        self.saved
    }
}

/// commit 실패.
#[derive(Debug)]
pub enum CommitError {
    /// current pointer 전환 실패. 메모리 상태는 갱신되지 않았습니다.
    PointerSwitch(CurrentPointerError),
}

impl fmt::Display for CommitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommitError::PointerSwitch(e) => write!(f, "current pointer 전환 실패: {e}"),
        }
    }
}

impl std::error::Error for CommitError {}

/// current pointer를 새 manifest로 전환하고, 성공 시에만 메모리 상태를 갱신합니다.
pub fn commit_save(
    root: &Path,
    plan: &SavePlan,
    prepared: &PreparedSave,
    memory: &mut SavedGenerationState,
) -> Result<(), CommitError> {
    commit_save_observed(root, plan, prepared, memory, &mut NoopObserver)
}

/// [`commit_save`]에 pointer 쓰기 단계 관찰자를 주입한 버전(fault injection 테스트용).
pub fn commit_save_observed(
    root: &Path,
    plan: &SavePlan,
    prepared: &PreparedSave,
    memory: &mut SavedGenerationState,
    observer: &mut dyn WriteObserver,
) -> Result<(), CommitError> {
    write_current_pointer_observed(root, &prepared.manifest_hash, observer)
        .map_err(CommitError::PointerSwitch)?;
    // pointer 전환 성공 후에만 in-memory saved generation 갱신.
    memory.saved = Some((plan.target_generation, prepared.manifest_hash));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atomic_file::WriteStage;
    use crate::chunk::envelope::ChunkKindCode;
    use crate::chunk::id::ChunkId;
    use crate::chunk::planner::LogicalChunkState;
    use crate::chunk::pointer::ChunkPointer;
    use crate::chunk::store::{ChunkStore, WriteOutcome};
    use crate::generation::current::{read_current_pointer, write_current_pointer};
    use crate::generation::loader::{write_manifest_file, LoadedGeneration};
    use crate::generation::manifest::GenerationManifest;
    use crate::save::executor::{prepare_save, NoopSaveObserver};
    use crate::save::plan::build_save_plan;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use stellar_core_types::ids::StableId;

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("stellar-commit-test-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    const KIND: ChunkKindCode = ChunkKindCode(0x8000_0001);

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    fn base_generation(dir: &TempDir, project_id: StableId) -> LoadedGeneration {
        let store = ChunkStore::new(&dir.path);
        let WriteOutcome::Written { hash, .. } = store.write_chunk(KIND, b"base-chunk").unwrap()
        else {
            panic!("Written이어야 함");
        };
        let manifest = GenerationManifest {
            generation: 1,
            parent_generation: None,
            project_id,
            schema_version: 1,
            chunk_pointers: vec![ChunkPointer {
                chunk_id: ChunkId::from_stable_id(stable(1_000)),
                content_hash: hash,
                kind: KIND,
                schema_version: 1,
            }],
            root_document_ids: vec![stable(7_000)],
        };
        let manifest_hash = write_manifest_file(&dir.path, &manifest).unwrap();
        write_current_pointer(&dir.path, &manifest_hash).unwrap();
        LoadedGeneration {
            manifest_hash,
            manifest,
        }
    }

    fn prepared_plan(
        dir: &TempDir,
        base: &LoadedGeneration,
        project_id: StableId,
    ) -> (crate::save::plan::SavePlan, PreparedSave) {
        let current = vec![LogicalChunkState::Dirty {
            chunk_id: base.manifest.chunk_pointers[0].chunk_id,
            kind: KIND,
            schema_version: 1,
            payload: b"modified".to_vec(),
        }];
        let plan =
            build_save_plan(Some(base), project_id, 1, &current, vec![stable(7_000)]).unwrap();
        let prepared = prepare_save(&dir.path, &plan, &mut NoopSaveObserver).unwrap();
        (plan, prepared)
    }

    struct FailAt {
        stage: WriteStage,
    }
    impl WriteObserver for FailAt {
        fn before(&mut self, stage: WriteStage) -> std::io::Result<()> {
            if stage == self.stage {
                return Err(std::io::Error::other(format!("주입된 중단: {stage}")));
            }
            Ok(())
        }
    }

    #[test]
    fn successful_commit_switches_pointer_and_updates_memory() {
        let dir = TempDir::new();
        let project_id = stable(42);
        let base = base_generation(&dir, project_id);
        let (plan, prepared) = prepared_plan(&dir, &base, project_id);

        let mut memory = SavedGenerationState::new(Some((1, base.manifest_hash)));
        commit_save(&dir.path, &plan, &prepared, &mut memory).unwrap();

        assert_eq!(
            read_current_pointer(&dir.path).unwrap(),
            prepared.manifest_hash
        );
        assert_eq!(memory.saved(), Some((2, prepared.manifest_hash)));
    }

    #[test]
    fn pointer_switch_failure_leaves_memory_unchanged() {
        // 검증 기준(V.5.4): pointer 전환 실패 시 memory generation 미변경.
        let dir = TempDir::new();
        let project_id = stable(42);
        let base = base_generation(&dir, project_id);
        let (plan, prepared) = prepared_plan(&dir, &base, project_id);

        let initial = Some((1, base.manifest_hash));
        for stage in WriteStage::ALL {
            if stage == WriteStage::SyncParentDir {
                continue; // rename 이후 — 전환은 이미 완료된 것으로 간주(아래 별도).
            }
            let mut memory = SavedGenerationState::new(initial);
            let err = commit_save_observed(
                &dir.path,
                &plan,
                &prepared,
                &mut memory,
                &mut FailAt { stage },
            )
            .unwrap_err();
            assert!(matches!(err, CommitError::PointerSwitch(_)), "{stage}");
            // 메모리 상태 미변경 + 디스크 pointer도 base 유지.
            assert_eq!(memory.saved(), initial, "stage {stage}");
            assert_eq!(
                read_current_pointer(&dir.path).unwrap(),
                base.manifest_hash,
                "stage {stage}"
            );
        }
    }

    #[test]
    fn first_save_commit_from_empty_memory() {
        let dir = TempDir::new();
        let project_id = stable(42);
        let current = vec![LogicalChunkState::Dirty {
            chunk_id: ChunkId::from_stable_id(stable(1_000)),
            kind: KIND,
            schema_version: 1,
            payload: b"first".to_vec(),
        }];
        let plan = build_save_plan(None, project_id, 1, &current, vec![stable(7_000)]).unwrap();
        let prepared = prepare_save(&dir.path, &plan, &mut NoopSaveObserver).unwrap();

        let mut memory = SavedGenerationState::default();
        assert_eq!(memory.saved(), None);
        commit_save(&dir.path, &plan, &prepared, &mut memory).unwrap();
        assert_eq!(memory.saved(), Some((1, prepared.manifest_hash)));
    }
}
