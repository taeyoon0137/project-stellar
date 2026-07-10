//! save transaction fault matrix (V.5.6, 테스트 전용).
//!
//! chunk write, fsync, manifest write, pointer replace의 **모든 지점**에 실패를
//! 자동 주입하고, 각 fixture에서 저장 불변식을 검증합니다:
//!
//! > 이전 세대 또는 새 세대 중 **정확히 하나만** 열립니다.
//!
//! 주입 지점:
//! - prepare 단계 직전 4곳(base check, chunk write, chunk verify, manifest write).
//! - chunk write·manifest write 내부 atomic write 단계 6곳씩(temp 생성, 기록,
//!   flush, fsync, rename, dir sync).
//! - pointer replace 내부 atomic write 단계 6곳.

#[cfg(test)]
mod tests {
    use crate::atomic_file::{WriteObserver, WriteStage};
    use crate::chunk::envelope::ChunkKindCode;
    use crate::chunk::id::ChunkId;
    use crate::chunk::planner::LogicalChunkState;
    use crate::chunk::pointer::ChunkPointer;
    use crate::chunk::store::{ChunkStore, WriteOutcome};
    use crate::generation::current::write_current_pointer;
    use crate::generation::loader::{write_manifest_file, LoadedGeneration};
    use crate::generation::manifest::GenerationManifest;
    use crate::generation::open::{open_project_storage, OpenOutcome};
    use crate::save::cancel::{run_save_observed, CancelToken};
    use crate::save::commit::SavedGenerationState;
    use crate::save::executor::{SaveObserver, SaveStage};
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
            let path = std::env::temp_dir().join(format!(
                "stellar-faultmatrix-test-{}-{n}",
                std::process::id()
            ));
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

    /// fault matrix의 주입 지점.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum FaultPoint {
        /// prepare 단계 직전.
        Before(SaveStage),
        /// chunk/manifest write 내부 atomic write 단계 직전.
        InFile(SaveStage, WriteStage),
        /// pointer replace 내부 atomic write 단계 직전.
        InPointer(WriteStage),
    }

    /// 전체 matrix를 나열합니다.
    fn all_fault_points() -> Vec<FaultPoint> {
        let mut points = Vec::new();
        for stage in [
            SaveStage::BaseCheck,
            SaveStage::ChunkWrite { index: 0 },
            SaveStage::ChunkVerify { index: 0 },
            SaveStage::ManifestWrite,
        ] {
            points.push(FaultPoint::Before(stage));
        }
        for op in [SaveStage::ChunkWrite { index: 0 }, SaveStage::ManifestWrite] {
            for stage in WriteStage::ALL {
                points.push(FaultPoint::InFile(op.clone(), stage));
            }
        }
        for stage in WriteStage::ALL {
            points.push(FaultPoint::InPointer(stage));
        }
        points
    }

    struct PrepareFault {
        point: FaultPoint,
    }
    impl SaveObserver for PrepareFault {
        fn before(&mut self, stage: &SaveStage) -> std::io::Result<()> {
            if self.point == FaultPoint::Before(stage.clone()) {
                return Err(std::io::Error::other("주입: before-stage"));
            }
            Ok(())
        }
        fn before_file_stage(&mut self, op: &SaveStage, stage: WriteStage) -> std::io::Result<()> {
            if self.point == FaultPoint::InFile(op.clone(), stage) {
                return Err(std::io::Error::other("주입: file-stage"));
            }
            Ok(())
        }
    }

    struct PointerFault {
        point: FaultPoint,
    }
    impl WriteObserver for PointerFault {
        fn before(&mut self, stage: WriteStage) -> std::io::Result<()> {
            if self.point == FaultPoint::InPointer(stage) {
                return Err(std::io::Error::other("주입: pointer-stage"));
            }
            Ok(())
        }
    }

    #[test]
    fn every_fault_point_leaves_exactly_one_openable_generation() {
        // 검증 기준(V.5.6): 모든 fixture에서 이전 세대 또는 새 세대 중 하나만 열림.
        let project_id = stable(42);
        for point in all_fault_points() {
            let dir = TempDir::new();
            let base = base_generation(&dir, project_id);
            let current = vec![LogicalChunkState::Dirty {
                chunk_id: base.manifest.chunk_pointers[0].chunk_id,
                kind: KIND,
                schema_version: 1,
                payload: b"modified".to_vec(),
            }];
            let plan =
                build_save_plan(Some(&base), project_id, 1, &current, vec![stable(7_000)]).unwrap();
            let initial = Some((1, base.manifest_hash));
            let mut memory = SavedGenerationState::new(initial);

            let mut prepare = PrepareFault {
                point: point.clone(),
            };
            let mut pointer = PointerFault {
                point: point.clone(),
            };
            let result = run_save_observed(
                &dir.path,
                &plan,
                &mut memory,
                &CancelToken::new(),
                &mut prepare,
                &mut pointer,
            );
            assert!(result.is_err(), "{point:?}에서 실패가 주입되어야 함");

            // 불변식 1: 실패한 저장은 in-memory saved generation을 바꾸지 않음.
            assert_eq!(memory.saved(), initial, "{point:?}");

            // 불변식 2: 프로젝트는 여전히 열리고, 열린 세대는 base(1) 또는 새(2)
            // 중 정확히 하나.
            let outcome = open_project_storage(&dir.path, project_id)
                .unwrap_or_else(|e| panic!("{point:?}: 열 수 없음 — {e}"));
            let OpenOutcome::Current(loaded) = outcome else {
                panic!("{point:?}: recovery 없이 current가 열려야 함");
            };
            let generation = loaded.manifest.generation;
            assert!(
                generation == 1 || generation == 2,
                "{point:?}: 열린 세대 {generation}"
            );

            // 불변식 3: pointer rename 이후 실패만 새 세대(2)로 열립니다.
            let expected_generation = match point {
                FaultPoint::InPointer(WriteStage::SyncParentDir) => 2,
                _ => 1,
            };
            assert_eq!(generation, expected_generation, "{point:?}");
        }
    }

    #[test]
    fn fault_free_save_opens_new_generation() {
        // 주입 없는 대조군: 새 세대가 열리고 메모리도 갱신됩니다.
        let project_id = stable(42);
        let dir = TempDir::new();
        let base = base_generation(&dir, project_id);
        let current = vec![LogicalChunkState::Dirty {
            chunk_id: base.manifest.chunk_pointers[0].chunk_id,
            kind: KIND,
            schema_version: 1,
            payload: b"modified".to_vec(),
        }];
        let plan =
            build_save_plan(Some(&base), project_id, 1, &current, vec![stable(7_000)]).unwrap();
        let mut memory = SavedGenerationState::new(Some((1, base.manifest_hash)));
        crate::save::cancel::run_save(&dir.path, &plan, &mut memory, &CancelToken::new()).unwrap();

        let outcome = open_project_storage(&dir.path, project_id).unwrap();
        let OpenOutcome::Current(loaded) = outcome else {
            panic!("current가 열려야 함");
        };
        assert_eq!(loaded.manifest.generation, 2);
        assert_eq!(memory.saved(), Some((2, loaded.manifest_hash)));
    }
}
