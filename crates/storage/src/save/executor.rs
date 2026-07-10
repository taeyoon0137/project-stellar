//! 저장 단계 실행기 (V.5.3).
//!
//! SavePlan(V.5.1)을 고정된 단계 순서로 실행합니다:
//!
//! 1. optimistic base check(V.5.2) — 실패 시 아무 것도 쓰지 않습니다.
//! 2. dirty chunk 전부 기록(V.2.4 immutable write).
//! 3. 기록한 chunk를 **재읽기 전체 검증**(V.2.6).
//! 4. 새 manifest 파일 기록(V.4.4, content-addressed).
//!
//! current pointer 전환(V.5.4)은 [`commit`](super) 단계의 몫입니다. 여기까지의
//! 산출물은 전부 **새 파일 추가**뿐이므로, 어느 단계에서 실패해도 current
//! generation은 그대로 열립니다.

use std::fmt;
use std::path::Path;

use crate::chunk::store::{ChunkStore, ChunkStoreError};
use crate::generation::loader::LoadError;
use crate::hash::ContentHash;

use super::optimistic::{check_base_generation, BaseCheckError};
use super::plan::SavePlan;

/// 실행 단계.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveStage {
    /// optimistic base check.
    BaseCheck,
    /// index번째 dirty chunk 기록.
    ChunkWrite {
        /// dirty chunk index.
        index: usize,
    },
    /// index번째 dirty chunk 재읽기 검증.
    ChunkVerify {
        /// dirty chunk index.
        index: usize,
    },
    /// manifest 파일 기록.
    ManifestWrite,
}

impl fmt::Display for SaveStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveStage::BaseCheck => f.write_str("base-check"),
            SaveStage::ChunkWrite { index } => write!(f, "chunk-write[{index}]"),
            SaveStage::ChunkVerify { index } => write!(f, "chunk-verify[{index}]"),
            SaveStage::ManifestWrite => f.write_str("manifest-write"),
        }
    }
}

/// 각 단계 직전에 호출되는 관찰자. `Err`는 그 지점의 강제 중단을 모사합니다.
pub trait SaveObserver {
    /// `stage` 실행 직전 호출됩니다.
    fn before(&mut self, stage: &SaveStage) -> std::io::Result<()>;

    /// `op`(ChunkWrite/ManifestWrite)가 수행하는 atomic write의 내부 단계
    /// (temp 생성·기록·flush·fsync·rename·dir sync) 직전 호출됩니다(V.5.6
    /// fault matrix용). 기본 구현은 통과입니다.
    fn before_file_stage(
        &mut self,
        op: &SaveStage,
        stage: crate::atomic_file::WriteStage,
    ) -> std::io::Result<()> {
        let _ = (op, stage);
        Ok(())
    }
}

/// [`SaveObserver::before_file_stage`]를 atomic write 관찰자로 잇는 adapter.
struct FileStageAdapter<'a> {
    op: SaveStage,
    observer: &'a mut dyn SaveObserver,
}

impl crate::atomic_file::WriteObserver for FileStageAdapter<'_> {
    fn before(&mut self, stage: crate::atomic_file::WriteStage) -> std::io::Result<()> {
        self.observer.before_file_stage(&self.op, stage)
    }
}

/// 아무 것도 주입하지 않는 기본 관찰자.
#[derive(Debug, Default)]
pub struct NoopSaveObserver;

impl SaveObserver for NoopSaveObserver {
    fn before(&mut self, _stage: &SaveStage) -> std::io::Result<()> {
        Ok(())
    }
}

/// 실행기 오류.
#[derive(Debug)]
pub enum SaveExecError {
    /// optimistic check 실패(V.5.2) — 아무 것도 쓰지 않았습니다.
    BaseCheck(BaseCheckError),
    /// chunk 기록·재읽기 검증 실패.
    Chunk(ChunkStoreError),
    /// manifest 기록 실패.
    Manifest(LoadError),
    /// 주입된 중단(관찰자).
    Aborted {
        /// 중단된 단계.
        stage: SaveStage,
        /// 원인.
        source: std::io::Error,
    },
}

impl fmt::Display for SaveExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveExecError::BaseCheck(e) => write!(f, "base check 실패: {e}"),
            SaveExecError::Chunk(e) => write!(f, "chunk 단계 실패: {e}"),
            SaveExecError::Manifest(e) => write!(f, "manifest 기록 실패: {e}"),
            SaveExecError::Aborted { stage, source } => {
                write!(f, "{stage} 단계에서 중단: {source}")
            }
        }
    }
}

