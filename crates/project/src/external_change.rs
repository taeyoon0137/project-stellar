//! 외부 변경 반영 상태 (XIII.3).
//!
//! 기술 아키텍처 22.5: 로컬 변경이 있는 창에 외부의 새 generation이 오면 `LoadedBase / CurrentMemory /
//! ExternalDisk` 세 snapshot을 비교해 reconciliation해야 합니다. 이 모듈은 그 비교 입력을 **보존**하는
//! 상태를 정의합니다(XIII.3.2).
//!
//! - [`DiskSnapshot`]: 디스크의 완성된 generation(V.4.4 검증 통과)과 manifest hash. generation 파일은
//!   content-addressed·불변이므로 hash와 manifest로 보존되며, GC(V.7)가 지우지 않도록
//!   [`ExternalChangePending::retained_manifests`]를 보존 목록에 넣습니다.
//! - [`MemorySnapshot`]: 창의 현재 메모리 문서 상태를 저장소 관점(logical chunk 상태)으로 캡처한
//!   불변 snapshot. hash는 이 상태를 base 위에 저장하면 만들어질 manifest의 content hash입니다(같은
//!   상태 → 같은 hash).
//! - [`ExternalChangePending`]: 세 snapshot을 함께 보존한 대기 상태. 세 hash는
//!   [`ExternalChangePending::hashes`]로 기록합니다.
//!
//! 대기 중 자동 저장 중지(XIII.3.3): Semantic Reconciliation(XIII.3.4) 전에는 대기 상태에서
//! [`WindowExternalState::autosave_gate`]가 `Suspended`이고, 저장 경로
//! ([`WindowExternalState::run_guarded_save`])는 아무 파일도 쓰지 않고 거부합니다. 셸에는 해결 선택지가
//! 없는 읽기 전용 alert([`ExternalChangeAlert`])만 한 번 돌려줍니다. 어느 쪽 버전으로도 자동 overwrite하지
//! 않으므로 대기 중 current pointer는 바뀌지 않습니다.
//!
//! 로컬 변경이 없는 창의 자동 reload(XIII.3.1)는 [`crate::window_session`]이 이 상태 위에서 실행하고
//! ([`WindowExternalState::adopt_reloaded`]), reconciliation(XIII.3.4)은 이후 연결합니다.
//!
//! agent lock(XIII.3.5): 잠금된 agent subtree와 외부 변경의 충돌 정책은 로드맵 Phase R3에서 연결합니다.
//! 초기 릴리스에는 agent lock 상태·feature가 **없으며**, 대기 상태는 위 세 snapshot만 보존하고 외부 변경
//! 판단은 agent 잠금을 참고하지 않습니다(테스트가 상태 구조와 crate 소스·feature 목록으로 확인).

/// 텍스트 원본 프로젝트의 열기·저장·외부 변경 반영과 깨진 원본 진단 (V.10.6·UX.4.4).
pub mod source;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use stellar_core_types::ids::StableId;
use stellar_storage::chunk::planner::LogicalChunkState;
use stellar_storage::generation::loader::{verify_generation, LoadError, LoadedGeneration};
use stellar_storage::generation::manifest::GenerationManifest;
use stellar_storage::hash::ContentHash;
use stellar_storage::save::cancel::{run_save, CancelToken, SaveRunError};
use stellar_storage::save::commit::SavedGenerationState;
use stellar_storage::save::plan::{build_save_plan, SavePlan, SavePlanError};

use crate::document_io::serialize::SerializeError;

/// 디스크의 완성된 generation snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskSnapshot {
    /// manifest content hash(current pointer 값).
    pub manifest_hash: ContentHash,
    /// 검증된 manifest.
    pub manifest: GenerationManifest,
}

impl DiskSnapshot {
    /// hash의 generation을 검증해 snapshot으로 만듭니다(manifest·project id·chunk 존재).
    pub fn load(
        root: &Path,
        manifest_hash: &ContentHash,
        project_id: StableId,
    ) -> Result<DiskSnapshot, LoadError> {
        Ok(DiskSnapshot::from(verify_generation(
            root,
            manifest_hash,
            project_id,
        )?))
    }

