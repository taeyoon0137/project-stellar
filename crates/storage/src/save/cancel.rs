//! 저장 취소 token과 전체 저장 파이프라인 (V.5.5).
//!
//! 취소 정책:
//!
//! - prepare(V.5.3)의 **각 단계 직전**과 pointer 전환 **직전**에 취소를 확인합니다.
//! - pointer 전환(V.5.4)이 시작된 이후에는 취소를 **무시**하고 완료 또는 실패까지
//!   진행합니다 — 전환은 atomic replace 하나이므로 중간 취소가 의미상 불가능하고,
//!   "절반 전환" 상태를 만들지 않기 위함입니다.

use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::atomic_file::WriteObserver;

use super::commit::{commit_save_observed, CommitError, SavedGenerationState};
use super::executor::{prepare_save, SaveExecError, SaveObserver, SaveStage};
use super::plan::SavePlan;

/// 저장 취소 token. 복제해 다른 스레드에서 `cancel()` 할 수 있습니다.
#[derive(Debug, Clone, Default)]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
}

impl CancelToken {
    /// 새 token(미취소 상태).
    pub fn new() -> CancelToken {
        CancelToken::default()
    }

    /// 취소를 요청합니다.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Release);
    }

    /// 취소 여부.
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }
}

/// 취소가 확인된 지점.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CancelPoint {
    /// prepare 단계 직전.
    BeforeStage(SaveStage),
    /// pointer 전환 직전(마지막 취소 지점).
    BeforePointerSwitch,
}

impl fmt::Display for CancelPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CancelPoint::BeforeStage(stage) => write!(f, "{stage} 직전"),
            CancelPoint::BeforePointerSwitch => f.write_str("pointer 전환 직전"),
        }
    }
}

/// 전체 저장 파이프라인 오류.
#[derive(Debug)]
pub enum SaveRunError {
    /// 취소되었습니다. current generation은 그대로입니다.
    Cancelled {
        /// 취소가 확인된 지점.
        point: CancelPoint,
    },
    /// prepare 실패(V.5.3).
    Exec(SaveExecError),
    /// commit 실패(V.5.4). 메모리 상태는 갱신되지 않았습니다.
    Commit(CommitError),
}

impl fmt::Display for SaveRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveRunError::Cancelled { point } => write!(f, "저장이 취소되었습니다: {point}"),
            SaveRunError::Exec(e) => write!(f, "저장 준비 실패: {e}"),
            SaveRunError::Commit(e) => write!(f, "저장 commit 실패: {e}"),
        }
    }
}

impl std::error::Error for SaveRunError {}

/// prepare 관찰자에 취소 검사를 끼워 넣는 wrapper.
struct CancelCheck<'a> {
    token: &'a CancelToken,
    inner: &'a mut dyn SaveObserver,
    cancelled_at: Option<SaveStage>,
}

impl SaveObserver for CancelCheck<'_> {
    fn before(&mut self, stage: &SaveStage) -> std::io::Result<()> {
        if self.token.is_cancelled() {
            self.cancelled_at = Some(stage.clone());
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "저장 취소",
            ));
        }
        self.inner.before(stage)
    }

    fn before_file_stage(
        &mut self,
        op: &SaveStage,
        stage: crate::atomic_file::WriteStage,
    ) -> std::io::Result<()> {
        // 파일 내부 단계에서는 취소를 새로 확인하지 않고(원자 단위 유지)
        // 내부 관찰자의 주입만 전달합니다.
        self.inner.before_file_stage(op, stage)
    }
}

/// 저장 전체를 실행합니다: prepare(각 단계 취소 확인) → 취소 최종 확인 →
/// pointer 전환(취소 무시).
pub fn run_save(
    root: &Path,
    plan: &SavePlan,
    memory: &mut SavedGenerationState,
    cancel: &CancelToken,
) -> Result<(), SaveRunError> {
    let mut noop_prepare = super::executor::NoopSaveObserver;
    let mut noop_pointer = crate::atomic_file::NoopObserver;
    run_save_observed(
        root,
        plan,
        memory,
        cancel,
        &mut noop_prepare,
        &mut noop_pointer,
    )
}