impl std::error::Error for SaveExecError {}

impl From<BaseCheckError> for SaveExecError {
    fn from(e: BaseCheckError) -> SaveExecError {
        SaveExecError::BaseCheck(e)
    }
}

impl From<ChunkStoreError> for SaveExecError {
    fn from(e: ChunkStoreError) -> SaveExecError {
        SaveExecError::Chunk(e)
    }
}

/// prepare 결과: pointer 전환(V.5.4)에 필요한 새 manifest hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedSave {
    /// 기록된 새 manifest의 content hash.
    pub manifest_hash: ContentHash,
}

/// 저장을 준비합니다: base check → dirty chunk 기록 → 재읽기 검증 → manifest 기록.
///
/// current pointer는 건드리지 않습니다. 실패 시 지금까지의 산출물(새 chunk·manifest
/// 파일)은 남을 수 있으나 어떤 것도 current generation에 연결되지 않습니다 —
/// current는 계속 이전 세대로 열립니다. orphan 정리는 GC(V.7) 소관입니다.
pub fn prepare_save(
    root: &Path,
    plan: &SavePlan,
    observer: &mut dyn SaveObserver,
) -> Result<PreparedSave, SaveExecError> {
    let abort = |stage: SaveStage, source: std::io::Error| SaveExecError::Aborted { stage, source };

    observer
        .before(&SaveStage::BaseCheck)
        .map_err(|e| abort(SaveStage::BaseCheck, e))?;
    check_base_generation(root, plan)?;

    let store = ChunkStore::new(root);
    for (index, write) in plan.dirty_chunks.iter().enumerate() {
        let stage = SaveStage::ChunkWrite { index };
        observer
            .before(&stage)
            .map_err(|e| abort(stage.clone(), e))?;
        let mut adapter = FileStageAdapter {
            op: stage,
            observer,
        };
        store.write_chunk_observed(write.kind, &write.payload, &mut adapter)?;
    }

    // 기록 직후 재읽기 전체 검증(V.2.6): envelope·length·hash.
    for (index, write) in plan.dirty_chunks.iter().enumerate() {
        let stage = SaveStage::ChunkVerify { index };
        observer.before(&stage).map_err(|e| abort(stage, e))?;
        let hash = ContentHash::compute(&write.payload);
        let read_back = store.read_chunk(&hash, write.kind)?;
        debug_assert_eq!(read_back, write.payload);
    }

    observer
        .before(&SaveStage::ManifestWrite)
        .map_err(|e| abort(SaveStage::ManifestWrite, e))?;
    let mut adapter = FileStageAdapter {
        op: SaveStage::ManifestWrite,
        observer,
    };
    let manifest_hash =
        crate::generation::loader::write_manifest_file_observed(root, &plan.manifest, &mut adapter)
            .map_err(SaveExecError::Manifest)?;

    Ok(PreparedSave { manifest_hash })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::envelope::ChunkKindCode;
    use crate::chunk::id::ChunkId;
    use crate::chunk::planner::LogicalChunkState;
    use crate::chunk::pointer::ChunkPointer;
    use crate::chunk::store::WriteOutcome;
    use crate::generation::current::{read_current_pointer, write_current_pointer};
    use crate::generation::loader::{
        load_current_generation, write_manifest_file, LoadedGeneration,
    };
    use crate::generation::manifest::GenerationManifest;
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
                .join(format!("stellar-executor-test-{}-{n}", std::process::id()));
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

    /// 실제 chunk 파일이 있는 base generation을 만들고 current로 전환합니다.
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

    /// base 위에서 chunk 하나를 수정하는 계획.
    fn modify_plan(base: &LoadedGeneration, project_id: StableId) -> SavePlan {
        let current = vec![LogicalChunkState::Dirty {
            chunk_id: base.manifest.chunk_pointers[0].chunk_id,
            kind: KIND,
            schema_version: 1,
            payload: b"modified-chunk".to_vec(),
        }];
        build_save_plan(Some(base), project_id, 1, &current, vec![stable(7_000)]).unwrap()
    }

    struct FailAt {
        stage: SaveStage,
    }
    impl SaveObserver for FailAt {
        fn before(&mut self, stage: &SaveStage) -> std::io::Result<()> {
            if *stage == self.stage {
                return Err(std::io::Error::other(format!("주입된 중단: {stage}")));
            }
            Ok(())
        }
    }

    #[test]
    fn successful_prepare_writes_chunks_and_manifest() {
        let dir = TempDir::new();
        let project_id = stable(42);
        let base = base_generation(&dir, project_id);
        let plan = modify_plan(&base, project_id);

        let prepared = prepare_save(&dir.path, &plan, &mut NoopSaveObserver).unwrap();
        let (_, expected_hash) = plan.manifest.encode_with_hash().unwrap();
        assert_eq!(prepared.manifest_hash, expected_hash);
        // current pointer는 아직 base(전환은 V.5.4).
        assert_eq!(read_current_pointer(&dir.path).unwrap(), base.manifest_hash);
    }

    #[test]
    fn failure_at_each_stage_keeps_current_generation_loadable() {
        // 검증 기준(V.5.3): 중간 실패 시 current generation 유지.
        let project_id = stable(42);
        let stages = [
            SaveStage::BaseCheck,
            SaveStage::ChunkWrite { index: 0 },
            SaveStage::ChunkVerify { index: 0 },
            SaveStage::ManifestWrite,
        ];
        for stage in stages {
            let dir = TempDir::new();
            let base = base_generation(&dir, project_id);
            let plan = modify_plan(&base, project_id);

            let err = prepare_save(
                &dir.path,
                &plan,
                &mut FailAt {
                    stage: stage.clone(),
                },
            )
            .unwrap_err();
            assert!(matches!(err, SaveExecError::Aborted { .. }), "{stage}");

            // current pointer 불변 + current generation이 여전히 완전하게 열림.
            assert_eq!(
                read_current_pointer(&dir.path).unwrap(),
                base.manifest_hash,
                "{stage} 실패 후 current pointer가 바뀌면 안 됨"
            );
            let loaded = load_current_generation(&dir.path, project_id).unwrap();
            assert_eq!(loaded.manifest.generation, 1, "{stage}");
        }
    }

    #[test]
    fn corrupted_written_chunk_is_caught_by_verify() {
        // 기록과 검증 사이에 chunk가 손상되면 재읽기 검증이 잡아냅니다.
        struct CorruptBetween {
            root: PathBuf,
            payload_hash: ContentHash,
        }
        impl SaveObserver for CorruptBetween {
            fn before(&mut self, stage: &SaveStage) -> std::io::Result<()> {
                if let SaveStage::ChunkVerify { .. } = stage {
                    let path = crate::chunk::path::ChunkObjectPath::for_hash(&self.payload_hash);
                    let file = self.root.join(path.to_relative_path());
                    let mut bytes = std::fs::read(&file)?;
                    let last = bytes.len() - 1;
                    bytes[last] ^= 0x01;
                    std::fs::write(&file, &bytes)?;
                }
                Ok(())
            }
        }

        let dir = TempDir::new();
        let project_id = stable(42);
        let base = base_generation(&dir, project_id);
        let plan = modify_plan(&base, project_id);
        let payload_hash = ContentHash::compute(&plan.dirty_chunks[0].payload);

        let err = prepare_save(
            &dir.path,
            &plan,
            &mut CorruptBetween {
                root: dir.path.clone(),
                payload_hash,
            },
        )
        .unwrap_err();
        assert!(matches!(err, SaveExecError::Chunk(_)), "{err:?}");
        // current는 여전히 base.
        let loaded = load_current_generation(&dir.path, project_id).unwrap();
        assert_eq!(loaded.manifest.generation, 1);
    }

    #[test]
    fn stale_base_writes_nothing() {
        let dir = TempDir::new();
        let project_id = stable(42);
        let base = base_generation(&dir, project_id);
        let plan = modify_plan(&base, project_id);
        // current를 다른 세대로 이동시켜 stale로 만듭니다.
        let mut other = base.manifest.clone();
        other.generation = 2;
        other.parent_generation = Some(1);
        let other_hash = write_manifest_file(&dir.path, &other).unwrap();
        write_current_pointer(&dir.path, &other_hash).unwrap();

        let err = prepare_save(&dir.path, &plan, &mut NoopSaveObserver).unwrap_err();
        assert!(matches!(err, SaveExecError::BaseCheck(_)));
        // 새 dirty chunk 파일이 생기지 않음.
        let new_chunk = crate::chunk::path::ChunkObjectPath::for_hash(&ContentHash::compute(
            &plan.dirty_chunks[0].payload,
        ));
        assert!(!dir.path.join(new_chunk.to_relative_path()).exists());
    }
}