    /// generation number.
    pub fn generation(&self) -> u64 {
        self.manifest.generation
    }
}

impl From<LoadedGeneration> for DiskSnapshot {
    fn from(loaded: LoadedGeneration) -> DiskSnapshot {
        DiskSnapshot {
            manifest_hash: loaded.manifest_hash,
            manifest: loaded.manifest,
        }
    }
}

/// 창의 메모리 문서 상태 snapshot(불변).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySnapshot {
    hash: ContentHash,
    schema_version: u32,
    chunks: Arc<[LogicalChunkState]>,
    root_document_ids: Arc<[StableId]>,
}

impl MemorySnapshot {
    /// base generation 위의 메모리 상태(logical chunk 상태 전체)를 캡처합니다.
    ///
    /// hash는 이 상태로 만든 SavePlan(V.5.1)의 manifest content hash입니다. 저장 계획과 같은 규칙을
    /// 쓰므로 같은 상태는 항상 같은 hash가 됩니다.
    pub fn capture(
        base: &DiskSnapshot,
        chunks: Vec<LogicalChunkState>,
        root_document_ids: Vec<StableId>,
    ) -> Result<MemorySnapshot, SnapshotError> {
        let loaded = LoadedGeneration {
            manifest_hash: base.manifest_hash,
            manifest: base.manifest.clone(),
        };
        let plan = build_save_plan(
            Some(&loaded),
            base.manifest.project_id,
            base.manifest.schema_version,
            &chunks,
            root_document_ids.clone(),
        )
        .map_err(SnapshotError::Plan)?;
        let (_, hash) = plan
            .manifest
            .encode_with_hash()
            .map_err(|e| SnapshotError::Plan(SavePlanError::Manifest(e)))?;
        Ok(MemorySnapshot {
            hash,
            schema_version: base.manifest.schema_version,
            chunks: chunks.into(),
            root_document_ids: root_document_ids.into(),
        })
    }

    /// snapshot hash.
    pub fn hash(&self) -> ContentHash {
        self.hash
    }

    /// 문서 schema version.
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// logical chunk 상태(불변 공유).
    pub fn chunks(&self) -> &[LogicalChunkState] {
        &self.chunks
    }

    /// root document 목록.
    pub fn root_document_ids(&self) -> &[StableId] {
        &self.root_document_ids
    }
}

/// snapshot 생성 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    /// 메모리 상태로 저장 계획을 만들 수 없음.
    Plan(SavePlanError),
    /// 메모리 문서를 직렬화할 수 없음(편집 중 불변식 위반 등).
    Serialize(SerializeError),
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapshotError::Plan(e) => write!(f, "메모리 snapshot을 만들 수 없습니다: {e}"),
            SnapshotError::Serialize(e) => {
                write!(f, "메모리 문서를 직렬화할 수 없습니다: {e}")
            }
        }
    }
}

impl std::error::Error for SnapshotError {}

/// 세 snapshot의 hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotHashes {
    /// 창이 연(또는 마지막으로 저장한) generation.
    pub loaded_base: ContentHash,
    /// 창의 메모리 상태.
    pub current_memory: ContentHash,
    /// 외부 writer가 만든 새 generation.
    pub external_disk: ContentHash,
}

/// 외부 변경 대기 상태: 세 snapshot을 보존합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalChangePending {
    /// 창의 base generation.
    pub loaded_base: DiskSnapshot,
    /// 창의 메모리 상태.
    pub current_memory: MemorySnapshot,
    /// 외부의 새 generation.
    pub external_disk: DiskSnapshot,
}

impl ExternalChangePending {
    /// 세 snapshot hash.
    pub fn hashes(&self) -> SnapshotHashes {
        SnapshotHashes {
            loaded_base: self.loaded_base.manifest_hash,
            current_memory: self.current_memory.hash(),
            external_disk: self.external_disk.manifest_hash,
        }
    }