/// [`run_save`]에 prepare·pointer 관찰자를 주입한 버전(fault/취소 fixture 테스트용).
pub fn run_save_observed(
    root: &Path,
    plan: &SavePlan,
    memory: &mut SavedGenerationState,
    cancel: &CancelToken,
    prepare_observer: &mut dyn SaveObserver,
    pointer_observer: &mut dyn WriteObserver,
) -> Result<(), SaveRunError> {
    let mut check = CancelCheck {
        token: cancel,
        inner: prepare_observer,
        cancelled_at: None,
    };
    let prepared = match prepare_save(root, plan, &mut check) {
        Ok(prepared) => prepared,
        Err(e) => {
            if let Some(stage) = check.cancelled_at {
                return Err(SaveRunError::Cancelled {
                    point: CancelPoint::BeforeStage(stage),
                });
            }
            return Err(SaveRunError::Exec(e));
        }
    };

    // pointer 전환 직전 마지막 취소 지점.
    if cancel.is_cancelled() {
        return Err(SaveRunError::Cancelled {
            point: CancelPoint::BeforePointerSwitch,
        });
    }

    // 이 지점부터 취소는 무시됩니다 — 완료 또는 실패까지 진행.
    commit_save_observed(root, plan, &prepared, memory, pointer_observer)
        .map_err(SaveRunError::Commit)
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
    use crate::save::executor::NoopSaveObserver;
    use crate::save::plan::build_save_plan;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicUsize;
    use stellar_core_types::ids::StableId;

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("stellar-cancel-test-{}-{n}", std::process::id()));
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

    fn modify_plan(base: &LoadedGeneration, project_id: StableId) -> crate::save::plan::SavePlan {
        let current = vec![LogicalChunkState::Dirty {
            chunk_id: base.manifest.chunk_pointers[0].chunk_id,
            kind: KIND,
            schema_version: 1,
            payload: b"modified".to_vec(),
        }];
        build_save_plan(Some(base), project_id, 1, &current, vec![stable(7_000)]).unwrap()
    }

    /// 지정 단계 직전에 취소를 거는 prepare 관찰자.
    struct CancelAt<'a> {
        stage: SaveStage,
        token: &'a CancelToken,
    }
    impl SaveObserver for CancelAt<'_> {
        fn before(&mut self, stage: &SaveStage) -> std::io::Result<()> {
            // 다음 단계 직전에 취소되도록 현재 단계에서 token만 세웁니다.
            if *stage == self.stage {
                self.token.cancel();
            }
            Ok(())
        }
    }

    #[test]
    fn cancel_before_each_prepare_stage_keeps_everything_unchanged() {
        // 검증 기준(V.5.5): 각 단계 cancel fixture 결과 정확 (prepare 구간).
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
            let initial = Some((1, base.manifest_hash));
            let mut memory = SavedGenerationState::new(initial);

            // 처음부터 취소된 token으로 시작하면 첫 단계에서 잡히므로,
            // 지정 단계 "직전" 취소를 위해 이전 단계에서 token을 세웁니다.
            let cancel = CancelToken::new();
            if stage == SaveStage::BaseCheck {
                cancel.cancel();
            }
            let mut inject = CancelAt {
                stage: prev_stage(&stage),
                token: &cancel,
            };
            let mut noop_pointer = crate::atomic_file::NoopObserver;
            let err = run_save_observed(
                &dir.path,
                &plan,
                &mut memory,
                &cancel,
                &mut inject,
                &mut noop_pointer,
            )
            .unwrap_err();

            let SaveRunError::Cancelled {
                point: CancelPoint::BeforeStage(at),
            } = err
            else {
                panic!("Cancelled여야 함: {stage}");
            };
            assert_eq!(at, stage);
            // current pointer·메모리 모두 불변.
            assert_eq!(read_current_pointer(&dir.path).unwrap(), base.manifest_hash);
            assert_eq!(memory.saved(), initial);
        }
    }

    /// stage 바로 이전 단계(BaseCheck는 자신 — 시작 전 취소로 처리).
    fn prev_stage(stage: &SaveStage) -> SaveStage {
        match stage {
            SaveStage::BaseCheck => SaveStage::BaseCheck,
            SaveStage::ChunkWrite { .. } => SaveStage::BaseCheck,
            SaveStage::ChunkVerify { .. } => SaveStage::ChunkWrite { index: 0 },
            SaveStage::ManifestWrite => SaveStage::ChunkVerify { index: 0 },
        }
    }

    #[test]
    fn cancel_before_pointer_switch_keeps_current() {
        let dir = TempDir::new();
        let project_id = stable(42);
        let base = base_generation(&dir, project_id);
        let plan = modify_plan(&base, project_id);
        let initial = Some((1, base.manifest_hash));
        let mut memory = SavedGenerationState::new(initial);

        // ManifestWrite 단계에서 token을 세우면 prepare는 끝나고
        // pointer 전환 직전 확인에서 잡힙니다.
        let cancel = CancelToken::new();
        let mut inject = CancelAt {
            stage: SaveStage::ManifestWrite,
            token: &cancel,
        };
        let mut noop_pointer = crate::atomic_file::NoopObserver;
        let err = run_save_observed(
            &dir.path,
            &plan,
            &mut memory,
            &cancel,
            &mut inject,
            &mut noop_pointer,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            SaveRunError::Cancelled {
                point: CancelPoint::BeforePointerSwitch
            }
        ));
        assert_eq!(read_current_pointer(&dir.path).unwrap(), base.manifest_hash);
        assert_eq!(memory.saved(), initial);
    }

    #[test]
    fn cancel_after_pointer_switch_started_is_ignored() {
        // 검증 기준(V.5.5): pointer 전환 시작 이후 취소는 무시하고 완료까지 진행.
        struct CancelDuringPointerWrite<'a> {
            token: &'a CancelToken,
        }
        impl WriteObserver for CancelDuringPointerWrite<'_> {
            fn before(&mut self, stage: WriteStage) -> std::io::Result<()> {
                if stage == WriteStage::CreateTemp {
                    // 전환이 시작된 직후 취소 요청.
                    self.token.cancel();
                }
                Ok(())
            }
        }

        let dir = TempDir::new();
        let project_id = stable(42);
        let base = base_generation(&dir, project_id);
        let plan = modify_plan(&base, project_id);
        let mut memory = SavedGenerationState::new(Some((1, base.manifest_hash)));

        let cancel = CancelToken::new();
        let mut noop_prepare = NoopSaveObserver;
        let mut pointer = CancelDuringPointerWrite { token: &cancel };
        run_save_observed(
            &dir.path,
            &plan,
            &mut memory,
            &cancel,
            &mut noop_prepare,
            &mut pointer,
        )
        .unwrap();

        // 취소가 요청됐지만 전환은 완료되었습니다.
        assert!(cancel.is_cancelled());
        let (generation, hash) = memory.saved().unwrap();
        assert_eq!(generation, 2);
        assert_eq!(read_current_pointer(&dir.path).unwrap(), hash);
    }

    #[test]
    fn uncancelled_save_completes() {
        let dir = TempDir::new();
        let project_id = stable(42);
        let base = base_generation(&dir, project_id);
        let plan = modify_plan(&base, project_id);
        let mut memory = SavedGenerationState::new(Some((1, base.manifest_hash)));
        run_save(&dir.path, &plan, &mut memory, &CancelToken::new()).unwrap();
        assert_eq!(memory.saved().unwrap().0, 2);
    }
}