    /// GC가 지우면 안 되는 디스크 generation(base, external).
    pub fn retained_manifests(&self) -> [ContentHash; 2] {
        [
            self.loaded_base.manifest_hash,
            self.external_disk.manifest_hash,
        ]
    }
}

/// 창의 디스크 동기 상태.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowDiskState {
    /// 로컬 미저장 변경 없음.
    Clean {
        /// base generation.
        base: DiskSnapshot,
    },
    /// 로컬 미저장 변경 있음.
    Dirty {
        /// base generation.
        base: DiskSnapshot,
    },
    /// 외부 변경 대기(세 snapshot 보존).
    ExternalChangePending(Box<ExternalChangePending>),
}

/// 외부 generation을 받은 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalChangeDecision {
    /// 로컬 변경이 없음: 새 generation으로 reload할 수 있음(실행은 XIII.3.1
    /// [`crate::window_session::WindowDocumentSession::on_external_generation`]).
    CleanWindow {
        /// 외부의 새 generation.
        external: DiskSnapshot,
    },
    /// 로컬 변경이 있어 세 snapshot을 보존한 대기 상태가 됨(또는 대기 중인 외부 generation 갱신).
    Pending(SnapshotHashes),
    /// 창의 base와 같은 generation(변화 없음).
    Unchanged,
}

/// 외부 변경 처리 실패.
#[derive(Debug)]
pub enum ExternalChangeError {
    /// 외부 generation 검증 실패(미완성·손상 — 상태를 바꾸지 않음).
    Load(LoadError),
    /// 메모리 snapshot 캡처 실패(상태를 바꾸지 않음).
    Snapshot(SnapshotError),
}

impl std::fmt::Display for ExternalChangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExternalChangeError::Load(e) => write!(f, "외부 generation을 열 수 없습니다: {e}"),
            ExternalChangeError::Snapshot(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ExternalChangeError {}

/// 창 하나의 외부 변경 상태 기계.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowExternalState {
    root: PathBuf,
    project_id: StableId,
    state: WindowDiskState,
    /// 아직 셸에 전달하지 않은 alert가 있는지.
    alert_due: bool,
}

/// 자동 저장 허용 여부.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutosaveGate {
    /// 자동 저장 가능.
    Allowed,
    /// 외부 변경 대기 중이라 자동 저장 중지.
    Suspended,
}

/// 외부 변경 대기 alert. 해결 선택지가 없는 읽기 전용 알림입니다(reconciliation은 XIII.3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExternalChangeAlert {
    /// 보존된 세 snapshot hash.
    pub hashes: SnapshotHashes,
    /// 읽기 전용 알림(항상 `true`): 셸은 저장·덮어쓰기 선택지를 제공하지 않습니다.
    pub read_only: bool,
}

/// 저장 거부·실패.
#[derive(Debug)]
pub enum GuardedSaveError {
    /// 외부 변경 대기 중이라 저장하지 않음(파일을 쓰지 않음).
    ExternalChangePending(SnapshotHashes),
    /// 저장 파이프라인 실패(V.5).
    Save(SaveRunError),
}

impl std::fmt::Display for GuardedSaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GuardedSaveError::ExternalChangePending(_) => {
                f.write_str("외부 변경 대기 중에는 저장하지 않습니다(자동 덮어쓰기 금지)")
            }
            GuardedSaveError::Save(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for GuardedSaveError {}

impl WindowExternalState {
    /// 연 generation을 base로 clean 상태에서 시작합니다.
    pub fn open(root: &Path, base: DiskSnapshot) -> WindowExternalState {
        WindowExternalState {
            root: root.to_path_buf(),
            project_id: base.manifest.project_id,
            state: WindowDiskState::Clean { base },
            alert_due: false,
        }
    }

    /// 현재 상태.
    pub fn state(&self) -> &WindowDiskState {
        &self.state
    }

    /// 대기 상태이면 그 내용.
    pub fn pending(&self) -> Option<&ExternalChangePending> {
        match &self.state {
            WindowDiskState::ExternalChangePending(pending) => Some(pending),
            _ => None,
        }
    }

    /// 창의 base generation(대기 중이면 보존된 LoadedBase).
    pub fn base(&self) -> &DiskSnapshot {
        match &self.state {
            WindowDiskState::Clean { base } | WindowDiskState::Dirty { base } => base,
            WindowDiskState::ExternalChangePending(pending) => &pending.loaded_base,
        }
    }

    /// 로컬 편집이 생겼음을 기록합니다(clean → dirty). 대기 중이면 그대로입니다.
    pub fn mark_dirty(&mut self) {
        if let WindowDiskState::Clean { base } = &self.state {
            self.state = WindowDiskState::Dirty { base: base.clone() };
        }
    }

    /// 저장 성공 뒤 새 base로 clean 상태가 됩니다. 대기 중에는 바꾸지 않습니다(대기 해소는
    /// reconciliation 몫).
    pub fn mark_saved(&mut self, base: DiskSnapshot) -> bool {
        if self.pending().is_some() {
            return false;
        }
        self.state = WindowDiskState::Clean { base };
        true
    }

    /// clean 창이 외부의 새 generation을 다시 불러온 뒤 그 generation을 base로 삼습니다(XIII.3.1).
    ///
    /// 로컬 미저장 변경이 있거나(dirty) 대기 중이면 바꾸지 않고 `false`입니다. 로컬 변경을 버리는
    /// reload는 이 경로로 할 수 없습니다.
    pub fn adopt_reloaded(&mut self, external: DiskSnapshot) -> bool {
        if !matches!(self.state, WindowDiskState::Clean { .. }) {
            return false;
        }
        self.state = WindowDiskState::Clean { base: external };
        true
    }

    /// 외부의 새 generation(XIII.2.5 `ExternalChange::Generation`)을 받습니다.
    ///
    /// - 로컬 변경이 없으면 `CleanWindow`(reload는 호출자가 XIII.3.1로 실행, 상태는 그대로).
    /// - 로컬 변경이 있으면 `capture_memory`로 현재 메모리 snapshot을 캡처해 세 snapshot을 보존한
    ///   `ExternalChangePending`으로 전환합니다. 이미 대기 중이면 ExternalDisk만 최신으로 바꾸고
    ///   LoadedBase는 유지합니다(메모리 snapshot은 새로 캡처).
    /// - 외부 generation이 검증되지 않거나 캡처가 실패하면 상태를 바꾸지 않고 오류입니다.
    pub fn on_external_generation(
        &mut self,
        manifest_hash: &ContentHash,
        capture_memory: impl FnOnce(&DiskSnapshot) -> Result<MemorySnapshot, SnapshotError>,
    ) -> Result<ExternalChangeDecision, ExternalChangeError> {
        if self.base().manifest_hash == *manifest_hash {
            return Ok(ExternalChangeDecision::Unchanged);
        }
        let external = DiskSnapshot::load(&self.root, manifest_hash, self.project_id)
            .map_err(ExternalChangeError::Load)?;
        if matches!(self.state, WindowDiskState::Clean { .. }) {
            return Ok(ExternalChangeDecision::CleanWindow { external });
        }
        let loaded_base = self.base().clone();
        let current_memory = capture_memory(&loaded_base).map_err(ExternalChangeError::Snapshot)?;
        let pending = ExternalChangePending {
            loaded_base,
            current_memory,
            external_disk: external,
        };
        let hashes = pending.hashes();
        // 새 외부 generation(새 정보)일 때만 alert를 다시 냅니다.
        let previous_external = self.pending().map(|p| p.external_disk.manifest_hash);
        if previous_external != Some(hashes.external_disk) {
            self.alert_due = true;
        }
        self.state = WindowDiskState::ExternalChangePending(Box::new(pending));
        Ok(ExternalChangeDecision::Pending(hashes))
    }

    /// 자동 저장 허용 여부. 외부 변경 대기 중이면 `Suspended`입니다.
    pub fn autosave_gate(&self) -> AutosaveGate {
        if self.pending().is_some() {
            AutosaveGate::Suspended
        } else {
            AutosaveGate::Allowed
        }
    }

    /// 셸에 보낼 읽기 전용 alert를 꺼냅니다. 같은 대기 상태에서는 한 번만 나옵니다.
    pub fn take_alert(&mut self) -> Option<ExternalChangeAlert> {
        if !self.alert_due {
            return None;
        }
        let hashes = self.pending()?.hashes();
        self.alert_due = false;
        Some(ExternalChangeAlert {
            hashes,
            read_only: true,
        })
    }

    /// 저장 경로: 대기 중이면 아무 파일도 쓰지 않고 거부하고, 아니면 저장 파이프라인(V.5)을 실행해
    /// 성공 시 새 base로 clean 상태가 됩니다.
    pub fn run_guarded_save(
        &mut self,
        plan: &SavePlan,
        memory: &mut SavedGenerationState,
        cancel: &CancelToken,
    ) -> Result<(), GuardedSaveError> {
        if let Some(pending) = self.pending() {
            return Err(GuardedSaveError::ExternalChangePending(pending.hashes()));
        }
        run_save(&self.root, plan, memory, cancel).map_err(GuardedSaveError::Save)?;
        if let Some((_, hash)) = memory.saved() {
            if let Ok(base) = DiskSnapshot::load(&self.root, &hash, self.project_id) {
                self.mark_saved(base);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::watcher::own_save::tests::{save_generation, TempDir, PROJECT};
    use stellar_storage::chunk::envelope::ChunkKindCode;
    use stellar_storage::chunk::id::ChunkId;
    use stellar_storage::save::commit::SavedGenerationState;

    /// save_generation fixture와 같은 chunk id의 메모리 편집.
    pub(crate) fn memory_edit(payload: &[u8]) -> Vec<LogicalChunkState> {
        vec![LogicalChunkState::Dirty {
            chunk_id: ChunkId::from_stable_id(StableId::new_v7(1, 1, 1)),
            kind: ChunkKindCode(0x8000_0001),
            schema_version: 1,
            payload: payload.to_vec(),
        }]
    }

    pub(crate) fn roots() -> Vec<StableId> {
        vec![StableId::new_v7(2, 2, 2)]
    }

    /// base generation(g1), 창 상태, 외부 writer의 새 generation(g2).
    pub(crate) struct Scenario {
        pub(crate) dir: TempDir,
        pub(crate) window: WindowExternalState,
        pub(crate) base: LoadedGeneration,
        pub(crate) external: LoadedGeneration,
    }

    pub(crate) fn scenario() -> Scenario {
        let dir = TempDir::new();
        let mut memory = SavedGenerationState::new(None);
        let base = save_generation(&dir.path, None, b"v1", &mut memory);
        let window = WindowExternalState::open(&dir.path, DiskSnapshot::from(base.clone()));
        let mut other = SavedGenerationState::new(memory.saved());
        let external = save_generation(&dir.path, Some(&base), b"external", &mut other);
        Scenario {
            dir,
            window,
            base,
            external,
        }
    }

    #[test]
    fn local_changes_create_pending_state_with_three_snapshot_hashes() {
        let mut sc = scenario();
        sc.window.mark_dirty();
        let decision = sc
            .window
            .on_external_generation(&sc.external.manifest_hash, |base| {
                MemorySnapshot::capture(base, memory_edit(b"local edit"), roots())
            })
            .unwrap();

        // 메모리 hash는 같은 상태로 만든 저장 계획의 manifest hash와 같음(결정론).
        let base_loaded = sc.base.clone();
        let expected_memory = build_save_plan(
            Some(&base_loaded),
            PROJECT,
            1,
            &memory_edit(b"local edit"),
            roots(),
        )
        .unwrap()
        .manifest
        .encode_with_hash()
        .unwrap()
        .1;
        let expected = SnapshotHashes {
            loaded_base: sc.base.manifest_hash,
            current_memory: expected_memory,
            external_disk: sc.external.manifest_hash,
        };
        assert_eq!(decision, ExternalChangeDecision::Pending(expected));
        let pending = sc.window.pending().expect("ExternalChangePending");
        assert_eq!(pending.hashes(), expected);
        // 세 hash는 서로 다름.
        assert_ne!(expected.loaded_base, expected.current_memory);
        assert_ne!(expected.current_memory, expected.external_disk);
        assert_ne!(expected.loaded_base, expected.external_disk);

        // 보존: 두 디스크 snapshot은 검증된 manifest 그대로, 메모리 snapshot은 캡처한 chunk 그대로.
        assert_eq!(pending.loaded_base.manifest, sc.base.manifest);
        assert_eq!(pending.external_disk.manifest, sc.external.manifest);
        assert_eq!(pending.current_memory.chunks(), memory_edit(b"local edit"));
        assert_eq!(
            pending.retained_manifests(),
            [sc.base.manifest_hash, sc.external.manifest_hash]
        );
        // 보존 목록은 GC mark(V.7.2)로 실제 도달 가능 hash를 만듦.
        let marked =
            stellar_storage::gc::mark::mark_reachable(&sc.dir.path, &pending.retained_manifests())
                .unwrap();
        assert!(marked.contains(&sc.base.manifest_hash));
        assert!(marked.contains(&sc.external.manifest_hash));
        // 대기 중에는 저장 성공 기록으로 상태가 바뀌지 않음.
        assert!(!sc
            .window
            .mark_saved(DiskSnapshot::from(sc.external.clone())));
    }

    #[test]
    fn same_memory_state_gives_same_hash() {
        let sc = scenario();
        let base = DiskSnapshot::from(sc.base.clone());
        let a = MemorySnapshot::capture(&base, memory_edit(b"x"), roots()).unwrap();
        let b = MemorySnapshot::capture(&base, memory_edit(b"x"), roots()).unwrap();
        let c = MemorySnapshot::capture(&base, memory_edit(b"y"), roots()).unwrap();
        assert_eq!(a.hash(), b.hash());
        assert_ne!(a.hash(), c.hash());
    }

    #[test]
    fn clean_window_is_not_pending_and_base_is_unchanged() {
        let mut sc = scenario();
        let decision = sc
            .window
            .on_external_generation(&sc.external.manifest_hash, |_| {
                panic!("clean 창은 메모리 snapshot을 캡처하지 않음")
            })
            .unwrap();
        assert!(matches!(
            decision,
            ExternalChangeDecision::CleanWindow { ref external } if external.manifest_hash == sc.external.manifest_hash
        ));
        assert!(matches!(sc.window.state(), WindowDiskState::Clean { .. }));
        assert_eq!(
            sc.window
                .on_external_generation(&sc.base.manifest_hash, |_| unreachable!())
                .unwrap(),
            ExternalChangeDecision::Unchanged
        );
    }

    #[test]
    fn later_external_generation_keeps_loaded_base() {
        let mut sc = scenario();
        sc.window.mark_dirty();
        sc.window
            .on_external_generation(&sc.external.manifest_hash, |base| {
                MemorySnapshot::capture(base, memory_edit(b"edit-1"), roots())
            })
            .unwrap();
        let mut other = SavedGenerationState::new(Some((2, sc.external.manifest_hash)));
        let newer = save_generation(&sc.dir.path, Some(&sc.external), b"newer", &mut other);
        let decision = sc
            .window
            .on_external_generation(&newer.manifest_hash, |base| {
                assert_eq!(base.manifest_hash, sc.base.manifest_hash, "LoadedBase 유지");
                MemorySnapshot::capture(base, memory_edit(b"edit-2"), roots())
            })
            .unwrap();
        let ExternalChangeDecision::Pending(hashes) = decision else {
            panic!("Pending이어야 함");
        };
        assert_eq!(hashes.loaded_base, sc.base.manifest_hash);
        assert_eq!(hashes.external_disk, newer.manifest_hash);
    }

    #[test]
    fn unverifiable_external_generation_leaves_state_unchanged() {
        let mut sc = scenario();
        sc.window.mark_dirty();
        let before = sc.window.clone();
        let bogus = ContentHash::compute(b"no such manifest");
        assert!(matches!(
            sc.window.on_external_generation(&bogus, |_| unreachable!()),
            Err(ExternalChangeError::Load(LoadError::MissingManifest { .. }))
        ));
        assert_eq!(sc.window, before);
    }

    #[test]
    fn coalesced_external_generation_drives_pending_state() {
        use crate::watcher::coalesce::{ChangeCoalescer, ExternalChange};
        use crate::watcher::own_save::OwnSaveLedger;
        use crate::watcher::{FileWatchEvent, WatchPath};
        let mut sc = scenario();
        sc.window.mark_dirty();
        let mut coalescer =
            ChangeCoalescer::new(&sc.dir.path, PROJECT, Some(sc.base.manifest_hash), 10);
        coalescer.push(
            &[FileWatchEvent::Modified(
                WatchPath::project(".stellar/cache/chunks/current").unwrap(),
            )],
            0,
        );
        let changes = coalescer.poll(10, &OwnSaveLedger::new());
        let [ExternalChange::Generation { manifest_hash, .. }] = changes.as_slice() else {
            panic!("Generation 하나여야 함: {changes:?}");
        };
        let decision = sc
            .window
            .on_external_generation(manifest_hash, |base| {
                MemorySnapshot::capture(base, memory_edit(b"local"), roots())
            })
            .unwrap();
        assert!(matches!(
            decision,
            ExternalChangeDecision::Pending(hashes) if hashes.external_disk == sc.external.manifest_hash
        ));
    }

    /// 디렉터리 트리의 (상대 경로 → bytes) snapshot.
    fn tree_bytes(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
        fn walk(root: &Path, dir: &Path, out: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                let rel = path.strip_prefix(root).unwrap().to_path_buf();
                if path.is_dir() {
                    walk(root, &path, out);
                } else {
                    out.insert(rel, std::fs::read(&path).unwrap());
                }
            }
        }
        let mut out = std::collections::BTreeMap::new();
        walk(root, root, &mut out);
        out
    }

    #[test]
    fn pending_state_suspends_autosave_and_never_moves_current_pointer() {
        use stellar_storage::generation::current::{current_pointer_path, read_current_pointer};
        let mut sc = scenario();
        sc.window.mark_dirty();
        assert_eq!(sc.window.autosave_gate(), AutosaveGate::Allowed);
        assert_eq!(sc.window.take_alert(), None);
        sc.window
            .on_external_generation(&sc.external.manifest_hash, |base| {
                MemorySnapshot::capture(base, memory_edit(b"local"), roots())
            })
            .unwrap();
        let before_pointer = std::fs::read(current_pointer_path(&sc.dir.path)).unwrap();
        let before_tree = tree_bytes(&sc.dir.path);

        // 자동 저장 중지, 읽기 전용 alert 한 번.
        assert_eq!(sc.window.autosave_gate(), AutosaveGate::Suspended);
        let alert = sc.window.take_alert().unwrap();
        assert!(alert.read_only);
        assert_eq!(alert.hashes, sc.window.pending().unwrap().hashes());
        assert_eq!(sc.window.take_alert(), None, "반복 알림 없음");

        // 메모리 상태로(창의 base 위) 저장하려 해도 거부: 어떤 파일도 쓰지 않음.
        let base = sc.base.clone();
        let plan =
            build_save_plan(Some(&base), PROJECT, 1, &memory_edit(b"local"), roots()).unwrap();
        let mut memory = SavedGenerationState::new(Some((1, sc.base.manifest_hash)));
        for _ in 0..3 {
            assert!(matches!(
                sc.window
                    .run_guarded_save(&plan, &mut memory, &CancelToken::new()),
                Err(GuardedSaveError::ExternalChangePending(_))
            ));
        }
        assert_eq!(
            std::fs::read(current_pointer_path(&sc.dir.path)).unwrap(),
            before_pointer,
            "pending 중 current pointer 변경"
        );
        assert_eq!(
            tree_bytes(&sc.dir.path),
            before_tree,
            "pending 중 파일 기록"
        );
        assert_eq!(
            read_current_pointer(&sc.dir.path).unwrap(),
            sc.external.manifest_hash
        );
        // 저장소 자체의 optimistic check(V.5.2)도 같은 계획을 거부함(이중 방어).
        assert!(matches!(
            run_save(&sc.dir.path, &plan, &mut memory, &CancelToken::new()),
            Err(SaveRunError::Exec(_))
        ));
        assert_eq!(
            std::fs::read(current_pointer_path(&sc.dir.path)).unwrap(),
            before_pointer
        );

        // 더 새 외부 generation은 새 정보이므로 alert를 다시 냄.
        let mut other = SavedGenerationState::new(Some((2, sc.external.manifest_hash)));
        let newer = save_generation(&sc.dir.path, Some(&sc.external), b"newer", &mut other);
        sc.window
            .on_external_generation(&newer.manifest_hash, |base| {
                MemorySnapshot::capture(base, memory_edit(b"local"), roots())
            })
            .unwrap();
        assert_eq!(
            sc.window.take_alert().map(|a| a.hashes.external_disk),
            Some(newer.manifest_hash)
        );
    }

    #[test]
    fn guarded_save_works_outside_pending_state() {
        let sc = scenario();
        // 외부 generation을 base로 다시 연 clean 창은 저장 가능.
        let mut window =
            WindowExternalState::open(&sc.dir.path, DiskSnapshot::from(sc.external.clone()));
        window.mark_dirty();
        assert_eq!(window.autosave_gate(), AutosaveGate::Allowed);
        let ext = sc.external.clone();
        let plan =
            build_save_plan(Some(&ext), PROJECT, 1, &memory_edit(b"saved"), roots()).unwrap();
        let mut memory = SavedGenerationState::new(Some((2, sc.external.manifest_hash)));
        window
            .run_guarded_save(&plan, &mut memory, &CancelToken::new())
            .unwrap();
        assert!(matches!(window.state(), WindowDiskState::Clean { .. }));
        assert_eq!(window.base().generation(), 3);
        assert_eq!(window.take_alert(), None);
    }

    /// 초기 릴리스(R3 이전) 빌드에 agent lock 상태가 없음을 확인합니다(XIII.3.5).
    #[test]
    fn initial_release_build_keeps_no_agent_state() {
        // 1) 대기 상태는 세 snapshot만 가짐: 필드가 늘면 이 완전 분해가 컴파일되지 않습니다.
        let sc = scenario();
        let pending = ExternalChangePending {
            loaded_base: DiskSnapshot::from(sc.base.clone()),
            current_memory: MemorySnapshot::capture(
                &DiskSnapshot::from(sc.base.clone()),
                memory_edit(b"m"),
                roots(),
            )
            .unwrap(),
            external_disk: DiskSnapshot::from(sc.external.clone()),
        };
        let ExternalChangePending {
            loaded_base: _,
            current_memory: _,
            external_disk: _,
        } = pending;
        // 창 상태 변형도 세 가지뿐(새 변형이 생기면 이 match가 컴파일되지 않습니다).
        let variant = |state: &WindowDiskState| match state {
            WindowDiskState::Clean { .. } => 0,
            WindowDiskState::Dirty { .. } => 1,
            WindowDiskState::ExternalChangePending(_) => 2,
        };
        assert_eq!(variant(sc.window.state()), 0);

        // 2) crate 소스에 agent lock 상태 식별자가 없음(검사 문자열은 이 파일과 겹치지 않게 조합).
        let forbidden = [
            concat!("Agent", "Lock"),
            concat!("agent", "_lock"),
            concat!("Locked", "Subtree"),
            concat!("locked", "_subtree"),
            concat!("Agent", "Edit"),
        ];
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![src];
        let mut scanned = 0;
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    scanned += 1;
                    for word in forbidden {
                        assert!(!text.contains(word), "{word} in {path:?}");
                    }
                }
            }
        }
        assert!(scanned > 10);

        // 3) agent lock feature가 없음(feature off가 유일한 빌드).
        let manifest =
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
                .unwrap();
        assert!(!manifest.to_ascii_lowercase().contains("agent"));
        assert!(!manifest.contains("[features]"));
    }
}
